//! Scope review only: inspect the selected directory, never enumerate or read its files.
use crate::{config::Device, engine::Engine, explorer, paths};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::PathBuf;

pub fn preview(d:&Device,b:&Value,forbidden:&[PathBuf])->Result<Value>{
    if !d.enabled {bail!("Device is disabled");}
    let input=b["root"].as_str().context("Choose a crawl folder or drive")?;
    if let Some(reason)=paths::crawl_protection(input,&d.platform){bail!("{reason}");}
    let path=explorer::checked(input,forbidden)?;
    paths::reject_links(&path)?;
    if !path.is_dir(){bail!("Choose a folder or drive");}
    let root=path.to_string_lossy().into_owned();
    let mut proposed=d.clone();proposed.roots=vec![root.clone()];
    paths::checked(&proposed,&root,forbidden,true)?;
    Ok(json!({"root":root,"device":d.id,"whole_volume":path.parent().is_none(),
        "source_access":"read_only","crawling_started":false,"model_calls":0,"files_read":0,
        "protections":["Operating system, application and recovery folders","Application data, caches and hidden folders","Credentials and private keys","Symbolic links, junctions and cloud placeholders","Retriever's private database and archive"],
        "excluded":d.exclude,"initial_analysis":"off","initial_schedule":"manual",
        "coverage":"Supported documents, images and videos inside the chosen scope. Unreadable and excluded paths are skipped. A whole-drive crawl does not mean every system file is indexed."}))
}
impl Engine {
    pub async fn crawl_preview(&self,b:&Value)->Result<Value>{
        let cfg=self.config.read().clone();
        let d=cfg.device(b["device"].as_str().context("Missing device")?)?;
        let mut result=self.source(d,"crawl_preview",b.clone()).await?;
        result["revision"]=json!(cfg.revision());result["limits"]=cfg.limits.clone();
        result["archive_path"]=json!(self.state.join("rag-archive").to_string_lossy());
        Ok(result)
    }
}
