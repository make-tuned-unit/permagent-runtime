//! `GET /api/runs` — the "agents at work" roster.
//!
//! A unified, best-effort snapshot of everything running or recently active:
//! background workers (their live state), scheduled jobs (with their #694 run
//! status), and active agent sessions. Powers the Automate tab's run-visibility
//! surface. Read-only — control (interrupt / run-now) reuses the existing
//! `/schedule/{id}/kill` + `/schedule/{id}/run_now` routes from the frontend.
//!
//! Best-effort by construction: a failing source contributes no rows rather than
//! failing the whole roster, so the surface never blanks on one bad reader.

use crate::state::AppState;
use axum::{extract::State, routing::get, Json, Router};
use chrono::Utc;
use permagent::agents::platform_extensions::librarian_state::{self, LibrarianPhase};
use permagent::scheduler::ScheduleRunStatus;
use permagent::session::session_manager::SessionType;
use serde::Serialize;
use std::sync::Arc;

/// One row in the run roster.
#[derive(Serialize)]
pub struct RunItem {
    /// "worker" | "schedule" | "session".
    pub kind: &'static str,
    pub id: String,
    pub name: String,
    /// "working" | "idle" | "error" | "missed" | "paused".
    pub status: &'static str,
    /// Human-readable context (current task, cron/interval, etc.).
    pub detail: Option<String>,
    /// Last time this unit did something (RFC3339).
    pub last_activity: Option<String>,
    /// When the current run started, for an elapsed clock (RFC3339).
    pub started_at: Option<String>,
    /// Whether the frontend can offer an interrupt control for this row.
    pub interruptible: bool,
}

#[derive(Serialize)]
pub struct RunsResponse {
    pub runs: Vec<RunItem>,
}

async fn list_runs(State(state): State<Arc<AppState>>) -> Json<RunsResponse> {
    let mut runs: Vec<RunItem> = Vec::new();

    // ── Background workers ──
    // The Librarian exposes a live-state singleton; the other interval-loop
    // workers (scheduler/steward/initiative/watcher) have no readable snapshot
    // yet — surfacing their live state is a follow-up (noted in the PR). Their
    // *scheduled* activity already shows up as schedule rows below.
    {
        let ls = librarian_state::get_state();
        let status = if ls.error_message.is_some() {
            "error"
        } else if ls.retry_in_progress {
            "working"
        } else {
            match ls.phase {
                LibrarianPhase::Idle => "idle",
                _ => "working",
            }
        };
        runs.push(RunItem {
            kind: "worker",
            id: "librarian".to_string(),
            name: "Librarian".to_string(),
            status,
            detail: Some(ls.current_task),
            last_activity: None,
            started_at: ls.session_stats.batch_started_at.map(|t| t.to_rfc3339()),
            interruptible: false,
        });
    }

    // ── Scheduled jobs ──
    let jobs = state.scheduler().list_scheduled_jobs().await;
    for j in &jobs {
        let status = if j.paused {
            "paused"
        } else if j.currently_running {
            "working"
        } else {
            match &j.last_status {
                Some(ScheduleRunStatus::Error) => "error",
                Some(ScheduleRunStatus::Missed) => "missed",
                _ => "idle",
            }
        };
        let detail = if let Some(at) = j.at {
            Some(format!("once at {}", at.to_rfc3339()))
        } else if let Some(secs) = j.every_seconds {
            Some(format!("every {secs}s"))
        } else {
            Some(j.cron.clone())
        };
        runs.push(RunItem {
            kind: "schedule",
            id: j.id.clone(),
            name: schedule_run_name(j),
            status,
            detail,
            last_activity: j.last_run.map(|t| t.to_rfc3339()),
            started_at: j.process_start_time.map(|t| t.to_rfc3339()),
            interruptible: j.currently_running,
        });
    }

    // ── Active agent sessions ──
    // Same "active in the last 2 min" derive the HenryHUD uses (see
    // henry_status.rs), scoped to User/Scheduled sessions.
    let two_min_ago = Utc::now() - chrono::Duration::seconds(120);
    let sessions = state
        .session_manager()
        .list_session_summaries()
        .await
        .unwrap_or_default();
    for s in sessions.iter().filter(|s| {
        s.updated_at >= two_min_ago
            && matches!(s.session_type, SessionType::User | SessionType::Scheduled)
    }) {
        let name = if s.name.is_empty() || s.name == "New Chat" {
            format!("Session {}", s.id.get(..8).unwrap_or(&s.id))
        } else {
            s.name.clone()
        };
        runs.push(RunItem {
            kind: "session",
            id: s.id.clone(),
            name,
            status: "working",
            detail: None,
            last_activity: Some(s.updated_at.to_rfc3339()),
            started_at: Some(s.created_at.to_rfc3339()),
            interruptible: false,
        });
    }

    Json(RunsResponse { runs })
}

/// The roster label for a scheduled job.
///
/// The id, never `source`. `source` is the absolute path to the recipe YAML,
/// so the roster rendered every scheduled job as
/// `/Users/<name>/…/scheduled_recipes/git-steward.yaml` — a local filesystem
/// path, including the user's home directory, displayed in a surface people
/// screen-share and record. The recipe cards beside it already render
/// `display_name || id`; this agrees with them.
fn schedule_run_name(job: &permagent::scheduler::ScheduledJob) -> String {
    job.id.clone()
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/runs", get(list_runs))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job_at(source: &str) -> permagent::scheduler::ScheduledJob {
        permagent::scheduler::ScheduledJob {
            id: "git-steward".to_string(),
            source: source.to_string(),
            ..Default::default()
        }
    }

    /// The roster label must never be derived from a filesystem path. This is
    /// a disclosure property, not a cosmetic one: the Automate tab is a
    /// screen-shared and screen-recorded surface, and `source` carries the
    /// user's home directory.
    #[test]
    fn a_scheduled_run_is_labelled_by_id_never_by_its_path() {
        let job = job_at("/Users/someone/Documents/dev/scheduled_recipes/git-steward.yaml");
        let name = schedule_run_name(&job);
        assert_eq!(name, "git-steward");
        assert!(
            !name.contains('/'),
            "roster label {name:?} contains a path separator — it is rendering a filesystem path"
        );
        assert!(
            !name.contains("/Users/") && !name.contains("/home/"),
            "roster label {name:?} discloses a home directory"
        );
        assert!(
            !name.ends_with(".yaml"),
            "roster label {name:?} is a filename, not a name"
        );
    }
}
