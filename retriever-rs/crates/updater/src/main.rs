use anyhow::Result;
use clap::{Parser, Subcommand};
use std::{
    io::{BufRead, Write},
    path::PathBuf,
};

#[derive(Parser)]
#[command(version)]
struct Args {
    #[arg(long)]
    repository: String,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Check {
        #[arg(long)]
        current: String,
        #[arg(long)]
        platform: String,
        #[arg(long)]
        cache: PathBuf,
        #[arg(long)]
        preview: bool,
        #[arg(long)]
        force: bool,
    },
    Download {
        #[arg(long)]
        current: String,
        #[arg(long)]
        platform: String,
        #[arg(long)]
        release_id: u64,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        preview: bool,
    },
    Verify {
        #[arg(long)]
        receipt: PathBuf,
    },
}
fn emit(value: impl serde::Serialize) {
    println!("{}", serde_json::to_string(&value).unwrap());
    let _ = std::io::stdout().flush();
}
async fn run() -> Result<()> {
    let args = Args::parse();
    match args.command {
        Command::Check {
            current,
            platform,
            cache,
            preview,
            force,
        } => emit(
            retriever_updater::check(
                &args.repository,
                &current,
                &platform,
                preview,
                &cache,
                force,
            )
            .await?,
        ),
        Command::Download {
            current,
            platform,
            release_id,
            output,
            preview,
        } => {
            // A detached reader cannot keep Tokio's runtime alive after a
            // successful download when the desktop still holds stdin open.
            let (sender, receiver) = tokio::sync::oneshot::channel();
            std::thread::spawn(move || {
                for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                    if line == "CANCEL" {
                        let _ = sender.send(());
                        break;
                    }
                }
            });
            let cancel = async {
                if receiver.await.is_err() {
                    std::future::pending::<()>().await;
                }
            };
            let result = retriever_updater::download(
                &args.repository,
                release_id,
                &current,
                &platform,
                preview,
                &output,
                |received, total| {
                    emit(serde_json::json!({"event":"progress","received":received,"total":total}))
                },
                cancel,
            )
            .await?;
            emit(serde_json::json!({"event":"downloaded","download":result}));
        }
        Command::Verify { receipt } => emit(retriever_updater::verify(&receipt, &args.repository)?),
    }
    Ok(())
}
#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{}", serde_json::json!({"error":error.to_string()}));
        std::process::exit(1)
    }
}
