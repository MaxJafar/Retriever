//! Reviewed, no-overwrite filesystem operations with durable inspection receipts.
use crate::{
    agent,
    config::{Device, atomic_json},
    paths,
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};
fn name(value: &str) -> Result<&str> {
    if value.is_empty()
        || value.len() > 255
        || value.contains(['/', '\\', ':', '\0', '\n', '\r'])
        || value.ends_with(['.', ' '])
        || !paths::safe_name(value)
        || ["CON", "PRN", "AUX", "NUL", "COM1", "LPT1"].contains(
            &value
                .split('.')
                .next()
                .unwrap_or("")
                .to_uppercase()
                .as_str(),
        )
    {
        bail!("Invalid file name");
    }
    Ok(value)
}
pub fn prepare(d: &Device, b: &Value, forbidden: &[PathBuf]) -> Result<Value> {
    let op = b["operation"].as_str().context("Missing operation")?;
    if !["mkdir", "rename", "move", "trash"].contains(&op) {
        bail!("Unsupported reviewed operation");
    }
    let items = b["items"].as_array().context("Missing items")?;
    if items.is_empty() || items.len() > 100 {
        bail!("Review 1–100 items at a time");
    }
    let mut rows = Vec::new();
    let mut sources = HashSet::new();
    let mut destinations = HashSet::new();
    for item in items {
        let (source, parent, pin) = if op == "mkdir" {
            (
                None,
                crate::explorer::checked_operation(
                    d,&b["scope"],
                    item["parent"].as_str().context("Missing parent")?,
                    forbidden,
                    true,
                )?,
                Value::Null,
            )
        } else {
            let source = crate::explorer::checked_operation(
                d,&b["scope"],
                item["path"].as_str().context("Missing source")?,
                forbidden,
                true,
            )?;
            let canonical=source.canonicalize()?;
            if forbidden.iter().any(|f|f.canonicalize().ok().is_some_and(|f|f.starts_with(&canonical))){bail!("Cannot move a folder containing Retriever state");}
            if d.roots.iter().any(|r| {
                paths::normalize(r, d.platform == "windows")
                    == paths::normalize(&source.to_string_lossy(), d.platform == "windows")
            }) {
                bail!("Included roots cannot be moved");
            }
            let pin = agent::pin(&source)?;
            if item.get("pin").is_some_and(|p| p != &pin) {
                bail!("Source changed; refresh before reviewing");
            }
            if op=="trash" && (source.parent().is_none() || crate::paths::crawl_protection(&source.to_string_lossy(),&d.platform).is_some() || crate::paths::expand("~/").canonicalize().ok().as_ref()==source.canonicalize().ok().as_ref()) {bail!("System folders, home and private application data cannot be trashed");}
            let parent = if op == "rename" || op=="trash" {
                source.parent().context("Missing parent")?.to_owned()
            } else {
                crate::explorer::checked_operation(
                    d,&b["scope"],
                    item["parent"].as_str().context("Missing parent")?,
                    forbidden,
                    true,
                )?
            };
            (Some(source), parent, pin)
        };
        if !parent.is_dir() {
            bail!("Destination must be a folder");
        }
        let fallback = source
            .as_ref()
            .and_then(|s| s.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let destination = if op=="trash" {
            let generated=format!(".retriever-trash-{}",uuid::Uuid::new_v4());
            let hidden=item["name"].as_str().unwrap_or(&generated);
            uuid::Uuid::parse_str(hidden.strip_prefix(".retriever-trash-").context("Invalid trash identity")?)?;
            parent.join(hidden)
        }else{parent.join(name(item["name"].as_str().unwrap_or(&fallback))?)};
        crate::explorer::checked_operation(d,&b["scope"], &destination.to_string_lossy(), forbidden, false)?;
        if let Some(source) = &source {
            if source == &destination || destination.starts_with(source) {
                bail!("Choose a different destination outside the source");
            }
            if !sources.insert(source.clone()) {
                bail!("Duplicate source");
            }
        }
        if !destinations.insert(destination.to_string_lossy().to_lowercase()) {
            bail!("Destination names must be unique");
        }
        rows.push(json!({"source":source.map(|p|p.to_string_lossy().into_owned()),"destination":destination.to_string_lossy(),"pin":pin}));
    }
    for row in &rows {
        let destination = Path::new(row["destination"].as_str().unwrap());
        if destination.exists() && !sources.contains(destination) {
            bail!("Destination already exists");
        }
    }
    for a in &sources {
        for b in &sources {
            if a != b && a.starts_with(b) {
                bail!("Select a folder or its children, not both");
            }
        }
    }
    Ok(json!({"operation":op,"scope":b["scope"],"items":rows}))
}
fn journal_path(id: &str) -> Result<PathBuf> {
    uuid::Uuid::parse_str(id).context("Invalid receipt identity")?;
    let dir = std::env::var_os("RETRIEVER_AGENT_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| paths::expand("~/.retriever-agent-candidate"))
        .join("receipts");
    fs::create_dir_all(&dir)?;
    paths::reject_links(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(dir.join(format!("{id}.json")))
}
pub fn inspect(b: &Value) -> Result<Value> {
    let id = b["id"].as_str().context("Missing receipt id")?;
    let path = journal_path(id)?;
    let value:Value=serde_json::from_slice(&fs::read(path)?).unwrap_or(json!({"id":id,"state":"inspection_required","ok":false,"error":"Interrupted receipt creation; inspect source paths before planning any retry"}));
    Ok(value)
}
pub fn execute(d: &Device, b: &Value, forbidden: &[PathBuf]) -> Result<Value> {
    let id = b["id"].as_str().context("Missing operation identity")?;
    let path = journal_path(id)?;
    // Exclusive claim before touching any source. An interrupted/ambiguous claim
    // can only be inspected, never silently replayed after SSH reconnects.
    let claim = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .context("Receipt already exists; inspect it before any retry")?;
    claim.sync_all()?;
    drop(claim);
    let plan = &b["plan"];
    let op = plan["operation"].as_str().context("Missing operation")?;
    let raw = plan["items"].as_array().context("Missing plan items")?;
    let items:Vec<_>=raw.iter().map(|r|json!({"path":r["source"],"parent":Path::new(r["destination"].as_str().unwrap_or("")).parent().map(|p|p.to_string_lossy()),"name":Path::new(r["destination"].as_str().unwrap_or("")).file_name().map(|p|p.to_string_lossy()),"pin":r["pin"]})).collect();
    let checked = prepare(d, &json!({"operation":op,"scope":plan["scope"],"items":items}), forbidden)?;
    if checked["items"] != plan["items"] {
        bail!("Reviewed plan changed");
    }
    let mut receipt = json!({"id":id,"ok":false,"state":"running","operation":op,"items":raw,"stages":[],"created":crate::now()});
    atomic_json(&path, &receipt)?;
    let mut staged: Vec<(PathBuf, PathBuf, PathBuf, Value)> = Vec::new();
    let result = (|| -> Result<()> {
        if op == "mkdir" {
            for (i, row) in raw.iter().enumerate() {
                let destination = Path::new(row["destination"].as_str().unwrap());
                crate::explorer::checked_operation(d,&plan["scope"], &destination.to_string_lossy(), forbidden, false)?;
                receipt["next_action"] = json!({"action":"mkdir","item":i});
                atomic_json(&path, &receipt)?;
                fs::create_dir(destination)?;
                receipt["items"][i]["after"] = agent::pin(destination)?;
                atomic_json(&path, &receipt)?;
            }
        } else {
            for (i, row) in raw.iter().enumerate() {
                let source = PathBuf::from(row["source"].as_str().unwrap());
                let destination = PathBuf::from(row["destination"].as_str().unwrap());
                let temp = source
                    .parent()
                    .unwrap()
                    .join(format!(".retriever-rename-{}", uuid::Uuid::new_v4()));
                crate::explorer::checked_operation(d,&plan["scope"], &source.to_string_lossy(), forbidden, true)?;
                if agent::pin(&source)? != row["pin"] {
                    bail!("Source changed before mutation");
                }
                receipt["next_action"] =
                    json!({"action":"stage","item":i,"temporary":temp.to_string_lossy()});
                atomic_json(&path, &receipt)?;
                exclusive_rename(&source, &temp)?;
                staged.push((source, destination, temp.clone(), row["pin"].clone()));
                receipt["stages"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"item":i,"temporary":temp.to_string_lossy()}));
                atomic_json(&path, &receipt)?;
            }
            for (i, (_, destination, temp, pin)) in staged.iter().enumerate() {
                crate::explorer::checked_operation(d,&plan["scope"], &destination.to_string_lossy(), forbidden, false)?;
                if agent::pin(temp)? != *pin {
                    bail!("Staged content changed");
                }
                receipt["next_action"] = json!({"action":"publish","item":i});
                atomic_json(&path, &receipt)?;
                exclusive_rename(temp, destination)?;
                receipt["items"][i]["after"] = agent::pin(destination)?;
                atomic_json(&path, &receipt)?;
            }
        }
        Ok(())
    })();
    // On failure keep a full receipt and remaining staging paths. No blind
    // rollback can overwrite concurrent edits; inspection is mandatory.
    match result {
        Ok(()) => {
            receipt["ok"] = json!(true);
            receipt["state"] = json!("applied");
            receipt["next_action"] = Value::Null;
        }
        Err(e) => {
            receipt["state"] = json!("inspection_required");
            receipt["error"] = json!(e.to_string());
        }
    }
    atomic_json(&path, &receipt)?;
    Ok(receipt)
}
pub fn exclusive_rename(source: &Path, destination: &Path) -> Result<()> {
    paths::reject_links(source)?;
    paths::reject_links(destination)?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        unsafe extern "system" {
            fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
        }
        let source: Vec<u16> = source.as_os_str().encode_wide().chain([0]).collect();
        let destination: Vec<u16> = destination.as_os_str().encode_wide().chain([0]).collect();
        if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    #[cfg(target_os = "linux")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        unsafe extern "C" {
            fn renameat2(
                olddir: i32,
                old: *const std::ffi::c_char,
                newdir: i32,
                new: *const std::ffi::c_char,
                flags: u32,
            ) -> i32;
        }
        let source = CString::new(source.as_os_str().as_bytes())?;
        let destination = CString::new(destination.as_os_str().as_bytes())?;
        if unsafe { renameat2(-100, source.as_ptr(), -100, destination.as_ptr(), 1) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        unsafe extern "C" {
            fn renamex_np(
                old: *const std::ffi::c_char,
                new: *const std::ffi::c_char,
                flags: u32,
            ) -> i32;
        }
        let source = CString::new(source.as_os_str().as_bytes())?;
        let destination = CString::new(destination.as_os_str().as_bytes())?;
        if unsafe { renamex_np(source.as_ptr(), destination.as_ptr(), 4) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(())
}
