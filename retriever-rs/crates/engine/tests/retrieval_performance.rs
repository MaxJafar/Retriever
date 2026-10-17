use retriever_engine::{
    store::Store,
    vectors::{Row, Snapshot},
};
use rusqlite::params;
use serde_json::json;
use std::{collections::HashSet, io::Write, sync::Arc, time::Instant};

fn snapshot(rows: usize, dimensions: usize, clustered: bool) -> (tempfile::TempDir, Arc<Snapshot>) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    std::fs::create_dir(dir.path().join("vectors")).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let mut bytes = Vec::with_capacity(rows * dimensions * 4);
    let mut identities = Vec::with_capacity(rows);
    for i in 0..rows {
        let mut values = Vec::with_capacity(dimensions);
        for j in 0..dimensions {
            values.push(if clustered {
                if j == 0 {
                    1.0 - (i as f32) * 0.0001
                } else {
                    0.001 * (i + j) as f32
                }
            } else {
                (((i as u64 + 1).wrapping_mul(6364136223846793005u64.wrapping_add(j as u64 * 17)))
                    .wrapping_add(j as u64 * 193)
                    % 1000) as f32
                    / 1000.0
                    - 0.5
            });
        }
        let values = retriever_engine::vectors::normalize(&values, dimensions).unwrap();
        for value in values {
            bytes.write_all(&value.to_le_bytes()).unwrap();
        }
        identities.push(Row {
            passage: i as i64,
            file: if clustered && i < 80 {
                1
            } else {
                (i / 5 + 2) as i64
            },
            digest: "fixture".into(),
            extraction: "test".into(),
        });
    }
    std::fs::write(dir.path().join("vectors").join(format!("{id}.f32")), &bytes).unwrap();
    store
        .db
        .lock()
        .execute(
            "INSERT INTO snapshots VALUES(?,?,?,?,?,1)",
            params![
                id,
                "test",
                dimensions,
                serde_json::to_string(&identities).unwrap(),
                retriever_engine::digest(&bytes)
            ],
        )
        .unwrap();
    let snapshot = Snapshot::load(&store, "test").unwrap().unwrap();
    (dir, snapshot)
}

#[test]
fn shortlist_is_diverse_and_filters_are_applied_before_ranking() {
    let (_dir, snapshot) = snapshot(100, 4, true);
    let query = vec![1.0, 0.0, 0.0, 0.0];
    let old = snapshot.score(&query, 10).unwrap();
    assert!(old.iter().all(|(r, _)| r.file == 1));
    let diverse = snapshot.score_files(&query, 5, None).unwrap();
    assert_eq!(diverse.len(), 5);
    assert_eq!(
        diverse
            .iter()
            .map(|(r, _)| r.file)
            .collect::<HashSet<_>>()
            .len(),
        5
    );
    let allowed = HashSet::from([21]);
    let filtered = snapshot.score_files(&query, 2, Some(&allowed)).unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].0.file, 21);
    assert!(
        snapshot
            .score_files(&query, 2, Some(&HashSet::new()))
            .unwrap()
            .is_empty()
    );
    let repeated = snapshot.score_files(&query, 5, None).unwrap();
    assert_eq!(
        diverse
            .iter()
            .map(|(r, s)| (r.passage, *s))
            .collect::<Vec<_>>(),
        repeated
            .iter()
            .map(|(r, s)| (r.passage, *s))
            .collect::<Vec<_>>()
    );
    assert_eq!(snapshot.score_files(&query, 1, None).unwrap().len(), 1);
}

#[test]
#[ignore = "Release-mode CPU benchmark, 50,000 passages x 768 dimensions"]
fn repeated_semantic_ranking_benchmark() {
    let (_dir, snapshot) = snapshot(50_000, 768, false);
    let query = vec![0.5; 768];
    let mut before = Vec::new();
    let mut cached = Vec::new();
    snapshot.score(&query, 60).unwrap();
    let cold_start = Instant::now();
    snapshot.score_files(&query, 60, None).unwrap();
    let cold_ms = cold_start.elapsed().as_secs_f64() * 1000.0;
    for _ in 0..25 {
        let t = Instant::now();
        snapshot.score(&query, 60).unwrap();
        before.push(t.elapsed().as_secs_f64() * 1000.0);
        let t = Instant::now();
        snapshot.score_files(&query, 60, None).unwrap();
        cached.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    before.sort_by(f64::total_cmp);
    cached.sort_by(f64::total_cmp);
    let report = json!({"passages":50_000,"dimensions":768,"files":10_000,"iterations":25,"baseline_repeat_p95_ms":before[23],"cached_repeat_p95_ms":cached[23],"new_cold_ms":cold_ms,"scope":"in-process release-build vector ranking only; model and file IO excluded"});
    println!("{report}");
    if let Ok(path) = std::env::var("RETRIEVER_BENCH_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
