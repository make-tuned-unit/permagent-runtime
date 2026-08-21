//! Executable skill packages — skills as runnable artifacts, not markdown.
//!
//! A package is a directory with `manifest.toml` or `package.json` naming an
//! `entrypoint`. The runner executes that entrypoint and returns structured
//! stdout/stderr + exit code. Paths outside the skills root are refused.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

/// Default skills root relative to the process cwd (repo `skills/`).
pub const DEFAULT_SKILLS_REL: &str = "skills";
/// Env override for tests and deployments.
pub const SKILLS_ROOT_ENV: &str = "PERMAGENT_EXECUTABLE_SKILLS_ROOT";
const RUN_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SkillManifest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub entrypoint: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SkillRunResult {
    pub name: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parsed_json: Option<serde_json::Value>,
}

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

/// Load a skill directory's manifest (`package.json` preferred, then `manifest.toml`).
pub fn load_manifest(dir: &Path) -> Result<SkillManifest, SkillRunError> {
    let pkg = dir.join("package.json");
    if pkg.is_file() {
        let raw = std::fs::read_to_string(&pkg).map_err(|e| SkillRunError::Io(e.to_string()))?;
        return serde_json::from_str(&raw)
            .map_err(|e| SkillRunError::Io(format!("package.json: {e}")));
    }
    let toml_path = dir.join("manifest.toml");
    if toml_path.is_file() {
        let raw =
            std::fs::read_to_string(&toml_path).map_err(|e| SkillRunError::Io(e.to_string()))?;
        return parse_minimal_toml(&raw)
            .ok_or_else(|| SkillRunError::Io("manifest.toml missing name/entrypoint".into()));
    }
    Err(SkillRunError::Io(format!(
        "no package.json or manifest.toml in {}",
        dir.display()
    )))
}

/// Tiny TOML subset: `name = "..."`, `description = "..."`, `entrypoint = "..."`.
fn parse_minimal_toml(raw: &str) -> Option<SkillManifest> {
    let mut name = None;
    let mut description = String::new();
    let mut entrypoint = None;
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim();
        let v = v.trim().trim_matches('"').to_string();
        match k {
            "name" => name = Some(v),
            "description" => description = v,
            "entrypoint" => entrypoint = Some(v),
            _ => {}
        }
    }
    Some(SkillManifest {
        name: name?,
        description,
        entrypoint: entrypoint?,
    })
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

/// Execute one skill by name/path under `root`.
pub async fn run_skill(root: &Path, name: &str) -> Result<SkillRunResult, SkillRunError> {
    let dir = resolve_skill_dir(root, name)?;
    let manifest = load_manifest(&dir)?;
    let entry = dir.join(&manifest.entrypoint);
    if !entry.is_file() {
        return Err(SkillRunError::Io(format!(
            "entrypoint '{}' not found in {}",
            manifest.entrypoint,
            dir.display()
        )));
    }

    let mut cmd = if looks_like_shell(&manifest.entrypoint) {
        let mut c = Command::new("sh");
        c.arg(&entry);
        c
    } else {
        Command::new(&entry)
    };
    cmd.current_dir(&dir);
    crate::subprocess::configure_subprocess(&mut cmd);

    let output = timeout(RUN_TIMEOUT, cmd.output())
        .await
        .map_err(|_| SkillRunError::Io("skill timed out".into()))?
        .map_err(|e| SkillRunError::Io(e.to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let parsed_json = serde_json::from_str(stdout.trim()).ok();
    let exit_code = output.status.code().unwrap_or(-1);

    Ok(SkillRunResult {
        name: manifest.name,
        exit_code,
        stdout,
        stderr,
        parsed_json,
    })
}

fn looks_like_shell(entrypoint: &str) -> bool {
    entrypoint.ends_with(".sh") || entrypoint.ends_with(".bash")
}

/// Convenience: run `name` against [`skills_root`].
pub async fn run_named(name: &str) -> Result<SkillRunResult, SkillRunError> {
    run_skill(&skills_root(), name).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_example(dir: &Path) {
        let skill = dir.join("examples").join("hello-json");
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("package.json"),
            r#"{"name":"hello-json","description":"fixed JSON","entrypoint":"run.sh"}"#,
        )
        .unwrap();
        fs::write(
            skill.join("run.sh"),
            "#!/bin/sh\necho '{\"ok\":true,\"skill\":\"hello-json\"}'\n",
        )
        .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn repo_example_skill_runs() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("skills");
        let root = std::fs::canonicalize(&root).expect("repo skills/ dir");
        let result = run_skill(&root, "examples/hello-json")
            .await
            .expect("repo example skill");
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.parsed_json.unwrap()["skill"], "hello-json");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn runner_executes_example_skill_json() {
        let tmp = tempfile::tempdir().unwrap();
        write_example(tmp.path());
        let result = run_skill(tmp.path(), "examples/hello-json")
            .await
            .expect("example skill should run");
        assert_eq!(result.exit_code, 0);
        let json = result.parsed_json.expect("stdout is JSON");
        assert_eq!(json["ok"], true);
        assert_eq!(json["skill"], "hello-json");
    }

    #[tokio::test]
    async fn path_outside_skills_root_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        write_example(tmp.path());
        let err = run_skill(tmp.path(), "../hello-json").await.unwrap_err();
        match err {
            SkillRunError::Refused(msg) => assert!(msg.contains(".."), "{msg}"),
            other => panic!("expected refuse, got {other}"),
        }
    }

    #[tokio::test]
    async fn absolute_path_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        write_example(tmp.path());
        let err = run_skill(tmp.path(), "/tmp").await.unwrap_err();
        assert!(matches!(err, SkillRunError::Refused(_)));
    }
}
