//! Detecting a provider-unavailable (billing/auth/quota) failure, and
//! deciding when a round of tasks against the same provider must stop
//! launching more of them.
//!
//! ## The defect this exists for
//!
//! 2026-09-14, M22 zai round w1
//! (`permagent-worktrees/laneM17/glm47-m22/w1.json` and its
//! `w1-runs/glm47-<task>-*/data/logs/llm_request.*.jsonl`): starting at task
//! #9, every provider call to z.ai returned
//!
//! ```text
//! {"error": "Credits exhausted: Insufficient balance or no resource package. Please recharge."}
//! ```
//!
//! The harness never got a single token back — each task ended in ~4s via
//! the coordinator's own rule-5 free-text guard — yet the driver recorded
//! all 11 remaining tasks as `fail` with `cost_usd: null` and finished the
//! round normally. That is dishonest grading (the harness was never really
//! attempted) and wasteful (11 near-instant, guaranteed-identical failures
//! that could have been skipped after the second one). This module gives
//! [`crate::runner::run_task`] a way to recognize the condition and
//! [`crate::runner::run_tier`] a way to stop the round early instead of
//! grinding through the rest.
//!
//! ## Detection
//!
//! [`detect`] scans a block of text — either a harness run's captured
//! stdout+stderr log, or one of its `data/logs/llm_request.*.jsonl`
//! provider-call logs (see [`detect_in_run`], which checks both, harness log
//! first) — line by line for a billing/auth/quota marker. The z.ai failure
//! above never reaches the harness's own log verbatim (the harness only
//! paraphrases it, e.g. "the provider rejected the request for billing
//! reasons"); the raw error body land in `llm_request.*.jsonl` instead
//! (confirmed against the exact task-#9 run cited above), which is why
//! [`detect_in_run`] must check both sources — checking only the harness log
//! would silently miss the real-world case this module was built from.
//!
//! [`PROVIDER_UNAVAILABLE_MARKERS`] holds every plain-substring marker,
//! matched case-insensitively. Two further shapes cannot be expressed as a
//! plain substring without risking false positives on an ordinary line
//! number or token count (the same problem [`crate::harness_log`]'s
//! `RATE_LIMIT_RE` already solved for a bare `429`), so they are separate,
//! documented regexes right below the const: [`HTTP_BILLING_RE`] (401/402/403
//! with billing-shaped context) and [`QUOTA_429_RE`] (429 with `quota`
//! context). Together these three are the ONLY place a provider
//! billing/auth/quota substring should be hardcoded — add a new marker here,
//! not at a call site.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

/// Plain, case-insensitive substring markers for a provider billing/auth/quota
/// failure. Checked before either regex below since they are unambiguous
/// literal phrases with no bare-number false-positive risk.
///
/// - `"credits exhausted"` / `"insufficient balance"` — z.ai's literal error
///   body once an account's balance is spent (the exact M22 zai w1 defect
///   text above), and also Anthropic's own credits-exhausted phrasing seen
///   in `permagentd`'s planner-decomposition fallback logs ("Credits
///   exhausted: Your credit balance is too low to access the Anthropic
///   API...").
/// - `"insufficient_quota"` — the OpenAI-shaped quota error code.
pub const PROVIDER_UNAVAILABLE_MARKERS: &[&str] = &[
    "credits exhausted",
    "insufficient balance",
    "insufficient_quota",
];

/// An HTTP 401/402/403 co-occurring with billing-shaped wording on the same
/// line. A bare status code alone is common in ordinary text (a port
/// number, a line count), so — mirroring `harness_log::RATE_LIMIT_RE`'s
/// bare-`429` guard — this requires explicit `status`/`http` framing next to
/// the code AND one of `billing`/`payment`/`credit`/`subscription`/`plan`/
/// `quota`/`balance` within 80 characters either side of it.
// NOTE: built with `concat!` over several one-line raw strings rather than a
// single multi-line raw string with a trailing `\` — a raw string does NOT
// support the ordinary string literal's backslash-newline continuation (that
// escape does not exist inside `r"..."` at all), so a trailing `\` before a
// line break would land in the compiled PATTERN as a literal `\` followed by
// a real newline character rather than being elided.
static HTTP_BILLING_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)\b(?:status|http)\b[^0-9\n]{0,10}\b40[123]\b.{0,80}\b(?:billing|payment|credit|subscription|plan|quota|balance)\b",
        r"|\b(?:billing|payment|credit|subscription|plan|quota|balance)\b.{0,80}\b(?:status|http)\b[^0-9\n]{0,10}\b40[123]\b",
    ))
    .expect("HTTP_BILLING_RE is a valid regex")
});

/// A 429 explicitly tied to a quota, not a generic rate limit (which
/// `harness_log::RATE_LIMIT_RE` already covers as its own, unrelated
/// signal). Same HTTP-status-context requirement as [`HTTP_BILLING_RE`].
static QUOTA_429_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)\b(?:status|http)\b[^0-9\n]{0,10}\b429\b.{0,80}\bquota\b",
        r"|\bquota\b.{0,80}\b(?:status|http)\b[^0-9\n]{0,10}\b429\b",
        r"|\(\s*429\s*\).{0,80}\bquota\b",
    ))
    .expect("QUOTA_429_RE is a valid regex")
});

/// Scan `text` line by line for a provider-unavailable marker, returning the
/// first one found (a [`PROVIDER_UNAVAILABLE_MARKERS`] entry verbatim, or a
/// fixed label for one of the two regex shapes). `None` means no marker
/// matched — NOT that the provider is known-healthy, just that this text
/// gave no evidence either way.
pub fn detect(text: &str) -> Option<String> {
    for line in text.lines() {
        let lower = line.to_lowercase();
        for marker in PROVIDER_UNAVAILABLE_MARKERS {
            if lower.contains(marker) {
                return Some((*marker).to_string());
            }
        }
        if HTTP_BILLING_RE.is_match(line) {
            return Some("HTTP 401/402/403 billing".to_string());
        }
        if QUOTA_429_RE.is_match(line) {
            return Some("429 quota".to_string());
        }
    }
    None
}

/// Every `llm_request.*.jsonl` file directly under `<data_root>/logs`
/// (`SubprocessRecipeSource`/`build_invocation` scope every run's
/// `PERMAGENT_PATH_ROOT` to its own `data_root`, so this is exactly one
/// run's provider-call logs), in a stable (sorted) order.
fn llm_request_log_paths(data_root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(data_root.join("logs")) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("llm_request.") && n.ends_with(".jsonl"))
        })
        .collect();
    paths.sort();
    paths
}

/// Detect a provider-unavailable marker for one task's run: the harness's
/// own captured log first, then (best-effort — a missing/unreadable file is
/// not an error) each `llm_request.*.jsonl` under `data_root`. See the
/// module docs for why both sources must be checked: the raw provider error
/// body this module was built to catch never reaches the harness log
/// verbatim, only the `llm_request` logs.
pub fn detect_in_run(harness_log_text: &str, data_root: &Path) -> Option<String> {
    if let Some(marker) = detect(harness_log_text) {
        return Some(marker);
    }
    for path in llm_request_log_paths(data_root) {
        if let Ok(text) = fs::read_to_string(&path) {
            if let Some(marker) = detect(&text) {
                return Some(marker);
            }
        }
    }
    None
}

/// How many CONSECUTIVE provider-unavailable tasks trip [`ConsecutiveTracker`]
/// — Lane K10's repair item (2): a single transient billing/auth/quota
/// failure is not evidence the whole round is doomed, but two in a row (the
/// M22 zai w1 shape, where the SAME failure recurred on every remaining
/// task) is.
pub const CONSECUTIVE_STOP_THRESHOLD: usize = 2;

/// Tracks CONSECUTIVE provider-unavailable task results within one round
/// (one [`crate::runner::run_tier`] call) and decides when the round must
/// stop launching further tasks. Mirrors
/// [`crate::runner::BudgetTracker`]'s shape (attempted count, one-shot stop
/// message) but is deliberately a separate tracker: a provider-unavailable
/// stop is not a budget-cap stop, and conflating them would misreport the
/// M22 zai w1 shape as "cap reached" when no cap was ever set.
#[derive(Debug, Default)]
pub struct ConsecutiveTracker {
    attempted: usize,
    consecutive: usize,
    stopped_marker: Option<String>,
}

impl ConsecutiveTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// The marker the round stopped on, once tripped. `None` means the
    /// round is still launching tasks normally.
    pub fn stopped_marker(&self) -> Option<&str> {
        self.stopped_marker.as_deref()
    }

    /// Record one just-launched task's provider-unavailable evidence (`None`
    /// when the task ran without hitting this condition — including a hard
    /// runner error unrelated to provider billing, which must not extend a
    /// provider-unavailable streak). Returns the one-line `PROVIDER
    /// UNAVAILABLE: …` stop message exactly once — the call that pushes
    /// `consecutive` to [`CONSECUTIVE_STOP_THRESHOLD`] — so a caller
    /// printing it after every task does not repeat it.
    pub fn record(&mut self, tier_name: &str, provider_error: Option<&str>) -> Option<String> {
        self.attempted += 1;
        match provider_error {
            Some(marker) => {
                self.consecutive += 1;
                if self.consecutive >= CONSECUTIVE_STOP_THRESHOLD && self.stopped_marker.is_none() {
                    self.stopped_marker = Some(marker.to_string());
                    return Some(format!(
                        "PROVIDER UNAVAILABLE: {tier_name} {marker} after {} tasks",
                        self.attempted
                    ));
                }
            }
            None => self.consecutive = 0,
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact `llm_request.*.jsonl` error body from the M22 zai w1 defect
    /// (`glm47-select-ambiguous-choice-escalates-decision-...-9/data/logs/llm_request.0.jsonl`).
    #[test]
    fn detects_the_zai_credits_exhausted_body_verbatim() {
        let line = r#"{"error": "Credits exhausted: Insufficient balance or no resource package. Please recharge."}"#;
        assert_eq!(detect(line).as_deref(), Some("credits exhausted"));
    }

    #[test]
    fn detects_the_anthropic_credits_exhausted_phrasing() {
        let line = "Planner decomposition failed (LLM decomposition failed: complete_fast \
                     physical attempt failed: Credits exhausted: Your credit balance is too \
                     low to access the Anthropic API. Please go to Plans & Billing to upgrade \
                     or purchase credits.); escalating to default provider";
        assert_eq!(detect(line).as_deref(), Some("credits exhausted"));
    }

    #[test]
    fn detects_insufficient_quota_marker() {
        assert_eq!(
            detect(r#"{"error":{"code":"insufficient_quota","message":"..."}}"#).as_deref(),
            Some("insufficient_quota")
        );
    }

    #[test]
    fn detects_http_401_with_billing_context() {
        assert_eq!(
            detect("request failed: status: 401 — billing information is invalid").as_deref(),
            Some("HTTP 401/402/403 billing")
        );
    }

    #[test]
    fn detects_http_403_with_subscription_context_reversed_order() {
        assert_eq!(
            detect("your subscription has lapsed (HTTP 403)").as_deref(),
            Some("HTTP 401/402/403 billing")
        );
    }

    #[test]
    fn detects_429_with_quota_context() {
        assert_eq!(
            detect("Request failed: quota exceeded (status: 429)").as_deref(),
            Some("429 quota")
        );
    }

    #[test]
    fn bare_status_codes_without_billing_or_quota_context_do_not_match() {
        // A generic rate limit (no quota wording) is `harness_log`'s signal,
        // not this module's.
        assert_eq!(detect("Request failed: rate limited (status: 429)"), None);
        // An ordinary 401 with no billing context anywhere nearby.
        assert_eq!(detect("auth check returned status: 401, retrying"), None);
        // A token count or id that merely contains these digits.
        assert_eq!(detect("processed 10,429 tokens across the run"), None);
        assert_eq!(detect("goal card 01a08578-...-7b7fe0189403"), None);
    }

    #[test]
    fn detect_in_run_falls_back_to_llm_request_logs_when_the_harness_log_has_no_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let data_root = tmp.path().join("data");
        let logs_dir = data_root.join("logs");
        fs::create_dir_all(&logs_dir).unwrap();
        // The harness log only ever paraphrases — never the raw marker.
        let harness_log_text =
            "zai rejected the request — the provider rejected the request for billing reasons \
             (credit balance too low), and no fallback model is configured.";
        fs::write(
            logs_dir.join("llm_request.0.jsonl"),
            r#"{"error": "Credits exhausted: Insufficient balance or no resource package. Please recharge."}"#,
        )
        .unwrap();

        assert_eq!(
            detect(harness_log_text),
            None,
            "harness log alone has no verbatim marker"
        );
        assert_eq!(
            detect_in_run(harness_log_text, &data_root).as_deref(),
            Some("credits exhausted")
        );
    }

    #[test]
    fn detect_in_run_is_none_when_neither_source_has_a_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let data_root = tmp.path().join("data");
        fs::create_dir_all(data_root.join("logs")).unwrap();
        fs::write(
            data_root.join("logs").join("llm_request.0.jsonl"),
            r#"{"ok": true}"#,
        )
        .unwrap();
        assert_eq!(detect_in_run("nothing to see here", &data_root), None);
    }

    #[test]
    fn detect_in_run_is_none_when_data_root_has_no_logs_dir() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            detect_in_run("nothing to see here", &tmp.path().join("data")),
            None
        );
    }

    #[test]
    fn single_transient_error_does_not_stop() {
        let mut t = ConsecutiveTracker::new();
        assert_eq!(t.record("zai-tier", Some("credits exhausted")), None);
        assert!(t.stopped_marker().is_none());
        // A later successful task resets the streak.
        assert_eq!(t.record("zai-tier", None), None);
        assert!(t.stopped_marker().is_none());
        assert_eq!(t.record("zai-tier", Some("credits exhausted")), None);
        assert!(
            t.stopped_marker().is_none(),
            "the streak was reset by the intervening success, so this is only 1 consecutive"
        );
    }

    #[test]
    fn two_consecutive_trips_the_stop_exactly_once() {
        let mut t = ConsecutiveTracker::new();
        assert_eq!(t.record("zai-tier", None), None, "task 1: ok");
        assert_eq!(
            t.record("zai-tier", Some("credits exhausted")),
            None,
            "task 2: first failure, not yet tripped"
        );
        let msg = t
            .record("zai-tier", Some("credits exhausted"))
            .expect("task 3: second consecutive failure trips the stop");
        assert_eq!(
            msg, "PROVIDER UNAVAILABLE: zai-tier credits exhausted after 3 tasks",
            "{msg}"
        );
        assert_eq!(t.stopped_marker(), Some("credits exhausted"));
        // Tripping is reported exactly once, even if the caller keeps
        // recording (it should not, in practice, once `stopped_marker` is
        // `Some`, but the tracker itself must not repeat the message).
        assert_eq!(t.record("zai-tier", Some("credits exhausted")), None);
    }

    #[test]
    fn a_hard_runner_error_with_no_provider_marker_resets_the_streak() {
        // `run_tier` passes `None` for any task whose `provider_error` was
        // never set — including a hard runner error unrelated to billing
        // (e.g. a `RecipeSource` failure) — which must not silently extend
        // a provider-unavailable streak.
        let mut t = ConsecutiveTracker::new();
        assert_eq!(t.record("zai-tier", Some("credits exhausted")), None);
        assert_eq!(t.record("zai-tier", None), None);
        assert_eq!(t.record("zai-tier", Some("credits exhausted")), None);
        assert!(t.stopped_marker().is_none());
    }
}
