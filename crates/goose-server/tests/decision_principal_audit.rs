//! Authenticated Decision-Inbox answer attribution, end to end.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use permagent::decisions::{self, NewDecision};
use permagent::sqlx::Row;
use tower::ServiceExt;

fn answer_request(id: &str, token: &str) -> Request<Body> {
    Request::builder()
        .uri(format!("/api/decisions/{id}/answer"))
        .method("POST")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"answer":"approve"}"#))
        .unwrap()
}

async fn tier_two_decision(
    pool: &permagent::sqlx::Pool<permagent::sqlx::Sqlite>,
    label: &str,
) -> decisions::Decision {
    decisions::create_decision(
        pool,
        NewDecision {
            kind: "malformed".to_string(),
            headline: Some(format!("{label} principal audit")),
            detail: Some("Deliberately malformed to exercise the Tier-2 gate".to_string()),
            payload: serde_json::json!({}),
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

async fn recorded_principal(
    pool: &permagent::sqlx::Pool<permagent::sqlx::Sqlite>,
    decision_id: &str,
) -> (String, String) {
    let row = permagent::sqlx::query(
        "SELECT acted_by, principal FROM decision_audit \
         WHERE decision_id = ? AND outcome = 'approve'",
    )
    .bind(decision_id)
    .fetch_one(pool)
    .await
    .unwrap();
    (row.get("acted_by"), row.get("principal"))
}

#[tokio::test(flavor = "multi_thread")]
async fn master_and_device_answers_record_the_admitting_principal() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let master_token = state.daemon_token.clone().unwrap();
    let (device_token, device) = state.device_registry.pair("Audit test device");
    let pool = state.session_manager().pool_clone().await.unwrap();
    let app = permagent_daemon::routes::configure(state);

    let master_decision = tier_two_decision(&pool, "master").await;
    let response = app
        .clone()
        .oneshot(answer_request(&master_decision.id, &master_token))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        recorded_principal(&pool, &master_decision.id).await,
        (decisions::ACTOR_JESSE.to_string(), "master".to_string())
    );

    let device_decision = tier_two_decision(&pool, "device").await;
    let response = app
        .oneshot(answer_request(&device_decision.id, &device_token))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a paired device must remain able to approve Tier-2 decisions"
    );
    assert_eq!(
        recorded_principal(&pool, &device_decision.id).await,
        (decisions::ACTOR_JESSE.to_string(), device.id)
    );

    let report = decisions::verify_audit_chain(&pool).await.unwrap();
    assert!(report.intact, "{}", report.detail);
}
