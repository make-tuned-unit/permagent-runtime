//! Strix — the security character agent.
//!
//! Strix continuously probes the user's OWN projects for security flaws and
//! keeps a living fix checklist. It wraps the Strix pentest engine
//! (github.com/usestrix/strix, Apache-2.0) as an external scanner and ingests
//! its SARIF output; the agent layer here is the standing character: the
//! enable flag, the scope guard, and the identity Henry can describe.
//!
//! Two rules are in CODE, not in a prompt, because a security agent that runs
//! live exploit tooling must not be able to talk itself out of them:
//!
//! 1. **Scope.** Only paths inside the user's own registered project roots are
//!    scannable. Anything else — a URL, a neighbour's checkout, a system
//!    directory — is refused before the scanner is invoked.
//! 2. **Reporting, not remediation.** Strix files findings. It never edits code
//!    to "fix" what it found, and anything intrusive routes to the Decision
//!    Inbox for the user rather than executing.

use std::path::{Path, PathBuf};

use crate::config::Config;

/// Config key in `~/.permagent/config.yaml`. Off by default: a scanner that
/// runs live exploit tooling is switched on deliberately, never by upgrade.
pub const STRIX_ENABLED_KEY: &str = "strix_enabled";
/// Self-knowledge id (also the World roster id and the agent.yaml worker key).
pub const STRIX_FEATURE_ID: &str = "strix";
/// The character's name.
pub const STRIX_NAME: &str = "Strix";

pub fn is_enabled() -> bool {
    Config::global()
        .get_param::<bool>(STRIX_ENABLED_KEY)
        .unwrap_or(false)
}

/// What a proposed operation is allowed to do without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanPosture {
    /// Read-only inspection of source and configuration — runs autonomously.
    Passive,
    /// Live traffic against a locally-running target — proposes first.
    Active,
    /// Anything that would modify the target or reach outside it — refused.
    Forbidden,
}

/// Classify a scan mode. Deliberately conservative: an unknown mode is
/// `Forbidden`, never "probably fine".
pub fn classify(mode: &str) -> ScanPosture {
    match mode.trim().to_ascii_lowercase().as_str() {
        "source" | "sast" | "secrets" | "deps" | "config" => ScanPosture::Passive,
        "dast" | "fuzz" | "probe" => ScanPosture::Active,
        _ => ScanPosture::Forbidden,
    }
}

/// Why a target was refused. Carried into the log/report so a refusal is never
/// silent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeRefusal {
    /// Not a filesystem path (a URL, a hostname).
    NotAPath,
    /// A real path, but outside every registered project root.
    OutsideProjectRoots,
}

/// The scope guard. `roots` is the set of the user's own project roots.
///
/// Canonicalizes before comparing so `..` traversal cannot escape a root, and
/// requires a genuine path-component boundary so `/work/app-evil` does not pass
/// as a child of `/work/app`.
pub fn check_scope(target: &str, roots: &[PathBuf]) -> Result<PathBuf, ScopeRefusal> {
    let raw = target.trim();
    if raw.is_empty() || raw.contains("://") {
        return Err(ScopeRefusal::NotAPath);
    }
    let candidate = Path::new(raw);
    let resolved = candidate
        .canonicalize()
        .unwrap_or_else(|_| candidate.to_path_buf());
    for root in roots {
        let root_resolved = root.canonicalize().unwrap_or_else(|_| root.clone());
        if resolved == root_resolved || resolved.starts_with(&root_resolved) {
            return Ok(resolved);
        }
    }
    Err(ScopeRefusal::OutsideProjectRoots)
}

/// Self-knowledge descriptor. Rendered into `<permagent_self>` only while the
/// flag is on (see `self_knowledge::worker_descriptor_visible`), so the brief —
/// and its snapshots — are byte-identical until Strix is deliberately enabled.
pub const SELF_KNOWLEDGE_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: STRIX_FEATURE_ID,
        display_name: "Strix",
        category: crate::agents::self_knowledge::FeatureCategory::Worker,
        what_it_does:
            "Continuously probes the user's own projects for security flaws — exposed secrets, \
             vulnerable dependencies, injection and access-control weaknesses, risky \
             configuration — and keeps a living checklist of what to fix, each item carrying \
             its severity, CWE, location, and remediation",
        why_it_matters:
            "Security review that happens on its own cadence instead of when someone remembers \
             to ask. When the user asks what is wrong with a project, Strix's findings are the \
             standing answer — and it only ever reports: it never edits code to fix what it \
             found, and anything intrusive is proposed for approval rather than performed",
        state_source: crate::agents::self_knowledge::StateSource::Queryable,
        teaching: &[],
    };

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_scan_modes_are_forbidden_not_assumed_safe() {
        assert_eq!(classify("source"), ScanPosture::Passive);
        assert_eq!(classify("SECRETS"), ScanPosture::Passive);
        assert_eq!(classify("dast"), ScanPosture::Active);
        assert_eq!(classify("rm -rf"), ScanPosture::Forbidden);
        assert_eq!(classify(""), ScanPosture::Forbidden);
    }

    #[test]
    fn scope_guard_refuses_urls_and_foreign_paths() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("myapp");
        std::fs::create_dir_all(root.join("src")).unwrap();
        let roots = vec![root.clone()];

        // In scope: the root itself and anything beneath it.
        assert!(check_scope(root.to_str().unwrap(), &roots).is_ok());
        assert!(check_scope(root.join("src").to_str().unwrap(), &roots).is_ok());

        // Refused: remote targets — Strix scans the user's own code, not hosts.
        assert_eq!(
            check_scope("https://example.com", &roots),
            Err(ScopeRefusal::NotAPath)
        );
        // Refused: outside every root.
        assert_eq!(
            check_scope("/etc", &roots),
            Err(ScopeRefusal::OutsideProjectRoots)
        );
        // Refused: a sibling that merely shares a name prefix.
        let sibling = tmp.path().join("myapp-other");
        std::fs::create_dir_all(&sibling).unwrap();
        assert_eq!(
            check_scope(sibling.to_str().unwrap(), &roots),
            Err(ScopeRefusal::OutsideProjectRoots)
        );
        // Refused: traversal out of a root cannot escape via `..`.
        let escape = root.join("..").join("elsewhere");
        std::fs::create_dir_all(tmp.path().join("elsewhere")).unwrap();
        assert_eq!(
            check_scope(escape.to_str().unwrap(), &roots),
            Err(ScopeRefusal::OutsideProjectRoots)
        );
    }
}
