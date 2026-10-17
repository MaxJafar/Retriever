use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub label: String,
    pub transport: String,
    pub platform: String,
    pub enabled: bool,
    pub roots: Vec<String>,
    pub exclude: Vec<String>,
    #[serde(default)]
    pub analysis_roots: Vec<String>,
    #[serde(default)]
    pub ssh: String,
    #[serde(default = "default_agent")]
    pub agent: String,
}
fn default_agent() -> String {
    "retriever-agent".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub schema_version: u32,
    pub configured: bool,
    pub role: String,
    pub endpoint: String,
    pub ssh_config: String,
    pub devices: Vec<Device>,
    pub limits: Value,
    pub search: Value,
    #[serde(default)]
    pub appearance: Value,
}
impl Default for Config {
    fn default() -> Self {
        let embedding = json!({"provider":"native","model":"embeddinggemma-2","model_revision":"unprovisioned",
            "dimensions":768,"batch_size":1,"chunk_chars":1600,"context_tokens":2048,"document_prefix":"title: none | text: ","query_prefix":"task: search result | query: ","modalities":["text"]});
        let relevance = json!({"provider":"native","model":"laya-multilingual","model_revision":"unprovisioned",
            "candidates":4,"weight":0.25,"min_confidence":0.7,"max_chars":900,"context_tokens":1024});
        let model = json!({"provider":"disabled"});
        Self {
            schema_version: 2,
            configured: false,
            role: "coordinator".into(),
            endpoint: "http://127.0.0.1:18780".into(),
            ssh_config: String::new(),
            devices: vec![Device {
                id: "local".into(),
                label: "This computer".into(),
                transport: "local".into(),
                platform: if cfg!(windows) {
                    "windows"
                } else if cfg!(target_os = "macos") {
                    "macos"
                } else {
                    "linux"
                }
                .into(),
                enabled: true,
                roots: vec![],
                exclude: vec![],
                analysis_roots: vec![],
                ssh: String::new(),
                agent: default_agent(),
            }],
            limits: json!({"max_file_bytes":134217728,"max_media_bytes":2147483648u64,"max_total_bytes":536870912,"max_files":5000,"max_text_chars":1048576,"ssh_timeout":120}),
            search: json!({"limit":30,"embedding":embedding,"relevance":relevance,"vision":model,"llm":model}),
            appearance: json!({"always_on_top":false}),
        }
    }
}
impl Config {
    pub fn load(state: &Path) -> Result<Self> {
        let file = state.join("config.json");
        let mut config: Self = if file.exists() {
            serde_json::from_slice(&fs::read(file)?)?
        } else {
            Self::default()
        };
        // Upgrade old provider selections in memory. Preserve folders, metadata and the
        // original on-disk config until a normal settings save; never contact old APIs.
        let defaults=Self::default();
        for role in ["embedding","relevance"] {
            if !matches!(config.search[role]["provider"].as_str(),Some("native"|"disabled")) || config.schema_version<2 {config.search[role]=defaults.search[role].clone();}
        }
        for role in ["vision","llm"] {config.search[role]=json!({"provider":"disabled"});}
        for d in &mut config.devices { d.analysis_roots.clear(); }
        config.schema_version=2;
        Self::validate(&config)?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        if !["coordinator", "standalone", "client"].contains(&self.role.as_str())
            || self.devices.len() > 100
        {
            bail!("Invalid application role or device count");
        }
        let url = reqwest::Url::parse(&self.endpoint)?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !(url.scheme() == "https"
                || (url.scheme() == "http"
                    && ["127.0.0.1", "localhost", "[::1]", "::1"]
                        .contains(&url.host_str().unwrap_or(""))))
        {
            bail!("Coordinator requires HTTPS or loopback HTTP");
        }
        let mut ids = HashSet::new();
        let mut local = 0;
        for d in &self.devices {
            if d.id.is_empty()
                || d.id.len() > 64
                || !d
                    .id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
                || !ids.insert(&d.id)
            {
                bail!("Invalid or duplicate device identity");
            }
            if !["local", "ssh"].contains(&d.transport.as_str())
                || !["windows", "macos", "linux", "auto"].contains(&d.platform.as_str())
            {
                bail!("Invalid device connection");
            }
            if d.transport == "local" {
                local += 1;
            }
            if d.transport == "ssh"
                && (d.ssh.is_empty()
                    || d.ssh.starts_with('-')
                    || !d
                        .ssh
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)))
            {
                bail!("SSH requires a concrete saved alias");
            }
            if d.agent != "retriever-agent"
                && (!(d.agent.starts_with('/') || (d.platform=="windows" && d.agent.len()>3 && d.agent.as_bytes()[0].is_ascii_alphabetic() && d.agent.as_bytes().get(1..3)==Some(b":/")))
                    || d.agent.chars().any(char::is_control)
                    || d.agent
                        .contains(['\'', '"', '\n', '\r', '\0', ';', '$', '`', '&', '|', '<', '>', '^', '%', '!', '(', ')', '\\']))
            {
                bail!("Agent must be retriever-agent or an absolute path using forward slashes without shell operators");
            }
            if d.roots.len() > 100
                || d.exclude.len() > 100
                || d.roots
                    .iter()
                    .any(|r| !crate::paths::absolute(r) || r.len() > 4096)
                || d.analysis_roots.iter().any(|r| !d.roots.contains(r))
            {
                bail!("Invalid included or analysis folders");
            }
        }
        if local > 1 {
            bail!("Only one device can use the local filesystem");
        }
        for role in ["embedding", "relevance", "vision", "llm"] {
            validate_model(role, &self.search[role])?;
        }
        for (field, lo, hi) in [
            ("max_file_bytes", 1024, 268435456),
            ("max_total_bytes", 1024, 2147483648),
            ("max_files", 1, 100000),
            ("max_text_chars", 256, 16000000),
            ("ssh_timeout", 1, 300),
        ] {
            if !self.limits[field]
                .as_u64()
                .is_some_and(|v| (lo..=hi).contains(&v))
            {
                bail!("Invalid scan budget: {field}");
            }
        }
        if let Some(value) = self.limits.get("max_media_bytes") {
            if !value
                .as_u64()
                .is_some_and(|v| (1024..=8589934592).contains(&v))
            {
                bail!("Invalid scan budget: max_media_bytes");
            }
        }
        Ok(())
    }
    pub fn revision(&self) -> String {
        crate::digest(serde_json::to_vec(self).unwrap())
    }
    pub fn device(&self, id: &str) -> Result<&Device> {
        self.devices
            .iter()
            .find(|d| d.id == id && d.enabled)
            .context("Choose an enabled device")
    }
    pub fn save(&self, state: &Path) -> Result<()> {
        self.validate()?;
        atomic_json(&state.join("config.json"), self)
    }
}
pub fn validate_model(role: &str, s: &Value) -> Result<()> {
    let provider=s["provider"].as_str().unwrap_or("disabled");
    for key in ["base_url","api_key_env","api_key"] {
        if s[key].as_str().is_some_and(|s|!s.is_empty()){bail!("External model endpoints and credentials are not supported");}
    }
    if provider=="disabled" { return Ok(()); }
    if !["embedding","relevance"].contains(&role) || provider!="native" {
        bail!("Only built-in EmbeddingGemma and Laya are supported; model APIs and LLMs have been removed");
    }
    let expected=if role=="embedding" {crate::native_models::EMBEDDING_MODEL} else {crate::native_models::LAYA_MODEL};
    if s["model"]!=expected {bail!("Use the bundled {expected} model");}
    for key in ["base_url","api_key_env","api_key"] {if s[key].as_str().is_some_and(|s|!s.is_empty()){bail!("External model endpoints and credentials are not supported");}}
    for (key,lo,hi) in [("batch_size",1,32),("chunk_chars",256,1800),("candidates",1,6),("max_chars",32,4000)] {
        if s.get(key).is_some_and(|v|!v.as_u64().is_some_and(|n|(lo..=hi).contains(&n))){bail!("Invalid {role} budget: {key}");}
    }
    let context=if role=="embedding"{2048}else{1024};
    if s["context_tokens"]!=context {bail!("The built-in {role} context is fixed at {context} tokens");}
    if role=="embedding" && (s["document_prefix"]!="title: none | text: " || s["query_prefix"]!="task: search result | query: ") {bail!("Use the built-in EmbeddingGemma retrieval prefixes");}
    if role=="embedding" && s["dimensions"]!=768 {bail!("EmbeddingGemma uses 768 dimensions");}
    for key in ["min_confidence","weight"] {if s.get(key).is_some_and(|v|!v.as_f64().is_some_and(|n|n.is_finite()&&(0.0..=1.0).contains(&n))){bail!("Invalid relevance {key}");}}
    Ok(())
}
pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().context("Missing parent")?;
    fs::create_dir_all(parent)?;
    let mut f = tempfile::NamedTempFile::new_in(parent)?;
    use std::io::Write;
    f.write_all(&serde_json::to_vec_pretty(value)?)?;
    f.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    f.persist(path).map_err(|e| e.error)?;
    Ok(())
}
pub fn default_state() -> PathBuf {
    for name in ["RETRIEVER_STATE", "GOLDEN_RETRIEVER_STATE", "SEARCHX_STATE"] {
        if let Some(p) = std::env::var_os(name) {
            return p.into();
        }
    }
    let home =
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default();
    if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_else(||PathBuf::from(&home).join("AppData/Local").into_os_string())).join("Retriever Candidate/state")
    } else if cfg!(target_os="macos") {
        PathBuf::from(home).join("Library/Application Support/Retriever Candidate/state")
    } else {
        PathBuf::from(std::env::var_os("XDG_DATA_HOME").unwrap_or_else(||PathBuf::from(home).join(".local/share").into_os_string())).join("retriever-candidate")
    }
}
