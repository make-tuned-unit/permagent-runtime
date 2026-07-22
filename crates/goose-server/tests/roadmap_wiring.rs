//! Integration-wiring proofs for post-creation roadmap editing (#251) through
//! the real HTTP router (`routes::cards::routes`) against an AppState rooted
//! at a throwaway PERMAGENT_PATH_ROOT.
//!
//! Covers, end to end over HTTP with round-trip persistence:
//!   - POST /roadmap/goals: root insert lands Ready; a dependent insert lands
//!     Triage with its validated depends_on persisted;
//!   - PUT  /roadmap/goals/{id}/dependencies: a cycle is rejected (400, named
//!     in the error) and nothing is written; a valid edit persists;
//!   - POST /roadmap/goals/{id}/remove: dependents are rewired onto the
//!     removed goal's own deps and the removed goal is cancelled;
//!   - PATCH /cards/{id}: a raw metadata write touching depends_on is refused
//!     (protected key) — the validated endpoint is the only writer.
//!
//! Runs as its own integration-test binary (own process): PERMAGENT_PATH_ROOT
//! and the startup singletons are per-process, so this is the single test in
//! the binary (same pattern as decision_wiring.rs).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use permagent::cards;
use permagent::projects::PERSONAL_PROJECT_ID;

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

async fn body_text(resp: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8_lossy(&bytes).to_string()
}

fn req(method: &str, uri: &str, body: Option<serde_json::Value>) -> Request<Body> {
    let builder = Request::builder()
        .uri(uri)
        .method(method)
        .header("content-type", "application/json");
    match body {
        Some(b) => builder.body(Body::from(b.to_string())).unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

async fn state_of(pool: &sqlx::Pool<sqlx::Sqlite>, card_id: &str) -> String {
    let card = cards::get_card(pool, card_id).await.unwrap().unwrap();
    let col = cards::get_column(pool, &card.column_id)
        .await
        .unwrap()
        .unwrap();
    col.state_binding.unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn roadmap_editing_round_trips_through_router() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let app = permagent_daemon::routes::cards::routes(state.clone());
    let pool = state.session_manager().pool_clone().await.unwrap();

    cards::seed_goal_columns(&pool, PERSONAL_PROJECT_ID)
        .await
        .unwrap();

    // ── Insert a root goal: created and promoted straight to Ready ──
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            &format!("/api/projects/{}/roadmap/goals", PERSONAL_PROJECT_ID),
            Some(serde_json::json!({ "title": "Root goal" })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let root = body_json(resp).await;
    let root_id = root["id"].as_str().unwrap().to_string();
    assert_eq!(root["cardType"], "goal");
    assert_eq!(
        state_of(&pool, &root_id).await,
        "ready",
        "root insert → Ready"
    );

    // ── Insert a dependent goal: waits in Triage with deps persisted ──
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            &format!("/api/projects/{}/roadmap/goals", PERSONAL_PROJECT_ID),
            Some(serde_json::json!({ "title": "Child goal", "dependsOn": [root_id] })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let child = body_json(resp).await;
    let child_id = child["id"].as_str().unwrap().to_string();
    assert_eq!(state_of(&pool, &child_id).await, "triage");
    assert_eq!(
        child["metadataJson"]["depends_on"],
        serde_json::json!([root_id]),
        "validated depends_on persisted on the card"
    );

    // ── A dangling dependency id is rejected at insert ──
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            &format!("/api/projects/{}/roadmap/goals", PERSONAL_PROJECT_ID),
            Some(serde_json::json!({ "title": "Bad", "dependsOn": ["nonexistent"] })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // ── Dependency edit that would create a cycle: 400, nothing written ──
    let resp = app
        .clone()
        .oneshot(req(
            "PUT",
            &format!(
                "/api/projects/{}/roadmap/goals/{}/dependencies",
                PERSONAL_PROJECT_ID, root_id
            ),
            Some(serde_json::json!({ "dependsOn": [child_id] })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let err = body_text(resp).await;
    assert!(err.contains("cycle"), "cycle named in the error: {err}");
    let root_card = cards::get_card(&pool, &root_id).await.unwrap().unwrap();
    assert!(
        root_card
            .metadata_json
            .get("depends_on")
            .map(|v| v == &serde_json::json!([]))
            .unwrap_or(true),
        "rejected edit must not write"
    );

    // ── Raw metadata PATCH touching depends_on is refused (protected) ──
    // Carry the card's existing metadata and change ONLY depends_on, so the
    // refusal is specifically about the dependency key.
    let child_card = cards::get_card(&pool, &child_id).await.unwrap().unwrap();
    let mut sneaky = child_card.metadata_json.as_object().cloned().unwrap();
    sneaky.insert("depends_on".to_string(), serde_json::json!([]));
    let resp = app
        .clone()
        .oneshot(req(
            "PATCH",
            &format!("/api/projects/{}/cards/{}", PERSONAL_PROJECT_ID, child_id),
            Some(serde_json::json!({ "metadataJson": serde_json::Value::Object(sneaky) })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let err = body_text(resp).await;
    assert!(err.contains("depends_on"), "{err}");

    // ── Valid dependency edit persists (re-parent child → no deps yet: use a
    //    second root so the graph edit is a real re-parent) ──
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            &format!("/api/projects/{}/roadmap/goals", PERSONAL_PROJECT_ID),
            Some(serde_json::json!({ "title": "Second root" })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let root2 = body_json(resp).await;
    let root2_id = root2["id"].as_str().unwrap().to_string();

    let resp = app
        .clone()
        .oneshot(req(
            "PUT",
            &format!(
                "/api/projects/{}/roadmap/goals/{}/dependencies",
                PERSONAL_PROJECT_ID, child_id
            ),
            Some(serde_json::json!({ "dependsOn": [root_id, root2_id] })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let updated = body_json(resp).await;
    assert_eq!(
        updated["metadataJson"]["depends_on"],
        serde_json::json!([root_id, root2_id])
    );

    // ── Remove root2: child rewired onto root2's deps (none extra), root2
    //    cancelled ──
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            &format!(
                "/api/projects/{}/roadmap/goals/{}/remove",
                PERSONAL_PROJECT_ID, root2_id
            ),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let removal = body_json(resp).await;
    assert_eq!(removal["removed"], serde_json::json!(true));
    assert_eq!(removal["cancelled"], serde_json::json!(true));
    assert_eq!(removal["rewiredDependents"], serde_json::json!(1));

    assert_eq!(state_of(&pool, &root2_id).await, "cancelled");
    let child_after = cards::get_card(&pool, &child_id).await.unwrap().unwrap();
    assert_eq!(
        child_after.metadata_json.get("depends_on"),
        Some(&serde_json::json!([root_id])),
        "child rewired: root2 spliced out, root kept"
    );

    // ── #252: per-goal auto-approve toggle round-trips over HTTP ──
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            &format!(
                "/api/projects/{}/cards/{}/auto-approve",
                PERSONAL_PROJECT_ID, child_id
            ),
            Some(serde_json::json!({ "enabled": true })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let flagged = body_json(resp).await;
    assert_eq!(
        flagged["metadataJson"]["auto_approve"],
        serde_json::json!(true)
    );
    let persisted = cards::get_card(&pool, &child_id).await.unwrap().unwrap();
    assert_eq!(
        persisted.metadata_json.get("auto_approve"),
        Some(&serde_json::json!(true)),
        "flag persisted on the card"
    );

    // Raw metadata PATCH touching auto_approve is refused (protected key).
    let mut sneaky = persisted.metadata_json.as_object().cloned().unwrap();
    sneaky.insert("auto_approve".to_string(), serde_json::json!(false));
    let resp = app
        .clone()
        .oneshot(req(
            "PATCH",
            &format!("/api/projects/{}/cards/{}", PERSONAL_PROJECT_ID, child_id),
            Some(serde_json::json!({ "metadataJson": serde_json::Value::Object(sneaky) })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let err = body_text(resp).await;
    assert!(err.contains("auto_approve"), "{err}");

    // Toggle back off through the audited endpoint: flag removed.
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            &format!(
                "/api/projects/{}/cards/{}/auto-approve",
                PERSONAL_PROJECT_ID, child_id
            ),
            Some(serde_json::json!({ "enabled": false })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let unflagged = body_json(resp).await;
    assert!(unflagged["metadataJson"].get("auto_approve").is_none());
}
