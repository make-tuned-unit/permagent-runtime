use crate::routes::errors::ErrorResponse;
use crate::state::AppState;
use axum::{
    extract::State,
    routing::{get, put},
    Json, Router,
};
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

async fn get_identity(State(state): State<Arc<AppState>>) -> Json<IdentityResponse> {
    let persona = state.persona.read().await;
    Json(IdentityResponse::from(&*persona))
}

/// Longest agent name the identity route accepts. Generous for any real name,
/// short enough that the chat header and world nameplate can always show it.
const MAX_NAME_CHARS: usize = 40;

/// The agent's name is what every surface prints; an empty or whitespace-only
/// one would leave the chat header, the nameplate and the greeting blank. The
/// UI already substitutes a default for a blank field — this is the server
/// refusing to store a blank one from any client.
fn validated_first_name(raw: &str) -> Result<String, ErrorResponse> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(ErrorResponse::bad_request("The agent needs a name."));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(ErrorResponse::bad_request(format!(
            "Keep the agent's name under {MAX_NAME_CHARS} characters."
        )));
    }
    Ok(name.to_string())
}

async fn put_identity(
    State(state): State<Arc<AppState>>,
    Json(update): Json<IdentityUpdate>,
) -> Result<Json<IdentityResponse>, ErrorResponse> {
    let first_name = validated_first_name(&update.first_name)?;
    let new_persona = PrimaryPersona {
        first_name,
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
    agent_identity::save_agent_config(&config).map_err(|err| {
        ErrorResponse::internal(format!("Couldn't save the agent's identity: {err}"))
    })?;

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

    // #629 multi-client liveness: persona/voice edits push to every open client
    // (chat header, world nameplate, settings) — they re-read /api/agent/identity.
    permagent::events::emit(permagent::events::identity_changed(&response.display_name));

    Ok(Json(response))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/agent/identity", get(get_identity))
        .route("/api/agent/identity", put(put_identity))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serial_test::serial;
    use tower::ServiceExt;

    async fn body_json(response: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn first_name_is_trimmed_and_required() {
        assert_eq!(validated_first_name("  Nova ").unwrap(), "Nova");
        let blank = validated_first_name("   ").expect_err("blank name refused");
        assert_eq!(blank.status, StatusCode::BAD_REQUEST);
        assert!(blank.message.contains("name"));
        let long = "x".repeat(MAX_NAME_CHARS + 1);
        assert_eq!(
            validated_first_name(&long).unwrap_err().status,
            StatusCode::BAD_REQUEST
        );
    }

    /// An empty name is refused at the route, before anything is written.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn put_identity_rejects_an_empty_name() {
        let root = crate::test_support::test_root();
        let home = tempfile::tempdir().unwrap();
        let _guard = env_lock::lock_env([
            ("HOME", Some(home.path().to_str().unwrap())),
            ("PERMAGENT_PATH_ROOT", Some(root.to_str().unwrap())),
        ]);
        let state = AppState::new(true).await.unwrap();
        let app = routes(state);
        let put = Request::builder()
            .uri("/api/agent/identity")
            .method("PUT")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"first_name":"  ","last_name":null,"nickname":null,
                    "traits":[],"tone":"","opening_greeting":"","voice_id":null}"#,
            ))
            .unwrap();
        let response = app.oneshot(put).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = body_json(response).await;
        assert!(body["message"].as_str().unwrap().contains("name"));
    }

    /// #167 pin: PUT persists to agent.yaml AND hot-reloads shared state, the
    /// PUT response is the full fresh persona (the UI adopts it directly), and
    /// a subsequent GET — same daemon or a fresh boot over the same disk —
    /// returns the saved values.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn put_identity_persists_and_get_round_trips() {
        // HOME stays a throwaway so agent.yaml (keyed off HOME) is isolated and
        // survives the in-test "daemon restart"; only PERMAGENT_PATH_ROOT (the
        // global session-pool DB root) resolves to the shared, process-lifetime
        // root so it never outlives a per-test tempdir (#858).
        let root = crate::test_support::test_root();
        let home = tempfile::tempdir().unwrap();
        let _guard = env_lock::lock_env([
            ("HOME", Some(home.path().to_str().unwrap())),
            ("PERMAGENT_PATH_ROOT", Some(root.to_str().unwrap())),
        ]);

        let state = AppState::new(true).await.unwrap();
        let app = routes(state);

        let put = Request::builder()
            .uri("/api/agent/identity")
            .method("PUT")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"first_name":"Henry","last_name":null,"nickname":null,
                    "traits":["direct"],"tone":"warm",
                    "opening_greeting":"Hey boss!","voice_id":"af_heart"}"#,
            ))
            .unwrap();
        let response = app.clone().oneshot(put).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let saved = body_json(response).await;
        // The PUT response is the FULL persona — the UI sets state from it
        // instead of a dependent re-GET.
        assert_eq!(saved["first_name"], "Henry");
        assert_eq!(saved["display_name"], "Henry");
        assert_eq!(saved["opening_greeting"], "Hey boss!");
        assert_eq!(saved["voice_id"], "af_heart");

        // GET on the live daemon reflects the hot-reloaded persona.
        let get_req = Request::builder()
            .uri("/api/agent/identity")
            .method("GET")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(get_req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let loaded = body_json(response).await;
        assert_eq!(loaded["first_name"], "Henry");
        assert_eq!(loaded["opening_greeting"], "Hey boss!");

        // And the persona survives a daemon restart: a FRESH AppState over the
        // same disk loads the saved values (save-not-persisting is the bug).
        let state2 = AppState::new(true).await.unwrap();
        let app2 = routes(state2);
        let get_req2 = Request::builder()
            .uri("/api/agent/identity")
            .method("GET")
            .body(Body::empty())
            .unwrap();
        let response = app2.oneshot(get_req2).await.unwrap();
        let rebooted = body_json(response).await;
        assert_eq!(rebooted["first_name"], "Henry");
        assert_eq!(rebooted["opening_greeting"], "Hey boss!");
        assert_eq!(rebooted["traits"], serde_json::json!(["direct"]));
    }
}
