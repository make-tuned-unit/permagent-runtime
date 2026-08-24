mod agent_state_tick;
mod analytics;
mod analytics_drain;
mod app_catalog;
mod automation;
pub(crate) use permagent::backup;
mod brain_ops;
mod commands;
mod concierge;
mod configuration;
mod device_registry;
mod error;
mod event_at_backfill;
mod growth_sweep;
mod logging;
mod middleware;
mod notification_router;
mod openapi;
mod proactive;
/// Pin this test binary's config to a temp root before any test body runs.
///
/// `lib.rs` arms the identical initialiser (#1017) and this file did not — but
/// `main.rs` re-declares `mod routes` and `mod state`, so every route test is
/// compiled a SECOND time into the `permagentd` bin test binary. Nothing pinned
/// the root there before the process-global `SESSION_STORAGE` `LazyLock`
/// captured `Paths::spectral_db()`, so it captured the developer's real
/// `~/.permagent` and those tests wrote fixture projects into the user's own
/// database. Forty of them turned up in the Projects tab.
///
/// `test_support::test_root()` is not sufficient alone: it pins correctly, but
/// only from the first test that calls it, and the global pool is fixed by
/// whatever touches it first. `test_support::tests::
/// the_session_db_never_resolves_to_the_real_permagent_dir` compiles into both
/// roots and fails loudly if either ever loses this again.
#[cfg(test)]
#[ctor::ctor(unsafe)]
fn pin_config_for_daemon_bin_tests() {
    permagent::config::base::pin_config_to_temp_root_for_tests();
}

mod finance_rsi_sweep;
mod picker_close_scan;
mod routes;
mod session_event_bus;
mod state;
mod steward_sweep;
mod strix;
mod watcher_insights;
// The route modules are compiled into BOTH crate roots (this bin and the lib),
// so the shared test-support helper their `#[cfg(test)]` modules reference must
// be declared in both roots — see the matching decl in lib.rs (#858).
#[cfg(test)]
mod test_support;
mod tunnel;
// Some verification items (test-support consts/fields) are only consumed by
// the lib target's tests; the bin needs the module for the startup hook wire.
#[allow(dead_code)]
mod verification;
mod voice;
mod wal_checkpoint;

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
