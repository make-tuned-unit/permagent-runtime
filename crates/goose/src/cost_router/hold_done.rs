//! Premature-done hold — Local / Mechanical workers that claim done too soon.
//!
//! AdvisorGate hides a bad “done” and injects a plan. We already have a
//! stricter cross-family reviewer. This is only a hold: do not emit success,
//! inject a short plan, park to the Decision Inbox if the hold repeats.
//! No neural APPROVE/REDO.

use serde::{Deserialize, Serialize};

use super::recommend::WorkflowRole;
use super::tool_signals::ToolTranscriptSignals;

/// Metadata key on the goal card. Survives re-dispatch.
pub const HOLD_METADATA_KEY: &str = "hold_done";

/// How many holds before we park instead of injecting another plan.
pub const MAX_HOLDS: u8 = 2;

/// Lane R18g: the exact two `Park` reasons [`decide_goal_hold`] ever raises —
/// both a verify/commit gap, never a jail, approval, or safety refusal (those
/// are raised by wholly separate code paths that never call this function).
/// Named so a headless-run policy elsewhere can check a park reason against
/// these by identity instead of duplicating (and risking drift from) the
/// literal text.
pub const PARK_REASON_NO_VERIFY: &str = "held twice without verify — handing to the Decision Inbox";
pub const PARK_REASON_UNCOMMITTED: &str =
    "held twice with verified but uncommitted work — handing to the Decision Inbox";

/// Lane R21a (A21 Q1): the read-only counterpart of [`PARK_REASON_NO_VERIFY`]
/// — a goal whose acceptance is a report/review (`expects_change: false`, see
/// [`goal_expects_change`]) held twice without ever leaving a review report.
/// Never raised for a mutating goal — see [`decide_goal_done_hold`].
pub const PARK_REASON_NO_REVIEW_REPORT: &str =
    "held twice without a review report — handing to the Decision Inbox";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldOutcome {
    Allow,
    Hold { inject_plan: String, hold_count: u8 },
    Park { reason: String },
}

/// Lane R20b: which broad family a `Park` (or any other free-text `unblock`
/// decision's own detail) reason belongs to. `decide_goal_hold` itself only
/// ever produces [`ParkReason::VerifyGap`]/[`ParkReason::UncommittedWork`]
/// (the two [`PARK_REASON_NO_VERIFY`]/[`PARK_REASON_UNCOMMITTED`] constants
/// above) — the other variants exist so ANY reason text, including one
/// raised by a wholly different producer, can be classified the same honest
/// way instead of an ad hoc byte comparison. A20's Q1 analysis
/// (HARNESS_LOOP_LOG.md "## A20 analysis — M19 zai z2") found a SECOND
/// producer this module never sees at all: a coordinator's own free-text
/// `unblock` decision whose detail described the runaway-loop guard
/// blocking a repeated `goal_status` poll — [`ParkReason::PollLoop`] names
/// that shape so any future caller that classifies an arbitrary park/unblock
/// reason (not just the two literals this module raises) has one honest
/// place to do it. `orchestrator::maybe_headless_unblock_verify_gap` is the
/// one caller today: it auto-answers `VerifyGap`/`UncommittedWork` only —
/// every other variant, `PollLoop` included, always stays parked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkReason {
    /// [`PARK_REASON_NO_VERIFY`] — held twice with no successful verify.
    VerifyGap,
    /// [`PARK_REASON_UNCOMMITTED`] — verified but never committed.
    UncommittedWork,
    /// A wall/wallclock-budget refusal
    /// (`orchestrator::wall_budget_exhausted_refusal`'s own reason text —
    /// "wall budget exhausted: …", "wallclock cap exhausted …").
    WallBudget,
    /// A20's second producer: a coordinator's own free-text `unblock` about
    /// the runaway-loop guard blocking a repeated status poll.
    PollLoop,
    /// A jail, approval, credential, or other safety refusal.
    Safety,
    /// Anything else — an unrecognized reason is never auto-answered.
    Other,
}

/// Pure classifier for [`ParkReason`]. Exact-matches the two constants this
/// module produces first (the only two `decide_goal_hold` can ever raise),
/// then falls back to keyword heuristics for reasons raised by OTHER
/// producers — never silently drops back to a byte comparison a caller has
/// to duplicate.
pub fn classify_park_reason(reason: &str) -> ParkReason {
    if reason == PARK_REASON_NO_VERIFY {
        return ParkReason::VerifyGap;
    }
    if reason == PARK_REASON_UNCOMMITTED {
        return ParkReason::UncommittedWork;
    }
    let lower = reason.to_ascii_lowercase();
    if lower.contains("wall budget") || lower.contains("wallclock") {
        return ParkReason::WallBudget;
    }
    if lower.contains("runaway-loop") || lower.contains("goal_status") {
        return ParkReason::PollLoop;
    }
    if lower.contains("jail")
        || lower.contains("approval")
        || lower.contains("credential")
        || lower.contains("safety")
    {
        return ParkReason::Safety;
    }
    ParkReason::Other
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HoldState {
    pub count: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_plan: Option<String>,
}

impl HoldState {
    pub fn from_metadata(meta: &serde_json::Map<String, serde_json::Value>) -> Self {
        meta.get(HOLD_METADATA_KEY)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    }

    pub fn write_into(&self, meta: &mut serde_json::Map<String, serde_json::Value>) {
        if let Ok(v) = serde_json::to_value(self) {
            meta.insert(HOLD_METADATA_KEY.to_string(), v);
        }
    }
}

/// Pure: should this worker's "done" be accepted, held, or parked?
///
/// - Any role after a successful verify → Allow (goal callers also require committed work).
/// - Any role without one → Hold (or Park past [`MAX_HOLDS`]).
/// - The one exception to the receipt: a hands-on role that verified green and
///   then went on repeating a failing command still holds.
pub fn decide_hold(
    role: WorkflowRole,
    verify_ran: bool,
    signals: &ToolTranscriptSignals,
    prior_holds: u8,
) -> HoldOutcome {
    decide_goal_hold(role, verify_ran, false, signals, prior_holds)
}

/// Goal workers must also leave committed work past their dispatch baseline.
/// Ordinary after-turn callers have no goal branch and use [`decide_hold`].
pub fn decide_goal_hold(
    role: WorkflowRole,
    verify_ran: bool,
    uncommitted_work: bool,
    signals: &ToolTranscriptSignals,
    prior_holds: u8,
) -> HoldOutcome {
    // `verify_ran` means a SUCCESSFUL verify AFTER the latest mutation — the
    // caller computes positions, not mere historical presence. That receipt
    // leads: transcript text may not talk a green run into a failure, which is
    // exactly what used to happen when a passing suite printed `0 failed`
    // (see `tool_signals::extract`, now keyed off the wire's `is_error`).
    //
    // It is still not a blank cheque. A hands-on worker that verified and then
    // kept re-running one failing command has not finished, and that residual
    // spin is the only thing left that can outrank the receipt.
    let hands_on = matches!(role, WorkflowRole::Mechanical | WorkflowRole::Local);
    if verify_ran && !uncommitted_work && !(hands_on && signals.spinning >= 0.5) {
        return HoldOutcome::Allow;
    }

    let next = prior_holds.saturating_add(1);
    if next > MAX_HOLDS {
        return HoldOutcome::Park {
            reason: if verify_ran && uncommitted_work {
                PARK_REASON_UNCOMMITTED.into()
            } else {
                PARK_REASON_NO_VERIFY.into()
            },
        };
    }

    let inject_plan = if verify_ran && uncommitted_work {
        "verified but uncommitted: commit your work on the goal branch, then declare done".into()
    } else if verify_ran {
        // Never "verify is still failing" — verify passed. Name what is.
        "Verify passed, but you are still repeating a command that keeps failing. Stop retrying it; change the approach, then verify once.".into()
    } else {
        "Do not declare this done. Run verify, read the errors, make one focused fix, and run verify again.".into()
    };
    HoldOutcome::Hold {
        inject_plan,
        hold_count: next,
    }
}

// ── Lane R21a (A21 Q1): read-only goals reach review with a verified
// no-change report instead of a commit ─────────────────────────────────────
//
// The held-out fixture `select-review-request-does-not-mutate` asks the
// harness to REVIEW code and report, without changing anything. The worker
// correctly makes no edits and reports its findings — but `decide_goal_hold`
// only ever recognizes "verified and committed" as a legitimate `done`, so a
// correct read-only outcome held forever and the goal never reached review.
//
// The fix is honest, not permissive: a goal only ever takes this branch when
// BOTH (a) its own card metadata says no code change was expected
// (`expects_change: false`, set once by the planner at roadmap-creation time
// from its acceptance criteria — never re-derived from, or overridable by,
// anything the worker itself says) AND (b) the worktree the worker actually
// touched is genuinely clean: zero commits past baseline and no tracked
// modifications. A read-only goal whose worker DID mutate files is not
// read-only for hold purposes — [`decide_goal_done_hold`] falls straight
// back to the ordinary commit-required [`decide_goal_hold`], unchanged.

/// Metadata key on the goal card (written once, at roadmap-creation time, by
/// `materialize_roadmap_transaction`'s call to [`infer_expects_change`]).
/// Absent on every goal minted before this lane and on every non-roadmap
/// creation path — [`goal_expects_change`] treats absence as `true`, so
/// those goals keep today's commit-required behavior exactly as before.
pub const EXPECTS_CHANGE_METADATA_KEY: &str = "expects_change";

/// Read a goal card's `expects_change` intent. Defaults to `true` (a normal,
/// mutating goal) whenever the key is missing or not a bool — an explicit
/// `false` is the only thing that ever opts a goal out of the commit-required
/// rule, so a card this lane never touched behaves exactly as it always has.
pub fn goal_expects_change(meta: &serde_json::Map<String, serde_json::Value>) -> bool {
    meta.get(EXPECTS_CHANGE_METADATA_KEY)
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// Keywords whose presence in a goal's acceptance criteria (or description)
/// mean a code change IS expected, however review-shaped the rest of the
/// text reads — these always win over a read-only-sounding phrase in the
/// same goal, because a mixed "review this AND fix it" goal is not
/// read-only.
///
/// Lane R29b (A28 Miss 3 repair): `"write "` used to be in this list, but
/// it is genuinely ambiguous — "write a report" is exactly as common an
/// English phrase as "write the fix," and the former is squarely read-only.
/// Since [`matches_word_boundary`] now matches `"write"` as a real whole
/// word (not just a substring), keeping it here would make "write a
/// report" register as mutation-intent solely because it shares a verb with
/// "write the migration script" — the same class of false positive this
/// lane exists to remove. Dropped rather than kept.
const CHANGE_KEYWORDS: &[&str] = &[
    "fix",
    "implement",
    "add ",
    "change",
    "modify",
    "refactor",
    "update",
    "create",
    "remove",
    "delete",
    "patch",
    "commit",
    "rename",
    "migrate",
    "upgrade",
    "install",
    "configure",
    "build ",
    "generate",
];

/// Keywords whose presence signals a read-only, report-shaped goal — only
/// consulted when nothing in [`CHANGE_KEYWORDS`] matched first.
const READ_ONLY_KEYWORDS: &[&str] = &[
    "review",
    "report",
    "analy", // analyze / analysis / analyse
    "audit",
    "evaluate",
    "assess",
    "investigate",
    "examine",
    "inspect",
    "summariz",
    "summaris",
    "no code change",
    "no change",
    "without changing",
    "without mutating",
    "does not mutate",
    "read-only",
    "read only",
];

/// [`READ_ONLY_KEYWORDS`] entries that are deliberate word STEMS (a prefix
/// of several inflections — "analy" for analyze/analysis/analyse,
/// "summariz"/"summaris" for summarize/summarise/summarizing/…) rather than
/// complete English words. [`matches_word_boundary`] matches these as a
/// prefix of a whole haystack word; every other keyword must equal a whole
/// word exactly. This is the one place the A28 Miss 3 fix stays a PREFIX
/// match rather than tightening to exact equality — unlike `"report"`
/// (which must never match inside `"reports.py"` / `"reporting"`), these
/// stems have no bare dictionary form of their own to be exact about.
const STEM_KEYWORDS: &[&str] = &["analy", "summariz", "summaris"];

/// Tokenize on non-alphanumeric boundaries — the same char-based scan named
/// in this lane's own instructions, never a byte-index slice. Used for both
/// the haystack and each keyword so a keyword like `"add "` (kept for
/// readability elsewhere) and one like `"add"` tokenize identically.
fn word_tokens(text: &str) -> Vec<&str> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect()
}

/// True when `keyword` matches `haystack_words` at a whole-word boundary —
/// never as a fragment of a longer word, so `"report"` does not match
/// `"reports.py"`, `"reporting"`, or `"reports"`. A multi-word keyword
/// (e.g. `"no code change"`) matches as a contiguous run of whole words. A
/// [`STEM_KEYWORDS`] entry matches as a prefix of a single haystack word
/// instead (see its doc comment).
fn matches_word_boundary(haystack_words: &[&str], keyword: &str) -> bool {
    let kw_words = word_tokens(keyword);
    if kw_words.is_empty() {
        return false;
    }
    if kw_words.len() == 1 {
        let kw = kw_words[0];
        return if STEM_KEYWORDS.contains(&kw) {
            haystack_words.iter().any(|w| w.starts_with(kw))
        } else {
            haystack_words.iter().any(|&w| w == kw)
        };
    }
    kw_words.len() <= haystack_words.len()
        && haystack_words
            .windows(kw_words.len())
            .any(|w| w == kw_words.as_slice())
}

/// Lane R21a: the ONLY place `expects_change` is ever derived — called once,
/// at roadmap-creation time (`materialize_roadmap_transaction`), from the
/// planner's own acceptance criteria plus the goal description (the planner
/// already writes ACs; this never re-runs later and a worker's own claims
/// can never influence it). Conservative by construction: any change-shaped
/// keyword anywhere in the text wins outright; only text that reads as pure
/// review/report/analysis with no change-shaped keyword at all comes back
/// `false`. Empty/ambiguous input defaults `true` — silence is never treated
/// as a licence to skip the commit-required rule.
///
/// Lane R29b (A28 Miss 3): matching is whole-word (via
/// [`matches_word_boundary`]), not a raw substring test — a filename or
/// identifier mentioned in the text (`"reports.py"`) can no longer flip a
/// genuine fix-and-verify goal to read-only just because a keyword happens
/// to be one of its letter-runs. [`CHANGE_KEYWORDS`] is still checked
/// first, so explicit mutation intent ("fix", "implement", "change",
/// "update", "add", "remove", …) always outranks a read-only-sounding word
/// in the same text — that precedence already existed here and is
/// unchanged by this lane.
pub fn infer_expects_change(acceptance_criteria: &[String], description: &str) -> bool {
    let mut haystack = description.to_ascii_lowercase();
    for ac in acceptance_criteria {
        haystack.push(' ');
        haystack.push_str(&ac.to_ascii_lowercase());
    }
    if haystack.trim().is_empty() {
        return true;
    }
    let haystack_words = word_tokens(&haystack);
    if CHANGE_KEYWORDS
        .iter()
        .any(|kw| matches_word_boundary(&haystack_words, kw))
    {
        return true;
    }
    !READ_ONLY_KEYWORDS
        .iter()
        .any(|kw| matches_word_boundary(&haystack_words, kw))
}

/// How long a worker's final report text must be, trimmed, to count as a
/// genuine review artifact rather than a bare "done" claim. Deliberately
/// low and generous — the reviewer, not this gate, judges whether the
/// report is any good; this only asks whether the worker left SOMETHING to
/// review at all.
pub const MIN_REVIEW_REPORT_CHARS: usize = 40;

/// Does this text read like a real review/analysis report? See
/// [`MIN_REVIEW_REPORT_CHARS`] for the (generous) bar.
pub fn looks_like_review_report(text: &str) -> bool {
    text.trim().chars().count() >= MIN_REVIEW_REPORT_CHARS
}

/// The read-only counterpart of [`decide_goal_hold`]: a review artifact (the
/// worker's own report text, a `review_report` tool result, or an equivalent
/// findings message — anything [`looks_like_review_report`] recognizes)
/// stands in for the commit+verify receipt. Never called directly by a goal
/// caller — [`decide_goal_done_hold`] is the one dispatcher that decides
/// whether a goal is eligible for this rule at all.
pub fn decide_read_only_goal_hold(
    role: WorkflowRole,
    has_review_report: bool,
    signals: &ToolTranscriptSignals,
    prior_holds: u8,
) -> HoldOutcome {
    let hands_on = matches!(role, WorkflowRole::Mechanical | WorkflowRole::Local);
    if has_review_report && !(hands_on && signals.spinning >= 0.5) {
        return HoldOutcome::Allow;
    }

    let next = prior_holds.saturating_add(1);
    if next > MAX_HOLDS {
        return HoldOutcome::Park {
            reason: PARK_REASON_NO_REVIEW_REPORT.into(),
        };
    }

    let inject_plan = if has_review_report {
        // Same shape as decide_goal_hold's spin brake: the report exists,
        // but a hands-on role kept repeating a failing command afterward.
        "You left a review report, but you are still repeating a command that keeps failing. \
         Stop retrying it; your review is the deliverable here, not a passing command."
            .to_string()
    } else {
        "This goal is read-only — no code change is expected. Do not declare this done without \
         leaving a clear review report: what you checked and what you found. Do not make any \
         edits; if you find the goal actually requires a change, say so in the report instead of \
         making it."
            .to_string()
    };
    HoldOutcome::Hold {
        inject_plan,
        hold_count: next,
    }
}

/// Route a goal `done` claim to the read-only rule
/// ([`decide_read_only_goal_hold`]) when the goal's own metadata says no code
/// change was expected AND the worktree genuinely carries none — otherwise
/// fall back to the ordinary commit-required [`decide_goal_hold`]. A
/// read-only goal that DID mutate files (`worktree_mutated`) is not
/// read-only for hold purposes: normal commit/verify rules apply, exactly as
/// if `expects_change` were absent. This is the one function a goal `done`
/// claim caller (`maybe_hold_review`) should call.
#[allow(clippy::too_many_arguments)]
pub fn decide_goal_done_hold(
    role: WorkflowRole,
    expects_change: bool,
    worktree_mutated: bool,
    verify_ran: bool,
    uncommitted_work: bool,
    has_review_report: bool,
    signals: &ToolTranscriptSignals,
    prior_holds: u8,
) -> HoldOutcome {
    if !expects_change && !worktree_mutated {
        return decide_read_only_goal_hold(role, has_review_report, signals, prior_holds);
    }
    decide_goal_hold(role, verify_ran, uncommitted_work, signals, prior_holds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet() -> ToolTranscriptSignals {
        ToolTranscriptSignals::default()
    }

    #[test]
    fn mechanical_without_verify_holds() {
        let out = decide_hold(WorkflowRole::Mechanical, false, &quiet(), 0);
        match out {
            HoldOutcome::Hold {
                hold_count,
                inject_plan,
            } => {
                assert_eq!(hold_count, 1);
                assert!(inject_plan.contains("verify"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn local_without_verify_holds() {
        assert!(matches!(
            decide_hold(WorkflowRole::Local, false, &quiet(), 0),
            HoldOutcome::Hold { .. }
        ));
    }

    #[test]
    fn orchestrate_after_verify_allows() {
        assert_eq!(
            decide_hold(WorkflowRole::Orchestrate, true, &quiet(), 0),
            HoldOutcome::Allow
        );
    }

    #[test]
    fn mechanical_after_clean_verify_allows() {
        assert_eq!(
            decide_hold(WorkflowRole::Mechanical, true, &quiet(), 0),
            HoldOutcome::Allow
        );
    }

    #[test]
    fn verified_but_uncommitted_hold_uses_the_existing_cap() {
        for prior in 0..MAX_HOLDS {
            assert_eq!(
                decide_goal_hold(WorkflowRole::Mechanical, true, true, &quiet(), prior),
                HoldOutcome::Hold {
                    inject_plan: "verified but uncommitted: commit your work on the goal branch, then declare done".into(),
                    hold_count: prior + 1,
                }
            );
        }
        match decide_goal_hold(WorkflowRole::Mechanical, true, true, &quiet(), MAX_HOLDS) {
            HoldOutcome::Park { reason } => assert!(reason.contains("uncommitted")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn repeated_hold_parks() {
        let out = decide_hold(WorkflowRole::Mechanical, false, &quiet(), MAX_HOLDS);
        assert!(matches!(out, HoldOutcome::Park { .. }));
    }

    #[test]
    fn spinning_after_verify_still_holds_mechanical() {
        let spinning = ToolTranscriptSignals {
            spinning: 0.7,
            ..Default::default()
        };
        match decide_hold(WorkflowRole::Mechanical, true, &spinning, 0) {
            HoldOutcome::Hold { inject_plan, .. } => {
                // The bug this replaces: a hold raised AFTER a green verify used
                // to tell the model "verify is still failing the same way".
                assert!(
                    !inject_plan.contains("Verify is still failing"),
                    "never contradict a green verify: {inject_plan}"
                );
                assert!(inject_plan.contains("Verify passed"));
            }
            other => panic!("{other:?}"),
        }
    }

    /// Severity alone never outranks the receipt — a hard error EARLIER in the
    /// run that a later green verify resolved is history, not a reason to hold.
    #[test]
    fn severity_after_verify_does_not_hold() {
        let severe = ToolTranscriptSignals {
            severity: 1.0,
            ..Default::default()
        };
        assert_eq!(
            decide_hold(WorkflowRole::Mechanical, true, &severe, 0),
            HoldOutcome::Allow
        );
    }

    /// The spin brake is scoped to hands-on roles, as it always was: a judgment
    /// role's receipt is terminal.
    #[test]
    fn spinning_after_verify_allows_a_judgment_role() {
        let spinning = ToolTranscriptSignals {
            spinning: 0.7,
            ..Default::default()
        };
        assert_eq!(
            decide_hold(WorkflowRole::Review, true, &spinning, 0),
            HoldOutcome::Allow
        );
    }

    /// Without a verify the plan must ask for one, whatever the signals say.
    #[test]
    fn the_no_verify_plan_asks_for_a_verify() {
        let spinning = ToolTranscriptSignals {
            spinning: 0.7,
            ..Default::default()
        };
        match decide_hold(WorkflowRole::Mechanical, false, &spinning, 0) {
            HoldOutcome::Hold { inject_plan, .. } => {
                assert!(inject_plan.starts_with("Do not declare this done."));
            }
            other => panic!("{other:?}"),
        }
    }

    // ── Lane R20b: ParkReason classification ────────────────────────────

    #[test]
    fn classify_park_reason_recognizes_verify_gap() {
        assert_eq!(
            classify_park_reason(PARK_REASON_NO_VERIFY),
            ParkReason::VerifyGap
        );
    }

    #[test]
    fn classify_park_reason_recognizes_uncommitted_work() {
        assert_eq!(
            classify_park_reason(PARK_REASON_UNCOMMITTED),
            ParkReason::UncommittedWork
        );
    }

    #[test]
    fn classify_park_reason_recognizes_wall_budget() {
        assert_eq!(
            classify_park_reason(
                "wall budget exhausted: 12s remaining, the last attempt took 100s"
            ),
            ParkReason::WallBudget
        );
        assert_eq!(
            classify_park_reason("wallclock cap exhausted (900s elapsed of 900s)"),
            ParkReason::WallBudget
        );
    }

    /// A20's second producer: the coordinator's own free-text unblock about
    /// the runaway-loop guard blocking a repeated `goal_status` poll.
    #[test]
    fn classify_park_reason_recognizes_poll_loop() {
        assert_eq!(
            classify_park_reason(
                "I cannot poll goal_status again without triggering the runaway-loop guard"
            ),
            ParkReason::PollLoop
        );
    }

    #[test]
    fn classify_park_reason_recognizes_safety() {
        assert_eq!(
            classify_park_reason("blocked by the write jail: path escapes the sandbox"),
            ParkReason::Safety
        );
        assert_eq!(
            classify_park_reason("needs verification_approval before this command may run"),
            ParkReason::Safety
        );
        assert_eq!(
            classify_park_reason("refused: a credential assignment was detected"),
            ParkReason::Safety
        );
    }

    #[test]
    fn classify_park_reason_falls_back_to_other() {
        assert_eq!(
            classify_park_reason("some brand new reason no producer has used before"),
            ParkReason::Other
        );
    }

    // ── Lane R21a: read-only goals reach review with a verified no-change
    // report instead of a commit ────────────────────────────────────────────

    fn report(chars: usize) -> String {
        "x".repeat(chars)
    }

    #[test]
    fn goal_expects_change_defaults_true_when_absent() {
        let meta = serde_json::Map::new();
        assert!(goal_expects_change(&meta));
    }

    #[test]
    fn goal_expects_change_reads_an_explicit_false() {
        let mut meta = serde_json::Map::new();
        meta.insert(
            EXPECTS_CHANGE_METADATA_KEY.to_string(),
            serde_json::json!(false),
        );
        assert!(!goal_expects_change(&meta));
    }

    #[test]
    fn infer_expects_change_is_false_for_a_pure_review_goal() {
        assert!(!infer_expects_change(
            &["Review the billing module and report findings; do not mutate anything".into()],
            "select-review-request-does-not-mutate",
        ));
    }

    #[test]
    fn infer_expects_change_defaults_true_on_empty_input() {
        assert!(infer_expects_change(&[], ""));
    }

    #[test]
    fn infer_expects_change_is_true_when_a_change_keyword_is_present() {
        // "review and fix" is not read-only, even though "review" appears.
        assert!(infer_expects_change(
            &["Review the auth flow and fix the token refresh bug".into()],
            "",
        ));
    }

    #[test]
    fn infer_expects_change_is_true_for_an_ordinary_implementation_goal() {
        assert!(infer_expects_change(
            &["The new endpoint returns 200 for a valid request".into()],
            "Implement the /health endpoint",
        ));
    }

    // ── Lane R29b (A28 Miss 3): whole-word matching, not raw substring ──────

    /// The exact A28 miss: a change-shaped goal whose description names a
    /// consumer filename (`reports.py`) must not be flipped read-only by
    /// `"report"` matching inside that filename — and here "fix" is also
    /// present, so `CHANGE_KEYWORDS`' pre-existing precedence decides it
    /// outright regardless.
    #[test]
    fn infer_expects_change_true_for_fix_with_a_reports_py_filename() {
        assert!(infer_expects_change(
            &[],
            "Fix the failing tests in cli.py/reports.py",
        ));
    }

    /// A genuinely read-only goal ("write a report") must still read
    /// read-only — `"write"` no longer sits in `CHANGE_KEYWORDS` (it was
    /// exactly as ambiguous as the `"report"`/`"reports.py"` bug this lane
    /// fixes), so the only whole-word keyword left in this text is the
    /// read-only `"report"`.
    #[test]
    fn infer_expects_change_false_for_write_a_report() {
        assert!(!infer_expects_change(&[], "Write a report on the module"));
    }

    /// `"report"` must never match as a fragment of `"reports"` — this is
    /// the literal A28 Miss 3 substring: with no other change or read-only
    /// signal in the text, the filename alone must not flip the default.
    #[test]
    fn infer_expects_change_reports_py_filename_alone_does_not_match_report() {
        assert!(infer_expects_change(&[], "reports.py"));
    }

    /// `"report"` must also never match inside `"reporting"` — a different
    /// word entirely, not a filename this time.
    #[test]
    fn infer_expects_change_reporting_does_not_match_report() {
        assert!(infer_expects_change(
            &[],
            "The ingest pipeline needs continuous reporting visibility",
        ));
    }

    /// The real A28 Miss 3 fixture, reconstructed: prose that is genuinely
    /// fix-shaped but — as the actual goal card did — avoids every literal
    /// `CHANGE_KEYWORDS` verb, while naming a `reports.py` consumer module
    /// in a scope-preservation clause. With word-boundary matching, no
    /// keyword in either list matches at all, so the function falls back to
    /// its documented default: silence is never a licence to skip the
    /// commit-required rule.
    #[test]
    fn infer_expects_change_reconstructed_a28_miss3_fixture_defaults_true() {
        assert!(infer_expects_change(
            &["Preserve existing call sites in cli.py/reports.py".into()],
            "Correct the two underlying bugs; must raise ValueError and must return min(...)",
        ));
    }

    /// The stem keywords (`"analy"`, `"summariz"`, `"summaris"`) are
    /// deliberately partial — they must keep matching their full
    /// inflections (unlike `"report"`, which must not).
    #[test]
    fn infer_expects_change_stem_keywords_still_match_their_inflections() {
        assert!(!infer_expects_change(&[], "Analyze the query planner"));
        assert!(!infer_expects_change(
            &[],
            "Summarize the incident timeline"
        ));
    }

    #[test]
    fn looks_like_review_report_rejects_a_bare_done_claim() {
        assert!(!looks_like_review_report(""));
        assert!(!looks_like_review_report("done"));
    }

    #[test]
    fn looks_like_review_report_accepts_real_findings() {
        assert!(looks_like_review_report(&report(MIN_REVIEW_REPORT_CHARS)));
    }

    #[test]
    fn read_only_goal_with_a_report_reaches_review() {
        assert_eq!(
            decide_goal_done_hold(
                WorkflowRole::Mechanical,
                /* expects_change */ false,
                /* worktree_mutated */ false,
                /* verify_ran */ false,
                /* uncommitted_work */ false,
                /* has_review_report */ true,
                &quiet(),
                0,
            ),
            HoldOutcome::Allow
        );
    }

    #[test]
    fn read_only_goal_without_a_report_holds_then_parks() {
        for prior in 0..MAX_HOLDS {
            let out = decide_goal_done_hold(
                WorkflowRole::Mechanical,
                false,
                false,
                false,
                false,
                false,
                &quiet(),
                prior,
            );
            match out {
                HoldOutcome::Hold { hold_count, .. } => assert_eq!(hold_count, prior + 1),
                other => panic!("{other:?}"),
            }
        }
        match decide_goal_done_hold(
            WorkflowRole::Mechanical,
            false,
            false,
            false,
            false,
            false,
            &quiet(),
            MAX_HOLDS,
        ) {
            HoldOutcome::Park { reason } => assert_eq!(reason, PARK_REASON_NO_REVIEW_REPORT),
            other => panic!("{other:?}"),
        }
    }

    /// A read-only goal whose worker DID mutate files falls back to the
    /// ordinary commit-required rule — a report is not enough once the
    /// worktree is no longer clean.
    #[test]
    fn read_only_goal_that_mutated_falls_back_to_commit_required() {
        let out = decide_goal_done_hold(
            WorkflowRole::Mechanical,
            /* expects_change */ false,
            /* worktree_mutated */ true,
            /* verify_ran */ false,
            /* uncommitted_work */ false,
            /* has_review_report */ true,
            &quiet(),
            0,
        );
        match out {
            HoldOutcome::Hold { inject_plan, .. } => {
                assert!(inject_plan.starts_with("Do not declare this done."));
            }
            other => panic!("{other:?}"),
        }
    }

    /// A normal (mutating) goal with no commit still holds, unchanged by any
    /// of the read-only machinery above.
    #[test]
    fn a_mutating_goal_with_no_commit_still_holds() {
        let out = decide_goal_done_hold(
            WorkflowRole::Mechanical,
            true,
            true,
            false,
            false,
            false,
            &quiet(),
            0,
        );
        assert!(matches!(out, HoldOutcome::Hold { .. }));
    }

    #[test]
    fn read_only_hold_plan_never_asks_for_a_verify() {
        match decide_read_only_goal_hold(WorkflowRole::Mechanical, false, &quiet(), 0) {
            HoldOutcome::Hold { inject_plan, .. } => {
                assert!(!inject_plan.to_ascii_lowercase().contains("run verify"));
                assert!(inject_plan.contains("review report"));
            }
            other => panic!("{other:?}"),
        }
    }
}
