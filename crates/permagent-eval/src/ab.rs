//! Paired A/B over the fixture suite — "did this harness change help?"
//!
//! [`paired`](crate::paired) answers the statistics question (delta + the
//! 30-run honesty guard) but knows nothing about how to *produce* two arms.
//! This module is the missing half: it defines what a **configuration** is,
//! turns two sets of [`TaskResult`]s into a serialisable, renderable
//! [`AbReport`], and refuses to let a thin sample be read as a verdict.
//!
//! ## What "configuration A/B" means here
//!
//! The eval already spawns the real `permagent` binary once per (task, tier).
//! The only thing that can differ between two such spawns — without rebuilding
//! the harness — is the **environment** the child sees. So an arm is exactly
//! that: a label plus a set of environment variables to set (and names to
//! remove). [`KNOBS`] is the curated registry of the ones that actually change
//! harness behaviour today; `--a-env`/`--b-env` is the escape hatch for
//! anything not yet in it.
//!
//! ## The honesty rule, restated at the report level
//!
//! [`paired::compare`](crate::paired::compare) already flags a small sample.
//! This module goes one step further and makes the *verdict itself*
//! unavailable below the threshold: [`Verdict::Inconclusive`] is what a thin
//! run produces, and [`SampleRequirement`] says in concrete terms — repeats,
//! fixtures, runs — what it would take to earn a real one. A delta printed
//! without that context is how a coin flip becomes a design decision.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::metrics::{aggregate, TaskResult};
use crate::oracle::OracleOutcome;
use crate::paired::{compare, PairedReport, MIN_RUNS_PER_ARM_FOR_CONFIDENCE};
use crate::report::{fmt_pct, fmt_usd};
use crate::tier::Tier;

/// The stable identifier written into every result file, so a consumer (CI, a
/// future UI, `permagent-eval report`) can tell what it is reading.
pub const AB_SCHEMA: &str = "permagent-eval/ab-result@1";

// ---------------------------------------------------------------------------
// Arms and the knob registry
// ---------------------------------------------------------------------------

/// One side of the comparison: a human label plus the environment that defines
/// it. Applied on top of the invocation's own environment, so it survives the
/// runner's ambient-env scrub (see [`crate::invocation`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArmConfig {
    pub label: String,
    /// Variables to SET on the harness child.
    pub env: Vec<(String, String)>,
    /// Variable NAMES to REMOVE from the harness child's inherited environment.
    pub unset: Vec<String>,
}

impl ArmConfig {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            env: Vec::new(),
            unset: Vec::new(),
        }
    }

    /// Add a `KEY=VALUE` pair as given on the command line. An empty `VALUE`
    /// (`KEY=`) sets the variable to the empty string — to *remove* a variable
    /// use [`ArmConfig::unset`] instead, since "set to empty" and "absent" are
    /// different to most `env::var` call sites.
    pub fn with_env_kv(mut self, kv: &str) -> anyhow::Result<Self> {
        let (k, v) = kv.split_once('=').ok_or_else(|| {
            anyhow::anyhow!("env override {kv:?} must be KEY=VALUE (an `=` is required)")
        })?;
        if k.trim().is_empty() {
            anyhow::bail!("env override {kv:?} has an empty variable name");
        }
        self.env.push((k.to_string(), v.to_string()));
        Ok(self)
    }

    /// Whether this arm changes anything at all about the child environment.
    pub fn is_empty(&self) -> bool {
        self.env.is_empty() && self.unset.is_empty()
    }
}

/// A harness behaviour that can be flipped from the environment, and therefore
/// A/B'd by this command without rebuilding anything.
///
/// `off`/`on` are the environment each arm gets. A knob whose "off" side is the
/// shipped default still sets it explicitly: relying on the ambient default
/// would make the control arm depend on the operator's shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Knob {
    /// CLI name (`--knob <name>`).
    pub name: &'static str,
    /// One line for `--help` and the report.
    pub summary: &'static str,
    /// What the harness does by default, stated so a reader knows which arm is
    /// the status quo.
    pub shipped_default: &'static str,
    /// Environment for the control (A) arm.
    pub off: &'static [(&'static str, &'static str)],
    /// Environment for the treatment (B) arm.
    pub on: &'static [(&'static str, &'static str)],
}

impl Knob {
    pub fn control_arm(&self) -> ArmConfig {
        ArmConfig {
            label: format!("{} off", self.name),
            env: pairs(self.off),
            unset: Vec::new(),
        }
    }

    pub fn treatment_arm(&self) -> ArmConfig {
        ArmConfig {
            label: format!("{} on", self.name),
            env: pairs(self.on),
            unset: Vec::new(),
        }
    }
}

fn pairs(kv: &[(&str, &str)]) -> Vec<(String, String)> {
    kv.iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

/// The knobs that are genuinely A/B-able through the environment today.
///
/// Membership is earned, not assumed. Every entry was traced to a live read on
/// the `permagent run` path; three families that *look* like knobs were
/// deliberately left out:
///
/// - **best-of-N** (`PERMAGENT_BEST_OF_N_ENABLED`, `PERMAGENT_BEST_OF_N`,
///   `crates/goose/src/cost_router/best_of_n.rs:76-81,382-392`) — the flag is
///   read, but `run_best_of_n_sampling`
///   (`agents/platform_extensions/best_of_n_adapter.rs:247`) has no caller
///   outside its own tests. Setting it changes nothing measurable, and an A/B
///   that silently compares a configuration with itself is worse than no A/B.
/// - **`MAX_HOLDS`, `CODE_MAP_INJECT_MAX_CHARS`, `OFFLOAD_TOKEN_THRESHOLD`** —
///   hard `const`s. Changing them needs a rebuild, so they are out of scope
///   for an environment-only comparison.
/// - **numeric knobs with no natural on/off** (`GOOSE_AUTO_COMPACT_THRESHOLD`,
///   `GOOSE_TOOL_CALL_CUTOFF`, `GOOSE_MAX_TURNS`, `GOOSE_CONTEXT_LIMIT`) — real
///   levers, but a two-arm registry entry would have to invent the two values.
///   Use `--a-env`/`--b-env` and state the values in the run.
///
/// **Boolean spelling matters.** Keys read through `Config::get_param::<bool>`
/// (`crates/goose/src/config/base.rs:910-922`) parse the env value as JSON
/// first, so `1` deserialises as a *number*, fails the `bool` conversion, and
/// falls back to the default — an arm that looks set and is silently the
/// control. Those knobs spell it `true`/`false`. Knobs read with a raw
/// `std::env::var` parser accept `0`/`1`, and are marked as such below.
pub const KNOBS: &[Knob] = &[
    Knob {
        name: "repo-map",
        summary: "inject a repo map into the coding-harness system prompt \
                  (PERMAGENT_CODING_REPO_MAP_TOKENS; 0 disables mapping entirely)",
        shipped_default: "on, 4096 tokens (crates/goose-cli/src/session/builder.rs:666,709-711)",
        off: &[("PERMAGENT_CODING_REPO_MAP_TOKENS", "0")],
        on: &[("PERMAGENT_CODING_REPO_MAP_TOKENS", "4096")],
    },
    Knob {
        name: "premature-done-guard",
        summary: "hold a turn that claims done without having run a verification tool \
                  (PERMAGENT_AFTER_TURN_HOLD; raw env parser, so 0/1 are fine)",
        shipped_default: "on (crates/goose/src/after_turn.rs:153-159)",
        off: &[("PERMAGENT_AFTER_TURN_HOLD", "0")],
        on: &[("PERMAGENT_AFTER_TURN_HOLD", "1")],
    },
    Knob {
        name: "reviewer-mandate",
        summary: "require a cross-family reviewer delegate before a file-changing turn ends \
                  (REVIEWER_MANDATE; get_param bool, so true/false only)",
        shipped_default: "on (crates/goose/src/after_turn.rs:380-384)",
        off: &[("REVIEWER_MANDATE", "false")],
        on: &[("REVIEWER_MANDATE", "true")],
    },
    Knob {
        name: "reviewer-trivial-skip",
        summary: "skip the mandatory independent reviewer for docs-only, test-only, \
                  formatting-only, or ≤3-line diffs (the recipe's own trivial-diff rule), \
                  instead of always delegating a cross-family review \
                  (PERMAGENT_REVIEWER_MANDATE_SKIP_TRIVIAL; raw env parser, so 0/1 are fine)",
        shipped_default: "on (crates/goose/src/after_turn.rs:615,617-625)",
        off: &[("PERMAGENT_REVIEWER_MANDATE_SKIP_TRIVIAL", "0")],
        on: &[("PERMAGENT_REVIEWER_MANDATE_SKIP_TRIVIAL", "1")],
    },
    Knob {
        name: "tool-pair-summarization",
        summary: "summarise tool request/response pairs during compaction instead of dropping \
                  them (GOOSE_TOOL_PAIR_SUMMARIZATION; get_param bool)",
        shipped_default: "on (crates/goose/src/context_mgmt/mod.rs:25-28)",
        off: &[("GOOSE_TOOL_PAIR_SUMMARIZATION", "false")],
        on: &[("GOOSE_TOOL_PAIR_SUMMARIZATION", "true")],
    },
    Knob {
        name: "lessons",
        summary: "inject distilled lessons into the prompt (PERMAGENT_LESSONS_ENABLED; raw env \
                  parser). This is the knob paired.rs was written for — it has twice measured \
                  a LOSS",
        shipped_default: "off (crates/goose/src/lessons/mod.rs:139-143)",
        off: &[("PERMAGENT_LESSONS_ENABLED", "false")],
        on: &[("PERMAGENT_LESSONS_ENABLED", "true")],
    },
    Knob {
        name: "librarian-atoms",
        summary: "layered/provenance memory recall for search_memory (LIBRARIAN_ATOMS_ENABLED; \
                  get_param bool). Measured −9.2pp once already",
        shipped_default: "off (crates/goose/src/agents/platform_extensions/ext_manager.rs:644-649)",
        off: &[("LIBRARIAN_ATOMS_ENABLED", "false")],
        on: &[("LIBRARIAN_ATOMS_ENABLED", "true")],
    },
    Knob {
        name: "acr",
        summary: "associative-recall spreading over Brain memory (PERMAGENT_ACR_MODE: \
                  off | precision | completeness)",
        shipped_default: "off (crates/goose/src/brain_handle.rs:1827-1890)",
        off: &[("PERMAGENT_ACR_MODE", "off")],
        on: &[("PERMAGENT_ACR_MODE", "precision")],
    },
];

/// Look up a knob by its CLI name.
pub fn knob(name: &str) -> Option<&'static Knob> {
    KNOBS.iter().find(|k| k.name == name)
}

/// The knob names, for `--help` and error messages.
pub fn knob_names() -> Vec<&'static str> {
    KNOBS.iter().map(|k| k.name).collect()
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

/// What the run is allowed to claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The sample is under the honesty threshold. No directional claim is
    /// available — this is a refusal, not a "no effect" finding.
    Inconclusive,
    /// Enough runs, and the treatment solved more.
    Improved,
    /// Enough runs, and the treatment solved fewer.
    Regressed,
    /// Enough runs, and the two arms tied.
    NoChange,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Inconclusive => "INCONCLUSIVE (sample below the honesty threshold)",
            Verdict::Improved => "improved",
            Verdict::Regressed => "regressed",
            Verdict::NoChange => "no change",
        }
    }

    /// Whether a directional claim was earned. `false` for
    /// [`Verdict::Inconclusive`].
    pub fn is_conclusive(self) -> bool {
        !matches!(self, Verdict::Inconclusive)
    }
}

/// Concrete terms for what a conclusive run would take, so "not enough data"
/// is actionable rather than a shrug.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleRequirement {
    /// [`MIN_RUNS_PER_ARM_FOR_CONFIDENCE`].
    pub runs_per_arm_needed: usize,
    /// Runs actually collected in the *smaller* arm — the one that binds.
    pub runs_per_arm_have: usize,
    /// Fixtures in the suite.
    pub fixtures: usize,
    /// `--repeats` the run used.
    pub repeats_used: usize,
    /// The smallest `--repeats` that would clear the threshold on this suite
    /// (`ceil(needed / fixtures)`); 0 when the suite is empty.
    pub repeats_needed: usize,
}

impl SampleRequirement {
    pub fn compute(fixtures: usize, repeats_used: usize, runs_have: usize) -> Self {
        let needed = MIN_RUNS_PER_ARM_FOR_CONFIDENCE;
        let repeats_needed = if fixtures == 0 {
            0
        } else {
            needed.div_ceil(fixtures)
        };
        SampleRequirement {
            runs_per_arm_needed: needed,
            runs_per_arm_have: runs_have,
            fixtures,
            repeats_used,
            repeats_needed,
        }
    }

    pub fn is_met(&self) -> bool {
        self.runs_per_arm_have >= self.runs_per_arm_needed
    }

    /// The sentence printed when the guard refuses.
    pub fn what_it_would_take(&self) -> String {
        if self.fixtures == 0 {
            return format!(
                "no fixtures were run, so no sample exists (need {} runs per arm)",
                self.runs_per_arm_needed
            );
        }
        format!(
            "have {} run(s) in the smaller arm, need {}; with {} fixture(s) that is \
             --repeats {} (you ran --repeats {}), i.e. {} runs per arm and {} harness \
             invocations in total across both arms",
            self.runs_per_arm_have,
            self.runs_per_arm_needed,
            self.fixtures,
            self.repeats_needed,
            self.repeats_used,
            self.repeats_needed * self.fixtures,
            2 * self.repeats_needed * self.fixtures,
        )
    }
}

/// The model tier both arms ran under. Held constant by construction: the A/B
/// command resolves exactly one tier, because a delta across two models
/// measures the models, not the harness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TierIdentity {
    pub name: String,
    pub provider: String,
    pub model: String,
    pub pinned_packs: bool,
}

impl From<&Tier> for TierIdentity {
    fn from(t: &Tier) -> Self {
        TierIdentity {
            name: t.name.clone(),
            provider: t.provider.clone(),
            model: t.model.clone(),
            pinned_packs: t.pin_packs,
        }
    }
}

/// One arm's measured outcome. Costs and tokens come from that arm's own
/// isolated ledgers, summed — never estimated here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArmReport {
    pub label: String,
    /// The environment that defined this arm (sorted, so the file is stable).
    pub env: BTreeMap<String, String>,
    pub unset: Vec<String>,
    /// Harness invocations that produced a graded verdict.
    pub runs: usize,
    pub solved: usize,
    pub pass_rate: f64,
    /// Runs skipped by a `--budget-usd` stop (excluded from `runs`).
    pub not_run: usize,
    /// Runs whose oracle could not produce a verdict.
    pub errored: usize,
    pub total_cost_usd: Option<f64>,
    pub any_cost_unknown: bool,
    pub any_estimated: bool,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub total_tool_calls: usize,
    pub median_duration_secs: Option<f64>,
}

impl ArmReport {
    pub fn build(cfg: &ArmConfig, results: &[TaskResult]) -> Self {
        let agg = aggregate(results);
        let t = token_totals(results);
        ArmReport {
            label: cfg.label.clone(),
            env: cfg.env.iter().cloned().collect(),
            unset: cfg.unset.clone(),
            runs: agg.attempted,
            solved: agg.solved,
            pass_rate: agg.pass_rate,
            not_run: agg.not_run,
            errored: results
                .iter()
                .filter(|r| r.oracle == OracleOutcome::Errored)
                .count(),
            total_cost_usd: agg.total_cost_usd,
            any_cost_unknown: agg.any_cost_unknown,
            any_estimated: agg.any_estimated,
            input_tokens: t.input,
            output_tokens: t.output,
            cache_read_tokens: t.cache_read,
            cache_write_tokens: t.cache_write,
            total_tool_calls: agg.total_tool_calls,
            median_duration_secs: agg.median_duration_secs,
        }
    }
}

/// Summed token counts, `None` for a field no run reported.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenTotals {
    pub input: Option<i64>,
    pub output: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
}

/// Sum each token field across `results`, keeping `None` when nothing was
/// known — a zero would claim "measured zero tokens", which is a different and
/// much more flattering statement than "not measured".
pub fn token_totals(results: &[TaskResult]) -> TokenTotals {
    fn sum(results: &[TaskResult], pick: impl Fn(&TaskResult) -> Option<i64>) -> Option<i64> {
        let mut total = 0i64;
        let mut any = false;
        for r in results {
            if let Some(v) = pick(r) {
                total = total.saturating_add(v);
                any = true;
            }
        }
        any.then_some(total)
    }
    TokenTotals {
        input: sum(results, |r| r.cost.input_tokens),
        output: sum(results, |r| r.cost.output_tokens),
        cache_read: sum(results, |r| r.cost.cache_read_tokens),
        cache_write: sum(results, |r| r.cost.cache_write_tokens),
    }
}

/// One fixture's contribution to the delta. Per-fixture counts are what turn
/// "+12 points" into something a human can check — a single fixture flipping
/// on a small suite is visible here and invisible in the aggregate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixtureDelta {
    pub task_id: String,
    pub control_runs: usize,
    pub control_solved: usize,
    pub treatment_runs: usize,
    pub treatment_solved: usize,
    /// treatment pass-rate − control pass-rate, on this fixture alone.
    pub delta: f64,
}

/// Per-fixture pass deltas, in first-seen order (control first, then any
/// fixture only the treatment ran). Not-run results are excluded from both
/// numerator and denominator, matching [`crate::metrics::pass_rate`].
pub fn per_fixture(control: &[TaskResult], treatment: &[TaskResult]) -> Vec<FixtureDelta> {
    let mut order: Vec<String> = Vec::new();
    for r in control.iter().chain(treatment.iter()) {
        if !order.iter().any(|id| id == &r.task_id) {
            order.push(r.task_id.clone());
        }
    }
    order
        .into_iter()
        .map(|id| {
            let (c_runs, c_solved) = tally(control, &id);
            let (t_runs, t_solved) = tally(treatment, &id);
            let rate = |solved: usize, runs: usize| {
                if runs == 0 {
                    0.0
                } else {
                    solved as f64 / runs as f64
                }
            };
            FixtureDelta {
                task_id: id,
                control_runs: c_runs,
                control_solved: c_solved,
                treatment_runs: t_runs,
                treatment_solved: t_solved,
                delta: rate(t_solved, t_runs) - rate(c_solved, c_runs),
            }
        })
        .collect()
}

fn tally(results: &[TaskResult], task_id: &str) -> (usize, usize) {
    let mut runs = 0;
    let mut solved = 0;
    for r in results
        .iter()
        .filter(|r| r.task_id == task_id && r.oracle != OracleOutcome::NotRun)
    {
        runs += 1;
        if r.solved {
            solved += 1;
        }
    }
    (runs, solved)
}

/// The whole comparison, ready to serialise. Deliberately free of
/// [`TaskResult`] so a saved file round-trips through serde and can be
/// re-rendered by `permagent-eval report` without the crate that produced it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AbReport {
    pub schema: String,
    pub tier: TierIdentity,
    /// The registry knob under test, when one was named.
    pub knob: Option<String>,
    /// A one-line description of what the knob changes (registry knobs only).
    pub knob_summary: Option<String>,
    pub repeats: usize,
    pub fixtures: usize,
    pub control: ArmReport,
    pub treatment: ArmReport,
    /// treatment − control, as a fraction (multiply by 100 for points).
    pub delta_pass_rate: f64,
    pub verdict: Verdict,
    /// [`paired::compare`](crate::paired::compare)'s warning, verbatim.
    pub sample_warning: Option<String>,
    pub requirement: SampleRequirement,
    pub per_fixture: Vec<FixtureDelta>,
}

impl AbReport {
    /// Build the report from both arms' raw results. `fixtures` is the number
    /// of distinct tasks in the suite and `repeats` the `--repeats` used;
    /// together they are what [`SampleRequirement`] turns into advice.
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        tier: &Tier,
        knob: Option<&Knob>,
        repeats: usize,
        fixtures: usize,
        control_cfg: &ArmConfig,
        control: &[TaskResult],
        treatment_cfg: &ArmConfig,
        treatment: &[TaskResult],
    ) -> Self {
        let paired: PairedReport = compare(control, treatment);
        let control_arm = ArmReport::build(control_cfg, control);
        let treatment_arm = ArmReport::build(treatment_cfg, treatment);

        // The guard binds on the SMALLER arm, exactly as `paired::compare`
        // does — a large control never licenses a thin treatment.
        //
        // It also binds on ATTEMPTED runs, which `paired::compare` cannot see:
        // that function counts `results.len()`, so a `--budget-usd` stop that
        // filled an arm with never-launched placeholders would look to it like
        // a full sample. Requiring BOTH guards means the stricter one wins and
        // a budget-truncated comparison can never clear the threshold on the
        // strength of runs that did not happen.
        let runs_have = control_arm.runs.min(treatment_arm.runs);
        let requirement = SampleRequirement::compute(fixtures, repeats, runs_have);
        let delta = treatment_arm.pass_rate - control_arm.pass_rate;
        let verdict = if paired.sample_warning.is_some() || !requirement.is_met() {
            Verdict::Inconclusive
        } else if delta > f64::EPSILON {
            Verdict::Improved
        } else if delta < -f64::EPSILON {
            Verdict::Regressed
        } else {
            Verdict::NoChange
        };

        AbReport {
            schema: AB_SCHEMA.to_string(),
            tier: TierIdentity::from(tier),
            knob: knob.map(|k| k.name.to_string()),
            knob_summary: knob.map(|k| k.summary.to_string()),
            repeats,
            fixtures,
            control: control_arm,
            treatment: treatment_arm,
            delta_pass_rate: delta,
            verdict,
            sample_warning: paired.sample_warning,
            requirement,
            per_fixture: per_fixture(control, treatment),
        }
    }
}

/// Render a saved report as a compact Markdown table. Pure and
/// clock-independent, so `report <file>` reproduces byte-identical output.
pub fn render_markdown(r: &AbReport) -> String {
    let mut out = String::new();
    out.push_str("# Paired A/B — Permagent coding harness\n\n");
    out.push_str(&format!(
        "- tier `{}` (`{}` / `{}`, packs {})\n",
        r.tier.name,
        r.tier.provider,
        r.tier.model,
        if r.tier.pinned_packs {
            "pinned"
        } else {
            "native routing"
        }
    ));
    match (&r.knob, &r.knob_summary) {
        (Some(k), Some(s)) => out.push_str(&format!("- knob `{k}` — {s}\n")),
        (Some(k), None) => out.push_str(&format!("- knob `{k}`\n")),
        _ => out.push_str("- knob: ad-hoc (`--a-env` / `--b-env`)\n"),
    }
    out.push_str(&format!(
        "- {} fixture(s) × {} repeat(s) per arm\n\n",
        r.fixtures, r.repeats
    ));

    // Raw counts share the cell with the percentage, never a bare rate: a
    // percentage on a small n is how a thin result gets read as a green light.
    out.push_str("| arm | solved | pass rate | measured $ | in tok | out tok | cache-read tok | tool calls | median s |\n");
    out.push_str("|---|---:|---:|---:|---:|---:|---:|---:|---:|\n");
    for (tag, a) in [("A", &r.control), ("B", &r.treatment)] {
        out.push_str(&format!(
            "| {tag}: {} | {}/{} | {} | {}{} | {} | {} | {} | {} | {} |\n",
            a.label,
            a.solved,
            a.runs,
            fmt_pct(a.pass_rate),
            fmt_usd(a.total_cost_usd),
            if a.any_cost_unknown || a.any_estimated {
                "*"
            } else {
                ""
            },
            fmt_tokens(a.input_tokens),
            fmt_tokens(a.output_tokens),
            fmt_tokens(a.cache_read_tokens),
            a.total_tool_calls,
            match a.median_duration_secs {
                Some(s) => format!("{s:.1}"),
                None => "n/a".to_string(),
            },
        ));
    }
    if r.control.any_cost_unknown
        || r.treatment.any_cost_unknown
        || r.control.any_estimated
        || r.treatment.any_estimated
    {
        out.push_str(
            "\n`*` at least one run's cost was unknown or estimated — totals are a lower bound.\n",
        );
    }

    out.push_str(&format!(
        "\n**delta {:+.1} points** (B − A) · verdict: **{}**\n",
        r.delta_pass_rate * 100.0,
        r.verdict.label()
    ));
    if !r.verdict.is_conclusive() {
        out.push_str(&format!(
            "\n> The delta above is NOT evidence. {}\n",
            r.requirement.what_it_would_take()
        ));
        if let Some(w) = &r.sample_warning {
            out.push_str(&format!(">\n> {w}\n"));
        }
    }

    out.push_str("\n## Per-fixture pass deltas\n\n");
    out.push_str("| fixture | A solved | B solved | delta |\n|---|---:|---:|---:|\n");
    for f in &r.per_fixture {
        out.push_str(&format!(
            "| `{}` | {}/{} | {}/{} | {:+.1} |\n",
            f.task_id,
            f.control_solved,
            f.control_runs,
            f.treatment_solved,
            f.treatment_runs,
            f.delta * 100.0
        ));
    }
    out
}

fn fmt_tokens(v: Option<i64>) -> String {
    match v {
        Some(n) => n.to_string(),
        None => "n/a".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cost::CostReading;

    fn result(task: &str, solved: bool) -> TaskResult {
        TaskResult::new(
            task,
            "regression",
            if solved {
                OracleOutcome::Pass
            } else {
                OracleOutcome::Fail
            },
            CostReading::known_with_tokens(0.01, false, 1, Some(100), Some(20), Some(10), Some(5)),
        )
    }

    fn arm(label: &str) -> ArmConfig {
        ArmConfig::new(label)
    }

    /// `n` fixtures × `repeats`, the first `solved_per_fixture` runs of each
    /// fixture solved.
    fn suite(fixtures: usize, repeats: usize, solved_per_fixture: usize) -> Vec<TaskResult> {
        let mut out = Vec::new();
        for f in 0..fixtures {
            for r in 0..repeats {
                out.push(result(&format!("fix{f}"), r < solved_per_fixture));
            }
        }
        out
    }

    #[test]
    fn a_thin_sample_yields_no_verdict_and_says_what_it_would_take() {
        let tier = Tier::builtin("local").unwrap();
        let c = suite(5, 1, 0);
        let t = suite(5, 1, 1); // treatment solves everything: +100 points
        let r = AbReport::build(&tier, None, 1, 5, &arm("a"), &c, &arm("b"), &t);

        assert_eq!(r.verdict, Verdict::Inconclusive, "5 runs must not conclude");
        assert!(!r.verdict.is_conclusive());
        assert!(r.sample_warning.is_some());
        // The delta is still reported — hiding it would be its own dishonesty.
        assert!((r.delta_pass_rate - 1.0).abs() < 1e-9);
        // …and the refusal is actionable: 30 runs over 5 fixtures = 6 repeats.
        assert_eq!(r.requirement.repeats_needed, 6);
        let advice = r.requirement.what_it_would_take();
        assert!(advice.contains("--repeats 6"), "{advice}");
        assert!(advice.contains("need 30"), "{advice}");
    }

    /// The report must be a *wiring* of `paired::compare`, not a second
    /// implementation of it — two implementations of the honesty rule is how
    /// one of them quietly stops firing.
    #[test]
    fn the_report_carries_paired_compares_own_warning_and_delta_verbatim() {
        let tier = Tier::builtin("local").unwrap();
        let c = suite(4, 1, 1);
        let t = suite(4, 1, 3);
        let direct = crate::paired::compare(&c, &t);
        let r = AbReport::build(&tier, None, 1, 4, &arm("a"), &c, &arm("b"), &t);
        assert_eq!(r.sample_warning, direct.sample_warning);
        assert!((r.delta_pass_rate - direct.delta).abs() < 1e-12);
        assert_eq!(r.control.solved, direct.control.solved);
        assert_eq!(r.treatment.runs, direct.treatment.runs);
    }

    #[test]
    fn a_sufficient_sample_earns_a_directional_verdict() {
        let tier = Tier::builtin("local").unwrap();
        // 17 fixtures × 2 repeats = 34 runs per arm, over the 30 threshold.
        let c = suite(17, 2, 1); // 50%
        let t = suite(17, 2, 2); // 100%
        let r = AbReport::build(&tier, None, 2, 17, &arm("a"), &c, &arm("b"), &t);
        assert_eq!(r.verdict, Verdict::Improved);
        assert!(r.sample_warning.is_none());
        assert!((r.delta_pass_rate - 0.5).abs() < 1e-9);
        assert!(r.requirement.is_met());
        assert_eq!(r.requirement.repeats_needed, 2);
    }

    #[test]
    fn a_regression_is_named_a_regression() {
        let tier = Tier::builtin("local").unwrap();
        let c = suite(17, 2, 2);
        let t = suite(17, 2, 1);
        let r = AbReport::build(&tier, None, 2, 17, &arm("a"), &c, &arm("b"), &t);
        assert_eq!(r.verdict, Verdict::Regressed);
        assert!(r.delta_pass_rate < 0.0);
    }

    #[test]
    fn a_tie_on_a_big_enough_sample_is_no_change_not_inconclusive() {
        let tier = Tier::builtin("local").unwrap();
        let c = suite(17, 2, 1);
        let t = suite(17, 2, 1);
        let r = AbReport::build(&tier, None, 2, 17, &arm("a"), &c, &arm("b"), &t);
        assert_eq!(r.verdict, Verdict::NoChange);
    }

    /// A fat control arm must not license a thin treatment arm.
    #[test]
    fn the_smaller_arm_binds_the_guard() {
        let tier = Tier::builtin("local").unwrap();
        let c = suite(17, 4, 2);
        let t = suite(2, 1, 1);
        let r = AbReport::build(&tier, None, 4, 17, &arm("a"), &c, &arm("b"), &t);
        assert_eq!(r.verdict, Verdict::Inconclusive);
        assert_eq!(r.requirement.runs_per_arm_have, 2);
    }

    #[test]
    fn not_run_results_are_excluded_from_runs_and_from_fixture_tallies() {
        let tier = Tier::builtin("local").unwrap();
        let mut c = suite(2, 1, 1);
        c.push(TaskResult::new(
            "fix9",
            "regression",
            OracleOutcome::NotRun,
            CostReading::unknown(),
        ));
        let t = suite(2, 1, 1);
        let r = AbReport::build(&tier, None, 1, 2, &arm("a"), &c, &arm("b"), &t);
        assert_eq!(r.control.runs, 2, "a budget-skipped run is not a run");
        assert_eq!(r.control.not_run, 1);
        let skipped = r
            .per_fixture
            .iter()
            .find(|f| f.task_id == "fix9")
            .expect("the skipped fixture is still listed");
        assert_eq!(skipped.control_runs, 0);
    }

    /// A budget stop must not be able to buy a verdict. `paired::compare`
    /// counts every result, including never-launched ones, so on its own it
    /// would clear the 30-run bar here; the attempted-runs guard must veto.
    #[test]
    fn a_budget_truncated_arm_cannot_clear_the_threshold_with_runs_that_never_happened() {
        let tier = Tier::builtin("local").unwrap();
        let control = suite(35, 1, 1); // 35 real runs, all solved
        let mut treatment = suite(20, 1, 0); // 20 real runs…
        for i in 0..15 {
            // …plus 15 that a budget stop skipped before they could launch.
            treatment.push(TaskResult::new(
                format!("skipped{i}"),
                "regression",
                OracleOutcome::NotRun,
                CostReading::unknown(),
            ));
        }
        // paired::compare, counting raw results, sees 35 vs 35 and does not warn.
        assert!(crate::paired::compare(&control, &treatment)
            .sample_warning
            .is_none());
        // The report must refuse anyway.
        let r = AbReport::build(
            &tier,
            None,
            1,
            35,
            &arm("a"),
            &control,
            &arm("b"),
            &treatment,
        );
        assert_eq!(r.verdict, Verdict::Inconclusive);
        assert_eq!(r.requirement.runs_per_arm_have, 20);
        assert_eq!(r.treatment.not_run, 15);
    }

    #[test]
    fn per_fixture_deltas_localise_which_fixture_moved() {
        let c = vec![result("a", true), result("b", false)];
        let t = vec![result("a", true), result("b", true)];
        let d = per_fixture(&c, &t);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].task_id, "a");
        assert!((d[0].delta).abs() < 1e-9, "fixture a did not move");
        assert!((d[1].delta - 1.0).abs() < 1e-9, "fixture b flipped to pass");
    }

    #[test]
    fn token_totals_are_summed_and_unknown_stays_unknown() {
        let t = token_totals(&[result("a", true), result("b", false)]);
        assert_eq!(t.input, Some(200));
        assert_eq!(t.output, Some(40));
        let none = token_totals(&[TaskResult::new(
            "a",
            "c",
            OracleOutcome::Pass,
            CostReading::unknown(),
        )]);
        assert_eq!(none.input, None, "unmeasured must not read as zero");
    }

    #[test]
    fn a_report_round_trips_through_json() {
        let tier = Tier::builtin("haiku").unwrap();
        let c = suite(3, 2, 1);
        let t = suite(3, 2, 2);
        let r = AbReport::build(&tier, None, 2, 3, &arm("a"), &c, &arm("b"), &t);
        let text = serde_json::to_string_pretty(&r).unwrap();
        let back: AbReport = serde_json::from_str(&text).unwrap();
        assert_eq!(back, r);
        assert_eq!(back.schema, AB_SCHEMA);
        // …and re-rendering the parsed copy reproduces the same Markdown.
        assert_eq!(render_markdown(&back), render_markdown(&r));
    }

    #[test]
    fn markdown_shows_counts_costs_and_the_refusal() {
        let tier = Tier::builtin("local").unwrap();
        let c = suite(3, 1, 0);
        let t = suite(3, 1, 1);
        let md = render_markdown(&AbReport::build(
            &tier,
            None,
            1,
            3,
            &arm("off"),
            &c,
            &arm("on"),
            &t,
        ));
        assert!(md.contains("0/3"), "raw counts must be visible:\n{md}");
        assert!(md.contains("3/3"));
        assert!(md.contains("$0.0300"), "measured spend per arm:\n{md}");
        assert!(md.contains("INCONCLUSIVE"), "{md}");
        assert!(md.contains("NOT evidence"), "{md}");
        assert!(md.contains("--repeats 10"), "{md}");
    }

    #[test]
    fn env_kv_parsing_rejects_a_missing_equals_and_an_empty_name() {
        assert!(ArmConfig::new("x").with_env_kv("FOO").is_err());
        assert!(ArmConfig::new("x").with_env_kv("=bar").is_err());
        let a = ArmConfig::new("x").with_env_kv("FOO=bar").unwrap();
        assert_eq!(a.env, vec![("FOO".to_string(), "bar".to_string())]);
        // An empty value is legal and means "set to empty", not "unset".
        let b = ArmConfig::new("x").with_env_kv("FOO=").unwrap();
        assert_eq!(b.env[0].1, "");
    }

    #[test]
    fn every_registry_knob_has_distinct_arms_and_a_documented_default() {
        for k in KNOBS {
            assert!(!k.summary.is_empty(), "{} needs a summary", k.name);
            assert!(
                !k.shipped_default.is_empty(),
                "{} must state the shipped default",
                k.name
            );
            assert!(!k.on.is_empty(), "{} must set something when on", k.name);
            assert_ne!(
                k.off, k.on,
                "{} would compare a configuration against itself",
                k.name
            );
            // Both arms must pin the SAME variables, or the delta measures a
            // variable that is set on one side and merely ambient on the other.
            let off_keys: Vec<&str> = k.off.iter().map(|(a, _)| *a).collect();
            let on_keys: Vec<&str> = k.on.iter().map(|(a, _)| *a).collect();
            assert_eq!(off_keys, on_keys, "{} pins different vars per arm", k.name);
        }
    }

    #[test]
    fn knob_lookup_is_by_exact_name() {
        for name in knob_names() {
            assert_eq!(knob(name).unwrap().name, name);
        }
        assert!(knob("no-such-knob").is_none());
    }

    /// `reviewer-trivial-skip` resolves by name and its arms produce exactly
    /// the env pairs the raw `std::env::var` parser in
    /// `crates/goose/src/after_turn.rs` (harness-loop worktree, lines
    /// 615,617-625) accepts: `0` for off, `1` for on.
    #[test]
    fn reviewer_trivial_skip_resolves_and_produces_the_expected_env_pairs() {
        let k = knob("reviewer-trivial-skip").expect("registered in KNOBS");
        assert_eq!(k.name, "reviewer-trivial-skip");

        let off = k.control_arm();
        assert_eq!(off.label, "reviewer-trivial-skip off");
        assert_eq!(
            off.env,
            vec![(
                "PERMAGENT_REVIEWER_MANDATE_SKIP_TRIVIAL".to_string(),
                "0".to_string()
            )]
        );
        assert!(off.unset.is_empty());

        let on = k.treatment_arm();
        assert_eq!(on.label, "reviewer-trivial-skip on");
        assert_eq!(
            on.env,
            vec![(
                "PERMAGENT_REVIEWER_MANDATE_SKIP_TRIVIAL".to_string(),
                "1".to_string()
            )]
        );
        assert!(on.unset.is_empty());
    }
}
