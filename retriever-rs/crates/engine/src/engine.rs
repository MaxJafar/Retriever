use crate::{
    config::{Config, Device},
    providers::{Providers, fingerprint, probe_key},
    scheduler::Priority,
    store::Store,
    vectors::Snapshot,
};
use anyhow::{Result, bail};
use parking_lot::{Mutex, RwLock};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use tokio::sync::broadcast;

pub struct Engine {
    pub started: Instant,
    pub state: PathBuf,
    pub config: RwLock<Config>,
    pub store: Store,
    pub disk: Arc<crate::disk::DiskSpace>,
    pub health: Arc<crate::health::DriveHealth>,
    pub burner: Arc<crate::burner::DiskBurner>,
    pub providers: Providers,
    pub token: String,
    pub syncing: AtomicBool,
    pub cancel: AtomicBool,
    pub progress: RwLock<Value>,
    pub events: broadcast::Sender<Value>,
    snapshots: Mutex<HashMap<String, Arc<Snapshot>>>,
    search_cache: Mutex<VecDeque<(String, Value, usize)>>,
    pub mutation: tokio::sync::Mutex<()>,
    pub nodes: crate::nodes::NodePool,
    pub transfers: Mutex<HashMap<String, Arc<crate::workspace::TransferJob>>>,
    pub runtime_gate: tokio::sync::RwLock<()>,
    pub shutdown: tokio::sync::watch::Sender<bool>,
    pub update_token: String,
}
impl Engine {
    pub fn open(state: &Path) -> Result<Arc<Self>> {
        if state.join("index.sqlite").exists() {
            bail!(
                "Legacy state cannot be opened by the candidate. Run migrate into a separate empty directory."
            );
        }
        fs::create_dir_all(state)?;
        let state = state.canonicalize()?;
        let mut cfg = Config::load(&state)?;
        for d in &mut cfg.devices {
            if d.transport == "local" {
                for r in &mut d.roots {
                    *r = crate::paths::expand(r).to_string_lossy().into_owned();
                }
            }
        }
        if !state.join("config.json").exists() {
            cfg.save(&state)?;
        }
        let token_file = state.join("api-token");
        if !token_file.exists() {
            use std::io::Write;
            let mut f = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&token_file)?;
            write!(
                f,
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            )?;
            f.sync_all()?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&state, fs::Permissions::from_mode(0o700))?;
            fs::set_permissions(&token_file, fs::Permissions::from_mode(0o600))?;
        }
        let token = fs::read_to_string(&token_file)?.trim().to_owned();
        if token.len() < 32 {
            bail!("Invalid pairing token");
        }
        let update_token = crate::updates::local_update_token(&state)?;
        let store = Store::open(&state)?;
        store.purge(&cfg)?;
        store.db.lock().execute(
            "UPDATE jobs SET state='interrupted' WHERE state IN ('queued','running')",
            [],
        )?;
        store.db.lock().execute(
            "UPDATE crawl_folders SET state='interrupted',next_run=0 WHERE state='running'",
            [],
        )?;
        store.db.lock().execute(
            "UPDATE workspace_plans SET state='inspection_required' WHERE state='running'",
            [],
        )?;
        if let Err(error) = crate::native_models::NativeModels::seed_packaged(&state) {
            // Keep filename and text search usable if installation runs out of
            // space or a packaged asset fails its checksum.
            eprintln!("Local model installation needs attention: {error}");
        }
        let providers = Providers::new(&state)?;
        for role in ["embedding", "relevance"] {
            cfg.search[role]["model_revision"] = json!(providers.native.revision());
        }
        fs::create_dir_all(state.join("cache"))?;
        let (events, _) = broadcast::channel(256);
        let disk = crate::disk::DiskSpace::open(&state)?;
        let health = crate::health::DriveHealth::open(&state)?;
        let burner = crate::burner::DiskBurner::open(&state)?;
        let engine = Arc::new(Self {
            started: Instant::now(),
            state,
            config: RwLock::new(cfg),
            store,
            disk,
            health,
            burner,
            providers,
            token,
            syncing: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            progress: RwLock::new(json!({})),
            events,
            snapshots: Mutex::new(HashMap::new()),
            search_cache: Mutex::new(VecDeque::new()),
            transfers: Mutex::new(HashMap::new()),
            mutation: tokio::sync::Mutex::new(()),
            runtime_gate: tokio::sync::RwLock::new(()),
            shutdown: tokio::sync::watch::channel(false).0,
            update_token,
            nodes: crate::nodes::NodePool::default(),
        });
        engine.reconcile_knowledge()?;
        Ok(engine)
    }
    pub fn status(&self) -> Result<Value> {
        let cfg = self.config.read().clone();
        let devices = self.store.device_status(&cfg)?;
        let files = devices
            .iter()
            .map(|d| d["files"].as_u64().unwrap_or(0))
            .sum::<u64>();
        let embedding = self.model_status("embedding", &cfg)?;
        let mut result = json!({"product":"Retriever","version":env!("CARGO_PKG_VERSION"),"runtime":"rust","desktop_api_version":1,"update_protocol":1,"candidate":true,"cutover_allowed":false,
            "configured":cfg.configured,"role":cfg.role,"hostname":std::env::var("COMPUTERNAME").or_else(|_|std::env::var("HOSTNAME")).unwrap_or_default(),"devices":devices,"files":files,"chunks":files,
            "syncing":self.syncing.load(Ordering::SeqCst),"can_index":cfg.configured&&cfg.role!="client","semantic_available":embedding["available"],"embedding":embedding,
            "last_sync":self.progress.read().clone(),"routing":self.providers.scheduler.status(),"search_routing":{"version":1,"queue_wait_ms":120},"analysis":{"categories":[],"tags":[]},
            "local_device":cfg.devices.iter().find(|d|d.transport=="local").map(|d|d.id.clone()).unwrap_or_default(),"laya":{"available":false,"detail":"Built-in local Laya"}});
        for role in ["relevance", "vision", "llm"] {
            result[role] = self.model_status(role, &cfg)?;
        }
        result["laya"] = result["relevance"].clone();
        result["storage"] = json!({"root":self.state,"models":self.state.join("models"),"index":self.state.join("retriever.sqlite"),"cache":self.state.join("cache"),"archive":self.state.join("rag-archive")});
        Ok(result)
    }
    pub fn model_status(&self, role: &str, cfg: &Config) -> Result<Value> {
        let s = &cfg.search[role];
        if !["embedding", "relevance"].contains(&role) || s["provider"] == "disabled" {
            return Ok(
                json!({"enabled":false,"available":false,"verified":false,"provider":"disabled","state":"removed"}),
            );
        }
        let native = self.providers.native.status(role);
        let installed = native["installed"] == true;
        let indexed = role != "embedding"
            || self.store.db.lock().query_row(
                "SELECT EXISTS(SELECT 1 FROM snapshots WHERE fingerprint=? AND active=1)",
                [fingerprint(s)],
                |r| r.get::<_, bool>(0),
            )?;
        let health = self.providers.health.lock().get(role).cloned();
        let verified = health
            .as_ref()
            .is_some_and(|h| h["state"] == "ready" && h["fingerprint"] == fingerprint(s));
        let failed = health
            .as_ref()
            .is_some_and(|h| h["state"] == "unavailable" && h["fingerprint"] == fingerprint(s));
        let available = installed && indexed && !failed;
        Ok(
            json!({"available":available,"enabled":true,"provider":"native","model":s["model"],
            "label":if role=="embedding"{"EmbeddingGemma"}else{"Laya"},"state":if failed{"unavailable"}else if available{"available"}else if installed{"ready_to_index"}else{"models_missing"},
            "verified":verified,"capabilities":{"text":true},"native":native,"health":health,
            "detail":if available{"Local retrieval ready"}else if installed{"Index selected folders to enable meaning search"}else{"Local model bundle needs to be installed in the app data models folder"}}),
        )
    }
    pub fn configure(&self, value: Value, expected: Option<&str>) -> Result<Value> {
        let _mutation = self.mutation.try_lock().map_err(|_| {
            anyhow::anyhow!("conflict: Wait for the active file operation before changing folders")
        })?;
        let credentials = value.get("api_keys").cloned();
        let mut cfg: Config = serde_json::from_value(value)?;
        for role in ["embedding", "relevance"] {
            cfg.search[role]["model_revision"] = json!(self.providers.native.revision());
        }
        cfg.validate()?;
        let mut current = self.config.write();
        if expected.is_some_and(|e| e != current.revision()) {
            bail!("conflict: Configuration changed. Reload settings before saving.");
        }
        if let Some(changes) = credentials {
            self.providers
                .save_credentials(&self.state, &cfg.search, &changes)?;
        }
        // A newly included scope must never inherit an old due schedule.
        // Explicit settings for roots that remain included are preserved.
        {
            let db = self.store.db.lock();
            for device in &cfg.devices {
                for root in &device.roots {
                    if !current
                        .devices
                        .iter()
                        .any(|old| old.id == device.id && old.roots.contains(root))
                    {
                        db.execute("INSERT INTO crawl_folders(device,root,analysis,interval_minutes) VALUES(?,?,?,0) ON CONFLICT(device,root) DO UPDATE SET interval_minutes=0,next_run=0,analysis=excluded.analysis,revision=revision+1",rusqlite::params![device.id,root,if device.analysis_roots.contains(root){"text_vision"}else{"off"}])?;
                    }
                }
            }
        }
        cfg.save(&self.state)?;
        self.store.purge(&cfg)?;
        *current = cfg.clone();
        drop(current);
        self.reconcile_knowledge()?;
        self.snapshots.lock().clear();
        self.archive_catalogue()?;
        Ok(
            json!({"config":cfg,"revision":cfg.revision(),"state":"Settings saved. Start indexing when ready."}),
        )
    }
    pub fn plan_configuration(&self, value: Option<Value>) -> Result<Value> {
        let cfg = match value {
            Some(v) => serde_json::from_value::<Config>(v)?,
            None => self.config.read().clone(),
        };
        cfg.validate()?;
        let mut capabilities = json!({"semantic":false});
        let mut disclosures = Vec::new();
        for role in ["embedding", "relevance", "vision", "llm"] {
            capabilities[role] = self.model_status(role, &cfg)?;
            let s = &cfg.search[role];
            if s["provider"] != "disabled" && s["provider"] != "native" {
                disclosures.push(json!({"purpose":role,"endpoint":s["base_url"],"model":s["model"],"hosting":s["hosting"],"sent":if role=="embedding"{"screened passages and search queries"}else if role=="vision"{"pixels from explicitly opted-in folders"}else{"bounded prompts and evidence"}}));
            }
        }
        capabilities["semantic"] = capabilities["embedding"]["available"].clone();
        let devices = cfg
            .devices
            .iter()
            .map(|d| {
                let mut v = serde_json::to_value(d).unwrap();
                v["installation"] = json!(if d.transport == "local" {
                    "Bundled Rust engine"
                } else {
                    "Matching Rust agent required over pinned SSH"
                });
                v
            })
            .collect::<Vec<_>>();
        Ok(
            json!({"role":cfg.role,"storage":self.state.to_string_lossy(),"devices":devices,"capabilities":capabilities,"api_data":disclosures,"steps":["Save selected folders","Load the bundled EmbeddingGemma and Laya models","Index locally"],"warnings":["Candidate runtime: cutover stays disabled until all parity and installation gates pass"]}),
        )
    }
    pub async fn source(&self, d: &Device, action: &str, body: Value) -> Result<Value> {
        let mut validation = self.config.read().clone();
        validation.devices = vec![d.clone()];
        validation.validate()?;
        let mut body = body;
        body["_framed_response"] = json!(true);
        body["device_config"] = serde_json::to_value(d)?;
        if d.transport == "local" {
            let dbody = body;
            let action = action.to_owned();
            let forbidden = vec![self.state.clone()];
            return tokio::task::spawn_blocking(move || {
                crate::agent::dispatch(&action, &dbody, &forbidden)
            })
            .await?;
        }
        let cfg = self.config.read().clone();
        self.nodes.call(&cfg, d, action, body).await
    }

    pub fn start_sync(self: &Arc<Self>, index_only: bool) -> Result<Value> {
        self.start_job(index_only, None)
    }
    pub fn cancel_jobs(&self) -> Result<Value> {
        self.cancel.store(true, Ordering::SeqCst);
        self.store
            .db
            .lock()
            .execute("UPDATE background_state SET paused=1 WHERE id=1", [])?;
        Ok(
            json!({"state":"Cancellation requested. Pending learning waits for the next crawl or manual index run."}),
        )
    }
    pub(crate) fn start_job(
        self: &Arc<Self>,
        index_only: bool,
        scope: Option<(String, String)>,
    ) -> Result<Value> {
        let _runtime = self.runtime_gate.try_read().map_err(|_| anyhow::anyhow!("conflict: Update preparation is in progress"))?;
        if *self.shutdown.borrow() {bail!("conflict: Retriever is stopping for an update");}
        let _mutation = self.mutation.try_lock().map_err(|_| {
            anyhow::anyhow!("conflict: Wait for the active file operation before indexing")
        })?;
        if self.config.read().role == "client" {
            bail!("This client uses its coordinator for indexing");
        }
        if self.syncing.swap(true, Ordering::SeqCst) {
            return Ok(json!({"state":"Indexing is already running"}));
        }
        self.cancel.store(false, Ordering::SeqCst);
        let id = uuid::Uuid::new_v4().to_string();
        let engine = self.clone();
        let job = id.clone();
        if let Err(error) = self.store.db.lock().execute(
            "INSERT INTO jobs VALUES(?, 'index', 'running','{}',?,?)",
            params![id, crate::now(), crate::now()],
        ) {
            self.syncing.store(false, Ordering::SeqCst);
            return Err(error.into());
        }
        if let Err(error) = self
            .store
            .db
            .lock()
            .execute("UPDATE background_state SET paused=0 WHERE id=1", [])
        {
            self.syncing.store(false, Ordering::SeqCst);
            return Err(error.into());
        }
        self.progress(&id, json!({"id":id,"phase":"starting"}));
        tokio::spawn(async move {
            let result = engine.sync_run(&job, index_only, scope).await;
            let state = if engine.cancel.load(Ordering::SeqCst) {
                "cancelled"
            } else if result.is_err() {
                "failed"
            } else if !engine.progress.read()["embedding_warning"].is_null()
                || engine.progress.read()["collection_partial"] == true
            {
                "partial"
            } else {
                "complete"
            };
            let mut final_progress = engine.progress.read().clone();
            final_progress["id"] = json!(job);
            final_progress["phase"] = json!(state);
            final_progress["error"] = json!(result.err().map(|e| e.to_string()));
            final_progress["finished_at"] = json!(crate::now());
            engine.progress(&job, final_progress);
            let _ = engine.store.db.lock().execute(
                "UPDATE jobs SET state=?,progress=?,updated=? WHERE id=?",
                params![state, engine.progress.read().to_string(), crate::now(), job],
            );
            engine.syncing.store(false, Ordering::SeqCst);
        });
        Ok(json!({"id":id,"state":"Indexing started"}))
    }
    pub(crate) fn progress(&self, id: &str, value: Value) {
        *self.progress.write() = value.clone();
        let _ = self.events.send(json!({"job":id,"progress":value}));
    }
    async fn sync_run(
        self: &Arc<Self>,
        id: &str,
        index_only: bool,
        scope: Option<(String, String)>,
    ) -> Result<()> {
        let cfg = self.config.read().clone();
        if !cfg.configured {
            bail!("Save included folders first");
        }
        if !index_only {
            self.crawl_roots(id, &cfg, scope.as_ref()).await?;
        }
        if self.cancel.load(Ordering::SeqCst) {
            return Ok(());
        }
        let knowledge = self.enrich_pending(id).await?;
        let embedding = self.embed_pending(id, &cfg).await;
        if let Err(error) = &embedding {
            if error.to_string().to_lowercase().contains("source changed")
                || error.to_string().contains("source_changed")
            {
                return Err(anyhow::anyhow!(error.to_string()));
            }
        }
        let archive = self.archive_catalogue();
        let mut progress = self.progress.read().clone();
        progress["phase"] = json!("indexed");
        progress["knowledge"] = knowledge;
        progress["archive_warning"] = json!(archive.err().map(|e| e.to_string()));
        progress["embedding_warning"] = json!(embedding.err().map(|e| e.to_string()));
        progress["word_search_available"] = json!(true);
        progress["collection_partial"] = json!(
            self.store
                .device_status(&cfg)?
                .iter()
                .any(|d| d["status"] == "partial")
        );
        self.progress(id, progress);
        Ok(())
    }
    pub(crate) fn snapshot_dirty(&self, fp: &str) -> Result<bool> {
        Ok(self.store.db.lock().query_row("SELECT COALESCE((SELECT revision FROM snapshot_revisions WHERE fingerprint=?),-1)<>(SELECT value FROM revision WHERE id=1)",[fp],|r|r.get(0))?)
    }
    pub(crate) fn invalidate_snapshot(&self, fp: &str) {
        self.snapshots.lock().remove(fp);
        self.search_cache.lock().clear();
    }
    pub fn embedding_pending(&self) -> Result<bool> {
        let cfg = self.config.read().clone();
        let settings = &cfg.search["embedding"];
        if settings["provider"] == "disabled"
            || self.providers.native.status("embedding")["installed"] != true
            || !self.providers.retry_ready("embedding", settings)
        {
            return Ok(false);
        }
        let fp = fingerprint(settings);
        if self.snapshot_dirty(&fp)? {
            return Ok(true);
        }
        Ok(self.store.db.lock().query_row("SELECT EXISTS(SELECT 1 FROM passages p JOIN files f ON f.id=p.file_id AND f.digest=p.digest LEFT JOIN vectors v ON v.passage_id=p.id AND v.fingerprint=?1 LEFT JOIN embedding_attempts a ON a.file_id=f.id AND a.fingerprint=?1 AND a.digest=f.digest WHERE v.passage_id IS NULL AND COALESCE(a.retry_after,0)<=?2)",params![fp,crate::now()],|r|r.get(0))?)
    }
    pub async fn probe(&self, role: &str, settings: Value) -> Result<Value> {
        if !["embedding", "relevance"].contains(&role) {
            bail!("Invalid model role");
        }
        let key = probe_key(role, &settings);
        self.providers.health.lock().remove(role);
        // A failed recheck must not leave an earlier capability marked verified.
        self.store
            .db
            .lock()
            .execute("DELETE FROM provider_probes WHERE fingerprint=?", [&key])?;
        let result = self.probe_model(role, &settings).await;
        if result.is_err() {
            self.providers.invalid_response(role, &settings);
        }
        let result = result?;
        self.store.db.lock().execute(
            "INSERT OR REPLACE INTO provider_probes VALUES(?,?,?,?)",
            params![key, role, result.to_string(), crate::now()],
        )?;
        Ok(result)
    }
    async fn probe_model(&self, role: &str, settings: &Value) -> Result<Value> {
        crate::config::validate_model(role, settings)?;
        match role {
            "embedding" => {
                let text = format!(
                    "{}Retriever capability probe",
                    settings["document_prefix"].as_str().unwrap_or("")
                );
                let vectors = self
                    .providers
                    .embed(settings, vec![json!(text)], Priority::Interactive, 768)
                    .await?;
                Ok(
                    json!({"ok":true,"dimensions":vectors[0].len(),"modalities":["text"],"runtime":"in_process"}),
                )
            }
            "relevance" => {
                self.providers
                    .relevance(
                        settings,
                        "golden dog",
                        &[json!({"id":1,"snippet":"A golden dog retrieves a ball."})],
                    )
                    .await?;
                Ok(json!({"ok":true,"runtime":"in_process"}))
            }
            _ => bail!("Only built-in retrieval models are supported"),
        }
    }
    pub async fn search(
        &self,
        q: &str,
        device: &str,
        kind: &str,
        mode: &str,
        phase: &str,
    ) -> Result<Value> {
        if !["hybrid", "exact", "semantic"].contains(&mode)
            || !["instant", "full"].contains(&phase)
            || !["", "documents", "code", "image", "audio", "video"].contains(&kind)
        {
            bail!("Invalid search filter");
        }
        let started = Instant::now();
        let q: String = q.trim().chars().take(800).collect();
        let cfg = self.config.read().clone();
        let limit = cfg.search["limit"].as_u64().unwrap_or(30).clamp(1, 100) as usize;
        let epoch = self.store.cache_epoch()?;
        let config_bytes = serde_json::to_vec(&cfg)?;
        let cache_key = crate::digest(&serde_json::to_vec(&(
            &q,
            device,
            kind,
            mode,
            phase,
            epoch,
            &config_bytes,
        ))?);
        let cached = {
            let mut cache=self.search_cache.lock();
            cache.iter().position(|(key,_,_)|*key==cache_key).map(|at|{
                let entry=cache.remove(at).unwrap();let value=entry.1.clone();cache.push_back(entry);value
            })
        };
        if let Some(mut response) = cached {
            if epoch == self.store.cache_epoch()?
                && config_bytes == serde_json::to_vec(&*self.config.read())?
            {
                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                response["elapsed_ms"] = json!(elapsed);
                response["engine_ms"] = json!(elapsed);
                response["inference_ms"] = json!(0);
                response["routing"]["cache_hit"] = json!(true);
                response["routing"]["response_cached"] = json!(true);
                response["routing"]["embedding_cached"] = response["semantic_available"].clone();
                response["routing"]["relevance_cached"] = json!(matches!(
                    response["relevance"]["state"].as_str(),
                    Some("applied" | "abstained")
                ));
                response["retrieval"]["semantic_ran"] = json!(false);
                return Ok(response);
            }
        }
        let (mut rows, exact_file) = if mode == "semantic" {
            (Vec::new(), false)
        } else {
            let exact = self.store.exact_name(&q, limit, &cfg, device, kind)?;
            if exact.is_empty() {
                (
                    self.store.lexical(&q, limit.max(60), &cfg, device, kind)?,
                    false,
                )
            } else {
                (exact, true)
            }
        };
        let terms = q.split_whitespace().count();
        let fast = phase == "instant"
            || mode == "exact"
            || q.is_empty()
            || (mode != "semantic" && ((terms == 1 && q.chars().count() <= 3) || exact_file));
        let mut semantic = false;
        let mut semantic_partial = false;
        let lexical_ids = rows
            .iter()
            .filter_map(|r| r["id"].as_i64())
            .collect::<std::collections::HashSet<_>>();
        let lexical_scores: HashMap<i64, f64> = rows
            .iter()
            .filter_map(|r| Some((r["id"].as_i64()?, r["score"].as_f64()?)))
            .collect();
        let mut detail = String::new();
        let mut rank_state = "skipped";
        let mut inference_ms = 0.0;
        let mut embedding_cached = false;
        let mut relevance_cached = false;
        if !fast && cfg.search["embedding"]["provider"] != "disabled" {
            let s = &cfg.search["embedding"];
            let fp = fingerprint(s);
            let loaded = self.snapshots.lock().get(&fp).cloned();
            let snap = if let Some(s) = loaded {
                Some(s)
            } else {
                let s = Snapshot::load(&self.store, &fp)?;
                if let Some(s) = &s {
                    self.snapshots.lock().insert(fp.clone(), s.clone());
                }
                s
            };
            if let Some(snap) = snap {
                let passages: usize =
                    self.store
                        .db
                        .lock()
                        .query_row("SELECT count(*) FROM passages", [], |r| r.get(0))?;
                semantic_partial = snap.rows.len() < passages || self.snapshot_dirty(&fp)?;
                let start = Instant::now();
                let result = self.providers.query(s, &q, snap.dimensions).await;
                inference_ms += start.elapsed().as_secs_f64() * 1000.0;
                match result {
                    Ok((query, cached)) => {
                        embedding_cached = cached;
                        semantic = true;
                        let mut seen = std::collections::HashSet::new();
                        let allowed = if device.is_empty() && kind.is_empty() {
                            None
                        } else {
                            let db = self.store.db.lock();
                            let mut statement = db.prepare("SELECT id FROM files WHERE (?1='' OR device=?1) AND (?2='' OR type=?2)")?;
                            Some(
                                statement
                                    .query_map(params![device, kind], |r| r.get::<_, i64>(0))?
                                    .collect::<rusqlite::Result<std::collections::HashSet<_>>>()?,
                            )
                        };
                        // A large catalogue scan belongs on a blocking worker, keeping
                        // HTTP/navigation responsive while uncached searches run.
                        let scoring = snap.clone();
                        let ranked = tokio::task::spawn_blocking(move || {
                            scoring.score_files(&query, limit * 2, allowed.as_ref())
                        })
                        .await??;
                        for (rank, (identity, score)) in ranked.into_iter().enumerate() {
                            if !seen.insert(identity.file) {
                                continue;
                            }
                            if let Some(mut r) = self.store.summary(identity.file, &cfg)? {
                                if r["digest"] != identity.digest
                                    || (!device.is_empty() && r["device"] != device)
                                    || (!kind.is_empty() && r["type"] != kind)
                                {
                                    continue;
                                }
                                let passage:Option<(String,String)>=self.store.db.lock().query_row("SELECT text,coordinates FROM passages WHERE id=? AND file_id=? AND digest=? AND extraction_revision=?",params![identity.passage,identity.file,identity.digest,identity.extraction],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
                                let Some((text, coordinates)) = passage else {
                                    continue;
                                };
                                let fusion = 1.0 / (60.0 + rank as f64);
                                if let Some(existing) =
                                    rows.iter_mut().find(|r| r["id"] == identity.file)
                                {
                                    existing["semantic_score"] = json!(score);
                                    // A rank among the first few indexed files is
                                    // not comparable with a catalogue-wide rank.
                                    // Preserve lexical score and its supporting
                                    // passage until semantic coverage is complete.
                                    if mode == "hybrid" && semantic_partial {
                                        continue;
                                    }
                                    existing["score"] =
                                        json!(existing["score"].as_f64().unwrap_or(0.0) + fusion);
                                    existing["snippet"] =
                                        json!(text.chars().take(900).collect::<String>());
                                    existing["coordinates"] = serde_json::from_str(&coordinates)?;
                                    existing["generated"] =
                                        json!(existing["coordinates"]["origin"] == "generated");
                                } else {
                                    r["snippet"] =
                                        json!(text.chars().take(900).collect::<String>());
                                    r["coordinates"] = serde_json::from_str(&coordinates)?;
                                    r["generated"] =
                                        json!(r["coordinates"]["origin"] == "generated");
                                    r["score"] = json!(fusion);
                                    r["semantic_score"] = json!(score);
                                    r["retrieval"] = json!("meaning");
                                    r.as_object_mut().unwrap().remove("text");
                                    rows.push(r);
                                }
                            }
                        }
                    }
                    Err(e) => detail = e.to_string(),
                }
            } else {
                detail =
                    "Index selected folders with the built-in EmbeddingGemma model first".into();
            }
        }
        if mode == "semantic" && cfg.search["embedding"]["provider"] == "disabled" {
            detail = "Enable the built-in EmbeddingGemma model to search by meaning".into();
        }
        if semantic && semantic_partial && detail.is_empty() {
            detail = if mode == "hybrid" {
                "Meaning index is still building; word matches are shown first."
            } else {
                "Meaning index is still building; results cover indexed passages only."
            }
            .into();
        }
        if mode == "hybrid" && semantic && !semantic_partial {
            for row in &mut rows {
                // Rank agreement alone flattens meaningful similarity margins:
                // an incidental word hit must not erase a much stronger meaning
                // match. This bounded boost still preserves lexical-only hits.
                let similarity = row["semantic_score"]
                    .as_f64()
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0);
                row["score"] = json!(row["score"].as_f64().unwrap_or(0.0) * (1.0 + similarity));
            }
        }
        let rank_rows = |a: &Value, b: &Value| {
            let coverage_order = if mode == "hybrid" && semantic_partial {
                let word_hit =
                    |r: &Value| r["id"].as_i64().is_some_and(|id| lexical_ids.contains(&id));
                word_hit(b).cmp(&word_hit(a))
            } else {
                std::cmp::Ordering::Equal
            };
            coverage_order.then_with(|| {
                b["score"]
                    .as_f64()
                    .unwrap_or(0.0)
                    .total_cmp(&a["score"].as_f64().unwrap_or(0.0))
            })
        };
        rows.sort_by(&rank_rows);
        if !fast && cfg.search["relevance"]["provider"] != "disabled" && rows.len() > 1 {
            let start = Instant::now();
            match self
                .providers
                .relevance_with_cache(&cfg.search["relevance"], &q, &rows)
                .await
            {
                Ok((scores, cached)) => {
                    relevance_cached = cached;
                    // An uncertain shortlist cannot justify promoting another
                    // hit past the uncertain evidence. Keep retrieval ordering
                    // unless the entire scored shortlist can be compared.
                    let comparable = relevance_comparable(&scores, &cfg.search["relevance"]);
                    rank_state = if comparable { "applied" } else { "abstained" };
                    for r in &mut rows {
                        if let Some(s) = scores.iter().find(|s| s["id"] == r["id"]) {
                            r["relevance"] = s.clone();
                            if comparable {
                                r["score"] = json!(
                                    r["score"].as_f64().unwrap_or(0.0)
                                        * (1.0
                                            + cfg.search["relevance"]["weight"]
                                                .as_f64()
                                                .unwrap_or(0.25)
                                                .clamp(0.0, 1.0)
                                                * (s["relevance"].as_f64().unwrap_or(0.5) - 0.5))
                                );
                            }
                        }
                    }
                }
                Err(_) => rank_state = "unavailable",
            }
            inference_ms += start.elapsed().as_secs_f64() * 1000.0;
        }
        // Do not deliver excluded or superseded content after inference completed.
        let latest = self.config.read().clone();
        rows.retain(|r| {
            if r["generated"] == true
                && !self.annotation_current(r["id"].as_i64().unwrap_or(0), &r["coordinates"])
            {
                return false;
            }
            r["id"]
                .as_i64()
                .and_then(|id| self.store.summary(id, &latest).ok().flatten())
                .is_some_and(|v| v["digest"] == r["digest"])
        });
        let agreement_limited =
            rank_state == "applied" && bound_relevance_by_agreement(&mut rows, &lexical_scores);
        rows.sort_by(rank_rows);
        rows.truncate(limit);
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        let response = json!({"query":q,"results":rows,"mode":mode,"elapsed_ms":elapsed,"engine_ms":(elapsed-inference_ms).max(0.0),"inference_ms":inference_ms,
            "routing":{"route":if fast{"words"}else{"hybrid"},"complete":phase=="full"||mode=="exact"||(mode!="semantic"&&(exact_file||(terms==1&&q.chars().count()<=3))),"cache_hit":embedding_cached&&(rank_state=="skipped"||relevance_cached),"embedding_cached":embedding_cached,"relevance_cached":relevance_cached,"phase":phase},"retrieval":{"semantic_ran":semantic,"semantic_available":semantic,"partial_index":semantic&&semantic_partial,"detail":detail},"relevance":{"state":rank_state,"applied":rank_state=="applied","agreement_limited":agreement_limited},"detail":detail,"semantic_available":semantic});
        let response_bytes=serde_json::to_vec(&response)?.len();
        if detail.is_empty()
            && rank_state != "unavailable"
            && epoch == self.store.cache_epoch()?
            && config_bytes == serde_json::to_vec(&*self.config.read())?
            && response_bytes <= 256 * 1024
        {
            let mut cache = self.search_cache.lock();
            while cache.len() >= 128 || cache.iter().map(|entry|entry.2).sum::<usize>()+response_bytes>8*1024*1024 {
                cache.pop_front();
            }
            cache.push_back((cache_key, response.clone(), response_bytes));
        }
        Ok(response)
    }
    pub async fn assist(&self, _body: &Value) -> Result<Value> {
        bail!("Generative AI has been removed. Search returns indexed passages with source links.");
    }
}

fn relevance_comparable(scores: &[Value], settings: &Value) -> bool {
    scores.len() > 1
        && scores.iter().all(|s| {
            s["truncated"] == false
                && s["confidence"].as_f64().unwrap_or(0.0)
                    >= settings["min_confidence"].as_f64().unwrap_or(0.7)
        })
}

/// Rows remain in their original retrieval order while Laya adjusts their
/// scores. If both lexical evidence and semantic similarity favor an earlier
/// hit, cap a later hit below it. A short classifier decision cannot overturn
/// that agreement. Disagreement, tied evidence, or a missing modality leaves
/// Laya free to refine the order. Apply this only after removing stale sources.
fn bound_relevance_by_agreement(rows: &mut [Value], lexical: &HashMap<i64, f64>) -> bool {
    let evidence = rows
        .iter()
        .map(|r| {
            let words = *lexical.get(&r["id"].as_i64()?)?;
            let meaning = r["semantic_score"].as_f64()?;
            (words.is_finite() && meaning.is_finite()).then_some((words, meaning))
        })
        .collect::<Vec<_>>();
    let mut limited = false;
    for weaker in 0..rows.len() {
        let Some((words, meaning)) = evidence[weaker] else {
            continue;
        };
        let Some(proposed) = rows[weaker]["score"].as_f64().filter(|s| s.is_finite()) else {
            continue;
        };
        let mut bounded = proposed;
        for stronger in 0..weaker {
            let Some((other_words, other_meaning)) = evidence[stronger] else {
                continue;
            };
            if other_words >= words
                && other_meaning >= meaning
                && (other_words > words || other_meaning > meaning)
            {
                if let Some(ceiling) = rows[stronger]["score"].as_f64().filter(|s| s.is_finite()) {
                    // Strict ordering also survives clients that sort by score.
                    bounded = bounded.min(ceiling.next_down());
                }
            }
        }
        if bounded < proposed {
            rows[weaker]["score"] = json!(bounded);
            rows[weaker]["rank_limited_by_agreement"] = json!(true);
            limited = true;
        }
    }
    limited
}

#[cfg(test)]
mod ranking_tests {
    use super::*;
    #[test]
    fn confident_classifier_cannot_reverse_joint_retrieval_evidence() {
        // Numeric reproduction of the real-corpus failure; no query, filename
        // or document-specific rule is involved.
        let mut rows = vec![
            json!({"id":1,"score":0.05570342230868535,"semantic_score":0.8133257031440735}),
            json!({"id":2,"score":0.056088219142848075,"semantic_score":0.628582239151001}),
        ];
        let lexical = HashMap::from([(1, 1.0 / 60.0), (2, 1.0 / 61.0)]);
        assert!(bound_relevance_by_agreement(&mut rows, &lexical));
        assert!(rows[0]["score"].as_f64().unwrap() > rows[1]["score"].as_f64().unwrap());
        assert_eq!(rows[1]["rank_limited_by_agreement"], true);
    }
    #[test]
    fn relevance_can_resolve_disagreement_ties_and_single_modality_results() {
        for (lexical, meaning) in [
            (HashMap::from([(1, 0.5), (2, 0.4)]), [0.6, 0.8]),
            (HashMap::from([(1, 0.5), (2, 0.5)]), [0.8, 0.8]),
            (HashMap::from([(1, 0.5)]), [0.8, 0.6]),
            (HashMap::new(), [0.8, 0.6]),
        ] {
            let mut rows = vec![
                json!({"id":1,"score":0.4,"semantic_score":meaning[0]}),
                json!({"id":2,"score":0.5,"semantic_score":meaning[1]}),
            ];
            let before = rows.clone();
            assert!(!bound_relevance_by_agreement(&mut rows, &lexical));
            assert_eq!(rows, before);
        }
    }
    #[test]
    fn agreement_caps_propagate_without_penalizing_already_lower_scores() {
        let mut rows = vec![
            json!({"id":1,"score":0.8,"semantic_score":0.9}),
            json!({"id":2,"score":0.9,"semantic_score":0.8}),
            json!({"id":3,"score":0.9,"semantic_score":0.7}),
            json!({"id":4,"score":0.6,"semantic_score":0.6}),
        ];
        let lexical = HashMap::from([(1, 0.9), (2, 0.8), (3, 0.7), (4, 0.6)]);
        assert!(bound_relevance_by_agreement(&mut rows, &lexical));
        for pair in rows.windows(2) {
            assert!(pair[0]["score"].as_f64().unwrap() > pair[1]["score"].as_f64().unwrap());
        }
        assert_eq!(rows[3]["score"], 0.6);
        assert!(!bound_relevance_by_agreement(&mut rows, &lexical));
    }
    #[test]
    fn uncertain_or_truncated_shortlist_must_not_promote_other_hits() {
        let settings = json!({"min_confidence":0.7});
        let positive = json!({"id":1,"relevance":0.95,"confidence":0.9,"truncated":false});
        let uncertain = json!({"id":2,"relevance":0.5,"confidence":0.6,"truncated":false});
        assert!(!relevance_comparable(
            &[positive.clone(), uncertain],
            &settings
        ));
        let truncated = json!({"id":2,"relevance":0.1,"confidence":0.95,"truncated":true});
        assert!(!relevance_comparable(
            &[positive.clone(), truncated],
            &settings
        ));
        let negative = json!({"id":2,"relevance":0.1,"confidence":0.95,"truncated":false});
        assert!(relevance_comparable(&[positive, negative], &settings));
    }
}
