//! Integration-wiring proofs for the growth action lifecycle (#1048) through
//! the real HTTP router (`routes::growth_actions::routes`) against an AppState
//! rooted at a throwaway PERMAGENT_PATH_ROOT.
//!
//! These exist because the unit tests in `routes::growth_actions` stop at the
//! pure predicates. `verify_mode` was tested by asserting that a row with a
//! `verified_at` returns `Recheck` — which is its definition, not its effect —
//! and `reject_pointless_archive` the same way. Both guards are enforced at
//! call sites inside handlers, so deleting the `&& !rechecking` from the status
//! write, or the whole `if rechecking { return }` branch, left every test in
//! the crate green while a second Verify click re-stamped `verified_at` and
//! dragged a judged action back into measurement. `verified_at` is the pivot
//! `metrics::pivot_date` measures every comparison window from, so that is a
//! silent, data-visible corruption of a finished experiment.
//!
//! The claim that this could not be written ("the handler needs an `AppState`
//! and there is no such harness in this crate") is false: sixteen files in this
//! directory build exactly this harness. roadmap_wiring.rs is the pattern.
//!
//! Runs as its own integration-test binary (own process): PERMAGENT_PATH_ROOT
//! and the startup singletons are per-process, so this is the single test in
//! the binary (same pattern as decision_wiring.rs).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use permagent::growth::store::{self as growth_store, ActionSeed};
use permagent::projects::{self, CreateProject};

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
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

/// A frozen baseline in the shape `sweep::Baseline` deserialises, so the
/// re-check path can be asserted to hand back THIS one rather than a
/// recomputed one.
fn baseline() -> serde_json::Value {
    serde_json::json!({
        "metric": "sessions",
        "dir": "up",
        "pivot": "2026-08-13",
        "takenAt": "2026-08-12T00:00:00Z",
        "before": {},
        "weekly": [4.0, 6.0],
        "earliestEvent": null
    })
}

fn seed(title: &str) -> ActionSeed {
    ActionSeed {
        title: title.into(),
        recommendation: format!("recommendation for {title}"),
        category: Some("aeo".into()),
        artifact_kind: Some("prompt".into()),
        artifact: None,
        target_metric: Some("sessions".into()),
        target_dir: Some("up".into()),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_growth_action_lifecycle_holds_through_the_router() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let app = permagent_daemon::routes::growth_actions::routes(state.clone());
    let pool = state.session_manager().pool_clone().await.unwrap();

    let project = projects::create_project(
        &pool,
        CreateProject {
            name: "GrocerySaver".to_string(),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    // ── A finished experiment: verified, measured, judged ──
    let action = growth_store::upsert_suggested(&pool, &project.id, &seed("Add FAQPage schema"))
        .await
        .unwrap();
    let frozen = baseline().to_string();
    growth_store::record_verification(
        &pool,
        &project.id,
        &action.id,
        growth_store::VerificationEvidence {
            baseline_json: Some(&frozen),
            ..growth_store::VerificationEvidence::new("git_commit", "2026-08-12T00:00:00Z")
        },
    )
    .await
    .unwrap();
    growth_store::set_status(
        &pool,
        &project.id,
        &action.id,
        growth_store::STATUS_JUDGED,
        None,
    )
    .await
    .unwrap();

    let before = growth_store::get(&pool, &project.id, &action.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.verified_at.as_deref(), Some("2026-08-12T00:00:00Z"));

    // ── Re-check: reports, never records ──
    // The body is what the card actually sends: `targetBody()` always supplies
    // the row's own pre-registration, which is what used to drive the
    // `set_status(.., STATUS_DONE, supplied)` write on every click.
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            &format!(
                "/api/projects/{}/growth-actions/{}/verify",
                project.id, action.id
            ),
            Some(serde_json::json!({ "targetMetric": "sessions", "targetDir": "up" })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let payload = body_json(resp).await;
    assert_eq!(payload["verified"], true, "the stored fact is reported");
    // The frozen baseline is handed back rather than recomputed: `pivot` is the
    // day every comparison window is measured from, and it is still the one
    // stored at first verification.
    assert_eq!(payload["baseline"]["pivot"], "2026-08-13");
    assert_eq!(payload["baseline"]["weekly"], serde_json::json!([4.0, 6.0]));

    let after = growth_store::get(&pool, &project.id, &action.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after.verified_at, before.verified_at,
        "the measurement pivot must not move: every comparison window is \
         measured from it, and sliding it forward against a baseline frozen \
         days earlier computes the verdict with the result already in view"
    );
    assert_eq!(
        after.status,
        growth_store::STATUS_JUDGED,
        "a judged action must not be dragged back into measurement"
    );

    // ── Archiving a judged action: allowed, and it leaves the board ──
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            &format!(
                "/api/projects/{}/growth-actions/{}/status",
                project.id, action.id
            ),
            Some(serde_json::json!({ "status": "archived" })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["status"], "archived");
    assert!(
        growth_store::board(&pool, &project.id)
            .await
            .unwrap()
            .is_empty(),
        "archiving is what takes an action off the board"
    );

    // ── Archiving something nothing has happened to: refused, with a reason ──
    let untouched =
        growth_store::upsert_suggested(&pool, &project.id, &seed("Rewrite the homepage"))
            .await
            .unwrap();
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            &format!(
                "/api/projects/{}/growth-actions/{}/status",
                project.id, untouched.id
            ),
            Some(serde_json::json!({ "status": "archived" })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    // REGRESSION: this body used to be `text/plain`, and the UI parses every
    // error body as JSON — so this carefully worded refusal reached the user as
    // the literal string "Unknown error". `apiFetch` reads `.message`.
    let refusal = body_json(resp).await;
    assert!(
        refusal["message"]
            .as_str()
            .unwrap_or_default()
            .contains("Dismiss it instead"),
        "the refusal has to survive as JSON the UI can read: {refusal}"
    );

    // ── Dismissal is that exit, and it is reachable over the same route ──
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            &format!(
                "/api/projects/{}/growth-actions/{}/status",
                project.id, untouched.id
            ),
            Some(serde_json::json!({ "status": "dismissed" })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["status"], "dismissed");

    // ── A dismissed action is dismissible from ANY status ──
    //
    // REGRESSION for the user report of 2026-08-19: "some of the actions I am
    // seeing in the Grow tab are stale ones that I already ran. I should be
    // able to dismiss it." The panel used to offer the control only on
    // `suggested`; the route has always accepted it from anywhere, and the
    // panel now keys on the durable row instead of a status allowlist, so this
    // pins that the server half of that pairing is real.
    let already_done = growth_store::upsert_suggested(&pool, &project.id, &seed("Add alt text"))
        .await
        .unwrap();
    growth_store::set_status(
        &pool,
        &project.id,
        &already_done.id,
        growth_store::STATUS_DONE,
        None,
    )
    .await
    .unwrap();
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            &format!(
                "/api/projects/{}/growth-actions/{}/status",
                project.id, already_done.id
            ),
            Some(serde_json::json!({ "status": "dismissed" })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["status"], "dismissed");

    // ── The board splits Actions from Tracking ──
    //
    // The user, 2026-08-19: "once a verified action was taken it should
    // disappear from the list of Actions and go into a tracker view". It is
    // MOVED, never hidden — #1053 kept in-flight work on the board precisely so
    // the sweep could not measure something the user could no longer see.
    let resp = app
        .clone()
        .oneshot(req(
            "GET",
            &format!("/api/projects/{}/growth-actions", project.id),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let board = body_json(resp).await;
    let titles = |key: &str| -> Vec<String> {
        board[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["title"].as_str().unwrap_or_default().to_string())
            .collect()
    };
    assert!(
        titles("actions").is_empty(),
        "everything on this project is filed, dismissed or measured: {:?}",
        titles("actions")
    );
    // The verified-then-judged action from the top of this test was archived,
    // so it is on the shelf rather than in Tracking — and its frozen baseline
    // travels with it, which is what the Tracking card renders the verdict
    // against.
    assert_eq!(titles("archived"), vec!["Add FAQPage schema"]);
    assert_eq!(
        board["archived"][0]["identity"]["baseline"]["pivot"], "2026-08-13",
        "the frozen baseline has to reach the card: {board}"
    );

    // ── A review outlives the request that asked for it ──
    //
    // REGRESSION for the user report of 2026-08-19: "I pressed Review my
    // analytics and then clicked another tab, when I went back it looks like it
    // stopped running." The generation used to run inside this handler, so the
    // whole feature was hostage to one HTTP request and the only record that it
    // was happening was a `useState` in the component that issued it.
    //
    // No provider is configured here, so the review fails fast — which is the
    // point: what is asserted is that the POST answers immediately and the work
    // then completes on its own, writing its reason where the next GET finds
    // it, with no client holding anything open.
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            &format!("/api/projects/{}/growth-actions/generate", project.id),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let started = body_json(resp).await;
    assert!(
        started.get("actions").is_some(),
        "the POST answers with the board as it stands, not with nothing: {started}"
    );

    // The review is running (or has already finished) on a task of its own.
    // Poll the same surface the panel polls until it reports itself done.
    let mut settled = serde_json::Value::Null;
    for _ in 0..100 {
        let resp = app
            .clone()
            .oneshot(req(
                "GET",
                &format!("/api/projects/{}/growth-actions", project.id),
                None,
            ))
            .await
            .unwrap();
        settled = body_json(resp).await;
        if settled["generating"] == serde_json::Value::Bool(false) && settled["reason"].is_string()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(
        settled["generating"],
        serde_json::Value::Bool(false),
        "the review has to finish on its own: {settled}"
    );
    assert!(
        settled["reason"].is_string(),
        "a review that ran with nobody waiting still records what it found: {settled}"
    );
}
