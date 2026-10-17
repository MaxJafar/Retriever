//! Interactive directory metadata is separate from the opt-in AI crawl scope.
//! Never reads a device handle, starts a crawler, or calls a model.
use crate::{agent, config::Device, engine::Engine, paths};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{fs, path::{Component, PathBuf}};

pub fn checked(input: &str, forbidden: &[PathBuf]) -> Result<PathBuf> {
    if input.is_empty() || input.len()>4096 || input.contains('\0') || input.starts_with(r"\\.\") || input.starts_with(r"\\?\") {bail!("Choose a regular filesystem path");}
    let p=paths::expand(input);
    if !p.is_absolute() || p.components().any(|c|matches!(c,Component::ParentDir)){bail!("Choose an absolute path without traversal");}
    // OS permissions apply. Resolve system symlink ancestors on macOS, but do
    // not let a link turn the private application state into a browsable tree.
    let canonical=p.canonicalize()?;
    if forbidden.iter().any(|f|f.canonicalize().ok().is_some_and(|f|canonical.starts_with(f))){bail!("Retriever's private state is not a source folder");}
    Ok(p)
}
pub fn checked_operation(d:&Device,scope:&Value,input:&str,forbidden:&[PathBuf],exists:bool)->Result<PathBuf>{
    if scope!="filesystem" {return paths::checked(d,input,forbidden,exists);}
    if !d.enabled {bail!("Device is disabled");}
    let p=paths::expand(input);paths::reject_links(&p)?;
    if exists {return checked(input,forbidden);}
    if !p.is_absolute() || p.components().any(|c|matches!(c,Component::ParentDir)) || input.starts_with(r"\\.\") || input.starts_with(r"\\?\") {bail!("Invalid destination");}
    checked(&p.parent().context("Missing parent")?.to_string_lossy(),forbidden)?;
    Ok(p)
}
pub fn stat(d:&Device,b:&Value,forbidden:&[PathBuf])->Result<Value>{
    if !d.enabled {bail!("Device is disabled");}
    let p=checked(b["path"].as_str().context("Missing path")?,forbidden)?;let pin=agent::pin(&p)?;
    Ok(json!({"path":p.to_string_lossy(),"name":p.file_name().unwrap_or_default().to_string_lossy(),"kind":pin["kind"],"size":pin["size"],"pin":pin}))
}
pub fn preview(d:&Device,b:&Value,forbidden:&[PathBuf])->Result<Value>{
    let info=stat(d,b,forbidden)?;let path=PathBuf::from(info["path"].as_str().unwrap());
    if info["kind"]=="folder"{return Ok(json!({"text":"Open this folder to browse its contents."}));}
    let bytes=crate::extract::read_bounded(&path,32*1024*1024)?;
    let extracted=crate::extract::extract(&path,&bytes,64000,2000)?;
    if agent::pin(&path)?!=info["pin"]{bail!("File changed during preview");}
    Ok(json!({"text":extracted.text,"detail":extracted.detail,"model_calls":0,"indexed":false}))
}
pub fn media_duration(d:&Device,b:&Value,forbidden:&[PathBuf])->Result<Value>{
    static DECODER:std::sync::LazyLock<parking_lot::Mutex<()>>=std::sync::LazyLock::new(||parking_lot::Mutex::new(()));
    let _permit=DECODER.try_lock().context("Media metadata reader is busy")?;
    let info=stat(d,b,forbidden)?;let path=PathBuf::from(info["path"].as_str().unwrap());
    if crate::extract::kind(&path.extension().unwrap_or_default().to_string_lossy())!=Some("video"){bail!("Length is available for video files");}
    let runtime=tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let seconds=runtime.block_on(crate::media::duration(&path))?;
    if agent::pin(&path)?!=info["pin"]{bail!("File changed during metadata read");}
    Ok(json!({"duration_seconds":seconds,"pin":info["pin"]}))
}
pub fn locations() -> Value {
    let home=paths::expand("~/");let mut entries=vec![json!({"name":"Home","path":home.to_string_lossy(),"kind":"home"})];
    #[cfg(windows)]
    {
        // GetLogicalDrives is a mount-point bitmask, not a storage telemetry call.
        #[link(name="kernel32")] unsafe extern "system" {fn GetLogicalDrives()->u32;}
        let drives=unsafe{GetLogicalDrives()};
        for i in 0..26 {if drives&(1<<i)!=0 {let path=format!("{}:\\",(b'A'+i)as char);entries.push(json!({"name":format!("Drive ({}:)",(b'A'+i)as char),"path":path,"kind":"volume"}));}}
    }
    #[cfg(not(windows))]
    {
        entries.push(json!({"name":"Filesystem","path":"/","kind":"volume"}));
        for mount in ["/Volumes","/media","/mnt"] {if let Ok(children)=fs::read_dir(mount){for item in children.flatten().take(64){if item.path().is_dir(){entries.push(json!({"name":item.file_name().to_string_lossy(),"path":item.path().to_string_lossy(),"kind":"volume"}));}}}}
    }
    json!({"locations":entries,"scope":"interactive_filesystem","crawling_started":false})
}
pub fn list(d:&Device,b:&Value,forbidden:&[PathBuf])->Result<Value>{
    if !d.enabled {bail!("Device is disabled");}
    let path=checked(b["path"].as_str().context("Choose a folder")?,forbidden)?;
    let offset=b["offset"].as_u64().unwrap_or(0)as usize;if offset>1_000_000 {bail!("Invalid page");}
    let key=b["sort"].as_str().unwrap_or("name");if !["name","size","modified","type"].contains(&key){bail!("Invalid sort");}
    let desc=b["direction"]=="desc";let mut entries=Vec::new();let mut skipped=0;
    for entry in fs::read_dir(&path)? {
        let Ok(entry)=entry else{skipped+=1;continue};if entry.file_name().to_string_lossy().starts_with(".retriever-trash-"){continue;}let p=entry.path();
        if checked(&p.to_string_lossy(),forbidden).is_err(){skipped+=1;continue;}
        let Ok(m)=entry.metadata() else{skipped+=1;continue};
        let ext=p.extension().unwrap_or_default().to_string_lossy().to_lowercase();
        entries.push(json!({"device":d.id,"path":p.to_string_lossy(),"name":entry.file_name().to_string_lossy(),"kind":if m.is_dir(){"folder"}else{"file"},"type":crate::extract::kind(&ext).unwrap_or("other"),"extension":ext,"size":if m.is_dir(){0}else{m.len()},"modified":agent::mtime(&m)as f64/1e9,"indexed_scope":paths::eligible(d,&p.to_string_lossy()),"metadata":{}}));
        if entries.len()>100_000{bail!("Folder exceeds the interactive listing limit (100,000 entries)");}
    }
    entries.sort_by(|a,b|{let folder=(b["kind"]=="folder").cmp(&(a["kind"]=="folder"));if !folder.is_eq(){return folder};let order=if key=="size"||key=="modified"{a[key].as_f64().unwrap_or(0.).total_cmp(&b[key].as_f64().unwrap_or(0.))}else{a[key].as_str().unwrap_or("").to_lowercase().cmp(&b[key].as_str().unwrap_or("").to_lowercase())};(if desc{order.reverse()}else{order}).then_with(||a["path"].as_str().cmp(&b["path"].as_str()))});
    let total=entries.len();Ok(json!({"path":path.to_string_lossy(),"parent":path.parent().map(|p|p.to_string_lossy()),"entries":entries.into_iter().skip(offset).take(200).collect::<Vec<_>>(),"total":total,"offset":offset,"next_offset":if offset+200<total{Some(offset+200)}else{None},"skipped":skipped,"cached":false,"crawling_started":false}))
}
/// One level only. No file contents, recursion, health calls or crawl side effects.
pub fn children(d:&Device,b:&Value,forbidden:&[PathBuf])->Result<Value>{
    if !d.enabled {bail!("Device is disabled");}
    let path=checked(b["path"].as_str().context("Choose a folder")?,forbidden)?;
    paths::reject_links(&path)?;
    let offset=b["offset"].as_u64().unwrap_or(0) as usize;
    if offset>100_000 {bail!("Invalid folder page");}
    let mut entries=Vec::new();let mut skipped=0;let started=std::time::Instant::now();
    for (seen,entry) in fs::read_dir(&path)?.enumerate(){
        if seen>=100_000 || started.elapsed().as_secs()>=5 {bail!("This folder is too large for the sidebar. Open it in a tab instead.");}
        let Ok(entry)=entry else {skipped+=1;continue};if entry.file_name().to_string_lossy().starts_with(".retriever-trash-"){continue;}
        let Ok(kind)=entry.file_type() else {skipped+=1;continue};
        if !kind.is_dir() || kind.is_symlink() {continue;}
        let p=entry.path();
        if paths::reject_links(&p).is_err() || checked(&p.to_string_lossy(),forbidden).is_err(){skipped+=1;continue;}
        entries.push(json!({"path":p.to_string_lossy(),"name":entry.file_name().to_string_lossy(),"kind":"folder"}));
    }
    entries.sort_by(|a,b|a["name"].as_str().unwrap_or("").to_lowercase().cmp(&b["name"].as_str().unwrap_or("").to_lowercase()).then_with(||a["path"].as_str().cmp(&b["path"].as_str())));
    let total=entries.len();
    Ok(json!({"entries":entries.into_iter().skip(offset).take(100).collect::<Vec<_>>(),"total":total,"next_offset":if offset+100<total {Some(offset+100)} else {None},"skipped":skipped,"model_calls":0,"crawling_started":false}))
}
impl Engine {
    pub async fn explorer_children(&self,b:&Value)->Result<Value>{let cfg=self.config.read().clone();let d=cfg.device(b["device"].as_str().context("Missing device")?)?;self.source(d,"explorer_children",b.clone()).await}
    pub async fn file_duration(&self,b:&Value)->Result<Value>{
        use rusqlite::{OptionalExtension,params};
        let cfg=self.config.read().clone();let d=cfg.device(b["device"].as_str().context("Missing device")?)?;let path=b["path"].as_str().context("Missing path")?;
        let stat=self.source(d,"explorer_stat",json!({"path":path})).await?;
        let cached:Option<f64>=self.store.db.lock().query_row("SELECT duration_seconds FROM media_properties WHERE device=? AND path=? AND size=? AND mtime_ns=?",params![d.id,path,stat["pin"]["size"].as_i64(),stat["pin"]["mtime_ns"].as_i64()],|r|r.get(0)).optional()?;
        if let Some(seconds)=cached{return Ok(json!({"duration_seconds":seconds,"cached":true}));}
        let result=self.source(d,"media_duration",json!({"path":path})).await?;
        if result["pin"]!=stat["pin"]{bail!("File changed during metadata read");}
        let seconds=result["duration_seconds"].as_f64().filter(|v|v.is_finite()&&*v>0.).context("Invalid duration")?;
        self.store.db.lock().execute("INSERT INTO media_properties VALUES(?,?,?,?,?,?) ON CONFLICT(device,path) DO UPDATE SET size=excluded.size,mtime_ns=excluded.mtime_ns,duration_seconds=excluded.duration_seconds,checked_at=excluded.checked_at",params![d.id,path,stat["pin"]["size"].as_i64(),stat["pin"]["mtime_ns"].as_i64(),seconds,crate::now()])?;
        Ok(json!({"duration_seconds":seconds,"cached":false}))
    }
    pub async fn explorer_preview(&self,b:&Value)->Result<Value>{let cfg=self.config.read().clone();let d=cfg.device(b["device"].as_str().context("Missing device")?)?;self.source(d,"explorer_preview",b.clone()).await}
    pub async fn explorer_locations(&self,b:&Value)->Result<Value>{let cfg=self.config.read().clone();let d=cfg.device(b["device"].as_str().context("Missing device")?)?;self.source(d,"explorer_locations",json!({})).await}
    pub async fn explorer_browse(&self,b:&Value)->Result<Value>{
        let cfg=self.config.read().clone();let d=cfg.device(b["device"].as_str().context("Missing device")?)?;
        let mut result=self.source(d,"explorer_list",b.clone()).await?;
        if let Some(rows)=result["entries"].as_array_mut(){for row in rows {row["metadata"]=self.metadata_scoped(&d.id,row["path"].as_str().unwrap_or(""),true)?;self.attach_duration(row)?;}}
        Ok(result)
    }
    pub fn attach_duration(&self,row:&mut Value)->Result<()> {
        use rusqlite::{OptionalExtension,params};
        let duration:Option<f64>=self.store.db.lock().query_row("SELECT COALESCE((SELECT m.duration_seconds FROM media_properties m WHERE m.device=f.device AND m.path=f.path AND m.size=f.size AND m.mtime_ns=f.mtime_ns),json_extract(a.body,'$.sampling.duration_seconds')) FROM files f LEFT JOIN file_analysis a ON a.file_id=f.id WHERE f.device=? AND f.path=?",params![row["device"].as_str().unwrap_or(""),row["path"].as_str().unwrap_or("")],|r|r.get::<_,Option<f64>>(0)).optional()?.flatten();
        row["duration_seconds"]=duration.filter(|v|v.is_finite()&&*v>0.).map_or(Value::Null,|v|json!(v));Ok(())
    }
}
