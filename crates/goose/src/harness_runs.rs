//! Read-only projection of coding-harness runs in the user's terminals, so a
//! chat session can answer "how is the work in my terminals going?".
//!
//! A terminal harness process holds one row in `harness_process_leases`
//! (written by the CLI lease writer, heartbeated every 15 s). This module
//! joins that row to the work it owns, all from the shared local database:
//!
//! - **Goals:** a goal card belongs to a run when its
//!   `metadata_json.dispatched_lifecycle` equals the lease's `lifecycle_id`,
//!   when its `created_by_session_id` is the run's coordinator session, or
//!   when its `worker_session_id` is a session parented (at any depth) to the
//!   coordinator.
//! - **Decisions:** open `decisions` rows whose `goal_id` is one of those goals.
//! - **Spend:** `cost_ledger` rows for the coordinator, its descendant
//!   sessions, and the worker sessions named on the run's goals.
//!
//! Discipline:
//! - **Read-only and zero model calls.** Nothing here writes, requeues,
//!   dispatches or reclaims, so it is safe to call while a terminal is mid-run.
//! - **Bounded.** Every list query carries a `LIMIT` ([`MAX_RUNS`],
//!   [`MAX_GOALS_PER_RUN`], [`MAX_JOURNAL_ROWS`], [`MAX_ATTENTION_ITEMS`]).
//! - **No message text, no file contents.** Worker activity is reduced to the
//!   most recent tool NAME and its timestamp; titles come from session names,
//!   card titles and decision headlines; free text (errors, reasons) is
//!   truncated by characters, never by bytes.
//! - **Local only.** Every function takes the pool it reads. The chat tools
//!   pass the process's own session pool, which lives under the process's
//!   `Paths` root — an eval run rooted elsewhere (`PERMAGENT_PATH_ROOT`) writes
//!   its leases to its own database and is never visible here.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use sqlx::{Pool, Row, Sqlite};

/// A lease whose heartbeat is at most this old (and whose phase is not
/// `exited`) is live; older is lost. One window shared with the ownership
/// check in [`crate::harness_lease`], so chat and goal recovery never disagree.
pub use crate::harness_lease::LEASE_LIVE_WINDOW_SECS;

/// Most runs any one listing returns.
pub const MAX_RUNS: i64 = 20;
/// Most goals one run's status lists.
pub const MAX_GOALS_PER_RUN: i64 = 50;
/// Most activity-journal rows one run's status lists.
pub const MAX_JOURNAL_ROWS: i64 = 20;
/// Most items the attention report lists.
pub const MAX_ATTENTION_ITEMS: usize = 50;
/// A lost (stale, never exited) lease stays visible this long after its last
/// heartbeat, so a terminal that died overnight is still reported the next
/// morning without keeping ancient crashes around forever.
pub const LOST_RUN_HORIZON_SECS: i64 = 72 * 3600;
/// Default look-back for runs that exited cleanly.
pub const DEFAULT_RECENT_ENDED: std::time::Duration = std::time::Duration::from_secs(30 * 60);
/// Look-back used when resolving an explicit run selector, so "the run in
/// alpha from this afternoon" still resolves after it exited.
pub const SELECTOR_ENDED_WINDOW: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

const TITLE_MAX_CHARS: usize = 100;
const LAST_ERROR_MAX_CHARS: usize = 200;
const REASON_MAX_CHARS: usize = 160;
const ACTIVITY_MAX_CHARS: usize = 120;
const HEADLINE_MAX_CHARS: usize = 80;
const SHORT_SHA_CHARS: usize = 12;
const RUN_HANDLE_CHARS: usize = 12;
/// How many of a worker's newest assistant messages are scanned for its last
/// tool request. Bounds the `json_each` walk.
const WORKER_MESSAGE_SCAN: i64 = 20;

/// Decision kinds that block a run on a gate rather than a review.
const GATE_DECISION_KINDS: &[&str] = &[
    "risk_gate",
    "session_gate",
    "tool_approval",
    "unblock",
    "choice",
    "capability_gap",
];

// ── Liveness ────────────────────────────────────────────────────────────

/// How a lease reads right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunLiveness {
    /// Not exited, heartbeat within [`LEASE_LIVE_WINDOW_SECS`].
    Live,
    /// Not exited, heartbeat stale: the terminal most likely died.
    Lost,
    /// The process recorded its own exit.
    Ended,
}

impl RunLiveness {
    pub fn label(self) -> &'static str {
        match self {
            RunLiveness::Live => "live",
            RunLiveness::Lost => "LOST",
            RunLiveness::Ended => "ended",
        }
    }
}

/// Classify one lease. An unparseable heartbeat is treated as stale (lost),
/// never as live: claiming a terminal is working on the strength of a
/// timestamp we could not read is worse than flagging it.
pub fn classify_lease(phase: &str, heartbeat_at: &str, now: DateTime<Utc>) -> RunLiveness {
    if phase == "exited" {
        return RunLiveness::Ended;
    }
    match parse_ts(heartbeat_at) {
        Some(beat) if (now - beat).num_seconds() <= LEASE_LIVE_WINDOW_SECS => RunLiveness::Live,
        _ => RunLiveness::Lost,
    }
}

/// RFC 3339, or SQLite's `CURRENT_TIMESTAMP` shape read as UTC.
fn parse_ts(raw: &str) -> Option<DateTime<Utc>> {
    let raw = raw.trim();
    if let Ok(ts) = DateTime::parse_from_rfc3339(raw) {
        return Some(ts.with_timezone(&Utc));
    }
    ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f"]
        .iter()
        .find_map(|fmt| chrono::NaiveDateTime::parse_from_str(raw, fmt).ok())
        .map(|naive| naive.and_utc())
}

fn age_secs(raw: &str, now: DateTime<Utc>) -> Option<i64> {
    parse_ts(raw).map(|ts| (now - ts).num_seconds().max(0))
}

fn rfc3339(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Truncate by characters (never bytes), marking the cut.
pub fn truncate_chars(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max).collect();
    out.push('…');
    out
}

fn short_handle(lease_id: &str) -> String {
    lease_id.chars().take(RUN_HANDLE_CHARS).collect()
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

fn basename(path: &str) -> Option<String> {
    Path::new(path.trim_end_matches('/'))
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
}

/// Repo display name: the main checkout's basename, with the worktree
/// directory in parentheses when the run is in a linked worktree — two
/// terminals on two worktrees of one repo must not look identical.
fn repo_display(working_dir: &str, repo_root: Option<&str>) -> String {
    let cwd_name = basename(working_dir);
    match repo_root.and_then(basename) {
        Some(root_name) => match cwd_name {
            Some(cwd) if cwd != root_name => format!("{root_name} ({cwd})"),
            _ => root_name,
        },
        None => cwd_name.unwrap_or_else(|| working_dir.to_string()),
    }
}

fn is_missing_lease_table(error: &sqlx::Error) -> bool {
    error
        .to_string()
        .contains("no such table: harness_process_leases")
}

// ── SQL fragments ───────────────────────────────────────────────────────

/// Two CTEs every per-run query starts with: `run_sessions` (the coordinator
/// and every session parented to it, at any depth) and `run_goals` (the goal
/// cards the run owns). Binds, in order: coordinator session id, lease
/// lifecycle id, coordinator session id.
macro_rules! run_cte {
    () => {
        "WITH RECURSIVE run_sessions(id) AS ( \
             SELECT ? \
             UNION \
             SELECT s.id FROM sessions s JOIN run_sessions r ON s.parent_session_id = r.id \
         ), \
         run_goals(id) AS ( \
             SELECT c.id FROM cards c \
             WHERE c.card_type = 'goal' AND c.archived_at IS NULL \
               AND ( json_extract(c.metadata_json, '$.dispatched_lifecycle') = ? \
                  OR json_extract(c.metadata_json, '$.created_by_session_id') = ? \
                  OR json_extract(c.metadata_json, '$.worker_session_id') \
                     IN (SELECT id FROM run_sessions) ) \
         ) "
    };
}

fn bind_run<'q>(
    query: sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments>,
    lease: &'q LeaseRow,
) -> sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments> {
    query
        .bind(lease.session_id.as_str())
        .bind(lease.lifecycle_id.as_str())
        .bind(lease.session_id.as_str())
}

// ── Leases ──────────────────────────────────────────────────────────────

/// One `harness_process_leases` row.
#[derive(Debug, Clone, Serialize)]
pub struct LeaseRow {
    pub lease_id: String,
    pub lifecycle_id: String,
    pub session_id: String,
    pub session_name: Option<String>,
    pub working_dir: String,
    pub repo_root: Option<String>,
    pub project_id: Option<String>,
    pub mode: String,
    pub recipe: Option<String>,
    pub pid: i64,
    pub phase: String,
    pub started_at: String,
    pub heartbeat_at: String,
    pub last_activity_at: Option<String>,
    pub last_activity: Option<String>,
    pub ended_at: Option<String>,
    pub exit_status: Option<String>,
}

/// Leases that are live or lost (heartbeat within [`LOST_RUN_HORIZON_SECS`])
/// plus those that exited within `ended_within`. Non-exited first, newest
/// heartbeat first, at most [`MAX_RUNS`]. A database that predates the lease
/// table reads as "no runs", not as an error.
pub async fn fetch_leases(
    pool: &Pool<Sqlite>,
    now: DateTime<Utc>,
    ended_within: std::time::Duration,
) -> Result<Vec<LeaseRow>, String> {
    let lost_cutoff = rfc3339(now - chrono::Duration::seconds(LOST_RUN_HORIZON_SECS));
    let ended_cutoff = rfc3339(
        now - chrono::Duration::from_std(ended_within).unwrap_or_else(|_| chrono::Duration::zero()),
    );
    let rows = sqlx::query(
        "SELECT lease_id, lifecycle_id, session_id, session_name, working_dir, repo_root, \
                project_id, mode, recipe, pid, phase, started_at, heartbeat_at, \
                last_activity_at, last_activity, ended_at, exit_status \
         FROM harness_process_leases \
         WHERE (phase != 'exited' AND julianday(heartbeat_at) >= julianday(?)) \
            OR (phase = 'exited' \
                AND julianday(COALESCE(ended_at, heartbeat_at)) >= julianday(?)) \
         ORDER BY (phase != 'exited') DESC, julianday(heartbeat_at) DESC \
         LIMIT ?",
    )
    .bind(lost_cutoff)
    .bind(ended_cutoff)
    .bind(MAX_RUNS)
    .fetch_all(pool)
    .await;
    let rows = match rows {
        Ok(rows) => rows,
        Err(error) if is_missing_lease_table(&error) => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    Ok(rows
        .iter()
        .map(|row| LeaseRow {
            lease_id: row.get("lease_id"),
            lifecycle_id: row.get("lifecycle_id"),
            session_id: row.get("session_id"),
            session_name: row.get("session_name"),
            working_dir: row.get("working_dir"),
            repo_root: row.get("repo_root"),
            project_id: row.get("project_id"),
            mode: row.get("mode"),
            recipe: row.get("recipe"),
            pid: row.get("pid"),
            phase: row.get("phase"),
            started_at: row.get("started_at"),
            heartbeat_at: row.get("heartbeat_at"),
            last_activity_at: row.get("last_activity_at"),
            last_activity: row.get("last_activity"),
            ended_at: row.get("ended_at"),
            exit_status: row.get("exit_status"),
        })
        .collect())
}

// ── Run summaries ───────────────────────────────────────────────────────

/// Goal counts for one run.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GoalCounts {
    /// Count per goal state binding (`triage`, `ready`, `in_progress`, …).
    pub by_state: BTreeMap<String, i64>,
    pub total: i64,
    /// Goals in `complete`.
    pub done: i64,
    pub in_progress: i64,
    /// Goals parked for a human.
    pub parked: i64,
    /// Distinct goals waiting on the user: parked and unfinished, or holding
    /// an open decision.
    pub waiting_on_you: i64,
}

/// One run as the listing shows it.
#[derive(Debug, Clone, Serialize)]
pub struct RunSummary {
    /// Short handle (lease id prefix) the user or agent can pass back.
    pub run: String,
    pub lease_id: String,
    pub lifecycle_id: String,
    pub session_id: String,
    /// The session's name (or, failing that, the harness prompt title) — a
    /// label, never a message body.
    pub title: Option<String>,
    pub repo: String,
    pub working_dir: String,
    pub repo_root: Option<String>,
    pub project_id: Option<String>,
    pub mode: String,
    pub recipe: Option<String>,
    pub pid: i64,
    pub phase: String,
    pub liveness: RunLiveness,
    pub started_at: String,
    pub age_secs: Option<i64>,
    pub heartbeat_at: String,
    pub heartbeat_age_secs: Option<i64>,
    pub last_activity_at: Option<String>,
    pub last_activity: Option<String>,
    pub ended_at: Option<String>,
    pub exit_status: Option<String>,
    pub goals: GoalCounts,
    /// Open decisions tied to this run's goals.
    pub open_decisions: i64,
    /// Settled spend so far across the coordinator and its workers (USD).
    pub spend_usd: f64,
}

/// Every live and lost run, plus runs that exited within `include_ended_within`.
pub async fn list_runs(
    pool: &Pool<Sqlite>,
    now: DateTime<Utc>,
    include_ended_within: std::time::Duration,
) -> Result<Vec<RunSummary>, String> {
    let leases = fetch_leases(pool, now, include_ended_within).await?;
    let mut out = Vec::with_capacity(leases.len());
    for lease in &leases {
        out.push(summarize_run(pool, lease, now).await?);
    }
    Ok(out)
}

/// Build the summary for one lease.
pub async fn summarize_run(
    pool: &Pool<Sqlite>,
    lease: &LeaseRow,
    now: DateTime<Utc>,
) -> Result<RunSummary, String> {
    let goals = goal_counts(pool, lease).await?;
    let open_decisions = open_decision_count(pool, lease).await?;
    let spend_usd = run_spend(pool, lease).await?;
    let title = run_title(pool, lease).await;
    Ok(RunSummary {
        run: short_handle(&lease.lease_id),
        lease_id: lease.lease_id.clone(),
        lifecycle_id: lease.lifecycle_id.clone(),
        session_id: lease.session_id.clone(),
        title,
        repo: repo_display(&lease.working_dir, lease.repo_root.as_deref()),
        working_dir: lease.working_dir.clone(),
        repo_root: lease.repo_root.clone(),
        project_id: lease.project_id.clone(),
        mode: lease.mode.clone(),
        recipe: lease.recipe.clone(),
        pid: lease.pid,
        phase: lease.phase.clone(),
        liveness: classify_lease(&lease.phase, &lease.heartbeat_at, now),
        started_at: lease.started_at.clone(),
        age_secs: age_secs(&lease.started_at, now),
        heartbeat_at: lease.heartbeat_at.clone(),
        heartbeat_age_secs: age_secs(&lease.heartbeat_at, now),
        last_activity_at: lease.last_activity_at.clone(),
        last_activity: non_empty(lease.last_activity.clone())
            .map(|a| truncate_chars(&a, ACTIVITY_MAX_CHARS)),
        ended_at: lease.ended_at.clone(),
        exit_status: lease.exit_status.clone(),
        goals,
        open_decisions,
        spend_usd,
    })
}

async fn goal_counts(pool: &Pool<Sqlite>, lease: &LeaseRow) -> Result<GoalCounts, String> {
    let sql = concat!(
        run_cte!(),
        "SELECT COALESCE(bc.state_binding, 'unknown') AS goal_state, COUNT(*) AS n, \
                COALESCE(SUM(CASE WHEN COALESCE(json_extract(c.metadata_json, \
                    '$.needs_human_attention'), 0) = 1 THEN 1 ELSE 0 END), 0) AS parked, \
                COALESCE(SUM(CASE WHEN (COALESCE(json_extract(c.metadata_json, \
                    '$.needs_human_attention'), 0) = 1 \
                    AND COALESCE(bc.state_binding, '') NOT IN ('complete', 'cancelled')) \
                  OR EXISTS (SELECT 1 FROM decisions d \
                             WHERE d.goal_id = c.id AND d.status = 'open') \
                  THEN 1 ELSE 0 END), 0) AS waiting \
         FROM run_goals g \
         JOIN cards c ON c.id = g.id \
         LEFT JOIN board_columns bc ON bc.id = c.column_id \
         GROUP BY goal_state"
    );
    let rows = bind_run(sqlx::query(sql), lease)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    let mut counts = GoalCounts::default();
    for row in &rows {
        let state: String = row.get("goal_state");
        let n: i64 = row.get("n");
        counts.parked += row.get::<i64, _>("parked");
        counts.waiting_on_you += row.get::<i64, _>("waiting");
        counts.total += n;
        match state.as_str() {
            "complete" => counts.done += n,
            "in_progress" => counts.in_progress += n,
            _ => {}
        }
        counts.by_state.insert(state, n);
    }
    Ok(counts)
}

async fn open_decision_count(pool: &Pool<Sqlite>, lease: &LeaseRow) -> Result<i64, String> {
    let sql = concat!(
        run_cte!(),
        "SELECT COUNT(*) FROM decisions d \
         WHERE d.status = 'open' AND d.goal_id IN (SELECT id FROM run_goals)"
    );
    bind_run(sqlx::query(sql), lease)
        .fetch_one(pool)
        .await
        .map(|row| row.get::<i64, _>(0))
        .map_err(|e| e.to_string())
}

async fn run_spend(pool: &Pool<Sqlite>, lease: &LeaseRow) -> Result<f64, String> {
    let sql = concat!(
        run_cte!(),
        "SELECT COALESCE(SUM(l.cost_usd), 0.0) FROM cost_ledger l \
         WHERE l.session_id IN (SELECT id FROM run_sessions) \
            OR l.session_id IN ( \
                SELECT json_extract(c.metadata_json, '$.worker_session_id') \
                FROM cards c JOIN run_goals g ON g.id = c.id)"
    );
    bind_run(sqlx::query(sql), lease)
        .fetch_one(pool)
        .await
        .map(|row| row.get::<f64, _>(0))
        .map_err(|e| e.to_string())
}

/// Latest harness-run snapshot telemetry for the coordinator session, if the
/// daemon ever received one. Reads only label-shaped fields.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RunTelemetry {
    pub status: Option<String>,
    pub prompt_title: Option<String>,
    pub tool_calls: Option<u64>,
    pub verification_verdict: Option<String>,
    /// Tool name of the gate the coordinator is waiting on, if any.
    pub pending_gate_tool: Option<String>,
    pub updated_at: Option<String>,
}

async fn run_telemetry(pool: &Pool<Sqlite>, session_id: &str) -> Option<RunTelemetry> {
    let row = sqlx::query(
        "SELECT status, updated_at, snapshot_json FROM harness_run_snapshots \
         WHERE session_id = ? ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .ok()??;
    let snapshot: serde_json::Value =
        serde_json::from_str(&row.get::<String, _>("snapshot_json")).unwrap_or_default();
    let text = |key: &str, max: usize| {
        snapshot
            .get(key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| truncate_chars(s, max))
    };
    Some(RunTelemetry {
        status: Some(row.get("status")),
        prompt_title: text("promptTitle", TITLE_MAX_CHARS),
        tool_calls: snapshot.get("toolCalls").and_then(|v| v.as_u64()),
        verification_verdict: text("verificationVerdict", 40),
        pending_gate_tool: snapshot
            .pointer("/pendingGate/toolName")
            .and_then(|v| v.as_str())
            .map(|s| truncate_chars(s, 60)),
        updated_at: Some(row.get("updated_at")),
    })
}

async fn run_title(pool: &Pool<Sqlite>, lease: &LeaseRow) -> Option<String> {
    if let Some(name) = non_empty(lease.session_name.clone()) {
        return Some(truncate_chars(&name, TITLE_MAX_CHARS));
    }
    let names: Option<(String, String)> =
        sqlx::query_as("SELECT name, description FROM sessions WHERE id = ?")
            .bind(&lease.session_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    if let Some((name, description)) = names {
        if let Some(label) = non_empty(Some(name)).or_else(|| non_empty(Some(description))) {
            return Some(truncate_chars(&label, TITLE_MAX_CHARS));
        }
    }
    run_telemetry(pool, &lease.session_id)
        .await
        .and_then(|t| t.prompt_title)
}

// ── Run status ──────────────────────────────────────────────────────────

/// One goal as `run_status` reports it.
#[derive(Debug, Clone, Serialize)]
pub struct GoalStatusRow {
    pub card_id: String,
    pub title: String,
    pub state: String,
    pub attempt_count: u64,
    pub worker_key: Option<String>,
    pub worker_session_id: Option<String>,
    pub worker_session_name: Option<String>,
    pub receipt_state: Option<String>,
    pub receipt_heartbeat_age_secs: Option<i64>,
    /// Name of the worker's most recent tool request — the name only.
    pub last_tool: Option<String>,
    pub last_tool_age_secs: Option<i64>,
    pub last_error: Option<String>,
    pub needs_human_attention: bool,
    pub attention_reason: Option<String>,
    pub abandoned_work: bool,
    pub verify_verdict: Option<String>,
    /// `kind: headline` of an open gate-shaped decision on this goal.
    pub pending_gate: Option<String>,
    /// `awaiting_approval` (open review decision) or `in_review`.
    pub review_state: Option<String>,
    pub landed_commit: Option<String>,
    /// Landing outcome when a landing was attempted but did not land.
    pub landing_outcome: Option<String>,
    pub open_decisions: i64,
    pub updated_at: String,
}

/// One activity-journal row for the run.
#[derive(Debug, Clone, Serialize)]
pub struct JournalRow {
    pub ts: String,
    pub age_secs: Option<i64>,
    pub kind: String,
    pub actor: String,
    pub title: String,
    pub ref_kind: Option<String>,
    pub ref_id: Option<String>,
}

/// Full status for one run.
#[derive(Debug, Clone, Serialize)]
pub struct RunStatus {
    pub run: RunSummary,
    pub goals: Vec<GoalStatusRow>,
    /// True when the run owns more goals than [`MAX_GOALS_PER_RUN`].
    pub goals_truncated: bool,
    pub journal: Vec<JournalRow>,
    pub telemetry: Option<RunTelemetry>,
}

/// A run offered back when a selector does not pick exactly one.
#[derive(Debug, Clone, Serialize)]
pub struct RunChoice {
    pub run: String,
    pub lease_id: String,
    pub session_id: String,
    pub title: Option<String>,
    pub repo: String,
    pub working_dir: String,
    pub phase: String,
    pub liveness: RunLiveness,
    pub heartbeat_age_secs: Option<i64>,
}

/// Outcome of resolving a run selector.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "resolution", rename_all = "snake_case")]
pub enum RunSelection {
    Resolved(Box<RunStatus>),
    /// No run is live or recent at all.
    NoRuns,
    /// The selector matched none, several, or (when omitted) not exactly one
    /// live run.
    Choose {
        reason: String,
        choices: Vec<RunChoice>,
    },
}

fn choice_for(lease: &LeaseRow, title: Option<String>, now: DateTime<Utc>) -> RunChoice {
    RunChoice {
        run: short_handle(&lease.lease_id),
        lease_id: lease.lease_id.clone(),
        session_id: lease.session_id.clone(),
        title,
        repo: repo_display(&lease.working_dir, lease.repo_root.as_deref()),
        working_dir: lease.working_dir.clone(),
        phase: lease.phase.clone(),
        liveness: classify_lease(&lease.phase, &lease.heartbeat_at, now),
        heartbeat_age_secs: age_secs(&lease.heartbeat_at, now),
    }
}

/// Pick the leases a selector names, tier by tier: exact id, id prefix,
/// exact session name, then a case-insensitive fragment of the session name,
/// working directory or repo root. The first tier with any match wins.
fn match_selector<'a>(
    leases: &'a [LeaseRow],
    titles: &[Option<String>],
    selector: &str,
) -> Vec<&'a LeaseRow> {
    let wanted = selector.trim();
    let wanted_lower = wanted.to_lowercase();
    let exact: Vec<&LeaseRow> = leases
        .iter()
        .filter(|l| l.lease_id == wanted || l.session_id == wanted)
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    if wanted.chars().count() >= 4 {
        let prefix: Vec<&LeaseRow> = leases
            .iter()
            .filter(|l| l.lease_id.starts_with(wanted) || l.session_id.starts_with(wanted))
            .collect();
        if !prefix.is_empty() {
            return prefix;
        }
    }
    let label_of = |i: usize| {
        leases[i]
            .session_name
            .clone()
            .or_else(|| titles.get(i).cloned().flatten())
            .unwrap_or_default()
            .to_lowercase()
    };
    let named: Vec<&LeaseRow> = (0..leases.len())
        .filter(|&i| !wanted_lower.is_empty() && label_of(i) == wanted_lower)
        .map(|i| &leases[i])
        .collect();
    if !named.is_empty() {
        return named;
    }
    if wanted_lower.is_empty() {
        return Vec::new();
    }
    (0..leases.len())
        .filter(|&i| {
            let l = &leases[i];
            label_of(i).contains(&wanted_lower)
                || l.working_dir.to_lowercase().contains(&wanted_lower)
                || l.repo_root
                    .as_deref()
                    .is_some_and(|r| r.to_lowercase().contains(&wanted_lower))
        })
        .map(|i| &leases[i])
        .collect()
}

/// Resolve `selector` (lease id or prefix, session id, session name, or a
/// repo/cwd fragment) to one run and report its status. With no selector,
/// the single live run is used; zero or several live runs return the choices.
pub async fn run_status(
    pool: &Pool<Sqlite>,
    selector: Option<&str>,
    now: DateTime<Utc>,
) -> Result<RunSelection, String> {
    let selector = selector.map(str::trim).filter(|s| !s.is_empty());
    let window = if selector.is_some() {
        SELECTOR_ENDED_WINDOW
    } else {
        DEFAULT_RECENT_ENDED
    };
    let leases = fetch_leases(pool, now, window).await?;
    if leases.is_empty() {
        return Ok(RunSelection::NoRuns);
    }
    let mut titles = Vec::with_capacity(leases.len());
    for lease in &leases {
        titles.push(run_title(pool, lease).await);
    }
    let title_of = |lease: &LeaseRow| {
        leases
            .iter()
            .position(|l| l.lease_id == lease.lease_id)
            .and_then(|i| titles[i].clone())
    };
    let is_live =
        |l: &&LeaseRow| classify_lease(&l.phase, &l.heartbeat_at, now) == RunLiveness::Live;

    let (candidates, reason_prefix): (Vec<&LeaseRow>, String) = match selector {
        Some(sel) => (
            match_selector(&leases, &titles, sel),
            format!("\"{}\"", truncate_chars(sel, 60)),
        ),
        None => (leases.iter().filter(is_live).collect(), String::new()),
    };

    let chosen: Option<&LeaseRow> = match candidates.len() {
        1 => Some(candidates[0]),
        0 => None,
        _ => {
            let live: Vec<&&LeaseRow> = candidates.iter().filter(|l| is_live(*l)).collect();
            if selector.is_some() && live.len() == 1 {
                Some(*live[0])
            } else {
                None
            }
        }
    };

    let Some(lease) = chosen else {
        let (reason, offered): (String, Vec<&LeaseRow>) = match (selector, candidates.len()) {
            (None, 0) => (
                "No terminal harness run is live right now; these are lost or recently ended."
                    .to_string(),
                leases.iter().collect(),
            ),
            (None, n) => (
                format!("{n} terminal harness runs are live; name one."),
                candidates.clone(),
            ),
            (Some(_), 0) => (
                format!("No run matches {reason_prefix}; these are the known runs."),
                leases.iter().collect(),
            ),
            (Some(_), n) => (
                format!("{n} runs match {reason_prefix}; name one."),
                candidates.clone(),
            ),
        };
        return Ok(RunSelection::Choose {
            reason,
            choices: offered
                .into_iter()
                .map(|l| choice_for(l, title_of(l), now))
                .collect(),
        });
    };

    let run = summarize_run(pool, lease, now).await?;
    let (goals, goals_truncated) = goal_rows(pool, lease, now).await?;
    let journal = journal_rows(pool, lease, now).await?;
    let telemetry = run_telemetry(pool, &lease.session_id).await;
    Ok(RunSelection::Resolved(Box::new(RunStatus {
        run,
        goals,
        goals_truncated,
        journal,
        telemetry,
    })))
}

struct OpenDecision {
    id: String,
    kind: String,
    goal_id: Option<String>,
    project_id: Option<String>,
    headline: String,
    created_at: String,
}

async fn open_decisions_for_run(
    pool: &Pool<Sqlite>,
    lease: &LeaseRow,
) -> Result<Vec<OpenDecision>, String> {
    let sql = concat!(
        run_cte!(),
        "SELECT d.id, d.kind, d.goal_id, d.project_id, d.headline, d.created_at \
         FROM decisions d \
         WHERE d.status = 'open' AND d.goal_id IN (SELECT id FROM run_goals) \
         ORDER BY d.created_at DESC LIMIT ?"
    );
    let rows = bind_run(sqlx::query(sql), lease)
        .bind(MAX_ATTENTION_ITEMS as i64)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(decision_from_row).collect())
}

fn decision_from_row(row: &sqlx::sqlite::SqliteRow) -> OpenDecision {
    OpenDecision {
        id: row.get("id"),
        kind: row.get("kind"),
        goal_id: row.get("goal_id"),
        project_id: row.get("project_id"),
        headline: truncate_chars(&row.get::<String, _>("headline"), HEADLINE_MAX_CHARS),
        created_at: row.get("created_at"),
    }
}

async fn goal_rows(
    pool: &Pool<Sqlite>,
    lease: &LeaseRow,
    now: DateTime<Utc>,
) -> Result<(Vec<GoalStatusRow>, bool), String> {
    let sql = concat!(
        run_cte!(),
        "SELECT c.id, c.title, COALESCE(bc.state_binding, 'unknown') AS goal_state, \
                c.metadata_json, c.updated_at \
         FROM run_goals g \
         JOIN cards c ON c.id = g.id \
         LEFT JOIN board_columns bc ON bc.id = c.column_id \
         ORDER BY c.updated_at DESC \
         LIMIT ?"
    );
    // One extra row tells us whether the list was cut.
    let rows = bind_run(sqlx::query(sql), lease)
        .bind(MAX_GOALS_PER_RUN + 1)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    let truncated = rows.len() as i64 > MAX_GOALS_PER_RUN;

    let mut decisions_by_goal: HashMap<String, Vec<OpenDecision>> = HashMap::new();
    for decision in open_decisions_for_run(pool, lease).await? {
        if let Some(goal_id) = decision.goal_id.clone() {
            decisions_by_goal.entry(goal_id).or_default().push(decision);
        }
    }

    let mut out = Vec::new();
    for row in rows.iter().take(MAX_GOALS_PER_RUN as usize) {
        let card_id: String = row.get("id");
        let meta: serde_json::Value =
            serde_json::from_str(&row.get::<String, _>("metadata_json")).unwrap_or_default();
        let decisions = decisions_by_goal.remove(&card_id).unwrap_or_default();
        out.push(
            goal_row(
                pool,
                GoalRowInput {
                    card_id,
                    title: row.get("title"),
                    state: row.get("goal_state"),
                    updated_at: row.get("updated_at"),
                    meta,
                    decisions,
                },
                now,
            )
            .await,
        );
    }
    Ok((out, truncated))
}

struct GoalRowInput {
    card_id: String,
    title: String,
    state: String,
    updated_at: String,
    meta: serde_json::Value,
    decisions: Vec<OpenDecision>,
}

fn meta_str(meta: &serde_json::Value, pointer: &str) -> Option<String> {
    meta.pointer(pointer)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
}

async fn goal_row(pool: &Pool<Sqlite>, input: GoalRowInput, now: DateTime<Utc>) -> GoalStatusRow {
    let meta = &input.meta;
    let worker_session_id = meta_str(meta, "/worker_session_id")
        .or_else(|| meta_str(meta, "/execution_receipt/session_id"));
    let (worker_session_name, last_tool, last_tool_age_secs) = match &worker_session_id {
        Some(sid) => {
            let name = session_name(pool, sid).await;
            let (tool, at) = last_tool_request(pool, sid).await.unzip();
            (name, tool, at.map(|ts| (now.timestamp() - ts).max(0)))
        }
        None => (None, None, None),
    };

    let landing_outcome = meta_str(meta, "/dispatch_evidence/landing/outcome");
    let landed_commit = if landing_outcome.as_deref() == Some("landed") {
        meta_str(meta, "/dispatch_evidence/landing/landed_head")
            .map(|sha| sha.chars().take(SHORT_SHA_CHARS).collect())
    } else {
        None
    };

    let has_review_decision = input.decisions.iter().any(|d| d.kind == "approve_review");
    let review_state = if has_review_decision {
        Some("awaiting_approval".to_string())
    } else if input.state == "review" {
        Some("in_review".to_string())
    } else {
        None
    };
    let pending_gate = input
        .decisions
        .iter()
        .find(|d| GATE_DECISION_KINDS.contains(&d.kind.as_str()))
        .map(|d| format!("{}: {}", d.kind, d.headline));

    GoalStatusRow {
        title: truncate_chars(&input.title, TITLE_MAX_CHARS),
        state: input.state,
        attempt_count: meta
            .get("attempt_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        worker_key: meta_str(meta, "/worker_key")
            .or_else(|| meta_str(meta, "/execution_receipt/worker_key")),
        worker_session_id,
        worker_session_name,
        receipt_state: meta_str(meta, "/execution_receipt/state"),
        receipt_heartbeat_age_secs: meta_str(meta, "/execution_receipt/last_heartbeat_at")
            .and_then(|ts| age_secs(&ts, now)),
        last_tool,
        last_tool_age_secs,
        last_error: meta_str(meta, "/last_error").map(|e| truncate_chars(&e, LAST_ERROR_MAX_CHARS)),
        needs_human_attention: meta
            .get("needs_human_attention")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        attention_reason: meta_str(meta, "/needs_human_attention_reason")
            .map(|r| truncate_chars(&r, REASON_MAX_CHARS)),
        abandoned_work: meta
            .get("abandoned_work")
            .is_some_and(|v| !v.is_null() && v.as_str().is_none_or(|s| !s.is_empty())),
        verify_verdict: meta_str(meta, "/dispatch_evidence/verdict/status"),
        pending_gate,
        review_state,
        landed_commit,
        landing_outcome: landing_outcome.filter(|o| o != "landed"),
        open_decisions: input.decisions.len() as i64,
        updated_at: input.updated_at,
        card_id: input.card_id,
    }
}

async fn session_name(pool: &Pool<Sqlite>, session_id: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT name FROM sessions WHERE id = ?")
        .bind(session_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .filter(|n| !n.trim().is_empty())
        .map(|n| truncate_chars(&n, TITLE_MAX_CHARS))
}

/// The worker's most recent tool request: `(tool name, created_timestamp)`.
/// Reads only the `toolCall.value.name` field of the newest
/// [`WORKER_MESSAGE_SCAN`] assistant messages — never their text or arguments.
async fn last_tool_request(pool: &Pool<Sqlite>, session_id: &str) -> Option<(String, i64)> {
    sqlx::query_as::<_, (String, i64)>(
        "SELECT json_extract(j.value, '$.toolCall.value.name') AS tool_name, \
                m.created_timestamp \
         FROM (SELECT id, content_json, created_timestamp FROM messages \
               WHERE session_id = ? AND role = 'assistant' AND json_valid(content_json) \
               ORDER BY id DESC LIMIT ?) m, \
              json_each(m.content_json) j \
         WHERE json_extract(j.value, '$.type') = 'toolRequest' \
           AND json_extract(j.value, '$.toolCall.value.name') IS NOT NULL \
         ORDER BY m.id DESC, CAST(j.key AS INTEGER) DESC \
         LIMIT 1",
    )
    .bind(session_id)
    .bind(WORKER_MESSAGE_SCAN)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .map(|(name, at)| (truncate_chars(&name, 80), at))
}

async fn journal_rows(
    pool: &Pool<Sqlite>,
    lease: &LeaseRow,
    now: DateTime<Utc>,
) -> Result<Vec<JournalRow>, String> {
    let sql = concat!(
        run_cte!(),
        "SELECT aj.ts, aj.kind, aj.actor, aj.title, aj.ref_kind, aj.ref_id \
         FROM activity_journal aj \
         WHERE (aj.ref_kind = 'goal' AND aj.ref_id IN (SELECT id FROM run_goals)) \
            OR (aj.ref_kind = 'decision' AND aj.ref_id IN ( \
                SELECT d.id FROM decisions d WHERE d.goal_id IN (SELECT id FROM run_goals))) \
         ORDER BY aj.ts DESC \
         LIMIT ?"
    );
    let rows = bind_run(sqlx::query(sql), lease)
        .bind(MAX_JOURNAL_ROWS)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .map(|row| {
            let ts: String = row.get("ts");
            JournalRow {
                age_secs: age_secs(&ts, now),
                ts,
                kind: row.get("kind"),
                actor: row.get("actor"),
                title: truncate_chars(&row.get::<String, _>("title"), TITLE_MAX_CHARS),
                ref_kind: row.get("ref_kind"),
                ref_id: row.get("ref_id"),
            }
        })
        .collect())
}

// ── Attention ───────────────────────────────────────────────────────────

/// What kind of thing is waiting on the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionKind {
    /// An open decision on a run goal (review, escalation, …).
    Decision,
    /// An open gate-shaped decision, or the coordinator's own pending gate.
    PendingGate,
    /// A goal parked for a human.
    ParkedGoal,
    /// An open project-level decision (no goal) on a run's project.
    ProjectDecision,
    /// A terminal that stopped heartbeating while goals were still in progress.
    LostRun,
}

/// One attention item.
#[derive(Debug, Clone, Serialize)]
pub struct AttentionItem {
    pub kind: AttentionKind,
    pub run: String,
    pub repo: String,
    pub liveness: RunLiveness,
    pub goal_id: Option<String>,
    pub goal_title: Option<String>,
    pub decision_id: Option<String>,
    pub decision_kind: Option<String>,
    /// Short label: a decision headline, a park reason, or a lost-run summary.
    pub summary: String,
    pub since: Option<String>,
}

/// Everything waiting on the user across live, lost and recently ended runs.
#[derive(Debug, Clone, Serialize)]
pub struct AttentionReport {
    pub runs_considered: usize,
    pub items: Vec<AttentionItem>,
    pub truncated: bool,
}

/// Collect attention items across runs.
pub async fn attention(pool: &Pool<Sqlite>, now: DateTime<Utc>) -> Result<AttentionReport, String> {
    let leases = fetch_leases(pool, now, DEFAULT_RECENT_ENDED).await?;
    let mut items: Vec<AttentionItem> = Vec::new();
    let mut seen_projects: HashSet<String> = HashSet::new();
    for lease in &leases {
        collect_run_attention(pool, lease, now, &mut items, &mut seen_projects).await?;
    }
    let truncated = items.len() > MAX_ATTENTION_ITEMS;
    items.truncate(MAX_ATTENTION_ITEMS);
    Ok(AttentionReport {
        runs_considered: leases.len(),
        items,
        truncated,
    })
}

async fn collect_run_attention(
    pool: &Pool<Sqlite>,
    lease: &LeaseRow,
    now: DateTime<Utc>,
    items: &mut Vec<AttentionItem>,
    seen_projects: &mut HashSet<String>,
) -> Result<(), String> {
    let liveness = classify_lease(&lease.phase, &lease.heartbeat_at, now);
    let run = short_handle(&lease.lease_id);
    let repo = repo_display(&lease.working_dir, lease.repo_root.as_deref());
    let item = |kind: AttentionKind, summary: String| AttentionItem {
        kind,
        run: run.clone(),
        repo: repo.clone(),
        liveness,
        goal_id: None,
        goal_title: None,
        decision_id: None,
        decision_kind: None,
        summary,
        since: None,
    };

    if liveness == RunLiveness::Lost {
        let counts = goal_counts(pool, lease).await?;
        if counts.in_progress > 0 {
            items.push(AttentionItem {
                since: Some(lease.heartbeat_at.clone()),
                ..item(
                    AttentionKind::LostRun,
                    format!(
                        "terminal exited unexpectedly (phase {}, last heartbeat {}) with {} goal{} \
                         still in progress",
                        lease.phase,
                        age_secs(&lease.heartbeat_at, now)
                            .map(|s| format!("{} ago", human_duration(s)))
                            .unwrap_or_else(|| "unknown".to_string()),
                        counts.in_progress,
                        if counts.in_progress == 1 { "" } else { "s" }
                    ),
                )
            });
        }
    }

    let titles = goal_titles_for_run(pool, lease).await?;
    for decision in open_decisions_for_run(pool, lease).await? {
        let kind = if GATE_DECISION_KINDS.contains(&decision.kind.as_str()) {
            AttentionKind::PendingGate
        } else {
            AttentionKind::Decision
        };
        items.push(AttentionItem {
            goal_title: decision
                .goal_id
                .as_ref()
                .and_then(|g| titles.get(g).map(|(t, _)| t.clone())),
            goal_id: decision.goal_id.clone(),
            decision_id: Some(decision.id.clone()),
            decision_kind: Some(decision.kind.clone()),
            since: Some(decision.created_at.clone()),
            ..item(kind, decision.headline.clone())
        });
    }

    for (goal_id, (title, parked)) in &titles {
        if let Some(reason) = parked {
            items.push(AttentionItem {
                goal_id: Some(goal_id.clone()),
                goal_title: Some(title.clone()),
                ..item(AttentionKind::ParkedGoal, reason.clone())
            });
        }
    }

    if let Some(gate_tool) = run_telemetry(pool, &lease.session_id)
        .await
        .filter(|t| liveness == RunLiveness::Live || t.status.as_deref() == Some("waiting_gate"))
        .and_then(|t| t.pending_gate_tool)
    {
        items.push(item(
            AttentionKind::PendingGate,
            format!("coordinator is waiting on approval for {gate_tool}"),
        ));
    }

    if let Some(project_id) = lease.project_id.clone() {
        if seen_projects.insert(project_id.clone()) {
            for decision in open_project_decisions(pool, &project_id).await? {
                items.push(AttentionItem {
                    decision_id: Some(decision.id.clone()),
                    decision_kind: Some(decision.kind.clone()),
                    since: Some(decision.created_at.clone()),
                    ..item(AttentionKind::ProjectDecision, decision.headline.clone())
                });
            }
        }
    }
    Ok(())
}

/// Titles of the run's goals keyed by id, with a park reason for goals parked
/// for a human and not yet finished. Bounded by [`MAX_GOALS_PER_RUN`].
async fn goal_titles_for_run(
    pool: &Pool<Sqlite>,
    lease: &LeaseRow,
) -> Result<BTreeMap<String, (String, Option<String>)>, String> {
    let sql = concat!(
        run_cte!(),
        "SELECT c.id, c.title, COALESCE(bc.state_binding, 'unknown') AS goal_state, \
                COALESCE(json_extract(c.metadata_json, '$.needs_human_attention'), 0) AS parked, \
                json_extract(c.metadata_json, '$.needs_human_attention_reason') AS reason, \
                json_extract(c.metadata_json, '$.last_error') AS last_error \
         FROM run_goals g \
         JOIN cards c ON c.id = g.id \
         LEFT JOIN board_columns bc ON bc.id = c.column_id \
         ORDER BY c.updated_at DESC \
         LIMIT ?"
    );
    let rows = bind_run(sqlx::query(sql), lease)
        .bind(MAX_GOALS_PER_RUN)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    let mut out = BTreeMap::new();
    for row in &rows {
        let state: String = row.get("goal_state");
        let parked = row.try_get::<i64, _>("parked").unwrap_or(0) == 1
            && !matches!(state.as_str(), "complete" | "cancelled");
        let reason = parked.then(|| {
            let reason: Option<String> = row.try_get("reason").ok().flatten();
            let last_error: Option<String> = row.try_get("last_error").ok().flatten();
            non_empty(reason)
                .or_else(|| non_empty(last_error))
                .map(|r| format!("parked for you: {}", truncate_chars(&r, REASON_MAX_CHARS)))
                .unwrap_or_else(|| "parked for you".to_string())
        });
        out.insert(
            row.get::<String, _>("id"),
            (
                truncate_chars(&row.get::<String, _>("title"), TITLE_MAX_CHARS),
                reason,
            ),
        );
    }
    Ok(out)
}

async fn open_project_decisions(
    pool: &Pool<Sqlite>,
    project_id: &str,
) -> Result<Vec<OpenDecision>, String> {
    let rows = sqlx::query(
        "SELECT id, kind, goal_id, project_id, headline, created_at FROM decisions \
         WHERE status = 'open' AND goal_id IS NULL AND project_id = ? \
         ORDER BY created_at DESC LIMIT ?",
    )
    .bind(project_id)
    .bind(MAX_ATTENTION_ITEMS as i64)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .map(decision_from_row)
        .filter(|d| d.project_id.is_some())
        .collect())
}

/// "45s", "12m", "3h 5m", "2d 4h".
pub fn human_duration(secs: i64) -> String {
    let secs = secs.max(0);
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => match (s % 3600) / 60 {
            0 => format!("{}h", s / 3600),
            m => format!("{}h {m}m", s / 3600),
        },
        s => match (s % 86_400) / 3600 {
            0 => format!("{}d", s / 86_400),
            h => format!("{}d {h}h", s / 86_400),
        },
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Seeded fixture shared by this module's tests and the chat extension's.

    use super::*;

    /// The lease table exactly as the lease writer creates it. `IF NOT EXISTS`
    /// so it is a no-op once the schema chain creates the table itself.
    pub const LEASE_TABLE_SQL: &str = "CREATE TABLE IF NOT EXISTS harness_process_leases (lease_id TEXT PRIMARY KEY, lifecycle_id TEXT NOT NULL, session_id TEXT NOT NULL, session_name TEXT, working_dir TEXT NOT NULL, repo_root TEXT, project_id TEXT, mode TEXT NOT NULL, recipe TEXT, pid INTEGER NOT NULL, phase TEXT NOT NULL, started_at TEXT NOT NULL, heartbeat_at TEXT NOT NULL, last_activity_at TEXT, last_activity TEXT, ended_at TEXT, exit_status TEXT)";

    /// Planted in message text, tool arguments, a prompt context and a
    /// decision detail. No output may ever contain it.
    pub const SENTINEL: &str = "SENTINEL-7f3a-do-not-leak";

    pub const PROJECT: &str = "proj-alpha";
    pub const LIVE_LEASE: &str = "hpl-live-0001-aaaa";
    pub const LOST_LEASE: &str = "hpl-lost-0002-bbbb";
    pub const LIVE_LIFECYCLE: &str = "life-live";
    pub const LOST_LIFECYCLE: &str = "life-lost";
    pub const LIVE_COORD: &str = "sess-coord-live";
    pub const LOST_COORD: &str = "sess-coord-lost";

    pub fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-17T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    pub fn ago(secs: i64) -> String {
        rfc3339(now() - chrono::Duration::seconds(secs))
    }

    pub async fn empty_pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::session::spectral_schema::init_spectral_db(&pool)
            .await
            .unwrap();
        sqlx::query(LEASE_TABLE_SQL).execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO projects (id, slug, name, root_path) \
             VALUES (?, 'alpha', 'Alpha', '/work/alpha')",
        )
        .bind(PROJECT)
        .execute(&pool)
        .await
        .unwrap();
        for (i, state) in [
            "triage",
            "ready",
            "in_progress",
            "review",
            "complete",
            "failed",
            "cancelled",
        ]
        .iter()
        .enumerate()
        {
            sqlx::query(
                "INSERT INTO board_columns (id, project_id, name, position, column_kind, \
                 state_binding) VALUES (?, ?, ?, ?, 'state', ?)",
            )
            .bind(format!("col-alpha-{state}"))
            .bind(PROJECT)
            .bind(*state)
            .bind(i as i64)
            .bind(*state)
            .execute(&pool)
            .await
            .unwrap();
        }
        pool
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_lease(
        pool: &Pool<Sqlite>,
        lease_id: &str,
        lifecycle: &str,
        session_id: &str,
        working_dir: &str,
        repo_root: Option<&str>,
        phase: &str,
        heartbeat_at: &str,
        ended_at: Option<&str>,
    ) {
        sqlx::query(
            "INSERT INTO harness_process_leases (lease_id, lifecycle_id, session_id, \
             session_name, working_dir, repo_root, project_id, mode, recipe, pid, phase, \
             started_at, heartbeat_at, last_activity_at, last_activity, ended_at, exit_status) \
             VALUES (?, ?, ?, NULL, ?, ?, ?, 'headless', 'permagent-coding', 4242, ?, ?, ?, ?, \
             'developer__shell', ?, ?)",
        )
        .bind(lease_id)
        .bind(lifecycle)
        .bind(session_id)
        .bind(working_dir)
        .bind(repo_root)
        .bind(PROJECT)
        .bind(phase)
        .bind(ago(900))
        .bind(heartbeat_at)
        .bind(heartbeat_at)
        .bind(ended_at)
        .bind(ended_at.map(|_| "completed"))
        .execute(pool)
        .await
        .unwrap();
    }

    pub async fn insert_session(pool: &Pool<Sqlite>, id: &str, name: &str, parent: Option<&str>) {
        sqlx::query(
            "INSERT INTO sessions (id, name, working_dir, session_type, parent_session_id) \
             VALUES (?, ?, '/work/alpha', ?, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(if parent.is_some() {
            "sub_agent"
        } else {
            "user"
        })
        .bind(parent)
        .execute(pool)
        .await
        .unwrap();
    }

    pub async fn insert_goal(
        pool: &Pool<Sqlite>,
        id: &str,
        title: &str,
        state: &str,
        meta: serde_json::Value,
    ) {
        sqlx::query(
            "INSERT INTO cards (id, project_id, card_type, title, column_id, metadata_json, \
             updated_at) VALUES (?, ?, 'goal', ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(PROJECT)
        .bind(title)
        .bind(format!("col-alpha-{state}"))
        .bind(meta.to_string())
        .bind(ago(60))
        .execute(pool)
        .await
        .unwrap();
    }

    pub async fn insert_decision(
        pool: &Pool<Sqlite>,
        id: &str,
        kind: &str,
        goal_id: Option<&str>,
        headline: &str,
    ) {
        sqlx::query(
            "INSERT INTO decisions (id, kind, goal_id, project_id, tier, headline, detail, \
             status, created_at) VALUES (?, ?, ?, ?, 1, ?, ?, 'open', ?)",
        )
        .bind(id)
        .bind(kind)
        .bind(goal_id)
        .bind(PROJECT)
        .bind(headline)
        .bind(format!("detail body {SENTINEL}"))
        .bind(ago(120))
        .execute(pool)
        .await
        .unwrap();
    }

    pub async fn insert_cost(pool: &Pool<Sqlite>, call_id: &str, session_id: &str, usd: f64) {
        sqlx::query(
            "INSERT INTO cost_ledger (call_id, ts, session_id, cost_usd) VALUES (?, ?, ?, ?)",
        )
        .bind(call_id)
        .bind(ago(30))
        .bind(session_id)
        .bind(usd)
        .execute(pool)
        .await
        .unwrap();
    }

    pub async fn insert_journal(
        pool: &Pool<Sqlite>,
        id: &str,
        ts: &str,
        ref_kind: &str,
        ref_id: &str,
        title: &str,
    ) {
        sqlx::query(
            "INSERT INTO activity_journal (id, ts, kind, actor, title, detail, ref_kind, ref_id) \
             VALUES (?, ?, 'goal_state_changed', 'system', ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(ts)
        .bind(title)
        .bind(format!("journal detail {SENTINEL}"))
        .bind(ref_kind)
        .bind(ref_id)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Two leases (one live, one stale), goals across states, decisions,
    /// cost rows for coordinator and workers, worker messages carrying the
    /// sentinel, a journal row and a harness snapshot.
    pub async fn seeded_pool() -> Pool<Sqlite> {
        let pool = empty_pool().await;

        insert_session(&pool, LIVE_COORD, "Ship the lease reader", None).await;
        insert_session(&pool, "sess-worker-1", "worker one", Some(LIVE_COORD)).await;
        insert_session(&pool, "sess-worker-2", "worker two", Some(LIVE_COORD)).await;
        insert_session(&pool, LOST_COORD, "Refactor beta", None).await;
        insert_session(&pool, "sess-worker-3", "worker three", Some(LOST_COORD)).await;
        insert_session(&pool, "sess-unrelated", "someone else", None).await;

        insert_lease(
            &pool,
            LIVE_LEASE,
            LIVE_LIFECYCLE,
            LIVE_COORD,
            "/work/alpha-wt",
            Some("/work/alpha"),
            "turn",
            &ago(5),
            None,
        )
        .await;
        insert_lease(
            &pool,
            LOST_LEASE,
            LOST_LIFECYCLE,
            LOST_COORD,
            "/work/beta",
            None,
            "draining",
            &ago(300),
            None,
        )
        .await;

        let long_error = "é".repeat(150) + &"x".repeat(150);
        insert_goal(
            &pool,
            "g1",
            "Write the lease reader",
            "in_progress",
            serde_json::json!({
                "dispatched_lifecycle": LIVE_LIFECYCLE,
                "worker_session_id": "sess-worker-1",
                "worker_key": "opus",
                "attempt_count": 2,
                "last_error": long_error,
                "execution_receipt": {
                    "state": "running",
                    "session_id": "sess-worker-1",
                    "last_heartbeat_at": ago(12),
                },
            }),
        )
        .await;
        insert_goal(
            &pool,
            "g2",
            "Add the chat tools",
            "review",
            serde_json::json!({
                "created_by_session_id": LIVE_COORD,
                "worker_session_id": "sess-worker-2",
                "attempt_count": 1,
                "dispatch_evidence": {"verdict": {"status": "pass"}},
            }),
        )
        .await;
        insert_goal(
            &pool,
            "g3",
            "Schema contract",
            "complete",
            serde_json::json!({
                "created_by_session_id": LIVE_COORD,
                "attempt_count": 1,
                "dispatch_evidence": {
                    "commits": ["abc"],
                    "landing": {"outcome": "landed", "landed_head": "0123456789abcdef0123"},
                },
            }),
        )
        .await;
        insert_goal(
            &pool,
            "g4",
            "Decide the retention window",
            "ready",
            serde_json::json!({
                "created_by_session_id": LIVE_COORD,
                "needs_human_attention": true,
                "needs_human_attention_reason": "budget exhausted",
            }),
        )
        .await;
        insert_goal(
            &pool,
            "g5",
            "Beta refactor step",
            "in_progress",
            serde_json::json!({
                "dispatched_lifecycle": LOST_LIFECYCLE,
                "worker_session_id": "sess-worker-3",
                "attempt_count": 1,
            }),
        )
        .await;
        insert_goal(
            &pool,
            "g-other",
            "Not this run",
            "in_progress",
            serde_json::json!({"dispatched_lifecycle": "life-other"}),
        )
        .await;

        insert_decision(
            &pool,
            "dec-review",
            "approve_review",
            Some("g2"),
            "Review: Add the chat tools",
        )
        .await;
        insert_decision(
            &pool,
            "dec-project",
            "project_intel_proposal",
            None,
            "New competitor found",
        )
        .await;

        insert_cost(&pool, "c1", LIVE_COORD, 0.50).await;
        insert_cost(&pool, "c2", "sess-worker-1", 0.25).await;
        insert_cost(&pool, "c3", "sess-worker-2", 0.05).await;
        insert_cost(&pool, "c4", "sess-unrelated", 9.00).await;
        insert_cost(&pool, "c5", LOST_COORD, 1.00).await;

        let text_and_tool = serde_json::json!([
            {"type": "text", "text": format!("I will now {SENTINEL}")},
            {"type": "toolRequest", "id": "t1", "toolCall": {"status": "success", "value": {
                "name": "developer__shell", "arguments": {"command": format!("echo {SENTINEL}")}
            }}}
        ]);
        sqlx::query(
            "INSERT INTO messages (message_id, session_id, role, content_json, created_timestamp) \
             VALUES ('m1', 'sess-worker-1', 'assistant', ?, ?)",
        )
        .bind(text_and_tool.to_string())
        .bind(now().timestamp() - 30)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO messages (message_id, session_id, role, content_json, created_timestamp) \
             VALUES ('m2', ?, 'user', ?, ?)",
        )
        .bind(LIVE_COORD)
        .bind(serde_json::json!([{"type": "text", "text": SENTINEL}]).to_string())
        .bind(now().timestamp() - 600)
        .execute(&pool)
        .await
        .unwrap();

        insert_journal(
            &pool,
            "j1",
            &ago(90),
            "goal",
            "g1",
            "Write the lease reader → in_progress",
        )
        .await;
        insert_journal(
            &pool,
            "j2",
            &ago(80),
            "goal",
            "g-other",
            "Not this run → in_progress",
        )
        .await;

        let snapshot = serde_json::json!({
            "promptTitle": "Ship the lease reader",
            "promptContext": SENTINEL,
            "promptDigest": SENTINEL,
            "toolCalls": 14,
            "verificationVerdict": "pending",
            "pendingGate": {"requestId": "r1", "toolName": "Write", "tier": "1"},
        });
        sqlx::query(
            "INSERT INTO harness_run_snapshots (run_id, session_id, project, status, started_at, \
             updated_at, is_terminal, snapshot_json) VALUES ('harness-1', ?, 'alpha', \
             'waiting_gate', ?, ?, 0, ?)",
        )
        .bind(LIVE_COORD)
        .bind(ago(900))
        .bind(ago(5))
        .bind(snapshot.to_string())
        .execute(&pool)
        .await
        .unwrap();

        pool
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    fn assert_no_sentinel<T: Serialize>(value: &T) {
        let json = serde_json::to_string(value).unwrap();
        assert!(
            !json.contains(SENTINEL),
            "output leaked message/detail text: {json}"
        );
    }

    #[test]
    fn the_live_window_boundary_is_inclusive_at_45_seconds() {
        let now = now();
        assert_eq!(classify_lease("turn", &ago(0), now), RunLiveness::Live);
        assert_eq!(classify_lease("turn", &ago(45), now), RunLiveness::Live);
        assert_eq!(classify_lease("idle", &ago(46), now), RunLiveness::Lost);
        assert_eq!(classify_lease("exited", &ago(1), now), RunLiveness::Ended);
        assert_eq!(classify_lease("turn", "not a time", now), RunLiveness::Lost);
        // SQLite's own timestamp shape reads as UTC.
        assert_eq!(
            classify_lease("turn", "2026-09-17 11:59:30", now),
            RunLiveness::Live
        );
    }

    #[test]
    fn truncation_counts_characters_not_bytes() {
        let text = "é".repeat(10);
        let cut = truncate_chars(&text, 4);
        assert_eq!(cut, "éééé…");
        assert_eq!(truncate_chars("short", 10), "short");
    }

    #[test]
    fn repo_display_separates_worktrees_of_one_repo() {
        assert_eq!(
            repo_display("/work/alpha-wt", Some("/work/alpha")),
            "alpha (alpha-wt)"
        );
        assert_eq!(repo_display("/work/alpha", Some("/work/alpha/")), "alpha");
        assert_eq!(repo_display("/work/beta", None), "beta");
    }

    #[test]
    fn human_durations_read_naturally() {
        assert_eq!(human_duration(45), "45s");
        assert_eq!(human_duration(720), "12m");
        assert_eq!(human_duration(3 * 3600 + 300), "3h 5m");
        assert_eq!(human_duration(2 * 86_400 + 4 * 3600), "2d 4h");
    }

    #[tokio::test]
    async fn list_runs_reports_live_and_lost_runs_with_goals_decisions_and_spend() {
        let pool = seeded_pool().await;
        let runs = list_runs(&pool, now(), DEFAULT_RECENT_ENDED).await.unwrap();
        assert_eq!(runs.len(), 2);

        let live = &runs[0];
        assert_eq!(live.lease_id, LIVE_LEASE);
        assert_eq!(live.liveness, RunLiveness::Live);
        assert_eq!(live.repo, "alpha (alpha-wt)");
        assert_eq!(live.title.as_deref(), Some("Ship the lease reader"));
        assert_eq!(live.goals.total, 4, "g1..g4 only, never g-other");
        assert_eq!(live.goals.done, 1);
        assert_eq!(live.goals.in_progress, 1);
        assert_eq!(live.goals.parked, 1);
        assert_eq!(live.goals.waiting_on_you, 2, "parked g4 + g2's open review");
        assert_eq!(live.open_decisions, 1);
        assert!(
            (live.spend_usd - 0.80).abs() < 1e-9,
            "coordinator + both workers, never the unrelated session: {}",
            live.spend_usd
        );
        assert_eq!(live.heartbeat_age_secs, Some(5));
        assert_eq!(live.age_secs, Some(900));

        let lost = &runs[1];
        assert_eq!(lost.lease_id, LOST_LEASE);
        assert_eq!(lost.liveness, RunLiveness::Lost);
        assert_eq!(lost.goals.total, 1);
        assert!((lost.spend_usd - 1.00).abs() < 1e-9);
        assert_no_sentinel(&runs);
    }

    #[tokio::test]
    async fn ended_runs_appear_only_inside_the_requested_window() {
        let pool = empty_pool().await;
        insert_lease(
            &pool,
            "hpl-ended-recent",
            "l1",
            "s1",
            "/work/a",
            None,
            "exited",
            &ago(600),
            Some(&ago(600)),
        )
        .await;
        insert_lease(
            &pool,
            "hpl-ended-old",
            "l2",
            "s2",
            "/work/b",
            None,
            "exited",
            &ago(7200),
            Some(&ago(7200)),
        )
        .await;
        let runs = list_runs(&pool, now(), DEFAULT_RECENT_ENDED).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].lease_id, "hpl-ended-recent");
        assert_eq!(runs[0].liveness, RunLiveness::Ended);
        let wider = list_runs(&pool, now(), std::time::Duration::from_secs(3 * 3600))
            .await
            .unwrap();
        assert_eq!(wider.len(), 2);
    }

    #[tokio::test]
    async fn a_database_without_the_lease_table_reads_as_no_runs() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::session::spectral_schema::init_spectral_db(&pool)
            .await
            .unwrap();
        // The schema chain may already create the table after integration;
        // drop it to simulate an older database.
        sqlx::query("DROP TABLE IF EXISTS harness_process_leases")
            .execute(&pool)
            .await
            .unwrap();
        assert!(list_runs(&pool, now(), DEFAULT_RECENT_ENDED)
            .await
            .unwrap()
            .is_empty());
        assert!(matches!(
            run_status(&pool, None, now()).await.unwrap(),
            RunSelection::NoRuns
        ));
    }

    #[tokio::test]
    async fn run_status_uses_the_single_live_run_when_no_selector_is_given() {
        let pool = seeded_pool().await;
        let RunSelection::Resolved(status) = run_status(&pool, None, now()).await.unwrap() else {
            panic!("exactly one live run must resolve");
        };
        assert_eq!(status.run.lease_id, LIVE_LEASE);
        assert_eq!(status.goals.len(), 4);
        assert!(!status.goals_truncated);

        let g1 = status.goals.iter().find(|g| g.card_id == "g1").unwrap();
        assert_eq!(g1.state, "in_progress");
        assert_eq!(g1.attempt_count, 2);
        assert_eq!(g1.worker_session_id.as_deref(), Some("sess-worker-1"));
        assert_eq!(g1.worker_session_name.as_deref(), Some("worker one"));
        assert_eq!(g1.receipt_state.as_deref(), Some("running"));
        assert_eq!(g1.receipt_heartbeat_age_secs, Some(12));
        assert_eq!(g1.last_tool.as_deref(), Some("developer__shell"));
        assert_eq!(g1.last_tool_age_secs, Some(30));
        let err = g1.last_error.as_deref().unwrap();
        assert_eq!(
            err.chars().count(),
            LAST_ERROR_MAX_CHARS + 1,
            "cut by chars + marker"
        );

        let g2 = status.goals.iter().find(|g| g.card_id == "g2").unwrap();
        assert_eq!(g2.verify_verdict.as_deref(), Some("pass"));
        assert_eq!(g2.review_state.as_deref(), Some("awaiting_approval"));
        assert_eq!(g2.open_decisions, 1);

        let g3 = status.goals.iter().find(|g| g.card_id == "g3").unwrap();
        assert_eq!(g3.landed_commit.as_deref(), Some("0123456789ab"));

        let g4 = status.goals.iter().find(|g| g.card_id == "g4").unwrap();
        assert!(g4.needs_human_attention);
        assert_eq!(g4.attention_reason.as_deref(), Some("budget exhausted"));

        assert_eq!(status.journal.len(), 1, "only rows for this run's goals");
        assert_eq!(status.journal[0].ref_id.as_deref(), Some("g1"));
        let telemetry = status.telemetry.as_ref().unwrap();
        assert_eq!(telemetry.tool_calls, Some(14));
        assert_eq!(telemetry.pending_gate_tool.as_deref(), Some("Write"));
        assert_no_sentinel(&status);
    }

    #[tokio::test]
    async fn run_status_offers_choices_for_zero_or_many_live_runs() {
        // Zero live: the stale lease is the only one.
        let pool = seeded_pool().await;
        sqlx::query("DELETE FROM harness_process_leases WHERE lease_id = ?")
            .bind(LIVE_LEASE)
            .execute(&pool)
            .await
            .unwrap();
        match run_status(&pool, None, now()).await.unwrap() {
            RunSelection::Choose { choices, reason } => {
                assert_eq!(choices.len(), 1);
                assert_eq!(choices[0].liveness, RunLiveness::Lost);
                assert!(reason.contains("No terminal harness run is live"));
            }
            other => panic!("expected choices, got {other:?}"),
        }

        // Many live.
        let pool = seeded_pool().await;
        insert_lease(
            &pool,
            "hpl-live-0003-cccc",
            "life-3",
            "sess-3",
            "/work/gamma",
            None,
            "idle",
            &ago(2),
            None,
        )
        .await;
        match run_status(&pool, None, now()).await.unwrap() {
            RunSelection::Choose { choices, .. } => assert_eq!(choices.len(), 2),
            other => panic!("expected choices, got {other:?}"),
        }

        // No leases at all.
        let pool = empty_pool().await;
        assert!(matches!(
            run_status(&pool, None, now()).await.unwrap(),
            RunSelection::NoRuns
        ));
    }

    #[tokio::test]
    async fn run_status_resolves_every_selector_shape() {
        let pool = seeded_pool().await;
        for (selector, expected) in [
            (LOST_LEASE, LOST_LEASE),
            ("hpl-lost", LOST_LEASE),
            (LOST_COORD, LOST_LEASE),
            ("Refactor beta", LOST_LEASE),
            ("beta", LOST_LEASE),
            ("alpha-wt", LIVE_LEASE),
            ("lease reader", LIVE_LEASE),
        ] {
            match run_status(&pool, Some(selector), now()).await.unwrap() {
                RunSelection::Resolved(status) => {
                    assert_eq!(status.run.lease_id, expected, "selector {selector:?}")
                }
                other => panic!("selector {selector:?} did not resolve: {other:?}"),
            }
        }
        match run_status(&pool, Some("no-such-repo"), now())
            .await
            .unwrap()
        {
            RunSelection::Choose { choices, reason } => {
                assert_eq!(choices.len(), 2);
                assert!(reason.contains("No run matches"));
            }
            other => panic!("expected choices, got {other:?}"),
        }
        // "work" matches both; exactly one is live, so the live one wins.
        match run_status(&pool, Some("work"), now()).await.unwrap() {
            RunSelection::Resolved(status) => assert_eq!(status.run.lease_id, LIVE_LEASE),
            other => panic!("expected the live run, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn attention_collects_decisions_parks_gates_and_lost_runs() {
        let pool = seeded_pool().await;
        let report = attention(&pool, now()).await.unwrap();
        assert_eq!(report.runs_considered, 2);
        let kinds: Vec<AttentionKind> = report.items.iter().map(|i| i.kind).collect();
        assert!(kinds.contains(&AttentionKind::LostRun), "{kinds:?}");
        assert!(kinds.contains(&AttentionKind::Decision));
        assert!(kinds.contains(&AttentionKind::ParkedGoal));
        assert!(kinds.contains(&AttentionKind::PendingGate));
        assert_eq!(
            kinds
                .iter()
                .filter(|k| **k == AttentionKind::ProjectDecision)
                .count(),
            1,
            "a project shared by two runs reports its decisions once"
        );
        let lost = report
            .items
            .iter()
            .find(|i| i.kind == AttentionKind::LostRun)
            .unwrap();
        assert!(lost.summary.contains("terminal exited unexpectedly"));
        assert!(lost.summary.contains("1 goal still in progress"));
        let decision = report
            .items
            .iter()
            .find(|i| i.kind == AttentionKind::Decision)
            .unwrap();
        assert_eq!(decision.goal_title.as_deref(), Some("Add the chat tools"));
        assert_no_sentinel(&report);
    }

    #[tokio::test]
    async fn every_listing_honours_its_limit() {
        let pool = empty_pool().await;
        for i in 0..(MAX_RUNS + 5) {
            insert_lease(
                &pool,
                &format!("hpl-many-{i:04}"),
                "life-many",
                "sess-many",
                "/work/many",
                None,
                "turn",
                &ago(1),
                None,
            )
            .await;
        }
        let runs = list_runs(&pool, now(), DEFAULT_RECENT_ENDED).await.unwrap();
        assert_eq!(runs.len() as i64, MAX_RUNS);

        let pool = empty_pool().await;
        insert_session(&pool, "sess-big", "big run", None).await;
        insert_lease(
            &pool,
            "hpl-big-0001",
            "life-big",
            "sess-big",
            "/work/big",
            None,
            "turn",
            &ago(1),
            None,
        )
        .await;
        for i in 0..(MAX_GOALS_PER_RUN + 10) {
            insert_goal(
                &pool,
                &format!("gb-{i}"),
                "goal",
                "ready",
                serde_json::json!({"dispatched_lifecycle": "life-big"}),
            )
            .await;
        }
        for i in 0..(MAX_JOURNAL_ROWS + 10) {
            insert_journal(
                &pool,
                &format!("jb-{i}"),
                &ago(i + 1),
                "goal",
                "gb-0",
                "moved",
            )
            .await;
        }
        let RunSelection::Resolved(status) = run_status(&pool, None, now()).await.unwrap() else {
            panic!("single live run must resolve");
        };
        assert_eq!(status.goals.len() as i64, MAX_GOALS_PER_RUN);
        assert!(status.goals_truncated);
        assert_eq!(
            status.run.goals.total,
            MAX_GOALS_PER_RUN + 10,
            "counts are not capped"
        );
        assert_eq!(status.journal.len() as i64, MAX_JOURNAL_ROWS);
    }

    #[tokio::test]
    async fn reads_are_scoped_to_the_pool_they_are_given() {
        // The chat tools read the process's own session pool (its Paths root).
        // A run recorded in another database — an eval child rooted with
        // PERMAGENT_PATH_ROOT — is invisible.
        let other_root = seeded_pool().await;
        let own_root = empty_pool().await;
        assert_eq!(
            list_runs(&other_root, now(), DEFAULT_RECENT_ENDED)
                .await
                .unwrap()
                .len(),
            2
        );
        assert!(list_runs(&own_root, now(), DEFAULT_RECENT_ENDED)
            .await
            .unwrap()
            .is_empty());
        assert!(attention(&own_root, now()).await.unwrap().items.is_empty());
    }
}
