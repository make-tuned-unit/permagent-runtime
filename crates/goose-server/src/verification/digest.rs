//! Evidence digest — deterministic Rust assembly (no LLM) of the evidence
//! shown in the decision inbox.
//!
//! Layered evidence (amendment A3): machine-assembled plain-language
//! summary fields (`checks_summary`, `verifier_summary`, `costs` with dollars
//! first) are rendered first by the UI; raw check/diff/verifier output stays
//! in the detail layer. All strings are plain-text data (S2). Whole digest is
//! capped at 64KiB.

use serde::{Deserialize, Serialize};

use super::checks::{CheckResult, CheckStatus, CompletionCheck};
use super::verifier::{VerdictStatus, VerifierRun};

/// Whole-digest serialized size cap.
pub const MAX_DIGEST_BYTES: usize = 64 * 1024;
/// Per-check output excerpt cap.
pub const MAX_EXCERPT_BYTES: usize = 2 * 1024;
/// Max per-file diff rows.
pub const MAX_PER_FILE: usize = 50;
/// Max out-of-path files listed.
pub const MAX_OUT_OF_PATH: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChecksSummary {
    pub passed_count: usize,
    pub total_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tests_run: Option<u64>,
    /// e.g. "All 4 automated checks passed (412 tests)"
    pub one_line: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Costs {
    /// Dollars first: present when a token rate exists in config, else null
    /// (token counts below are the fallback detail).
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accumulated_total_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_session_id: Option<String>,
    pub attempt_count: u64,
    /// Note explaining the cost basis (e.g. "no token rate configured").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DigestCheck {
    #[serde(rename = "type")]
    pub check_type: String,
    pub summary: String,
    pub status: CheckStatus,
    /// Raw output excerpt, ≤2KiB (detail layer).
    pub output_excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerFileDiff {
    pub path: String,
    pub insertions: u64,
    pub deletions: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiffSummary {
    pub files_changed: usize,
    pub insertions: u64,
    pub deletions: u64,
    pub per_file: Vec<PerFileDiff>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifierDetail {
    pub status: VerdictStatus,
    pub rationale: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceDigest {
    pub version: u32,
    pub goal_id: String,
    pub goal_title: String,
    // ── Summary layer (rendered first by the UI) ──
    pub checks_summary: ChecksSummary,
    /// One-liner like "Aria's reviewer confirmed the work matches what was approved"
    /// (the configured persona name, resolved at assembly).
    pub verifier_summary: String,
    pub costs: Costs,
    // ── Detail layer ──
    pub checks: Vec<DigestCheck>,
    pub diff: DiffSummary,
    pub out_of_path_files: Vec<String>,
    pub verifier: VerifierDetail,
    pub started_at: String,
    pub finished_at: String,
}

// ── Summary-line builders ───────────────────────────────────────────────────

/// Extract a "N tests" count from check outputs (e.g. cargo's "412 passed").
pub fn extract_tests_run(results: &[CheckResult]) -> Option<u64> {
    static RE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(\d+) passed").unwrap());
    let mut total: u64 = 0;
    let mut found = false;
    for r in results {
        if let Some(out) = r.evidence.stdout_tail.as_deref() {
            for cap in RE.captures_iter(out) {
                if let Ok(n) = cap[1].parse::<u64>() {
                    total += n;
                    found = true;
                }
            }
        }
    }
    found.then_some(total)
}

pub fn checks_one_line(passed: usize, total: usize, tests_run: Option<u64>) -> String {
    let tests_suffix = tests_run
        .map(|n| format!(" ({} tests)", n))
        .unwrap_or_default();
    if total == 0 {
        "No automated checks were declared for this goal".to_string()
    } else if passed == total {
        format!("All {} automated checks passed{}", total, tests_suffix)
    } else {
        format!(
            "{} of {} automated checks passed{}",
            passed, total, tests_suffix
        )
    }
}

pub fn verifier_one_line(
    persona_name: &str,
    status: VerdictStatus,
    degraded_reason: Option<&str>,
) -> String {
    match (status, degraded_reason) {
        (_, Some(reason)) => format!(
            "{persona_name}'s reviewer could not complete its review ({reason}) — a human decision is needed"
        ),
        (VerdictStatus::Pass, None) => {
            format!("{persona_name}'s reviewer confirmed the work matches what was approved")
        }
        (VerdictStatus::Fail, None) => {
            format!("{persona_name}'s reviewer found the work does not match what was approved")
        }
        (VerdictStatus::Uncertain, None) => {
            format!("{persona_name}'s reviewer could not confirm the work matches what was approved — a human decision is needed")
        }
    }
}

/// Build the costs block, leading with dollars.
///
/// Cost is single-sourced: `ledger_cost_usd` is the worker session's
/// `accumulated_cost_usd` — the sum of the per-call `cost_ledger`, every entry of
/// which was folded by the one canonical `cost_of` (cache-aware). When present it
/// IS the cost, verbatim. The legacy blended `$/1k-token` estimate is only a
/// fallback for sessions that predate the ledger (or a model with no registry
/// price), and is labelled as an estimate so the two are never confused.
pub fn build_costs(
    ledger_cost_usd: Option<f64>,
    tokens: Option<i64>,
    usd_per_1k_tokens: Option<f64>,
    worker_session_id: Option<String>,
    attempt_count: u64,
) -> Costs {
    let (cost_usd, note) = match ledger_cost_usd {
        // Authoritative: the per-call ledger total. No note — this is THE number.
        Some(c) => (Some(c), None),
        None => {
            let blended = match (tokens, usd_per_1k_tokens) {
                (Some(t), Some(rate)) if t >= 0 => Some((t as f64 / 1000.0) * rate),
                _ => None,
            };
            let note = match blended {
                Some(_) => Some(
                    "estimated from a blended token rate (no per-call ledger cost)".to_string(),
                ),
                None => Some(match tokens {
                    Some(_) => "no ledger cost or token rate — showing raw token count".to_string(),
                    None => "worker cost and token usage unavailable".to_string(),
                }),
            };
            (blended, note)
        }
    };
    Costs {
        cost_usd,
        accumulated_total_tokens: tokens,
        worker_session_id,
        attempt_count,
        note,
    }
}

// ── Assembly ────────────────────────────────────────────────────────────────

fn excerpt_for(result: &CheckResult) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(code) = result.evidence.exit_code {
        parts.push(format!("exit: {}", code));
    }
    if let Some(s) = result.evidence.http_status {
        parts.push(format!("http: {}", s));
    }
    if let Some(m) = result.evidence.message.as_deref() {
        parts.push(m.to_string());
    }
    if let Some(out) = result.evidence.stdout_tail.as_deref() {
        if !out.trim().is_empty() {
            parts.push(format!("stdout: {}", out.trim_end()));
        }
    }
    if let Some(err) = result.evidence.stderr_tail.as_deref() {
        if !err.trim().is_empty() {
            parts.push(format!("stderr: {}", err.trim_end()));
        }
    }
    if let Some(body) = result.evidence.body_excerpt.as_deref() {
        if !body.trim().is_empty() {
            parts.push(format!("body: {}", body.trim_end()));
        }
    }
    if let Some(matches) = result.evidence.matches.as_ref() {
        if !matches.is_empty() {
            parts.push(format!("matches:\n{}", matches.join("\n")));
        }
    }
    let joined = parts.join("\n");
    let (tail, _) = super::checks::tail_str(&joined, MAX_EXCERPT_BYTES);
    tail
}

/// Deterministic digest assembly (no LLM).
#[allow(clippy::too_many_arguments)]
pub fn assemble_digest(
    persona_name: &str,
    goal_id: &str,
    goal_title: &str,
    checks: &[CompletionCheck],
    check_results: &[CheckResult],
    diff: DiffSummary,
    out_of_path_files: Vec<String>,
    verifier_status: VerdictStatus,
    verifier_run: &VerifierRun,
    costs: Costs,
    started_at: &str,
    finished_at: &str,
) -> EvidenceDigest {
    let passed = check_results
        .iter()
        .filter(|r| r.status == CheckStatus::Pass)
        .count();
    let total = check_results.len();
    let tests_run = extract_tests_run(check_results);

    let digest_checks: Vec<DigestCheck> = check_results
        .iter()
        .map(|r| DigestCheck {
            check_type: r.check_type.clone(),
            summary: checks
                .get(r.check_index)
                .map(|c| c.summary())
                .unwrap_or_else(|| r.check_type.clone()),
            status: r.status,
            output_excerpt: excerpt_for(r),
        })
        .collect();

    let rationale = verifier_run
        .grades
        .as_ref()
        .map(|g| g.rationale.clone())
        .unwrap_or_default();

    let mut diff = diff;
    diff.per_file.truncate(MAX_PER_FILE);
    let mut out_of_path = out_of_path_files;
    out_of_path.truncate(MAX_OUT_OF_PATH);

    let digest = EvidenceDigest {
        version: 1,
        goal_id: goal_id.to_string(),
        goal_title: goal_title.to_string(),
        checks_summary: ChecksSummary {
            passed_count: passed,
            total_count: total,
            tests_run,
            one_line: checks_one_line(passed, total, tests_run),
        },
        verifier_summary: verifier_one_line(
            persona_name,
            verifier_status,
            verifier_run.degraded_reason.as_deref(),
        ),
        costs,
        checks: digest_checks,
        diff,
        out_of_path_files: out_of_path,
        verifier: VerifierDetail {
            status: verifier_status,
            rationale,
            model: verifier_run.model.clone(),
            degraded_reason: verifier_run.degraded_reason.clone(),
        },
        started_at: started_at.to_string(),
        finished_at: finished_at.to_string(),
    };

    enforce_size_cap(digest)
}

/// Enforce the 64KiB whole-digest cap by progressively shrinking detail-layer
/// content (excerpts first, then per-file rows). Summary layer is never cut.
fn enforce_size_cap(mut digest: EvidenceDigest) -> EvidenceDigest {
    let size = |d: &EvidenceDigest| serde_json::to_vec(d).map(|v| v.len()).unwrap_or(usize::MAX);

    if size(&digest) <= MAX_DIGEST_BYTES {
        return digest;
    }
    // Pass 1: shrink excerpts to 512B each.
    for c in &mut digest.checks {
        let (tail, _) = super::checks::tail_str(&c.output_excerpt, 512);
        c.output_excerpt = tail;
    }
    if size(&digest) <= MAX_DIGEST_BYTES {
        return digest;
    }
    // Pass 2: drop excerpts entirely.
    for c in &mut digest.checks {
        c.output_excerpt = "(omitted — digest size cap)".to_string();
    }
    if size(&digest) <= MAX_DIGEST_BYTES {
        return digest;
    }
    // Pass 3: drop per-file diff rows.
    digest.diff.per_file.clear();
    digest
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verification::checks::CheckEvidence;
    use crate::verification::verifier::{Grade, RubricGrades};

    fn passing_result(stdout: Option<&str>) -> CheckResult {
        CheckResult {
            check_index: 0,
            check_type: "command_exit_zero".to_string(),
            status: CheckStatus::Pass,
            started_at: "2026-06-11T00:00:00Z".to_string(),
            duration_ms: 5,
            evidence: CheckEvidence {
                exit_code: Some(0),
                stdout_tail: stdout.map(String::from),
                ..Default::default()
            },
            truncated: false,
        }
    }

    fn run_with_grades() -> VerifierRun {
        VerifierRun {
            grades: Some(RubricGrades {
                q1_intent: Grade::Pass,
                q2_evidence: Grade::Pass,
                q3_checks: Grade::Pass,
                q4_paths: Grade::Pass,
                rationale: "Looks right.".to_string(),
            }),
            raw_output: String::new(),
            degraded_reason: None,
            model: "qwen2.5:7b".to_string(),
        }
    }

    #[test]
    fn checks_one_line_variants() {
        assert_eq!(
            checks_one_line(4, 4, Some(412)),
            "All 4 automated checks passed (412 tests)"
        );
        assert_eq!(
            checks_one_line(2, 4, None),
            "2 of 4 automated checks passed"
        );
        assert!(checks_one_line(0, 0, None).contains("No automated checks"));
    }

    #[test]
    fn verifier_one_line_variants() {
        assert!(verifier_one_line("Aria", VerdictStatus::Pass, None)
            .contains("confirmed the work matches"));
        assert!(verifier_one_line("Aria", VerdictStatus::Fail, None).contains("does not match"));
        assert!(verifier_one_line("Aria", VerdictStatus::Uncertain, None)
            .contains("human decision is needed"));
        let degraded =
            verifier_one_line("Aria", VerdictStatus::Uncertain, Some("Ollama unreachable"));
        assert!(degraded.contains("could not complete"));
        assert!(degraded.contains("Ollama unreachable"));
    }

    #[test]
    fn verifier_one_line_uses_configured_persona_name() {
        // The persona name is interpolated — no hardcoded "Henry" leak.
        for status in [
            VerdictStatus::Pass,
            VerdictStatus::Fail,
            VerdictStatus::Uncertain,
        ] {
            let line = verifier_one_line("Nova", status, None);
            assert!(line.starts_with("Nova's reviewer"), "got: {line}");
            assert!(!line.contains("Henry"), "leaked Henry: {line}");
        }
        let degraded = verifier_one_line("Nova", VerdictStatus::Pass, Some("offline"));
        assert!(degraded.starts_with("Nova's reviewer"));
        assert!(!degraded.contains("Henry"));
    }

    #[test]
    fn tests_run_extraction() {
        let results = vec![passing_result(Some(
            "test result: ok. 410 passed; 0 failed\nrunning 2 tests\ntest result: ok. 2 passed",
        ))];
        assert_eq!(extract_tests_run(&results), Some(412));
        assert_eq!(extract_tests_run(&[passing_result(None)]), None);
    }

    #[test]
    fn costs_prefers_ledger_over_blended_estimate() {
        // Ledger cost present → it IS the number (verbatim), no note, even when a
        // blended rate would compute something different.
        let c = build_costs(
            Some(2.5),
            Some(120_000),
            Some(0.01),
            Some("s-1".to_string()),
            2,
        );
        assert!((c.cost_usd.unwrap() - 2.5).abs() < 1e-9);
        assert!(c.note.is_none());
        assert_eq!(c.accumulated_total_tokens, Some(120_000));
    }

    #[test]
    fn costs_falls_back_to_blended_estimate_when_no_ledger() {
        // No ledger cost → blended $/1k estimate, clearly labelled as an estimate.
        let c = build_costs(None, Some(120_000), Some(0.01), None, 2);
        assert!((c.cost_usd.unwrap() - 1.2).abs() < 1e-9);
        assert!(c.note.as_deref().unwrap().contains("estimated"));
    }

    #[test]
    fn costs_null_with_note_when_no_ledger_and_no_rate() {
        let c = build_costs(None, Some(120_000), None, None, 1);
        assert_eq!(c.cost_usd, None);
        assert!(c
            .note
            .as_deref()
            .unwrap()
            .contains("no ledger cost or token rate"));
        let c2 = build_costs(None, None, Some(0.01), None, 1);
        assert_eq!(c2.cost_usd, None);
        assert!(c2.note.as_deref().unwrap().contains("unavailable"));
    }

    #[test]
    fn digest_round_trips_with_deny_unknown_fields() {
        let digest = assemble_digest(
            "Aria",
            "g-1",
            "Toy goal",
            &[],
            &[passing_result(Some("ok"))],
            DiffSummary::default(),
            vec![],
            VerdictStatus::Pass,
            &run_with_grades(),
            build_costs(None, None, None, None, 1),
            "2026-06-11T00:00:00Z",
            "2026-06-11T00:00:10Z",
        );
        let json = serde_json::to_string(&digest).unwrap();
        let parsed: EvidenceDigest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.checks_summary.passed_count, 1);

        // Unknown fields rejected.
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v.as_object_mut()
            .unwrap()
            .insert("sneaky".to_string(), serde_json::json!(true));
        assert!(serde_json::from_value::<EvidenceDigest>(v).is_err());
    }

    #[test]
    fn size_cap_enforced() {
        let big = "x".repeat(10_000);
        let results: Vec<CheckResult> = (0..30)
            .map(|i| {
                let mut r = passing_result(Some(&big));
                r.check_index = 0;
                r.evidence.stderr_tail = Some(big.clone());
                let _ = i;
                r
            })
            .collect();
        let per_file: Vec<PerFileDiff> = (0..200)
            .map(|i| PerFileDiff {
                path: format!("src/some/deeply/nested/path/file_{}.rs", i),
                insertions: 1,
                deletions: 1,
            })
            .collect();
        let digest = assemble_digest(
            "Aria",
            "g-1",
            "Big goal",
            &[],
            &results,
            DiffSummary {
                files_changed: 200,
                insertions: 200,
                deletions: 200,
                per_file,
            },
            (0..100).map(|i| format!("out/of/path/{}.rs", i)).collect(),
            VerdictStatus::Pass,
            &run_with_grades(),
            build_costs(None, None, None, None, 1),
            "2026-06-11T00:00:00Z",
            "2026-06-11T00:00:10Z",
        );
        let serialized = serde_json::to_vec(&digest).unwrap();
        assert!(
            serialized.len() <= MAX_DIGEST_BYTES,
            "digest is {} bytes",
            serialized.len()
        );
        // Caps applied to lists.
        assert!(digest.diff.per_file.len() <= MAX_PER_FILE);
        assert!(digest.out_of_path_files.len() <= MAX_OUT_OF_PATH);
        // Summary layer survives.
        assert!(!digest.checks_summary.one_line.is_empty());
    }

    #[test]
    fn excerpt_capped_at_2kib() {
        let big = "y".repeat(50_000);
        let mut r = passing_result(Some(&big));
        r.evidence.stderr_tail = Some(big.clone());
        let digest = assemble_digest(
            "Aria",
            "g-1",
            "Goal",
            &[],
            &[r],
            DiffSummary::default(),
            vec![],
            VerdictStatus::Pass,
            &run_with_grades(),
            build_costs(None, None, None, None, 1),
            "t0",
            "t1",
        );
        assert!(digest.checks[0].output_excerpt.len() <= MAX_EXCERPT_BYTES);
    }
}
