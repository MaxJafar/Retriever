use crate::{config::Config, extract::Extraction};
use anyhow::Result;
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

#[derive(Default)]
struct LexicalCache {
    epoch: Option<(u64, i64)>,
    entries: VecDeque<(String, Vec<Value>)>,
}

pub struct Store {
    pub db: Mutex<Connection>,
    pub state: PathBuf,
    lexical_cache: Mutex<LexicalCache>,
}
impl Store {
    /// Rechunk imported cached evidence without reading source files. The byte
    /// ceiling is conservative even for byte-fallback tokenization; coordinates
    /// keep their original page/sheet/cell and character offsets.
    pub fn rechunk_cached_passages(&self,cfg:&Config)->Result<usize>{
        let chunk=crate::crawl::effective_chunk(cfg);
        let ceiling=crate::extract::PASSAGE_BYTE_LIMIT;
        let mut db=self.db.lock();let tx=db.transaction()?;
        let files:Vec<i64>={let mut stmt=tx.prepare("SELECT DISTINCT file_id FROM passages WHERE length(CAST(text AS BLOB))>? AND ordinal<1000000")?;stmt.query_map([ceiling],|r|r.get(0))?.collect::<rusqlite::Result<_>>()?};
        for file in &files {
            let rows:Vec<(String,String,String,String)>={let mut stmt=tx.prepare("SELECT digest,text,coordinates,extraction_revision FROM passages WHERE file_id=? AND ordinal<1000000 ORDER BY ordinal")?;stmt.query_map([file],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?.collect::<rusqlite::Result<_>>()?};
            tx.execute("DELETE FROM passages WHERE file_id=? AND ordinal<1000000",[file])?;
            let mut ordinal=0;
            for (digest,text,coords,signature) in rows {
                let original:Value=serde_json::from_str(&coords)?;
                let signature=crate::digest(format!("native-safe-v1:{chunk}:{signature}"));
                for part in crate::extract::passages(&text,chunk,None) {
                    let mut coords=original.clone();let base=original["char_start"].as_u64().unwrap_or(0);
                    coords["char_start"]=json!(base+part.coordinates.char_start as u64);
                    coords["char_end"]=json!(base+part.coordinates.char_end as u64);
                    if let Some(line)=original["line_start"].as_u64(){
                        coords["line_start"]=json!(line+part.coordinates.line_start.unwrap_or(1) as u64-1);
                        coords["line_end"]=json!(line+part.coordinates.line_end.unwrap_or(1) as u64-1);
                    }
                    tx.execute("INSERT INTO passages(file_id,ordinal,digest,text,coordinates,extraction_revision) VALUES(?,?,?,?,?,?)",params![file,ordinal,digest,part.text,coords.to_string(),signature])?;ordinal+=1;
                }
            }
        }
        if !files.is_empty(){tx.execute("UPDATE revision SET value=value+1 WHERE id=1",[])?;}
        tx.commit()?;Ok(files.len())
    }
    pub fn open(state: &Path) -> Result<Self> {
        let db = Connection::open(state.join("retriever.sqlite"))?;
        db.busy_timeout(std::time::Duration::from_secs(10))?;
        let had_analysis_fts: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='analysis_fts')",
            [],
            |r| r.get(0),
        )?;
        db.execute_batch(include_str!("schema.sql"))?;
        if !had_analysis_fts {
            db.execute(
                "INSERT INTO analysis_fts(analysis_fts) VALUES('rebuild')",
                [],
            )?;
        }
        Ok(Self {
            db: Mutex::new(db),
            state: state.to_owned(),
            lexical_cache: Mutex::new(LexicalCache::default()),
        })
    }
    pub fn revision(&self) -> Result<i64> {
        Ok(self
            .db
            .lock()
            .query_row("SELECT value FROM revision WHERE id=1", [], |r| r.get(0))?)
    }
    pub fn cache_epoch(&self) -> Result<(u64, i64)> {
        let db = self.db.lock();
        Ok((
            db.total_changes(),
            db.query_row("PRAGMA data_version", [], |r| r.get(0))?,
        ))
    }
    pub fn upsert(
        &self,
        device: &str,
        path: &str,
        bytes: &[u8],
        mtime: i64,
        ext: &str,
        extraction: &Extraction,
        scan: &str,
    ) -> Result<i64> {
        self.upsert_record(
            device,
            path,
            &crate::digest(bytes),
            bytes.len() as i64,
            mtime,
            ext,
            extraction,
            scan,
        )
    }
    pub fn upsert_record(
        &self,
        device: &str,
        path: &str,
        digest: &str,
        size: i64,
        mtime: i64,
        ext: &str,
        extraction: &Extraction,
        scan: &str,
    ) -> Result<i64> {
        let mut db=self.db.lock();let tx=db.transaction()?;
        let id=Self::upsert_transaction(&tx,device,path,digest,size,mtime,ext,extraction,scan)?;
        tx.commit()?;Ok(id)
    }
    fn upsert_transaction(tx:&rusqlite::Transaction<'_>,device:&str,path:&str,digest:&str,size:i64,mtime:i64,ext:&str,extraction:&Extraction,scan:&str)->Result<i64> {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        tx.execute("INSERT INTO files(device,path,name,type,extension,text,detail,size,mtime_ns,digest,indexed_at,scan_id) VALUES(?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(device,path) DO UPDATE SET name=excluded.name,type=excluded.type,extension=excluded.extension,text=excluded.text,detail=excluded.detail,size=excluded.size,mtime_ns=excluded.mtime_ns,digest=excluded.digest,indexed_at=excluded.indexed_at,scan_id=excluded.scan_id",
            params![device,path,name,crate::extract::kind(ext).unwrap_or("documents"),format!(".{ext}"),extraction.text,extraction.detail,size,mtime,digest,crate::now(),scan])?;
        let id: i64 = tx.query_row(
            "SELECT id FROM files WHERE device=? AND path=?",
            params![device, path],
            |r| r.get(0),
        )?;
        // The source digest alone cannot detect changed passage sizes or a new
        // extraction implementation. Reuse vectors only for identical passages.
        let extraction_signature = crate::digest(serde_json::to_vec(&serde_json::json!({
            "revision": crate::extract::EXTRACTION_REVISION,
            "passages": extraction.passages,
        }))?);
        let current: Option<(String, String)> = tx
            .query_row(
                "SELECT digest,extraction_revision FROM passages WHERE file_id=? LIMIT 1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if current.as_ref().map(|(d, e)| (d.as_str(), e.as_str()))
            != Some((digest, extraction_signature.as_str()))
        {
            // Snapshot rows also carry this signature, so old mapped vectors
            // cannot attach to reused SQLite passage IDs after re-extraction.
            tx.execute("UPDATE revision SET value=value+1 WHERE id=1", [])?;
            tx.execute("DELETE FROM passages WHERE file_id=?", [id])?;
            for (ordinal, p) in extraction.passages.iter().enumerate() {
                tx.execute("INSERT INTO passages(file_id,ordinal,digest,text,coordinates,extraction_revision) VALUES(?,?,?,?,?,?)",params![id,ordinal,digest,p.text,p.coordinates_json(),extraction_signature])?;
            }
        }
        Ok(id)
    }
    /// One commit per bounded crawler page, including extraction identities.
    pub fn apply_collected_page(&self,device:&str,records:&[Value],signature:&str,scan:&str)->Result<(u64,u64)> {
        use anyhow::{Context,bail};
        let mut db=self.db.lock();let tx=db.transaction()?;let mut changed=0;let mut reused=0;
        for record in records {
            let path=record["path"].as_str().context("Missing file identity")?;
            let digest=record["digest"].as_str().context("Missing content digest")?;
            let size=record["size"].as_i64().context("Missing file size")?;
            let mtime=record["mtime_ns"].as_i64().unwrap_or(0);
            if record["unchanged"]==true {
                if tx.execute("UPDATE files SET scan_id=?,size=?,mtime_ns=?,indexed_at=? WHERE device=? AND path=? AND digest=?",params![scan,size,mtime,crate::now(),device,path,digest])?!=1 {bail!("Catalogue changed during incremental scan");}
                reused+=1;
            } else {
                let extraction=serde_json::from_value(record["extraction"].clone())?;
                let id=Self::upsert_transaction(&tx,device,path,digest,size,mtime,record["extension"].as_str().unwrap_or(""),&extraction,scan)?;
                tx.execute("INSERT OR REPLACE INTO extraction_cache VALUES(?,?)",params![id,signature])?;
                changed+=1;
            }
        }
        tx.commit()?;Ok((changed,reused))
    }
    pub fn mark_seen(&self, device: &str, path: &str, scan: &str) -> Result<()> {
        self.db.lock().execute(
            "UPDATE files SET scan_id=? WHERE device=? AND path=?",
            params![scan, device, path],
        )?;
        Ok(())
    }
    pub fn finish_scan(
        &self,
        device: &str,
        scan: &str,
        complete: bool,
        report: &Value,
    ) -> Result<()> {
        let mut db = self.db.lock();
        let tx = db.transaction()?;
        if complete {
            tx.execute(
                "DELETE FROM files WHERE device=? AND scan_id<>?",
                params![device, scan],
            )?;
        }
        tx.execute("INSERT INTO devices VALUES(?,?,?,?) ON CONFLICT(id) DO UPDATE SET status=excluded.status,checked_at=excluded.checked_at,report=excluded.report",params![device,if complete{"ready"}else{"partial"},crate::now(),report.to_string()])?;
        tx.commit()?;
        Ok(())
    }
    pub fn purge(&self, cfg: &Config) -> Result<()> {
        let mut db = self.db.lock();
        let tx = db.transaction()?;
        let ids: Vec<i64> = {
            let mut stmt = tx.prepare("SELECT id,device,path FROM files")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            rows.filter_map(|r| match r {
                Ok((id, d, p))
                    if !cfg
                        .devices
                        .iter()
                        .any(|x| x.id == d && crate::paths::eligible(x, &p)) =>
                {
                    Some(Ok(id))
                }
                Err(e) => Some(Err(e)),
                _ => None,
            })
            .collect::<rusqlite::Result<_>>()?
        };
        for id in ids {
            tx.execute("DELETE FROM files WHERE id=?", [id])?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn result(&self, id: i64, cfg: &Config) -> Result<Option<Value>> {
        self.read_result(id, cfg, true)
    }
    pub fn summary(&self, id: i64, cfg: &Config) -> Result<Option<Value>> {
        self.read_result(id, cfg, false)
    }
    fn read_result(&self, id: i64, cfg: &Config, include_text: bool) -> Result<Option<Value>> {
        let db = self.db.lock();
        let columns = if include_text {
            "f.*"
        } else {
            "f.id,f.device,f.path,f.name,f.type,f.extension,f.detail,f.size,f.mtime_ns,f.digest,f.indexed_at"
        };
        let row=db.query_row(&format!("SELECT {columns},COALESCE(d.status,'offline') source_status FROM files f LEFT JOIN devices d ON d.id=f.device WHERE f.id=?"),[id],row_json).optional()?;
        Ok(row.filter(|r| {
            cfg.devices.iter().any(|d| {
                d.id == r["device"].as_str().unwrap_or("")
                    && crate::paths::eligible(d, r["path"].as_str().unwrap_or(""))
            })
        }))
    }
    pub fn exact_name(
        &self,
        q: &str,
        limit: usize,
        cfg: &Config,
        device: &str,
        kind: &str,
    ) -> Result<Vec<Value>> {
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let db = self.db.lock();
        let mut statement = db.prepare("SELECT f.id,f.device,f.path,f.name,f.type,f.extension,f.detail,f.size,f.mtime_ns,f.digest,f.indexed_at,COALESCE(d.status,'offline') source_status FROM files f LEFT JOIN devices d ON d.id=f.device WHERE (f.name=?1 COLLATE NOCASE OR f.path=?1 COLLATE NOCASE OR f.path=?2 COLLATE NOCASE OR f.path=?3 COLLATE NOCASE) AND (?4='' OR f.device=?4) AND (?5='' OR f.type=?5) ORDER BY f.id LIMIT ?6")?;
        let rows = statement.query_map(
            params![
                q,
                q.replace('\\', "/"),
                q.replace('/', "\\"),
                device,
                kind,
                (limit * 4).min(4000)
            ],
            row_json,
        )?;
        let mut out = Vec::new();
        for row in rows {
            let mut row = row?;
            if !cfg.devices.iter().any(|d| {
                d.id == row["device"]
                    && crate::paths::eligible(d, row["path"].as_str().unwrap_or(""))
            }) {
                continue;
            }
            row["score"] = json!(1.0 / (60.0 + out.len() as f64));
            row["retrieval"] = json!("filename");
            row["snippet"] = json!("");
            out.push(row);
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }
    pub fn lexical(
        &self,
        q: &str,
        limit: usize,
        cfg: &Config,
        device: &str,
        kind: &str,
    ) -> Result<Vec<Value>> {
        let terms: Vec<_> = q
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .take(32)
            .map(|t| format!("\"{}\"*", t.replace('"', "\"\"")))
            .collect();
        if terms.is_empty() {
            return Ok(vec![]);
        }
        let db = self.db.lock();
        // Let FTS choose a bounded matching excerpt. Returning f.text here used
        // to allocate full documents for every candidate on each keystroke.
        // Both own writes and commits on other connections invalidate cached
        // excerpts. Device/root/exclusion changes also form part of the key.
        let epoch = (
            db.total_changes(),
            db.query_row("PRAGMA data_version", [], |r| r.get::<_, i64>(0))?,
        );
        let key = serde_json::to_string(&(q, limit, device, kind, &cfg.devices))?;
        {
            let mut cache = self.lexical_cache.lock();
            if cache.epoch != Some(epoch) {
                cache.entries.clear();
                cache.epoch = Some(epoch);
            }
            if let Some((_, rows)) = cache.entries.iter().find(|(k, _)| *k == key) {
                return Ok(rows.clone());
            }
        }
        let mut stmt=db.prepare("SELECT f.id,f.device,f.path,f.name,f.type,f.extension,f.detail,f.size,f.mtime_ns,f.digest,f.indexed_at,COALESCE(d.status,'offline') source_status,bm25(fts,5.0,1.0) rank,snippet(fts,1,'','',' … ',48) snippet FROM fts JOIN files f ON f.id=fts.rowid LEFT JOIN devices d ON d.id=f.device WHERE fts MATCH ? AND rank MATCH 'bm25(5.0,1.0)' AND (?='' OR f.device=?) AND (?='' OR f.type=?) ORDER BY fts.rank LIMIT ?")?;
        let rows = stmt.query_map(
            params![
                terms.join(" OR "),
                device,
                device,
                kind,
                kind,
                (limit * 4).min(4000)
            ],
            row_json,
        )?;
        let mut out = Vec::new();
        for row in rows {
            let mut r = row?;
            if !cfg.devices.iter().any(|d| {
                d.id == r["device"].as_str().unwrap_or("")
                    && crate::paths::eligible(d, r["path"].as_str().unwrap_or(""))
            }) {
                continue;
            }
            r["snippet"] = Value::String(
                r["snippet"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(900)
                    .collect(),
            );
            r["score"] = json!(1.0 / (60.0 + out.len() as f64));
            r["retrieval"] = json!("words");
            r.as_object_mut().unwrap().remove("text");
            out.push(r);
            if out.len() >= limit {
                break;
            }
        }
        let mut generated=db.prepare("SELECT f.id,f.device,f.path,f.name,f.type,f.extension,f.detail,f.size,f.mtime_ns,f.digest,f.indexed_at,COALESCE(d.status,'offline') source_status,a.body provenance,bm25(analysis_fts) rank,snippet(analysis_fts,0,'','',' … ',48) snippet FROM analysis_fts JOIN file_analysis a ON a.file_id=analysis_fts.rowid JOIN files f ON f.id=a.file_id LEFT JOIN devices d ON d.id=f.device WHERE analysis_fts MATCH ? AND (?='' OR f.device=?) AND (?='' OR f.type=?) ORDER BY rank,f.id LIMIT ?")?;
        let matches = generated.query_map(
            params![terms.join(" OR "), device, device, kind, kind, limit],
            row_json,
        )?;
        for (rank, r) in matches.enumerate() {
            let mut r = r?;
            if !cfg.devices.iter().any(|d| {
                d.id == r["device"].as_str().unwrap_or("")
                    && crate::paths::eligible(d, r["path"].as_str().unwrap_or(""))
            }) {
                continue;
            }
            r["coordinates"] = serde_json::from_str(r["provenance"].as_str().unwrap_or("{}"))?;
            r.as_object_mut().unwrap().remove("provenance");
            r["snippet"] = json!(
                r["snippet"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(900)
                    .collect::<String>()
            );
            // Generated text is an independent lexical index, not an append-only
            // tail behind every incidental original-word match. Use its own
            // rank, and keep the stronger excerpt when both indexes match.
            r["score"] = json!(1.0 / (60.0 + rank as f64));
            r["retrieval"] = json!("generated");
            r["generated"] = json!(true);
            if let Some(existing) = out.iter_mut().find(|old| old["id"] == r["id"]) {
                if r["score"].as_f64() > existing["score"].as_f64() {
                    *existing = r;
                }
            } else {
                out.push(r);
            }
        }
        out.sort_by(|a, b| {
            b["score"]
                .as_f64()
                .unwrap_or(0.0)
                .total_cmp(&a["score"].as_f64().unwrap_or(0.0))
        });
        out.truncate(limit);
        if serde_json::to_vec(&out)?.len() <= 256 * 1024 {
            let mut cache = self.lexical_cache.lock();
            if cache.entries.len() >= 32 {
                cache.entries.pop_front();
            }
            cache.entries.push_back((key, out.clone()));
        }
        Ok(out)
    }
    pub fn device_status(&self, cfg: &Config) -> Result<Vec<Value>> {
        let db = self.db.lock();
        let mut out = Vec::new();
        for d in &cfg.devices {
            let mut v = serde_json::to_value(d)?;
            let (status, checked, report) = db
                .query_row(
                    "SELECT status,checked_at,report FROM devices WHERE id=?",
                    [&d.id],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, f64>(1)?,
                            r.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()?
                .unwrap_or(("not_indexed".into(), 0.0, "{}".into()));
            v["status"] = json!(status);
            v["checked_at"] = json!(checked);
            v["report"] = serde_json::from_str(&report)?;
            v["files"] = json!(db.query_row(
                "SELECT count(*) FROM files WHERE device=?",
                [&d.id],
                |r| r.get::<_, i64>(0)
            )?);
            out.push(v);
        }
        Ok(out)
    }
}
impl crate::extract::Passage {
    fn coordinates_json(&self) -> String {
        serde_json::to_string(&self.coordinates).unwrap()
    }
}
pub fn row_json(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let mut m = serde_json::Map::new();
    for (i, name) in row.as_ref().column_names().iter().enumerate() {
        let v = match row.get_ref(i)? {
            rusqlite::types::ValueRef::Null => Value::Null,
            rusqlite::types::ValueRef::Integer(v) => json!(v),
            rusqlite::types::ValueRef::Real(v) => json!(v),
            rusqlite::types::ValueRef::Text(v) => json!(String::from_utf8_lossy(v)),
            rusqlite::types::ValueRef::Blob(_) => continue,
        };
        m.insert(name.to_string(), v);
    }
    Ok(Value::Object(m))
}
pub fn snippet(text: &str, query: &str, len: usize) -> String {
    let word = query.split_whitespace().next().unwrap_or("").to_lowercase();
    let lower = text.to_lowercase();
    let at = lower
        .find(&word)
        .map(|i| lower[..i].chars().count().saturating_sub(80))
        .unwrap_or(0);
    text.chars().skip(at).take(len).collect()
}
