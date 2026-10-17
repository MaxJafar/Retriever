//! Lossless, content-addressed mmap blocks. Old flat snapshots remain readable.
use anyhow::{Context, Result, bail};
use memmap2::Mmap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
};

const BLOCK_BYTES: usize = 4 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Segment {
    sha256: String,
    bytes: usize,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: u32,
    segments: Vec<Segment>,
}

pub fn load(
    directory: &Path,
    id: &str,
    dimensions: usize,
    rows: usize,
    expected: &str,
) -> Result<Vec<Mmap>> {
    if dimensions == 0 || dimensions > 16384 || rows == 0 {
        bail!("Invalid snapshot dimensions");
    }
    let mut maps = Vec::new();
    let flat = directory.join(format!("{id}.f32"));
    if flat.exists() {
        crate::paths::reject_links(&flat)?;
        maps.push(unsafe { Mmap::map(&File::open(flat)?)? });
    } else {
        let path = directory.join(format!("{id}.segments.json"));
        crate::paths::reject_links(&path)?;
        if fs::metadata(&path)?.len() > 16 * 1024 * 1024 {
            bail!("Snapshot manifest exceeds budget");
        }
        let manifest: Manifest = serde_json::from_slice(&fs::read(path)?)?;
        if manifest.format != 1 || manifest.segments.is_empty() || manifest.segments.len() > 100_000
        {
            bail!("Invalid snapshot manifest");
        }
        for segment in manifest.segments {
            if segment.sha256.len() != 64
                || !segment.sha256.bytes().all(|c| c.is_ascii_hexdigit())
                || segment.bytes == 0
                || segment.bytes > BLOCK_BYTES
                || segment.bytes % (dimensions * 4) != 0
            {
                bail!("Invalid snapshot block");
            }
            let path = directory
                .join("blocks")
                .join(format!("{}.f32", segment.sha256));
            crate::paths::reject_links(&path)?;
            let file = File::open(path)?;
            if file.metadata()?.len() != segment.bytes as u64 {
                bail!("Snapshot block length mismatch");
            }
            let map = unsafe { Mmap::map(&file)? };
            if crate::digest(&map) != segment.sha256 {
                bail!("Snapshot block checksum mismatch");
            }
            maps.push(map);
        }
    }
    let expected_length = rows
        .checked_mul(dimensions * 4)
        .context("Snapshot overflow")?;
    let mut sha = Sha256::new();
    let mut length = 0usize;
    for map in &maps {
        length = length.checked_add(map.len()).context("Snapshot overflow")?;
        sha.update(map);
    }
    if length != expected_length || hex::encode(sha.finalize()) != expected {
        bail!("Snapshot integrity check failed");
    }
    Ok(maps)
}

fn block(directory: &Path, bytes: &[u8]) -> Result<Segment> {
    let sha256 = crate::digest(bytes);
    let path = directory.join(format!("{sha256}.f32"));
    crate::paths::reject_links(directory)?;
    let existing = if path.exists() {
        true
    } else {
        let mut output = tempfile::NamedTempFile::new_in(directory)?;
        output.write_all(bytes)?;
        output.as_file().sync_all()?;
        match output.persist_noclobber(&path) {
            Ok(_) => false,
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => true,
            Err(error) => return Err(error.error.into()),
        }
    };
    if existing {
        crate::paths::reject_links(&path)?;
        let file = File::open(&path)?;
        if file.metadata()?.len() != bytes.len() as u64 {
            bail!("Existing vector block length mismatch");
        }
        let map = unsafe { Mmap::map(&file)? };
        if crate::digest(&map) != sha256 {
            bail!("Existing vector block is corrupt; preserved for inspection");
        }
    }
    Ok(Segment {
        sha256,
        bytes: bytes.len(),
    })
}

pub fn publish(
    reader: &rusqlite::Connection,
    directory: &Path,
    id: &str,
    fingerprint: &str,
    dimensions: usize,
) -> Result<()> {
    let blocks = directory.join("blocks");
    fs::create_dir_all(&blocks)?;
    crate::paths::reject_links(&blocks)?;
    let rows_per_block = (BLOCK_BYTES / (dimensions * 4)).max(1) as i64;
    let mut buffer = Vec::with_capacity(BLOCK_BYTES);
    let mut segments = Vec::new();
    let mut bucket = None;
    let mut statement=reader.prepare("SELECT p.id,v.vector FROM vectors v JOIN passages p ON p.id=v.passage_id JOIN files f ON f.id=p.file_id AND f.digest=p.digest WHERE v.fingerprint=? ORDER BY p.id")?;
    let mut cursor = statement.query([fingerprint])?;
    while let Some(row) = cursor.next()? {
        let id: i64 = row.get(0)?;
        let bytes: Vec<u8> = row.get(1)?;
        if bytes.len() != dimensions * 4 {
            bail!("Vector dimensions changed during publication");
        }
        let next = id.div_euclid(rows_per_block);
        // Stable passage-ID ranges avoid rewriting every following block when
        // one document is removed. Blocks retain exact f32 bytes and order.
        if bucket.is_some_and(|current| current != next) && !buffer.is_empty() {
            segments.push(block(&blocks, &buffer)?);
            buffer.clear();
        }
        bucket = Some(next);
        buffer.extend_from_slice(&bytes);
    }
    if !buffer.is_empty() {
        segments.push(block(&blocks, &buffer)?);
    }
    let manifest = Manifest {
        format: 1,
        segments,
    };
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(directory.join(format!("{id}.segments.json")))?;
    serde_json::to_writer(&mut output, &manifest)?;
    output.sync_all()?;
    Ok(())
}

/// Materialize the exact legacy representation before rolling back to an older
/// engine. No catalogue rows, blocks, caches or existing files are removed.
pub fn materialize(
    directory: &Path,
    id: &str,
    dimensions: usize,
    rows: usize,
    expected: &str,
) -> Result<bool> {
    let maps = load(directory, id, dimensions, rows, expected)?;
    let destination = directory.join(format!("{id}.f32"));
    if destination.exists() {
        return Ok(false);
    }
    let mut output = tempfile::NamedTempFile::new_in(directory)?;
    for map in &maps {
        output.write_all(map)?;
    }
    output.as_file().sync_all()?;
    output
        .persist_noclobber(&destination)
        .map_err(|e| e.error)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (tempfile::TempDir, rusqlite::Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE files(id INTEGER,digest TEXT); CREATE TABLE passages(id INTEGER,file_id INTEGER,digest TEXT); CREATE TABLE vectors(passage_id INTEGER,vector BLOB,fingerprint TEXT); INSERT INTO files VALUES(1,'a'),(2,'b'); INSERT INTO passages VALUES(1,1,'a'),(262144,2,'b');").unwrap();
        for (id, value) in [(1, 1f32), (262144, 2f32)] {
            db.execute(
                "INSERT INTO vectors VALUES(?,?,'test')",
                rusqlite::params![id, value.to_le_bytes().repeat(4)],
            )
            .unwrap();
        }
        (dir, db)
    }
    #[test]
    fn segments_reuse_unchanged_bytes_and_export_exact_legacy_format() {
        let (dir, db) = setup();
        let expected = [1f32.to_le_bytes().repeat(4), 2f32.to_le_bytes().repeat(4)].concat();
        publish(&db, dir.path(), "first", "test", 4).unwrap();
        publish(&db, dir.path(), "second", "test", 4).unwrap();
        assert_eq!(fs::read_dir(dir.path().join("blocks")).unwrap().count(), 2);
        let maps = load(dir.path(), "first", 4, 2, &crate::digest(&expected)).unwrap();
        assert_eq!(
            maps.iter()
                .flat_map(|m| m.iter().copied())
                .collect::<Vec<_>>(),
            expected
        );
        drop(maps);
        assert!(materialize(dir.path(), "first", 4, 2, &crate::digest(&expected)).unwrap());
        assert_eq!(fs::read(dir.path().join("first.f32")).unwrap(), expected);
        assert!(!materialize(dir.path(), "first", 4, 2, &crate::digest(&expected)).unwrap());
        assert_eq!(
            load(dir.path(), "first", 4, 2, &crate::digest(&expected))
                .unwrap()
                .len(),
            1
        );
        db.execute(
            "UPDATE vectors SET vector=? WHERE passage_id=1",
            [3f32.to_le_bytes().repeat(4)],
        )
        .unwrap();
        publish(&db, dir.path(), "third", "test", 4).unwrap();
        assert_eq!(fs::read_dir(dir.path().join("blocks")).unwrap().count(), 3);
        assert_eq!(fs::read(dir.path().join("first.f32")).unwrap(), expected);
    }
    #[test]
    fn corrupt_and_path_escaping_blocks_are_rejected_without_publication() {
        let (dir, db) = setup();
        publish(&db, dir.path(), "test", "test", 4).unwrap();
        let expected = [1f32.to_le_bytes().repeat(4), 2f32.to_le_bytes().repeat(4)].concat();
        let path = dir
            .path()
            .join("blocks")
            .join(format!("{}.f32", crate::digest(&expected[..16])));
        fs::write(path, [0u8; 16]).unwrap();
        assert!(materialize(dir.path(), "test", 4, 2, &crate::digest(&expected)).is_err());
        assert!(!dir.path().join("test.f32").exists());
        assert!(publish(&db, dir.path(), "later", "test", 4).is_err());
        fs::write(
            dir.path().join("test.segments.json"),
            br#"{"format":1,"segments":[{"sha256":"../escape","bytes":16}]}"#,
        )
        .unwrap();
        assert!(load(dir.path(), "test", 4, 2, &crate::digest(&expected)).is_err());
    }
}
