//! A bounded, disk-backed enumeration shared by local and SSH collectors.
//! Continuations never walk the source tree again. Every file is still hashed
//! and screened: timestamps alone are not proof that content is unchanged.
use crate::{
    agent::{mtime, pin},
    config::Device,
    extract, paths,
};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

fn session_dir(forbidden: &[PathBuf]) -> Result<PathBuf> {
    let dir = forbidden
        .first()
        .cloned()
        .unwrap_or_else(|| paths::expand("~/.retriever-agent-candidate"))
        .join("inventory");
    paths::reject_links(&dir)?;
    fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(dir)
}
fn binding(d: &Device, body: &Value) -> String {
    crate::digest(json!({"device":d,"root":body["root"],"limits":body["limits"],"chunk":body["chunk_chars"],"version":2}).to_string())
}
pub fn dispatch(d: &Device, action: &str, body: &Value, forbidden: &[PathBuf]) -> Result<Value> {
    let dir = session_dir(forbidden)?;
    if action == "inventory_begin" {
        return begin(d, body, forbidden, &dir);
    }
    let token = uuid::Uuid::parse_str(
        body["session"]
            .as_str()
            .context("Missing inventory session")?,
    )?;
    let path = dir.join(format!("{token}.sqlite"));
    paths::reject_links(&path)?;
    let mut db = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    let (expected, created, complete): (String, f64, bool) =
        db.query_row("SELECT binding,created,complete FROM meta", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
    if expected != binding(d, body) {
        bail!("Inventory configuration changed");
    }
    if action == "inventory_close" {
        drop(db);
        fs::remove_file(path)?;
        return Ok(json!({"closed":true}));
    }
    if crate::now() - created > 86400.0 {
        bail!("Inventory expired; restart the scan");
    }
    if action == "inventory_finish" {
        let stable = {
            let mut stmt = db.prepare("SELECT path,pin FROM directories")?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            let mut stable = true;
            for row in rows {
                let (p, expected) = row?;
                stable &= pin(std::path::Path::new(&p))
                    .ok()
                    .is_some_and(|v| v.to_string() == expected);
            }
            stable
        };
        let remaining: i64 =
            db.query_row("SELECT count(*) FROM entries WHERE collected=0", [], |r| {
                r.get(0)
            })?;
        return Ok(
            json!({"complete":complete && stable && remaining==0,"tree_stable":stable,"remaining":remaining}),
        );
    }
    let offset = body["offset"].as_u64().unwrap_or(0).min(1_000_000) as i64;
    let count: i64 = db.query_row("SELECT count(*) FROM entries", [], |r| r.get(0))?;
    if offset > count {
        bail!("Invalid inventory continuation");
    }
    let entries: Vec<(i64, String, i64, i64, String)> = {
        let mut stmt = db.prepare("SELECT position,path,size,mtime,extension FROM entries WHERE position>=? ORDER BY position LIMIT 100")?;
        stmt.query_map([offset], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<rusqlite::Result<_>>()?
    };
    if action == "inventory_page" {
        return Ok(
            json!({"records":entries.iter().map(|(_,p,s,m,e)|json!({"path":p,"size":s,"mtime_ns":m,"extension":e})).collect::<Vec<_>>(),"total":count}),
        );
    }
    if action != "inventory_collect" {
        bail!("Unknown inventory action");
    }
    let max_text = body["limits"]["max_text_chars"]
        .as_u64()
        .unwrap_or(1048576)
        .min(16000000) as usize;
    let chunk = body["chunk_chars"].as_u64().unwrap_or(2000).clamp(64, 8000) as usize;
    let max_total = body["limits"]["max_total_bytes"]
        .as_u64()
        .unwrap_or(536870912)
        .clamp(1024, 2147483648);
    let max_files = body["limits"]["max_files"]
        .as_u64()
        .unwrap_or(100)
        .clamp(1, 100) as usize;
    let mut records = Vec::new();
    let mut errors = Vec::new();
    let mut used = 0;
    let mut bytes_read = 0u64;
    let mut next = offset;
    let tx = db.transaction()?;
    for (position, p, size, modified, ext) in entries.into_iter().take(max_files) {
        if position > offset && bytes_read.saturating_add(size.max(0) as u64) > max_total {
            break;
        }
        let result = (|| -> Result<Value> {
            let path = paths::checked(d, &p, forbidden, true)?;
            let before = pin(&path)?;
            if before["size"] != size || before["mtime_ns"] != modified || before["kind"] != "file"
            {
                bail!("source_changed_during_scan");
            }
            let max = extract::source_limit(&body["limits"], &p);
            let (bytes, digest, read) = if extract::kind(&ext) == Some("video") {
                let (digest, read) = extract::stream_source(&path, max, None)?;
                (Vec::new(), digest, read)
            } else {
                let bytes = extract::read_bounded(&path, max)?;
                if paths::secret_content(&bytes) {
                    bail!("secret_content");
                }
                let digest = crate::digest(&bytes);
                let read = bytes.len() as u64;
                (bytes, digest, read)
            };
            bytes_read += read;
            let known = &body["known"][&p];
            let old_digest = known.as_str().or_else(|| known["digest"].as_str());
            let unchanged = old_digest == Some(digest.as_str())
                && (!known.is_object() || known["can_reuse"] == true);
            let mut record = json!({"path":p,"extension":ext,"size":size,"mtime_ns":modified,"digest":digest,"unchanged":unchanged});
            if !unchanged {
                let extraction = match extract::extract(&path, &bytes, max_text, chunk) {
                    Ok(extraction) => extraction,
                    Err(error) if error.to_string().contains("secret_content") => {
                        return Err(error);
                    }
                    Err(error) => extract::Extraction {
                        // Content decoding can fail without making the file or
                        // the directory inventory disappear. Replacing the text
                        // also invalidates stale passages on a changed file.
                        text: String::new(),
                        detail: format!(
                            "metadata: extraction_failed ({})",
                            error.to_string().chars().take(160).collect::<String>()
                        ),
                        passages: Vec::new(),
                    },
                };
                record["extraction"] = serde_json::to_value(extraction)?;
            }
            if pin(&path)? != before {
                bail!("source_changed_during_scan");
            }
            Ok(record)
        })();
        match result {
            Ok(record) => {
                used += serde_json::to_vec(&record)?.len();
                records.push(record);
            }
            Err(error) => {
                errors.push(json!({"path":p,"reason":error.to_string()}));
                // A deliberate privacy exclusion is fully resolved. It must
                // not keep every subsequent folder crawl permanently partial.
                if error.to_string() != "secret_content" {
                    tx.execute("UPDATE meta SET complete=0", [])?;
                }
            }
        }
        tx.execute(
            "UPDATE entries SET collected=1 WHERE position=?",
            [position],
        )?;
        next = position + 1;
        if used >= 1400000 {
            break;
        }
    }
    tx.commit()?;
    Ok(
        json!({"records":records,"errors":errors,"source_bytes":bytes_read,"total":count,"next_offset":if next<count{Some(next)}else{None}}),
    )
}
fn begin(d: &Device, body: &Value, forbidden: &[PathBuf], dir: &std::path::Path) -> Result<Value> {
    let root = body["root"].as_str().context("Missing crawl folder")?;
    if !d.roots.iter().any(|r| {
        paths::normalize(r, d.platform == "windows")
            == paths::normalize(root, d.platform == "windows")
    }) {
        bail!("Choose an included root folder");
    }
    let source = paths::checked(d, root, forbidden, true)?;
    if !source.is_dir() {
        bail!("Source folder is unavailable");
    }
    // Abandoned sessions have a hard lifetime. Only UUID-named files owned by
    // this cache are removed, without following links or recursive deletion.
    let mut active = 0;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|v| v.to_str()) != Some("sqlite")
            || uuid::Uuid::parse_str(path.file_stem().and_then(|v| v.to_str()).unwrap_or(""))
                .is_err()
        {
            continue;
        }
        paths::reject_links(&path)?;
        if entry
            .metadata()?
            .modified()?
            .elapsed()
            .unwrap_or_default()
            .as_secs()
            > 86400
        {
            fs::remove_file(&path)?;
        } else {
            active += 1;
        }
    }
    if active >= 64 {
        bail!("Too many active inventories; close the previous scan");
    }
    let token = uuid::Uuid::new_v4();
    let path = dir.join(format!("{token}.sqlite"));
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let result = (|| -> Result<Value> {
        let mut db = Connection::open(&path)?;
        db.execute_batch("CREATE TABLE meta(binding TEXT,created REAL,complete INTEGER); CREATE TABLE entries(position INTEGER PRIMARY KEY,path TEXT UNIQUE,size INTEGER,mtime INTEGER,extension TEXT,collected INTEGER DEFAULT 0); CREATE TABLE directories(path TEXT PRIMARY KEY,pin TEXT);")?;
        let tx = db.transaction()?;
        let mut count = 0;
        let mut complete = true;
        let mut errors = Vec::new();
        let started=std::time::Instant::now();
        let mut visited=0usize;
        for entry in walkdir::WalkDir::new(&source)
            .follow_links(false)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(|e| {
                paths::eligible(d, &e.path().to_string_lossy())
                    && paths::reject_links(e.path()).is_ok()
                    && !e.path().starts_with(dir)
            })
        {
            visited+=1;
            if visited>250_000 || started.elapsed().as_secs()>=30 {
                complete=false;
                errors.push(json!({"reason":"enumeration_budget","detail":"Choose a smaller crawl scope to finish coverage. Existing indexed files are retained."}));
                break;
            }
            let entry = match entry {
                Ok(v) => v,
                Err(_) => {
                    complete = false;
                    if errors.len() < 100 {
                        errors.push(json!({"reason":"directory_unreadable"}));
                    }
                    continue;
                }
            };
            let p = entry.path();
            if paths::checked(d, &p.to_string_lossy(), forbidden, true).is_err() {
                continue;
            }
            if entry.file_type().is_dir() {
                tx.execute(
                    "INSERT INTO directories VALUES(?,?)",
                    params![p.to_string_lossy(), pin(p)?.to_string()],
                )?;
                continue;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let ext = p
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            if extract::kind(&ext).is_none() {
                continue;
            }
            let m = entry.metadata()?;
            if count >= 1_000_000 {
                bail!("Inventory budget exceeded");
            }
            tx.execute(
                "INSERT INTO entries(position,path,size,mtime,extension) VALUES(?,?,?,?,?)",
                params![count, p.to_string_lossy(), m.len() as i64, mtime(&m), ext],
            )?;
            count += 1;
        }
        tx.execute(
            "INSERT INTO meta VALUES(?,?,?)",
            params![binding(d, body), crate::now(), complete],
        )?;
        tx.commit()?;
        Ok(json!({"session":token,"total":count,"complete":complete,"errors":errors}))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&path);
    }
    result
}
