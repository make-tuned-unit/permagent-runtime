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

use crate::agent_pass::{record_pass, Pass};
use crate::state::AppState;
use permagent::agent_runs::Trigger;
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
                if let Err(e) = sweep_once(&state, Trigger::Interval).await {
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

/// One sweep, recorded.
///
/// The pass itself lives in [`sweep_pass`]; this wrapper exists so that exactly
/// ONE run row is written per invocation, whichever of the pass's exits it
/// takes, with `started_at` stamped before any work. Same shape as the Guard's
/// loop, deliberately — the two surfaces have to be readable side by side.
///
/// A pass that cannot get a pool records nothing and returns the error it
/// always has: there is nowhere to write the row, and inventing one would be
/// the opposite of what this record is for.
async fn sweep_once(state: &Arc<AppState>, trigger: Trigger) -> Result<(), String> {
    let started_at = chrono::Utc::now();
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| e.to_string())?;
    let pass = sweep_pass(&pool).await;
    let _ = record_pass(
        &pool,
        steward::SELF_KNOWLEDGE_FEATURE.id,
        trigger,
        started_at,
        &pass.outcome,
    )
    .await;
    pass.result
}

/// Run the Steward's git-health sweep now, because a person asked.
///
/// Called by `POST /api/agents/{id}/run`, which reports the run row this pass
/// records rather than its own account of what happened.
///
/// The same body, the same gate, the same refusals as the interval pass — only
/// the recorded trigger differs, so a manual run lands in the same history
/// rather than a parallel one. The pass body re-checks `steward_scan_enabled`
/// for this caller's sake: `spawn` is what keeps a switched-off sweep from
/// recording a skip every fifteen minutes, and a manual press has no `spawn`
/// above it.
pub async fn run_pass_now(state: &Arc<AppState>) -> Result<(), String> {
    sweep_once(state, Trigger::Manual).await
}

/// The Steward's actual pass.
///
/// Returns a [`Pass`] rather than returning early so `sweep_once` can record
/// whichever way it went. Every `return` below used to be a bare `Ok(())`: a
/// sweep that found a non-repo root, or had no repo to survey at all, left the
/// same trace as a sweep that never happened.
async fn sweep_pass(pool: &Pool<Sqlite>) -> Pass {
    // Unreachable from the interval loop, which checks the same flag in `spawn`
    // before calling at all; this is the gate for a MANUAL pass. `Err` so the
    // person who pressed the button is told why nothing happened.
    if !is_enabled() {
        let reason = format!("the git-health sweep is off ({STEWARD_SCAN_ENABLED_KEY}=false)");
        return Pass::skipped(reason.clone()).returning(Err(reason));
    }

    let projects = match projects::list_projects(pool, Some("active")).await {
        Ok(projects) => projects,
        // Not a skip: the sweep could not read its own worklist.
        Err(e) => return Pass::failed(None, e),
    };

    let project = match choose_target(&projects) {
        Ok(project) => project,
        Err(reason) => return Pass::skipped(reason),
    };
    let root = PathBuf::from(project.root_path.clone().unwrap_or_default());

    announce("working");
    let health = git_health::collect_repo_health(&root).await;
    // The rotation advances on every attempt — survey, skip, or error — so
    // one broken project can never starve the rest of the cycle.
    if let Err(e) = stamp_last_scan(pool, project).await {
        tracing::warn!(target: "permagentd::steward", "last-scan stamp failed: {e}");
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
        return Pass::skipped(not_a_repo_skip_reason(&project.name));
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
            pool,
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
            pool,
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
    // Whether the briefing actually landed, not whether we tried to file one:
    // the run row's `produced` line is a claim that something exists to go and
    // read, so it has to be answerable by the same call that would have made it.
    let mut briefed = false;
    if !flags.is_empty() {
        let summary = flags.join("; ");
        briefed = permagent::briefings::file_briefing(
            pool,
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
        .await
        .is_some();
        if let Err(e) = steward::surface_repo_health_report(
            pool,
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
    // One repo per sweep by rotation, so `examined` is 1 — what this pass
    // actually surveyed, not the size of the fleet it is working through.
    Pass::completed(Some(1), produced_line(proposed, briefed))
}

/// Pick the repo this sweep surveys, or say why there is nothing to survey.
///
/// Pure over the project list for the same reason as the Guard's: this refusal
/// used to be a bare `return Ok(())`, so "no project has a root path" and "the
/// Steward swept and everything was tidy" were the same silence from outside.
fn choose_target(projects: &[Project]) -> Result<&Project, String> {
    // ONE project per sweep, rotating least-recently-scanned first — same
    // shape as the Guard: one focused pass per interval, predictable cycle,
    // and a never-scanned project sorts first (empty stamp).
    let mut candidates: Vec<&Project> = projects
        .iter()
        .filter(|p| p.id != PERSONAL_PROJECT_ID && p.root_path.is_some())
        .collect();
    candidates.sort_by_key(|p| last_scan_stamp(p));
    candidates.first().copied().ok_or_else(|| {
        "no active project outside the default Personal bucket has a root path to survey"
            .to_string()
    })
}

/// The skip reason a root that is not a git repository is recorded under —
/// the loop's own words, in one place so the log line and the row agree.
fn not_a_repo_skip_reason(project_name: &str) -> String {
    format!("{project_name}: root is not a readable git repository — nothing surveyed")
}

/// The one-line `produced` summary for a completed sweep.
///
/// `None` when the repo was tidy and green, which is the normal healthy result:
/// the pass still records an `ok` row, it just has nothing to point at. Only
/// outputs that actually landed are named — a proposal the anti-nag gate
/// suppressed was not produced, and neither was a briefing that failed to file.
fn produced_line(proposed: usize, briefed: bool) -> Option<String> {
    let mut parts = Vec::new();
    if proposed > 0 {
        parts.push(format!("{proposed} repo-hygiene decision(s) filed"));
    }
    if briefed {
        parts.push("1 repo-health briefing filed".to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::project_fixture;
    use permagent::agent_runs::{recent_for_agent, Outcome};

    // COVERED here: the rotation choice and its refusal, the not-a-repo skip
    // wording, the `produced` line, and the fact that each lands in an
    // `agent_runs` row under `git_steward` with the trigger it was given.
    //
    // NOT covered: `sweep_pass` end-to-end. It needs an `AppState`, a real git
    // repository with worktrees and merged branches, and `gh` authenticated
    // against a GitHub remote — a test of it would assert the developer's
    // machine. The seams below are the parts that decide what the row says.

    async fn runs_pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        permagent::session::spectral_schema::apply_agent_runs_schema(&pool)
            .await
            .unwrap();
        pool
    }

    fn started() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(1_760_000_000, 0).unwrap()
    }

    /// Metadata for a project the Steward has already swept, keyed off the
    /// production constant so a rename cannot leave this test passing against
    /// a key nothing writes.
    fn swept_at(stamp: &str) -> serde_json::Value {
        let mut meta = serde_json::Map::new();
        meta.insert(
            LAST_SCAN_KEY.to_string(),
            serde_json::Value::String(stamp.to_string()),
        );
        serde_json::Value::Object(meta)
    }

    /// The row the run record exists for: a sweep that surveyed a repo and
    /// found it tidy and green writes nothing anywhere else in the system, so
    /// without this row it looks exactly like a sweep that never ran.
    #[tokio::test]
    async fn a_sweep_that_found_a_tidy_repo_still_writes_one_ok_run_row() {
        let pool = runs_pool().await;
        let pass = Pass::completed(Some(1), produced_line(0, false));
        assert!(pass.result.is_ok());
        let _ = record_pass(
            &pool,
            steward::SELF_KNOWLEDGE_FEATURE.id,
            Trigger::Interval,
            started(),
            &pass.outcome,
        )
        .await;

        let runs = recent_for_agent(&pool, steward::SELF_KNOWLEDGE_FEATURE.id, 10)
            .await
            .unwrap();
        assert_eq!(runs.len(), 1, "exactly one row per pass");
        assert_eq!(runs[0].outcome, Outcome::Ok);
        assert_eq!(
            runs[0].examined,
            Some(1),
            "the rotation surveys one repo per sweep, and that is what it examined"
        );
        assert_eq!(
            runs[0].produced, None,
            "a tidy repo produced nothing and must not claim otherwise"
        );
    }

    /// A fleet with nothing to survey is not a Steward that found everything
    /// clean — the row has to be able to tell those apart.
    #[test]
    fn a_fleet_with_nothing_to_survey_says_so_rather_than_going_quiet() {
        assert_eq!(
            choose_target(&[]).unwrap_err(),
            "no active project outside the default Personal bucket has a root path to survey"
        );

        let personal_only = vec![project_fixture(
            PERSONAL_PROJECT_ID,
            "Personal",
            Some("/tmp/personal"),
            serde_json::json!({}),
        )];
        assert!(choose_target(&personal_only).is_err());

        let rootless = vec![project_fixture("p1", "Notes", None, serde_json::json!({}))];
        assert!(choose_target(&rootless).is_err());
    }

    /// The rotation itself, pinned: least-recently-swept first, a never-swept
    /// project ahead of every swept one.
    #[test]
    fn the_least_recently_swept_project_goes_next() {
        let projects = vec![
            project_fixture(
                "a",
                "Alpha",
                Some("/tmp/a"),
                swept_at("2026-08-10T00:00:00Z"),
            ),
            project_fixture("b", "Beta", Some("/tmp/b"), serde_json::json!({})),
            project_fixture(
                "c",
                "Gamma",
                Some("/tmp/c"),
                swept_at("2026-08-01T00:00:00Z"),
            ),
        ];
        assert_eq!(
            choose_target(&projects).unwrap().id,
            "b",
            "never swept goes first"
        );
    }

    /// A skip row is only worth having if it carries the reason the sweep
    /// actually had — so the reason comes from the production helper rather
    /// than being retyped here.
    #[tokio::test]
    async fn a_skipped_sweep_records_the_reason_the_sweep_actually_had() {
        let pool = runs_pool().await;
        let reason = choose_target(&[]).unwrap_err();
        let _ = record_pass(
            &pool,
            steward::SELF_KNOWLEDGE_FEATURE.id,
            Trigger::Interval,
            started(),
            &Pass::skipped(reason.clone()).outcome,
        )
        .await;

        let runs = recent_for_agent(&pool, steward::SELF_KNOWLEDGE_FEATURE.id, 10)
            .await
            .unwrap();
        assert_eq!(runs[0].outcome, Outcome::Skipped);
        assert_eq!(runs[0].reason.as_deref(), Some(reason.as_str()));
        assert_eq!(
            runs[0].examined, None,
            "a pass that declined to survey examined nothing — never a fabricated 0"
        );
    }

    /// A root that is not a git repository is the Steward's most common skip,
    /// and the row has to name the project or the reader cannot act on it.
    #[tokio::test]
    async fn a_root_that_is_not_a_repository_is_recorded_as_a_named_skip() {
        let pool = runs_pool().await;
        let reason = not_a_repo_skip_reason("Atlas");
        assert!(reason.starts_with("Atlas:"), "name the project: {reason}");
        let _ = record_pass(
            &pool,
            steward::SELF_KNOWLEDGE_FEATURE.id,
            Trigger::Interval,
            started(),
            &Pass::skipped(reason.clone()).outcome,
        )
        .await;

        let runs = recent_for_agent(&pool, steward::SELF_KNOWLEDGE_FEATURE.id, 10)
            .await
            .unwrap();
        assert_eq!(
            runs[0].outcome,
            Outcome::Skipped,
            "a non-repo root is the sweep working as designed, not a failure"
        );
        assert_eq!(runs[0].reason.as_deref(), Some(reason.as_str()));
    }

    /// `produced` names only what actually landed. A proposal the anti-nag gate
    /// suppressed and a briefing that failed to file both produced nothing.
    #[test]
    fn produced_names_only_the_outputs_that_landed() {
        assert_eq!(produced_line(0, false), None);
        assert_eq!(
            produced_line(2, false).unwrap(),
            "2 repo-hygiene decision(s) filed"
        );
        assert_eq!(
            produced_line(0, true).unwrap(),
            "1 repo-health briefing filed"
        );
        let both = produced_line(1, true).unwrap();
        assert!(
            both.contains("decision(s)") && both.contains("briefing"),
            "{both}"
        );
    }

    /// A manual run and a scheduled one land in the same history, each labelled
    /// for what it was. (`run_pass_now` binds `Trigger::Manual` and `spawn`
    /// binds `Trigger::Interval` in one line each; what is testable without an
    /// `AppState` is that the trigger handed to `record_pass` survives the
    /// round trip, which is what those two lines depend on.)
    #[tokio::test]
    async fn a_manual_pass_is_recorded_as_manual_beside_the_scheduled_ones() {
        let pool = runs_pool().await;
        let _ = record_pass(
            &pool,
            steward::SELF_KNOWLEDGE_FEATURE.id,
            Trigger::Interval,
            started(),
            &Pass::completed(Some(1), None).outcome,
        )
        .await;
        let _ = record_pass(
            &pool,
            steward::SELF_KNOWLEDGE_FEATURE.id,
            Trigger::Manual,
            started() + chrono::Duration::seconds(30),
            &Pass::completed(Some(1), None).outcome,
        )
        .await;

        let runs = recent_for_agent(&pool, steward::SELF_KNOWLEDGE_FEATURE.id, 10)
            .await
            .unwrap();
        assert_eq!(runs.len(), 2, "one history, not two");
        assert_eq!(runs[0].trigger, Trigger::Manual);
        assert_eq!(runs[1].trigger, Trigger::Interval);
    }
}
