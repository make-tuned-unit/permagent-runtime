//! The terminal side of `harness_process_leases`: this process says it is
//! running a coding harness, and keeps saying so until it exits.
//!
//! WHY A LEASE AND NOT THE HARNESS-RUN HEARTBEAT. `spend_announce`'s
//! heartbeat POSTs to the daemon, only while a turn is in flight, and only
//! when the daemon is up and its token readable. That leaves a headless run's
//! goal drain and an interactive session's idle time invisible, and gives
//! other processes no way to tell a live terminal from a dead one. The lease
//! is a direct write to the shared database through the session manager's
//! pool, heartbeated by a task that lives as long as the process does.
//!
//! ISOLATION. The pool comes from the session manager, which resolves its
//! database through `Paths` and therefore honours `PERMAGENT_PATH_ROOT`: an
//! eval run writes its lease into its own root, never the operator's DB.
//!
//! CRASH SEMANTICS. Nothing here runs on a crash. A killed process leaves its
//! row non-exited with a stale heartbeat, which readers classify as lost
//! (`permagent::harness_lease::classify`). Only a normal exit writes `exited`.
//!
//! Every write is a single autocommit statement and every failure is logged
//! and swallowed: liveness reporting must never fail or stall a run.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use permagent::harness_lease::{
    lease_timestamp, LeasePhase, LEASE_ACTIVITY_MAX_CHARS, LEASE_ACTIVITY_MIN_GAP_SECS,
    LEASE_HEARTBEAT_INTERVAL_SECS,
};
use permagent::session::SessionManager;
use sqlx::{Pool, Sqlite};

/// Exited leases older than this are pruned when a new lease is written.
const EXITED_LEASE_RETENTION_DAYS: i64 = 7;

/// Upper bound on the `git rev-parse` used to find the repo root at start.
const REPO_ROOT_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Wall clock used for every timestamp this module writes. Injectable so tests
/// can observe a heartbeat without waiting on real time.
pub type LeaseClock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// Cadences and clock for one lease.
#[derive(Clone)]
pub struct LeaseTiming {
    pub heartbeat_interval: Duration,
    pub activity_min_gap: Duration,
    pub clock: LeaseClock,
}

impl Default for LeaseTiming {
    fn default() -> Self {
        Self {
            heartbeat_interval: Duration::from_secs(LEASE_HEARTBEAT_INTERVAL_SECS),
            activity_min_gap: Duration::from_secs(LEASE_ACTIVITY_MIN_GAP_SECS),
            clock: Arc::new(Utc::now),
        }
    }
}

/// How the harness was launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseMode {
    Headless,
    Interactive,
}

impl LeaseMode {
    pub fn from_interactive(interactive: bool) -> Self {
        if interactive {
            LeaseMode::Interactive
        } else {
            LeaseMode::Headless
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            LeaseMode::Headless => "headless",
            LeaseMode::Interactive => "interactive",
        }
    }
}

/// Everything the lease row records about its process at start.
#[derive(Debug, Clone)]
pub struct LeaseStart {
    pub lifecycle_id: String,
    pub session_id: String,
    pub session_name: Option<String>,
    pub working_dir: String,
    pub repo_root: Option<String>,
    pub project_id: Option<String>,
    pub mode: LeaseMode,
    pub recipe: Option<String>,
    pub pid: u32,
}

/// Only a coding-harness session writes a lease. Every other CLI session
/// (plain chat, `permagent term`, a non-harness recipe) writes nothing.
pub fn wants_harness_lease(coding_harness: bool) -> bool {
    coding_harness
}

/// A handle to this process's lease row. Cheap to clone; the heartbeat task
/// stops when [`HarnessLease::finish`] runs or the last handle is dropped.
#[derive(Clone)]
pub struct HarnessLease {
    inner: Arc<LeaseInner>,
}

struct LeaseInner {
    pool: Pool<Sqlite>,
    lease_id: String,
    timing: LeaseTiming,
    last_activity_claim: Mutex<Option<Instant>>,
    finished: AtomicBool,
    heartbeat: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Drop for LeaseInner {
    fn drop(&mut self) {
        if let Some(handle) = self
            .heartbeat
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            handle.abort();
        }
    }
}

impl HarnessLease {
    /// Start the lease for a CLI session when it is a coding harness: resolve
    /// the session's name, working dir, repo root and project, insert the row
    /// and start the heartbeat. `None` for a non-harness session (no query at
    /// all) or when the row could not be written.
    pub async fn start_for_session(
        session_manager: &SessionManager,
        session_id: &str,
        coding_harness: bool,
        interactive: bool,
        recipe_title: Option<&str>,
        timing: LeaseTiming,
    ) -> Option<Self> {
        if !wants_harness_lease(coding_harness) {
            return None;
        }
        let pool = match session_manager.pool_clone().await {
            Ok(pool) => pool,
            Err(error) => {
                tracing::debug!(%error, "harness lease: no database pool; not writing a lease");
                return None;
            }
        };
        let session = match session_manager.get_session(session_id, false).await {
            Ok(session) => session,
            Err(error) => {
                tracing::debug!(%error, session_id, "harness lease: session lookup failed");
                return None;
            }
        };
        let repo_root = git_toplevel(&session.working_dir).await;
        let project_id =
            permagent::harness_lease::project_id_for_working_dir(&pool, &session.working_dir).await;
        let start = LeaseStart {
            lifecycle_id:
                permagent::agents::platform_extensions::orchestrator::daemon_lifecycle_id()
                    .to_string(),
            session_id: session_id.to_string(),
            session_name: Some(session.name.clone()).filter(|name| !name.is_empty()),
            working_dir: session.working_dir.to_string_lossy().into_owned(),
            repo_root,
            project_id,
            mode: LeaseMode::from_interactive(interactive),
            recipe: recipe_title.map(str::to_string),
            pid: std::process::id(),
        };
        Self::start(pool, start, timing).await
    }

    /// Insert the row in phase `starting` and spawn the heartbeat.
    pub async fn start(pool: Pool<Sqlite>, start: LeaseStart, timing: LeaseTiming) -> Option<Self> {
        let now = (timing.clock)();
        let stamp = lease_timestamp(now);
        let lease_id = uuid::Uuid::now_v7().to_string();

        let cutoff = lease_timestamp(now - chrono::Duration::days(EXITED_LEASE_RETENTION_DAYS));
        if let Err(error) = sqlx::query(
            "DELETE FROM harness_process_leases WHERE phase = 'exited' AND ended_at < ?",
        )
        .bind(&cutoff)
        .execute(&pool)
        .await
        {
            tracing::debug!(%error, "harness lease: pruning old exited leases failed");
        }

        let inserted = sqlx::query(
            "INSERT INTO harness_process_leases (
                lease_id, lifecycle_id, session_id, session_name, working_dir, repo_root,
                project_id, mode, recipe, pid, phase, started_at, heartbeat_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&lease_id)
        .bind(&start.lifecycle_id)
        .bind(&start.session_id)
        .bind(&start.session_name)
        .bind(&start.working_dir)
        .bind(&start.repo_root)
        .bind(&start.project_id)
        .bind(start.mode.as_str())
        .bind(&start.recipe)
        .bind(i64::from(start.pid))
        .bind(LeasePhase::Starting.as_str())
        .bind(&stamp)
        .bind(&stamp)
        .execute(&pool)
        .await;
        if let Err(error) = inserted {
            tracing::warn!(%error, session_id = %start.session_id, "harness lease: insert failed");
            return None;
        }

        let lease = Self {
            inner: Arc::new(LeaseInner {
                pool,
                lease_id,
                timing,
                last_activity_claim: Mutex::new(None),
                finished: AtomicBool::new(false),
                heartbeat: Mutex::new(None),
            }),
        };
        let handle = tokio::spawn(heartbeat_loop(
            lease.inner.pool.clone(),
            lease.inner.lease_id.clone(),
            lease.inner.timing.clone(),
        ));
        *lease
            .inner
            .heartbeat
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(handle);
        Some(lease)
    }

    pub fn lease_id(&self) -> &str {
        &self.inner.lease_id
    }

    /// Record a phase change (`turn`, `idle`, `draining`). Refreshes the
    /// heartbeat in the same statement. Never moves a row out of `exited`, and
    /// `exited` itself is written only by [`HarnessLease::finish`].
    pub async fn set_phase(&self, phase: LeasePhase) {
        if phase == LeasePhase::Exited || self.inner.finished.load(Ordering::SeqCst) {
            return;
        }
        let stamp = lease_timestamp((self.inner.timing.clock)());
        if let Err(error) = sqlx::query(
            "UPDATE harness_process_leases SET phase = ?, heartbeat_at = ?
             WHERE lease_id = ? AND phase != 'exited'",
        )
        .bind(phase.as_str())
        .bind(&stamp)
        .bind(&self.inner.lease_id)
        .execute(&self.inner.pool)
        .await
        {
            tracing::debug!(%error, phase = phase.as_str(), "harness lease: phase write failed");
        }
    }

    /// Note that a tool was called, by NAME only. At most one write per
    /// `activity_min_gap`; the write itself is detached so the reply stream
    /// never waits on the database.
    pub fn note_tool_activity(&self, tool_name: &str) {
        if !self.claim_activity_slot() {
            return;
        }
        let lease = self.clone();
        let tool_name = tool_name.to_string();
        tokio::spawn(async move {
            lease.write_activity(&tool_name).await;
        });
    }

    /// [`HarnessLease::note_tool_activity`], awaited. Returns whether a write
    /// was attempted (false when throttled or finished).
    pub async fn record_tool_activity(&self, tool_name: &str) -> bool {
        if !self.claim_activity_slot() {
            return false;
        }
        self.write_activity(tool_name).await;
        true
    }

    fn claim_activity_slot(&self) -> bool {
        if self.inner.finished.load(Ordering::SeqCst) {
            return false;
        }
        let mut last = self
            .inner
            .last_activity_claim
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Instant::now();
        if let Some(previous) = *last {
            if now.duration_since(previous) < self.inner.timing.activity_min_gap {
                return false;
            }
        }
        *last = Some(now);
        true
    }

    async fn write_activity(&self, tool_name: &str) {
        let name: String = tool_name.chars().take(LEASE_ACTIVITY_MAX_CHARS).collect();
        let stamp = lease_timestamp((self.inner.timing.clock)());
        if let Err(error) = sqlx::query(
            "UPDATE harness_process_leases SET last_activity = ?, last_activity_at = ?
             WHERE lease_id = ? AND phase != 'exited'",
        )
        .bind(&name)
        .bind(&stamp)
        .bind(&self.inner.lease_id)
        .execute(&self.inner.pool)
        .await
        {
            tracing::debug!(%error, "harness lease: activity write failed");
        }
    }

    /// Normal exit: stop the heartbeat and write `exited`, `ended_at` and
    /// `exit_status`. Idempotent; only the first call writes.
    pub async fn finish(&self, exit_status: &str) {
        if self.inner.finished.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(handle) = self
            .inner
            .heartbeat
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            handle.abort();
        }
        let stamp = lease_timestamp((self.inner.timing.clock)());
        if let Err(error) = sqlx::query(
            "UPDATE harness_process_leases
             SET phase = 'exited', ended_at = ?, heartbeat_at = ?, exit_status = ?
             WHERE lease_id = ? AND phase != 'exited'",
        )
        .bind(&stamp)
        .bind(&stamp)
        .bind(exit_status)
        .bind(&self.inner.lease_id)
        .execute(&self.inner.pool)
        .await
        {
            tracing::warn!(%error, "harness lease: exit write failed; the row will read as lost");
        }
    }
}

/// One `UPDATE` per interval for as long as the task lives. Deliberately
/// independent of turns: the headless drain and interactive idle time are
/// exactly the stretches the per-turn heartbeat never covered.
async fn heartbeat_loop(pool: Pool<Sqlite>, lease_id: String, timing: LeaseTiming) {
    let mut ticker = tokio::time::interval(timing.heartbeat_interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick completes immediately; the insert already stamped it.
    ticker.tick().await;
    loop {
        ticker.tick().await;
        let stamp = lease_timestamp((timing.clock)());
        if let Err(error) = sqlx::query(
            "UPDATE harness_process_leases SET heartbeat_at = ?
             WHERE lease_id = ? AND phase != 'exited'",
        )
        .bind(&stamp)
        .bind(&lease_id)
        .execute(&pool)
        .await
        {
            tracing::debug!(%error, "harness lease: heartbeat write failed");
        }
    }
}

/// `git rev-parse --show-toplevel` for `dir`, bounded; `None` outside a repo
/// or on any failure.
async fn git_toplevel(dir: &std::path::Path) -> Option<String> {
    let output = tokio::time::timeout(
        REPO_ROOT_PROBE_TIMEOUT,
        tokio::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .arg("rev-parse")
            .arg("--show-toplevel")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!root.is_empty()).then_some(root)
}
