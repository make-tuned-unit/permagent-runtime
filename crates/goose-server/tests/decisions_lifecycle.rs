//! Decision Inbox lifecycle, end to end through the HTTP router.
//!
//! Substitutes for a curl demonstration against a live daemon: the sandbox
//! blocks long-running background daemon processes, so this drives the exact
//! same axum router (`routes::decisions::routes`) with tower oneshot requests
//! against an AppState rooted at a throwaway PERMAGENT_PATH_ROOT.
//!
//! Runs as its own integration-test binary (own process) so the global
//! AgentManager singleton is constructed against the temp root and never
//! touches the live data root.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use permagent::cards::{self, CreateCard};
use permagent::decisions::{self, NewDecision};
use permagent::projects::PERSONAL_PROJECT_ID;

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

// One test per integration binary (own process): PERMAGENT_PATH_ROOT and the
// startup singletons are per-process, so #[serial] had nothing to serialize
// against here — it was a no-op (superseding #695). The real flake was a
// server-side lock-upgrade race in the approve effect
// (goal_transition::advance_goal_checked), fixed there with BEGIN IMMEDIATE.
#[tokio::test(flavor = "multi_thread")]
async fn decision_lifecycle_through_router() {
    // Throwaway data root for the whole process (single test in this binary).
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let app = permagent_daemon::routes::decisions::routes(state.clone());
    let pool = state.session_manager().pool_clone().await.unwrap();

    // ── 1. Empty inbox with summary envelope ──
    let resp = app.clone().oneshot(get("/api/decisions")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(v["summary"]["total_pending"], 0);
    assert!(v["summary"]["handled_count"].is_i64());
    assert!(v["summary"]["goals_in_flight"].is_i64());
    assert_eq!(v["items"].as_array().unwrap().len(), 0);

    // ── 2. Seed a goal in Review with its approve_review decision (what
    //       handle_goal_completion produces on worker success) ──
    cards::seed_goal_columns(&pool, PERSONAL_PROJECT_ID)
        .await
        .unwrap();
    let review_col = cards::get_goal_column(&pool, PERSONAL_PROJECT_ID, "review")
        .await
        .unwrap()
        .unwrap();
    let goal = cards::create_card(
        &pool,
        CreateCard {
            project_id: PERSONAL_PROJECT_ID.to_string(),
            title: "Lifecycle demo goal".to_string(),
            description: Some("integration".to_string()),
            card_type: Some("goal".to_string()),
            column_id: Some(review_col.id.clone()),
            created_by: None,
            metadata_json: Some(serde_json::json!({
                "attempt_count": 1,
                "goal_state": "review"
            })),
        },
    )
    .await
    .unwrap();
    let decision = decisions::create_decision(
        &pool,
        NewDecision {
            kind: "approve_review".to_string(),
            goal_id: Some(goal.id.clone()),
            project_id: Some(PERSONAL_PROJECT_ID.to_string()),
            headline: Some("Review the finished work on \"Lifecycle demo goal\"".to_string()),
            detail: Some("Worker reported success; inspect and approve or reject.".to_string()),
            payload: serde_json::json!({}),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(decision.tier, 1, "goal_approve_standard seeds at tier 1");

    // ── 3. Inbox lists the open decision, ranked, with goal title joined ──
    let resp = app.clone().oneshot(get("/api/decisions")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(v["summary"]["total_pending"], 1);
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], serde_json::json!(decision.id));
    assert_eq!(items[0]["kind"], "approve_review");
    assert_eq!(items[0]["goal_title"], "Lifecycle demo goal");
    assert!(!items[0]["headline"].as_str().unwrap().is_empty());
    assert!(!items[0]["detail"].as_str().unwrap().is_empty());

    // ── 4. Answer: approve. HTTP attribution is 'jesse'; the gated effect
    //       (Review → Complete through the guard) executes atomically after ──
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/decisions/{}/answer", decision.id),
            serde_json::json!({"answer": "approve", "note": "looks good"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(v["decision"]["status"], "answered");
    assert_eq!(v["decision"]["acted_by"], "jesse");
    assert_eq!(v["decision"]["answer"], "approve");
    assert_eq!(v["effect"], "goal approved: Review → Complete");
    assert!(v["effectError"].is_null(), "effect must succeed: {}", v);

    // The goal actually completed.
    let card = cards::get_card(&pool, &goal.id).await.unwrap().unwrap();
    let col = cards::get_column(&pool, &card.column_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(col.state_binding.as_deref(), Some("complete"));

    // ── 5. Answering again races out: 409 Conflict ──
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/decisions/{}/answer", decision.id),
            serde_json::json!({"answer": "approve"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // Unknown decision: 404.
    let resp = app
        .clone()
        .oneshot(post_json(
            "/api/decisions/no-such-decision/answer",
            serde_json::json!({"answer": "approve"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // ── 6. History shows the resolved decision (cursor on audit.seq) ──
    let resp = app
        .clone()
        .oneshot(get("/api/decisions/history?limit=10"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    let items = v["items"].as_array().unwrap();
    assert!(
        items
            .iter()
            .any(|i| i["id"] == serde_json::json!(decision.id)),
        "resolved decision must appear in history: {}",
        v
    );
    assert!(v["nextBefore"].is_i64());

    // Inbox is empty again.
    let resp = app.clone().oneshot(get("/api/decisions")).await.unwrap();
    let v = body_json(resp).await;
    assert_eq!(v["summary"]["total_pending"], 0);
    assert_eq!(v["summary"]["handled_count"], 1);

    // ── 7. Audit hash chain is intact end-to-end ──
    let report = decisions::verify_audit_chain(&pool).await.unwrap();
    assert!(report.intact, "{}", report.detail);
    assert!(
        report.total_rows >= 3,
        "created + answered + transition rows expected, got {}",
        report.total_rows
    );

    // ── 8. Lane L4 overflow contract: default top 10, ?all=1 returns all ──
    for i in 0..12 {
        decisions::create_decision(
            &pool,
            NewDecision {
                kind: "choice".to_string(),
                project_id: Some(PERSONAL_PROJECT_ID.to_string()),
                headline: Some(format!("Pick an option for batch item {}", i)),
                detail: Some("overflow contract test".to_string()),
                payload: serde_json::json!({
                    "question": "Which?",
                    "options": [
                        {"id": "a", "label": "A"},
                        {"id": "b", "label": "B"}
                    ]
                }),
                rank: Some(i as f64),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    let resp = app.clone().oneshot(get("/api/decisions")).await.unwrap();
    let v = body_json(resp).await;
    assert_eq!(
        v["items"].as_array().unwrap().len(),
        10,
        "default is top 10"
    );
    assert_eq!(
        v["summary"]["total_pending"], 12,
        "+M more computed from summary"
    );

    let resp = app
        .clone()
        .oneshot(get("/api/decisions?all=1"))
        .await
        .unwrap();
    let v = body_json(resp).await;
    assert_eq!(
        v["items"].as_array().unwrap().len(),
        12,
        "?all=1 returns every open decision"
    );

    std::env::remove_var("PERMAGENT_PATH_ROOT");
}
