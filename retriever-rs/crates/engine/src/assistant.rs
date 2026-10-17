//! Read-only engine observability and a bounded derived catalogue archive.
use crate::engine::Engine;
use anyhow::Result;
use serde_json::{json,Value};
use std::fs;

impl Engine {
    pub fn assistant_status(&self)->Result<Value>{
        let status=self.status()?;let crawl=self.crawl_status()?;
        let db=self.store.db.lock();
        let count=|table:&str|->Result<i64>{Ok(db.query_row(&format!("SELECT count(*) FROM {table}"),[],|r|r.get(0))?)};
        let mut result=json!({"files":count("files")?,"passages":count("passages")?,"vectors":count("vectors")?,"descriptions":count("file_analysis")?,"folders":crawl["folders"],"running":crawl["running"],"routing":status["routing"],"uptime_seconds":self.started.elapsed().as_secs(),"archive_path":self.state.join("rag-archive").to_string_lossy(),"index_path":self.state.join("retriever.sqlite").to_string_lossy(),"health_collection":"blocked","storage":status["storage"],"embedding_cache_entries":count("embedding_cache")?,"models":[]});
        for role in ["embedding","relevance"]{let mut model=status[role].clone();model["role"]=json!(role);model["uptime_seconds"]=Value::Null;result["models"].as_array_mut().unwrap().push(model);}
        let path=self.state.join("rag-archive/catalogue.json");result["archive_updated"]=fs::metadata(path).ok().and_then(|m|m.modified().ok()).and_then(|t|t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(Value::Null,|t|json!(t.as_secs_f64()));Ok(result)
    }
    pub fn archive_catalogue(&self)->Result<Value>{
        let cfg=self.config.read().clone();let db=self.store.db.lock();
        let mut statement=db.prepare("SELECT f.id,f.device,f.path,f.name,f.digest,f.indexed_at,(SELECT count(*) FROM passages p WHERE p.file_id=f.id),a.updated_at FROM files f LEFT JOIN file_analysis a ON a.file_id=f.id ORDER BY f.id")?;
        let mut cursor=statement.query([])?;let mut files=Vec::new();let mut truncated=false;
        while let Some(r)=cursor.next()?{let device:String=r.get(1)?;let path:String=r.get(2)?;if !cfg.devices.iter().any(|d|d.id==device&&crate::paths::eligible(d,&path)){continue;}
            files.push(json!({"id":r.get::<_,i64>(0)?,"device":device,"path":path,"name":r.get::<_,String>(3)?,"source_digest":r.get::<_,String>(4)?,"indexed_at":r.get::<_,f64>(5)?,"passages":r.get::<_,i64>(6)?,"description_updated":r.get::<_,Option<f64>>(7)?}));
            if files.len()>=100_000 {truncated=true;break;}
        }
        drop(cursor);drop(statement);drop(db);
        let dir=self.state.join("rag-archive");crate::paths::reject_links(&dir)?;fs::create_dir_all(&dir)?;
        let archive=json!({"version":1,"generated_at":crate::now(),"kind":"derived_catalogue","source":"retriever.sqlite","contains_source_text":false,"files":files,"limit":100000,"truncated":truncated});
        crate::config::atomic_json(&dir.join("catalogue.json"),&archive)?;Ok(json!({"path":dir.to_string_lossy(),"files":archive["files"].as_array().unwrap().len()}))
    }
}
