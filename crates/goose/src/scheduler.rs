use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, Duration, FixedOffset, Local, Utc};
use futures::future::FutureExt as _;
use serde::{Deserialize, Serialize};
use thiserror::Error;
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
        what_it_does: "Runs saved recipes and reminders in the background on cron, one-time, or interval schedules — retrying transient failures, recovering runs missed during downtime, and escalating repeated failures to the Decision Inbox. Guardrails are enforced, not advisory: schedules cannot fire more often than every 15 minutes; automations YOU create land paused until the user approves them in Automate (never resume or re-run one yourself, never recreate one the user deleted, never create a variant because one looks inactive); a job that fails three fires in a row auto-pauses; and a headless scheduled run withholds the two self-replicating extensions (orchestrator, recipe_author) unless the recipe declares them — everything else in the user's enabled set, shell included, is inherited, so declaring `extensions` narrows what a run gets rather than widening it",
        why_it_matters:
            "Lets you promise recurring or future work and actually deliver it without the user re-asking — within limits that exist because an unguarded automation loop once burned real money overnight; work with the approval gate, not around it",
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

use crate::agents::{Agent, SessionConfig};
use crate::agents::{AgentEvent, AgentRuntimeOutcome};
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
use crate::session::{ScheduleSessionSummary, Session, SessionManager};

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
    AnswerRefused(crate::decisions::AnswerError),
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
            SchedulerError::AnswerRefused(e) => write!(f, "{}", e),
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

#[derive(Clone, Serialize, Deserialize, Debug, utoipa::ToSchema)]
pub struct FailedScheduleRun {
    pub run_number: u64,
    pub timestamp: DateTime<Utc>,
    pub session_id: Option<String>,
    pub error: String,
}

#[derive(Clone, Serialize, Deserialize, Debug, utoipa::ToSchema)]
pub struct SchedulePauseEpisode {
    pub id: String,
    pub decision_id: String,
    pub failed_runs: Vec<FailedScheduleRun>,
    pub failure_count: u32,
    pub last_error: String,
    pub headline: String,
    pub effect: Option<serde_json::Value>,
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
    /// When this job last ran and SUCCEEDED. Distinct from `last_run` because
    /// the number that matters is the age of the last success: a job can be
    /// firing every day and not have worked in three weeks, and `last_run`
    /// alone reads as healthy the whole time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success: Option<DateTime<Utc>>,
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

    // ── Runaway guardrails (2026-08-05 credit-burn incident) ──
    /// Failures since the last successful fire. Reset on success; at
    /// `AUTO_PAUSE_AFTER_CONSECUTIVE_FAILURES` the job is auto-paused.
    #[serde(default)]
    pub consecutive_failures: u32,
    /// True when this job was created by an agent and the user has not yet
    /// approved it. Such jobs are created paused; agent tools refuse to
    /// unpause them — only the user's unpause (Automate UI) clears the flag.
    #[serde(default)]
    pub requires_approval: bool,
    /// True when the most recent run was a startup catch-up for a window the
    /// daemon slept through, rather than a fire at the job's own scheduled
    /// instant. Recorded so a catch-up is never passed off as an on-time run.
    #[serde(default)]
    pub last_run_was_catch_up: bool,
    /// Persisted with the breaker, so inbox/delivery failure cannot lose work.
    #[serde(default)]
    pub pause_episode: Option<SchedulePauseEpisode>,
    #[serde(default)]
    pub failed_runs: Vec<FailedScheduleRun>,
    /// Catch-up starts here after resume; failure history keeps its real dates.
    #[serde(default)]
    pub resumed_at: Option<DateTime<Utc>>,
}

/// Reliability + kind constants. Retry backoff is exponential and capped so a
/// flapping job cannot storm.
const RETRY_BASE_SECS: u64 = 10;
const RETRY_MAX_SECS: u64 = 300;
/// Fires that fail back-to-back before the job is auto-paused.
const AUTO_PAUSE_AFTER_CONSECUTIVE_FAILURES: u32 = 3;
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
    if let Some(violation) = schedule_floor_violation(job) {
        return Err(violation);
    }
    Ok(())
}

/// Minimum allowed firing interval for any schedule, in seconds. Every-minute
/// agent-created crons burned ~$20 of API credits overnight on 2026-08-05; a
/// floor makes that class of runaway impossible to install. A shipped build has
/// no override: `PERMAGENT_MIN_SCHEDULE_INTERVAL_SECS` is read in test builds
/// only, because a guardrail a runtime env var can switch off is advisory.
const DEFAULT_MIN_SCHEDULE_INTERVAL_SECS: u64 = 900;

#[cfg(test)]
fn min_schedule_interval_secs() -> u64 {
    std::env::var("PERMAGENT_MIN_SCHEDULE_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MIN_SCHEDULE_INTERVAL_SECS)
}

#[cfg(not(test))]
fn min_schedule_interval_secs() -> u64 {
    DEFAULT_MIN_SCHEDULE_INTERVAL_SECS
}

/// `Some(reason)` when the job would fire more often than the interval floor.
/// Cron cadence is measured empirically: the minimum gap over the next several
/// occurrences (catches `*/1 * * * *` and dense field lists alike). `at` jobs
/// fire once and are exempt.
fn schedule_floor_violation(job: &ScheduledJob) -> Option<String> {
    let floor = min_schedule_interval_secs();
    if floor == 0 {
        return None;
    }
    if let Some(secs) = job.every_seconds {
        if secs < floor {
            return Some(format!(
                "interval every_seconds={} is below the minimum of {}s; \
                 this floor is not user-overridable",
                secs, floor
            ));
        }
    }
    let cron_text = job.cron.trim();
    if !cron_text.is_empty() {
        if let Some(cron) = parse_cron_schedule(cron_text) {
            // Only gaps BETWEEN occurrences measure cadence; the lead-in from
            // "now" to the first fire does not.
            let mut cursor = Utc::now();
            let mut prev: Option<DateTime<Utc>> = None;
            let mut min_gap: Option<i64> = None;
            for _ in 0..8 {
                let Some(next) = cron.find_next_occurrence(&cursor, false).ok() else {
                    break;
                };
                if let Some(p) = prev {
                    let gap = next.signed_duration_since(p).num_seconds();
                    min_gap = Some(min_gap.map_or(gap, |m: i64| m.min(gap)));
                }
                prev = Some(next);
                cursor = next;
            }
            if let Some(gap) = min_gap {
                if gap > 0 && (gap as u64) < floor {
                    return Some(format!(
                        "cron '{}' fires every {}s — below the minimum of {}s; \
                         this floor is not user-overridable",
                        cron_text, gap, floor
                    ));
                }
            }
        }
    }
    None
}

/// A cron job is only caught up when its period is LONGER than a day. This
/// preserves the original no-catch-up ruling exactly where that ruling was
/// right: a sub-daily cron gets many chances per day, so a missed tick is a
/// blip and firing every missed job at boot is a wake-storm. It is wrong only
/// at long periods — a weekly job needs the daemon alive at one specific
/// 60-second instant per week, and on a dev machine that restarts several
/// times a day the odds of hitting it are poor. `storage-insights` (Sunday
/// 19:00) went five days stale that way and would have stayed stale until the
/// next Sunday.
const CATCH_UP_MIN_PERIOD: Duration = Duration::hours(24);
/// How late a missed fire may be and still be worth running. Past this the run
/// is stale noise rather than a recovery, and the job's own next natural tick
/// is close enough.
const CATCH_UP_MAX_LATENESS: Duration = Duration::days(3);
/// Space catch-up runs so a boot with several of them due does not start them
/// all at once — the wake-storm the original ruling was written to avoid.
const CATCH_UP_STAGGER_SECS: u64 = 30;

const CLAIM_TICK_TOLERANCE: Duration = Duration::seconds(900);
// A seconds-granularity cron has at most 901 occurrences in the inclusive
// window, even when tests disable the production 900-second interval floor.
const CLAIM_TICK_MAX_OCCURRENCES: usize = 902;
const CLAIM_EXTENDED_LOOKBACK: Duration = Duration::hours(24);
// The production 900-second interval floor permits at most 97 inclusive fires.
const CLAIM_EXTENDED_MAX_OCCURRENCES: usize = 100;
// A wedge detector for manual admission, NOT a timeout or cancellation policy.
const CLAIM_WEDGE_AGE: Duration = Duration::days(7);

/// Name the nominal wall-clock occurrence, never the callback's arrival time.
/// IANA names currently fall back to Local (chrono-tz is not yet wired): two
/// processes with different TZ environments must still contend for the same
/// nominal 09:00, even when they reach it at different instants. Local remains
/// a dynamic zone, not a snapshot of today's offset. Pre-existing cron
/// semantics: a repeated fall-back 01:30 runs once; a nonexistent spring-forward
/// 02:30 does not fire that day.
fn cron_fire_key(job: &ScheduledJob, occurrence: DateTime<Utc>) -> String {
    let naive = match resolve_cron_timezone(job.tz.as_deref()) {
        CronTimezone::Local => occurrence.with_timezone(&Local).naive_local(),
        CronTimezone::Fixed(offset) => occurrence.with_timezone(&offset).naive_local(),
    };
    format!("cron:{}", naive.format("%Y-%m-%dT%H:%M:%S"))
}

fn recent_cron_occurrence<T: chrono::TimeZone>(
    cron: &croner::Cron,
    now: DateTime<T>,
) -> Option<DateTime<Utc>> {
    let start = now.clone() - CLAIM_TICK_TOLERANCE;
    cron.iter_from(start)
        .take(CLAIM_TICK_MAX_OCCURRENCES)
        .take_while(|at| at <= &now)
        .last()
        .map(|at| at.with_timezone(&Utc))
        .or_else(|| {
            cron.iter_from(now.clone() - CLAIM_EXTENDED_LOOKBACK)
                .take(CLAIM_EXTENDED_MAX_OCCURRENCES)
                .take_while(|at| at <= &now)
                .last()
                .map(|at| at.with_timezone(&Utc))
        })
}

fn key_for_tick(job: &ScheduledJob, now: DateTime<Utc>) -> (String, &'static str) {
    if let Some(at) = job.at {
        return (format!("at:{}", at.to_rfc3339()), "at");
    }
    if let Some(every) = job.every_seconds.filter(|every| *every > 0) {
        // Validated intervals fit i64 (MAX_INTERVAL_SECS). Euclidean division
        // also gives the correct epoch bucket for instants before the epoch.
        let every = i64::try_from(every).unwrap_or(i64::MAX);
        return (
            format!("interval:{}", now.timestamp().div_euclid(every) * every),
            "interval",
        );
    }
    let occurrence = parse_cron_schedule(&job.cron).and_then(|cron| {
        match resolve_cron_timezone(job.tz.as_deref()) {
            CronTimezone::Local => recent_cron_occurrence(&cron, now.with_timezone(&Local)),
            CronTimezone::Fixed(offset) => {
                recent_cron_occurrence(&cron, now.with_timezone(&offset))
            }
        }
    });
    if let Some(at) = occurrence {
        return (cron_fire_key(job, at), "cron");
    }
    // Neither bounded search found an occurrence. This fail-awake fallback is
    // the one remaining key-derivation case where two processes can both win.
    tracing::warn!(job = %job.id,
        "No cron occurrence in either claim search window; fallback key may let two processes both win");
    let minute = DateTime::<Utc>::from_timestamp(now.timestamp().div_euclid(60) * 60, 0)
        .expect("minute containing a valid callback timestamp");
    (format!("fallback:{}", minute.to_rfc3339()), "cron")
}

fn key_for_catch_up(job: &ScheduledJob) -> Option<String> {
    missed_fire_at(job).map(|at| cron_fire_key(job, at))
}

#[derive(Clone)]
struct ClaimOwner {
    pid: i64,
    token: String,
}

fn claim_owner() -> &'static ClaimOwner {
    static OWNER: std::sync::OnceLock<ClaimOwner> = std::sync::OnceLock::new();
    OWNER.get_or_init(|| ClaimOwner {
        pid: i64::from(std::process::id()),
        token: uuid::Uuid::new_v4().to_string(),
    })
}

#[derive(Debug, PartialEq, Eq)]
enum ClaimOutcome {
    Won,
    Lost,
    StoreUnavailable(String),
}

async fn prune_claims(pool: &sqlx::SqlitePool, now: DateTime<Utc>, alive: fn(i64) -> bool) {
    let cutoff = (now - Duration::days(7)).to_rfc3339();
    let _ = sqlx::query(
        "DELETE FROM schedule_fire_claims WHERE finished_at IS NOT NULL
         AND julianday(finished_at) < julianday(?)",
    )
    .bind(&cutoff)
    .execute(pool)
    .await;
    let rows = sqlx::query_as::<_, (String, String, i64, String, String)>(
        "SELECT job_id, fire_key, owner_pid, owner_token, claimed_at FROM schedule_fire_claims
         WHERE finished_at IS NULL AND started_at IS NULL
         AND julianday(claimed_at) < julianday(?)",
    )
    .bind(&cutoff)
    .fetch_all(pool)
    .await;
    if let Ok(rows) = rows {
        for (job_id, fire_key, pid, token, claimed_at) in rows {
            if alive(pid) {
                continue;
            }
            // Preserve any takeover or start that raced the liveness probe.
            let _ = sqlx::query(
                "DELETE FROM schedule_fire_claims
                 WHERE job_id = ? AND fire_key = ? AND owner_pid = ? AND owner_token = ?
                 AND claimed_at = ? AND finished_at IS NULL AND started_at IS NULL
                 AND julianday(claimed_at) < julianday(?)",
            )
            .bind(job_id)
            .bind(fire_key)
            .bind(pid)
            .bind(token)
            .bind(claimed_at)
            .bind(&cutoff)
            .execute(pool)
            .await;
        }
    }
}

async fn try_claim_fire(
    pool: &sqlx::SqlitePool,
    job_id: &str,
    fire_key: &str,
    kind: &str,
    owner: &ClaimOwner,
) -> ClaimOutcome {
    try_claim_fire_with_probe(
        pool,
        job_id,
        fire_key,
        kind,
        owner,
        Utc::now(),
        owner_is_alive,
    )
    .await
}

async fn try_claim_fire_with_probe(
    pool: &sqlx::SqlitePool,
    job_id: &str,
    fire_key: &str,
    kind: &str,
    owner: &ClaimOwner,
    now: DateTime<Utc>,
    alive: fn(i64) -> bool,
) -> ClaimOutcome {
    let result = sqlx::query(
        "INSERT INTO schedule_fire_claims
         (job_id, fire_key, kind, claimed_at, owner_pid, owner_token)
         VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(job_id, fire_key) DO NOTHING",
    )
    .bind(job_id)
    .bind(fire_key)
    .bind(kind)
    .bind(now.to_rfc3339())
    .bind(owner.pid)
    .bind(&owner.token)
    .execute(pool)
    .await;
    match result {
        Ok(result) if result.rows_affected() == 1 => {
            prune_claims(pool, now, alive).await;
            ClaimOutcome::Won
        }
        Ok(_) => {
            let row = sqlx::query_as::<_, (i64, String, Option<String>, Option<String>)>(
                "SELECT owner_pid, owner_token, started_at, finished_at
                 FROM schedule_fire_claims WHERE job_id = ? AND fire_key = ?",
            )
            .bind(job_id)
            .bind(fire_key)
            .fetch_optional(pool)
            .await;
            let (pid, token, started, finished) = match row {
                Ok(Some(row)) => row,
                Ok(None) => return ClaimOutcome::Lost,
                Err(error) => return ClaimOutcome::StoreUnavailable(error.to_string()),
            };
            if finished.is_some() || token == owner.token || pid == owner.pid || alive(pid) {
                return ClaimOutcome::Lost;
            }
            if started.is_some() {
                tracing::warn!(job = %job_id, fire_key = %fire_key,
                    "Dead claim owner started this occurrence; it may be half-done; human review required");
                return ClaimOutcome::Lost;
            }
            // Only an unstarted occurrence can change hands. Both racing takers
            // compare the old token; SQLite allows exactly one guarded update.
            match sqlx::query(
                "UPDATE schedule_fire_claims SET owner_pid = ?, owner_token = ?, claimed_at = ?
                 WHERE job_id = ? AND fire_key = ? AND owner_token = ?
                 AND finished_at IS NULL AND started_at IS NULL",
            )
            .bind(owner.pid)
            .bind(&owner.token)
            .bind(now.to_rfc3339())
            .bind(job_id)
            .bind(fire_key)
            .bind(token)
            .execute(pool)
            .await
            {
                Ok(result) if result.rows_affected() == 1 => ClaimOutcome::Won,
                Ok(_) => ClaimOutcome::Lost,
                Err(error) => ClaimOutcome::StoreUnavailable(error.to_string()),
            }
        }
        Err(error) => ClaimOutcome::StoreUnavailable(error.to_string()),
    }
}

#[derive(Clone)]
struct FireClaim {
    pool: sqlx::SqlitePool,
    job_id: String,
    fire_key: String,
    owner: ClaimOwner,
}

impl FireClaim {
    async fn start(&self) -> anyhow::Result<()> {
        // Fail awake on a store error after admission: a missing durable start
        // mark risks replay if we die mid-run, but a database hiccup must not
        // skip the automation. A successful write matching no row still means
        // ownership was lost and does not authorize execution.
        let result = sqlx::query(
            "UPDATE schedule_fire_claims SET started_at = ?
             WHERE job_id = ? AND fire_key = ? AND owner_token = ? AND owner_pid = ?
             AND finished_at IS NULL AND started_at IS NULL",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(&self.job_id)
        .bind(&self.fire_key)
        .bind(&self.owner.token)
        .bind(self.owner.pid)
        .execute(&self.pool)
        .await;
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(job = %self.job_id, fire_key = %self.fire_key, %error,
                    "Start-marker write failed; running fail-awake risks replay by takeover if this process dies mid-run");
                return Ok(());
            }
        };
        if result.rows_affected() != 1 {
            return Err(anyhow!(
                "Fire claim no longer admits execution for '{}' ({})",
                self.job_id,
                self.fire_key
            ));
        }
        Ok(())
    }

    async fn finish(&self) {
        let _ = sqlx::query(
            "UPDATE schedule_fire_claims SET finished_at = ?
             WHERE job_id = ? AND fire_key = ? AND owner_token = ? AND owner_pid = ?
             AND finished_at IS NULL",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(&self.job_id)
        .bind(&self.fire_key)
        .bind(&self.owner.token)
        .bind(self.owner.pid)
        .execute(&self.pool)
        .await;
    }
}

// Some(None) means fail awake; None means another process already owns the
// fire. Carry Some(claim) through retries/catch-up, never re-claim a continuation.
async fn admit_fire(
    manager: &SessionManager,
    job_id: &str,
    key: String,
    kind: &str,
) -> Option<Option<FireClaim>> {
    let pool = match manager.pool_clone().await {
        Ok(pool) => pool,
        Err(error) => {
            tracing::warn!(job = %job_id, %error, "Fire claim store unavailable; running fail-awake");
            return Some(None);
        }
    };
    match try_claim_fire(&pool, job_id, &key, kind, claim_owner()).await {
        ClaimOutcome::Won => Some(Some(FireClaim {
            pool,
            job_id: job_id.to_string(),
            fire_key: key,
            owner: claim_owner().clone(),
        })),
        ClaimOutcome::Lost => {
            tracing::info!(job = %job_id, fire_key = %key, "Scheduled fire already claimed; skipping");
            None
        }
        ClaimOutcome::StoreUnavailable(error) => {
            tracing::warn!(job = %job_id, %error, "Fire claim store unavailable; running fail-awake");
            Some(None)
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn owner_is_alive(pid: i64) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    // Signal zero probes existence/permission without delivering a signal.
    (unsafe { libc::kill(pid, 0) == 0 })
        || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn owner_is_alive(_pid: i64) -> bool {
    false
}

/// Serialize the liveness read and manual row insertion with a write lock.
/// A plain SELECT followed by INSERT lets two manual requests both pass. The
/// first (no-op) write acquires SQLite's reserved lock before taking a snapshot.
/// A transaction also rolls back on cancellation/errors, returning a clean pool
/// connection. The seven-day wedge detector does not cancel a running job.
async fn try_claim_manual(
    pool: &sqlx::SqlitePool,
    job_id: &str,
    key: &str,
    owner: &ClaimOwner,
    now: DateTime<Utc>,
    alive: fn(i64) -> bool,
) -> ClaimOutcome {
    let result: Result<bool, sqlx::Error> = async {
        let mut tx = pool.begin().await?;
        sqlx::query("UPDATE schedule_fire_claims SET finished_at = finished_at WHERE 0")
            .execute(&mut *tx).await?;
        let rows = sqlx::query_as::<_, (String, i64, String, String)>(
            "SELECT fire_key, owner_pid, owner_token, claimed_at FROM schedule_fire_claims
             WHERE job_id = ? AND finished_at IS NULL",
        ).bind(job_id).fetch_all(&mut *tx).await?;
        for (fire_key, pid, token, claimed_at) in rows {
            let wedged = DateTime::parse_from_rfc3339(&claimed_at)
                .is_ok_and(|claimed| now.signed_duration_since(claimed) > CLAIM_WEDGE_AGE);
            if wedged {
                tracing::error!(job = %job_id, fire_key = %fire_key,
                    "Seven-day wedge detector: finishing unfinished claim so run-now is not blocked; this is not a run timeout");
            } else if pid == owner.pid && token == owner.token {
                continue;
            }
            if !wedged && alive(pid) {
                return Ok(false);
            }
            sqlx::query(
                "UPDATE schedule_fire_claims SET finished_at = ?
                 WHERE job_id = ? AND fire_key = ? AND finished_at IS NULL",
            ).bind(now.to_rfc3339()).bind(job_id).bind(fire_key)
                .execute(&mut *tx).await?;
        }
        sqlx::query(
            "INSERT INTO schedule_fire_claims
             (job_id, fire_key, kind, claimed_at, owner_pid, owner_token)
             VALUES (?, ?, 'manual', ?, ?, ?)",
        ).bind(job_id).bind(key).bind(now.to_rfc3339())
            .bind(owner.pid).bind(&owner.token).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(true)
    }.await;
    match result {
        Ok(true) => {
            prune_claims(pool, now, alive).await;
            ClaimOutcome::Won
        }
        Ok(false) => ClaimOutcome::Lost,
        Err(error) => ClaimOutcome::StoreUnavailable(error.to_string()),
    }
}

/// Should this missed job be run once, late, at startup?
///
/// Pure so the bound is testable without a scheduler: `period` is the job's
/// natural cron interval and `lateness` is how long ago the fire it missed was
/// due. One-shot (`at`) and interval jobs are excluded by the caller — `at`
/// jobs already self-catch-up, and an interval job re-fires on its own within
/// one interval.
fn should_catch_up(period: Duration, lateness: Duration) -> bool {
    period > CATCH_UP_MIN_PERIOD
        && lateness >= Duration::zero()
        && lateness <= CATCH_UP_MAX_LATENESS
}

/// The cron's natural period, measured empirically as the SMALLEST gap over
/// the next several occurrences — the same way `schedule_floor_violation`
/// measures cadence.
///
/// Smallest, not the next gap, because a cron's spacing is not uniform: a
/// weekday job (`* * 1-5`) has a three-day gap across every weekend, and
/// reading that one gap would make a job with a chance to run every weekday
/// look like a job that only runs twice a week. The question this answers is
/// "how often does this job get an opportunity, at its most frequent" — and if
/// that is daily or better, a missed window is a blip, not a lost week.
fn cron_period(cron: &str, from: DateTime<Utc>) -> Option<Duration> {
    let schedule = parse_cron_schedule(cron)?;
    let mut cursor = from;
    let mut prev: Option<DateTime<Utc>> = None;
    let mut min_gap: Option<Duration> = None;
    for _ in 0..8 {
        let next = schedule.find_next_occurrence(&cursor, false).ok()?;
        if let Some(p) = prev {
            let gap = next - p;
            min_gap = Some(min_gap.map_or(gap, |m: Duration| m.min(gap)));
        }
        prev = Some(next);
        cursor = next;
    }
    min_gap
}

/// The fire this job missed: the first scheduled occurrence strictly after
/// `last_run`. `None` when the job has never run or the cron will not parse.
fn missed_fire_at(job: &ScheduledJob) -> Option<DateTime<Utc>> {
    let last_run = job.last_run.into_iter().chain(job.resumed_at).max()?;
    let schedule = parse_cron_schedule(&job.cron)?;
    // Search IN the armed clock; converting a UTC search afterwards changes
    // which occurrence a non-UTC expression names.
    match resolve_cron_timezone(job.tz.as_deref()) {
        CronTimezone::Local => schedule
            .find_next_occurrence(&last_run.with_timezone(&Local), false)
            .ok()
            .map(|at| at.with_timezone(&Utc)),
        CronTimezone::Fixed(offset) => schedule
            .find_next_occurrence(&last_run.with_timezone(&offset), false)
            .ok()
            .map(|at| at.with_timezone(&Utc)),
    }
}

/// Pure missed-run predicate: was a scheduled fire due in the window since
/// `last_run` that did not run? Used at startup to flag `Missed`. Paused jobs are
/// never "missed". Cron occurrences are evaluated in the job's armed clock.
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
        return match job.last_run.into_iter().chain(job.resumed_at).max() {
            // Never run → interval anchors from arm time (unknown at load); don't
            // claim a miss.
            None => false,
            Some(lr) => now.signed_duration_since(lr).num_seconds().max(0) as u64 >= secs,
        };
    }
    // Cron: the next scheduled fire strictly after last_run is already in the
    // past. Requires a known last_run (a never-run cron can't be judged missed).
    missed_fire_at(job).is_some_and(|next| next <= now)
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

fn lock_schedule_file(storage_path: &Path) -> Result<fs::File, SchedulerError> {
    if let Some(parent) = storage_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut lock_path = storage_path.as_os_str().to_owned();
    lock_path.push(".lock");
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(PathBuf::from(lock_path))?;
    fs2::FileExt::lock_exclusive(&lock)?;
    Ok(lock)
}

/// Write a jobs snapshot to `storage_path` atomically (temp file + rename), so a
/// concurrent reader (`sync_from_storage`, another instance) never observes a
/// half-written file. Takes no lock — callers pass an already-snapshotted list.
fn write_jobs_to_disk(storage_path: &Path, list: &[ScheduledJob]) -> Result<(), SchedulerError> {
    let _lock = lock_schedule_file(storage_path)?;
    let disk: Vec<ScheduledJob> = fs::read_to_string(storage_path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    let merged: Vec<_> = list
        .iter()
        .map(|job| {
            disk.iter()
                .find(|stored| stored.id == job.id && newer_schedule_state(stored, job))
                .unwrap_or(job)
                .clone()
        })
        .collect();
    write_locked_jobs_to_disk(storage_path, &merged)
}

fn newer_schedule_state(stored: &ScheduledJob, live: &ScheduledJob) -> bool {
    stored.run_count > live.run_count || stored.resumed_at > live.resumed_at
}

fn refresh_schedule_state(storage_path: &Path, jobs: &mut JobsMap) {
    let disk: Vec<ScheduledJob> = fs::read_to_string(storage_path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    for stored in disk {
        if let Some((_, live)) = jobs.get_mut(&stored.id) {
            if !live.currently_running && newer_schedule_state(&stored, live) {
                *live = stored;
            }
        }
    }
}

fn write_locked_jobs_to_disk(
    storage_path: &Path,
    list: &[ScheduledJob],
) -> Result<(), SchedulerError> {
    if let Some(parent) = storage_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(list)?;
    let mut tmp = storage_path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, data)?;
    fs::OpenOptions::new().write(true).open(&tmp)?.sync_all()?;
    fs::rename(&tmp, storage_path)?;
    #[cfg(unix)]
    if let Some(parent) = storage_path.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
tokio::task_local! {
    static FAIL_PAUSE_PERSIST: bool;
}

async fn persist_jobs(
    storage_path: &Path,
    jobs: &Arc<Mutex<JobsMap>>,
) -> Result<(), SchedulerError> {
    // The lock is intentionally held across the write: it serializes every disk
    // write with the in-memory map so `sync_from_storage` (which reads disk under
    // the same lock) never sees a stale snapshot and removes a live job. Writing
    // unlocked would let a fire's stale snapshot clobber a concurrent add.
    let jobs_guard = jobs.lock().await;
    let list: Vec<ScheduledJob> = jobs_guard.values().map(|(_, j)| j.clone()).collect();
    #[cfg(test)]
    if FAIL_PAUSE_PERSIST.try_with(|fail| *fail).unwrap_or(false)
        && list.iter().any(|job| job.pause_episode.is_some())
    {
        return Err(SchedulerError::PersistError(
            "injected pause persist failure".to_string(),
        ));
    }
    write_jobs_to_disk(storage_path, &list)
}

fn record_failed_fire(job: &mut ScheduledJob, result: &anyhow::Result<String>) {
    if let Err(error) = result {
        job.failed_runs.push(FailedScheduleRun {
            run_number: job.run_count.saturating_add(1),
            timestamp: Utc::now(),
            session_id: job.current_session_id.clone(),
            error: error.to_string(),
        });
    }
}

fn begin_pause_episode(job: &mut ScheduledJob) {
    let name = fs::read_to_string(&job.source)
        .ok()
        .and_then(|text| serde_yaml::from_str::<Recipe>(&text).ok())
        .map(|recipe| recipe.title)
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| job.id.clone());
    let suffix = " paused after 3 failed runs";
    let headline = format!(
        "{}{}",
        name.chars()
            .take(80 - suffix.chars().count())
            .collect::<String>(),
        suffix
    );
    let mut failed_runs: Vec<_> = job.failed_runs.iter().rev().take(3).cloned().collect();
    failed_runs.reverse();
    job.pause_episode = Some(SchedulePauseEpisode {
        id: uuid::Uuid::now_v7().to_string(),
        decision_id: uuid::Uuid::now_v7().to_string(),
        failed_runs,
        failure_count: job.consecutive_failures,
        last_error: job.last_error.clone().unwrap_or_default(),
        headline,
        effect: None,
    });
}

async fn recover_pause_job(
    manager: &SessionManager,
    job: &mut ScheduledJob,
) -> Result<(), SchedulerError> {
    let pool = manager.pool_clone().await?;
    let raw: Option<String> = sqlx::query_scalar(
        "SELECT job_json FROM schedule_pause_episodes WHERE job_id = ? AND (answer IS NULL OR answer = 'reject' OR resolved_at IS NULL)
         ORDER BY pause_episode_id DESC LIMIT 1",
    ).bind(&job.id).fetch_optional(&pool).await.map_err(|e| SchedulerError::PersistError(e.to_string()))?;
    if let Some(raw) = raw {
        let persisted: ScheduledJob = serde_json::from_str(&raw)?;
        if persisted.run_count > job.run_count {
            *job = persisted;
        }
    }
    Ok(())
}

async fn stage_pause_jobs(
    manager: &SessionManager,
    jobs: &Arc<Mutex<JobsMap>>,
) -> Result<(), SchedulerError> {
    let snapshot: Vec<_> = jobs
        .lock()
        .await
        .values()
        .filter(|(_, job)| job.pause_episode.is_some())
        .map(|(_, job)| job.clone())
        .collect();
    if snapshot.is_empty() {
        return Ok(());
    }
    let pool = manager.pool_clone().await?;
    for job in snapshot {
        crate::decisions::stage_schedule_pause(&pool, &job)
            .await
            .map_err(SchedulerError::PersistError)?;
    }
    Ok(())
}

async fn reconcile_pause_jobs(
    manager: &SessionManager,
    storage_path: &Path,
) -> Result<(), SchedulerError> {
    let snapshot: Vec<ScheduledJob> = match fs::read_to_string(storage_path) {
        Ok(raw) => serde_json::from_str(&raw)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if !snapshot.iter().any(|job| job.pause_episode.is_some()) {
        return Ok(());
    }
    let pool = manager.pool_clone().await?;
    for job in snapshot {
        crate::decisions::reconcile_schedule_pause(&pool, &job)
            .await
            .map_err(SchedulerError::PersistError)?;
        if let Some(episode) = &job.pause_episode {
            if let Some(effect) = &episode.effect {
                crate::decisions::resolve_schedule_pause(&pool, &job.id, &episode.id, effect)
                    .await
                    .map_err(SchedulerError::PersistError)?;
            }
        }
    }
    Ok(())
}

#[derive(Clone)]
struct ResumeAuthority {
    episode: String,
    audit: serde_json::Value,
}

tokio::task_local! {
    // Carries authenticated authority across the existing SchedulerTrait seam;
    // agent-facing unpause cannot manufacture a pending episode answer.
    static RESUME_AUTHORITY: ResumeAuthority;
}

/// Authentication is supplied by the HTTP principal boundary, never inferred
/// from decision payload. The decision id binds the answer to its ledger row.
pub struct SchedulePauseAnswer<'a> {
    pub decision_id: &'a str,
    pub answer: &'a str,
    pub acted_by: &'a str,
    pub principal: &'a str,
    pub from_automate: bool,
}

fn normalize_schedule_answer_note(note: Option<&str>) -> Option<String> {
    note.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

/// Shared by the authenticated Inbox and Automate routes. An intent survives a
/// crash; a matching file effect makes retry safe without replaying a resume.
pub async fn scheduled_job_resume(
    scheduler: &dyn SchedulerTrait,
    pool: &sqlx::SqlitePool,
    job_id: &str,
    episode_id: &str,
    request: SchedulePauseAnswer<'_>,
) -> Result<(), SchedulerError> {
    scheduled_job_resume_with_note(scheduler, pool, job_id, episode_id, request, None).await
}

pub async fn scheduled_job_resume_with_note(
    scheduler: &dyn SchedulerTrait,
    pool: &sqlx::SqlitePool,
    job_id: &str,
    episode_id: &str,
    request: SchedulePauseAnswer<'_>,
    note: Option<&str>,
) -> Result<(), SchedulerError> {
    let SchedulePauseAnswer {
        decision_id,
        answer,
        acted_by,
        principal,
        from_automate,
    } = request;
    let answer_note = normalize_schedule_answer_note(note);
    let decision = crate::decisions::get_decision(pool, decision_id)
        .await
        .map_err(SchedulerError::PersistError)?
        .ok_or(SchedulerError::AnswerRefused(
            crate::decisions::AnswerError::NotFound,
        ))?;
    if let Err(error) =
        crate::decisions::check_answer_authority(pool, &decision, acted_by, Some(principal)).await
    {
        crate::decisions::audit_schedule_refusal(pool, &decision, acted_by, principal)
            .await
            .map_err(SchedulerError::PersistError)?;
        return Err(SchedulerError::AnswerRefused(error));
    }
    if !matches!(answer, "approve" | "reject") {
        return Err(SchedulerError::AnswerRefused(
            crate::decisions::AnswerError::Invalid(
                "authenticated scheduler answer required".to_string(),
            ),
        ));
    }
    let job = scheduler
        .list_scheduled_jobs()
        .await
        .into_iter()
        .find(|j| j.id == job_id)
        .ok_or_else(|| SchedulerError::JobNotFound(job_id.to_string()))?;
    let episode = job
        .pause_episode
        .as_ref()
        .filter(|e| e.id == episode_id)
        .ok_or_else(|| SchedulerError::AnyhowError(anyhow!("stale pause episode")))?;
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|e| SchedulerError::PersistError(e.to_string()))?;
    let row: (Option<String>, Option<String>, String) = sqlx::query_as(
        "SELECT answer, resolved_at, decision_id FROM schedule_pause_episodes WHERE job_id = ? AND pause_episode_id = ?",
    ).bind(job_id).bind(episode_id).fetch_one(&mut *tx).await.map_err(|e| SchedulerError::PersistError(e.to_string()))?;
    if row.2 != decision_id
        || decision.kind != "unblock"
        || decision.payload["reason"] != crate::decisions::CONSECUTIVE_FAILURE_LIMIT
    {
        tx.rollback()
            .await
            .map_err(|e| SchedulerError::PersistError(e.to_string()))?;
        crate::decisions::audit_schedule_refusal(pool, &decision, acted_by, principal)
            .await
            .map_err(SchedulerError::PersistError)?;
        return Err(SchedulerError::AnswerRefused(
            crate::decisions::AnswerError::Forbidden(
                "answered decision does not own this pause episode".to_string(),
            ),
        ));
    }
    if decision.status != "open" && row.0.is_none() {
        return Err(SchedulerError::AnswerRefused(
            crate::decisions::AnswerError::AlreadyResolved(decision.status),
        ));
    }
    if let Some(prior) = &row.0 {
        if prior != answer {
            if from_automate && prior == "reject" && answer == "approve" && row.1.is_some() {
                // A later explicit Automate resume supersedes acknowledgement,
                // retaining the original answer in the append-only audit.
                sqlx::query("UPDATE schedule_pause_episodes SET answer = NULL, resolved_at = NULL, effect_json = NULL
                    WHERE job_id = ? AND pause_episode_id = ?")
                    .bind(job_id).bind(episode_id).execute(&mut *tx).await
                    .map_err(|e| SchedulerError::PersistError(e.to_string()))?;
            } else {
                return Err(SchedulerError::AnyhowError(anyhow!(
                    "pause episode already has a different answer"
                )));
            }
        } else if row.1.is_some() {
            return Ok(());
        }
    }
    sqlx::query(
        "UPDATE schedule_pause_episodes SET answer = ?, actor = ?, principal = ?, requested_at = ?
        WHERE job_id = ? AND pause_episode_id = ? AND answer IS NULL",
    )
    .bind(answer)
    .bind(acted_by)
    .bind(principal)
    .bind(Utc::now().to_rfc3339())
    .bind(job_id)
    .bind(episode_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| SchedulerError::PersistError(e.to_string()))?;
    // Persist note to the same field the generic decision path uses so audit
    // and history read one source of truth, even if effect resolution retries.
    sqlx::query(
        "UPDATE decisions SET answer_note = COALESCE(answer_note, ?)
        WHERE id = ? AND status = 'open'",
    )
    .bind(&answer_note)
    .bind(decision_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| SchedulerError::PersistError(e.to_string()))?;
    let audit: (String, String, String) = sqlx::query_as(
        "SELECT actor, principal, requested_at FROM schedule_pause_episodes WHERE job_id = ? AND pause_episode_id = ?",
    ).bind(job_id).bind(episode_id).fetch_one(&mut *tx).await.map_err(|e| SchedulerError::PersistError(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| SchedulerError::PersistError(e.to_string()))?;
    let effect = serde_json::json!({
        "operation": if answer == "approve" { "scheduled_job_resume" } else { "keep_paused" },
        "actor": audit.0, "principal": audit.1, "time": audit.2,
        "decision_id": row.2, "pause_episode_id": episode_id,
        "before": {"paused": job.paused, "consecutive_failures": job.consecutive_failures, "retry_count": job.retry_count},
        "after": {"paused": answer != "approve", "consecutive_failures": if answer == "approve" { 0 } else { job.consecutive_failures },
                  "retry_count": if answer == "approve" { 0 } else { job.retry_count }},
    });
    if answer == "approve" && episode.effect.is_none() {
        RESUME_AUTHORITY
            .scope(
                ResumeAuthority {
                    episode: episode_id.to_string(),
                    audit: effect.clone(),
                },
                scheduler.unpause_schedule(job_id),
            )
            .await?;
    }
    let effect = if answer == "approve" {
        let applied = scheduler
            .list_scheduled_jobs()
            .await
            .into_iter()
            .find(|job| job.id == job_id)
            .ok_or_else(|| SchedulerError::JobNotFound(job_id.to_string()))?;
        applied
            .pause_episode
            .as_ref()
            .filter(|episode| episode.id == episode_id)
            .and_then(|episode| episode.effect.clone())
            .ok_or_else(|| {
                SchedulerError::PersistError("resume effect was not durably recorded".to_string())
            })?
    } else {
        effect
    };
    crate::decisions::resolve_schedule_pause(pool, job_id, episode_id, &effect)
        .await
        .map_err(SchedulerError::PersistError)
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
    active_claims: Arc<Mutex<HashMap<String, FireClaim>>>,
    session_manager: Arc<SessionManager>,
    brain: Arc<tokio::sync::RwLock<Option<crate::brain_handle::SafeBrain>>>,
    persona: Arc<tokio::sync::RwLock<Option<crate::config::agent_identity::SharedPersona>>>,
    agent_config:
        Arc<tokio::sync::RwLock<Option<crate::config::agent_identity::SharedAgentConfig>>>,
}

/// Storage load behavior at constructor time.
#[derive(Copy, Clone, Eq, PartialEq)]
enum StorageLoadMode {
    /// Full daemon path: reconcile stale persisted state and spawn catch-up runs.
    ReconcileAndCatchUp,
    /// Inspection path: load the file into memory without persisting any reconciliation.
    NoReconcile,
}

impl Scheduler {
    async fn build(
        storage_path: PathBuf,
        session_manager: Arc<SessionManager>,
        load_mode: StorageLoadMode,
        start_cron: bool,
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
            active_claims: Arc::new(Mutex::new(HashMap::new())),
            session_manager,
            brain: Arc::new(tokio::sync::RwLock::new(None)),
            persona: Arc::new(tokio::sync::RwLock::new(None)),
            agent_config: Arc::new(tokio::sync::RwLock::new(None)),
        });

        arc_self
            .sweep_claims_at_startup(load_mode, Utc::now(), owner_is_alive)
            .await;
        arc_self.load_jobs_from_storage(load_mode).await;
        if let Err(error) = arc_self.reconcile_pause_decisions().await {
            tracing::warn!(%error, "pause delivery remains pending at startup");
        }
        if start_cron {
            arc_self
                .tokio_scheduler
                .start()
                .await
                .map_err(|e| SchedulerError::SchedulerInternalError(e.to_string()))?;
        }

        Ok(arc_self)
    }

    async fn sweep_claims_at_startup(
        &self,
        mode: StorageLoadMode,
        now: DateTime<Utc>,
        alive: fn(i64) -> bool,
    ) {
        if mode == StorageLoadMode::NoReconcile {
            return;
        }
        let result: anyhow::Result<()> = async {
            let pool = self.session_manager.pool_clone().await?;
            let rows = sqlx::query_as::<_, (String, String, i64, String)>(
                "SELECT job_id, fire_key, owner_pid, owner_token FROM schedule_fire_claims
                 WHERE finished_at IS NULL AND started_at IS NOT NULL",
            )
            .fetch_all(&pool)
            .await?;
            let owner = claim_owner();
            // A started run cannot safely resume; retire it to unblock run-now.
            // Leave unstarted dead-owner claims available for guarded takeover.
            for (job_id, fire_key, pid, token) in rows {
                if (pid == owner.pid && token != owner.token) || !alive(pid) {
                    let result = sqlx::query(
                        "UPDATE schedule_fire_claims SET finished_at = ?
                         WHERE job_id = ? AND fire_key = ? AND owner_token = ?
                         AND owner_pid = ? AND finished_at IS NULL AND started_at IS NOT NULL",
                    )
                    .bind(now.to_rfc3339())
                    .bind(&job_id)
                    .bind(&fire_key)
                    .bind(token)
                    .bind(pid)
                    .execute(&pool)
                    .await?;
                    if result.rows_affected() == 1 {
                        tracing::warn!(job = %job_id, fire_key = %fire_key,
                            "Startup finished claim belonging to a dead owner or our recycled PID");
                    }
                }
            }
            Ok(())
        }
        .await;
        if let Err(error) = result {
            tracing::warn!(%error, "Claim startup sweep failed; continuing startup");
        }
    }

    pub async fn new(
        storage_path: PathBuf,
        session_manager: Arc<SessionManager>,
    ) -> Result<Arc<Self>, SchedulerError> {
        Self::build(
            storage_path,
            session_manager,
            StorageLoadMode::ReconcileAndCatchUp,
            true,
        )
        .await
    }

    pub async fn new_without_cron(
        storage_path: PathBuf,
        session_manager: Arc<SessionManager>,
    ) -> Result<Arc<Self>, SchedulerError> {
        Self::build(
            storage_path,
            session_manager,
            StorageLoadMode::NoReconcile,
            false,
        )
        .await
    }

    /// The production callback, shared by every armed kind. Passing its arrival
    /// instant explicitly lets tests exercise admission without a live cron tick.
    fn task_for_job(
        &self,
        job: ScheduledJob,
    ) -> impl Fn(DateTime<Utc>) -> ScheduledTaskFuture + Send + Sync + 'static {
        let job_for_task = job.clone();
        let jobs_arc = self.jobs.clone();
        let storage_path = self.storage_path.clone();
        let running_tasks_arc = self.running_tasks.clone();
        let active_claims_arc = self.active_claims.clone();
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
        move |current_time: DateTime<Utc>| -> ScheduledTaskFuture {
            tracing::info!("Scheduled task triggered for job '{}'", job_for_task.id);
            let task_job_id = job_for_task.id.clone();
            let current_jobs_arc = jobs_arc.clone();
            let local_storage_path = storage_path.clone();
            let job_to_execute = job_for_task.clone();
            let running_tasks = running_tasks_arc.clone();
            let active_claims = active_claims_arc.clone();
            let brain_for_task = brain_arc.clone();
            let persona_for_task = persona_arc.clone();
            let agent_config_for_task = agent_config_arc.clone();
            let session_manager_for_task = session_manager_arc.clone();

            Box::pin(async move {
                let should_execute = {
                    let mut jobs_guard = current_jobs_arc.lock().await;
                    refresh_schedule_state(&local_storage_path, &mut jobs_guard);
                    jobs_guard
                        .get(&task_job_id)
                        .map(|(_, j)| !j.paused)
                        .unwrap_or(false)
                };

                if !should_execute {
                    return;
                }

                let (key, kind) = key_for_tick(&job_to_execute, current_time);
                let Some(fire_claim) =
                    admit_fire(&session_manager_for_task, &task_job_id, key, kind).await
                else {
                    return;
                };
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
                        Some((_, job)) if !job.currently_running && !job.paused => {
                            if let Some(claim) = &fire_claim {
                                active_claims
                                    .lock()
                                    .await
                                    .insert(task_job_id.clone(), claim.clone());
                            }
                            let prior = job.last_status;
                            job.last_run = Some(current_time);
                            // A fire at the job's own scheduled instant clears
                            // the catch-up mark: the record must never keep
                            // calling an on-time run a recovery.
                            job.last_run_was_catch_up = false;
                            job.currently_running = true;
                            job.process_start_time = Some(current_time);
                            Some(prior)
                        }
                        // already running, or removed between the checks → don't claim
                        _ => None,
                    }
                };
                let Some(prior_status) = prior_status else {
                    if let Some(claim) = &fire_claim {
                        claim.finish().await;
                    }
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

                // A cron fire is the ONE schedule change with no HTTP route to
                // emit from, and it is exactly what the Automate tab's run
                // history is showing. Without this the tab would wait out its
                // 60s backstop poll for the most interesting event it has.
                crate::events::emit(crate::events::schedule_changed(&task_job_id, "run_started"));

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
                let mut execution_started = false;
                let final_result = loop {
                    let brain_snapshot = brain_for_task.read().await.clone();
                    let persona_snapshot = persona_for_task.read().await.clone();
                    let ac_snapshot = agent_config_for_task.read().await.clone();
                    // Isolate a job panic (durability F3): a panic inside
                    // execute_job is converted to an Err so cleanup still runs and
                    // other jobs keep firing.
                    let attempt_result = std::panic::AssertUnwindSafe(async {
                        if !execution_started {
                            if let Some(claim) = &fire_claim {
                                claim.start().await?;
                            }
                            execution_started = true;
                        }
                        execute_job(
                            job_to_execute.clone(),
                            (current_jobs_arc.clone(), session_manager_for_task.clone()),
                            task_job_id.clone(),
                            cancel_token.clone(),
                            brain_snapshot,
                            persona_snapshot,
                            ac_snapshot,
                        )
                        .await
                    })
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
                            // A typed `RuntimeOutcome::Failed` is never retried. It
                            // is one bit: a spend refusal before the wire and an
                            // accounting failure after every tool ran set it
                            // identically, and this loop re-executes the WHOLE job —
                            // so "don't know" is read as "don't repeat". It still
                            // records Error and counts toward the auto-pause breaker
                            // below. Splitting the outcome belongs in agent.rs. A
                            // real stream `Err` retries exactly as before.
                            let retryable = is_retryable_failure(&e);
                            if attempt < max_retries && !cancel_token.is_cancelled() && retryable {
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

                crate::events::emit(crate::events::schedule_changed(
                    &task_job_id,
                    "run_finished",
                ));

                // Record terminal outcome onto the job's reliability fields.
                let (max_retries, retry_spent) = {
                    let mut jobs_guard = current_jobs_arc.lock().await;
                    if let Some((_, job)) = jobs_guard.get_mut(&task_job_id) {
                        record_failed_fire(job, &final_result);
                        job.currently_running = false;
                        job.current_session_id = None;
                        job.process_start_time = None;
                        job.run_count = job.run_count.saturating_add(1);
                        match &final_result {
                            Ok(_) => {
                                job.last_status = Some(ScheduleRunStatus::Ok);
                                job.last_success = job.last_run.or_else(|| Some(Utc::now()));
                                job.last_error = None;
                                job.retry_count = 0;
                                job.consecutive_failures = 0;
                            }
                            Err(e) => {
                                job.last_status = Some(ScheduleRunStatus::Error);
                                job.last_error = Some(e.to_string());
                                job.consecutive_failures =
                                    job.consecutive_failures.saturating_add(1);
                                // Circuit breaker: a job that fails every fire
                                // burns provider spend for nothing (5,110
                                // credit-error retries in 2.5h on 2026-08-05).
                                // Pause it; the user resumes from Automate.
                                if job.consecutive_failures >= AUTO_PAUSE_AFTER_CONSECUTIVE_FAILURES
                                    && !job.paused
                                {
                                    job.paused = true;
                                    begin_pause_episode(job);
                                    tracing::error!(
                                        target: "durability",
                                        job = %task_job_id,
                                        "scheduled job auto-paused after {} consecutive failures",
                                        job.consecutive_failures
                                    );
                                }
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

                if let Err(error) =
                    stage_pause_jobs(&session_manager_for_task, &current_jobs_arc).await
                {
                    tracing::error!(%error, "failed to stage pause intent; keeping in-memory pause");
                }
                match persist_jobs(&local_storage_path, &current_jobs_arc).await {
                    Ok(()) => {
                        if let Err(error) =
                            reconcile_pause_jobs(&session_manager_for_task, &local_storage_path)
                                .await
                        {
                            tracing::warn!(%error, "pause delivery remains pending");
                        }
                    }
                    Err(error) => {
                        tracing::error!(%error, "Failed to persist job completion; pause delivery remains pending")
                    }
                }

                // Retire the claim after recording the terminal outcome and
                // pause intent; a finished claim must not precede its evidence.
                if let Some(claim) = &fire_claim {
                    claim.finish().await;
                }
                if let Some(claim) = &fire_claim {
                    let mut active = active_claims.lock().await;
                    if active
                        .get(&task_job_id)
                        .is_some_and(|current| current.fire_key == claim.fire_key)
                    {
                        active.remove(&task_job_id);
                    }
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
        }
    }

    /// Build a `tokio_cron_scheduler` job, dispatching on its kind:
    /// `at` → one-shot, `every_seconds` → interval, else cron (tz-aware).
    fn create_job_task(&self, job: ScheduledJob) -> Result<Job, SchedulerError> {
        let task = self.task_for_job(job.clone());
        let task_closure = move |_uuid: uuid::Uuid, _l: TokioJobScheduler| task(Utc::now());

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

        let job_id = stored_job.id.clone();
        {
            // Insert AND persist under a single lock hold so the new job reaches
            // disk before the lock is released. Otherwise sync_from_storage could
            // acquire the lock in the gap, see the job in memory but not on disk,
            // and delete it (issue #924).
            let mut jobs_guard = self.jobs.lock().await;
            jobs_guard.insert(stored_job.id.clone(), (job_uuid, stored_job));
            let list: Vec<ScheduledJob> = jobs_guard.values().map(|(_, j)| j.clone()).collect();
            write_jobs_to_disk(&self.storage_path, &list)?;
        }

        // #1090 pattern: the announcement belongs to the writer, not the caller.
        // Every entry point — the /schedule routes, platform__manage_schedule,
        // platform__recipes' create_recipe, `schedule_recipe` — lands here, so
        // an agent-created automation now shows up on the Automate tab exactly
        // like a hand-created one instead of waiting out the 60s backstop poll.
        crate::events::emit(crate::events::schedule_changed(&job_id, "created"));
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

    async fn load_jobs_from_storage(self: &Arc<Self>, mode: StorageLoadMode) {
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
        // Long-cadence jobs whose window was slept through, to be run once
        // after the job map is populated (see `should_catch_up`).
        let mut catch_up_jobs: Vec<ScheduledJob> = Vec::new();

        for mut job_to_load in list {
            let original_run_count = job_to_load.run_count;
            if let Err(error) = recover_pause_job(&self.session_manager, &mut job_to_load).await {
                // Fail closed if the durable pause ledger cannot be checked.
                job_to_load.paused = true;
                tracing::warn!(%error, "pause recovery unavailable; keeping schedule paused");
            }
            reconciled |= job_to_load.run_count != original_run_count;
            // Interval floor applies to persisted jobs too: add-time validation
            // only covers jobs created after the guard shipped, and schedule.json
            // is hand-editable. Violators are paused (not dropped) so the user
            // sees them in Automate with the reason, instead of them firing.
            if mode == StorageLoadMode::ReconcileAndCatchUp && !job_to_load.paused {
                if let Some(violation) = schedule_floor_violation(&job_to_load) {
                    job_to_load.paused = true;
                    job_to_load.last_error = Some(format!(
                        "paused at load: {} — edit the schedule, then resume",
                        violation
                    ));
                    reconciled = true;
                    tracing::warn!(
                        "Scheduled job '{}' paused at load: {}",
                        job_to_load.id,
                        violation
                    );
                }
            }
            if job_to_load.pause_episode.is_none() && !Path::new(&job_to_load.source).exists() {
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
            if mode == StorageLoadMode::ReconcileAndCatchUp
                && job_to_load.at.is_some()
                && job_to_load.last_run.is_some()
            {
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
            if mode == StorageLoadMode::ReconcileAndCatchUp && job_to_load.currently_running {
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

            // Normalized in memory in BOTH modes. `create_job_task` converts a
            // 5-field cron on its own, so this is not what keeps a legacy job
            // loadable; it decides the canonical form held in memory, which an
            // inspection-mode `add`/`remove` later persists for every job. Keeping
            // it mode-independent means the CLI writes the same form the daemon
            // would. Only the write-back on load is gated on the mode (below).
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
            // (amber). Detection stays the default for cron/interval jobs — a
            // catch-up for every missed sub-daily tick is a wake-storm, and it
            // would run before brain/persona are wired. One-shot jobs
            // self-catch-up: they re-arm below with a zero delay and fire once
            // immediately, then delete themselves.
            //
            // The one exception (D20) is a cron whose period is longer than a
            // day. Detection alone is the wrong tool there: applying a
            // "late for 2× its period" rule to a weekly job means a 14-day
            // time-to-notice, which is absurd, and marking it amber does not
            // make the run happen. A weekly job's window is a single 60-second
            // instant per week; if the daemon was not up for it, running the
            // job late is still worth exactly as much as running it on time.
            // Collapse the problem instead of monitoring it — bounded by
            // `should_catch_up`, once, and marked as a catch-up in the record.
            if mode == StorageLoadMode::ReconcileAndCatchUp {
                let now = Utc::now();
                if is_run_missed(&job_to_load, now) {
                    let catch_up = job_to_load.at.is_none()
                        && job_to_load.every_seconds.is_none()
                        && missed_fire_at(&job_to_load)
                            .zip(cron_period(&job_to_load.cron, now))
                            .is_some_and(|(due, period)| should_catch_up(period, now - due));
                    if catch_up {
                        tracing::warn!(
                            target: "durability",
                            "Job '{}' missed its window during downtime; running it once as a \
                             catch-up (its cadence is longer than a day, so the next natural \
                             tick is too far away to wait for)",
                            job_to_load.id
                        );
                        catch_up_jobs.push(job_to_load.clone());
                    } else {
                        tracing::warn!(
                            target: "durability",
                            "Job '{}' missed a scheduled run during downtime; marking Missed",
                            job_to_load.id
                        );
                        job_to_load.last_status = Some(ScheduleRunStatus::Missed);
                        reconciled = true;
                    }
                    // Catch-up changes no status until its durable claim wins.
                }
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
        if mode == StorageLoadMode::ReconcileAndCatchUp && (crons_normalized || reconciled) {
            if let Err(e) = persist_jobs(&self.storage_path, &self.jobs).await {
                tracing::error!("Failed to persist scheduler reconciliation; pause delivery remains pending: {}", e);
            }
        }

        if mode == StorageLoadMode::ReconcileAndCatchUp && !catch_up_jobs.is_empty() {
            self.clone().spawn_catch_up_runs(catch_up_jobs);
        }
    }

    /// Admit the missed occurrence captured at load time, before the stagger
    /// and before touching any of the job's run-state fields.
    async fn claim_catch_up(&self, missed_job: &ScheduledJob) -> Option<Option<FireClaim>> {
        {
            let jobs = self.jobs.lock().await;
            match jobs.get(&missed_job.id) {
                Some((_, job)) if !job.paused && !job.currently_running => {}
                _ => return None,
            }
        }
        let key = key_for_catch_up(missed_job)?;
        admit_fire(&self.session_manager, &missed_job.id, key, "catch_up").await
    }

    /// Run slept-through long-cadence jobs staggered, off the load path, after
    /// the job map is populated. A continuation carries its original claim.
    fn spawn_catch_up_runs(self: Arc<Self>, missed_jobs: Vec<ScheduledJob>) {
        tokio::spawn(async move {
            // Keep the occurrence identified at load time. Re-reading last_run
            // after a stagger could accidentally name the NEXT occurrence.
            for job in missed_jobs {
                let id = job.id.clone();
                let Some(claim) = self.claim_catch_up(&job).await else {
                    continue;
                };
                tokio::time::sleep(std::time::Duration::from_secs(CATCH_UP_STAGGER_SECS)).await;
                match self.run_claimed(&id, claim, true).await {
                    Ok(session) => tracing::info!(
                        target: "durability",
                        "Catch-up run started for '{}' (session {})",
                        id,
                        session
                    ),
                    Err(e) => tracing::warn!(
                        target: "durability",
                        "Catch-up run for '{}' did not start: {}",
                        id,
                        e
                    ),
                }
            }
        });
    }

    async fn sync_from_storage(&self) {
        if !self.storage_path.exists() {
            return;
        }
        // Read + parse + diff the disk file UNDER the jobs lock so a concurrent
        // add_scheduled_job (which inserts + persists under the same lock) is
        // never observed half-done — otherwise a just-added job (in memory, not
        // yet flushed to disk) would be diffed as "removed on disk" and deleted
        // (issue #924). No await runs inside the locked block.
        let (jobs_to_add, jobs_to_remove): (Vec<ScheduledJob>, Vec<(String, JobId)>) = {
            let jobs_guard = self.jobs.lock().await;
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

        for mut job in jobs_to_add {
            if let Err(error) = recover_pause_job(&self.session_manager, &mut job).await {
                job.paused = true;
                tracing::warn!(%error, "pause recovery unavailable; keeping schedule paused");
            }
            // Same floor treatment as the startup loader, for the same reason:
            // schedule.json is hand-editable and this path arms whatever it finds
            // there — on every list_scheduled_jobs, not just at boot. Pausing in
            // memory is enough to stop the fire (create_job_task re-reads `paused`
            // from this map at fire time); the loader persists it at next boot.
            if !job.paused {
                if let Some(violation) = schedule_floor_violation(&job) {
                    job.paused = true;
                    job.last_error = Some(format!(
                        "paused at load: {} — edit the schedule, then resume",
                        violation
                    ));
                    tracing::warn!("Scheduled job '{}' paused at load: {}", job.id, violation);
                }
            }
            if job.pause_episode.is_none() && !Path::new(&job.source).exists() {
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

    pub async fn reconcile_pause_decisions(&self) -> Result<(), SchedulerError> {
        reconcile_pause_jobs(&self.session_manager, &self.storage_path).await
    }

    pub async fn list_scheduled_jobs(&self) -> Vec<ScheduledJob> {
        self.sync_from_storage().await;
        refresh_schedule_state(&self.storage_path, &mut *self.jobs.lock().await);
        if let Err(error) = self.reconcile_pause_decisions().await {
            tracing::warn!(%error, "pause delivery remains pending");
        }
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
        crate::events::emit(crate::events::schedule_changed(id, "deleted"));
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

    /// Recent runs for every schedule that has one, batched into a single SQL
    /// query — the Automate tab's replacement for calling [`Self::sessions`]
    /// once per job on every poll tick. See
    /// `SessionManager::list_recent_sessions_by_schedule`.
    pub async fn recent_sessions_by_schedule(
        &self,
        limit_per_schedule: usize,
    ) -> Result<Vec<ScheduleSessionSummary>, SchedulerError> {
        self.session_manager
            .list_recent_sessions_by_schedule(limit_per_schedule)
            .await
            .map_err(|e| SchedulerError::StorageError(io::Error::other(e)))
    }

    pub async fn run_now(&self, sched_id: &str) -> Result<String, SchedulerError> {
        self.run_now_with_liveness(sched_id, Utc::now(), owner_is_alive)
            .await
    }

    async fn run_now_with_liveness(
        &self,
        sched_id: &str,
        now: DateTime<Utc>,
        alive: fn(i64) -> bool,
    ) -> Result<String, SchedulerError> {
        let key = format!("manual:{}", uuid::Uuid::new_v4());
        let claim = match self.session_manager.pool_clone().await {
            Ok(pool) => {
                match try_claim_manual(&pool, sched_id, &key, claim_owner(), now, alive).await {
                    ClaimOutcome::Won => Some(FireClaim {
                        pool,
                        job_id: sched_id.to_string(),
                        fire_key: key,
                        owner: claim_owner().clone(),
                    }),
                    ClaimOutcome::Lost => {
                        tracing::info!(job = %sched_id, "Manual run refused: another process is alive");
                        return Err(SchedulerError::AnyhowError(anyhow!(
                            "Job '{}' is already running",
                            sched_id
                        )));
                    }
                    ClaimOutcome::StoreUnavailable(error) => {
                        tracing::warn!(job = %sched_id, %error, "Manual claim store unavailable; running fail-awake");
                        None
                    }
                }
            }
            Err(error) => {
                tracing::warn!(job = %sched_id, %error, "Manual claim store unavailable; running fail-awake");
                None
            }
        };
        self.run_claimed(sched_id, claim, false).await
    }

    async fn run_claimed(
        &self,
        sched_id: &str,
        claim: Option<FireClaim>,
        catch_up: bool,
    ) -> Result<String, SchedulerError> {
        let result = self.run_now_inner(sched_id, claim.as_ref(), catch_up).await;
        if let Some(claim) = &claim {
            claim.finish().await;
            // An early refusal must not remove a different run's claim.
            let mut active = self.active_claims.lock().await;
            if active
                .get(sched_id)
                .is_some_and(|current| current.fire_key == claim.fire_key)
            {
                active.remove(sched_id);
            }
        }
        result
    }

    async fn run_now_inner(
        &self,
        sched_id: &str,
        claim: Option<&FireClaim>,
        catch_up: bool,
    ) -> Result<String, SchedulerError> {
        let job_to_run = {
            let mut jobs_guard = self.jobs.lock().await;
            refresh_schedule_state(&self.storage_path, &mut jobs_guard);
            match jobs_guard.get_mut(sched_id) {
                Some((_, job)) => {
                    // Approval gate backstop: run_now used to fire an
                    // approval-pending job directly, bypassing the paused
                    // state entirely — an agent could create_recipe (lands
                    // paused) then run_recipe it on demand. Enforced HERE so
                    // every caller (HTTP route, recipe_author, schedule_tool)
                    // inherits it; the user approves by unpausing, which
                    // clears the flag.
                    if job.requires_approval {
                        return Err(SchedulerError::AnyhowError(anyhow!(
                            "Job '{}' is awaiting the user's approval and cannot be run \
                             until the user resumes it from Automate",
                            sched_id
                        )));
                    }
                    if job.currently_running || (catch_up && job.paused) {
                        return Err(SchedulerError::AnyhowError(anyhow!(
                            "Job '{}' is already running",
                            sched_id
                        )));
                    }
                    if let Some(claim) = claim {
                        self.active_claims
                            .lock()
                            .await
                            .insert(sched_id.to_string(), claim.clone());
                    }
                    job.last_run_was_catch_up = catch_up;
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
        // Announced here, not at the HTTP route, so an agent's run_recipe /
        // manage_schedule(run_now) moves the Automate tab's run history the same
        // way the button does. Mirrors the cron path's pair in `create_job_task`.
        crate::events::emit(crate::events::schedule_changed(sched_id, "run_started"));

        let job_start_instant = std::time::Instant::now();
        let brain_snapshot = self.brain.read().await.clone();
        let persona_snapshot = self.persona.read().await.clone();
        let ac_snapshot = self.agent_config.read().await.clone();
        // Contain a panic exactly like the cron path does: an unwinding
        // execute_job used to skip everything below — the token stayed in
        // running_tasks and `currently_running` stayed true, wedging the
        // schedule as "running" with a Stop button that cancelled a token
        // nobody was watching.
        let result = std::panic::AssertUnwindSafe(async {
            if let Some(claim) = claim {
                claim.start().await?;
            }
            execute_job(
                job_to_run,
                (self.jobs.clone(), self.session_manager.clone()),
                sched_id.to_string(),
                cancel_token.clone(),
                brain_snapshot,
                persona_snapshot,
                ac_snapshot,
            )
            .await
        })
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
                job = %sched_id,
                "run_now job panicked (contained, cleanup still runs): {msg}"
            );
            Err(anyhow!("job '{}' panicked: {}", sched_id, msg))
        });

        {
            let mut tasks = self.running_tasks.lock().await;
            tasks.remove(sched_id);
        }

        crate::events::emit(crate::events::schedule_changed(sched_id, "run_finished"));

        let duration_ms = job_start_instant.elapsed().as_millis() as u64;

        {
            let mut jobs_guard = self.jobs.lock().await;
            if let Some((_, job)) = jobs_guard.get_mut(sched_id) {
                record_failed_fire(job, &result);
                job.currently_running = false;
                job.current_session_id = None;
                job.process_start_time = None;
                job.last_run = Some(Utc::now());
                // Manual runs record their outcome like cron fires do — a
                // failed Run Now used to leave last_status untouched, so the
                // Automate view showed stale health for exactly the runs the
                // user was watching most closely.
                // A manual or catch-up run is a run: count it the way the cron
                // path does. It never was before, so Automate's run count
                // undercounted every job that had ever been run by hand.
                job.run_count = job.run_count.saturating_add(1);
                match &result {
                    Ok(_) => {
                        job.last_status = Some(ScheduleRunStatus::Ok);
                        job.last_success = job.last_run;
                        job.last_error = None;
                        job.retry_count = 0;
                        job.consecutive_failures = 0;
                    }
                    Err(e) => {
                        job.last_status = Some(ScheduleRunStatus::Error);
                        job.last_error = Some(e.to_string());
                        job.consecutive_failures = job.consecutive_failures.saturating_add(1);
                        // The breaker applies here too: without it, repeated
                        // run_now calls could drive a failing job past the
                        // 3-strike limit indefinitely (agent-reachable via
                        // run_recipe).
                        if job.consecutive_failures >= AUTO_PAUSE_AFTER_CONSECUTIVE_FAILURES
                            && !job.paused
                        {
                            job.paused = true;
                            begin_pause_episode(job);
                            tracing::error!(
                                target: "durability",
                                job = %sched_id,
                                "scheduled job auto-paused after {} consecutive failures (run_now)",
                                job.consecutive_failures
                            );
                        }
                    }
                }
            }
        }

        if let Err(error) = stage_pause_jobs(&self.session_manager, &self.jobs).await {
            tracing::error!(%error, "failed to stage pause intent; keeping in-memory pause");
        }
        if let Err(error) = persist_jobs(&self.storage_path, &self.jobs).await {
            tracing::error!(%error, "Failed to persist job completion; pause delivery remains pending");
            return Err(error);
        }
        if let Err(error) = self.reconcile_pause_decisions().await {
            tracing::warn!(%error, "pause delivery remains pending");
        }

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
                    // A later manual pause is not the old, already-resumed
                    // breaker episode. Its evidence remains in the episode table.
                    if job
                        .pause_episode
                        .as_ref()
                        .is_some_and(|episode| episode.effect.is_some())
                    {
                        job.pause_episode = None;
                    }
                }
                None => return Err(SchedulerError::JobNotFound(sched_id.to_string())),
            }
        }

        persist_jobs(&self.storage_path, &self.jobs).await?;
        crate::events::emit(crate::events::schedule_changed(sched_id, "paused"));
        Ok(())
    }

    pub async fn unpause_schedule(&self, sched_id: &str) -> Result<(), SchedulerError> {
        self.sync_from_storage().await;
        let mut jobs = self.jobs.lock().await;
        let (_, current) = jobs
            .get(sched_id)
            .ok_or_else(|| SchedulerError::JobNotFound(sched_id.to_string()))?;
        let mut candidate = current.clone();
        let _file_lock = lock_schedule_file(&self.storage_path)?;
        let disk_jobs: Vec<ScheduledJob> =
            serde_json::from_str(&fs::read_to_string(&self.storage_path)?)?;
        let disk_job = disk_jobs
            .iter()
            .find(|job| job.id == sched_id)
            .ok_or_else(|| SchedulerError::JobNotFound(sched_id.to_string()))?;
        // The file lock also covers the comparison: a stale inbox tap cannot
        // overwrite a newer episode persisted by another scheduler process.
        if disk_job.run_count > candidate.run_count || disk_job.resumed_at > candidate.resumed_at {
            candidate = disk_job.clone();
        }
        // Resume is the one path that can un-do a floor pause, so the floor
        // must be checked here or the guard would be merely advisory. The
        // refusal is recorded on the live job as before, so Automate shows why.
        if let Some(violation) = schedule_floor_violation(&candidate) {
            if let Some((_, live)) = jobs.get_mut(sched_id) {
                live.last_error = Some(violation.clone());
            }
            return Err(SchedulerError::InvalidScheduleSpec(violation));
        }
        if candidate.currently_running {
            return Err(SchedulerError::AnyhowError(anyhow!(
                "cannot resume a running job"
            )));
        }
        if candidate.at.is_some_and(|at| at <= Utc::now()) {
            return Err(SchedulerError::InvalidScheduleSpec(
                "one-time schedule needs a future occurrence".to_string(),
            ));
        }
        if let Some(episode) = &mut candidate.pause_episode {
            let authority = RESUME_AUTHORITY.try_with(Clone::clone).map_err(|_| {
                SchedulerError::AnyhowError(anyhow!(
                    "auto-paused jobs require an authenticated episode answer"
                ))
            })?;
            if episode.id != authority.episode {
                return Err(SchedulerError::AnyhowError(anyhow!("stale pause episode")));
            }
            if episode.effect.is_some() {
                return Ok(());
            }
            let mut audit = authority.audit;
            audit["applied_at"] = serde_json::json!(Utc::now());
            episode.effect = Some(audit);
        }
        candidate.paused = false;
        // Unpausing IS approval: agent tools refuse approval-pending jobs;
        // the authenticated Automate path is the user's approval surface.
        candidate.requires_approval = false;
        candidate.consecutive_failures = 0;
        candidate.retry_count = 0;
        candidate.resumed_at = Some(Utc::now());
        // Write the candidate before publishing it to fired tasks. A failed
        // write leaves the live job paused and the inbox request unresolved.
        let list: Vec<_> = disk_jobs
            .iter()
            .map(|job| {
                if job.id == sched_id {
                    candidate.clone()
                } else {
                    job.clone()
                }
            })
            .collect();
        write_locked_jobs_to_disk(&self.storage_path, &list)?;
        if let Some((_, job)) = jobs.get_mut(sched_id) {
            *job = candidate;
        }
        drop(jobs);
        crate::events::emit(crate::events::schedule_changed(sched_id, "unpaused"));
        Ok(())
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
                    // Validate a candidate, not the stored job: a rejected cron
                    // must leave the job exactly as it was, still armed on its
                    // legal schedule.
                    let mut candidate = job.clone();
                    candidate.cron = new_cron;
                    validate_schedule_spec(&candidate)
                        .map_err(SchedulerError::InvalidScheduleSpec)?;
                    *job = candidate.clone();
                    (*uuid, candidate)
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

        persist_jobs(&self.storage_path, &self.jobs).await?;
        crate::events::emit(crate::events::schedule_changed(sched_id, "updated"));
        Ok(())
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

        persist_jobs(&self.storage_path, &self.jobs).await?;
        crate::events::emit(crate::events::schedule_changed(sched_id, "updated"));
        Ok(())
    }

    /// How long a cancelled run gets to observe its token and clean up before
    /// Stop force-clears the schedule state. Short in tests so the watchdog is
    /// testable without a 15-second sleep.
    #[cfg(not(test))]
    const KILL_FORCE_CLEAR_GRACE: std::time::Duration = std::time::Duration::from_secs(15);
    #[cfg(test)]
    const KILL_FORCE_CLEAR_GRACE: std::time::Duration = std::time::Duration::from_millis(200);

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

        let stopping_claim = self.active_claims.lock().await.get(sched_id).cloned();
        let cancelled_live_token = {
            let tasks = self.running_tasks.lock().await;
            match tasks.get(sched_id) {
                Some(token) => {
                    token.cancel();
                    true
                }
                None => false,
            }
        };

        // `currently_running` with no cancel token means the run is DEAD — it
        // died without reaching its cleanup (e.g. a process restart mid-run
        // before boot reconciliation existed). Returning Ok while clearing
        // nothing left the schedule wedged as "running" forever, with Stop and
        // Delete both bouncing off it. Clear the run state here so Stop always
        // means stopped; a live run's own teardown path is untouched.
        if !cancelled_live_token {
            tracing::warn!(
                "Stop requested for schedule '{}' but no live run exists — clearing wedged \
                 running state",
                sched_id
            );
            {
                let mut jobs_guard = self.jobs.lock().await;
                if let Some((_, job)) = jobs_guard.get_mut(sched_id) {
                    job.currently_running = false;
                    job.current_session_id = None;
                    job.process_start_time = None;
                    job.last_status = Some(ScheduleRunStatus::Error);
                    job.last_error = Some("run died without cleanup; cleared by Stop".to_string());
                }
            }
            if let Some(claim) = &stopping_claim {
                claim.finish().await;
            }
            persist_jobs(&self.storage_path, &self.jobs).await?;
        } else {
            // A cancelled run normally observes its token between steps and
            // cleans up within moments. A run parked inside an await that
            // never checks the token — a hung provider call, observed live
            // 2026-08-06 after 40 stalled minutes — never does, and Stop
            // reported success while the schedule stayed "running" forever.
            // Watchdog: after a grace period, force-clear so Stop always
            // converges to stopped. If the zombie ever wakes, its own teardown
            // re-clears idempotently (the small overwrite window for a
            // freshly-started next run is the same one slow teardowns always
            // had — see the claim guard in create_job_task).
            let jobs = self.jobs.clone();
            let tasks = self.running_tasks.clone();
            let storage_path = self.storage_path.clone();
            let id = sched_id.to_string();
            // Identity of the run being stopped. Without it the watchdog clears
            // whatever is running when it fires — including a DIFFERENT, healthy
            // run started during the grace window (a cron fire, or the user
            // pressing Run now). Killing that one and recording "ignored
            // cancellation" would be a lie about the wrong run.
            let stopping = {
                let g = self.jobs.lock().await;
                g.get(sched_id).and_then(|(_, j)| j.process_start_time)
            };
            tokio::spawn(async move {
                tokio::time::sleep(Self::KILL_FORCE_CLEAR_GRACE).await;
                let still_running = {
                    let g = jobs.lock().await;
                    match g.get(&id) {
                        // Ours to clear only if the SAME run is still in flight.
                        Some((_, j)) => j.currently_running && j.process_start_time == stopping,
                        None => false,
                    }
                };
                if !still_running {
                    return;
                }
                tracing::warn!(
                    "schedule '{}' did not observe cancellation within {:?} — force-clearing \
                     the wedged run",
                    id,
                    Self::KILL_FORCE_CLEAR_GRACE
                );
                tasks.lock().await.remove(&id);
                {
                    let mut g = jobs.lock().await;
                    if let Some((_, job)) = g.get_mut(&id) {
                        job.currently_running = false;
                        job.current_session_id = None;
                        job.process_start_time = None;
                        job.last_status = Some(ScheduleRunStatus::Error);
                        job.last_error =
                            Some("run ignored cancellation; force-cleared by Stop".to_string());
                    }
                }
                if let Some(claim) = &stopping_claim {
                    claim.finish().await;
                }
                let _ = persist_jobs(&storage_path, &jobs).await;
            });
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

/// Resolve the complete, non-interactive parameter set for a scheduled run.
///
/// `recipe_dir` is a built-in template variable rather than a user-declared
/// parameter. Interactive recipe execution injects it while parsing/building
/// (`recipe::build_recipe::apply_values_to_parameters`); the scheduler used to
/// omit it, which made strict rendering fail and sent the entire raw
/// `{{ ... }}` prompt to the model.
fn scheduled_recipe_params(
    recipe: &Recipe,
    recipe_path: &Path,
    job_id: &str,
) -> HashMap<String, String> {
    let mut params = HashMap::new();
    for p in recipe.parameters.as_deref().unwrap_or_default() {
        let value = p.default.clone().unwrap_or_default();
        if p.default.is_none() {
            tracing::warn!(
                "Scheduled job '{}': parameter '{}' has no default; rendering as empty",
                job_id,
                p.key
            );
        }
        params.insert(p.key.clone(), value);
    }
    if let Some(recipe_dir) = recipe_path.parent() {
        params.insert(
            crate::recipe::BUILT_IN_RECIPE_DIR_PARAM.to_string(),
            recipe_dir.to_string_lossy().into_owned(),
        );
    }
    params
}

/// Render a scheduled recipe's prompt with `params`, or fail the run.
///
/// Rendering is strict, so one undefined variable fails the whole template.
/// This used to fall back to the RAW prompt: the job then handed the model
/// literal `{{ ... }}`, burned a model call on a confused reply, and still
/// finished `completed` — a silent degradation nobody notices. Returning the
/// error instead makes the caller record `Error`/`last_error` and escalate.
fn render_scheduled_prompt(
    raw_prompt: &str,
    params: &HashMap<String, String>,
    job_id: &str,
) -> Result<String> {
    if params.is_empty() {
        return Ok(raw_prompt.to_string());
    }
    crate::recipe::template_recipe::render_recipe_content_with_params(raw_prompt, params).map_err(
        |e| {
            tracing::error!(
                target: "permagentd::scheduler",
                job = %job_id,
                "parameter rendering failed ({e}); failing the run rather than sending the unrendered template"
            );
            anyhow!("scheduled job '{job_id}': recipe parameter rendering failed: {e}")
        },
    )
}

/// The agent a scheduled run IS, when it is one at all — `(agent_id, name)`.
///
/// A job earns an identity only by declaring one (`worker_persona`) that names
/// a real worker descriptor. That is the whole mapping, and deliberately so:
/// `workspace-snapshot`, `health-review` and `storage-insights` declare no
/// persona because they are the scheduler's own errands, not a character's, and
/// handing them a borrowed orb would animate an agent that is not working.
/// `git-steward` is the one starter that IS an agent's embodiment today, and
/// the alias map here is the same one the agents surface resolves through, so
/// its runs land on `git_steward` — the exact id `steward_sweep.rs::announce`
/// already uses. Before this, the weekday recipe's multi-minute fleet pass was
/// invisible in the World while the 15-minute native sweep was not (D11).
fn job_announce_identity(job: &ScheduledJob) -> Option<(&'static str, &'static str)> {
    let key = job.worker_persona.as_deref()?;
    let descriptor_id = crate::config::agent_identity::descriptor_id_for_worker_key(key);
    crate::agents::self_knowledge::WORKER_DESCRIPTORS
        .iter()
        .find(|d| d.id == descriptor_id)
        .map(|d| (d.id, d.display_name))
}

/// Native one-tool scheduler entry. Cron and Run now both reach this AFTER
/// acquiring their existing fire claim. No Agent/provider is constructed.
/// Like Phase 5's storage contract, persistence is mandatory before success;
/// this branch deliberately never enters storage's serializer or extraction.
async fn execute_model_review_job(
    job: &ScheduledJob,
    pool: &sqlx::SqlitePool,
    input: crate::model_review::inventory::ScanInput,
    cancel: &CancellationToken,
) -> Result<String> {
    let content = fs::read_to_string(&job.source)?;
    let recipe: serde_json::Value = serde_yaml::from_str(&content)?;
    anyhow::ensure!(recipe.get("native_tool").and_then(|v| v.as_str()) == Some("scan_model_review"),
        "Model Review requires native_tool: scan_model_review; customized recipes are preserved but cannot invoke inference");
    anyhow::ensure!(
        recipe
            .get("produces_model_review")
            .and_then(|v| v.as_bool())
            == Some(true),
        "Model Review requires its typed persistence contract"
    );
    anyhow::ensure!(!cancel.is_cancelled(), "Model Review cancelled");
    let inventory =
        tokio::task::spawn_blocking(move || crate::model_review::inventory::scan(&input)).await?;
    anyhow::ensure!(!cancel.is_cancelled(), "Model Review cancelled");
    let report = crate::model_review::report::build(inventory, Utc::now())?;
    // Phase 5: an error (including a missing ledger) fails the scheduler run.
    crate::model_review::report::persist_report(pool, &report).await?;
    Ok(report.id)
}

#[allow(clippy::too_many_lines)]
async fn execute_job(
    job: ScheduledJob,
    jobs_and_storage: (Arc<Mutex<JobsMap>>, Arc<SessionManager>),
    job_id: String,
    cancel_token: CancellationToken,
    brain: Option<crate::brain_handle::SafeBrain>,
    persona: Option<crate::config::agent_identity::SharedPersona>,
    agent_config: Option<crate::config::agent_identity::SharedAgentConfig>,
) -> Result<String> {
    let (jobs, session_manager) = jobs_and_storage;
    let native_model_review = if job.source.is_empty() {
        false
    } else {
        let content = fs::read_to_string(&job.source)?;
        let recipe: serde_json::Value = serde_yaml::from_str(&content)?;
        recipe.get("native_tool").and_then(|v| v.as_str()) == Some("scan_model_review")
            && recipe
                .get("produces_model_review")
                .and_then(|v| v.as_bool())
                == Some(true)
    };
    if native_model_review {
        let pool = session_manager.pool_clone().await?;
        return execute_model_review_job(
            &job,
            &pool,
            crate::model_review::inventory::ScanInput::local(),
            &cancel_token,
        )
        .await;
    }
    // Wrapped HERE rather than in the cron tick so a manual `run_now` announces
    // too: from the World's side there is no difference between the Steward
    // sweeping because it is Tuesday and sweeping because someone pressed Run.
    let announce = job_announce_identity(&job);
    if let Some((id, name)) = announce {
        crate::events::emit(crate::events::agent_state_changed(id, name, "working"));
    }
    let result = crate::providers::inflight::background(execute_job_inner(
        job,
        jobs,
        job_id,
        cancel_token,
        brain,
        persona,
        agent_config,
    ))
    .await;
    if let Some((id, name)) = announce {
        // A failed run is a real failure, and the Steward HUD already draws it
        // (`SWEEP BLOCKED`). Silence on failure would leave the orb amber until
        // the next fire — a worse lie than the red one is true.
        let state = if result.is_ok() { "available" } else { "error" };
        crate::events::emit(crate::events::agent_state_changed(id, name, state));
    }
    result
}

#[allow(clippy::too_many_lines)]
async fn execute_job_inner(
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
                agent.set_worker_key(Some(worker_key.clone())).await;
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

    // One scheduled RUN is one budget task, and it must be declared here.
    //
    // `starts_new_budget_task` (agent.rs) only begins one for an INTERACTIVE
    // session type, on the reasoning that a child or headless session inherits
    // its parent's identity at creation. A scheduled run has no parent: the
    // scheduler creates it from nothing. So it began no task, carried
    // `task_id: None` into `reserve_bounded_provider_invocation`, and every
    // paid call was refused with "paid provider reservation requires a durable
    // task identity" — which is why EVERY scheduled session between 2026-09-19
    // and 2026-09-21 recorded zero calls and zero cost. The automations were
    // not slow or throttled; they could not spend at all.
    //
    // Per run, not per schedule: the per-task ceiling should bound one night's
    // health review, not creep across every run the schedule ever makes.
    agent
        .config
        .session_manager
        .begin_budget_task(&session.id)
        .await?;

    // A recipe that declares no `extensions:` used to inherit the user's ENTIRE
    // enabled set — including `orchestrator` (dispatch goals, spawn agents) and
    // `recipe_author` (create more schedules via its process-global scheduler
    // handle). That closed the self-replication loop behind the 2026-08-05
    // frenzy: a scheduled session recreated deleted schedules and dispatched
    // goals unattended. Headless sessions now get those only when the recipe
    // declares them explicitly (and agent-created recipes land paused pending
    // user approval, so the declaration is user-visible before the first run).
    const HEADLESS_DENYLIST: &[&str] = &["orchestrator", "recipe_author"];
    let declared = recipe.extensions.is_some();
    let recipe_dir = recipe_path.parent();
    let extensions: Vec<_> =
        resolve_extensions_for_new_session(recipe.extensions.as_deref(), None, recipe_dir)?
            .into_iter()
            .filter(|ext| {
                let key = crate::config::extensions::name_to_key(&ext.name());
                let denied = !declared && HEADLESS_DENYLIST.contains(&key.as_str());
                if denied {
                    tracing::info!(
                        "Scheduled job '{}': withholding inherited extension '{}' \
                     (declare it in the recipe's `extensions:` to grant it)",
                        job_id,
                        key
                    );
                }
                !denied
            })
            .collect();
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

    let raw_prompt = recipe
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

    // A scheduled run has nobody to fill parameters interactively, so declared
    // parameter defaults must be rendered into the prompt here. Without this,
    // `{{ repo_path }}` reached the model verbatim and the git-steward spent
    // eight consecutive mornings asking an empty room which repository to
    // steward — every run "ok", ~22k tokens, zero output.
    let params = scheduled_recipe_params(&recipe, recipe_path, &job.id);
    let prompt_text: String = render_scheduled_prompt(raw_prompt, &params, &job.id)?;
    let prompt_text = prompt_text.as_str();

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

    let mut stream_error: Option<String> = None;
    let mut runtime_outcome: Option<AgentRuntimeOutcome> = None;
    while let Some(message_result) = stream.next().await {
        tokio::task::yield_now().await;

        match message_result {
            Ok(AgentEvent::Message(msg)) => {
                conversation.push(msg);
            }
            Ok(AgentEvent::HistoryReplaced(updated)) => {
                conversation = updated;
            }
            Ok(event) => {
                if let Some(outcome) = runtime_outcome_of(&event) {
                    runtime_outcome = Some(outcome);
                }
            }
            Err(e) => {
                tracing::error!("Error in agent stream: {}", e);
                stream_error = Some(e.to_string());
                break;
            }
        }
    }
    let has_runtime_outcome = runtime_outcome.is_some();
    let had_stream_error = stream_error.is_some();
    // Reported at the SAME terminal point as a stream error (end of this fn),
    // so the post-drain phases run for it exactly as they do for that path.
    let mut agent_turn_failed = false;
    // Do not copy assistant text into failure surfaces: this error fans out to
    // retry and terminal logs, activity payloads, and optional inbox decisions;
    // the last assistant message can contain private success text in sticky
    // mid-stream failure cases. Report only a fixed pointer to the session.
    match decide_stream_completion(runtime_outcome, stream_error.take()) {
        StreamCompletionDecision::Complete => {
            if !has_runtime_outcome && !had_stream_error {
                tracing::debug!(
                    "Scheduled job '{}' stream ended without a RuntimeOutcome event; \
                     preserving legacy success behavior",
                    job_id
                );
            }
        }
        StreamCompletionDecision::StreamError(err) => {
            stream_error = Some(err);
        }
        StreamCompletionDecision::FailedOutcome => {
            agent_turn_failed = true;
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
            let remember_session_id = session.id.clone();

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
                            // Associate with the scheduled run's session so
                            // same-session memories co-rank on recall (#131).
                            session_id: Some(remember_session_id.clone()),
                            // This run IS the episode (R45). The session id is
                            // minted once per scheduled run, so every turn of
                            // one run shares it — the job id would instead merge
                            // every run the job has ever made into a single
                            // unbounded episode, and the time-gap heuristic
                            // would split a long run in half.
                            episode_id: Some(remember_session_id),
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
                        // remember_with returns Err if the session association
                        // fails even when the memory itself was committed, so
                        // don't claim it was lost. Fire-and-forget: logged only.
                        tracing::warn!(
                            target: "permagentd::brain",
                            "remember_with returned an error for scheduled turn {} (the memory may still be persisted; session association or a later step failed): {}",
                            key_for_log,
                            e
                        );
                    }
                }
            });
        }
    }

    // ── Phase 5: Persist a valid report before this run can succeed. ──
    // Only recipes explicitly declaring this output contract are affected.
    // Read the raw declaration: Recipe intentionally ignores extension keys.
    let declares_findings = serde_yaml::from_str::<serde_json::Value>(&recipe_content)
        .ok()
        .and_then(|v| v.get("produces_findings").and_then(|v| v.as_bool()))
        .unwrap_or(false);
    if declares_findings {
        let full_output = conversation
            .messages()
            .iter()
            .filter(|m| m.role == rmcp::model::Role::Assistant)
            .map(|m| m.as_concat_text())
            .collect::<Vec<_>>()
            .join("\n");
        let saved = std::env::var("HOME")
            .map_err(anyhow::Error::from)
            .and_then(|home| {
                let home = std::path::PathBuf::from(home);
                crate::storage_health::scanner::persist_report(
                    &full_output,
                    &session.id,
                    &home.join(".permagent/automation/findings"),
                    &home,
                    &crate::storage_health::inuse::action::ActionProbe,
                )
            });
        if let Err(error) = saved {
            tracing::warn!(target: "permagentd::automation", %error, "Findings report was not saved");
            return Err(anyhow!(
                "Report incomplete — cleanup findings could not be saved."
            ));
        }
    } else {
        // Other automations keep their existing optional extraction contract.
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
        let exit_type = if stream_error.is_some() {
            "error"
        } else {
            "normal"
        };
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

    // A mid-stream failure (provider 400, network death) used to return Ok
    // here, so the job was stamped last_status=ok with no assistant reply —
    // during the 2026-08-05 credit exhaustion every run "succeeded" while
    // producing nothing. Surface it: last_status/last_error, the failure
    // activity event, retries, and escalation all key off this Err.
    if let Some(err) = stream_error {
        return Err(anyhow!(
            "agent stream failed for scheduled job '{}' (session {} was created but the \
             turn did not complete): {}",
            job_id,
            session.id,
            err
        ));
    }
    if agent_turn_failed {
        return Err(anyhow::Error::new(AgentTurnFailed {
            session_id: session.id.clone(),
        }));
    }

    Ok(session.id)
}

fn failed_outcome_message(session_id: &str) -> String {
    format!("agent turn reported a failed outcome; open session {session_id} for the agent's explanation")
}

#[derive(Debug, Error)]
#[error("{}", failed_outcome_message(session_id))]
struct AgentTurnFailed {
    session_id: String,
}

#[derive(Debug, PartialEq, Eq)]
enum StreamCompletionDecision {
    Complete,
    StreamError(String),
    FailedOutcome,
}

fn runtime_outcome_of(event: &AgentEvent) -> Option<AgentRuntimeOutcome> {
    match event {
        AgentEvent::RuntimeOutcome(outcome) => Some(*outcome),
        _ => None,
    }
}

fn is_retryable_failure(error: &anyhow::Error) -> bool {
    error.downcast_ref::<AgentTurnFailed>().is_none()
}

fn decide_stream_completion(
    runtime_outcome: Option<AgentRuntimeOutcome>,
    stream_error: Option<String>,
) -> StreamCompletionDecision {
    if let Some(err) = stream_error {
        return StreamCompletionDecision::StreamError(err);
    }

    match runtime_outcome {
        // If max-turns was hit, the agent emits Succeeded by design; keep
        // treating that as a successful stream completion.
        Some(AgentRuntimeOutcome::Succeeded) => StreamCompletionDecision::Complete,
        Some(AgentRuntimeOutcome::Cancelled) => StreamCompletionDecision::Complete,
        Some(AgentRuntimeOutcome::Failed) => StreamCompletionDecision::FailedOutcome,
        None => StreamCompletionDecision::Complete,
    }
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

    async fn recent_sessions_by_schedule(
        &self,
        limit_per_schedule: usize,
    ) -> Result<Vec<ScheduleSessionSummary>, SchedulerError> {
        self.recent_sessions_by_schedule(limit_per_schedule).await
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
    use std::time::Instant;
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
    /// Every scheduled RUN must open its own budget task before it can spend.
    ///
    /// Reported 2026-09-21: "Nightly Health Review ... I did not send this
    /// request because provider spend could not be authorized: provider budget
    /// is unknown; refusing call: paid provider reservation requires a durable
    /// task identity." The ledger showed EVERY scheduled session from
    /// 2026-09-19 to 2026-09-21 with zero calls and zero cost — health-review,
    /// storage-insights and workspace-snapshot alike. They were not throttled;
    /// they could not spend at all, because `starts_new_budget_task`
    /// (agent.rs) opens one only for an interactive session type and a
    /// scheduled session has no parent to inherit an identity from.
    ///
    /// A source guard, and honest about it: exercising the real path needs a
    /// provider, a recipe on disk and a live agent, so what this pins is that
    /// the run-creation site still declares a budget task. Delete the call and
    /// this fails; that is the regression that silently killed automations.
    #[test]
    fn a_scheduled_run_opens_its_own_budget_task() {
        let src = include_str!("scheduler.rs");
        let at = src
            .find("SessionType::Scheduled,")
            .expect("the scheduled run is still created here");
        let after = &src[at..];
        let begins = after
            .find("begin_budget_task")
            .expect("a scheduled run must open a budget task or it cannot spend");
        let next_fn = after.find("\nasync fn ").unwrap_or(after.len());
        assert!(
            begins < next_fn,
            "the budget task must be opened in the same function that creates the run",
        );
    }

    #[test]
    fn stream_completion_decision_table() {
        assert_eq!(
            decide_stream_completion(Some(AgentRuntimeOutcome::Failed), None),
            StreamCompletionDecision::FailedOutcome
        );
        assert_eq!(
            decide_stream_completion(Some(AgentRuntimeOutcome::Succeeded), None),
            StreamCompletionDecision::Complete
        );
        assert_eq!(
            decide_stream_completion(Some(AgentRuntimeOutcome::Cancelled), None),
            StreamCompletionDecision::Complete
        );
        assert_eq!(
            decide_stream_completion(None, None),
            StreamCompletionDecision::Complete
        );
        assert_eq!(
            decide_stream_completion(Some(AgentRuntimeOutcome::Succeeded), None,),
            StreamCompletionDecision::Complete
        );

        let preserved = "stream transport failed".to_string();
        for outcome in [
            Some(AgentRuntimeOutcome::Failed),
            Some(AgentRuntimeOutcome::Succeeded),
            Some(AgentRuntimeOutcome::Cancelled),
            None,
        ] {
            assert_eq!(
                decide_stream_completion(outcome, Some(preserved.clone())),
                StreamCompletionDecision::StreamError(preserved.clone())
            );
        }
    }

    #[test]
    fn retryability_predicate_treats_typed_failed_outcome_as_non_retryable() {
        let wrapped = anyhow::Error::new(AgentTurnFailed {
            session_id: "sess-123".to_string(),
        })
        .context("wrapped");
        // anyhow's `Display` of a contexted error is the context alone; the
        // pointer message is in the chain (`{:#}`) and the typed value is what
        // the retry loop downcasts — both must survive `.context()`.
        assert_eq!(
            wrapped.root_cause().to_string(),
            failed_outcome_message("sess-123"),
            "typed failed outcome text must remain the fixed session pointer"
        );
        assert!(
            !is_retryable_failure(&wrapped),
            "typed failed outcome must not be retried even when wrapped in context"
        );

        let plain = anyhow!("stream transport failed");
        assert!(
            is_retryable_failure(&plain),
            "plain stream errors remain retryable"
        );
    }

    #[test]
    fn runtime_outcome_extractor_only_captures_runtime_outcome_events() {
        assert_eq!(
            runtime_outcome_of(&AgentEvent::RuntimeOutcome(AgentRuntimeOutcome::Failed)),
            Some(AgentRuntimeOutcome::Failed)
        );
        assert_eq!(
            runtime_outcome_of(&AgentEvent::Message(Message::user().with_text("noop"))),
            None
        );
    }

    #[test]
    fn scheduled_drain_captures_runtime_outcome_and_retry_checks_predicate() {
        let src = include_str!("scheduler.rs");
        let production = src
            .split_once("#[cfg(test)]\nmod tests")
            .map(|(before, _)| before)
            .unwrap_or(src);
        let loop_ix = production
            .find("while let Some(message_result) = stream.next().await")
            .expect("scheduler drain loop must exist");
        let extract_ix = production
            .find("runtime_outcome_of(&event)")
            .expect("drain loop must capture RuntimeOutcome through extractor");
        let assign_ix = production
            .find("runtime_outcome = Some(outcome);")
            .expect("drain loop must retain RuntimeOutcome values");
        let retry_ix = production
            .find("is_retryable_failure(&e)")
            .expect("bounded retry loop must check retryability predicate");

        assert!(
            loop_ix < extract_ix,
            "RuntimeOutcome handling must stay inside the drain loop"
        );
        assert!(
            extract_ix < assign_ix,
            "RuntimeOutcome handling must assign runtime_outcome after extraction"
        );
        assert!(
            retry_ix < loop_ix,
            "bounded retry loop should call is_retryable_failure before stream draining"
        );
    }

    /// `#[tokio::test]`, not `#[test]`: `Agent::new()` reaches sqlx's pool
    /// constructor, which panics ("requires a Tokio context") off a runtime.
    #[tokio::test]
    async fn scheduled_job_agents_are_headless() {
        let agent = new_scheduled_job_agent();
        assert!(agent.is_headless(), "scheduled-job agents must be headless");
    }

    /// D11: the weekday `git-steward` recipe is the Steward's OWN embodiment —
    /// a multi-minute LLM pass over every repo — and it announced nothing, so
    /// the World's Steward pill sat at STANDING BY through the whole run while
    /// the native 15-min sweep (which announces) was the only thing that could
    /// ever light it.
    ///
    /// The mapping is deliberately narrow: a job announces as an agent only
    /// when it DECLARES one (`worker_persona`) and that persona names a real
    /// worker descriptor. `workspace-snapshot` and `health-review` declare no
    /// persona — they are the scheduler's own work, not a character's — so
    /// they stay silent rather than being given a borrowed identity.
    #[test]
    fn only_a_worker_persona_job_announces_as_an_agent() {
        let steward = ScheduledJob {
            id: "git-steward".to_string(),
            worker_persona: Some("steward".to_string()),
            ..Default::default()
        };
        assert_eq!(
            job_announce_identity(&steward),
            Some((
                crate::steward::SELF_KNOWLEDGE_FEATURE.id,
                crate::steward::SELF_KNOWLEDGE_FEATURE.display_name
            )),
            "the recipe must announce on the SAME id the native sweep does \
             (steward_sweep.rs::announce), or it lights a second, empty orb"
        );

        for id in ["workspace-snapshot", "health-review", "storage-insights"] {
            let job = ScheduledJob {
                id: id.to_string(),
                ..Default::default()
            };
            assert_eq!(
                job_announce_identity(&job),
                None,
                "{id} declares no worker persona and must not borrow one"
            );
        }

        let unknown = ScheduledJob {
            id: "custom".to_string(),
            worker_persona: Some("not_a_worker".to_string()),
            ..Default::default()
        };
        assert_eq!(
            job_announce_identity(&unknown),
            None,
            "a persona key that names no worker descriptor has no orb to light"
        );
    }

    /// `recipe_dir` is a built-in template variable, not a declared parameter.
    /// The interactive path injects it (`build_recipe::apply_values_to_parameters`);
    /// the scheduler must too. It did not, and because rendering is strict, one
    /// undefined variable failed the WHOLE render — so the nightly health-review
    /// job mailed the model `{{ hours }}`/`{{ log_dir }}`/`{{ recipe_dir }}`
    /// verbatim every night while still reporting `completed`.
    #[test]
    fn scheduled_recipe_params_include_recipe_dir_and_render_all_defaults() {
        let dir = tempdir().unwrap();
        let recipe_path = dir.path().join("health-review.yaml");
        let recipe: Recipe = serde_yaml::from_str(
            r#"
version: "1"
title: health
description: scheduled diagnostic
parameters:
  - key: hours
    input_type: string
    requirement: optional
    description: evidence window
    default: "24"
  - key: log_dir
    input_type: string
    requirement: optional
    description: daemon log directory
    default: "$HOME/.permagent/logs"
prompt: 'collect {{ hours }}h from {{ log_dir }} with {{ recipe_dir }}/collector.py'
"#,
        )
        .unwrap();
        let params = scheduled_recipe_params(&recipe, &recipe_path, "health-review");
        let rendered = crate::recipe::template_recipe::render_recipe_content_with_params(
            recipe.prompt.as_deref().unwrap(),
            &params,
        )
        .unwrap();

        assert!(rendered.contains("collect 24h from $HOME/.permagent/logs"));
        assert!(rendered.contains(&format!("{}/collector.py", dir.path().display())));
        assert!(!rendered.contains("{{"), "{rendered}");
    }

    /// A render that cannot be completed must FAIL the job, never hand the raw
    /// `{{ ... }}` template to the model. The old fallback did exactly that:
    /// it logged a WARN and sent the unrendered prompt, so the run still ended
    /// `completed` — a wasted model call producing a confused reply that is
    /// easy to miss. Failing is loud: the caller records `Error`, `last_error`,
    /// and a consecutive-failure escalation.
    #[test]
    fn render_failure_fails_the_job_instead_of_sending_the_raw_template() {
        let raw_prompt = "audit {{ undeclared_and_not_built_in }} now";
        let mut params = HashMap::new();
        params.insert("hours".to_string(), "24".to_string());

        let result = render_scheduled_prompt(raw_prompt, &params, "health-review");

        let err = match result {
            Ok(rendered) => panic!("render failure fell back to a prompt: {rendered}"),
            Err(e) => e.to_string(),
        };
        assert!(
            !err.contains("{{"),
            "the error must describe the failure, not carry the template: {err}"
        );
        assert!(err.contains("health-review"), "{err}");
    }

    /// The approval gate must hold on the run_now path: an agent could
    /// otherwise create_recipe (lands paused + requires_approval) and fire it
    /// immediately via run_recipe, bypassing the pause entirely.
    #[tokio::test]
    async fn run_now_refuses_approval_pending_job() {
        let _guard = env_lock::lock_env([("PERMAGENT_MIN_SCHEDULE_INTERVAL_SECS", Some("0"))]);
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let recipe_path = create_test_recipe(temp_dir.path(), "pending_job");
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let scheduler = Scheduler::new(storage_path, session_manager).await.unwrap();

        let job = ScheduledJob {
            id: "pending_job".to_string(),
            source: recipe_path.to_string_lossy().to_string(),
            cron: "0 0 * * * *".to_string(),
            paused: true,
            requires_approval: true,
            ..Default::default()
        };
        scheduler.add_scheduled_job(job, true).await.unwrap();

        let err = scheduler
            .run_now("pending_job")
            .await
            .expect_err("run_now must refuse an approval-pending job");
        assert!(
            err.to_string().contains("approval"),
            "refusal must name the approval gate, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn update_schedule_rejects_floor_violation_without_mutating_job() {
        let _guard = env_lock::lock_env([("PERMAGENT_MIN_SCHEDULE_INTERVAL_SECS", None::<&str>)]);
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let recipe_path = create_test_recipe(temp_dir.path(), "update_floor");
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let scheduler = Scheduler::new(storage_path, session_manager).await.unwrap();
        let original_cron = "0 0 * * * *";

        scheduler
            .add_scheduled_job(
                ScheduledJob {
                    id: "update_floor".to_string(),
                    source: recipe_path.to_string_lossy().to_string(),
                    cron: original_cron.to_string(),
                    ..Default::default()
                },
                false,
            )
            .await
            .unwrap();

        let err = scheduler
            .update_schedule("update_floor", "0 */1 * * * *".to_string())
            .await
            .expect_err("every-minute update must be rejected");
        assert!(err.to_string().contains("below the minimum"));
        let jobs = scheduler.list_scheduled_jobs().await;
        let job = jobs.iter().find(|job| job.id == "update_floor").unwrap();
        assert_eq!(job.cron, original_cron);
    }

    #[tokio::test]
    async fn sync_from_storage_pauses_floor_violation() {
        let _guard = env_lock::lock_env([("PERMAGENT_MIN_SCHEDULE_INTERVAL_SECS", None::<&str>)]);
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let recipe_path = create_test_recipe(temp_dir.path(), "synced_floor");
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let scheduler = Scheduler::new(storage_path.clone(), session_manager)
            .await
            .unwrap();
        let job = ScheduledJob {
            id: "synced_floor".to_string(),
            source: recipe_path.to_string_lossy().to_string(),
            cron: "0 */1 * * * *".to_string(),
            ..Default::default()
        };
        fs::write(
            &storage_path,
            serde_json::to_string_pretty(&vec![job]).unwrap(),
        )
        .unwrap();

        let jobs = scheduler.list_scheduled_jobs().await;
        let job = jobs.iter().find(|job| job.id == "synced_floor").unwrap();
        assert!(job.paused);
        assert!(job
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("below the minimum")));
    }

    #[tokio::test]
    async fn unpause_schedule_refuses_floor_violation() {
        let _guard = env_lock::lock_env([("PERMAGENT_MIN_SCHEDULE_INTERVAL_SECS", None::<&str>)]);
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let recipe_path = create_test_recipe(temp_dir.path(), "paused_floor");
        let job = ScheduledJob {
            id: "paused_floor".to_string(),
            source: recipe_path.to_string_lossy().to_string(),
            cron: "0 */1 * * * *".to_string(),
            paused: true,
            ..Default::default()
        };
        fs::write(
            &storage_path,
            serde_json::to_string_pretty(&vec![job]).unwrap(),
        )
        .unwrap();
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let scheduler = Scheduler::new(storage_path, session_manager).await.unwrap();

        let err = scheduler
            .unpause_schedule("paused_floor")
            .await
            .expect_err("resume must reject an every-minute schedule");
        assert!(err.to_string().contains("below the minimum"));
        let jobs = scheduler.list_scheduled_jobs().await;
        let job = jobs.iter().find(|job| job.id == "paused_floor").unwrap();
        assert!(job.paused);
        assert!(job
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("below the minimum")));
    }

    #[test]
    fn interval_floor_rejects_runaway_schedules() {
        let _guard = env_lock::lock_env([("PERMAGENT_MIN_SCHEDULE_INTERVAL_SECS", None::<&str>)]);
        // Every-minute cron — the exact shape that burned credits on 2026-08-05.
        let every_minute = ScheduledJob {
            cron: "0 */1 * * * *".to_string(),
            ..Default::default()
        };
        assert!(validate_schedule_spec(&every_minute).is_err());
        // Sub-floor fixed interval.
        let every_minute_secs = ScheduledJob {
            every_seconds: Some(60),
            ..Default::default()
        };
        assert!(validate_schedule_spec(&every_minute_secs).is_err());
        // Hourly cron and daily cron stay legal.
        let hourly = ScheduledJob {
            cron: "0 0 * * * *".to_string(),
            ..Default::default()
        };
        assert!(validate_schedule_spec(&hourly).is_ok());
        let weekdays_6am = ScheduledJob {
            cron: "0 0 6 * * 1-5".to_string(),
            ..Default::default()
        };
        assert!(validate_schedule_spec(&weekdays_6am).is_ok());
        // One-time jobs are exempt (they fire once).
        let one_shot = ScheduledJob {
            at: Some(Utc::now() + chrono::Duration::seconds(30)),
            ..Default::default()
        };
        assert!(validate_schedule_spec(&one_shot).is_ok());
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
            tz: Some("utc".to_string()),
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

    /// D20: a weekly starter that sleeps through its one instant per week must
    /// run once, late — but the no-catch-up ruling for sub-daily crons stands.
    #[test]
    fn only_long_cadence_jobs_catch_up_and_only_while_the_run_is_still_worth_it() {
        let weekly = chrono::Duration::days(7);
        let daily = chrono::Duration::hours(24);
        let hourly = chrono::Duration::hours(1);

        // storage-insights: Sunday 19:00, one day late. Run it.
        assert!(should_catch_up(weekly, chrono::Duration::days(1)));
        assert!(should_catch_up(weekly, chrono::Duration::days(3)));

        // Too stale to be a recovery — the next natural tick is nearer.
        assert!(!should_catch_up(weekly, chrono::Duration::days(4)));

        // The original ruling, preserved: a sub-daily cron gets many chances a
        // day, and firing every missed one at boot is the wake-storm.
        assert!(!should_catch_up(hourly, chrono::Duration::hours(2)));
        assert!(
            !should_catch_up(daily, chrono::Duration::hours(2)),
            "a daily job lands in some live window on its own; catch-up is for longer cadences"
        );

        // Not yet due is never a catch-up.
        assert!(!should_catch_up(weekly, chrono::Duration::hours(-1)));
    }

    #[test]
    fn cron_period_and_missed_fire_read_the_real_schedule() {
        // The live storage-insights schedule: Sunday 19:00 UTC.
        let anchor = ts("2026-08-26T14:38:30Z");
        assert_eq!(
            cron_period("0 0 19 * * 0", anchor),
            Some(chrono::Duration::days(7)),
            "a Sunday cron's period is a week"
        );
        assert_eq!(
            cron_period("0 0 8 * * 1-5", anchor),
            Some(chrono::Duration::days(1)),
            "a weekday-daily cron's period is a day, so it must not catch up"
        );
        // The reason the measurement is the SMALLEST gap: read from a Thursday,
        // the very next gap for a weekday job is the three-day weekend, and
        // that reading would wrongly make it look weekly enough to catch up.
        assert_eq!(
            cron_period("0 0 8 * * 1-5", ts("2026-09-03T09:00:00Z")),
            Some(chrono::Duration::days(1)),
            "the weekend gap must not be mistaken for the job's cadence"
        );
        // Twice a week (Mon + Thu): its smallest gap is three days, so a missed
        // window IS worth catching up.
        assert_eq!(
            cron_period("0 0 10 * * 1,4", anchor),
            Some(chrono::Duration::days(3))
        );

        let job = ScheduledJob {
            cron: "0 0 19 * * 0".to_string(),
            tz: Some("utc".to_string()),
            last_run: Some(anchor),
            ..Default::default()
        };
        assert_eq!(
            missed_fire_at(&job),
            Some(ts("2026-08-30T19:00:00Z")),
            "the missed fire is the first occurrence after the last run"
        );

        let never_run = ScheduledJob {
            cron: "0 0 19 * * 0".to_string(),
            ..Default::default()
        };
        assert_eq!(missed_fire_at(&never_run), None);
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

    /// The Automate Stop wedge (2026-08-06): a run that died without cleanup
    /// leaves `currently_running=true` with no cancel token. Stop used to
    /// return Ok while clearing nothing — the schedule stayed "running"
    /// forever. Stop must clear the wedged state so the job is re-runnable.
    #[tokio::test]
    async fn kill_clears_wedged_running_state_when_no_live_run_exists() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let recipe_path = create_test_recipe(temp_dir.path(), "wedged_job");
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let scheduler = Scheduler::new(storage_path, session_manager).await.unwrap();

        let job = ScheduledJob {
            id: "wedged_job".to_string(),
            source: recipe_path.to_string_lossy().to_string(),
            cron: "0 0 0 1 1 *".to_string(),
            paused: true,
            ..Default::default()
        };
        scheduler.add_scheduled_job(job, true).await.unwrap();

        // Wedge it the way a dead run does: running per the job record, but no
        // entry in running_tasks.
        {
            let mut jobs_guard = scheduler.jobs.lock().await;
            let (_, job) = jobs_guard.get_mut("wedged_job").unwrap();
            job.currently_running = true;
            job.current_session_id = None;
            job.process_start_time = Some(Utc::now());
        }

        scheduler
            .kill_running_job("wedged_job")
            .await
            .expect("Stop on a wedged schedule must succeed");

        let jobs = scheduler.list_scheduled_jobs().await;
        let job = jobs.iter().find(|j| j.id == "wedged_job").unwrap();
        assert!(
            !job.currently_running,
            "Stop must clear the wedged running flag"
        );
        assert!(job.process_start_time.is_none());

        // And a second Stop now reports the truth: nothing is running.
        assert!(scheduler.kill_running_job("wedged_job").await.is_err());
    }

    /// The live-but-deaf wedge (2026-08-06, observed in production): the run
    /// task exists and its token is cancelled, but it is parked inside an
    /// await that never checks the token (a hung provider call), so its own
    /// teardown never runs. Stop must force-clear after the grace period.
    #[tokio::test]
    async fn kill_force_clears_a_run_that_ignores_cancellation() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let recipe_path = create_test_recipe(temp_dir.path(), "deaf_job");
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let scheduler = Scheduler::new(storage_path, session_manager).await.unwrap();

        let job = ScheduledJob {
            id: "deaf_job".to_string(),
            source: recipe_path.to_string_lossy().to_string(),
            cron: "0 0 0 1 1 *".to_string(),
            paused: true,
            ..Default::default()
        };
        scheduler.add_scheduled_job(job, true).await.unwrap();

        // Simulate a live run that will never observe cancellation: running
        // per the job record, with a real token in running_tasks and no task
        // watching it.
        {
            let mut jobs_guard = scheduler.jobs.lock().await;
            jobs_guard.get_mut("deaf_job").unwrap().1.currently_running = true;
        }
        scheduler
            .running_tasks
            .lock()
            .await
            .insert("deaf_job".to_string(), CancellationToken::new());

        scheduler.kill_running_job("deaf_job").await.unwrap();

        // Immediately after Stop the run may legitimately still be winding
        // down — force-clear only fires after the grace period (200ms in
        // tests).
        sleep(Duration::from_millis(600)).await;

        let jobs = scheduler.list_scheduled_jobs().await;
        let job = jobs.iter().find(|j| j.id == "deaf_job").unwrap();
        assert!(
            !job.currently_running,
            "Stop must force-clear a run that ignores cancellation"
        );
        assert!(
            !scheduler
                .running_tasks
                .lock()
                .await
                .contains_key("deaf_job"),
            "the orphaned token must be removed"
        );
    }

    #[tokio::test]
    async fn test_job_runs_on_schedule() {
        let _guard = env_lock::lock_env([
            ("GOOSE_PROVIDER", Some("openai")),
            ("GOOSE_MODEL", Some("gpt-4o")),
            ("OPENAI_API_KEY", Some("fake-openai-no-keyring")),
            ("OPENAI_CUSTOM_HEADERS", Some("")),
            // These tests fire every second; disable the production interval
            // floor (which would reject the job at add time).
            ("PERMAGENT_MIN_SCHEDULE_INTERVAL_SECS", Some("0")),
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
    async fn new_without_cron_loads_jobs_but_does_not_run_them() {
        let _guard = env_lock::lock_env([
            ("GOOSE_PROVIDER", Some("openai")),
            ("GOOSE_MODEL", Some("gpt-4o")),
            ("OPENAI_API_KEY", Some("fake-openai-no-keyring")),
            ("OPENAI_CUSTOM_HEADERS", Some("")),
            ("PERMAGENT_MIN_SCHEDULE_INTERVAL_SECS", Some("0")),
        ]);
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let recipe_path = create_test_recipe(temp_dir.path(), "inspection_job");
        let seeded_jobs = vec![ScheduledJob {
            id: "inspection_job".to_string(),
            source: recipe_path.to_string_lossy().to_string(),
            cron: "* * * * * *".to_string(),
            ..Default::default()
        }];
        fs::write(
            &storage_path,
            serde_json::to_string_pretty(&seeded_jobs).unwrap(),
        )
        .unwrap();

        let no_cron_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let no_cron = Scheduler::new_without_cron(storage_path.clone(), no_cron_manager)
            .await
            .unwrap();
        // Fixed wait only on the negative half: extra delay can only make a false
        // positive (still none) more likely, never turn a true negative into a failure.
        sleep(Duration::from_millis(1500)).await;
        let inspection_jobs = no_cron.list_scheduled_jobs().await;
        let inspection_job = inspection_jobs
            .iter()
            .find(|job| job.id == "inspection_job")
            .unwrap();
        assert!(
            inspection_job.last_run.is_none(),
            "read-only constructor must not execute jobs"
        );
        drop(no_cron);

        let live_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let live_scheduler = Scheduler::new(storage_path, live_manager).await.unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut saw_last_run = false;
        while Instant::now() < deadline {
            let live_jobs = live_scheduler.list_scheduled_jobs().await;
            let live_job = live_jobs
                .iter()
                .find(|job| job.id == "inspection_job")
                .unwrap();
            if live_job.last_run.is_some() {
                saw_last_run = true;
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        assert!(
            saw_last_run,
            "live constructor should execute scheduled jobs"
        );
        drop(live_scheduler);
    }

    #[tokio::test]
    async fn new_without_cron_lists_the_same_jobs_as_new() {
        let _guard = env_lock::lock_env([("PERMAGENT_MIN_SCHEDULE_INTERVAL_SECS", Some("0"))]);
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let recipe_path = create_test_recipe(temp_dir.path(), "same_jobs");
        let seeded_jobs = vec![
            ScheduledJob {
                id: "same_jobs_hourly".to_string(),
                source: recipe_path.to_string_lossy().to_string(),
                cron: "0 0 * * * *".to_string(),
                ..Default::default()
            },
            ScheduledJob {
                id: "same_jobs_daily".to_string(),
                source: recipe_path.to_string_lossy().to_string(),
                cron: "0 0 0 * * *".to_string(),
                ..Default::default()
            },
        ];
        fs::write(
            &storage_path,
            serde_json::to_string_pretty(&seeded_jobs).unwrap(),
        )
        .unwrap();

        let inspect_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let inspect_scheduler = Scheduler::new_without_cron(storage_path.clone(), inspect_manager)
            .await
            .unwrap();
        let mut inspect_pairs: Vec<(String, String)> = inspect_scheduler
            .list_scheduled_jobs()
            .await
            .into_iter()
            .map(|job| (job.id, job.cron))
            .collect();
        inspect_pairs.sort();

        let live_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let live_scheduler = Scheduler::new(storage_path, live_manager).await.unwrap();
        let mut live_pairs: Vec<(String, String)> = live_scheduler
            .list_scheduled_jobs()
            .await
            .into_iter()
            .map(|job| (job.id, job.cron))
            .collect();
        live_pairs.sort();

        assert_eq!(
            inspect_pairs, live_pairs,
            "inspection constructor should read the same persisted job set"
        );
    }

    #[tokio::test]
    async fn new_without_cron_does_not_rewrite_schedule_json_on_open() {
        let _guard = env_lock::lock_env([("PERMAGENT_MIN_SCHEDULE_INTERVAL_SECS", Some("0"))]);
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let recipe_path = create_test_recipe(temp_dir.path(), "no_rewrite");
        let seeded_jobs = vec![ScheduledJob {
            id: "no_rewrite".to_string(),
            source: recipe_path.to_string_lossy().to_string(),
            cron: "0 0 * * * *".to_string(),
            currently_running: true,
            current_session_id: Some("stale-session".to_string()),
            process_start_time: Some(Utc::now()),
            ..Default::default()
        }];
        fs::write(
            &storage_path,
            serde_json::to_vec_pretty(&seeded_jobs).unwrap(),
        )
        .unwrap();
        let before = fs::read(&storage_path).unwrap();

        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let _scheduler = Scheduler::new_without_cron(storage_path.clone(), session_manager)
            .await
            .unwrap();

        let after = fs::read(&storage_path).unwrap();
        assert_eq!(
            before, after,
            "inspection open must not reconcile or rewrite schedule.json"
        );
    }

    #[tokio::test]
    async fn new_without_cron_preserves_legacy_job_after_removing_another_job() {
        let _guard = env_lock::lock_env([("PERMAGENT_MIN_SCHEDULE_INTERVAL_SECS", Some("0"))]);
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let legacy_recipe = create_test_recipe(temp_dir.path(), "legacy-job");
        let other_recipe = create_test_recipe(temp_dir.path(), "same_jobs_hourly");
        // Cron input and expected normalization are copied from
        // normalize_cron_converts_5field_and_is_idempotent above.
        let seeded_jobs = vec![
            ScheduledJob {
                id: "legacy-job".to_string(),
                source: legacy_recipe.to_string_lossy().to_string(),
                cron: "0 6 * * 1-5".to_string(),
                paused: true,
                ..Default::default()
            },
            ScheduledJob {
                id: "same_jobs_hourly".to_string(),
                source: other_recipe.to_string_lossy().to_string(),
                cron: "0 0 * * * *".to_string(),
                paused: true,
                ..Default::default()
            },
        ];
        let before = serde_json::to_vec_pretty(&seeded_jobs).unwrap();
        fs::write(&storage_path, &before).unwrap();

        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let scheduler = Scheduler::new_without_cron(storage_path.clone(), session_manager)
            .await
            .unwrap();
        assert_eq!(fs::read(&storage_path).unwrap(), before);

        let listed = scheduler.list_scheduled_jobs().await;
        assert_eq!(listed.len(), 2);
        let legacy = listed
            .iter()
            .find(|job| job.id == seeded_jobs[0].id)
            .unwrap();
        assert_eq!(legacy.cron, "0 0 6 * * 1-5");
        assert!(listed.iter().any(|job| job.id == seeded_jobs[1].id));
        assert_eq!(fs::read(&storage_path).unwrap(), before);

        scheduler
            .remove_scheduled_job(&seeded_jobs[1].id, true)
            .await
            .unwrap();
        let persisted: Vec<ScheduledJob> =
            serde_json::from_slice(&fs::read(&storage_path).unwrap()).unwrap();
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].id, seeded_jobs[0].id);
        assert_eq!(persisted[0].source, seeded_jobs[0].source);
        assert_eq!(persisted[0].cron, "0 0 6 * * 1-5");
        assert!(legacy_recipe.exists());
        assert!(!other_recipe.exists());
    }

    #[tokio::test]
    async fn only_live_constructor_clears_persisted_running_flag() {
        let _guard = env_lock::lock_env([("PERMAGENT_MIN_SCHEDULE_INTERVAL_SECS", Some("0"))]);
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let recipe_path = create_test_recipe(temp_dir.path(), "no_rewrite");
        // Reuse new_without_cron_does_not_rewrite_schedule_json_on_open's
        // stale-flag fixture. Pausing prevents a cron tick from racing the checks.
        let seeded_jobs = vec![ScheduledJob {
            id: "no_rewrite".to_string(),
            source: recipe_path.to_string_lossy().to_string(),
            cron: "0 0 * * * *".to_string(),
            currently_running: true,
            paused: true,
            current_session_id: Some("stale-session".to_string()),
            process_start_time: Some(Utc::now()),
            ..Default::default()
        }];
        let before = serde_json::to_vec_pretty(&seeded_jobs).unwrap();
        fs::write(&storage_path, &before).unwrap();

        let inspect_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let inspect_scheduler = Scheduler::new_without_cron(storage_path.clone(), inspect_manager)
            .await
            .unwrap();
        let after_inspect = fs::read(&storage_path).unwrap();
        assert_eq!(after_inspect, before);
        let inspect_disk: Vec<ScheduledJob> = serde_json::from_slice(&after_inspect).unwrap();
        assert_eq!(inspect_disk.len(), 1);
        assert!(inspect_disk[0].currently_running);
        let inspect_jobs = inspect_scheduler.list_scheduled_jobs().await;
        assert_eq!(inspect_jobs.len(), 1);
        assert!(inspect_jobs[0].currently_running);
        drop(inspect_scheduler);

        let live_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let live_scheduler = Scheduler::new(storage_path.clone(), live_manager)
            .await
            .unwrap();
        let live_disk: Vec<ScheduledJob> =
            serde_json::from_slice(&fs::read(&storage_path).unwrap()).unwrap();
        assert_eq!(live_disk.len(), 1);
        assert_eq!(live_disk[0].id, seeded_jobs[0].id);
        assert!(!live_disk[0].currently_running);
        assert!(live_disk[0].current_session_id.is_none());
        assert!(live_disk[0].process_start_time.is_none());
        let live_jobs = live_scheduler.list_scheduled_jobs().await;
        assert_eq!(live_jobs.len(), 1);
        assert!(!live_jobs[0].currently_running);
    }

    /// D11, on the real cron path: a fire of a worker-persona job must put an
    /// `agent_state_changed` frame on the bus, or the World View has nothing to
    /// render no matter how long the run takes. The run itself fails here (no
    /// real provider), which is the point of asserting only the FIRST frame:
    /// `working` is emitted before the work, not derived from its outcome.
    #[tokio::test]
    async fn worker_persona_job_announces_working_on_the_bus() {
        let _guard = env_lock::lock_env([
            ("GOOSE_PROVIDER", Some("openai")),
            ("GOOSE_MODEL", Some("gpt-4o")),
            ("OPENAI_API_KEY", Some("fake-openai-no-keyring")),
            ("OPENAI_CUSTOM_HEADERS", Some("")),
            ("PERMAGENT_MIN_SCHEDULE_INTERVAL_SECS", Some("0")),
        ]);
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let recipe_path = create_test_recipe(temp_dir.path(), "git-steward");
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let scheduler = Scheduler::new(storage_path, session_manager).await.unwrap();

        let mut bus = crate::events::subscribe();

        scheduler
            .add_scheduled_job(
                ScheduledJob {
                    id: "git-steward".to_string(),
                    source: recipe_path.to_string_lossy().to_string(),
                    cron: "* * * * * *".to_string(),
                    worker_persona: Some("steward".to_string()),
                    ..Default::default()
                },
                true,
            )
            .await
            .unwrap();
        sleep(Duration::from_millis(1500)).await;

        let mut announced = false;
        while let Ok(evt) = bus.try_recv() {
            let payload = &evt.payload;
            if payload["agent_id"] == crate::steward::SELF_KNOWLEDGE_FEATURE.id
                && payload["state"] == "working"
            {
                announced = true;
            }
        }
        assert!(
            announced,
            "a git-steward fire must announce `working` on the git_steward id"
        );
    }

    #[tokio::test]
    async fn test_paused_job_does_not_run() {
        let _guard = env_lock::lock_env([
            ("GOOSE_PROVIDER", Some("openai")),
            ("GOOSE_MODEL", Some("gpt-4o")),
            ("OPENAI_API_KEY", Some("fake-openai-no-keyring")),
            ("OPENAI_CUSTOM_HEADERS", Some("")),
            // These tests fire every second; disable the production interval
            // floor (which would reject the job at add time).
            ("PERMAGENT_MIN_SCHEDULE_INTERVAL_SECS", Some("0")),
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
            // These tests fire every second; disable the production interval
            // floor (which would reject the job at add time).
            ("PERMAGENT_MIN_SCHEDULE_INTERVAL_SECS", Some("0")),
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

    /// R1 "one writer, one announcement": the AGENT's schedule path must
    /// announce identically to the human's.
    ///
    /// FAILS BEFORE: every `schedule_changed` emit lived in the
    /// `/schedule/*` HTTP handlers. `platform__manage_schedule` and
    /// `platform__recipes` call the SchedulerTrait directly — no handler
    /// involved — so an automation the agent created, paused, resumed or
    /// deleted announced nothing at all, and the Automate tab (which listens on
    /// this exact frame, `lib/useScheduleEvents.ts`) waited out its 60s backstop
    /// poll or missed it entirely.
    ///
    /// Driven through `Arc<dyn SchedulerTrait>` on purpose: that is the type
    /// the agent tools hold, so this exercises their path and not a shortcut.
    #[tokio::test]
    async fn the_agents_schedule_path_announces_every_mutation() {
        let _guard = env_lock::lock_env([("PERMAGENT_MIN_SCHEDULE_INTERVAL_SECS", Some("0"))]);
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let recipe_path = create_test_recipe(temp_dir.path(), "r1_agent_path");
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let scheduler: Arc<dyn crate::scheduler_trait::SchedulerTrait> =
            Scheduler::new(storage_path, session_manager).await.unwrap();

        let changes = |rx: &mut tokio::sync::broadcast::Receiver<crate::events::PermagentEvent>| {
            let mut seen = Vec::new();
            while let Ok(e) = rx.try_recv() {
                if e.event_type == crate::events::PermagentEventType::ScheduleChanged
                    && e.payload.get("schedule_id").and_then(|v| v.as_str())
                        == Some("r1_agent_path")
                {
                    seen.push(
                        e.payload
                            .get("change")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    );
                }
            }
            seen
        };

        let mut rx = crate::events::subscribe();
        scheduler
            .add_scheduled_job(
                ScheduledJob {
                    id: "r1_agent_path".to_string(),
                    source: recipe_path.to_string_lossy().to_string(),
                    cron: "0 0 * * * *".to_string(),
                    ..Default::default()
                },
                false,
            )
            .await
            .unwrap();
        assert_eq!(changes(&mut rx), vec!["created"], "create was silent");

        let mut rx = crate::events::subscribe();
        scheduler.pause_schedule("r1_agent_path").await.unwrap();
        assert_eq!(changes(&mut rx), vec!["paused"], "pause was silent");

        let mut rx = crate::events::subscribe();
        scheduler.unpause_schedule("r1_agent_path").await.unwrap();
        assert_eq!(changes(&mut rx), vec!["unpaused"], "unpause was silent");

        let mut rx = crate::events::subscribe();
        scheduler
            .update_schedule("r1_agent_path", "0 30 * * * *".to_string())
            .await
            .unwrap();
        assert_eq!(changes(&mut rx), vec!["updated"], "update was silent");

        // A rejected write announces nothing: the bus is for real mutations.
        let mut rx = crate::events::subscribe();
        assert!(scheduler
            .remove_scheduled_job("no-such-job", false)
            .await
            .is_err());
        assert!(
            changes(&mut rx).is_empty(),
            "a failed delete announced a change"
        );

        let mut rx = crate::events::subscribe();
        scheduler
            .remove_scheduled_job("r1_agent_path", false)
            .await
            .unwrap();
        assert_eq!(changes(&mut rx), vec!["deleted"], "delete was silent");
    }
}

#[cfg(test)]
mod fire_claim_tests {
    use super::*;

    fn instant(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn weekly_job(id: &str) -> ScheduledJob {
        ScheduledJob {
            id: id.to_string(),
            cron: "0 0 10 * * 1".to_string(),
            tz: Some("-02:00".to_string()),
            last_run: Some(instant("2026-09-14T12:00:00Z")),
            ..Default::default()
        }
    }

    async fn scheduler_at(dir: &Path) -> Arc<Scheduler> {
        Scheduler::new_without_cron(
            dir.join("schedule.json"),
            Arc::new(SessionManager::new(dir.to_path_buf())),
        )
        .await
        .unwrap()
    }

    async fn install_job(scheduler: &Scheduler, job: ScheduledJob) {
        scheduler
            .jobs
            .lock()
            .await
            .insert(job.id.clone(), (uuid::Uuid::new_v4(), job));
        persist_jobs(&scheduler.storage_path, &scheduler.jobs)
            .await
            .unwrap();
    }

    async fn snapshot(scheduler: &Scheduler, id: &str) -> serde_json::Value {
        serde_json::to_value(&scheduler.jobs.lock().await.get(id).unwrap().1).unwrap()
    }

    async fn insert_competitor(pool: &sqlx::SqlitePool, job: &str, key: &str) {
        // Deliberately seed a competing process's row directly, not by walking
        // this scheduler's happy path. Columns/shape copied from the schema.
        sqlx::query(
            "INSERT INTO schedule_fire_claims
             (job_id, fire_key, kind, claimed_at, owner_pid, owner_token)
             VALUES (?, ?, 'cron', ?, ?, ?)",
        )
        .bind(job)
        .bind(key)
        .bind(Utc::now().to_rfc3339())
        .bind(claim_owner().pid)
        .bind(uuid::Uuid::new_v4().to_string())
        .execute(pool)
        .await
        .unwrap();
    }

    #[test]
    fn missed_occurrence_uses_armed_clock_and_one_cron_formatter() {
        let job = weekly_job("clock");
        let due = instant("2026-09-21T12:00:00Z");
        assert_eq!(missed_fire_at(&job), Some(due));
        assert!(!is_run_missed(&job, instant("2026-09-21T10:00:00Z")));
        assert!(is_run_missed(&job, due));
        // Prefix and complete seconds field copied from cron_fire_key's format.
        let expected = "cron:2026-09-21T10:00:00";
        assert_eq!(key_for_tick(&job, due).0, expected);
        assert_eq!(key_for_catch_up(&job).as_deref(), Some(expected));
    }

    #[test]
    fn fire_key_table_preserves_occurrences_seconds_midnight_and_kind() {
        for (cron, tz, last, due, expected) in [
            (
                "17 0 10 * * 1",
                "utc",
                "2026-09-14T10:00:17Z",
                "2026-09-21T10:00:17Z",
                "cron:2026-09-21T10:00:17",
            ),
            (
                "0 0 0 * * 1",
                "utc",
                "2026-09-14T00:00:00Z",
                "2026-09-21T00:00:00Z",
                "cron:2026-09-21T00:00:00",
            ),
            (
                "0 0 10 * * 1",
                "-02:00",
                "2026-09-14T12:00:00Z",
                "2026-09-21T12:00:00Z",
                "cron:2026-09-21T10:00:00",
            ),
        ] {
            let job = ScheduledJob {
                cron: cron.to_string(),
                tz: Some(tz.to_string()),
                last_run: Some(instant(last)),
                ..Default::default()
            };
            let due = instant(due);
            for delay in [
                Duration::milliseconds(5),
                Duration::seconds(61),
                Duration::minutes(14),
            ] {
                assert_eq!(key_for_tick(&job, due + delay).0, expected);
            }
            assert_eq!(key_for_catch_up(&job).as_deref(), Some(expected));
            assert_ne!(key_for_tick(&job, due + Duration::days(7)).0, expected);
            let mut advanced = job.clone();
            advanced.last_run = Some(due);
            assert_ne!(key_for_catch_up(&advanced), key_for_catch_up(&job));
        }
        let west = weekly_job("tz");
        let mut east = west.clone();
        east.tz = Some("+02:00".to_string());
        assert_eq!(
            key_for_tick(&west, instant("2026-09-21T12:00:00Z")).0,
            key_for_tick(&east, instant("2026-09-21T08:00:00Z")).0,
        );
        let interval = ScheduledJob {
            every_seconds: Some(900),
            ..Default::default()
        };
        for seconds in [5, 61, 899] {
            assert_eq!(
                key_for_tick(&interval, DateTime::from_timestamp(seconds, 0).unwrap()).0,
                "interval:0"
            );
        }
        assert_eq!(
            key_for_tick(&interval, DateTime::from_timestamp(900, 0).unwrap()).0,
            "interval:900"
        );
        let at = ScheduledJob {
            at: Some(instant("2026-09-21T12:00:00Z")),
            ..Default::default()
        };
        assert_eq!(
            key_for_tick(&at, instant("2026-09-22T12:00:00Z")).0,
            "at:2026-09-21T12:00:00+00:00"
        );
        let seconds = ScheduledJob {
            cron: "* * * * * *".to_string(),
            tz: Some("utc".to_string()),
            ..Default::default()
        };
        assert_eq!(
            key_for_tick(&seconds, instant("2026-09-21T12:00:01Z")).0,
            "cron:2026-09-21T12:00:01"
        );
    }

    #[test]
    fn sparse_cron_searches_forward_and_falls_back_outside_tolerance() {
        // Expression copied from cron_period_and_missed_fire_read_the_real_schedule.
        let job = ScheduledJob {
            cron: "0 0 10 * * 1,4".to_string(),
            tz: Some("utc".to_string()),
            ..Default::default()
        };
        assert_eq!(
            key_for_tick(&job, instant("2026-09-24T10:00:05Z")).0,
            "cron:2026-09-24T10:00:00"
        );
        for arrival in [
            "2026-09-24T10:16:59Z",
            "2026-09-24T10:17:01Z",
            "2026-09-25T09:59:00Z",
        ] {
            assert_eq!(
                key_for_tick(&job, instant(arrival)).0,
                "cron:2026-09-24T10:00:00"
            );
        }
        // Derivation correction: Thursday is 72h01m before Sunday 10:01,
        // outside the specified 24h search. Friday 09:59 above tests stage two.
        assert_eq!(
            key_for_tick(&job, instant("2026-09-27T10:01:00Z")).0,
            "fallback:2026-09-27T10:01:00+00:00"
        );
    }

    // These production-path tests are not redundant with key/schema/claim-unit tests.
    // Removing production admission is caught by:
    // direct_competitor_blocks_production_tick_without_state_mutation;
    // tick_and_catch_up_contend_in_both_directions_and_across_minutes;
    // manual_liveness_guard_refuses_alive_and_finishes_dead_owners;
    // retry_is_a_continuation_of_one_claim_and_failure_finishes_it.
    #[tokio::test]
    async fn direct_competitor_blocks_production_tick_without_state_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let scheduler = scheduler_at(dir.path()).await;
        let job = weekly_job("competing");
        install_job(&scheduler, job.clone()).await;
        let before = snapshot(&scheduler, &job.id).await;
        let disk_before = fs::read(&scheduler.storage_path).unwrap();
        let pool = scheduler.session_manager.pool_clone().await.unwrap();
        let due = instant("2026-09-21T12:00:00Z");
        let key = key_for_tick(&job, due).0;
        insert_competitor(&pool, &job.id, &key).await;
        assert_eq!(
            try_claim_fire(&pool, &job.id, &key, "cron", claim_owner()).await,
            ClaimOutcome::Lost
        );
        scheduler.task_for_job(job.clone())(due).await;
        assert_eq!(snapshot(&scheduler, &job.id).await, before);
        assert_eq!(fs::read(&scheduler.storage_path).unwrap(), disk_before);
        assert!(scheduler.running_tasks.lock().await.is_empty());
        assert_eq!(
            try_claim_fire(
                &pool,
                &job.id,
                &key_for_tick(&job, due + Duration::days(7)).0,
                "cron",
                claim_owner()
            )
            .await,
            ClaimOutcome::Won
        );
        assert_eq!(
            try_claim_fire(&pool, "different-job", &key, "cron", claim_owner()).await,
            ClaimOutcome::Won
        );
    }

    #[tokio::test]
    async fn separate_pools_race_for_one_database_key() {
        let dir = tempfile::tempdir().unwrap();
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(dir.path().join("claims.db"))
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(30));
        let first = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_with(options.clone())
            .await
            .unwrap();
        crate::session::spectral_schema::apply_schedule_fire_claims_schema(&first)
            .await
            .unwrap();
        let second = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_with(options)
            .await
            .unwrap();
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let spawn = |pool: sqlx::SqlitePool, barrier: Arc<tokio::sync::Barrier>| {
            tokio::spawn(async move {
                let owner = ClaimOwner {
                    pid: claim_owner().pid,
                    token: uuid::Uuid::new_v4().to_string(),
                };
                barrier.wait().await;
                try_claim_fire(&pool, "race", "interval:0", "interval", &owner).await
            })
        };
        let a = spawn(first, barrier.clone());
        let b = spawn(second, barrier);
        let results = [a.await.unwrap(), b.await.unwrap()];
        assert_eq!(
            results
                .iter()
                .filter(|outcome| **outcome == ClaimOutcome::Won)
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|outcome| **outcome == ClaimOutcome::Lost)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn missing_claim_store_fails_awake_in_production_callback() {
        let dir = tempfile::tempdir().unwrap();
        let scheduler = scheduler_at(dir.path()).await;
        let job = weekly_job("unavailable");
        install_job(&scheduler, job.clone()).await;
        let pool = scheduler.session_manager.pool_clone().await.unwrap();
        sqlx::query("DROP TABLE schedule_fire_claims")
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(
            try_claim_fire(&pool, &job.id, "interval:0", "interval", claim_owner()).await,
            ClaimOutcome::StoreUnavailable(_)
        ));
        scheduler.task_for_job(job.clone())(instant("2026-09-21T12:00:00Z")).await;
        let jobs = scheduler.jobs.lock().await;
        let result = &jobs.get(&job.id).unwrap().1;
        assert_eq!(result.run_count, 1);
        assert_eq!(result.last_status, Some(ScheduleRunStatus::Ok));
        assert!(!result.currently_running);
    }

    #[tokio::test]
    async fn start_marker_store_error_still_executes_once() {
        let dir = tempfile::tempdir().unwrap();
        let scheduler = scheduler_at(dir.path()).await;
        let job = weekly_job("start-unavailable");
        install_job(&scheduler, job.clone()).await;
        let claim = scheduler.claim_catch_up(&job).await.unwrap().unwrap();
        // Reuse the missing-store seam, but fail AFTER successful admission.
        sqlx::query("DROP TABLE schedule_fire_claims")
            .execute(&claim.pool)
            .await
            .unwrap();
        assert_eq!(
            scheduler
                .run_claimed(&job.id, Some(claim), true)
                .await
                .unwrap(),
            job.id
        );
        let jobs = scheduler.jobs.lock().await;
        let result = &jobs.get(&job.id).unwrap().1;
        assert_eq!(result.run_count, 1);
        assert_eq!(result.last_status, Some(ScheduleRunStatus::Ok));
        assert!(!result.currently_running);
    }

    #[tokio::test]
    async fn tick_and_catch_up_contend_in_both_directions_and_across_minutes() {
        for catch_up_first in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let scheduler = scheduler_at(dir.path()).await;
            let job = weekly_job("same-fire");
            install_job(&scheduler, job.clone()).await;
            let due = instant("2026-09-21T12:00:00Z");
            if catch_up_first {
                let claim = scheduler.claim_catch_up(&job).await.unwrap();
                let before = snapshot(&scheduler, &job.id).await;
                scheduler.task_for_job(job.clone())(due + Duration::seconds(61)).await;
                assert_eq!(snapshot(&scheduler, &job.id).await, before);
                // The own catch-up claim is a continuation, never a manual
                // admission (which would create a second row).
                assert_eq!(
                    scheduler.run_claimed(&job.id, claim, true).await.unwrap(),
                    job.id
                );
            } else {
                scheduler.task_for_job(job.clone())(due + Duration::seconds(59)).await;
                let before = snapshot(&scheduler, &job.id).await;
                assert!(scheduler.claim_catch_up(&job).await.is_none());
                assert_eq!(snapshot(&scheduler, &job.id).await, before);
            }
            let pool = scheduler.session_manager.pool_clone().await.unwrap();
            let count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM schedule_fire_claims WHERE job_id = ?")
                    .bind(&job.id)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(count, 1);
            assert_eq!(
                try_claim_fire(&pool, &job.id, "interval:0", "interval", claim_owner()).await,
                ClaimOutcome::Won
            );
        }
        let dir = tempfile::tempdir().unwrap();
        let first = scheduler_at(dir.path()).await;
        let job = weekly_job("minute-boundary");
        install_job(&first, job.clone()).await;
        let second = scheduler_at(dir.path()).await;
        // Simulated arrivals straddle a minute. Neither enters the derivation:
        // both production catch-up paths use the same persisted job snapshot.
        for arrival in [
            instant("2026-09-21T12:00:59Z"),
            instant("2026-09-21T12:01:01Z"),
        ] {
            assert!(is_run_missed(&job, arrival));
        }
        let first_claim = first.claim_catch_up(&job).await.unwrap().unwrap();
        assert!(second.claim_catch_up(&job).await.is_none());
        first_claim.finish().await;
    }

    #[tokio::test]
    async fn manual_liveness_guard_refuses_alive_and_finishes_dead_owners() {
        let dir = tempfile::tempdir().unwrap();
        let scheduler = scheduler_at(dir.path()).await;
        let job = weekly_job("manual-guard");
        install_job(&scheduler, job.clone()).await;
        let pool = scheduler.session_manager.pool_clone().await.unwrap();
        let key = key_for_catch_up(&job).unwrap();
        insert_competitor(&pool, &job.id, &key).await;
        // A live owner below the seven-day wedge threshold still blocks.
        sqlx::query("UPDATE schedule_fire_claims SET claimed_at = ? WHERE job_id = ?")
            .bind((Utc::now() - Duration::days(6)).to_rfc3339())
            .bind(&job.id)
            .execute(&pool)
            .await
            .unwrap();
        let before = snapshot(&scheduler, &job.id).await;
        let error = scheduler
            .run_now_with_liveness(&job.id, Utc::now(), |_| true)
            .await
            .unwrap_err();
        match error {
            SchedulerError::AnyhowError(error) => assert_eq!(
                error.to_string(),
                format!("Job '{}' is already running", job.id)
            ),
            other => panic!("unexpected refusal: {other:?}"),
        }
        assert_eq!(snapshot(&scheduler, &job.id).await, before);
        assert_eq!(
            scheduler
                .run_now_with_liveness(&job.id, Utc::now(), |_| false)
                .await
                .unwrap(),
            job.id
        );
        assert_eq!(
            scheduler
                .run_now_with_liveness(&job.id, Utc::now(), |_| true)
                .await
                .unwrap(),
            job.id
        );
        let unfinished: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM schedule_fire_claims WHERE finished_at IS NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(unfinished, 0);
        let manual: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM schedule_fire_claims WHERE kind = 'manual'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(manual, 2, "two deliberate presses are two requests");
    }

    #[tokio::test]
    async fn retry_is_a_continuation_of_one_claim_and_failure_finishes_it() {
        let dir = tempfile::tempdir().unwrap();
        let scheduler = scheduler_at(dir.path()).await;
        let mut job = weekly_job("retry-claim");
        // A missing local recipe fails before any provider/network work.
        job.source = dir
            .path()
            .join("missing.yaml")
            .to_string_lossy()
            .into_owned();
        job.max_retries = 1;
        job.last_status = Some(ScheduleRunStatus::Error); // no escalation for this existing streak
        install_job(&scheduler, job.clone()).await;
        scheduler.task_for_job(job.clone())(instant("2026-09-21T12:00:00Z")).await;
        let jobs = scheduler.jobs.lock().await;
        let result = &jobs.get(&job.id).unwrap().1;
        assert_eq!(result.run_count, 1);
        assert_eq!(result.retry_count, 1);
        assert_eq!(result.last_status, Some(ScheduleRunStatus::Error));
        assert!(!result.currently_running);
        drop(jobs);
        let pool = scheduler.session_manager.pool_clone().await.unwrap();
        let rows: (i64, i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), COUNT(started_at), COUNT(finished_at) FROM schedule_fire_claims WHERE job_id = ?",
        ).bind(&job.id).fetch_one(&pool).await.unwrap();
        assert_eq!(rows, (1, 1, 1));
    }

    #[tokio::test]
    async fn manual_admission_serializes_liveness_and_registration() {
        let dir = tempfile::tempdir().unwrap();
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(dir.path().join("manual.db"))
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(30));
        let first = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_with(options.clone())
            .await
            .unwrap();
        crate::session::spectral_schema::apply_schedule_fire_claims_schema(&first)
            .await
            .unwrap();
        let second = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_with(options)
            .await
            .unwrap();
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let spawn = |pool: sqlx::SqlitePool, barrier: Arc<tokio::sync::Barrier>| {
            tokio::spawn(async move {
                let owner = ClaimOwner {
                    pid: claim_owner().pid + 1,
                    token: uuid::Uuid::new_v4().to_string(),
                };
                let key = format!("manual:{}", uuid::Uuid::new_v4());
                barrier.wait().await;
                try_claim_manual(&pool, "manual-race", &key, &owner, Utc::now(), |_| true).await
            })
        };
        let a = spawn(first, barrier.clone());
        let b = spawn(second, barrier);
        let results = [a.await.unwrap(), b.await.unwrap()];
        assert_eq!(
            results
                .iter()
                .filter(|outcome| **outcome == ClaimOutcome::Won)
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|outcome| **outcome == ClaimOutcome::Lost)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn dead_unstarted_owner_is_taken_over_once_by_racing_production_callbacks() {
        let dir = tempfile::tempdir().unwrap();
        let first = scheduler_at(dir.path()).await;
        let second = scheduler_at(dir.path()).await;
        let job = weekly_job("takeover-race");
        install_job(&first, job.clone()).await;
        install_job(&second, job.clone()).await;
        let pool = first.session_manager.pool_clone().await.unwrap();
        let other_pool = second.session_manager.pool_clone().await.unwrap();
        let due = instant("2026-09-21T12:00:00Z");
        let key = key_for_tick(&job, due).0;
        insert_competitor(&pool, &job.id, &key).await;
        sqlx::query("UPDATE schedule_fire_claims SET owner_pid = -1")
            .execute(&pool)
            .await
            .unwrap();
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let spawn =
            |scheduler: Arc<Scheduler>, job: ScheduledJob, barrier: Arc<tokio::sync::Barrier>| {
                tokio::spawn(async move {
                    barrier.wait().await;
                    scheduler.task_for_job(job)(due).await;
                })
            };
        let a = spawn(first.clone(), job.clone(), barrier.clone());
        let b = spawn(second.clone(), job.clone(), barrier);
        a.await.unwrap();
        b.await.unwrap();
        let first_runs = first.jobs.lock().await.get(&job.id).unwrap().1.run_count;
        let second_runs = second.jobs.lock().await.get(&job.id).unwrap().1.run_count;
        assert_eq!(first_runs + second_runs, 1);
        let row: (String, bool, bool) = sqlx::query_as(
            "SELECT owner_token, started_at IS NOT NULL, finished_at IS NOT NULL
             FROM schedule_fire_claims WHERE job_id = ? AND fire_key = ?",
        )
        .bind(&job.id)
        .bind(&key)
        .fetch_one(&other_pool)
        .await
        .unwrap();
        assert_eq!(row, (claim_owner().token.clone(), true, true));
        assert_eq!(
            try_claim_fire(&other_pool, &job.id, &key, "cron", claim_owner()).await,
            ClaimOutcome::Lost
        );
    }

    #[tokio::test]
    async fn separate_owner_takeover_race_has_one_winner_and_one_run() {
        let dir = tempfile::tempdir().unwrap();
        let scheduler = scheduler_at(dir.path()).await;
        let job = weekly_job("takeover-owners");
        install_job(&scheduler, job.clone()).await;
        let first = scheduler.session_manager.pool_clone().await.unwrap();
        let other_manager = SessionManager::new(dir.path().to_path_buf());
        let second = other_manager.pool_clone().await.unwrap();
        let key = key_for_catch_up(&job).unwrap();
        insert_competitor(&first, &job.id, &key).await;
        sqlx::query("UPDATE schedule_fire_claims SET owner_pid = -1")
            .execute(&first)
            .await
            .unwrap();
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let spawn = |pool: sqlx::SqlitePool, pid: i64| {
            let barrier = barrier.clone();
            let job_id = job.id.clone();
            let fire_key = key.clone();
            tokio::spawn(async move {
                let owner = ClaimOwner {
                    pid,
                    token: uuid::Uuid::new_v4().to_string(),
                };
                barrier.wait().await;
                let outcome = try_claim_fire_with_probe(
                    &pool,
                    &job_id,
                    &fire_key,
                    "catch_up",
                    &owner,
                    Utc::now(),
                    |pid| pid != -1,
                )
                .await;
                (
                    outcome,
                    FireClaim {
                        pool,
                        job_id,
                        fire_key,
                        owner,
                    },
                )
            })
        };
        let a = spawn(first, 101);
        let b = spawn(second, 102);
        let results = [a.await.unwrap(), b.await.unwrap()];
        assert_eq!(
            results
                .iter()
                .filter(|(outcome, _)| *outcome == ClaimOutcome::Won)
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|(outcome, _)| *outcome == ClaimOutcome::Lost)
                .count(),
            1
        );
        for (outcome, claim) in results {
            if outcome == ClaimOutcome::Won {
                scheduler
                    .run_claimed(&job.id, Some(claim), true)
                    .await
                    .unwrap();
            }
        }
        assert_eq!(
            scheduler
                .jobs
                .lock()
                .await
                .get(&job.id)
                .unwrap()
                .1
                .run_count,
            1
        );
    }

    #[tokio::test]
    async fn dead_started_and_live_unstarted_owners_never_run_again() {
        for started in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let scheduler = scheduler_at(dir.path()).await;
            let job = weekly_job("no-takeover");
            install_job(&scheduler, job.clone()).await;
            let pool = scheduler.session_manager.pool_clone().await.unwrap();
            let due = instant("2026-09-21T12:00:00Z");
            let key = key_for_tick(&job, due).0;
            insert_competitor(&pool, &job.id, &key).await;
            if started {
                sqlx::query("UPDATE schedule_fire_claims SET owner_pid = -1, started_at = ?")
                    .bind(due.to_rfc3339())
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            let before = snapshot(&scheduler, &job.id).await;
            let other = ClaimOwner {
                pid: -2,
                token: uuid::Uuid::new_v4().to_string(),
            };
            let alive: fn(i64) -> bool = if started { |_| false } else { |_| true };
            assert_eq!(
                try_claim_fire_with_probe(&pool, &job.id, &key, "cron", &other, due, alive).await,
                ClaimOutcome::Lost
            );
            assert_eq!(
                try_claim_fire(&pool, &job.id, &key, "cron", claim_owner()).await,
                ClaimOutcome::Lost
            );
            scheduler.task_for_job(job.clone())(due).await;
            assert_eq!(snapshot(&scheduler, &job.id).await, before);
            let unfinished: bool =
                sqlx::query_scalar("SELECT finished_at IS NULL FROM schedule_fire_claims")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert!(unfinished);
        }
    }

    #[tokio::test]
    async fn catch_up_stays_unstarted_until_execution_and_stale_owner_cannot_start() {
        let dir = tempfile::tempdir().unwrap();
        let scheduler = scheduler_at(dir.path()).await;
        let job = weekly_job("stagger-start");
        install_job(&scheduler, job.clone()).await;
        let claim = scheduler.claim_catch_up(&job).await.unwrap().unwrap();
        let unstarted: bool =
            sqlx::query_scalar("SELECT started_at IS NULL FROM schedule_fire_claims")
                .fetch_one(&claim.pool)
                .await
                .unwrap();
        assert!(unstarted);
        let stale = FireClaim {
            owner: ClaimOwner {
                pid: -1,
                token: uuid::Uuid::new_v4().to_string(),
            },
            ..claim.clone()
        };
        assert!(stale.start().await.is_err());
        scheduler
            .run_claimed(&job.id, Some(claim.clone()), true)
            .await
            .unwrap();
        let stamps: (bool, bool) = sqlx::query_as(
            "SELECT started_at IS NOT NULL, finished_at IS NOT NULL FROM schedule_fire_claims",
        )
        .fetch_one(&claim.pool)
        .await
        .unwrap();
        assert_eq!(stamps, (true, true));
        assert_eq!(
            scheduler
                .jobs
                .lock()
                .await
                .get(&job.id)
                .unwrap()
                .1
                .run_count,
            1
        );
    }

    #[tokio::test]
    async fn daemon_startup_retires_started_claims_and_preserves_unstarted_takeover() {
        let dir = tempfile::tempdir().unwrap();
        let manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
        let pool = manager.pool_clone().await.unwrap();
        for key in ["dead", "recycled", "recycled-unstarted", "ours", "finished"] {
            insert_competitor(&pool, "startup", key).await;
        }
        sqlx::query("UPDATE schedule_fire_claims SET owner_pid = -1 WHERE fire_key = 'dead'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE schedule_fire_claims SET owner_token = ? WHERE fire_key = 'ours'")
            .bind(&claim_owner().token)
            .execute(&pool)
            .await
            .unwrap();
        let job = weekly_job("startup-takeover");
        let key = key_for_catch_up(&job).unwrap();
        insert_competitor(&pool, &job.id, &key).await;
        sqlx::query("UPDATE schedule_fire_claims SET owner_pid = -1 WHERE job_id = ?")
            .bind(&job.id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE schedule_fire_claims SET started_at = ? WHERE fire_key IN ('dead', 'recycled', 'ours')")
            .bind(Utc::now().to_rfc3339()).execute(&pool).await.unwrap();
        let old_finish = instant("2026-09-21T12:00:00Z").to_rfc3339();
        sqlx::query("UPDATE schedule_fire_claims SET finished_at = ? WHERE fire_key = 'finished'")
            .bind(&old_finish)
            .execute(&pool)
            .await
            .unwrap();
        let inspection =
            Scheduler::new_without_cron(dir.path().join("schedule.json"), manager.clone())
                .await
                .unwrap();
        let unfinished: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM schedule_fire_claims WHERE finished_at IS NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(unfinished, 5);
        drop(inspection);
        let daemon = Scheduler::build(
            dir.path().join("schedule.json"),
            manager,
            StorageLoadMode::ReconcileAndCatchUp,
            false,
        )
        .await
        .unwrap();
        let rows: Vec<(String, bool)> = sqlx::query_as(
            "SELECT fire_key, finished_at IS NOT NULL FROM schedule_fire_claims WHERE job_id = 'startup' ORDER BY fire_key",
        ).fetch_all(&pool).await.unwrap();
        assert_eq!(
            rows,
            vec![
                ("dead".to_string(), true),
                ("finished".to_string(), true),
                ("ours".to_string(), false),
                ("recycled".to_string(), true),
                ("recycled-unstarted".to_string(), false)
            ]
        );
        let surviving: (i64, bool, bool) = sqlx::query_as(
            "SELECT owner_pid, started_at IS NULL, finished_at IS NULL
             FROM schedule_fire_claims WHERE job_id = ? AND fire_key = ?",
        )
        .bind(&job.id)
        .bind(&key)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(surviving, (-1, true, true));
        install_job(&daemon, job.clone()).await;
        let claim = daemon.claim_catch_up(&job).await.unwrap().unwrap();
        daemon
            .run_claimed(&job.id, Some(claim), true)
            .await
            .unwrap();
        assert!(daemon.claim_catch_up(&job).await.is_none());
        assert_eq!(
            daemon.jobs.lock().await.get(&job.id).unwrap().1.run_count,
            1
        );
        let taken_over: (String, bool, bool) = sqlx::query_as(
            "SELECT owner_token, started_at IS NOT NULL, finished_at IS NOT NULL
             FROM schedule_fire_claims WHERE job_id = ? AND fire_key = ?",
        )
        .bind(&job.id)
        .bind(&key)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(taken_over, (claim_owner().token.clone(), true, true));
        let preserved: String = sqlx::query_scalar(
            "SELECT finished_at FROM schedule_fire_claims WHERE fire_key = 'finished'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(preserved, old_finish);
        // Probe seam: another live PID survives; sweep errors cannot fail build.
        insert_competitor(&pool, "startup", "live").await;
        sqlx::query(
            "UPDATE schedule_fire_claims SET owner_pid = ?, started_at = ? WHERE fire_key = 'live'",
        )
        .bind(claim_owner().pid + 1)
        .bind(Utc::now().to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();
        daemon
            .sweep_claims_at_startup(StorageLoadMode::ReconcileAndCatchUp, Utc::now(), |_| true)
            .await;
        let live: bool = sqlx::query_scalar(
            "SELECT finished_at IS NULL FROM schedule_fire_claims WHERE fire_key = 'live'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(live);
        sqlx::query("DROP TABLE schedule_fire_claims")
            .execute(&pool)
            .await
            .unwrap();
        Scheduler::build(
            dir.path().join("schedule.json"),
            daemon.session_manager.clone(),
            StorageLoadMode::ReconcileAndCatchUp,
            false,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn seven_day_wedge_detector_unblocks_manual_even_when_pid_is_alive() {
        let dir = tempfile::tempdir().unwrap();
        let scheduler = scheduler_at(dir.path()).await;
        let job = weekly_job("wedge");
        install_job(&scheduler, job.clone()).await;
        let pool = scheduler.session_manager.pool_clone().await.unwrap();
        let key = key_for_catch_up(&job).unwrap();
        insert_competitor(&pool, &job.id, &key).await;
        let claimed = Utc::now();
        sqlx::query("UPDATE schedule_fire_claims SET claimed_at = ?, started_at = ?")
            .bind(claimed.to_rfc3339())
            .bind(claimed.to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();
        let boundary = claimed + CLAIM_WEDGE_AGE;
        assert!(scheduler
            .run_now_with_liveness(&job.id, boundary, |_| true)
            .await
            .is_err());
        assert_eq!(
            scheduler
                .jobs
                .lock()
                .await
                .get(&job.id)
                .unwrap()
                .1
                .run_count,
            0
        );
        let now = boundary + Duration::seconds(1);
        scheduler
            .run_now_with_liveness(&job.id, now, |_| true)
            .await
            .unwrap();
        let finished: String = sqlx::query_scalar(
            "SELECT finished_at FROM schedule_fire_claims WHERE job_id = ? AND fire_key = ?",
        )
        .bind(&job.id)
        .bind(&key)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(finished, now.to_rfc3339());
        assert_eq!(
            scheduler
                .jobs
                .lock()
                .await
                .get(&job.id)
                .unwrap()
                .1
                .run_count,
            1
        );
        let stamps: (bool, bool) = sqlx::query_as(
            "SELECT started_at IS NOT NULL, finished_at IS NOT NULL FROM schedule_fire_claims WHERE kind = 'manual'",
        ).fetch_one(&pool).await.unwrap();
        assert_eq!(stamps, (true, true));
    }

    #[tokio::test]
    async fn retention_deletes_old_finished_and_dead_unstarted_claims_only() {
        let dir = tempfile::tempdir().unwrap();
        let scheduler = scheduler_at(dir.path()).await;
        let pool = scheduler.session_manager.pool_clone().await.unwrap();
        let old = (Utc::now() - Duration::days(8)).to_rfc3339();
        for key in [
            "interval:0",
            "interval:900",
            "interval:1800",
            "interval:3600",
            "interval:4500",
            "interval:5400",
            "interval:6300",
        ] {
            insert_competitor(&pool, "retention", key).await;
        }
        sqlx::query("UPDATE schedule_fire_claims SET claimed_at = ?")
            .bind(&old)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE schedule_fire_claims SET finished_at = ? WHERE fire_key = ?")
            .bind(&old)
            .bind("interval:0")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE schedule_fire_claims SET finished_at = ? WHERE fire_key = ?")
            .bind(Utc::now().to_rfc3339())
            .bind("interval:900")
            .execute(&pool)
            .await
            .unwrap();
        let now = Utc::now();
        sqlx::query("UPDATE schedule_fire_claims SET owner_pid = -1 WHERE fire_key IN ('interval:3600', 'interval:4500', 'interval:5400', 'interval:6300')")
            .execute(&pool).await.unwrap();
        sqlx::query(
            "UPDATE schedule_fire_claims SET started_at = ? WHERE fire_key = 'interval:4500'",
        )
        .bind(&old)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE schedule_fire_claims SET claimed_at = ? WHERE fire_key = 'interval:5400'",
        )
        .bind(now.to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE schedule_fire_claims SET claimed_at = ? WHERE fire_key = 'interval:6300'",
        )
        .bind((now - Duration::days(7)).to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            try_claim_fire_with_probe(
                &pool,
                "retention",
                "interval:2700",
                "interval",
                claim_owner(),
                now,
                |pid| pid != -1
            )
            .await,
            ClaimOutcome::Won
        );
        let keys: Vec<String> =
            sqlx::query_scalar("SELECT fire_key FROM schedule_fire_claims ORDER BY fire_key")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            keys,
            vec![
                "interval:1800",
                "interval:2700",
                "interval:4500",
                "interval:5400",
                "interval:6300",
                "interval:900"
            ]
        );
    }

    #[tokio::test]
    async fn stop_paths_finish_only_the_claim_of_the_stopped_run() {
        for live_token in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let scheduler = scheduler_at(dir.path()).await;
            let mut job = weekly_job("stopped");
            job.currently_running = true;
            job.process_start_time = Some(Utc::now());
            install_job(&scheduler, job.clone()).await;
            let claim = admit_fire(
                &scheduler.session_manager,
                &job.id,
                key_for_catch_up(&job).unwrap(),
                "catch_up",
            )
            .await
            .unwrap()
            .unwrap();
            scheduler
                .active_claims
                .lock()
                .await
                .insert(job.id.clone(), claim.clone());
            let pool = scheduler.session_manager.pool_clone().await.unwrap();
            assert_eq!(
                try_claim_fire(&pool, &job.id, "interval:0", "interval", claim_owner()).await,
                ClaimOutcome::Won
            );
            if live_token {
                scheduler
                    .running_tasks
                    .lock()
                    .await
                    .insert(job.id.clone(), CancellationToken::new());
            }
            scheduler.kill_running_job(&job.id).await.unwrap();
            // The watchdog uses the existing shortened cfg(test) grace.
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let finished: bool = sqlx::query_scalar(
                        "SELECT finished_at IS NOT NULL FROM schedule_fire_claims WHERE job_id = ? AND fire_key = ?",
                    ).bind(&job.id).bind(&claim.fire_key).fetch_one(&pool).await.unwrap();
                    if finished { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }).await.unwrap();
            let unrelated_finished: bool = sqlx::query_scalar(
                "SELECT finished_at IS NOT NULL FROM schedule_fire_claims WHERE job_id = ? AND fire_key = ?",
            ).bind(&job.id).bind("interval:0").fetch_one(&pool).await.unwrap();
            assert!(!unrelated_finished);
            assert_eq!(
                scheduler
                    .jobs
                    .lock()
                    .await
                    .get(&job.id)
                    .unwrap()
                    .1
                    .last_status,
                Some(ScheduleRunStatus::Error)
            );
        }
    }
    async fn s2n_fixture(dir: &Path) -> (Arc<Scheduler>, ScheduledJob, sqlx::SqlitePool) {
        let scheduler = scheduler_at(dir).await;
        let mut job = weekly_job("s2n-pause");
        job.source = dir
            .join("missing-recipe.yaml")
            .to_string_lossy()
            .into_owned();
        install_job(&scheduler, job.clone()).await;
        let pool = scheduler.session_manager.pool_clone().await.unwrap();
        (scheduler, job, pool)
    }

    async fn s2n_fail_three(scheduler: &Scheduler, job: &ScheduledJob) -> SchedulePauseEpisode {
        for _ in 0..3 {
            assert!(scheduler.run_now(&job.id).await.is_err());
        }
        scheduler
            .jobs
            .lock()
            .await
            .get(&job.id)
            .unwrap()
            .1
            .pause_episode
            .clone()
            .unwrap()
    }

    async fn s2n_request_count(pool: &sqlx::SqlitePool) -> i64 {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM schedule_pause_episodes
            WHERE EXISTS (SELECT 1 FROM decisions WHERE id = decision_id)",
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn acceptance_01_zero_retries_third_failure_persists_one_valid_decision() {
        let dir = tempfile::tempdir().unwrap();
        let (scheduler, job, pool) = s2n_fixture(dir.path()).await;
        for count in 1..=3 {
            scheduler.task_for_job(job.clone())(
                instant("2026-09-21T12:00:00Z") + Duration::weeks(count),
            )
            .await;
            assert_eq!(
                s2n_request_count(&pool).await,
                if count == 3 { 1 } else { 0 }
            );
        }
        let saved: Vec<ScheduledJob> =
            serde_json::from_str(&fs::read_to_string(&scheduler.storage_path).unwrap()).unwrap();
        let paused = saved.iter().find(|j| j.id == job.id).unwrap();
        assert!(paused.paused);
        let episode = paused.pause_episode.as_ref().unwrap();
        let decision = crate::decisions::get_decision(&pool, &episode.decision_id)
            .await
            .unwrap()
            .unwrap();
        let payload: crate::decisions::ScheduledJobUnblockPayload =
            serde_json::from_value(decision.payload).unwrap();
        payload.validate().unwrap();
        assert_eq!(payload.last_error, paused.last_error.clone().unwrap());
        assert_eq!(payload.failed_runs.len(), 3);
        assert!(payload
            .failed_runs
            .iter()
            .all(|run| run.session_id.is_none()));
        assert_eq!(decision.detail, crate::decisions::AUTO_PAUSE_BODY);
    }

    #[tokio::test]
    async fn acceptance_02_cron_catch_up_manual_count_terminal_fires() {
        for path in 0..3 {
            let dir = tempfile::tempdir().unwrap();
            let (scheduler, job, pool) = s2n_fixture(dir.path()).await;
            for count in 1..=3 {
                match path {
                    0 => {
                        scheduler.task_for_job(job.clone())(
                            instant("2026-09-21T12:00:00Z") + Duration::weeks(count),
                        )
                        .await
                    }
                    1 => {
                        assert!(scheduler.run_claimed(&job.id, None, true).await.is_err());
                    }
                    _ => {
                        assert!(scheduler.run_now(&job.id).await.is_err());
                    }
                }
            }
            let jobs = scheduler.jobs.lock().await;
            let finished = &jobs.get(&job.id).unwrap().1;
            assert_eq!(finished.consecutive_failures, 3);
            assert_eq!(finished.run_count, 3);
            assert_eq!(s2n_request_count(&pool).await, 1);
        }
    }

    #[tokio::test]
    async fn acceptance_03_success_resets_streak_already_paused_keeps_episode() {
        let dir = tempfile::tempdir().unwrap();
        let (scheduler, job, pool) = s2n_fixture(dir.path()).await;
        for _ in 0..2 {
            assert!(scheduler.run_now(&job.id).await.is_err());
        }
        scheduler
            .jobs
            .lock()
            .await
            .get_mut(&job.id)
            .unwrap()
            .1
            .source
            .clear();
        scheduler.run_now(&job.id).await.unwrap();
        assert_eq!(
            scheduler
                .jobs
                .lock()
                .await
                .get(&job.id)
                .unwrap()
                .1
                .consecutive_failures,
            0
        );
        scheduler
            .jobs
            .lock()
            .await
            .get_mut(&job.id)
            .unwrap()
            .1
            .source = job.source.clone();
        let episode = s2n_fail_three(&scheduler, &job).await;
        assert!(scheduler.run_now(&job.id).await.is_err());
        assert_eq!(
            scheduler
                .jobs
                .lock()
                .await
                .get(&job.id)
                .unwrap()
                .1
                .pause_episode
                .as_ref()
                .unwrap()
                .id,
            episode.id
        );
        assert_eq!(s2n_request_count(&pool).await, 1);
    }

    #[tokio::test]
    async fn acceptance_04_retry_escalation_is_promoted_with_three_fire_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let (scheduler, job, pool) = s2n_fixture(dir.path()).await;
        // Producer fixture copied from escalation_creates_unblock_decision.
        escalate_persistent_failure(
            &scheduler.session_manager,
            &job.id,
            3,
            3,
            "provider timeout",
        )
        .await;
        let prior: String = sqlx::query_scalar("SELECT id FROM decisions WHERE kind = 'unblock'")
            .fetch_one(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO decisions (id, kind, tier, headline, detail, payload_json, created_at)
            SELECT 'legacy-extra', kind, tier, headline, detail, payload_json, '9999' FROM decisions WHERE id = ?")
            .bind(&prior).execute(&pool).await.unwrap();
        s2n_fail_three(&scheduler, &job).await;
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM decisions WHERE kind = 'unblock' AND status = 'open'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1);
        let decision = crate::decisions::get_decision(&pool, &prior)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            decision.payload["reason"],
            crate::decisions::CONSECUTIVE_FAILURE_LIMIT
        );
        assert_eq!(decision.payload["failed_runs"].as_array().unwrap().len(), 3);
        let duplicate = crate::decisions::get_decision(&pool, "legacy-extra")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(duplicate.status, "superseded"); // reconciliation UPDATE
        assert!(duplicate.resolved_at.is_some());
        let outcome: String = sqlx::query_scalar(
            "SELECT outcome FROM decision_audit WHERE decision_id = 'legacy-extra'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(outcome, "superseded");
        scheduler.reconcile_pause_decisions().await.unwrap();
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM decisions WHERE kind = 'unblock' AND status = 'open'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1);
        // The promoted row, rather than the provisional episode id, is authoritative.
        let episode = scheduler
            .jobs
            .lock()
            .await
            .get(&job.id)
            .unwrap()
            .1
            .pause_episode
            .clone()
            .unwrap();
        scheduled_job_resume(
            scheduler.as_ref(),
            &pool,
            &job.id,
            &episode.id,
            SchedulePauseAnswer {
                decision_id: &prior,
                answer: "approve",
                acted_by: crate::decisions::ACTOR_JESSE,
                principal: "master",
                from_automate: false,
            },
        )
        .await
        .unwrap();
        assert!(!scheduler.jobs.lock().await.get(&job.id).unwrap().1.paused);
    }

    #[tokio::test]
    async fn acceptance_05_concurrent_reconciliation_and_restart_deduplicate() {
        let dir = tempfile::tempdir().unwrap();
        let (scheduler, job, pool) = s2n_fixture(dir.path()).await;
        s2n_fail_three(&scheduler, &job).await;
        let (a, b) = tokio::join!(
            scheduler.reconcile_pause_decisions(),
            scheduler.reconcile_pause_decisions()
        );
        a.unwrap();
        b.unwrap();
        let reopened = scheduler_at(dir.path()).await;
        reopened.reconcile_pause_decisions().await.unwrap();
        assert_eq!(s2n_request_count(&pool).await, 1);
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM decisions WHERE kind = 'unblock'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn acceptance_06_delivery_and_resume_persistence_failures_remain_pending() {
        let dir = tempfile::tempdir().unwrap();
        let (scheduler, job, pool) = s2n_fixture(dir.path()).await;
        sqlx::query("CREATE TRIGGER s2n_reject_decision BEFORE INSERT ON decisions BEGIN SELECT RAISE(ABORT, 'test failure'); END")
            .execute(&pool).await.unwrap();
        let episode = s2n_fail_three(&scheduler, &job).await;
        assert!(scheduler.jobs.lock().await.get(&job.id).unwrap().1.paused);
        assert_eq!(s2n_request_count(&pool).await, 0);
        sqlx::query("DROP TRIGGER s2n_reject_decision")
            .execute(&pool)
            .await
            .unwrap();
        scheduler.reconcile_pause_decisions().await.unwrap();
        // A directory at the temporary-file path deterministically rejects writes.
        let tmp = scheduler.storage_path.with_file_name("schedule.json.tmp");
        fs::create_dir(&tmp).unwrap();
        assert!(scheduled_job_resume(
            scheduler.as_ref(),
            &pool,
            &job.id,
            &episode.id,
            SchedulePauseAnswer {
                decision_id: &episode.decision_id,
                answer: "approve",
                acted_by: crate::decisions::ACTOR_JESSE,
                principal: "master",
                from_automate: false
            }
        )
        .await
        .is_err());
        assert!(scheduler.jobs.lock().await.get(&job.id).unwrap().1.paused);
        let decision = crate::decisions::get_decision(&pool, &episode.decision_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(decision.status, "open"); // decisions schema default / reconciliation query
        fs::remove_dir(tmp).unwrap();
        scheduled_job_resume(
            scheduler.as_ref(),
            &pool,
            &job.id,
            &episode.id,
            SchedulePauseAnswer {
                decision_id: &episode.decision_id,
                answer: "approve",
                acted_by: crate::decisions::ACTOR_JESSE,
                principal: "master",
                from_automate: false,
            },
        )
        .await
        .unwrap();
        assert!(!scheduler.jobs.lock().await.get(&job.id).unwrap().1.paused);
    }

    #[tokio::test]
    async fn correction_persist_failure_defers_delivery_in_both_paths_and_restart() {
        for cron in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let (scheduler, job, pool) = s2n_fixture(dir.path()).await;
            for _ in 0..2 {
                assert!(scheduler.run_now(&job.id).await.is_err());
            }
            FAIL_PAUSE_PERSIST
                .scope(true, async {
                    if cron {
                        scheduler.task_for_job(job.clone())(instant("2026-09-21T12:00:00Z")).await;
                    } else {
                        assert!(scheduler.run_now(&job.id).await.is_err());
                    }
                })
                .await;
            assert!(scheduler.jobs.lock().await.get(&job.id).unwrap().1.paused);
            scheduler.list_scheduled_jobs().await;
            assert_eq!(s2n_request_count(&pool).await, 0);
            let decisions: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM decisions WHERE kind = 'unblock'")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(decisions, 0);
            let pending: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM schedule_pause_episodes
                WHERE notification_json IS NULL AND answer IS NULL",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(pending, 1);
            if cron {
                // Startup recovers the pending pause and persists it before delivery.
                let reopened = Scheduler::build(
                    scheduler.storage_path.clone(),
                    scheduler.session_manager.clone(),
                    StorageLoadMode::ReconcileAndCatchUp,
                    false,
                )
                .await
                .unwrap();
                reopened.reconcile_pause_decisions().await.unwrap();
                assert!(reopened.jobs.lock().await.get(&job.id).unwrap().1.paused);
            } else {
                persist_jobs(&scheduler.storage_path, &scheduler.jobs)
                    .await
                    .unwrap();
                scheduler.reconcile_pause_decisions().await.unwrap();
            }
            scheduler.reconcile_pause_decisions().await.unwrap();
            assert_eq!(s2n_request_count(&pool).await, 1);
            let decisions: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM decisions WHERE kind = 'unblock'")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(decisions, 1);
        }
    }

    #[tokio::test]
    async fn correction_forged_decision_cannot_resume_or_resolve_another_episode() {
        let dir = tempfile::tempdir().unwrap();
        let (scheduler, job, pool) = s2n_fixture(dir.path()).await;
        let episode = s2n_fail_three(&scheduler, &job).await;
        let before = scheduler.jobs.lock().await.get(&job.id).unwrap().1.clone();
        sqlx::query("INSERT INTO decisions (id, kind, tier, headline, detail, payload_json, created_at)
            SELECT 'D_fake', kind, tier, headline, detail, payload_json, created_at FROM decisions WHERE id = ?")
            .bind(&episode.decision_id).execute(&pool).await.unwrap();
        let result = scheduled_job_resume(
            scheduler.as_ref(),
            &pool,
            &job.id,
            &episode.id,
            SchedulePauseAnswer {
                decision_id: "D_fake",
                answer: "approve",
                acted_by: crate::decisions::ACTOR_JESSE,
                principal: "master",
                from_automate: false,
            },
        )
        .await;
        assert!(matches!(
            result,
            Err(SchedulerError::AnswerRefused(
                crate::decisions::AnswerError::Forbidden(_)
            ))
        ));
        let after = scheduler.jobs.lock().await.get(&job.id).unwrap().1.clone();
        assert!(after.paused);
        assert_eq!(after.run_count, before.run_count);
        assert_eq!(after.consecutive_failures, before.consecutive_failures);
        assert_eq!(after.retry_count, before.retry_count);
        assert!(after.pause_episode.as_ref().unwrap().effect.is_none());
        for id in ["D_fake", episode.decision_id.as_str()] {
            let decision = crate::decisions::get_decision(&pool, id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(decision.status, "open"); // decisions schema default
        }
        let untouched: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM schedule_pause_episodes
            WHERE answer IS NULL AND resolved_at IS NULL AND effect_json IS NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(untouched, 1);
        let outcome: String =
            sqlx::query_scalar("SELECT outcome FROM decision_audit WHERE decision_id = 'D_fake'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(outcome, "refused"); // audit_schedule_refusal
        assert!(
            crate::decisions::verify_audit_chain(&pool)
                .await
                .unwrap()
                .intact
        );
    }

    #[tokio::test]
    async fn correction_schedule_answer_uses_user_only_policy_gate() {
        let dir = tempfile::tempdir().unwrap();
        let (scheduler, job, pool) = s2n_fixture(dir.path()).await;
        let episode = s2n_fail_three(&scheduler, &job).await;
        // A downgraded stored decision cannot bypass the live action policy.
        sqlx::query("UPDATE decisions SET tier = 0 WHERE id = ?")
            .bind(&episode.decision_id)
            .execute(&pool)
            .await
            .unwrap();
        let answer = crate::decisions::DecisionAnswer {
            answer: "approve".to_string(),
            note: None,
            choice_id: None,
            input_text: None,
        };
        for (actor, principal) in [
            (crate::decisions::ACTOR_JESSE, ""),
            (crate::decisions::ACTOR_HENRY, "master"),
        ] {
            let ordinary = crate::decisions::answer_decision_with_principal(
                &pool,
                &episode.decision_id,
                &answer,
                actor,
                principal,
            )
            .await
            .unwrap_err();
            assert!(matches!(
                &ordinary,
                crate::decisions::AnswerError::Forbidden(_)
            ));
            let result = scheduled_job_resume(
                scheduler.as_ref(),
                &pool,
                &job.id,
                &episode.id,
                SchedulePauseAnswer {
                    decision_id: &episode.decision_id,
                    answer: "approve",
                    acted_by: actor,
                    principal,
                    from_automate: false,
                },
            )
            .await;
            match result {
                Err(SchedulerError::AnswerRefused(
                    error @ crate::decisions::AnswerError::Forbidden(_),
                )) => {
                    assert_eq!(error.to_string(), ordinary.to_string());
                }
                other => panic!("expected the shared answer refusal, got {other:?}"),
            }
        }
        // Lowering the risk-policy row cannot grant automation user authority.
        sqlx::query("UPDATE risk_policy SET tier = 0 WHERE action_class = ?")
            .bind(crate::decisions::AUTOMATION_RESUME_SCHEDULE)
            .execute(&pool)
            .await
            .unwrap();
        let decision = crate::decisions::get_decision(&pool, &episode.decision_id)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            crate::decisions::check_answer_authority(
                &pool,
                &decision,
                crate::decisions::ACTOR_HENRY,
                Some("master")
            )
            .await,
            Err(crate::decisions::AnswerError::Forbidden(_))
        ));
        assert_eq!(decision.status, "open");
        assert!(scheduler.jobs.lock().await.get(&job.id).unwrap().1.paused);
        let untouched: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM schedule_pause_episodes WHERE answer IS NULL AND resolved_at IS NULL")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(untouched, 1);
        let refusals: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM decision_audit WHERE decision_id = ? AND outcome = 'refused'",
        )
        .bind(&episode.decision_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(refusals, 2);
    }

    #[tokio::test]
    async fn acceptance_07_scheduled_unblock_cannot_use_goal_answer_path() {
        let dir = tempfile::tempdir().unwrap();
        let (scheduler, job, pool) = s2n_fixture(dir.path()).await;
        let episode = s2n_fail_three(&scheduler, &job).await;
        let answer = crate::decisions::DecisionAnswer {
            answer: "approve".to_string(),
            note: None,
            choice_id: None,
            input_text: None,
        };
        assert!(crate::decisions::answer_decision(
            &pool,
            &episode.decision_id,
            &answer,
            crate::decisions::ACTOR_JESSE
        )
        .await
        .is_err());
        let decision = crate::decisions::get_decision(&pool, &episode.decision_id)
            .await
            .unwrap()
            .unwrap();
        let parsed: crate::decisions::UnblockContract =
            serde_json::from_value(decision.payload).unwrap();
        match parsed {
            crate::decisions::UnblockContract::ScheduledJob(payload) => {
                payload.validate().unwrap();
                assert_eq!(payload.pause_episode_id, episode.id);
            }
            crate::decisions::UnblockContract::Goal(_) => {
                panic!("scheduled unblock decoded as a goal")
            }
        }
        let goal = crate::decisions::UnblockPayload {
            reason: crate::decisions::UnblockReason::AttemptCap,
            spent: Some(3),
            cap: Some(3),
        };
        let parsed: crate::decisions::UnblockContract =
            serde_json::from_value(serde_json::to_value(goal).unwrap()).unwrap();
        assert!(matches!(parsed, crate::decisions::UnblockContract::Goal(_)));
    }

    #[tokio::test]
    async fn acceptance_08_s2n_resume_persists_trimmed_note_in_decision_answer_note() {
        let dir = tempfile::tempdir().unwrap();
        let (scheduler, job, pool) = s2n_fixture(dir.path()).await;
        let episode = s2n_fail_three(&scheduler, &job).await;
        scheduled_job_resume_with_note(
            scheduler.as_ref(),
            &pool,
            &job.id,
            &episode.id,
            SchedulePauseAnswer {
                decision_id: &episode.decision_id,
                answer: "approve",
                acted_by: crate::decisions::ACTOR_JESSE,
                principal: "master",
                from_automate: false,
            },
            Some("  unblock after verifying upstream fix  "),
        )
        .await
        .unwrap();
        let note: Option<String> =
            sqlx::query_scalar("SELECT answer_note FROM decisions WHERE id = ?")
                .bind(&episode.decision_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            note.as_deref(),
            Some("unblock after verifying upstream fix")
        );
    }

    #[tokio::test]
    async fn acceptance_08b_s2n_resume_without_note_keeps_decision_answer_note_null() {
        let dir = tempfile::tempdir().unwrap();
        let (scheduler, job, pool) = s2n_fixture(dir.path()).await;
        let episode = s2n_fail_three(&scheduler, &job).await;
        scheduled_job_resume_with_note(
            scheduler.as_ref(),
            &pool,
            &job.id,
            &episode.id,
            SchedulePauseAnswer {
                decision_id: &episode.decision_id,
                answer: "approve",
                acted_by: crate::decisions::ACTOR_JESSE,
                principal: "master",
                from_automate: false,
            },
            Some("   "),
        )
        .await
        .unwrap();
        let note: Option<String> =
            sqlx::query_scalar("SELECT answer_note FROM decisions WHERE id = ?")
                .bind(&episode.decision_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(note.is_none());
    }

    #[tokio::test]
    async fn acceptance_08c_s2n_keep_paused_persists_note() {
        let dir = tempfile::tempdir().unwrap();
        let (scheduler, job, pool) = s2n_fixture(dir.path()).await;
        let episode = s2n_fail_three(&scheduler, &job).await;
        scheduled_job_resume_with_note(
            scheduler.as_ref(),
            &pool,
            &job.id,
            &episode.id,
            SchedulePauseAnswer {
                decision_id: &episode.decision_id,
                answer: "reject",
                acted_by: crate::decisions::ACTOR_JESSE,
                principal: "master",
                from_automate: false,
            },
            Some("  keep paused while monitoring downstream alerts  "),
        )
        .await
        .unwrap();
        let note: Option<String> =
            sqlx::query_scalar("SELECT answer_note FROM decisions WHERE id = ?")
                .bind(&episode.decision_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            note.as_deref(),
            Some("keep paused while monitoring downstream alerts")
        );
        assert!(scheduler.jobs.lock().await.get(&job.id).unwrap().1.paused);
    }

    #[tokio::test]
    async fn acceptance_08d_s2n_retry_after_recorded_answer_keeps_first_note() {
        let dir = tempfile::tempdir().unwrap();
        let (scheduler, job, pool) = s2n_fixture(dir.path()).await;
        let episode = s2n_fail_three(&scheduler, &job).await;
        scheduled_job_resume_with_note(
            scheduler.as_ref(),
            &pool,
            &job.id,
            &episode.id,
            SchedulePauseAnswer {
                decision_id: &episode.decision_id,
                answer: "approve",
                acted_by: crate::decisions::ACTOR_JESSE,
                principal: "master",
                from_automate: false,
            },
            Some("first operator note"),
        )
        .await
        .unwrap();
        scheduled_job_resume_with_note(
            scheduler.as_ref(),
            &pool,
            &job.id,
            &episode.id,
            SchedulePauseAnswer {
                decision_id: &episode.decision_id,
                answer: "approve",
                acted_by: crate::decisions::ACTOR_JESSE,
                principal: "master",
                from_automate: false,
            },
            Some("second operator note"),
        )
        .await
        .unwrap();
        let note: Option<String> =
            sqlx::query_scalar("SELECT answer_note FROM decisions WHERE id = ?")
                .bind(&episode.decision_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(note.as_deref(), Some("first operator note"));
    }

    #[tokio::test]
    async fn acceptance_09_resume_resets_audits_preserves_evidence_without_catch_up() {
        let dir = tempfile::tempdir().unwrap();
        let (scheduler, job, pool) = s2n_fixture(dir.path()).await;
        let episode = s2n_fail_three(&scheduler, &job).await;
        let before = scheduler.jobs.lock().await.get(&job.id).unwrap().1.clone();
        scheduled_job_resume(
            scheduler.as_ref(),
            &pool,
            &job.id,
            &episode.id,
            SchedulePauseAnswer {
                decision_id: &episode.decision_id,
                answer: "approve",
                acted_by: crate::decisions::ACTOR_JESSE,
                principal: "master",
                from_automate: false,
            },
        )
        .await
        .unwrap();
        let after = scheduler.jobs.lock().await.get(&job.id).unwrap().1.clone();
        assert!(!after.paused);
        assert_eq!(after.consecutive_failures, 0);
        assert_eq!(after.retry_count, 0);
        assert_eq!(after.run_count, before.run_count);
        assert_eq!(after.last_error, before.last_error);
        assert_eq!(after.failed_runs.len(), before.failed_runs.len());
        assert!(!is_run_missed(&after, after.resumed_at.unwrap()));
        assert!(missed_fire_at(&after).unwrap() > after.resumed_at.unwrap());
        let effect: String = sqlx::query_scalar(
            "SELECT effect_json FROM schedule_pause_episodes WHERE pause_episode_id = ?",
        )
        .bind(&episode.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let effect: serde_json::Value = serde_json::from_str(&effect).unwrap();
        assert_eq!(effect["actor"], crate::decisions::ACTOR_JESSE);
        assert_eq!(effect["decision_id"], episode.decision_id);
        assert_eq!(effect["before"]["paused"], true);
        assert_eq!(effect["after"]["paused"], false);
    }

    #[tokio::test]
    async fn acceptance_10_keep_paused_once_then_automate_resumes_same_request() {
        let dir = tempfile::tempdir().unwrap();
        let (scheduler, job, pool) = s2n_fixture(dir.path()).await;
        let episode = s2n_fail_three(&scheduler, &job).await;
        for _ in 0..2 {
            scheduled_job_resume(
                scheduler.as_ref(),
                &pool,
                &job.id,
                &episode.id,
                SchedulePauseAnswer {
                    decision_id: &episode.decision_id,
                    answer: "reject",
                    acted_by: crate::decisions::ACTOR_JESSE,
                    principal: "master",
                    from_automate: false,
                },
            )
            .await
            .unwrap();
        }
        assert!(scheduler.jobs.lock().await.get(&job.id).unwrap().1.paused);
        let audits: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM decision_audit WHERE decision_id = ? AND outcome = 'reject'",
        )
        .bind(&episode.decision_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(audits, 1);
        scheduled_job_resume(
            scheduler.as_ref(),
            &pool,
            &job.id,
            &episode.id,
            SchedulePauseAnswer {
                decision_id: &episode.decision_id,
                answer: "approve",
                acted_by: crate::decisions::ACTOR_JESSE,
                principal: "master",
                from_automate: true,
            },
        )
        .await
        .unwrap();
        assert!(!scheduler.jobs.lock().await.get(&job.id).unwrap().1.paused);
        assert_eq!(s2n_request_count(&pool).await, 1);
    }

    #[tokio::test]
    async fn acceptance_11_duplicate_stale_deleted_and_unauthorized_answers() {
        let dir = tempfile::tempdir().unwrap();
        let (scheduler, job, pool) = s2n_fixture(dir.path()).await;
        let first = s2n_fail_three(&scheduler, &job).await;
        assert!(scheduled_job_resume(
            scheduler.as_ref(),
            &pool,
            &job.id,
            &first.id,
            SchedulePauseAnswer {
                decision_id: &first.decision_id,
                answer: "approve",
                acted_by: crate::decisions::ACTOR_JESSE,
                principal: "",
                from_automate: false
            }
        )
        .await
        .is_err());
        for _ in 0..2 {
            scheduled_job_resume(
                scheduler.as_ref(),
                &pool,
                &job.id,
                &first.id,
                SchedulePauseAnswer {
                    decision_id: &first.decision_id,
                    answer: "approve",
                    acted_by: crate::decisions::ACTOR_JESSE,
                    principal: "master",
                    from_automate: false,
                },
            )
            .await
            .unwrap();
        }
        let second = s2n_fail_three(&scheduler, &job).await;
        assert!(scheduled_job_resume(
            scheduler.as_ref(),
            &pool,
            &job.id,
            &first.id,
            SchedulePauseAnswer {
                decision_id: &first.decision_id,
                answer: "approve",
                acted_by: crate::decisions::ACTOR_JESSE,
                principal: "master",
                from_automate: false
            }
        )
        .await
        .is_err());
        assert!(scheduler.jobs.lock().await.get(&job.id).unwrap().1.paused);
        scheduler.jobs.lock().await.remove(&job.id);
        persist_jobs(&scheduler.storage_path, &scheduler.jobs)
            .await
            .unwrap();
        assert!(scheduled_job_resume(
            scheduler.as_ref(),
            &pool,
            &job.id,
            &second.id,
            SchedulePauseAnswer {
                decision_id: &second.decision_id,
                answer: "approve",
                acted_by: crate::decisions::ACTOR_JESSE,
                principal: "master",
                from_automate: false
            }
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn acceptance_12_three_new_failed_fires_create_new_episode() {
        let dir = tempfile::tempdir().unwrap();
        let (scheduler, job, pool) = s2n_fixture(dir.path()).await;
        let first = s2n_fail_three(&scheduler, &job).await;
        scheduled_job_resume(
            scheduler.as_ref(),
            &pool,
            &job.id,
            &first.id,
            SchedulePauseAnswer {
                decision_id: &first.decision_id,
                answer: "approve",
                acted_by: crate::decisions::ACTOR_JESSE,
                principal: "master",
                from_automate: false,
            },
        )
        .await
        .unwrap();
        let second = s2n_fail_three(&scheduler, &job).await;
        assert_ne!(first.id, second.id);
        assert_ne!(first.decision_id, second.decision_id);
        assert_eq!(s2n_request_count(&pool).await, 2);
        assert!(second.failed_runs.iter().all(|run| run.run_number > 3));
    }

    #[tokio::test]
    async fn acceptance_02_internal_retry_is_not_another_failed_fire() {
        let dir = tempfile::tempdir().unwrap();
        let (scheduler, mut job, pool) = s2n_fixture(dir.path()).await;
        job.max_retries = 1;
        install_job(&scheduler, job.clone()).await;
        // Missing recipe fails before a provider is constructed. The production
        // ten-second backoff is intentional; no cron clock or network is used.
        scheduler.task_for_job(job.clone())(instant("2026-09-21T12:00:00Z")).await;
        let jobs = scheduler.jobs.lock().await;
        let finished = &jobs.get(&job.id).unwrap().1;
        assert_eq!(finished.retry_count, 1);
        assert_eq!(finished.run_count, 1);
        assert_eq!(finished.consecutive_failures, 1);
        assert_eq!(finished.failed_runs.len(), 1);
        assert_eq!(s2n_request_count(&pool).await, 0);
    }

    #[tokio::test]
    async fn acceptance_06_resolution_failure_replays_durable_effect_without_running() {
        let dir = tempfile::tempdir().unwrap();
        let (scheduler, job, pool) = s2n_fixture(dir.path()).await;
        let episode = s2n_fail_three(&scheduler, &job).await;
        sqlx::query(
            "CREATE TRIGGER s2n_reject_resolution BEFORE UPDATE OF status ON decisions
            BEGIN SELECT RAISE(ABORT, 'test failure'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(scheduled_job_resume(
            scheduler.as_ref(),
            &pool,
            &job.id,
            &episode.id,
            SchedulePauseAnswer {
                decision_id: &episode.decision_id,
                answer: "approve",
                acted_by: crate::decisions::ACTOR_JESSE,
                principal: "master",
                from_automate: false
            }
        )
        .await
        .is_err());
        let saved = scheduler.jobs.lock().await.get(&job.id).unwrap().1.clone();
        assert!(!saved.paused);
        assert!(saved.pause_episode.as_ref().unwrap().effect.is_some());
        assert_eq!(saved.run_count, 3);
        sqlx::query("DROP TRIGGER s2n_reject_resolution")
            .execute(&pool)
            .await
            .unwrap();
        scheduler.reconcile_pause_decisions().await.unwrap();
        let decision = crate::decisions::get_decision(&pool, &episode.decision_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(decision.status, "answered"); // resolve_schedule_pause update
        assert_eq!(
            scheduler
                .jobs
                .lock()
                .await
                .get(&job.id)
                .unwrap()
                .1
                .run_count,
            3
        );
    }

    #[tokio::test]
    async fn acceptance_05_duplicate_fire_delivery_counts_once() {
        let dir = tempfile::tempdir().unwrap();
        let (scheduler, job, pool) = s2n_fixture(dir.path()).await;
        let task = scheduler.task_for_job(job.clone());
        let due = instant("2026-09-21T12:00:00Z");
        tokio::join!(task(due), task(due));
        let jobs = scheduler.jobs.lock().await;
        let finished = &jobs.get(&job.id).unwrap().1;
        assert_eq!(finished.run_count, 1);
        assert_eq!(finished.failed_runs.len(), 1);
        assert_eq!(s2n_request_count(&pool).await, 0);
    }
    #[tokio::test]
    async fn sovereign_discovery_sends_zero_requests() {
        let dir = tempfile::tempdir().unwrap();
        let _env = env_lock::lock_env([
            ("PERMAGENT_PATH_ROOT", Some(dir.path().to_str().unwrap())),
            ("SOVEREIGN_MODE", Some("true")),
            (
                "GOOSE_PROVIDER",
                Some("invalid-provider-must-not-be-constructed"),
            ),
            ("GOOSE_MODEL", Some("unused-native-model")),
        ]);
        struct ResetSovereignCache;
        impl Drop for ResetSovereignCache {
            fn drop(&mut self) {
                crate::sovereignty::invalidate_global_mode_cache();
            }
        }
        let _cache = ResetSovereignCache;
        crate::sovereignty::invalidate_global_mode_cache();
        assert!(crate::sovereignty::global_sovereign_mode());
        std::fs::write(dir.path().join("config.yaml"), "{}\n").unwrap();
        let recipe = dir.path().join("model_review.yaml");
        std::fs::write(
            &recipe,
            include_str!("../../goose-server/src/automation/model_review.yaml"),
        )
        .unwrap();
        let scheduler = scheduler_at(dir.path()).await;
        let pool = scheduler.session_manager.pool_clone().await.unwrap();
        let job = ScheduledJob {
            id: "native-review-without-starter".into(),
            starter_id: None,
            source: recipe.to_string_lossy().into_owned(),
            ..Default::default()
        };
        crate::session::session_manager::TEST_SESSION_MANAGER.scope(
            scheduler.session_manager.clone(),
            async {
                // Calibrate the actual transport guard against this isolated sink:
                // sovereign denial must leave a blocked record before scan baselines.
                assert!(!crate::sovereignty::guard_outbound_egress(
                    crate::sovereignty::EgressKind::CodeScan,
                    "https://example.invalid", "audit-probe",
                ).await);
                let rows = crate::sovereignty::recent_egress_rows(&pool, 10).await.unwrap();
                assert_eq!(rows.len(), 1);
                assert!(rows[0].blocked);
                let (_, probe) = crate::sovereignty::plan_outbound_egress(
                    crate::sovereignty::EgressKind::CodeScan,
                    "https://example.invalid", "audit-probe", true,
                );
                let mut previous_report = None;
                for fail_audit in [false, true] {
                    if fail_audit {
                        sqlx::query("CREATE TRIGGER reject_egress BEFORE INSERT ON egress_audit BEGIN SELECT RAISE(ABORT, 'audit sink unavailable'); END")
                            .execute(&pool).await.unwrap();
                        assert!(crate::sovereignty::record_egress(probe.clone()).await.is_err());
                    }
                    let before: (i64, i64) = sqlx::query_as(
                        "SELECT (SELECT COUNT(*) FROM egress_audit), (SELECT COUNT(*) FROM cost_ledger)",
                    ).fetch_one(&pool).await.unwrap();
                    // Same dispatch used by cron and Run now, before Agent setup.
                    // The invalid provider makes an accidental inference path fail.
                    let id = execute_job(
                        job.clone(), (scheduler.jobs.clone(), scheduler.session_manager.clone()),
                        job.id.clone(), CancellationToken::new(), None, None, None,
                    ).await.unwrap();
                    let after: (i64, i64) = sqlx::query_as(
                        "SELECT (SELECT COUNT(*) FROM egress_audit), (SELECT COUNT(*) FROM cost_ledger)",
                    ).fetch_one(&pool).await.unwrap();
                    assert_eq!(after.0 - before.0, 0, "scan attempted outbound egress");
                    assert_eq!(after.1 - before.1, 0, "scan performed inference");
                    let report = crate::model_review::report::latest(&pool).await.unwrap().unwrap();
                    assert_eq!(report.id, id);
                    assert_ne!(previous_report.as_ref(), Some(&id));
                    previous_report = Some(id);
                    // Exact field value from model_review::report::build.
                    assert_eq!(report.discovery_mode, "local-only");
                    assert!(report.summary.contains(crate::model_review::LOCAL_ONLY));
                    assert!(report.policy_blocked);
                    assert!(report.findings.iter().any(|finding|
                        finding.tier == crate::model_review::findings::FindingTier::PolicyBlocked));
                }
            },
        ).await;
    }

    #[tokio::test]
    async fn model_review_starter_without_native_recipe_uses_ordinary_path() {
        let dir = tempfile::tempdir().unwrap();
        let sentinel = "invalid-provider-must-not-be-constructed";
        let _env = env_lock::lock_env([
            ("PERMAGENT_PATH_ROOT", Some(dir.path().to_str().unwrap())),
            ("GOOSE_PROVIDER", Some(sentinel)),
            ("GOOSE_MODEL", Some("unused-native-model")),
        ]);
        let recipe = dir.path().join("ordinary.yaml");
        let mut declaration: serde_json::Value = serde_yaml::from_str(include_str!(
            "../../goose-server/src/automation/model_review.yaml"
        ))
        .unwrap();
        let fields = declaration.as_object_mut().unwrap();
        fields.remove("native_tool");
        fields.remove("produces_model_review");
        fields.insert("extensions".into(), serde_json::json!([]));
        std::fs::write(&recipe, serde_yaml::to_string(&declaration).unwrap()).unwrap();
        let scheduler = scheduler_at(dir.path()).await;
        let pool = scheduler.session_manager.pool_clone().await.unwrap();
        let job = ScheduledJob {
            id: "ordinary-model-review".into(),
            starter_id: Some("model_review".into()),
            source: recipe.to_string_lossy().into_owned(),
            ..Default::default()
        };
        let error = crate::session::session_manager::TEST_SESSION_MANAGER
            .scope(
                scheduler.session_manager.clone(),
                execute_job(
                    job.clone(),
                    (scheduler.jobs.clone(), scheduler.session_manager.clone()),
                    job.id.clone(),
                    CancellationToken::new(),
                    None,
                    None,
                    None,
                ),
            )
            .await
            .unwrap_err();
        // Exact provider-factory failure from providers::init::get_from_registry:
        // proves this reached ordinary provider setup, not native validation.
        assert_eq!(error.to_string(), format!("Unknown provider: {sentinel}"));
        assert!(crate::model_review::report::latest(&pool)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn scheduled_model_review_executes_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let _guard = env_lock::lock_env([
            ("PERMAGENT_PATH_ROOT", Some(root)),
            (
                "GOOSE_PROVIDER",
                Some("invalid-provider-must-not-be-constructed"),
            ),
        ]);
        std::fs::write(dir.path().join("config.yaml"), "{}\n").unwrap();
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        std::fs::write(
            dir.path().join("models/whisper-base-q8_0.gguf"),
            b"controlled inventory",
        )
        .unwrap();
        let recipe = dir.path().join("model_review.yaml");
        std::fs::write(
            &recipe,
            include_str!("../../goose-server/src/automation/model_review.yaml"),
        )
        .unwrap();
        let scheduler = scheduler_at(dir.path()).await;
        let job = ScheduledJob {
            id: "model_review".into(),
            starter_id: Some("model_review".into()),
            cron: "0 30 19 * * 0".into(),
            source: recipe.to_string_lossy().into_owned(),
            ..Default::default()
        };
        install_job(&scheduler, job.clone()).await;
        let pool = scheduler.session_manager.pool_clone().await.unwrap();
        // The real cron entry owns a fire claim and calls the native scanner.
        let task = scheduler.task_for_job(job.clone());
        let due = instant("2026-09-20T22:30:00Z");
        tokio::join!(task(due), task(due));
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM model_review_reports")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1, "duplicate fire must scan once");
        let report = crate::model_review::report::latest(&pool)
            .await
            .unwrap()
            .unwrap();
        assert!(report.summary.contains(crate::model_review::LOCAL_ONLY));
        assert!(report
            .inventory
            .artifacts
            .iter()
            .any(|a| a.path == dir.path().join("models/whisper-base-q8_0.gguf")));
        assert!(!report.findings.is_empty());
        assert_eq!(
            scheduler
                .jobs
                .lock()
                .await
                .get(&job.id)
                .unwrap()
                .1
                .last_status,
            Some(ScheduleRunStatus::Ok)
        );
        // Manual Run now shares the very same dispatch and typed persistence.
        scheduler.run_now(&job.id).await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM model_review_reports")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 2);
        sqlx::query("DROP TABLE model_review_findings")
            .execute(&pool)
            .await
            .unwrap();
        assert!(scheduler.run_now(&job.id).await.is_err());
        assert_eq!(
            scheduler
                .jobs
                .lock()
                .await
                .get(&job.id)
                .unwrap()
                .1
                .last_status,
            Some(ScheduleRunStatus::Error)
        );
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM model_review_reports")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 2, "persistence failure must roll back the report");
    }
}
