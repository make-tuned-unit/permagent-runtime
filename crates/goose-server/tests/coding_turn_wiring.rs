//! `POST /api/coding-sessions/turn` — the harness's half of the shared Brain.
//!
//! The harness runs in its own process and never mounts a Brain (`GLOBAL_BRAIN`
//! is a per-process singleton the CLI never populates, and a second writer of
//! one Spectral database is a corruption story). So it posts each completed
//! turn to the owner. Before this route it could only post ONE distilled
//! summary, at exit — a coding session was invisible to Chat until the terminal
//! tab closed.
//!
//! Asserted here, through the COMPOSED router (`routes::configure`) so the real
//! bearer middleware is in the path, against a REAL Brain:
//!
//!   1. an anonymous or wrongly-tokened POST is refused — memory is not a
//!      public write surface;
//!   2. an authenticated turn lands as a real memory, through the same
//!      `spawn_persist_chat_turn` a Chat turn takes;
//!   3. `(sessionId, turnIdx)` is the key, so a client retry — which is exactly
//!      what a fire-and-forget writer produces — cannot duplicate a memory;
//!   4. a half-empty turn is refused rather than stored as a hollow memory.
//!
//! The working directory rides along as the wing-decision evidence
//! `spawn_persist_chat_turn` takes in its `tool_text` slot; it shapes which
//! wing the memory lands in rather than appearing in its content, so it is not
//! asserted on the content here.
//!
//! Own integration binary: needs a REAL Brain (brain_dir + ontology.toml at
//! AppState build time — see `project_reindex_scoping.rs` for why the shared
//! lib-test root cannot provide one) and `PERMAGENT_PATH_ROOT` is per-process.
//!
//! The token here is the one `AppState::new` mints under the throwaway test
//! root. Nothing in this file reads the developer's real
//! `~/.permagent/secrets/daemon_token.json`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::time::Duration;
use tower::ServiceExt;

const ONTOLOGY_TOML: &str = include_str!("../../goose/assets/ontology.toml");

fn turn_req(bearer: Option<&str>, body: serde_json::Value) -> Request<Body> {
    let mut b = Request::builder()
        .method("POST")
        .uri("/api/coding-sessions/turn")
        .header("content-type", "application/json");
    if let Some(t) = bearer {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::from(body.to_string())).unwrap()
}

/// The write is detached inside the handler (a turn must never wait on a memory
/// write), so the memory appears shortly after the 200, not before it.
async fn await_memory(
    brain: &permagent::brain_handle::SafeBrain,
    key: &str,
) -> Option<spectral::ingest::Memory> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Some(m)) = brain.get_memory_by_key(key).await {
            return Some(m);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    None
}

#[tokio::test(flavor = "multi_thread")]
async fn a_harness_turn_becomes_a_brain_memory_behind_the_daemon_token() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());
    std::env::set_var("HOME", tmp.path());
    std::fs::create_dir_all(permagent::config::paths::Paths::brain_dir()).unwrap();
    std::fs::write(
        permagent::config::paths::Paths::brain_ontology(),
        ONTOLOGY_TOML,
    )
    .unwrap();

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let token = state
        .daemon_token
        .clone()
        .expect("test AppState should have generated a daemon token");
    let brain = state
        .brain
        .as_ref()
        .expect("AppState must mount the Brain when brain_dir + ontology exist")
        .clone();
    let app = permagent_daemon::routes::configure(state.clone());

    let body = serde_json::json!({
        "sessionId": "harness-abc",
        "turnIdx": 4,
        "userText": "why does the picker scanner die overnight?",
        "assistantText": "It hit the file-descriptor limit; raised it in the launchd plist.",
        "workingDir": "/Users/j/Documents/dev/permagent-runtime",
    });

    // (1) Memory is not a public write surface. An unauthenticated caller could
    // otherwise plant anything the agent will later recall as background.
    for (name, bearer) in [("no token", None), ("wrong token", Some("not-the-token"))] {
        let resp = app
            .clone()
            .oneshot(turn_req(bearer, body.clone()))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "/api/coding-sessions/turn must 401 with {name}"
        );
    }

    // (5) A turn with no answer in it is not a turn. Storing it would leave a
    // memory that says a session happened and nothing about what it did.
    let resp = app
        .clone()
        .oneshot(turn_req(
            Some(&token),
            serde_json::json!({
                "sessionId": "harness-abc",
                "turnIdx": 0,
                "userText": "hello",
                "assistantText": "   ",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "a half-empty turn must be refused, not stored hollow"
    );

    // (2) The authenticated turn is accepted…
    let resp = app
        .clone()
        .oneshot(turn_req(Some(&token), body.clone()))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // …and lands as a real memory under the shared chat-turn key shape, which
    // is what makes a coding turn and a chat turn the same kind of memory to
    // recall.
    let key = "chat-harness-abc-4";
    let memory = await_memory(&brain, key)
        .await
        .expect("an accepted harness turn must reach the Brain");
    assert!(
        memory.content.contains("file-descriptor limit"),
        "the assistant's answer is the half worth remembering: {}",
        memory.content
    );
    assert!(
        memory.content.contains("picker scanner"),
        "the question it answered has to survive too: {}",
        memory.content
    );

    // (4) Idempotency. `persist_turn` is fire-and-forget with no retry ceiling
    // of its own; a duplicate POST must not become a duplicate memory.
    let resp = app
        .clone()
        .oneshot(turn_req(Some(&token), body.clone()))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let again = brain
        .get_memory_by_key(key)
        .await
        .unwrap()
        .expect("the memory is still there after a retry");
    assert_eq!(
        again.id, memory.id,
        "a retried turn must land on its own memory, not mint a second one"
    );
    assert_eq!(
        again.content, memory.content,
        "a retry must not append to what it already wrote"
    );

    // …while a genuinely different turn is a genuinely different memory. Without
    // this the idempotency above would also be satisfied by a key that ignored
    // `turnIdx` entirely — which would collapse a whole session into one row.
    let mut second = body.clone();
    second["turnIdx"] = serde_json::json!(5);
    second["assistantText"] = serde_json::json!("Then I restarted it under launchd.");
    let resp = app
        .clone()
        .oneshot(turn_req(Some(&token), second))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let next = await_memory(&brain, "chat-harness-abc-5")
        .await
        .expect("turn 5 is its own memory");
    assert_ne!(next.id, memory.id);
}
