use clap::{Parser, Subcommand};
use retriever_engine::{config, engine::Engine};
use std::path::PathBuf;
#[derive(Parser)]
#[command(version, about = "Retriever independent Rust coordinator (candidate)")]
struct Args {
    #[arg(long, global = true)]
    state: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Serve {
        #[arg(long, default_value = "127.0.0.1")]
        host: std::net::IpAddr,
        #[arg(long, default_value_t = 18780)]
        port: u16,
    },
    /// Read-only drive telemetry helper. No candidate state is opened.
    #[command(hide = true)]
    HealthProbe,
    Status,
    Init,
    Search {
        query: String,
        #[arg(long, default_value = "hybrid")]
        mode: String,
    },
    Migrate {
        #[arg(long)]
        from: PathBuf,
    },
    Sync {
        #[arg(long)]
        index_only: bool,
    },
    Probe {
        role: String,
    },
    /// Install the bundled retrieval models from a local folder into app data.
    InstallModels { bundle: PathBuf },

    /// Export lossless flat snapshots for rollback. Stop the coordinator first.
    SnapshotCompatibility,

}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if matches!(args.command, Command::HealthProbe) {
        println!("{}", retriever_engine::health_platform::collect()?);
        return Ok(());
    }
    let state = args.state.unwrap_or_else(config::default_state);
    if matches!(args.command, Command::SnapshotCompatibility) {
        println!("{}", retriever_engine::vectors::export_legacy_snapshots(&state)?);
        return Ok(());
    }
    if let Command::InstallModels { bundle } = args.command {
        let installed=retriever_engine::native_models::NativeModels::install_bundle(&state,&bundle)?;
        println!("{}",serde_json::json!({"installed":installed,"models":state.join("models")}));
        return Ok(());
    }
    if let Command::Migrate { from } = args.command {
        println!(
            "{}",
            serde_json::to_string_pretty(&retriever_engine::migration::import(&from, &state)?)?
        );
        return Ok(());
    }
    let engine = Engine::open(&state)?;
    let output = match args.command {
        Command::Serve { host, port } => {
            retriever_engine::http::serve(engine, std::net::SocketAddr::new(host, port)).await?;
            return Ok(());
        }
        Command::Status => engine.status()?,
        Command::Init => serde_json::json!({"state":state,"candidate":true}),
        Command::Search { query, mode } => engine.search(&query, "", "", &mode, "full").await?,
        Command::Probe { role } => {
            let settings = engine.config.read().search[&role].clone();
            engine.probe(&role, settings).await?
        }
        Command::Sync { index_only } => {
            engine.start_sync(index_only)?;
            while engine.syncing.load(std::sync::atomic::Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            engine.progress.read().clone()
        }
        Command::Migrate { .. } | Command::InstallModels { .. } | Command::HealthProbe | Command::SnapshotCompatibility => unreachable!(),
    };
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}
