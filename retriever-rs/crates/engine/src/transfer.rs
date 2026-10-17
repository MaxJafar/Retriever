use crate::{
    agent,
    config::{Device, atomic_json},
    paths,
};
use anyhow::{Context, Result, bail};
use base64::Engine as _;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
const CHUNK: usize = 512 * 1024;
pub const MAX_FILE_SIZE: u64 = 1024 * 1024 * 1024 * 1024;
struct TransferLock(PathBuf);
impl Drop for TransferLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn journal(id: &str) -> Result<PathBuf> {
    uuid::Uuid::parse_str(id)?;
    let root = std::env::var_os("RETRIEVER_AGENT_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| paths::expand("~/.retriever-agent-candidate"))
        .join("transfers");
    fs::create_dir_all(&root)?;
    paths::reject_links(&root)?;
    Ok(root.join(format!("{id}.json")))
}
fn hash(path: &Path) -> Result<String> {
    let mut f = fs::File::open(path)?;
    let mut sha = Sha256::new();
    let mut buffer = vec![0; CHUNK];
    loop {
        let n = f.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        sha.update(&buffer[..n]);
    }
    Ok(hex::encode(sha.finalize()))
}
fn screened_hash(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut sha = Sha256::new();
    let mut tail = Vec::new();
    let mut bytes = vec![0; crate::transfer_codec::BLOCK];
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        sha.update(&bytes[..n]);
        tail.extend_from_slice(&bytes[..n]);
        if paths::secret_content(&tail) {
            bail!("secret_content");
        }
        tail = tail[tail.len().saturating_sub(512)..].to_vec();
    }
    Ok(hex::encode(sha.finalize()))
}

fn local_copy(d: &Device, b: &Value, forbidden: &[PathBuf], receipt: &Path) -> Result<Value> {
    let id = b["id"].as_str().context("Missing transfer identity")?;
    let source = crate::explorer::checked_operation(
        d,
        &b["scope"],
        b["source"].as_str().context("Missing source")?,
        forbidden,
        true,
    )?;
    let destination = crate::explorer::checked_operation(
        d,
        &b["scope"],
        b["path"].as_str().context("Missing destination")?,
        forbidden,
        false,
    )?;
    let pin = agent::pin(&source)?;
    if pin != b["pin"] || pin["kind"] != "file" {
        bail!("Source changed since review");
    }
    if pin["size"].as_u64().unwrap_or(u64::MAX) > MAX_FILE_SIZE {
        bail!("Copy exceeds file budget");
    }
    if destination.exists() {
        bail!("Destination already exists");
    }
    let temp = destination
        .parent()
        .context("Missing parent")?
        .join(format!(".retriever-transfer-{id}"));
    let claim = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(receipt)?;
    claim.sync_all()?;
    drop(claim);
    let mut value = json!({"id":id,"device":d.id,"path":destination,"temporary":temp,"size":pin["size"],"offset":0,
        "scope":b["scope"],"created":crate::now(),"state":"copying","method":"native","metadata":"native"});
    atomic_json(receipt, &value)?;
    let expected = screened_hash(&source)?;
    if agent::pin(&source)? != pin {
        bail!("Source changed during verification");
    }
    crate::native_copy::copy(&source, &temp)?;
    if agent::pin(&source)? != pin || hash(&temp)? != expected {
        bail!("Source changed or copied checksum mismatch; inspect receipt");
    }
    flush_copy(&temp)?;
    value["offset"] = pin["size"].clone();
    value["sha256"] = json!(expected);
    value["state"] = json!("publishing");
    atomic_json(receipt, &value)?;
    crate::operations::exclusive_rename(&temp, &destination)?;
    value["state"] = json!("applied");
    value["pin"] = agent::pin(&destination)?;
    atomic_json(receipt, &value)?;
    Ok(value)
}
fn flush_copy(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        let original = fs::metadata(path)?.permissions();
        if original.readonly() {
            let mut writable = original.clone();
            writable.set_readonly(false);
            fs::set_permissions(path, writable)?;
        }
        let result = fs::OpenOptions::new()
            .write(true)
            .open(path)
            .and_then(|file| file.sync_all());
        let restored = fs::set_permissions(path, original);
        result?;
        restored?;
    }
    #[cfg(not(windows))]
    {
        fs::File::open(path)?.sync_all()?;
    }
    Ok(())
}
pub fn dispatch(d: &Device, action: &str, b: &Value, forbidden: &[PathBuf]) -> Result<Value> {
    if action == "transfer_read" || action == "transfer_read_v2" {
        let v2 = action == "transfer_read_v2";
        let budget = if v2 {
            crate::transfer_codec::BLOCK
        } else {
            CHUNK
        };
        let p = crate::explorer::checked_operation(
            d,
            &b["scope"],
            b["path"].as_str().context("Missing source")?,
            forbidden,
            true,
        )?;
        let pin = agent::pin(&p)?;
        if pin != b["pin"] || pin["kind"] != "file" {
            bail!("Source changed since review");
        }
        let offset = b["offset"].as_u64().context("Missing offset")?;
        let length = b["length"]
            .as_u64()
            .unwrap_or(budget as u64)
            .min(budget as u64);
        let size = pin["size"].as_u64().unwrap();
        if offset > size {
            bail!("Invalid read offset");
        }
        let start = offset.saturating_sub(512);
        let mut f = fs::File::open(&p)?;
        f.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        f.take(length + offset - start + 512)
            .read_to_end(&mut bytes)?;
        if paths::secret_content(&bytes) {
            bail!("secret_content");
        }
        let begin = (offset - start) as usize;
        let end = bytes.len().min(begin + length as usize);
        if agent::pin(&p)? != pin {
            bail!("Source changed during transfer");
        }
        if v2 {
            let mut result = crate::transfer_codec::encode(&bytes[begin..end])?;
            result["pin"] = pin;
            result["offset"] = json!(offset);
            result["eof"] = json!(offset + (end - begin) as u64 >= size);
            return Ok(result);
        }
        return Ok(
            json!({"pin":pin,"offset":offset,"data":base64::engine::general_purpose::STANDARD.encode(&bytes[begin..end]),"eof":offset+(end-begin)as u64>=size}),
        );
    }
    let id = b["id"].as_str().context("Missing transfer identity")?;
    let receipt = journal(id)?;
    let _lock = if action != "transfer_inspect" {
        let path = receipt.with_extension("lock");
        let file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .context("Transfer is active or interrupted; inspect before retrying")?;
        file.sync_all()?;
        Some(TransferLock(path))
    } else {
        None
    };
    if action == "transfer_local_copy" {
        return local_copy(d, b, forbidden, &receipt);
    }
    if action == "transfer_begin" {
        let destination = crate::explorer::checked_operation(
            d,
            &b["scope"],
            b["path"].as_str().context("Missing destination")?,
            forbidden,
            false,
        )?;
        if destination.exists() {
            bail!("Destination already exists");
        }
        let size = b["size"].as_u64().context("Missing size")?;
        if size > MAX_FILE_SIZE {
            bail!("Copy exceeds file budget");
        }
        let temp = destination
            .parent()
            .unwrap()
            .join(format!(".retriever-transfer-{id}"));
        let claim = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&receipt)
            .context("Transfer exists; inspect it before retrying")?;
        claim.sync_all()?;
        // Windows denies atomic replacement while the exclusive claim handle is open.
        drop(claim);
        let value = json!({"id":id,"device":d.id,"path":destination.to_string_lossy(),"temporary":temp.to_string_lossy(),"size":size,"offset":0,"state":"receiving","scope":b["scope"],"created":crate::now(),"mtime_ns":b["mtime_ns"],"wire_bytes":0,"compressed_blocks":0});
        atomic_json(&receipt, &value)?;
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(temp)?
            .sync_all()?;
        return Ok(value);
    }
    let mut value: Value = serde_json::from_slice(&fs::read(&receipt)?)
        .context("Interrupted transfer receipt; manual inspection required")?;
    if value["device"] != d.id {
        bail!("Transfer belongs to a different source");
    }
    if action == "transfer_inspect" {
        return Ok(value);
    }
    if value["state"] != "receiving"
        || crate::now()
            - value["updated"]
                .as_f64()
                .or_else(|| value["created"].as_f64())
                .unwrap_or(0.0)
            > 3600.0
    {
        bail!("Transfer cannot be replayed; inspect receipt");
    }
    let destination = crate::explorer::checked_operation(
        d,
        &value["scope"],
        value["path"].as_str().unwrap(),
        forbidden,
        false,
    )?;
    let temp_path = PathBuf::from(
        value["temporary"]
            .as_str()
            .context("Missing temporary path")?,
    );
    let temp = temp_path.as_path();
    paths::reject_links(temp)?;
    if temp.parent() != destination.parent()
        || temp.file_name().unwrap_or_default() != format!(".retriever-transfer-{id}").as_str()
    {
        bail!("Invalid transfer staging path");
    }
    let expected = value["offset"].as_u64().unwrap_or(0);
    if fs::metadata(temp)?.len() != expected {
        bail!("Interrupted write; inspect receipt before any retry");
    }
    if action == "transfer_write" || action == "transfer_write_v2" {
        if b["offset"].as_u64() != Some(expected) {
            bail!("Transfer offset mismatch; inspect receipt");
        }
        let v2 = action == "transfer_write_v2";
        let bytes = if v2 {
            crate::transfer_codec::decode(b)?
        } else {
            base64::engine::general_purpose::STANDARD
                .decode(b["data"].as_str().context("Missing data")?)?
        };
        if bytes.len()
            > if v2 {
                crate::transfer_codec::BLOCK
            } else {
                CHUNK
            }
            || expected + bytes.len() as u64 > value["size"].as_u64().unwrap()
        {
            bail!("Transfer chunk exceeds budget");
        }
        // Screen across chunk boundaries as well as inside each frame.
        let mut file = fs::OpenOptions::new().read(true).append(true).open(temp)?;
        file.seek(SeekFrom::Start(expected.saturating_sub(512)))?;
        let mut context = Vec::new();
        (&mut file).take(512).read_to_end(&mut context)?;
        context.extend_from_slice(&bytes);
        if paths::secret_content(&context) {
            bail!("secret_content");
        }
        file.write_all(&bytes)?;
        file.sync_data()?;
        value["offset"] = json!(expected + bytes.len() as u64);
        value["updated"] = json!(crate::now());
        value["wire_bytes"] = json!(
            value["wire_bytes"].as_u64().unwrap_or(0)
                + b["data"].as_str().unwrap_or("").len() as u64
        );
        if b["encoding"] == "lz4-block" {
            value["compressed_blocks"] =
                json!(value["compressed_blocks"].as_u64().unwrap_or(0) + 1);
        }
        atomic_json(&receipt, &value)?;
        return Ok(json!({"id":id,"offset":value["offset"],"state":"receiving"}));
    }
    if action == "transfer_finish" {
        if expected != value["size"].as_u64().unwrap() {
            bail!("Incomplete copy");
        }
        let actual = hash(temp)?;
        if b["sha256"].as_str() != Some(&actual) {
            bail!("Checksum mismatch; destination was not published");
        }
        if let Some(ns) = value["mtime_ns"].as_u64() {
            let file = fs::OpenOptions::new().write(true).open(temp)?;
            file.set_times(
                fs::FileTimes::new()
                    .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_nanos(ns)),
            )?;
            file.sync_all()?;
            value["metadata"] = json!("modified_time");
        }
        value["state"] = json!("publishing");
        value["sha256"] = json!(actual);
        atomic_json(&receipt, &value)?;
        crate::operations::exclusive_rename(temp, &destination)?;
        value["state"] = json!("applied");
        value["pin"] = agent::pin(&destination)?;
        atomic_json(&receipt, &value)?;
        return Ok(value);
    }
    bail!("Unknown transfer action")
}
