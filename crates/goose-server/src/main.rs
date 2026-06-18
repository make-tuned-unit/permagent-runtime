mod agent_state_tick;
mod app_catalog;
mod automation;
mod backup;
mod brain_ops;
mod brain_sync;
mod commands;
mod configuration;
mod error;
mod logging;
mod middleware;
mod openapi;
mod routes;
mod session_event_bus;
mod state;
mod tunnel;
// Some verification items (test-support consts/fields) are only consumed by
// the lib target's tests; the bin needs the module for the startup hook wire.
#[allow(dead_code)]
mod verification;
mod voice;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use permagent::agents::validate_extensions;
use permagent_mcp::{
    mcp_server_runner::{serve, McpCommand},
    AutoVisualiserRouter, ComputerControllerServer, MemoryServer, TutorialServer,
};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the agent server
    Agent {
        /// Bind address (overrides config.yaml and env vars)
        #[arg(long)]
        host: Option<String>,
        /// Listen port (overrides config.yaml and env vars)
        #[arg(long)]
        port: Option<u16>,
    },
    /// Run the MCP server
    Mcp {
        #[arg(value_parser = clap::value_parser!(McpCommand))]
        server: McpCommand,
    },
    /// Validate a bundled-extensions JSON file
    #[command(name = "validate-extensions")]
    ValidateExtensions {
        /// Path to the bundled-extensions JSON file
        path: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Capture panics to local crash reports for diagnostics (#299). Installed
    // first so a panic anywhere in startup is recorded. Bundling is consent-gated.
    permagent::session::crash_capture::install_panic_hook();

    let cli = Cli::parse();

    match cli.command {
        Commands::Agent { host, port } => {
            commands::agent::run(host, port).await?;
        }
        Commands::Mcp { server } => {
            logging::setup_logging(Some(&format!("mcp-{}", server.name())))?;
            match server {
                McpCommand::AutoVisualiser => serve(AutoVisualiserRouter::new()).await?,
                McpCommand::ComputerController => serve(ComputerControllerServer::new()).await?,
                McpCommand::Memory => serve(MemoryServer::new()).await?,
                McpCommand::Tutorial => serve(TutorialServer::new()).await?,
            }
        }
        Commands::ValidateExtensions { path } => {
            match validate_extensions::validate_bundled_extensions(&path) {
                Ok(msg) => println!("{msg}"),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}
