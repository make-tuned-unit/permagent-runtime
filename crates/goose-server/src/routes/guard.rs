//! The Guard's fix loop: findings are sent to a coding agent, the fix is
//! verified from the project's own git history, and a rescan confirms it.
//!
//! The lifecycle (statuses, transitions, reconcile) lives in `crate::strix`;
//! this module is the HTTP surface and the git evidence for "verify".
//!
//! Honesty rules, same as the Grow actions' verify:
//!   * a verify that cannot read the repo (no root path, not a git repo) is a
//!     FAILED CHECK with a plain reason — never a 500, and never a pass;
//!   * a pass names the commit and what it touched, so the person can audit it;
//!   * a verified commit is only `fix_committed`. The finding becomes `fixed`
//!     only when a later completed scan no longer reports it.

use crate::routes::analytics_verify::Check;
use crate::routes::growth_actions::ApiError;
use crate::routes::growth_verify::{fail, git, parse_commits, pass};
use crate::state::AppState;
use crate::strix::{self as guard, Finding, StoreError};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use permagent::projects::{self, Project};
use serde::Deserialize;
use sqlx::{Pool, Sqlite};
use std::sync::Arc;

type ApiResult<T> = Result<Json<T>, ApiError>;

fn internal(e: impl ToString) -> ApiError {
    ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn store_error(e: StoreError) -> ApiError {
    match e {
        StoreError::NotFound => ApiError(StatusCode::NOT_FOUND, "unknown finding".to_string()),
        StoreError::Rejected(message) => ApiError(StatusCode::BAD_REQUEST, message),
        StoreError::Failed(message) => internal(message),
    }
}

async fn load(
    state: &Arc<AppState>,
    project_id: &str,
) -> Result<(Pool<Sqlite>, Project), ApiError> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(internal)?;
    let project = projects::get_project_by_id_or_slug(&pool, project_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "unknown project".to_string()))?;
    Ok((pool, project))
}

fn find(project: &Project, finding_id: &str) -> Result<Finding, ApiError> {
    guard::stored_findings(&project.metadata_json)
        .into_iter()
        .find(|f| f.id == finding_id)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "unknown finding".to_string()))
}

// ── Routes ─────────────────────────────────────────────────────────────

async fn get_guard(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
) -> ApiResult<serde_json::Value> {
    let (_pool, project) = load(&state, &project_id).await?;
    let mut findings = guard::stored_findings(&project.metadata_json);
    let counts = guard::counts(&findings);
    guard::display_order(&mut findings);
    let stamps = guard::stamps(&project.metadata_json);
    Ok(Json(serde_json::json!({
        "findings": findings,
        "counts": counts,
        "last_scan": stamps.last_scan,
        "last_attempt": stamps.last_attempt,
        "rescan_requested_at": stamps.rescan_requested_at,
        "enabled": permagent::strix::is_enabled(),
    })))
}

#[derive(Deserialize)]
struct SentBody {
    agent: String,
}

async fn mark_sent(
    State(state): State<Arc<AppState>>,
    Path((project_id, finding_id)): Path<(String, String)>,
    Json(body): Json<SentBody>,
) -> ApiResult<serde_json::Value> {
    let (pool, project) = load(&state, &project_id).await?;
    let now = chrono::Utc::now().to_rfc3339();
    let finding = guard::mutate_finding(&pool, &project, &finding_id, |f| {
        guard::apply_sent(f, &body.agent, &now)
    })
    .await
    .map_err(store_error)?;
    Ok(Json(serde_json::json!({ "finding": finding })))
}

#[derive(Deserialize)]
struct StatusBody {
    status: String,
    #[serde(default)]
    reason: Option<String>,
}

async fn set_status(
    State(state): State<Arc<AppState>>,
    Path((project_id, finding_id)): Path<(String, String)>,
    Json(body): Json<StatusBody>,
) -> ApiResult<serde_json::Value> {
    let (pool, project) = load(&state, &project_id).await?;
    let now = chrono::Utc::now().to_rfc3339();
    let finding = guard::mutate_finding(&pool, &project, &finding_id, |f| {
        guard::apply_status(f, &body.status, body.reason.as_deref(), &now)
    })
    .await
    .map_err(store_error)?;
    Ok(Json(serde_json::json!({ "finding": finding })))
}

async fn verify_finding(
    State(state): State<Arc<AppState>>,
    Path((project_id, finding_id)): Path<(String, String)>,
) -> ApiResult<serde_json::Value> {
    let (pool, project) = load(&state, &project_id).await?;
    let current = find(&project, &finding_id)?;
    if !current.status.is_active() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "This finding is already closed. Reopen it before verifying a fix.".to_string(),
        ));
    }

    // The git reads can be slow, so they run outside the project's lock; the
    // status change below re-validates under it.
    let evidence = check_fix(&project, &current).await;

    let mut finding = current;
    let mut rescan_requested = false;
    let verified = evidence.commit.is_some();
    if let Some((sha, subject)) = evidence.commit.as_ref() {
        let now = chrono::Utc::now().to_rfc3339();
        finding = guard::mutate_finding(&pool, &project, &finding_id, |f| {
            guard::apply_verified(f, sha, subject, &evidence.detail, &now)
        })
        .await
        .map_err(store_error)?;
        // A verified fix wants a confirming scan. The request can be refused
        // (rescanned recently, Guard off); that is reported, not an error.
        rescan_requested = match guard::request_rescan(&pool, &project).await {
            Ok(outcome) => outcome.queued,
            Err(e) => {
                tracing::warn!(target: "permagentd::strix", "rescan request failed: {e}");
                false
            }
        };
    }

    Ok(Json(serde_json::json!({
        "verified": verified,
        "finding": finding,
        "detail": evidence.detail,
        "commit": evidence.commit.as_ref().map(|(sha, _)| sha),
        "checks": evidence.checks,
        "rescan_requested": rescan_requested,
    })))
}

async fn rescan(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
) -> ApiResult<serde_json::Value> {
    let (pool, project) = load(&state, &project_id).await?;
    let outcome = guard::request_rescan(&pool, &project)
        .await
        .map_err(internal)?;
    Ok(Json(serde_json::json!({
        "queued": outcome.queued,
        "requested_at": outcome.requested_at,
        "reason": outcome.reason,
    })))
}

// ── Verify: evidence from git ──────────────────────────────────────────

const LOCATION_ID: &str = "git_location";
const LOCATION_LABEL: &str = "A commit since the fix was sent changed the finding's file";
const MESSAGE_ID: &str = "git_message";
const MESSAGE_LABEL: &str = "A commit message names this finding";
const REPO_ID: &str = "git_repo";
const REPO_LABEL: &str = "The project's repo can be read";

/// What the git checks concluded.
struct Evidence {
    checks: Vec<Check>,
    /// Full sha and subject of the commit that earned the pass.
    commit: Option<(String, String)>,
    /// One sentence for the finding card: the passing check's own words, or
    /// why nothing passed.
    detail: String,
}

impl Evidence {
    fn unreadable(detail: String) -> Self {
        Evidence {
            detail: detail.clone(),
            checks: vec![fail(REPO_ID, REPO_LABEL, detail)],
            commit: None,
        }
    }
}

/// `git --since` as a raw Unix timestamp (`@<seconds>`). git's calendar
/// parser silently IGNORES dates from year 2100 on (every commit then counts),
/// and RFC 3339 forms vary across versions; `@<seconds>` is exact. A stamp that
/// cannot be parsed falls back to now, so nothing is verified by accident.
fn git_since(stamp: &str) -> String {
    let secs = chrono::DateTime::parse_from_rfc3339(stamp)
        .map(|t| t.timestamp())
        .unwrap_or_else(|_| chrono::Utc::now().timestamp());
    format!("@{secs}")
}

/// The finding's file as a repo-relative path: `file://` and a leading `./`
/// removed, a `:line` or `#L…` suffix dropped, and an absolute path under the
/// project root made relative to it. None when nothing is left.
fn normalize_location(location: &str, root: Option<&str>) -> Option<String> {
    let mut loc = location.trim();
    loc = loc.strip_prefix("file://").unwrap_or(loc);
    if let Some((path, _fragment)) = loc.split_once('#') {
        loc = path;
    }
    // `path:12` or `path:12:3`
    let mut path = loc.to_string();
    for _ in 0..2 {
        if let Some((head, tail)) = path.rsplit_once(':') {
            if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) {
                path = head.to_string();
            }
        }
    }
    let mut path = path.as_str();
    while let Some(rest) = path.strip_prefix("./") {
        path = rest;
    }
    if let Some(root) = root
        .map(|r| r.trim_end_matches('/'))
        .filter(|r| !r.is_empty())
    {
        if let Some(rest) = path.strip_prefix(root).and_then(|r| r.strip_prefix('/')) {
            path = rest;
        }
    }
    (!path.is_empty()).then(|| path.to_string())
}

/// Does a changed file in a commit correspond to the finding's location?
/// Matched on whole path components, so `db.rs` does not match `mydb.rs`.
fn location_matches(changed: &str, wanted: &str) -> bool {
    changed == wanted
        || wanted.ends_with(&format!("/{changed}"))
        || changed.ends_with(&format!("/{wanted}"))
}

/// Does a commit message carry `Guard: <finding id>` on a line of its own
/// (optionally followed by more text after a space or comma)?
fn message_names_finding(message: &str, finding_id: &str) -> bool {
    let needle = format!("Guard: {finding_id}");
    message.lines().any(|line| {
        let line = line.trim();
        line.strip_prefix(&needle).is_some_and(|rest| {
            rest.is_empty() || rest.starts_with(|c: char| c.is_whitespace() || c == ',')
        })
    })
}

/// Run the checks. Never fails: anything unreadable is a failed check.
async fn check_fix(project: &Project, finding: &Finding) -> Evidence {
    let Some(root) = project
        .root_path
        .as_deref()
        .filter(|p| !p.trim().is_empty())
    else {
        return Evidence::unreadable(
            "This project has no root path, so there is no repo to read. Set the project's \
             local folder, or verify another way."
                .to_string(),
        );
    };
    let dir = std::path::Path::new(root);
    if git(dir, &["rev-parse", "--git-dir"]).await.is_none() {
        return Evidence::unreadable(format!(
            "{} is not a git repository, so commits cannot be read.",
            dir.display()
        ));
    }

    // Since the fix was sent; failing that, since the Guard first saw it.
    let since_stamp = finding
        .sent_at
        .as_deref()
        .or(finding.first_seen_at.as_deref())
        .unwrap_or(&finding.found_at);
    let since_arg = format!("--since={}", git_since(since_stamp));

    let mut checks = Vec::new();
    let mut commit: Option<(String, String)> = None;
    let mut detail: Option<String> = None;

    // Check 1: a commit touched the finding's file.
    match finding
        .location
        .as_deref()
        .and_then(|l| normalize_location(l, Some(root)))
    {
        None => checks.push(fail(
            LOCATION_ID,
            LOCATION_LABEL,
            "This finding names no file, so a changed file cannot be matched to it.",
        )),
        Some(wanted) => {
            let log = git(
                dir,
                &["log", &since_arg, "--format=%x00%H%x1f%s", "--name-only"],
            )
            .await;
            match log {
                None => checks.push(fail(
                    LOCATION_ID,
                    LOCATION_LABEL,
                    "git log failed, so commits could not be read.",
                )),
                Some(raw) => {
                    let commits = parse_commits(&raw);
                    let hit = commits.iter().find_map(|c| {
                        c.files
                            .iter()
                            .find(|f| location_matches(f, &wanted))
                            .map(|f| (c, f.clone()))
                    });
                    match hit {
                        Some((c, file)) => {
                            let sentence = format!(
                                "Commit {} \"{}\" changed {file}, where the finding is.",
                                c.sha.get(..8).unwrap_or(&c.sha),
                                c.subject
                            );
                            commit = Some((c.sha.clone(), c.subject.clone()));
                            detail = Some(sentence.clone());
                            checks.push(pass(LOCATION_ID, LOCATION_LABEL, sentence));
                        }
                        None if commits.is_empty() => checks.push(fail(
                            LOCATION_ID,
                            LOCATION_LABEL,
                            format!("No commits in {} since {since_stamp}.", dir.display()),
                        )),
                        None => checks.push(fail(
                            LOCATION_ID,
                            LOCATION_LABEL,
                            format!(
                                "{} commit(s) landed since {since_stamp} but none changed {wanted}.",
                                commits.len()
                            ),
                        )),
                    }
                }
            }
        }
    }

    // Check 2: a commit message says it fixes this finding.
    let grep = format!("--grep=Guard: {}", finding.id);
    let log = git(
        dir,
        &[
            "log",
            &since_arg,
            "--fixed-strings",
            &grep,
            "--format=%x00%H%x1f%B",
        ],
    )
    .await;
    match log {
        None => checks.push(fail(
            MESSAGE_ID,
            MESSAGE_LABEL,
            "git log failed, so commit messages could not be read.",
        )),
        Some(raw) => {
            let named = raw.split('\u{0}').find_map(|record| {
                let (sha, message) = record.trim_matches('\n').split_once('\u{1f}')?;
                message_names_finding(message, &finding.id).then(|| {
                    (
                        sha.trim().to_string(),
                        message
                            .lines()
                            .next()
                            .unwrap_or_default()
                            .trim()
                            .to_string(),
                    )
                })
            });
            match named {
                Some((sha, subject)) => {
                    let sentence = format!(
                        "Commit {} \"{subject}\" says it fixes this finding (Guard: {}).",
                        sha.get(..8).unwrap_or(&sha),
                        finding.id
                    );
                    if commit.is_none() {
                        commit = Some((sha, subject));
                        detail = Some(sentence.clone());
                    }
                    checks.push(pass(MESSAGE_ID, MESSAGE_LABEL, sentence));
                }
                None => checks.push(fail(
                    MESSAGE_ID,
                    MESSAGE_LABEL,
                    format!(
                        "No commit message since {since_stamp} contains \"Guard: {}\".",
                        finding.id
                    ),
                )),
            }
        }
    }

    let detail = detail.unwrap_or_else(|| {
        checks
            .iter()
            .map(|c| c.detail.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    });
    Evidence {
        checks,
        commit,
        detail,
    }
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/projects/{project_id}/guard", get(get_guard))
        .route("/api/projects/{project_id}/guard/rescan", post(rescan))
        .route(
            "/api/projects/{project_id}/guard/findings/{finding_id}/sent",
            post(mark_sent),
        )
        .route(
            "/api/projects/{project_id}/guard/findings/{finding_id}/verify",
            post(verify_finding),
        )
        .route(
            "/api/projects/{project_id}/guard/findings/{finding_id}/status",
            post(set_status),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locations_are_normalised_to_repo_relative_paths() {
        assert_eq!(
            normalize_location("file:///repo/src/db.rs", Some("/repo")).as_deref(),
            Some("src/db.rs")
        );
        assert_eq!(
            normalize_location("./src/db.rs", None).as_deref(),
            Some("src/db.rs")
        );
        assert_eq!(
            normalize_location("././src/db.rs:42:7", None).as_deref(),
            Some("src/db.rs")
        );
        assert_eq!(
            normalize_location("src/db.rs#L10-L20", None).as_deref(),
            Some("src/db.rs")
        );
        assert_eq!(
            normalize_location("/repo/src/db.rs", Some("/repo/")).as_deref(),
            Some("src/db.rs")
        );
        // Not under the root: kept, and matched by suffix later.
        assert_eq!(
            normalize_location("/workspace/src/db.rs", Some("/repo")).as_deref(),
            Some("/workspace/src/db.rs")
        );
        assert_eq!(normalize_location("  ", None), None);
        assert_eq!(normalize_location("file://", None), None);
    }

    #[test]
    fn locations_match_on_whole_path_components() {
        assert!(location_matches("src/db.rs", "src/db.rs"));
        assert!(location_matches("apps/api/src/db.rs", "src/db.rs"));
        assert!(location_matches("src/db.rs", "/workspace/src/db.rs"));
        assert!(!location_matches("src/mydb.rs", "db.rs"));
        assert!(!location_matches("src/db.rs", "src/other.rs"));
    }

    #[test]
    fn a_commit_message_must_name_the_exact_finding() {
        let id = "CWE-89:src/db.rs";
        assert!(message_names_finding(
            "Fix it\n\nGuard: CWE-89:src/db.rs\n",
            id
        ));
        assert!(message_names_finding(
            "Fix it\n\nGuard: CWE-89:src/db.rs, CWE-79:x",
            id
        ));
        assert!(message_names_finding("  Guard: CWE-89:src/db.rs  ", id));
        // A longer id that merely starts with this one is a different finding.
        assert!(!message_names_finding("Guard: CWE-89:src/db.rs.bak", id));
        assert!(!message_names_finding("This mentions CWE-89:src/db.rs", id));
        assert!(!message_names_finding("", id));
    }

    #[test]
    fn since_is_an_exact_unix_timestamp_and_a_bad_stamp_means_now() {
        assert_eq!(git_since("2026-10-14T10:30:00+02:00"), "@1791966600");
        let now = chrono::Utc::now().timestamp();
        let got: i64 = git_since("not a date")[1..].parse().unwrap();
        assert!((got - now).abs() < 5);
    }

    #[test]
    fn store_errors_map_to_the_right_status() {
        assert_eq!(store_error(StoreError::NotFound).0, StatusCode::NOT_FOUND);
        assert_eq!(
            store_error(StoreError::Rejected("no".into())).0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            store_error(StoreError::Failed("db".into())).0,
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    fn sh(dir: &std::path::Path, cmd: &str) {
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "command failed: {cmd}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn project_at(root: Option<&std::path::Path>) -> Project {
        Project {
            id: "p1".into(),
            user_id: "u".into(),
            slug: "p".into(),
            name: "P".into(),
            description: String::new(),
            status: "active".into(),
            root_path: root.map(|r| r.display().to_string()),
            site_url: None,
            repo_url: None,
            notes: String::new(),
            metadata_json: serde_json::json!({}),
            graph_entity_id: None,
            tags: vec![],
            created_at: String::new(),
            updated_at: String::new(),
            last_opened_at: String::new(),
        }
    }

    fn finding_at(location: &str, sent_at: &str) -> Finding {
        Finding {
            id: "CWE-89:src/db.rs".into(),
            title: "SQL injection".into(),
            severity: "high".into(),
            location: Some(location.into()),
            found_at: "2000-01-01T00:00:00+00:00".into(),
            sent_at: Some(sent_at.into()),
            ..Default::default()
        }
    }

    const GIT: &str = "git -c user.name=t -c user.email=t@example.com";

    #[tokio::test]
    async fn no_root_path_is_a_failed_check_with_a_reason_not_a_pass() {
        let ev = check_fix(
            &project_at(None),
            &finding_at("src/db.rs", "2000-01-01T00:00:00Z"),
        )
        .await;
        assert!(ev.commit.is_none());
        assert_eq!(ev.checks.len(), 1);
        assert!(!ev.checks[0].passed);
        assert!(ev.detail.contains("no root path"), "{}", ev.detail);
    }

    #[tokio::test]
    async fn a_folder_that_is_not_a_repo_is_a_failed_check() {
        let tmp = tempfile::tempdir().unwrap();
        let ev = check_fix(
            &project_at(Some(tmp.path())),
            &finding_at("src/db.rs", "2000-01-01T00:00:00Z"),
        )
        .await;
        assert!(ev.commit.is_none());
        assert!(!ev.checks[0].passed);
        assert!(ev.detail.contains("not a git repository"), "{}", ev.detail);
    }

    #[tokio::test]
    async fn a_commit_touching_the_finding_file_verifies_it() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        sh(dir, "git init -q && mkdir -p src");
        sh(
            dir,
            "echo a > src/db.rs && echo a > other.txt && git add -A",
        );
        sh(dir, &format!("{GIT} commit -q -m 'Initial'"));
        sh(dir, "echo b > src/db.rs && git add -A");
        sh(
            dir,
            &format!("{GIT} commit -q -m 'Use parameterised queries'"),
        );

        // Location spelled the way a scanner might: file:// and a line number.
        let loc = format!("file://{}/src/db.rs:12", dir.display());
        let ev = check_fix(
            &project_at(Some(dir)),
            &finding_at(&loc, "2000-01-01T00:00:00Z"),
        )
        .await;
        let (sha, subject) = ev.commit.clone().expect("a commit touched the file");
        assert_eq!(sha.len(), 40);
        assert_eq!(subject, "Use parameterised queries");
        let loc_check = ev.checks.iter().find(|c| c.id == LOCATION_ID).unwrap();
        assert!(loc_check.passed);
        assert!(
            loc_check.detail.contains("src/db.rs"),
            "{}",
            loc_check.detail
        );
        let msg_check = ev.checks.iter().find(|c| c.id == MESSAGE_ID).unwrap();
        assert!(!msg_check.passed, "no commit message names the finding");
        assert_eq!(ev.detail, loc_check.detail);
    }

    #[tokio::test]
    async fn a_commit_elsewhere_does_not_verify_the_finding() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        sh(dir, "git init -q && echo a > other.txt && git add -A");
        sh(dir, &format!("{GIT} commit -q -m 'Unrelated'"));
        let ev = check_fix(
            &project_at(Some(dir)),
            &finding_at("src/db.rs", "2000-01-01T00:00:00Z"),
        )
        .await;
        assert!(ev.commit.is_none());
        assert!(ev.checks.iter().all(|c| !c.passed));
        assert!(
            ev.detail.contains("none changed src/db.rs"),
            "{}",
            ev.detail
        );
    }

    #[tokio::test]
    async fn a_guard_trailer_verifies_even_when_the_file_moved() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        sh(dir, "git init -q && echo a > moved.txt && git add -A");
        sh(
            dir,
            &format!("{GIT} commit -q -m 'Remove the query builder' -m 'Guard: CWE-89:src/db.rs'"),
        );
        let ev = check_fix(
            &project_at(Some(dir)),
            &finding_at("src/db.rs", "2000-01-01T00:00:00Z"),
        )
        .await;
        let (_, subject) = ev.commit.clone().expect("the trailer names the finding");
        assert_eq!(subject, "Remove the query builder");
        assert!(
            ev.checks
                .iter()
                .find(|c| c.id == MESSAGE_ID)
                .unwrap()
                .passed
        );
        assert!(
            !ev.checks
                .iter()
                .find(|c| c.id == LOCATION_ID)
                .unwrap()
                .passed
        );
        assert!(
            ev.detail.contains("Guard: CWE-89:src/db.rs"),
            "{}",
            ev.detail
        );
    }

    #[tokio::test]
    async fn commits_before_the_fix_was_sent_do_not_count() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        sh(
            dir,
            "git init -q && mkdir src && echo a > src/db.rs && git add -A",
        );
        sh(dir, &format!("{GIT} commit -q -m 'Touches db'"));
        // Sent far in the future: every existing commit predates it.
        let ev = check_fix(
            &project_at(Some(dir)),
            &finding_at("src/db.rs", "2999-01-01T00:00:00Z"),
        )
        .await;
        assert!(ev.commit.is_none());
    }
}
