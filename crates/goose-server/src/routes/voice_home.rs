//! Round 3 (Jesse 2026-10-14): this is the suite-wide CURRENT chat session.
//! Users can still start new chats, but a new or selected chat applies to every
//! chat surface (desktop dock, Orb, World voice, phone, watch).
//!   GET  /api/chat/current-session        (alias: /api/voice/home-session)
//!   POST /api/chat/current-session        {session_id, origin_client?}  select a plain chat
//!   POST /api/chat/current-session/new    {origin_client?}              fresh chat, made current
//! Every real change emits `current_chat_session_changed {session_id, name,
//! origin_client}` on /events. Coding/harness session creation never touches it.
//!
//!
//! The "voice home" session: ONE persistent, non-coding conversation with the
//! agent that the World, the Orb, the iPhone and the Watch all speak into
//! (Jesse decision D1, voice audit V-10).
//!
//! Before this, voice reused whatever chat session was sticky in the dock, so a
//! spoken question could land in an Auto-mode coding-coordinator session and
//! trip its guards. `GET /api/voice/home-session` returns a stable session id,
//! created on first use as a plain chat (`SessionType::User`, no recipe, no
//! parent, no project hint, working dir = the profile data dir). The id is
//! persisted under the profile so it survives restarts. A stored id is only
//! honoured while the session still exists and is still a plain chat; if it was
//! deleted or acquired a recipe, a fresh one is created and recorded.

use crate::state::AppState;
use axum::{
    extract::{Path as AxPath, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use permagent::session::session_manager::{Session, SessionManager, SessionType};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

const STATE_FILE: &str = "voice_home_session.json";

/// Serialises first-use creation so two clients opening at once share one id.
static CREATE_LOCK: Mutex<()> = Mutex::const_new(());

#[derive(Serialize, Deserialize)]
struct Stored {
    session_id: String,
}

#[derive(Serialize)]
struct HomeSessionResponse {
    session_id: String,
    name: String,
    created: bool,
}

fn state_path() -> PathBuf {
    permagent::config::paths::Paths::data_dir().join(STATE_FILE)
}

fn read_stored(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<Stored>(&text)
        .ok()
        .map(|s| s.session_id)
        .filter(|s| !s.trim().is_empty())
}

fn write_stored(path: &Path, session_id: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let body = serde_json::to_string(&Stored {
        session_id: session_id.to_string(),
    })
    .map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)
}

/// A session may serve as the voice home only if it is a plain chat: a User
/// session with no recipe (the coding harness is a recipe), no parent (not a
/// dispatched worker) and no schedule.
pub(crate) fn is_plain_chat(session: &Session) -> bool {
    session.session_type == SessionType::User
        && session.recipe.is_none()
        && session.parent_session_id.is_none()
        && session.schedule_id.is_none()
}

/// The stored current session, if it still exists and is still a plain chat.
async fn read_current(manager: &SessionManager, path: &Path) -> Option<Session> {
    let id = read_stored(path)?;
    let session = manager.get_session(&id, false).await.ok()?;
    is_plain_chat(&session).then_some(session)
}

/// Create a fresh plain chat and record it as current. Nothing here is a coding
/// or project session: no recipe, no parent, no project hint.
async fn create_current(
    manager: &SessionManager,
    path: &Path,
) -> anyhow::Result<HomeSessionResponse> {
    // System-generated title: the chat is named from its first exchange like
    // any other, since it is now the suite's ordinary current chat.
    let name = "New Chat".to_string();
    let mode = permagent::config::Config::global()
        .get_goose_mode()
        .unwrap_or_default();
    let session = manager
        .create_session(
            permagent::config::paths::Paths::data_dir(),
            name.clone(),
            SessionType::User,
            mode,
        )
        .await?;
    write_stored(path, &session.id)?;
    Ok(HomeSessionResponse {
        session_id: session.id,
        name,
        created: true,
    })
}

async fn resolve(manager: &SessionManager, path: &Path) -> anyhow::Result<HomeSessionResponse> {
    let _guard = CREATE_LOCK.lock().await;
    if let Some(session) = read_current(manager, path).await {
        return Ok(HomeSessionResponse {
            session_id: session.id,
            name: session.name,
            created: false,
        });
    }
    create_current(manager, path).await
}

#[derive(Debug, PartialEq, Eq)]
enum SetError {
    NotFound,
    NotPlainChat,
    Storage,
}

/// Make an existing plain chat current. `changed` is false when it already was.
async fn set_current(
    manager: &SessionManager,
    path: &Path,
    session_id: &str,
) -> Result<(Session, bool), SetError> {
    let _guard = CREATE_LOCK.lock().await;
    let session = manager
        .get_session(session_id, false)
        .await
        .map_err(|_| SetError::NotFound)?;
    if !is_plain_chat(&session) {
        return Err(SetError::NotPlainChat);
    }
    let changed = read_stored(path).as_deref() != Some(session.id.as_str());
    if changed {
        write_stored(path, &session.id).map_err(|_| SetError::Storage)?;
    }
    Ok((session, changed))
}

async fn home_session(
    State(state): State<Arc<AppState>>,
) -> Result<Json<HomeSessionResponse>, StatusCode> {
    resolve(&state.session_manager(), &state_path())
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("current chat session unavailable: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

fn announce(session_id: &str, name: &str, origin_client: Option<&str>) {
    permagent::events::emit(permagent::events::current_chat_session_changed(
        session_id,
        name,
        origin_client
            .filter(|o| !o.trim().is_empty())
            .unwrap_or("unknown"),
    ));
}

#[derive(Deserialize)]
struct SetCurrentBody {
    session_id: String,
    #[serde(default)]
    origin_client: Option<String>,
}

#[derive(Deserialize, Default)]
struct NewCurrentBody {
    #[serde(default)]
    origin_client: Option<String>,
}

/// POST /api/chat/current-session: make an existing plain chat current.
/// 404 unknown id; 422 for a coding/recipe/sub-agent/scheduled session.
async fn set_current_session(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SetCurrentBody>,
) -> Result<Json<HomeSessionResponse>, StatusCode> {
    let (session, changed) = set_current(&state.session_manager(), &state_path(), &body.session_id)
        .await
        .map_err(|e| match e {
            SetError::NotFound => StatusCode::NOT_FOUND,
            SetError::NotPlainChat => StatusCode::UNPROCESSABLE_ENTITY,
            SetError::Storage => StatusCode::INTERNAL_SERVER_ERROR,
        })?;
    if changed {
        announce(&session.id, &session.name, body.origin_client.as_deref());
    }
    Ok(Json(HomeSessionResponse {
        session_id: session.id,
        name: session.name,
        created: false,
    }))
}

/// POST /api/chat/current-session/new: start a fresh plain chat and make it
/// current everywhere.
async fn new_current_session(
    State(state): State<Arc<AppState>>,
    body: Option<Json<NewCurrentBody>>,
) -> Result<Json<HomeSessionResponse>, StatusCode> {
    let origin = body.and_then(|Json(b)| b.origin_client);
    let created = {
        let _guard = CREATE_LOCK.lock().await;
        create_current(&state.session_manager(), &state_path()).await
    }
    .map_err(|e| {
        tracing::error!("new current chat session failed: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    announce(&created.session_id, &created.name, origin.as_deref());
    Ok(Json(created))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Decision {
    Approve,
    Deny,
}

#[derive(Deserialize)]
struct ConfirmBody {
    decision: Decision,
    /// The session the confirm card belongs to (it is in the
    /// `voice_confirm_requested` payload). The gate refuses a foreign session.
    session_id: String,
}

#[derive(Serialize)]
struct ConfirmResponse {
    confirm_id: String,
    outcome: &'static str,
}

/// Apply the user's tap to the L2 gate. `None` = unknown, expired or foreign id.
fn apply_decision(decision: &Decision, session_id: &str, confirm_id: &str) -> Option<&'static str> {
    use permagent::events::voice_origin::{approve_confirmation, deny_confirmation};
    match decision {
        Decision::Approve => approve_confirmation(session_id, confirm_id).then_some("approved"),
        Decision::Deny => deny_confirmation(session_id, confirm_id).then_some("denied"),
    }
}

/// POST /api/voice/confirm/{confirm_id}: the on-screen Confirm / Dismiss tap.
async fn resolve_confirm(
    AxPath(confirm_id): AxPath<String>,
    Json(body): Json<ConfirmBody>,
) -> Result<Json<ConfirmResponse>, StatusCode> {
    let outcome = apply_decision(&body.decision, &body.session_id, &confirm_id)
        .ok_or(StatusCode::NOT_FOUND)?;
    permagent::events::emit(permagent::events::voice_confirm_resolved(
        &body.session_id,
        &confirm_id,
        outcome,
    ));
    Ok(Json(ConfirmResponse {
        confirm_id,
        outcome,
    }))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/voice/home-session", get(home_session))
        .route(
            "/api/chat/current-session",
            get(home_session).post(set_current_session),
        )
        .route("/api/chat/current-session/new", post(new_current_session))
        .route("/api/voice/confirm/{confirm_id}", post(resolve_confirm))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn stored_id_round_trips_and_rejects_blank() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(STATE_FILE);
        assert_eq!(read_stored(&path), None);
        write_stored(&path, "abc").unwrap();
        assert_eq!(read_stored(&path).as_deref(), Some("abc"));
        write_stored(&path, "  ").unwrap();
        assert_eq!(read_stored(&path), None);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn creates_once_reuses_and_replaces_a_deleted_or_coding_session() {
        let root = crate::test_support::test_root();
        let home = tempfile::tempdir().unwrap();
        let _guard = env_lock::lock_env([
            ("HOME", Some(home.path().to_str().unwrap())),
            ("PERMAGENT_PATH_ROOT", Some(root.to_str().unwrap())),
        ]);
        let state = AppState::new(true).await.unwrap();
        let manager = state.session_manager();
        let file = home.path().join(STATE_FILE);

        let first = resolve(manager, &file).await.unwrap();
        assert!(first.created);
        let session = manager.get_session(&first.session_id, false).await.unwrap();
        assert!(is_plain_chat(&session));
        assert!(session.recipe.is_none());

        // Stable across calls (and, via the file, across restarts).
        let again = resolve(manager, &file).await.unwrap();
        assert!(!again.created);
        assert_eq!(again.session_id, first.session_id);
        assert_eq!(
            read_stored(&file).as_deref(),
            Some(first.session_id.as_str())
        );

        // Deleted: a new plain chat replaces it.
        manager.delete_session(&first.session_id).await.unwrap();
        let replaced = resolve(manager, &file).await.unwrap();
        assert!(replaced.created);
        // Session ids are per-day counters, so the replacement may reuse the
        // deleted id; what matters is that a live plain chat now stands behind it.
        let live = manager
            .get_session(&replaced.session_id, false)
            .await
            .unwrap();
        assert!(is_plain_chat(&live));

        // A stored id that is NOT a plain chat is never returned.
        let other = manager
            .create_session(
                home.path().to_path_buf(),
                "Sub".into(),
                SessionType::SubAgent,
                Default::default(),
            )
            .await
            .unwrap();
        write_stored(&file, &other.id).unwrap();
        let fixed = resolve(manager, &file).await.unwrap();
        assert_ne!(fixed.session_id, other.id);
        assert!(fixed.created);
    }

    #[test]
    fn confirm_decisions_hit_the_gate_and_unknown_ids_are_none() {
        use permagent::events::voice_origin::{begin, end, gate_tool_call, VoiceGate, VoiceOrigin};
        let sid = "voice-home-confirm-test";
        begin(sid, VoiceOrigin::resolve(Some("ios_voice"), None));
        let args = serde_json::json!({"command": "rm -rf x"});
        let req = match gate_tool_call(sid, "shell", &args) {
            VoiceGate::NeedsConfirmation(r) => r,
            other => panic!("shell must need confirmation, got {other:?}"),
        };
        // Foreign session and unknown id are refused.
        assert_eq!(
            apply_decision(&Decision::Approve, "other", &req.confirm_id),
            None
        );
        assert_eq!(apply_decision(&Decision::Approve, sid, "nope"), None);
        assert_eq!(
            apply_decision(&Decision::Approve, sid, &req.confirm_id),
            Some("approved")
        );
        // Deny removes it; a second deny is then unknown.
        let req2 = match gate_tool_call(sid, "shell", &serde_json::json!({"command": "ls"})) {
            VoiceGate::NeedsConfirmation(r) => r,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            apply_decision(&Decision::Deny, sid, &req2.confirm_id),
            Some("denied")
        );
        assert_eq!(apply_decision(&Decision::Deny, sid, &req2.confirm_id), None);
        end(sid);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn confirm_route_404s_for_unknown_ids_and_rejects_bad_decisions() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;
        let root = crate::test_support::test_root();
        let home = tempfile::tempdir().unwrap();
        let _guard = env_lock::lock_env([
            ("HOME", Some(home.path().to_str().unwrap())),
            ("PERMAGENT_PATH_ROOT", Some(root.to_str().unwrap())),
        ]);
        let app = routes(AppState::new(true).await.unwrap());
        let post = |body: &'static str| {
            Request::builder()
                .uri("/api/voice/confirm/missing")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap()
        };
        let r = app
            .clone()
            .oneshot(post(r#"{"decision":"approve","session_id":"s"}"#))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        let r = app
            .oneshot(post(r#"{"decision":"maybe","session_id":"s"}"#))
            .await
            .unwrap();
        assert!(r.status().is_client_error() && r.status() != StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn set_current_accepts_plain_chats_and_rejects_the_rest() {
        let root = crate::test_support::test_root();
        let home = tempfile::tempdir().unwrap();
        let _guard = env_lock::lock_env([
            ("HOME", Some(home.path().to_str().unwrap())),
            ("PERMAGENT_PATH_ROOT", Some(root.to_str().unwrap())),
        ]);
        let state = AppState::new(true).await.unwrap();
        let manager = state.session_manager();
        let file = home.path().join(STATE_FILE);

        let mk = |name: &'static str, t: SessionType| {
            manager.create_session(
                home.path().to_path_buf(),
                name.into(),
                t,
                Default::default(),
            )
        };
        let a = mk("A", SessionType::User).await.unwrap();
        let b = mk("B", SessionType::User).await.unwrap();

        let (s, changed) = set_current(manager, &file, &a.id).await.unwrap();
        assert!(changed && s.id == a.id);
        assert_eq!(read_stored(&file).as_deref(), Some(a.id.as_str()));
        // Same again: no change (no event).
        assert!(!set_current(manager, &file, &a.id).await.unwrap().1);
        assert!(set_current(manager, &file, &b.id).await.unwrap().1);
        assert_eq!(read_stored(&file).as_deref(), Some(b.id.as_str()));

        // Rejections leave current untouched.
        for t in [
            SessionType::SubAgent,
            SessionType::Scheduled,
            SessionType::Terminal,
            SessionType::Hidden,
        ] {
            let bad = mk("bad", t).await.unwrap();
            assert_eq!(
                set_current(manager, &file, &bad.id).await.unwrap_err(),
                SetError::NotPlainChat
            );
        }
        assert_eq!(
            set_current(manager, &file, "does-not-exist")
                .await
                .unwrap_err(),
            SetError::NotFound
        );
        assert_eq!(read_stored(&file).as_deref(), Some(b.id.as_str()));
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn new_chat_becomes_current_and_the_routes_are_wired() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;
        let root = crate::test_support::test_root();
        let home = tempfile::tempdir().unwrap();
        let _guard = env_lock::lock_env([
            ("HOME", Some(home.path().to_str().unwrap())),
            ("PERMAGENT_PATH_ROOT", Some(root.to_str().unwrap())),
        ]);
        let state = AppState::new(true).await.unwrap();
        let app = routes(state);
        let call = |method: &str, uri: &str, body: &'static str| {
            Request::builder()
                .uri(uri.to_string())
                .method(method)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap()
        };
        let json = |r: axum::response::Response| async move {
            let bytes = axum::body::to_bytes(r.into_body(), usize::MAX)
                .await
                .unwrap();
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
        };

        let first = json(
            app.clone()
                .oneshot(call("GET", "/api/chat/current-session", ""))
                .await
                .unwrap(),
        )
        .await;
        let alias = json(
            app.clone()
                .oneshot(call("GET", "/api/voice/home-session", ""))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(first["session_id"], alias["session_id"]);

        let r = app
            .clone()
            .oneshot(call(
                "POST",
                "/api/chat/current-session/new",
                r#"{"origin_client":"desktop"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let fresh = json(r).await;
        assert_ne!(fresh["session_id"], first["session_id"]);
        let now = json(
            app.clone()
                .oneshot(call("GET", "/api/chat/current-session", ""))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(now["session_id"], fresh["session_id"]);

        // Select the older one back; unknown -> 404.
        let body: &'static str = Box::leak(
            format!(
                r#"{{"session_id":"{}"}}"#,
                first["session_id"].as_str().unwrap()
            )
            .into_boxed_str(),
        );
        let r = app
            .clone()
            .oneshot(call("POST", "/api/chat/current-session", body))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let r = app
            .oneshot(call(
                "POST",
                "/api/chat/current-session",
                r#"{"session_id":"nope"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
    }
}
