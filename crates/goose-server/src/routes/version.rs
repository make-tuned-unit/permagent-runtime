use axum::{routing::get, Json, Router};
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;

use crate::state::AppState;

static BOOT_TIME: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);

#[derive(Serialize, utoipa::ToSchema)]
pub struct VersionInfo {
    git_sha: &'static str,
    git_branch: &'static str,
    build_timestamp: &'static str,
    rust_version: &'static str,
    spectral_pin: &'static str,
    uptime_seconds: u64,
    permagentd_version: &'static str,
}

#[utoipa::path(get, path = "/api/version",
    responses(
        (status = 200, description = "Daemon build metadata and uptime", body = VersionInfo),
    )
)]
async fn version() -> Json<VersionInfo> {
    Json(VersionInfo {
        git_sha: env!("PERMAGENT_GIT_SHA"),
        git_branch: env!("PERMAGENT_GIT_BRANCH"),
        build_timestamp: env!("PERMAGENT_BUILD_TIMESTAMP"),
        rust_version: env!("PERMAGENT_RUST_VERSION"),
        spectral_pin: env!("PERMAGENT_SPECTRAL_PIN"),
        uptime_seconds: BOOT_TIME.elapsed().as_secs(),
        permagentd_version: env!("CARGO_PKG_VERSION"),
    })
}

pub fn routes(state: Arc<AppState>) -> Router {
    // Initialize boot time on first route registration
    let _ = *BOOT_TIME;
    Router::new()
        .route("/api/version", get(version))
        .with_state(state)
}
