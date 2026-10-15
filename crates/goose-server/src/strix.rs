//! The Guard's sweep loop — the standing security agent (Strix engine).
//!
//! Every pass walks the user's own active projects, runs the Strix pentest
//! engine over each in read-only posture, and turns its SARIF findings into a
//! living fix checklist on that project's Overview. Nothing is remediated:
//! The Guard reports, and every scan is instructed static-only — the read-only
//! posture rides the engine's instruction channel (see `scan_project`).
//!
//! Honesty laws, inherited from the Watcher's loop:
//!   * no findings → silence, never filler;
//!   * the scanner absent (no Docker, no `strix` binary) is a stated fact in
//!     the log, not a degraded pretend-scan;
//!   * a missed preflight is SCAN BLOCKED, never ON WATCH pretending a hunt
//!     happened;
//!   * `strix_last_scan` is stamped only on a finished scan — a failed attempt
//!     advances rotation via `strix_last_attempt` so Overview cannot say
//!     "scanned clean" for a skip;
//!   * a per-project scan that stops working is said out loud ONCE, on the
//!     transition out of a working state, and counted quietly thereafter. Only
//!     whole-sweep preflight failures used to reach a surface, which is how the
//!     Guard attempted a scan every day for twenty days, completed none, and
//!     still read as healthy;
//!   * when `strix_docker_ssh` is set, the scan runs on that host after rsync
//!     (a forwarded Docker socket is not enough: Strix bind-mounts the local
//!     path, which does not exist on the remote daemon);
//!   * every target passes `strix::check_scope` before the scanner is invoked,
//!     so a path outside the user's own project roots cannot be reached even
//!     if a project row is malformed.

use crate::state::AppState;
use permagent::projects::{self, Project, UpdateProject};
use permagent::strix;
use sqlx::{Pool, Sqlite};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

/// Findings ride `projects.metadata_json.strix_findings` — same
/// no-migration storage the Watcher's insights use.
const METADATA_KEY: &str = "strix_findings";
/// When this project last finished a scan (ISO-8601). Overview honesty: only
/// a completed scan writes this. Failed attempts do not.
const LAST_SCAN_KEY: &str = "strix_last_scan";
/// Last sweep *attempt* (ISO-8601), including failures. Rotation uses this
/// so one broken project cannot starve the rest, without lying that it scanned.
const LAST_ATTEMPT_KEY: &str = "strix_last_attempt";
/// Consecutive failed scans for this project; reset to 0 by a completed scan.
/// 0 → 1 is the transition that briefs. Everything above 1 is digest-only.
pub const FAILURE_STREAK_KEY: &str = "strix_failure_streak";
/// When the current failure streak started failing (ISO-8601).
pub const LAST_FAILURE_KEY: &str = "strix_last_failure";
/// Why the last scan failed, in the scanner's own words.
pub const LAST_ERROR_KEY: &str = "strix_last_error";
/// Closed findings (fixed or dismissed) kept per project as history; the
/// oldest closed ones age out. ACTIVE findings are never aged out — the old
/// 40-item cap on the whole list silently dropped real, unfixed findings once a
/// project had more than 40 of them.
const MAX_CLOSED_KEPT: usize = 60;
/// Set when someone asks for an on-demand rescan of this project (ISO-8601).
/// The loop takes a requested project first and clears the key when that scan
/// completes or fails.
const RESCAN_KEY: &str = "strix_rescan_requested_at";
/// A project is rescanned on request at most this often: a scan spends real
/// credits and minutes, and a "Verify" button must not become a way to loop it.
const RESCAN_MIN_GAP: Duration = Duration::from_secs(30 * 60);
/// A request the loop could not act on (scanner down, sovereign mode) stops
/// counting as pending after this, so a stale flag cannot force a preflight
/// every tick forever.
const RESCAN_EXPIRES: Duration = Duration::from_secs(6 * 3600);
/// Wakes the loop the moment a rescan is requested instead of at the next
/// 15-minute tick. `notify_one` stores a permit, so a request that lands while
/// a sweep is running is still seen.
static RESCAN_NOTIFY: tokio::sync::Notify = tokio::sync::Notify::const_new();
/// One async mutex per project guarding every read-modify-write of the
/// project's metadata that touches `strix_findings`. A scan can run for
/// twenty minutes and `update_project` replaces `metadata_json` wholesale, so
/// without this a "mark sent" click during a scan's write (or two clicks) could
/// silently revert each other.
static PROJECT_LOCKS: LazyLock<std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// The lock for one project (by canonical id, not slug).
pub fn project_lock(project_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut map = PROJECT_LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    map.entry(project_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}
/// How often the loop wakes to check the flag and whether a sweep is due.
/// Cheap (two config reads), so interval changes take effect within minutes.
const CHECK_EVERY: Duration = Duration::from_secs(15 * 60);
/// Sweep cadence config key (`~/.permagent/config.yaml`), in hours.
const SWEEP_HOURS_KEY: &str = "strix_sweep_hours";
/// Default sweep cadence. Daily, not 6-hourly: every pass is a real agentic
/// scan of every active project on the USER'S API credits, and a security
/// posture does not change four times a day. Clamped to [1h, 1 week].
const DEFAULT_SWEEP_HOURS: u64 = 24;

fn sweep_interval() -> Duration {
    let hours = permagent::config::Config::global()
        .get_param::<u64>(SWEEP_HOURS_KEY)
        .unwrap_or(DEFAULT_SWEEP_HOURS)
        .clamp(1, 168);
    Duration::from_secs(hours * 3600)
}
/// Let boot (and any in-flight goal work) settle before the first sweep.
const STARTUP_DELAY: Duration = Duration::from_secs(300);
/// The short settle used instead of `STARTUP_DELAY` when the Guard is enabled
/// and has never scanned anything — just long enough for the DB to come up.
const FIRST_SWEEP_SETTLE: Duration = Duration::from_secs(15);
/// Hard bound on one project's scan.
const SCAN_TIMEOUT: Duration = Duration::from_secs(20 * 60);
/// Poll cadence for the mid-scan sovereignty re-check.
const SOVEREIGN_POLL: Duration = Duration::from_secs(30);
/// After SIGTERM, wait for strix to `docker rm` its own sandbox before
/// SIGKILL. 500ms left containers behind on the 2026-08-27 Reckonize
/// timeout — the scanner was killed before its handler ran.
const SCAN_TERM_GRACE: Duration = Duration::from_secs(3);
/// The default bucket is not a project Strix reports on.
const PERSONAL_PROJECT_ID: &str = "00000000-0000-0000-0000-000000000001";

/// Where a finding is in its life. Old rows carry no status and read as
/// `Open`; an unrecognised value also reads as `Open` rather than failing the
/// whole list (`other` must stay the last variant).
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FindingStatus {
    Reopened,
    Sent,
    FixCommitted,
    NotSeen,
    Fixed,
    Dismissed,
    #[default]
    #[serde(other)]
    Open,
}

impl FindingStatus {
    /// Still needs attention: everything except fixed and dismissed.
    pub fn is_active(self) -> bool {
        !matches!(self, Self::Fixed | Self::Dismissed)
    }
}

/// One finding, as rendered on the project's Overview and in briefings.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Default)]
pub struct Finding {
    pub id: String,
    pub title: String,
    pub severity: String,
    pub cwe: Option<String>,
    pub location: Option<String>,
    pub remediation: Option<String>,
    pub found_at: String,
    // Lifecycle. Every field below is optional so metadata written before the
    // lifecycle existed still reads (as `open`).
    #[serde(default)]
    pub status: FindingStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_seen_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix_commit_subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix_verified_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dismissed_reason: Option<String>,
}

impl Finding {
    /// Fill the defaults the contract promises for rows that predate the
    /// lifecycle: `first_seen_at` = `found_at`, and a status time.
    pub fn normalised(mut self) -> Self {
        if self.first_seen_at.is_none() && !self.found_at.is_empty() {
            self.first_seen_at = Some(self.found_at.clone());
        }
        if self.status_at.is_none() && !self.found_at.is_empty() {
            self.status_at = Some(self.found_at.clone());
        }
        self
    }

    fn set_status(&mut self, status: FindingStatus, now: &str) {
        self.status = status;
        self.status_at = Some(now.to_string());
    }

    /// Forget everything about a fix attempt: the finding is open again, so
    /// "sent to an agent" and "fix verified" would now be claims about a
    /// previous life of it.
    fn clear_fix_trail(&mut self) {
        self.sent_at = None;
        self.sent_agent = None;
        self.fix_commit = None;
        self.fix_commit_subject = None;
        self.fix_verified_at = None;
        self.verify_detail = None;
    }
}

/// What one completed scan did to the stored list.
#[derive(Debug, Clone, Default)]
pub struct Reconciled {
    /// The full list to store.
    pub findings: Vec<Finding>,
    /// Never seen before on this project (a dismissed or fixed finding that is
    /// reported again is NOT fresh).
    pub fresh: Vec<Finding>,
    /// Had a verified fix commit and was not reported by this scan.
    pub confirmed_fixed: Vec<Finding>,
    /// Was fixed or unconfirmed and is reported again.
    pub reopened: Vec<Finding>,
}

/// Fold one COMPLETED scan into the stored findings. Pure.
///
/// Call it on every completed scan, including a clean one: a scan that
/// reports nothing is exactly the evidence that a fix worked, and the old union
/// merge threw it away. A failed or skipped scan must not call this at all — it
/// proves nothing about what is fixed.
pub fn reconcile(existing: &[Finding], incoming: &[Finding], now: &str) -> Reconciled {
    let mut reported: HashMap<&str, &Finding> = HashMap::new();
    for f in incoming {
        reported.entry(f.id.as_str()).or_insert(f);
    }
    let known: HashSet<&str> = existing.iter().map(|f| f.id.as_str()).collect();

    let mut out: Vec<Finding> = Vec::with_capacity(existing.len() + incoming.len());
    let mut placed: HashSet<String> = HashSet::new();
    let mut confirmed_fixed = Vec::new();
    let mut reopened = Vec::new();

    for old in existing {
        if !placed.insert(old.id.clone()) {
            continue;
        }
        let mut f = old.clone().normalised();
        match reported.get(f.id.as_str()) {
            Some(new) => {
                f.last_seen_at = Some(now.to_string());
                // A dismissed finding stays dismissed however often it is seen:
                // the person said they accept it, and re-opening it every scan
                // would make dismissing pointless.
                if f.status != FindingStatus::Dismissed {
                    f.title = new.title.clone();
                    f.severity = new.severity.clone();
                    f.cwe = new.cwe.clone();
                    f.location = new.location.clone();
                    f.remediation = new.remediation.clone();
                    if matches!(f.status, FindingStatus::Fixed | FindingStatus::NotSeen) {
                        f.clear_fix_trail();
                        f.set_status(FindingStatus::Reopened, now);
                        reopened.push(f.clone());
                    } else if f.status == FindingStatus::FixCommitted
                        && f.fix_verified_at.as_deref().is_some_and(|at| at < now)
                    {
                        // The commit landed but a scan finished after it still
                        // reports the problem: the fix did not work. Leaving it
                        // at fix_committed would hide exactly the failure the
                        // rescan exists to catch.
                        let sha = f.fix_commit.clone().unwrap_or_default();
                        f.clear_fix_trail();
                        f.verify_detail = Some(format!(
                            "A scan after fix commit {} still reports this, so the fix did not clear it.",
                            sha.get(..10).unwrap_or(&sha)
                        ));
                        f.set_status(FindingStatus::Reopened, now);
                        reopened.push(f.clone());
                    }
                }
            }
            None => match f.status {
                FindingStatus::FixCommitted => {
                    f.set_status(FindingStatus::Fixed, now);
                    confirmed_fixed.push(f.clone());
                }
                FindingStatus::Open | FindingStatus::Reopened | FindingStatus::Sent => {
                    f.set_status(FindingStatus::NotSeen, now);
                }
                // Already not_seen, fixed or dismissed: nothing new to say.
                _ => {}
            },
        }
        out.push(f);
    }

    let mut fresh = Vec::new();
    let mut seen_new: HashSet<&str> = HashSet::new();
    for new in incoming {
        if known.contains(new.id.as_str()) || !seen_new.insert(new.id.as_str()) {
            continue;
        }
        let mut f = new.clone();
        f.status = FindingStatus::Open;
        f.status_at = Some(now.to_string());
        f.first_seen_at = Some(now.to_string());
        f.last_seen_at = Some(now.to_string());
        fresh.push(f);
    }

    // Newest first, as before; fresh ones lead.
    let mut findings = fresh.clone();
    findings.extend(out);
    cap_closed(&mut findings);

    Reconciled {
        findings,
        fresh,
        confirmed_fixed,
        reopened,
    }
}

/// Keep the newest `MAX_CLOSED_KEPT` closed findings; never touch active ones.
fn cap_closed(findings: &mut Vec<Finding>) {
    let mut closed: Vec<usize> = findings
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.status.is_active())
        .map(|(i, _)| i)
        .collect();
    if closed.len() <= MAX_CLOSED_KEPT {
        return;
    }
    // Newest status change first; list position breaks ties deterministically.
    closed.sort_by(|a, b| {
        findings[*b]
            .status_at
            .cmp(&findings[*a].status_at)
            .then(a.cmp(b))
    });
    let drop: HashSet<usize> = closed[MAX_CLOSED_KEPT..].iter().copied().collect();
    let mut i = 0usize;
    findings.retain(|_| {
        let keep = !drop.contains(&i);
        i += 1;
        keep
    });
}

/// The counts the Guard panel shows.
#[derive(serde::Serialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    pub active: usize,
    /// open + reopened: nobody has acted on these yet.
    pub open: usize,
    pub sent: usize,
    /// A fix commit is verified; waiting for a scan to confirm it.
    pub awaiting_rescan: usize,
    /// Not seen in the latest scan but no fix commit: unconfirmed.
    pub unconfirmed: usize,
    pub fixed: usize,
    pub dismissed: usize,
}

pub fn counts(findings: &[Finding]) -> Counts {
    let mut c = Counts::default();
    for f in findings {
        match f.status {
            FindingStatus::Open | FindingStatus::Reopened => c.open += 1,
            FindingStatus::Sent => c.sent += 1,
            FindingStatus::FixCommitted => c.awaiting_rescan += 1,
            FindingStatus::NotSeen => c.unconfirmed += 1,
            FindingStatus::Fixed => c.fixed += 1,
            FindingStatus::Dismissed => c.dismissed += 1,
        }
        if f.status.is_active() {
            c.active += 1;
        }
    }
    c
}

fn severity_rank(s: &str) -> u8 {
    match s {
        "critical" => 0,
        "high" => 1,
        "medium" => 2,
        _ => 3,
    }
}

/// Order for display: active first by severity, then closed newest first.
pub fn display_order(findings: &mut [Finding]) {
    findings.sort_by(|a, b| {
        let (aa, ba) = (a.status.is_active(), b.status.is_active());
        ba.cmp(&aa).then_with(|| {
            if aa {
                severity_rank(&a.severity).cmp(&severity_rank(&b.severity))
            } else {
                b.status_at.cmp(&a.status_at)
            }
        })
    });
}

/// Why a status change is refused. The routes turn these into 400/404.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    NotFound,
    Rejected(String),
    Failed(String),
}

/// The agent label stored on a "sent" finding.
const MAX_AGENT_CHARS: usize = 80;
/// Longest reason a dismissal may carry.
pub const MAX_DISMISS_REASON_CHARS: usize = 280;

/// open/reopened/sent/not_seen -> sent. Sending again (a second agent, or a
/// retry) is allowed and keeps the FIRST `sent_at`, so verify still counts a
/// commit made after the first send.
pub fn apply_sent(f: &mut Finding, agent: &str, now: &str) -> Result<(), StoreError> {
    let agent = agent.trim();
    if agent.is_empty() || agent.chars().count() > MAX_AGENT_CHARS {
        return Err(StoreError::Rejected(format!(
            "Say which coding agent it was sent to (1 to {MAX_AGENT_CHARS} characters)."
        )));
    }
    if !matches!(
        f.status,
        FindingStatus::Open
            | FindingStatus::Reopened
            | FindingStatus::Sent
            | FindingStatus::NotSeen
    ) {
        return Err(StoreError::Rejected(format!(
            "A finding that is {} cannot be sent to an agent; verify or reopen it instead.",
            status_word(f.status)
        )));
    }
    if f.sent_at.is_none() || !matches!(f.status, FindingStatus::Sent | FindingStatus::NotSeen) {
        f.sent_at = Some(now.to_string());
    }
    f.sent_agent = Some(agent.to_string());
    f.set_status(FindingStatus::Sent, now);
    Ok(())
}

/// open/reopened/sent/not_seen -> fix_committed, after a passing verify.
/// A finding already `fix_committed` just refreshes its evidence.
pub fn apply_verified(
    f: &mut Finding,
    commit: &str,
    subject: &str,
    detail: &str,
    now: &str,
) -> Result<(), StoreError> {
    if !f.status.is_active() {
        return Err(StoreError::Rejected(format!(
            "A finding that is {} cannot be verified. Reopen it first.",
            status_word(f.status)
        )));
    }
    f.fix_commit = Some(commit.to_string());
    f.fix_commit_subject = Some(subject.to_string());
    f.fix_verified_at = Some(now.to_string());
    f.verify_detail = Some(detail.to_string());
    if f.status != FindingStatus::FixCommitted {
        f.set_status(FindingStatus::FixCommitted, now);
    }
    Ok(())
}

/// any active -> dismissed (reason required), or fixed/dismissed -> open.
pub fn apply_status(
    f: &mut Finding,
    status: &str,
    reason: Option<&str>,
    now: &str,
) -> Result<(), StoreError> {
    match status {
        "dismissed" => {
            if !f.status.is_active() {
                return Err(StoreError::Rejected(format!(
                    "A finding that is {} cannot be dismissed.",
                    status_word(f.status)
                )));
            }
            let reason = reason.map(str::trim).unwrap_or_default();
            let len = reason.chars().count();
            if len == 0 || len > MAX_DISMISS_REASON_CHARS {
                return Err(StoreError::Rejected(format!(
                    "Say why you are dismissing it (1 to {MAX_DISMISS_REASON_CHARS} characters)."
                )));
            }
            f.dismissed_reason = Some(reason.to_string());
            f.set_status(FindingStatus::Dismissed, now);
            Ok(())
        }
        "open" => {
            if f.status.is_active() {
                return Err(StoreError::Rejected(format!(
                    "A finding that is {} is already active; only a fixed or dismissed one can be reopened.",
                    status_word(f.status)
                )));
            }
            f.dismissed_reason = None;
            f.clear_fix_trail();
            f.set_status(FindingStatus::Open, now);
            Ok(())
        }
        other => Err(StoreError::Rejected(format!(
            "Unknown status \"{other}\"; use \"dismissed\" (with a reason) or \"open\"."
        ))),
    }
}

fn status_word(s: FindingStatus) -> &'static str {
    match s {
        FindingStatus::Open => "open",
        FindingStatus::Reopened => "reopened",
        FindingStatus::Sent => "sent",
        FindingStatus::FixCommitted => "fix committed",
        FindingStatus::NotSeen => "not seen",
        FindingStatus::Fixed => "fixed",
        FindingStatus::Dismissed => "dismissed",
    }
}

/// The findings stored in a project's metadata, with lifecycle defaults filled.
pub fn stored_findings(meta: &serde_json::Value) -> Vec<Finding> {
    meta.as_object()
        .and_then(|m| m.get(METADATA_KEY))
        .and_then(|v| serde_json::from_value::<Vec<Finding>>(v.clone()).ok())
        .unwrap_or_default()
        .into_iter()
        .map(Finding::normalised)
        .collect()
}

/// The scan stamps the Guard panel shows.
#[derive(Debug, Clone, Default)]
pub struct Stamps {
    pub last_scan: Option<String>,
    pub last_attempt: Option<String>,
    pub rescan_requested_at: Option<String>,
}

pub fn stamps(meta: &serde_json::Value) -> Stamps {
    let get = |k: &str| {
        meta.as_object()
            .and_then(|m| m.get(k))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    Stamps {
        last_scan: get(LAST_SCAN_KEY),
        last_attempt: get(LAST_ATTEMPT_KEY),
        rescan_requested_at: get(RESCAN_KEY),
    }
}

/// Read-modify-write the project's findings under the project's lock.
/// `change` gets the stored list and returns the list to store plus a value.
/// The project row is re-read INSIDE the lock for the same reason
/// `write_meta` does: whatever else wrote metadata meanwhile must survive.
pub async fn update_findings<R>(
    pool: &Pool<Sqlite>,
    project: &Project,
    change: impl FnOnce(Vec<Finding>) -> Result<(Vec<Finding>, R), StoreError>,
) -> Result<R, StoreError> {
    let lock = project_lock(&project.id);
    let _held = lock.lock().await;
    let fresh = projects::get_project_by_id_or_slug(pool, &project.id)
        .await
        .map_err(StoreError::Failed)?
        .ok_or(StoreError::NotFound)?;
    let mut meta = fresh.metadata_json.as_object().cloned().unwrap_or_default();
    let existing = stored_findings(&fresh.metadata_json);
    let (updated, out) = change(existing)?;
    meta.insert(
        METADATA_KEY.to_string(),
        serde_json::to_value(&updated).map_err(|e| StoreError::Failed(e.to_string()))?,
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
    .map_err(StoreError::Failed)?;
    Ok(out)
}

/// Change one finding by id under the project's lock and return it.
pub async fn mutate_finding(
    pool: &Pool<Sqlite>,
    project: &Project,
    finding_id: &str,
    change: impl FnOnce(&mut Finding) -> Result<(), StoreError>,
) -> Result<Finding, StoreError> {
    update_findings(pool, project, |mut all| {
        let f = all
            .iter_mut()
            .find(|f| f.id == finding_id)
            .ok_or(StoreError::NotFound)?;
        change(f)?;
        let done = f.clone();
        Ok((all, done))
    })
    .await
}

// ── On-demand rescan ───────────────────────────────────────────────────

fn parse_ts(stamp: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(stamp)
        .ok()
        .map(|t| t.with_timezone(&chrono::Utc))
}

/// How long ago `stamp` was; a future stamp (clock skew) reads as just now.
fn age_of(stamp: Option<&str>, now: chrono::DateTime<chrono::Utc>) -> Option<Duration> {
    let t = parse_ts(stamp?)?;
    Some((now - t).to_std().unwrap_or(Duration::ZERO))
}

/// What the rescan gate needs to know. Plain data so the rules are testable.
pub struct RescanGate<'a> {
    pub enabled: bool,
    pub sovereign: bool,
    pub has_root: bool,
    pub requested_at: Option<&'a str>,
    pub last_attempt: Option<&'a str>,
}

/// None when a rescan may be queued; otherwise the reason it cannot, in words
/// the person can act on.
pub fn rescan_refusal(gate: &RescanGate<'_>, now: chrono::DateTime<chrono::Utc>) -> Option<String> {
    if !gate.enabled {
        return Some(
            "The Guard is turned off. Turn it on in Settings, then ask for a rescan.".into(),
        );
    }
    if gate.sovereign {
        return Some(
            "Sovereign mode is on and the scanner uses a cloud model, so no scan will run.".into(),
        );
    }
    if !gate.has_root {
        return Some("This project has no local folder to scan. Set its folder first.".into());
    }
    let gap = RESCAN_MIN_GAP;
    if let Some(age) = age_of(gate.requested_at, now).filter(|a| *a < gap) {
        return Some(format!(
            "A rescan was already requested {} minute(s) ago. The Guard rescans a project at most every {} minutes.",
            age.as_secs() / 60,
            gap.as_secs() / 60
        ));
    }
    if let Some(age) = age_of(gate.last_attempt, now).filter(|a| *a < gap) {
        return Some(format!(
            "This project was scanned {} minute(s) ago. The Guard rescans a project at most every {} minutes.",
            age.as_secs() / 60,
            gap.as_secs() / 60
        ));
    }
    None
}

/// A rescan request the loop should still act on.
fn rescan_pending(meta: &serde_json::Value, now: chrono::DateTime<chrono::Utc>) -> bool {
    age_of(stamps(meta).rescan_requested_at.as_deref(), now).is_some_and(|a| a < RESCAN_EXPIRES)
}

/// The result of asking for a rescan.
#[derive(Debug, Clone)]
pub struct RescanOutcome {
    pub queued: bool,
    pub requested_at: Option<String>,
    pub reason: Option<String>,
}

/// Queue a rescan of one project, or say why not. Never errors for a refusal.
pub async fn request_rescan(
    pool: &Pool<Sqlite>,
    project: &Project,
) -> Result<RescanOutcome, String> {
    let lock = project_lock(&project.id);
    let _held = lock.lock().await;
    let fresh = projects::get_project_by_id_or_slug(pool, &project.id)
        .await?
        .ok_or_else(|| "unknown project".to_string())?;
    let st = stamps(&fresh.metadata_json);
    let gate = RescanGate {
        enabled: strix::is_enabled(),
        sovereign: permagent::sovereignty::global_sovereign_mode(),
        has_root: fresh
            .root_path
            .as_deref()
            .is_some_and(|r| !r.trim().is_empty()),
        requested_at: st.rescan_requested_at.as_deref(),
        last_attempt: st.last_attempt.as_deref(),
    };
    let now = chrono::Utc::now();
    if let Some(reason) = rescan_refusal(&gate, now) {
        return Ok(RescanOutcome {
            queued: false,
            requested_at: st.rescan_requested_at,
            reason: Some(reason),
        });
    }
    let stamp = now.to_rfc3339();
    write_meta_locked(
        pool,
        &fresh,
        &[(RESCAN_KEY, serde_json::Value::String(stamp.clone()))],
    )
    .await?;
    RESCAN_NOTIFY.notify_one();
    tracing::info!(
        target: "permagentd::strix",
        project = %fresh.name,
        "rescan requested — the loop takes this project first"
    );
    Ok(RescanOutcome {
        queued: true,
        requested_at: Some(stamp),
        reason: None,
    })
}

/// True when any active project has a live rescan request.
async fn any_rescan_pending(state: &Arc<AppState>) -> bool {
    let Ok(pool) = state.session_manager().pool_clone().await else {
        return false;
    };
    let now = chrono::Utc::now();
    projects::list_projects(&pool, Some("active"))
        .await
        .map(|ps| {
            ps.iter()
                .any(|p| p.root_path.is_some() && rescan_pending(&p.metadata_json, now))
        })
        .unwrap_or(false)
}

pub fn spawn(state: Arc<AppState>) {
    // The loop always spawns and re-reads the flag every tick, so flipping
    // `strix_enabled` in Settings takes effect at the next tick — no daemon
    // restart, and no silently-absent loop either way.
    if strix::is_enabled() {
        tracing::info!(
            target: "permagentd::strix",
            "The Guard enabled — security sweeps every {}h, read-only posture",
            sweep_interval().as_secs() / 3600
        );
    } else {
        tracing::info!(
            target: "permagentd::strix",
            "The Guard is off ({}=false) — sweep loop idle until enabled",
            strix::STRIX_ENABLED_KEY
        );
    }
    tokio::spawn(async move {
        let mut last_sweep: Option<tokio::time::Instant>;
        // First-value latency (audit 2026-08-11): a user who just enabled the
        // Guard on a never-scanned fleet should not wait STARTUP_DELAY +
        // CHECK_EVERY for the first evidence it exists. One short settle for
        // the DB to come up, then sweep immediately — only in the genuinely
        // never-scanned case; an established install keeps the full boot
        // settle.
        let first_sweep_now = if strix::is_enabled() {
            tokio::time::sleep(FIRST_SWEEP_SETTLE).await;
            strix::is_enabled() && never_scanned(&state).await
        } else {
            false
        };
        if first_sweep_now {
            last_sweep = Some(tokio::time::Instant::now());
            if let Err(e) = sweep_once(&state).await {
                tracing::warn!(target: "permagentd::strix", "first sweep skipped: {e}");
            }
        } else {
            tokio::time::sleep(STARTUP_DELAY).await;
            // `last_sweep` is in-memory, so without this seed EVERY daemon
            // restart swept STARTUP_DELAY later (12 sweeps in 3h on
            // 2026-09-20). The durable record is each project's
            // `strix_last_attempt`: the newest one is when the last sweep ran.
            last_sweep = seeded_last_sweep(&state).await;
        }
        loop {
            let interval_due = last_sweep.is_none_or(|t| t.elapsed() >= sweep_interval());
            // A pending rescan request bypasses ONLY the interval; every other
            // gate (enabled, sovereign, preflight, scope, egress audit) still
            // applies inside `sweep_once`.
            let requested = strix::is_enabled() && any_rescan_pending(&state).await;
            if strix::is_enabled() && (interval_due || requested) {
                // A rescan-only sweep must not use up the scheduled sweep's slot.
                if interval_due {
                    last_sweep = Some(tokio::time::Instant::now());
                }
                if let Err(e) = sweep_once(&state).await {
                    tracing::warn!(target: "permagentd::strix", "sweep skipped: {e}");
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(CHECK_EVERY) => {}
                _ = RESCAN_NOTIFY.notified() => {}
            }
        }
    });
}

/// The World shows the Guard working only while it genuinely is — the honesty
/// clamp in `agentStatus.ts` refuses to animate a `sim` agent as busy, so this
/// is what earns the amber pose and the work halo.
fn announce(state_label: &str) {
    permagent::events::emit(permagent::events::agent_state_changed(
        strix::STRIX_FEATURE_ID,
        strix::STRIX_NAME,
        state_label,
    ));
}

async fn sweep_once(state: &Arc<AppState>) -> Result<(), String> {
    // Sovereign mode is enforced and audited at the scan itself; this cheap
    // early return only avoids pointless preflight and database work.
    if permagent::sovereignty::global_sovereign_mode() {
        tracing::info!(
            target: "permagentd::strix",
            "sweep skipped: sovereign mode is on and the scanner reaches a cloud model"
        );
        return Ok(());
    }
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| e.to_string())?;

    // Dependency preflight — refuse loudly, once per condition. Before this,
    // a missing scanner or stopped Docker surfaced only as a per-project
    // daemon-log warning: the user who flipped the toggle saw nothing, ever.
    let config = permagent::config::Config::global();
    match preflight().await {
        Ok(()) => {
            // Recovered (or always fine): clear the stamp so a future
            // breakage is news again.
            if config.get_param::<String>(PREFLIGHT_BRIEFED_KEY).is_ok() {
                let _ = config.delete(PREFLIGHT_BRIEFED_KEY);
            }
        }
        Err(failure) => {
            let prev = config.get_param::<String>(PREFLIGHT_BRIEFED_KEY).ok();
            if report_preflight_failure(&pool, prev.as_deref(), &failure).await {
                let _ = config.set_param(PREFLIGHT_BRIEFED_KEY, failure.clone());
            }
            announce("error");
            return Err(format!("preflight failed: {failure}"));
        }
    }

    let projects = projects::list_projects(&pool, Some("active")).await?;

    let roots: Vec<PathBuf> = projects
        .iter()
        .filter_map(|p| p.root_path.as_ref().map(PathBuf::from))
        .collect();
    if roots.is_empty() {
        return Ok(());
    }

    // ONE project per sweep, rotating least-recently-scanned first (ruling
    // 2026-08-06): a whole-fleet pass four times a day was the wrong shape —
    // one focused scan per interval spreads cost evenly and gives each
    // project a fresh report on a predictable cycle. A never-scanned project
    // sorts first (empty stamp).
    let mut candidates: Vec<&Project> = projects
        .iter()
        .filter(|p| p.id != PERSONAL_PROJECT_ID && p.root_path.is_some())
        .collect();
    // A project someone asked to rescan goes first, whatever the rotation says.
    let now_utc = chrono::Utc::now();
    candidates.sort_by_key(|p| {
        (
            !rescan_pending(&p.metadata_json, now_utc),
            rotation_stamp(p),
        )
    });
    let Some(project) = candidates.first().copied() else {
        return Ok(());
    };
    let rescan_requested = rescan_pending(&project.metadata_json, now_utc);
    if rescan_requested {
        tracing::info!(
            target: "permagentd::strix",
            project = %project.name,
            "taking the requested rescan first"
        );
    }
    let root = project.root_path.clone().unwrap_or_default();

    // The scope guard runs even though the target came from our own
    // project table: a malformed row must not become a scan of `/`.
    let target = match strix::check_scope(&root, &roots) {
        Ok(p) => p,
        Err(refusal) => {
            tracing::warn!(
                target: "permagentd::strix",
                project = %project.name,
                root = %root,
                "refused out-of-scope scan target: {refusal:?}"
            );
            // Stamp the attempt so an unresolvable root cannot pin the rotation,
            // but do not stamp last_scan — Overview must not say this was clean.
            let _ = stamp_last_attempt(&pool, project).await;
            clear_rescan_request(&pool, project, rescan_requested).await;
            return Ok(());
        }
    };

    // The audited choke point for the sweep. The scanner drives its OWN cloud
    // LLM over the user's source, so this is real outbound egress that the
    // provider guard never sees: it is recorded here — blocked or allowed —
    // like any other cloud call, and a refusal (sovereign mode, or an audit the
    // daemon cannot write) means no scan rather than an unlogged one.
    let model = strix_model();
    if !permagent::sovereignty::guard_outbound_egress(
        permagent::sovereignty::EgressKind::CodeScan,
        &model,
        &project.name,
    )
    .await
    {
        let reason = if permagent::sovereignty::global_sovereign_mode() {
            "sovereign mode"
        } else {
            "egress audit unavailable"
        };
        tracing::info!(
            target: "permagentd::strix",
            project = %project.name,
            "scan refused: {reason}"
        );
        // Stamp the attempt so a refused project cannot pin the rotation.
        // Do not stamp last_scan: a sovereignty skip is not a clean hunt.
        if let Err(e) = stamp_last_attempt(&pool, project).await {
            tracing::warn!(target: "permagentd::strix", "last-attempt stamp failed: {e}");
        }
        clear_rescan_request(&pool, project, rescan_requested).await;
        return Ok(());
    }

    announce("working");
    let outcome = scan_project(&target).await;
    // Rotation advances on every attempt so one broken project cannot starve
    // the rest. last_scan is success-only — Overview honesty.
    if let Err(e) = stamp_last_attempt(&pool, project).await {
        tracing::warn!(target: "permagentd::strix", "last-attempt stamp failed: {e}");
    }
    match outcome {
        Ok(findings) => {
            if let Err(e) = stamp_last_scan(&pool, project).await {
                tracing::warn!(target: "permagentd::strix", "last-scan stamp failed: {e}");
            }
            let reported = findings.len();
            // EVERY completed scan reconciles, a clean one included: reporting
            // nothing is how a fix gets confirmed.
            match apply_completed_scan(&pool, project, findings).await {
                Ok(rec) => {
                    if reported == 0 {
                        tracing::info!(
                            target: "permagentd::strix",
                            project = %project.name,
                            confirmed_fixed = rec.confirmed_fixed.len(),
                            "clean — no findings"
                        );
                    }
                    // The deliverable: a security report note on the project,
                    // findings plus an ordered fix plan. Notes index into the
                    // Brain, so "ask your agent to read the Guard's report and
                    // dispatch a fix goal" works with no extra plumbing. A clean
                    // scan that confirms nothing stays silent.
                    if reported > 0 || !rec.confirmed_fixed.is_empty() {
                        let body =
                            security_report_markdown(&project.name, &rec.findings, &rec.fresh);
                        let title = format!(
                            "Security report — {} — {}",
                            project.name,
                            chrono::Utc::now().format("%Y-%m-%d")
                        );
                        if let Err(e) = permagent::project_notes::create_note_indexed(
                            &pool,
                            state.brain.as_ref(),
                            &project.id,
                            Some(&title),
                            &body,
                        )
                        .await
                        {
                            tracing::warn!(
                                target: "permagentd::strix",
                                project = %project.name,
                                "report note not saved: {e}"
                            );
                        }
                    }
                    brief_new_findings(&pool, project, &rec.fresh).await;
                    brief_confirmed_fixed(&pool, project, &rec.confirmed_fixed).await;
                    brief_reopened(&pool, project, &rec.reopened).await;
                }
                Err(e) => tracing::warn!(
                    target: "permagentd::strix",
                    project = %project.name,
                    "findings not recorded: {e:?}"
                ),
            }
            announce("available");
        }
        Err(e) if strix::is_empty_input_skip(&e) => {
            // Empty tree / empty model input is a skip, not a scanner crash.
            // last_scan stays unset (Overview honesty); last_attempt already
            // advanced so this project cannot pin the rotation.
            tracing::info!(
                target: "permagentd::strix",
                project = %project.name,
                "scan skipped: no scannable files"
            );
            announce("available");
        }
        Err(e) => {
            // A missing scanner is a stated fact, not a silent skip.
            tracing::warn!(
                target: "permagentd::strix",
                project = %project.name,
                "scan did not run: {e}"
            );
            if record_scan_failure(&pool, project, &e).await {
                brief_scan_failure(&pool, project, &e).await;
            }
            announce("error");
        }
    }
    // Completed, skipped or failed: the request has been served.
    clear_rescan_request(&pool, project, rescan_requested).await;
    tracing::info!(
        target: "permagentd::strix",
        project = %project.name,
        "sweep complete — next sweep takes the next least-recently-scanned project"
    );
    Ok(())
}

/// Clear the project's rescan request once its scan has finished or failed.
async fn clear_rescan_request(pool: &Pool<Sqlite>, project: &Project, requested: bool) {
    if !requested {
        return;
    }
    if let Err(e) = write_meta(pool, project, &[(RESCAN_KEY, serde_json::Value::Null)]).await {
        tracing::warn!(target: "permagentd::strix", "rescan flag not cleared: {e}");
    }
}

/// Fold a completed scan into the stored findings under the project's lock.
async fn apply_completed_scan(
    pool: &Pool<Sqlite>,
    project: &Project,
    incoming: Vec<Finding>,
) -> Result<Reconciled, StoreError> {
    let now = chrono::Utc::now().to_rfc3339();
    update_findings(pool, project, |existing| {
        let rec = reconcile(&existing, &incoming, &now);
        Ok((rec.findings.clone(), rec))
    })
    .await
}

/// True only when NO active project carries a last-scan stamp — the Guard has
/// genuinely never run here. Any error reads as "not first time": the honest
/// failure mode is the normal (slow) startup path, never an eager scan on bad
/// data.
async fn never_scanned(state: &Arc<AppState>) -> bool {
    let Ok(pool) = state.session_manager().pool_clone().await else {
        return false;
    };
    match projects::list_projects(&pool, Some("active")).await {
        Ok(projects) => {
            !projects.is_empty() && projects.iter().all(|p| rotation_stamp(p).is_empty())
        }
        Err(_) => false,
    }
}

/// The in-memory sweep clock, rebuilt from the database after a restart.
/// None (sweep on the next due check) when there is no stamp or the DB is not
/// readable — the failure mode is one extra sweep, never a skipped one.
async fn seeded_last_sweep(state: &Arc<AppState>) -> Option<tokio::time::Instant> {
    let pool = state.session_manager().pool_clone().await.ok()?;
    let projects = projects::list_projects(&pool, Some("active")).await.ok()?;
    let since = time_since_last_sweep(
        projects.iter().map(|p| &p.metadata_json),
        chrono::Utc::now(),
    )?;
    // The monotonic clock cannot go back further than the machine's uptime.
    // Right after a reboot, a sweep that is not yet due is seeded as "now":
    // late by at most the uptime, never an extra sweep.
    let seeded = tokio::time::Instant::now()
        .checked_sub(since)
        .or_else(|| (since < sweep_interval()).then(tokio::time::Instant::now));
    if seeded.is_some() {
        tracing::info!(
            target: "permagentd::strix",
            "last sweep was {}m ago (from strix_last_attempt) — next due in {}m",
            since.as_secs() / 60,
            sweep_interval().saturating_sub(since).as_secs() / 60
        );
    }
    seeded
}

/// How long ago the most recent sweep attempt happened, from the newest
/// `strix_last_attempt` (falling back to `last_scan`) across projects. A
/// stamp in the future (clock skew) reads as "just now". None when no
/// project has a parseable stamp.
fn time_since_last_sweep<'a>(
    metas: impl IntoIterator<Item = &'a serde_json::Value>,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<Duration> {
    let newest = metas
        .into_iter()
        .filter_map(|m| {
            chrono::DateTime::parse_from_rfc3339(&rotation_stamp_from_meta(m))
                .ok()
                .map(|t| t.with_timezone(&chrono::Utc))
        })
        .max()?;
    Some((now - newest).to_std().unwrap_or(Duration::ZERO))
}

/// Least-recently-attempted stamp for rotation. Prefer `strix_last_attempt`
/// so failed sweeps still rotate; fall back to last_scan for metadata written
/// before that key existed. Empty sorts first (never tried).
fn rotation_stamp(project: &Project) -> String {
    rotation_stamp_from_meta(&project.metadata_json)
}

fn rotation_stamp_from_meta(meta: &serde_json::Value) -> String {
    let obj = meta.as_object();
    obj.and_then(|m| m.get(LAST_ATTEMPT_KEY))
        .or_else(|| obj.and_then(|m| m.get(LAST_SCAN_KEY)))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

async fn stamp_last_scan(pool: &Pool<Sqlite>, project: &Project) -> Result<(), String> {
    // A completed scan ends the failure streak: the next breakage is news again.
    write_meta(
        pool,
        project,
        &[
            (LAST_SCAN_KEY, now_value()),
            (FAILURE_STREAK_KEY, serde_json::Value::from(0u64)),
            (LAST_ERROR_KEY, serde_json::Value::Null),
        ],
    )
    .await
}

async fn stamp_last_attempt(pool: &Pool<Sqlite>, project: &Project) -> Result<(), String> {
    write_meta(pool, project, &[(LAST_ATTEMPT_KEY, now_value())]).await
}

fn now_value() -> serde_json::Value {
    serde_json::Value::String(chrono::Utc::now().to_rfc3339())
}

/// Record a failed scan and report whether this is the TRANSITION into
/// failure — the first miss after a run that worked.
///
/// The rule comes from dead-letter alerting doctrine (a CloudWatch alarm fires
/// on the state change into ALARM, not per datapoint): push once on 0→nonzero,
/// then go quiet and let the digest carry the standing state. A per-tick
/// briefing for a project that fails every day is the fastest way to teach the
/// reader to ignore the Guard entirely.
async fn record_scan_failure(pool: &Pool<Sqlite>, project: &Project, error: &str) -> bool {
    let fresh = projects::get_project_by_id_or_slug(pool, &project.id)
        .await
        .ok()
        .flatten();
    let prior = fresh
        .as_ref()
        .unwrap_or(project)
        .metadata_json
        .as_object()
        .and_then(|m| m.get(FAILURE_STREAK_KEY))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let _ = write_meta(
        pool,
        project,
        &[
            (FAILURE_STREAK_KEY, serde_json::Value::from(prior + 1)),
            (LAST_FAILURE_KEY, now_value()),
            (LAST_ERROR_KEY, serde_json::Value::String(error.to_string())),
        ],
    )
    .await;
    scan_failure_should_brief(prior)
}

/// Pure transition gate, split out so the "brief once, then count quietly"
/// rule is testable without a projects table.
fn scan_failure_should_brief(prior_streak: u64) -> bool {
    prior_streak == 0
}

/// Write ISO-8601 stamps / counters into project metadata without touching
/// findings. A `Null` value removes the key.
async fn write_meta(
    pool: &Pool<Sqlite>,
    project: &Project,
    entries: &[(&str, serde_json::Value)],
) -> Result<(), String> {
    // Same lock as the findings writers: this replaces `metadata_json`
    // wholesale, so an unguarded stamp could revert a finding change.
    let lock = project_lock(&project.id);
    let _held = lock.lock().await;
    write_meta_locked(pool, project, entries).await
}

/// `write_meta` for callers that already hold the project's lock.
async fn write_meta_locked(
    pool: &Pool<Sqlite>,
    project: &Project,
    entries: &[(&str, serde_json::Value)],
) -> Result<(), String> {
    // Re-read for the same reason `update_findings` does: `project` was
    // snapshotted before a scan that can run for twenty minutes, and
    // `update_project` replaces `metadata_json` wholesale. Writing the stale
    // snapshot back silently reverts every metadata change anything else made
    // to this project during the scan — analytics config, notes state, another
    // agent's edit. The window shrinks to one read-modify-write.
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
    for (key, value) in entries {
        if value.is_null() {
            meta.remove(*key);
        } else {
            meta.insert((*key).to_string(), value.clone());
        }
    }
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

/// File the one-per-transition briefing for a project whose scan stopped
/// working. Mirrors the whole-sweep `preflight_failed` briefing that already
/// exists — the gap this closes is that a PER-PROJECT failure had no surface
/// at all, only a `tracing::warn`, which is how the Guard attempted a scan
/// every day from 2026-08-11 to 2026-08-31, completed none, and still looked
/// healthy from the Settings toggle.
async fn brief_scan_failure(pool: &Pool<Sqlite>, project: &Project, error: &str) {
    permagent::briefings::file_briefing(
        pool,
        permagent::briefings::NewBriefing {
            from_agent: strix::STRIX_FEATURE_ID.to_string(),
            kind: "scan_failed".to_string(),
            severity: permagent::briefings::Severity::Attention,
            summary: format!(
                "The Guard's scan of {} stopped working: {error}",
                project.name
            ),
            detail: Some(format!(
                "This is the first failure since {} last completed a scan, so it is reported \
                 once. Further failures are counted, not re-announced — the daily job digest \
                 carries the standing state. Findings already on the project's Overview are \
                 from the last scan that finished and are not being refreshed.",
                project.name
            )),
            ref_kind: Some("project".to_string()),
            ref_id: Some(project.id.clone()),
        },
    )
    .await;
}

/// The report note: what is outstanding, then a fix plan ordered by severity —
/// written so your agent can turn it into a dispatchable goal verbatim. Only
/// ACTIVE findings are listed and planned; fixed and dismissed ones are
/// counted, because a fix plan that includes fixed items sends work in circles.
fn security_report_markdown(project_name: &str, current: &[Finding], fresh: &[Finding]) -> String {
    let c = counts(current);
    let mut ordered: Vec<&Finding> = current.iter().filter(|f| f.status.is_active()).collect();
    ordered.sort_by_key(|f| severity_rank(&f.severity));

    let mut out = format!(
        "The Guard scanned **{project_name}**: {} active ({} awaiting a confirming rescan, {} unconfirmed), {} fixed, {} dismissed. {} new this scan.\n\n## Findings\n\n",
        c.active,
        c.awaiting_rescan,
        c.unconfirmed,
        c.fixed,
        c.dismissed,
        fresh.len(),
    );
    if ordered.is_empty() {
        out.push_str("Nothing active.\n");
    }
    for f in &ordered {
        let new_tag = if fresh.iter().any(|n| n.id == f.id) {
            " · NEW"
        } else {
            ""
        };
        out.push_str(&format!(
            "- **[{}]{}** {}{}\n",
            f.severity.to_uppercase(),
            new_tag,
            f.title,
            f.location
                .as_deref()
                .map(|l| format!(" — `{l}`"))
                .unwrap_or_default(),
        ));
        if let Some(cwe) = f.cwe.as_deref() {
            out.push_str(&format!("  - {cwe}\n"));
        }
    }
    out.push_str("\n## Fix plan\n\n");
    for (i, f) in ordered.iter().enumerate() {
        out.push_str(&format!(
            "{}. {}: {}\n",
            i + 1,
            f.title,
            f.remediation
                .as_deref()
                .unwrap_or("review the finding location and remove the exposure"),
        ));
    }
    // Addressed to the user, so it must not name the agent: every user's agent
    // carries their own configured name. "your agent" reads correctly whatever
    // they called it.
    out.push_str(
        "\nTo act on this: ask your agent to read this report and dispatch a fix goal \
         (Claude Code or Codex) for the plan above.\n",
    );
    out
}

/// Run the scanner over one project and parse its SARIF. Strix (the engine)
/// writes `findings.sarif` per run; SARIF is preferred over its bespoke JSON
/// because it is schema-validated and dedupes on CWE.
/// The scanner's model, as a LiteLLM model string (`strix_llm` in config).
/// Defaults to GPT-5.4-mini: sweeps recur on the user's credits, and finding
/// exposed secrets does not need a frontier model. Haiku was the previous
/// default; Anthropic credit exhaustion (and a stale 3.5-Sonnet pin on the
/// reviewer) took the Guard and the independent-review gate down together.
const STRIX_LLM_KEY: &str = "strix_llm";
const DEFAULT_STRIX_LLM: &str = "openai/gpt-5.4-mini";

/// The cloud model the scanner drives — the destination the user's source
/// actually reaches, and so what the egress audit records.
fn strix_model() -> String {
    permagent::config::Config::global()
        .get_param::<String>(STRIX_LLM_KEY)
        .unwrap_or_else(|_| DEFAULT_STRIX_LLM.to_string())
}

/// Which keychain secret fills `LLM_API_KEY` for a LiteLLM `provider/model` string.
/// `deepseek` and `custom_deepseek` share `DEEPSEEK_API_KEY`; an unknown prefix
/// is left unset rather than guessed.
fn strix_llm_secret_name(model: &str) -> Option<&'static str> {
    match model.split('/').next().unwrap_or_default() {
        "anthropic" => Some("ANTHROPIC_API_KEY"),
        "openai" => Some("OPENAI_API_KEY"),
        "groq" => Some("GROQ_API_KEY"),
        "gemini" | "google" => Some("GOOGLE_API_KEY"),
        "deepseek" | "custom_deepseek" => Some("DEEPSEEK_API_KEY"),
        "moonshot" => Some("MOONSHOT_API_KEY"),
        _ => None,
    }
}

/// The scanner's failure reason: stderr first, falling back to stdout. Strix
/// often prints the real failure on stdout (or nothing at all), which used to
/// log as `scanner exited exit status: 2: ` with a blank reason.
///
/// Strix renders errors inside a box-drawn (rich) panel, so "the last line"
/// was the panel's bottom border and the stored error read `╰────╯`. The
/// reason is now the text INSIDE the last panel (see [`stream_failure_detail`]).
fn scanner_failure_detail(output: &std::process::Output) -> String {
    let stderr = stream_failure_detail(&String::from_utf8_lossy(&output.stderr));
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = stream_failure_detail(&String::from_utf8_lossy(&output.stdout));
    if !stdout.is_empty() {
        return stdout;
    }
    "(scanner printed nothing)".to_string()
}

/// Box-drawing characters rich uses for panel borders (square and rounded).
const PANEL_CHARS: &[char] = &[
    '│', '╭', '╮', '╰', '╯', '─', '┌', '┐', '└', '┘', '┃', '━', '┏', '┓', '┗', '┛', '║', '═', '╔',
    '╗', '╚', '╝',
];

/// Drop ANSI SGR/CSI escapes (`ESC [ ... final-byte`) in case the scanner
/// forced colour on a pipe.
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// A line's text with panel borders and surrounding whitespace removed.
fn panel_text(line: &str) -> String {
    line.trim()
        .trim_matches(|c: char| PANEL_CHARS.contains(&c) || c.is_whitespace())
        .to_string()
}

/// The failure reason inside one output stream.
///
/// If the stream contains a box-drawn panel, return the text inside the LAST
/// one: inner lines with the `│` borders stripped and blank lines dropped,
/// joined by a space, prefixed with the panel title when the top border has
/// one (`╭─ Error ─╮` → `Error: ...`). Otherwise (or for an empty panel), the
/// last line that is not pure border. Empty when the stream has no text.
fn stream_failure_detail(raw: &str) -> String {
    let lines: Vec<String> = raw.lines().map(strip_ansi).collect();
    const TOP: [char; 4] = ['╭', '┌', '┏', '╔'];
    const BOTTOM: [char; 4] = ['╰', '└', '┗', '╚'];
    fn starts_with(line: &str, set: &[char]) -> bool {
        line.trim_start().starts_with(set)
    }
    let bottom = lines.iter().rposition(|l| starts_with(l.as_str(), &BOTTOM));
    if let Some(bottom) = bottom {
        let top = lines[..bottom]
            .iter()
            .rposition(|l| starts_with(l.as_str(), &TOP));
        if let Some(top) = top {
            let inner: Vec<String> = lines[top + 1..bottom]
                .iter()
                .map(|l| panel_text(l))
                .filter(|l| !l.is_empty())
                .collect();
            if !inner.is_empty() {
                let body = inner.join(" ");
                let title = panel_text(&lines[top]);
                return if title.is_empty() {
                    body
                } else {
                    format!("{title}: {body}")
                };
            }
        }
    }
    lines
        .iter()
        .map(|l| panel_text(l))
        .rfind(|l| !l.is_empty())
        .unwrap_or_default()
}

/// Environment that switches off the scanner's own usage telemetry. Forced on
/// every run, local and remote (the remote export list is built from
/// [`scanner_env`], see [`remote_scan_exports`]).
const SCANNER_PRIVACY_ENV: &[(&str, &str)] = &[("STRIX_TELEMETRY", "false")];

/// Build the scanner's LLM environment from Permagent's own config/keychain,
/// so Guard setup never touches the launchd plist: the model rides
/// `strix_llm`, and the API key is the SAME provider secret the user already
/// stored for chat (looked up by the model string's provider prefix). If the
/// user exported STRIX_LLM/LLM_API_KEY themselves, those win — we only fill
/// what is absent.
///
/// It also always carries [`SCANNER_PRIVACY_ENV`]: strix ships PostHog/Scarf
/// usage beacons that are ON unless `STRIX_TELEMETRY=false`, and the Guard
/// must not phone a third party home just because the user enabled a scan.
fn scanner_env() -> Vec<(&'static str, String)> {
    let mut env: Vec<(&'static str, String)> = SCANNER_PRIVACY_ENV
        .iter()
        .map(|(k, v)| (*k, (*v).to_string()))
        .collect();
    let config = permagent::config::Config::global();
    let model = strix_model();
    if std::env::var("STRIX_LLM").is_err() {
        env.push(("STRIX_LLM", model.clone()));
    }
    if std::env::var("LLM_API_KEY").is_err() {
        let secret_key = strix_llm_secret_name(&model);
        if let Some(name) = secret_key {
            if let Ok(key) = config.get_secret::<String>(name) {
                env.push(("LLM_API_KEY", key));
            }
        }
    }
    env
}

/// Locate the `strix` CLI. The daemon runs under launchd with a bare PATH, so
/// a plain `Command::new("strix")` misses the places users actually install it
/// (pipx → ~/.local/bin, Homebrew → /opt/homebrew/bin). Falling back to those
/// turns "silently never scans" into "works after `pipx install strix-agent`".
fn resolve_strix_bin() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        let pipx = home.join(".local/bin/strix");
        if pipx.is_file() {
            return pipx;
        }
    }
    let brew = PathBuf::from("/opt/homebrew/bin/strix");
    if brew.is_file() {
        return brew;
    }
    PathBuf::from("strix")
}

/// Locate the `docker` CLI the same way `resolve_strix_bin` locates strix:
/// launchd's bare PATH misses the places Docker Desktop actually installs it.
fn resolve_docker_bin() -> PathBuf {
    for candidate in ["/usr/local/bin/docker", "/opt/homebrew/bin/docker"] {
        let p = PathBuf::from(candidate);
        if p.is_file() {
            return p;
        }
    }
    PathBuf::from("docker")
}

/// Is the strix binary an existing file? The pipx/brew fallbacks are absolute;
/// the bare-name fallback is searched on PATH.
fn strix_bin_present() -> bool {
    let bin = resolve_strix_bin();
    if bin.is_absolute() {
        return bin.is_file();
    }
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|d| d.join(&bin).is_file()))
        .unwrap_or(false)
}

const SCAN_INSTRUCTION: &str =
    "Static code analysis only. Do not run the application, do not send network \
     traffic to any host, and do not modify, create, or delete any files in the \
     target. Report findings; never remediate.";

const REMOTE_SCAN_DIR: &str = "permagent-strix-scans";
const SSH_CONNECT_SECS: u64 = 8;
const RSYNC_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// SSH target from config: `user@host` or a Host alias (`m1`). Rejects shell
/// metacharacters so it cannot become `host; rm -rf`.
fn validate_ssh_target(s: &str) -> Result<(), String> {
    let ok = !s.is_empty()
        && s.matches('@').count() <= 1
        && !s.starts_with('@')
        && !s.ends_with('@')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(format!("strix_docker_ssh is not a usable SSH target: {s}"))
    }
}

fn posix_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Reap leftover `strix-sandbox` containers after a killed scan. The sweep
/// is one project at a time, so this cannot take down a concurrent Guard
/// scan. Remote Colima needs DOCKER_HOST; local Docker Desktop does not.
const STRIX_SANDBOX_CLEANUP_REMOTE: &str = "export PATH=/opt/homebrew/bin:/usr/local/bin:$PATH; \
     export DOCKER_HOST=unix://$HOME/.colima/default/docker.sock; \
     ids=$(docker ps -a --format '{{.ID}} {{.Image}}' 2>/dev/null | awk '/strix-sandbox/ {print $1}'); \
     if [ -n \"$ids\" ]; then docker rm -f $ids; fi";

const STRIX_SANDBOX_CLEANUP_LOCAL: &str = "export PATH=/opt/homebrew/bin:/usr/local/bin:$PATH; \
     ids=$(docker ps -a --format '{{.ID}} {{.Image}}' 2>/dev/null | awk '/strix-sandbox/ {print $1}'); \
     if [ -n \"$ids\" ]; then docker rm -f $ids; fi";

fn ssh_base_args() -> Vec<String> {
    let mut args = vec![
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        format!("ConnectTimeout={SSH_CONNECT_SECS}"),
    ];
    if let Some(identity) = strix::docker_ssh_identity() {
        args.push("-i".into());
        args.push(identity);
    }
    args
}

fn ssh_command(ssh_target: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("ssh");
    for a in ssh_base_args() {
        cmd.arg(a);
    }
    cmd.arg(ssh_target);
    cmd
}

fn rsync_ssh_transport() -> String {
    let mut e = format!("ssh -o BatchMode=yes -o ConnectTimeout={SSH_CONNECT_SECS}");
    if let Some(identity) = strix::docker_ssh_identity() {
        e.push_str(" -i ");
        e.push_str(&posix_single_quote(&identity));
    }
    e
}

/// Where Strix ≥1.4 writes its per-run output, relative to the process's
/// working directory. Its own `--resume` help names it: "the dir under
/// ./strix_runs/".
const STRIX_RUNS_DIR: &str = "strix_runs";

/// Path-safe slug for one target: the local path flattened, bounded, and
/// stripped to characters that are safe in a shell word and a filename.
fn target_slug(target: &std::path::Path) -> String {
    let mut slug: String = target
        .to_string_lossy()
        .trim_start_matches('/')
        .chars()
        .map(|c| {
            if c == '/' || c.is_whitespace() {
                '_'
            } else {
                c
            }
        })
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if slug.is_empty() {
        slug = "scan".into();
    }
    const MAX_SLUG: usize = 80;
    if slug.len() > MAX_SLUG {
        slug.truncate(MAX_SLUG);
    }
    slug
}

/// Path-safe remote relative dir for the rsynced copy:
/// `permagent-strix-scans/<slug>`, one slash, slug bounded so a long local
/// path cannot blow the remote filesystem.
fn remote_scan_rel(target: &std::path::Path) -> String {
    format!("{REMOTE_SCAN_DIR}/{}", target_slug(target))
}

/// Where the remote scanner is `cd`'d before it runs, and therefore where its
/// `strix_runs/` output lands. Deliberately a SIBLING of the scanned copy, not
/// a child: the copy is rsynced with `--delete` at the top of every scan, so
/// output written inside it would be destroyed by the next sweep and would
/// also be handed to the scanner as part of its own target.
fn remote_runs_rel(target: &std::path::Path) -> String {
    format!("{REMOTE_SCAN_DIR}/.runs/{}", target_slug(target))
}

/// The daemon-owned scratch directory this scan's run output is read from.
/// Never inside the user's project: the Guard's own instruction to the scanner
/// is "do not modify, create, or delete any files in the target", and pulling
/// results into `<project>/.strix` broke that promise on every remote scan.
fn local_run_dir(target: &std::path::Path) -> PathBuf {
    std::env::temp_dir()
        .join("permagent-strix-runs")
        .join(target_slug(target))
}

/// Reset the scratch dir so `find_sarif`'s newest-run pick cannot land on a
/// previous sweep's SARIF when this sweep produced none.
fn fresh_local_run_dir(target: &std::path::Path) -> Result<PathBuf, String> {
    let dir = local_run_dir(target);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create scan run dir: {e}"))?;
    Ok(dir)
}

/// The remote scan command. Pure so the cwd/target split is testable without
/// an SSH host — and without putting the API key in `scanner_env` on a wire a
/// test can read.
fn remote_strix_command(exports: &str, scan_rel: &str, runs_rel: &str) -> String {
    format!(
        "{exports}cd \"$HOME/{runs_rel}\" && exec strix --target \"$HOME/{scan_rel}\" \
         --non-interactive --scan-mode standard --scope-mode full --instruction {instr}",
        instr = posix_single_quote(SCAN_INSTRUCTION),
    )
}

#[cfg(test)]
fn remote_home() -> String {
    match strix::docker_ssh_target() {
        Some(t) if t.contains('@') => {
            let user = t.split('@').next().unwrap_or("example-user");
            format!("/Users/{user}")
        }
        _ => "/Users/example-user".into(),
    }
}

#[cfg(test)]
fn remote_scan_abs(target: &std::path::Path) -> String {
    format!("{}/{}", remote_home(), remote_scan_rel(target))
}

async fn ssh_run(ssh_target: &str, remote: &str) -> Result<std::process::Output, String> {
    let mut cmd = ssh_command(ssh_target);
    cmd.arg(remote)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(20), cmd.output())
        .await
        .map_err(|_| format!("ssh {ssh_target} timed out"))?
        .map_err(|e| format!("ssh {ssh_target}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "ssh {ssh_target} exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .last()
                .unwrap_or_default()
        ));
    }
    Ok(output)
}

async fn cleanup_strix_sandboxes(ssh_target: Option<&str>) {
    if let Some(ssh) = ssh_target {
        if let Err(e) = ssh_run(ssh, STRIX_SANDBOX_CLEANUP_REMOTE).await {
            tracing::warn!(
                target: "permagentd::strix",
                "leftover strix sandbox cleanup failed: {e}"
            );
        }
        return;
    }
    let mut cmd = tokio::process::Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(STRIX_SANDBOX_CLEANUP_LOCAL)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    match tokio::time::timeout(Duration::from_secs(20), cmd.output()).await {
        Ok(Ok(output)) if output.status.success() => {}
        Ok(Ok(output)) => tracing::warn!(
            target: "permagentd::strix",
            "leftover strix sandbox cleanup exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ),
        Ok(Err(e)) => tracing::warn!(
            target: "permagentd::strix",
            "leftover strix sandbox cleanup failed: {e}"
        ),
        Err(_) => tracing::warn!(
            target: "permagentd::strix",
            "leftover strix sandbox cleanup timed out"
        ),
    }
}

async fn rsync_to_remote(target: &std::path::Path, ssh_target: &str) -> Result<(), String> {
    let rel = remote_scan_rel(target);
    ssh_run(ssh_target, &format!("mkdir -p \"$HOME/{rel}\"")).await?;
    let mut cmd = tokio::process::Command::new("rsync");
    cmd.args(["-a", "--delete"]);
    for exclude in strix::SCAN_EXCLUDES {
        cmd.arg(format!("--exclude={exclude}"));
    }
    cmd.args(["-e", &rsync_ssh_transport()])
        .arg(format!("{}/", target.display()))
        .arg(format!("{ssh_target}:{rel}/"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(RSYNC_TIMEOUT, cmd.output())
        .await
        .map_err(|_| "rsync to the scanner host timed out".to_string())?
        .map_err(|e| format!("rsync to the scanner host: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "rsync to the scanner host exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .last()
                .unwrap_or_default()
        ));
    }
    Ok(())
}

/// Pull the scanner's run output back into the daemon's own scratch dir.
async fn rsync_strix_back(
    runs_rel: &str,
    local: &std::path::Path,
    ssh_target: &str,
) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new("rsync");
    cmd.args(["-a", "-e", &rsync_ssh_transport()])
        .arg(format!("{ssh_target}:{runs_rel}/"))
        .arg(format!("{}/", local.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(RSYNC_TIMEOUT, cmd.output())
        .await
        .map_err(|_| "rsync of .strix back timed out".to_string())?
        .map_err(|e| format!("rsync of .strix back: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "rsync of .strix back exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .last()
                .unwrap_or_default()
        ));
    }
    Ok(())
}

fn preflight_briefing_detail(remote: Option<&str>) -> String {
    match remote {
        Some(host) => format!(
            "Sweeps are skipped until this is fixed. Docker and strix run on {host} \
             (Colima), not this Mac — a forwarded socket cannot bind-mount this \
             machine's paths. After a reboot there: ssh {host} \
             'PATH=/opt/homebrew/bin:$PATH colima start'. The loop retries on its \
             own; nothing to restart here."
        ),
        None => "Sweeps are skipped until this is fixed. `pipx install strix-agent` \
             installs the scanner; Docker Desktop must be installed and running. \
             The sweep loop retries automatically — nothing to restart."
            .to_string(),
    }
}

/// Dependency preflight (audit 2026-08-11): the Guard needs the `strix` CLI
/// AND a running Docker daemon. When `strix_docker_ssh` is set, those live on
/// the remote host — local `docker info` is the wrong question and was why
/// overnight sweeps silently skipped on a Mac that has no Docker by design.
async fn preflight() -> Result<(), String> {
    if let Some(ssh) = strix::docker_ssh_target() {
        return preflight_remote(&ssh).await;
    }
    preflight_local().await
}

/// Local preflight's Docker failure. Names the missing `strix_docker_ssh`
/// because that is the only way this path runs.
const LOCAL_DOCKER_DOWN: &str = "Docker is not running on this Mac (`docker info` failed) — \
     strix_docker_ssh is not set, so the Guard tried this Mac's Docker. Set \
     strix_docker_ssh to scan on a remote Docker host instead";

async fn preflight_local() -> Result<(), String> {
    let mut missing = Vec::new();
    if !strix_bin_present() {
        missing.push(
            "the `strix` scanner is not installed (fix: `pipx install strix-agent`)".to_string(),
        );
    }
    let mut cmd = tokio::process::Command::new(resolve_docker_bin());
    cmd.arg("info")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let docker_ok = matches!(
        tokio::time::timeout(Duration::from_secs(15), cmd.status()).await,
        Ok(Ok(status)) if status.success()
    );
    if !docker_ok {
        // Only reached when `strix_docker_ssh` is unset (see `preflight`):
        // say so, or a user whose scans are meant to run on another host
        // (and whose config lost that key) sees a baffling "Docker down".
        missing.push(LOCAL_DOCKER_DOWN.to_string());
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(missing.join("; "))
    }
}

async fn preflight_remote(ssh_target: &str) -> Result<(), String> {
    validate_ssh_target(ssh_target)?;
    let mut missing = Vec::new();
    if let Err(e) = ssh_run(ssh_target, "true").await {
        return Err(format!(
            "cannot SSH to {ssh_target} ({e}); Guard scans on that host, not this Mac"
        ));
    }
    let docker_check = "export PATH=/opt/homebrew/bin:$PATH; \
         export DOCKER_HOST=unix://$HOME/.colima/default/docker.sock; \
         docker info >/dev/null";
    if ssh_run(ssh_target, docker_check).await.is_err() {
        missing.push(format!(
            "Docker is not running on {ssh_target} (Colima). After a reboot: \
             ssh {ssh_target} 'PATH=/opt/homebrew/bin:$PATH colima start'"
        ));
    }
    let strix_check = "export PATH=/opt/homebrew/bin:$HOME/.local/bin:$PATH; \
         test -x \"$HOME/.local/bin/strix\" || command -v strix >/dev/null";
    if ssh_run(ssh_target, strix_check).await.is_err() {
        missing.push(format!(
            "the `strix` scanner is not installed on {ssh_target} \
             (fix: ssh {ssh_target} 'pipx install strix-agent')"
        ));
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(missing.join("; "))
    }
}

/// Config stamp: the preflight failure that has already been briefed. One
/// briefing per CONDITION, not one per 15-minute tick — a new, different
/// failure briefs again; a preflight that recovers clears the stamp so a
/// future breakage is news again.
const PREFLIGHT_BRIEFED_KEY: &str = "strix_preflight_briefed";

/// Pure dedupe gate for the preflight briefing.
fn preflight_should_brief(previously_briefed: Option<&str>, failure: &str) -> bool {
    previously_briefed != Some(failure)
}

/// File the one-per-condition preflight briefing. Returns true when a briefing
/// was actually filed — the caller persists the stamp only then, so a failed
/// DB write retries at the next tick instead of going silent forever.
async fn report_preflight_failure(
    pool: &Pool<Sqlite>,
    previously_briefed: Option<&str>,
    failure: &str,
) -> bool {
    if !preflight_should_brief(previously_briefed, failure) {
        return false;
    }
    permagent::briefings::file_briefing(
        pool,
        permagent::briefings::NewBriefing {
            from_agent: strix::STRIX_FEATURE_ID.to_string(),
            kind: "preflight_failed".to_string(),
            severity: permagent::briefings::Severity::ActionRequired,
            summary: format!("The Guard is enabled but cannot run: {failure}"),
            detail: Some(preflight_briefing_detail(
                strix::docker_ssh_target().as_deref(),
            )),
            ref_kind: None,
            ref_id: None,
        },
    )
    .await
    .is_some()
}

/// Kill a timed-out scan and everything it started.
///
/// SIGTERM first — deliberately unlike the goal engine's straight SIGKILL —
/// because strix owns Docker containers that only its own handler can remove;
/// a SIGKILLed scanner leaves them running. Then SIGKILL the whole group so a
/// scanner that ignores the term still goes, along with its tool subprocesses.
#[cfg(unix)]
async fn kill_scan_tree(pid: u32) {
    let group = -(pid as i32);
    // SAFETY: signalling a process group by pid; an already-dead group is a
    // harmless ESRCH.
    unsafe { libc::kill(group, libc::SIGTERM) };
    tokio::time::sleep(SCAN_TERM_GRACE).await;
    unsafe { libc::kill(group, libc::SIGKILL) };
}

#[cfg(not(unix))]
async fn kill_scan_tree(_pid: u32) {}

/// Wait for the scan under its `SCAN_TIMEOUT` bound, aborting it early if the
/// sovereignty toggle flips mid-flight. Without the re-check, turning sovereign
/// mode on during a scan left up to `SCAN_TIMEOUT` — twenty minutes — of a
/// cloud model still reading the user's source, because the flag was only ever
/// read at the top of the sweep. `sovereign` and `poll` are injected so the
/// abort path is testable without process-global config or a 30-second wait.
async fn wait_supervised(
    child: tokio::process::Child,
    sovereign: impl Fn() -> bool,
    poll: Duration,
) -> Result<std::process::Output, String> {
    let pid = child.id();
    let deadline = tokio::time::Instant::now() + SCAN_TIMEOUT;
    let mut wait = std::pin::pin!(child.wait_with_output());
    loop {
        tokio::select! {
            result = &mut wait => return result.map_err(|e| e.to_string()),
            _ = tokio::time::sleep_until(deadline) => {
                if let Some(pid) = pid {
                    kill_scan_tree(pid).await;
                }
                return Err(format!(
                    "scan exceeded its {}-minute bound (scanner killed; leftover strix \
                     sandbox containers were reaped if any remained)",
                    SCAN_TIMEOUT.as_secs() / 60
                ));
            }
            _ = tokio::time::sleep(poll) => {
                if sovereign() {
                    if let Some(pid) = pid {
                        kill_scan_tree(pid).await;
                    }
                    return Err(
                        "scan aborted mid-flight: sovereign mode was turned on (scanner killed)"
                            .to_string(),
                    );
                }
            }
        }
    }
}

async fn scan_project(target: &std::path::Path) -> Result<Vec<Finding>, String> {
    if !strix::has_scannable_files(target) {
        return Err("no scannable files".into());
    }
    if let Some(ssh) = strix::docker_ssh_target() {
        tracing::info!(
            target: "permagentd::strix",
            host = %ssh,
            "scanning on the remote Docker host (rsync + strix there, .strix back)"
        );
        return scan_project_remote(target, &ssh).await;
    }
    scan_project_local(target).await
}

/// The `export ...;` prefix for the remote (strix_docker_ssh) scan: PATH and
/// the Colima DOCKER_HOST, then every scanner env var single-quoted — which
/// includes `STRIX_TELEMETRY=false` so the M1 run is as quiet as a local one.
fn remote_scan_exports(env: &[(&'static str, String)]) -> String {
    let mut exports = String::from(
        "export PATH=/opt/homebrew/bin:$HOME/.local/bin:/usr/bin:/bin; \
         export DOCKER_HOST=unix://$HOME/.colima/default/docker.sock; ",
    );
    for (k, v) in env {
        if *k == "DOCKER_HOST" {
            continue;
        }
        exports.push_str(&format!("export {k}={}; ", posix_single_quote(v)));
    }
    exports
}

async fn scan_project_remote(
    target: &std::path::Path,
    ssh_target: &str,
) -> Result<Vec<Finding>, String> {
    validate_ssh_target(ssh_target)?;
    let local_runs = fresh_local_run_dir(target)?;
    rsync_to_remote(target, ssh_target).await?;
    let rel = remote_scan_rel(target);
    let runs_rel = remote_runs_rel(target);
    // Clear the remote run dir too. Without this, m1 accumulated a run
    // directory per attempt forever (13 of them, half a megabyte of log
    // apiece, by 2026-08-31) and `find_sarif`'s newest-wins pick could return
    // a previous sweep's report for a scan that produced none.
    ssh_run(
        ssh_target,
        &format!("rm -rf \"$HOME/{runs_rel}\" && mkdir -p \"$HOME/{runs_rel}\""),
    )
    .await?;
    let exports = remote_scan_exports(&scanner_env());
    let remote = remote_strix_command(&exports, &rel, &runs_rel);
    let mut cmd = ssh_command(ssh_target);
    cmd.arg(remote)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    permagent::subprocess::configure_subprocess(&mut cmd);

    let child = cmd
        .spawn()
        .map_err(|e| format!("ssh to {ssh_target} is not runnable ({e})"))?;
    let output = wait_supervised(
        child,
        permagent::sovereignty::global_sovereign_mode,
        SOVEREIGN_POLL,
    )
    .await;
    if output.is_err() {
        let _ = ssh_run(
            ssh_target,
            &format!("pkill -f {} || true", posix_single_quote(&rel)),
        )
        .await;
        cleanup_strix_sandboxes(Some(ssh_target)).await;
    }
    let output = output?;
    if !strix::scanner_completed(output.status) {
        return Err(strix::classify_scanner_failure(
            output.status,
            &scanner_failure_detail(&output),
        ));
    }
    rsync_strix_back(&runs_rel, &local_runs, ssh_target).await?;
    let sarif =
        find_sarif(&local_runs).ok_or_else(|| "scan produced no findings.sarif".to_string())?;
    let raw = std::fs::read_to_string(&sarif).map_err(|e| e.to_string())?;
    parse_sarif(&raw)
}

async fn scan_project_local(target: &std::path::Path) -> Result<Vec<Finding>, String> {
    let local_runs = fresh_local_run_dir(target)?;
    let mut cmd = tokio::process::Command::new(resolve_strix_bin());
    // The scanner writes `strix_runs/<run>/findings.sarif` relative to its own
    // cwd, so the cwd is what decides where the report lands — and under
    // launchd the daemon's cwd is not somewhere it may write.
    cmd.current_dir(&local_runs);
    for (k, v) in scanner_env() {
        cmd.env(k, v);
    }
    // Posture: the scope guard is code; the read-only posture rides the
    // engine's instruction channel because the external CLI has no passive
    // flag — its --scan-mode is depth (quick/standard/deep), not intrusiveness.
    // `standard` + `full` scope: a recurring whole-project sweep, not a
    // CI diff check and not an open-ended deep engagement per tick.
    cmd.arg("--target")
        .arg(target)
        .arg("--non-interactive")
        .arg("--scan-mode")
        .arg("standard")
        .arg("--scope-mode")
        .arg("full")
        .arg("--instruction")
        .arg(SCAN_INSTRUCTION)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // The scanner is an agentic CLI that drives a cloud model on the user's own
    // key and starts Docker sandboxes. Tokio does NOT kill a spawned process
    // when its handle drops, and `wait_with_output` consumes the handle — so a
    // timeout used to leave a process nobody owned, still spending, still
    // egressing, outliving the daemon that started it and deaf to the
    // sovereignty toggle. Its own process group so the kill reaches the tools
    // it spawned, and `kill_on_drop` as the backstop for any other drop path.
    cmd.kill_on_drop(true);
    permagent::subprocess::configure_subprocess(&mut cmd);

    let child = cmd.spawn().map_err(|e| {
        format!("`strix` is not runnable ({e}) — install it and Docker to enable sweeps")
    })?;
    let output = wait_supervised(
        child,
        permagent::sovereignty::global_sovereign_mode,
        SOVEREIGN_POLL,
    )
    .await;
    if output.is_err() {
        cleanup_strix_sandboxes(None).await;
    }
    let output = output?;
    if !strix::scanner_completed(output.status) {
        return Err(strix::classify_scanner_failure(
            output.status,
            &scanner_failure_detail(&output),
        ));
    }
    let sarif =
        find_sarif(&local_runs).ok_or_else(|| "scan produced no findings.sarif".to_string())?;
    let raw = std::fs::read_to_string(&sarif).map_err(|e| e.to_string())?;
    parse_sarif(&raw)
}

/// Locate the run's `findings.sarif` under `root`. The engine writes per-run
/// directories; the newest one wins.
///
/// Two layouts are searched because the engine moved its output: Strix ≥1.4
/// writes `<cwd>/strix_runs/<run-name>/findings.sarif` (its own `--resume`
/// help calls it "the dir under ./strix_runs/"), while older builds wrote
/// `<target>/.strix/runs/<run>/findings.sarif`. Looking only in the old place
/// meant a scan that ran to completion still ended as "scan produced no
/// findings.sarif".
fn find_sarif(root: &std::path::Path) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for runs in [root.join(STRIX_RUNS_DIR), root.join(".strix").join("runs")] {
        let Ok(entries) = std::fs::read_dir(runs) else {
            continue;
        };
        for entry in entries.flatten() {
            let candidate = entry.path().join("findings.sarif");
            if !candidate.is_file() {
                continue;
            }
            let modified = candidate
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            if best.as_ref().is_none_or(|(t, _)| modified > *t) {
                best = Some((modified, candidate));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// Parse SARIF 2.1.0 into findings. Tolerant by design: a shape we don't
/// recognise yields no findings rather than an error, because a scanner
/// upgrade must never take the sweep loop down.
pub fn parse_sarif(raw: &str) -> Result<Vec<Finding>, String> {
    let doc: serde_json::Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut out = Vec::new();

    let runs = doc.get("runs").and_then(|r| r.as_array());
    for run in runs.into_iter().flatten() {
        // Rule metadata carries the CWE and the remediation text.
        let rules = run
            .pointer("/tool/driver/rules")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();
        for result in run
            .get("results")
            .and_then(|r| r.as_array())
            .into_iter()
            .flatten()
        {
            let rule_id = result
                .get("ruleId")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let rule = rules
                .iter()
                .find(|r| r.get("id").and_then(|v| v.as_str()) == Some(rule_id.as_str()));
            let title = result
                .pointer("/message/text")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    rule.and_then(|r| r.pointer("/shortDescription/text"))
                        .and_then(|v| v.as_str())
                })
                .unwrap_or("Unnamed finding")
                .to_string();
            // SARIF `level` is warning/error/note; map to the severity words
            // the checklist speaks.
            let severity = match result
                .get("level")
                .and_then(|v| v.as_str())
                .unwrap_or("warning")
            {
                "error" => "high",
                "note" => "low",
                _ => "medium",
            }
            .to_string();
            let location = result
                .pointer("/locations/0/physicalLocation/artifactLocation/uri")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let remediation = rule
                .and_then(|r| r.pointer("/help/text"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let cwe = if rule_id.to_ascii_uppercase().starts_with("CWE-") {
                Some(rule_id.to_ascii_uppercase())
            } else {
                None
            };
            out.push(Finding {
                id: format!("{rule_id}:{}", location.clone().unwrap_or_default()),
                title,
                severity,
                cwe,
                location,
                remediation,
                found_at: now.clone(),
                ..Default::default()
            });
        }
    }
    Ok(out)
}

/// Tell Henry only when it matters: new findings at medium severity or above.
/// An unchanged finding set stays silent, and low-only noise never briefs.
async fn brief_new_findings(pool: &Pool<Sqlite>, project: &Project, fresh: &[Finding]) {
    let serious = fresh.iter().filter(|f| f.severity != "low").count();
    if serious == 0 {
        return;
    }
    permagent::briefings::file_briefing(
        pool,
        permagent::briefings::NewBriefing {
            from_agent: strix::STRIX_FEATURE_ID.to_string(),
            kind: "security_findings".to_string(),
            severity: permagent::briefings::Severity::Attention,
            summary: format!(
                "{serious} new security finding{} on {}",
                if serious == 1 { "" } else { "s" },
                project.name
            ),
            detail: Some(
                "Open the project's Overview for the checklist — each item carries its \
                 severity, CWE, location, and how to fix it."
                    .to_string(),
            ),
            ref_kind: Some("project".to_string()),
            ref_id: Some(project.id.clone()),
        },
    )
    .await;
}

/// One briefing when a rescan confirms fixes — the payoff of the whole loop.
async fn brief_confirmed_fixed(pool: &Pool<Sqlite>, project: &Project, fixed: &[Finding]) {
    if fixed.is_empty() {
        return;
    }
    let n = fixed.len();
    permagent::briefings::file_briefing(
        pool,
        permagent::briefings::NewBriefing {
            from_agent: strix::STRIX_FEATURE_ID.to_string(),
            kind: "security_fixed".to_string(),
            severity: permagent::briefings::Severity::Info,
            summary: format!(
                "{n} security finding{} confirmed fixed on {}",
                if n == 1 { "" } else { "s" },
                project.name
            ),
            detail: Some(format!(
                "Each had a verified fix commit and was absent from the latest scan: {}.",
                finding_titles(fixed)
            )),
            ref_kind: Some("project".to_string()),
            ref_id: Some(project.id.clone()),
        },
    )
    .await;
}

/// One briefing when findings that were fixed or unconfirmed are reported again.
async fn brief_reopened(pool: &Pool<Sqlite>, project: &Project, reopened: &[Finding]) {
    if reopened.is_empty() {
        return;
    }
    let n = reopened.len();
    permagent::briefings::file_briefing(
        pool,
        permagent::briefings::NewBriefing {
            from_agent: strix::STRIX_FEATURE_ID.to_string(),
            kind: "security_reopened".to_string(),
            severity: permagent::briefings::Severity::Attention,
            summary: format!(
                "{n} security finding{} came back on {}",
                if n == 1 { "" } else { "s" },
                project.name
            ),
            detail: Some(format!(
                "The latest scan reported these again after they had been fixed or left \
                 unconfirmed: {}.",
                finding_titles(reopened)
            )),
            ref_kind: Some("project".to_string()),
            ref_id: Some(project.id.clone()),
        },
    )
    .await;
}

fn finding_titles(findings: &[Finding]) -> String {
    let mut titles: Vec<&str> = findings.iter().take(5).map(|f| f.title.as_str()).collect();
    if findings.len() > 5 {
        titles.push("and more");
    }
    titles.join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sarif_results_with_rule_metadata() {
        let raw = r#"{
          "runs": [{
            "tool": {"driver": {"rules": [
              {"id": "CWE-89", "shortDescription": {"text": "SQL injection"},
               "help": {"text": "Use parameterised queries."}}
            ]}},
            "results": [{
              "ruleId": "CWE-89",
              "level": "error",
              "message": {"text": "Unsanitised input reaches a query"},
              "locations": [{"physicalLocation": {"artifactLocation": {"uri": "src/db.rs"}}}]
            }]
          }]
        }"#;
        let findings = parse_sarif(raw).unwrap();
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.severity, "high");
        assert_eq!(f.cwe.as_deref(), Some("CWE-89"));
        assert_eq!(f.location.as_deref(), Some("src/db.rs"));
        assert_eq!(f.remediation.as_deref(), Some("Use parameterised queries."));
        assert_eq!(f.title, "Unsanitised input reaches a query");
    }

    #[test]
    fn unknown_sarif_shapes_yield_no_findings_not_an_error() {
        // A scanner upgrade must never take the sweep loop down.
        assert!(parse_sarif(r#"{"runs": []}"#).unwrap().is_empty());
        assert!(parse_sarif(r#"{"version": "2.1.0"}"#).unwrap().is_empty());
        assert!(parse_sarif("not json").is_err());
    }

    fn finding(id: &str, severity: &str) -> Finding {
        Finding {
            id: id.to_string(),
            title: "t".to_string(),
            severity: severity.to_string(),
            cwe: None,
            location: None,
            remediation: None,
            found_at: "2026-08-05T00:00:00Z".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn preflight_brief_gate_is_once_per_condition() {
        assert!(preflight_should_brief(None, "docker down"));
        assert!(!preflight_should_brief(Some("docker down"), "docker down"));
        assert!(preflight_should_brief(Some("docker down"), "strix missing"));
    }

    #[test]
    fn strix_llm_secret_covers_openai_and_deepseek() {
        assert_eq!(
            strix_llm_secret_name("openai/gpt-5.4-mini"),
            Some("OPENAI_API_KEY")
        );
        assert_eq!(
            strix_llm_secret_name("deepseek/deepseek-chat"),
            Some("DEEPSEEK_API_KEY")
        );
        assert_eq!(
            strix_llm_secret_name("custom_deepseek/deepseek-chat"),
            Some("DEEPSEEK_API_KEY")
        );
        assert_eq!(strix_llm_secret_name("unknown/model"), None);
        assert_eq!(DEFAULT_STRIX_LLM, "openai/gpt-5.4-mini");
    }

    #[test]
    fn scanner_failure_prefers_stderr_then_stdout() {
        let with_stderr = std::process::Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(2 << 8),
            stdout: b"noise\n".to_vec(),
            stderr: b"\nAuthentication Error, Model not found\n".to_vec(),
        };
        assert_eq!(
            scanner_failure_detail(&with_stderr),
            "Authentication Error, Model not found"
        );
        let stdout_only = std::process::Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(2 << 8),
            stdout: b"LiteLLM: 404 model not found\n".to_vec(),
            stderr: b"\n".to_vec(),
        };
        assert_eq!(
            scanner_failure_detail(&stdout_only),
            "LiteLLM: 404 model not found"
        );
        let empty = std::process::Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(2 << 8),
            stdout: b"".to_vec(),
            stderr: b"".to_vec(),
        };
        assert_eq!(scanner_failure_detail(&empty), "(scanner printed nothing)");
    }

    /// Strix prints its errors inside a rich panel. The stored reason was the
    /// bottom border (`╰────╯`); it must be the text inside the panel.
    #[test]
    fn scanner_failure_reads_inside_the_last_box_drawn_panel() {
        let stdout = "\
Strix v0.4 starting scan of /Users/me/project
╭──────────────────────────── STRIX ─────────────────────────────╮
│                                                                │
│  Scan started                                                  │
│                                                                │
╰────────────────────────────────────────────────────────────────╯
Some progress line
╭──────────────────────────── Error ─────────────────────────────╮
│                                                                │
│  LLM CONNECTION FAILED                                         │
│  litellm.RateLimitError: OpenAIException - Rate limit reached  │
│  for gpt-5.4-mini on tokens per min (TPM)                      │
│                                                                │
╰────────────────────────────────────────────────────────────────╯
";
        let out = std::process::Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(1 << 8),
            stdout: stdout.as_bytes().to_vec(),
            stderr: b"\n".to_vec(),
        };
        assert_eq!(
            scanner_failure_detail(&out),
            "Error: LLM CONNECTION FAILED litellm.RateLimitError: OpenAIException - \
             Rate limit reached for gpt-5.4-mini on tokens per min (TPM)"
        );

        // Untitled panel, ANSI colour forced on, on stderr.
        let stderr = "\x1b[31m╭──────────╮\x1b[0m\n│ \x1b[1mboom\x1b[0m     │\n╰──────────╯\n";
        assert_eq!(stream_failure_detail(stderr), "boom");

        // A panel with nothing inside falls back to the last non-border line.
        assert_eq!(
            stream_failure_detail("real reason\n╭────╮\n│    │\n╰────╯\n"),
            "real reason"
        );
        // A stream that is only borders has no reason; stdout is used next.
        let borders_only = std::process::Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(1 << 8),
            stdout: b"docker: permission denied\n".to_vec(),
            stderr: "╭──╮\n╰──╯\n".as_bytes().to_vec(),
        };
        assert_eq!(
            scanner_failure_detail(&borders_only),
            "docker: permission denied"
        );
    }

    #[test]
    fn scanner_telemetry_is_off_locally_and_on_the_remote_host() {
        assert!(SCANNER_PRIVACY_ENV.contains(&("STRIX_TELEMETRY", "false")));
        let env: Vec<(&'static str, String)> = SCANNER_PRIVACY_ENV
            .iter()
            .map(|(k, v)| (*k, (*v).to_string()))
            .chain([
                ("STRIX_LLM", "openai/x".to_string()),
                ("DOCKER_HOST", "tcp://evil".to_string()),
            ])
            .collect();
        let exports = remote_scan_exports(&env);
        assert!(exports.contains("export STRIX_TELEMETRY='false'; "));
        assert!(exports.contains("export STRIX_LLM='openai/x'; "));
        assert!(exports.contains("export DOCKER_HOST=unix://$HOME/.colima/default/docker.sock; "));
        assert!(!exports.contains("tcp://evil"));
    }

    #[test]
    fn local_docker_failure_names_the_unset_ssh_target() {
        assert!(LOCAL_DOCKER_DOWN.contains("strix_docker_ssh is not set"));
        assert!(LOCAL_DOCKER_DOWN.contains("this Mac's Docker"));
    }

    /// A daemon restart must not reset the sweep clock (12 sweeps in 3h on
    /// 2026-09-20): the newest stamp across projects is the last sweep.
    #[test]
    fn sweep_clock_is_seeded_from_the_newest_attempt_stamp() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-20T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let metas = [
            serde_json::json!({ LAST_ATTEMPT_KEY: "2026-09-20T09:00:00+00:00" }),
            serde_json::json!({ LAST_ATTEMPT_KEY: "2026-09-20T11:30:00+00:00",
                                LAST_SCAN_KEY: "2026-09-19T00:00:00+00:00" }),
            // Pre-attempt-key metadata falls back to last_scan.
            serde_json::json!({ LAST_SCAN_KEY: "2026-09-20T10:00:00+00:00" }),
            serde_json::json!({}),
            serde_json::json!({ LAST_ATTEMPT_KEY: "not a time" }),
        ];
        assert_eq!(
            time_since_last_sweep(metas.iter(), now),
            Some(Duration::from_secs(30 * 60))
        );
        // Clock skew: a future stamp reads as "just swept", never negative.
        let future = [serde_json::json!({ LAST_ATTEMPT_KEY: "2026-09-20T13:00:00Z" })];
        assert_eq!(
            time_since_last_sweep(future.iter(), now),
            Some(Duration::ZERO)
        );
        // Never swept: no seed, so the loop's normal first-sweep path runs.
        let none = [serde_json::json!({}), serde_json::json!({ "other": 1 })];
        assert_eq!(time_since_last_sweep(none.iter(), now), None);
    }

    /// Preflight failure files ONE briefing, not one per 15-minute tick — and
    /// a DIFFERENT failure is news, so it briefs again.
    #[tokio::test]
    async fn preflight_failure_briefs_once_not_every_tick() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        permagent::session::spectral_schema::apply_briefings_schema(&pool)
            .await
            .unwrap();

        let mut stamp: Option<String> = None;
        for _ in 0..4 {
            if report_preflight_failure(&pool, stamp.as_deref(), "docker down").await {
                stamp = Some("docker down".to_string());
            }
        }
        assert_eq!(
            permagent::briefings::unacknowledged_count(&pool).await,
            1,
            "the same broken condition must brief exactly once"
        );

        assert!(report_preflight_failure(&pool, stamp.as_deref(), "strix missing").await);
        assert_eq!(permagent::briefings::unacknowledged_count(&pool).await, 2);
    }

    /// A timed-out scan must take its whole tree with it. Tokio keeps a spawned
    /// process alive when its handle drops, and `wait_with_output` consumes the
    /// handle — so before this the timeout left an agentic scanner running on
    /// the user's own API key, spending and egressing, owned by nobody.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_timed_out_scan_takes_its_subprocesses_with_it() {
        use tokio::process::Command;

        // A leader that outlives a naive kill plus a grandchild in its group —
        // the tool subprocesses a real scan spawns.
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c")
            .arg("sleep 300 & echo $! > /dev/null; sleep 300")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        cmd.kill_on_drop(true);
        permagent::subprocess::configure_subprocess(&mut cmd);

        let mut child = cmd.spawn().expect("/bin/sh spawns");
        let pid = child.id().expect("a freshly spawned child has a pid");

        // The flag the kill depends on: its own group, so the signal reaches
        // everything the scanner started rather than only the leader.
        // SAFETY: reading the process group of a live child.
        assert_eq!(
            unsafe { libc::getpgid(pid as i32) },
            pid as i32,
            "the scanner must lead its own process group"
        );

        kill_scan_tree(pid).await;

        // Assert through `wait` rather than `kill(pid, 0)`: a SIGKILLed leader
        // is still a group member until it is reaped, so a liveness probe
        // would flake.
        let status = tokio::time::timeout(std::time::Duration::from_secs(10), child.wait())
            .await
            .expect("the killed scanner is reaped well inside its 300s sleep")
            .expect("wait succeeds");
        assert!(
            !status.success(),
            "the scanner was signalled, not left to finish"
        );
    }

    /// The toggle has to reach a scan already in flight: the sweep reads the
    /// flag once, and a scan runs for up to twenty minutes on the user's source
    /// against a cloud model. A stand-in long-running child proves the
    /// supervision aborts rather than waiting out the bound.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_sovereign_flip_aborts_an_in_flight_scan() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use tokio::process::Command;

        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c")
            .arg("sleep 60")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.kill_on_drop(true);
        permagent::subprocess::configure_subprocess(&mut cmd);
        let child = cmd.spawn().expect("/bin/sh spawns");

        let probe = Arc::new(AtomicBool::new(false));
        let flip = Arc::clone(&probe);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            flip.store(true, Ordering::SeqCst);
        });

        let result = tokio::time::timeout(
            Duration::from_secs(10),
            wait_supervised(
                child,
                || probe.load(Ordering::SeqCst),
                Duration::from_millis(20),
            ),
        )
        .await
        .expect("the abort must not wait out the 20-minute scan bound");
        assert!(
            result.unwrap_err().contains("sovereign mode"),
            "the scan must end naming the sovereignty flip, not the timeout"
        );
    }

    #[test]
    fn sarif_is_found_where_strix_actually_writes_it() {
        // Strix 1.4.1 writes `<cwd>/strix_runs/<run-name>/findings.sarif`
        // (`--resume` help: "the dir under ./strix_runs/"). Live proof on m1,
        // 2026-08-31: ten `~/strix_runs/*/findings.sarif`, and not one
        // `.strix/` directory anywhere under `~/permagent-strix-scans`. The
        // daemon was looking in `<target>/.strix/runs` — so even a scan that
        // exited 0 would have died on "scan produced no findings.sarif".
        let tmp = tempfile::tempdir().unwrap();
        let run = tmp.path().join(STRIX_RUNS_DIR).join("some-run_1a2b");
        std::fs::create_dir_all(&run).unwrap();
        std::fs::write(run.join("findings.sarif"), "{}").unwrap();
        assert_eq!(
            find_sarif(tmp.path()),
            Some(run.join("findings.sarif")),
            "the current run layout must be found"
        );

        // Older builds wrote `<root>/.strix/runs/<run>/findings.sarif`; a user
        // who has not upgraded the scanner must not silently stop completing.
        let legacy_root = tempfile::tempdir().unwrap();
        let legacy = legacy_root.path().join(".strix").join("runs").join("older");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("findings.sarif"), "{}").unwrap();
        assert_eq!(
            find_sarif(legacy_root.path()),
            Some(legacy.join("findings.sarif")),
            "the legacy run layout must still be found"
        );

        assert_eq!(find_sarif(tempfile::tempdir().unwrap().path()), None);
    }

    #[test]
    fn remote_scan_writes_its_runs_outside_the_tree_it_is_scanning() {
        let target = std::path::Path::new("/Users/j/Documents/dev/permagent-runtime");
        let scan = remote_scan_rel(target);
        let runs = remote_runs_rel(target);
        assert_ne!(scan, runs);
        assert!(
            !runs.starts_with(&format!("{scan}/")),
            "run output inside the scanned copy would be rsynced away by --delete \
             and fed back to the scanner: {runs}"
        );
        let cmd = remote_strix_command("export STRIX_LLM='x'; ", &scan, &runs);
        assert!(
            cmd.contains(&format!("cd \"$HOME/{runs}\"")),
            "the scanner's cwd decides where strix_runs/ lands: {cmd}"
        );
        assert!(cmd.contains(&format!("--target \"$HOME/{scan}\"")));
        assert!(cmd.contains("--non-interactive"));
        assert!(cmd.contains("export STRIX_LLM="));
    }

    #[test]
    fn remote_scan_slug_is_path_safe_and_bounded() {
        let rel = remote_scan_rel(std::path::Path::new(
            "/Users/j/Documents/dev/permagent-runtime",
        ));
        assert_eq!(
            rel,
            "permagent-strix-scans/Users_j_Documents_dev_permagent-runtime"
        );
        assert_eq!(
            remote_scan_abs(std::path::Path::new("/Users/j/app")),
            "/Users/example-user/permagent-strix-scans/Users_j_app"
        );
        let long = format!("/{}", "a".repeat(200));
        let rel = remote_scan_rel(std::path::Path::new(&long));
        assert!(rel.starts_with("permagent-strix-scans/"));
        assert!(rel.len() <= "permagent-strix-scans/".len() + 80);
        assert_eq!(rel.matches('/').count(), 1);
    }

    #[test]
    fn ssh_target_rejects_shell_metacharacters() {
        assert!(validate_ssh_target("example-user@m1").is_ok());
        assert!(validate_ssh_target("m1").is_ok());
        assert!(validate_ssh_target("user@host.example").is_ok());
        assert!(validate_ssh_target("example-user@m1; rm -rf /").is_err());
        assert!(validate_ssh_target("host$(reboot)").is_err());
        assert!(validate_ssh_target("").is_err());
        assert!(validate_ssh_target("@m1").is_err());
    }

    #[test]
    fn sandbox_cleanup_targets_only_strix_images() {
        for script in [STRIX_SANDBOX_CLEANUP_REMOTE, STRIX_SANDBOX_CLEANUP_LOCAL] {
            assert!(
                script.contains("strix-sandbox"),
                "must filter by the sandbox image family"
            );
            assert!(
                script.contains("docker rm -f"),
                "a SIGKILLed scanner leaves them running"
            );
            assert!(
                !script.contains("docker rm -f $(docker ps"),
                "must not rm every container on the host"
            );
        }
        assert!(STRIX_SANDBOX_CLEANUP_REMOTE.contains("colima/default/docker.sock"));
        assert!(!STRIX_SANDBOX_CLEANUP_LOCAL.contains("DOCKER_HOST"));
    }

    #[test]
    fn preflight_briefing_names_the_remote_host_when_configured() {
        let remote = preflight_briefing_detail(Some("example-user@m1"));
        assert!(remote.contains("example-user@m1"));
        assert!(remote.contains("Colima"));
        assert!(!remote.contains("Docker Desktop"));
        let local = preflight_briefing_detail(None);
        assert!(local.contains("Docker Desktop"));
    }

    #[test]
    fn rotation_prefers_last_attempt_over_last_scan() {
        let mut meta = serde_json::Map::new();
        meta.insert(
            LAST_SCAN_KEY.to_string(),
            serde_json::Value::String("2026-08-11T00:00:00Z".into()),
        );
        meta.insert(
            LAST_ATTEMPT_KEY.to_string(),
            serde_json::Value::String("2026-08-21T01:19:00Z".into()),
        );
        assert_eq!(
            rotation_stamp_from_meta(&serde_json::Value::Object(meta)),
            "2026-08-21T01:19:00Z"
        );
        let mut only_scan = serde_json::Map::new();
        only_scan.insert(
            LAST_SCAN_KEY.to_string(),
            serde_json::Value::String("2026-08-11T00:00:00Z".into()),
        );
        assert_eq!(
            rotation_stamp_from_meta(&serde_json::Value::Object(only_scan)),
            "2026-08-11T00:00:00Z"
        );
    }

    fn fake_project(name: &str) -> Project {
        Project {
            id: "p1".into(),
            user_id: "u1".into(),
            slug: name.to_lowercase(),
            name: name.into(),
            description: String::new(),
            status: "active".into(),
            root_path: Some("/tmp/p1".into()),
            site_url: None,
            repo_url: None,
            notes: String::new(),
            metadata_json: serde_json::json!({}),
            graph_entity_id: None,
            tags: Vec::new(),
            created_at: String::new(),
            updated_at: String::new(),
            last_opened_at: String::new(),
        }
    }

    /// A per-project scan failure had NO surface — only `tracing::warn` — which
    /// is how twenty days of zero completed scans looked healthy. It must brief
    /// on the transition into failure, and only on the transition: a project
    /// that fails every day for a fortnight must not file a briefing a day.
    #[test]
    fn scan_failure_briefs_on_the_transition_not_every_tick() {
        assert!(
            scan_failure_should_brief(0),
            "the first failure after a completed scan is the news"
        );
        for streak in 1..=20u64 {
            assert!(
                !scan_failure_should_brief(streak),
                "failure #{} must be counted, not re-announced",
                streak + 1
            );
        }
    }

    #[tokio::test]
    async fn scan_failure_briefing_names_the_project_and_the_error() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        permagent::session::spectral_schema::apply_briefings_schema(&pool)
            .await
            .unwrap();

        let project = fake_project("Atlas Atlantic");
        brief_scan_failure(&pool, &project, "scanner aborted with an empty model input").await;

        let (kind, severity, summary, ref_id): (String, String, String, Option<String>) =
            sqlx::query_as(
                "SELECT kind, severity, summary, ref_id FROM agent_briefings \
                 WHERE from_agent = 'strix'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(kind, "scan_failed");
        assert_eq!(severity, "attention");
        assert!(summary.contains("Atlas Atlantic"), "{summary}");
        assert!(summary.contains("empty model input"), "{summary}");
        assert_eq!(ref_id.as_deref(), Some("p1"));
    }

    // ── Lifecycle ──────────────────────────────────────────────────────

    const T0: &str = "2026-10-01T00:00:00+00:00";
    const T1: &str = "2026-10-02T00:00:00+00:00";

    fn with_status(id: &str, status: FindingStatus) -> Finding {
        let mut f = finding(id, "high").normalised();
        f.status = status;
        f.status_at = Some(T0.to_string());
        f
    }

    fn status_of(rec: &Reconciled, id: &str) -> FindingStatus {
        rec.findings.iter().find(|f| f.id == id).unwrap().status
    }

    #[test]
    fn a_new_finding_is_fresh_and_open() {
        let rec = reconcile(&[], &[finding("a", "high")], T1);
        assert_eq!(rec.fresh.len(), 1);
        let f = &rec.findings[0];
        assert_eq!(f.status, FindingStatus::Open);
        assert_eq!(f.first_seen_at.as_deref(), Some(T1));
        assert_eq!(f.last_seen_at.as_deref(), Some(T1));
    }

    #[test]
    fn a_finding_still_reported_keeps_its_status_and_updates_last_seen() {
        for status in [
            FindingStatus::Open,
            FindingStatus::Reopened,
            FindingStatus::Sent,
            FindingStatus::FixCommitted,
        ] {
            let rec = reconcile(&[with_status("a", status)], &[finding("a", "high")], T1);
            assert_eq!(status_of(&rec, "a"), status);
            assert_eq!(rec.findings[0].last_seen_at.as_deref(), Some(T1));
            assert!(rec.fresh.is_empty() && rec.reopened.is_empty());
            assert!(rec.confirmed_fixed.is_empty());
        }
    }

    #[test]
    fn a_fix_commit_that_a_later_scan_still_reports_is_reopened() {
        let mut f = with_status("a", FindingStatus::FixCommitted);
        f.fix_commit = Some("0123456789abcdef".to_string());
        f.fix_verified_at = Some(T0.to_string());
        let rec = reconcile(&[f], &[finding("a", "high")], T1);
        assert_eq!(status_of(&rec, "a"), FindingStatus::Reopened);
        assert_eq!(rec.reopened.len(), 1);
        let got = &rec.findings[0];
        assert!(got.fix_commit.is_none());
        assert!(got.verify_detail.as_deref().unwrap().contains("0123456789"));
    }

    #[test]
    fn an_unreported_finding_without_a_fix_commit_is_unconfirmed_not_fixed() {
        for status in [
            FindingStatus::Open,
            FindingStatus::Reopened,
            FindingStatus::Sent,
        ] {
            let rec = reconcile(&[with_status("a", status)], &[], T1);
            assert_eq!(status_of(&rec, "a"), FindingStatus::NotSeen);
            assert!(rec.confirmed_fixed.is_empty());
        }
    }

    #[test]
    fn a_clean_scan_confirms_a_verified_fix() {
        let rec = reconcile(&[with_status("a", FindingStatus::FixCommitted)], &[], T1);
        assert_eq!(status_of(&rec, "a"), FindingStatus::Fixed);
        assert_eq!(rec.confirmed_fixed.len(), 1);
        assert_eq!(rec.findings[0].status_at.as_deref(), Some(T1));
    }

    #[test]
    fn closed_and_unconfirmed_findings_are_stable_when_unreported() {
        for status in [
            FindingStatus::NotSeen,
            FindingStatus::Fixed,
            FindingStatus::Dismissed,
        ] {
            let rec = reconcile(&[with_status("a", status)], &[], T1);
            assert_eq!(status_of(&rec, "a"), status);
            assert_eq!(rec.findings[0].status_at.as_deref(), Some(T0));
            assert!(rec.confirmed_fixed.is_empty());
        }
    }

    #[test]
    fn a_reported_fixed_or_unconfirmed_finding_reopens_and_forgets_its_fix() {
        for status in [FindingStatus::Fixed, FindingStatus::NotSeen] {
            let mut old = with_status("a", status);
            old.sent_at = Some(T0.into());
            old.sent_agent = Some("agent".into());
            old.fix_commit = Some("abc".into());
            old.verify_detail = Some("x".into());
            let rec = reconcile(&[old], &[finding("a", "high")], T1);
            assert_eq!(status_of(&rec, "a"), FindingStatus::Reopened);
            assert_eq!(rec.reopened.len(), 1);
            assert!(rec.fresh.is_empty(), "a known finding is never fresh");
            let f = &rec.findings[0];
            assert!(f.sent_at.is_none() && f.fix_commit.is_none() && f.verify_detail.is_none());
        }
    }

    #[test]
    fn a_dismissed_finding_stays_dismissed_and_is_never_fresh() {
        let mut old = with_status("a", FindingStatus::Dismissed);
        old.dismissed_reason = Some("accepted risk".into());
        let rec = reconcile(&[old], &[finding("a", "critical")], T1);
        assert_eq!(status_of(&rec, "a"), FindingStatus::Dismissed);
        assert!(rec.fresh.is_empty() && rec.reopened.is_empty());
        let f = &rec.findings[0];
        assert_eq!(f.last_seen_at.as_deref(), Some(T1));
        assert_eq!(f.status_at.as_deref(), Some(T0));
        assert_eq!(f.dismissed_reason.as_deref(), Some("accepted risk"));
    }

    #[test]
    fn a_clean_scan_over_an_empty_list_changes_nothing() {
        let rec = reconcile(&[], &[], T1);
        assert!(rec.findings.is_empty() && rec.fresh.is_empty());
    }

    #[test]
    fn duplicate_ids_in_a_scan_make_one_finding() {
        let rec = reconcile(&[], &[finding("a", "high"), finding("a", "high")], T1);
        assert_eq!(rec.findings.len(), 1);
        assert_eq!(rec.fresh.len(), 1);
    }

    #[test]
    fn active_findings_are_never_dropped_and_only_closed_history_is_capped() {
        let mut existing: Vec<Finding> = (0..100)
            .map(|i| with_status(&format!("active-{i}"), FindingStatus::Open))
            .collect();
        for i in 0..70 {
            let mut f = with_status(&format!("closed-{i}"), FindingStatus::Fixed);
            // closed-69 is the newest.
            f.status_at = Some(format!("2026-09-01T00:00:{i:02}+00:00"));
            existing.push(f);
        }
        // Everything active is still reported, so nothing changes status.
        let incoming: Vec<Finding> = (0..100)
            .map(|i| finding(&format!("active-{i}"), "high"))
            .collect();
        let rec = reconcile(&existing, &incoming, T1);
        let active = rec.findings.iter().filter(|f| f.status.is_active()).count();
        let closed: Vec<&Finding> = rec
            .findings
            .iter()
            .filter(|f| !f.status.is_active())
            .collect();
        assert_eq!(active, 100);
        assert_eq!(closed.len(), MAX_CLOSED_KEPT);
        assert!(closed.iter().any(|f| f.id == "closed-69"), "newest kept");
        assert!(closed.iter().all(|f| f.id != "closed-0"), "oldest aged out");
    }

    #[test]
    fn counts_split_the_list_the_way_the_panel_shows_it() {
        let list = vec![
            with_status("1", FindingStatus::Open),
            with_status("2", FindingStatus::Reopened),
            with_status("3", FindingStatus::Sent),
            with_status("4", FindingStatus::FixCommitted),
            with_status("5", FindingStatus::NotSeen),
            with_status("6", FindingStatus::Fixed),
            with_status("7", FindingStatus::Dismissed),
        ];
        let c = counts(&list);
        assert_eq!(
            c,
            Counts {
                active: 5,
                open: 2,
                sent: 1,
                awaiting_rescan: 1,
                unconfirmed: 1,
                fixed: 1,
                dismissed: 1
            }
        );
    }

    #[test]
    fn display_order_is_active_by_severity_then_closed_newest_first() {
        let mut low = with_status("low", FindingStatus::Open);
        low.severity = "low".into();
        let mut crit = with_status("crit", FindingStatus::Sent);
        crit.severity = "critical".into();
        let mut old = with_status("old", FindingStatus::Fixed);
        old.status_at = Some("2026-09-01T00:00:00+00:00".into());
        let mut new = with_status("new", FindingStatus::Dismissed);
        new.status_at = Some("2026-09-30T00:00:00+00:00".into());
        let mut list = vec![old, low, new, crit];
        display_order(&mut list);
        let ids: Vec<&str> = list.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["crit", "low", "new", "old"]);
    }

    #[test]
    fn metadata_from_before_the_lifecycle_reads_as_open() {
        let meta = serde_json::json!({ "strix_findings": [{
            "id": "CWE-89:src/db.rs", "title": "SQL injection", "severity": "high",
            "cwe": "CWE-89", "location": "src/db.rs", "remediation": null,
            "found_at": "2026-08-05T00:00:00Z"
        }]});
        let list = stored_findings(&meta);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].status, FindingStatus::Open);
        assert_eq!(
            list[0].first_seen_at.as_deref(),
            Some("2026-08-05T00:00:00Z")
        );
        // Unset lifecycle fields are omitted, not written as nulls.
        let json = serde_json::to_value(&list[0]).unwrap();
        assert!(json.get("sent_at").is_none());
        assert_eq!(json["status"], "open");
    }

    #[test]
    fn an_unknown_status_reads_as_open_instead_of_losing_the_list() {
        let f: Finding = serde_json::from_value(serde_json::json!({
            "id": "a", "title": "t", "severity": "low", "cwe": null,
            "location": null, "remediation": null, "found_at": "x", "status": "mystery"
        }))
        .unwrap();
        assert_eq!(f.status, FindingStatus::Open);
    }

    fn rejected(r: Result<(), StoreError>) -> String {
        match r {
            Err(StoreError::Rejected(m)) => m,
            other => panic!("expected a rejection, got {other:?}"),
        }
    }

    #[test]
    fn sent_is_for_active_unfixed_findings_and_needs_an_agent() {
        let mut f = with_status("a", FindingStatus::Open);
        assert!(apply_sent(&mut f, "  ", T1).is_err());
        assert!(apply_sent(&mut f, &"x".repeat(81), T1).is_err());
        apply_sent(&mut f, "coding agent", T1).unwrap();
        assert_eq!(f.status, FindingStatus::Sent);
        assert_eq!(f.sent_agent.as_deref(), Some("coding agent"));
        assert_eq!(f.sent_at.as_deref(), Some(T1));
        // Sending again keeps the first sent_at.
        apply_sent(&mut f, "another agent", "2026-10-03T00:00:00+00:00").unwrap();
        assert_eq!(f.sent_at.as_deref(), Some(T1));
        assert_eq!(f.sent_agent.as_deref(), Some("another agent"));
        let mut n = with_status("a", FindingStatus::NotSeen);
        apply_sent(&mut n, "agent", T1).unwrap();
        assert_eq!(n.status, FindingStatus::Sent);
        for status in [
            FindingStatus::FixCommitted,
            FindingStatus::Fixed,
            FindingStatus::Dismissed,
        ] {
            let mut f = with_status("a", status);
            rejected(apply_sent(&mut f, "agent", T1));
            assert_eq!(
                f.status, status,
                "a refused change leaves the finding alone"
            );
        }
        let mut r = with_status("a", FindingStatus::Reopened);
        apply_sent(&mut r, "agent", T1).unwrap();
    }

    #[test]
    fn verify_moves_active_findings_to_fix_committed_and_refuses_closed_ones() {
        for status in [
            FindingStatus::Open,
            FindingStatus::Reopened,
            FindingStatus::Sent,
            FindingStatus::NotSeen,
            FindingStatus::FixCommitted,
        ] {
            let mut f = with_status("a", status);
            apply_verified(&mut f, "abc123", "Fix it", "detail", T1).unwrap();
            assert_eq!(f.status, FindingStatus::FixCommitted);
            assert_eq!(f.fix_commit.as_deref(), Some("abc123"));
            assert_eq!(f.fix_commit_subject.as_deref(), Some("Fix it"));
            assert_eq!(f.fix_verified_at.as_deref(), Some(T1));
            assert_eq!(f.verify_detail.as_deref(), Some("detail"));
        }
        for status in [FindingStatus::Fixed, FindingStatus::Dismissed] {
            let mut f = with_status("a", status);
            rejected(apply_verified(&mut f, "abc", "s", "d", T1));
        }
    }

    #[test]
    fn dismissing_needs_a_reason_of_one_to_280_characters() {
        let mut f = with_status("a", FindingStatus::Sent);
        rejected(apply_status(&mut f, "dismissed", None, T1));
        rejected(apply_status(&mut f, "dismissed", Some("   "), T1));
        rejected(apply_status(
            &mut f,
            "dismissed",
            Some(&"x".repeat(281)),
            T1,
        ));
        assert_eq!(f.status, FindingStatus::Sent);
        apply_status(&mut f, "dismissed", Some(&"x".repeat(280)), T1).unwrap();
        assert_eq!(f.status, FindingStatus::Dismissed);
        assert_eq!(f.dismissed_reason.as_deref().map(str::len), Some(280));
        // Closed findings cannot be dismissed again.
        rejected(apply_status(&mut f, "dismissed", Some("again"), T1));
    }

    #[test]
    fn reopening_only_works_on_closed_findings_and_clears_the_reason() {
        let mut f = with_status("a", FindingStatus::Open);
        rejected(apply_status(&mut f, "open", None, T1));
        let mut d = with_status("a", FindingStatus::Dismissed);
        d.dismissed_reason = Some("accepted".into());
        apply_status(&mut d, "open", None, T1).unwrap();
        assert_eq!(d.status, FindingStatus::Open);
        assert!(d.dismissed_reason.is_none());
        let mut x = with_status("a", FindingStatus::Fixed);
        x.fix_commit = Some("abc".into());
        apply_status(&mut x, "open", None, T1).unwrap();
        assert!(x.fix_commit.is_none());
        rejected(apply_status(&mut x, "fixed", None, T1));
    }

    fn gate<'a>(requested: Option<&'a str>, attempt: Option<&'a str>) -> RescanGate<'a> {
        RescanGate {
            enabled: true,
            sovereign: false,
            has_root: true,
            requested_at: requested,
            last_attempt: attempt,
        }
    }

    fn at(s: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(s)
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    #[test]
    fn a_rescan_is_refused_when_the_guard_is_off_sovereign_or_has_no_folder() {
        let now = at("2026-10-14T12:00:00+00:00");
        assert!(rescan_refusal(&gate(None, None), now).is_none());
        let mut g = gate(None, None);
        g.enabled = false;
        assert!(rescan_refusal(&g, now).unwrap().contains("turned off"));
        let mut g = gate(None, None);
        g.sovereign = true;
        assert!(rescan_refusal(&g, now).unwrap().contains("Sovereign"));
        let mut g = gate(None, None);
        g.has_root = false;
        assert!(rescan_refusal(&g, now).unwrap().contains("no local folder"));
    }

    #[test]
    fn a_rescan_is_refused_within_thirty_minutes_of_the_last_request_or_scan() {
        let now = at("2026-10-14T12:00:00+00:00");
        let recent = "2026-10-14T11:45:00+00:00";
        let old = "2026-10-14T11:20:00+00:00";
        assert!(rescan_refusal(&gate(Some(recent), None), now)
            .unwrap()
            .contains("already requested 15 minute"));
        assert!(rescan_refusal(&gate(None, Some(recent)), now)
            .unwrap()
            .contains("scanned 15 minute"));
        assert!(rescan_refusal(&gate(Some(old), Some(old)), now).is_none());
        // A stamp in the future (clock skew) counts as just now.
        assert!(rescan_refusal(&gate(None, Some("2026-10-14T13:00:00+00:00")), now).is_some());
        // An unparseable stamp must not block forever.
        assert!(rescan_refusal(&gate(Some("junk"), Some("junk")), now).is_none());
    }

    #[test]
    fn a_stale_rescan_request_stops_counting_as_pending() {
        let now = at("2026-10-14T12:00:00+00:00");
        let meta = |t: &str| serde_json::json!({ "strix_rescan_requested_at": t });
        assert!(rescan_pending(&meta("2026-10-14T11:00:00+00:00"), now));
        assert!(!rescan_pending(&meta("2026-10-14T05:00:00+00:00"), now));
        assert!(!rescan_pending(&serde_json::json!({}), now));
    }

    #[test]
    fn the_report_lists_and_plans_active_findings_only() {
        let mut fixed = with_status("fixed-one", FindingStatus::Fixed);
        fixed.title = "Already fixed thing".into();
        let mut gone = with_status("dismissed-one", FindingStatus::Dismissed);
        gone.title = "Dismissed thing".into();
        let mut live = with_status("live-one", FindingStatus::FixCommitted);
        live.title = "Live thing".into();
        let mut stale = with_status("stale-one", FindingStatus::NotSeen);
        stale.title = "Stale thing".into();
        let md = security_report_markdown("Atlas", &[fixed, gone, live, stale], &[]);
        assert!(
            md.contains(
                "2 active (1 awaiting a confirming rescan, 1 unconfirmed), 1 fixed, 1 dismissed"
            ),
            "{md}"
        );
        assert!(md.contains("Live thing") && md.contains("Stale thing"));
        assert!(!md.contains("Already fixed thing") && !md.contains("Dismissed thing"));
    }

    #[tokio::test]
    async fn confirmed_fix_and_reopen_briefings_name_the_project() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        permagent::session::spectral_schema::apply_briefings_schema(&pool)
            .await
            .unwrap();
        let project = fake_project("Atlas Atlantic");
        brief_confirmed_fixed(&pool, &project, &[]).await;
        brief_reopened(&pool, &project, &[]).await;
        let none: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_briefings")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(none, 0, "nothing to say, nothing filed");

        brief_confirmed_fixed(&pool, &project, &[finding("a", "high")]).await;
        brief_reopened(&pool, &project, &[finding("b", "high")]).await;
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT kind, summary FROM agent_briefings ORDER BY kind")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "security_fixed");
        assert!(rows[0]
            .1
            .contains("1 security finding confirmed fixed on Atlas Atlantic"));
        assert_eq!(rows[1].0, "security_reopened");
        assert!(rows[1].1.contains("came back on Atlas Atlantic"));
    }
}
