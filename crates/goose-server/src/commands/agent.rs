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
        // Normalize preset sidebar order to the code-owned canon (runs every
        // start; user-created workspaces untouched).
        match permagent::workspaces::ensure_canonical_workspace_order(&pool).await {
            Ok(true) => info!("Reordered sidebar workspaces to the canonical order"),
            Ok(false) => {}
            Err(e) => tracing::warn!("Failed to normalize workspace order: {}", e),
        }
    }

    // Initialize default provider from config.yaml so new sessions work immediately.
    // This runs on EVERY daemon start (CLI-spawned or launchctl-loaded) to ensure
    // the runtime state matches config.yaml — the single source of truth.
    {
        let config = permagent::config::Config::global();
        let provider_name: Result<String, _> = config.get_goose_provider();
        let model_name: Result<String, _> = config.get_goose_model();

        match &provider_name {
            Ok(name) => {
                let model = match &model_name {
                    Ok(m) => permagent::model::ModelConfig::new(m).ok(),
                    Err(_) => None,
                };
                match permagent::providers::create(name, model.unwrap_or_default(), vec![]).await {
                    Ok(provider) => {
                        app_state.agent_manager.set_default_provider(provider).await;
                        info!(
                            "Default provider initialized: {} (model: {})",
                            name,
                            model_name.as_deref().unwrap_or("default")
                        );
                    }
                    Err(e) => {
                        tracing::warn!("Failed to initialize default provider '{}': {}", name, e);
                    }
                }
            }
            Err(_) => {
                tracing::warn!(
                    "GOOSE_PROVIDER not configured — run 'permagent setup' or configure via Settings"
                );
            }
        }
    }

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
