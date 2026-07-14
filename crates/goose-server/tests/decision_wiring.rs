//! Integration-wiring proofs for the Decision Inbox swarm, through the real
//! HTTP router (`routes::decisions::routes`) against an AppState rooted at a
//! throwaway PERMAGENT_PATH_ROOT.
//!
//! Covers:
//!   wire 5 — daemon startup installs the decision sink AND the post-Review
//!            verification hook (idempotently);
//!   wire 3 — GET /api/decisions reranks open rows before listing;
//!   wire 1 — a `choice` answer on a parked goal makes it re-dispatch
//!            eligible (Triage → Ready, attention cleared, cap extended);
//!   wire 2 — absence seam: with no Brain in the throwaway root the Learn
//!            ingestion call no-ops and never breaks the answer path (the
//!            live Brain path is proven by L3's decision_inbox_trace harness
//!            and learn's unit tests — standing up a real spectral Brain
//!            here would be disproportionate).
//!
//! Runs as its own integration-test binary (own process): the AgentManager
//! singleton, GOAL_REVIEW_HOOK, and the global decision sink are
//! process-wide, so this is the single test in the binary.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::sync::Arc;
use tower::ServiceExt;

use permagent::cards::{self, CreateCard};
use permagent::decisions::{self, NewDecision};
use permagent::projects::PERSONAL_PROJECT_ID;
use serial_test::serial;

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn post_json(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

// #[serial]: this test mutates the process-global PERMAGENT_PATH_ROOT env var
// and shared startup singletons; without serialization it races other AppState
// tests (flaky assertions from cross-test DB/path bleed).
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn integration_wiring_through_router_and_startup() {
    // Throwaway data root for the whole process (single test in this binary).
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let app = permagent_daemon::routes::decisions::routes(state.clone());
    let pool = state.session_manager().pool_clone().await.unwrap();

    // ── Wire 5: startup installed both hooks ──
    assert!(
        permagent::agents::platform_extensions::orchestrator::GOAL_REVIEW_HOOK
            .get()
            .is_some(),
        "AppState::new must install the post-Review verification hook"
    );
    // The sink slot is already occupied by startup's SqlDecisionSink: a
    // second install is rejected (OnceLock semantics), never replaced.
    let rejected = permagent::decision_inbox::escalate::install_decision_sink(Arc::new(
        permagent::decision_inbox::escalate::InMemoryDecisionSink::default(),
    ));
    assert!(
        rejected.is_err(),
        "decision sink must already be installed at startup"
    );
    // Re-running the review-hook installer is an idempotent no-op.
    permagent_daemon::verification::install_review_hook();
    assert!(
        permagent::agents::platform_extensions::orchestrator::GOAL_REVIEW_HOOK
            .get()
            .is_some()
    );

    // ── Wire 2 seam: no Brain in the throwaway root ──
    // The Learn wire in answer_decision_handler is gated on
    // get_global_brain(); with no Brain mounted it must no-op and the answer
    // path must stay fully functional (proven by the 200 + effect below).
    assert!(
        permagent::agents::platform_extensions::get_global_brain().is_none(),
        "throwaway root must not mount a Brain"
    );

    // ── Wire 3: GET /api/decisions reranks open rows before listing ──
    cards::seed_goal_columns(&pool, PERSONAL_PROJECT_ID)
        .await
        .unwrap();
    let triage_col = cards::get_goal_column(&pool, PERSONAL_PROJECT_ID, "triage")
        .await
        .unwrap()
        .unwrap();

    // A critical-priority goal: its unblock decision scores 0.30
    // (0.3 * priority_norm 1.0); the goal-less choice decision scores 0.15
    // (priority Normal). Seed the persisted ranks INVERTED.
    let critical_goal = cards::create_card(
        &pool,
        CreateCard {
            project_id: PERSONAL_PROJECT_ID.to_string(),
            title: "Critical wiring goal".to_string(),
            description: None,
            card_type: Some("goal".to_string()),
            column_id: Some(triage_col.id.clone()),
            created_by: None,
            metadata_json: Some(serde_json::json!({"priority": "critical"})),
        },
    )
    .await
    .unwrap();

    let d_first = decisions::create_decision(
        &pool,
        NewDecision {
            kind: "unblock".to_string(),
            goal_id: Some(critical_goal.id.clone()),
            project_id: Some(PERSONAL_PROJECT_ID.to_string()),
            headline: Some("A critical goal is stuck and needs your direction".to_string()),
            detail: Some("rerank wiring test".to_string()),
            payload: serde_json::json!({"reason": "stuck"}),
            rank: Some(0.01), // seeded WRONG: lowest
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let d_second = decisions::create_decision(
        &pool,
        NewDecision {
            kind: "choice".to_string(),
            goal_id: None,
            project_id: Some(PERSONAL_PROJECT_ID.to_string()),
            headline: Some("Pick an option for the rerank test".to_string()),
            detail: Some("rerank wiring test".to_string()),
            payload: serde_json::json!({
                "question": "Which?",
                "options": [
                    {"id": "a", "label": "A"},
                    {"id": "b", "label": "B"}
                ]
            }),
            rank: Some(9.0), // seeded WRONG: highest
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let resp = app.clone().oneshot(get("/api/decisions")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(
        items[0]["id"],
        serde_json::json!(d_first.id),
        "GET must rerank: critical-goal decision first despite seeded rank 0.01: {}",
        v
    );
    assert_eq!(items[1]["id"], serde_json::json!(d_second.id));
    // The bogus persisted rank was actually overwritten by the curation score.
    let new_rank: Option<f64> =
        permagent::sqlx::query_scalar("SELECT rank FROM decisions WHERE id = ?")
            .bind(&d_second.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let new_rank = new_rank.expect("rank must be set after rerank");
    assert!(
        new_rank < 1.0,
        "seeded rank 9.0 must be overwritten by the curation score, got {}",
        new_rank
    );

    // ── Wire 1: choice answer on a parked goal → re-dispatch eligible ──
    let parked = cards::create_card(
        &pool,
        CreateCard {
            project_id: PERSONAL_PROJECT_ID.to_string(),
            title: "Parked goal awaiting a choice".to_string(),
            description: None,
            card_type: Some("goal".to_string()),
            column_id: Some(triage_col.id.clone()),
            created_by: None,
            metadata_json: Some(serde_json::json!({
                "needs_human_attention": true,
                "attempt_count": 3,
            })),
        },
    )
    .await
    .unwrap();
    let choice = decisions::create_decision(
        &pool,
        NewDecision {
            kind: "choice".to_string(),
            goal_id: Some(parked.id.clone()),
            project_id: Some(PERSONAL_PROJECT_ID.to_string()),
            headline: Some("Choose which storage backend to build on".to_string()),
            detail: Some("two viable backends".to_string()),
            payload: serde_json::json!({
                "question": "Which backend?",
                "options": [
                    {"id": "sqlite", "label": "SQLite"},
                    {"id": "postgres", "label": "Postgres"}
                ]
            }),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/decisions/{}/answer", choice.id),
            serde_json::json!({
                "answer": "choice",
                "choiceId": "postgres",
                "note": "scale matters"
            }),
        ))
        .await
        .unwrap();
    // 200 also proves wire 2's absence tolerance: the Learn call sits on
    // this path and must not break it with no Brain mounted.
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(v["decision"]["status"], "answered");
    assert_eq!(v["decision"]["acted_by"], "jesse");
    let effect = v["effect"].as_str().expect("resume effect must run");
    assert!(
        effect.contains("resumed") && effect.contains("re-dispatch eligible"),
        "unexpected effect: {}",
        effect
    );
    assert!(v["effectError"].is_null(), "effect must succeed: {}", v);

    // The goal is re-dispatch eligible: Ready through the guard, attention
    // flag cleared, attempt cap extended past the spent attempts.
    let card = cards::get_card(&pool, &parked.id).await.unwrap().unwrap();
    let col = cards::get_column(&pool, &card.column_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(col.state_binding.as_deref(), Some("ready"));
    assert_eq!(
        card.metadata_json.get("needs_human_attention"),
        Some(&serde_json::Value::Bool(false))
    );
    assert!(
        card.metadata_json["budget"]["attempt_cap"]
            .as_u64()
            .unwrap()
            > 3,
        "attempt cap must be extended: {}",
        card.metadata_json
    );

    std::env::remove_var("PERMAGENT_PATH_ROOT");
}
