//! Bounded, versioned protocol shared by the coordinator and model-free agents.
use anyhow::{Result, bail};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, Write};

pub const VERSION: u32 = 1;
pub const MAX_MESSAGE: usize = 2 * 1024 * 1024;
pub const MAX_RESPONSE: usize = 256 * 1024 * 1024;
const FRAGMENT_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub id: String,
    pub action: String,
    #[serde(default)]
    pub body: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub version: u32,
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fragment: Option<Fragment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fragment {
    pub offset: usize,
    pub total: usize,
    pub sha256: String,
    pub data: String,
}

pub fn read_frame(reader: &mut impl BufRead) -> Result<Option<Request>> {
    let mut bytes = Vec::new();
    // read_until alone allows an untrusted peer to allocate unbounded memory.
    let count = std::io::Read::take(&mut *reader, (MAX_MESSAGE + 1) as u64)
        .read_until(b'\n', &mut bytes)?;
    if count == 0 {
        return Ok(None);
    }
    if count > MAX_MESSAGE || !bytes.ends_with(b"\n") {
        bail!("invalid frame length");
    }
    let req: Request = serde_json::from_slice(&bytes)?;
    if req.version != VERSION || req.id.is_empty() || req.id.len() > 128 {
        bail!("unsupported protocol or request id");
    }
    Ok(Some(req))
}

pub fn write_frame(writer: &mut impl Write, response: &Response) -> Result<()> {
    let mut bytes = serde_json::to_vec(response)?;
    if bytes.len() >= MAX_MESSAGE {
        bail!("response exceeds protocol budget");
    }
    bytes.push(b'\n');
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

/// Optional version-1 framing extension, enabled by the requesting coordinator.
/// Long extractions retain full text and coordinates while each wire message
/// stays below MAX_MESSAGE. Peers never send arbitrary filesystem references.
pub fn write_response(writer: &mut impl Write, response: &Response) -> Result<()> {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(response)?;
    if bytes.len() < MAX_MESSAGE {
        return write_frame(writer, response);
    }
    if bytes.len() > MAX_RESPONSE {
        bail!("response exceeds extraction budget");
    }
    let sha256 = hex::encode(Sha256::digest(&bytes));
    for (i, part) in bytes.chunks(FRAGMENT_BYTES).enumerate() {
        write_frame(
            writer,
            &Response {
                version: VERSION,
                id: response.id.clone(),
                ok: true,
                data: None,
                error: None,
                fragment: Some(Fragment {
                    offset: i * FRAGMENT_BYTES,
                    total: bytes.len(),
                    sha256: sha256.clone(),
                    data: base64::engine::general_purpose::STANDARD.encode(part),
                }),
            },
        )?;
    }
    Ok(())
}

pub struct ResponseCollector {
    id: String,
    bytes: Vec<u8>,
    expected: Option<(usize, String)>,
    finished: bool,
}
impl ResponseCollector {
    pub fn new(id: &str) -> Self {
        Self {
            id: id.into(),
            bytes: Vec::new(),
            expected: None,
            finished: false,
        }
    }
    pub fn push(&mut self, frame: &[u8]) -> Result<Option<Response>> {
        use sha2::{Digest, Sha256};
        if self.finished || frame.len() > MAX_MESSAGE || !frame.ends_with(b"\n") {
            bail!("Invalid response frame");
        }
        let response: Response = serde_json::from_slice(frame)?;
        if response.version != VERSION || response.id != self.id {
            bail!("Agent response identity mismatch");
        }
        let Some(fragment) = response.fragment else {
            if self.expected.is_some() {
                bail!("Incomplete fragmented response");
            }
            self.finished = true;
            return Ok(Some(response));
        };
        if !response.ok
            || response.data.is_some()
            || response.error.is_some()
            || fragment.total == 0
            || fragment.total > MAX_RESPONSE
            || fragment.sha256.len() != 64
            || !fragment.sha256.bytes().all(|c| c.is_ascii_hexdigit())
            || fragment.offset != self.bytes.len()
        {
            bail!("Invalid response fragment");
        }
        if let Some((total, sha)) = &self.expected {
            if *total != fragment.total || sha != &fragment.sha256 {
                bail!("Response fragment changed identity");
            }
        } else {
            self.expected = Some((fragment.total, fragment.sha256.clone()));
        }
        let part = base64::engine::general_purpose::STANDARD.decode(fragment.data)?;
        if part.is_empty()
            || part.len() > FRAGMENT_BYTES
            || self.bytes.len() + part.len() > fragment.total
        {
            bail!("Invalid response fragment size");
        }
        self.bytes.extend_from_slice(&part);
        if self.bytes.len() != fragment.total {
            return Ok(None);
        }
        if hex::encode(Sha256::digest(&self.bytes)) != fragment.sha256 {
            bail!("Response checksum mismatch");
        }
        let complete: Response = serde_json::from_slice(&self.bytes)?;
        if complete.version != VERSION || complete.id != self.id || complete.fragment.is_some() {
            bail!("Invalid assembled response");
        }
        self.finished = true;
        self.bytes.clear();
        Ok(Some(complete))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Coordinates {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slide: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sheet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_start: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_end: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_seconds: Option<f64>,
    pub char_start: usize,
    pub char_end: usize,
}
