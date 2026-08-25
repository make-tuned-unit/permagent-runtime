//! Pure construction of the `permagent run` invocation for one (task, tier).
//!
//! This is the seam the whole eval turns on, so it is a plain data transform with
//! no I/O and is exhaustively unit-tested. The [`runner`](crate::runner) module
//! turns an [`Invocation`] into a real subprocess.
//!
//! The command shape (validated against `permagent-cli`'s `run` command):
//!
//! ```text
//! permagent run --recipe <recipe_path>
//!               --provider <provider> --model <model>
//!               --output-format text [--max-turns <n>]
//! ```
//!
//! `--recipe`, `-t`/`--text` and `-i`/`--instructions` are declared pairwise
//! mutually exclusive on `permagent-cli`'s `InputOptions`
//! (`crates/goose-cli/src/cli.rs:188-220`, three `conflicts_with` pairs) — so
//! this crate NEVER emits `-t`/`--text`/`-i`/`--instructions` alongside
//! `--recipe`. Every task runs via a recipe file, and the task's prompt is
//! embedded INSIDE that recipe as its own `prompt:` key (see
//! [`recipe_with_prompt`]) rather than passed as a separate flag — a headless
//! recipe run takes its prompt from `recipe.prompt`
//! (`crates/goose-cli/src/cli.rs:1421`, `:1586-1600`,
//! `crates/goose-cli/src/recipes/extract_from_cli.rs:49-52`). `recipe_path`
//! is the per-task recipe file the [`runner`](crate::runner) writes into the
//! run's scratch dir before spawning; `build_invocation` itself does no I/O.
//!
//! with environment:
//! - `PERMAGENT_PATH_ROOT=<data_root>` — isolate this run's session + cost-ledger
//!   database so the whole ledger is exactly this run (parent + any sub-agents),
//!   with no cross-run contamination.
//! - `GOOSE_MODE=auto` — unattended tool execution (headless can't confirm).
//! - `PERMAGENT_DISABLE_KEYRING=1` — read provider secrets from the environment
//!   instead of blocking on the OS keychain. Set UNLESS the invocation was built
//!   with `use_keyring: true` (`--use-keyring`), in which case this var is left
//!   unset AND is additionally listed in [`Invocation::unset_envs`] so the
//!   [`runner`](crate::runner) removes any copy an operator exported in their own
//!   shell — otherwise that export would silently defeat the flag. Use
//!   `--use-keyring` when provider secrets live only in the OS keychain (e.g. the
//!   signed bundled CLI reading macOS Keychain service `permagent`/account
//!   `secrets`) and are not present as environment variables.
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
    /// Environment variable NAMES to REMOVE from the inherited environment
    /// before [`envs`](Self::envs) is applied — e.g. `PERMAGENT_DISABLE_KEYRING`
    /// when the invocation was built with `use_keyring: true`, so an operator's
    /// own shell export of it cannot silently defeat `--use-keyring`. Applied by
    /// the [`runner`](crate::runner)'s subprocess glue via `Command::env_remove`.
    pub unset_envs: Vec<String>,
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
    /// human display only — it is not used to actually spawn the process. Each
    /// [`unset_envs`](Self::unset_envs) entry renders as a leading `env -u NAME`
    /// so the removal is visible, not just the sets.
    pub fn display_line(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for name in &self.unset_envs {
            parts.push(format!("env -u {name}"));
        }
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
/// isolated `data_root`, using `permagent_bin` as the program and
/// `recipe_path` as the (already-written, prompt-embedded) recipe file to
/// pass to `--recipe`. When `use_keyring` is true, `PERMAGENT_DISABLE_KEYRING`
/// is left unset (rather than forced to `1`) AND is added to
/// [`Invocation::unset_envs`], so the child can read provider secrets from the
/// OS keychain and an operator's own shell export of the disable var cannot
/// silently override the flag.
///
/// Deliberately never emits `-t`/`--text` or `-i`/`--instructions`: see the
/// module docs for why (`crates/goose-cli/src/cli.rs:188-220` — they conflict
/// with `--recipe` and the CLI refuses the command outright if both are
/// present).
pub fn build_invocation(
    task: &TaskSpec,
    tier: &Tier,
    workdir: &Path,
    data_root: &Path,
    recipe_path: &Path,
    permagent_bin: &str,
    use_keyring: bool,
) -> Invocation {
    let mut args: Vec<String> = vec![
        "run".to_string(),
        "--recipe".to_string(),
        recipe_path.to_string_lossy().into_owned(),
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
    // NOTE: no `-t`/`--text`, no `-i`/`--instructions`. The task's prompt is
    // embedded INSIDE `recipe_path` (see `recipe_with_prompt`) — do not
    // "helpfully" add either flag back here, they are declared mutually
    // exclusive with `--recipe` and the run will be refused.

    let mut envs: Vec<(String, String)> = vec![
        (
            "PERMAGENT_PATH_ROOT".to_string(),
            data_root.to_string_lossy().into_owned(),
        ),
        ("GOOSE_MODE".to_string(), "auto".to_string()),
    ];
    let mut unset_envs: Vec<String> = Vec::new();
    if use_keyring {
        unset_envs.push("PERMAGENT_DISABLE_KEYRING".to_string());
    } else {
        envs.push(("PERMAGENT_DISABLE_KEYRING".to_string(), "1".to_string()));
    }
    envs.extend(tier.pack_env());

    Invocation {
        program: permagent_bin.to_string(),
        args,
        envs,
        unset_envs,
        cwd: workdir.to_path_buf(),
    }
}

/// Produce a recipe YAML with `prompt` embedded as the top-level `prompt:`
/// key — replacing any existing `prompt:` key already in `base_recipe_yaml`
/// rather than duplicating it (a duplicate top-level key makes `serde_yaml`
/// error at load; see [`strip_top_level_key`]).
///
/// The prompt is written as a YAML block literal, `prompt: |-` (STRIP
/// chomping), with every line indented two spaces past the key's own column
/// (column 0). Because the block's content is delimited purely by
/// indentation, a prompt line that merely LOOKS like YAML (e.g. `foo: bar`)
/// is never (mis)parsed as a mapping key — it stays literal text.
///
/// Strip chomping (rather than the default `|` clip) is what makes the round
/// trip through a YAML parser exact for the common case of a prompt with no
/// trailing newline — which is how every task's `prompt:` field in this crate
/// is written (e.g. `"build tic tac toe"`): clip would silently ADD one on
/// the way back out. The trade-off is that trailing newlines on `prompt`
/// itself are normalized away (not round-tripped) — acceptable, since a
/// prompt's trailing blank lines are never semantically meaningful to the
/// agent.
///
/// `title` (and every other key) is left completely untouched: this only
/// ever adds or replaces the `prompt:` key. That matters beyond cosmetics —
/// `is_coding_harness_recipe` gates repo-map injection on an EXACT title
/// match (`crates/goose-cli/src/recipes/builtin_recipes.rs:31`), so mangling
/// the title here would silently turn off that context for every run.
pub fn recipe_with_prompt(base_recipe_yaml: &str, prompt: &str) -> String {
    let base = strip_top_level_key(base_recipe_yaml, "prompt");
    let base = base.trim_end_matches('\n');

    let mut out = String::from(base);
    out.push('\n');
    out.push_str("prompt: |-\n");

    let trimmed_prompt = prompt.trim_end_matches('\n');
    if !trimmed_prompt.is_empty() {
        for line in trimmed_prompt.split('\n') {
            if line.is_empty() {
                // A blank line inside the block scalar.
                out.push('\n');
            } else {
                out.push_str("  ");
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out
}

/// Remove a top-level `"{key}:"` line — and, if present, its indented or
/// blank continuation lines (a full block-scalar or nested-mapping value) —
/// from `yaml`, leaving every other line untouched. A purely textual,
/// indentation-based operation (not a real YAML parse): it looks only for a
/// line starting with `"{key}:"` at column 0, then consumes lines that are
/// blank or indented (i.e. belong to that key's value) up to the next
/// column-0 line or EOF.
fn strip_top_level_key(yaml: &str, key: &str) -> String {
    let prefix = format!("{key}:");
    let mut out_lines: Vec<&str> = Vec::new();
    let mut lines = yaml.lines().peekable();
    while let Some(line) = lines.next() {
        if line.starts_with(&prefix) {
            while let Some(&next) = lines.peek() {
                if next.is_empty() || next.starts_with(' ') || next.starts_with('\t') {
                    lines.next();
                } else {
                    break;
                }
            }
            continue;
        }
        out_lines.push(line);
    }
    out_lines.join("\n")
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

    /// The per-task recipe file path (already-written, prompt-embedded) that
    /// every `build_invocation` call needs — a placeholder here since this
    /// module never does I/O.
    fn recipe_path() -> PathBuf {
        PathBuf::from("/r/recipe.yaml")
    }

    #[test]
    fn constructs_expected_command() {
        let tier = Tier::builtin("frontier").unwrap();
        let inv = build_invocation(
            &spec(),
            &tier,
            Path::new("/tmp/work"),
            Path::new("/tmp/data"),
            &recipe_path(),
            "permagent",
            false,
        );
        assert_eq!(inv.program, "permagent");
        assert_eq!(inv.cwd, PathBuf::from("/tmp/work"));

        // recipe / provider / model all present and paired; the PROMPT is not
        // a separate arg — it lives inside the recipe file (see
        // `recipe_with_prompt`), never as `-t`/`--text`.
        let pos = |flag: &str| inv.args.iter().position(|a| a == flag).unwrap();
        assert_eq!(inv.args[pos("--recipe") + 1], "/r/recipe.yaml");
        assert_eq!(inv.args[pos("--provider") + 1], "anthropic");
        assert_eq!(inv.args[pos("--model") + 1], "claude-opus-4-8");
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
            &recipe_path(),
            "permagent",
            false,
        );
        assert_eq!(inv.env("PERMAGENT_PATH_ROOT"), Some("/d"));
        assert_eq!(inv.env("GOOSE_MODE"), Some("auto"));
        assert_eq!(inv.env("PERMAGENT_DISABLE_KEYRING"), Some("1"));
        assert!(inv.unset_envs.is_empty());
    }

    #[test]
    fn use_keyring_omits_disable_var_and_lists_it_for_removal() {
        let tier = Tier::builtin("local").unwrap();
        let inv = build_invocation(
            &spec(),
            &tier,
            Path::new("/w"),
            Path::new("/d"),
            &recipe_path(),
            "permagent",
            true,
        );
        assert_eq!(inv.env("PERMAGENT_DISABLE_KEYRING"), None);
        assert_eq!(
            inv.unset_envs,
            vec!["PERMAGENT_DISABLE_KEYRING".to_string()]
        );
        // Everything else about the invocation is unaffected.
        assert_eq!(inv.env("PERMAGENT_PATH_ROOT"), Some("/d"));
        assert_eq!(inv.env("GOOSE_MODE"), Some("auto"));
    }

    #[test]
    fn display_line_renders_the_unset_env_readably() {
        let inv = build_invocation(
            &spec(),
            &Tier::builtin("local").unwrap(),
            Path::new("/w"),
            Path::new("/d"),
            &recipe_path(),
            "permagent",
            true,
        );
        let line = inv.display_line();
        assert!(line.contains("env -u PERMAGENT_DISABLE_KEYRING"), "{line}");
        assert!(
            !line.contains("PERMAGENT_DISABLE_KEYRING=1"),
            "the disable var must not also be SET when --use-keyring is on: {line}"
        );
    }

    #[test]
    fn pins_pack_env_for_pinned_tier_and_not_for_native() {
        let tier = Tier::builtin("local").unwrap();
        let inv = build_invocation(
            &spec(),
            &tier,
            Path::new("/w"),
            Path::new("/d"),
            &recipe_path(),
            "permagent",
            false,
        );
        assert_eq!(inv.env("PERMAGENT_PACK_EDIT_PROVIDER"), Some("ollama"));
        assert_eq!(inv.env("PERMAGENT_PACK_LOCAL_MODEL"), Some("qwen3"));

        let native = Tier::builtin("local").unwrap().with_pin_packs(false);
        let inv2 = build_invocation(
            &spec(),
            &native,
            Path::new("/w"),
            Path::new("/d"),
            &recipe_path(),
            "permagent",
            false,
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
            &recipe_path(),
            "permagent",
            false
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
            &recipe_path(),
            "permagent",
            false,
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
            &recipe_path(),
            "/opt/permagent/bin/permagent",
            false,
        );
        assert_eq!(inv.program, "/opt/permagent/bin/permagent");
    }

    /// The whole point of the peer-reported bug fix: `--recipe` and
    /// `-t`/`--text`/`-i`/`--instructions` are declared pairwise mutually
    /// exclusive on `permagent-cli`'s `InputOptions`
    /// (`crates/goose-cli/src/cli.rs:188-220`) — the CLI refuses to run if
    /// both are present. This asserts the invariant directly so nobody
    /// "helpfully" adds `-t <prompt>` back.
    #[test]
    fn argv_never_carries_both_a_recipe_and_a_text_flag() {
        let mut s = spec();
        s.prompt = "build tic tac toe".to_string();
        let inv = build_invocation(
            &s,
            &Tier::builtin("frontier").unwrap(),
            Path::new("/w"),
            Path::new("/d"),
            &recipe_path(),
            "permagent",
            false,
        );
        assert!(inv.args.iter().any(|a| a == "--recipe"), "{:?}", inv.args);
        for forbidden in ["-t", "--text", "-i", "--instructions"] {
            assert!(
                !inv.args.iter().any(|a| a == forbidden),
                "argv must never carry {forbidden} alongside --recipe: {:?}",
                inv.args
            );
        }
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
    fn display_line_shows_the_recipe_path_and_no_prompt_flag() {
        let inv = build_invocation(
            &spec(),
            &Tier::custom("c", "ollama", "qwen3"),
            Path::new("/w"),
            Path::new("/d"),
            Path::new("/data/run-1/recipe.yaml"),
            "permagent",
            false,
        );
        let line = inv.display_line();
        assert!(line.contains("PERMAGENT_PATH_ROOT=/d"));
        assert!(
            line.contains("permagent run --recipe /data/run-1/recipe.yaml"),
            "{line}"
        );
        // The prompt text itself never appears on the command line — it is
        // inside the recipe file.
        assert!(!line.contains("build tic tac toe"), "{line}");
        assert!(!line.contains(" -t "), "{line}");
    }

    // --- recipe_with_prompt ---------------------------------------------

    /// A minimal but representative base recipe (mirrors the shape of
    /// `crates/goose-cli/src/recipes/builtin/permagent-coding.yaml`: version,
    /// title, description, instructions, no pre-existing `prompt:` key).
    fn base_recipe() -> String {
        [
            "version: \"1.0.0\"",
            "title: Permagent Coding Harness",
            "description: >-",
            "  a coding harness",
            "instructions: |",
            "  You are the harness.",
            "  Do good work.",
            "",
        ]
        .join("\n")
    }

    #[test]
    fn recipe_with_prompt_round_trips_a_single_line_prompt() {
        let yaml = recipe_with_prompt(&base_recipe(), "build tic tac toe");
        let value: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("valid yaml");
        assert_eq!(
            value["prompt"].as_str(),
            Some("build tic tac toe"),
            "{yaml}"
        );
    }

    #[test]
    fn recipe_with_prompt_replaces_rather_than_duplicates_an_existing_prompt_key() {
        let base = format!("{}prompt: stale prompt\n", base_recipe());
        let yaml = recipe_with_prompt(&base, "fresh prompt");
        // Exactly one `prompt:` top-level key.
        let prompt_key_lines = yaml.lines().filter(|l| l.starts_with("prompt:")).count();
        assert_eq!(prompt_key_lines, 1, "{yaml}");
        let value: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("valid yaml");
        assert_eq!(value["prompt"].as_str(), Some("fresh prompt"), "{yaml}");
    }

    #[test]
    fn recipe_with_prompt_replaces_a_block_style_existing_prompt_key() {
        let base = format!(
            "{}prompt: |\n  stale line one\n  stale line two\ninstructions_after: keep me\n",
            base_recipe()
        );
        let yaml = recipe_with_prompt(&base, "fresh prompt");
        let prompt_key_lines = yaml.lines().filter(|l| l.starts_with("prompt:")).count();
        assert_eq!(prompt_key_lines, 1, "{yaml}");
        assert!(yaml.contains("instructions_after: keep me"), "{yaml}");
        let value: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("valid yaml");
        assert_eq!(value["prompt"].as_str(), Some("fresh prompt"), "{yaml}");
    }

    #[test]
    fn recipe_with_prompt_keeps_multiline_prompt_line_breaks() {
        let prompt = "line one\nline two\nline three";
        let yaml = recipe_with_prompt(&base_recipe(), prompt);
        let value: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("valid yaml");
        assert_eq!(value["prompt"].as_str(), Some(prompt), "{yaml}");
    }

    #[test]
    fn recipe_with_prompt_survives_a_prompt_line_that_looks_like_yaml() {
        // If this were spliced in unindented (or as a plain scalar), a line
        // like `foo: bar` would be (mis)parsed as a mapping key instead of
        // staying literal prompt text.
        let prompt = "context:\nfoo: bar\ndo the thing";
        let yaml = recipe_with_prompt(&base_recipe(), prompt);
        let value: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("valid yaml");
        assert_eq!(value["prompt"].as_str(), Some(prompt), "{yaml}");
    }

    #[test]
    fn recipe_with_prompt_preserves_the_title_verbatim() {
        // `is_coding_harness_recipe` gates repo-map injection on an exact
        // title match (crates/goose-cli/src/recipes/builtin_recipes.rs:31) —
        // this must never be touched.
        let yaml = recipe_with_prompt(&base_recipe(), "do the task");
        assert!(
            yaml.lines().any(|l| l == "title: Permagent Coding Harness"),
            "{yaml}"
        );
        let value: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("valid yaml");
        assert_eq!(
            value["title"].as_str(),
            Some("Permagent Coding Harness"),
            "{yaml}"
        );
    }
}
