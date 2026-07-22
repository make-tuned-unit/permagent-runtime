use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use permagent::config::Config;
#[cfg(feature = "telemetry")]
use permagent::posthog::emit_event;
use permagent::posthog::TELEMETRY_ENABLED_KEY;
use permagent::session::crash_capture::{
    crash_reports_consented, export_redacted_bundle, CRASH_REPORTS_CONSENT_KEY,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use utoipa::ToSchema;

use crate::routes::errors::ErrorResponse;
use crate::state::AppState;

#[derive(Debug, Deserialize, ToSchema)]
pub struct TelemetryEventRequest {
    pub event_name: String,
    #[serde(default)]
    pub properties: HashMap<String, serde_json::Value>,
}

#[utoipa::path(
    post,
    path = "/telemetry/event",
    request_body = TelemetryEventRequest,
    responses(
        (status = 202, description = "Event accepted for processing")
    )
)]
async fn send_telemetry_event(
    State(_state): State<Arc<AppState>>,
    Json(request): Json<TelemetryEventRequest>,
) -> StatusCode {
    let event_name = request.event_name;
    let properties = request.properties;

    #[cfg(feature = "telemetry")]
    tokio::spawn(async move {
        if let Err(e) = emit_event(&event_name, properties).await {
            tracing::debug!("Failed to send telemetry event: {}", e);
        }
    });

    StatusCode::ACCEPTED
}

/// Whether the product-analytics telemetry opt-in (`GOOSE_TELEMETRY_ENABLED`)
/// is on. Read directly from config so this is authoritative and does not
/// require the `telemetry` feature. Off by default (explicit opt-in).
fn analytics_consented() -> bool {
    Config::global()
        .get_param::<bool>(TELEMETRY_ENABLED_KEY)
        .unwrap_or(false)
}

/// Current diagnostics consent — **two independent** opt-ins (#327 split).
///
/// - `crashReportsConsented`: the authoritative crash-report gate
///   ([`permagent::session::crash_capture::crash_reports_consented`]), reading
///   the dedicated `crash_reports_consent` key.
/// - `analyticsConsented`: the product-analytics telemetry opt-in
///   (`GOOSE_TELEMETRY_ENABLED`).
///
/// Both are **off by default** (explicit opt-in) and settable independently, so
/// a user can help fix crashes without sharing usage analytics, or vice versa.
/// The Settings toggles render from these, never a hardcoded UI default (#845).
#[derive(Debug, Serialize, ToSchema)]
pub struct ConsentStatus {
    /// Whether the user has consented to sharing crash reports.
    #[serde(rename = "crashReportsConsented")]
    pub crash_reports_consented: bool,
    /// Whether the user has consented to product-analytics telemetry.
    #[serde(rename = "analyticsConsented")]
    pub analytics_consented: bool,
}

impl ConsentStatus {
    fn current() -> Self {
        Self {
            crash_reports_consented: crash_reports_consented(),
            analytics_consented: analytics_consented(),
        }
    }
}

/// Update either or both consent toggles. Omitted fields are left unchanged.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetConsentRequest {
    /// Desired crash-report sharing consent. `None` leaves it unchanged.
    #[serde(rename = "crashReports", default)]
    pub crash_reports: Option<bool>,
    /// Desired product-analytics telemetry consent. `None` leaves it unchanged.
    #[serde(rename = "analytics", default)]
    pub analytics: Option<bool>,
}

#[utoipa::path(
    get,
    path = "/telemetry/consent",
    responses(
        (status = 200, description = "Current crash-report + analytics consent", body = ConsentStatus)
    )
)]
async fn get_telemetry_consent() -> Json<ConsentStatus> {
    Json(ConsentStatus::current())
}

#[utoipa::path(
    post,
    path = "/telemetry/consent",
    request_body = SetConsentRequest,
    responses(
        (status = 200, description = "Consent updated; returns the authoritative values", body = ConsentStatus)
    )
)]
async fn set_telemetry_consent(
    Json(request): Json<SetConsentRequest>,
) -> Result<Json<ConsentStatus>, ErrorResponse> {
    // Two independent keys (#327 split). We write only what was provided, then
    // re-read both gates so the response is the authoritative value, not the
    // request echo — an env override can still force a value off.
    if let Some(crash) = request.crash_reports {
        Config::global().set_param(CRASH_REPORTS_CONSENT_KEY, crash)?;
    }
    if let Some(analytics) = request.analytics {
        Config::global().set_param(TELEMETRY_ENABLED_KEY, analytics)?;
    }
    Ok(Json(ConsentStatus::current()))
}

/// A locally-written, redacted crash-report export (#327 MVP).
#[derive(Debug, Serialize, ToSchema)]
pub struct CrashExportResponse {
    /// Absolute path of the redacted bundle written to local disk.
    pub path: String,
    /// Number of captured crash reports included.
    #[serde(rename = "reportCount")]
    pub report_count: usize,
    /// The exact redacted bundle text, so the UI can show the user what would be
    /// shared before they choose to attach it anywhere.
    pub content: String,
}

#[utoipa::path(
    post,
    path = "/telemetry/crash-report/export",
    responses(
        (status = 200, description = "Redacted crash-report bundle written locally", body = CrashExportResponse)
    )
)]
async fn export_crash_report() -> Result<Json<CrashExportResponse>, ErrorResponse> {
    // Local-only, sovereign-safe: writes a REDACTED bundle to disk and returns
    // it for preview. No network path — the user is the actor and decides
    // whether to share the file. Not gated on consent (that gates ambient
    // sharing) and never suppressed by sovereign mode (a local write is not
    // egress).
    let export = export_redacted_bundle()
        .map_err(|e| ErrorResponse::internal(format!("crash-report export failed: {e}")))?;
    Ok(Json(CrashExportResponse {
        path: export.path.to_string_lossy().into_owned(),
        report_count: export.report_count,
        content: export.content,
    }))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/telemetry/event", post(send_telemetry_event))
        .route("/telemetry/consent", get(get_telemetry_consent))
        .route("/telemetry/consent", post(set_telemetry_consent))
        .route("/telemetry/crash-report/export", post(export_crash_report))
        .with_state(state)
}
