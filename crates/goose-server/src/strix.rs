//! Strix's sweep loop — the standing security agent.
//!
//! Every pass walks the user's own active projects, runs the Strix pentest
//! engine over each in read-only posture, and turns its SARIF findings into a
//! living fix checklist on that project's Overview. Nothing is remediated:
//! Strix reports, and anything intrusive is proposed for the user rather than
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
/// One sweep every six hours. A security posture does not change by the minute,
/// and each pass costs real scanner time.
const TICK: Duration = Duration::from_secs(6 * 3600);
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
    if !strix::is_enabled() {
        tracing::debug!(
            target: "permagentd::strix",
            "Strix is off ({}=false) — no security sweeps will run",
            strix::STRIX_ENABLED_KEY
        );
        return;
    }
    tracing::info!(
        target: "permagentd::strix",
        "Strix enabled — security sweeps every {}h, read-only posture",
        TICK.as_secs() / 3600
    );
    tokio::spawn(async move {
        tokio::time::sleep(STARTUP_DELAY).await;
        loop {
            if let Err(e) = sweep_once(&state).await {
                tracing::debug!(target: "permagentd::strix", "sweep skipped: {e}");
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

/// The World shows Strix working only while it genuinely is — the honesty
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
        match scan_project(&target).await {
            Ok(findings) if findings.is_empty() => {
                tracing::info!(
                    target: "permagentd::strix",
                    project = %project.name,
                    "clean — no findings"
                );
            }
            Ok(findings) => {
                let count = findings.len();
                record_findings(&pool, &project, findings).await?;
                brief_if_serious(&pool, &project, count).await;
                swept += 1;
            }
            Err(e) => {
                // A missing scanner is a stated fact, not a silent skip.
                tracing::warn!(
                    target: "permagentd::strix",
                    project = %project.name,
                    "scan did not run: {e}"
                );
                announce("error");
                return Ok(());
            }
        }
    }
    announce("available");
    tracing::info!(target: "permagentd::strix", "sweep complete — {swept} project(s) with findings");
    Ok(())
}

/// Run the scanner over one project and parse its SARIF. Strix (the engine)
/// writes `findings.sarif` per run; SARIF is preferred over its bespoke JSON
/// because it is schema-validated and dedupes on CWE.
async fn scan_project(target: &std::path::Path) -> Result<Vec<Finding>, String> {
    let mut cmd = tokio::process::Command::new("strix");
    cmd.arg("--target")
        .arg(target)
        .arg("--non-interactive")
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

/// Merge findings into the project's metadata, newest first, deduped on id so
/// a finding that persists across sweeps does not multiply.
async fn record_findings(
    pool: &Pool<Sqlite>,
    project: &Project,
    findings: Vec<Finding>,
) -> Result<(), String> {
    let mut meta = project
        .metadata_json
        .as_object()
        .cloned()
        .unwrap_or_default();
    let existing: Vec<Finding> = meta
        .get(METADATA_KEY)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

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
    .map(|_| ())
}

/// Tell Henry only when it matters — a briefing for every clean sweep is noise.
async fn brief_if_serious(pool: &Pool<Sqlite>, project: &Project, count: usize) {
    permagent::briefings::file_briefing(
        pool,
        permagent::briefings::NewBriefing {
            from_agent: strix::STRIX_FEATURE_ID.to_string(),
            kind: "security_findings".to_string(),
            severity: permagent::briefings::Severity::Attention,
            summary: format!(
                "{count} security finding{} on {}",
                if count == 1 { "" } else { "s" },
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
}
