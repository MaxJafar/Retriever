//! Local, opt-in acceptance harness. State and query files are never packaged.
use anyhow::{Context, Result, bail};
use retriever_engine::engine::Engine;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let state = PathBuf::from(args.get(1).context("state directory required")?);
    let action = args.get(2).map(String::as_str).unwrap_or("crawl");
    let engine = Engine::open(&state)?;
    let report_path = state.join(format!("acceptance-{action}.json"));
    let start = Instant::now();
    if action == "probe" {
        for role in ["embedding", "relevance", "llm", "vision"] {
            let settings = engine.config.read().search[role].clone();
            if settings["provider"] != "disabled" {
                engine.probe(role, settings).await?;
            }
        }
    } else if ["crawl", "index", "recrawl", "folder"].contains(&action) {
        let mut rounds = Vec::new();
        for round in 0..100 {
            if action == "folder" && round == 0 {
                engine.start_folder(&json!({"device":args.get(3).context("device required")?,"root":args.get(4).context("root required")?}))?;
            } else {
                engine.start_sync(action == "index" || round > 0)?;
            }
            let mut previous = Value::Null;
            while engine.syncing.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let progress = engine.progress.read().clone();
                if progress != previous {
                    std::fs::write(
                        state.join("acceptance-progress.json"),
                        serde_json::to_vec_pretty(&progress)?,
                    )?;
                    previous = progress;
                }
                if start.elapsed() > Duration::from_secs(3600) {
                    engine.cancel.store(true, Ordering::SeqCst);
                    bail!("Acceptance run deadline exceeded");
                }
            }
            rounds.push(engine.progress.read().clone());
            if rounds.last().unwrap()["phase"] == "failed" {
                break;
            }
            if engine.knowledge_pending()? == 0 {
                break;
            }
        }
        std::fs::write(
            &report_path,
            serde_json::to_vec_pretty(
                &json!({"elapsed_ms":start.elapsed().as_secs_f64()*1000.,"rounds":rounds,"status":engine.status()?,"providers":engine.providers.health.lock().clone()}),
            )?,
        )?;
    } else if action == "bench" {
        let path = args.get(3).context("query JSON required")?;
        let queries: Vec<Value> = serde_json::from_slice(&std::fs::read(path)?)?;
        let mut results = Vec::new();
        for mode in ["hybrid", "exact", "semantic"] {
            for query in &queries {
                let q = query["query"].as_str().context("query required")?;
                let mut runs = Vec::new();
                for _ in 0..6 {
                    let request_start = Instant::now();
                    let response = engine
                        .search(q, "", query["kind"].as_str().unwrap_or(""), mode, "full")
                        .await?;
                    let rank = response["results"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .position(|r| {
                            query["expected"].as_array().unwrap().iter().any(|e| {
                                e.as_str().is_some_and(|p| {
                                    r["path"].as_str().is_some_and(|v| {
                                        v.replace('\\', "/").ends_with(&p.replace('\\', "/"))
                                    })
                                })
                            })
                        });
                    runs.push(json!({"rank":rank.map(|r|r+1),"wall_ms":request_start.elapsed().as_secs_f64()*1000.,"response":response}));
                }
                results.push(json!({"query":query,"mode":mode,"runs":runs}));
                std::fs::write(
                    &report_path,
                    serde_json::to_vec_pretty(
                        &json!({"results":results,"providers":engine.providers.health.lock().clone()}),
                    )?,
                )?;
            }
        }
    } else {
        bail!("Unknown action");
    }
    println!(
        "{}",
        json!({"action":action,"elapsed_ms":start.elapsed().as_secs_f64()*1000.,"report":report_path,"files":engine.status()?["files"]})
    );
    Ok(())
}
