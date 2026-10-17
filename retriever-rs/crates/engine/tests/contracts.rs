use axum::{
        body::{Body, to_bytes},
    http::Request,
};
use retriever_engine::{
    config::Config,
    engine::Engine,
    extract, paths, providers,
    scheduler::{Priority, Scheduler},
    vectors,
};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{Ordering},
    },
    time::Duration,
};
use tower::ServiceExt;

struct Fixture {
    root: tempfile::TempDir,
    engine: Arc<Engine>,
}

fn office_archive(parts: &[(&str, &str)]) -> Vec<u8> {
    use std::io::Write;
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, content) in parts {
        archive
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(content.as_bytes()).unwrap();
    }
    archive.finish().unwrap().into_inner()
}

#[test]
fn office_preserves_cell_values_runs_and_relationship_order() {
    let spreadsheet = office_archive(&[
        (
            "xl/workbook.xml",
            r#"<workbook xmlns:r="relationships"><sheets><sheet name="First" r:id="r2"/><sheet name="Second" r:id="r1"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships><Relationship Id="r1" Type="office/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="r2" Type="office/worksheet" Target="worksheets/sheet2.xml"/></Relationships>"#,
        ),
        (
            "xl/sharedStrings.xml",
            "<sst><si><t>Unused string</t></si><si><r><t>Golden </t></r><r><t>retriever &amp; friends</t></r><rPh><t>phonetic hint</t></rPh></si></sst>",
        ),
        (
            "xl/worksheets/sheet1.xml",
            "<worksheet><sheetData><row><c t=\"b\"><v>1</v></c><c><f>SUM(A1)</f><v>42</v></c></row></sheetData></worksheet>",
        ),
        (
            "xl/worksheets/sheet2.xml",
            "<worksheet><sheetData><row><c t=\"s\"><v>1</v></c><c t=\"inlineStr\"><is><t>日本語</t></is></c></row></sheetData></worksheet>",
        ),
    ]);
    let result = extract::extract(
        std::path::Path::new("values.xlsx"),
        &spreadsheet,
        10000,
        500,
    )
    .unwrap();
    assert_eq!(
        result.text,
        "First\nGolden retriever & friends\t日本語\n\nSecond\nTRUE\t42\n\n"
    );
    assert!(!result.text.contains("Unused"));
    let slides = office_archive(&[
        (
            "ppt/presentation.xml",
            r#"<presentation xmlns:r="relationships"><sldIdLst><sldId id="256" r:id="b"/><sldId id="257" r:id="a"/></sldIdLst></presentation>"#,
        ),
        (
            "ppt/_rels/presentation.xml.rels",
            r#"<Relationships><Relationship Id="a" Type="office/slide" Target="slides/slide2.xml"/><Relationship Id="b" Type="office/slide" Target="slides/slide10.xml"/></Relationships>"#,
        ),
        (
            "ppt/slides/slide2.xml",
            "<slide><p><r><t>Second</t></r></p></slide>",
        ),
        (
            "ppt/slides/slide10.xml",
            "<slide><p><r><t>First</t></r></p></slide>",
        ),
    ]);
    assert_eq!(
        extract::extract(std::path::Path::new("slides.pptx"), &slides, 10000, 500)
            .unwrap()
            .text,
        "First\n\nSecond\n\n"
    );
    let document = office_archive(&[(
        "word/document.xml",
        "<document><p><r><t xml:space=\"preserve\">Golden </t></r><r><t>dog</t><tab/><t>runs</t><br/><t>Home</t></r><del><r><t>removed</t></r></del></p></document>",
    )]);
    assert_eq!(
        extract::extract(std::path::Path::new("doc.docx"), &document, 10000, 500)
            .unwrap()
            .text,
        "Golden dog\truns\nHome\n"
    );
}

#[test]
fn office_rejects_decoded_secrets_and_document_types() {
    for xml in [
        "<document><p><r><t>api_</t></r><r><t>key = abcdefgh123456</t></r></p></document>",
        "<!DOCTYPE document [<!ENTITY content SYSTEM 'file:///tmp/nope'>]><document><p><t>&content;</t></p></document>",
    ] {
        let bytes = office_archive(&[("word/document.xml", xml)]);
        assert!(extract::extract(std::path::Path::new("doc.docx"), &bytes, 10000, 500).is_err());
    }
}

#[test]
fn large_agent_responses_are_bounded_ordered_and_checksum_verified() {
    use retriever_protocol::{Response, ResponseCollector};
    let response = Response {
        version: 1,
        id: "long-document".into(),
        ok: true,
        data: Some(json!({"text":"日本語の文書\n".repeat(160000)})),
        error: None,
        fragment: None,
    };
    let mut bytes = Vec::new();
    retriever_protocol::write_response(&mut bytes, &response).unwrap();
    let frames = bytes.split_inclusive(|b| *b == b'\n').collect::<Vec<_>>();
    assert!(frames.len() > 2);
    assert!(
        frames
            .iter()
            .all(|f| f.len() <= retriever_protocol::MAX_MESSAGE)
    );
    let mut collector = ResponseCollector::new("long-document");
    for frame in &frames[..frames.len() - 1] {
        assert!(collector.push(frame).unwrap().is_none());
    }
    assert_eq!(
        collector
            .push(frames.last().unwrap())
            .unwrap()
            .unwrap()
            .data,
        response.data
    );
    assert!(collector.push(frames[0]).is_err());
    assert!(
        ResponseCollector::new("other-request")
            .push(frames[0])
            .is_err()
    );
    assert!(
        ResponseCollector::new("long-document")
            .push(frames[1])
            .is_err()
    );
    let mut repeated = ResponseCollector::new("long-document");
    repeated.push(frames[0]).unwrap();
    assert!(repeated.push(frames[0]).is_err());
    let mut damaged = ResponseCollector::new("long-document");
    for (i, frame) in frames.iter().enumerate() {
        let mut frame: Value = serde_json::from_slice(frame).unwrap();
        frame["fragment"]["sha256"] = json!("0".repeat(64));
        let mut encoded = serde_json::to_vec(&frame).unwrap();
        encoded.push(b'\n');
        if i + 1 == frames.len() {
            assert!(damaged.push(&encoded).is_err());
        } else {
            assert!(damaged.push(&encoded).unwrap().is_none());
        }
    }
}

#[tokio::test]
async fn scan_page_budgets_continue_without_truncating_the_catalogue() {
    let f = Fixture::new();
    for name in ["first.txt", "second.txt", "third.txt"] {
        f.file(name, &"x".repeat(800));
    }
    let mut cfg = f.engine.config.read().clone();
    cfg.limits["max_files"] = json!(1);
    cfg.limits["max_total_bytes"] = json!(1024);
    f.engine
        .configure(serde_json::to_value(cfg.clone()).unwrap(), None)
        .unwrap();
    let page = f
        .engine
        .source(&cfg.devices[0], "scan", json!({"limits":cfg.limits}))
        .await
        .unwrap();
    assert_eq!(page["records"].as_array().unwrap().len(), 1);
    assert_eq!(page["next_offset"], 1);
    assert_eq!(page["source_bytes"], 800);
    f.sync().await;
    assert_eq!(f.engine.status().unwrap()["files"], 3);
    assert!(
        f.engine
            .source(
                &cfg.devices[0],
                "scan",
                json!({"limits":cfg.limits,"offset":9})
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn long_multibyte_documents_and_changed_chunk_sizes_preserve_identity() {
    let f = Fixture::new();
    let text = format!("{}last evidence saffron", "日本語の文書\n".repeat(50000));
    let path = f.file("日本語.txt", &text);
    f.sync().await;
    let result = f
        .engine
        .search("saffron", "", "", "exact", "full")
        .await
        .unwrap();
    let id = result["results"][0]["id"].as_i64().unwrap();
    let prior: (i64, String) = f
        .engine
        .store
        .db
        .lock()
        .query_row(
            "SELECT count(*),extraction_revision FROM passages WHERE file_id=?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    f.engine.store.db.lock().execute("INSERT INTO vectors SELECT id,'old',x'0000803f00000000',2 FROM passages WHERE file_id=?", [id]).unwrap();
    let mut cfg = f.engine.config.read().clone();
    cfg.search["embedding"]["chunk_chars"] = json!(256);
    f.engine
        .configure(serde_json::to_value(cfg).unwrap(), None)
        .unwrap();
    f.sync().await;
    let next: (i64, String) = f
        .engine
        .store
        .db
        .lock()
        .query_row(
            "SELECT count(*),extraction_revision FROM passages WHERE file_id=?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(next.0 > prior.0);
    assert_ne!(next.1, prior.1);
    let vectors: i64 = f
        .engine
        .store
        .db
        .lock()
        .query_row("SELECT count(*) FROM vectors", [], |r| r.get(0))
        .unwrap();
    assert_eq!(vectors, 0);
    let stored = f
        .engine
        .store
        .result(id, &f.engine.config.read())
        .unwrap()
        .unwrap();
    assert_eq!(stored["path"], path);
    assert_eq!(stored["text"], text);
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix("retriever-test-")
            .tempdir()
            .unwrap();
        let source = root.path().join("files");
        std::fs::create_dir(&source).unwrap();
        let engine = Engine::open(&root.path().join("state")).unwrap();
        let mut cfg = Config::default();
        cfg.configured = true;
        cfg.devices[0].roots = vec![source.canonicalize().unwrap().to_string_lossy().into()];
        engine
            .configure(serde_json::to_value(cfg).unwrap(), None)
            .unwrap();
        Self { root, engine }
    }
    fn file(&self, name: &str, text: &str) -> String {
        let p = std::path::Path::new(&self.engine.config.read().devices[0].roots[0]).join(name);
        std::fs::write(&p, text).unwrap();
        p.to_string_lossy().into()
    }
    async fn sync(&self) {
        self.engine.start_sync(false).unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            while self.engine.syncing.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(10)).await;
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
}
#[tokio::test]
async fn unicode_long_documents_have_complete_searchable_coverage() {
    let f = Fixture::new();
    f.file(
        "İşlər-日本語.md",
        &format!(
            "{}\nfinal evidence marmalade",
            "Hello résumé\n".repeat(3000)
        ),
    );
    f.sync().await;
    let r = f
        .engine
        .search("marmalade", "", "", "exact", "full")
        .await
        .unwrap();
    assert_eq!(r["results"].as_array().unwrap().len(), 1);
    let db = f.engine.store.db.lock();
    let total:i64=db.query_row("SELECT sum(json_extract(coordinates,'$.char_end')-json_extract(coordinates,'$.char_start')) FROM passages",[],|r|r.get(0)).unwrap();
    let len: i64 = db
        .query_row("SELECT length(text) FROM files", [], |r| r.get(0))
        .unwrap();
    assert_eq!(total, len);
}
#[tokio::test]
async fn incomplete_scan_preserves_catalogue_and_successful_scan_prunes() {
    let f = Fixture::new();
    let p = f.file("guide.md", "golden dog");
    f.sync().await;
    std::fs::remove_file(p).unwrap();
    let original = f.engine.config.read().devices[0].roots[0].clone();
    f.engine.config.write().devices[0]
        .roots
        .push(f.root.path().join("missing").to_string_lossy().into());
    f.sync().await;
    assert_eq!(f.engine.status().unwrap()["files"], 1);
    f.engine.config.write().devices[0].roots = vec![original];
    f.sync().await;
    assert_eq!(f.engine.status().unwrap()["files"], 0);
}
#[tokio::test]
async fn immediate_exclusion_and_secret_replacement_remove_search_results() {
    let f = Fixture::new();
    f.file("plan.md", "public evidence");
    f.file("keep.md", "public record");
    f.sync().await;
    let mut cfg = f.engine.config.read().clone();
    cfg.devices[0].exclude.push("plan.md".into());
    f.engine
        .configure(serde_json::to_value(cfg).unwrap(), None)
        .unwrap();
    assert_eq!(f.engine.status().unwrap()["files"], 1);
    f.file("keep.md", "api_key = abcdefgh123456");
    f.sync().await;
    assert_eq!(f.engine.status().unwrap()["files"], 0);
}
#[test]
fn traversals_and_secret_paths_are_denied() {
    let f = Fixture::new();
    let d = f.engine.config.read().devices[0].clone();
    let root = &d.roots[0];
    for tail in [
        "../outside.md",
        ".env",
        "credentials.json",
        "node_modules/hello.js",
        "secrets.txt",
    ] {
        assert!(!paths::eligible(&d, &format!("{root}/{tail}")));
    }
    assert!(paths::eligible(&d, &format!("{root}/valid.txt")));
    assert!(paths::checked(&d, &format!("{root}/../outside.md"), &[], false).is_err());
}
#[test]
fn vector_validation_rejects_dimension_drift_zero_and_nonfinite() {
    for v in [
        vec![],
        vec![0.0, 0.0],
        vec![f32::NAN, 1.0],
        vec![f32::INFINITY, 1.0],
        vec![1.0],
    ] {
        assert!(vectors::normalize(&v, 2).is_err());
    }
    assert_eq!(vectors::normalize(&[3.0, 4.0], 2).unwrap(), vec![0.6, 0.8]);

}
#[test]
fn relevance_requires_exact_ids_and_valid_probabilities() {
    let items = vec![json!({"id":7})];
    for bad in [
        json!([]),
        json!([{"id":8,"relevance":1,"confidence":1,"truncated":false}]),
        json!([{"id":7,"relevance":2,"confidence":1,"truncated":false}]),
        json!([{"id":7,"relevance":0.8,"confidence":0.8}]),
    ] {
        assert!(providers::validate_decisions(&bad, &items).is_err());
    }
    assert!(
        providers::validate_decisions(
            &json!([{"id":7,"relevance":0.1,"confidence":0.3,"truncated":false}]),
            &items
        )
        .is_ok()
    );
}
#[tokio::test]
async fn scheduler_prioritizes_interactive_and_releases_cancelled_waiters() {
    let s = Scheduler::default();
    let held = s
        .acquire("mini", Priority::Background, Duration::from_secs(1))
        .await
        .unwrap();
    let background = s.clone();
    let b = tokio::spawn(async move {
        background
            .acquire("mini", Priority::Background, Duration::from_secs(2))
            .await
            .unwrap()
    });
    tokio::task::yield_now().await;
    let interactive = s.clone();
    let i = tokio::spawn(async move {
        interactive
            .acquire("mini", Priority::Interactive, Duration::from_secs(2))
            .await
            .unwrap()
    });
    tokio::task::yield_now().await;
    drop(held);
    let ip = tokio::time::timeout(Duration::from_millis(500), i)
        .await
        .unwrap()
        .unwrap();
    assert!(!b.is_finished());
    drop(ip);
    drop(b.await.unwrap());
    let held = s
        .acquire("mini", Priority::Background, Duration::from_secs(1))
        .await
        .unwrap();
    assert!(
        s.acquire("mini", Priority::Interactive, Duration::from_millis(10))
            .await
            .is_err()
    );
    drop(held);
    assert_eq!(s.status()["groups"][0]["queued"], 0);
    assert!(
        s.acquire("mini", Priority::Background, Duration::from_millis(50))
            .await
            .is_ok()
    );
}
#[tokio::test]
async fn independent_hosts_do_not_block_each_other() {
    let s = Scheduler::default();
    let _a = s
        .acquire("a", Priority::Background, Duration::from_secs(1))
        .await
        .unwrap();
    let _b = s
        .acquire("b", Priority::Background, Duration::from_millis(1))
        .await
        .unwrap();
    let a = json!({"base_url":"http://mini:1235/v1"});
    let b = json!({"base_url":"http://mini:1236/v1"});
    assert_eq!(providers::group(&a), providers::group(&b));
}
#[tokio::test]
async fn filters_reject_invalid_types_before_reading_even_empty_catalogues() {
    let f = Fixture::new();
    let bad = [
        json!({"match":1}),
        json!({"match":"all","extra":true}),
        json!({"rules":{}}),
        json!({"rules":[{"field":"name); DROP TABLE files","operator":"is","value":"x"}]}),
        json!({"rules":[{"field":"name","operator":"is","value":null}]}),
        json!({"rules":[{"field":"size","operator":"greater","value":"5"}]}),
        json!({"rules":[{"field":"size","operator":"contains","value":5}]}),
        json!({"rules":[{"field":"modified","operator":"is","value":-1}]}),
        json!({"rules":[{"field":"starred","operator":"is","value":1}]}),
        json!({"rules":[{"field":"tags","operator":"starts_with","value":"tag"}]}),
        json!({"rules":[{"field":"name","operator":"is","value":"x","extra":false}]}),
    ];
    for filters in &bad {
        assert!(
            f.engine.library(&json!({"filters":filters})).is_err(),
            "Accepted {filters}"
        );
        assert!(
            f.engine
                .save_view(&json!({"name":"Invalid","filters":filters}))
                .await
                .is_err()
        );
    }
    f.file("A.txt", "hello");
    f.sync().await;
    let numeric = json!({"rules":[{"field":"size","operator":"is","value":5.0}]});
    assert_eq!(
        f.engine.library(&json!({"filters":numeric})).unwrap()["total"],
        1
    );
    assert_eq!(f.engine.library(&json!({"filters":{"rules":[{"field":"name","operator":"is","value":"x' OR 1=1 --"}]}})).unwrap()["total"], 0);
    assert!(
        f.engine
            .library(&json!({"query":"no matching rows","filters":bad[0]}))
            .is_err()
    );
    assert_eq!(f.engine.library(&json!({})).unwrap()["total"], 1);
}

#[tokio::test]
async fn secret_replacement_cannot_fall_back_to_cached_preview() {
    let f = Fixture::new();
    let path = f.file("public.md", "Old public passage");
    f.sync().await;
    std::fs::write(&path, "api_key = abcdefgh123456").unwrap();
    assert!(
        f.engine
            .preview(&json!({"device":"local","path":path}))
            .await
            .is_err()
    );
    let result = f
        .engine
        .search("Old public passage", "", "", "exact", "full")
        .await
        .unwrap();
    assert!(result["results"].as_array().unwrap().is_empty());
}
#[tokio::test]
async fn visual_preview_is_bounded_decoded_and_never_uses_a_model() {
    let f = Fixture::new();
    let path = f.file("preview.png", "");
    image::RgbImage::from_pixel(40, 30, image::Rgb([240, 180, 35]))
        .save(&path)
        .unwrap();
    f.sync().await;
    let result = f
        .engine
        .preview(&json!({"device":"local","path":path,"visual":true}))
        .await
        .unwrap();
    assert!(
        result["image"]
            .as_str()
            .unwrap()
            .starts_with("data:image/jpeg;base64,")
    );
    assert_eq!(result["coverage"], "Image preview");
    assert_eq!(result["offline"], false);
    assert!(!result["text"].as_str().unwrap().contains("PNG"));
    let plain = f
        .engine
        .preview(&json!({"device":"local","path":path}))
        .await
        .unwrap();
    assert!(
        plain.get("image").is_none(),
        "Property dialogs must not decode media"
    );
    std::fs::write(&path, "api_key = abcdefgh123456").unwrap();
    assert!(
        f.engine
            .preview(&json!({"device":"local","path":path,"visual":true}))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn quick_preview_marks_large_local_files_online_and_never_shows_office_bytes() {
    let f = Fixture::new();
    let text = "Preview passage. 日本語\n".repeat(45000);
    let path = f.file("large.txt", &text);
    let office = f.file("report.docx", "");
    std::fs::write(&office,office_archive(&[("word/document.xml","<w:document xmlns:w=\"w\"><w:body><w:p><w:r><w:t>Readable office preview</w:t></w:r></w:p></w:body></w:document>")])).unwrap();
    f.sync().await;
    let result = f
        .engine
        .preview(&json!({"device":"local","path":path}))
        .await
        .unwrap();
    assert_eq!(result["offline"], false);
    assert!(
        result["text"]
            .as_str()
            .unwrap()
            .starts_with("Preview passage")
    );
    let result = f
        .engine
        .preview(&json!({"device":"local","path":office,"visual":true}))
        .await
        .unwrap();
    assert!(
        result["text"]
            .as_str()
            .unwrap()
            .contains("Readable office preview")
    );
    assert!(!result["text"].as_str().unwrap().starts_with("PK"));
}
#[tokio::test]
async fn api_auth_origin_revision_and_static_contracts() {
    let f = Fixture::new();
    let app = retriever_engine::http::router(f.engine.clone());
    let request = |path: &str, token: bool| {
        let mut r = Request::builder()
            .uri(path)
            .header("Host", "127.0.0.1:18780");
        if token {
            r = r.header("Authorization", format!("Bearer {}", f.engine.token));
        }
        r.body(Body::empty()).unwrap()
    };
    assert_eq!(
        app.clone()
            .oneshot(request("/", false))
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        app.clone()
            .oneshot(request("/api/status", false))
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        app.clone()
            .oneshot(request("/api/status", true))
            .await
            .unwrap()
            .status(),
        200
    );
    let cfg = serde_json::to_string(&*f.engine.config.read()).unwrap();
    let r = Request::builder()
        .method("POST")
        .uri("/api/config")
        .header("Host", "127.0.0.1:18780")
        .header("Authorization", format!("Bearer {}", f.engine.token))
        .header("Content-Type", "application/json")
        .header("If-Match", "stale")
        .body(Body::from(cfg))
        .unwrap();
    assert_eq!(app.clone().oneshot(r).await.unwrap().status(), 409);
    let r = Request::builder()
        .method("POST")
        .uri("/api/login")
        .header("Host", "127.0.0.1:18780")
        .header("Origin", "https://evil.example")
        .header("Content-Type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    assert_eq!(app.clone().oneshot(r).await.unwrap().status(), 403);
    let r = Request::builder()
        .method("POST")
        .uri("/api/login")
        .header("Host", "127.0.0.1:18780")
        .header("Content-Type", "application/json")
        .body(Body::from(json!({"token":f.engine.token}).to_string()))
        .unwrap();
    let response = app.oneshot(r).await.unwrap();
    assert_eq!(response.status(), 200);
    assert!(
        response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .contains("HttpOnly; SameSite=Strict")
    );
    let b = to_bytes(response.into_body(), 1000).await.unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&b).unwrap()["ok"], true);
}
#[tokio::test]
async fn properties_have_optimistic_concurrency_and_saved_views_survive_restart() {
    let f = Fixture::new();
    let path = f.file("notes.md", "Text");
    f.sync().await;
    let body = json!({"device":"local","path":path,"revision":0,"owner":"Sam","tags":["design"],"starred":true,"properties":{"stage":"draft"}});
    let value = f.engine.annotate(&body).unwrap();
    assert_eq!(value["revision"], 1);
    assert!(
        f.engine
            .annotate(&body)
            .unwrap_err()
            .to_string()
            .starts_with("conflict:")
    );
    f.engine
        .comment(&json!({"device":"local","path":path,"author":"Sam","body":"Reviewed"}))
        .unwrap();
    assert_eq!(
        f.engine.metadata("local", &path).unwrap()["comments"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let v=f.engine.save_view(&json!({"name":"Design","kind":"view","filters":{"match":"all","rules":[{"field":"tags","operator":"contains","value":"design"}]}})).await.unwrap();
    let list = f.engine.library(&v["body"]).unwrap();
    assert_eq!(list["total"], 1);
    let reopened = Engine::open(&f.engine.state).unwrap();
    assert_eq!(reopened.workspace().unwrap()["views"][0]["name"], "Design");
}
#[test]
fn evidence_coordinates_are_unicode_character_offsets() {
    let text = "αβγ\nRésumé 日本語\n".repeat(1000);
    let chunks = extract::passages(&text, 256, Some(3));
    let combined: String = chunks.iter().map(|p| p.text.as_str()).collect();
    assert_eq!(combined, text);
    assert!(chunks.iter().all(|p| p.coordinates.page == Some(3)));
    assert_eq!(
        chunks.last().unwrap().coordinates.char_end,
        text.chars().count()
    );
}

#[tokio::test]
async fn typed_properties_validate_atomically_and_collections_follow_edits() {
    let f = Fixture::new();
    let path = f.file("launch.md", "Launch plan");
    f.sync().await;
    let single = f
        .engine
        .save_property(&json!({"name":"Team","kind":"select","options":["Design","Engineering"]}))
        .unwrap();
    let multi = f
        .engine
        .save_property(
            &json!({"name":"Channels","kind":"multi_select","options":["Web","Desktop"]}),
        )
        .unwrap();
    let date = f
        .engine
        .save_property(&json!({"name":"Due date","kind":"date"}))
        .unwrap();
    let number = f
        .engine
        .save_property(&json!({"name":"Estimate","kind":"number"}))
        .unwrap();
    let mut props = json!({"priority":"urgent","legacy":"keep me"});
    props[single["id"].as_str().unwrap()] = json!("Engineering");
    props[multi["id"].as_str().unwrap()] = json!(["Web", "Desktop"]);
    props[date["id"].as_str().unwrap()] = json!("2028-02-29");
    props[number["id"].as_str().unwrap()] = json!(3.5);
    let mut body = json!({"device":"local","path":path,"revision":0,"properties":props,"tags":[" Launch ","launch","Review","Обзор"]});
    let metadata = f.engine.annotate(&body).unwrap();
    assert_eq!(metadata["tags"], json!(["Launch", "Review", "Обзор"]));
    assert_eq!(f.engine.library(&json!({"filters":{"match":"all","rules":[{"field":"tags","operator":"contains","value":"обзор"}]}})).unwrap()["total"], 1);
    assert_eq!(metadata["priority"], "urgent");
    body["revision"] = json!(1);
    body["properties"][date["id"].as_str().unwrap()] = json!("2027-02-29");
    assert!(f.engine.annotate(&body).is_err());
    assert_eq!(f.engine.metadata("local", &path).unwrap()["revision"], 1);
    body["properties"][date["id"].as_str().unwrap()] = json!("2028-02-29");
    body["properties"][single["id"].as_str().unwrap()] = json!(["Engineering"]);
    assert!(f.engine.annotate(&body).is_err());
    body["properties"][single["id"].as_str().unwrap()] = json!("Engineering");
    body["properties"][multi["id"].as_str().unwrap()] = json!(["Web", "Web"]);
    assert!(f.engine.annotate(&body).is_err());
    body["properties"][multi["id"].as_str().unwrap()] = json!(["Web"]);
    let view=f.engine.save_view(&json!({"name":"Launch priorities","kind":"view","filters":{"match":"all","rules":[{"field":"priority","operator":"is","value":"urgent"},{"field":"tags","operator":"contains","value":"launch"}]}})).await.unwrap();
    assert_eq!(f.engine.library(&view["body"]).unwrap()["total"], 1);
    body["properties"]["priority"] = json!("low");
    f.engine.annotate(&body).unwrap();
    assert_eq!(f.engine.library(&view["body"]).unwrap()["total"], 0);
    let reopened = Engine::open(&f.engine.state).unwrap();
    assert_eq!(
        reopened.property_definitions().unwrap()["properties"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(
        reopened.metadata("local", &path).unwrap()["properties"]["legacy"],
        "keep me"
    );
    assert_eq!(
        reopened
            .library(&json!({"sort":"priority","direction":"desc"}))
            .unwrap()["entries"][0]["metadata"]["priority"],
        "low"
    );
}

#[test]
fn property_empty_select_can_grow_without_losing_values_and_tags_are_reused() {
    let f=Fixture::new();
    let path=f.file("tags.md","Fixture notes");
    let mut schema=f.engine.save_property(&json!({"name":"Milestone","kind":"multi_select","options":[]})).unwrap();
    schema["options"]=json!(["Review"]);
    let saved=f.engine.save_property(&schema).unwrap();
    assert_eq!(saved["revision"],2);
    assert!(f.engine.save_property(&schema).unwrap_err().to_string().starts_with("conflict:"));
    let id=saved["id"].as_str().unwrap().to_owned();
    let mut values=json!({});values[id.as_str()]=json!(["Review"]);
    f.engine.annotate(&json!({"device":"local","path":path,"revision":0,"tags":["Launch","Review"],"properties":values})).unwrap();
    let definitions=f.engine.property_definitions().unwrap();
    assert_eq!(definitions["tags"],json!(["Launch","Review"]));
    let mut extended=saved;extended["options"]=json!(["Review","Ready"]);
    f.engine.save_property(&extended).unwrap();
    assert_eq!(f.engine.metadata("local",&path).unwrap()["properties"][id.as_str()],json!(["Review"]));
}

#[test]
fn property_schema_rejects_duplicates_stale_writes_and_destructive_option_changes() {
    let f = Fixture::new();
    assert!(
        f.engine
            .save_property(&json!({"name":"Stage","kind":"select","options":["Draft","draft"]}))
            .is_err()
    );
    let mut schema = f
        .engine
        .save_property(&json!({"name":"Stage","kind":"select","options":["Draft","Done"]}))
        .unwrap();
    assert!(
        f.engine
            .save_property(&json!({"name":"stage","kind":"text"}))
            .is_err()
    );
    schema["revision"] = json!(0);
    assert!(
        f.engine
            .save_property(&schema)
            .unwrap_err()
            .to_string()
            .starts_with("conflict:")
    );
    schema["revision"] = json!(1);
    schema["options"] = json!(["Draft"]);
    assert!(f.engine.save_property(&schema).is_err());
    schema["options"] = json!(["Draft", "Done", "Review"]);
    let saved = f.engine.save_property(&schema).unwrap();
    assert_eq!(saved["revision"], 2);
}

#[test]
fn comments_keep_the_latest_page_without_advancing_property_revision() {
    let f = Fixture::new();
    let path = f.file("discussion.md", "Notes");
    for i in 0..205 {
        f.engine.comment(&json!({"device":"local","path":path,"author":"Reviewer","body":format!("Comment {i:03}")})).unwrap();
    }
    let metadata = f.engine.metadata("local", &path).unwrap();
    let comments = metadata["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 200);
    assert_eq!(comments.last().unwrap()["body"], "Comment 204");
    assert_eq!(metadata["revision"], 0);
}
#[test]
fn protocol_limits_and_versions_are_enforced() {
    use std::io::Cursor;
    assert!(
        retriever_protocol::read_frame(&mut Cursor::new(
            b"{\"version\":2,\"id\":\"1\",\"action\":\"probe\"}\n"
        ))
        .is_err()
    );
    assert!(
        retriever_protocol::read_frame(&mut Cursor::new(vec![
            b'x';
            retriever_protocol::MAX_MESSAGE + 2
        ]))
        .is_err()
    );
    let req = retriever_protocol::read_frame(&mut Cursor::new(
        b"{\"version\":1,\"id\":\"1\",\"action\":\"probe\"}\n",
    ))
    .unwrap()
    .unwrap();
    assert_eq!(req.action, "probe");
}
#[test]
fn legacy_state_is_refused_without_touching_it() {
    let root = tempfile::tempdir().unwrap();
    let legacy = root.path().join("index.sqlite");
    std::fs::write(&legacy, "existing data").unwrap();
    assert!(Engine::open(root.path()).is_err());
    assert_eq!(std::fs::read_to_string(legacy).unwrap(), "existing data");
    assert!(!root.path().join("retriever.sqlite").exists());
}
#[test]
fn short_video_and_audio_windows_never_skip_short_clips() {
    assert_eq!(
        retriever_engine::media::sample_times(0.4, 5.0, 100),
        vec![0.2]
    );
    assert_eq!(
        retriever_engine::media::sample_times(10.0, 5.0, 100),
        vec![0.0, 5.0]
    );
    assert_eq!(
        retriever_engine::media::sample_times(1000.0, 5.0, 3).len(),
        3
    );
}
#[tokio::test]
async fn reviewed_rename_swaps_preserve_ids_and_undo_checks_for_edits() {
    let f = Fixture::new();
    let a = f.file("alpha.md", "first evidence");
    let b = f.file("beta.md", "second evidence");
    f.sync().await;
    let original = f.engine.library(&json!({})).unwrap()["entries"][0]["id"].clone();
    let plan=f.engine.operation_plan(&json!({"device":"local","operation":"rename","items":[{"path":a,"name":"beta.md"},{"path":b,"name":"alpha.md"}]})).await.unwrap();
    let receipt = f
        .engine
        .apply_operation(plan["id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(receipt["state"], "applied", "{receipt}");
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "first evidence");
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "second evidence");
    let moved = f
        .engine
        .store
        .result(original.as_i64().unwrap(), &f.engine.config.read())
        .unwrap()
        .unwrap();
    assert_eq!(moved["path"], b);
    assert!(
        f.engine
            .apply_operation(plan["id"].as_str().unwrap())
            .await
            .is_err()
    );
    let undo = f
        .engine
        .undo_plan(plan["id"].as_str().unwrap())
        .await
        .unwrap();
    std::fs::write(&b, "changed after review and before undo").unwrap();
    let result = f
        .engine
        .apply_operation(undo["id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(result["state"], "inspection_required");
    assert_eq!(
        std::fs::read_to_string(&b).unwrap(),
        "changed after review and before undo"
    );
}
#[tokio::test]
async fn collisions_and_expired_plans_do_not_mutate() {
    let f = Fixture::new();
    let a = f.file("alpha.md", "original");
    f.file("beta.md", "existing");
    assert!(f.engine.operation_plan(&json!({"device":"local","operation":"rename","items":[{"path":a,"name":"beta.md"}]})).await.is_err());
    let p = f
        .engine
        .operation_plan(
            &json!({"device":"local","operation":"rename","items":[{"path":a,"name":"gamma.md"}]}),
        )
        .await
        .unwrap();
    let id = p["id"].as_str().unwrap();
    f.engine
        .store
        .db
        .lock()
        .execute("UPDATE workspace_plans SET created=0 WHERE id=?", [id])
        .unwrap();
    assert!(f.engine.apply_operation(id).await.is_err());
    assert_eq!(std::fs::read_to_string(a).unwrap(), "original");
}
#[test]
fn migration_preserves_identity_and_metadata_but_no_vectors_or_live_plans() {
    use rusqlite::params;
    let root = tempfile::Builder::new()
        .prefix("retriever-migration-")
        .tempdir()
        .unwrap();
    let old = root.path().join("old");
    std::fs::create_dir(&old).unwrap();
    let source = root.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let mut cfg = Config::default();
    cfg.configured = true;
    cfg.devices[0].roots = vec![source.canonicalize().unwrap().to_string_lossy().into()];
    cfg.save(&old).unwrap();
    let mut legacy_config = serde_json::to_value(&cfg).unwrap();
    legacy_config["limits"]["max_text_chars"] = json!(134217728);
    std::fs::write(
        old.join("config.json"),
        serde_json::to_vec(&legacy_config).unwrap(),
    )
    .unwrap();
    std::fs::write(old.join("api-token"), "a".repeat(40)).unwrap();
    let db = rusqlite::Connection::open(old.join("index.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE files(id INTEGER PRIMARY KEY,device TEXT,path TEXT,name TEXT,type TEXT,extension TEXT,text TEXT,detail TEXT,size INTEGER,mtime_ns INTEGER,digest TEXT,indexed_at REAL);CREATE TABLE workspace_meta(device TEXT,path TEXT,owner TEXT,tags TEXT,status TEXT,starred INTEGER,properties TEXT,revision INTEGER);CREATE TABLE workspace_plans(id TEXT,body TEXT,state TEXT,created REAL,receipt TEXT);CREATE TABLE embedding_vectors(vector BLOB);").unwrap();
    let path = source
        .canonicalize()
        .unwrap()
        .join("example.md")
        .to_string_lossy()
        .into_owned();
    db.execute("INSERT INTO files VALUES(42,'local',?,'example.md','documents','.md','original content','text',16,0,'digest',0)",[&path]).unwrap();
    db.execute(
        "INSERT INTO workspace_meta VALUES('local',?,'Sam','[]','draft',1,'{}',4)",
        [&path],
    )
    .unwrap();
    db.execute(
        "INSERT INTO workspace_plans VALUES(?,'{}','preview',0,'{}')",
        [uuid::Uuid::new_v4().to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO workspace_plans VALUES('completed-rename','{}','applied',0,'{\"ok\":true}')",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO workspace_plans VALUES('interrupted-copy','{}','check_required',0,'{\"check_required\":true}')",[]).unwrap();
    db.execute(
        "INSERT INTO embedding_vectors VALUES(?)",
        params![vec![1u8, 2, 3]],
    )
    .unwrap();
    drop(db);
    let candidate = root.path().join("candidate");
    let report = retriever_engine::migration::import(&old, &candidate).unwrap();
    assert_eq!(report["vectors_imported"], 0);
    let engine = Engine::open(&candidate).unwrap();
    assert!(engine.store.result(42, &cfg).unwrap().is_some());
    assert_eq!(engine.metadata("local", &path).unwrap()["owner"], "Sam");
    let history = engine.workspace().unwrap();
    let mut states = history["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["state"].as_str().unwrap())
        .collect::<Vec<_>>();
    states.sort();
    assert_eq!(states, vec!["applied", "expired", "inspection_required"]);
    assert_eq!(engine.config.read().limits["max_text_chars"], 16000000);
    assert!(
        report["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap().contains("max_text_chars"))
    );
    let vectors: i64 = engine
        .store
        .db
        .lock()
        .query_row("SELECT count(*) FROM vectors", [], |r| r.get(0))
        .unwrap();
    assert_eq!(vectors, 0);
    assert_eq!(
        std::fs::read_to_string(candidate.join("api-token")).unwrap(),
        "a".repeat(40)
    );
}
#[tokio::test]
async fn chunked_copy_is_verified_and_never_overwrites() {
    let f = Fixture::new();
    let content = "ordinary document content\n".repeat(40000);
    let source = f.file("large.md", &content);
    let parent = std::path::Path::new(&source)
        .parent()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let plan=f.engine.operation_plan(&json!({"device":"local","target_device":"local","operation":"copy","parent":parent,"items":[{"path":source,"name":"copied.md"}]})).await.unwrap();
    let r = f
        .engine
        .apply_operation(plan["id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(r["state"], "applied", "{r}");
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&parent).join("copied.md")).unwrap(),
        content
    );
    assert!(f.engine.operation_plan(&json!({"device":"local","target_device":"local","operation":"copy","parent":parent,"items":[{"path":source,"name":"copied.md"}]})).await.is_err());
}
#[test]
fn bad_checksum_does_not_publish_and_interrupted_transfer_needs_inspection() {
    use base64::Engine as _;
    let f = Fixture::new();
    let d = f.engine.config.read().devices[0].clone();
    let id = uuid::Uuid::new_v4().to_string();
    let dest = std::path::Path::new(&d.roots[0]).join("copy.txt");
    let call = |action: &str, b: Value| retriever_engine::transfer::dispatch(&d, action, &b, &[]);
    call("transfer_begin", json!({"id":id,"path":dest,"size":3})).unwrap();
    call(
        "transfer_write",
        json!({"id":id,"offset":0,"data":base64::engine::general_purpose::STANDARD.encode(b"abc")}),
    )
    .unwrap();
    assert!(call("transfer_finish", json!({"id":id,"sha256":"wrong"})).is_err());
    assert!(!dest.exists());
    assert_eq!(
        call("transfer_inspect", json!({"id":id})).unwrap()["offset"],
        3
    );
    assert!(call("transfer_write", json!({"id":id,"offset":0,"data":"YWJj"})).is_err());
    assert!(call("transfer_begin", json!({"id":id,"path":dest,"size":3})).is_err());
    call(
        "transfer_finish",
        json!({"id":id,"sha256":retriever_engine::digest(b"abc")}),
    )
    .unwrap();
    assert_eq!(std::fs::read(dest).unwrap(), b"abc");
}

#[test]
fn browser_exposes_native_search_and_no_model_apis_or_generation() {
    let html=include_str!("../../../web/index.html");
    for id in ["nativeStorage","nextStep","saveSetup","setupDone","setupError"] {
        assert_eq!(html.matches(&format!("id=\"{id}\"")).count(),1);
    }
    for removed in ["embeddingProvider","embeddingAPI","relevanceProvider","visionProvider","llmProvider","askNow","summarize","aiRename","aiSort"] {
        assert!(!html.contains(&format!("id=\"{removed}\"")),"{removed}");
    }
}
