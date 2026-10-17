//! Folder schedules belong to the coordinator, never to a launcher process.
use crate::{
    config::{Config, Device},
    engine::Engine,
    paths,
};
use anyhow::{Context, Result, bail};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

pub fn within(device: &Device, root: &str, path: &str) -> bool {
    let root = paths::normalize(root, device.platform == "windows");
    let path = paths::normalize(path, device.platform == "windows");
    path == root || path.starts_with(&(root + "/"))
}
pub fn extraction_signature(cfg: &Config) -> String {
    crate::digest(json!({"revision":crate::extract::EXTRACTION_REVISION,"max_file_bytes":cfg.limits["max_file_bytes"],"max_media_bytes":cfg.limits["max_media_bytes"],"max_text_chars":cfg.limits["max_text_chars"],"chunk_chars":effective_chunk(cfg)}).to_string())
}
pub fn effective_chunk(cfg: &Config) -> usize {
    let s = &cfg.search["embedding"];
    // The extractor separately enforces the UTF-8 byte ceiling. Dividing all
    // text by four needlessly multiplied English/code inference work.
    (s["chunk_chars"].as_u64().unwrap_or(2000) as usize)
        .clamp(64, 8000)
}
impl Engine {
    pub fn crawl_status(&self) -> Result<Value> {
        let cfg = self.config.read().clone();
        let db = self.store.db.lock();
        let mut folders = Vec::new();
        for d in cfg.devices.iter().filter(|d| d.enabled) {
            for root in &d.roots {
                db.execute(
                    "INSERT OR IGNORE INTO crawl_folders(device,root,analysis,interval_minutes) VALUES(?,?,?,0)",
                    params![
                        d.id,
                        root,
                        if d.analysis_roots.contains(root) {
                            "text_vision"
                        } else {
                            "off"
                        }
                    ],
                )?;
                let mut row = db.query_row(
                    "SELECT * FROM crawl_folders WHERE device=? AND root=?",
                    params![d.id, root],
                    crate::store::row_json,
                )?;
                row["report"] = serde_json::from_str(row["report"].as_str().unwrap_or("{}"))?;
                row["label"] = json!(d.label);
                folders.push(row);
            }
        }
        Ok(
            json!({"folders":folders,"running":self.syncing.load(Ordering::SeqCst),"progress":self.progress.read().clone(),"scheduler":"coordinator","catch_up":"one scan per overdue folder","disclosure":"Selected folders are screened and extracted into the local catalogue. EmbeddingGemma indexes cached passages; Laya reranks search results on this computer. Originals remain in their folders."}),
        )
    }
    pub fn crawl_settings(&self, body: &Value) -> Result<Value> {
        let _guard = self
            .mutation
            .try_lock()
            .map_err(|_| anyhow::anyhow!("conflict: A file operation is active"))?;
        let cfg = self.config.read().clone();
        if cfg.role == "client" {
            bail!("Change crawl settings on the coordinator");
        }
        let device = body["device"].as_str().context("Missing device")?;
        let root = body["root"].as_str().context("Missing root")?;
        let d = cfg.device(device)?;
        if !d.enabled || !d.roots.contains(&root.to_owned()) {
            bail!("Choose an enabled included folder");
        }
        let interval = body["interval_minutes"]
            .as_i64()
            .context("Choose a whole number of minutes")?;
        if interval != 0 && !(15..=43200).contains(&interval) {
            bail!("Use manual (0), or 15 to 43200 minutes");
        }
        let analysis = body["analysis"]
            .as_str()
            .context("Missing analysis choice")?;
        if analysis != "off" {
            bail!("Generative analysis was removed; native retrieval is automatic");
        }
        let revision = body["revision"]
            .as_i64()
            .context("Missing folder revision")?;
        self.crawl_status()?;
        {
            let mut db = self.store.db.lock();
            let tx = db.transaction()?;
            let changed=tx.execute("UPDATE crawl_folders SET interval_minutes=?,analysis=?,revision=revision+1,next_run=CASE WHEN ?=0 THEN 0 ELSE ?+?*60 END WHERE device=? AND root=? AND revision=?",params![interval,analysis,interval,crate::now(),interval,device,root,revision])?;
            if changed == 0 {
                bail!("conflict: Folder settings changed. Refresh before saving.");
            }
            tx.commit()?;
        }
        self.reconcile_knowledge()?;
        self.crawl_status()
    }
    pub fn due_folder(&self, now: f64) -> Result<Option<(String, String)>> {
        let status = self.crawl_status()?;
        if !self.config.read().configured || self.config.read().role == "client" {
            return Ok(None);
        }
        let mut folders = status["folders"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| {
                f["interval_minutes"].as_i64().unwrap_or(0) > 0
                    && f["next_run"].as_f64().unwrap_or(0.0) <= now
            })
            .collect::<Vec<_>>();
        folders.sort_by(|a, b| {
            a["next_run"]
                .as_f64()
                .unwrap_or(0.0)
                .total_cmp(&b["next_run"].as_f64().unwrap_or(0.0))
        });
        Ok(folders.first().map(|f| {
            (
                f["device"].as_str().unwrap().into(),
                f["root"].as_str().unwrap().into(),
            )
        }))
    }
    pub fn start_folder(self: &Arc<Self>, body: &Value) -> Result<Value> {
        let device = body["device"].as_str().context("Missing device")?;
        let root = body["root"].as_str().context("Missing root")?;
        let cfg = self.config.read();
        let d = cfg.device(device)?;
        if !d.enabled || !d.roots.contains(&root.to_owned()) {
            bail!("Choose an enabled included folder");
        }
        drop(cfg);
        if body.get("index_only").is_some_and(|v|!v.is_boolean()){bail!("index_only must be a boolean");}
        self.start_job(body["index_only"].as_bool().unwrap_or(false), Some((device.into(), root.into())))
    }
    pub(crate) async fn crawl_roots(
        self: &Arc<Self>,
        job: &str,
        cfg: &Config,
        scope: Option<&(String, String)>,
    ) -> Result<()> {
        self.crawl_status()?;
        for d in cfg.devices.iter().filter(|d| d.enabled) {
            let mut completed_roots = Vec::new();
            let mut all_complete = true;
            let mut reports = Vec::new();
            for root in &d.roots {
                if scope.is_some_and(|(device, folder)| device != &d.id || folder != root) {
                    continue;
                }
                if self.cancel.load(Ordering::SeqCst) {
                    return Ok(());
                }
                self.store.db.lock().execute("UPDATE crawl_folders SET state='running',last_started=? WHERE device=? AND root=?",params![crate::now(),d.id,root])?;
                let result = self.crawl_root(job, cfg, d, root).await;
                let report = result.unwrap_or_else(
                    |e| json!({"complete":false,"errors":[{"reason":e.to_string()}]}),
                );
                let cancelled = self.cancel.load(Ordering::SeqCst);
                let complete = report["complete"] == true && !cancelled;
                all_complete &= complete;
                if complete {
                    completed_roots.push(root.clone());
                }
                reports.push(report.clone());
                let db = self.store.db.lock();
                let (interval, failures): (i64, i64) = db.query_row(
                    "SELECT interval_minutes,failures FROM crawl_folders WHERE device=? AND root=?",
                    params![d.id, root],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                let failures = if complete || cancelled {
                    0
                } else {
                    (failures + 1).min(10)
                };
                let delay = if complete || cancelled {
                    interval * 60
                } else {
                    ((60i64) << failures.min(6)).min(3600)
                };
                db.execute("UPDATE crawl_folders SET state=?,last_completed=CASE WHEN ? THEN ? ELSE last_completed END,next_run=?,failures=?,report=? WHERE device=? AND root=?",params![if cancelled{"cancelled"}else if complete{"ready"}else{"partial"},complete,crate::now(),if interval==0{0.0}else{crate::now()+delay as f64},failures,report.to_string(),d.id,root])?;
            }
            if !reports.is_empty() {
                let latest = self.config.read();
                let mut db = self.store.db.lock();
                let tx = db.transaction()?;
                if all_complete
                    && latest.revision() == cfg.revision()
                    && !self.cancel.load(Ordering::SeqCst)
                {
                    let stale: Vec<(i64, String)> = {
                        let mut stmt =
                            tx.prepare("SELECT id,path FROM files WHERE device=? AND scan_id<>?")?;
                        stmt.query_map(params![d.id, job], |r| Ok((r.get(0)?, r.get(1)?)))?
                            .collect::<rusqlite::Result<_>>()?
                    };
                    for (id, path) in stale {
                        if completed_roots.iter().any(|root| within(d, root, &path)) {
                            tx.execute("DELETE FROM files WHERE id=?", [id])?;
                        }
                    }
                }
                tx.execute("INSERT INTO devices VALUES(?,?,?,?) ON CONFLICT(id) DO UPDATE SET status=excluded.status,checked_at=excluded.checked_at,report=excluded.report",params![d.id,if all_complete{"ready"}else{"partial"},crate::now(),json!({"complete":all_complete,"folders":reports}).to_string()])?;
                tx.commit()?;
            }
        }
        Ok(())
    }
    async fn crawl_root(&self, job: &str, cfg: &Config, d: &Device, root: &str) -> Result<Value> {
        let signature = extraction_signature(cfg);
        let mut body = json!({"root":root,"limits":cfg.limits,"chunk_chars":effective_chunk(cfg)});
        let begin = self.source(d, "inventory_begin", body.clone()).await?;
        body["session"] = begin["session"].clone();
        let mut report = json!({"complete":false,"total":begin["total"],"changed":0,"unchanged":0,"source_bytes":0,"errors":begin["errors"]});
        let result=async {
            let mut offset=0u64;
            loop {
                if self.cancel.load(Ordering::SeqCst) {return Ok(());}
                body["offset"]=json!(offset);
                let page=self.source(d,"inventory_page",body.clone()).await?;
                let mut known=serde_json::Map::new();
                {
                    let db=self.store.db.lock();
                    let mut stmt=db.prepare("SELECT f.digest,COALESCE(c.signature,''),f.detail FROM files f LEFT JOIN extraction_cache c ON c.file_id=f.id WHERE f.device=? AND f.path=?")?;
                    for file in page["records"].as_array().context("Malformed inventory page")? {
                        let path=file["path"].as_str().context("Missing inventory path")?;
                        let known_file:Option<(String,String,String)>=stmt.query_row(params![d.id,path],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
                        if let Some((digest,policy,detail))=known_file {known.insert(path.into(),json!({"digest":digest,"can_reuse":policy==signature && !detail.contains("pdfium_not_packaged")}));}
                    }
                }
                body["known"]=Value::Object(known);
                let page=self.source(d,"inventory_collect",body.clone()).await?;
                report["source_bytes"]=json!(report["source_bytes"].as_u64().unwrap_or(0)+page["source_bytes"].as_u64().unwrap_or(0));
                for error in page["errors"].as_array().context("Malformed inventory errors")? {
                    let reason=error["reason"].as_str().unwrap_or("");
                    if reason.contains("secret_content") {
                        self.store.db.lock().execute("DELETE FROM files WHERE device=? AND path=?",params![d.id,error["path"].as_str().unwrap_or("")])?;
                    } else if reason.contains("source_changed") {
                        self.store.db.lock().execute("UPDATE files SET text='',digest='',detail='source_changed_during_scan' WHERE device=? AND path=?",params![d.id,error["path"].as_str().unwrap_or("")])?;
                    }
                    if report["errors"].as_array().unwrap().len()<200 {report["errors"].as_array_mut().unwrap().push(error.clone());}
                }
                {
                    let latest=self.config.read();
                    let records=page["records"].as_array().context("Malformed inventory records")?;
                    if latest.revision()!=cfg.revision() || records.iter().any(|r|!latest.devices.iter().any(|a|a.id==d.id && paths::eligible(a,r["path"].as_str().unwrap_or("")))) {
                        bail!("Configuration changed during scan; catalogue retained");
                    }
                    let (changed,reused)=self.store.apply_collected_page(&d.id,records,&signature,job)?;
                    report["changed"]=json!(report["changed"].as_u64().unwrap_or(0)+changed);
                    report["unchanged"]=json!(report["unchanged"].as_u64().unwrap_or(0)+reused);
                }
                self.progress(job,json!({"phase":"collecting","current_device":d.id,"root":root,"progress":report}));
                if let Some(next)=page["next_offset"].as_u64() {if next<=offset{bail!("Invalid scan continuation");}offset=next;} else {break;}
            }
            let finish=self.source(d,"inventory_finish",body.clone()).await?;
            let latest=self.config.read();
            let complete=finish["complete"]==true && latest.revision()==cfg.revision() && !self.cancel.load(Ordering::SeqCst);
            report["complete"]=json!(complete); report["tree_stable"]=finish["tree_stable"].clone();
            Ok::<(),anyhow::Error>(())
        }.await;
        // Close even on cancellation, provider failures, or a changed config.
        let _ = self.source(d, "inventory_close", body).await;
        if let Err(e) = result {
            report["errors"]
                .as_array_mut()
                .unwrap()
                .push(json!({"reason":e.to_string()}));
        }
        Ok(report)
    }
}
pub async fn scheduler(engine: Arc<Engine>) {
    let mut clock = tokio::time::interval(Duration::from_secs(30));
    clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        clock.tick().await;
        if engine.syncing.load(Ordering::SeqCst) {
            continue;
        }
        if let Ok(Some((device, root))) = engine.due_folder(crate::now()) {
            let _ = engine.start_folder(&json!({"device":device,"root":root}));
        } else if engine.config.read().configured
            && engine.config.read().role != "client"
            && !engine
                .store
                .db
                .lock()
                .query_row("SELECT paused FROM background_state WHERE id=1", [], |r| {
                    r.get::<_, bool>(0)
                })
                .unwrap_or(true)
            && (engine.knowledge_pending().unwrap_or(0) > 0
                || engine.embedding_pending().unwrap_or(false))
        {
            // Drain a bounded annotation batch without walking the folders again.
            let _ = engine.start_sync(true);
        }
    }
}
