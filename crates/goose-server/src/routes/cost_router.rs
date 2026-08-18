//! Cost-router routing surface — the per-role model map, readable and
//! overridable from the app.
//!
//! The router routes each workflow role (orchestrate / edit / mechanical /
//! review / local) to the model best for the job, local or cloud, as cheaply as
//! it can. Until now that map had ZERO UI/HTTP surface — only the CLI
//! (`permagent packs recommend|apply|set|show|clear`,
//! `crates/goose-cli/src/commands/packs.rs`). This route is the daemon seam
//! Settings → Models → Routing renders, over exactly the same core:
//!
//!   - the recommender ([`permagent::cost_router::recommend`]) for the
//!     derived best-fit per role — including a live probe of the local Ollama
//!     daemon (≤ 3 s), so a user with five pulled models and one cloud key sees
//!     both sides;
//!   - the role map ([`permagent::cost_router::role_map`]) for the hand-set
//!     override per role. A hand-set role always wins; the writer here goes
//!     through `Config::global().set_param` on the SAME
//!     `PERMAGENT_ROLE_<ROLE>_{PROVIDER,MODEL}` keys the CLI writes, so `/config`
//!     and `permagent packs show` see it.
//!
//! Auth is handled by the bearer-token middleware (protected group).
//!
//! Endpoints:
//!   GET    /api/cost-router/roles         — every role: configured + recommended + fit + KB/discovery notes
//!   PUT    /api/cost-router/roles/{role}  — hand-set { provider, model } for a role; returns the row
//!   DELETE /api/cost-router/roles/{role}  — clear the hand-set mapping; returns the row

use axum::{
    extract::{Json, Path},
    http::StatusCode,
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use permagent::cost_router::{
    clear_role_model, discover_available_models_async, kb_is_stale, lookup_with_confidence,
    recommend_from_available, role_model, set_role_model, AvailableModel, LookupConfidence,
    Recommendation, RoleModel, RoleRecommendation, WorkflowRole, KB_SNAPSHOT_DATE,
};

use crate::state::AppState;

// ── Wire types ───────────────────────────────────────────────────────────

/// A concrete provider+model pair on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleModelView {
    pub provider: String,
    pub model: String,
}

impl From<RoleModel> for RoleModelView {
    fn from(rm: RoleModel) -> Self {
        Self {
            provider: rm.provider,
            model: rm.model,
        }
    }
}

/// One role's row: what the user hand-set (`configured`), what the recommender
/// derived (`recommended`), and how honest the derived pick is (`floor_met`,
/// `warnings`, `confidence`). Both may be `null` — the role then runs on the
/// session model, which the UI labels "session model (no fit)".
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RoleRow {
    pub role: WorkflowRole,
    /// Human label ("Orchestrate", "Edit", …).
    pub label: String,
    /// One-line description from [`WorkflowRole::description`].
    pub description: String,
    pub configured: Option<RoleModelView>,
    pub recommended: Option<RoleModelView>,
    /// Whether the recommended model clears the role's capability floor.
    /// `true` when there is no recommendation at all — nothing was under-fit,
    /// the role simply falls through to the session model.
    pub floor_met: bool,
    pub warnings: Vec<String>,
    /// How the recommended model matched its knowledge-base row (`exact`,
    /// `alias`, `family_estimate`); `null` when there is no recommendation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<LookupConfidence>,
    /// The recommender's transparent reasoning for the pick (may be empty).
    pub reason: String,
}

/// The knowledge-base snapshot the derived map is built from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KbView {
    pub snapshot_date: String,
    pub stale: bool,
}

/// What discovery found: the providers with a usable key (or keyless local),
/// and the local models actually pulled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiscoveredView {
    pub providers: Vec<String>,
    pub local_models: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RolesResponse {
    pub roles: Vec<RoleRow>,
    pub kb: KbView,
    pub discovered: DiscoveredView,
}

/// `PUT /api/cost-router/roles/{role}` body.
#[derive(Debug, Deserialize)]
pub struct SetRoleRequest {
    pub provider: String,
    pub model: String,
}

// ── Pure assembly ────────────────────────────────────────────────────────

fn role_label(role: WorkflowRole) -> String {
    let s = role.as_str();
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Build one role's row from the recommender's entry for it (if any) and the
/// hand-set mapping (if any). Pure — the unit-tested seam.
pub fn role_row(
    role: WorkflowRole,
    rec: Option<&RoleRecommendation>,
    configured: Option<RoleModel>,
) -> RoleRow {
    // A recommendation with an empty provider/model is the recommender saying
    // "nothing fits" (e.g. LOCAL with no on-device model) — surface as `null`,
    // not as an empty pair the UI would render as "/".
    let has_pick = rec.is_some_and(|r| !r.provider.is_empty() && !r.model.is_empty());
    let recommended = if has_pick {
        rec.map(|r| RoleModelView {
            provider: r.provider.clone(),
            model: r.model.clone(),
        })
    } else {
        None
    };
    let confidence = if has_pick {
        rec.and_then(|r| lookup_with_confidence(&r.provider, &r.model).map(|(_, c)| c))
    } else {
        None
    };
    RoleRow {
        role,
        label: role_label(role),
        description: role.description().to_string(),
        configured: configured.map(Into::into),
        recommended,
        floor_met: rec.is_none_or(|r| r.floor_met),
        warnings: rec.map(|r| r.warnings.clone()).unwrap_or_default(),
        confidence,
        reason: rec.map(|r| r.reason.clone()).unwrap_or_default(),
    }
}

/// Project the discovered model list into the providers seen and the local
/// (Ollama) models pulled. Deduped, in discovery order (already sorted).
pub fn discovered_view(available: &[AvailableModel]) -> DiscoveredView {
    let mut providers: Vec<String> = Vec::new();
    let mut local_models: Vec<String> = Vec::new();
    for a in available {
        if !providers.contains(&a.provider) {
            providers.push(a.provider.clone());
        }
        if a.provider == "ollama" && !local_models.contains(&a.model) {
            local_models.push(a.model.clone());
        }
    }
    DiscoveredView {
        providers,
        local_models,
    }
}

/// Assemble the full response from the recommendation, the discovered list, a
/// per-role reader for the hand-set mapping, and today's date (for the KB
/// staleness note). Pure over its inputs so it is testable without config or a
/// network probe.
pub fn build_roles_response(
    rec: &Recommendation,
    available: &[AvailableModel],
    configured: impl Fn(WorkflowRole) -> Option<RoleModel>,
    today: chrono::NaiveDate,
) -> RolesResponse {
    let roles = WorkflowRole::all()
        .into_iter()
        .map(|role| {
            let r = rec.recommendations.iter().find(|r| r.role == role);
            role_row(role, r, configured(role))
        })
        .collect();
    RolesResponse {
        roles,
        kb: KbView {
            snapshot_date: KB_SNAPSHOT_DATE.to_string(),
            stale: kb_is_stale(today),
        },
        discovered: discovered_view(available),
    }
}

/// Parse the `{role}` path segment; 400 with the accepted set on a miss.
fn parse_role(tag: &str) -> Result<WorkflowRole, (StatusCode, String)> {
    WorkflowRole::from_tag(tag).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            format!(
                "unknown role '{tag}' — expected one of: orchestrate (alias: hard), edit, \
                 mechanical, review, local"
            ),
        )
    })
}

// ── Handlers ─────────────────────────────────────────────────────────────

/// Discover (with the live Ollama probe) and recommend — the one IO step every
/// handler shares, so PUT/DELETE return a row consistent with GET.
async fn discover_and_recommend() -> (Vec<AvailableModel>, Recommendation) {
    let available = discover_available_models_async().await;
    let rec = recommend_from_available(&available);
    (available, rec)
}

async fn get_roles() -> Json<RolesResponse> {
    let (available, rec) = discover_and_recommend().await;
    Json(build_roles_response(
        &rec,
        &available,
        role_model,
        chrono::Utc::now().date_naive(),
    ))
}

async fn row_for(role: WorkflowRole) -> RoleRow {
    let (_, rec) = discover_and_recommend().await;
    let r = rec.recommendations.iter().find(|r| r.role == role);
    role_row(role, r, role_model(role))
}

async fn put_role(
    Path(role): Path<String>,
    Json(req): Json<SetRoleRequest>,
) -> Result<Json<RoleRow>, (StatusCode, String)> {
    let role = parse_role(&role)?;
    let (provider, model) = (req.provider.trim(), req.model.trim());
    if provider.is_empty() || model.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "provider and model must both be non-empty".to_string(),
        ));
    }
    set_role_model(role, provider, model).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to persist {}: {e}", role.as_str()),
        )
    })?;
    Ok(Json(row_for(role).await))
}

async fn delete_role(Path(role): Path<String>) -> Result<Json<RoleRow>, (StatusCode, String)> {
    let role = parse_role(&role)?;
    clear_role_model(role).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to clear {}: {e}", role.as_str()),
        )
    })?;
    Ok(Json(row_for(role).await))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/cost-router/roles", get(get_roles))
        .route(
            "/api/cost-router/roles/{role}",
            axum::routing::put(put_role).delete(delete_role),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(role: WorkflowRole, provider: &str, model: &str, floor_met: bool) -> RoleRecommendation {
        RoleRecommendation {
            role,
            provider: provider.to_string(),
            model: model.to_string(),
            display_name: model.to_string(),
            family: provider.to_string(),
            blended_cost_per_mtok: 0.0,
            reason: format!("picked {provider}/{model}"),
            warnings: Vec::new(),
            floor_met,
        }
    }

    #[test]
    fn row_carries_configured_and_recommended_independently() {
        let r = rec(WorkflowRole::Edit, "openai", "gpt-5.6", true);
        let row = role_row(
            WorkflowRole::Edit,
            Some(&r),
            Some(RoleModel {
                provider: "anthropic".into(),
                model: "claude-sonnet-5".into(),
            }),
        );
        assert_eq!(row.role, WorkflowRole::Edit);
        assert_eq!(row.label, "Edit");
        assert_eq!(row.description, WorkflowRole::Edit.description());
        assert_eq!(
            row.configured,
            Some(RoleModelView {
                provider: "anthropic".into(),
                model: "claude-sonnet-5".into()
            })
        );
        assert_eq!(
            row.recommended,
            Some(RoleModelView {
                provider: "openai".into(),
                model: "gpt-5.6".into()
            })
        );
        assert!(row.floor_met);
        assert_eq!(row.reason, "picked openai/gpt-5.6");
    }

    #[test]
    fn empty_recommendation_is_null_not_an_empty_pair() {
        // The recommender's "nothing fits" (empty provider/model — e.g. LOCAL with
        // no on-device model) must serialize as `null`, and floor_met stays true
        // (nothing was under-fit; the role just falls through).
        let r = rec(WorkflowRole::Local, "", "", true);
        let row = role_row(WorkflowRole::Local, Some(&r), None);
        assert_eq!(row.recommended, None);
        assert_eq!(row.configured, None);
        assert_eq!(row.confidence, None);
        assert!(row.floor_met);
        let json = serde_json::to_value(&row).unwrap();
        assert_eq!(json["recommended"], serde_json::Value::Null);
        assert_eq!(json["configured"], serde_json::Value::Null);
        assert_eq!(json["role"], "local");
        assert!(json.get("confidence").is_none());
    }

    #[test]
    fn under_fit_and_warnings_pass_through() {
        let mut r = rec(WorkflowRole::Orchestrate, "ollama", "qwen3", false);
        r.warnings.push("below the capability floor".to_string());
        let row = role_row(WorkflowRole::Orchestrate, Some(&r), None);
        assert!(!row.floor_met);
        assert_eq!(row.warnings, vec!["below the capability floor".to_string()]);
    }

    #[test]
    fn confidence_reflects_kb_lookup() {
        // A family-tagged Ollama id resolves as an estimate; an unknown id has no
        // confidence at all (still shown as the recommendation, unlabelled).
        let est = rec(WorkflowRole::Mechanical, "ollama", "qwen3-coder:30b", true);
        let row = role_row(WorkflowRole::Mechanical, Some(&est), None);
        assert_eq!(row.confidence, Some(LookupConfidence::FamilyEstimate));
        let json = serde_json::to_value(&row).unwrap();
        assert_eq!(json["confidence"], "family_estimate");

        let unknown = rec(WorkflowRole::Mechanical, "custom", "mystery-1", true);
        let row = role_row(WorkflowRole::Mechanical, Some(&unknown), None);
        assert_eq!(row.confidence, None);
        assert!(row.recommended.is_some());
    }

    #[test]
    fn no_recommendation_row_is_session_model_fallthrough() {
        let row = role_row(WorkflowRole::Review, None, None);
        assert_eq!(row.recommended, None);
        assert_eq!(row.configured, None);
        assert!(row.floor_met);
        assert!(row.warnings.is_empty());
        assert_eq!(row.reason, "");
    }

    #[test]
    fn response_has_every_role_in_order_and_kb_and_discovery() {
        let available = vec![
            AvailableModel::new("anthropic", "claude-sonnet-5"),
            AvailableModel::new("ollama", "qwen3-coder:30b"),
            AvailableModel::new("ollama", "qwen3:latest"),
            AvailableModel::new("anthropic", "claude-haiku-4-5"),
        ];
        let recommendation = Recommendation {
            recommendations: vec![rec(
                WorkflowRole::Edit,
                "anthropic",
                "claude-sonnet-5",
                true,
            )],
            considered: vec![],
            unknown_models: vec![],
            estimated_models: vec![],
        };
        let configured = |role: WorkflowRole| {
            (role == WorkflowRole::Mechanical).then(|| RoleModel {
                provider: "ollama".into(),
                model: "qwen3-coder:30b".into(),
            })
        };
        // A date well past the snapshot → stale; the snapshot date itself → fresh.
        let far = chrono::NaiveDate::from_ymd_opt(2030, 1, 1).unwrap();
        let resp = build_roles_response(&recommendation, &available, configured, far);

        let roles: Vec<WorkflowRole> = resp.roles.iter().map(|r| r.role).collect();
        assert_eq!(roles, WorkflowRole::all().to_vec());
        assert!(resp.roles[1].recommended.is_some(), "edit has a rec");
        assert!(resp.roles[2].configured.is_some(), "mechanical is hand-set");
        assert!(resp.roles[0].recommended.is_none() && resp.roles[0].configured.is_none());

        assert_eq!(resp.kb.snapshot_date, KB_SNAPSHOT_DATE);
        assert!(resp.kb.stale);
        assert_eq!(resp.discovered.providers, vec!["anthropic", "ollama"]);
        assert_eq!(
            resp.discovered.local_models,
            vec!["qwen3-coder:30b", "qwen3:latest"]
        );

        let fresh = build_roles_response(
            &recommendation,
            &available,
            configured,
            permagent::cost_router::kb_snapshot_date(),
        );
        assert!(!fresh.kb.stale);
    }

    #[test]
    fn role_path_segment_parses_with_aliases_and_rejects_unknown() {
        assert_eq!(parse_role("edit").unwrap(), WorkflowRole::Edit);
        assert_eq!(parse_role("HARD").unwrap(), WorkflowRole::Orchestrate);
        let (status, msg) = parse_role("nonsense").unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(msg.contains("nonsense"));
    }
}
