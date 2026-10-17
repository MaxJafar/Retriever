use retriever_engine::{config::Config,engine::Engine,paths};
use serde_json::json;
use std::fs;

#[test]
fn system_crawl_boundaries_are_cross_platform_and_cannot_be_included(){
    let mut device=Config::default().devices[0].clone();device.platform="windows".into();device.roots=vec!["C:/".into()];
    for path in ["C:/Windows/System32/config.txt","c:/PROGRAM FILES/a.txt","C:/ProgramData/report.txt","C:/Users/A/AppData/Local/a.txt","C:/System Volume Information/a.txt","C:/Recovery/a.txt","C:/Windows. /a.txt","C:/Users/A/.ssh/id_rsa","C:/Users/A/passwords.txt"]{assert!(!paths::eligible(&device,path),"{path}");}
    assert!(paths::eligible(&device,"C:/Users/A/Documents/report.txt"));
    assert!(paths::eligible(&device,r"\\?\C:\Users\A\Documents\report.txt"));
    assert!(!paths::eligible(&device,r"\\?\C:\Windows\report.txt"));
    device.platform="macos".into();device.roots=vec!["/".into()];
    for path in ["/System/Volumes/Data/report.txt","/Applications/App/report.txt","/private/var/report.txt","/Users/a/Library/report.txt","/Volumes/Backup/System/file.txt","/dev/disk0","/usr/local/file.txt","/Users/a/.config/a.txt"]{assert!(!paths::eligible(&device,path),"{path}");}
    assert!(paths::eligible(&device,"/Users/a/Documents/report.txt"));
    assert!(paths::eligible(&device,"/Volumes/Archive/Projects/report.txt"));
}

#[tokio::test]
async fn tree_is_paged_directory_only_and_does_not_crawl(){
    let temp=tempfile::Builder::new().prefix("retriever-tree-").tempdir().unwrap();let root=temp.path().join("volume");fs::create_dir(&root).unwrap();
    for i in 0..105 {fs::create_dir(root.join(format!("folder-{i:03}"))).unwrap();}
    fs::write(root.join("file.txt"),"never read").unwrap();let state=root.join("private-state");let engine=Engine::open(&state).unwrap();
    let d=engine.config.read().devices[0].id.clone();let revision=engine.config.read().revision();
    let first=engine.explorer_children(&json!({"device":d,"path":root})).await.unwrap();assert_eq!(first["total"],105);assert_eq!(first["entries"].as_array().unwrap().len(),100);assert_eq!(first["entries"][0]["name"],"folder-000");assert_eq!(first["next_offset"],100);
    let last=engine.explorer_children(&json!({"device":d,"path":root,"offset":100})).await.unwrap();assert_eq!(last["entries"].as_array().unwrap().len(),5);assert!(last["next_offset"].is_null());assert_eq!(last["crawling_started"],false);assert_eq!(last["model_calls"],0);
    assert!(engine.explorer_children(&json!({"device":d,"path":state})).await.is_err());assert!(engine.explorer_children(&json!({"device":d,"path":root,"offset":100001})).await.is_err());
    assert_eq!(engine.config.read().revision(),revision);assert_eq!(engine.status().unwrap()["files"],0);
}

#[tokio::test]
async fn crawl_review_is_read_only_and_new_roots_start_manual(){
    let temp=tempfile::Builder::new().prefix("retriever-setup-").tempdir().unwrap();let root=temp.path().join("sources");fs::create_dir(&root).unwrap();fs::write(root.join("document.txt"),"untouched").unwrap();
    let engine=Engine::open(&temp.path().join("state")).unwrap();let d=engine.config.read().devices[0].id.clone();
    let plan=engine.crawl_preview(&json!({"device":d,"root":root})).await.unwrap();assert_eq!(plan["files_read"],0);assert_eq!(plan["initial_schedule"],"manual");assert_eq!(plan["initial_analysis"],"off");assert!(engine.config.read().devices[0].roots.is_empty());
    let mut cfg=engine.config.read().clone();cfg.devices[0].roots.push(root.to_string_lossy().into());cfg.configured=true;engine.configure(serde_json::to_value(&cfg).unwrap(),Some(plan["revision"].as_str().unwrap())).unwrap();
    let folder=engine.crawl_status().unwrap()["folders"][0].clone();assert_eq!(folder["interval_minutes"],0);assert!(engine.due_folder(retriever_engine::now()+10000.).unwrap().is_none());
    engine.crawl_settings(&json!({"device":d,"root":root,"revision":folder["revision"],"interval_minutes":60,"analysis":"off"})).unwrap();
    engine.configure(serde_json::to_value(&cfg).unwrap(),None).unwrap();assert_eq!(engine.crawl_status().unwrap()["folders"][0]["interval_minutes"],60);
    cfg.devices[0].roots.clear();engine.configure(serde_json::to_value(&cfg).unwrap(),None).unwrap();cfg.devices[0].roots.push(root.to_string_lossy().into());engine.configure(serde_json::to_value(&cfg).unwrap(),None).unwrap();assert_eq!(engine.crawl_status().unwrap()["folders"][0]["interval_minutes"],0);
    assert_eq!(fs::read_to_string(root.join("document.txt")).unwrap(),"untouched");assert_eq!(engine.status().unwrap()["files"],0);
}

#[cfg(unix)]
#[tokio::test]
async fn tree_links_are_never_followed(){
    let temp=tempfile::Builder::new().prefix("retriever-link-").tempdir().unwrap();let root=temp.path().join("files");fs::create_dir(&root).unwrap();std::os::unix::fs::symlink(&root,root.join("loop")).unwrap();let engine=Engine::open(&temp.path().join("state")).unwrap();let d=engine.config.read().devices[0].id.clone();assert_eq!(engine.explorer_children(&json!({"device":d,"path":root})).await.unwrap()["total"],0);assert!(engine.crawl_preview(&json!({"device":d,"root":root.join("loop")})).await.is_err());
}
