use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use permagent_harvester::{config::Config, db, http, stage::Stage};
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Run,
    Once {
        stage: Stage,
    },
    Status,
    Drafts {
        #[command(subcommand)]
        command: DraftCommand,
    },
}

#[derive(Subcommand)]
enum DraftCommand {
    List,
    Show { id: i64 },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stdout)
        .init();
    let cli = Cli::parse();
    let config = Config::load(cli.config.as_deref())?;
    let path = config.db_path()?;
    match cli.command {
        Command::Run => {
            let token = config.bearer_token()?;
            db::open_and_migrate(&path)?;
            let listener = tokio::net::TcpListener::bind(&config.daemon.bind)
                .await
                .with_context(|| format!("failed to bind {}", config.daemon.bind))?;
            tracing::info!(bind=%config.daemon.bind,"daemon listening");
            axum::serve(listener, http::router(path, token)).await?;
        }
        Command::Once { stage } => {
            let connection = db::open_and_migrate(&path)?;
            db::tick(&connection, stage)?;
            println!("{stage}: ok");
        }
        Command::Status => {
            let connection = db::open_and_migrate(&path)?;
            for stage_status in db::statuses(&connection)? {
                println!(
                    "{}\t{}",
                    stage_status.stage,
                    stage_status.started_at.as_deref().unwrap_or("never")
                );
            }
        }
        Command::Drafts {
            command: DraftCommand::List,
        } => {
            let connection = db::open_and_migrate(&path)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&db::drafts(&connection)?)?
            );
        }
        Command::Drafts {
            command: DraftCommand::Show { id },
        } => {
            let connection = db::open_and_migrate(&path)?;
            match db::draft(&connection, id)? {
                Some(draft) => println!("{}", serde_json::to_string_pretty(&draft)?),
                None => bail!("draft {id} not found"),
            }
        }
    }
    Ok(())
}
