//! Cached text -> content-addressed embeddings. Source checks occur per file,
//! not for every small passage batch; publishing a file's vectors is atomic.
use crate::{config::Config, engine::Engine, providers::fingerprint, scheduler::Priority, vectors};
use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, atomic::Ordering},
    time::Instant,
};
type PendingFile = (i64, String, String, String);

impl Engine {
    pub(crate) async fn embed_pending(self: &Arc<Self>, job: &str, cfg: &Config) -> Result<()> {
        let settings = &cfg.search["embedding"];
        if settings["provider"] == "disabled" {
            return Ok(());
        }
        if self.providers.native.status("embedding")["installed"] != true {
            bail!("Install the bundled EmbeddingGemma model in the app data models folder");
        }
        if !self.providers.retry_ready("embedding", settings) {
            bail!("Local embedding runtime is cooling down; pending work will resume");
        }
        self.store.rechunk_cached_passages(cfg)?;
        let fp = fingerprint(settings);
        let dims = 768;
        let batch = settings["batch_size"].as_u64().unwrap_or(1).clamp(1, 32) as usize;
        let mut wrote = false;
        let mut inferred = 0;
        let mut reused = 0;
        let mut files = 0;
        let mut source_checks = 0;
        // A full vector snapshot duplicates every published vector. Bound the
        // number written during a pass instead of publishing on a timer: large
        // libraries must not consume another full copy every few minutes.
        let (passage_count, has_snapshot): (usize, bool) = {
            let db = self.store.db.lock();
            let count = db.query_row("SELECT count(*) FROM passages", [], |r| r.get(0))?;
            let active = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM snapshots WHERE fingerprint=? AND active=1)",
                [&fp],
                |r| r.get(0),
            )?;
            (count, active)
        };
        let publication_step = passage_count.div_ceil(8).max(32);
        let mut publish_after = if has_snapshot { publication_step } else { 32 };
        let mut unpublished_passages = 0;
        let mut inference_error = None;
        let mut cursor = 0i64;
        let started = Instant::now();
        let (mut source_ms, mut inference_ms, mut cache_write_ms) = (0u128, 0u128, 0u128);
        'work: loop {
            if self.cancel.load(Ordering::SeqCst)
                || fingerprint(&self.config.read().search["embedding"]) != fp
            {
                break;
            }
            let documents: Vec<PendingFile> = {
                let db = self.store.db.lock();
                let mut statement=db.prepare("SELECT f.id,f.device,f.path,f.digest FROM files f WHERE f.id>?3 AND EXISTS(SELECT 1 FROM passages p LEFT JOIN vectors v ON v.passage_id=p.id AND v.fingerprint=?1 WHERE p.file_id=f.id AND p.digest=f.digest AND v.passage_id IS NULL) AND NOT EXISTS(SELECT 1 FROM embedding_attempts a WHERE a.file_id=f.id AND a.fingerprint=?1 AND a.digest=f.digest AND a.retry_after>?2) ORDER BY f.id LIMIT 8")?;
                statement
                    .query_map(params![fp, crate::now(), cursor], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                    })?
                    .collect::<rusqlite::Result<_>>()?
            };
            let Some(last) = documents.last() else {
                break;
            };
            // Each file is considered once in this pass. Restarting at the first
            // row after every file makes large initial indexes quadratic and can
            // repeatedly revisit unavailable files as their retry time expires.
            cursor = last.0;
            source_checks += documents.len();
            let checked = Instant::now();
            let before = self.fingerprint_files(cfg, &documents).await?;
            source_ms += checked.elapsed().as_millis();
            let mut completed_files = Vec::new();
            for ((file, device, path, digest), before) in documents.into_iter().zip(before) {
                if !before.as_ref().is_ok_and(|v| v["digest"] == digest) {
                    self.defer_embedding(
                        file,
                        &fp,
                        &digest,
                        before
                            .err()
                            .map(|e| e.to_string())
                            .unwrap_or_else(|| "source_changed_before_inference".into()),
                    )?;
                    continue;
                }
                let passages: Vec<(i64, String)> = {
                    let db = self.store.db.lock();
                    let mut stmt=db.prepare("SELECT p.id,p.text FROM passages p LEFT JOIN vectors v ON v.passage_id=p.id AND v.fingerprint=? WHERE p.file_id=? AND p.digest=? AND v.passage_id IS NULL ORDER BY p.ordinal")?;
                    stmt.query_map(params![fp, file, digest], |r| Ok((r.get(0)?, r.get(1)?)))?
                        .collect::<rusqlite::Result<_>>()?
                };
                let mut completed = Vec::with_capacity(passages.len());
                for rows in passages.chunks(batch) {
                    if self.cancel.load(Ordering::SeqCst)
                        || self.config.read().revision() != cfg.revision()
                    {
                        break 'work;
                    }
                    let inputs = rows
                        .iter()
                        .map(|(_, t)| {
                            format!(
                                "{}{}",
                                settings["document_prefix"].as_str().unwrap_or(""),
                                t
                            )
                        })
                        .collect::<Vec<_>>();
                    let keys = inputs.iter().map(crate::digest).collect::<Vec<_>>();
                    let mut resolved = HashMap::new();
                    let mut missing = Vec::new();
                    let mut missing_keys = Vec::new();
                    {
                        let db = self.store.db.lock();
                        let mut stmt=db.prepare_cached("SELECT vector,dimensions FROM embedding_cache WHERE fingerprint=? AND content_hash=?")?;
                        for (key, input) in keys.iter().zip(&inputs) {
                            if resolved.contains_key(key) || missing_keys.contains(key) {
                                continue;
                            }
                            let cached: Option<(Vec<u8>, usize)> = stmt
                                .query_row(params![fp, key], |r| Ok((r.get(0)?, r.get(1)?)))
                                .optional()?;
                            if let Some((bytes, width)) = cached {
                                if width == dims && bytes.len() == width * 4 {
                                    let v = bytes
                                        .chunks_exact(4)
                                        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                                        .collect::<Vec<_>>();
                                    if let Ok(v) = vectors::normalize(&v, width) {
                                        resolved.insert(key.clone(), v);
                                        continue;
                                    }
                                }
                            }
                            missing_keys.push(key.clone());
                            missing.push(json!(input));
                        }
                    }
                    if !missing.is_empty() {
                        let inference_started = Instant::now();
                        let values = match self
                            .providers
                            .embed(settings, missing, Priority::Background, dims)
                            .await
                        {
                            Ok(v) => v,
                            Err(e) => {
                                inference_error = Some(e);
                                break 'work;
                            }
                        };
                        inference_ms += inference_started.elapsed().as_millis();
                        for (key, v) in missing_keys.iter().zip(values) {
                            resolved.insert(key.clone(), v);
                        }
                        let cache_started = Instant::now();
                        let mut db = self.store.db.lock();
                        let tx = db.transaction()?;
                        for key in &missing_keys {
                            let bytes = resolved[key]
                                .iter()
                                .flat_map(|v| v.to_le_bytes())
                                .collect::<Vec<_>>();
                            tx.execute(
                                "INSERT OR REPLACE INTO embedding_cache VALUES(?,?,?,?)",
                                params![fp, key, bytes, dims],
                            )?;
                        }
                        tx.commit()?;
                        cache_write_ms += cache_started.elapsed().as_millis();
                    }
                    inferred += missing_keys.len();
                    reused += rows.len() - missing_keys.len();
                    for ((id, _), key) in rows.iter().zip(&keys) {
                        completed.push((
                            *id,
                            resolved[key]
                                .iter()
                                .flat_map(|v| v.to_le_bytes())
                                .collect::<Vec<_>>(),
                        ));
                    }
                    self.progress(job,json!({"phase":"embedding","progress":{"files":files,"inferred_passages":inferred,"reused_passages":reused,"source_checks":source_checks,"elapsed_ms":started.elapsed().as_millis(),"source_ms":source_ms,"inference_ms":inference_ms,"cache_write_ms":cache_write_ms}}));
                }
                completed_files.push(((file, device, path, digest), completed));
            }
            if completed_files.is_empty() {
                continue;
            }
            let documents = completed_files
                .iter()
                .map(|(file, _)| file.clone())
                .collect::<Vec<_>>();
            source_checks += documents.len();
            let checked = Instant::now();
            let after = self.fingerprint_files(cfg, &documents).await?;
            source_ms += checked.elapsed().as_millis();
            for (((file, _device, _path, digest), completed), after) in
                completed_files.into_iter().zip(after)
            {
                if !after.as_ref().is_ok_and(|v| v["digest"] == digest) {
                    self.defer_embedding(
                        file,
                        &fp,
                        &digest,
                        after
                            .err()
                            .map(|e| e.to_string())
                            .unwrap_or_else(|| "source_changed_during_inference".into()),
                    )?;
                    continue;
                }
                let completed_count = completed.len();
                {
                    let current = self.config.read();
                    if current.revision() != cfg.revision() || self.cancel.load(Ordering::SeqCst) {
                        break 'work;
                    }
                    let mut db = self.store.db.lock();
                    let tx = db.transaction()?;
                    for (id, bytes) in completed {
                        tx.execute("INSERT OR IGNORE INTO vectors SELECT p.id,?,?,? FROM passages p JOIN files f ON f.id=p.file_id WHERE p.id=? AND p.file_id=? AND p.digest=? AND f.digest=?",params![fp,bytes,dims,id,file,digest,digest])?;
                    }
                    tx.execute(
                        "DELETE FROM embedding_attempts WHERE file_id=? AND fingerprint=?",
                        params![file, fp],
                    )?;
                    tx.commit()?;
                }
                wrote = true;
                files += 1;
                unpublished_passages += completed_count;
            }
            // A new library gets one early preview, then at most eight progress
            // snapshots per full pass, followed by the final checkpoint.
            if unpublished_passages >= publish_after {
                let engine = self.clone();
                let key = fp.clone();
                tokio::task::spawn_blocking(move || vectors::publish(&engine.store, &key))
                    .await??;
                self.invalidate_snapshot(&fp);
                wrote = false;
                unpublished_passages = 0;
                publish_after = publication_step;
            }
        }
        if wrote || self.snapshot_dirty(&fp)? {
            let engine = self.clone();
            let key = fp.clone();
            tokio::task::spawn_blocking(move || vectors::publish(&engine.store, &key)).await??;
            self.invalidate_snapshot(&fp);
        }
        let pending:i64=self.store.db.lock().query_row("SELECT count(*) FROM passages p LEFT JOIN vectors v ON v.passage_id=p.id AND v.fingerprint=? WHERE v.passage_id IS NULL",[&fp],|r|r.get(0))?;
        self.progress(job,json!({"phase":"indexed","embedding":{"files":files,"inferred_passages":inferred,"reused_passages":reused,"source_checks":source_checks,"pending_passages":pending,"elapsed_ms":started.elapsed().as_millis(),"source_ms":source_ms,"inference_ms":inference_ms,"cache_write_ms":cache_write_ms}}));
        if let Some(error) = inference_error {
            return Err(error);
        }
        if pending > 0 {
            bail!(
                "{pending} passages await available, unchanged source files; completed vectors are searchable"
            );
        }
        Ok(())
    }
    async fn fingerprint_files(
        &self,
        cfg: &Config,
        files: &[PendingFile],
    ) -> Result<Vec<std::result::Result<Value, String>>> {
        let mut results = vec![Err("Source verification did not complete".into()); files.len()];
        let mut groups = HashMap::<String, Vec<usize>>::new();
        for (index, (_, device, _, _)) in files.iter().enumerate() {
            groups.entry(device.clone()).or_default().push(index);
        }
        for (device, indices) in groups {
            let source = cfg.device(&device)?;
            let mut validation = cfg.clone();
            validation.devices = vec![source.clone()];
            validation.validate()?;
            let mut bodies = Vec::with_capacity(indices.len());
            for &index in &indices {
                let path = &files[index].2;
                bodies.push(json!({"path":path,"maximum":crate::extract::source_limit(&cfg.limits,path),"device_config":source,"_framed_response":true}));
            }
            if source.transport == "local" {
                for (index, body) in indices.into_iter().zip(bodies) {
                    results[index] = self
                        .source(source, "fingerprint", body)
                        .await
                        .map_err(|error| error.to_string());
                }
            } else {
                match self.nodes.fingerprints(cfg, source, bodies).await {
                    Ok(values) => {
                        for (index, value) in indices.into_iter().zip(values) {
                            results[index] = value;
                        }
                    }
                    Err(error) => {
                        for index in indices {
                            results[index] = Err(error.to_string());
                        }
                    }
                }
            }
        }
        Ok(results)
    }
    fn defer_embedding(&self, file: i64, fp: &str, digest: &str, error: String) -> Result<()> {
        let db = self.store.db.lock();
        if error.contains("source_changed") || error.contains("secret_content") {
            db.execute("UPDATE files SET text='',digest='',detail='source_changed_during_inference' WHERE id=?",[file])?;
        }
        db.execute(
            "INSERT OR REPLACE INTO embedding_attempts VALUES(?,?,?,?,?)",
            params![file, fp, digest, crate::now() + 300.0, error],
        )?;
        Ok(())
    }
}
