use crate::engine::Engine;
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::any,
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{collections::HashMap, convert::Infallible, sync::Arc};
use subtle::ConstantTimeEq;
pub fn router(engine: Arc<Engine>) -> Router {
    Router::new().fallback(any(handle)).with_state(engine)
}
pub async fn serve(engine: Arc<Engine>, address: std::net::SocketAddr) -> anyhow::Result<()> {
    if !address.ip().is_loopback() {
        anyhow::bail!("Bind the candidate to loopback and use a reviewed SSH tunnel");
    }
    let listener = tokio::net::TcpListener::bind(address).await?;
    let scheduler = tokio::spawn(crate::crawl::scheduler(engine.clone()));
    let health_monitor = tokio::spawn(engine.health.clone().scheduler());
    let mut shutdown = engine.shutdown.subscribe();
    axum::serve(listener, router(engine))
        .with_graceful_shutdown(async move {
            tokio::select! { _=tokio::signal::ctrl_c()=>{}, _=shutdown.wait_for(|stop|*stop)=>{} }
        })
        .await?;
    scheduler.abort();
    health_monitor.abort();
    Ok(())
}
fn reply(code: StatusCode, value: Value) -> Response {
    let mut r = (code, Json(value)).into_response();
    secure_headers(r.headers_mut());
    r
}
fn secure_headers(h: &mut HeaderMap) {
    for (name, value) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        (
            "content-security-policy",
            "default-src 'self'; script-src 'self' qrc:; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
    ] {
        h.insert(name, HeaderValue::from_static(value));
    }
}
fn equal(a: &str, b: &str) -> bool {
    bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}
async fn handle(State(engine): State<Arc<Engine>>, request: Request) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let headers = request.headers().clone();
    let path = uri.path();
    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let hosturl = reqwest::Url::parse(&format!("http://{host}"));
    let hosturl = match hosturl {
        Ok(v) if v.username().is_empty() && v.password().is_none() => v,
        _ => return reply(StatusCode::FORBIDDEN, json!({"error":"Host denied"})),
    };
    let configured = reqwest::Url::parse(&engine.config.read().endpoint).ok();
    let hostname = hosturl.host_str().unwrap_or("");
    if !["localhost", "127.0.0.1", "::1", "[::1]"].contains(&hostname)
        && configured.as_ref().and_then(|u| u.host_str()) != Some(hostname)
    {
        return reply(StatusCode::FORBIDDEN, json!({"error":"Host denied"}));
    }
    if method == "GET" {
        if let Some((bytes, mime)) = asset(path) {
            let mut r = Response::new(Body::from(bytes));
            r.headers_mut()
                .insert("content-type", HeaderValue::from_static(mime));
            secure_headers(r.headers_mut());
            return r;
        }
    }
    let cookie_name = format!("searchx_auth_{}", hosturl.port().unwrap_or(80));
    let authorized = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|v| equal(v, &engine.token))
        || headers
            .get("cookie")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| {
                v.split(';')
                    .filter_map(|c| c.trim().split_once('='))
                    .any(|(k, v)| {
                        (k == cookie_name || k == "searchx_auth") && equal(v, &engine.token)
                    })
            });
    if method == "POST" {
        if headers
            .get("origin")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| {
                reqwest::Url::parse(v)
                    .ok()
                    .map(|u| u.authority().to_lowercase())
                    != Some(host.to_lowercase())
            })
            || headers
                .get("sec-fetch-site")
                .is_some_and(|v| v == "cross-site")
        {
            return reply(StatusCode::FORBIDDEN, json!({"error":"Origin denied"}));
        }
    }
    if !authorized && !(method == "POST" && path == "/api/login") {
        return reply(
            StatusCode::UNAUTHORIZED,
            json!({"error":"Sign in with the coordinator pairing token"}),
        );
    }
    if method == "GET" && path == "/api/events" {
        let mut shutdown=engine.shutdown.subscribe();
        let stream = tokio_stream::wrappers::BroadcastStream::new(engine.events.subscribe())
            .take_until(async move {let _=shutdown.wait_for(|stop|*stop).await;})
            .filter_map(|event| async move {
                Some(Ok::<_, Infallible>(
                    Event::default().event("progress").data(
                        event
                            .map(|v| v.to_string())
                            .unwrap_or_else(|_| "{\"resync\":true}".into()),
                    ),
                ))
            });
        let mut response = Sse::new(stream)
            .keep_alive(KeepAlive::default())
            .into_response();
        secure_headers(response.headers_mut());
        return response;
    }
    let params: HashMap<String, String> = reqwest::Url::parse(&format!("http://127.0.0.1{uri}"))
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect();
    let mut body = json!({});
    if method == "POST" {
        if headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            != Some("application/json")
        {
            return reply(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                json!({"error":"Use application/json"}),
            );
        }
        let bytes = match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            to_bytes(request.into_body(), 256 * 1024),
        )
        .await
        {
            Ok(Ok(v)) => v,
            Ok(Err(_)) => {
                return reply(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    json!({"error":"Request is too large"}),
                );
            }
            Err(_) => {
                return reply(
                    StatusCode::REQUEST_TIMEOUT,
                    json!({"error":"Request body timed out"}),
                );
            }
        };
        body = match serde_json::from_slice::<Value>(&bytes) {
            Ok(v) if v.is_object() => v,
            _ => {
                return reply(
                    StatusCode::BAD_REQUEST,
                    json!({"error":"Expected a JSON object"}),
                );
            }
        };
    }
    if path == "/api/login" && method == "POST" {
        if !body["token"]
            .as_str()
            .is_some_and(|v| equal(v, &engine.token))
        {
            return reply(
                StatusCode::UNAUTHORIZED,
                json!({"error":"Pairing token is incorrect"}),
            );
        }
        let mut r = reply(StatusCode::OK, json!({"ok":true}));
        r.headers_mut().insert(
            "set-cookie",
            HeaderValue::from_str(&format!(
                "{cookie_name}={}; HttpOnly; SameSite=Strict; Path=/{}",
                engine.token,
                if ["localhost", "127.0.0.1", "[::1]", "::1"].contains(&hostname) {
                    ""
                } else {
                    "; Secure"
                }
            ))
            .unwrap(),
        );
        return r;
    }
    let result: anyhow::Result<(StatusCode,Value)>=async {
        let ok=StatusCode::OK;
        if path=="/api/runtime/shutdown" && method=="POST" {
            let local=headers.get("x-retriever-update-token").and_then(|v|v.to_str().ok()).unwrap_or("");
            if !equal(local,&engine.update_token) {return Ok((StatusCode::FORBIDDEN,json!({"error":"Local update authorization is required"})));}
            return Ok((ok,engine.stop_for_update(local,body["expected_version"].as_str().unwrap_or("")).await?));
        }
        let _runtime=engine.runtime_gate.read().await;
        if *engine.shutdown.borrow() {return Ok((StatusCode::SERVICE_UNAVAILABLE,json!({"error":"Retriever is stopping for an update"})));}
        if method=="GET" {return Ok((ok,match path {
            "/api/status"=>engine.status()?,"/api/assistant"=>engine.assistant_status()?,"/api/config"=>{let cfg=engine.config.read();json!({"config":*cfg,"revision":cfg.revision()})},
            "/api/workspace"=>engine.workspace()?,
            "/api/workspace/properties"=>engine.property_definitions()?,
            "/api/crawl"=>engine.crawl_status()?,
            "/api/disk"=>engine.disk.status()?,
            "/api/burner"=>engine.burner.status(),
            "/api/health"=>engine.health.status()?,
            "/api/plan"=>engine.plan_configuration(None)?,
            "/api/nodes/discover"=>crate::nodes::discover().await?,
            "/api/ssh-aliases"=>ssh_aliases(&engine.config.read().ssh_config)?,
            _ if path.starts_with("/api/workspace/operation/")=>engine.operation(path.trim_start_matches("/api/workspace/operation/"))?,
            "/api/search"=>engine.search(params.get("q").map(String::as_str).unwrap_or(""),params.get("device").map(String::as_str).unwrap_or(""),params.get("kind").map(String::as_str).unwrap_or(""),params.get("mode").map(String::as_str).unwrap_or("hybrid"),params.get("phase").map(String::as_str).unwrap_or("full")).await?,
            _ if path.starts_with("/api/result/")=>{let id=path.trim_start_matches("/api/result/").parse::<i64>()?;match engine.store.result(id,&engine.config.read())?{Some(v)=>v,None=>return Ok((StatusCode::NOT_FOUND,json!({"error":"Result is no longer in an enabled folder"}))) }},
            _=>return Ok((StatusCode::NOT_FOUND,json!({"error":"Route has not reached candidate parity","candidate":true})))
        }));}
        if method!="POST" {return Ok((StatusCode::METHOD_NOT_ALLOWED,json!({"error":"Method not allowed"})));}
        Ok((if ["/api/sync","/api/jobs/cancel","/api/workspace/apply"].contains(&path){StatusCode::ACCEPTED}else{ok},match path {
            "/api/config"=>engine.configure(body,headers.get("if-match").and_then(|v|v.to_str().ok()))?,
            "/api/explorer/locations"=>engine.explorer_locations(&body).await?,
            "/api/explorer/browse"=>engine.explorer_browse(&body).await?,
            "/api/explorer/children"=>engine.explorer_children(&body).await?,
            "/api/crawl/preview"=>engine.crawl_preview(&body).await?,
            "/api/explorer/preview"=>engine.explorer_preview(&body).await?,
            "/api/explorer/duration"=>engine.file_duration(&body).await?,
            "/api/assistant/archive"=>engine.archive_catalogue()?,
            "/api/burner/inspect"=>engine.burner.inspect(serde_json::from_value(body)?)?,
            "/api/burner/start"=>engine.burner.start(serde_json::from_value(body)?)?,
            "/api/burner/cancel"=>engine.burner.cancel(body["id"].as_str().unwrap_or(""))?,
            "/api/burner/sample"=>{let burner=engine.burner.clone();tokio::task::spawn_blocking(move||burner.sample()).await??},
            "/api/devices/storage"=>engine.remote_storage(&body).await?,
            "/api/crawl/settings"=>engine.crawl_settings(&body)?,
            "/api/health/refresh"=>engine.health.refresh()?,
            "/api/health/settings"=>engine.health.configure(&body)?,
            "/api/health/detail"=>engine.health.detail(body["id"].as_str().unwrap_or(""))?,
            "/api/disk/scan"=>engine.disk.start(body["path"].as_str().unwrap_or(""))?,
            "/api/disk/cancel"=>engine.disk.cancel(body["id"].as_str().unwrap_or(""))?,
            "/api/disk/view"=>{let disk=engine.disk.clone();tokio::task::spawn_blocking(move||disk.view(&body)).await??},
            "/api/crawl/run"=>engine.start_folder(&body)?,
            "/api/plan"=>engine.plan_configuration(Some(body))?,
            "/api/nodes/discover"=>crate::nodes::discover().await?,
            "/api/ssh-aliases"=>ssh_aliases(body["path"].as_str().unwrap_or(""))?,
            "/api/folders"=>{let cfg=engine.config.read().clone();let d=if body["device"].is_object(){serde_json::from_value::<crate::config::Device>(body["device"].clone())?}else{cfg.device(body["device"].as_str().unwrap_or(""))?.clone()};engine.source(&d,"folders",body).await?},
            "/api/sync"=>{for key in ["index_only","force_embeddings"] {if body.get(key).is_some_and(|v| !v.is_boolean()) {anyhow::bail!("{key} must be a boolean");}} if body["force_embeddings"]==true {anyhow::bail!("Explicit embedding reset requires a reviewed candidate rebuild");}engine.start_sync(body["index_only"].as_bool().unwrap_or(false))?},
            "/api/jobs/cancel"=>engine.cancel_jobs()?,
            "/api/workspace/browse"=>engine.browse(&body).await?,"/api/workspace/library"=>engine.library(&body)?,
            "/api/workspace/inspect"=>engine.metadata_scoped(body["device"].as_str().unwrap_or(""),body["path"].as_str().unwrap_or(""),body["scope"]=="filesystem")?,
            "/api/workspace/property"=>engine.save_property(&body)?,
            "/api/workspace/plan"=>engine.operation_plan(&body).await?,
            "/api/workspace/apply"=>engine.apply_operation(body["id"].as_str().unwrap_or("")).await?,
            "/api/workspace/start"=>engine.start_operation(body["id"].as_str().unwrap_or(""))?,
            "/api/workspace/cancel"=>engine.cancel_operation(body["id"].as_str().unwrap_or(""))?,
            "/api/workspace/undo"=>engine.undo_plan(body["id"].as_str().unwrap_or("")).await?,
            "/api/workspace/metadata"=>engine.annotate(&body)?,"/api/workspace/comment"=>engine.comment(&body)?,"/api/workspace/view"=>engine.save_view(&body).await?,"/api/workspace/preview"=>engine.preview(&body).await?,
            "/api/model-probe"=>{let role=body["purpose"].as_str().unwrap_or("");let settings=body.get("settings").cloned().unwrap_or_else(||engine.config.read().search[role].clone());engine.probe(role,settings).await?},
            "/api/assist"=>engine.assist(&body).await?,
            "/api/device-probe"=>{let cfg=engine.config.read().clone();engine.source(cfg.device(body["device"].as_str().unwrap_or(""))?,"probe",json!({})).await?},
            "/api/file-icons"=>json!({"icons":{},"native":false}),
            _=>return Ok((StatusCode::NOT_IMPLEMENTED,json!({"error":"This operation is unavailable until candidate safety and parity checks pass","candidate":true})))
        }))
    }.await;
    match result {
        Ok((status, value)) => reply(status, value),
        Err(e) => {
            let message = e.to_string();
            let code = if message.starts_with("conflict:") {
                StatusCode::CONFLICT
            } else if path == "/api/assist" || path == "/api/model-probe" {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::BAD_REQUEST
            };
            reply(code, json!({"error":message}))
        }
    }
}
fn ssh_aliases(path: &str) -> anyhow::Result<Value> {
    let path = if path.is_empty() {
        crate::paths::expand("~/.ssh/config")
    } else {
        crate::paths::expand(path)
    };
    if !path.exists() {
        return Ok(json!({"aliases":[],"ssh_available":true}));
    }
    let text = String::from_utf8(crate::extract::read_bounded(&path, 262144)?)?;
    let mut aliases = Vec::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        if words.next().is_some_and(|w| w.eq_ignore_ascii_case("host")) {
            for word in words {
                if !word.starts_with('-')
                    && word
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
                {
                    aliases.push(word);
                }
            }
        }
    }
    aliases.sort();
    aliases.dedup();
    Ok(json!({"aliases":aliases,"ssh_available":true}))
}
fn asset(path: &str) -> Option<(&'static [u8], &'static str)> {
    // Compile-time, explicit asset allowlist: no arbitrary filesystem reads.
    macro_rules! asset {
        ($file:literal,$mime:literal) => {
            Some((
                include_bytes!(concat!("../../../web/", $file)).as_slice(),
                $mime,
            ))
        };
    }
    match path {
        "/" | "/index.html" => asset!("index.html", "text/html; charset=utf-8"),
        "/app.js" => asset!("app.js", "application/javascript"),
        "/workspace.js" => asset!("workspace.js", "application/javascript"),
        "/experience.js" => asset!("experience.js", "application/javascript"),
        "/crawl.js" => asset!("crawl.js", "application/javascript"),
        "/crawl.css" => asset!("crawl.css", "text/css"),
        "/theme.js" => asset!("theme.js", "application/javascript"),
        "/sync.js" => asset!("sync.js", "application/javascript"),
        "/clients.js" => asset!("clients.js", "application/javascript"),
        "/style.css" => asset!("style.css", "text/css"),
        "/workspace.css" => asset!("workspace.css", "text/css"),
        "/skin.css" => asset!("skin.css", "text/css"),
        "/retriever.css" => asset!("retriever.css", "text/css"),
        "/retriever.png" | "/golden-retriever.png" => asset!("retriever.png", "image/png"),
        _ => None,
    }
}
