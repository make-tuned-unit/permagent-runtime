//! The Build tab's cost meter, from a ledger write to a bus frame.
//!
//! The bug this pins: the meter read `$0.00` for a whole day of coding. Not
//! because the money was unrecorded — the CLI harness writes a correct
//! `cost_ledger` row per turn and prints a correct total in its own footer —
//! but because the harness mints its OWN session id and nothing ever told the
//! browser that id existed. The meter subscribed to the chat session, which is
//! idle the entire time the user is coding, so it was reading a real number off
//! the wrong account.
//!
//! `POST /api/coding-sessions/spend` is the seam that closes it: the harness
//! announces its session id, the daemon re-reads the rollup the harness already
//! wrote, and emits it on the bus every surface already listens to.
//!
//! Asserted here, against a real `AppState`, a real router and real ledger
//! writes:
//!
//!   1. the announce reports the session's rollup, not zero — the whole
//!      complaint;
//!   2. it counts a cache-read call at its cache-read rate, so a harness that
//!      is mostly cache hits is not billed as if it were not;
//!   3. it emits `session_spend_changed` carrying the figures, so the meter
//!      moves without polling;
//!   4. `today_usd` spans sessions, so the "today" figure is not just this
//!      session's total wearing a different label;
//!   5. an estimated price is REPORTED as estimated, so a fail-closed worst
//!      case cannot render as a bill;
//!   6. the announce writes nothing — a second writer of the rollup would
//!      double every number on the meter.
//!
//! Its own integration binary (own process): `PERMAGENT_PATH_ROOT` and the
//! global event bus are per-process, same as `liveness_wire.rs`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::time::Duration;
use tower::ServiceExt;

use permagent::config::GooseMode;
use permagent::session::session_manager::{CostLedgerRow, CostTier};
use permagent::session::SessionType;

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

fn req(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// One provider call as the harness records it. `cost_usd` is the number the
/// meter shows, and it is supplied rather than recomputed here on purpose: this
/// test is about whether the announcement carries what the ledger holds, not
/// about re-testing `cost_breakdown`, which has its own tests.
#[allow(clippy::too_many_arguments)]
fn call(
    session_id: &str,
    model: &str,
    input: i64,
    output: i64,
    cache_read: i64,
    cost_usd: f64,
    cache_read_cost: f64,
    is_estimated: bool,
) -> CostLedgerRow {
    CostLedgerRow {
        call_id: uuid::Uuid::new_v4().to_string(),
        ts: chrono::Utc::now().to_rfc3339(),
        session_id: session_id.to_string(),
        parent_session_id: None,
        task_id: None,
        goal_id: None,
        subagent_id: None,
        provider: Some("zai".to_string()),
        model: Some(model.to_string()),
        cost_tier: CostTier::PaidApi,
        is_headless: false,
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cache_read,
        cache_write_tokens: 0,
        input_cost: cost_usd - cache_read_cost,
        output_cost: 0.0,
        cache_read_cost,
        cache_write_cost: 0.0,
        cost_usd,
        cache_savings_usd: 0.0,
        is_estimated,
    }
}

async fn drain(
    rx: &mut tokio::sync::broadcast::Receiver<permagent::events::PermagentEvent>,
) -> Vec<permagent::events::PermagentEvent> {
    let mut seen = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
            Ok(Ok(ev)) => seen.push(ev),
            _ => break,
        }
    }
    seen
}

#[tokio::test(flavor = "multi_thread")]
async fn a_coding_turn_announces_real_spend_on_the_bus() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let app = permagent_daemon::routes::coding_session::routes(state.clone());
    let manager = state.session_manager();

    // The harness's own session — created the way `permagent run` creates one,
    // with no relationship to any browser chat session.
    let coding = manager
        .create_session(
            tmp.path().to_path_buf(),
            "CLI Session".to_string(),
            SessionType::User,
            GooseMode::default(),
        )
        .await
        .unwrap();

    // Turn one: a plain call, priced from a published rate.
    //
    // TWO writes, because production does two. `append_cost_ledger` advances
    // the MONEY rollups (`cost_usd`, `accumulated_cost_usd`, the cache
    // accumulators) and deliberately does not touch
    // `accumulated_total_tokens` — that column is maintained by the agent's
    // usage path through the session update builder. A fixture that wrote only
    // the ledger would leave the token figure at 0 and quietly prove nothing
    // about the number the meter actually renders.
    manager
        .append_cost_ledger(&call(
            &coding.id, "glm-5.3", 12_000, 800, 0, 0.030, 0.0, false,
        ))
        .await
        .unwrap();
    manager
        .update(&coding.id)
        .accumulated_total_tokens(Some(12_800))
        .apply()
        .await
        .unwrap();

    let mut rx = permagent::events::subscribe();
    let resp = app
        .clone()
        .oneshot(req(
            "/api/coding-sessions/spend",
            serde_json::json!({ "sessionId": coding.id, "workingDir": "/tmp/proj" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let first = body_json(resp).await;

    // (1) The number the meter was reporting as $0.00.
    assert_eq!(first["sessionUsd"].as_f64().unwrap(), 0.030);
    assert_eq!(first["turnUsd"].as_f64().unwrap(), 0.030);
    assert_eq!(first["totalTokens"].as_i64().unwrap(), 12_800);
    assert_eq!(first["estimated"], false);

    // (3) …and it reaches the bus, which is what makes the meter live.
    let frames = drain(&mut rx).await;
    let frame = frames
        .iter()
        .find(|f| {
            f.event_type == permagent::events::PermagentEventType::SessionSpendChanged
                && f.payload["session_id"] == coding.id
        })
        .expect("a turn that spent money must announce it, or no window learns");
    assert_eq!(frame.payload["session_usd"].as_f64().unwrap(), 0.030);
    assert_eq!(frame.payload["model"], "glm-5.3");
    assert_eq!(frame.payload["provider"], "zai");
    assert_eq!(frame.payload["working_dir"], "/tmp/proj");
    assert_eq!(frame.payload["final_turn"], false);

    // (2) Turn two is mostly cache reads. The cheap rate has to survive the
    // trip: a harness whose context is 90% cached and billed as if it were not
    // is the difference between a usable meter and a scary one.
    manager
        .append_cost_ledger(&call(
            &coding.id, "glm-5.3", 600, 400, 20_000, 0.0032, 0.0020, false,
        ))
        .await
        .unwrap();
    manager
        .update(&coding.id)
        .accumulated_total_tokens(Some(13_800))
        .apply()
        .await
        .unwrap();
    let second = body_json(
        app.clone()
            .oneshot(req(
                "/api/coding-sessions/spend",
                serde_json::json!({ "sessionId": coding.id }),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        second["turnUsd"].as_f64().unwrap(),
        0.0032,
        "the cached turn must be billed at what it cost, not at the input rate"
    );
    assert!(
        (second["sessionUsd"].as_f64().unwrap() - 0.0332).abs() < 1e-9,
        "the session total accumulates: {second}"
    );
    assert_eq!(
        second["totalTokens"].as_i64().unwrap(),
        13_800,
        "the token figure tracks the session too, not just the money"
    );

    // (4) "Today" is every session, not this one relabelled.
    let other = manager
        .create_session(
            tmp.path().to_path_buf(),
            "Another session".to_string(),
            SessionType::User,
            GooseMode::default(),
        )
        .await
        .unwrap();
    manager
        .append_cost_ledger(&call(&other.id, "glm-5.3", 100, 100, 0, 0.500, 0.0, false))
        .await
        .unwrap();
    let third = body_json(
        app.clone()
            .oneshot(req(
                "/api/coding-sessions/spend",
                serde_json::json!({ "sessionId": coding.id }),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert!(
        (third["sessionUsd"].as_f64().unwrap() - 0.0332).abs() < 1e-9,
        "another session's spend is not this session's"
    );
    assert!(
        (third["todayUsd"].as_f64().unwrap() - 0.5332).abs() < 1e-9,
        "today spans every session: {third}"
    );

    // (5) An estimate must arrive labelled. `worst_case_pricing` charges the
    // registry's most expensive rate when a model has no published row — the
    // safe direction for a spend cap, and a lie if rendered as a plain bill.
    manager
        .append_cost_ledger(&call(
            &coding.id,
            "glm-5.3-unpriced",
            1_000,
            1_000,
            0,
            0.900,
            0.0,
            true,
        ))
        .await
        .unwrap();
    let mut rx = permagent::events::subscribe();
    let fourth = body_json(
        app.clone()
            .oneshot(req(
                "/api/coding-sessions/spend",
                serde_json::json!({ "sessionId": coding.id, "finalTurn": true }),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        fourth["estimated"], true,
        "an unpriced model's fail-closed worst case must not read as a bill"
    );
    let frames = drain(&mut rx).await;
    let closing = frames
        .iter()
        .find(|f| {
            f.event_type == permagent::events::PermagentEventType::SessionSpendChanged
                && f.payload["final_turn"] == true
        })
        .expect("the closing announcement is the one that pins the total");
    assert_eq!(closing.payload["estimated"], true);

    // (6) Announcing is not writing. The rollup must be exactly the sum of the
    // rows the HARNESS wrote — four announcements have happened, and if any of
    // them had written a row too, this would be larger.
    let ledger_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cost_ledger WHERE session_id = ?")
            .bind(&coding.id)
            .fetch_one(&manager.pool_clone().await.unwrap())
            .await
            .unwrap();
    assert_eq!(
        ledger_rows, 3,
        "the announce endpoint must never write a ledger row — a second writer \
         of the rollup would double every number on the meter"
    );

    // An unknown session is a 404, not a $0.00. The two are easy to confuse and
    // they mean opposite things: one is "this session spent nothing", the other
    // is "I am looking at the wrong account" — which is the exact failure this
    // whole seam exists to end.
    let resp = app
        .oneshot(req(
            "/api/coding-sessions/spend",
            serde_json::json!({ "sessionId": "no-such-session" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
