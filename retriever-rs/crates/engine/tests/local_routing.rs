use retriever_engine::{config::Config, providers::Providers, scheduler::Priority};
use serde_json::json;
#[tokio::test]
async fn generation_and_external_model_requests_are_rejected_without_connections() {
    let directory=tempfile::tempdir().unwrap();
    let providers=Providers::new(directory.path()).unwrap();
    for role in ["llm","vision"] {
        let settings=json!({"provider":"openai","base_url":"http://127.0.0.1:9/v1","model":"obsolete"});
        assert!(providers.generate(role,&settings,"test",json!({}),Priority::Interactive).await.is_err());
    }
    let mut settings=Config::default().search["embedding"].clone();
    settings["base_url"]=json!("http://127.0.0.1:9/v1");
    assert!(providers.embed(&settings,vec![json!("test")],Priority::Interactive,768).await.is_err());
    assert!(providers.health.lock().is_empty());
    assert!(!providers.native.status("embedding")["runtime_started"].as_bool().unwrap());
}
