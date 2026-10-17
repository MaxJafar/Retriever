use retriever_engine::{engine::Engine,explorer,paths};
use serde_json::json;
use std::fs;

#[tokio::test]
async fn interactive_browsing_does_not_expand_crawl_or_expose_state(){
    let temp=tempfile::tempdir().unwrap();let root=temp.path().join("sources");fs::create_dir(&root).unwrap();fs::write(root.join("visible.txt"),"fixture").unwrap();fs::write(root.join(".hidden"),"fixture").unwrap();
    let engine=Engine::open(&temp.path().join("state")).unwrap();let device=engine.config.read().devices[0].clone();
    assert!(!paths::eligible(&device,root.to_str().unwrap()));
    let result=engine.explorer_browse(&json!({"device":device.id,"path":root})).await.unwrap();assert_eq!(result["total"],2);assert_eq!(result["crawling_started"],false);assert_eq!(engine.status().unwrap()["files"],0);
    assert!(engine.explorer_browse(&json!({"device":device.id,"path":engine.state})).await.is_err());
    assert!(engine.explorer_browse(&json!({"device":device.id,"path":root.join("../state")})).await.is_err());
    assert!(engine.browse(&json!({"device":device.id,"path":root})).await.is_err());
    assert!(explorer::checked(r"\\.\PhysicalDrive0",&[]).is_err());
    assert!(explorer::checked(r"\\?\GLOBALROOT\Device\Harddisk0",&[]).is_err());
    let path=root.join("visible.txt");
    assert!(engine.metadata(&device.id,path.to_str().unwrap()).is_err());
    let data=engine.annotate(&json!({"device":device.id,"path":path,"scope":"filesystem","tags":["Review"],"revision":0})).unwrap();
    assert_eq!(data["tags"][0],"Review");
}

#[tokio::test]
async fn explicit_filesystem_batch_preserves_review_and_no_overwrite(){
    let temp=tempfile::tempdir().unwrap();let root=temp.path().join("files");fs::create_dir(&root).unwrap();fs::write(root.join("one.txt"),"one").unwrap();fs::write(root.join("two.txt"),"two").unwrap();
    let engine=Engine::open(&temp.path().join("state")).unwrap();let d=engine.config.read().devices[0].clone();
    let body=json!({"device":d.id,"scope":"filesystem","operation":"rename","items":[{"path":root.join("one.txt"),"name":"first.txt"},{"path":root.join("two.txt"),"name":"second.txt"}]});
    let plan=engine.operation_plan(&body).await.unwrap();assert!(root.join("one.txt").exists());
    let result=engine.apply_operation(plan["id"].as_str().unwrap()).await.unwrap();assert_eq!(result["state"],"applied");assert_eq!(fs::read_to_string(root.join("first.txt")).unwrap(),"one");
    assert!(engine.apply_operation(plan["id"].as_str().unwrap()).await.is_err());
    let undo=engine.undo_plan(plan["id"].as_str().unwrap()).await.unwrap();assert_eq!(engine.apply_operation(undo["id"].as_str().unwrap()).await.unwrap()["state"],"applied");
    assert!(engine.operation_plan(&json!({"device":d.id,"scope":"filesystem","operation":"rename","items":[{"path":root.join("one.txt"),"name":"two.txt"}]})).await.is_err());
    let destination=root.join("copies");fs::create_dir(&destination).unwrap();
    let copy=engine.operation_plan(&json!({"device":d.id,"target_device":d.id,"scope":"filesystem","operation":"copy","parent":destination,"items":[{"path":root.join("one.txt")}]})).await.unwrap();
    assert_eq!(engine.apply_operation(copy["id"].as_str().unwrap()).await.unwrap()["state"],"applied");assert_eq!(fs::read_to_string(destination.join("one.txt")).unwrap(),"one");
    assert_eq!(engine.status().unwrap()["files"],0);
}

#[test]
fn archive_and_assistant_do_not_fabricate_models_or_drive_health(){
    let temp=tempfile::tempdir().unwrap();let engine=Engine::open(temp.path()).unwrap();
    let archive=engine.archive_catalogue().unwrap();assert_eq!(archive["files"],0);
    let data:serde_json::Value=serde_json::from_slice(&fs::read(temp.path().join("rag-archive/catalogue.json")).unwrap()).unwrap();assert_eq!(data["contains_source_text"],false);
    let status=engine.assistant_status().unwrap();assert_eq!(status["health_collection"],"blocked");assert_eq!(status["models"].as_array().unwrap().len(),2);assert!(status["models"].as_array().unwrap().iter().all(|m|m["uptime_seconds"].is_null()&&m["enabled"]==true&&m["available"]==false));
}

#[test]
fn remote_storage_is_bounded_metadata_and_excludes_private_state(){
    let temp=tempfile::tempdir().unwrap();let state=temp.path().join("private");fs::create_dir(&state).unwrap();fs::write(state.join("secret.txt"),"private").unwrap();fs::write(temp.path().join("small.txt"),"123").unwrap();fs::write(temp.path().join("large.txt"),"123456789").unwrap();
    let device=retriever_engine::config::Config::default().devices[0].clone();
    let value=retriever_engine::remote::storage_snapshot(&device,&json!({"path":temp.path()}),&[state]).unwrap();
    assert_eq!(value["logical_bytes"],12);assert_eq!(value["files"],2);assert_eq!(value["largest"][0]["name"],"large.txt");assert_eq!(value["skipped"],1);assert_eq!(value["complete"],false);assert_eq!(value["health_collection"],"blocked");assert_eq!(value["model_calls"],0);
    let mut disabled=device;disabled.enabled=false;assert!(retriever_engine::remote::storage_snapshot(&disabled,&json!({"path":temp.path()}),&[]).is_err());
}

#[test]
fn agent_path_is_portable_without_shell_expansion(){
    let mut cfg=retriever_engine::config::Config::default();cfg.devices[0].platform="windows".into();
    cfg.devices[0].agent="C:/Users/Test User/AppData/Local/Retriever Agent/bin/retriever-agent.exe".into();assert!(cfg.validate().is_ok());
    for path in ["C:/a&calc.exe","C:/%PATH%/a.exe","C:/a\tb.exe","A€","C:/a$()/b.exe","C:/a`command`","C:/a\"b.exe"]{cfg.devices[0].agent=path.into();assert!(cfg.validate().is_err(),"{path}");}
    cfg.devices[0].platform="macos".into();cfg.devices[0].agent="/Users/Test User/.local/bin/retriever-agent".into();assert!(cfg.validate().is_ok());
}
