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
//!
//! The rest of the inherited environment flows through too, EXCEPT the
//! router-family variables ([`SCRUBBED_ENV_PREFIXES`]): pack pins, cheap-tier
//! pins/anchors and budget gates set in the operator's shell would otherwise
//! leak into every child run and contaminate the measurement — most insidiously
//! under `--native-routing`, whose whole point is that the tier sets *no* pack
//! env. The scrub is applied at spawn time by the
//! [`runner`](crate::runner)'s subprocess glue; when a tier pins packs, its own
//! pins in [`Invocation::envs`] are re-set on top of the scrub.

use crate::task::TaskSpec;
use crate::tier::Tier;
use std::path::{Path, PathBuf};

/// Env-var name prefixes scrubbed from the *inherited* environment before a
/// child run is spawned. These are the cost-router family knobs an operator may
/// have exported in their own shell; inheriting them would contaminate the run
/// under measurement:
///
/// - `PERMAGENT_PACK_*` — cost-router pack pins (#720). A tier that pins packs
///   sets exactly its own via [`Invocation::envs`]; a `--native-routing` tier
///   sets none, and must genuinely see none.
/// - `PERMAGENT_CHEAP_*` — cheap-tier pin/anchor overrides (incl. the PIN),
///   which would silently redirect "native" routing to the operator's model.
/// - `PERMAGENT_BUDGET_*` — session/task budget gates, which could abort or
///   degrade an eval run mid-task and skew pass-rate.
///
/// Everything else (notably provider API keys) is inherited on purpose.
pub const SCRUBBED_ENV_PREFIXES: [&str; 3] =
    ["PERMAGENT_PACK_", "PERMAGENT_CHEAP_", "PERMAGENT_BUDGET_"];

/// True when `name` is a router-family variable that must not leak from the
/// operator's environment into a child run (see [`SCRUBBED_ENV_PREFIXES`]).
pub fn is_scrubbed_env(name: &str) -> bool {
    SCRUBBED_ENV_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

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
    fn scrubbed_env_covers_the_router_family_and_nothing_else() {
        // The whole pack / cheap / budget families are scrubbed…
        for name in [
            "PERMAGENT_PACK_EDIT_PROVIDER",
            "PERMAGENT_PACK_LOCAL_MODEL",
            "PERMAGENT_CHEAP_PIN_PROVIDER",
            "PERMAGENT_CHEAP_PIN_MODEL",
            "PERMAGENT_CHEAP_PIN_KEY_ENV",
            "PERMAGENT_CHEAP_ANCHOR_MODEL",
            "PERMAGENT_BUDGET_TASK_HARD_USD",
            "PERMAGENT_BUDGET_SESSION_SOFT_USD",
        ] {
            assert!(is_scrubbed_env(name), "{name} should be scrubbed");
        }
        // …while API keys and the harness's own isolation env are inherited/kept.
        for name in [
            "ANTHROPIC_API_KEY",
            "MOONSHOT_API_KEY",
            "MINIMAX_API_KEY",
            "PATH",
            "HOME",
            "PERMAGENT_PATH_ROOT",
            "PERMAGENT_DISABLE_KEYRING",
            "GOOSE_MODE",
            // The playbook A/B toggle — must flow through (see the dedicated test).
            "PERMAGENT_PLAYBOOK_ENABLED",
        ] {
            assert!(!is_scrubbed_env(name), "{name} should NOT be scrubbed");
        }
    }

    /// Measurement lock-in for the learning playbook (increment 1). The playbook
    /// consultation injects at the harness's decompose step, gated by
    /// `PERMAGENT_PLAYBOOK_ENABLED`. A with-vs-without decompose eval on the mini
    /// drives the two arms by exporting that flag, which reaches every child run
    /// only because it is NOT a scrubbed router-family knob. This pins the
    /// passthrough: a future scrub-prefix change that swept it would silently
    /// disable the A/B seam and make the measurement a misleading null.
    #[test]
    fn playbook_ab_flag_flows_through_to_the_harness() {
        assert!(
            !is_scrubbed_env("PERMAGENT_PLAYBOOK_ENABLED"),
            "PERMAGENT_PLAYBOOK_ENABLED is the playbook A/B toggle the decompose eval drives — \
             scrubbing it would silently disable the measurement seam"
        );
        // Sanity: it does not accidentally match a router-family prefix.
        for prefix in SCRUBBED_ENV_PREFIXES {
            assert!(
                !"PERMAGENT_PLAYBOOK_ENABLED".starts_with(prefix),
                "the playbook flag must not collide with scrubbed prefix {prefix}"
            );
        }
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
