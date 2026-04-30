use crate::state::AppState;
use axum::{extract::State, routing::{get, put}, Json, Router};
use permagent::config::agent_identity::{self, PrimaryPersona};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize)]
struct IdentityResponse {
    first_name: String,
    last_name: Option<String>,
    nickname: Option<String>,
    display_name: String,
    traits: Vec<String>,
    tone: String,
    opening_greeting: String,
    voice_id: Option<String>,
}

impl From<&PrimaryPersona> for IdentityResponse {
    fn from(p: &PrimaryPersona) -> Self {
        Self {
            first_name: p.first_name.clone(),
            last_name: p.last_name.clone(),
            nickname: p.nickname.clone(),
            display_name: p.display_name(),
            traits: p.traits.clone(),
            tone: p.tone.clone(),
            opening_greeting: p.opening_greeting.clone(),
            voice_id: p.voice_id.clone(),
        }
    }
}

#[derive(Deserialize)]
struct IdentityUpdate {
    first_name: String,
    last_name: Option<String>,
    nickname: Option<String>,
    traits: Vec<String>,
    tone: String,
    opening_greeting: String,
    voice_id: Option<String>,
}

async fn get_identity(
    State(state): State<Arc<AppState>>,
) -> Json<IdentityResponse> {
    let persona = state.persona.read().await;
    Json(IdentityResponse::from(&*persona))
}

async fn put_identity(
    State(state): State<Arc<AppState>>,
    Json(update): Json<IdentityUpdate>,
) -> Result<Json<IdentityResponse>, axum::http::StatusCode> {
    let new_persona = PrimaryPersona {
        first_name: update.first_name,
        last_name: update.last_name,
        nickname: update.nickname,
        traits: update.traits,
        tone: update.tone,
        opening_greeting: update.opening_greeting,
        voice_id: update.voice_id,
    };

    // Persist to disk (preserve workers from agent_config)
    let workers = {
        let ac = state.agent_config.read().await;
        ac.workers.clone()
    };
    let config = agent_identity::AgentConfig {
        primary: new_persona.clone(),
        workers,
    };
    agent_identity::save_agent_config(&config)
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;

    // Hot-reload via RwLock
    let response = IdentityResponse::from(&new_persona);
    {
        let mut guard = state.persona.write().await;
        *guard = new_persona.clone();
    }
    {
        let mut ac = state.agent_config.write().await;
        ac.primary = new_persona;
    }

    tracing::info!(
        target: "permagentd::agent",
        "Agent identity updated: {}",
        response.display_name
    );

    Ok(Json(response))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/agent/identity", get(get_identity))
        .route("/api/agent/identity", put(put_identity))
        .with_state(state)
}
