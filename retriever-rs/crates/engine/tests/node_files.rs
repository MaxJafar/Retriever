use retriever_engine::{agent,config::Config,engine::Engine,nodes,operations};
use serde_json::json;
use std::fs;

#[test]
fn discovery_is_presence_only_and_excludes_phones(){
    let view=nodes::discovered(&json!({"Self":{"HostName":"pc","OS":"windows","Online":true,"TailscaleIPs":["100.81.1.2"]},"Peer":{"a":{"HostName":"mac","OS":"macOS","Online":false,"TailscaleIPs":["100.81.1.3"]},"b":{"HostName":"phone","OS":"android","Online":true,"TailscaleIPs":["100.81.1.4"]}}}));
    assert_eq!(view["nodes"].as_array().unwrap().len(),2);assert_eq!(view["nodes"][0]["platform"],"macos");assert_eq!(view["nodes"][0]["online"],false);assert_eq!(view["discovery_only"],true);assert_eq!(view["health_collection"],false);
    let probe=agent::dispatch("probe",&json!({}),&[]).unwrap();assert_eq!(probe["node_protocol"],1);assert_eq!(probe["models_required"],false);
}
#[tokio::test]
async fn remote_style_trash_restores_and_never_overwrites(){
    let temp=tempfile::Builder::new().prefix("retriever-node-").tempdir().unwrap();let root=temp.path().join("files");fs::create_dir(&root).unwrap();let file=root.join("keep.txt");fs::write(&file,"fixture original").unwrap();
    let engine=Engine::open(&temp.path().join("state")).unwrap();let id=engine.config.read().devices[0].id.clone();
    let plan=engine.operation_plan(&json!({"device":id,"operation":"trash","scope":"filesystem","items":[{"path":file}]})).await.unwrap();
    assert!(file.exists());let receipt=engine.apply_operation(plan["id"].as_str().unwrap()).await.unwrap();assert_eq!(receipt["receipt"]["ok"],true);assert!(!file.exists());assert_eq!(engine.explorer_browse(&json!({"device":id,"path":root})).await.unwrap()["total"],0);
    let undo=engine.undo_plan(plan["id"].as_str().unwrap()).await.unwrap();fs::write(&file,"concurrent edit").unwrap();let conflict=engine.apply_operation(undo["id"].as_str().unwrap()).await.unwrap();assert_eq!(conflict["receipt"]["ok"],false);assert_eq!(fs::read_to_string(&file).unwrap(),"concurrent edit");
    fs::remove_file(&file).unwrap();let undo=engine.undo_plan(plan["id"].as_str().unwrap()).await.unwrap();assert_eq!(engine.apply_operation(undo["id"].as_str().unwrap()).await.unwrap()["receipt"]["ok"],true);assert_eq!(fs::read_to_string(&file).unwrap(),"fixture original");assert!(engine.apply_operation(plan["id"].as_str().unwrap()).await.is_err());
}
#[test]
fn trash_rejects_arbitrary_destinations_roots_and_changed_sources(){
    let temp=tempfile::Builder::new().prefix("retriever-trash-").tempdir().unwrap();let file=temp.path().join("keep.txt");fs::write(&file,"one").unwrap();let d=Config::default().devices.remove(0);
    assert!(operations::prepare(&d,&json!({"operation":"trash","scope":"filesystem","items":[{"path":file,"name":"../elsewhere"}]}),&[]).is_err());
    let pin=agent::pin(&file).unwrap();fs::write(&file,"changed longer").unwrap();assert!(operations::prepare(&d,&json!({"operation":"trash","scope":"filesystem","items":[{"path":file,"pin":pin}]}),&[]).is_err());
    let state=temp.path().join("state");fs::create_dir(&state).unwrap();assert!(operations::prepare(&d,&json!({"operation":"trash","scope":"filesystem","items":[{"path":temp.path()}]}),&[state]).is_err());
    assert!(operations::prepare(&d,&json!({"operation":"mkdir","scope":"filesystem","items":[{"parent":temp.path(),"name":"ordinary-folder"}]}),&[temp.path().join("state")]).is_ok());
}
