//! Task specs: the curated coding problems the harness is evaluated against.
//!
//! On disk a task is a directory under the tasks root:
//!
//! ```text
//! tasks/<id>/
//!   task.yaml      # the spec (this module)
//!   workspace/     # seed files copied into the run dir — what the agent sees & edits
//!   oracle/        # the deterministic test — copied OVER the workspace after the
//!                  # run (so the agent cannot tamper with its own grader), then run
//! ```
//!
//! The `test` command is the oracle: it runs in the finished workspace and its
//! exit status is the sole pass/fail signal (0 = solved).

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// The default recipe: the built-in Permagent coding harness (#719), resolved by
/// `permagent run --recipe permagent-coding`.
pub const DEFAULT_RECIPE: &str = "permagent-coding";

/// Default wall-clock ceiling for one harness run, in seconds.
pub const DEFAULT_HARNESS_TIMEOUT_SECS: u64 = 900;

/// Default wall-clock ceiling for one oracle (test) run, in seconds.
pub const DEFAULT_ORACLE_TIMEOUT_SECS: u64 = 120;

fn default_recipe() -> String {
    DEFAULT_RECIPE.to_string()
}

/// The declarative contents of a `task.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TaskSpec {
    /// Stable identifier. Must equal the task directory name.
    pub id: String,

    /// Short human-readable title.
    pub title: String,

    /// Loose grouping (e.g. `game-ui`, `classic`, `bug-fix`) — reporting only.
    #[serde(default)]
    pub category: String,

    /// The objective handed to the coding harness as the run prompt (`-t`).
    pub prompt: String,

    /// Recipe name or path passed to `--recipe`. Defaults to the built-in coding
    /// harness.
    #[serde(default = "default_recipe")]
    pub recipe: String,

    /// The oracle command, as an argv vector, run in the finished workspace.
    /// Exit code 0 means the task is solved. Must be non-empty.
    pub test: Vec<String>,

    /// Optional per-task override of the harness wall-clock ceiling (seconds).
    #[serde(default)]
    pub harness_timeout_secs: Option<u64>,

    /// Optional per-task override of the oracle wall-clock ceiling (seconds).
    #[serde(default)]
    pub oracle_timeout_secs: Option<u64>,

    /// Optional cap on agent turns (`--max-turns`), bounding runaway cost/time.
    #[serde(default)]
    pub max_turns: Option<u32>,
}

impl TaskSpec {
    /// Parse a spec from YAML text (does not validate directory coupling).
    pub fn from_yaml(text: &str) -> Result<Self> {
        let spec: TaskSpec =
            serde_yaml::from_str(text).context("failed to parse task.yaml as a TaskSpec")?;
        spec.validate()?;
        Ok(spec)
    }

    /// Validate the spec's internal invariants (independent of the filesystem).
    pub fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            bail!("task id must not be empty");
        }
        if !is_safe_id(&self.id) {
            bail!(
                "task id {:?} is not filesystem-safe (use a-z, 0-9, '-' and '_')",
                self.id
            );
        }
        if self.title.trim().is_empty() {
            bail!("task {:?}: title must not be empty", self.id);
        }
        if self.prompt.trim().is_empty() {
            bail!("task {:?}: prompt must not be empty", self.id);
        }
        if self.recipe.trim().is_empty() {
            bail!("task {:?}: recipe must not be empty", self.id);
        }
        if self.test.is_empty() {
            bail!(
                "task {:?}: test (oracle argv) must have at least the program name",
                self.id
            );
        }
        if self.test.iter().any(|arg| arg.is_empty()) {
            bail!(
                "task {:?}: test argv must not contain empty arguments",
                self.id
            );
        }
        Ok(())
    }

    /// The effective harness timeout, applying the default when unset.
    pub fn harness_timeout_secs(&self) -> u64 {
        self.harness_timeout_secs
            .unwrap_or(DEFAULT_HARNESS_TIMEOUT_SECS)
    }

    /// The effective oracle timeout, applying the default when unset.
    pub fn oracle_timeout_secs(&self) -> u64 {
        self.oracle_timeout_secs
            .unwrap_or(DEFAULT_ORACLE_TIMEOUT_SECS)
    }
}

/// A task spec paired with its on-disk directory.
#[derive(Debug, Clone)]
pub struct Task {
    pub spec: TaskSpec,
    pub dir: PathBuf,
}

impl Task {
    /// The seed workspace directory (may be absent for from-scratch tasks).
    pub fn workspace_dir(&self) -> PathBuf {
        self.dir.join("workspace")
    }

    /// The hidden oracle directory whose files overwrite the workspace before
    /// grading (tamper-proofing the test).
    pub fn oracle_dir(&self) -> PathBuf {
        self.dir.join("oracle")
    }

    pub fn has_workspace(&self) -> bool {
        self.workspace_dir().is_dir()
    }

    pub fn has_oracle(&self) -> bool {
        self.oracle_dir().is_dir()
    }
}

/// True for identifiers containing only `[a-z0-9_-]` and at least one char.
fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Load one task from its directory, requiring `id` to match the directory name.
pub fn load_task(dir: &Path) -> Result<Task> {
    let manifest = dir.join("task.yaml");
    let text = std::fs::read_to_string(&manifest)
        .with_context(|| format!("reading task manifest {}", manifest.display()))?;
    let spec = TaskSpec::from_yaml(&text)
        .with_context(|| format!("in task manifest {}", manifest.display()))?;

    let dir_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if spec.id != dir_name {
        bail!(
            "task id {:?} does not match its directory name {:?} ({})",
            spec.id,
            dir_name,
            dir.display()
        );
    }
    Ok(Task {
        spec,
        dir: dir.to_path_buf(),
    })
}

/// Load every task directory under `tasks_dir`, sorted by id, rejecting
/// duplicates. A directory with no `task.yaml` is skipped (not an error), so the
/// root can hold READMEs and helpers alongside tasks.
pub fn load_task_set(tasks_dir: &Path) -> Result<Vec<Task>> {
    if !tasks_dir.is_dir() {
        bail!("tasks directory does not exist: {}", tasks_dir.display());
    }
    let mut tasks: Vec<Task> = Vec::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(tasks_dir)
        .with_context(|| format!("reading tasks directory {}", tasks_dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir() && p.join("task.yaml").is_file())
        .collect();
    entries.sort();

    for dir in entries {
        let task = load_task(&dir)?;
        if tasks.iter().any(|t| t.spec.id == task.spec.id) {
            bail!("duplicate task id {:?}", task.spec.id);
        }
        tasks.push(task);
    }
    tasks.sort_by(|a, b| a.spec.id.cmp(&b.spec.id));
    Ok(tasks)
}

/// Filter a loaded task set down to an explicit id selection, preserving order
/// and erroring on any unknown id.
pub fn select_tasks(all: Vec<Task>, ids: &[String]) -> Result<Vec<Task>> {
    if ids.is_empty() {
        return Ok(all);
    }
    let mut selected = Vec::new();
    for id in ids {
        let found = all
            .iter()
            .find(|t| &t.spec.id == id)
            .with_context(|| format!("unknown task id {id:?}"))?;
        selected.push(found.clone());
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_yaml() -> &'static str {
        "id: fizzbuzz\n\
         title: FizzBuzz\n\
         category: classic\n\
         prompt: implement fizzbuzz\n\
         test: [python3, check.py]\n"
    }

    #[test]
    fn parses_minimal_spec_and_applies_defaults() {
        let spec = TaskSpec::from_yaml(minimal_yaml()).unwrap();
        assert_eq!(spec.id, "fizzbuzz");
        assert_eq!(spec.recipe, DEFAULT_RECIPE);
        assert_eq!(
            spec.test,
            vec!["python3".to_string(), "check.py".to_string()]
        );
        assert_eq!(spec.harness_timeout_secs(), DEFAULT_HARNESS_TIMEOUT_SECS);
        assert_eq!(spec.oracle_timeout_secs(), DEFAULT_ORACLE_TIMEOUT_SECS);
        assert_eq!(spec.max_turns, None);
    }

    #[test]
    fn honours_explicit_overrides() {
        let yaml = "id: t1\n\
             title: T\n\
             prompt: do it\n\
             recipe: my-recipe\n\
             test: [bash, run.sh]\n\
             harness_timeout_secs: 60\n\
             oracle_timeout_secs: 5\n\
             max_turns: 12\n";
        let spec = TaskSpec::from_yaml(yaml).unwrap();
        assert_eq!(spec.recipe, "my-recipe");
        assert_eq!(spec.harness_timeout_secs(), 60);
        assert_eq!(spec.oracle_timeout_secs(), 5);
        assert_eq!(spec.max_turns, Some(12));
    }

    #[test]
    fn rejects_empty_prompt() {
        let yaml = "id: t1\ntitle: T\nprompt: '   '\ntest: [x]\n";
        assert!(TaskSpec::from_yaml(yaml).is_err());
    }

    #[test]
    fn rejects_empty_test() {
        let yaml = "id: t1\ntitle: T\nprompt: p\ntest: []\n";
        assert!(TaskSpec::from_yaml(yaml).is_err());
    }

    #[test]
    fn rejects_unsafe_id() {
        let yaml = "id: 'Bad Id!'\ntitle: T\nprompt: p\ntest: [x]\n";
        assert!(TaskSpec::from_yaml(yaml).is_err());
    }

    #[test]
    fn load_task_requires_id_matches_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("mismatch");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("task.yaml"),
            "id: other\ntitle: T\nprompt: p\ntest: [x]\n",
        )
        .unwrap();
        assert!(load_task(&dir).is_err());
    }

    #[test]
    fn load_task_set_skips_non_task_dirs_and_sorts() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for id in ["bravo", "alpha"] {
            let dir = root.join(id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("task.yaml"),
                format!("id: {id}\ntitle: T\nprompt: p\ntest: [x]\n"),
            )
            .unwrap();
        }
        // A stray directory with no task.yaml is ignored, not an error.
        std::fs::create_dir_all(root.join("helpers")).unwrap();
        std::fs::write(root.join("README.md"), "hi").unwrap();

        let tasks = load_task_set(root).unwrap();
        let ids: Vec<&str> = tasks.iter().map(|t| t.spec.id.as_str()).collect();
        assert_eq!(ids, vec!["alpha", "bravo"]);
    }

    #[test]
    fn select_tasks_filters_and_rejects_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for id in ["a", "b", "c"] {
            let dir = root.join(id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("task.yaml"),
                format!("id: {id}\ntitle: T\nprompt: p\ntest: [x]\n"),
            )
            .unwrap();
        }
        let all = load_task_set(root).unwrap();
        let picked = select_tasks(all.clone(), &["c".to_string(), "a".to_string()]).unwrap();
        let ids: Vec<&str> = picked.iter().map(|t| t.spec.id.as_str()).collect();
        assert_eq!(ids, vec!["c", "a"]);

        assert!(select_tasks(all, &["nope".to_string()]).is_err());
    }
}
