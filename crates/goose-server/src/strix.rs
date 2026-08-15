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
//!   * every target passes `strix::check_scope` before the scanner is invoked,
//!     so a path outside the user's own project roots cannot be reached even
//!     if a project row is malformed.

use crate::state::AppState;
use permagent::projects::{self, Project, UpdateProject};
use permagent::strix;
use sqlx::{Pool, Sqlite};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

/// Findings ride `projects.metadata_json.strix_findings` — same
/// no-migration storage the Watcher's insights use.
const METADATA_KEY: &str = "strix_findings";
/// When this project was last scanned (ISO-8601), for the one-per-sweep
/// rotation: the least-recently-scanned active project goes next.
const LAST_SCAN_KEY: &str = "strix_last_scan";
/// Keep the most recent findings per project; older ones age out.
const MAX_KEPT: usize = 40;
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
/// The default bucket is not a project Strix reports on.
const PERSONAL_PROJECT_ID: &str = "00000000-0000-0000-0000-000000000001";

/// One finding, as rendered on the project's Overview and in briefings.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct Finding {
    pub id: String,
    pub title: String,
    pub severity: String,
    pub cwe: Option<String>,
    pub location: Option<String>,
    pub remediation: Option<String>,
    pub found_at: String,
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
        let mut last_sweep: Option<tokio::time::Instant> = None;
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
                tracing::debug!(target: "permagentd::strix", "first sweep skipped: {e}");
            }
        } else {
            tokio::time::sleep(STARTUP_DELAY).await;
        }
        loop {
            let due = last_sweep.is_none_or(|t| t.elapsed() >= sweep_interval());
            if strix::is_enabled() && due {
                last_sweep = Some(tokio::time::Instant::now());
                if let Err(e) = sweep_once(&state).await {
                    tracing::debug!(target: "permagentd::strix", "sweep skipped: {e}");
                }
            }
            tokio::time::sleep(CHECK_EVERY).await;
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

    // ONE project per sweep, rotating least-recently-scanned first (Jesse,
    // 2026-08-06): a whole-fleet pass four times a day was the wrong shape —
    // one focused scan per interval spreads cost evenly and gives each
    // project a fresh report on a predictable cycle. A never-scanned project
    // sorts first (empty stamp).
    let mut candidates: Vec<&Project> = projects
        .iter()
        .filter(|p| p.id != PERSONAL_PROJECT_ID && p.root_path.is_some())
        .collect();
    candidates.sort_by_key(|p| last_scan_stamp(p));
    let Some(project) = candidates.first().copied() else {
        return Ok(());
    };
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
            // Stamp anyway so an unresolvable root cannot pin the rotation.
            let _ = stamp_last_scan(&pool, project).await;
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
        // Stamp anyway, for the same reason an unresolvable root does: a
        // refused project must not pin the rotation.
        if let Err(e) = stamp_last_scan(&pool, project).await {
            tracing::warn!(target: "permagentd::strix", "last-scan stamp failed: {e}");
        }
        return Ok(());
    }

    announce("working");
    let outcome = scan_project(&target).await;
    // The rotation advances on every attempt — success, clean, or error —
    // so one broken project can never starve the rest of the cycle.
    if let Err(e) = stamp_last_scan(&pool, project).await {
        tracing::warn!(target: "permagentd::strix", "last-scan stamp failed: {e}");
    }
    match outcome {
        Ok(findings) if findings.is_empty() => {
            tracing::info!(
                target: "permagentd::strix",
                project = %project.name,
                "clean — no findings"
            );
            announce("available");
        }
        Ok(findings) => {
            match record_findings(&pool, project, findings).await {
                Ok(fresh) => {
                    // The deliverable: a security report note on the project,
                    // findings plus an ordered fix plan. Notes index into the
                    // Brain, so "ask Henry to read the Guard's report and
                    // dispatch a fix goal" works with no extra plumbing.
                    let current: Vec<Finding> = current_findings(&pool, &project.id).await;
                    let body = security_report_markdown(&project.name, &current, &fresh);
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
                    brief_new_findings(&pool, project, &fresh).await;
                }
                Err(e) => tracing::warn!(
                    target: "permagentd::strix",
                    project = %project.name,
                    "findings not recorded: {e}"
                ),
            }
            announce("available");
        }
        Err(e) => {
            // A missing scanner is a stated fact, not a silent skip.
            tracing::warn!(
                target: "permagentd::strix",
                project = %project.name,
                "scan did not run: {e}"
            );
            announce("error");
        }
    }
    tracing::info!(
        target: "permagentd::strix",
        project = %project.name,
        "sweep complete — next sweep takes the next least-recently-scanned project"
    );
    Ok(())
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
            !projects.is_empty() && projects.iter().all(|p| last_scan_stamp(p).is_empty())
        }
        Err(_) => false,
    }
}

/// The project's last-scan stamp, empty if never scanned (sorts first).
fn last_scan_stamp(project: &Project) -> String {
    project
        .metadata_json
        .as_object()
        .and_then(|m| m.get(LAST_SCAN_KEY))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

/// Advance the rotation: write the last-scan stamp without touching findings.
async fn stamp_last_scan(pool: &Pool<Sqlite>, project: &Project) -> Result<(), String> {
    // Re-read for the same reason `record_findings` does: `project` was
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

/// Re-read the project's current (merged) findings after recording.
async fn current_findings(pool: &Pool<Sqlite>, project_id: &str) -> Vec<Finding> {
    match projects::get_project_by_id_or_slug(pool, project_id).await {
        Ok(Some(p)) => p
            .metadata_json
            .as_object()
            .and_then(|m| m.get(METADATA_KEY))
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// The report note: what was found, then a fix plan ordered by severity —
/// written so Henry can turn it into a dispatchable goal verbatim.
fn security_report_markdown(project_name: &str, current: &[Finding], fresh: &[Finding]) -> String {
    let sev_rank = |s: &str| match s {
        "critical" => 0,
        "high" => 1,
        "medium" => 2,
        _ => 3,
    };
    let mut ordered: Vec<&Finding> = current.iter().collect();
    ordered.sort_by_key(|f| sev_rank(&f.severity));

    let mut out = format!(
        "The Guard scanned **{project_name}** and found {} open finding{} ({} new this scan).\n\n## Findings\n\n",
        current.len(),
        if current.len() == 1 { "" } else { "s" },
        fresh.len(),
    );
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
/// Defaults to Haiku: sweeps recur on the user's credits, and finding exposed
/// secrets and vulnerable dependencies does not need a frontier model.
const STRIX_LLM_KEY: &str = "strix_llm";
const DEFAULT_STRIX_LLM: &str = "anthropic/claude-haiku-4-5-20251001";

/// The cloud model the scanner drives — the destination the user's source
/// actually reaches, and so what the egress audit records.
fn strix_model() -> String {
    permagent::config::Config::global()
        .get_param::<String>(STRIX_LLM_KEY)
        .unwrap_or_else(|_| DEFAULT_STRIX_LLM.to_string())
}

/// Build the scanner's LLM environment from Permagent's own config/keychain,
/// so Guard setup never touches the launchd plist: the model rides
/// `strix_llm`, and the API key is the SAME provider secret the user already
/// stored for chat (looked up by the model string's provider prefix). If the
/// user exported STRIX_LLM/LLM_API_KEY themselves, those win — we only fill
/// what is absent.
fn scanner_env() -> Vec<(&'static str, String)> {
    let mut env = Vec::new();
    let config = permagent::config::Config::global();
    let model = strix_model();
    if std::env::var("STRIX_LLM").is_err() {
        env.push(("STRIX_LLM", model.clone()));
    }
    if std::env::var("LLM_API_KEY").is_err() {
        let secret_key = match model.split('/').next().unwrap_or_default() {
            "anthropic" => Some("ANTHROPIC_API_KEY"),
            "openai" => Some("OPENAI_API_KEY"),
            "groq" => Some("GROQ_API_KEY"),
            "gemini" | "google" => Some("GOOGLE_API_KEY"),
            _ => None,
        };
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

/// Dependency preflight (audit 2026-08-11): the Guard needs the `strix` CLI
/// AND a running Docker daemon. The failure is a stated fact the sweep refuses
/// on — never a degraded pretend-scan — and `sweep_once` files ONE briefing
/// per distinct failure (see [`report_preflight_failure`]).
async fn preflight() -> Result<(), String> {
    let mut missing = Vec::new();
    if !strix_bin_present() {
        missing.push("the `strix` scanner is not installed (fix: `pipx install strix-agent`)");
    }
    let docker_ok = {
        let mut cmd = tokio::process::Command::new(resolve_docker_bin());
        cmd.arg("info")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        cmd.kill_on_drop(true);
        matches!(
            tokio::time::timeout(Duration::from_secs(15), cmd.status()).await,
            Ok(Ok(status)) if status.success()
        )
    };
    if !docker_ok {
        missing.push("Docker is not running (`docker info` failed)");
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
            detail: Some(
                "Sweeps are skipped until this is fixed. `pipx install strix-agent` installs \
                 the scanner; Docker Desktop must be installed and running. The sweep loop \
                 retries automatically — nothing to restart."
                    .to_string(),
            ),
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
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
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
                    "scan exceeded its {}-minute bound (scanner killed; a Docker container it \
                     started may need `docker ps` cleanup)",
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
    let mut cmd = tokio::process::Command::new(resolve_strix_bin());
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
        .arg(
            "Static code analysis only. Do not run the application, do not send network \
             traffic to any host, and do not modify, create, or delete any files in the \
             target. Report findings; never remediate.",
        )
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
    .await?;
    if !output.status.success() {
        return Err(format!(
            "scanner exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .last()
                .unwrap_or_default()
        ));
    }
    let sarif = find_sarif(target).ok_or_else(|| "scan produced no findings.sarif".to_string())?;
    let raw = std::fs::read_to_string(&sarif).map_err(|e| e.to_string())?;
    parse_sarif(&raw)
}

/// Locate the run's `findings.sarif`. The engine writes per-run directories;
/// the newest one wins.
fn find_sarif(target: &std::path::Path) -> Option<PathBuf> {
    let runs = target.join(".strix").join("runs");
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(runs).ok()?.flatten() {
        let candidate = entry.path().join("findings.sarif");
        if !candidate.is_file() {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        if best.as_ref().is_none_or(|(t, _)| modified > *t) {
            best = Some((modified, candidate));
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
            });
        }
    }
    Ok(out)
}

/// The findings that were not already on the checklist. Briefing on these —
/// and only these — is what stops the same medium finding re-alerting Henry
/// every six hours forever.
fn fresh_findings(existing: &[Finding], incoming: &[Finding]) -> Vec<Finding> {
    incoming
        .iter()
        .filter(|f| !existing.iter().any(|old| old.id == f.id))
        .cloned()
        .collect()
}

/// Merge findings into the project's metadata, newest first, deduped on id so
/// a finding that persists across sweeps does not multiply. Returns the
/// findings that are new this sweep.
async fn record_findings(
    pool: &Pool<Sqlite>,
    project: &Project,
    findings: Vec<Finding>,
) -> Result<Vec<Finding>, String> {
    // Re-read: `project` was captured BEFORE the last-scan stamp was written,
    // so writing its metadata back would erase the stamp and pin the rotation
    // to this project forever — it would be least-recently-scanned every tick.
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
    let existing: Vec<Finding> = meta
        .get(METADATA_KEY)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let fresh = fresh_findings(&existing, &findings);

    let mut merged = findings;
    for old in existing {
        if !merged.iter().any(|f| f.id == old.id) {
            merged.push(old);
        }
    }
    merged.truncate(MAX_KEPT);

    meta.insert(
        METADATA_KEY.to_string(),
        serde_json::to_value(&merged).map_err(|e| e.to_string())?,
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
    .map(|_| fresh)
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
        }
    }

    #[test]
    fn preflight_brief_gate_is_once_per_condition() {
        assert!(preflight_should_brief(None, "docker down"));
        assert!(!preflight_should_brief(Some("docker down"), "docker down"));
        assert!(preflight_should_brief(Some("docker down"), "strix missing"));
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

    #[test]
    fn only_genuinely_new_findings_are_fresh() {
        let existing = vec![finding("CWE-89:src/db.rs", "high")];
        let incoming = vec![
            finding("CWE-89:src/db.rs", "high"),
            finding("CWE-79:src/ui.rs", "medium"),
        ];
        let fresh = fresh_findings(&existing, &incoming);
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].id, "CWE-79:src/ui.rs");
        // An unchanged set is entirely stale — this is the re-brief suppressor.
        assert!(fresh_findings(&existing, &existing).is_empty());
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
        let status = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
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
            Duration::from_secs(5),
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
}
