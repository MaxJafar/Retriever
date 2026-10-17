use crate::{
    config::{Config, atomic_json},
    store::Store,
};
use anyhow::{Result, bail};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::{fs, path::Path};

/// Read-only SQLite transaction pins one coherent legacy view. No vector tables
/// or old executables enter candidate state; all pending plans become expired.
pub fn import(source: &Path, destination: &Path) -> Result<Value> {
    let source = source.canonicalize()?;
    if destination.exists() && fs::read_dir(destination)?.next().is_some() {
        bail!("Candidate destination must be empty");
    }
    fs::create_dir_all(destination)?;
    let destination = destination.canonicalize()?;
    if destination == source || destination.starts_with(&source) {
        bail!("Candidate state must be separate from legacy state");
    }
    let mut config: Value = serde_json::from_slice(&fs::read(source.join("config.json"))?)?;
    let mut warnings = Vec::new();
    let defaults = serde_json::to_value(Config::default())?;
    config["schema_version"]=json!(2);
    config["search"]=defaults["search"].clone();
    if let Some(devices)=config["devices"].as_array_mut(){for d in devices{d["analysis_roots"]=json!([]);}}
    warnings.push("Native EmbeddingGemma and Laya use the local model bundle; previous model endpoints are retired".into());
    // Old deployments could permit much larger extraction buffers. Import their
    // catalogue intact, but keep the candidate's next scan within its documented
    // memory bounds and make the changed limit explicit in the acceptance report.
    for (key, lo, hi) in [
        ("max_file_bytes", 1024, 268435456),
        ("max_media_bytes", 1024, 8589934592),
        ("max_total_bytes", 1024, 2147483648),
        ("max_files", 1, 100000),
        ("max_text_chars", 256, 16000000),
        ("ssh_timeout", 1, 300),
    ] {
        let previous = config["limits"][key]
            .as_u64()
            .unwrap_or(defaults["limits"][key].as_u64().unwrap());
        let bounded = previous.clamp(lo, hi);
        if bounded != previous {
            warnings.push(format!("Candidate {key} is bounded at {bounded}; imported cached content is retained until source reconciliation"));
        }
        config["limits"][key] = json!(bounded);
    }
    config["endpoint"] = json!("http://127.0.0.1:18780");
    let config: Config = serde_json::from_value(config)?;
    config.save(&destination)?;
    for name in ["api-token"] {
        if source.join(name).is_file() {
            let bytes = fs::read(source.join(name))?;
            fs::write(destination.join(name), bytes)?;
        }
    }
    let db = Connection::open_with_flags(
        source.join("index.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    db.execute_batch("BEGIN DEFERRED")?;
    let store = Store::open(&destination)?;
    let tables = [
        (
            "files",
            "id,device,path,name,type,extension,text,detail,size,mtime_ns,digest,indexed_at",
        ),
        ("devices", "id,status,checked_at,report"),
        (
            "workspace_meta",
            "device,path,owner,tags,status,starred,properties,revision",
        ),
        ("workspace_comments", "id,device,path,author,body,created"),
        ("workspace_views", "id,name,kind,body,updated"),
        ("workspace_plans", "id,body,state,created,receipt"),
        (
            "folder_sync_pairs",
            "id,name,body,baseline,revision,last_sync",
        ),
    ];
    let mut counts = serde_json::Map::new();
    {
        let mut target = store.db.lock();
        let tx = target.transaction()?;
        for (table, columns) in tables {
            let exists: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
                [table],
                |r| r.get(0),
            )?;
            if !exists {
                continue;
            }
            let mut stmt = db.prepare(&format!("SELECT {columns} FROM {table}"))?;
            let n = stmt.column_count();
            let mut cursor = stmt.query([])?;
            let sql = format!(
                "INSERT INTO {table}({columns}) VALUES({})",
                vec!["?"; n].join(",")
            );
            let mut insert = tx.prepare(&sql)?;
            let mut count = 0;
            while let Some(row) = cursor.next()? {
                let values = (0..n)
                    .map(|i| row.get::<_, rusqlite::types::Value>(i))
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                insert.execute(rusqlite::params_from_iter(values))?;
                count += 1;
            }
            counts.insert(table.into(), json!(count));
        }
        tx.execute("UPDATE workspace_plans SET state='inspection_required' WHERE state IN ('applying','running','check_required')",[])?;
        tx.execute("UPDATE workspace_plans SET state='expired' WHERE state NOT IN ('applied','complete','completed','failed','partial','undone','inspection_required')",[])?;
        tx.execute("UPDATE devices SET status='offline'", [])?;
        tx.commit()?;
    }
    db.execute_batch("COMMIT")?;
    // Original passages are rebuilt from cached text; vectors are always fresh.
    {
        let mut target = store.db.lock();
        let tx = target.transaction()?;
        {
            let mut s = tx.prepare("SELECT id,digest,text FROM files ORDER BY id")?;
            let mut files = s.query([])?;
            while let Some(row) = files.next()? {
                let id: i64 = row.get(0)?;
                let digest: String = row.get(1)?;
                let text: String = row.get(2)?;
                for (ordinal, p) in crate::extract::passages(&text, crate::crawl::effective_chunk(&config), None)
                    .into_iter()
                    .enumerate()
                {
                    tx.execute("INSERT INTO passages(file_id,ordinal,digest,text,coordinates,extraction_revision) VALUES(?,?,?,?,?,?)",rusqlite::params![id,ordinal,digest,p.text,serde_json::to_string(&p.coordinates)?,crate::extract::EXTRACTION_REVISION])?;
                }
            }
        }
        tx.commit()?;
    }
    let report = json!({"schema":1,"candidate":true,"complete":false,"cutover_allowed":false,"imported":counts,"vectors_imported":0,"sources":"cached until reconciled","semantic_ready":false,"warnings":warnings,"created":crate::now()});
    atomic_json(&destination.join("candidate.json"), &report)?;
    Ok(report)
}
