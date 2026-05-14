use crate::state::AppState;
use axum::{extract::State, routing::get, Json, Router};
use permagent::agents::platform_extensions::{AGENT_EXTENSIONS, PLATFORM_EXTENSIONS};
use serde::Serialize;
use std::sync::Arc;

#[derive(Serialize)]
struct AgentInfo {
    id: String,
    name: String,
    role: String,
    source: String,
}

#[derive(Serialize)]
struct AgentsResponse {
    agents: Vec<AgentInfo>,
}

async fn list_agents(State(state): State<Arc<AppState>>) -> Json<AgentsResponse> {
    let mut agents = Vec::new();

    // Primary persona from agent.yaml
    let persona = state.persona.read().await;
    agents.push(AgentInfo {
        id: "henry".to_string(),
        name: persona.first_name.clone(),
        role: "Primary agent".to_string(),
        source: "persona".to_string(),
    });
    drop(persona);

    // Agent-type platform extensions
    for &ext_name in AGENT_EXTENSIONS {
        if let Some(ext) = PLATFORM_EXTENSIONS.get(ext_name) {
            agents.push(AgentInfo {
                id: ext_name.to_string(),
                name: ext.display_name.to_string(),
                role: ext.description.to_string(),
                source: "extension".to_string(),
            });
        }
    }

    Json(AgentsResponse { agents })
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/agents", get(list_agents))
        .with_state(state)
}
