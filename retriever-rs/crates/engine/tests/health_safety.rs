#![cfg(windows)]

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use retriever_engine::{engine::Engine, http::router};
use serde_json::{Value, json};
use tower::ServiceExt;

// In-memory HTTP routing only. No server, devices, external services or models.
#[tokio::test]
async fn windows_health_http_routes_cannot_enable_or_launch_collection() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::open(dir.path()).unwrap();
    let app = router(engine.clone());
    for (method, path, body, expected) in [
        ("GET", "/api/health", json!({}), StatusCode::OK),
        ("POST", "/api/health/refresh", json!({}), StatusCode::OK),
        (
            "POST",
            "/api/health/settings",
            json!({"interval_minutes":1,"temperature_c":70,"remaining_life_percent":10}),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1")
            .header("authorization", format!("Bearer {}", engine.token))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), expected, "{path}");
        let data: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
        assert!(!data["error"].as_str().unwrap().is_empty(), "{path}");
        if expected == StatusCode::OK {
            assert_eq!(data["probe_available"], false);
            assert_eq!(data["refreshing"], false);
            assert_eq!(data["last_attempt"].as_f64(), Some(0.));
        }
    }
    assert_eq!(engine.health.settings().unwrap()["interval_minutes"], 0);
    assert_eq!(
        engine.health.status().unwrap()["last_attempt"].as_f64(),
        Some(0.)
    );
}
