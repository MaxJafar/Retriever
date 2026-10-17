//! Local retrieval orchestration. Model inference never uses a network endpoint.
use crate::{config::validate_model,native_models::NativeModels,scheduler::{Priority,Scheduler},vectors::normalize};
use anyhow::{Context,Result,bail};
use parking_lot::Mutex;
use serde_json::{Value,json};
use std::{collections::{HashMap,VecDeque},path::Path,sync::Arc,time::{Duration,Instant}};

// Instant search already returns words. A full search can wait for the current
// bounded background passage, then take priority over the following passage.
const INTERACTIVE_QUEUE_WAIT:Duration=Duration::from_secs(5);

#[derive(Clone)]
pub struct Providers {
    pub native: NativeModels,
    pub scheduler: Scheduler,
    queries: Arc<Mutex<VecDeque<(String,Vec<f32>)>>>,
    decisions: Arc<Mutex<VecDeque<(String,Vec<Value>)>>>,
    pub health: Arc<Mutex<HashMap<String,Value>>>,
}
pub fn fingerprint(s:&Value)->String {
    // Throughput, deadlines and shortlist choices do not change the vector space.
    crate::digest(json!({"provider":s["provider"],"model":s["model"],"revision":s["model_revision"],
        "dimensions":s["dimensions"],"document_prefix":s["document_prefix"],"query_prefix":s["query_prefix"],
        "runtime":"retriever-native-v1"}).to_string())
}
pub fn probe_key(role:&str,s:&Value)->String{crate::digest(format!("native-v1:{role}:{}",fingerprint(s)))}
pub fn group(_s:&Value)->String{"local-retrieval".into()}
impl Providers {
    pub fn new(state:&Path)->Result<Self>{Ok(Self{native:NativeModels::new(state)?,scheduler:Scheduler::default(),queries:Default::default(),decisions:Default::default(),health:Default::default()})}
    pub fn invalid_response(&self,role:&str,s:&Value){self.health.lock().insert(role.into(),json!({"state":"unavailable","fingerprint":fingerprint(s),"retry_after":crate::now()+60.0}));}
    pub fn retry_ready(&self,role:&str,s:&Value)->bool{self.health.lock().get(role).is_none_or(|h|h["fingerprint"]!=fingerprint(s)||h["retry_after"].as_f64().unwrap_or(0.0)<=crate::now())}
    fn record<T>(&self,role:&str,s:&Value,start:Instant,result:&Result<T>){
        let mut health=self.health.lock();let previous=health.get(role).cloned().unwrap_or(json!({}));
        health.insert(role.into(),json!({"state":if result.is_ok(){"ready"}else{"unavailable"},"fingerprint":fingerprint(s),"requests":previous["requests"].as_u64().unwrap_or(0)+1,"last_request_ms":start.elapsed().as_secs_f64()*1000.0,"retry_after":if result.is_ok(){0.0}else{crate::now()+60.0},"runtime":"in_process","error":result.as_ref().err().map(|e|e.to_string())}));
    }
    pub fn save_credentials(&self,_state:&Path,_settings:&Value,changes:&Value)->Result<()> {
        if changes.as_object().is_some_and(|o|o.values().any(|v|v.as_str().is_some_and(|s|!s.is_empty()))) {bail!("Model API credentials are no longer supported");}Ok(())
    }
    pub async fn embed(&self,s:&Value,inputs:Vec<Value>,priority:Priority,expected:usize)->Result<Vec<Vec<f32>>>{
        validate_model("embedding",s)?;
        let _permit=self.scheduler.acquire(&group(s),priority,if priority==Priority::Interactive{INTERACTIVE_QUEUE_WAIT}else{Duration::from_secs(120)}).await?;
        self.embed_acquired(s,inputs,expected).await
    }
    async fn embed_acquired(&self,s:&Value,inputs:Vec<Value>,expected:usize)->Result<Vec<Vec<f32>>>{
        if s["provider"]!="native"{bail!("Native EmbeddingGemma is required");}
        if !self.retry_ready("embedding",s){bail!("Local embedding runtime is cooling down");}
        let texts=inputs.into_iter().map(|v|v.as_str().map(str::to_owned).context("Native retrieval embeds extracted text only")).collect::<Result<Vec<_>>>()?;
        let start=Instant::now();let result=self.native.embed(texts).await.and_then(|rows|rows.into_iter().map(|r|normalize(&r,expected)).collect());
        self.record("embedding",s,start,&result);result
    }
    pub async fn query(&self,s:&Value,query:&str,expected:usize)->Result<(Vec<f32>,bool)>{
        validate_model("embedding",s)?;
        let key=crate::digest(format!("{}:{expected}:{query}",fingerprint(s)));
        let cached=||self.queries.lock().iter().find(|(k,_)|k==&key).map(|(_,v)|v.clone());
        if let Some(v)=cached(){return Ok((v,true));}
        let _permit=self.scheduler.acquire(&group(s),Priority::Interactive,INTERACTIVE_QUEUE_WAIT).await?;
        if let Some(v)=cached(){return Ok((v,true));}
        let input=format!("{}{query}",s["query_prefix"].as_str().unwrap_or(""));
        let vector=self.embed_acquired(s,vec![json!(input)],expected).await?.remove(0);
        let mut cache=self.queries.lock();cache.push_back((key,vector.clone()));while cache.len()>256{cache.pop_front();}Ok((vector,false))
    }
    pub async fn relevance(&self,s:&Value,query:&str,items:&[Value])->Result<Vec<Value>>{Ok(self.relevance_with_cache(s,query,items).await?.0)}
    pub async fn relevance_with_cache(&self,s:&Value,query:&str,items:&[Value])->Result<(Vec<Value>,bool)>{
        validate_model("relevance",s)?;
        if s["provider"]!="native"{bail!("Native Laya is required");}
        let count=s["candidates"].as_u64().unwrap_or(4).clamp(1,6) as usize;
        let chars=s["max_chars"].as_u64().unwrap_or(900).clamp(32,4000) as usize;
        let bounded=items.iter().take(count).map(|r|json!({"id":r["id"],"name":r["name"].as_str().unwrap_or("").chars().take(256).collect::<String>(),"passage":r["snippet"].as_str().unwrap_or("").chars().take(chars).collect::<String>()})).collect::<Vec<_>>();
        if bounded.is_empty(){return Ok((vec![],true));}
        let key=crate::digest(format!("{}:{}",fingerprint(s),json!({"query":query,"items":bounded})));
        let cached=||self.decisions.lock().iter().find(|(k,_)|k==&key).map(|(_,v)|v.clone());
        if let Some(v)=cached(){return Ok((v,true));}
        let _permit=self.scheduler.acquire(&group(s),Priority::Interactive,INTERACTIVE_QUEUE_WAIT).await?;
        if let Some(v)=cached(){return Ok((v,true));}
        if !self.retry_ready("relevance",s){bail!("Local Laya runtime is cooling down");}
        let start=Instant::now();let result=self.native.relevance(query.into(),bounded.clone()).await.and_then(|v|validate_decisions(&json!(v),&bounded));
        self.record("relevance",s,start,&result);let decisions=result?;
        let mut cache=self.decisions.lock();cache.push_back((key,decisions.clone()));while cache.len()>256{cache.pop_front();}Ok((decisions,false))
    }
    // Retired routes return a clear error for old clients; no generative backend exists.
    pub async fn generate(&self,_role:&str,_s:&Value,_system:&str,_data:Value,_priority:Priority)->Result<Value>{bail!("Generative AI is no longer part of Retriever. Search returns indexed evidence.")}
    pub async fn vision(&self,_s:&Value,_system:&str,_data:Value,_image:&Value,_priority:Priority)->Result<Value>{bail!("Vision generation is no longer part of Retriever")}
}
pub fn bounded_text(text:&str,maximum_bytes:usize)->String{let mut end=text.len().min(maximum_bytes);while !text.is_char_boundary(end){end-=1;}text[..end].to_owned()}
pub fn validate_decisions(raw:&Value,items:&[Value])->Result<Vec<Value>>{
    let arr=raw.as_array().context("Invalid relevance result")?;if arr.len()!=items.len(){bail!("Incomplete relevance decisions");}
    let mut ids=std::collections::HashSet::new();
    for v in arr{let id=v["id"].as_i64().context("Invalid source id")?;if !ids.insert(id)||!items.iter().any(|i|i["id"]==id){bail!("Unexpected relevance id");}
        for k in ["relevance","confidence"]{if !v[k].as_f64().is_some_and(|n|n.is_finite()&&(0.0..=1.0).contains(&n)){bail!("Invalid relevance probability");}}
        if v["truncated"].as_bool().is_none(){bail!("Missing truncation decision");}
    }Ok(arr.clone())
}
