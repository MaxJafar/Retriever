use crate::{engine::Engine, store::row_json};
use anyhow::{Context, Result, bail};
use base64::Engine as _;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
pub struct TransferJob {
    cancel: AtomicBool,
    progress: parking_lot::Mutex<Value>,
}
fn check_copy_cancel(job: &Option<Arc<TransferJob>>) -> Result<()> {
    if job
        .as_ref()
        .is_some_and(|j| j.cancel.load(Ordering::SeqCst))
    {
        bail!(
            "Copy stopped. Completed files and unpublished partial copies remain in their receipts; originals are unchanged"
        );
    }
    Ok(())
}
fn copy_progress(
    job: &Option<Arc<TransferJob>>,
    completed: u64,
    total: u64,
    files: usize,
    row: &Value,
    started: &std::time::Instant,
    phase: &str,
) {
    if let Some(job) = job {
        *job.progress.lock() = json!({"completed_bytes":completed,"total_bytes":total,"files_done":files,
            "source":row["source"],"phase":phase,"elapsed_seconds":started.elapsed().as_secs_f64(),"stopping":job.cancel.load(Ordering::SeqCst)});
    }
}

impl Engine {
    async fn copy_plan(&self, b: &Value) -> Result<Value> {
        let cfg = self.config.read().clone();
        let source = cfg.device(text(b, "device", 64)?)?;
        let target = cfg.device(text(b, "target_device", 64)?)?;
        let source_caps = self.source(source, "probe", json!({})).await?;
        let target_caps = if source.id == target.id {
            source_caps.clone()
        } else {
            self.source(target, "probe", json!({})).await?
        };
        let modern = source_caps["transfer_protocol"] == 2 && target_caps["transfer_protocol"] == 2;
        let parent = self
            .source(
                target,
                if b["scope"] == "filesystem" {
                    "explorer_stat"
                } else {
                    "stat"
                },
                json!({"path":text(b,"parent",4096)?}),
            )
            .await?;
        if parent["kind"] != "folder" {
            bail!("Choose a destination folder");
        }
        let items = b["items"].as_array().context("Missing copy items")?;
        if items.is_empty() || items.len() > 100 {
            bail!("Copy 1–100 regular files at a time");
        }
        let mut rows = Vec::new();
        let mut destinations = std::collections::HashSet::new();
        for item in items {
            let info = self
                .source(
                    source,
                    if b["scope"] == "filesystem" {
                        "explorer_stat"
                    } else {
                        "stat"
                    },
                    json!({"path":text(item,"path",4096)?}),
                )
                .await?;
            if info["kind"] != "file"
                || info["size"].as_u64().unwrap_or(u64::MAX)
                    > if modern {
                        crate::transfer::MAX_FILE_SIZE
                    } else {
                        2 * 1024 * 1024 * 1024
                    }
            {
                bail!(
                    "Choose regular files within the connected devices' copy limit (older agents: 2 GiB)"
                );
            }
            if item.get("pin").is_some_and(|pin| pin != &info["pin"]) {
                bail!("Source changed since selection");
            }
            let name = item["name"]
                .as_str()
                .unwrap_or(info["name"].as_str().unwrap_or(""));
            let prepared = self
                .source(
                    target,
                    "prepare",
                    json!({"operation":"mkdir","scope":b["scope"],"items":[{"parent":parent["path"],"name":name}]}),
                )
                .await?;
            let destination = prepared["items"][0]["destination"]
                .as_str()
                .context("Missing destination")?;
            if !destinations.insert(destination.to_lowercase()) {
                bail!("Copy destinations must be unique");
            }
            rows.push(json!({"source":info["path"],"destination":destination,"pin":info["pin"],"transfer_id":uuid::Uuid::new_v4().to_string()}));
        }
        Ok(
            json!({"operation":"copy","scope":b["scope"],"target_device":target.id,"items":rows,
            "transfer_protocol":if modern{2}else{1},"method":if modern&&source.id==target.id{"native"}else if modern{"adaptive"}else{"compatible"}}),
        )
    }
    async fn copy_files(&self, plan: &Value) -> Result<Value> {
        use sha2::{Digest, Sha256};
        let cfg = self.config.read().clone();
        let source = cfg.device(plan["device"].as_str().unwrap_or(""))?;
        let target = cfg.device(plan["target_device"].as_str().unwrap_or(""))?;
        let modern = plan["transfer_protocol"] == 2;
        let operation_id = plan["id"].as_str().context("Missing operation identity")?;
        let job = self.transfers.lock().get(operation_id).cloned();
        let total: u64 = plan["items"]
            .as_array()
            .context("Missing copy items")?
            .iter()
            .map(|r| r["pin"]["size"].as_u64().unwrap_or(0))
            .sum();
        let started = std::time::Instant::now();
        let mut completed = 0u64;
        let mut done = Vec::new();
        for row in plan["items"].as_array().context("Missing copy items")? {
            check_copy_cancel(&job)?;
            let id = row["transfer_id"]
                .as_str()
                .context("Missing transfer identity")?;
            let info = self
                .source(
                    source,
                    if plan["scope"] == "filesystem" {
                        "explorer_stat"
                    } else {
                        "stat"
                    },
                    json!({"path":row["source"]}),
                )
                .await?;
            if info["pin"] != row["pin"] {
                bail!("Source changed since review");
            }
            copy_progress(&job, completed, total, done.len(), row, &started, "copying");
            let receipt = if modern && source.id == target.id {
                self.source(target,"transfer_local_copy",json!({"scope":plan["scope"],"id":id,"source":row["source"],"path":row["destination"],"pin":row["pin"]})).await?
            } else {
                self.source(
                target,
                "transfer_begin",
                json!({"scope":plan["scope"],"id":id,"path":row["destination"],"size":row["pin"]["size"],"mtime_ns":if modern{row["pin"]["mtime_ns"].clone()}else{Value::Null}}),
            )
            .await?;
                let mut offset = 0;
                let mut sha = Sha256::new();
                let read_action = if modern {
                    "transfer_read_v2"
                } else {
                    "transfer_read"
                };
                let write_action = if modern {
                    "transfer_write_v2"
                } else {
                    "transfer_write"
                };
                let budget = if modern {
                    crate::transfer_codec::BLOCK
                } else {
                    512 * 1024
                };
                let read_body = |at| json!({"scope":plan["scope"],"path":row["source"],"pin":row["pin"],"offset":at,"length":budget});
                let mut chunk = self.source(source, read_action, read_body(0)).await?;
                loop {
                    check_copy_cancel(&job)?;
                    if chunk["offset"] != offset || chunk["pin"] != row["pin"] {
                        bail!("Invalid source read identity");
                    }
                    let bytes = if modern {
                        crate::transfer_codec::decode(&chunk)?
                    } else {
                        base64::engine::general_purpose::STANDARD
                            .decode(chunk["data"].as_str().context("Missing chunk")?)?
                    };
                    let next_offset = offset + bytes.len() as u64;
                    let size = row["pin"]["size"].as_u64().context("Missing source size")?;
                    if bytes.len() > budget
                        || (bytes.is_empty() && chunk["eof"] != true)
                        || next_offset > size
                        || chunk["eof"].as_bool() != Some(next_offset == size)
                    {
                        bail!("Invalid transfer chunk");
                    }
                    sha.update(&bytes);
                    let mut write = chunk.clone();
                    write["id"] = json!(id);
                    let eof = chunk["eof"] == true;
                    // At most one read and one write in flight. Each source/target
                    // retains its identity-checked channel and durable receipts.
                    let (ack, next) = if !eof && source.id != target.id {
                        let (ack, next) = tokio::join!(
                            self.source(target, write_action, write),
                            self.source(source, read_action, read_body(next_offset))
                        );
                        (ack?, Some(next?))
                    } else {
                        (self.source(target, write_action, write).await?, None)
                    };
                    offset = next_offset;
                    if ack["offset"] != offset {
                        bail!("Transfer acknowledgement mismatch; inspect receipt");
                    }
                    copy_progress(
                        &job,
                        completed + offset,
                        total,
                        done.len(),
                        row,
                        &started,
                        "copying",
                    );
                    if eof {
                        break;
                    }
                    chunk = if let Some(next) = next {
                        next
                    } else {
                        self.source(source, read_action, read_body(offset)).await?
                    };
                }
                let source_after = self
                    .source(
                        source,
                        if plan["scope"] == "filesystem" {
                            "explorer_stat"
                        } else {
                            "stat"
                        },
                        json!({"path":row["source"]}),
                    )
                    .await?;
                if source_after["pin"] != row["pin"] {
                    bail!("Source changed before publication");
                }
                let expected = hex::encode(sha.finalize());
                check_copy_cancel(&job)?;
                copy_progress(
                    &job,
                    completed + offset,
                    total,
                    done.len(),
                    row,
                    &started,
                    "verifying",
                );
                let receipt = self
                    .source(
                        target,
                        "transfer_finish",
                        json!({"id":id,"sha256":expected}),
                    )
                    .await?;
                if receipt["state"] != "applied" || receipt["sha256"] != expected {
                    bail!("Destination verification failed; inspect receipt");
                }
                receipt
            };
            if receipt["state"] != "applied" {
                bail!("Destination was not published; inspect receipt");
            }
            let mut item = row.clone();
            item["after"] = receipt["pin"].clone();
            item["sha256"] = receipt["sha256"].clone();
            item["transfer"] = json!({"method":receipt["method"].as_str().unwrap_or(if modern{"adaptive"}else{"compatible"}),"wire_bytes":receipt["wire_bytes"],"compressed_blocks":receipt["compressed_blocks"],"metadata":receipt["metadata"]});
            completed += row["pin"]["size"].as_u64().unwrap_or(0);
            done.push(item);
            self.store.db.lock().execute(
                "UPDATE workspace_plans SET receipt=? WHERE id=?",
                params![
                    json!({"ok":false,"items":done,"check_required":true}).to_string(),
                    plan["id"].as_str()
                ],
            )?;
            copy_progress(&job, completed, total, done.len(), row, &started, "copying");
        }
        Ok(json!({"ok":true,"items":done,"state":"applied"}))
    }
    pub async fn operation_plan(&self, b: &Value) -> Result<Value> {
        let cfg = self.config.read().clone();
        let d = cfg.device(text(b, "device", 64)?)?;
        let mut plan = if b["operation"] == "copy" {
            self.copy_plan(b).await?
        } else {
            self.source(d, "prepare", b.clone()).await?
        };
        plan["device"] = json!(d.id);
        plan["id"] = json!(uuid::Uuid::new_v4().to_string());
        plan["created"] = json!(crate::now());
        plan["config"] = json!(cfg.revision());
        self.store.db.lock().execute(
            "INSERT INTO workspace_plans VALUES(?,?,'preview',?,'{}')",
            params![plan["id"].as_str(), plan.to_string(), crate::now()],
        )?;
        Ok(plan)
    }
    pub fn operation(&self, id: &str) -> Result<Value> {
        let db = self.store.db.lock();
        let mut v = db
            .query_row(
                "SELECT id,state,receipt FROM workspace_plans WHERE id=?",
                [id],
                row_json,
            )
            .optional()?
            .context("Operation not found")?;
        v["receipt"] = serde_json::from_str(v["receipt"].as_str().unwrap_or("{}"))?;
        v["transfer"] = self
            .transfers
            .lock()
            .get(id)
            .map(|job| job.progress.lock().clone())
            .unwrap_or(json!({}));
        Ok(v)
    }
    pub fn start_operation(self: &Arc<Self>, id: &str) -> Result<Value> {
        uuid::Uuid::parse_str(id)?;
        let operation = self.operation(id)?;
        if operation["state"] != "preview" {
            bail!("This plan has been used; inspect its receipt");
        }
        let mut jobs = self.transfers.lock();
        if !jobs.is_empty() {
            bail!("Another file operation is active");
        }
        jobs.insert(id.into(), Arc::new(TransferJob::default()));
        drop(jobs);
        let engine = self.clone();
        let id = id.to_owned();
        let response = json!({"id":id,"state":"queued"});
        tokio::spawn(async move {
            if let Err(error) = engine.apply_operation(&id).await {
                let receipt = json!({"ok":false,"error":error.to_string(),"check_required":true});
                let _=engine.store.db.lock().execute("UPDATE workspace_plans SET state='inspection_required',receipt=? WHERE id=? AND state IN ('preview','running')",params![receipt.to_string(),id]);
            }
            engine.transfers.lock().remove(&id);
        });
        Ok(response)
    }
    pub fn cancel_operation(&self, id: &str) -> Result<Value> {
        let jobs = self.transfers.lock();
        let job = jobs.get(id).context("File operation is no longer active")?;
        job.cancel.store(true, Ordering::SeqCst);
        Ok(
            json!({"id":id,"state":"stopping","message":"Stops between transfer blocks or after the current native file copy; originals remain intact"}),
        )
    }
    pub async fn apply_operation(&self, id: &str) -> Result<Value> {
        let _permit = self
            .mutation
            .try_lock()
            .map_err(|_| anyhow::anyhow!("conflict: Another mutation is active"))?;
        if self.syncing.load(std::sync::atomic::Ordering::SeqCst) {
            bail!("conflict: Wait for indexing before applying a file operation");
        }
        let cfg = self.config.read().clone();
        let plan: Value = {
            let mut db = self.store.db.lock();
            let tx = db.transaction()?;
            let (body, state, created): (String, String, f64) = tx.query_row(
                "SELECT body,state,created FROM workspace_plans WHERE id=?",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            if state != "preview" {
                bail!("conflict: This plan has been used. Inspect its receipt before retrying.");
            }
            let plan: Value = serde_json::from_str(&body)?;
            if crate::now() - created > 600.0 || plan["config"] != cfg.revision() {
                bail!("Plan expired or settings changed; review again");
            }
            tx.execute(
                "UPDATE workspace_plans SET state='running' WHERE id=?",
                [id],
            )?;
            tx.commit()?;
            plan
        };
        let result = if plan["operation"] == "copy" {
            self.copy_files(&plan).await
        } else {
            self.source(
                cfg.device(plan["device"].as_str().unwrap_or(""))?,
                "execute",
                json!({"id":id,"plan":plan}),
            )
            .await
        };
        let receipt = match result {
            Ok(v) => v,
            Err(e) => {
                let raw: String = self.store.db.lock().query_row(
                    "SELECT receipt FROM workspace_plans WHERE id=?",
                    [id],
                    |r| r.get(0),
                )?;
                let mut partial: Value = serde_json::from_str(&raw)?;
                partial["ok"] = json!(false);
                partial["state"] = json!("inspection_required");
                partial["error"] = json!(e.to_string());
                partial["check_required"] = json!(true);
                partial["plan"] = plan["items"].clone();
                partial
            }
        };
        let state = if receipt["ok"] == true {
            "applied"
        } else {
            "inspection_required"
        };
        self.store.db.lock().execute(
            "UPDATE workspace_plans SET state=?,receipt=? WHERE id=?",
            params![state, receipt.to_string(), id],
        )?;
        if state == "applied" {
            self.relocate_metadata(&plan)?;
        }
        self.operation(id)
    }
    fn relocate_metadata(&self, plan: &Value) -> Result<()> {
        if plan["operation"] == "mkdir" || plan["operation"] == "copy" {
            return Ok(());
        }
        let d = plan["device"].as_str().unwrap_or("");
        let mappings = plan["items"].as_array().context("Missing mappings")?;
        let mut db = self.store.db.lock();
        let tx = db.transaction()?;
        // Stage catalogue paths in the same transaction so swaps preserve IDs
        // and never collide with the other half of the rename.
        let mut changes = Vec::new();
        for table in ["files", "workspace_meta", "workspace_comments"] {
            let records: Vec<(i64, String)> = {
                let mut s =
                    tx.prepare(&format!("SELECT rowid,path FROM {table} WHERE device=?"))?;
                s.query_map([d], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()?
            };
            for (id, path) in records {
                if let Some(m) = mappings.iter().find(|m| {
                    path == m["source"].as_str().unwrap_or("")
                        || path
                            .strip_prefix(m["source"].as_str().unwrap_or(""))
                            .is_some_and(|tail| tail.starts_with(['/', '\\']))
                }) {
                    let old = m["source"].as_str().unwrap();
                    let new = format!(
                        "{}{}",
                        m["destination"].as_str().unwrap(),
                        &path[old.len()..]
                    );
                    let temp = format!("retriever-catalogue-{}-{id}", uuid::Uuid::new_v4());
                    tx.execute(
                        &format!("UPDATE {table} SET path=? WHERE rowid=?"),
                        params![temp, id],
                    )?;
                    changes.push((table, id, new));
                }
            }
        }
        for (table, id, path) in changes {
            tx.execute(
                &format!("UPDATE {table} SET path=? WHERE rowid=?"),
                params![path, id],
            )?;
            if table == "files" {
                let name = path.rsplit(['/', '\\']).next().unwrap_or(&path);
                let extension = name
                    .rsplit_once('.')
                    .map(|(_, ext)| ext.to_lowercase())
                    .unwrap_or_default();
                tx.execute(
                    "UPDATE files SET name=?,extension=?,type=? WHERE id=?",
                    params![
                        name,
                        format!(".{extension}"),
                        crate::extract::kind(&extension).unwrap_or("other"),
                        id
                    ],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    pub async fn undo_plan(&self, id: &str) -> Result<Value> {
        let (raw, state, receipt): (String, String, String) = self.store.db.lock().query_row(
            "SELECT body,state,receipt FROM workspace_plans WHERE id=?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let plan: Value = serde_json::from_str(&raw)?;
        let receipt: Value = serde_json::from_str(&receipt)?;
        if state != "applied"
            || !["rename", "move", "trash"].contains(&plan["operation"].as_str().unwrap_or(""))
        {
            bail!("Only completed rename, move and trash operations support this undo");
        }
        let items: Vec<_> = receipt["items"]
            .as_array()
            .context("Missing receipt items")?
            .iter()
            .map(|r| {
                let source = r["source"].as_str().unwrap_or("");
                let (parent, name) = source.rsplit_once(['/', '\\']).unwrap_or(("", source));
                json!({"path":r["destination"],"parent":parent,"name":name,"pin":r["after"]})
            })
            .collect();
        self.operation_plan(&json!({"device":plan["device"],"operation":"move","scope":plan["scope"],"items":items}))
            .await
    }
    pub fn workspace(&self) -> Result<Value> {
        let cfg = self.config.read().clone();
        let db = self.store.db.lock();
        let mut stmt = db.prepare("SELECT * FROM workspace_views ORDER BY name")?;
        let mut views = stmt
            .query_map([], row_json)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for v in &mut views {
            v["body"] = serde_json::from_str(v["body"].as_str().unwrap_or("{}"))?;
        }
        let mut stmt =
            db.prepare("SELECT * FROM workspace_plans ORDER BY created DESC LIMIT 20")?;
        let mut history = stmt
            .query_map([], row_json)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for v in &mut history {
            let body: Value = serde_json::from_str(v["body"].as_str().unwrap_or("{}"))?;
            v["operation"] = body["operation"].clone();
            v["receipt"] = serde_json::from_str(v["receipt"].as_str().unwrap_or("{}"))?;
        }
        let mut stmt = db.prepare("SELECT * FROM folder_sync_pairs ORDER BY name")?;
        let pairs = stmt
            .query_map([], row_json)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(
            json!({"devices":cfg.devices.iter().filter(|d|d.enabled).collect::<Vec<_>>(),"views":views,"history":history,"transfer":{},"models_on_sources":false,"tunnels":[],"sync_pairs":pairs}),
        )
    }
    pub fn metadata(&self, device: &str, path: &str) -> Result<Value> {
        self.metadata_scoped(device, path, false)
    }
    pub fn metadata_scoped(&self, device: &str, path: &str, interactive: bool) -> Result<Value> {
        let cfg = self.config.read();
        if interactive {
            let d = cfg.device(device)?;
            if !d.enabled
                || !crate::paths::absolute(path)
                || path.replace('\\', "/").split('/').any(|p| p == "..")
            {
                bail!("Invalid interactive file identity");
            }
            if d.transport == "local" {
                crate::explorer::checked(path, &[self.state.clone()])?;
            }
        } else if !crate::paths::eligible(cfg.device(device)?, path) {
            bail!("File is outside included folders");
        }
        let db = self.store.db.lock();
        let mut value=db.query_row("SELECT * FROM workspace_meta WHERE device=? AND path=?",params![device,path],row_json).optional()?.unwrap_or(json!({"device":device,"path":path,"owner":"","tags":"[]","status":"","starred":0,"properties":"{}","revision":0}));
        value["tags"] = serde_json::from_str(value["tags"].as_str().unwrap_or("[]"))?;
        value["properties"] = serde_json::from_str(value["properties"].as_str().unwrap_or("{}"))?;
        value["priority"] = value["properties"]["priority"]
            .as_str()
            .unwrap_or("")
            .into();
        value["starred"] = json!(value["starred"].as_i64().unwrap_or(0) != 0);
        let mut stmt=db.prepare("SELECT * FROM (SELECT id,author,body,created FROM workspace_comments WHERE device=? AND path=? ORDER BY created DESC,id DESC LIMIT 200) ORDER BY created,id")?;
        value["comments"] = json!(
            stmt.query_map(params![device, path], row_json)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        );
        Ok(value)
    }
    pub fn annotate(&self, b: &Value) -> Result<Value> {
        let d = text(b, "device", 64)?;
        let p = text(b, "path", 4096)?;
        self.metadata_scoped(d, p, b["scope"] == "filesystem")?;
        let owner = b["owner"].as_str().unwrap_or("");
        let status = b["status"].as_str().unwrap_or("");
        let tags = b.get("tags").cloned().unwrap_or(json!([]));
        let properties = b.get("properties").cloned().unwrap_or(json!({}));
        if owner.len() > 100
            || status.len() > 100
            || !tags.as_array().is_some_and(|a| {
                a.len() <= 50 && a.iter().all(|t| t.as_str().is_some_and(|s| s.len() <= 60))
            })
            || !properties.is_object()
            || properties.to_string().len() > 16000
        {
            bail!("Invalid metadata");
        }
        let expected = b["revision"]
            .as_i64()
            .context("Missing property revision")?;
        {
            let mut db = self.store.db.lock();
            let tx = db.transaction()?;
            crate::properties::validate_values(&tx, &properties)?;
            let revision = tx
                .query_row(
                    "SELECT revision FROM workspace_meta WHERE device=? AND path=?",
                    params![d, p],
                    |r| r.get::<_, i64>(0),
                )
                .optional()?
                .unwrap_or(0);
            if revision != expected {
                bail!("conflict: Properties changed; reload before saving");
            }
            let mut clean_tags = Vec::<String>::new();
            for tag in tags.as_array().unwrap() {
                let tag = tag.as_str().unwrap().trim();
                if !tag.is_empty()
                    && !clean_tags
                        .iter()
                        .any(|v| v.to_lowercase() == tag.to_lowercase())
                {
                    clean_tags.push(tag.into());
                }
            }
            tx.execute("INSERT INTO workspace_meta VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(device,path) DO UPDATE SET owner=excluded.owner,tags=excluded.tags,status=excluded.status,starred=excluded.starred,properties=excluded.properties,revision=excluded.revision",params![d,p,owner,json!(clean_tags).to_string(),status,b["starred"].as_bool().unwrap_or(false),properties.to_string(),revision+1])?;
            tx.commit()?;
        }
        self.metadata_scoped(d, p, b["scope"] == "filesystem")
    }
    pub fn comment(&self, b: &Value) -> Result<Value> {
        let d = text(b, "device", 64)?;
        let p = text(b, "path", 4096)?;
        self.metadata_scoped(d, p, b["scope"] == "filesystem")?;
        let author = text(b, "author", 100)?;
        let body = text(b, "body", 8000)?;
        self.store.db.lock().execute(
            "INSERT INTO workspace_comments VALUES(?,?,?,?,?,?)",
            params![
                uuid::Uuid::new_v4().to_string(),
                d,
                p,
                author,
                body,
                crate::now()
            ],
        )?;
        self.metadata_scoped(d, p, b["scope"] == "filesystem")
    }
    pub async fn save_view(&self, b: &Value) -> Result<Value> {
        let id = b["id"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        uuid::Uuid::parse_str(&id).context("Invalid saved view ID")?;
        if b["delete"] == true || b["remove"] == true {
            self.store
                .db
                .lock()
                .execute("DELETE FROM workspace_views WHERE id=?", [&id])?;
            return Ok(json!({"ok":true}));
        }
        let name = text(b, "name", 100)?;
        let kind = b["kind"].as_str().unwrap_or("view");
        if !["view", "share"].contains(&kind) {
            bail!("Invalid saved view type");
        }
        let body = if kind == "share" {
            let cfg = self.config.read().clone();
            let d = cfg.device(text(b, "device", 64)?)?;
            let folder = self
                .source(d, "stat", json!({"path":text(b,"path",4096)?}))
                .await?;
            if folder["kind"] != "folder" {
                bail!("Choose a folder to share");
            }
            json!({"device":d.id,"path":folder["path"]})
        } else {
            b.get("body").cloned().unwrap_or_else(||json!({"query":b["query"].as_str().unwrap_or(""),"filters":b.get("filters").cloned().unwrap_or(json!({"match":"all","rules":[]}))}))
        };
        if kind == "view" {
            validate_filters(&body["filters"])?;
        }
        if body.to_string().len() > 16000 {
            bail!("Saved view is too large");
        }
        self.store.db.lock().execute("INSERT INTO workspace_views VALUES(?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,kind=excluded.kind,body=excluded.body,updated=excluded.updated",params![id,name,kind,body.to_string(),crate::now()])?;
        Ok(json!({"id":id,"name":name,"kind":kind,"body":body}))
    }
    pub async fn browse(&self, b: &Value) -> Result<Value> {
        let cfg = self.config.read().clone();
        let d = cfg.device(text(b, "device", 64)?)?;
        let mut result = self.source(d, "list", b.clone()).await?;
        if let Some(entries) = result["entries"].as_array_mut() {
            for row in entries {
                row["metadata"] = self.metadata(&d.id, row["path"].as_str().unwrap_or(""))?;
                self.attach_duration(row)?;
                let id: Option<i64> = self
                    .store
                    .db
                    .lock()
                    .query_row(
                        "SELECT id FROM files WHERE device=? AND path=?",
                        params![d.id, row["path"].as_str().unwrap_or("")],
                        |r| r.get(0),
                    )
                    .optional()?;
                if let Some(id) = id {
                    row["id"] = json!(id);
                }
            }
        }
        Ok(result)
    }
    pub fn library(&self, b: &Value) -> Result<Value> {
        // Validate even when the catalogue is empty or the query matches no rows.
        validate_filters(&b["filters"])?;
        let offset = b["offset"].as_u64().unwrap_or(0) as usize;
        if offset > 1_000_000 {
            bail!("Invalid page");
        }
        let cfg = self.config.read().clone();
        let query = b["query"].as_str().unwrap_or("").to_lowercase();
        let sort = match b["sort"].as_str().unwrap_or("name") {
            "name" => "f.name COLLATE NOCASE",
            "modified" => "f.mtime_ns",
            "size" => "f.size",
            "owner" => "m.owner COLLATE NOCASE",
            "type" => "f.extension COLLATE NOCASE",
            "priority" => {
                "CASE json_extract(m.properties,'$.priority') WHEN 'urgent' THEN 4 WHEN 'high' THEN 3 WHEN 'medium' THEN 2 WHEN 'low' THEN 1 ELSE 0 END"
            }
            _ => bail!("Invalid library sort"),
        };
        let direction = match b["direction"].as_str().unwrap_or("asc") {
            "asc" => "ASC",
            "desc" => "DESC",
            _ => bail!("Invalid sort direction"),
        };
        let sql = format!(
            "SELECT f.id,f.device,f.path,f.name,f.type,f.extension,f.size,f.mtime_ns,COALESCE(d.status,'offline') source_status,COALESCE(m.owner,'') owner,COALESCE(m.tags,'[]') tags,COALESCE(m.status,'') status,COALESCE(m.starred,0) starred,COALESCE(m.properties,'{{}}') properties FROM files f LEFT JOIN workspace_meta m ON f.device=m.device AND f.path=m.path LEFT JOIN devices d ON d.id=f.device ORDER BY {sort} {direction},f.id"
        );
        let db = self.store.db.lock();
        let mut stmt = db.prepare(&sql)?;
        let mut cursor = stmt.query([])?;
        let mut rows = Vec::new();
        let mut total = 0;
        while let Some(row) = cursor.next()? {
            let mut v = row_json(row)?;
            if !v["name"]
                .as_str()
                .unwrap_or("")
                .to_lowercase()
                .contains(&query)
                || !cfg.devices.iter().any(|d| {
                    d.id == v["device"].as_str().unwrap_or("")
                        && crate::paths::eligible(d, v["path"].as_str().unwrap_or(""))
                })
            {
                continue;
            }
            let tags: Value = serde_json::from_str(v["tags"].as_str().unwrap_or("[]"))?;
            v["tags"] = tags.clone();
            v["properties"] = serde_json::from_str(v["properties"].as_str().unwrap_or("{}"))?;
            v["priority"] = v["properties"]["priority"].as_str().unwrap_or("").into();
            v["starred"] = json!(v["starred"].as_i64().unwrap_or(0) != 0);
            v["modified"] = json!(v["mtime_ns"].as_f64().unwrap_or(0.0) / 1e9);
            if !matches_filters(&v, &b["filters"]) {
                continue;
            }
            if total >= offset && rows.len() < 200 {
                v["metadata"] = json!({"owner":v["owner"],"tags":tags,"status":v["status"],"starred":v["starred"],"properties":v["properties"],"priority":v["priority"]});
                v["kind"] = json!("file");
                v["cached"] = json!(v["source_status"] != "ready");
                rows.push(v);
            }
            total += 1;
        }
        drop(cursor);
        drop(stmt);
        drop(db);
        for row in &mut rows {
            self.attach_duration(row)?;
        }
        Ok(
            json!({"entries":rows,"total":total,"offset":offset,"next_offset":if offset+200<total{Some(offset+200)}else{None},"cached":true,"filters":b.get("filters").cloned().unwrap_or(json!({"match":"all","rules":[]}))}),
        )
    }
    pub async fn preview(&self, b: &Value) -> Result<Value> {
        let d = text(b, "device", 64)?;
        let path = text(b, "path", 4096)?;
        let metadata = self.metadata(d, path)?;
        let cfg = self.config.read().clone();
        let id: Option<i64> = self
            .store
            .db
            .lock()
            .query_row(
                "SELECT id FROM files WHERE device=? AND path=?",
                params![d, path],
                |r| r.get(0),
            )
            .optional()?;
        let cached = id
            .map(|id| self.store.result(id, &cfg))
            .transpose()?
            .flatten();
        let live_entry = self
            .source(cfg.device(d)?, "stat", json!({"path":path}))
            .await;
        if let Ok(entry) = &live_entry {
            if entry["kind"] == "folder" {
                return Ok(
                    json!({"device":d,"path":path,"name":entry["name"],"kind":"folder","metadata":metadata,"text":"Open this folder to browse its contents.","preview":{"kind":"folder","text":"Open this folder to browse its contents."}}),
                );
            }
        }
        let live = self
            .source(
                cfg.device(d)?,
                "read",
                json!({"path":path,"maximum":1048576}),
            )
            .await;
        let (text, pin, offline) = match live {
            Ok(v) => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(v["data"].as_str().unwrap_or(""))?;
                (
                    String::from_utf8_lossy(&bytes)
                        .chars()
                        .take(64000)
                        .collect::<String>(),
                    v["pin"].clone(),
                    false,
                )
            }
            Err(e) => {
                let reason = e.to_string().to_lowercase();
                if reason.contains("secret_content")
                    || reason.contains("outside")
                    || reason.contains("symlink")
                    || reason.contains("junction")
                    || reason.contains("symbolic")
                    || reason.contains("reparse")
                    || reason.contains("changed")
                {
                    if let Some(id) = id {
                        if reason.contains("secret_content") {
                            self.store
                                .db
                                .lock()
                                .execute("DELETE FROM files WHERE id=?", [id])?;
                        } else {
                            self.store.db.lock().execute("UPDATE files SET text='',detail='source_changed_preview',digest='' WHERE id=?", [id])?;
                        }
                    }
                    return Err(e);
                }
                if let Some(r) = &cached {
                    (
                        r["text"]
                            .as_str()
                            .unwrap_or("")
                            .chars()
                            .take(64000)
                            .collect(),
                        live_entry
                            .as_ref()
                            .ok()
                            .map(|v| v["pin"].clone())
                            .unwrap_or(Value::Null),
                        live_entry.is_err(),
                    )
                } else {
                    return Err(e);
                }
            }
        };
        let ext = path
            .rsplit_once('.')
            .map(|(_, e)| e.to_lowercase())
            .unwrap_or_default();
        let text = if matches!(
            crate::extract::kind(&ext),
            Some("image" | "audio" | "video")
        ) || ["pdf", "docx", "xlsx", "pptx"].contains(&ext.as_str())
        {
            cached
                .as_ref()
                .and_then(|r| r["text"].as_str())
                .unwrap_or("No text preview is available for this format.")
                .chars()
                .take(64000)
                .collect::<String>()
        } else {
            text
        };
        let indexed = cached.as_ref().map(
            |r| json!({"id":r["id"],"type":r["type"],"digest":r["digest"],"detail":r["detail"]}),
        );
        let content_cached = offline
            || pin["size"].as_u64().unwrap_or(0) > 1048576
            || ["pdf", "docx", "xlsx", "pptx"].contains(&ext.as_str())
            || matches!(
                crate::extract::kind(&ext),
                Some("image" | "video" | "audio")
            );
        let mut reply = json!({"device":d,"path":path,"name":path.rsplit(['/', '\\']).next(),"kind":"file","extension":format!(".{ext}"),"size":pin["size"],"text":text,"content":text,"preview":{"kind":"text","text":text,"cached":offline},"pin":pin,"metadata":metadata,"cached":offline,"offline":offline,"source_status":if offline{"offline_cached"}else{"live"},"indexed":indexed});
        reply["cached"] = json!(content_cached);
        reply["preview"]["cached"] = json!(content_cached);
        if b["visual"] == true
            && !offline
            && (ext == "pdf" || matches!(crate::extract::kind(&ext), Some("image" | "video")))
        {
            match self
                .source(
                    cfg.device(d)?,
                    "preview_image",
                    json!({"path":path,"pin":pin}),
                )
                .await
            {
                Ok(image) => {
                    reply["image"] = image["image"].clone();
                    reply["coverage"] = image["coverage"].clone();
                }
                Err(error) => {
                    let message = error.to_string().to_lowercase();
                    reply["visual_pending"] = json!(message.contains("preview_busy"));
                    if [
                        "secret_content",
                        "outside",
                        "symlink",
                        "junction",
                        "symbolic",
                        "reparse",
                        "changed",
                    ]
                    .iter()
                    .any(|word| message.contains(word))
                    {
                        return Err(error);
                    }
                    reply["visual_error"] = json!(if message.contains("budget") {
                        "This file exceeds the quick-preview budget. Open it for the full view."
                    } else if message.contains("preview_busy") {
                        "Another preview is finishing. Select the file again to retry."
                    } else {
                        "Visual preview unavailable. Indexed text is shown when available."
                    });
                }
            }
        }
        Ok(reply)
    }
}
pub fn text<'a>(v: &'a Value, key: &str, max: usize) -> Result<&'a str> {
    let s = v[key].as_str().context(format!("Missing {key}"))?;
    if s.is_empty() || s.len() > max || s.contains('\0') {
        bail!("Invalid {key}");
    }
    Ok(s)
}
fn validate_filters(filter: &Value) -> Result<()> {
    if !filter.is_null()
        && (!filter.is_object()
            || filter
                .as_object()
                .is_some_and(|v| v.keys().any(|k| !["match", "rules"].contains(&k.as_str())))
            || filter
                .get("match")
                .is_some_and(|v| !matches!(v.as_str(), Some("all" | "any")))
            || filter.get("rules").is_some_and(|v| !v.is_array()))
    {
        bail!("Invalid filter structure");
    }
    let Some(rules) = filter["rules"].as_array() else {
        return Ok(());
    };
    if rules.len() > 20 {
        bail!("Too many filters");
    }
    for r in rules {
        if !r.as_object().is_some_and(|v| {
            v.len() == 3
                && ["field", "operator", "value"]
                    .iter()
                    .all(|k| v.contains_key(*k))
        }) {
            bail!("Invalid filter rule");
        }
        let field = r["field"].as_str().unwrap_or("");
        if ![
            "name",
            "device",
            "type",
            "extension",
            "path",
            "size",
            "modified",
            "owner",
            "tags",
            "status",
            "priority",
            "starred",
            "category",
        ]
        .contains(&field)
        {
            bail!("Invalid filter field");
        }
        let op = r["operator"].as_str().unwrap_or("");
        let value = &r["value"];
        if !["is", "is_not", "contains", "starts_with", "greater", "less"].contains(&op) {
            bail!("Invalid filter operator");
        }
        if ["size", "modified"].contains(&field) {
            if !value
                .as_f64()
                .is_some_and(|n| n.is_finite() && (0.0..=1e20).contains(&n))
                || !["is", "is_not", "greater", "less"].contains(&op)
            {
                bail!("Invalid numeric filter");
            }
        } else if field == "starred" {
            if !value.is_boolean() || !["is", "is_not"].contains(&op) {
                bail!("Invalid favorite filter");
            }
        } else if !value.as_str().is_some_and(|s| {
            s.chars().count() <= if field == "path" { 4096 } else { 200 }
                && !s.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
        }) || ["greater", "less"].contains(&op)
            || (field == "tags" && !["contains", "is_not"].contains(&op))
        {
            bail!("Invalid text filter");
        }
    }
    Ok(())
}
fn matches_filters(row: &Value, filter: &Value) -> bool {
    let Some(rules) = filter["rules"].as_array() else {
        return true;
    };
    if rules.is_empty() {
        return true;
    }
    let mut matches = Vec::with_capacity(rules.len());
    for r in rules {
        let field = r["field"].as_str().unwrap_or("");
        let a = &row[field];
        let b = &r["value"];
        // Legacy catalogues omit the extension's dot; current collectors retain it.
        // Both forms must describe the same file type, including saved filters.
        let normalize = |value: &str| {
            let value = value.to_lowercase();
            if field == "extension" {
                value.trim().trim_start_matches('.').to_owned()
            } else {
                value
            }
        };
        let string = normalize(a.as_str().unwrap_or(""));
        let needle = normalize(b.as_str().unwrap_or("").trim());
        let same = if ["size", "modified"].contains(&field) {
            a.as_f64() == b.as_f64()
        } else if field == "tags" {
            a.as_array().is_some_and(|a| {
                a.iter()
                    .any(|s| normalize(s.as_str().unwrap_or("")) == needle)
            })
        } else {
            a == b || (a.is_string() && b.is_string() && string == needle)
        };
        matches.push(match r["operator"].as_str().unwrap_or("") {
            "is" => same,
            "is_not" => !same,
            "contains" => {
                if field == "tags" {
                    same
                } else {
                    string.contains(&needle)
                }
            }
            "starts_with" => string.starts_with(&needle),
            "greater" => a.as_f64().unwrap_or(0.0) > b.as_f64().unwrap_or(0.0),
            "less" => a.as_f64().unwrap_or(0.0) < b.as_f64().unwrap_or(0.0),
            _ => false,
        });
    }
    if filter["match"] == "any" {
        matches.iter().any(|v| *v)
    } else {
        matches.iter().all(|v| *v)
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;

    #[test]
    fn imported_and_recrawled_extensions_share_filter_semantics() {
        for extension in ["pdf", ".pdf", ".PDF"] {
            for value in ["pdf", ".pdf", ".PDF"] {
                let filter = json!({"match":"all","rules":[{"field":"extension","operator":"is","value":value}]});
                assert!(matches_filters(&json!({"extension":extension}), &filter));
                assert!(!matches_filters(&json!({"extension":".docx"}), &filter));
                let inverse =
                    json!({"rules":[{"field":"extension","operator":"is_not","value":value}]});
                assert!(!matches_filters(&json!({"extension":extension}), &inverse));
            }
        }
        // Dot normalization must not change filename/path filter semantics.
        assert!(!matches_filters(
            &json!({"name":".config"}),
            &json!({"rules":[{"field":"name","operator":"is","value":"config"}]})
        ));
        assert!(!matches_filters(
            &json!({"name":" config"}),
            &json!({"rules":[{"field":"name","operator":"is","value":"config"}]})
        ));
    }
}
