//! Sovereignty controls + the egress audit log.
//!
//! The sovereignty guarantee: with sovereign mode on, all **cloud** inference is
//! blocked (fail-closed) and never happens; every cloud call (blocked or, when
//! mode is off, allowed) is recorded in an append-only local audit log. These
//! endpoints let the UI toggle the boundary and read "everything that has left
//! this machine, and when".
//!
//! Auth is handled by the bearer-token middleware (protected group).
//!
//! Endpoints:
//!   GET  /api/security/sovereignty  — current status
//!   POST /api/security/sovereignty  — set { enabled?, capturePrompts?, strictAudit? }
//!   GET  /api/security/egress-log   — recent cloud-egress audit entries (?limit=)

use axum::{
    extract::{Json, Query},
    http::StatusCode,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use permagent::config::{Config, ConfigError};
use permagent::providers::providers as list_providers;
use permagent::sovereignty::{self, EgressLogEntry};

use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SovereigntyStatus {
    /// Global sovereign mode: when on, all cloud inference is blocked + audited.
    enabled: bool,
    /// Whether the audit log captures full prompts (vs a hash only).
    capture_prompts: bool,
    /// Strict (fail-closed) audit: when on, an egress-audit write failure for
    /// an *allowed* cloud call fails the call itself (no unlogged egress).
    strict_audit: bool,
    /// Whether a LOCAL inference provider (`local` or `ollama`) is registered,
    /// so sovereign work has somewhere to run rather than only being refused.
    local_provider_available: bool,
}

async fn current_status() -> SovereigntyStatus {
    SovereigntyStatus {
        enabled: sovereignty::global_sovereign_mode(),
        capture_prompts: sovereignty::capture_prompts_enabled(),
        strict_audit: sovereignty::strict_audit_enabled(),
        local_provider_available: local_provider_available().await,
    }
}

/// A local provider is present if the built-in in-process `local` provider or
/// `ollama` is registered.
async fn local_provider_available() -> bool {
    list_providers()
        .await
        .iter()
        .any(|(m, _)| m.name == "local" || m.name == "ollama")
}

async fn get_status() -> Json<SovereigntyStatus> {
    Json(current_status().await)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetSovereigntyRequest {
    enabled: Option<bool>,
    capture_prompts: Option<bool>,
    /// Additive (#770 follow-up): omitting it leaves the knob untouched, so
    /// pre-existing clients that only send `enabled`/`capturePrompts` are
    /// unaffected.
    strict_audit: Option<bool>,
}

/// Apply the plain config-backed knobs of a patch to an explicit `Config` —
/// split out of [`set_status`] so the write path is unit-testable against a
/// temp config file instead of the process-global one. `enabled` is NOT
/// handled here: it goes through [`sovereignty::set_global_sovereign_mode`],
/// which also updates the process-wide mode cache.
fn apply_config_knobs(req: &SetSovereigntyRequest, config: &Config) -> Result<(), ConfigError> {
    if let Some(capture) = req.capture_prompts {
        config.set_param(sovereignty::SOVEREIGN_CAPTURE_PROMPTS_KEY, capture)?;
    }
    if let Some(strict) = req.strict_audit {
        config.set_param(sovereignty::SOVEREIGN_STRICT_AUDIT_KEY, strict)?;
    }
    Ok(())
}

async fn set_status(
    Json(req): Json<SetSovereigntyRequest>,
) -> Result<Json<SovereigntyStatus>, (StatusCode, String)> {
    if let Some(enabled) = req.enabled {
        sovereignty::set_global_sovereign_mode(enabled)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    apply_config_knobs(&req, Config::global())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(current_status().await))
}

#[derive(Deserialize)]
struct LogQuery {
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    100
}

async fn get_egress_log(
    Query(params): Query<LogQuery>,
) -> Result<Json<Vec<EgressLogEntry>>, (StatusCode, String)> {
    let limit = params.limit.clamp(1, 1000);
    let entries = sovereignty::recent_egress(limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(entries))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/security/sovereignty", get(get_status))
        .route("/api/security/sovereignty", post(set_status))
        .route("/api/security/egress-log", get(get_egress_log))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GET surfaces the strict-audit knob beside the main toggle, additively:
    /// the new camelCase `strictAudit` field appears WITH the pre-existing
    /// fields intact. Reads ride the env-override seam (`get_param` checks the
    /// uppercased env var first), so the REAL read path is exercised without
    /// touching the process-global config file.
    #[tokio::test]
    async fn status_reports_strict_audit_additively() {
        let _guard = env_lock::lock_env([
            ("SOVEREIGN_MODE", Some("false")),
            ("SOVEREIGN_CAPTURE_PROMPTS", Some("false")),
            ("SOVEREIGN_STRICT_AUDIT", Some("true")),
        ]);
        // The mode cache is process-global; force a reload from the pinned env
        // on both sides so this test neither reads nor leaves stale state.
        permagent::sovereignty::invalidate_global_mode_cache();

        let status = current_status().await;
        assert!(status.strict_audit);
        assert!(!status.enabled);
        assert!(!status.capture_prompts);

        let wire = serde_json::to_value(&status).unwrap();
        assert_eq!(wire["strictAudit"], serde_json::json!(true));
        for existing in ["enabled", "capturePrompts", "localProviderAvailable"] {
            assert!(
                wire.get(existing).is_some(),
                "back-compat: pre-existing field {existing:?} must survive the additive change"
            );
        }

        permagent::sovereignty::invalidate_global_mode_cache();
    }

    /// Wire back-compat for POST: a legacy body without `strictAudit` parses
    /// (knob untouched), and the new field parses beside the old ones.
    #[test]
    fn set_request_parses_legacy_and_strict_audit_bodies() {
        let legacy: SetSovereigntyRequest = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert_eq!(legacy.enabled, Some(true));
        assert_eq!(legacy.capture_prompts, None);
        assert_eq!(legacy.strict_audit, None);

        let patch: SetSovereigntyRequest =
            serde_json::from_str(r#"{"strictAudit":true,"capturePrompts":false}"#).unwrap();
        assert_eq!(patch.enabled, None);
        assert_eq!(patch.capture_prompts, Some(false));
        assert_eq!(patch.strict_audit, Some(true));
    }

    /// The POST write path persists the knob under the real config key and
    /// round-trips both directions — proven against a temp `Config`, never the
    /// process-global one.
    #[test]
    fn apply_config_knobs_persists_strict_audit() {
        let tmp = tempfile::tempdir().unwrap();
        let config = Config::new(tmp.path().join("config.yaml"), "permagent-test").unwrap();
        // The read-back must come from the file, not an ambient env override.
        let _guard = env_lock::lock_env([
            ("SOVEREIGN_STRICT_AUDIT", None::<&str>),
            ("SOVEREIGN_CAPTURE_PROMPTS", None::<&str>),
        ]);

        let on = SetSovereigntyRequest {
            enabled: None,
            capture_prompts: None,
            strict_audit: Some(true),
        };
        apply_config_knobs(&on, &config).unwrap();
        assert!(config
            .get_param::<bool>(sovereignty::SOVEREIGN_STRICT_AUDIT_KEY)
            .unwrap());

        let off = SetSovereigntyRequest {
            enabled: None,
            capture_prompts: Some(true),
            strict_audit: Some(false),
        };
        apply_config_knobs(&off, &config).unwrap();
        assert!(!config
            .get_param::<bool>(sovereignty::SOVEREIGN_STRICT_AUDIT_KEY)
            .unwrap());
        assert!(config
            .get_param::<bool>(sovereignty::SOVEREIGN_CAPTURE_PROMPTS_KEY)
            .unwrap());

        // Omitted knob = untouched: a None patch must not write the key.
        let tmp2 = tempfile::tempdir().unwrap();
        let fresh = Config::new(tmp2.path().join("config.yaml"), "permagent-test").unwrap();
        let noop = SetSovereigntyRequest {
            enabled: None,
            capture_prompts: None,
            strict_audit: None,
        };
        apply_config_knobs(&noop, &fresh).unwrap();
        assert!(fresh
            .get_param::<bool>(sovereignty::SOVEREIGN_STRICT_AUDIT_KEY)
            .is_err());
    }
}
