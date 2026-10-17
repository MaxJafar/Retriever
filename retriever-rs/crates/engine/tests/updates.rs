use retriever_engine::engine::Engine;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn local_update_requires_local_secret_and_quiescent_library() {
    let root = tempfile::tempdir().unwrap();
    let engine = Engine::open(root.path()).unwrap();
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        engine
            .stop_for_update(&engine.token, version)
            .await
            .is_err()
    );
    assert!(
        engine
            .stop_for_update(&engine.update_token, "wrong-version")
            .await
            .is_err()
    );
    engine.syncing.store(true, Ordering::SeqCst);
    assert!(
        engine
            .stop_for_update(&engine.update_token, version)
            .await
            .is_err()
    );
    engine.syncing.store(false, Ordering::SeqCst);
    let request = engine.runtime_gate.read().await;
    assert!(
        engine
            .stop_for_update(&engine.update_token, version)
            .await
            .is_err()
    );
    drop(request);
    let original = std::fs::read(root.path().join("api-token")).unwrap();
    let stopped = engine
        .stop_for_update(&engine.update_token, version)
        .await
        .unwrap();
    assert_eq!(stopped["checkpointed"], true);
    assert!(*engine.shutdown.borrow());
    assert!(engine.start_sync(true).is_err());
    assert_eq!(
        original,
        std::fs::read(root.path().join("api-token")).unwrap()
    );
    assert_eq!(
        engine
            .store
            .db
            .lock()
            .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
}

#[tokio::test]
async fn update_authorization_does_not_grant_shared_clients_shutdown() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let root = tempfile::tempdir().unwrap();
    let engine = Engine::open(root.path()).unwrap();
    let response = retriever_engine::http::router(engine.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/runtime/shutdown")
                .header("host", "localhost")
                .header("authorization", format!("Bearer {}", engine.token))
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    "{{\"expected_version\":\"{}\"}}",
                    env!("CARGO_PKG_VERSION")
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(!*engine.shutdown.borrow());
}
