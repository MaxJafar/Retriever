use retriever_engine::{config::Config, engine::Engine};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{Ordering},
    },
    time::Duration,
};

struct Fixture {
    dir: tempfile::TempDir,
    engine: Arc<Engine>,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("retriever-crawl-")
            .tempdir()
            .unwrap();
        let files = dir.path().join("files");
        std::fs::create_dir(&files).unwrap();
        let engine = Engine::open(&dir.path().join("state")).unwrap();
        let mut cfg = Config::default();
        cfg.configured = true;
        cfg.devices[0].roots = vec![files.canonicalize().unwrap().to_string_lossy().into()];
        engine
            .configure(serde_json::to_value(cfg).unwrap(), None)
            .unwrap();
        Self { dir, engine }
    }
    fn file(&self, name: &str, text: &str) -> String {
        let path = std::path::Path::new(&self.engine.config.read().devices[0].roots[0]).join(name);
        std::fs::write(&path, text).unwrap();
        path.to_string_lossy().into()
    }
    async fn wait(&self) {
        tokio::time::timeout(Duration::from_secs(15), async {
            while self.engine.syncing.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_ne!(
            self.engine.progress.read()["phase"],
            "failed",
            "{}",
            self.engine.progress.read()
        );
    }
    async fn sync(&self, index_only: bool) {
        self.engine.start_sync(index_only).unwrap();
        self.wait().await;
    }
    fn folder(&self) -> Value {
        self.engine.crawl_status().unwrap()["folders"][0].clone()
    }
    fn settings(&self, mut f: Value, minutes: i64, analysis: &str) {
        f["interval_minutes"] = json!(minutes);
        f["analysis"] = json!(analysis);
        self.engine.crawl_settings(&f).unwrap();
    }
}

#[tokio::test]
async fn explicit_meaning_mode_does_not_silently_return_word_matches() {
    let f = Fixture::new();
    f.file("gardening.md", "Growing tomatoes in a greenhouse");
    f.sync(false).await;
    assert_eq!(
        f.engine
            .search("tomatoes", "", "", "exact", "full")
            .await
            .unwrap()["results"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        f.engine
            .search("tomatoes", "", "", "semantic", "full")
            .await
            .unwrap()["results"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn exact_filename_and_path_lookup_is_scoped_and_avoids_content_search() {
    let f = Fixture::new();
    let path = f.file("Annual Plan 2026.md", "unrelated body");
    f.file("references.md", "Annual Plan 2026.md Annual Plan 2026.md");
    f.sync(false).await;
    for query in ["annual plan 2026.MD".to_owned(), path.replace('\\', "/")] {
        let result = f
            .engine
            .search(&query, "", "", "hybrid", "full")
            .await
            .unwrap();
        assert_eq!(result["results"].as_array().unwrap().len(), 1);
        assert_eq!(result["results"][0]["name"], "Annual Plan 2026.md");
        assert_eq!(result["results"][0]["retrieval"], "filename");
        assert_eq!(result["routing"]["route"], "words");
    }
    assert!(
        f.engine
            .search("Annual Plan 2026.md", "nonexistent", "", "hybrid", "full")
            .await
            .unwrap()["results"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    f.engine.config.write().devices[0].exclude.push(path);
    let result = f
        .engine
        .search("Annual Plan 2026.md", "", "", "hybrid", "full")
        .await
        .unwrap();
    assert!(
        result["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["name"] != "Annual Plan 2026.md")
    );
}

#[tokio::test]
async fn lexical_cache_invalidates_on_content_exclusion_and_external_commits() {
    let f = Fixture::new();
    let path = f.file("garden.md", "tomatoes in a greenhouse");
    f.sync(false).await;
    let cfg = f.engine.config.read().clone();
    for _ in 0..2 {
        assert_eq!(
            f.engine
                .store
                .lexical("tomatoes", 30, &cfg, "", "")
                .unwrap()
                .len(),
            1
        );
    }
    let mut excluded = cfg.clone();
    excluded.devices[0].exclude.push(path.clone());
    assert!(
        f.engine
            .store
            .lexical("tomatoes", 30, &excluded, "", "")
            .unwrap()
            .is_empty()
    );
    let db = rusqlite::Connection::open(f.dir.path().join("state/retriever.sqlite")).unwrap();
    db.execute(
        "UPDATE files SET text='new peppers',digest='changed' WHERE name='garden.md'",
        [],
    )
    .unwrap();
    assert!(
        f.engine
            .store
            .lexical("tomatoes", 30, &cfg, "", "")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        f.engine
            .store
            .lexical("peppers", 30, &cfg, "", "")
            .unwrap()
            .len(),
        1
    );
    f.engine
        .store
        .db
        .lock()
        .execute("DELETE FROM files", [])
        .unwrap();
    assert!(
        f.engine
            .store
            .lexical("peppers", 30, &cfg, "", "")
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn response_cache_respects_content_and_scope_changes() {
    let f = Fixture::new();
    let path = f.file("garden.md", "tomatoes in a greenhouse");
    f.sync(false).await;
    let first = f
        .engine
        .search("tomatoes", "", "", "exact", "full")
        .await
        .unwrap();
    let repeated = f
        .engine
        .search("tomatoes", "", "", "exact", "full")
        .await
        .unwrap();
    assert_eq!(first["results"], repeated["results"]);
    assert_eq!(repeated["routing"]["response_cached"], true);
    let mut excluded = f.engine.config.read().clone();
    excluded.devices[0].exclude.push(path);
    *f.engine.config.write() = excluded;
    assert!(
        f.engine
            .search("tomatoes", "", "", "exact", "full")
            .await
            .unwrap()["results"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    f.engine.config.write().devices[0].exclude.clear();
    assert_eq!(
        f.engine
            .search("tomatoes", "", "", "exact", "full")
            .await
            .unwrap()["results"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let db = rusqlite::Connection::open(f.dir.path().join("state/retriever.sqlite")).unwrap();
    db.execute(
        "UPDATE files SET text='new peppers',digest='changed' WHERE name='garden.md'",
        [],
    )
    .unwrap();
    assert!(
        f.engine
            .search("tomatoes", "", "", "exact", "full")
            .await
            .unwrap()["results"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn generated_matches_compete_with_source_matches_at_their_own_rank() {
    let f = Fixture::new();
    for n in 0..10 {
        f.file(&format!("incidental{n}.md"), "world tool configuration");
    }
    f.file("story.md", "Замки и гильдии");
    f.sync(false).await;
    f.engine.store.db.lock().execute("INSERT INTO file_analysis SELECT id,'fixture','medieval world castles magic guilds','fiction','[]','{}',0 FROM files WHERE name='story.md'", []).unwrap();
    let cfg = f.engine.config.read().clone();
    let result = f
        .engine
        .store
        .lexical("medieval world castles magic guilds", 30, &cfg, "", "")
        .unwrap();
    assert!(result.iter().position(|r| r["name"] == "story.md").unwrap() < 3);
    assert_eq!(result.iter().filter(|r| r["name"] == "story.md").count(), 1);
}

#[tokio::test]
async fn unreadable_text_remains_findable_and_privacy_skips_do_not_poison_folder_completion() {
    let f = Fixture::new();
    let log = f.file("capture.log", "readable before corruption");
    f.sync(false).await;
    std::fs::write(&log, b"binary\0\xfflog").unwrap();
    f.file("notes.txt", "api_key = abcdefgh123456789");
    f.sync(false).await;
    assert_eq!(f.folder()["report"]["complete"], true);
    let result = f
        .engine
        .search("capture.log", "", "", "exact", "full")
        .await
        .unwrap();
    assert_eq!(result["results"].as_array().unwrap().len(), 1);
    assert!(
        result["results"][0]["detail"]
            .as_str()
            .unwrap()
            .contains("extraction_failed")
    );
    assert!(
        f.engine
            .search("readable before corruption", "", "", "exact", "full")
            .await
            .unwrap()["results"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(f.engine.status().unwrap()["files"], 1);
}

#[tokio::test]
async fn incremental_crawl_reuses_passages_but_detects_changed_bytes_with_preserved_metadata() {
    let f = Fixture::new();
    let path = f.file("guide.md", "golden dog");
    f.sync(false).await;
    let ids: Vec<(i64, i64, String)> = {
        let db = f.engine.store.db.lock();
        let mut s=db.prepare("SELECT f.id,p.id,p.extraction_revision FROM files f JOIN passages p ON p.file_id=f.id").unwrap();
        s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    f.sync(false).await;
    assert_eq!(f.folder()["report"]["unchanged"], 1);
    assert_eq!(f.folder()["report"]["changed"], 0);
    {
        let db = f.engine.store.db.lock();
        let current:(i64,i64,String)=db.query_row("SELECT f.id,p.id,p.extraction_revision FROM files f JOIN passages p ON p.file_id=f.id",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(current, ids[0]);
    }
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::fs::write(&path, "silver cat").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    f.sync(false).await;
    assert_eq!(f.folder()["report"]["changed"], 1);
    assert_eq!(
        f.engine
            .search("silver", "", "", "exact", "full")
            .await
            .unwrap()["results"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let mut cfg = f.engine.config.read().clone();
    cfg.search["embedding"]["chunk_chars"] = json!(256);
    f.engine
        .configure(serde_json::to_value(cfg).unwrap(), None)
        .unwrap();
    f.sync(false).await;
    assert_eq!(f.folder()["report"]["changed"], 1);
}

#[tokio::test]
async fn folder_schedules_persist_validate_revisions_and_prune_only_the_selected_root() {
    let f = Fixture::new();
    let a = f.file("one.md", "firstroot evidence");
    let other = f.dir.path().join("second");
    std::fs::create_dir(&other).unwrap();
    std::fs::write(other.join("two.md"), "secondroot evidence").unwrap();
    let mut cfg = f.engine.config.read().clone();
    cfg.devices[0]
        .roots
        .push(other.canonicalize().unwrap().to_string_lossy().into());
    f.engine
        .configure(serde_json::to_value(cfg).unwrap(), None)
        .unwrap();
    let initial = f.folder();
    f.settings(initial.clone(), 0, "off");
    assert!(f.engine.crawl_settings(&initial).is_err());
    let mut invalid = f.folder();
    invalid["interval_minutes"] = json!(1);
    assert!(f.engine.crawl_settings(&invalid).is_err());
    assert!(f.engine.due_folder(retriever_engine::now()).unwrap().is_none());
    // Scheduling is explicit; including a new root alone must not start a crawl.
    let second=f.engine.crawl_status().unwrap()["folders"][1].clone();
    f.settings(second,60,"off");
    f.engine.store.db.lock().execute("UPDATE crawl_folders SET next_run=0 WHERE interval_minutes>0",[]).unwrap();
    let due = f
        .engine
        .due_folder(retriever_engine::now())
        .unwrap()
        .unwrap();
    assert_ne!(due.1, initial["root"].as_str().unwrap());
    f.sync(false).await;
    assert!(
        f.engine
            .due_folder(retriever_engine::now())
            .unwrap()
            .is_none()
    );
    std::fs::remove_file(a).unwrap();
    f.engine.start_folder(&initial).unwrap();
    f.wait().await;
    assert_eq!(f.engine.status().unwrap()["files"], 1);
    assert_eq!(
        f.engine
            .search("secondroot", "", "", "exact", "full")
            .await
            .unwrap()["results"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let reopened = Engine::open(&f.dir.path().join("state")).unwrap();
    assert_eq!(
        reopened.crawl_status().unwrap()["folders"][0]["interval_minutes"],
        0
    );
    assert!(
        reopened
            .due_folder(retriever_engine::now())
            .unwrap()
            .is_none()
    );
    let second = reopened.crawl_status().unwrap()["folders"][1].clone();
    std::fs::rename(&other, f.dir.path().join("offline")).unwrap();
    reopened.start_folder(&second).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while reopened.syncing.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let failed = reopened.crawl_status().unwrap()["folders"][1].clone();
    assert_eq!(failed["state"], "partial");
    assert!(failed["next_run"].as_f64().unwrap() > retriever_engine::now());
    assert_eq!(reopened.status().unwrap()["files"], 1);
}

#[tokio::test]
async fn inventory_enumerates_once_and_rejects_pruning_after_directory_changes() {
    let f = Fixture::new();
    for n in 0..205 {
        f.file(&format!("item{n:03}.md"), "Stable evidence");
    }
    let cfg = f.engine.config.read().clone();
    let d = &cfg.devices[0];
    let mut body = json!({"root":d.roots[0],"limits":cfg.limits,"chunk_chars":2000});
    let first = f
        .engine
        .source(d, "inventory_begin", body.clone())
        .await
        .unwrap();
    assert_eq!(first["total"], 205);
    body["session"] = first["session"].clone();
    f.file("new-after-enumeration.md", "later evidence");
    let mut collected = 0;
    loop {
        let page = f
            .engine
            .source(d, "inventory_page", body.clone())
            .await
            .unwrap();
        assert_eq!(page["total"], 205);
        let mut known = serde_json::Map::new();
        for r in page["records"].as_array().unwrap() {
            known.insert(
                r["path"].as_str().unwrap().into(),
                json!(retriever_engine::digest("Stable evidence")),
            );
        }
        body["known"] = Value::Object(known);
        let page = f
            .engine
            .source(d, "inventory_collect", body.clone())
            .await
            .unwrap();
        for r in page["records"].as_array().unwrap() {
            assert_eq!(r["unchanged"], true);
            assert!(r.get("extraction").is_none());
            collected += 1;
        }
        if page["next_offset"].is_null() {
            break;
        }
        body["offset"] = page["next_offset"].clone();
    }
    assert_eq!(collected, 205);
    let finish = f
        .engine
        .source(d, "inventory_finish", body.clone())
        .await
        .unwrap();
    assert_eq!(finish["complete"], false);
    assert_eq!(finish["tree_stable"], false);
    let mut changed = body.clone();
    changed["chunk_chars"] = json!(256);
    assert!(f.engine.source(d, "inventory_page", changed).await.is_err());
    f.engine
        .source(d, "inventory_close", body.clone())
        .await
        .unwrap();
    assert!(f.engine.source(d, "inventory_page", body).await.is_err());
    assert_eq!(
        std::fs::read_dir(f.engine.state.join("inventory"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn annotations_reject_unknown_citations_and_secret_output() {
    let evidence = vec![json!({"id":"42","text":"evidence"})];
    let mut answer =
        json!({"summary":"Description","category":"Notes","tags":[],"citations":["999"]});
    assert!(retriever_engine::knowledge::validate_annotation(&answer, &evidence).is_err());
    answer["citations"] = json!(["42"]);
    assert!(retriever_engine::knowledge::validate_annotation(&answer, &evidence).is_ok());
    answer["summary"] = json!("api_key=abcdefghijklmnop");
    assert!(retriever_engine::knowledge::validate_annotation(&answer, &evidence).is_err());
}

#[tokio::test]
async fn crawl_api_contract_is_shared_authenticated_and_revision_checked() {
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;
    let f = Fixture::new();
    let app = retriever_engine::http::router(f.engine.clone());
    let unauth = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/crawl")
                .header("host", "localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauth.status(), 401);
    let get = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/crawl")
                .header("host", "localhost")
                .header("authorization", format!("Bearer {}", f.engine.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get.status(), 200);
    let body: Value =
        serde_json::from_slice(&to_bytes(get.into_body(), 100000).await.unwrap()).unwrap();
    let mut folder = body["folders"][0].clone();
    folder["interval_minutes"] = json!(1440);
    for expected in [200, 409] {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/crawl/settings")
                    .method("POST")
                    .header("host", "localhost")
                    .header("authorization", format!("Bearer {}", f.engine.token))
                    .header("content-type", "application/json")
                    .body(Body::from(folder.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status().as_u16(), expected);
    }
    for asset in ["/crawl.js", "/crawl.css"] {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(asset)
                    .header("host", "localhost")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
    }
}

#[tokio::test]
async fn multilingual_embedding_chunks_fit_small_contexts_without_dropping_characters() {
    let f = Fixture::new();
    f.file("unicode.md", &"🐕".repeat(1000));
    f.sync(false).await;
    let db = f.engine.store.db.lock();
    let (chars,bytes,vectors):(i64,i64,i64)=db.query_row("SELECT sum(length(text)),max(length(CAST(text AS BLOB))),(SELECT count(*) FROM vectors) FROM passages",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(chars, 1000);
    assert!(bytes <= 1964);
    assert_eq!(vectors, 0);
}

#[tokio::test]
async fn cancellation_pauses_background_indexing_until_resumed() {
    let f = Fixture::new();
    f.file("public.md", "Public trip planning notes");
    let nested = std::path::Path::new(&f.engine.config.read().devices[0].roots[0]).join("personal");
    std::fs::create_dir(&nested).unwrap();
    std::fs::write(nested.join("journal.md"), "Private notes not opted into AI").unwrap();
    let mut cfg = f.engine.config.read().clone();
    cfg.devices[0].roots.push(nested.to_string_lossy().into());
    f.engine
        .configure(serde_json::to_value(cfg).unwrap(), None)
        .unwrap();
    f.sync(false).await;
    let notes:i64=f.engine.store.db.lock().query_row("SELECT count(*) FROM file_analysis a JOIN files f ON f.id=a.file_id WHERE f.name='journal.md'",[],|r|r.get(0)).unwrap();
    assert_eq!(notes, 0);
    f.engine.cancel_jobs().unwrap();
    let reopened = Engine::open(&f.dir.path().join("state")).unwrap();
    assert!(
        reopened
            .store
            .db
            .lock()
            .query_row("SELECT paused FROM background_state WHERE id=1", [], |r| {
                r.get::<_, bool>(0)
            })
            .unwrap()
    );
    f.sync(true).await;
    assert!(
        !f.engine
            .store
            .db
            .lock()
            .query_row("SELECT paused FROM background_state WHERE id=1", [], |r| {
                r.get::<_, bool>(0)
            })
            .unwrap()
    );
}
