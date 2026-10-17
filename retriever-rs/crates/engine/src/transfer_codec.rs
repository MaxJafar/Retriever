//! Independent, bounded lossless blocks. Incompressible content stays raw.
use anyhow::{Context, Result, bail};
use base64::Engine as _;
use serde_json::{Value, json};

pub const BLOCK: usize = 1024 * 1024;
const SAMPLE: usize = 16 * 1024;

pub fn encode(bytes: &[u8]) -> Result<Value> {
    if bytes.len() > BLOCK {
        bail!("Transfer block exceeds budget");
    }
    // Sample three regions before spending CPU on a full block. A mixed block
    // can still fall back to raw; compression must save at least five percent.
    let worth_trying = bytes.len() >= 1024
        && [0, bytes.len() / 2, bytes.len().saturating_sub(SAMPLE)]
            .into_iter()
            .any(|start| {
                let sample = &bytes[start..(start + SAMPLE).min(bytes.len())];
                lz4_flex::block::compress(sample).len() * 100 < sample.len() * 95
            });
    let compressed = worth_trying.then(|| lz4_flex::block::compress(bytes));
    let (encoding, data) = match compressed.as_deref() {
        Some(data) if data.len() * 100 < bytes.len() * 95 => ("lz4-block", data),
        _ => ("raw", bytes),
    };
    Ok(
        json!({"encoding":encoding,"raw_length":bytes.len(),"sha256":crate::digest(bytes),
        "data":base64::engine::general_purpose::STANDARD.encode(data)}),
    )
}

pub fn decode(block: &Value) -> Result<Vec<u8>> {
    let length = block["raw_length"]
        .as_u64()
        .context("Missing block length")?;
    if length > BLOCK as u64 {
        bail!("Transfer decode exceeds budget");
    }
    let data = block["data"].as_str().context("Missing block data")?;
    if data.len() > BLOCK.div_ceil(3) * 4 {
        bail!("Encoded block exceeds budget");
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(data)?;
    let decoded = match block["encoding"].as_str() {
        Some("raw") => bytes,
        Some("lz4-block") => {
            let mut output = vec![0; length as usize];
            let actual = lz4_flex::block::decompress_into(&bytes, &mut output)?;
            if actual != output.len() {
                bail!("Decoded block length mismatch");
            }
            output
        }
        _ => bail!("Unsupported transfer encoding"),
    };
    if decoded.len() != length as usize || block["sha256"] != crate::digest(&decoded) {
        bail!("Transfer block checksum mismatch");
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adaptive_blocks_are_lossless_and_bounded() {
        let text = b"A real text block repeats words and identifiers.\n".repeat(18000);
        let encoded = encode(&text).unwrap();
        assert_eq!(encoded["encoding"], "lz4-block");
        assert_eq!(decode(&encoded).unwrap(), text);
        let mut seed = 0x18abcdef_u64;
        let noise: Vec<u8> = (0..BLOCK)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed as u8
            })
            .collect();
        let encoded = encode(&noise).unwrap();
        assert_eq!(encoded["encoding"], "raw");
        assert_eq!(decode(&encoded).unwrap(), noise);
        assert_eq!(decode(&encode(&[]).unwrap()).unwrap(), Vec::<u8>::new());
        assert!(encode(&vec![0; BLOCK + 1]).is_err());
    }
    #[test]
    fn corrupt_and_oversized_blocks_never_decode() {
        let valid = encode(&vec![b'a'; BLOCK]).unwrap();
        for key in ["sha256", "encoding", "data"] {
            let mut bad = valid.clone();
            bad[key] = json!("invalid");
            assert!(decode(&bad).is_err());
        }
        let mut bad = valid.clone();
        bad["raw_length"] = json!(BLOCK + 1);
        assert!(decode(&bad).is_err());
        let mut bad = valid;
        bad["raw_length"] = json!(1);
        assert!(decode(&bad).is_err());
    }
}
