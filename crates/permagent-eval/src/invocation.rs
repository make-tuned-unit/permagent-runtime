//! Pure construction of the `permagent run` invocation for one (task, tier).
//!
//! This is the seam the whole eval turns on, so it is a plain data transform with
//! no I/O and is exhaustively unit-tested. The [`runner`](crate::runner) module
//! turns an [`Invocation`] into a real subprocess.
//!
//! The command shape (validated against `permagent-cli`'s `run` command):
//!
//! ```text
//! permagent run --recipe <recipe> -t <prompt>
//!               --provider <provider> --model <model>
//!               --output-format text [--max-turns <n>]
//! ```
//!
//! with environment:
//! - `PERMAGENT_PATH_ROOT=<data_root>` — isolate this run's session + cost-ledger
//!   database so the whole ledger is exactly this run (parent + any sub-agents),
//!   with no cross-run contamination.
//! - `GOOSE_MODE=auto` — unattended tool execution (headless can't confirm).
//! - `PERMAGENT_DISABLE_KEYRING=1` — read provider secrets from the environment
//!   instead of blocking on the OS keychain.
//! - the tier's `PERMAGENT_PACK_*` pins (when pinning packs).
//!
//! Provider API keys are intentionally NOT set here — they flow through from the
//! ambient environment the operator configured on the machine that runs the eval.

use crate::task::TaskSpec;
use crate::tier::Tier;
use std::path::{Path, PathBuf};

/// A fully-resolved, ready-to-spawn command: program, args, extra environment and
/// working directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    /// The program to execute (the `permagent` binary, or an override path).
    pub program: String,
    /// Argument vector (does not include `program`).
    pub args: Vec<String>,
    /// Environment variables to SET on top of the inherited environment.
    pub envs: Vec<(String, String)>,
    /// Working directory the harness runs in (the copied task workspace).
    pub cwd: PathBuf,
}

impl Invocation {
    /// Look up a set environment value (test/debug helper).
    pub fn env(&self, key: &str) -> Option<&str> {
        self.envs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Render as a copy-pasteable shell-ish line (for `plan` / logs). This is for
    /// human display only — it is not used to actually spawn the process.
    pub fn display_line(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for (k, v) in &self.envs {
            parts.push(format!("{k}={}", shell_quote(v)));
        }
        parts.push(self.program.clone());
        for a in &self.args {
            parts.push(shell_quote(a));
        }
        parts.join(" ")
    }
}

/// Build the invocation for `task` under `tier`, running in `workdir` with an
/// isolated `data_root`, using `permagent_bin` as the program.
pub fn build_invocation(
    task: &TaskSpec,
    tier: &Tier,
    workdir: &Path,
    data_root: &Path,
    permagent_bin: &str,
) -> Invocation {
    let mut args: Vec<String> = vec![
        "run".to_string(),
        "--recipe".to_string(),
        task.recipe.clone(),
        "--provider".to_string(),
        tier.provider.clone(),
        "--model".to_string(),
        tier.model.clone(),
        "--output-format".to_string(),
        "text".to_string(),
    ];
    if let Some(max_turns) = task.max_turns {
        args.push("--max-turns".to_string());
        args.push(max_turns.to_string());
    }
    // The prompt goes last as the `-t/--text` value.
    args.push("-t".to_string());
    args.push(task.prompt.clone());

    let mut envs: Vec<(String, String)> = vec![
        (
            "PERMAGENT_PATH_ROOT".to_string(),
            data_root.to_string_lossy().into_owned(),
        ),
        ("GOOSE_MODE".to_string(), "auto".to_string()),
        ("PERMAGENT_DISABLE_KEYRING".to_string(), "1".to_string()),
    ];
    envs.extend(tier.pack_env());

    Invocation {
        program: permagent_bin.to_string(),
        args,
        envs,
        cwd: workdir.to_path_buf(),
    }
}

/// Minimal shell quoting for display: wrap in single quotes when the value has
/// characters a shell would treat specially, escaping embedded single quotes.
fn shell_quote(s: &str) -> String {
    let needs_quoting = s.is_empty()
        || s.chars()
            .any(|c| c.is_whitespace() || "\"'\\$`*?()[]{}<>|&;#!~".contains(c));
    if !needs_quoting {
        return s.to_string();
    }
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> TaskSpec {
        TaskSpec::from_yaml(
            "id: ttt\ntitle: Tic Tac Toe\nprompt: build tic tac toe\ntest: [node, check.mjs]\n",
        )
        .unwrap()
    }

    #[test]
    fn constructs_expected_command() {
        let tier = Tier::builtin("frontier").unwrap();
        let inv = build_invocation(
            &spec(),
            &tier,
            Path::new("/tmp/work"),
            Path::new("/tmp/data"),
            "permagent",
        );
        assert_eq!(inv.program, "permagent");
        assert_eq!(inv.cwd, PathBuf::from("/tmp/work"));

        // recipe / provider / model / prompt all present and paired.
        let pos = |flag: &str| inv.args.iter().position(|a| a == flag).unwrap();
        assert_eq!(inv.args[pos("--recipe") + 1], "permagent-coding");
        assert_eq!(inv.args[pos("--provider") + 1], "anthropic");
        assert_eq!(inv.args[pos("--model") + 1], "claude-opus-4-8");
        assert_eq!(inv.args[pos("-t") + 1], "build tic tac toe");
        assert_eq!(inv.args[pos("--output-format") + 1], "text");
        assert_eq!(inv.args[0], "run");
    }

    #[test]
    fn sets_isolation_and_headless_env() {
        let tier = Tier::builtin("local").unwrap();
        let inv = build_invocation(
            &spec(),
            &tier,
            Path::new("/w"),
            Path::new("/d"),
            "permagent",
        );
        assert_eq!(inv.env("PERMAGENT_PATH_ROOT"), Some("/d"));
        assert_eq!(inv.env("GOOSE_MODE"), Some("auto"));
        assert_eq!(inv.env("PERMAGENT_DISABLE_KEYRING"), Some("1"));
    }

    #[test]
    fn pins_pack_env_for_pinned_tier_and_not_for_native() {
        let tier = Tier::builtin("local").unwrap();
        let inv = build_invocation(
            &spec(),
            &tier,
            Path::new("/w"),
            Path::new("/d"),
            "permagent",
        );
        assert_eq!(inv.env("PERMAGENT_PACK_EDIT_PROVIDER"), Some("ollama"));
        assert_eq!(inv.env("PERMAGENT_PACK_LOCAL_MODEL"), Some("qwen3"));

        let native = Tier::builtin("local").unwrap().with_pin_packs(false);
        let inv2 = build_invocation(
            &spec(),
            &native,
            Path::new("/w"),
            Path::new("/d"),
            "permagent",
        );
        assert_eq!(inv2.env("PERMAGENT_PACK_EDIT_PROVIDER"), None);
    }

    #[test]
    fn max_turns_is_emitted_only_when_set() {
        let mut s = spec();
        assert!(!build_invocation(
            &s,
            &Tier::custom("c", "p", "m"),
            Path::new("/w"),
            Path::new("/d"),
            "permagent"
        )
        .args
        .iter()
        .any(|a| a == "--max-turns"));
        s.max_turns = Some(25);
        let inv = build_invocation(
            &s,
            &Tier::custom("c", "p", "m"),
            Path::new("/w"),
            Path::new("/d"),
            "permagent",
        );
        let pos = inv.args.iter().position(|a| a == "--max-turns").unwrap();
        assert_eq!(inv.args[pos + 1], "25");
    }

    #[test]
    fn honours_custom_binary_path() {
        let inv = build_invocation(
            &spec(),
            &Tier::custom("c", "ollama", "qwen3"),
            Path::new("/w"),
            Path::new("/d"),
            "/opt/permagent/bin/permagent",
        );
        assert_eq!(inv.program, "/opt/permagent/bin/permagent");
    }

    #[test]
    fn display_line_quotes_the_prompt() {
        let inv = build_invocation(
            &spec(),
            &Tier::custom("c", "ollama", "qwen3"),
            Path::new("/w"),
            Path::new("/d"),
            "permagent",
        );
        let line = inv.display_line();
        assert!(line.contains("'build tic tac toe'"));
        assert!(line.contains("PERMAGENT_PATH_ROOT=/d"));
        assert!(line.contains("permagent run --recipe permagent-coding"));
    }
}
