//! The Guard — the security character agent, born of Strix.
//!
//! The Guard continuously probes the user's OWN projects for security flaws
//! and keeps a living fix checklist. It wraps the Strix pentest engine
//! (github.com/usestrix/strix, Apache-2.0) as an external scanner and ingests
//! its SARIF output; the agent layer here is the standing character: the
//! enable flag, the scope guard, and the identity Henry can describe. All
//! internal ids, config keys and log targets keep the `strix` spelling — only
//! the display name is "The Guard".
//!
//! Two rules are in CODE, not in a prompt, because a security agent that runs
//! live exploit tooling must not be able to talk itself out of them:
//!
//! 1. **Scope.** Only paths inside the user's own registered project roots are
//!    scannable. Anything else — a URL, a neighbour's checkout, a system
//!    directory — is refused before the scanner is invoked.
//! 2. **Reporting, not remediation.** The Guard files findings. It never edits
//!    code to "fix" what it found. Sweeps are instructed static-only (no live
//!    traffic, no target modification) via the engine's instruction channel —
//!    an instruction, not an enforced code path. `classify`/`ScanPosture`
//!    exist for a future active-scan gate but are not wired at runtime, so no
//!    claim of "intrusive ops are proposed for approval" is made anywhere.

use std::path::{Path, PathBuf};

use crate::config::Config;

/// Config key in `~/.permagent/config.yaml`. Off by default: a scanner that
/// runs live exploit tooling is switched on deliberately, never by upgrade.
pub const STRIX_ENABLED_KEY: &str = "strix_enabled";
/// Optional `user@host`. When set, sweeps rsync the project there, run `strix`
/// against that machine's Docker (Colima), and pull `.strix` back. A forwarded
/// Docker socket is not enough: the engine bind-mounts the local path, which
/// does not exist on the remote daemon. Empty / unset = scan on this Mac.
pub const STRIX_DOCKER_SSH_KEY: &str = "strix_docker_ssh";
/// Optional identity file for the remote host (`ssh -i`). Also read from the
/// `STRIX_DOCKER_SSH_IDENTITY` env var so launchd does not need a key path in
/// config.yaml. Default SSH config (`Host m1`, agent, etc.) is enough when
/// this is unset.
pub const STRIX_DOCKER_SSH_IDENTITY_KEY: &str = "strix_docker_ssh_identity";
/// Self-knowledge id (also the World roster id and the agent.yaml worker key).
pub const STRIX_FEATURE_ID: &str = "strix";
/// The character's name. The engine underneath stays Strix; the character the
/// user meets is The Guard.
pub const STRIX_NAME: &str = "The Guard";

pub fn is_enabled() -> bool {
    Config::global()
        .get_param::<bool>(STRIX_ENABLED_KEY)
        .unwrap_or(false)
}

/// The remote scanner host, if configured. Whitespace-only is treated as unset
/// so a leftover blank key cannot send rsync to an empty `user@`.
pub fn docker_ssh_target() -> Option<String> {
    Config::global()
        .get_param::<String>(STRIX_DOCKER_SSH_KEY)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Optional `ssh -i` path for the remote host.
pub fn docker_ssh_identity() -> Option<String> {
    if let Ok(env) = std::env::var("STRIX_DOCKER_SSH_IDENTITY") {
        let trimmed = env.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    Config::global()
        .get_param::<String>(STRIX_DOCKER_SSH_IDENTITY_KEY)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
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
///
/// A target that will not canonicalize is REFUSED, never approximated. The
/// fallback to the raw path was the whole guarantee undone: `starts_with` is
/// purely lexical over components and does not resolve `..`, so
/// `/work/app/../../../etc/nope` — which does not exist, so does not resolve —
/// "started with" `/work/app` and was handed back as an approved scan root. A
/// path the Guard cannot resolve is not a path it may scan, and a directory
/// that is not there is no use to the scanner regardless.
pub fn check_scope(target: &str, roots: &[PathBuf]) -> Result<PathBuf, ScopeRefusal> {
    let raw = target.trim();
    if raw.is_empty() || raw.contains("://") {
        return Err(ScopeRefusal::NotAPath);
    }
    let candidate = Path::new(raw);
    let Ok(resolved) = candidate.canonicalize() else {
        return Err(ScopeRefusal::NotAPath);
    };
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
/// and its snapshots — are byte-identical until the Guard is deliberately enabled.
pub const SELF_KNOWLEDGE_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: STRIX_FEATURE_ID,
        display_name: "The Guard",
        category: crate::agents::self_knowledge::FeatureCategory::Worker,
        what_it_does:
            "Continuously probes the user's own projects for security flaws — exposed secrets, \
             vulnerable dependencies, injection and access-control weaknesses, risky \
             configuration — and keeps a living checklist of what to fix, each item carrying \
             its severity, CWE, location, and remediation. Powered by the Strix pentest engine",
        why_it_matters:
            "Security review that happens on its own cadence instead of when someone remembers \
             to ask. When the user asks what is wrong with a project, the Guard's findings are the \
             standing answer — and it only ever reports: it never edits code to fix what it \
             found, and every sweep is instructed to stay static-only, sending no live traffic \
             at the target and modifying nothing",
        state_source: crate::agents::self_knowledge::StateSource::Queryable,
        // The setup lesson: run it FOR the user through the shell — nobody
        // should have to visit a website or edit a plist to arm the Guard.
        // The scanner's model and API key come from Permagent's own config
        // (strix_llm, default Haiku on the provider key already stored), so
        // install + enable is genuinely the whole job.
        teaching: &[
            crate::agents::self_knowledge::TeachingStep {
                title: "Check the ground",
                body: "If `strix_docker_ssh` is set in ~/.permagent/config.yaml, Docker and the \
                       scanner live on THAT host (Colima), not this Mac — a forwarded Docker \
                       socket is not enough because Strix bind-mounts the local path. Check with \
                       `ssh <host> 'PATH=/opt/homebrew/bin:$PATH docker info'` and \
                       `ssh <host> ~/.local/bin/strix --version`. After a reboot of that host, \
                       `ssh <host> 'PATH=/opt/homebrew/bin:$PATH colima start'`. If the key is \
                       unset, Docker and Python 3.12+ must be installed and running here. Report \
                       what you found in one plain sentence before going further.",
                open_surface: None,
                confirm: None,
            },
            crate::agents::self_knowledge::TeachingStep {
                title: "Install the scanner for them",
                body: "Install `strix-agent` on the machine that runs Docker. With \
                       `strix_docker_ssh` set: `ssh <host> 'pipx install strix-agent'` (if pipx \
                       is missing there, brew-install pipx on that host first). Verify with \
                       `ssh <host> ~/.local/bin/strix --help`. This Mac does not need a local \
                       strix or a local Docker VM when that key is set. Without the key: \
                       `pipx install strix-agent` here and verify `~/.local/bin/strix --help`.",
                open_surface: None,
                confirm: None,
            },
            crate::agents::self_knowledge::TeachingStep {
                title: "Costs, stated plainly",
                body: "Tell them: each sweep is a real AI scan of ONE active project (rotating, \
                       least-recently-scanned first) on their API credits, defaulting to a \
                       small fast model (Haiku) on the key they already stored, once a day. \
                       Each scan files a security report with a fix plan as a note on that \
                       project — they can ask you to read it and dispatch a fix goal. The \
                       cadence has a 'Sweep every' picker in Settings; the model can be \
                       changed for them via the strix_llm entry in ~/.permagent/config.yaml. \
                       Never imply it is free.",
                open_surface: None,
                confirm: None,
            },
            crate::agents::self_knowledge::TeachingStep {
                title: "Arm the Guard",
                body: "Bring them to Settings → Models and have them flip 'Enable the Guard' \
                       themselves — a scanner that runs exploit tooling is switched on by the \
                       user, never by you. The first sweep starts within about 15 minutes; \
                       findings land on each project's Overview.",
                open_surface: Some(crate::agents::self_knowledge::SurfaceRef {
                    tab: "Settings",
                    section: Some("models"),
                }),
                confirm: None,
            },
        ],
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

        // Refused: remote targets — the Guard scans the user's own code, not hosts.
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
        // Refused: traversal that does NOT exist on disk. This is the case the
        // guard used to approve — `starts_with` is lexical, so an unresolvable
        // `<root>/../../..` still "started with" the root and came back as an
        // approved scan target.
        let phantom = root.join("..").join("..").join("nowhere-at-all");
        assert_eq!(
            check_scope(phantom.to_str().unwrap(), &roots),
            Err(ScopeRefusal::NotAPath)
        );
        // Refused: a plain path inside the root that is simply not there.
        assert_eq!(
            check_scope(root.join("no-such-dir").to_str().unwrap(), &roots),
            Err(ScopeRefusal::NotAPath)
        );
    }
}
