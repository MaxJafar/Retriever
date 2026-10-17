use retriever_engine::{config::Config,engine::Engine,native_models::{NativeModels,laya_sequence},providers};
use serde_json::{Value,json};
use std::{fs,path::Path,sync::atomic::Ordering,time::Instant};

#[test]
fn native_defaults_and_external_models_are_rejected(){
    let cfg=Config::default();cfg.validate().unwrap();
    assert_eq!(cfg.search["embedding"]["provider"],"native");assert_eq!(cfg.search["relevance"]["provider"],"native");
    for role in ["vision","llm"]{assert_eq!(cfg.search[role]["provider"],"disabled");}
    for provider in ["openai","ollama","llamacpp","laya","systemone"]{
        let mut c=cfg.clone();c.search["embedding"]["provider"]=json!(provider);assert!(c.validate().is_err());
    }
    let mut b=cfg.search["embedding"].clone();b["batch_size"]=json!(16);
    assert_eq!(providers::fingerprint(&b),providers::fingerprint(&cfg.search["embedding"]));
    b["model_revision"]=json!("new-weights");assert_ne!(providers::fingerprint(&b),providers::fingerprint(&cfg.search["embedding"]));
}

#[test]
fn legacy_config_upgrades_without_writing_or_losing_folders(){
    let dir=tempfile::tempdir().unwrap();let mut cfg=Config::default();cfg.schema_version=1;
    cfg.devices[0].roots=vec![dir.path().to_string_lossy().into()];cfg.search["embedding"]["provider"]=json!("openai");
    cfg.search["embedding"]["base_url"]=json!("https://obsolete.invalid/v1");
    let before=serde_json::to_vec(&cfg).unwrap();fs::write(dir.path().join("config.json"),&before).unwrap();
    let migrated=Config::load(dir.path()).unwrap();assert_eq!(migrated.devices[0].roots,cfg.devices[0].roots);
    assert_eq!(migrated.search["embedding"]["provider"],"native");assert!(migrated.search["embedding"]["base_url"].is_null());
    assert_eq!(fs::read(dir.path().join("config.json")).unwrap(),before);
}

#[tokio::test]
async fn missing_models_keep_lexical_index_usable_and_no_generator(){
    let dir=tempfile::tempdir().unwrap();let docs=tempfile::Builder::new().prefix("retriever-docs-").tempdir().unwrap();fs::write(docs.path().join("travel.txt"),"Winter travel budget is 400 dollars.").unwrap();
    let engine=Engine::open(dir.path()).unwrap();let mut cfg=engine.config.read().clone();cfg.configured=true;cfg.devices[0].roots=vec![docs.path().to_string_lossy().into()];
    engine.configure(serde_json::to_value(cfg).unwrap(),None).unwrap();
    engine.start_sync(false).unwrap();while engine.syncing.load(Ordering::SeqCst){tokio::time::sleep(std::time::Duration::from_millis(25)).await;}
    let hits=engine.search("travel","","","exact","full").await.unwrap();assert_eq!(hits["results"].as_array().unwrap().len(),1);
    assert_eq!(engine.status().unwrap()["embedding"]["state"],"models_missing");
    assert!(engine.probe("llm",json!({"provider":"openai"})).await.is_err());
}

fn link_bundle(source:&Path,dest:&Path){
    fs::create_dir_all(dest).unwrap();
    for entry in fs::read_dir(source).unwrap(){let entry=entry.unwrap();let p=entry.path();let out=dest.join(entry.file_name());if p.is_dir(){link_bundle(&p,&out)}else{fs::hard_link(&p,&out).or_else(|_|fs::copy(&p,&out).map(|_|())).unwrap();}}
}

#[test]
fn native_chunks_preserve_unicode_without_overfragmenting_ascii(){
    for text in ["a".repeat(8000),"🐕".repeat(3000),"日本語 Azərbaycan café\n".repeat(400)] {
        let parts=retriever_engine::extract::passages(&text,1600,None);
        assert_eq!(parts.iter().map(|p|p.text.as_str()).collect::<String>(),text);
        assert!(parts.iter().all(|p|p.text.len()<=retriever_engine::extract::PASSAGE_BYTE_LIMIT));
        if text.is_ascii(){assert_eq!(parts.len(),5);}
        for pair in parts.windows(2){assert_eq!(pair[0].coordinates.char_end,pair[1].coordinates.char_start);}
    }
}

#[test]
fn bundle_install_is_checked_atomic_and_never_overwrites_existing_models(){
    let source=tempfile::tempdir().unwrap();let target=tempfile::tempdir().unwrap();
    let mut hashes=serde_json::Map::new();
    for name in ["embeddinggemma-2.gguf","laya/model.onnx","laya/tokenizer.json","laya/config.json",retriever_engine::native_models::runtime_name()] {
        let path=source.path().join(name);fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path,name).unwrap();hashes.insert(name.into(),json!(retriever_engine::digest(name)));
    }
    let mut manifest=json!({"version":1,"embedding_model":"embeddinggemma-2","relevance_model":"laya-multilingual","sha256":hashes});
    let save=|m:&Value|fs::write(source.path().join("bundle.json"),serde_json::to_vec(m).unwrap()).unwrap();
    save(&manifest);
    assert!(NativeModels::install_bundle(target.path(),source.path()).unwrap());
    assert!(!NativeModels::install_bundle(target.path(),source.path()).unwrap());
    let bad=tempfile::tempdir().unwrap();fs::write(source.path().join("laya/model.onnx"),"corrupt").unwrap();
    assert!(NativeModels::install_bundle(bad.path(),source.path()).is_err());
    assert!(!bad.path().join("models").exists());
    assert_eq!(fs::read_to_string(target.path().join("models/laya/model.onnx")).unwrap(),"laya/model.onnx");
    manifest["sha256"]["../escape"]=json!("0".repeat(64));save(&manifest);
    assert!(NativeModels::install_bundle(bad.path(),source.path()).is_err());
}

#[test]
fn imported_oversized_passages_rechunk_from_cache_with_source_coordinates(){
    use retriever_engine::extract::{Extraction,Passage};
    use retriever_protocol::Coordinates;
    let state=tempfile::tempdir().unwrap();let engine=Engine::open(state.path()).unwrap();
    let text="日本語の記録\n".repeat(800);
    let extraction=Extraction{text:text.clone(),detail:"text".into(),passages:vec![Passage{text:text.clone(),coordinates:Coordinates{page:Some(7),char_start:400,char_end:400+text.chars().count(),line_start:Some(20),line_end:Some(820),..Default::default()}}]};
    let file=engine.store.upsert_record("local","/synthetic/report.txt","cached-digest",text.len() as i64,0,"txt",&extraction,"fixture").unwrap();
    assert_eq!(engine.store.rechunk_cached_passages(&Config::default()).unwrap(),1);
    let db=engine.store.db.lock();let mut stmt=db.prepare("SELECT text,coordinates FROM passages WHERE file_id=? ORDER BY ordinal").unwrap();
    let rows:Vec<(String,String)>=stmt.query_map([file],|r|Ok((r.get(0)?,r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(rows.iter().map(|r|r.0.as_str()).collect::<String>(),text);
    for (part,coordinates) in &rows{assert!(part.len()<=retriever_engine::extract::PASSAGE_BYTE_LIMIT);let v:Value=serde_json::from_str(coordinates).unwrap();assert_eq!(v["page"],7);}
    let first:Value=serde_json::from_str(&rows[0].1).unwrap();let last:Value=serde_json::from_str(&rows.last().unwrap().1).unwrap();
    assert_eq!(first["char_start"],400);assert_eq!(first["line_start"],20);assert_eq!(last["char_end"],400+text.chars().count());
    drop(stmt);drop(db);assert_eq!(engine.store.rechunk_cached_passages(&Config::default()).unwrap(),0);
}

#[tokio::test]
#[ignore = "requires a prepared, local native model bundle; no downloads"]
async fn real_native_models_parity_retrieval_and_incremental_reuse(){
    let source=std::env::var_os("RETRIEVER_NATIVE_MODELS").expect("RETRIEVER_NATIVE_MODELS");
    let state=tempfile::tempdir().unwrap();link_bundle(Path::new(&source),&state.path().join("models"));
    let root=state.path().join("models");let fixtures:Vec<Value>=serde_json::from_slice(&fs::read(root.join("laya/parity.json")).unwrap()).unwrap();
    let tok=tokenizers::Tokenizer::from_file(root.join("laya/tokenizer.json")).unwrap();let cfg:Value=serde_json::from_slice(&fs::read(root.join("laya/config.json")).unwrap()).unwrap();
    let native=NativeModels::new(state.path()).unwrap();
    for f in &fixtures{
        let (ids,markers,_)=laya_sequence(&tok,&cfg,f["query"].as_str().unwrap(),&f["item"]).unwrap();
        assert_eq!(json!(ids),f["input_ids"]);assert_eq!(json!(markers),f["markers"]);
        let output=native.relevance(f["query"].as_str().unwrap().into(),vec![f["item"].clone()]).await.unwrap();
        assert!((output[0]["relevance"].as_f64().unwrap()-f["relevance"].as_f64().unwrap()).abs()<0.003);
    }
    drop(native);
    let docs=tempfile::Builder::new().prefix("retriever-docs-").tempdir().unwrap();
    let travel="The winter trip budget covers flights, hotel rooms and mountain hiking. Total travel cost is 400 dollars.";
    fs::write(docs.path().join("travel.txt"),travel).unwrap();fs::write(docs.path().join("duplicate.txt"),travel).unwrap();
    fs::write(docs.path().join("bread.txt"),"Bake bread with flour, water and yeast. Leave the dough to rise before placing it in the oven.").unwrap();
    fs::write(docs.path().join("database.txt"),"Back up the database every night and verify restoration of customer records.").unwrap();
    let engine=Engine::open(state.path()).unwrap();let mut cfg=engine.config.read().clone();cfg.configured=true;cfg.devices[0].roots=vec![docs.path().to_string_lossy().into()];
    engine.configure(serde_json::to_value(cfg).unwrap(),None).unwrap();
    let start=Instant::now();engine.start_sync(false).unwrap();while engine.syncing.load(Ordering::SeqCst){tokio::time::sleep(std::time::Duration::from_millis(25)).await;}
    let first_ms=start.elapsed().as_millis();let progress=engine.progress.read().clone();assert!(progress["embedding_warning"].is_null(),"{progress}");
    assert_eq!(engine.store.db.lock().query_row("SELECT count(*) FROM embedding_cache",[],|r|r.get::<_,i64>(0)).unwrap(),3);
    let requests=engine.providers.health.lock()["embedding"]["requests"].clone();
    let start=Instant::now();engine.start_sync(false).unwrap();while engine.syncing.load(Ordering::SeqCst){tokio::time::sleep(std::time::Duration::from_millis(25)).await;}
    let repeat_ms=start.elapsed().as_millis();assert_eq!(engine.providers.health.lock()["embedding"]["requests"],requests);
    let start=Instant::now();let result=engine.search("cost of a winter vacation","","","semantic","full").await.unwrap();let query_ms=start.elapsed().as_millis();
    assert_eq!(result["retrieval"]["semantic_ran"],true,"{result}");
    assert!(matches!(result["results"][0]["name"].as_str(),Some("travel.txt"|"duplicate.txt")),"{result}");
    let before=engine.providers.health.lock().clone();let _=engine.search("cost of a winter vacation","","","semantic","full").await.unwrap();assert_eq!(engine.providers.health.lock().clone(),before);
    // Reproduce an initial snapshot containing only an unrelated document.
    // Its first-place semantic rank must not outrank evidence in pending files.
    let fp=retriever_engine::providers::fingerprint(&engine.config.read().search["embedding"]);
    engine.store.db.lock().execute("DELETE FROM vectors WHERE passage_id IN(SELECT p.id FROM passages p JOIN files f ON f.id=p.file_id WHERE f.name<>'bread.txt')",[]).unwrap();
    retriever_engine::vectors::publish(&engine.store,&fp).unwrap();drop(engine);
    let engine=Engine::open(state.path()).unwrap();
    let partial=engine.search("winter trip budget","","","hybrid","full").await.unwrap();
    assert_eq!(partial["retrieval"]["partial_index"],true,"{partial}");
    assert!(matches!(partial["results"][0]["name"].as_str(),Some("travel.txt"|"duplicate.txt")),"{partial}");
    assert!(partial["results"][0]["snippet"].as_str().unwrap().contains("winter trip budget"));
    engine.start_sync(false).unwrap();wait_for_index(&engine).await;
    assert_eq!(engine.store.db.lock().query_row("SELECT count(*) FROM vectors",[],|r|r.get::<_,i64>(0)).unwrap(),4);
    // New identical content reuses persistent vectors, even in a new process/engine.
    drop(engine);fs::write(docs.path().join("third-copy.txt"),travel).unwrap();let engine=Engine::open(state.path()).unwrap();
    engine.start_sync(false).unwrap();while engine.syncing.load(Ordering::SeqCst){tokio::time::sleep(std::time::Duration::from_millis(25)).await;}
    assert!(engine.providers.health.lock().get("embedding").is_none(),"Duplicate file should need zero inference");
    assert_eq!(engine.store.db.lock().query_row("SELECT count(*) FROM vectors",[],|r|r.get::<_,i64>(0)).unwrap(),5);
    // Hold the scheduler after the source fingerprint but before inference.
    // A physical edit in that window must discard the file's vectors and text.
    let held=engine.providers.scheduler.acquire("local-retrieval",retriever_engine::scheduler::Priority::Interactive,std::time::Duration::from_secs(1)).await.unwrap();
    let changed=docs.path().join("changing.txt");fs::write(&changed,"Original microscope calibration evidence").unwrap();
    engine.start_sync(false).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(30),async {
        while !engine.providers.scheduler.status()["groups"].as_array().unwrap().iter().any(|g|g["queued"].as_u64().unwrap_or(0)>0){tokio::time::sleep(std::time::Duration::from_millis(10)).await;}
    }).await.unwrap();
    fs::write(&changed,"Changed while the model was waiting").unwrap();drop(held);
    wait_for_index(&engine).await;
    let stale:i64=engine.store.db.lock().query_row("SELECT count(*) FROM vectors v JOIN passages p ON p.id=v.passage_id JOIN files f ON f.id=p.file_id WHERE f.name='changing.txt'",[],|r|r.get(0)).unwrap();assert_eq!(stale,0);
    assert!(engine.search("microscope calibration","","","exact","full").await.unwrap()["results"].as_array().unwrap().is_empty());
    fs::remove_file(changed).unwrap();
    // Keep a cached record for an unavailable source, then index a later file.
    // Unavailable work must neither consume inference nor starve available work.
    let mut config=engine.config.read().clone();config.search["embedding"]["provider"]=json!("disabled");engine.configure(serde_json::to_value(&config).unwrap(),None).unwrap();
    let offline=docs.path().join("offline.txt");fs::write(&offline,"Telescope mirror manufacturing notes").unwrap();
    fs::write(docs.path().join("online.txt"),"Glacier measurements and seasonal ice melting observations").unwrap();
    engine.start_sync(false).unwrap();wait_for_index(&engine).await;fs::remove_file(offline).unwrap();
    config.search["embedding"]["provider"]=json!("native");engine.configure(serde_json::to_value(config).unwrap(),None).unwrap();
    let calls=engine.providers.health.lock()["embedding"]["requests"].as_u64().unwrap();
    engine.start_sync(true).unwrap();wait_for_index(&engine).await;
    assert_eq!(engine.providers.health.lock()["embedding"]["requests"].as_u64().unwrap()-calls,1);
    assert_eq!(engine.progress.read()["phase"],"partial");
    assert_eq!(engine.store.db.lock().query_row("SELECT count(*) FROM vectors v JOIN passages p ON p.id=v.passage_id JOIN files f ON f.id=p.file_id WHERE f.name='online.txt'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    println!("NATIVE_ACCEPTANCE {}",json!({"parity_cases":fixtures.len(),"first_index_ms":first_ms,"unchanged_index_ms":repeat_ms,"cold_query_ms":query_ms,"documents":5,"unique_vectors":3,"repeat_index_model_calls":0,"repeat_query_model_calls":0,"cross_restart_duplicate_model_calls":0}));
}

async fn wait_for_index(engine:&std::sync::Arc<Engine>){
    tokio::time::timeout(std::time::Duration::from_secs(180),async {
        while engine.syncing.load(Ordering::SeqCst){tokio::time::sleep(std::time::Duration::from_millis(25)).await;}
    }).await.unwrap();
    assert_ne!(engine.progress.read()["phase"],"failed","{}",engine.progress.read());
}
