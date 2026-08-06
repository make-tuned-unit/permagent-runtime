//! The Guard's sweep loop — the standing security agent (Strix engine).
//!
//! Every pass walks the user's own active projects, runs the Strix pentest
//! engine over each in read-only posture, and turns its SARIF findings into a
//! living fix checklist on that project's Overview. Nothing is remediated:
//! The Guard reports, and anything intrusive is proposed for the user rather than
//! performed (`permagent::strix::classify`).
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
/// Hard bound on one project's scan.
const SCAN_TIMEOUT: Duration = Duration::from_secs(20 * 60);
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
        tokio::time::sleep(STARTUP_DELAY).await;
        let mut last_sweep: Option<tokio::time::Instant> = None;
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
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| e.to_string())?;
    let projects = projects::list_projects(&pool, Some("active")).await?;

    let roots: Vec<PathBuf> = projects
        .iter()
        .filter_map(|p| p.root_path.as_ref().map(PathBuf::from))
        .collect();
    if roots.is_empty() {
        return Ok(());
    }

    announce("working");
    let mut swept = 0usize;
    let mut attempted = 0usize;
    let mut scan_errors = 0usize;
    for project in projects {
        if project.id == PERSONAL_PROJECT_ID {
            continue;
        }
        let Some(root) = project.root_path.clone() else {
            continue;
        };
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
                continue;
            }
        };
        attempted += 1;
        match scan_project(&target).await {
            Ok(findings) if findings.is_empty() => {
                tracing::info!(
                    target: "permagentd::strix",
                    project = %project.name,
                    "clean — no findings"
                );
            }
            Ok(findings) => {
                match record_findings(&pool, &project, findings).await {
                    Ok(fresh) => brief_new_findings(&pool, &project, &fresh).await,
                    Err(e) => tracing::warn!(
                        target: "permagentd::strix",
                        project = %project.name,
                        "findings not recorded: {e}"
                    ),
                }
                swept += 1;
            }
            Err(e) => {
                // A missing scanner is a stated fact, not a silent skip — but
                // one broken project must not end the sweep for the rest.
                tracing::warn!(
                    target: "permagentd::strix",
                    project = %project.name,
                    "scan did not run: {e}"
                );
                scan_errors += 1;
            }
        }
    }
    // SCAN BLOCKED only when nothing could run at all (scanner absent);
    // otherwise the World returns to idle so a one-off failure cannot latch.
    if attempted > 0 && scan_errors == attempted {
        announce("error");
    } else {
        announce("available");
    }
    tracing::info!(
        target: "permagentd::strix",
        "sweep complete — {swept} project(s) with findings, {scan_errors} scan error(s)"
    );
    Ok(())
}

/// Run the scanner over one project and parse its SARIF. Strix (the engine)
/// writes `findings.sarif` per run; SARIF is preferred over its bespoke JSON
/// because it is schema-validated and dedupes on CWE.
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

async fn scan_project(target: &std::path::Path) -> Result<Vec<Finding>, String> {
    let mut cmd = tokio::process::Command::new(resolve_strix_bin());
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

    let child = cmd.spawn().map_err(|e| {
        format!("`strix` is not runnable ({e}) — install it and Docker to enable sweeps")
    })?;
    let output = tokio::time::timeout(SCAN_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| {
            format!(
                "scan exceeded its {}-minute bound",
                SCAN_TIMEOUT.as_secs() / 60
            )
        })?
        .map_err(|e| e.to_string())?;
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
    let mut meta = project
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
}
