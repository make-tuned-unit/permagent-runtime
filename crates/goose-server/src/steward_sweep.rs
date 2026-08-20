//! The Steward's git-health sweep loop (shape cloned from `strix.rs`).
//!
//! Every pass surveys ONE of the user's active projects (least-recently-scanned
//! first) read-only, and turns detections into exactly two kinds of output:
//!
//!  * **Proposals** — reapable worktrees (merged + clean + fully pushed) and
//!    deletable branches (merged + unprotected + not checked out) become
//!    Tier-2 (user-only) `risk_gate` decisions via
//!    `permagent::steward::hygiene::propose_repo_hygiene`. The sweep itself
//!    NEVER mutates a repository — the only mutation path is the Decision
//!    Inbox effect arm, which re-verifies every predicate at apply time.
//!  * **Alert-only** — failing CI and a dirty primary tree have no effect arm,
//!    so they become a briefing plus a repo-health board card, never a
//!    decision (no fake approve buttons).
//!
//! Honesty laws, inherited from the Guard's loop: no findings → silence, never
//! filler; `gh` being unavailable is a stated fact, not a pretend-clean CI.

use crate::state::AppState;
use permagent::projects::{self, Project, UpdateProject};
use permagent::steward::{self, git_health, hygiene};
use sqlx::{Pool, Sqlite};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

/// Config flag (`~/.permagent/config.yaml`). DEFAULT OFF — the sweep files
/// approval items and runs `gh`, so it must be an explicit opt-in.
pub const STEWARD_SCAN_ENABLED_KEY: &str = "steward_scan_enabled";
/// When this project was last swept (ISO-8601), for the one-per-sweep
/// rotation: the least-recently-scanned active project goes next.
const LAST_SCAN_KEY: &str = "steward_last_scan";
/// How often the loop wakes to check the flag and whether a sweep is due.
const CHECK_EVERY: Duration = Duration::from_secs(15 * 60);
/// Sweep cadence config key, in hours.
const SWEEP_HOURS_KEY: &str = "steward_sweep_hours";
/// Repo hygiene does not change four times a day. Clamped to [1h, 1 week].
const DEFAULT_SWEEP_HOURS: u64 = 24;
/// Let boot (and any in-flight goal work) settle before the first sweep.
const STARTUP_DELAY: Duration = Duration::from_secs(300);
/// The default bucket is not a project the Steward sweeps.
const PERSONAL_PROJECT_ID: &str = "00000000-0000-0000-0000-000000000001";

fn is_enabled() -> bool {
    permagent::config::Config::global()
        .get_param::<bool>(STEWARD_SCAN_ENABLED_KEY)
        .unwrap_or(false)
}

fn sweep_interval() -> Duration {
    let hours = permagent::config::Config::global()
        .get_param::<u64>(SWEEP_HOURS_KEY)
        .unwrap_or(DEFAULT_SWEEP_HOURS)
        .clamp(1, 168);
    Duration::from_secs(hours * 3600)
}

pub fn spawn(state: Arc<AppState>) {
    // The loop always spawns and re-reads the flag every tick, so flipping
    // `steward_scan_enabled` takes effect at the next tick — no daemon
    // restart, and no silently-absent loop either way.
    if is_enabled() {
        tracing::info!(
            target: "permagentd::steward",
            "Steward git-health sweep enabled — one repo per sweep, every {}h; detects only, \
             cleanup goes through user-only decisions",
            sweep_interval().as_secs() / 3600
        );
    } else {
        tracing::info!(
            target: "permagentd::steward",
            "Steward git-health sweep is off ({STEWARD_SCAN_ENABLED_KEY}=false) — loop idle \
             until enabled"
        );
    }
    tokio::spawn(async move {
        tokio::time::sleep(STARTUP_DELAY).await;
        let mut last_sweep: Option<tokio::time::Instant> = None;
        loop {
            let due = last_sweep.is_none_or(|t| t.elapsed() >= sweep_interval());
            if is_enabled() && due {
                last_sweep = Some(tokio::time::Instant::now());
                if let Err(e) = sweep_once(&state).await {
                    tracing::debug!(target: "permagentd::steward", "sweep skipped: {e}");
                }
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

/// The World shows the Steward working only while it genuinely is (same
/// honesty clamp as the Guard's loop — `agentStatus.ts`).
fn announce(state_label: &str) {
    permagent::events::emit(permagent::events::agent_state_changed(
        steward::SELF_KNOWLEDGE_FEATURE.id,
        steward::SELF_KNOWLEDGE_FEATURE.display_name,
        state_label,
    ));
}

async fn sweep_once(state: &Arc<AppState>) -> Result<(), String> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| e.to_string())?;
    let projects = projects::list_projects(&pool, Some("active")).await?;

    // ONE project per sweep, rotating least-recently-scanned first — same
    // shape as the Guard: one focused pass per interval, predictable cycle,
    // and a never-scanned project sorts first (empty stamp).
    let mut candidates: Vec<&Project> = projects
        .iter()
        .filter(|p| p.id != PERSONAL_PROJECT_ID && p.root_path.is_some())
        .collect();
    candidates.sort_by_key(|p| last_scan_stamp(p));
    let Some(project) = candidates.first().copied() else {
        return Ok(());
    };
    let root = PathBuf::from(project.root_path.clone().unwrap_or_default());

    announce("working");
    let health = git_health::collect_repo_health(&root).await;
    // The rotation advances on every attempt — survey, skip, or error — so
    // one broken project can never starve the rest of the cycle.
    if let Err(e) = stamp_last_scan(&pool, project).await {
        tracing::warn!(target: "permagentd::steward", "last-scan stamp failed: {e}");
    }
    // Growth actions the generator keeps reprinting because it never looks
    // at the checkout. The Steward does: if the change is already in this
    // repo, the card comes off the board so Review cannot propose it again.
    let dismissed = crate::routes::growth_verify::dismiss_already_present(&pool, project).await;
    if !dismissed.is_empty() {
        tracing::info!(
            target: "permagentd::steward",
            dismissed = dismissed.len(),
            project = %project.name,
            "dismissed growth actions already present in the repo"
        );
        permagent::events::emit(permagent::events::project_changed(
            &project.id,
            "growth_actions",
        ));
    }
    let Some(health) = health else {
        // A non-repo root is a stated fact, not a degraded pretend-survey.
        tracing::info!(
            target: "permagentd::steward",
            project = %project.name,
            root = %root.display(),
            "root is not a readable git repository — nothing surveyed"
        );
        announce("available");
        return Ok(());
    };

    // ── Proposals: detections WITH an effect arm ──
    let mut proposed = 0usize;
    for w in health.worktrees.iter().filter(|w| git_health::reapable(w)) {
        let name = Path::new(&w.entry.path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| w.entry.path.clone());
        let mut evidence = vec![
            format!("worktree: {}", w.entry.path),
            format!(
                "branch: {}",
                w.entry.branch.as_deref().unwrap_or("(detached)")
            ),
            format!("sha: {}", w.entry.head.as_deref().unwrap_or("unknown")),
            "working tree: clean; all commits on a remote".to_string(),
        ];
        if let Some(via) = &w.merged_via {
            evidence.push(format!("merged via: {via}"));
        }
        match hygiene::propose_repo_hygiene(
            &pool,
            hygiene::RepoHygieneProposal {
                action_class: hygiene::ACTION_REPO_WORKTREE_REAP.to_string(),
                repo_path: health.repo_path.clone(),
                worktree_path: Some(w.entry.path.clone()),
                branch: w.entry.branch.clone(),
                evidence,
                headline: format!("Tidy up: remove the finished worktree {name}?"),
                project_id: Some(project.id.clone()),
            },
        )
        .await
        {
            Ok(Some(_)) => proposed += 1,
            Ok(None) => {} // anti-nag: open or previously-rejected — silence
            Err(e) => tracing::warn!(
                target: "permagentd::steward",
                "worktree-reap proposal failed to file: {e}"
            ),
        }
    }
    for b in health.branches.iter().filter(|b| git_health::deletable(b)) {
        let mut evidence = vec![
            format!("branch: {}", b.branch.name),
            format!("sha: {}", b.branch.sha),
            format!("last commit: {}", b.branch.committer_date),
        ];
        if let Some(via) = &b.merged_via {
            evidence.push(format!("merged via: {via}"));
        }
        match hygiene::propose_repo_hygiene(
            &pool,
            hygiene::RepoHygieneProposal {
                action_class: hygiene::ACTION_REPO_BRANCH_DELETE.to_string(),
                repo_path: health.repo_path.clone(),
                worktree_path: None,
                branch: Some(b.branch.name.clone()),
                evidence,
                headline: format!("Tidy up: delete the merged branch {}?", b.branch.name),
                project_id: Some(project.id.clone()),
            },
        )
        .await
        {
            Ok(Some(_)) => proposed += 1,
            Ok(None) => {}
            Err(e) => tracing::warn!(
                target: "permagentd::steward",
                "branch-delete proposal failed to file: {e}"
            ),
        }
    }

    // ── Alert-only: detections WITHOUT an effect arm. A briefing + a
    //    repo-health card — NEVER a decision (no fake approve buttons). ──
    let failing_ci = failing_ci_runs(&root).await;
    let mut flags: Vec<String> = Vec::new();
    if health.dirty_primary {
        flags.push("primary working tree has uncommitted changes".to_string());
    }
    if let Some(runs) = &failing_ci {
        for r in runs {
            flags.push(format!("failing CI: {r}"));
        }
    }
    if !flags.is_empty() {
        let summary = flags.join("; ");
        permagent::briefings::file_briefing(
            &pool,
            permagent::briefings::NewBriefing {
                from_agent: "steward".to_string(),
                kind: "repo_health_alert".to_string(),
                severity: permagent::briefings::Severity::Attention,
                summary: format!("{}: {}", project.name, summary),
                detail: Some(format!(
                    "Repo: {}\nAlert-only findings (nothing was changed, nothing is proposed):\n- {}",
                    health.repo_path,
                    flags.join("\n- ")
                )),
                ref_kind: None,
                ref_id: None,
            },
        )
        .await;
        if let Err(e) = steward::surface_repo_health_report(
            &pool,
            steward::RepoHealthReport {
                repo_path: health.repo_path.clone(),
                summary,
                stale_merged_branches: Vec::new(),
                orphaned_worktrees: Vec::new(),
                health_flags: flags,
                project_id: Some(project.id.clone()),
            },
        )
        .await
        {
            tracing::warn!(
                target: "permagentd::steward",
                "repo-health card failed to save: {e}"
            );
        }
    }

    tracing::info!(
        target: "permagentd::steward",
        project = %project.name,
        proposed,
        "git-health sweep complete — next sweep takes the next least-recently-scanned project"
    );
    announce("available");
    Ok(())
}

/// Recent failing CI runs via `gh run list --limit 3`. `None` when gh is
/// unavailable/unauthenticated or the remote isn't GitHub — a stated absence,
/// never a pretend-clean result. `Some(vec![])` = checked and green.
async fn failing_ci_runs(repo: &Path) -> Option<Vec<String>> {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.current_dir(repo)
        .args([
            "run",
            "list",
            "--limit",
            "3",
            "--json",
            "conclusion,workflowName,headBranch",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let out = cmd.output().await.ok()?;
    if !out.status.success() {
        tracing::info!(
            target: "permagentd::steward",
            repo = %repo.display(),
            "gh unavailable for this repo — CI state not checked (stated, not pretended clean)"
        );
        return None;
    }
    let runs: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    Some(
        runs.as_array()?
            .iter()
            .filter(|r| r.get("conclusion").and_then(|c| c.as_str()) == Some("failure"))
            .map(|r| {
                format!(
                    "{} on {}",
                    r.get("workflowName")
                        .and_then(|v| v.as_str())
                        .unwrap_or("workflow"),
                    r.get("headBranch").and_then(|v| v.as_str()).unwrap_or("?")
                )
            })
            .collect(),
    )
}

/// The project's last-sweep stamp, empty if never swept (sorts first).
fn last_scan_stamp(project: &Project) -> String {
    project
        .metadata_json
        .as_object()
        .and_then(|m| m.get(LAST_SCAN_KEY))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

/// Advance the rotation. Re-reads the project first (same reason as the
/// Guard's `stamp_last_scan`): the snapshot predates a survey that can take
/// minutes, and `update_project` replaces `metadata_json` wholesale.
async fn stamp_last_scan(pool: &Pool<Sqlite>, project: &Project) -> Result<(), String> {
    let fresh = projects::get_project_by_id_or_slug(pool, &project.id)
        .await
        .ok()
        .flatten();
    let mut meta = fresh
        .as_ref()
        .unwrap_or(project)
        .metadata_json
        .as_object()
        .cloned()
        .unwrap_or_default();
    meta.insert(
        LAST_SCAN_KEY.to_string(),
        serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
    );
    projects::update_project(
        pool,
        &project.id,
        UpdateProject {
            metadata_json: Some(serde_json::Value::Object(meta)),
            ..Default::default()
        },
    )
    .await
    .map(|_| ())
}
