use crate::store::Store;
use anyhow::{Result, bail};
use memmap2::Mmap;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};
pub fn normalize(values: &[f32], expected: usize) -> Result<Vec<f32>> {
    if values.is_empty()
        || values.len() > 16384
        || (expected > 0 && values.len() != expected)
        || values.iter().any(|v| !v.is_finite())
    {
        bail!("Invalid embedding dimensions or non-finite vector");
    }
    let norm = values
        .iter()
        .map(|v| (*v as f64).powi(2))
        .sum::<f64>()
        .sqrt();
    if !norm.is_finite() || norm < 1e-12 {
        bail!("Embedding is a zero vector");
    }
    Ok(values.iter().map(|v| (*v as f64 / norm) as f32).collect())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Row {
    pub passage: i64,
    pub file: i64,
    pub digest: String,
    #[serde(default)]
    pub extraction: String,
}
pub struct Snapshot {
    pub id: String,
    pub fingerprint: String,
    pub dimensions: usize,
    pub rows: Vec<Row>,
    maps: Vec<Mmap>,
    // The snapshot is immutable; cache only row indices/scores, never file text.
    scores: parking_lot::Mutex<VecDeque<(String, Vec<(usize, f32)>)>>,
}
impl Snapshot {
    pub fn load(store: &Store, fingerprint: &str) -> Result<Option<Arc<Self>>> {
        let record: Option<(String, usize, String, String)> = store
            .db
            .lock()
            .query_row(
                "SELECT id,dimensions,rows,sha256 FROM snapshots WHERE fingerprint=? AND active=1",
                [fingerprint],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((id, dimensions, rows, sha)) = record else {
            return Ok(None);
        };
        if !id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
            bail!("Invalid snapshot identity");
        }
        let rows: Vec<Row> = serde_json::from_str(&rows)?;
        let maps = crate::vector_storage::load(
            &store.state.join("vectors"),
            &id,
            dimensions,
            rows.len(),
            &sha,
        )?;
        Ok(Some(Arc::new(Self {
            id,
            fingerprint: fingerprint.into(),
            dimensions,
            rows,
            maps,
            scores: parking_lot::Mutex::new(VecDeque::new()),
        })))
    }
    /// Keep the best passage per file and apply device/type filters before top-k.
    /// Long documents can no longer crowd every other file out of the shortlist.
    pub fn score_files(
        &self,
        query: &[f32],
        limit: usize,
        allowed: Option<&HashSet<i64>>,
    ) -> Result<Vec<(Row, f32)>> {
        let query = normalize(query, self.dimensions)?;
        let limit = limit.min(1000);
        if limit == 0 {
            return Ok(vec![]);
        }
        let mut hash = Sha256::new();
        for value in &query {
            hash.update(value.to_le_bytes());
        }
        hash.update(limit.to_le_bytes());
        hash.update([u8::from(allowed.is_some())]);
        if let Some(ids) = allowed {
            let mut ids: Vec<_> = ids.iter().copied().collect();
            ids.sort_unstable();
            for id in ids {
                hash.update(id.to_le_bytes());
            }
        }
        let key = hex::encode(hash.finalize());
        if let Some((_, values)) = self.scores.lock().iter().find(|(k, _)| *k == key) {
            return Ok(values
                .iter()
                .map(|(i, s)| (self.rows[*i].clone(), *s))
                .collect());
        }
        let mut files: HashMap<i64, (usize, f32)> = HashMap::new();
        for (i, bytes) in self
            .maps
            .iter()
            .flat_map(|map| map.chunks_exact(self.dimensions * 4))
            .enumerate()
        {
            if allowed.is_some_and(|ids| !ids.contains(&self.rows[i].file)) {
                continue;
            }
            // Independent accumulators shorten the dependency chain on CPU.
            let mut sums = [0.0f32; 4];
            for (j, (b, q)) in bytes.chunks_exact(4).zip(&query).enumerate() {
                sums[j & 3] += f32::from_le_bytes(b.try_into().unwrap()) * q;
            }
            let score = sums.into_iter().sum::<f32>();
            if !score.is_finite() {
                bail!("Corrupt snapshot vector");
            }
            let best = files.entry(self.rows[i].file).or_insert((i, score));
            if score > best.1 {
                *best = (i, score);
            }
        }
        let mut best: Vec<_> = files.into_values().collect();
        let order =
            |a: &(usize, f32), b: &(usize, f32)| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0));
        if best.len() > limit {
            best.select_nth_unstable_by(limit, order);
            best.truncate(limit);
        }
        best.sort_unstable_by(order);
        let result = best
            .iter()
            .map(|(i, s)| (self.rows[*i].clone(), *s))
            .collect();
        let mut cache = self.scores.lock();
        cache.push_back((key, best));
        while cache.len() > 16 {
            cache.pop_front();
        }
        Ok(result)
    }
    pub fn score(&self, query: &[f32], limit: usize) -> Result<Vec<(Row, f32)>> {
        let query = normalize(query, self.dimensions)?;
        let limit = limit.min(1000);
        if limit == 0 {
            return Ok(vec![]);
        }
        let mut best: Vec<(usize, f32)> = Vec::with_capacity(limit + 1);
        for (i, bytes) in self
            .maps
            .iter()
            .flat_map(|map| map.chunks_exact(self.dimensions * 4))
            .enumerate()
        {
            let score = bytes
                .chunks_exact(4)
                .zip(&query)
                .map(|(b, q)| f32::from_le_bytes(b.try_into().unwrap()) * q)
                .sum::<f32>();
            if !score.is_finite() {
                bail!("Corrupt snapshot vector");
            }
            if best.len() < limit || score > best.last().unwrap().1 {
                let at = best.partition_point(|(_, s)| *s >= score);
                best.insert(at, (i, score));
                if best.len() > limit {
                    best.pop();
                }
            }
        }
        Ok(best
            .into_iter()
            .map(|(i, s)| (self.rows[i].clone(), s))
            .collect())
    }
}
pub fn publish(store: &Store, fingerprint: &str) -> Result<Option<String>> {
    let directory = store.state.join("vectors");
    std::fs::create_dir_all(&directory)?;
    let id = uuid::Uuid::new_v4().to_string();
    // A separate WAL reader keeps searches available while vectors stream to disk.
    let reader = Connection::open_with_flags(
        store.state.join("retriever.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    reader.execute_batch("BEGIN DEFERRED")?;
    let revision: i64 =
        reader.query_row("SELECT value FROM revision WHERE id=1", [], |r| r.get(0))?;
    let mut rows = Vec::new();
    let mut sha = Sha256::new();
    let mut dimensions = 0;
    {
        let mut stmt=reader.prepare("SELECT p.id,p.file_id,p.digest,v.vector,v.dimensions,p.extraction_revision FROM vectors v JOIN passages p ON p.id=v.passage_id JOIN files f ON f.id=p.file_id AND f.digest=p.digest WHERE v.fingerprint=? ORDER BY p.id")?;
        let mut cursor = stmt.query([fingerprint])?;
        while let Some(r) = cursor.next()? {
            let d: usize = r.get(4)?;
            let b: Vec<u8> = r.get(3)?;
            if d == 0 || d > 16384 || b.len() != d * 4 || (dimensions != 0 && dimensions != d) {
                bail!("Mixed dimensions; rebuild this embedding index");
            }
            dimensions = d;
            rows.push(Row {
                passage: r.get(0)?,
                file: r.get(1)?,
                digest: r.get(2)?,
                extraction: r.get(5)?,
            });
            sha.update(&b);
        }
    }
    let sha = hex::encode(sha.finalize());
    let row_json = serde_json::to_string(&rows)?;
    let previous: Option<(String, String, String)> = reader
        .query_row(
            "SELECT id,sha256,rows FROM snapshots WHERE active=1 AND fingerprint=?",
            [fingerprint],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let unchanged = previous
        .as_ref()
        .is_some_and(|(_, old_sha, old_rows)| old_sha == &sha && old_rows == &row_json);
    // The first pass avoids any new files for an unchanged snapshot. The second
    // uses the same WAL read transaction and stores only changed immutable blocks.
    if !rows.is_empty() && !unchanged {
        crate::vector_storage::publish(&reader, &directory, &id, fingerprint, dimensions)?;
    }
    reader.execute_batch("COMMIT")?;
    let mut db = store.db.lock();
    let tx = db.transaction()?;
    let latest: i64 = tx.query_row("SELECT value FROM revision WHERE id=1", [], |r| r.get(0))?;
    if revision != latest {
        bail!("Catalogue changed while publishing vectors; retry indexing");
    }
    if rows.is_empty() {
        tx.execute(
            "UPDATE snapshots SET active=0 WHERE fingerprint=?",
            [fingerprint],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO snapshot_revisions SELECT ?,value FROM revision WHERE id=1",
            [fingerprint],
        )?;
        tx.commit()?;
        return Ok(None);
    }
    if let Some((previous_id, previous_sha, previous_rows)) = previous {
        if previous_sha == sha && previous_rows == row_json {
            tx.execute(
                "INSERT OR REPLACE INTO snapshot_revisions SELECT ?,value FROM revision WHERE id=1",
                [fingerprint],
            )?;
            tx.commit()?;
            return Ok(Some(previous_id));
        }
    }
    tx.execute(
        "UPDATE snapshots SET active=0 WHERE fingerprint=?",
        [fingerprint],
    )?;
    tx.execute(
        "INSERT INTO snapshots VALUES(?,?,?,?,?,1)",
        params![id, fingerprint, dimensions, row_json, sha],
    )?;
    tx.execute("UPDATE revision SET value=value+1 WHERE id=1", [])?;
    tx.execute(
        "INSERT OR REPLACE INTO snapshot_revisions SELECT ?,value FROM revision WHERE id=1",
        [fingerprint],
    )?;
    tx.commit()?;
    Ok(Some(id))
}

/// Run with the coordinator stopped before starting an older executable.
pub fn export_legacy_snapshots(state: &std::path::Path) -> Result<serde_json::Value> {
    let db = Connection::open_with_flags(
        state.join("retriever.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut statement =
        db.prepare("SELECT id,dimensions,rows,sha256 FROM snapshots WHERE active=1")?;
    let mut cursor = statement.query([])?;
    let mut exported = Vec::new();
    while let Some(row) = cursor.next()? {
        let id: String = row.get(0)?;
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
            bail!("Invalid snapshot identity");
        }
        let identities: Vec<Row> = serde_json::from_str(&row.get::<_, String>(2)?)?;
        let created = crate::vector_storage::materialize(
            &state.join("vectors"),
            &id,
            row.get(1)?,
            identities.len(),
            &row.get::<_, String>(3)?,
        )?;
        exported.push(serde_json::json!({"id":id,"created":created,"verified":true}));
    }
    Ok(serde_json::json!({"snapshots":exported,"metadata_unchanged":true}))
}
