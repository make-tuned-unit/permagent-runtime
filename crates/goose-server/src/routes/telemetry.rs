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
use permagent::session::crash_capture::crash_reports_consented;
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

/// Current diagnostics/crash-report sharing consent.
///
/// `crashReportsConsented` is the authoritative backend gate
/// ([`permagent::session::crash_capture::crash_reports_consented`]) — the same
/// value that decides whether crash reports may be bundled. It is **off by
/// default** (explicit opt-in). The Settings "Share anonymous diagnostics"
/// toggle must render from this, not from a hardcoded UI default (#845).
#[derive(Debug, Serialize, ToSchema)]
pub struct ConsentStatus {
    /// Whether the user has consented to sharing crash reports / diagnostics.
    #[serde(rename = "crashReportsConsented")]
    pub crash_reports_consented: bool,
}

impl ConsentStatus {
    fn current() -> Self {
        Self {
            crash_reports_consented: crash_reports_consented(),
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetConsentRequest {
    /// Desired consent value. `true` opts in to sharing crash reports /
    /// diagnostics; `false` opts back out.
    pub consented: bool,
}

#[utoipa::path(
    get,
    path = "/telemetry/consent",
    responses(
        (status = 200, description = "Current crash-report/diagnostics consent", body = ConsentStatus)
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
        (status = 200, description = "Consent updated; returns the authoritative value", body = ConsentStatus)
    )
)]
async fn set_telemetry_consent(
    Json(request): Json<SetConsentRequest>,
) -> Result<Json<ConsentStatus>, ErrorResponse> {
    // Writes the shared telemetry opt-in, which the crash-capture consent gate
    // reuses (see crash_capture::crash_reports_consented). We re-read the gate
    // after writing so the response is the authoritative value, not the request
    // echo — an env override (GOOSE_TELEMETRY_OFF) can still force it off.
    Config::global().set_param(TELEMETRY_ENABLED_KEY, request.consented)?;
    Ok(Json(ConsentStatus::current()))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/telemetry/event", post(send_telemetry_event))
        .route("/telemetry/consent", get(get_telemetry_consent))
        .route("/telemetry/consent", post(set_telemetry_consent))
        .with_state(state)
}
