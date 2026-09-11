use anyhow::Result;
use permagent_cli::cli::cli;

#[tokio::main]
async fn main() -> Result<()> {
    // Y16c (harness loop, G19 HIGH): stamp process start (unix seconds) as
    // early as possible so a headless goal's wallclock cap
    // (`headless_wallclock_cap_patch`, crates/goose — which cannot see this
    // binary's own start `Instant` across the crate boundary) can be
    // anchored to real elapsed process time instead of only the goal's own
    // (possibly late) `first_dispatched_at`. Never overrides an
    // already-set value: a wrapper that stamped this even earlier (e.g.
    // before spawning this binary) wins.
    if std::env::var("PERMAGENT_HEADLESS_STARTED_AT").is_err() {
        std::env::set_var(
            "PERMAGENT_HEADLESS_STARTED_AT",
            chrono::Utc::now().timestamp().to_string(),
        );
    }

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
