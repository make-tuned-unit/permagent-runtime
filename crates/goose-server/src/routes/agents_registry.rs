use axum::{routing::get, Json, Router};
use serde::Serialize;
use utoipa::ToSchema;

/// Agent entry from the Spectral registry.
/// Phase 1: returns an empty array until the setup wizard creates agents.
#[derive(Serialize, ToSchema)]
pub struct AgentRegistryEntry {
    pub id: String,
    pub name: String,
    pub role: String,
    pub task: String,
    pub status: String,
    pub sprite_asset: Option<String>,
    pub home_x: Option<i32>,
    pub home_y: Option<i32>,
    pub color: Option<String>,
    pub accent: Option<String>,
    pub hair: Option<String>,
    pub visor: Option<String>,
    pub outfit: Option<String>,
    pub capabilities: Vec<String>,
    pub model: Option<String>,
    pub endpoint: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/agents",
    responses(
        (status = 200, description = "List of registered agents", body = Vec<AgentRegistryEntry>),
    )
)]
pub async fn get_agents() -> Json<Vec<AgentRegistryEntry>> {
    // Phase 1: empty registry — agents will be populated by the setup wizard
    Json(vec![])
}

pub fn routes() -> Router {
    Router::new().route("/api/agents", get(get_agents))
}
