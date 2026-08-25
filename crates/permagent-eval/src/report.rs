//! Deterministic rendering of eval results as text, Markdown, or JSON.
//!
//! Rendering is pure (no clocks, no environment) so the output is stable and
//! testable. Costs render to four decimal places; `n/a` marks an unknown cost or
//! an undefined ratio (e.g. $/solved with nothing solved).

use crate::metrics::{aggregate, Aggregate, TaskResult};
use crate::oracle::OracleOutcome;
use serde_json::json;

/// One tier's slice of a report: its identity plus the per-task results.
#[derive(Debug, Clone)]
pub struct TierReport {
    pub tier: String,
    pub provider: String,
    pub model: String,
    pub pinned_packs: bool,
    pub results: Vec<TaskResult>,
}

impl TierReport {
    pub fn aggregate(&self) -> Aggregate {
        aggregate(&self.results)
    }
}

/// Format an optional dollar amount to 4dp, or `n/a` when unknown.
pub fn fmt_usd(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("${x:.4}"),
        None => "n/a".to_string(),
    }
}

/// Format a `[0,1]` fraction as a percentage to 1dp.
pub fn fmt_pct(fraction: f64) -> String {
    format!("{:.1}%", fraction * 100.0)
}

/// Per-task outcome label. [`OracleOutcome::NotRun`] renders as `SKIP` — a
/// distinct state from `FAIL`, so a budget-stopped task never reads as a
/// graded failure.
fn solved_mark(r: &TaskResult) -> &'static str {
    match r.oracle {
        OracleOutcome::NotRun => "SKIP",
        _ if r.solved => "PASS",
        _ => "FAIL",
    }
}

/// Footnote marker appended to a tier's dollar figures when they carry caveats:
/// `†` when some task costs are unknown (all dollar figures are a lower bound),
/// `‡` when some are estimated (a further under-count). Empty when every cost
/// is known exactly — the markers must never silently vanish from a report
/// whose numbers are not what they look like.
fn cost_caveat_marker(agg: &Aggregate) -> String {
    let mut mark = String::new();
    if agg.any_cost_unknown {
        mark.push('†');
    }
    if agg.any_estimated {
        mark.push('‡');
    }
    mark
}

/// The Markdown footnote explaining `†`.
const MD_UNKNOWN_NOTE: &str = "† some task costs are unknown — dollar figures are a lower bound";

/// The Markdown footnote explaining `‡`.
const MD_ESTIMATED_NOTE: &str = "‡ some task costs are estimated — dollar figures may under-count";

/// A compact one-line-per-tier plaintext summary.
pub fn render_text(reports: &[TierReport]) -> String {
    let mut out = String::new();
    out.push_str("Permagent coding-harness eval\n");
    out.push_str("=============================\n");
    for tr in reports {
        let agg = tr.aggregate();
        out.push_str(&format!(
            "\n[{}]  provider={} model={} packs={}\n",
            tr.tier,
            tr.provider,
            tr.model,
            if tr.pinned_packs { "pinned" } else { "native" }
        ));
        for r in &tr.results {
            out.push_str(&format!(
                "  {:<6} {:<24} cost={:<10} {:.1}s{}\n",
                solved_mark(r),
                r.task_id,
                fmt_usd(r.cost.usd),
                r.duration_secs,
                if r.harness_timed_out {
                    " (timed out)"
                } else {
                    ""
                },
            ));
        }
        out.push_str(&format!(
            "  --> {}/{} solved ({}), $/solved={}, median $/task={}, total={}{}{}\n",
            agg.solved,
            agg.attempted,
            fmt_pct(agg.pass_rate),
            fmt_usd(agg.dollars_per_solved),
            fmt_usd(agg.median_cost_per_task),
            fmt_usd(agg.total_cost_usd),
            if agg.any_cost_unknown {
                " [some costs unknown]"
            } else {
                ""
            },
            if agg.any_estimated {
                " [some costs estimated]"
            } else {
                ""
            },
        ));
        out.push_str(&format!(
            "  --> median wall-clock={}, tool calls={}, cache-hit={}, rate-limit events={}\n",
            match agg.median_duration_secs {
                Some(s) => format!("{s:.1}s"),
                None => "n/a".to_string(),
            },
            agg.total_tool_calls,
            match agg.cache_hit_rate {
                Some(r) => fmt_pct(r),
                None => "n/a".to_string(),
            },
            agg.total_rate_limit_events,
        ));
        if agg.not_run > 0 {
            out.push_str(&format!(
                "  --> {} task(s) skipped (budget stop) — pass-rate excludes them\n",
                agg.not_run
            ));
        }
    }
    out
}

/// A Markdown report: a per-task table and an at-a-glance tier comparison.
pub fn render_markdown(reports: &[TierReport]) -> String {
    let mut out = String::new();
    out.push_str("# Permagent coding-harness eval\n\n");

    for tr in reports {
        let agg = tr.aggregate();
        out.push_str(&format!("## Tier `{}`\n\n", tr.tier));
        out.push_str(&format!(
            "- provider: `{}`  model: `{}`  packs: {}\n",
            tr.provider,
            tr.model,
            if tr.pinned_packs {
                "pinned"
            } else {
                "native routing"
            }
        ));
        let mark = cost_caveat_marker(&agg);
        out.push_str(&format!(
            "- **{}/{} solved** ({}) · **$/solved {}{mark}** · median $/task {}{mark} · total {}{mark}\n",
            agg.solved,
            agg.attempted,
            fmt_pct(agg.pass_rate),
            fmt_usd(agg.dollars_per_solved),
            fmt_usd(agg.median_cost_per_task),
            fmt_usd(agg.total_cost_usd),
        ));
        out.push_str(&format!(
            "- median wall-clock {} · tool calls {} · cache-hit {} · rate-limit events {}\n",
            match agg.median_duration_secs {
                Some(s) => format!("{s:.1}s"),
                None => "n/a".to_string(),
            },
            agg.total_tool_calls,
            match agg.cache_hit_rate {
                Some(r) => fmt_pct(r),
                None => "n/a".to_string(),
            },
            agg.total_rate_limit_events,
        ));
        if agg.any_cost_unknown {
            out.push_str(&format!("- {MD_UNKNOWN_NOTE}\n"));
        }
        if agg.any_estimated {
            out.push_str(&format!("- {MD_ESTIMATED_NOTE}\n"));
        }
        if agg.not_run > 0 {
            out.push_str(&format!(
                "- {} task(s) **skipped** (budget stop) — excluded from pass-rate\n",
                agg.not_run
            ));
        }
        out.push('\n');
        out.push_str("| task | category | result | cost | seconds | tool calls |\n");
        out.push_str("|------|----------|--------|------|---------|------------|\n");
        for r in &tr.results {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {:.1} | {} |\n",
                r.task_id,
                r.category,
                solved_mark(r),
                fmt_usd(r.cost.usd),
                r.duration_secs,
                r.signals.tool_calls,
            ));
        }
        out.push('\n');
    }

    if reports.len() > 1 {
        out.push_str("## Tier comparison\n\n");
        out.push_str(
            "| tier | pass-rate | solved | $/solved | median $/task | total $ | \
             median wall-clock | tool calls | cache-hit | rate-limits |\n",
        );
        out.push_str(
            "|------|-----------|--------|----------|---------------|---------|\
             --------------------|------------|-----------|-------------|\n",
        );
        let mut any_unknown = false;
        let mut any_estimated = false;
        for tr in reports {
            let agg = tr.aggregate();
            let mark = cost_caveat_marker(&agg);
            any_unknown |= agg.any_cost_unknown;
            any_estimated |= agg.any_estimated;
            out.push_str(&format!(
                "| {} | {} | {}/{} | {}{mark} | {}{mark} | {}{mark} | {} | {} | {} | {} |\n",
                tr.tier,
                fmt_pct(agg.pass_rate),
                agg.solved,
                agg.attempted,
                fmt_usd(agg.dollars_per_solved),
                fmt_usd(agg.median_cost_per_task),
                fmt_usd(agg.total_cost_usd),
                match agg.median_duration_secs {
                    Some(s) => format!("{s:.1}s"),
                    None => "n/a".to_string(),
                },
                agg.total_tool_calls,
                match agg.cache_hit_rate {
                    Some(r) => fmt_pct(r),
                    None => "n/a".to_string(),
                },
                agg.total_rate_limit_events,
            ));
        }
        out.push('\n');
        // Footnotes for any marker that appears in the comparison table, so a
        // reader who pastes just this table still sees the caveats.
        if any_unknown {
            out.push_str(&format!("{MD_UNKNOWN_NOTE}\n\n"));
        }
        if any_estimated {
            out.push_str(&format!("{MD_ESTIMATED_NOTE}\n\n"));
        }
    }
    out
}

/// A machine-readable JSON report.
pub fn render_json(reports: &[TierReport]) -> serde_json::Value {
    let tiers: Vec<serde_json::Value> = reports
        .iter()
        .map(|tr| {
            let agg = tr.aggregate();
            let tasks: Vec<serde_json::Value> = tr
                .results
                .iter()
                .map(|r| {
                    json!({
                        "task_id": r.task_id,
                        "category": r.category,
                        "solved": r.solved,
                        "oracle": r.oracle.label(),
                        "cost_usd": r.cost.usd,
                        "cost_estimated": r.cost.estimated,
                        "ledger_rows": r.cost.ledger_rows,
                        "input_tokens": r.cost.input_tokens,
                        "output_tokens": r.cost.output_tokens,
                        "cache_read_tokens": r.cost.cache_read_tokens,
                        "cache_write_tokens": r.cost.cache_write_tokens,
                        "cache_hit_rate": r.cost.cache_hit_rate(),
                        "duration_secs": r.duration_secs,
                        "harness_exit": r.harness_exit,
                        "harness_timed_out": r.harness_timed_out,
                        "note": r.note,
                        "signals": {
                            "tool_calls": r.signals.tool_calls,
                            "tool_names": r.signals.tool_names,
                            "rate_limit_events": r.signals.rate_limit_events,
                            "max_turns_hit": r.signals.max_turns_hit,
                        },
                    })
                })
                .collect();
            json!({
                "tier": tr.tier,
                "provider": tr.provider,
                "model": tr.model,
                "pinned_packs": tr.pinned_packs,
                "summary": {
                    "total": agg.total,
                    "attempted": agg.attempted,
                    "not_run": agg.not_run,
                    "solved": agg.solved,
                    "pass_rate": agg.pass_rate,
                    "dollars_per_solved": agg.dollars_per_solved,
                    "median_cost_per_task": agg.median_cost_per_task,
                    "total_cost_usd": agg.total_cost_usd,
                    "any_cost_unknown": agg.any_cost_unknown,
                    "any_estimated": agg.any_estimated,
                    "median_duration_secs": agg.median_duration_secs,
                    "total_tool_calls": agg.total_tool_calls,
                    "total_rate_limit_events": agg.total_rate_limit_events,
                    "cache_hit_rate": agg.cache_hit_rate,
                },
                "tasks": tasks,
            })
        })
        .collect();
    json!({ "tiers": tiers })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cost::CostReading;
    use crate::oracle::OracleOutcome;

    fn sample() -> TierReport {
        let mk = |id: &str, solved: bool, cost: f64| {
            let o = if solved {
                OracleOutcome::Pass
            } else {
                OracleOutcome::Fail
            };
            let mut r = TaskResult::new(id, "classic", o, CostReading::known(cost, false, 3));
            r.duration_secs = 12.3;
            r
        };
        TierReport {
            tier: "local".to_string(),
            provider: "ollama".to_string(),
            model: "qwen3".to_string(),
            pinned_packs: true,
            results: vec![mk("fizzbuzz", true, 0.0), mk("roman", false, 0.0)],
        }
    }

    #[test]
    fn usd_and_pct_formatting() {
        assert_eq!(fmt_usd(Some(0.04213)), "$0.0421");
        assert_eq!(fmt_usd(None), "n/a");
        assert_eq!(fmt_pct(2.0 / 3.0), "66.7%");
    }

    #[test]
    fn text_report_contains_headline_numbers() {
        let text = render_text(&[sample()]);
        assert!(text.contains("[local]"));
        assert!(text.contains("1/2 solved"));
        assert!(text.contains("50.0%"));
        assert!(text.contains("PASS"));
        assert!(text.contains("FAIL"));
    }

    #[test]
    fn markdown_has_table_and_no_comparison_for_single_tier() {
        let md = render_markdown(&[sample()]);
        assert!(md.contains("## Tier `local`"));
        assert!(md.contains("| task | category | result | cost | seconds |"));
        assert!(!md.contains("## Tier comparison"));
    }

    #[test]
    fn markdown_adds_comparison_for_multiple_tiers() {
        let mut a = sample();
        a.tier = "local".to_string();
        let mut b = sample();
        b.tier = "frontier".to_string();
        let md = render_markdown(&[a, b]);
        assert!(md.contains("## Tier comparison"));
        assert!(md.contains("| frontier |"));
    }

    /// A tier with one known-but-estimated cost and one unknown cost, so both
    /// caveat flags are set.
    fn caveated_sample() -> TierReport {
        let mut solved_est = TaskResult::new(
            "alpha",
            "classic",
            OracleOutcome::Pass,
            CostReading::known(0.5, true, 2),
        );
        solved_est.duration_secs = 3.0;
        let mut solved_unknown = TaskResult::new(
            "bravo",
            "classic",
            OracleOutcome::Pass,
            CostReading::unknown(),
        );
        solved_unknown.duration_secs = 4.0;
        TierReport {
            tier: "kimi".to_string(),
            provider: "moonshot".to_string(),
            model: "kimi-k2.5".to_string(),
            pinned_packs: true,
            results: vec![solved_est, solved_unknown],
        }
    }

    #[test]
    fn text_report_renders_both_cost_caveats() {
        let text = render_text(&[caveated_sample()]);
        assert!(
            text.contains("[some costs unknown]"),
            "text must flag unknown costs: {text}"
        );
        assert!(
            text.contains("[some costs estimated]"),
            "text must flag estimated costs: {text}"
        );
    }

    #[test]
    fn text_report_omits_caveats_when_all_costs_known_exactly() {
        // sample() has two exactly-known ($0.00) costs — no caveats.
        let text = render_text(&[sample()]);
        assert!(!text.contains("some costs unknown"));
        assert!(!text.contains("some costs estimated"));
    }

    #[test]
    fn markdown_renders_cost_caveats_in_summary() {
        let md = render_markdown(&[caveated_sample()]);
        // The affected headline numbers carry BOTH footnote markers directly
        // (one known+estimated cost of $0.50 over 2 solves => $/solved $0.2500).
        assert!(
            md.contains("$/solved $0.2500†‡"),
            "markdown $/solved must carry the caveat markers: {md}"
        );
        assert!(
            md.contains("median $/task $0.5000†‡"),
            "markdown median must carry the caveat markers: {md}"
        );
        // …and both footnote note-lines are present so a pasted report is honest.
        assert!(
            md.contains(MD_UNKNOWN_NOTE),
            "markdown must render the unknown-cost note: {md}"
        );
        assert!(
            md.contains(MD_ESTIMATED_NOTE),
            "markdown must render the estimated-cost note: {md}"
        );
    }

    #[test]
    fn markdown_marks_caveats_in_the_tier_comparison_table() {
        let mut a = caveated_sample();
        a.tier = "kimi".to_string();
        let mut b = sample();
        b.tier = "frontier".to_string();
        let md = render_markdown(&[a, b]);
        assert!(md.contains("## Tier comparison"));
        // The comparison section must carry the marker + the footnote, so a
        // reader pasting only the table still sees the lower-bound caveat.
        let comparison = md.split("## Tier comparison").nth(1).unwrap();
        assert!(
            comparison.contains('†'),
            "comparison table must mark the caveated tier: {comparison}"
        );
        assert!(
            comparison.contains(MD_UNKNOWN_NOTE),
            "comparison must footnote the unknown-cost caveat: {comparison}"
        );
        assert!(
            comparison.contains(MD_ESTIMATED_NOTE),
            "comparison must footnote the estimated-cost caveat: {comparison}"
        );
    }

    #[test]
    fn markdown_omits_caveats_when_all_costs_known_exactly() {
        let md = render_markdown(&[sample()]);
        assert!(!md.contains(MD_UNKNOWN_NOTE));
        assert!(!md.contains(MD_ESTIMATED_NOTE));
        assert!(!md.contains('†'));
        assert!(!md.contains('‡'));
    }

    #[test]
    fn json_report_shape() {
        let v = render_json(&[sample()]);
        let tier = &v["tiers"][0];
        assert_eq!(tier["tier"], "local");
        assert_eq!(tier["summary"]["solved"], 1);
        assert_eq!(tier["summary"]["total"], 2);
        assert_eq!(tier["tasks"][0]["task_id"], "fizzbuzz");
        assert_eq!(tier["tasks"][0]["solved"], true);
        // $/solved with 1 solved and total cost 0.0 => 0.0
        assert_eq!(tier["summary"]["dollars_per_solved"], 0.0);
    }

    /// A tier report including a budget-stopped (`NotRun`) task, plus tool-call
    /// and rate-limit signals on the ones that did run — the shape the report
    /// must represent honestly.
    fn sample_with_skip_and_signals() -> TierReport {
        let mut solved = TaskResult::new(
            "alpha",
            "classic",
            OracleOutcome::Pass,
            CostReading::known_with_tokens(1.0, false, 1, Some(1000), Some(100), Some(400), None),
        );
        solved.duration_secs = 10.0;
        solved.signals.tool_calls = 4;
        solved.signals.tool_names = vec!["shell".to_string()];
        solved.signals.rate_limit_events = 1;

        let mut failed = TaskResult::new(
            "bravo",
            "classic",
            OracleOutcome::Fail,
            CostReading::known(0.5, false, 1),
        );
        failed.duration_secs = 20.0;
        failed.signals.tool_calls = 2;

        let mut skipped = TaskResult::new(
            "charlie",
            "classic",
            OracleOutcome::NotRun,
            CostReading::unknown(),
        );
        skipped.note = Some("skipped: --budget-usd cap reached".to_string());

        TierReport {
            tier: "sonnet5".to_string(),
            provider: "anthropic".to_string(),
            model: "claude-sonnet-5".to_string(),
            pinned_packs: true,
            results: vec![solved, failed, skipped],
        }
    }

    #[test]
    fn text_report_marks_not_run_as_skip_not_fail_and_shows_the_skip_count() {
        let text = render_text(&[sample_with_skip_and_signals()]);
        assert!(text.contains("SKIP"), "{text}");
        // pass-rate is over the 2 attempted tasks (1 solved of 2), not 3.
        assert!(text.contains("1/2 solved"), "{text}");
        assert!(text.contains("1 task(s) skipped"), "{text}");
    }

    #[test]
    fn text_report_shows_median_wall_clock_tool_calls_cache_hit_and_rate_limits() {
        let text = render_text(&[sample_with_skip_and_signals()]);
        // median of [10.0, 20.0] (the not-run task is excluded) = 15.0s
        assert!(text.contains("median wall-clock=15.0s"), "{text}");
        assert!(text.contains("tool calls=6"), "{text}");
        // cache-hit: only "alpha" has known tokens => 400/1000 = 40.0%
        assert!(text.contains("cache-hit=40.0%"), "{text}");
        assert!(text.contains("rate-limit events=1"), "{text}");
    }

    #[test]
    fn markdown_shows_skip_count_and_new_columns() {
        let md = render_markdown(&[sample_with_skip_and_signals()]);
        assert!(md.contains("**skipped**"), "{md}");
        assert!(md.contains("median wall-clock 15.0s"), "{md}");
        assert!(md.contains("tool calls 6"), "{md}");
        assert!(md.contains("cache-hit 40.0%"), "{md}");
        assert!(md.contains("rate-limit events 1"), "{md}");
        // Per-task table carries the SKIP row and its tool-call column.
        assert!(md.contains("| charlie | classic | SKIP |"), "{md}");
        assert!(
            md.contains("| alpha | classic | PASS | $1.0000 | 10.0 | 4 |"),
            "{md}"
        );
    }

    #[test]
    fn json_report_carries_not_run_state_tokens_and_signals() {
        let v = render_json(&[sample_with_skip_and_signals()]);
        let tier = &v["tiers"][0];
        assert_eq!(tier["summary"]["total"], 3);
        assert_eq!(tier["summary"]["attempted"], 2);
        assert_eq!(tier["summary"]["not_run"], 1);
        // pass-rate 1/2, not 1/3.
        assert!((tier["summary"]["pass_rate"].as_f64().unwrap() - 0.5).abs() < 1e-9);

        let alpha = &tier["tasks"][0];
        assert_eq!(alpha["task_id"], "alpha");
        assert_eq!(alpha["input_tokens"], 1000);
        assert_eq!(alpha["cache_read_tokens"], 400);
        assert!((alpha["cache_hit_rate"].as_f64().unwrap() - 0.4).abs() < 1e-9);
        assert_eq!(alpha["signals"]["tool_calls"], 4);
        assert_eq!(alpha["signals"]["tool_names"][0], "shell");
        assert_eq!(alpha["signals"]["rate_limit_events"], 1);

        let charlie = &tier["tasks"][2];
        assert_eq!(charlie["oracle"], "not_run");
        assert_eq!(charlie["solved"], false);
        assert_eq!(charlie["cost_usd"], serde_json::Value::Null);
    }
}
