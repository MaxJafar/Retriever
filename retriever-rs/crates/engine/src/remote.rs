//! SSH uses the existing framed agent protocol. No hardware-health action is
//! forwarded. Storage snapshots read directory metadata with explicit budgets.
use crate::{config::Device,engine::Engine};
use anyhow::{Context,Result,bail};
use serde_json::{json,Value};
use std::{path::PathBuf,time::{Instant,Duration}};

pub fn storage_snapshot(d:&Device,b:&Value,forbidden:&[PathBuf])->Result<Value>{
    static SCANNER:std::sync::LazyLock<parking_lot::Mutex<()>>=std::sync::LazyLock::new(||parking_lot::Mutex::new(()));
    let _permit=SCANNER.try_lock().context("A storage snapshot is already running; try again when it finishes")?;
    if !d.enabled {bail!("Device is disabled");}
    let root=crate::explorer::checked(b["path"].as_str().context("Choose a folder")?,forbidden)?;
    if !root.is_dir(){bail!("Choose a directory");}
    let started=Instant::now();let mut entries=0u64;let mut files=0u64;let mut bytes=0u64;let mut skipped=0u64;let mut largest=Vec::new();let mut complete=true;
    let mut excluded=0u64;
    for entry in walkdir::WalkDir::new(&root).follow_links(false).max_depth(128).into_iter().filter_entry(|e|{let allowed=!e.file_type().is_symlink()&&crate::explorer::checked(&e.path().to_string_lossy(),forbidden).is_ok();if !allowed{excluded+=1;}allowed}){
        if entries>=50_000||started.elapsed()>Duration::from_secs(20){complete=false;break;}
        let entry=match entry {Ok(e)=>e,Err(_)=>{skipped+=1;complete=false;continue}};entries+=1;
        if entry.depth()==128&&entry.file_type().is_dir(){skipped+=1;complete=false;}
        if !entry.file_type().is_file(){continue;}let m=match entry.metadata(){Ok(m)=>m,Err(_)=>{skipped+=1;complete=false;continue}};files+=1;bytes=bytes.saturating_add(m.len());
        largest.push(json!({"path":entry.path().to_string_lossy(),"name":entry.file_name().to_string_lossy(),"logical":m.len()}));largest.sort_by_key(|r|std::cmp::Reverse(r["logical"].as_u64().unwrap_or(0)));largest.truncate(30);
    }
    skipped+=excluded;complete&=excluded==0;
    Ok(json!({"device":d.id,"path":root.to_string_lossy(),"checked_at":crate::now(),"files":files,"entries":entries,"logical_bytes":bytes,"skipped":skipped,"complete":complete,"duration_ms":started.elapsed().as_millis(),"largest":largest,"basis":"logical file bytes; hard links counted per path","health_collection":"blocked","model_calls":0,"limits":{"entries":50000,"seconds":20,"depth":128}}))
}
impl Engine {
    pub async fn remote_storage(&self,b:&Value)->Result<Value>{let cfg=self.config.read().clone();let d=cfg.device(b["device"].as_str().context("Missing device")?)?;self.source(d,"storage_snapshot",b.clone()).await}
}
