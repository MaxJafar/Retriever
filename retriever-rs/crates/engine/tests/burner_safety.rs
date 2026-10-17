use axum::{body::{Body,to_bytes},http::{Request,StatusCode}};
use retriever_engine::{engine::Engine,http::router};
use serde_json::{json,Value};
use tower::ServiceExt;

#[tokio::test]
async fn burner_http_rejects_all_physical_targets_and_bypass_fields() {
    let temp=tempfile::tempdir().unwrap();let engine=Engine::open(temp.path()).unwrap();let app=router(engine.clone());
    for (method,path,body,expected) in [
        ("GET","/api/burner",json!({}),StatusCode::OK),
        ("POST","/api/burner/start",json!({"inspection_id":"anything","target_id":"\\\\.\\PhysicalDrive0"}),StatusCode::BAD_REQUEST),
        ("POST","/api/burner/start",json!({"inspection_id":"anything","target_id":"/dev/disk2"}),StatusCode::BAD_REQUEST),
        ("POST","/api/burner/start",json!({"inspection_id":"anything","target_id":"sandbox-8","force":true}),StatusCode::BAD_REQUEST),
        ("POST","/api/burner/inspect",json!({"path":"/dev/disk0"}),StatusCode::BAD_REQUEST),
        ("POST","/api/burner/inspect",json!({"path":"\\\\.\\PhysicalDrive0"}),StatusCode::BAD_REQUEST),
        ("POST","/api/burner/cancel",json!({"id":"unknown"}),StatusCode::BAD_REQUEST),
    ] {
        let request=Request::builder().method(method).uri(path).header("host","127.0.0.1")
            .header("authorization",format!("Bearer {}",engine.token)).header("content-type","application/json").body(Body::from(body.to_string())).unwrap();
        let response=app.clone().oneshot(request).await.unwrap();assert_eq!(response.status(),expected,"{path}");
        let data:Value=serde_json::from_slice(&to_bytes(response.into_body(),100_000).await.unwrap()).unwrap();
        if method=="GET" {assert_eq!(data["physical_writes"],false);assert_eq!(data["device_enumeration"],false);assert_eq!(data["history"],json!([]));}
        else {assert!(data["error"].is_string());}
    }
    let request=Request::builder().uri("/api/burner").header("host","127.0.0.1").body(Body::empty()).unwrap();
    assert_eq!(app.oneshot(request).await.unwrap().status(),StatusCode::UNAUTHORIZED);
    assert_eq!(engine.burner.status()["history"],json!([]));
    // Reading saved status never collects telemetry. The platform collection
    // block is specifically Windows; every fresh profile must disable polling.
    assert_eq!(engine.health.status().unwrap()["settings"]["interval_minutes"],0);
    #[cfg(windows)]
    assert_eq!(engine.health.status().unwrap()["probe_available"],false);
}
