use crate::{config::Device, extract, paths};
use anyhow::{Context, Result, bail};
use base64::Engine as _;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
pub fn mtime(m: &fs::Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}
pub fn pin(path: &Path) -> Result<Value> {
    paths::reject_links(path)?;
    let m = fs::metadata(path)?;
    Ok(json!({"size":m.len(),"mtime_ns":mtime(&m),"kind":if m.is_dir(){"folder"}else{"file"}}))
}
pub fn device(body: &Value) -> Result<Device> {
    let mut d: Device = serde_json::from_value(body["device_config"].clone())?;
    for r in &mut d.roots {
        *r = paths::expand(r).to_string_lossy().into_owned();
    }
    for r in &mut d.exclude {
        if r.starts_with('~') {
            *r = paths::expand(r).to_string_lossy().into_owned();
        }
    }
    Ok(d)
}
pub fn dispatch(action: &str, body: &Value, forbidden: &[PathBuf]) -> Result<Value> {
    if action == "probe" {
        return Ok(
            json!({"version":retriever_protocol::VERSION,"engine_version":env!("CARGO_PKG_VERSION"),"platform":std::env::consts::OS,"arch":std::env::consts::ARCH,"supported":true,"python_required":false,"inventory_version":1,"analysis_input":true,"node_protocol":1,"transfer_protocol":2,"capabilities":["browse","folder_tree","preview","copy_sha256","adaptive_lz4","native_copy","rename","move","trash_restore"],"models_required":false,"health_collection":false}),
        );
    }
    if action == "folders" {
        return folders(body, forbidden);
    }
    let d = device(body)?;
    if action=="explorer_locations" {if !d.enabled {bail!("Device is disabled");}return Ok(crate::explorer::locations());}
    if action=="explorer_list" {return crate::explorer::list(&d,body,forbidden);}
    if action=="explorer_children" {return crate::explorer::children(&d,body,forbidden);}
    if action=="crawl_preview" {return crate::onboarding::preview(&d,body,forbidden);}
    if action=="explorer_stat" {return crate::explorer::stat(&d,body,forbidden);}
    if action=="explorer_preview" {return crate::explorer::preview(&d,body,forbidden);}
    if action=="media_duration" {return crate::explorer::media_duration(&d,body,forbidden);}
    if action=="storage_snapshot" {return crate::remote::storage_snapshot(&d,body,forbidden);}
    if action.starts_with("inventory_") {
        return crate::inventory::dispatch(&d, action, body, forbidden);
    }
    if action == "analysis_input" {
        let p = paths::checked(
            &d,
            body["path"].as_str().context("Missing path")?,
            forbidden,
            true,
        )?;
        let before = pin(&p)?;
        let extension = p
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        if extract::kind(&extension) == Some("video") {
            let mut snapshot = tempfile::Builder::new()
                .prefix("retriever-video-")
                .suffix(&format!(".{extension}"))
                .tempfile()?;
            let maximum = body["maximum"].as_u64().unwrap_or(33554432).min(8589934592);
            let (digest, _) = extract::stream_source(&p, maximum, Some(snapshot.as_file_mut()))?;
            if body["digest"] != digest || before != pin(&p)? {
                bail!("source_changed_during_analysis");
            }
            let mut value = crate::media::video_thumbnails_snapshot(snapshot)?;
            value["digest"] = body["digest"].clone();
            return Ok(value);
        }
        let bytes = extract::read_bounded(
            &p,
            body["maximum"].as_u64().unwrap_or(33554432).min(268435456),
        )?;
        if paths::secret_content(&bytes) {
            bail!("secret_content");
        }
        if body["digest"] != crate::digest(&bytes) || before != pin(&p)? {
            bail!("source_changed_during_analysis");
        }
        let image = if extension == "pdf" {
            crate::media::pdf_page(&bytes, 0)?.0
        } else {
            crate::media::image_pixels(&bytes)?
        };
        return Ok(
            json!({"digest":body["digest"],"image":image,"coverage":if extension=="pdf"{"first_page"}else{"image"}}),
        );
    }
    if action.starts_with("transfer_") {
        return crate::transfer::dispatch(&d, action, body, forbidden);
    }
    if action == "preview_image" {
        // Only one interactive decoder per agent. Rapid selection changes cannot
        // queue an unbounded number of expensive snapshots or inference jobs.
        static PREVIEW: std::sync::LazyLock<parking_lot::Mutex<()>> =
            std::sync::LazyLock::new(|| parking_lot::Mutex::new(()));
        let _guard = PREVIEW.try_lock().context("preview_busy")?;
        let p = paths::checked(
            &d,
            body["path"].as_str().context("Missing path")?,
            forbidden,
            true,
        )?;
        let before = pin(&p)?;
        let extension = p
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        let maximum = if extract::kind(&extension) == Some("video") {
            268435456
        } else {
            33554432
        };
        if before["kind"] != "file" || before["size"].as_u64().unwrap_or(u64::MAX) > maximum {
            bail!("preview_budget");
        }
        if body
            .get("pin")
            .is_some_and(|v| !v.is_null() && v != &before)
        {
            bail!("source_changed_during_preview");
        }
        let (image, coverage) = if extract::kind(&extension) == Some("video") {
            let mut snapshot = tempfile::Builder::new()
                .prefix("retriever-preview-")
                .suffix(&format!(".{extension}"))
                .tempfile()?;
            extract::stream_source(&p, maximum, Some(snapshot.as_file_mut()))?;
            let image = crate::media::video_preview_snapshot(snapshot)?;
            (image, "Video frame at 0:00 · audio not played")
        } else {
            let bytes = extract::read_bounded(&p, maximum)?;
            if paths::secret_content(&bytes) {
                bail!("secret_content");
            }
            if extension == "pdf" {
                (crate::media::pdf_page(&bytes, 0)?.0, "PDF · first page")
            } else if extract::kind(&extension) == Some("image") {
                (crate::media::image_pixels(&bytes)?, "Image preview")
            } else {
                bail!("Unsupported visual preview");
            }
        };
        if before != pin(&p)? {
            bail!("source_changed_during_preview");
        }
        return Ok(
            json!({"image":image["content"][0]["image_url"]["url"],"coverage":coverage,"pin":before}),
        );
    }
    match action {
        "stat" => {
            let p = paths::checked(
                &d,
                body["path"].as_str().context("Missing path")?,
                forbidden,
                true,
            )?;
            let pin = pin(&p)?;
            Ok(
                json!({"path":p.to_string_lossy(),"name":p.file_name().unwrap_or_default().to_string_lossy(),"kind":pin["kind"],"size":pin["size"],"pin":pin}),
            )
        }
        "prepare" => crate::operations::prepare(&d, body, forbidden),
        "execute" => crate::operations::execute(&d, body, forbidden),
        "inspect" => crate::operations::inspect(body),
        "list" => list(&d, body, forbidden),
        "scan" => scan_page(&d, body, forbidden),
        "fingerprint" => {
            let p = paths::checked(
                &d,
                body["path"].as_str().context("Missing path")?,
                forbidden,
                true,
            )?;
            let before = pin(&p)?;
            let limit = body["maximum"].as_u64().unwrap_or(33554432).min(8589934592);
            if before["kind"] != "file" || before["size"].as_u64().unwrap_or(u64::MAX) > limit {
                bail!("file_budget");
            }
            let (digest, _) = extract::stream_source(&p, limit, None)?;
            if pin(&p)? != before {
                bail!("source_changed_during_verification");
            }
            Ok(json!({"path":p.to_string_lossy(),"digest":digest,"pin":before}))
        }
        "read" => {
            let p = paths::checked(
                &d,
                body["path"].as_str().context("Missing path")?,
                forbidden,
                true,
            )?;
            let before = pin(&p)?;
            if body.get("pin").is_some_and(|v| v != &before) {
                bail!("File changed since preview");
            }
            let bytes = extract::read_bounded(
                &p,
                body["maximum"]
                    .as_u64()
                    .unwrap_or(1024 * 1024)
                    .min(1024 * 1024),
            )?;
            if paths::secret_content(&bytes) {
                bail!("secret_content");
            }
            if before != pin(&p)? {
                bail!("File changed while reading");
            }
            Ok(
                json!({"path":p.to_string_lossy(),"pin":before,"sha256":crate::digest(&bytes),"data":base64::engine::general_purpose::STANDARD.encode(bytes)}),
            )
        }
        _ => bail!("Unsupported agent action; mutation protocol is not enabled in this candidate"),
    }
}
fn folders(body: &Value, forbidden: &[PathBuf]) -> Result<Value> {
    let home = paths::expand("~/");
    let requested = body["path"]
        .as_str()
        .filter(|p| !p.is_empty())
        .map(paths::expand)
        .unwrap_or(home.clone());
    if !requested.is_absolute()
        || requested
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        bail!("Invalid folder path");
    }
    paths::reject_links(&requested)?;
    let requested = requested.canonicalize()?;
    if forbidden.iter().any(|p| requested.starts_with(p)) {
        bail!("Application state is not a source folder");
    }
    let offset = body["offset"].as_u64().unwrap_or(0) as usize;
    let filter = body["query"].as_str().unwrap_or("").to_lowercase();
    let mut folders = Vec::new();
    for e in fs::read_dir(&requested)? {
        let Ok(e) = e else { continue };
        let p = e.path();
        if !paths::safe_name(&p.to_string_lossy())
            || paths::reject_links(&p).is_err()
            || !p.is_dir()
            || !e
                .file_name()
                .to_string_lossy()
                .to_lowercase()
                .contains(&filter)
        {
            continue;
        }
        folders.push(json!({"name":e.file_name().to_string_lossy(),"path":p.to_string_lossy()}));
    }
    folders.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .cmp(&b["name"].as_str().unwrap_or("").to_lowercase())
    });
    let total = folders.len();
    Ok(
        json!({"platform":if cfg!(windows){"windows"}else if cfg!(target_os="macos"){"macos"}else{"linux"},"path":requested.to_string_lossy(),"parent":requested.parent().map(|p|p.to_string_lossy()),"folders":folders.into_iter().skip(offset).take(200).collect::<Vec<_>>(),"total":total,"offset":offset,"next_offset":if offset+200<total{Some(offset+200)}else{None},"shortcuts":[{"name":"Home","path":home.to_string_lossy()}],"roots":[home.to_string_lossy()],"breadcrumbs":requested.ancestors().map(|p|json!({"name":p.file_name().unwrap_or_default().to_string_lossy(),"path":p.to_string_lossy()})).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>()}),
    )
}
pub fn list(d: &Device, body: &Value, forbidden: &[PathBuf]) -> Result<Value> {
    let path = paths::checked(
        d,
        body["path"].as_str().context("Missing folder")?,
        forbidden,
        true,
    )?;
    let offset = body["offset"].as_u64().unwrap_or(0) as usize;
    if offset > 1_000_000 {
        bail!("Invalid page offset");
    }
    let query = body["query"].as_str().unwrap_or("").to_lowercase();
    let mut entries = Vec::new();
    let mut skipped = 0;
    for e in fs::read_dir(&path)? {
        let Ok(e) = e else {
            skipped += 1;
            continue;
        };
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if paths::checked(d, &p.to_string_lossy(), forbidden, true).is_err() {
            skipped += 1;
            continue;
        }
        if !name.to_lowercase().contains(&query) {
            continue;
        }
        if let Ok(m) = e.metadata() {
            let kind = if m.is_dir() { "folder" } else { "file" };
            let ext = p
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            entries.push(json!({"device":d.id,"path":p.to_string_lossy(),"name":name,"kind":kind,"size":m.len(),"mtime_ns":mtime(&m),"modified":mtime(&m) as f64/1e9,"extension":format!(".{ext}"),"type":extract::kind(&ext).unwrap_or(if m.is_dir(){"folder"}else{"other"}),"pin":{"size":m.len(),"mtime_ns":mtime(&m),"kind":kind}}));
        }
    }
    let sort = body["sort"].as_str().unwrap_or("name");
    if !["name", "size", "modified", "type"].contains(&sort) {
        bail!("Invalid sort");
    }
    let descending =
        body["direction"]
            .as_str()
            .unwrap_or(if ["size", "modified"].contains(&sort) {
                "desc"
            } else {
                "asc"
            })
            == "desc";
    entries.sort_by(|a, b| {
        let folders = (b["kind"] == "folder").cmp(&(a["kind"] == "folder"));
        if !folders.is_eq() {
            return folders;
        }
        let cmp = if sort == "size" || sort == "modified" {
            a[sort]
                .as_f64()
                .unwrap_or(0.0)
                .total_cmp(&b[sort].as_f64().unwrap_or(0.0))
        } else {
            a[sort]
                .as_str()
                .unwrap_or("")
                .to_lowercase()
                .cmp(&b[sort].as_str().unwrap_or("").to_lowercase())
        };
        (if descending { cmp.reverse() } else { cmp })
            .then_with(|| a["path"].as_str().cmp(&b["path"].as_str()))
    });
    let total = entries.len();
    Ok(
        json!({"path":path.to_string_lossy(),"entries":entries.into_iter().skip(offset).take(200).collect::<Vec<_>>(),"total":total,"offset":offset,"next_offset":if offset+200<total{Some(offset+200)}else{None},"skipped":skipped,"cached":false}),
    )
}
pub fn scan_page(d: &Device, body: &Value, forbidden: &[PathBuf]) -> Result<Value> {
    use sha2::{Digest, Sha256};
    let max = body["limits"]["max_file_bytes"]
        .as_u64()
        .unwrap_or(33554432)
        .min(268435456);
    let max_text = body["limits"]["max_text_chars"]
        .as_u64()
        .unwrap_or(1048576)
        .min(16000000) as usize;
    let chunk = body["chunk_chars"]
        .as_u64()
        .unwrap_or(2000)
        .clamp(256, 8000) as usize;
    let offset = body["offset"].as_u64().unwrap_or(0) as usize;
    let mut candidates = Vec::new();
    let mut errors = Vec::new();
    let mut hash = Sha256::new();
    let mut complete = true;
    for root in &d.roots {
        let root = match paths::checked(d, root, forbidden, true) {
            Ok(p) => p,
            Err(_) => {
                errors.push(json!({"path":root,"reason":"root_unavailable_or_blocked"}));
                complete = false;
                continue;
            }
        };
        for e in walkdir::WalkDir::new(root)
            .follow_links(false)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(|e| {
                paths::safe_name(&e.path().to_string_lossy())
                    && paths::eligible(d, &e.path().to_string_lossy())
                    && paths::reject_links(e.path()).is_ok()
            })
        {
            let e = match e {
                Ok(v) => v,
                Err(_) => {
                    complete = false;
                    if errors.len() < 100 {
                        errors.push(json!({"reason":"directory_unreadable"}));
                    }
                    continue;
                }
            };
            if !e.file_type().is_file() {
                continue;
            }
            let p = e.path();
            let ext = p
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            if extract::kind(&ext).is_none() {
                continue;
            }
            if paths::checked(d, &p.to_string_lossy(), forbidden, true).is_err() {
                continue;
            }
            let Ok(m) = e.metadata() else {
                complete = false;
                continue;
            };
            hash.update(p.to_string_lossy().as_bytes());
            hash.update(m.len().to_le_bytes());
            hash.update(mtime(&m).to_le_bytes());
            candidates.push((p.to_owned(), m, ext));
            if candidates.len() > 1_000_000 {
                bail!("Scan catalogue budget exceeded");
            }
        }
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    candidates.dedup_by(|a, b| a.0 == b.0);
    let fingerprint = hex::encode(hash.finalize());
    if body["fingerprint"]
        .as_str()
        .is_some_and(|v| v != fingerprint)
    {
        bail!("Source changed between scan pages; previous catalogue retained");
    }
    let mut records = Vec::new();
    let mut used = 0;
    let max_files = body["limits"]["max_files"]
        .as_u64()
        .unwrap_or(5000)
        .clamp(1, 100_000) as usize;
    let max_total = body["limits"]["max_total_bytes"]
        .as_u64()
        .unwrap_or(536870912)
        .clamp(1024, 2147483648);
    let mut source_bytes = 0u64;
    let mut next = offset;
    let count = candidates.len();
    if offset > count {
        bail!("Invalid scan continuation");
    }
    for (p, m, ext) in candidates.into_iter().skip(offset).take(max_files.min(100)) {
        // These are per-page budgets: continue from the next file instead of
        // truncating the scan and destructively pruning the rest of the catalogue.
        if next > offset && source_bytes.saturating_add(m.len()) > max_total {
            break;
        }
        source_bytes = source_bytes.saturating_add(m.len());
        let result = (|| -> Result<Value> {
            let bytes = extract::read_bounded(&p, max)?;
            let extracted = extract::extract(&p, &bytes, max_text, chunk)?;
            let after = fs::metadata(&p)?;
            if m.len() != after.len() || mtime(&m) != mtime(&after) {
                bail!("source_changed_during_extraction");
            }
            Ok(
                json!({"path":p.to_string_lossy(),"name":p.file_name().unwrap_or_default().to_string_lossy(),"extension":ext,"size":bytes.len(),"mtime_ns":mtime(&m),"digest":crate::digest(&bytes),"extraction":extracted}),
            )
        })();
        match result {
            Ok(r) => {
                let size = serde_json::to_vec(&r)?.len();
                if used + size > 1400000 && !records.is_empty() {
                    break;
                }
                used += size;
                records.push(r);
            }
            Err(e) => {
                complete = false;
                if errors.len() < 100 {
                    errors.push(json!({"path":p.to_string_lossy(),"reason":e.to_string()}));
                }
            }
        }
        next += 1;
        if used >= 1400000 {
            break;
        }
    }
    Ok(
        json!({"records":records,"complete":complete,"errors":errors,"fingerprint":fingerprint,"next_offset":if next<count{Some(next)}else{None},"total":count,"source_bytes":source_bytes}),
    )
}
