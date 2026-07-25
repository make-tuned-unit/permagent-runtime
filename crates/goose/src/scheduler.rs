use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, FixedOffset, Local, Utc};
use futures::future::FutureExt as _;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_cron_scheduler::{job::JobId, Job, JobScheduler as TokioJobScheduler};
use tokio_util::sync::CancellationToken;

/// Self-knowledge descriptor for the Scheduler worker. Co-located with the
/// worker it describes; aggregated by `crate::agents::self_knowledge`. Queryable
/// — live job count is merged into the brief via `list_scheduled_jobs`.
pub const SELF_KNOWLEDGE_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "scheduler",
        display_name: "Scheduler",
        category: crate::agents::self_knowledge::FeatureCategory::Worker,
        what_it_does: "Runs saved recipes and reminders in the background on cron, one-time, or interval schedules — retrying transient failures, recovering runs missed during downtime, and escalating repeated failures to the Decision Inbox",
        why_it_matters:
            "Lets you promise recurring or future work and actually deliver it without the user re-asking",
        state_source: crate::agents::self_knowledge::StateSource::Queryable,
        // Queryable → the cleanest read-back loop in the tour: HasScheduledJob is
        // visible directly in the brief (the Scheduler line goes 0 → 1).
        teaching: &[
            crate::agents::self_knowledge::TeachingStep {
                title: "Open Automate",
                body: "Show them where recurring automations and reminders live.",
                open_surface: Some(crate::agents::self_knowledge::SurfaceRef {
                    tab: "Automate",
                    section: None,
                }),
                confirm: None,
            },
            crate::agents::self_knowledge::TeachingStep {
                title: "Schedule something real",
                body: "Offer to set up a simple recurring job — a daily digest, a weekly check-in — and create it for them so they see it actually works.",
                open_surface: None,
                confirm: Some(crate::agents::self_knowledge::ConfirmCheck::HasScheduledJob),
            },
        ],
    };

/// Self-knowledge descriptor for the "agents at work" run roster — the
/// run-visibility surface on the Automate tab (served by `/api/runs`). A Surface
/// (Static) because it is a viewing/control surface, not a queryable worker.
pub const RUN_ROSTER_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "run_roster",
        display_name: "Agents at work",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does: "A live roster on the Automate tab of everything running or recently active — background workers, scheduled jobs with their run status, and active agent sessions — each with a status dot, what it is doing, when it last acted, and a one-click stop for anything interruptible",
        why_it_matters:
            "Long-running and parallel agent work used to be invisible; this is where the user watches it happen and stops a runaway or stuck run — when they ask what you are doing right now, point them here",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[],
    };

use crate::agents::AgentEvent;
use crate::agents::{Agent, SessionConfig};
use crate::config::paths::Paths;
use crate::config::{resolve_extensions_for_new_session, Config};
use crate::conversation::message::Message;
use crate::conversation::Conversation;
#[cfg(feature = "telemetry")]
use crate::posthog;
use crate::providers::create;
use crate::recipe::Recipe;
use crate::scheduler_trait::SchedulerTrait;
use crate::session::session_manager::SessionType;
use crate::session::{Session, SessionManager};

type RunningTasksMap = HashMap<String, CancellationToken>;
type JobsMap = HashMap<String, (JobId, ScheduledJob)>;
/// The boxed future a fired scheduled task returns. Aliased so the shared
/// closure can name its return type (needed to drive the `dyn Future` unsizing
/// coercion) without an inline complex type.
type ScheduledTaskFuture = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

pub fn get_default_scheduler_storage_path() -> Result<PathBuf, io::Error> {
    let data_dir = Paths::data_dir();
    fs::create_dir_all(&data_dir)?;
    Ok(data_dir.join("schedule.json"))
}

pub fn get_default_scheduled_recipes_dir() -> Result<PathBuf, SchedulerError> {
    let data_dir = Paths::data_dir();
    let recipes_dir = data_dir.join("scheduled_recipes");
    fs::create_dir_all(&recipes_dir).map_err(SchedulerError::StorageError)?;
    Ok(recipes_dir)
}

#[derive(Debug)]
pub enum SchedulerError {
    JobIdExists(String),
    JobNotFound(String),
    StorageError(io::Error),
    RecipeLoadError(String),
    AgentSetupError(String),
    PersistError(String),
    CronParseError(String),
    InvalidScheduleSpec(String),
    SchedulerInternalError(String),
    AnyhowError(anyhow::Error),
}

impl std::fmt::Display for SchedulerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchedulerError::JobIdExists(id) => write!(f, "Job ID '{}' already exists.", id),
            SchedulerError::JobNotFound(id) => write!(f, "Job ID '{}' not found.", id),
            SchedulerError::StorageError(e) => write!(f, "Storage error: {}", e),
            SchedulerError::RecipeLoadError(e) => write!(f, "Recipe load error: {}", e),
            SchedulerError::AgentSetupError(e) => write!(f, "Agent setup error: {}", e),
            SchedulerError::PersistError(e) => write!(f, "Failed to persist schedules: {}", e),
            SchedulerError::CronParseError(e) => write!(f, "Invalid cron string: {}", e),
            SchedulerError::InvalidScheduleSpec(e) => write!(f, "Invalid schedule spec: {}", e),
            SchedulerError::SchedulerInternalError(e) => {
                write!(f, "Scheduler internal error: {}", e)
            }
            SchedulerError::AnyhowError(e) => write!(f, "Scheduler operation failed: {}", e),
        }
    }
}

impl std::error::Error for SchedulerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SchedulerError::StorageError(e) => Some(e),
            SchedulerError::AnyhowError(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<io::Error> for SchedulerError {
    fn from(err: io::Error) -> Self {
        SchedulerError::StorageError(err)
    }
}

impl From<serde_json::Error> for SchedulerError {
    fn from(err: serde_json::Error) -> Self {
        SchedulerError::PersistError(err.to_string())
    }
}

impl From<anyhow::Error> for SchedulerError {
    fn from(err: anyhow::Error) -> Self {
        SchedulerError::AnyhowError(err)
    }
}

/// Outcome of a job's most recent fire. Serialized snake_case into
/// `schedule.json` and the `/schedule/list` payload for the Automate tab.
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleRunStatus {
    /// The run completed successfully.
    Ok,
    /// The run failed (after any retries were exhausted).
    Error,
    /// The run was intentionally not executed (e.g. paused at fire time).
    Skipped,
    /// A scheduled fire was due during downtime and did not run.
    Missed,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default, utoipa::ToSchema)]
pub struct ScheduledJob {
    pub id: String,
    pub source: String,
    /// Cron expression (5- or 6-field). The default schedule kind. Empty when
    /// the job is a one-time (`at`) or interval (`every_seconds`) job.
    #[serde(default)]
    pub cron: String,
    pub last_run: Option<DateTime<Utc>>,
    #[serde(default)]
    pub currently_running: bool,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub current_session_id: Option<String>,
    #[serde(default)]
    pub process_start_time: Option<DateTime<Utc>>,
    /// Worker persona key from agent.yaml workers map.
    /// When set, the scheduled run uses the worker's identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_persona: Option<String>,

    // ── Reliability fields (all serde-default → old schedule.json loads clean) ──
    /// Total number of times this job has completed a fire (success or failure).
    #[serde(default)]
    pub run_count: u64,
    /// Retry attempts spent on the CURRENT failing fire; reset to 0 on success.
    #[serde(default)]
    pub retry_count: u32,
    /// Max automatic retries per fire. Default 0 preserves today's behavior
    /// (a failure is terminal, no retry).
    #[serde(default)]
    pub max_retries: u32,
    /// Outcome of the most recent fire. `None` = has never fired.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_status: Option<ScheduleRunStatus>,
    /// Human-readable error from the most recent failed fire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,

    // ── Richer schedule kinds (serde-default None → existing cron jobs unchanged) ──
    /// One-time fire at this instant. Mutually exclusive with cron/every_seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<DateTime<Utc>>,
    /// Fixed interval in seconds. Mutually exclusive with cron/at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_seconds: Option<u64>,
    /// Timezone for cron evaluation. `None` = system local (today's behavior).
    /// Accepts "UTC"/"Z" or a fixed offset ("+05:30", "-08:00"); IANA names fall
    /// back to local (see `resolve_cron_timezone`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,

    // ── Starter recipe versioning fields ──
    /// Non-null for starter recipes (e.g. "storage-insights", "workspace-snapshot").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starter_id: Option<String>,
    /// Embedded YAML version at install/upgrade time (e.g. "2.0.0").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starter_version: Option<String>,
    /// SHA-256 of the YAML content we last wrote to disk (install or upgrade).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starter_content_hash: Option<String>,
    /// True if the user has manually edited this starter recipe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_customized: Option<bool>,
}

/// Reliability + kind constants. Retry backoff is exponential and capped so a
/// flapping job cannot storm.
const RETRY_BASE_SECS: u64 = 10;
const RETRY_MAX_SECS: u64 = 300;
/// Interval bounds (seconds): reject 0/absurd values. One year ceiling.
const MIN_INTERVAL_SECS: u64 = 1;
const MAX_INTERVAL_SECS: u64 = 31_536_000;

/// Whether a terminal job failure should escalate to the Decision Inbox.
/// Escalate only when the user configured retries (`max_retries > 0`) AND this is
/// the first failure of a streak (`prior_status` wasn't already `Error`). So a
/// job that fails every fire escalates once (not per-fire spam), and default
/// no-retry jobs never escalate — preserving today's behavior exactly.
fn should_escalate_failure(max_retries: u32, prior_status: Option<ScheduleRunStatus>) -> bool {
    max_retries > 0 && prior_status != Some(ScheduleRunStatus::Error)
}

/// Exponential backoff for retry `attempt` (1-based), capped at `RETRY_MAX_SECS`.
fn retry_backoff(attempt: u32) -> std::time::Duration {
    let exp = attempt.saturating_sub(1).min(31);
    let secs = RETRY_BASE_SECS
        .saturating_mul(2u64.saturating_pow(exp))
        .min(RETRY_MAX_SECS);
    std::time::Duration::from_secs(secs)
}

/// Which timezone a cron job evaluates in. Split out so the (typed) engine call
/// picks the right `TimeZone` implementor. `Local` preserves today's DST-aware
/// behavior for jobs with no `tz`; `Fixed` is used for an explicit offset.
enum CronTimezone {
    Local,
    Fixed(FixedOffset),
}

/// Resolve a job's `tz` string to a concrete timezone for cron evaluation.
///
/// - `None` → `Local` (unchanged, DST-aware — existing cron jobs behave exactly
///   as before).
/// - `"UTC"`/`"Z"` → fixed +00:00.
/// - `"+HH:MM"` / `"-HH:MM"` / `"+HHMM"` → parsed fixed offset.
/// - Anything else (e.g. an IANA name like `America/New_York`) → `Local`, with a
///   warning. Full IANA support needs the `chrono-tz` database, deferred to a
///   follow-up so this PR adds no unverifiable lockfile change.
fn resolve_cron_timezone(tz: Option<&str>) -> CronTimezone {
    let Some(raw) = tz.map(str::trim).filter(|s| !s.is_empty()) else {
        return CronTimezone::Local;
    };
    if raw.eq_ignore_ascii_case("utc") || raw.eq_ignore_ascii_case("z") {
        return CronTimezone::Fixed(FixedOffset::east_opt(0).expect("0 is a valid offset"));
    }
    if let Some(offset) = parse_fixed_offset(raw) {
        return CronTimezone::Fixed(offset);
    }
    tracing::warn!(
        "Timezone '{}' is not a recognized offset (IANA names need chrono-tz, not yet wired); \
         falling back to system local time",
        raw
    );
    CronTimezone::Local
}

/// Parse a `+HH:MM` / `-HH:MM` / `+HHMM` fixed-offset string into a
/// `FixedOffset`. Returns `None` for any other shape.
// All slices below are on ASCII char boundaries: `raw[1..]` skips a single
// `+`/`-` byte, and `digits` is filtered to ASCII digits, so `digits[0..2]` /
// `[2..4]` are always valid boundaries.
#[allow(clippy::string_slice)]
fn parse_fixed_offset(raw: &str) -> Option<FixedOffset> {
    let bytes = raw.as_bytes();
    let sign = match bytes.first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let digits: String = raw[1..].chars().filter(|c| c.is_ascii_digit()).collect();
    let (hours, mins) = match digits.len() {
        4 => (
            digits[0..2].parse::<i32>().ok()?,
            digits[2..4].parse::<i32>().ok()?,
        ),
        2 => (digits[0..2].parse::<i32>().ok()?, 0),
        _ => return None,
    };
    if hours > 23 || mins > 59 {
        return None;
    }
    FixedOffset::east_opt(sign * (hours * 3600 + mins * 60))
}

/// Validate the schedule kind of a job spec: EXACTLY one of cron / at /
/// every_seconds must be set, and an interval must be within bounds. Returns a
/// human-readable reason on failure. Applied centrally in `add_scheduled_job` so
/// every entry point (route, agent tool, trait) is guarded.
fn validate_schedule_spec(job: &ScheduledJob) -> Result<(), String> {
    let has_cron = !job.cron.trim().is_empty();
    let has_at = job.at.is_some();
    let has_every = job.every_seconds.is_some();
    let count = [has_cron, has_at, has_every].iter().filter(|b| **b).count();
    if count != 1 {
        return Err(format!(
            "exactly one schedule kind must be set (cron, at, or every_seconds); got {} \
             (cron={}, at={}, every_seconds={})",
            count, has_cron, has_at, has_every
        ));
    }
    if let Some(secs) = job.every_seconds {
        if !(MIN_INTERVAL_SECS..=MAX_INTERVAL_SECS).contains(&secs) {
            return Err(format!(
                "interval every_seconds={} out of bounds [{}, {}]",
                secs, MIN_INTERVAL_SECS, MAX_INTERVAL_SECS
            ));
        }
    }
    Ok(())
}

/// Pure missed-run predicate: was a scheduled fire due in the window since
/// `last_run` that did not run? Used at startup to flag `Missed`. Paused jobs are
/// never "missed". Evaluated in UTC (a small skew vs the job's local firing tz is
/// immaterial to multi-hour-downtime detection).
fn is_run_missed(job: &ScheduledJob, now: DateTime<Utc>) -> bool {
    if job.paused {
        return false;
    }
    // One-time: due once `at` has passed and it never ran.
    if let Some(at) = job.at {
        return job.last_run.is_none() && at <= now;
    }
    // Interval: a fire was due if a full interval elapsed since the last run.
    if let Some(secs) = job.every_seconds {
        if secs == 0 {
            return false;
        }
        return match job.last_run {
            // Never run → interval anchors from arm time (unknown at load); don't
            // claim a miss.
            None => false,
            Some(lr) => now.signed_duration_since(lr).num_seconds().max(0) as u64 >= secs,
        };
    }
    // Cron: the next scheduled fire strictly after last_run is already in the
    // past. Requires a known last_run (a never-run cron can't be judged missed).
    match job.last_run {
        None => false,
        Some(lr) => match parse_cron_schedule(&job.cron) {
            Some(cron) => cron
                .find_next_occurrence(&lr, false)
                .map(|next| next <= now)
                .unwrap_or(false),
            None => false,
        },
    }
}

/// Parse a (normalized) cron string the SAME way `tokio_cron_scheduler` does, so
/// missed-detection agrees with the engine's firing. Returns `None` on a parse
/// error (caller treats an unparseable cron as "not missed").
fn parse_cron_schedule(cron: &str) -> Option<croner::Cron> {
    let normalized = normalize_cron_to_6field(cron).unwrap_or_else(|| cron.to_string());
    croner::Cron::new(&normalized)
        .with_seconds_required()
        .with_dom_and_dow()
        .parse()
        .ok()
}

async fn persist_jobs(
    storage_path: &Path,
    jobs: &Arc<Mutex<JobsMap>>,
) -> Result<(), SchedulerError> {
    let jobs_guard = jobs.lock().await;
    let list: Vec<ScheduledJob> = jobs_guard.values().map(|(_, j)| j.clone()).collect();
    if let Some(parent) = storage_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(&list)?;
    fs::write(storage_path, data)?;
    Ok(())
}

/// Normalize a cron expression to 6-field (seconds-prefixed) form.
///
/// Returns `Some(normalized)` if the input was legacy 5-field, `None` if it is
/// already 6-field (or any other field count — left untouched for
/// `create_job_task` to surface the error). Idempotent: a 6-field cron yields
/// `None`, so re-running over an already-normalized store is a no-op.
fn normalize_cron_to_6field(cron: &str) -> Option<String> {
    match cron.split_whitespace().count() {
        5 => Some(format!("0 {}", cron.trim())),
        _ => None,
    }
}

pub struct Scheduler {
    tokio_scheduler: TokioJobScheduler,
    jobs: Arc<Mutex<JobsMap>>,
    storage_path: PathBuf,
    running_tasks: Arc<Mutex<RunningTasksMap>>,
    session_manager: Arc<SessionManager>,
    brain: Arc<tokio::sync::RwLock<Option<crate::brain_handle::SafeBrain>>>,
    persona: Arc<tokio::sync::RwLock<Option<crate::config::agent_identity::SharedPersona>>>,
    agent_config:
        Arc<tokio::sync::RwLock<Option<crate::config::agent_identity::SharedAgentConfig>>>,
}

impl Scheduler {
    pub async fn new(
        storage_path: PathBuf,
        session_manager: Arc<SessionManager>,
    ) -> Result<Arc<Self>, SchedulerError> {
        let internal_scheduler = TokioJobScheduler::new()
            .await
            .map_err(|e| SchedulerError::SchedulerInternalError(e.to_string()))?;

        let jobs = Arc::new(Mutex::new(HashMap::new()));
        let running_tasks = Arc::new(Mutex::new(HashMap::new()));

        let arc_self = Arc::new(Self {
            tokio_scheduler: internal_scheduler,
            jobs,
            storage_path,
            running_tasks,
            session_manager,
            brain: Arc::new(tokio::sync::RwLock::new(None)),
            persona: Arc::new(tokio::sync::RwLock::new(None)),
            agent_config: Arc::new(tokio::sync::RwLock::new(None)),
        });

        arc_self.load_jobs_from_storage().await;
        arc_self
            .tokio_scheduler
            .start()
            .await
            .map_err(|e| SchedulerError::SchedulerInternalError(e.to_string()))?;

        Ok(arc_self)
    }

    /// Build a `tokio_cron_scheduler` job for `job`, dispatching on its kind:
    /// `at` → one-shot, `every_seconds` → interval, else cron (tz-aware). The
    /// fired closure is identical across kinds; only the arming differs. Renamed
    /// from `create_cron_task` now that it builds non-cron jobs too.
    fn create_job_task(&self, job: ScheduledJob) -> Result<Job, SchedulerError> {
        let job_for_task = job.clone();
        let jobs_arc = self.jobs.clone();
        let storage_path = self.storage_path.clone();
        let running_tasks_arc = self.running_tasks.clone();
        let brain_arc = self.brain.clone();
        let persona_arc = self.persona.clone();
        let agent_config_arc = self.agent_config.clone();
        let session_manager_arc = self.session_manager.clone();
        // One-shots delete themselves after they fire (see the closure tail), so
        // they never re-run on the next restart.
        let is_one_shot = job.at.is_some();

        // The fired task body — kind-agnostic. Built once, moved into whichever
        // constructor the kind selects below (match arms are mutually exclusive,
        // so the single closure is moved exactly once per path). The explicit
        // return type is REQUIRED: separated from its constructor, the closure
        // would otherwise infer a concrete `Pin<Box<{async block}>>` that won't
        // unify with the `Pin<Box<dyn Future>>` the three constructors expect —
        // annotating drives the unsizing coercion at the `Box::pin` below.
        let task_closure = move |_uuid: uuid::Uuid, _l: TokioJobScheduler| -> ScheduledTaskFuture {
            tracing::info!("Scheduled task triggered for job '{}'", job_for_task.id);
            let task_job_id = job_for_task.id.clone();
            let current_jobs_arc = jobs_arc.clone();
            let local_storage_path = storage_path.clone();
            let job_to_execute = job_for_task.clone();
            let running_tasks = running_tasks_arc.clone();
            let brain_for_task = brain_arc.clone();
            let persona_for_task = persona_arc.clone();
            let agent_config_for_task = agent_config_arc.clone();
            let session_manager_for_task = session_manager_arc.clone();

            Box::pin(async move {
                let should_execute = {
                    let jobs_guard = current_jobs_arc.lock().await;
                    jobs_guard
                        .get(&task_job_id)
                        .map(|(_, j)| !j.paused)
                        .unwrap_or(false)
                };

                if !should_execute {
                    return;
                }

                let current_time = Utc::now();
                // Atomically CLAIM this fire under the jobs lock — mirroring the
                // guard `run_now` already uses. If the job is still marked
                // `currently_running` (a prior scheduled fire, or a manual
                // `run_now`, is still in flight — e.g. an interval job whose run
                // outlasts its period, or a job whose inline retry loop is
                // backing off), skip this tick. Without this, two runs of the
                // same job race: the second clobbers the first's cancel token in
                // `running_tasks` (making the first unkillable) and the first to
                // finish clears `currently_running` while the second still runs.
                // Also snapshot the status the PREVIOUS fire left (to dedup
                // failure escalations to one per streak) at the moment we claim.
                let prior_status = {
                    let mut jobs_guard = current_jobs_arc.lock().await;
                    match jobs_guard.get_mut(&task_job_id) {
                        Some((_, job)) if !job.currently_running => {
                            let prior = job.last_status;
                            job.last_run = Some(current_time);
                            job.currently_running = true;
                            job.process_start_time = Some(current_time);
                            Some(prior)
                        }
                        // already running, or removed between the checks → don't claim
                        _ => None,
                    }
                };
                let Some(prior_status) = prior_status else {
                    tracing::info!(
                        "Scheduled job '{}' skipped this tick: a prior run is still in flight \
                         (or the job was unscheduled)",
                        task_job_id
                    );
                    return;
                };

                if let Err(e) = persist_jobs(&local_storage_path, &current_jobs_arc).await {
                    tracing::error!("Failed to persist job status: {}", e);
                }

                let cancel_token = CancellationToken::new();
                {
                    let mut tasks = running_tasks.lock().await;
                    tasks.insert(task_job_id.clone(), cancel_token.clone());
                }

                // Emit job started activity event
                crate::events::activity::emit_activity(
                    crate::events::activity::automation_job_started(&task_job_id, &task_job_id),
                );

                let job_start_instant = std::time::Instant::now();

                // ── Bounded retry loop ──
                // Attempt the run; on failure with retries remaining, back off and
                // retry INLINE (the fire stays `currently_running` for its whole
                // retry sequence). Bounded by the live `max_retries`, so a flapping
                // job can never storm. Default max_retries=0 → a single attempt,
                // exactly today's behavior.
                let mut attempt: u32 = 0;
                let final_result = loop {
                    let brain_snapshot = brain_for_task.read().await.clone();
                    let persona_snapshot = persona_for_task.read().await.clone();
                    let ac_snapshot = agent_config_for_task.read().await.clone();
                    // Isolate a job panic (durability F3): a panic inside
                    // execute_job is converted to an Err so cleanup still runs and
                    // other jobs keep firing.
                    let attempt_result = std::panic::AssertUnwindSafe(execute_job(
                        job_to_execute.clone(),
                        current_jobs_arc.clone(),
                        task_job_id.clone(),
                        cancel_token.clone(),
                        brain_snapshot,
                        persona_snapshot,
                        ac_snapshot,
                    ))
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| {
                        let msg = panic
                            .downcast_ref::<&str>()
                            .map(|s| s.to_string())
                            .or_else(|| panic.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "<non-string panic payload>".to_string());
                        tracing::error!(
                            target: "durability",
                            job = %task_job_id,
                            "scheduler job panicked (contained, loop survives): {msg}"
                        );
                        Err(anyhow!("job '{}' panicked: {}", task_job_id, msg))
                    });

                    match attempt_result {
                        Ok(session_id) => break Ok(session_id),
                        Err(e) => {
                            let max_retries = {
                                let g = current_jobs_arc.lock().await;
                                g.get(&task_job_id).map(|(_, j)| j.max_retries).unwrap_or(0)
                            };
                            if attempt < max_retries && !cancel_token.is_cancelled() {
                                attempt += 1;
                                {
                                    let mut g = current_jobs_arc.lock().await;
                                    if let Some((_, j)) = g.get_mut(&task_job_id) {
                                        j.retry_count = attempt;
                                        j.last_status = Some(ScheduleRunStatus::Error);
                                        j.last_error = Some(e.to_string());
                                    }
                                }
                                let _ = persist_jobs(&local_storage_path, &current_jobs_arc).await;
                                let backoff = retry_backoff(attempt);
                                tracing::warn!(
                                    target: "durability",
                                    job = %task_job_id,
                                    "scheduled job failed (attempt {}/{}), retrying in {:?}: {}",
                                    attempt, max_retries, backoff, e
                                );
                                tokio::time::sleep(backoff).await;
                                continue;
                            }
                            break Err(e);
                        }
                    }
                };

                {
                    let mut tasks = running_tasks.lock().await;
                    tasks.remove(&task_job_id);
                }

                // Record terminal outcome onto the job's reliability fields.
                let (max_retries, retry_spent) = {
                    let mut jobs_guard = current_jobs_arc.lock().await;
                    if let Some((_, job)) = jobs_guard.get_mut(&task_job_id) {
                        job.currently_running = false;
                        job.current_session_id = None;
                        job.process_start_time = None;
                        job.run_count = job.run_count.saturating_add(1);
                        match &final_result {
                            Ok(_) => {
                                job.last_status = Some(ScheduleRunStatus::Ok);
                                job.last_error = None;
                                job.retry_count = 0;
                            }
                            Err(e) => {
                                job.last_status = Some(ScheduleRunStatus::Error);
                                job.last_error = Some(e.to_string());
                            }
                        }
                        (job.max_retries, job.retry_count)
                    } else {
                        (0, 0)
                    }
                };

                // One-shot jobs are done the moment they fire — remove so they
                // never re-run on restart (the durable "fire exactly once").
                if is_one_shot {
                    current_jobs_arc.lock().await.remove(&task_job_id);
                }

                if let Err(e) = persist_jobs(&local_storage_path, &current_jobs_arc).await {
                    tracing::error!("Failed to persist job completion: {}", e);
                }

                let duration_ms = job_start_instant.elapsed().as_millis() as u64;
                match final_result {
                    Ok(ref session_id) => {
                        tracing::info!("Job '{}' completed", task_job_id);
                        crate::events::activity::emit_activity(
                            crate::events::activity::automation_job_completed(
                                &task_job_id,
                                &task_job_id,
                                session_id,
                                duration_ms,
                                0, // message count not easily available here
                            ),
                        );
                    }
                    Err(ref e) => {
                        tracing::error!("Job '{}' failed: {}", task_job_id, e);
                        crate::events::activity::emit_activity(
                            crate::events::activity::automation_job_failed(
                                &task_job_id,
                                &task_job_id,
                                &e.to_string(),
                            ),
                        );
                        #[cfg(feature = "telemetry")]
                        crate::posthog::emit_error("scheduler_job_failed", &e.to_string());

                        // Escalate to the Decision Inbox when a job the user gave
                        // retries to has exhausted them (i.e. it "keeps failing").
                        // Deduped to one decision per failure streak via
                        // `prior_status`, so a job that fails every fire does not
                        // spam. Default jobs (max_retries=0) never escalate —
                        // exactly today's behavior. Best-effort: logged, never
                        // fatal to the loop.
                        if should_escalate_failure(max_retries, prior_status) {
                            escalate_persistent_failure(
                                &session_manager_for_task,
                                &task_job_id,
                                retry_spent,
                                max_retries,
                                &e.to_string(),
                            )
                            .await;
                        }
                    }
                }
            })
        };

        // ── Kind dispatch ── one-shot / interval / cron(tz). ──────────────────
        let build_result = if let Some(at) = job.at {
            // Fire once, `at - now` from now (clamped to zero if already due →
            // a startup catch-up for a one-shot missed during downtime).
            let delay = at
                .signed_duration_since(Utc::now())
                .to_std()
                .unwrap_or(std::time::Duration::ZERO);
            Job::new_one_shot_async(delay, task_closure)
        } else if let Some(secs) = job.every_seconds {
            Job::new_repeated_async(std::time::Duration::from_secs(secs), task_closure)
        } else {
            // Cron (the default kind). Validate/normalize field count here only —
            // one-shot/interval jobs carry an empty cron.
            let cron_parts: Vec<&str> = job.cron.split_whitespace().collect();
            let cron = match cron_parts.len() {
                5 => {
                    tracing::warn!(
                        "Job '{}' has legacy 5-field cron '{}', converting to 6-field",
                        job.id,
                        job.cron
                    );
                    format!("0 {}", job.cron)
                }
                6 => job.cron.clone(),
                _ => {
                    return Err(SchedulerError::CronParseError(format!(
                        "Invalid cron expression '{}': expected 5 or 6 fields, got {}",
                        job.cron,
                        cron_parts.len()
                    )))
                }
            };
            match resolve_cron_timezone(job.tz.as_deref()) {
                CronTimezone::Local => {
                    Job::new_async_tz(&cron, Local::now().timezone(), task_closure)
                }
                CronTimezone::Fixed(offset) => Job::new_async_tz(&cron, offset, task_closure),
            }
        };
        build_result.map_err(|e| SchedulerError::CronParseError(e.to_string()))
    }

    pub async fn add_scheduled_job(
        &self,
        original_job_spec: ScheduledJob,
        make_copy: bool,
    ) -> Result<(), SchedulerError> {
        // Central kind guard: every entry point (route, agent tool, trait) funnels
        // here, so exactly-one-of cron/at/every and interval bounds are enforced
        // once. Existing cron-only callers pass validation unchanged.
        validate_schedule_spec(&original_job_spec).map_err(SchedulerError::InvalidScheduleSpec)?;
        {
            let jobs_guard = self.jobs.lock().await;
            if jobs_guard.contains_key(&original_job_spec.id) {
                return Err(SchedulerError::JobIdExists(original_job_spec.id.clone()));
            }
        }

        let mut stored_job = original_job_spec;
        if make_copy {
            let original_recipe_path = Path::new(&stored_job.source);
            if !original_recipe_path.is_file() {
                return Err(SchedulerError::RecipeLoadError(format!(
                    "Recipe file not found: {}",
                    stored_job.source
                )));
            }

            let scheduled_recipes_dir = get_default_scheduled_recipes_dir()?;
            let original_extension = original_recipe_path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("yaml");

            let destination_filename = format!("{}.{}", stored_job.id, original_extension);
            let destination_recipe_path = scheduled_recipes_dir.join(destination_filename);

            fs::copy(original_recipe_path, &destination_recipe_path)?;
            stored_job.source = destination_recipe_path.to_string_lossy().into_owned();
            stored_job.current_session_id = None;
            stored_job.process_start_time = None;
        }

        let cron_task = self.create_job_task(stored_job.clone())?;

        let job_uuid = self
            .tokio_scheduler
            .add(cron_task)
            .await
            .map_err(|e| SchedulerError::SchedulerInternalError(e.to_string()))?;

        {
            let mut jobs_guard = self.jobs.lock().await;
            jobs_guard.insert(stored_job.id.clone(), (job_uuid, stored_job));
        }

        persist_jobs(&self.storage_path, &self.jobs).await?;
        Ok(())
    }

    pub async fn schedule_recipe(
        &self,
        recipe_path: PathBuf,
        cron_schedule: Option<String>,
    ) -> Result<(), SchedulerError> {
        let recipe_path_str = recipe_path.to_string_lossy().to_string();

        let existing_job_id = {
            let jobs_guard = self.jobs.lock().await;
            jobs_guard
                .iter()
                .find(|(_, (_, job))| job.source == recipe_path_str)
                .map(|(id, _)| id.clone())
        };

        match cron_schedule {
            Some(cron) => {
                if let Some(job_id) = existing_job_id {
                    self.update_schedule(&job_id, cron).await
                } else {
                    let job_id = self.generate_unique_job_id(&recipe_path).await;
                    let job = ScheduledJob {
                        id: job_id,
                        source: recipe_path_str,
                        cron,
                        last_run: None,
                        currently_running: false,
                        paused: false,
                        current_session_id: None,
                        process_start_time: None,
                        worker_persona: None,
                        starter_id: None,
                        starter_version: None,
                        starter_content_hash: None,
                        user_customized: None,
                        ..Default::default()
                    };
                    self.add_scheduled_job(job, false).await
                }
            }
            None => {
                if let Some(job_id) = existing_job_id {
                    self.remove_scheduled_job(&job_id, false).await
                } else {
                    Ok(())
                }
            }
        }
    }

    async fn generate_unique_job_id(&self, path: &Path) -> String {
        let base_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unnamed")
            .to_string();

        let jobs_guard = self.jobs.lock().await;
        let mut id = base_id.clone();
        let mut counter = 1;

        while jobs_guard.contains_key(&id) {
            id = format!("{}_{}", base_id, counter);
            counter += 1;
        }

        id
    }

    async fn load_jobs_from_storage(self: &Arc<Self>) {
        if !self.storage_path.exists() {
            return;
        }
        let data = match fs::read_to_string(&self.storage_path) {
            Ok(data) => data,
            Err(e) => {
                tracing::error!(
                    "Failed to read {}: {}. Starting with empty schedule list.",
                    self.storage_path.display(),
                    e
                );
                return;
            }
        };
        if data.trim().is_empty() {
            return;
        }

        let list: Vec<ScheduledJob> = match serde_json::from_str(&data) {
            Ok(jobs) => jobs,
            Err(e) => {
                tracing::error!(
                    "Failed to parse {}: {}. Starting with empty schedule list.",
                    self.storage_path.display(),
                    e
                );
                return;
            }
        };

        // One-time, idempotent: rewrite any legacy 5-field cron to 6-field so the
        // scheduler stops converting (and warning) on every load. Persisted once
        // below; subsequent loads see 6-field and this is a no-op.
        let mut crons_normalized = false;
        // Also persist once if we reconcile any stale run-flags below.
        let mut reconciled = false;

        for mut job_to_load in list {
            if !Path::new(&job_to_load.source).exists() {
                tracing::warn!(
                    "Recipe file {} not found, skipping job '{}'",
                    job_to_load.source,
                    job_to_load.id
                );
                continue;
            }

            // A one-shot `at` job that already fired (last_run set) but is still
            // in storage was interrupted after firing but before its self-delete
            // (which only runs on a clean completion). Re-arming it would fire the
            // recipe a SECOND time — is_run_missed() is false for an already-fired
            // `at`, and the re-arm below clamps a past `at` to a zero delay. Drop
            // it: it already ran exactly once. `reconciled` persists the cleaned
            // list below so it does not resurface on the next boot.
            if job_to_load.at.is_some() && job_to_load.last_run.is_some() {
                tracing::warn!(
                    target: "durability",
                    "One-shot job '{}' already fired before a prior process exited; \
                     dropping it instead of re-firing",
                    job_to_load.id
                );
                reconciled = true;
                continue;
            }

            // Startup reconciliation (durability F2): a job persisted with
            // `currently_running = true` was left mid-run by a prior process that
            // died or half-died. At load time nothing is actually running, so the
            // flag is definitionally stale — and there is no other reset path, so
            // it would otherwise wedge Librarian/Steward/etc. forever. Clear it so
            // the job can fire again.
            if job_to_load.currently_running {
                tracing::warn!(
                    target: "durability",
                    "Reconciling stale currently_running=true for job '{}' left by a prior process; resetting so it can run again",
                    job_to_load.id
                );
                job_to_load.currently_running = false;
                job_to_load.current_session_id = None;
                job_to_load.process_start_time = None;
                reconciled = true;
            }

            if let Some(normalized) = normalize_cron_to_6field(&job_to_load.cron) {
                tracing::info!(
                    "Normalizing legacy 5-field cron for job '{}' to 6-field once: '{}' -> '{}'",
                    job_to_load.id,
                    job_to_load.cron,
                    normalized
                );
                job_to_load.cron = normalized;
                crons_normalized = true;
            }

            // Missed-run detection (feature B): if a scheduled fire was due while
            // the daemon was down, flag it so the Automate tab shows `Missed`
            // (amber). Detection only — we deliberately do NOT auto-execute a
            // catch-up for cron/interval jobs here (that would risk a wake-storm
            // and runs before brain/persona are wired); the job runs at its next
            // natural tick. One-shot jobs self-catch-up: they re-arm below with a
            // zero delay and fire once immediately, then delete themselves.
            if is_run_missed(&job_to_load, Utc::now()) {
                tracing::warn!(
                    target: "durability",
                    "Job '{}' missed a scheduled run during downtime; marking Missed",
                    job_to_load.id
                );
                job_to_load.last_status = Some(ScheduleRunStatus::Missed);
                reconciled = true;
            }

            let cron_task = match self.create_job_task(job_to_load.clone()) {
                Ok(task) => task,
                Err(e) => {
                    tracing::error!(
                        "Failed to create task for job '{}': {}. Skipping.",
                        job_to_load.id,
                        e
                    );
                    continue;
                }
            };

            let job_uuid = match self.tokio_scheduler.add(cron_task).await {
                Ok(uuid) => uuid,
                Err(e) => {
                    tracing::error!(
                        "Failed to add job '{}' to scheduler: {}. Skipping.",
                        job_to_load.id,
                        e
                    );
                    continue;
                }
            };

            let mut jobs_guard = self.jobs.lock().await;
            jobs_guard.insert(job_to_load.id.clone(), (job_uuid, job_to_load));
        }

        // Persist once if we normalized crons and/or reconciled stale run-flags,
        // so the next load is a no-op.
        if crons_normalized || reconciled {
            if let Err(e) = persist_jobs(&self.storage_path, &self.jobs).await {
                tracing::warn!("Failed to persist scheduler reconciliation: {}", e);
            }
        }
    }

    async fn sync_from_storage(&self) {
        if !self.storage_path.exists() {
            return;
        }
        let data = match fs::read_to_string(&self.storage_path) {
            Ok(d) => d,
            Err(_) => return,
        };
        if data.trim().is_empty() {
            return;
        }
        let disk_jobs: Vec<ScheduledJob> = match serde_json::from_str(&data) {
            Ok(jobs) => jobs,
            Err(_) => return,
        };

        let disk_ids: std::collections::HashSet<String> =
            disk_jobs.iter().map(|j| j.id.clone()).collect();

        let (jobs_to_add, jobs_to_remove): (Vec<ScheduledJob>, Vec<(String, JobId)>) = {
            let jobs_guard = self.jobs.lock().await;
            let to_add = disk_jobs
                .into_iter()
                .filter(|j| !jobs_guard.contains_key(&j.id))
                .collect();
            let to_remove = jobs_guard
                .iter()
                .filter(|(id, (_, j))| !disk_ids.contains(*id) && !j.currently_running)
                .map(|(id, (uuid, _))| (id.clone(), *uuid))
                .collect();
            (to_add, to_remove)
        };

        for job in jobs_to_add {
            if !Path::new(&job.source).exists() {
                tracing::warn!(
                    "Skipping sync of job '{}': recipe file not found at {}",
                    job.id,
                    job.source
                );
                continue;
            }
            let cron_task = match self.create_job_task(job.clone()) {
                Ok(t) => t,
                Err(e) => {
                    tracing::error!(
                        "Failed to create cron task for '{}' during sync: {}",
                        job.id,
                        e
                    );
                    continue;
                }
            };
            let uuid = match self.tokio_scheduler.add(cron_task).await {
                Ok(u) => u,
                Err(e) => {
                    tracing::error!("Failed to register job '{}' during sync: {}", job.id, e);
                    continue;
                }
            };
            self.jobs.lock().await.insert(job.id.clone(), (uuid, job));
        }

        for (id, uuid) in jobs_to_remove {
            let _ = self.tokio_scheduler.remove(&uuid).await;
            self.jobs.lock().await.remove(&id);
        }
    }

    pub async fn list_scheduled_jobs(&self) -> Vec<ScheduledJob> {
        self.sync_from_storage().await;
        self.jobs
            .lock()
            .await
            .values()
            .map(|(_, j)| j.clone())
            .collect()
    }

    pub async fn remove_scheduled_job(
        &self,
        id: &str,
        remove_recipe: bool,
    ) -> Result<(), SchedulerError> {
        let (job_uuid, recipe_path) = {
            let mut jobs_guard = self.jobs.lock().await;
            match jobs_guard.remove(id) {
                Some((uuid, job)) => (uuid, job.source.clone()),
                None => return Err(SchedulerError::JobNotFound(id.to_string())),
            }
        };

        self.tokio_scheduler
            .remove(&job_uuid)
            .await
            .map_err(|e| SchedulerError::SchedulerInternalError(e.to_string()))?;

        if remove_recipe {
            let path = Path::new(&recipe_path);
            if path.exists() {
                fs::remove_file(path)?;
            }
        }

        persist_jobs(&self.storage_path, &self.jobs).await?;
        Ok(())
    }

    pub async fn sessions(
        &self,
        sched_id: &str,
        limit: usize,
    ) -> Result<Vec<(String, Session)>, SchedulerError> {
        // Filter + cap pushed into SQL (newest first), so we never materialise
        // the full session table just to keep `limit` rows for one schedule.
        let schedule_sessions = self
            .session_manager
            .list_sessions_by_schedule_id(sched_id, limit)
            .await
            .map_err(|e| SchedulerError::StorageError(io::Error::other(e)))?
            .into_iter()
            .map(|s| (s.id.clone(), s))
            .collect();

        Ok(schedule_sessions)
    }

    pub async fn run_now(&self, sched_id: &str) -> Result<String, SchedulerError> {
        let job_to_run = {
            let mut jobs_guard = self.jobs.lock().await;
            match jobs_guard.get_mut(sched_id) {
                Some((_, job)) => {
                    if job.currently_running {
                        return Err(SchedulerError::AnyhowError(anyhow!(
                            "Job '{}' is already running",
                            sched_id
                        )));
                    }
                    job.currently_running = true;
                    job.process_start_time = Some(Utc::now());
                    job.clone()
                }
                None => return Err(SchedulerError::JobNotFound(sched_id.to_string())),
            }
        };

        persist_jobs(&self.storage_path, &self.jobs).await?;

        let cancel_token = CancellationToken::new();
        {
            let mut tasks = self.running_tasks.lock().await;
            tasks.insert(sched_id.to_string(), cancel_token.clone());
        }

        // Emit job started activity event
        crate::events::activity::emit_activity(crate::events::activity::automation_job_started(
            sched_id, sched_id,
        ));

        let job_start_instant = std::time::Instant::now();
        let brain_snapshot = self.brain.read().await.clone();
        let persona_snapshot = self.persona.read().await.clone();
        let ac_snapshot = self.agent_config.read().await.clone();
        let result = execute_job(
            job_to_run,
            self.jobs.clone(),
            sched_id.to_string(),
            cancel_token.clone(),
            brain_snapshot,
            persona_snapshot,
            ac_snapshot,
        )
        .await;

        {
            let mut tasks = self.running_tasks.lock().await;
            tasks.remove(sched_id);
        }

        let duration_ms = job_start_instant.elapsed().as_millis() as u64;

        {
            let mut jobs_guard = self.jobs.lock().await;
            if let Some((_, job)) = jobs_guard.get_mut(sched_id) {
                job.currently_running = false;
                job.current_session_id = None;
                job.process_start_time = None;
                job.last_run = Some(Utc::now());
            }
        }

        persist_jobs(&self.storage_path, &self.jobs).await?;

        match result {
            Ok(session_id) => {
                crate::events::activity::emit_activity(
                    crate::events::activity::automation_job_completed(
                        sched_id,
                        sched_id,
                        &session_id,
                        duration_ms,
                        0,
                    ),
                );
                Ok(session_id)
            }
            Err(e) => {
                crate::events::activity::emit_activity(
                    crate::events::activity::automation_job_failed(
                        sched_id,
                        sched_id,
                        &e.to_string(),
                    ),
                );
                Err(SchedulerError::AnyhowError(anyhow!(
                    "Job '{}' failed: {}",
                    sched_id,
                    e
                )))
            }
        }
    }

    pub async fn pause_schedule(&self, sched_id: &str) -> Result<(), SchedulerError> {
        {
            let mut jobs_guard = self.jobs.lock().await;
            match jobs_guard.get_mut(sched_id) {
                Some((_, job)) => {
                    if job.currently_running {
                        return Err(SchedulerError::AnyhowError(anyhow!(
                            "Cannot pause running schedule '{}'",
                            sched_id
                        )));
                    }
                    job.paused = true;
                }
                None => return Err(SchedulerError::JobNotFound(sched_id.to_string())),
            }
        }

        persist_jobs(&self.storage_path, &self.jobs).await
    }

    pub async fn unpause_schedule(&self, sched_id: &str) -> Result<(), SchedulerError> {
        {
            let mut jobs_guard = self.jobs.lock().await;
            match jobs_guard.get_mut(sched_id) {
                Some((_, job)) => job.paused = false,
                None => return Err(SchedulerError::JobNotFound(sched_id.to_string())),
            }
        }

        persist_jobs(&self.storage_path, &self.jobs).await
    }

    pub async fn update_schedule(
        &self,
        sched_id: &str,
        new_cron: String,
    ) -> Result<(), SchedulerError> {
        let (old_uuid, updated_job) = {
            let mut jobs_guard = self.jobs.lock().await;
            match jobs_guard.get_mut(sched_id) {
                Some((uuid, job)) => {
                    if job.currently_running {
                        return Err(SchedulerError::AnyhowError(anyhow!(
                            "Cannot update running schedule '{}'",
                            sched_id
                        )));
                    }
                    if new_cron == job.cron {
                        return Ok(());
                    }
                    job.cron = new_cron.clone();
                    (*uuid, job.clone())
                }
                None => return Err(SchedulerError::JobNotFound(sched_id.to_string())),
            }
        };

        self.tokio_scheduler
            .remove(&old_uuid)
            .await
            .map_err(|e| SchedulerError::SchedulerInternalError(e.to_string()))?;

        let cron_task = self.create_job_task(updated_job)?;
        let new_uuid = self
            .tokio_scheduler
            .add(cron_task)
            .await
            .map_err(|e| SchedulerError::SchedulerInternalError(e.to_string()))?;

        {
            let mut jobs_guard = self.jobs.lock().await;
            if let Some((uuid, _)) = jobs_guard.get_mut(sched_id) {
                *uuid = new_uuid;
            }
        }

        persist_jobs(&self.storage_path, &self.jobs).await
    }

    /// Generalized reschedule: change a job's KIND (cron / one-time / interval)
    /// and/or timezone, then re-arm. Sibling to `update_schedule` (which stays
    /// cron-only for back-compat). Setting one kind clears the others so the
    /// exactly-one invariant holds; `validate_schedule_spec` re-checks it. A
    /// currently-running job cannot be rescheduled.
    pub async fn update_schedule_spec(
        &self,
        sched_id: &str,
        cron: Option<String>,
        at: Option<DateTime<Utc>>,
        every_seconds: Option<u64>,
        tz: Option<String>,
    ) -> Result<(), SchedulerError> {
        let (old_uuid, updated_job) = {
            let mut jobs_guard = self.jobs.lock().await;
            match jobs_guard.get_mut(sched_id) {
                Some((uuid, job)) => {
                    if job.currently_running {
                        return Err(SchedulerError::AnyhowError(anyhow!(
                            "Cannot update running schedule '{}'",
                            sched_id
                        )));
                    }
                    // Apply exactly one kind, clearing the others.
                    if let Some(c) = cron {
                        job.cron = c;
                        job.at = None;
                        job.every_seconds = None;
                    } else if let Some(a) = at {
                        job.at = Some(a);
                        job.cron = String::new();
                        job.every_seconds = None;
                    } else if let Some(s) = every_seconds {
                        job.every_seconds = Some(s);
                        job.cron = String::new();
                        job.at = None;
                    }
                    // tz is independent of kind (applies to cron/at evaluation).
                    if tz.is_some() {
                        job.tz = tz;
                    }
                    (*uuid, job.clone())
                }
                None => return Err(SchedulerError::JobNotFound(sched_id.to_string())),
            }
        };

        validate_schedule_spec(&updated_job).map_err(SchedulerError::InvalidScheduleSpec)?;

        self.tokio_scheduler
            .remove(&old_uuid)
            .await
            .map_err(|e| SchedulerError::SchedulerInternalError(e.to_string()))?;

        let task = self.create_job_task(updated_job)?;
        let new_uuid = self
            .tokio_scheduler
            .add(task)
            .await
            .map_err(|e| SchedulerError::SchedulerInternalError(e.to_string()))?;

        {
            let mut jobs_guard = self.jobs.lock().await;
            if let Some((uuid, _)) = jobs_guard.get_mut(sched_id) {
                *uuid = new_uuid;
            }
        }

        persist_jobs(&self.storage_path, &self.jobs).await
    }

    pub async fn kill_running_job(&self, sched_id: &str) -> Result<(), SchedulerError> {
        {
            let jobs_guard = self.jobs.lock().await;
            match jobs_guard.get(sched_id) {
                Some((_, job)) if !job.currently_running => {
                    return Err(SchedulerError::AnyhowError(anyhow!(
                        "Schedule '{}' is not running",
                        sched_id
                    )));
                }
                None => return Err(SchedulerError::JobNotFound(sched_id.to_string())),
                _ => {}
            }
        }

        {
            let tasks = self.running_tasks.lock().await;
            if let Some(token) = tasks.get(sched_id) {
                token.cancel();
            }
        }

        Ok(())
    }

    pub async fn get_running_job_info(
        &self,
        sched_id: &str,
    ) -> Result<Option<(String, DateTime<Utc>)>, SchedulerError> {
        let jobs_guard = self.jobs.lock().await;
        match jobs_guard.get(sched_id) {
            Some((_, job)) if job.currently_running => {
                match (&job.current_session_id, &job.process_start_time) {
                    (Some(sid), Some(start)) => Ok(Some((sid.clone(), *start))),
                    _ => Ok(None),
                }
            }
            Some(_) => Ok(None),
            None => Err(SchedulerError::JobNotFound(sched_id.to_string())),
        }
    }
}

/// Escalate a persistently-failing scheduled job to the Decision Inbox so the
/// user is asked to intervene. Mirrors the Enricher's creation path (#495): build
/// a typed [`crate::decisions::NewDecision`] and call `create_decision`. We reuse
/// the existing `unblock` kind with reason `AttemptCap` — semantically "an
/// automated process exhausted its retry budget and is parked, needing human
/// direction", which is exactly a scheduled job that keeps failing. No new
/// `DecisionKind` is invented.
///
/// Best-effort by contract: any failure (pool unavailable, schema absent in a
/// test pool, malformed) is logged and swallowed — a failed escalation must
/// never take down the scheduler loop.
async fn escalate_persistent_failure(
    session_manager: &SessionManager,
    job_id: &str,
    retry_spent: u32,
    max_retries: u32,
    error: &str,
) {
    let pool = match session_manager.pool_clone().await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(
                target: "permagentd::scheduler",
                "Failed-job escalation for '{}' skipped: no pool: {}",
                job_id, e
            );
            return;
        }
    };

    let payload = crate::decisions::UnblockPayload {
        reason: crate::decisions::UnblockReason::AttemptCap,
        spent: Some(retry_spent as u64),
        cap: Some(max_retries as u64),
    };
    let payload_json = match serde_json::to_value(&payload) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                target: "permagentd::scheduler",
                "Failed-job escalation for '{}' skipped: payload serialize: {}",
                job_id, e
            );
            return;
        }
    };

    // Headline is user-facing and must be <= 80 chars (create_decision enforces).
    let mut headline = format!("A scheduled job keeps failing: {}", job_id);
    if headline.chars().count() > 80 {
        headline = headline.chars().take(79).collect::<String>() + "…";
    }
    let detail = format!(
        "Scheduled job '{}' failed after exhausting {} of {} retries. Last error: {}. \
         Intervene from the Automate tab (pause, edit, or delete it).",
        job_id, retry_spent, max_retries, error
    );

    let req = crate::decisions::NewDecision {
        kind: "unblock".to_string(),
        project_id: Some(crate::projects::PERSONAL_PROJECT_ID.to_string()),
        headline: Some(headline),
        detail: Some(detail),
        payload: payload_json,
        ..Default::default()
    };

    match crate::decisions::create_decision(&pool, req).await {
        Ok(d) if d.kind == "malformed" => tracing::warn!(
            target: "permagentd::scheduler",
            "Failed-job escalation for '{}' stored as malformed: {}",
            job_id, d.detail
        ),
        Ok(d) => tracing::info!(
            target: "permagentd::scheduler",
            "Escalated failing job '{}' to Decision Inbox as decision {}",
            job_id, d.id
        ),
        Err(e) => tracing::warn!(
            target: "permagentd::scheduler",
            "Failed-job escalation for '{}' failed: {}",
            job_id, e
        ),
    }
}

/// Build the agent for a headless scheduled-recipe run.
///
/// Scheduled jobs have NO approver: no terminal prompt exists and nobody is
/// watching an inbox mid-run, so an approval park could only hang the job
/// forever in the drain loop (the pre-#760 de-facto behavior), and any
/// `tool_approval` decision it filed would be undeliverable (this bare agent
/// is never registered in `AgentManager`, so answering reaches a fresh agent
/// with no waiter). Rather than silently widening permissions to
/// `GooseMode::Auto` — which would let anything able to schedule a recipe
/// bypass approve mode entirely — the agent is marked HEADLESS: tools the user
/// already always-allowed run normally, and any tool that would require
/// interactive approval is auto-denied with a recorded skip (never parked,
/// never filed). The user-visible remedies are pre-approving the tool or
/// running the recipe in auto mode.
fn new_scheduled_job_agent() -> Agent {
    let agent = Agent::new();
    agent.set_headless(true);
    agent
}

#[allow(clippy::too_many_lines)]
async fn execute_job(
    job: ScheduledJob,
    jobs: Arc<Mutex<JobsMap>>,
    job_id: String,
    cancel_token: CancellationToken,
    brain: Option<crate::brain_handle::SafeBrain>,
    persona: Option<crate::config::agent_identity::SharedPersona>,
    agent_config: Option<crate::config::agent_identity::SharedAgentConfig>,
) -> Result<String> {
    if job.source.is_empty() {
        return Ok(job.id.to_string());
    }

    let recipe_path = Path::new(&job.source);
    let recipe_content = fs::read_to_string(recipe_path)?;

    let recipe: Recipe = {
        let extension = recipe_path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("yaml")
            .to_lowercase();

        match extension.as_str() {
            "json" | "jsonl" => serde_json::from_str(&recipe_content)?,
            _ => serde_yaml::from_str(&recipe_content)?,
        }
    };

    let agent = new_scheduled_job_agent();

    // Wire persona into agent: worker persona if specified, else primary.
    if let Some(ref worker_key) = job.worker_persona {
        let mut resolved = false;
        if let Some(ref ac) = agent_config {
            let guard = ac.read().await;
            if let Some(worker) = guard.workers.get(worker_key) {
                agent
                    .set_persona_block_override(worker.system_prompt_block(), worker.display_name())
                    .await;
                resolved = true;
                tracing::info!(
                    target: "permagentd::brain",
                    "Scheduled job {} using worker persona: {}",
                    job_id,
                    worker_key
                );
            }
        }
        if !resolved {
            tracing::warn!(
                target: "permagentd::brain",
                "Worker persona '{}' not found for scheduled job {}, falling back to primary",
                worker_key,
                job_id
            );
            if let Some(ref p) = persona {
                agent.set_persona(p.clone()).await;
            }
        }
    } else if let Some(ref p) = persona {
        agent.set_persona(p.clone()).await;
    }

    let config = Config::global();
    let provider_name = config.get_goose_provider()?;
    let model_name = config.get_goose_model()?;
    let model_config =
        crate::model::ModelConfig::new(&model_name)?.with_canonical_limits(&provider_name);

    let session = agent
        .config
        .session_manager
        .create_session(
            std::env::current_dir()?,
            format!("Scheduled job: {}", job.id),
            SessionType::Scheduled,
            agent.config.goose_mode,
        )
        .await?;

    let extensions = resolve_extensions_for_new_session(recipe.extensions.as_deref(), None);
    for ext in &extensions {
        agent.add_extension(ext.clone(), &session.id).await?;
    }

    let agent_provider = create(&provider_name, model_config, extensions).await?;
    agent.update_provider(agent_provider, &session.id).await?;

    let mut jobs_guard = jobs.lock().await;
    if let Some((_, job_def)) = jobs_guard.get_mut(job_id.as_str()) {
        job_def.current_session_id = Some(session.id.clone());
    }
    drop(jobs_guard);

    let start_time = std::time::Instant::now();

    let recipe_display_name = recipe_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&job.id);
    let recipe_version = recipe.version.clone();

    tracing::info!(
        monotonic_counter.goose.session_starts = 1,
        session_type = "schedule",
        interface = "scheduler",
        interactive = false,
        "Scheduled session started"
    );

    tracing::info!(
        monotonic_counter.goose.recipe_runs = 1,
        recipe_name = %recipe_display_name,
        recipe_version = %recipe_version,
        session_type = "schedule",
        interface = "scheduler",
        "Recipe execution started"
    );

    #[cfg(feature = "telemetry")]
    tokio::spawn(async move {
        let mut props = HashMap::new();
        props.insert(
            "trigger".to_string(),
            serde_json::Value::String("automated".to_string()),
        );
        if let Err(e) = posthog::emit_event("schedule_job_started", props).await {
            tracing::debug!("Failed to send schedule telemetry: {}", e);
        }
    });

    let prompt_text = recipe
        .prompt
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            recipe
                .instructions
                .as_deref()
                .filter(|s| !s.trim().is_empty())
        })
        .ok_or_else(|| {
            anyhow!("Recipe must specify at least one of `instructions` or `prompt`.")
        })?;

    let user_message = Message::user().with_text(prompt_text);
    let mut conversation = Conversation::new_unvalidated(vec![user_message.clone()]);

    // ── Phase 3: Recall from brain before scheduled agent invocation ──
    const RECALL_SCORE_FLOOR: f64 = 0.7;
    const RECALL_TOP_K: usize = 3;

    if let Some(ref brain_handle) = brain {
        let recognition_ctx = spectral::graph::RecognitionContext::empty()
            .with_persona(crate::config::agent_identity::DEFAULT_PERSONA_KEY)
            .with_session(session.id.clone());
        match brain_handle
            .recall_cascade(prompt_text, &recognition_ctx)
            .await
        {
            Ok(result) => {
                let top_hits: Vec<_> = result
                    .merged_hits
                    .iter()
                    .filter(|hit| hit.signal_score >= RECALL_SCORE_FLOOR)
                    .take(RECALL_TOP_K)
                    .collect();

                if !top_hits.is_empty() {
                    let mut prefix = String::from("Relevant memories from past context:\n");
                    for hit in &top_hits {
                        prefix.push_str(&format!("- {}\n", hit.content));
                    }

                    tracing::info!(
                        target: "permagentd::brain",
                        "Recall injected {} memories into system prompt for scheduled job: {}",
                        top_hits.len(),
                        job_id
                    );

                    agent
                        .extend_system_prompt("memory_recall".to_string(), prefix)
                        .await;
                } else {
                    tracing::debug!(
                        target: "permagentd::brain",
                        "Recall returned no hits above {} threshold for scheduled job: {}",
                        RECALL_SCORE_FLOOR,
                        job_id
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    target: "permagentd::brain",
                    "Brain recall failed for scheduled job {}: {}",
                    job_id,
                    e
                );
            }
        }
    }

    let session_config = SessionConfig {
        id: session.id.clone(),
        schedule_id: Some(job.id.clone()),
        max_turns: None,
        retry_config: None,
    };

    let stream = agent
        .reply(user_message, session_config, Some(cancel_token))
        .await?;

    use futures::StreamExt;
    let mut stream = std::pin::pin!(stream);

    let mut stream_error = false;
    while let Some(message_result) = stream.next().await {
        tokio::task::yield_now().await;

        match message_result {
            Ok(AgentEvent::Message(msg)) => {
                conversation.push(msg);
            }
            Ok(AgentEvent::HistoryReplaced(updated)) => {
                conversation = updated;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::error!("Error in agent stream: {}", e);
                stream_error = true;
                break;
            }
        }
    }

    // ── Phase 4: Remember scheduled turn after response completes ──
    if let Some(ref brain_handle) = brain {
        let user_text = prompt_text.to_string();
        let assistant_text = conversation
            .messages()
            .iter()
            .rev()
            .find(|m| m.role == rmcp::model::Role::Assistant)
            .map(|m| m.as_concat_text())
            .unwrap_or_default();
        let turn_idx = conversation.len();

        if !user_text.is_empty() && !assistant_text.is_empty() {
            let brain_clone = brain_handle.clone();
            let remember_job_id = job_id.clone();

            tokio::spawn(async move {
                let key = format!("scheduled-{}-{}", remember_job_id, turn_idx);
                let content = format!("User: {}\nAssistant: {}", user_text, assistant_text);
                let device_id = *brain_clone.device_id();
                let key_for_log = key.clone();

                match brain_clone
                    .remember_with(
                        &key,
                        &content,
                        spectral::RememberOpts {
                            source: Some("scheduled".into()),
                            device_id: Some(device_id),
                            confidence: Some(1.0),
                            visibility: spectral::Visibility::Private,
                            wing: None,
                            ..Default::default()
                        },
                    )
                    .await
                {
                    Ok(_) => {
                        tracing::info!(
                            target: "permagentd::brain",
                            "Remembered scheduled turn: {}",
                            key_for_log
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "permagentd::brain",
                            "Failed to remember scheduled turn {}: {}",
                            key_for_log,
                            e
                        );
                    }
                }
            });
        }
    }

    // ── Phase 5: Extract structured findings from agent output ──
    // If the agent's response contains a <findings>[...]</findings> block,
    // parse it and store as actionable findings for the Automate tab UI.
    {
        let full_output = conversation
            .messages()
            .iter()
            .filter(|m| m.role == rmcp::model::Role::Assistant)
            .map(|m| m.as_concat_text())
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(start) = full_output.find("<findings>") {
            if let Some(end) = full_output.find("</findings>") {
                let json_str = full_output
                    .get(start + "<findings>".len()..end)
                    .unwrap_or("")
                    .trim();
                match serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                    Ok(findings) => {
                        let findings_dir = std::env::var("HOME")
                            .map(|h| {
                                std::path::PathBuf::from(h).join(".permagent/automation/findings")
                            })
                            .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/findings"));
                        let _ = std::fs::create_dir_all(&findings_dir);
                        let findings_path = findings_dir.join(format!("{}.json", session.id));
                        let findings_data = serde_json::json!({
                            "run_id": session.id,
                            "findings": findings,
                        });
                        let _ = std::fs::write(
                            &findings_path,
                            serde_json::to_string_pretty(&findings_data).unwrap_or_default(),
                        );
                        tracing::info!(
                            target: "permagentd::automation",
                            "Extracted {} findings from scheduled job {} (session {})",
                            findings.len(), job_id, session.id
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "permagentd::automation",
                            "Failed to parse findings JSON from job {}: {}",
                            job_id, e
                        );
                    }
                }
            }
        }
    }

    agent
        .config
        .session_manager
        .update(&session.id)
        .schedule_id(Some(job.id.clone()))
        .recipe(Some(recipe))
        .apply()
        .await?;

    {
        let session_duration = start_time.elapsed();
        let exit_type = if stream_error { "error" } else { "normal" };
        let (total_tokens, message_count) = agent
            .config
            .session_manager
            .get_session(&session.id, false)
            .await
            .map(|s| (s.total_tokens.unwrap_or(0), s.message_count))
            .unwrap_or((0, 0));

        tracing::info!(
            monotonic_counter.goose.session_completions = 1,
            session_type = "schedule",
            interface = "scheduler",
            exit_type,
            duration_ms = session_duration.as_millis() as u64,
            total_tokens,
            message_count,
            "Session completed"
        );

        tracing::info!(
            monotonic_counter.goose.session_duration_ms = session_duration.as_millis() as u64,
            session_type = "schedule",
            interface = "scheduler",
            "Session duration"
        );

        if total_tokens > 0 {
            tracing::info!(
                monotonic_counter.goose.session_tokens = total_tokens,
                session_type = "schedule",
                interface = "scheduler",
                "Session tokens"
            );
        }
    }

    #[cfg(feature = "telemetry")]
    {
        let duration_secs = start_time.elapsed().as_secs();
        tokio::spawn(async move {
            let mut props = HashMap::new();
            props.insert(
                "trigger".to_string(),
                serde_json::Value::String("automated".to_string()),
            );
            props.insert(
                "status".to_string(),
                serde_json::Value::String("completed".to_string()),
            );
            props.insert(
                "duration_seconds".to_string(),
                serde_json::Value::Number(serde_json::Number::from(duration_secs)),
            );
            if let Err(e) = posthog::emit_event("schedule_job_completed", props).await {
                tracing::debug!("Failed to send schedule telemetry: {}", e);
            }
        });
    }

    Ok(session.id)
}

#[async_trait]
impl SchedulerTrait for Scheduler {
    async fn set_brain(&self, brain: Option<crate::brain_handle::SafeBrain>) {
        let mut guard = self.brain.write().await;
        *guard = brain;
    }

    async fn set_persona(&self, persona: crate::config::agent_identity::SharedPersona) {
        let mut guard = self.persona.write().await;
        *guard = Some(persona);
    }

    async fn set_agent_config(&self, config: crate::config::agent_identity::SharedAgentConfig) {
        let mut guard = self.agent_config.write().await;
        *guard = Some(config);
    }

    async fn add_scheduled_job(
        &self,
        job: ScheduledJob,
        make_copy: bool,
    ) -> Result<(), SchedulerError> {
        self.add_scheduled_job(job, make_copy).await
    }

    async fn schedule_recipe(
        &self,
        recipe_path: PathBuf,
        cron_schedule: Option<String>,
    ) -> Result<(), SchedulerError> {
        self.schedule_recipe(recipe_path, cron_schedule).await
    }

    async fn list_scheduled_jobs(&self) -> Vec<ScheduledJob> {
        self.list_scheduled_jobs().await
    }

    async fn remove_scheduled_job(
        &self,
        id: &str,
        remove_recipe: bool,
    ) -> Result<(), SchedulerError> {
        self.remove_scheduled_job(id, remove_recipe).await
    }

    async fn pause_schedule(&self, id: &str) -> Result<(), SchedulerError> {
        self.pause_schedule(id).await
    }

    async fn unpause_schedule(&self, id: &str) -> Result<(), SchedulerError> {
        self.unpause_schedule(id).await
    }

    async fn run_now(&self, id: &str) -> Result<String, SchedulerError> {
        self.run_now(id).await
    }

    async fn sessions(
        &self,
        sched_id: &str,
        limit: usize,
    ) -> Result<Vec<(String, Session)>, SchedulerError> {
        self.sessions(sched_id, limit).await
    }

    async fn update_schedule(
        &self,
        sched_id: &str,
        new_cron: String,
    ) -> Result<(), SchedulerError> {
        self.update_schedule(sched_id, new_cron).await
    }

    async fn update_schedule_spec(
        &self,
        sched_id: &str,
        cron: Option<String>,
        at: Option<DateTime<Utc>>,
        every_seconds: Option<u64>,
        tz: Option<String>,
    ) -> Result<(), SchedulerError> {
        self.update_schedule_spec(sched_id, cron, at, every_seconds, tz)
            .await
    }

    async fn kill_running_job(&self, sched_id: &str) -> Result<(), SchedulerError> {
        self.kill_running_job(sched_id).await
    }

    async fn get_running_job_info(
        &self,
        sched_id: &str,
    ) -> Result<Option<(String, DateTime<Utc>)>, SchedulerError> {
        self.get_running_job_info(sched_id).await
    }

    async fn update_starter_fields(
        &self,
        sched_id: &str,
        starter_id: Option<String>,
        version: Option<String>,
        hash: Option<String>,
        user_customized: bool,
    ) {
        {
            let mut jobs_guard = self.jobs.lock().await;
            if let Some((_, job)) = jobs_guard.get_mut(sched_id) {
                if let Some(sid) = starter_id {
                    job.starter_id = Some(sid);
                }
                if let Some(v) = version {
                    job.starter_version = Some(v);
                }
                if let Some(h) = hash {
                    job.starter_content_hash = Some(h);
                }
                job.user_customized = Some(user_customized);
            }
        }
        if let Err(e) = persist_jobs(&self.storage_path, &self.jobs).await {
            tracing::error!("Failed to persist starter field update: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use tokio::time::{sleep, Duration};

    fn create_test_recipe(dir: &Path, name: &str) -> PathBuf {
        let recipe_path = dir.join(format!("{}.yaml", name));
        fs::write(&recipe_path, "prompt: test\n").unwrap();
        recipe_path
    }

    /// Scheduled jobs run with NO approver: `execute_job` must build its agent
    /// through `new_scheduled_job_agent` so approval-required tools auto-deny
    /// with a recorded skip instead of parking the drain loop forever or
    /// filing undeliverable `tool_approval` decisions (the bare job agent is
    /// never registered in `AgentManager`, so nothing could answer them).
    #[test]
    fn scheduled_job_agents_are_headless() {
        let agent = new_scheduled_job_agent();
        assert!(agent.is_headless(), "scheduled-job agents must be headless");
    }

    #[test]
    fn normalize_cron_converts_5field_and_is_idempotent() {
        // Legacy 5-field → 6-field (seconds prefixed).
        assert_eq!(
            normalize_cron_to_6field("0 6 * * 1-5").as_deref(),
            Some("0 0 6 * * 1-5")
        );
        assert_eq!(
            normalize_cron_to_6field("0 19 * * 0").as_deref(),
            Some("0 0 19 * * 0")
        );
        // Already 6-field → no change (idempotent).
        assert_eq!(normalize_cron_to_6field("0 0 6 * * 1-5"), None);
        // Other field counts left untouched (create_job_task surfaces the error).
        assert_eq!(normalize_cron_to_6field("* * * *"), None);
        assert_eq!(normalize_cron_to_6field(""), None);
    }

    fn ts(s: &str) -> DateTime<Utc> {
        s.parse::<DateTime<Utc>>().unwrap()
    }

    /// DURABILITY / back-compat proof: an old `schedule.json` entry with NONE of
    /// the new reliability/kind fields deserializes cleanly, with every new field
    /// taking its serde default. This is the property that lets us add fields
    /// with zero migration.
    #[test]
    fn old_schedule_json_loads_with_defaults() {
        // Exactly the pre-PR shape (a real legacy cron job) — no run_count,
        // max_retries, last_status, at, every_seconds, tz, etc.
        let old = r#"{
            "id": "legacy-job",
            "source": "/recipes/legacy.yaml",
            "cron": "0 0 8 * * 1-5",
            "last_run": "2026-01-01T08:00:00Z",
            "currently_running": false,
            "paused": false,
            "current_session_id": null,
            "process_start_time": null,
            "starter_id": "storage-insights"
        }"#;
        let job: ScheduledJob = serde_json::from_str(old).expect("old schedule.json must load");
        assert_eq!(job.id, "legacy-job");
        assert_eq!(job.cron, "0 0 8 * * 1-5");
        assert_eq!(job.starter_id.as_deref(), Some("storage-insights"));
        // New fields default — behavior is unchanged for this job.
        assert_eq!(job.run_count, 0);
        assert_eq!(job.retry_count, 0);
        assert_eq!(job.max_retries, 0, "default 0 → no retry, today's behavior");
        assert_eq!(job.last_status, None);
        assert_eq!(job.last_error, None);
        assert_eq!(job.at, None);
        assert_eq!(job.every_seconds, None);
        assert_eq!(job.tz, None);
        // A whole list round-trips too (this is how the store is actually read).
        let list: Vec<ScheduledJob> =
            serde_json::from_str(&format!("[{}]", old)).expect("list must load");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].max_retries, 0);
    }

    /// Kind → validation: EXACTLY one of cron/at/every, with interval bounds.
    #[test]
    fn validate_schedule_spec_requires_exactly_one_kind() {
        // Cron only (today's default) — valid.
        let cron_job = ScheduledJob {
            cron: "0 0 * * * *".to_string(),
            ..Default::default()
        };
        assert!(validate_schedule_spec(&cron_job).is_ok());
        // One-time only — valid.
        let at_job = ScheduledJob {
            at: Some(ts("2026-02-01T00:00:00Z")),
            ..Default::default()
        };
        assert!(validate_schedule_spec(&at_job).is_ok());
        // Interval only — valid.
        let every_job = ScheduledJob {
            every_seconds: Some(3600),
            ..Default::default()
        };
        assert!(validate_schedule_spec(&every_job).is_ok());
        // Zero kinds — invalid.
        assert!(validate_schedule_spec(&ScheduledJob::default()).is_err());
        // Two kinds — invalid.
        let two = ScheduledJob {
            cron: "0 0 * * * *".to_string(),
            every_seconds: Some(60),
            ..Default::default()
        };
        assert!(validate_schedule_spec(&two).is_err());
        // Interval out of bounds — invalid (0 and > 1 year).
        assert!(validate_schedule_spec(&ScheduledJob {
            every_seconds: Some(0),
            ..Default::default()
        })
        .is_err());
        assert!(validate_schedule_spec(&ScheduledJob {
            every_seconds: Some(MAX_INTERVAL_SECS + 1),
            ..Default::default()
        })
        .is_err());
    }

    /// Missed-run detection: given last_run + schedule + now → missed?
    #[test]
    fn is_run_missed_across_kinds() {
        let last = ts("2026-01-01T00:00:00Z");

        // Cron (hourly at :00): a fire at 01:00 was due by 02:00 → missed.
        let cron_job = ScheduledJob {
            cron: "0 0 * * * *".to_string(),
            last_run: Some(last),
            ..Default::default()
        };
        assert!(is_run_missed(&cron_job, ts("2026-01-01T02:00:00Z")));
        // ...but only 30 min later, the next :00 hasn't arrived → not missed.
        assert!(!is_run_missed(&cron_job, ts("2026-01-01T00:30:00Z")));
        // Never-run cron can't be judged missed (no anchor).
        let cron_never = ScheduledJob {
            cron: "0 0 * * * *".to_string(),
            ..Default::default()
        };
        assert!(!is_run_missed(&cron_never, ts("2030-01-01T00:00:00Z")));

        // Interval (1h): 2h elapsed since last run → a fire was due → missed.
        let every = ScheduledJob {
            every_seconds: Some(3600),
            last_run: Some(last),
            ..Default::default()
        };
        assert!(is_run_missed(&every, ts("2026-01-01T02:00:00Z")));
        assert!(!is_run_missed(&every, ts("2026-01-01T00:30:00Z")));

        // One-time: at passed and never ran → missed; already ran → not.
        let at_job = ScheduledJob {
            at: Some(ts("2026-01-01T00:00:00Z")),
            ..Default::default()
        };
        assert!(is_run_missed(&at_job, ts("2026-01-01T01:00:00Z")));
        let at_ran = ScheduledJob {
            at: Some(ts("2026-01-01T00:00:00Z")),
            last_run: Some(ts("2026-01-01T00:00:01Z")),
            ..Default::default()
        };
        assert!(!is_run_missed(&at_ran, ts("2026-01-01T01:00:00Z")));

        // Paused jobs are never missed.
        let paused = ScheduledJob {
            every_seconds: Some(3600),
            last_run: Some(last),
            paused: true,
            ..Default::default()
        };
        assert!(!is_run_missed(&paused, ts("2026-01-01T05:00:00Z")));
    }

    /// Escalation gate: only when retries were configured AND this is the first
    /// failure of a streak (dedup across fires); default jobs never escalate.
    #[test]
    fn should_escalate_failure_gates_on_retries_and_streak() {
        // No retries configured → never escalate (preserves today's behavior).
        assert!(!should_escalate_failure(0, None));
        assert!(!should_escalate_failure(0, Some(ScheduleRunStatus::Error)));
        // Retries configured, first failure of a streak → escalate.
        assert!(should_escalate_failure(3, None));
        assert!(should_escalate_failure(3, Some(ScheduleRunStatus::Ok)));
        assert!(should_escalate_failure(3, Some(ScheduleRunStatus::Missed)));
        // Already Error last time → deduped (don't re-escalate every fire).
        assert!(!should_escalate_failure(3, Some(ScheduleRunStatus::Error)));
    }

    #[test]
    fn retry_backoff_is_bounded_and_monotonic() {
        assert_eq!(retry_backoff(1), std::time::Duration::from_secs(10));
        assert_eq!(retry_backoff(2), std::time::Duration::from_secs(20));
        assert_eq!(retry_backoff(3), std::time::Duration::from_secs(40));
        // Capped at RETRY_MAX_SECS, and no overflow at large attempt counts.
        assert_eq!(
            retry_backoff(1000),
            std::time::Duration::from_secs(RETRY_MAX_SECS)
        );
    }

    #[test]
    fn timezone_resolution_offsets_and_fallback() {
        assert_eq!(parse_fixed_offset("+05:30"), FixedOffset::east_opt(19800));
        assert_eq!(parse_fixed_offset("-08:00"), FixedOffset::east_opt(-28800));
        assert_eq!(parse_fixed_offset("+0000"), FixedOffset::east_opt(0));
        assert_eq!(parse_fixed_offset("not-an-offset"), None);
        // "UTC"/"Z" resolve to a fixed +00:00; an IANA name falls back to Local.
        assert!(matches!(
            resolve_cron_timezone(Some("UTC")),
            CronTimezone::Fixed(_)
        ));
        assert!(matches!(resolve_cron_timezone(None), CronTimezone::Local));
        assert!(matches!(
            resolve_cron_timezone(Some("America/New_York")),
            CronTimezone::Local
        ));
    }

    /// Escalation end-to-end: a persistent failure files an `unblock` decision
    /// into the Decision Inbox (the same table the Enricher writes to).
    #[tokio::test]
    async fn escalation_creates_unblock_decision() {
        let temp = tempdir().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        escalate_persistent_failure(&sm, "flaky-digest", 3, 3, "provider timeout").await;

        let pool = sm.pool_clone().await.unwrap();
        let open = crate::decisions::list_open_decisions(&pool).await.unwrap();
        assert_eq!(open.len(), 1, "one decision should be filed");
        assert_eq!(open[0].decision.kind, "unblock");
        assert!(
            open[0].decision.detail.contains("flaky-digest"),
            "detail names the job: {}",
            open[0].decision.detail
        );
    }

    #[tokio::test]
    async fn test_job_runs_on_schedule() {
        let _guard = env_lock::lock_env([
            ("GOOSE_PROVIDER", Some("openai")),
            ("GOOSE_MODEL", Some("gpt-4o")),
            ("OPENAI_API_KEY", Some("fake-openai-no-keyring")),
            ("OPENAI_CUSTOM_HEADERS", Some("")),
        ]);
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let recipe_path = create_test_recipe(temp_dir.path(), "scheduled_job");
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let scheduler = Scheduler::new(storage_path, session_manager).await.unwrap();

        let job = ScheduledJob {
            id: "scheduled_job".to_string(),
            source: recipe_path.to_string_lossy().to_string(),
            cron: "* * * * * *".to_string(),
            last_run: None,
            currently_running: false,
            paused: false,
            current_session_id: None,
            process_start_time: None,
            worker_persona: None,
            starter_id: None,
            starter_version: None,
            starter_content_hash: None,
            user_customized: None,
            ..Default::default()
        };

        scheduler.add_scheduled_job(job, true).await.unwrap();
        sleep(Duration::from_millis(1500)).await;

        let jobs = scheduler.list_scheduled_jobs().await;
        assert!(jobs[0].last_run.is_some(), "Job should have run");
    }

    #[tokio::test]
    async fn test_paused_job_does_not_run() {
        let _guard = env_lock::lock_env([
            ("GOOSE_PROVIDER", Some("openai")),
            ("GOOSE_MODEL", Some("gpt-4o")),
            ("OPENAI_API_KEY", Some("fake-openai-no-keyring")),
            ("OPENAI_CUSTOM_HEADERS", Some("")),
        ]);
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let recipe_path = create_test_recipe(temp_dir.path(), "paused_job");
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let scheduler = Scheduler::new(storage_path, session_manager).await.unwrap();

        let job = ScheduledJob {
            id: "paused_job".to_string(),
            source: recipe_path.to_string_lossy().to_string(),
            cron: "* * * * * *".to_string(),
            last_run: None,
            currently_running: false,
            paused: false,
            current_session_id: None,
            process_start_time: None,
            worker_persona: None,
            starter_id: None,
            starter_version: None,
            starter_content_hash: None,
            user_customized: None,
            ..Default::default()
        };

        scheduler.add_scheduled_job(job, true).await.unwrap();
        scheduler.pause_schedule("paused_job").await.unwrap();
        sleep(Duration::from_millis(1500)).await;

        let jobs = scheduler.list_scheduled_jobs().await;
        assert!(jobs[0].last_run.is_none(), "Paused job should not run");
    }

    #[tokio::test]
    async fn test_job_with_no_prompt_does_not_panic() {
        let _guard = env_lock::lock_env([
            ("GOOSE_PROVIDER", Some("openai")),
            ("GOOSE_MODEL", Some("gpt-4o")),
            ("OPENAI_API_KEY", Some("fake-openai-no-keyring")),
            ("OPENAI_CUSTOM_HEADERS", Some("")),
        ]);
        let temp_dir = tempdir().unwrap();
        let recipe_path = temp_dir.path().join("no_prompt.yaml");
        fs::write(
            &recipe_path,
            "title: missing\ndescription: no prompt or instructions\n",
        )
        .unwrap();

        let storage_path = temp_dir.path().join("schedule.json");
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let scheduler = Scheduler::new(storage_path, session_manager).await.unwrap();

        let job = ScheduledJob {
            id: "no_prompt_job".to_string(),
            source: recipe_path.to_string_lossy().to_string(),
            cron: "* * * * * *".to_string(),
            last_run: None,
            currently_running: false,
            paused: false,
            current_session_id: None,
            process_start_time: None,
            worker_persona: None,
            starter_id: None,
            starter_version: None,
            starter_content_hash: None,
            user_customized: None,
            ..Default::default()
        };

        // Schedule the job and let it run — should not panic
        scheduler.add_scheduled_job(job, true).await.unwrap();
        sleep(Duration::from_millis(1500)).await;

        // The job should have attempted to run (last_run set) but not crashed the scheduler
        let jobs = scheduler.list_scheduled_jobs().await;
        assert!(
            jobs[0].last_run.is_some(),
            "Job should have attempted to run without panicking"
        );
    }
}
