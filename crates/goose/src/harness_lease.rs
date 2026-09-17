//! Coding-harness process leases: who is running a harness right now.
//!
//! A terminal `permagent run --recipe permagent-coding` process writes one
//! row to `harness_process_leases` (see
//! [`apply_harness_process_leases_schema`](crate::session::spectral_schema::apply_harness_process_leases_schema))
//! and heartbeats it for its whole lifetime, directly against the shared
//! database: no daemon and no token are involved. Every other process reads
//! the row to answer two questions it could not answer before:
//!
//! - **Is that run alive?** [`classify`] is the one liveness rule. In-memory
//!   registries (`GOAL_WORKERS`, `AgentManager`) only ever describe the
//!   process that owns them, and an execution receipt's `is_live` is bound to
//!   the reader's own lifecycle, so without a lease a daemon cannot tell a
//!   live terminal run from a dead one.
//! - **Which goals does it own?** `lifecycle_id` is the writer's
//!   `daemon_lifecycle_id()`, the same value stamped as `dispatched_lifecycle`
//!   on the goal cards that process dispatches.
//!
//! Crash semantics are deliberately passive: a killed process never writes
//! `exited`, so its row goes stale and reads as [`LeaseLiveness::Lost`]. The
//! reader never needs the writer's cooperation to notice a death.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, Pool, Sqlite};

/// A lease whose heartbeat is at most this old (and whose phase is not
/// `exited`) is live. Three missed beats at [`LEASE_HEARTBEAT_INTERVAL_SECS`].
pub const LEASE_LIVE_WINDOW_SECS: i64 = 45;

/// How often the owning process writes `heartbeat_at`.
pub const LEASE_HEARTBEAT_INTERVAL_SECS: u64 = 15;

/// Minimum spacing between two `last_activity` writes from one process.
pub const LEASE_ACTIVITY_MIN_GAP_SECS: u64 = 5;

/// Longest `last_activity` value a writer stores (characters, not bytes).
pub const LEASE_ACTIVITY_MAX_CHARS: usize = 80;

/// The lifecycle phase a process last reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LeasePhase {
    /// Row written; the first turn has not started.
    Starting,
    /// A model turn is in flight.
    Turn,
    /// Between turns (an interactive session waiting on its user, or a
    /// headless run between its turn and its drain).
    Idle,
    /// Headless drain: the coordinator turn ended and the process is waiting
    /// on dispatched goal workers before it exits.
    Draining,
    /// The process exited normally.
    Exited,
}

impl LeasePhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            LeasePhase::Starting => "starting",
            LeasePhase::Turn => "turn",
            LeasePhase::Idle => "idle",
            LeasePhase::Draining => "draining",
            LeasePhase::Exited => "exited",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "starting" => Some(LeasePhase::Starting),
            "turn" => Some(LeasePhase::Turn),
            "idle" => Some(LeasePhase::Idle),
            "draining" => Some(LeasePhase::Draining),
            "exited" => Some(LeasePhase::Exited),
            _ => None,
        }
    }
}

/// What a reader should believe about the process behind a lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaseLiveness {
    /// Not exited and heartbeated within [`LEASE_LIVE_WINDOW_SECS`].
    Live,
    /// Not exited, but the heartbeat is stale or unreadable: the process most
    /// likely crashed or was killed.
    Lost,
    /// The process wrote `exited`.
    Ended,
}

/// One `harness_process_leases` row.
#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize)]
pub struct Lease {
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

const LEASE_COLUMNS: &str = "lease_id, lifecycle_id, session_id, session_name, working_dir, \
     repo_root, project_id, mode, recipe, pid, phase, started_at, heartbeat_at, \
     last_activity_at, last_activity, ended_at, exit_status";

/// The canonical timestamp format for lease columns: RFC3339, UTC, `Z`,
/// millisecond precision. Fixed width, so text comparison orders correctly.
pub fn lease_timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Seconds since `heartbeat_at`, or `None` when it does not parse. A heartbeat
/// slightly in the future (clock skew between writes) reads as a negative age.
pub fn heartbeat_age_secs(heartbeat_at: &str, now: DateTime<Utc>) -> Option<i64> {
    DateTime::parse_from_rfc3339(heartbeat_at)
        .ok()
        .map(|beat| (now - beat.with_timezone(&Utc)).num_seconds())
}

/// The liveness rule, over the two columns it depends on.
pub fn classify_parts(phase: &str, heartbeat_at: &str, now: DateTime<Utc>) -> LeaseLiveness {
    if phase == LeasePhase::Exited.as_str() {
        return LeaseLiveness::Ended;
    }
    match heartbeat_age_secs(heartbeat_at, now) {
        Some(age) if age <= LEASE_LIVE_WINDOW_SECS => LeaseLiveness::Live,
        _ => LeaseLiveness::Lost,
    }
}

/// The liveness rule: live = not exited and heartbeat within
/// [`LEASE_LIVE_WINDOW_SECS`]; lost = not exited and stale; ended = exited.
pub fn classify(lease: &Lease, now: DateTime<Utc>) -> LeaseLiveness {
    classify_parts(&lease.phase, &lease.heartbeat_at, now)
}

/// Every lease a lifecycle has written, newest heartbeat first. Bounded: one
/// process normally writes exactly one.
pub async fn leases_for_lifecycle(
    pool: &Pool<Sqlite>,
    lifecycle_id: &str,
) -> Result<Vec<Lease>, sqlx::Error> {
    sqlx::query_as::<_, Lease>(&format!(
        "SELECT {LEASE_COLUMNS} FROM harness_process_leases \
         WHERE lifecycle_id = ? ORDER BY heartbeat_at DESC LIMIT 8"
    ))
    .bind(lifecycle_id)
    .fetch_all(pool)
    .await
}

/// The live lease for `lifecycle_id`, if that process is running now. A read
/// error (including a database that predates the table) is `None`: callers use
/// this to decide whether to hold off on recovery, and "no evidence of a live
/// owner" is the answer that preserves the pre-lease behaviour.
pub async fn live_lease_for_lifecycle(pool: &Pool<Sqlite>, lifecycle_id: &str) -> Option<Lease> {
    live_lease_for_lifecycle_at(pool, lifecycle_id, Utc::now()).await
}

/// [`live_lease_for_lifecycle`] with an explicit clock.
pub async fn live_lease_for_lifecycle_at(
    pool: &Pool<Sqlite>,
    lifecycle_id: &str,
    now: DateTime<Utc>,
) -> Option<Lease> {
    match leases_for_lifecycle(pool, lifecycle_id).await {
        Ok(leases) => leases
            .into_iter()
            .find(|lease| classify(lease, now) == LeaseLiveness::Live),
        Err(error) => {
            tracing::debug!(
                lifecycle_id,
                %error,
                "harness lease lookup failed; treating the lifecycle as having no live lease"
            );
            None
        }
    }
}

/// The ACTIVE project whose `root_path` equals `working_dir` or contains it,
/// longest root winning: the same precedence
/// [`crate::cards::goal_cards_for_project_matching_cwd`] uses. `None` when
/// nothing matches; never a guess.
pub async fn project_id_for_working_dir(
    pool: &Pool<Sqlite>,
    working_dir: &std::path::Path,
) -> Option<String> {
    let canonical = working_dir
        .canonicalize()
        .unwrap_or_else(|_| working_dir.to_path_buf());
    let dir = canonical.to_string_lossy().into_owned();
    crate::projects::list_projects(pool, Some("active"))
        .await
        .ok()?
        .into_iter()
        .filter_map(|project| {
            let root = project.root_path.clone()?;
            let matches = !root.is_empty() && (dir == root || dir.starts_with(&format!("{root}/")));
            matches.then_some((root.len(), project.id))
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, id)| id)
}
