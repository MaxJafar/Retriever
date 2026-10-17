//! Bounded local benchmark. Separate complete prefixed passages with U+001E,
//! or use one passage per line when passages contain no newlines.
//! Prints counts/timing only; optional vectors support before/after comparison.
use retriever_embedding_native::Embedder;
use std::{fs, path::Path, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() != 5 {
        return Err(
            "Usage: throughput <model.gguf> <passages.txt> <batch-size> <vectors.bin>".into(),
        );
    }
    let batch = args[3].parse::<usize>()?;
    if !(1..=32).contains(&batch) {
        return Err("Batch size must be 1..32".into());
    }
    let input = fs::read_to_string(&args[2])?;
    let rows = if input.contains('\u{001e}') {
        input.split('\u{001e}').map(str::to_owned).collect::<Vec<_>>()
    } else {
        input.lines().map(str::to_owned).collect::<Vec<_>>()
    };
    if rows.is_empty() || rows.len() > 64 || rows.iter().any(|r| r.is_empty() || r.len() > 8192) {
        return Err("Provide 1..64 bounded, complete passages".into());
    }
    let start = Instant::now();
    let mut model = Embedder::load(Path::new(&args[1]))?;
    let load_ms = start.elapsed().as_millis();
    let start = Instant::now();
    let mut bytes = Vec::new();
    for group in rows.chunks(batch) {
        for vector in model.embed(group)? {
            if vector.len() != 768 || vector.iter().any(|v| !v.is_finite()) {
                return Err("Invalid embedding".into());
            }
            bytes.extend(vector.into_iter().flat_map(f32::to_le_bytes));
        }
    }
    let inference_ms = start.elapsed().as_millis();
    fs::write(&args[4], bytes)?;
    println!(
        "{{\"passages\":{},\"bytes\":{},\"batch_size\":{},\"load_ms\":{},\"inference_ms\":{}}}",
        rows.len(),
        rows.iter().map(String::len).sum::<usize>(),
        batch,
        load_ms,
        inference_ms
    );
    Ok(())
}
