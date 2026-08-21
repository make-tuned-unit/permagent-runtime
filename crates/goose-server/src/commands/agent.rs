use crate::configuration;
use crate::state;
use anyhow::Result;
use axum_server::Handle;
use permagent::events;
#[cfg(any(feature = "rustls-tls", feature = "native-tls"))]
use permagent_daemon::tls::setup_tls;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::info;

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};

    let mut sigint = signal(SignalKind::interrupt()).expect("failed to install SIGINT handler");
    let mut sigterm = signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");

    tokio::select! {
        _ = sigint.recv() => {},
        _ = sigterm.recv() => {},
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

pub async fn run(host: Option<String>, port: Option<u16>) -> Result<()> {
    // Install the rustls crypto provider early, before any spawned tasks (tunnel,
    // gateways, etc.) try to open TLS connections. Both `ring` and `aws-lc-rs`
    // features are enabled on rustls (via different transitive deps), so rustls
    // cannot auto-detect a provider — we must pick one explicitly.
    #[cfg(feature = "rustls-tls")]
    let _ = rustls::crypto::ring::default_provider().install_default();

    crate::logging::setup_logging(Some("permagentd"))?;

    // Cap the launchd-managed daemon.err/daemon.log files (#560 F5) — the
    // HTTP access log below raises their growth rate.
    crate::logging::spawn_launchd_log_rotation();

    let settings = configuration::Settings::new()?.with_overrides(host, port);

    let app_state = state::AppState::new(settings.tls).await?;

    // Seed preset workspaces if user has none
    if let Ok(pool) = app_state.session_manager().pool_clone().await {
        match permagent::workspaces::seed_presets_if_empty(&pool).await {
            Ok(true) => info!("Seeded preset workspaces"),
            Ok(false) => {}
            Err(e) => tracing::warn!("Failed to seed workspaces: {}", e),
        }
        // Additive migration: ensure Projects workspace exists for existing users
        match permagent::workspaces::ensure_projects_workspace(&pool).await {
            Ok(true) => info!("Added Projects workspace for existing user"),
            Ok(false) => {}
            Err(e) => tracing::warn!("Failed to add Projects workspace: {}", e),
        }
        // Additive migration: ensure Grow workspace exists for existing users
        match permagent::workspaces::ensure_grow_workspace(&pool).await {
            Ok(true) => info!("Added Grow workspace for existing user"),
            Ok(false) => {}
            Err(e) => tracing::warn!("Failed to add Grow workspace: {}", e),
        }
        match permagent::workspaces::ensure_people_workspace(&pool).await {
            Ok(true) => info!("Added People workspace for existing user"),
            Ok(false) => {}
            Err(e) => tracing::warn!("Failed to add People workspace: {}", e),
        }
        match permagent::workspaces::ensure_finance_workspace(&pool).await {
            Ok(true) => info!("Added Finance workspace for existing user"),
            Ok(false) => {}
            Err(e) => tracing::warn!("Failed to add Finance workspace: {}", e),
        }
        // Normalize preset sidebar order to the code-owned canon (runs every
        // start; user-created workspaces untouched).
        match permagent::workspaces::ensure_canonical_workspace_order(&pool).await {
            Ok(true) => info!("Reordered sidebar workspaces to the canonical order"),
            Ok(false) => {}
            Err(e) => tracing::warn!("Failed to normalize workspace order: {}", e),
        }
    }

    // Initialize the default provider from config.yaml so new sessions work
    // immediately. Runs on EVERY daemon start (CLI-spawned or launchctl-loaded)
    // so runtime state matches config.yaml, the single source of truth.
    //
    // SPAWNED, NOT AWAITED — this is load-bearing. It used to be awaited right
    // here, before the router was built and long before the listener bound, so
    // a slow provider took the ENTIRE daemon with it. Observed 2026-07-30:
    // `providers::create("anthropic", …)` did not return for 5h22m. The process
    // looked perfectly healthy the whole time — background timers ticked, WAL
    // checkpoints ran, the log showed a clean 2-second boot — while nothing
    // could reach it, because the bind was 190 lines further down the same
    // function. It bound 20ms after the call finally returned.
    //
    // Nothing about serving the UI, projects, sessions or analytics needs a
    // model provider, so it must never gate the listener.
    tokio::spawn(init_default_provider(app_state.clone()));

    // Generate a random secret for the tunnel forwarding protocol.
    // The local server no longer checks this header — it is only used by the
    // tunnel proxy to authenticate forwarded requests to the remote endpoint.
    let tunnel_secret = hex::encode(rand::random::<[u8; 32]>());
    app_state
        .tunnel_manager
        .set_server_secret(tunnel_secret)
        .await;

    // CORS tightened from `Any` to the known app origins (C1/C2 audit): the
    // Tauri webview, loopback dev/served origins, same-origin companion
    // devices, plus `PERMAGENT_ALLOWED_ORIGINS` extras. Same allowlist as the
    // server-side origin guard (which enforces even when CORS wouldn't).
    // `last-event-id` is sent by EventSource on cross-origin auto-reconnects
    // (the Tauri webview's SSE to 127.0.0.1:3001 is cross-origin).
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, parts| {
            let host = parts
                .headers
                .get(axum::http::header::HOST)
                .and_then(|v| v.to_str().ok());
            origin.to_str().is_ok_and(|origin| {
                crate::middleware::origin_guard::origin_allowed(
                    origin,
                    host,
                    crate::middleware::origin_guard::env_allowed_origins().as_deref(),
                )
            })
        }))
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::DELETE,
            axum::http::Method::PATCH,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([
            axum::http::header::AUTHORIZATION,
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderName::from_static("last-event-id"),
        ]);

    let app = crate::routes::configure(app_state.clone()).layer(cors);

    let addr = settings.socket_addr();

    let tunnel_manager = app_state.tunnel_manager.clone();
    tokio::spawn(async move {
        tunnel_manager.check_auto_start().await;
    });

    let gateway_manager = app_state.gateway_manager.clone();
    tokio::spawn(async move {
        gateway_manager.check_auto_start().await;
    });

    // Echo / the Watcher (#672): gentle, rare proactive nudges from the hub.
    crate::proactive::spawn(app_state.clone());

    // Watcher project insights (2026-07-28): 1-2 grounded observations a day
    // per project, silently placed on the project Overview card.
    crate::watcher_insights::spawn(app_state.clone());
    // Pull web analytics from each project's own site relay (outbound only —
    // the daemon is never reachable from the internet).
    crate::analytics_drain::spawn(app_state.clone());
    // Re-evaluate open growth-action measurement windows. Local arithmetic over
    // events the drain above already pulled; no model call, no network.
    crate::growth_sweep::spawn(app_state.clone());

    // The Concierge (#640): flag-gated (`concierge_enabled`, default OFF),
    // draft-only, local-tier inbox triage. The loop always spawns and re-reads
    // the flag every tick, so a Settings → Features flip needs no restart.
    crate::concierge::spawn(app_state.clone());

    // Strix — the security sweep. No-ops unless `strix_enabled` is set, and
    // says so in the log either way.
    crate::strix::spawn(app_state.clone());

    // The Steward's git-health sweep. Detect/propose only (cleanup is
    // user-only via the Decision Inbox); no-ops unless `steward_scan_enabled`
    // is set, and says so in the log either way.
    crate::steward_sweep::spawn(app_state.clone());

    // RSI-14 heat on open holdings. Daily dedup; copy names the threshold,
    // never a sell. Independent of the Finance GET poll.
    crate::finance_rsi_sweep::spawn(app_state.clone());

    // Central notification policy (#66): classify workflow facts once, then
    // route them according to the user's channel thresholds.
    crate::notification_router::spawn(app_state.clone());

    // Emit daemon_started event after binding
    let version = env!("CARGO_PKG_VERSION");
    let config_path = std::env::var("PERMAGENT_CONFIG").unwrap_or_default();
    let spectral_path = std::env::var("PERMAGENT_SPECTRAL_DB").unwrap_or_default();

    if settings.tls {
        #[cfg(any(feature = "rustls-tls", feature = "native-tls"))]
        {
            let tls_setup = setup_tls(
                settings.tls_cert_path.as_deref(),
                settings.tls_key_path.as_deref(),
            )
            .await?;

            let handle = Handle::new();
            let shutdown_handle = handle.clone();
            tokio::spawn(async move {
                shutdown_signal().await;
                events::emit(events::daemon_stopped("user_request"));
                shutdown_handle.graceful_shutdown(None);
            });

            info!("listening on https://{}", addr);
            events::emit(events::daemon_started(
                version,
                &config_path,
                &spectral_path,
            ));

            #[cfg(feature = "rustls-tls")]
            axum_server::bind_rustls(addr, tls_setup.config)
                .handle(handle)
                .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>())
                .await?;

            #[cfg(feature = "native-tls")]
            axum_server::bind_openssl(addr, tls_setup.config)
                .handle(handle)
                .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>())
                .await?;
        }

        #[cfg(not(any(feature = "rustls-tls", feature = "native-tls")))]
        {
            anyhow::bail!(
                "TLS was requested but no TLS backend is enabled. \
                 Enable the `rustls-tls` or `native-tls` feature."
            );
        }
    } else {
        let listener = tokio::net::TcpListener::bind(addr).await?;

        info!("listening on http://{}", addr);
        events::emit(events::daemon_started(
            version,
            &config_path,
            &spectral_path,
        ));

        // ConnectInfo peer address is required by the loopback guard on the
        // browser bridge (#630); without it that guard fails closed.
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            shutdown_signal().await;
            events::emit(events::daemon_stopped("user_request"));
            // Brief pause to let WebSocket clients receive the event
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        })
        .await?;
    }

    #[cfg(feature = "otel")]
    if permagent::otel::otlp::is_otlp_initialized() {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        permagent::otel::otlp::shutdown_otlp();
    }

    info!("server shutdown complete");
    Ok(())
}

/// Build the configured default provider and install it, off the boot path.
///
/// Bounded: a provider that never answers reports itself instead of hanging
/// forever. The timeout is generous — a cold provider handshake on a slow link
/// is legitimate — but finite, which is the whole point. Failure is not fatal:
/// sessions surface a provider error, which is far better than an unreachable
/// daemon, and Settings can re-configure without a restart.
async fn init_default_provider(app_state: std::sync::Arc<state::AppState>) {
    const PROVIDER_INIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

    let config = permagent::config::Config::global();
    let Ok(name) = config.get_goose_provider() else {
        tracing::warn!(
            "GOOSE_PROVIDER not configured — run 'permagent setup' or configure via Settings"
        );
        return;
    };
    let model_name: Result<String, _> = config.get_goose_model();
    let model = match &model_name {
        Ok(m) => permagent::model::ModelConfig::new(m).ok(),
        Err(_) => None,
    };

    let started = std::time::Instant::now();
    // Spawned as its own task, and the deadline applied to the JOIN HANDLE
    // rather than to `create()` directly.
    //
    // `create()` resolves the provider's API key through `Config::get_secret`,
    // which reaches the macOS keychain via a synchronous FFI call
    // (`SecKeychainFindGenericPassword`). `tokio::time::timeout` cannot
    // interrupt a blocking sync call: it polls the inner future inline, so when
    // that poll blocks, the timeout's own timer never gets polled again and can
    // never fire. Awaiting `create()` directly therefore produced a deadline
    // that looked like a safety net and was not one — a wedged keychain hung
    // boot forever and NONE of the four arms below ever logged. The daemon
    // served HTTP normally, had no provider, and said nothing about why.
    //
    // Diagnosed by sampling a stuck process: all 397 samples in
    // SecKeychainFindGenericPassword beneath tokio::time::timeout::poll.
    //
    // With the work in a separate task the timeout lives in a different task
    // and does fire. The blocked task still holds a worker thread until the OS
    // call returns — that is a real remaining cost, and the honest trade is
    // that one wedged thread is much better than a silent daemon.
    let create_name = name.clone();
    let handle = tokio::spawn(async move {
        permagent::providers::create(&create_name, model.unwrap_or_default(), vec![]).await
    });
    match tokio::time::timeout(PROVIDER_INIT_TIMEOUT, handle).await {
        Ok(Err(join_err)) => {
            tracing::error!(
                "Default provider '{}' initializer panicked or was cancelled: {} — \
                 leaving it unset. Sessions will report a provider error until it is \
                 reconfigured in Settings.",
                name,
                join_err
            );
        }
        Ok(Ok(Ok(provider))) => {
            app_state.agent_manager.set_default_provider(provider).await;
            info!(
                "Default provider initialized: {} (model: {}) in {:?}",
                name,
                model_name.as_deref().unwrap_or("default"),
                started.elapsed()
            );
        }
        Ok(Ok(Err(e))) => {
            tracing::warn!("Failed to initialize default provider '{}': {}", name, e);
        }
        Err(_) => {
            tracing::error!(
                "Default provider '{}' did not initialize within {:?} — leaving it unset. \
                 Sessions will report a provider error until it is reconfigured in Settings. \
                 The daemon is otherwise serving normally. A common cause is a macOS \
                 keychain read that never returns; the credential is reachable again \
                 once the keychain is unlocked, and Settings can re-configure without \
                 a restart.",
                name,
                PROVIDER_INIT_TIMEOUT
            );
        }
    }
}
