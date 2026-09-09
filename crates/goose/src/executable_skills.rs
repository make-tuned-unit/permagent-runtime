//! Path-containment utilities for on-disk skill packages under the skills
//! root.
//!
//! This module used to also carry the executable-skill *runner*
//! (`run_named`/`run_skill`, gone as of the X-skill DAG) — it reached
//! `tokio::process::Command` directly, with no registry, no inputs schema,
//! and no `verification_approval::decide` call anywhere in the chain. That
//! capability is replaced, not kept alongside, by `crate::skill_run`, which
//! resolves a skill against the `skills` DB table and gates a `command`
//! runner through the same approval ladder that already gates completion
//! checks. What survives here is [`skills_root`] and [`resolve_skill_dir`]:
//! the path-containment logic was already sound (refuses absolute paths and
//! `..` escapes, canonicalizes and checks containment) and `skill_run` reuses
//! it verbatim for a manifest's `skill_path`.

use std::path::{Path, PathBuf};

/// Default skills root relative to the process cwd (repo `skills/`).
pub const DEFAULT_SKILLS_REL: &str = "skills";
/// Env override for tests and deployments.
pub const SKILLS_ROOT_ENV: &str = "PERMAGENT_EXECUTABLE_SKILLS_ROOT";

#[derive(Debug, thiserror::Error)]
pub enum SkillRunError {
    #[error("{0}")]
    Refused(String),
    #[error("{0}")]
    Io(String),
}

/// Resolve the skills root: env override, else `<cwd>/skills`.
pub fn skills_root() -> PathBuf {
    if let Ok(p) = std::env::var(SKILLS_ROOT_ENV) {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(DEFAULT_SKILLS_REL)
}

/// Resolve `name` (a skill directory name or relative path) under `root`.
/// Refuses absolute paths, `..` escape, and anything whose canonical path
/// is not under the canonical root.
pub fn resolve_skill_dir(root: &Path, name: &str) -> Result<PathBuf, SkillRunError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(SkillRunError::Refused("skill name is empty".into()));
    }
    let requested = Path::new(trimmed);
    if requested.is_absolute() {
        return Err(SkillRunError::Refused(
            "absolute skill paths are refused — pass a name under the skills root".into(),
        ));
    }
    if requested
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(SkillRunError::Refused(
            "skill path must not contain '..'".into(),
        ));
    }
    let joined = root.join(requested);
    let root_canon = std::fs::canonicalize(root).map_err(|e| {
        SkillRunError::Refused(format!(
            "skills root '{}' is not accessible: {e}",
            root.display()
        ))
    })?;
    let skill_canon = std::fs::canonicalize(&joined).map_err(|_| {
        SkillRunError::Refused(format!(
            "skill '{}' not found under {}",
            trimmed,
            root.display()
        ))
    })?;
    if !skill_canon.starts_with(&root_canon) {
        return Err(SkillRunError::Refused(format!(
            "skill path '{}' is outside the skills root",
            trimmed
        )));
    }
    Ok(skill_canon)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn path_outside_skills_root_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("examples/hello-json")).unwrap();
        let err = resolve_skill_dir(tmp.path(), "../hello-json").unwrap_err();
        match err {
            SkillRunError::Refused(msg) => assert!(msg.contains(".."), "{msg}"),
            other => panic!("expected refuse, got {other}"),
        }
    }

    #[test]
    fn absolute_path_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let err = resolve_skill_dir(tmp.path(), "/tmp").unwrap_err();
        assert!(matches!(err, SkillRunError::Refused(_)));
    }

    #[test]
    fn a_valid_relative_path_resolves_under_the_root() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("examples/hello-json")).unwrap();
        let root_canon = std::fs::canonicalize(tmp.path()).unwrap();
        let resolved = resolve_skill_dir(tmp.path(), "examples/hello-json").unwrap();
        assert!(resolved.starts_with(&root_canon));
    }
}
