//! Deterministic rendering of eval results as text, Markdown, or JSON.
//!
//! Rendering is pure (no clocks, no environment) so the output is stable and
//! testable. Costs render to four decimal places; `n/a` marks an unknown cost or
//! an undefined ratio (e.g. $/solved with nothing solved).

use crate::metrics::{aggregate, Aggregate, TaskResult};
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

fn solved_mark(solved: bool) -> &'static str {
    if solved {
        "PASS"
    } else {
        "FAIL"
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
                solved_mark(r.solved),
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
            agg.total,
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
            agg.total,
            fmt_pct(agg.pass_rate),
            fmt_usd(agg.dollars_per_solved),
            fmt_usd(agg.median_cost_per_task),
            fmt_usd(agg.total_cost_usd),
        ));
        if agg.any_cost_unknown {
            out.push_str(&format!("- {MD_UNKNOWN_NOTE}\n"));
        }
        if agg.any_estimated {
            out.push_str(&format!("- {MD_ESTIMATED_NOTE}\n"));
        }
        out.push('\n');
        out.push_str("| task | category | result | cost | seconds |\n");
        out.push_str("|------|----------|--------|------|---------|\n");
        for r in &tr.results {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {:.1} |\n",
                r.task_id,
                r.category,
                solved_mark(r.solved),
                fmt_usd(r.cost.usd),
                r.duration_secs,
            ));
        }
        out.push('\n');
    }

    if reports.len() > 1 {
        out.push_str("## Tier comparison\n\n");
        out.push_str("| tier | pass-rate | solved | $/solved | median $/task | total $ |\n");
        out.push_str("|------|-----------|--------|----------|---------------|---------|\n");
        let mut any_unknown = false;
        let mut any_estimated = false;
        for tr in reports {
            let agg = tr.aggregate();
            let mark = cost_caveat_marker(&agg);
            any_unknown |= agg.any_cost_unknown;
            any_estimated |= agg.any_estimated;
            out.push_str(&format!(
                "| {} | {} | {}/{} | {}{mark} | {}{mark} | {}{mark} |\n",
                tr.tier,
                fmt_pct(agg.pass_rate),
                agg.solved,
                agg.total,
                fmt_usd(agg.dollars_per_solved),
                fmt_usd(agg.median_cost_per_task),
                fmt_usd(agg.total_cost_usd),
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
                        "duration_secs": r.duration_secs,
                        "harness_exit": r.harness_exit,
                        "harness_timed_out": r.harness_timed_out,
                        "note": r.note,
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
                    "solved": agg.solved,
                    "pass_rate": agg.pass_rate,
                    "dollars_per_solved": agg.dollars_per_solved,
                    "median_cost_per_task": agg.median_cost_per_task,
                    "total_cost_usd": agg.total_cost_usd,
                    "any_cost_unknown": agg.any_cost_unknown,
                    "any_estimated": agg.any_estimated,
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
}
