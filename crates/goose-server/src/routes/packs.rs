//! Role-routing packs — the HTTP seam for `permagent packs recommend` / `apply`.
//!
//! The CLI remains the source of truth for persistence (`mappings_to_persist` +
//! `set_role_model`). This module exposes the same triples so Command Center can
//! prompt the user to Apply without digging for the CLI.

use crate::routes::errors::ErrorResponse;
use axum::{
    routing::{get, post},
    Json, Router,
};
use permagent::cost_router::{
    configured_role_models, mappings_to_persist, recommend_configured_async, set_role_model,
    should_prompt_role_routing, Recommendation,
};
use serde::Serialize;

#[derive(Serialize)]
pub struct ConfiguredRole {
    pub role: String,
    pub provider: String,
    pub model: String,
}

#[derive(Serialize)]
pub struct PacksResponse {
    pub prompt: bool,
    pub configured: Vec<ConfiguredRole>,
    pub recommendation: Recommendation,
}

#[derive(Serialize)]
pub struct ApplyPacksResponse {
    pub applied: Vec<ConfiguredRole>,
}

#[utoipa::path(
    get,
    path = "/api/packs",
    responses((status = 200, description = "Recommendation plus configured role map")),
)]
pub async fn get_packs() -> Result<Json<PacksResponse>, ErrorResponse> {
    let recommendation = recommend_configured_async().await;
    let configured = configured_role_models()
        .into_iter()
        .map(|(role, rm)| ConfiguredRole {
            role: role.as_str().to_string(),
            provider: rm.provider,
            model: rm.model,
        })
        .collect::<Vec<_>>();
    let prompt = should_prompt_role_routing(configured.is_empty(), &recommendation.recommendations);
    Ok(Json(PacksResponse {
        prompt,
        configured,
        recommendation,
    }))
}

#[utoipa::path(
    post,
    path = "/api/packs/apply",
    responses((status = 200, description = "Persisted role→model mappings")),
)]
pub async fn apply_packs() -> Result<Json<ApplyPacksResponse>, ErrorResponse> {
    let recommendation = recommend_configured_async().await;
    let to_persist = mappings_to_persist(&recommendation.recommendations);
    if to_persist.is_empty() {
        return Ok(Json(ApplyPacksResponse {
            applied: Vec::new(),
        }));
    }
    let mut applied = Vec::new();
    for (role, provider, model) in to_persist {
        set_role_model(role, &provider, &model)?;
        applied.push(ConfiguredRole {
            role: role.as_str().to_string(),
            provider,
            model,
        });
    }
    permagent::agents::self_knowledge::usage::record_usage("cost_optimizer");
    Ok(Json(ApplyPacksResponse { applied }))
}

pub fn routes() -> Router {
    Router::new()
        .route("/api/packs", get(get_packs))
        .route("/api/packs/apply", post(apply_packs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use permagent::cost_router::{RoleRecommendation, WorkflowRole};

    fn rec(role: WorkflowRole, provider: &str, model: &str) -> RoleRecommendation {
        RoleRecommendation {
            role,
            provider: provider.to_string(),
            model: model.to_string(),
            display_name: model.to_string(),
            family: provider.to_string(),
            blended_cost_per_mtok: 0.0,
            reason: String::new(),
            warnings: Vec::new(),
            floor_met: true,
        }
    }

    #[test]
    fn prompt_is_true_when_single_model_and_two_distinct_recs() {
        let recs = vec![
            rec(WorkflowRole::Orchestrate, "anthropic", "claude-opus-4-6"),
            rec(WorkflowRole::Mechanical, "ollama", "qwen3"),
        ];
        assert!(
            should_prompt_role_routing(true, &recs),
            "unconfigured + two distinct recommended models must prompt Apply"
        );
        assert!(
            !should_prompt_role_routing(false, &recs),
            "already-configured maps must not nag"
        );
    }

    #[test]
    fn prompt_is_false_when_considered_is_empty() {
        assert!(!should_prompt_role_routing(true, &[]));
    }
}
