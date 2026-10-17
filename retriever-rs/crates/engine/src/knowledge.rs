//! Compatibility for saved, digest-bound descriptions. No generation runtime.
use crate::engine::Engine;
use anyhow::{Context, Result, bail};
use rusqlite::params;
use serde_json::{Value,json};
impl Engine {
    pub(crate) fn annotation_current(&self,id:i64,coordinates:&Value)->bool {
        self.store.db.lock().query_row("SELECT EXISTS(SELECT 1 FROM file_analysis WHERE file_id=? AND signature=?)",params![id,coordinates["annotation_signature"].as_str().unwrap_or("")],|r|r.get(0)).unwrap_or(false)
    }
    pub fn reconcile_knowledge(&self)->Result<()> {
        // Stop retired per-folder generation settings, retaining saved metadata.
        self.store.db.lock().execute("UPDATE crawl_folders SET analysis='off',revision=revision+1 WHERE analysis<>'off'",[])?;
        Ok(())
    }
    pub fn knowledge_pending(&self)->Result<usize>{Ok(0)}
    pub async fn enrich_pending(&self,_job:&str)->Result<Value>{Ok(json!({"completed":0,"deferred":0,"more_pending":false}))}
}
pub fn validate_annotation(answer: &Value, evidence: &[Value]) -> Result<()> {
    for (key, max) in [("summary", 1200), ("category", 80)] {
        if !answer[key]
            .as_str()
            .is_some_and(|s| !s.trim().is_empty() && s.chars().count() <= max)
        {
            bail!("Invalid analysis {key}");
        }
    }
    let tags = answer["tags"].as_array().context("Invalid analysis tags")?;
    if tags.len() > 8
        || tags.iter().any(|t| {
            !t.as_str()
                .is_some_and(|s| !s.trim().is_empty() && s.chars().count() <= 48)
        })
    {
        bail!("Invalid analysis tags");
    }
    let citations = answer["citations"]
        .as_array()
        .context("Missing analysis evidence")?;
    if citations.is_empty()
        || citations.len() > 3
        || citations
            .iter()
            .any(|id| !id.is_string() || !evidence.iter().any(|e| &e["id"] == id))
    {
        bail!("Invalid analysis citation");
    }
    if crate::paths::secret_content(answer.to_string().as_bytes()) {
        bail!("secret_content");
    }
    Ok(())
}
