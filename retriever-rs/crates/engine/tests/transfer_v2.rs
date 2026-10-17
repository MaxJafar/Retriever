use retriever_engine::{agent, config::Device, transfer, transfer_codec};
use serde_json::{Value, json};

fn device(root: &std::path::Path) -> Device {
    serde_json::from_value(json!({"id":"test-copy","label":"Copy test","platform":if cfg!(windows){"windows"}else{"macos"},"transport":"local","roots":[root],"exclude":[],"enabled":true})).unwrap()
}

#[test]
fn compressed_transfer_rejects_corruption_and_preserves_time() {
    let dir = tempfile::Builder::new()
        .prefix("retriever-copy-")
        .tempdir()
        .unwrap();
    let d = device(dir.path());
    let id = uuid::Uuid::new_v4().to_string();
    let dest = dir.path().join("result.txt");
    let data = b"Exact file bytes, even at block boundaries.\n".repeat(18000);
    let call = |action: &str, body: Value| transfer::dispatch(&d, action, &body, &[]);
    call(
        "transfer_begin",
        json!({"id":id,"path":dest,"size":data.len(),"mtime_ns":1_650_000_000_000_000_000u64}),
    )
    .unwrap();
    let mut encoded = transfer_codec::encode(&data).unwrap();
    encoded["id"] = json!(id);
    encoded["offset"] = json!(0);
    let mut bad = encoded.clone();
    bad["sha256"] = json!("bad");
    assert!(call("transfer_write_v2", bad).is_err());
    assert_eq!(
        call("transfer_inspect", json!({"id":id})).unwrap()["offset"],
        0
    );
    call("transfer_write_v2", encoded.clone()).unwrap();
    assert!(call("transfer_write_v2", encoded).is_err());
    assert!(call("transfer_finish", json!({"id":id,"sha256":"wrong"})).is_err());
    assert!(!dest.exists());
    let receipt = call(
        "transfer_finish",
        json!({"id":id,"sha256":retriever_engine::digest(&data)}),
    )
    .unwrap();
    assert_eq!(receipt["state"], "applied");
    assert_eq!(receipt["compressed_blocks"], 1);
    assert_eq!(std::fs::read(&dest).unwrap(), data);
    assert_eq!(
        agent::pin(&dest).unwrap()["mtime_ns"],
        1_650_000_000_000_000_000u64
    );
}

#[test]
fn native_copy_preserves_original_and_rejects_existing_destination() {
    let dir = tempfile::Builder::new()
        .prefix("retriever-copy-")
        .tempdir()
        .unwrap();
    let d = device(dir.path());
    let source = dir.path().join("source.txt");
    let dest = dir.path().join("copy.txt");
    std::fs::write(&source, b"Native copy with exact bytes.\n").unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&source)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new().set_modified(
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_650_000_000),
            ),
        )
        .unwrap();
    let pin = agent::pin(&source).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let request = json!({"id":id,"source":source,"path":dest,"pin":pin});
    let receipt = transfer::dispatch(&d, "transfer_local_copy", &request, &[]).unwrap();
    assert_eq!(receipt["state"], "applied");
    assert_eq!(receipt["method"], "native");
    assert_eq!(
        std::fs::read(&source).unwrap(),
        std::fs::read(&dest).unwrap()
    );
    assert_eq!(agent::pin(&source).unwrap(), pin);
    assert_eq!(agent::pin(&dest).unwrap()["mtime_ns"], pin["mtime_ns"]);
    assert!(transfer::dispatch(&d, "transfer_local_copy", &request, &[]).is_err());
    let mut conflict = request;
    conflict["id"] = json!(uuid::Uuid::new_v4().to_string());
    assert!(transfer::dispatch(&d, "transfer_local_copy", &conflict, &[]).is_err());
}

#[test]
fn interrupted_and_oversized_blocks_never_publish() {
    let dir = tempfile::Builder::new()
        .prefix("retriever-copy-")
        .tempdir()
        .unwrap();
    let d = device(dir.path());
    let id = uuid::Uuid::new_v4().to_string();
    let dest = dir.path().join("copy.txt");
    let receipt = transfer::dispatch(
        &d,
        "transfer_begin",
        &json!({"id":id,"path":dest,"size":1}),
        &[],
    )
    .unwrap();
    let oversized = json!({"id":id,"offset":0,"raw_length":transfer_codec::BLOCK+1,"encoding":"lz4-block","data":"","sha256":""});
    assert!(transfer::dispatch(&d, "transfer_write_v2", &oversized, &[]).is_err());
    std::fs::write(receipt["temporary"].as_str().unwrap(), b"unacknowledged").unwrap();
    assert!(
        transfer::dispatch(
            &d,
            "transfer_finish",
            &json!({"id":id,"sha256":retriever_engine::digest(b"unacknowledged")}),
            &[]
        )
        .is_err()
    );
    assert!(!dest.exists());
}

#[tokio::test]
async fn background_copy_can_stop_without_publication_and_legacy_stream_still_works() {
    use retriever_engine::engine::Engine;
    let dir = tempfile::Builder::new()
        .prefix("retriever-copy-")
        .tempdir()
        .unwrap();
    let source = dir.path().join("source.txt");
    let destination = dir.path().join("destination");
    std::fs::write(&source, b"Copy job original").unwrap();
    std::fs::create_dir(&destination).unwrap();
    let engine = Engine::open(&dir.path().join("state")).unwrap();
    let device = engine.config.read().devices[0].id.clone();
    let body = json!({"device":device,"target_device":device,"scope":"filesystem","operation":"copy","parent":destination,"items":[{"path":source}]});
    let plan = engine.operation_plan(&body).await.unwrap();
    let id = plan["id"].as_str().unwrap();
    assert_eq!(plan["method"], "native");
    engine.start_operation(id).unwrap();
    assert!(engine.start_operation(id).is_err());
    engine.cancel_operation(id).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if engine.operation(id).unwrap()["state"] == "inspection_required" {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!destination.join("source.txt").exists());
    assert!(source.exists());
    assert!(engine.start_operation(id).is_err());
    let mut legacy = engine.operation_plan(&body).await.unwrap();
    legacy["transfer_protocol"] = json!(1);
    legacy["method"] = json!("compatible");
    engine
        .store
        .db
        .lock()
        .execute(
            "UPDATE workspace_plans SET body=? WHERE id=?",
            rusqlite::params![legacy.to_string(), legacy["id"].as_str()],
        )
        .unwrap();
    let result = engine
        .apply_operation(legacy["id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(result["state"], "applied");
    assert_eq!(
        result["receipt"]["items"][0]["transfer"]["method"],
        "compatible"
    );
    assert_eq!(
        std::fs::read(&source).unwrap(),
        std::fs::read(destination.join("source.txt")).unwrap()
    );
}

#[cfg(windows)]
#[test]
fn native_copy_preserves_readonly_attribute_and_alternate_stream() {
    let dir = tempfile::Builder::new()
        .prefix("retriever-copy-")
        .tempdir()
        .unwrap();
    let source = dir.path().join("source.txt");
    let dest = dir.path().join("destination.txt");
    std::fs::write(&source, b"Main stream").unwrap();
    std::fs::write(
        format!("{}:retriever-test", source.display()),
        b"Alternate stream",
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&source).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&source, permissions).unwrap();
    let result = transfer::dispatch(
        &device(dir.path()),
        "transfer_local_copy",
        &json!({"id":uuid::Uuid::new_v4().to_string(),"source":source,"path":dest,"pin":agent::pin(&source).unwrap()}),
        &[],
    );
    assert_eq!(result.unwrap()["state"], "applied");
    assert!(std::fs::metadata(&dest).unwrap().permissions().readonly());
    assert_eq!(
        std::fs::read(format!("{}:retriever-test", dest.display())).unwrap(),
        b"Alternate stream"
    );
    for path in [&source, &dest] {
        let mut p = std::fs::metadata(path).unwrap().permissions();
        p.set_readonly(false);
        std::fs::set_permissions(path, p).unwrap();
    }
}
