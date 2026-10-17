use anyhow::{Result, bail};
use retriever_protocol::Coordinates;
use serde::{Deserialize, Serialize};
use std::{fs, io::Read, path::Path};

pub const EXTRACTION_REVISION: &str = "retriever-native-text-v5";

/// Video is hashed/copied in bounded buffers; its budget is independent of
/// document decoders, which still have a strictly bounded in-memory input.
pub fn source_limit(limits: &serde_json::Value, path: &str) -> u64 {
    let ordinary = limits["max_file_bytes"]
        .as_u64()
        .unwrap_or(134217728)
        .min(268435456);
    let ext = Path::new(path)
        .extension()
        .unwrap_or_default()
        .to_string_lossy();
    if kind(&ext) == Some("video") {
        limits["max_media_bytes"]
            .as_u64()
            .unwrap_or(ordinary)
            .min(8589934592)
    } else {
        ordinary
    }
}

/// Full-content hash and screening, optionally writing a private decoder
/// snapshot. Memory use is constant even for multi-gigabyte recordings.
pub fn stream_source(
    path: &Path,
    max: u64,
    mut output: Option<&mut fs::File>,
) -> Result<(String, u64)> {
    use sha2::{Digest, Sha256};
    use std::io::Write;
    let mut input = fs::File::open(path)?;
    if input.metadata()?.len() > max {
        bail!("file_budget");
    }
    let mut sha = Sha256::new();
    let mut buffer = vec![0; 65536];
    let mut tail = Vec::with_capacity(65536 + 8192);
    let mut total = 0u64;
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        total = total.saturating_add(n as u64);
        if total > max {
            bail!("file_budget");
        }
        sha.update(&buffer[..n]);
        tail.extend_from_slice(&buffer[..n]);
        if crate::paths::secret_content(&tail) {
            bail!("secret_content");
        }
        if tail.len() > 8192 {
            tail.drain(..tail.len() - 8192);
        }
        if let Some(file) = output.as_mut() {
            file.write_all(&buffer[..n])?;
        }
    }
    if let Some(file) = output {
        file.flush()?;
    }
    Ok((hex::encode(sha.finalize()), total))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Passage {
    pub text: String,
    pub coordinates: Coordinates,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Extraction {
    pub text: String,
    pub detail: String,
    pub passages: Vec<Passage>,
}
pub fn kind(extension: &str) -> Option<&'static str> {
    Some(match extension.to_lowercase().as_str() {
        "txt" | "md" | "markdown" | "csv" | "log" | "rst" | "json" | "yaml" | "yml" | "toml"
        | "pdf" | "docx" | "xlsx" | "pptx" => "documents",
        "py" | "js" | "ts" | "tsx" | "jsx" | "swift" | "rs" | "go" | "java" | "c" | "cpp" | "h"
        | "sh" | "ps1" | "cs" | "html" | "css" => "code",
        "png" | "jpg" | "jpeg" | "webp" | "heic" | "gif" | "bmp" | "tif" | "tiff" => "image",
        "wav" | "mp3" | "m4a" | "flac" | "ogg" => "audio",
        "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v" => "video",
        _ => return None,
    })
}
/// Leave room for the retrieval prefix and special tokens, even when every
/// UTF-8 byte falls back to a separate token. ASCII can still use a full chunk.
pub const PASSAGE_BYTE_LIMIT: usize = 1960;
/// Character (not byte) coordinates, with complete long-document coverage.
pub fn passages(text: &str, chunk: usize, page: Option<u32>) -> Vec<Passage> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    let mut line = 1;
    while start < chars.len() {
        let mut end = (start + chunk.max(64)).min(chars.len());
        let mut bytes=0;
        for (index,c) in chars[start..end].iter().enumerate() {
            bytes+=c.len_utf8();
            if bytes>PASSAGE_BYTE_LIMIT {end=start+index;break;}
        }
        if end < chars.len() {
            if let Some(at) = chars[start + (end - start) / 2..end]
                .iter()
                .rposition(|c| *c == '\n')
            {
                end = start + (end - start) / 2 + at + 1;
            }
        }
        let part: String = chars[start..end].iter().collect();
        let lines = part.chars().filter(|c| *c == '\n').count() as u32;
        if !part.trim().is_empty() {
            out.push(Passage {
                text: part,
                coordinates: Coordinates {
                    page,
                    line_start: Some(line),
                    line_end: Some(line + lines),
                    char_start: start,
                    char_end: end,
                    ..Default::default()
                },
            });
        }
        line += lines;
        start = end;
    }
    out
}
pub fn extract(path: &Path, bytes: &[u8], max_chars: usize, chunk: usize) -> Result<Extraction> {
    if crate::paths::secret_content(bytes) {
        bail!("secret_content");
    }
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let text = match ext.as_str() {
        "docx" | "xlsx" | "pptx" => {
            return crate::office::extract_structured(bytes, &ext, max_chars, chunk);
        }
        "pdf" => return crate::media::pdf_text(bytes, max_chars, chunk),
        _ if matches!(kind(&ext), Some("image" | "audio" | "video")) => {
            return Ok(Extraction {
                text: String::new(),
                detail: "metadata: media_analysis_pending".into(),
                passages: vec![],
            });
        }
        _ => decode_text(bytes)?,
    };
    if text.chars().count() > max_chars {
        bail!("extracted_text_budget");
    }
    if crate::paths::secret_content(text.as_bytes()) {
        bail!("secret_content");
    }
    Ok(Extraction {
        passages: passages(&text, chunk, None),
        text,
        detail: "text".into(),
    })
}
/// Windows editors commonly produce UTF-16. Reject corrupt encodings rather
/// than silently indexing replacement characters and making the file unfindable.
pub fn decode_text(bytes: &[u8]) -> Result<String> {
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        let little = bytes[0] == 0xff;
        if (bytes.len() - 2) % 2 != 0 {
            bail!("invalid_utf16");
        }
        let units: Vec<_> = bytes[2..]
            .chunks_exact(2)
            .map(|b| {
                if little {
                    u16::from_le_bytes([b[0], b[1]])
                } else {
                    u16::from_be_bytes([b[0], b[1]])
                }
            })
            .collect();
        return Ok(String::from_utf16(&units)?);
    }
    let text = std::str::from_utf8(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes))?;
    if text.contains('\0') {
        bail!("binary_text_file");
    }
    Ok(text.to_owned())
}
pub fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>> {
    let f = fs::File::open(path)?;
    if f.metadata()?.len() > max {
        bail!("file_budget");
    }
    let mut bytes = Vec::new();
    f.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        bail!("file_budget");
    }
    Ok(bytes)
}
