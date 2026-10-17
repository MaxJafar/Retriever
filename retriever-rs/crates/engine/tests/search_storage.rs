use retriever_engine::{engine::Engine,store::Store,vectors};
use rusqlite::params;

#[tokio::test]
async fn filename_cache_survives_a_library_sized_query_set_and_invalidates_on_change() {
    let dir=tempfile::tempdir().unwrap();let engine=Engine::open(dir.path()).unwrap();
    for i in 0..68 {
        let value=engine.search(&format!("case-{i}.txt"),"","","exact","instant").await.unwrap();
        assert_eq!(value["routing"]["complete"],true);
    }
    assert_eq!(engine.search("case-0.txt","","","exact","instant").await.unwrap()["routing"]["response_cached"],true);
    engine.store.db.lock().execute("UPDATE revision SET value=value+1 WHERE id=1",[]).unwrap();
    assert_ne!(engine.search("case-0.txt","","","exact","instant").await.unwrap()["routing"]["response_cached"],true);
}

#[test]
fn repeated_snapshot_publication_adds_no_files_and_legacy_export_keeps_scores_identical() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();
    {
        let db=store.db.lock();
        for id in 1..=2 {
            db.execute("INSERT INTO files(id,device,path,name,type,extension,text,detail,size,mtime_ns,digest,indexed_at) VALUES(?,'local',?,?,'text','txt','test','',4,0,'digest',0)",params![id,format!("/file-{id}.txt"),format!("file-{id}.txt")]).unwrap();
            db.execute("INSERT INTO passages(id,file_id,ordinal,digest,text,coordinates,extraction_revision) VALUES(?,?,0,'digest','test','{}','test')",params![id,id]).unwrap();
            let vector=if id==1 {[1f32,0f32]}else{[0f32,1f32]};
            db.execute("INSERT INTO vectors VALUES(?,'test',?,2)",params![id,vector.into_iter().flat_map(f32::to_le_bytes).collect::<Vec<_>>()]).unwrap();
        }
    }
    let id=vectors::publish(&store,"test").unwrap().unwrap();
    let count=std::fs::read_dir(dir.path().join("vectors")).unwrap().count();
    assert_eq!(vectors::publish(&store,"test").unwrap().unwrap(),id);
    assert_eq!(std::fs::read_dir(dir.path().join("vectors")).unwrap().count(),count);
    let before=vectors::Snapshot::load(&store,"test").unwrap().unwrap().score_files(&[1.,0.],2,None).unwrap();
    vectors::export_legacy_snapshots(dir.path()).unwrap();
    let after=vectors::Snapshot::load(&store,"test").unwrap().unwrap().score_files(&[1.,0.],2,None).unwrap();
    assert_eq!(before.iter().map(|(r,s)|(r.passage,s.to_bits())).collect::<Vec<_>>(),after.iter().map(|(r,s)|(r.passage,s.to_bits())).collect::<Vec<_>>());
    assert_eq!(before[0].0.file,1);
}
