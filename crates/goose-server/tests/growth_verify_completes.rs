//! The Verify click, end to end, through the real router and against a real
//! git repo — the flow that was broken and the one no test covered.
//!
//! What shipped before this: the user pressed Verify, the check passed,
//! `record_verification` wrote `status = 'verified'`, `render_board` would
//! have bucketed the row into `tracking` on the next read — and the card sat in
//! the suggestions anyway, because `verify_action` was the one growth-actions
//! writer that never emitted `project_changed`. No client was told, so no
//! client refetched. Every existing test still passed: the store tests assert
//! the row moves, and the vitest suites feed the panel a STATIC board payload
//! that is identical before and after the click, so "the card moved" was never
//! once observed.
//!
//! This asserts the four facts that together are the feature, in the order the
//! user experiences them:
//!
//!   1. a fresh `suggested` action verifies from a real commit through the
//!      router (not a row pre-stamped by the test — that is the Recheck branch,
//!      which is all `growth_actions_wiring.rs` reaches);
//!   2. the sha and the check's own sentence are PERSISTED, so the receipt
//!      survives the response that carried it;
//!   3. a `project_changed(project, "growth_actions")` frame reaches the bus —
//!      the announcement the UI refetches on;
//!   4. the next board read has the action out of `actions` and into
//!      `tracking`, and `reopen` puts it back — but is refused once a verdict
//!      exists, because reopening clears the pivot those verdicts were measured
//!      from.
//!
//! Its own integration binary (own process): `PERMAGENT_PATH_ROOT` and the
//! global event bus are per-process, the same reason `liveness_wire.rs` and
//! `growth_actions_wiring.rs` are one test each.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::path::Path;
use std::process::Command;
use std::time::Duration;
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

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?} could not run: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A repo with one commit touching `src/pages/index.astro` — the path the
/// seeded action's prose names, so `verify_git`'s targeted branch is the one
/// exercised rather than the weaker "some commit happened here" fallback.
fn repo_with_a_matching_commit(root: &Path) -> String {
    std::fs::create_dir_all(root.join("src/pages")).unwrap();
    git(root, &["init", "--initial-branch=main"]);
    // Committer identity is set on the repo, never read from the machine: a CI
    // runner with no global git identity would otherwise fail at `commit`.
    git(root, &["config", "user.email", "test@permagent.local"]);
    git(root, &["config", "user.name", "Growth Test"]);
    std::fs::write(
        root.join("src/pages/index.astro"),
        "<script type=\"application/ld+json\">{\"@type\":\"FAQPage\"}</script>\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    git(
        root,
        &["commit", "-m", "Add an FAQPage block to the homepage"],
    );
    git(root, &["rev-parse", "HEAD"])
}

fn seed(title: &str) -> ActionSeed {
    ActionSeed {
        title: title.into(),
        // The path is named in the PROSE because that is where `named_paths`
        // reads it from — the generator is told to name the concrete file.
        recommendation: "Add an FAQPage block to src/pages/index.astro".into(),
        category: Some("aeo".into()),
        artifact_kind: Some("prompt".into()),
        artifact: None,
        target_metric: Some("sessions".into()),
        target_dir: Some("up".into()),
    }
}

/// Drain everything the bus has broadcast so far, up to a short deadline.
async fn drain(
    rx: &mut tokio::sync::broadcast::Receiver<permagent::events::PermagentEvent>,
) -> Vec<permagent::events::PermagentEvent> {
    let mut seen = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
            Ok(Ok(ev)) => seen.push(ev),
            Ok(Err(_)) => break,
            Err(_) => break,
        }
    }
    seen
}

fn announced_growth_actions(
    frames: &[permagent::events::PermagentEvent],
    project_id: &str,
) -> bool {
    frames.iter().any(|f| {
        f.event_type == permagent::events::PermagentEventType::ProjectChanged
            && f.payload["project_id"] == project_id
            && f.payload["change"] == "growth_actions"
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn verifying_an_action_files_it_with_its_commit_and_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let app = permagent_daemon::routes::growth_actions::routes(state.clone());
    let pool = state.session_manager().pool_clone().await.unwrap();

    let repo = tmp.path().join("grocerysaver");
    std::fs::create_dir_all(&repo).unwrap();

    let project = projects::create_project(
        &pool,
        CreateProject {
            name: "GrocerySaver".to_string(),
            root_path: Some(repo.to_string_lossy().to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    // The action is seeded BEFORE the commit exists: `verify_git` searches
    // `--since=created_at`, so a commit made first would not be found and the
    // test would prove nothing about the targeted branch.
    let action = growth_store::upsert_suggested(&pool, &project.id, &seed("FAQ schema"))
        .await
        .unwrap();
    assert_eq!(action.status, growth_store::STATUS_SUGGESTED);
    assert!(action.verified_commit.is_none());

    // A verified action must not be findable before it is verified. This is the
    // baseline the "it moved" assertion below is measured against — without it,
    // a board that put everything in `tracking` would pass.
    let board = body_json(
        app.clone()
            .oneshot(req(
                "GET",
                &format!("/api/projects/{}/growth-actions", project.id),
                None,
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        board["actions"].as_array().unwrap().len(),
        1,
        "a suggested action belongs in the list the user is deciding on"
    );
    assert!(board["tracking"].as_array().unwrap().is_empty());

    let sha = repo_with_a_matching_commit(&repo);

    let mut rx = permagent::events::subscribe();

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
    let verified = body_json(resp).await;
    assert_eq!(
        verified["verified"], true,
        "a commit touching the named path is exactly what this check exists to find: {verified}"
    );
    assert_eq!(verified["identity"]["status"], "verified");

    // (2) The receipt outlives the response that carried it.
    assert_eq!(
        verified["identity"]["verifiedCommit"], sha,
        "the card must name the commit that earned the verification, not just that one did"
    );
    let detail = verified["identity"]["verifiedDetail"]
        .as_str()
        .expect("a completed action shows the evidence the check gave");
    // `get(..8)` rather than `&sha[..8]`: a byte-index slice panics mid-char on
    // non-ASCII, and a test helper that can panic on its own input is a worse
    // failure report than the assertion it was helping to make.
    let short_sha = sha.get(..8).expect("a git sha is at least 8 chars");
    assert!(
        detail.contains(short_sha) && detail.contains("src/pages/index.astro"),
        "the stored sentence must be the check's own: {detail}"
    );

    let row = growth_store::get(&pool, &project.id, &action.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.verified_commit.as_deref(), Some(sha.as_str()));
    assert_eq!(
        row.verified_by.as_deref(),
        Some(growth_store::VERIFIED_BY_GIT)
    );
    assert!(row.verified_at.is_some());

    // (3) The announcement — the thing whose absence was the bug.
    let frames = drain(&mut rx).await;
    assert!(
        announced_growth_actions(&frames, &project.id),
        "verify must announce on the bus like every other writer, or no open \
         window ever learns the card moved; saw {} frame(s)",
        frames.len()
    );

    // (4) The card has left the suggestions.
    let board = body_json(
        app.clone()
            .oneshot(req(
                "GET",
                &format!("/api/projects/{}/growth-actions", project.id),
                None,
            ))
            .await
            .unwrap(),
    )
    .await;
    assert!(
        board["actions"].as_array().unwrap().is_empty(),
        "a verified action is not still asking for a decision: {board}"
    );
    let tracking = board["tracking"].as_array().unwrap();
    assert_eq!(tracking.len(), 1);
    assert_eq!(tracking[0]["identity"]["verifiedCommit"], sha);
    assert_eq!(
        tracking[0]["identity"]["verifiedDetail"].as_str(),
        Some(detail),
        "the board and the verify reply must show the SAME evidence — one \
         function builds both identities so they cannot drift"
    );

    // ── Reopen: the way back, while there is still a way back ──
    let mut rx = permagent::events::subscribe();
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            &format!(
                "/api/projects/{}/growth-actions/{}/reopen",
                project.id, action.id
            ),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let reopened = body_json(resp).await;
    assert_eq!(reopened["status"], growth_store::STATUS_SUGGESTED);
    assert!(
        reopened["verifiedAt"].is_null() && reopened["verifiedCommit"].is_null(),
        "a withdrawn claim must not keep the pivot the sweep measures from: {reopened}"
    );
    let frames = drain(&mut rx).await;
    assert!(
        announced_growth_actions(&frames, &project.id),
        "reopen moves the card between two lists, so it announces too"
    );

    let board = body_json(
        app.clone()
            .oneshot(req(
                "GET",
                &format!("/api/projects/{}/growth-actions", project.id),
                None,
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(board["actions"].as_array().unwrap().len(), 1);
    assert!(board["tracking"].as_array().unwrap().is_empty());

    // ── A judged action is history, not a draft ──
    let judged = growth_store::upsert_suggested(&pool, &project.id, &seed("Measured already"))
        .await
        .unwrap();
    growth_store::record_verification(
        &pool,
        &project.id,
        &judged.id,
        growth_store::VerificationEvidence::new(
            growth_store::VERIFIED_BY_GIT,
            "2026-08-12T00:00:00Z",
        ),
    )
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO growth_action_outcomes
            (action_id, window_days, before_json, after_json, delta_pct, verdict,
             rationale, confounders, judged_at)
         VALUES (?1, 28, '{}', '{}', 0.2, 'helped', 'fixture', NULL, '2026-09-10T00:00:00Z')",
    )
    .bind(&judged.id)
    .execute(&pool)
    .await
    .unwrap();

    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            &format!(
                "/api/projects/{}/growth-actions/{}/reopen",
                project.id, judged.id
            ),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::CONFLICT,
        "reopening a measured action would delete the before-window its verdict rests on"
    );
    let after = growth_store::get(&pool, &project.id, &judged.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after.verified_at.as_deref(),
        Some("2026-08-12T00:00:00Z"),
        "a refused reopen must not have written anything"
    );
}
