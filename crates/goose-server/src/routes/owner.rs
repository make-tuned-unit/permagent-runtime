//! Owner identity — who is running this Permagent instance.
//!
//! Sibling to `routes::identity` (`/api/agent/identity`), which is the
//! AGENT's own name/voice/tone. This is the OWNER: the human on the other
//! side of the conversation. Before this route existed there was no such
//! concept anywhere in the codebase — `people::Person::is_self` was hardcoded
//! `false`, and People → List showed the operator's own directory record
//! twice, flagged as a possible duplicate of itself (dagK owner-identity,
//! 2026-09-26).
//!
//! Storage is `permagent::config::agent_identity::OwnerIdentity`, persisted at
//! `~/.permagent/owner.yaml` (see that module for why it is a separate file
//! from `agent.yaml` rather than a new field on `AgentConfig`). There is no
//! in-memory cache here — the file is a few dozen bytes and GET reads it fresh
//! every time, the same tradeoff `load_agent_config` already makes.

use crate::state::AppState;
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, put},
    Json, Router,
};
use permagent::config::agent_identity::{self, OwnerIdentity};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Serialize, PartialEq, Eq)]
struct OwnerResponse {
    display_name: Option<String>,
    email: Option<String>,
    linked_person_id: Option<String>,
    /// The linked person's CURRENT display name, resolved server-side so the
    /// UI never has to reconcile a second fetch — and so a rename in People
    /// is reflected here without the owner record itself going stale. `None`
    /// when unlinked, or when the linked id no longer resolves to a person
    /// (deleted/merged away) — the UI treats that the same as unlinked rather
    /// than erroring, since the id is still harmlessly stored.
    linked_person_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OwnerUpdate {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    linked_person_id: Option<String>,
}

/// Same shape the People routes use for a profile-link field — local@domain,
/// no leading/trailing dot, at least one dot in the domain, no whitespace.
/// Duplicated rather than imported: `routes::people::validate_person_field`
/// is private to that module and folds email validation into a larger
/// per-field dispatch this route has no use for.
fn valid_email(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && domain.contains('.')
        && !value.chars().any(char::is_whitespace)
}

/// Trim, then fold an empty string to `None` — every `OwnerIdentity` field is
/// "unset", never "set to blank".
fn normalize(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

async fn resolve_linked_name(state: &AppState, owner: &OwnerIdentity) -> Option<String> {
    let id = owner.linked_person_id.as_deref()?;
    let pool = state.session_manager().pool_clone().await.ok()?;
    permagent::people::get_by_uuid(&pool, id)
        .await
        .ok()
        .flatten()
        .map(|p| p.display_name)
}

async fn get_owner(State(state): State<Arc<AppState>>) -> Json<OwnerResponse> {
    let owner = agent_identity::load_owner_identity();
    let linked_person_name = resolve_linked_name(&state, &owner).await;
    Json(OwnerResponse {
        display_name: owner.display_name,
        email: owner.email,
        linked_person_id: owner.linked_person_id,
        linked_person_name,
    })
}

async fn put_owner(
    State(state): State<Arc<AppState>>,
    Json(update): Json<OwnerUpdate>,
) -> Result<Json<OwnerResponse>, (StatusCode, String)> {
    let display_name = normalize(update.display_name);
    let email = normalize(update.email);
    if let Some(e) = &email {
        if !valid_email(e) {
            return Err((StatusCode::BAD_REQUEST, "Invalid email address".to_string()));
        }
    }

    let linked_person_id = normalize(update.linked_person_id);
    if let Some(id) = &linked_person_id {
        let pool = state
            .session_manager()
            .pool_clone()
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let exists = permagent::people::get_by_uuid(&pool, id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
            .is_some();
        if !exists {
            return Err((
                StatusCode::BAD_REQUEST,
                "linked_person_id does not match any person in the directory".to_string(),
            ));
        }
    }

    let new_owner = OwnerIdentity {
        display_name,
        email,
        linked_person_id,
    };
    agent_identity::save_owner_identity(&new_owner)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Feed the brain (best-effort, never blocks the response): the agent
    // should know who its user is. Only when there is a name to say —
    // "the owner's name was cleared" is not a fact worth remembering.
    if let Some(name) = new_owner.display_name.as_deref() {
        if let Ok(pool) = state.session_manager().pool_clone().await {
            let content = match new_owner.email.as_deref() {
                Some(email) => format!(
                    "The person operating this Permagent instance is named {name} ({email})."
                ),
                None => format!("The person operating this Permagent instance is named {name}."),
            };
            permagent::brain_feed::feed(
                &pool,
                permagent::brain_feed::FeedSource::Owner,
                permagent::brain_feed::feed_key("owner", "self", "identity"),
                None,
                content,
            )
            .await;
        }
    }

    // Multi-client liveness, two audiences:
    //  - People views: a link/unlink changes which row renders as "You", so
    //    reuse the existing `person_changed` → `bumpPeople()` wiring.
    //  - Settings → Preferences on another open client: `owner.yaml` is not on
    //    the generic `Config` key/value store, so its own writer must announce
    //    itself the same way `permission.yaml`'s does (config/permission.rs) —
    //    `config_changed` is the one "a settings surface should refetch" signal,
    //    not only ever a literal Config key.
    permagent::events::emit(permagent::events::person_changed(
        "",
        new_owner.linked_person_id.as_deref().unwrap_or(""),
        "owner_linked",
    ));
    permagent::events::emit(permagent::events::config_changed(
        &["owner"],
        "updated",
        false,
    ));

    let linked_person_name = resolve_linked_name(&state, &new_owner).await;
    Ok(Json(OwnerResponse {
        display_name: new_owner.display_name,
        email: new_owner.email,
        linked_person_id: new_owner.linked_person_id,
        linked_person_name,
    }))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/owner", get(get_owner).put(put_owner))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use serial_test::serial;
    use tower::ServiceExt;

    async fn body_json(response: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// A throwaway `HOME` for tests that build an `AppState` — some subsystems
    /// (not `owner_yaml_path`/`agent_yaml_path`, which resolve under
    /// `PERMAGENT_PATH_ROOT` directly, but others reached during `AppState::new`)
    /// read `HOME` via `dirs::home_dir()` rather than `PERMAGENT_PATH_ROOT`, so
    /// every AppState-building test still points it somewhere disposable.
    fn test_home() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn valid_email_accepts_ordinary_addresses_and_rejects_malformed_ones() {
        assert!(valid_email("jane@example.com"));
        assert!(!valid_email("not-an-email"));
        assert!(!valid_email("@example.com"));
        assert!(!valid_email("jane@"));
        assert!(!valid_email("jane@.com"));
        assert!(!valid_email("jane@example."));
        assert!(!valid_email("jane @example.com"));
    }

    #[test]
    fn normalize_folds_blank_and_whitespace_only_to_none() {
        assert_eq!(normalize(None), None);
        assert_eq!(normalize(Some("".to_string())), None);
        assert_eq!(normalize(Some("   ".to_string())), None);
        assert_eq!(
            normalize(Some("  Jane Doe  ".to_string())),
            Some("Jane Doe".to_string())
        );
    }

    /// #167-class pin, for the owner record: PUT persists to owner.yaml AND is
    /// readable back on a fresh `AppState` (a daemon restart), and the PUT
    /// response is the full fresh record so the UI can adopt it directly.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn put_owner_persists_and_get_round_trips_across_a_restart() {
        let root = crate::test_support::test_root();
        let home = test_home();
        let _guard = env_lock::lock_env([
            ("HOME", Some(home.path().to_str().unwrap())),
            ("PERMAGENT_PATH_ROOT", Some(root.to_str().unwrap())),
        ]);

        let state = AppState::new(true).await.unwrap();
        let app = routes(state);

        let put = Request::builder()
            .uri("/api/owner")
            .method("PUT")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"display_name":"Jane Doe","email":"jane@example.com"}"#,
            ))
            .unwrap();
        let response = app.clone().oneshot(put).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let saved = body_json(response).await;
        assert_eq!(saved["display_name"], "Jane Doe");
        assert_eq!(saved["email"], "jane@example.com");
        assert_eq!(saved["linked_person_id"], serde_json::Value::Null);

        let get_req = Request::builder()
            .uri("/api/owner")
            .method("GET")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(get_req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let loaded = body_json(response).await;
        assert_eq!(loaded["display_name"], "Jane Doe");

        // Survives a fresh AppState over the same disk (the daemon-restart case).
        let state2 = AppState::new(true).await.unwrap();
        let app2 = routes(state2);
        let get_req2 = Request::builder()
            .uri("/api/owner")
            .method("GET")
            .body(Body::empty())
            .unwrap();
        let response = app2.oneshot(get_req2).await.unwrap();
        let rebooted = body_json(response).await;
        assert_eq!(rebooted["display_name"], "Jane Doe");
        assert_eq!(rebooted["email"], "jane@example.com");
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn get_owner_defaults_to_every_field_null_when_never_configured() {
        let root = crate::test_support::test_root();
        let home = test_home();
        let _guard = env_lock::lock_env([
            ("HOME", Some(home.path().to_str().unwrap())),
            ("PERMAGENT_PATH_ROOT", Some(root.to_str().unwrap())),
        ]);
        // `PERMAGENT_PATH_ROOT` is the process-lifetime shared root every
        // AppState-building test in this binary resolves `owner.yaml` under
        // (see `test_support::test_root`'s doc comment) — a sibling test in
        // this same module may have already written one. Force the clean
        // "never configured" starting point this test is actually about,
        // rather than depending on test execution order.
        let _ = std::fs::remove_file(agent_identity::owner_yaml_path());

        let state = AppState::new(true).await.unwrap();
        let app = routes(state);
        let get_req = Request::builder()
            .uri("/api/owner")
            .method("GET")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(get_req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let loaded = body_json(response).await;
        assert_eq!(loaded["display_name"], serde_json::Value::Null);
        assert_eq!(loaded["email"], serde_json::Value::Null);
        assert_eq!(loaded["linked_person_id"], serde_json::Value::Null);
        assert_eq!(loaded["linked_person_name"], serde_json::Value::Null);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn put_owner_rejects_invalid_email() {
        let root = crate::test_support::test_root();
        let home = test_home();
        let _guard = env_lock::lock_env([
            ("HOME", Some(home.path().to_str().unwrap())),
            ("PERMAGENT_PATH_ROOT", Some(root.to_str().unwrap())),
        ]);

        let state = AppState::new(true).await.unwrap();
        let app = routes(state);
        let put = Request::builder()
            .uri("/api/owner")
            .method("PUT")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"email":"not-an-email"}"#))
            .unwrap();
        let response = app.oneshot(put).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn put_owner_rejects_a_linked_person_id_that_does_not_exist() {
        let root = crate::test_support::test_root();
        let home = test_home();
        let _guard = env_lock::lock_env([
            ("HOME", Some(home.path().to_str().unwrap())),
            ("PERMAGENT_PATH_ROOT", Some(root.to_str().unwrap())),
        ]);

        let state = AppState::new(true).await.unwrap();
        let app = routes(state);
        let put = Request::builder()
            .uri("/api/owner")
            .method("PUT")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"linked_person_id":"00000000-0000-0000-0000-000000000000"}"#,
            ))
            .unwrap();
        let response = app.oneshot(put).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// PUT with every field blank/whitespace-only clears the record —
    /// `normalize` folding blank to `None` must reach all the way to disk.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn put_owner_with_blank_fields_clears_the_record() {
        let root = crate::test_support::test_root();
        let home = test_home();
        let _guard = env_lock::lock_env([
            ("HOME", Some(home.path().to_str().unwrap())),
            ("PERMAGENT_PATH_ROOT", Some(root.to_str().unwrap())),
        ]);

        let state = AppState::new(true).await.unwrap();
        let app = routes(state);
        let put1 = Request::builder()
            .uri("/api/owner")
            .method("PUT")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"display_name":"Jane Doe"}"#))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(put1).await.unwrap().status(),
            StatusCode::OK
        );

        let put2 = Request::builder()
            .uri("/api/owner")
            .method("PUT")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"display_name":"   "}"#))
            .unwrap();
        let response = app.oneshot(put2).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let saved = body_json(response).await;
        assert_eq!(saved["display_name"], serde_json::Value::Null);
    }
}
