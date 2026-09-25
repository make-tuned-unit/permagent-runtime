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
    let mut request = Request::builder()
        .uri(uri)
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    request
        .extensions_mut()
        .insert(permagent_daemon::middleware::auth::AuthPrincipal::Master);
    request
}

fn git_in(root: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("git must be on PATH for this test");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// R49a: a real trunk on `main` plus a real LINKED worktree on `goal/run-1`
/// one commit ahead of it — the shape `land_approved_goal`
/// (`decisions_effects.rs`) actually needs, and the shape production always
/// hands it. Every genuine worker success writes `dispatch_evidence` from a
/// `GoalEvidence` value (`goal_engine.rs`) whose `worktree_path` field is a
/// plain, non-optional `String` — never absent — via
/// `cards::set_goal_dispatch_evidence(&tracker_pool, &tracker_card_id, v)`
/// at `orchestrator.rs`'s dispatch-teardown handler (the real
/// `handle_goal_completion` producer this test's fixture claims to mirror).
/// Returns (trunk dir, worktrees dir, worktree path, baseline sha, head sha)
/// — both TempDirs must outlive the test.
fn landable_goal_repo() -> (tempfile::TempDir, tempfile::TempDir, String, String, String) {
    let trunk = tempfile::tempdir().unwrap();
    let root = trunk.path();
    git_in(root, &["init", "-b", "main"]);
    git_in(root, &["config", "user.email", "test@example.com"]);
    git_in(root, &["config", "user.name", "Test"]);
    std::fs::write(root.join("README.md"), "base\n").unwrap();
    git_in(root, &["add", "-A"]);
    git_in(root, &["commit", "-m", "base"]);
    let baseline = git_in(root, &["rev-parse", "HEAD"]);
    git_in(root, &["checkout", "-b", "goal/run-1"]);
    std::fs::write(root.join("work.txt"), "the goal's work\n").unwrap();
    git_in(root, &["add", "-A"]);
    git_in(root, &["commit", "-m", "the work"]);
    let head = git_in(root, &["rev-parse", "HEAD"]);
    git_in(root, &["checkout", "main"]);

    let worktrees = tempfile::tempdir().unwrap();
    let worktree_path = worktrees.path().join("run-1");
    git_in(
        root,
        &[
            "worktree",
            "add",
            worktree_path.to_str().unwrap(),
            "goal/run-1",
        ],
    );

    (
        trunk,
        worktrees,
        worktree_path.to_string_lossy().into_owned(),
        baseline,
        head,
    )
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
    // D3: the windowed, agent-only count the inbox modal reads.
    assert_eq!(v["summary"]["handled"]["count"], 0);
    assert!(v["summary"]["handled"]["since"].is_string());
    assert!(v["summary"]["goals_in_flight"].is_i64());
    assert_eq!(v["items"].as_array().unwrap().len(), 0);

    // ── 2. Seed a goal in Review with its approve_review decision (what
    //       handle_goal_completion produces on worker success) ──
    //
    // R49a (root cause): this fixture used to seed `dispatch_evidence` as
    // just `{"verdict": {"status": "pass"}}` — no `commits`, no
    // `worktree_path`, no real git anywhere. That is not the shape a real
    // router-driven approval ever carries: every genuine worker success
    // writes `dispatch_evidence` from a `GoalEvidence` value
    // (`goal_engine.rs`) whose `worktree_path` is a plain, non-optional
    // `String`, via `cards::set_goal_dispatch_evidence` at
    // `orchestrator.rs`'s dispatch-teardown handler — the exact producer
    // this fixture's own comment claims to mirror. With no worktree hint,
    // `land_approved_goal` (`decisions_effects.rs`) could not even reach
    // its trunk-relative fallback (R43a/R43a-b) and refused with
    // `reason=no_evidence`, parking a goal the router had just approved.
    // Give it a real trunk + a real linked worktree one commit ahead, the
    // shape land_approved_goal actually needs and production always hands
    // it.
    let (trunk, _worktrees, worktree_path, baseline, head) = landable_goal_repo();
    permagent::projects::update_project(
        &pool,
        PERSONAL_PROJECT_ID,
        permagent::projects::UpdateProject {
            root_path: Some(Some(trunk.path().to_string_lossy().into_owned())),
            ..Default::default()
        },
    )
    .await
    .unwrap();

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
                "goal_state": "review",
                // approve_review_verification_status (decisions_effects.rs)
                // treats missing evidence as "pending", never an implicit
                // pass (56cee825) — an approve_review decision only clears
                // the router's answer boundary once the verifier recorded a
                // "pass" verdict here, the same shape
                // handle_goal_completion/set_goal_dispatch_evidence writes.
                "dispatch_evidence": {
                    "worktree_path": worktree_path,
                    "baseline_commit": baseline,
                    "head_commit": head,
                    "commits": [format!("{head} the work")],
                    "verdict": {"status": "pass"},
                }
            })),
        },
    )
    .await
    .unwrap();
    // Pin the precondition the router answer boundary enforces: a missing
    // verdict reads as "pending", never an implicit pass, and the fixture
    // above must satisfy it or step 4 below gets a false 409.
    assert_eq!(
        permagent::decisions_effects::approve_review_verification_status(&pool, &goal.id)
            .await
            .unwrap(),
        "pass"
    );
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
    // Modal redesign contract: plain summary, per-answer effects keyed on the
    // answers the route accepts, and the raw detail under `technical`.
    assert!(items[0]["summary"].is_string());
    assert!(items[0]["effects"]["approve"].is_string());
    assert!(items[0]["effects"]["reject"].is_string());
    assert_eq!(items[0]["effects"].as_object().unwrap().len(), 2);
    assert_eq!(items[0]["technical"]["detail"], items[0]["detail"]);
    assert_eq!(items[0]["technical"]["goal_id"], serde_json::json!(goal.id));

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
    // R49a: `land_approved_goal` always reports what it did, appended after
    // a semicolon (`decisions_effects.rs`'s `apply_decision_effect`) — a
    // bare "goal approved: Review → Complete" with no suffix can only mean
    // landing was never reached at all. This fixture's approval must
    // actually land the goal's real, already-committed work onto trunk.
    assert!(
        v["effect"]
            .as_str()
            .unwrap()
            .starts_with("goal approved: Review → Complete; landed:"),
        "the router approve path must land the goal, not park it: {}",
        v
    );
    assert!(v["effectError"].is_null(), "effect must succeed: {}", v);

    // The goal actually completed.
    let card = cards::get_card(&pool, &goal.id).await.unwrap().unwrap();
    let col = cards::get_column(&pool, &card.column_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(col.state_binding.as_deref(), Some("complete"));

    // Pin the corrected landing behaviour directly: the persisted outcome
    // says "landed" (not merely that the card moved to Complete — R43a-c
    // exists precisely because those two things used to diverge), and the
    // trunk really did fast-forward onto the worktree's head commit.
    let landing = card
        .metadata_json
        .pointer("/dispatch_evidence/landing")
        .expect("a successful router-driven approval must persist a landing outcome");
    assert_eq!(
        landing.get("outcome").and_then(|v| v.as_str()),
        Some("landed"),
        "router approve → land_approved_goal must actually land already-committed, \
         reviewed work rather than refuse it: {}",
        landing
    );
    assert_eq!(
        git_in(trunk.path(), &["rev-parse", "HEAD"]),
        head,
        "the trunk must fast-forward onto the goal's real head commit"
    );

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
    // The user answered it, so the agent-only count stays at zero.
    assert_eq!(v["summary"]["handled"]["count"], 0);

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
