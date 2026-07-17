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
//!
//! ## Oracle import discipline (tamper-proofing, enforced)
//!
//! The oracle overlay ([`Task::oracle_dir`]) only protects files that share a
//! name with a pristine oracle file. But an oracle is run *inside the finished
//! workspace* — for `python3 <script>` the interpreter puts the script's own
//! directory (the workspace) first on `sys.path`, and Node resolves bare
//! specifiers there too. So any module an oracle imports that the agent could
//! have written into the workspace is a shadowing hole: a hostile solution can
//! drop a `json.py` (or a `statistics.py`, …) that feeds the grader rigged data
//! and turns a real FAIL into a PASS.
//!
//! To make this a *rule* rather than a matter of authorship luck, every bundled
//! task is validated on load ([`Task::validate_oracle_imports`]):
//!
//! - **Python oracles** may only import `sys` (a built-in, compiled into the
//!   interpreter and thus unshadowable) plus the task's declared
//!   [`deliverables`](TaskSpec::deliverables) — the workspace modules the agent
//!   is *meant* to produce (e.g. `stats` for a `from stats import median`
//!   grader). Any other bare `import X` / `from X import …` is rejected.
//! - **Node oracles** may only use `node:`-prefixed specifiers in static
//!   `import … from "…"` / `require("…")` / `import("…")`. A deliverable is
//!   loaded by an absolute `file://` URL (`import(pathToFileURL(cwd()+"/x.mjs"))`),
//!   which is not a shadowable bare specifier.

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

    /// Workspace modules the oracle is permitted to import — the deliverables the
    /// agent must produce (e.g. `stats` for a `from stats import median` grader,
    /// or `roman` when the task id is `roman-numerals`). Enforced by
    /// [`Task::validate_oracle_imports`]: any Python oracle import beyond the
    /// built-in `sys` and these names, or any non-`node:` Node specifier, is a
    /// shadowing risk and rejected. Empty for oracles that import nothing from
    /// the workspace (e.g. Node graders that load deliverables by `file://` URL).
    #[serde(default)]
    pub deliverables: Vec<String>,
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

    /// Enforce the oracle import discipline (see the module docs): every Python
    /// oracle file may import only `sys` + the declared
    /// [`deliverables`](TaskSpec::deliverables); every Node oracle file may use
    /// only `node:`-prefixed static specifiers. Rejects shadowable imports so a
    /// hostile workspace module can never feed the grader rigged data.
    ///
    /// A no-op for tasks without an `oracle/` directory. Non-source files in the
    /// oracle dir are ignored.
    pub fn validate_oracle_imports(&self) -> Result<()> {
        if !self.has_oracle() {
            return Ok(());
        }
        for file in source_files_under(&self.oracle_dir())? {
            let src = std::fs::read_to_string(&file)
                .with_context(|| format!("reading oracle file {}", file.display()))?;
            match file.extension().and_then(|e| e.to_str()) {
                Some("py") => check_python_imports(&src, &self.spec.deliverables)
                    .with_context(|| format!("oracle {}", file.display()))?,
                Some("mjs") | Some("js") | Some("cjs") => check_node_imports(&src)
                    .with_context(|| format!("oracle {}", file.display()))?,
                _ => {}
            }
        }
        Ok(())
    }
}

/// Collect every regular file under `dir` (recursively), sorted for determinism.
fn source_files_under(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d)
            .with_context(|| format!("reading oracle directory {}", d.display()))?
        {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Whether a Python oracle may import top-level module `module`: only the
/// built-in `sys` (unshadowable) and the task's declared deliverables.
fn python_import_allowed(module: &str, deliverables: &[String]) -> bool {
    module == "sys" || deliverables.iter().any(|d| d == module)
}

/// The top-level module names statically imported by a Python source file.
/// Best-effort line scanner (skips comment lines); it does not resolve dynamic
/// `__import__`/`importlib`, which the discipline forbids relying on anyway.
pub fn python_imported_modules(src: &str) -> Vec<String> {
    let mut mods = Vec::new();
    for line in src.lines() {
        let t = line.trim_start();
        if t.starts_with('#') {
            continue;
        }
        if let Some(rest) = t.strip_prefix("import ") {
            // `import a.b as c, d` -> top-level of each comma-separated name.
            for part in rest.split(',') {
                if let Some(name) = part.split_whitespace().next() {
                    let top = name.split('.').next().unwrap_or("");
                    if !top.is_empty() {
                        mods.push(top.to_string());
                    }
                }
            }
        } else if let Some(rest) = t.strip_prefix("from ") {
            // `from a.b import x` -> `a`; a relative `from . import x` -> `.`.
            if let Some(module) = rest.split_whitespace().next() {
                if module.starts_with('.') {
                    mods.push(module.to_string());
                } else {
                    let top = module.split('.').next().unwrap_or("").trim();
                    if !top.is_empty() {
                        mods.push(top.to_string());
                    }
                }
            }
        }
    }
    mods
}

/// Reject any Python import outside the allowlist (`sys` + `deliverables`).
fn check_python_imports(src: &str, deliverables: &[String]) -> Result<()> {
    for module in python_imported_modules(src) {
        if !python_import_allowed(&module, deliverables) {
            bail!(
                "Python oracle imports {module:?}, which is not the built-in `sys` \
                 nor a declared deliverable ({deliverables:?}); a workspace \
                 `{module}.py` could shadow it and rig the grade. Declare it under \
                 `deliverables:` only if it is a workspace artifact the agent must \
                 produce, otherwise the oracle must not import it."
            );
        }
    }
    Ok(())
}

/// The static import/require specifiers used by a Node source file. Best-effort
/// line scanner: `import … from "x"`, side-effect `import "x"`, `require("x")`
/// and literal `import("x")`. A computed `import(pathToFileURL(...))` has no
/// literal specifier and is intentionally not matched.
pub fn node_static_specifiers(src: &str) -> Vec<String> {
    let mut specs = Vec::new();
    for line in src.lines() {
        let t = line.trim_start();
        if t.starts_with("//") {
            continue;
        }
        // `import … from "x"` / `export … from "x"`.
        if (t.starts_with("import ") || t.starts_with("export ")) && t.contains(" from ") {
            if let Some(after) = t.split(" from ").nth(1) {
                if let Some(s) = first_string_literal(after) {
                    specs.push(s);
                }
            }
        }
        // Side-effect `import "x"`.
        if let Some(rest) = t.strip_prefix("import ") {
            let rest = rest.trim_start();
            if rest.starts_with('"') || rest.starts_with('\'') {
                if let Some(s) = first_string_literal(rest) {
                    specs.push(s);
                }
            }
        }
        // `require("x")` and literal `import("x")`. Each split segment after a
        // marker is the call's argument text; a computed argument (e.g.
        // `import(pathToFileURL(...))`) does not start with a quote and is
        // skipped by `first_string_literal`.
        for marker in ["require(", "import("] {
            for after in t.split(marker).skip(1) {
                if let Some(s) = first_string_literal(after) {
                    specs.push(s);
                }
            }
        }
    }
    specs
}

/// Extract the first single- or double-quoted string literal at/near the start
/// of `s` (used to read an import/require specifier). Returns `None` when `s`
/// does not begin (after optional whitespace) with a quote.
fn first_string_literal(s: &str) -> Option<String> {
    let mut chars = s.trim_start().chars();
    let quote = chars.next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let mut out = String::new();
    for c in chars {
        if c == quote {
            return Some(out); // closing quote reached
        }
        out.push(c);
    }
    None // unterminated literal
}

/// Reject any Node specifier that is not `node:`-prefixed.
fn check_node_imports(src: &str) -> Result<()> {
    for spec in node_static_specifiers(src) {
        if !spec.starts_with("node:") {
            bail!(
                "Node oracle imports {spec:?}, which is not a `node:` built-in; a \
                 workspace module could shadow a bare specifier and rig the grade. \
                 Load deliverables by absolute `file://` URL \
                 (`import(pathToFileURL(process.cwd() + \"/<file>\"))`) instead."
            );
        }
    }
    Ok(())
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
    let task = Task {
        spec,
        dir: dir.to_path_buf(),
    };
    task.validate_oracle_imports()
        .with_context(|| format!("in task {}", dir.display()))?;
    Ok(task)
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
    fn python_scanner_extracts_top_level_modules() {
        let src = "#!/usr/bin/env python3\n\
                   # import commented_out\n\
                   import sys\n\
                   import os.path as p\n\
                   import json, statistics\n\
                       from stats import median\n\
                   from a.b.c import thing\n\
                   from . import rel\n\
                   print(\"could not import fizzbuzz\")\n";
        let mods = python_imported_modules(src);
        assert_eq!(
            mods,
            vec!["sys", "os", "json", "statistics", "stats", "a", "."]
        );
        // The string 'could not import fizzbuzz' must NOT be picked up.
        assert!(!mods.iter().any(|m| m == "fizzbuzz"));
        // The commented `import commented_out` must NOT be picked up.
        assert!(!mods.iter().any(|m| m == "commented_out"));
    }

    #[test]
    fn node_scanner_extracts_specifiers_but_not_computed_imports() {
        let src = "// import ./ignored-comment\n\
                   import fs from \"node:fs\";\n\
                   import { x } from 'node:url';\n\
                   import \"node:process\";\n\
                   const y = require(\"node:path\");\n\
                   const z = await import(\"./rigged.mjs\");\n\
                   const w = await import(pathToFileURL(process.cwd() + \"/merge.mjs\").href);\n";
        let specs = node_static_specifiers(src);
        assert!(specs.contains(&"node:fs".to_string()));
        assert!(specs.contains(&"node:url".to_string()));
        assert!(specs.contains(&"node:process".to_string()));
        assert!(specs.contains(&"node:path".to_string()));
        // A literal dynamic import IS caught (it is shadowable)…
        assert!(specs.contains(&"./rigged.mjs".to_string()));
        // …but a computed `import(pathToFileURL(...))` has no literal specifier.
        assert!(!specs.iter().any(|s| s.contains("merge.mjs")));
    }

    /// Build a task dir with a given oracle file and optional deliverables.
    fn task_with_oracle(
        dir: &Path,
        id: &str,
        oracle_name: &str,
        oracle_src: &str,
        deliverables: &str,
        workspace: &[(&str, &str)],
    ) {
        std::fs::create_dir_all(dir.join("oracle")).unwrap();
        std::fs::write(
            dir.join("task.yaml"),
            format!(
                "id: {id}\ntitle: T\nprompt: p\ntest: [runner, {oracle_name}]\ndeliverables: {deliverables}\n"
            ),
        )
        .unwrap();
        std::fs::write(dir.join("oracle").join(oracle_name), oracle_src).unwrap();
        if !workspace.is_empty() {
            std::fs::create_dir_all(dir.join("workspace")).unwrap();
            for (n, c) in workspace {
                std::fs::write(dir.join("workspace").join(n), c).unwrap();
            }
        }
    }

    #[test]
    fn oracle_import_discipline_allows_sys_and_declared_deliverables() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("good");
        task_with_oracle(
            &dir,
            "good",
            "check.py",
            "import sys\nfrom stats import median\n",
            "[stats]",
            &[],
        );
        // Loading enforces the discipline; a compliant oracle loads fine.
        assert!(load_task(&dir).is_ok());
    }

    /// The F12 regression from the #726 review: an oracle that `import json`
    /// while the workspace ships a rigged `json.py` must be REJECTED by
    /// validation — never silently graded PASS via the shadow module.
    #[test]
    fn oracle_importing_shadowable_stdlib_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("evil");
        task_with_oracle(
            &dir,
            "evil",
            "check.py",
            // Grader trusts `json` — but sys.path[0] is the workspace…
            "import sys\nimport json\n\
             def main():\n    return 0 if json.loads('rigged')['ok'] else 1\n\
             sys.exit(main())\n",
            "[]",
            // …where the agent dropped a rigged shadow module.
            &[("json.py", "def loads(_s):\n    return {'ok': True}\n")],
        );
        let err = load_task(&dir).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("json"),
            "rejection must name the shadowable import: {msg}"
        );
        assert!(
            msg.contains("deliverable") || msg.contains("shadow"),
            "rejection must explain the shadowing risk: {msg}"
        );
    }

    #[test]
    fn oracle_importing_undeclared_deliverable_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("undeclared");
        // Imports `stats` but declares no deliverables — must be rejected so the
        // allowlist is a conscious act, not inferred from the oracle itself.
        task_with_oracle(
            &dir,
            "undeclared",
            "check.py",
            "import sys\nfrom stats import median\n",
            "[]",
            &[],
        );
        assert!(load_task(&dir).is_err());
    }

    #[test]
    fn node_oracle_with_bare_specifier_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("nodeevil");
        task_with_oracle(
            &dir,
            "nodeevil",
            "check.mjs",
            "import fs from \"node:fs\";\nimport helper from \"rigged-helper\";\n",
            "[]",
            &[],
        );
        let err = load_task(&dir).unwrap_err();
        assert!(format!("{err:#}").contains("rigged-helper"));
    }

    #[test]
    fn node_oracle_with_only_node_specifiers_is_allowed() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("nodegood");
        task_with_oracle(
            &dir,
            "nodegood",
            "check.mjs",
            "import fs from \"node:fs\";\nimport { pathToFileURL } from \"node:url\";\n\
             const m = await import(pathToFileURL(process.cwd() + \"/x.mjs\").href);\n",
            "[]",
            &[],
        );
        assert!(load_task(&dir).is_ok());
    }

    #[test]
    fn bundled_tasks_pass_oracle_import_discipline() {
        // Loading the shipped task set enforces the discipline on every oracle;
        // if a future task adds a bare `import json`, this test goes red.
        let tasks_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tasks"));
        let tasks = load_task_set(tasks_dir).expect("bundled task set must load & pass discipline");
        assert!(
            tasks.len() >= 7,
            "expected the curated task set to be present"
        );
        for t in &tasks {
            t.validate_oracle_imports().unwrap_or_else(|e| {
                panic!("bundled task {} violates discipline: {e:#}", t.spec.id)
            });
        }
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
