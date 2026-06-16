use anyhow::Result;
use permagent_cli::cli::cli;

#[tokio::main]
async fn main() -> Result<()> {
    // Capture panics to local crash reports for diagnostics (#299). Installed
    // before anything else so a panic during startup is recorded. Bundling is
    // consent-gated; capture is local-only (no upload — see #327).
    permagent::session::crash_capture::install_panic_hook();

    if let Err(e) = permagent_cli::logging::setup_logging(None) {
        eprintln!("Warning: Failed to initialize logging: {}", e);
    }

    let result = cli().await;

    #[cfg(feature = "otel")]
    if permagent::otel::otlp::is_otlp_initialized() {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        permagent::otel::otlp::shutdown_otlp();
    }

    result
}
