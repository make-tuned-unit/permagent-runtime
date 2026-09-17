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
    // Only a compile-time column list is interpolated; the lifecycle id is bound.
    sqlx::query_as::<_, Lease>(sqlx::AssertSqlSafe(format!(
        "SELECT {LEASE_COLUMNS} FROM harness_process_leases \
         WHERE lifecycle_id = ? ORDER BY heartbeat_at DESC LIMIT 8"
    )))
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

/// Why a goal is believed to be owned by a live process other than this one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ForeignOwnerEvidence {
    /// The dispatching lifecycle holds a live harness lease: a terminal run.
    Lease {
        lease: Lease,
        heartbeat_age_secs: i64,
    },
    /// No lease describes the dispatching lifecycle, but the goal's execution
    /// receipt is non-terminal and was heartbeated recently. That beat is
    /// written by the dispatching process's completion tracker, so it is
    /// readable across processes even though `ExecutionReceipt::is_live` is
    /// bound to the reader's own lifecycle.
    ReceiptHeartbeat { heartbeat_age_secs: i64 },
}

/// A live owner of a goal, outside the reading process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LiveForeignOwner {
    /// The goal's `dispatched_lifecycle`, when it carries one.
    pub lifecycle_id: Option<String>,
    pub evidence: ForeignOwnerEvidence,
}

impl LiveForeignOwner {
    /// True when the only evidence is the receipt heartbeat (no lease row).
    pub fn is_receipt_only(&self) -> bool {
        matches!(self.evidence, ForeignOwnerEvidence::ReceiptHeartbeat { .. })
    }

    /// One human-readable line, e.g. `running in terminal "fix auth" at
    /// /repo (pid 4242), heartbeat 7s ago`.
    pub fn describe(&self) -> String {
        match &self.evidence {
            ForeignOwnerEvidence::Lease {
                lease,
                heartbeat_age_secs,
            } => {
                let place = match lease.session_name.as_deref().filter(|n| !n.is_empty()) {
                    Some(name) => format!("\"{name}\" at {}", lease.working_dir),
                    None => lease.working_dir.clone(),
                };
                format!(
                    "running in terminal {place} (pid {}), heartbeat {}s ago",
                    lease.pid,
                    (*heartbeat_age_secs).max(0)
                )
            }
            ForeignOwnerEvidence::ReceiptHeartbeat { heartbeat_age_secs } => format!(
                "owned by another live process (lifecycle {}), worker receipt heartbeat {}s ago",
                self.lifecycle_id.as_deref().unwrap_or("unknown"),
                (*heartbeat_age_secs).max(0)
            ),
        }
    }
}

/// The live foreign owner of a goal, judged from its card metadata, or `None`
/// when the goal belongs to `own_lifecycle` or nothing proves a live owner.
///
/// - A goal whose `dispatched_lifecycle` is `own_lifecycle` is never foreign:
///   the caller's in-process registries already answer for it.
/// - When the dispatching lifecycle has lease rows, the lease decides: live
///   means owned; lost or ended means the owner is gone, whatever the receipt
///   says, so a crashed terminal's goals recover as soon as its lease is stale.
/// - With no lease rows (the daemon, or a CLI build that predates leases), a
///   non-terminal receipt heartbeated within
///   [`DEFAULT_STALENESS_SECS`](crate::agents::platform_extensions::execution_receipt::DEFAULT_STALENESS_SECS)
///   means owned, regardless of lifecycle.
pub async fn live_foreign_owner_at(
    pool: &Pool<Sqlite>,
    metadata: &serde_json::Value,
    own_lifecycle: &str,
    now: DateTime<Utc>,
) -> Option<LiveForeignOwner> {
    use crate::agents::platform_extensions::execution_receipt::{
        ExecutionReceipt, DEFAULT_STALENESS_SECS,
    };

    let dispatched = metadata
        .get("dispatched_lifecycle")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    if dispatched.as_deref() == Some(own_lifecycle) {
        return None;
    }

    if let Some(lifecycle) = dispatched.as_deref() {
        match leases_for_lifecycle(pool, lifecycle).await {
            Ok(leases) if !leases.is_empty() => {
                return leases
                    .into_iter()
                    .find(|lease| classify(lease, now) == LeaseLiveness::Live)
                    .map(|lease| {
                        let heartbeat_age_secs =
                            heartbeat_age_secs(&lease.heartbeat_at, now).unwrap_or(0);
                        LiveForeignOwner {
                            lifecycle_id: dispatched.clone(),
                            evidence: ForeignOwnerEvidence::Lease {
                                lease,
                                heartbeat_age_secs,
                            },
                        }
                    });
            }
            Ok(_) => {}
            Err(error) => {
                tracing::debug!(
                    lifecycle,
                    %error,
                    "harness lease lookup failed; falling back to the execution receipt"
                );
            }
        }
    }

    let receipt = metadata
        .get("execution_receipt")
        .cloned()
        .and_then(|v| serde_json::from_value::<ExecutionReceipt>(v).ok())?;
    if receipt.dispatched_lifecycle == own_lifecycle || receipt.state.is_terminal() {
        return None;
    }
    let age = heartbeat_age_secs(&receipt.last_heartbeat_at, now)?;
    (age <= DEFAULT_STALENESS_SECS).then(|| LiveForeignOwner {
        lifecycle_id: dispatched.or(Some(receipt.dispatched_lifecycle.clone())),
        evidence: ForeignOwnerEvidence::ReceiptHeartbeat {
            heartbeat_age_secs: age,
        },
    })
}

/// True when `card` is owned by a live process other than `own_lifecycle`'s
/// (see [`live_foreign_owner_at`]). Status reads and restart recovery must
/// neither requeue nor redispatch such a goal.
pub async fn goal_owned_by_live_foreign_process(
    pool: &Pool<Sqlite>,
    card: &crate::cards::Card,
    own_lifecycle: &str,
) -> bool {
    live_foreign_owner_at(pool, &card.metadata_json, own_lifecycle, Utc::now())
        .await
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::platform_extensions::execution_receipt::{ExecutionReceipt, ReceiptState};
    use crate::session::SessionManager;

    async fn temp_pool() -> (tempfile::TempDir, Pool<Sqlite>) {
        let dir = tempfile::tempdir().unwrap();
        let sm = SessionManager::new(dir.path().to_path_buf());
        let pool = sm.pool_clone().await.unwrap();
        (dir, pool)
    }

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-17T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + chrono::Duration::seconds(secs)
    }

    async fn insert_lease(
        pool: &Pool<Sqlite>,
        lease_id: &str,
        lifecycle_id: &str,
        phase: &str,
        heartbeat_at: DateTime<Utc>,
    ) {
        sqlx::query(
            "INSERT INTO harness_process_leases (
                lease_id, lifecycle_id, session_id, session_name, working_dir, mode, pid,
                phase, started_at, heartbeat_at
            ) VALUES (?, ?, 'sess-1', 'fix auth', '/work/repo', 'headless', 4242, ?, ?, ?)",
        )
        .bind(lease_id)
        .bind(lifecycle_id)
        .bind(phase)
        .bind(lease_timestamp(
            heartbeat_at - chrono::Duration::seconds(600),
        ))
        .bind(lease_timestamp(heartbeat_at))
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn lease_schema_applies_twice_on_a_migrated_database() {
        let (_dir, pool) = temp_pool().await;
        // The session manager already applied it once while initializing.
        crate::session::spectral_schema::apply_harness_process_leases_schema(&pool)
            .await
            .unwrap();
        crate::session::spectral_schema::apply_harness_process_leases_schema(&pool)
            .await
            .unwrap();
        let objects: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master
             WHERE tbl_name = 'harness_process_leases' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert!(objects.contains(&"harness_process_leases".to_string()));
        assert!(objects.contains(&"idx_harness_leases_phase_hb".to_string()));
        assert!(objects.contains(&"idx_harness_leases_lifecycle".to_string()));

        // A database that reopens (daemon restart / second process) runs the
        // chain again without error and keeps its rows.
        insert_lease(&pool, "l-1", "life-1", "idle", at(0)).await;
        crate::session::spectral_schema::apply_harness_process_leases_schema(&pool)
            .await
            .unwrap();
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM harness_process_leases")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[tokio::test]
    async fn lease_schema_is_part_of_a_fresh_database() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::session::spectral_schema::init_spectral_db(&pool)
            .await
            .unwrap();
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name = 'harness_process_leases'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn classify_is_live_through_45_seconds_and_lost_after() {
        let now = at(100);
        for phase in ["starting", "turn", "idle", "draining"] {
            assert_eq!(
                classify_parts(phase, &lease_timestamp(at(100 - 45)), now),
                LeaseLiveness::Live,
                "{phase} at exactly the window is live"
            );
            assert_eq!(
                classify_parts(phase, &lease_timestamp(at(100 - 46)), now),
                LeaseLiveness::Lost,
                "{phase} one second past the window is lost"
            );
        }
        assert_eq!(
            classify_parts("exited", &lease_timestamp(now), now),
            LeaseLiveness::Ended
        );
        assert_eq!(
            classify_parts("exited", &lease_timestamp(at(-9999)), now),
            LeaseLiveness::Ended,
            "exited is ended however stale"
        );
        assert_eq!(
            classify_parts("idle", "not a timestamp", now),
            LeaseLiveness::Lost,
            "an unreadable heartbeat never reads as live"
        );
        assert_eq!(
            classify_parts("idle", &lease_timestamp(at(105)), now),
            LeaseLiveness::Live,
            "a heartbeat slightly ahead of the reader's clock is live"
        );
    }

    #[tokio::test]
    async fn live_lease_for_lifecycle_returns_only_a_live_row() {
        let (_dir, pool) = temp_pool().await;
        let now = Utc::now();
        insert_lease(
            &pool,
            "old",
            "life-live",
            "exited",
            now - chrono::Duration::hours(2),
        )
        .await;
        insert_lease(
            &pool,
            "live",
            "life-live",
            "turn",
            now - chrono::Duration::seconds(5),
        )
        .await;
        insert_lease(
            &pool,
            "stale",
            "life-lost",
            "draining",
            now - chrono::Duration::seconds(60),
        )
        .await;
        insert_lease(&pool, "done", "life-ended", "exited", now).await;

        let live = live_lease_for_lifecycle(&pool, "life-live").await.unwrap();
        assert_eq!(live.lease_id, "live");
        assert_eq!(live.pid, 4242);
        assert_eq!(live.session_name.as_deref(), Some("fix auth"));
        assert!(live_lease_for_lifecycle(&pool, "life-lost").await.is_none());
        assert!(live_lease_for_lifecycle(&pool, "life-ended")
            .await
            .is_none());
        assert!(live_lease_for_lifecycle(&pool, "never-seen")
            .await
            .is_none());
        assert_eq!(
            live_lease_for_lifecycle_at(&pool, "life-live", now + chrono::Duration::seconds(51))
                .await,
            None,
            "the same row is lost once the reader's clock passes the window"
        );
    }

    fn receipt_meta(
        lifecycle: &str,
        heartbeat: DateTime<Utc>,
        terminal: bool,
    ) -> serde_json::Value {
        let mut receipt = ExecutionReceipt::new(
            "permagent",
            "worker-1",
            serde_json::json!({}),
            lifecycle,
            heartbeat.to_rfc3339(),
            1,
        );
        if terminal {
            receipt.finalize(ReceiptState::Completed, heartbeat.to_rfc3339());
        }
        serde_json::json!({
            "dispatched_lifecycle": lifecycle,
            "execution_receipt": receipt,
        })
    }

    #[tokio::test]
    async fn foreign_owner_follows_the_lease_then_the_receipt() {
        let (_dir, pool) = temp_pool().await;
        let now = Utc::now();

        // Our own lifecycle is never foreign, however fresh.
        let own = receipt_meta("life-own", now, false);
        assert!(live_foreign_owner_at(&pool, &own, "life-own", now)
            .await
            .is_none());

        // A live lease for the dispatching lifecycle: owned, described.
        insert_lease(
            &pool,
            "t1",
            "life-terminal",
            "draining",
            now - chrono::Duration::seconds(7),
        )
        .await;
        let meta = serde_json::json!({ "dispatched_lifecycle": "life-terminal" });
        let owner = live_foreign_owner_at(&pool, &meta, "life-daemon", now)
            .await
            .expect("a live lease owns the goal");
        assert!(!owner.is_receipt_only());
        let line = owner.describe();
        assert!(line.contains("running in terminal"), "{line}");
        assert!(
            line.contains("/work/repo") && line.contains("pid 4242"),
            "{line}"
        );
        assert!(line.contains("heartbeat 7s ago"), "{line}");

        // A stale lease decides "gone", even when the receipt is still fresh.
        insert_lease(
            &pool,
            "t2",
            "life-crashed",
            "turn",
            now - chrono::Duration::seconds(90),
        )
        .await;
        let crashed = receipt_meta("life-crashed", now - chrono::Duration::seconds(10), false);
        assert!(live_foreign_owner_at(&pool, &crashed, "life-daemon", now)
            .await
            .is_none());

        // No lease row at all: a fresh non-terminal receipt vouches.
        let daemon_goal = receipt_meta(
            "life-old-daemon",
            now - chrono::Duration::seconds(20),
            false,
        );
        let owner = live_foreign_owner_at(&pool, &daemon_goal, "life-cli", now)
            .await
            .expect("a fresh foreign receipt owns the goal");
        assert!(owner.is_receipt_only());
        assert!(owner.describe().contains("life-old-daemon"));

        // ...but not once stale, and never when terminal.
        let stale = receipt_meta(
            "life-old-daemon",
            now - chrono::Duration::seconds(91),
            false,
        );
        assert!(live_foreign_owner_at(&pool, &stale, "life-cli", now)
            .await
            .is_none());
        let finished = receipt_meta("life-old-daemon", now, true);
        assert!(live_foreign_owner_at(&pool, &finished, "life-cli", now)
            .await
            .is_none());

        // Nothing to go on at all.
        assert!(
            live_foreign_owner_at(&pool, &serde_json::json!({}), "life-cli", now)
                .await
                .is_none()
        );
    }
}
