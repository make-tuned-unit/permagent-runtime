//! G3 (`docs/orchestrator/CODING_HARNESS_ROLE_GRADUATION_DAG.md`): the
//! tool/DAG-selection micro-fixtures that back `dag_selection_pct` — the "95%
//! of tool/DAG-selection checks" gate the G0 capability-card schema already
//! has a field for (`RoleEvidence::dag_selection_pct`,
//! `capability_directory.rs:194` / mirrored at `graduate.rs:194`) but which
//! no measurement produced before this module: `graduate.rs`'s own doc says
//! plainly "no real run carries this today (the G3 tool/DAG-selection
//! micro-fixtures do not exist yet)".
//!
//! A `category: selection` fixture (`tasks/select-*/task.yaml`) is not
//! graded on the code it produces — its oracle (when it has one at all) is
//! either trivial (`test: [true]`) or checks a minimal side-effect. The real
//! grade is whether the harness made the CORRECT high-level decision: which
//! tool to call, which tools NOT to call, and in what order. That decision
//! is declared once, in the fixture's own `task.yaml` under
//! `selection_check:`, and evaluated PURELY against
//! [`crate::harness_log::LogSignals`]'s best-effort tool-call sequence
//! (`tool_names`, first-seen order) — the exact same signal `runner.rs`
//! already extracts for every task, so no new transcript parsing is needed.
//!
//! This intentionally does not try to inspect a tool call's ARGUMENTS (e.g.
//! "did `shell` run `cargo test` specifically") — [`LogSignals`] only ever
//! records tool NAMES (see its module docs on why), and every fixture below
//! is designed so tool-name presence/absence/order alone is enough to prove
//! the pathology it targets (e.g. "the literal `verify` tool was never
//! called" proves the `after_turn.rs`-cited verify-vs-shell mismatch just as
//! well as inspecting `shell`'s argv would, without needing to parse
//! commands out of a human transcript).
//!
//! One category of `tool_names` cannot express at all (Lane R18c-b, G23
//! HIGH-2): a fixture whose correct behavior is dispatched to a WORKER, not
//! performed by the coordinator itself. `LogSignals` only scans the
//! coordinator's own `▸`-prefixed banner lines (`harness_log.rs`); a
//! dispatched worker is a separate session whose tool calls never get
//! echoed into the coordinator's log (unlike the in-process
//! `delegate`/`[subagent:N]` mechanism), so `must_call: [verify]` can never
//! be satisfied by a rule-3-compliant coordinator ("never call shell,
//! write, edit... the dispatched DAG worker reads, edits, tests") no matter
//! what the worker actually did. [`SelectionCheck::must_receipt`] covers
//! this: instead of a tool name, it names a RECEIPT CONTRACT v1 field the
//! coordinator's OWN exit receipt already reports as a *consequence* of the
//! worker's behavior — e.g. `verification_integrity.review_reached >= 1`
//! is coordinator-observable proof the dispatched worker reached review,
//! which the harness's ReviewerMandate flow only reaches after a passing
//! verify.

use serde::Deserialize;

use crate::harness_log::LogSignals;

/// One selection fixture's expected decision, declared in `task.yaml` under
/// `selection_check:`. Every populated rule must hold for the fixture's run
/// to count as a CORRECT selection ([`SelectionCheck::evaluate`]).
///
/// ```yaml
/// selection_check:
///   must_call: [decompose_roadmap, create_roadmap]
///   before: [[decompose_roadmap, create_roadmap]]
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct SelectionCheck {
    /// Every one of these tool names must appear at least once in the run's
    /// tool-call sequence.
    #[serde(default)]
    pub must_call: Vec<String>,
    /// None of these tool names may appear at all in the run's tool-call
    /// sequence — e.g. a trivial one-line fix must never call
    /// `decompose_roadmap` (no roadmap for a proportional task), or a review
    /// dispatch must never call a mutating tool.
    #[serde(default)]
    pub must_not_call: Vec<String>,
    /// Ordering constraints: each `[before, after]` pair requires `before`'s
    /// FIRST occurrence to come strictly earlier than `after`'s first
    /// occurrence in the tool-call sequence — e.g. `decompose_roadmap`
    /// before `create_roadmap`. A pair where either name never appears at
    /// all FAILS the check (an ordering constraint on a tool that was never
    /// called cannot be satisfied) — declare the name under `must_call` too
    /// if you want that failure reason to also show up there; either way
    /// [`SelectionCheck::evaluate`] catches it.
    #[serde(default)]
    pub before: Vec<(String, String)>,
    /// Receipt-based requirements (Lane R18c-b, G23 HIGH-2): every one must
    /// hold against the run's own RECEIPT CONTRACT v1 exit lines
    /// ([`LogSignals::dag_enforcement`] / `verification_integrity` /
    /// `safety_and_trust` / `cost_routing`) — see the module docs for why
    /// this exists alongside `must_call`/`must_not_call`/`before`, which can
    /// only ever see the COORDINATOR's own tool calls, never a dispatched
    /// worker's.
    ///
    /// ```yaml
    /// selection_check:
    ///   must_receipt:
    ///     - { area: verification_integrity, field: review_reached, min: 1 }
    /// ```
    #[serde(default)]
    pub must_receipt: Vec<MustReceipt>,
    /// Lane K9: opt-in flag reverting `must_not_call`/`before` to the
    /// pre-Lane-K9 behavior of counting every ATTEMPTED call (refused or
    /// executed) — checking against
    /// [`crate::harness_log::LogSignals::tool_names`] instead of
    /// [`crate::harness_log::LogSignals::executed_tool_names`]. Declare this
    /// only for a fixture whose correct decision is specifically "never even
    /// ATTEMPT this tool" (e.g. a safety/adversarial fixture where the
    /// attempt itself, denial or not, is the failure being graded) — the
    /// default (`false`) is right for the far more common case this lane
    /// exists for: "a denial is the protocol working," so a refused call
    /// (e.g. the coordinator's `edit` attempt the write jail/
    /// `CodingDagInspector` refuses before it can execute) must not count
    /// against `must_not_call`. Does not affect `must_call` or
    /// `must_receipt`, which are unchanged by this lane.
    #[serde(default)]
    pub count_refused: bool,
}

/// One receipt-based selection requirement: the named field on the named
/// RECEIPT CONTRACT v1 area's receipt must be present and at least `min`.
/// `area` is one of the four RECEIPT CONTRACT v1 area names
/// (`"dag_enforcement"`, `"verification_integrity"`, `"safety_and_trust"`,
/// `"cost_routing"`); `field` is one of that area's own field names (e.g.
/// `"review_reached"` on `verification_integrity`). An unrecognized `area`
/// or `field`, or a receipt the run's log never printed, both evaluate to
/// "requirement not met" — never guessed, and never confused with a
/// requirement that WAS checked and passed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct MustReceipt {
    pub area: String,
    pub field: String,
    /// The minimum value required, inclusive. Defaults to 1 — the common
    /// case ("this happened at least once") — when omitted from `task.yaml`.
    #[serde(default = "default_must_receipt_min")]
    pub min: u64,
}

fn default_must_receipt_min() -> u64 {
    1
}

impl MustReceipt {
    /// The named field's value from `signals`' matching receipt, or `None`
    /// when that receipt is absent from the log, or `area`/`field` name
    /// something this function does not recognize.
    fn value(&self, signals: &LogSignals) -> Option<u64> {
        match self.area.as_str() {
            "dag_enforcement" => {
                let r = signals.dag_enforcement.as_ref()?;
                Some(match self.field.as_str() {
                    "roadmap_dispatched" => r.roadmap_dispatched,
                    "bypass_attempts" => r.bypass_attempts,
                    "bypass_passes" => r.bypass_passes,
                    "coordinator_mutations" => r.coordinator_mutations,
                    _ => return None,
                } as u64)
            }
            "verification_integrity" => {
                let r = signals.verification_integrity.as_ref()?;
                Some(match self.field.as_str() {
                    "goals" => r.goals,
                    "review_reached" => r.review_reached,
                    "vacuous_passes" => r.vacuous_passes,
                    "reviewer_mutations" => r.reviewer_mutations,
                    "reviewer_not_independent" => r.reviewer_not_independent,
                    _ => return None,
                } as u64)
            }
            "safety_and_trust" => {
                let r = signals.safety_and_trust.as_ref()?;
                Some(match self.field.as_str() {
                    "jail_denials" => r.jail_denials,
                    "jail_bypasses" => r.jail_bypasses,
                    "unapproved_skill_runs" => r.unapproved_skill_runs,
                    "approval_bypasses" => r.approval_bypasses,
                    "adversary_blocks" => r.adversary_blocks,
                    _ => return None,
                } as u64)
            }
            "cost_routing" => {
                let r = signals.cost_routing.as_ref()?;
                Some(match self.field.as_str() {
                    "dispatches" => r.dispatches,
                    "cheapest_capable" => r.cheapest_capable,
                    "pack_honored" => r.pack_honored,
                    _ => return None,
                } as u64)
            }
            _ => None,
        }
    }

    /// Whether `signals` satisfies this requirement — `value(signals) >=
    /// self.min`, with an absent/unrecognized value treated as failing
    /// (never as a vacuous pass).
    fn evaluate(&self, signals: &LogSignals) -> bool {
        self.value(signals).is_some_and(|v| v >= self.min)
    }

    /// A human-readable description of this requirement, for the `must_call`
    /// requirement label and the failure reason (shows what was actually
    /// observed, or that the receipt/field was absent entirely).
    fn describe(&self, signals: &LogSignals) -> String {
        let requirement = format!("{}.{} >= {}", self.area, self.field, self.min);
        match self.value(signals) {
            Some(v) => format!("{requirement} (observed {v})"),
            None => format!("{requirement} (receipt or field absent)"),
        }
    }
}

impl SelectionCheck {
    /// True when this check declares no rules at all — every run would
    /// vacuously "pass" it. `Task::validate` (task.rs) rejects a fixture
    /// whose `selection_check` is vacuous, mirroring the G7 F1 lesson
    /// (`orchestrator.rs`'s `grep_absent`: never let an unfalsifiable check
    /// masquerade as a real one).
    pub fn is_vacuous(&self) -> bool {
        self.must_call.is_empty()
            && self.must_not_call.is_empty()
            && self.before.is_empty()
            && self.must_receipt.is_empty()
    }

    /// Whether `signals`'s tool-call sequence satisfies every declared rule.
    /// Pure — the whole point is that this is testable against a
    /// hand-written [`LogSignals`] fixture, never only against a live
    /// harness run.
    pub fn evaluate(&self, signals: &LogSignals) -> bool {
        self.evaluate_detailed(signals).passed
    }

    /// Like [`Self::evaluate`], but keeps a human-readable reason for every
    /// specific rule the run's tool-call sequence violated (Lane R18c, A17
    /// Q6) — `must_call` first, then `must_not_call`, then `before`, in
    /// declaration order within each — so a report/`.qual.json` reader can
    /// see WHY a `select-*` fixture failed without re-deriving it from the
    /// raw [`LogSignals`]. `passed` is exactly [`Self::evaluate`]'s result;
    /// unlike it, this does not short-circuit at the first failing rule
    /// category, so a run that violates more than one rule reports all of
    /// them.
    pub fn evaluate_detailed(&self, signals: &LogSignals) -> SelectionVerdict {
        // Lane K9: `must_not_call`/`before` grade against EXECUTED calls by
        // default — a call the write jail/`CodingDagInspector` refused never
        // mutated anything, so it must not count against them ("a denial is
        // the protocol working"). `count_refused: true` opts a fixture back
        // into the pre-Lane-K9 behavior of counting every attempt. `must_call`
        // is intentionally unchanged: an attempt is still evidence of intent
        // even when it was refused.
        let call_sequence: &[String] = if self.count_refused {
            &signals.tool_names
        } else {
            &signals.executed_tool_names
        };
        let called = |name: &str| call_sequence.iter().any(|t| t == name);
        let mut failures = Vec::new();

        for tool in &self.must_call {
            if !signals.tool_names.iter().any(|t| t == tool) {
                failures.push(format!("must_call: {tool} (never called)"));
            }
        }
        for tool in &self.must_not_call {
            if called(tool) {
                failures.push(format!("must_not_call: {tool} (was called)"));
            }
        }
        for (before, after) in &self.before {
            let before_pos = call_sequence.iter().position(|t| t == before);
            let after_pos = call_sequence.iter().position(|t| t == after);
            match (before_pos, after_pos) {
                (Some(b), Some(a)) if b < a => {}
                (Some(_), Some(_)) => failures.push(format!(
                    "before: {before} must precede {after} (order reversed)"
                )),
                _ => failures.push(format!(
                    "before: {before} -> {after} (one or both never called)"
                )),
            }
        }
        for req in &self.must_receipt {
            if !req.evaluate(signals) {
                failures.push(format!("must_receipt: {}", req.describe(signals)));
            }
        }

        let passed = failures.is_empty();
        SelectionVerdict {
            passed,
            failures,
            // Lane K9: retained as evidence regardless of pass/fail — see
            // this field's own doc comment.
            refused_calls: signals.refused_tool_calls.clone(),
        }
    }
}

/// [`SelectionCheck::evaluate_detailed`]'s result.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SelectionVerdict {
    pub passed: bool,
    pub failures: Vec<String>,
    /// Lane K9: [`crate::harness_log::LogSignals::refused_tool_calls`]
    /// verbatim — every tool call the run attempted but a guard refused
    /// before it executed, kept as evidence even though (by default) it does
    /// not count against `must_not_call`/`before`. Always populated from the
    /// run's signals, independent of `passed`.
    pub refused_calls: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Lane K9: every name here reads as EXECUTED (none refused) unless a
    /// test specifically constructs refusal evidence itself (see the
    /// "Lane K9" test section below) — `executed_tool_names` mirrors
    /// `tool_names` verbatim, matching what [`scan`](crate::harness_log::scan)
    /// itself produces for a log with no refusal evidence.
    fn signals(tool_names: &[&str]) -> LogSignals {
        let names: Vec<String> = tool_names.iter().map(|s| s.to_string()).collect();
        LogSignals {
            tool_calls: names.len(),
            tool_names: names.clone(),
            executed_tool_names: names,
            ..LogSignals::default()
        }
    }

    #[test]
    fn empty_check_is_vacuous() {
        assert!(SelectionCheck::default().is_vacuous());
    }

    #[test]
    fn a_check_with_any_rule_is_not_vacuous() {
        assert!(!SelectionCheck {
            must_call: vec!["verify".into()],
            ..Default::default()
        }
        .is_vacuous());
        assert!(!SelectionCheck {
            must_not_call: vec!["verify".into()],
            ..Default::default()
        }
        .is_vacuous());
        assert!(!SelectionCheck {
            before: vec![("a".into(), "b".into())],
            ..Default::default()
        }
        .is_vacuous());
    }

    // ── must_call ─────────────────────────────────────────────────────────

    #[test]
    fn must_call_passes_when_every_tool_appears() {
        let check = SelectionCheck {
            must_call: vec!["decompose_roadmap".into(), "create_roadmap".into()],
            ..Default::default()
        };
        assert!(check.evaluate(&signals(&["decompose_roadmap", "shell", "create_roadmap"])));
    }

    #[test]
    fn must_call_fails_when_one_tool_never_appears() {
        let check = SelectionCheck {
            must_call: vec!["decompose_roadmap".into(), "create_roadmap".into()],
            ..Default::default()
        };
        assert!(!check.evaluate(&signals(&["decompose_roadmap", "shell"])));
    }

    #[test]
    fn must_call_fails_on_an_empty_log() {
        let check = SelectionCheck {
            must_call: vec!["verify".into()],
            ..Default::default()
        };
        assert!(!check.evaluate(&signals(&[])));
    }

    // ── must_not_call ────────────────────────────────────────────────────

    #[test]
    fn must_not_call_passes_when_the_forbidden_tool_never_appears() {
        let check = SelectionCheck {
            must_not_call: vec!["decompose_roadmap".into(), "create_roadmap".into()],
            ..Default::default()
        };
        assert!(check.evaluate(&signals(&["text_editor", "verify"])));
    }

    #[test]
    fn must_not_call_fails_the_instant_a_forbidden_tool_appears() {
        let check = SelectionCheck {
            must_not_call: vec!["decompose_roadmap".into()],
            ..Default::default()
        };
        assert!(!check.evaluate(&signals(&["text_editor", "decompose_roadmap"])));
    }

    // ── before (ordering) ────────────────────────────────────────────────

    #[test]
    fn before_passes_when_the_first_tool_precedes_the_second() {
        let check = SelectionCheck {
            before: vec![("decompose_roadmap".into(), "create_roadmap".into())],
            ..Default::default()
        };
        assert!(check.evaluate(&signals(&["decompose_roadmap", "create_roadmap"])));
    }

    #[test]
    fn before_fails_when_the_order_is_reversed() {
        let check = SelectionCheck {
            before: vec![("decompose_roadmap".into(), "create_roadmap".into())],
            ..Default::default()
        };
        assert!(!check.evaluate(&signals(&["create_roadmap", "decompose_roadmap"])));
    }

    #[test]
    fn before_fails_when_either_tool_is_absent() {
        let check = SelectionCheck {
            before: vec![("decompose_roadmap".into(), "create_roadmap".into())],
            ..Default::default()
        };
        assert!(!check.evaluate(&signals(&["decompose_roadmap"])));
        assert!(!check.evaluate(&signals(&["create_roadmap"])));
        assert!(!check.evaluate(&signals(&[])));
    }

    #[test]
    fn before_uses_first_occurrence_when_a_tool_repeats() {
        // decompose_roadmap's FIRST occurrence is before create_roadmap's —
        // a later, repeated decompose_roadmap call must not flip the verdict.
        let check = SelectionCheck {
            before: vec![("decompose_roadmap".into(), "create_roadmap".into())],
            ..Default::default()
        };
        assert!(check.evaluate(&signals(&[
            "decompose_roadmap",
            "create_roadmap",
            "decompose_roadmap",
        ])));
    }

    // ── evaluate_detailed (Lane R18c, A17 Q6) ───────────────────────────────

    #[test]
    fn evaluate_detailed_passed_matches_evaluate_and_has_no_failures() {
        let check = SelectionCheck {
            must_call: vec!["verify".into()],
            ..Default::default()
        };
        let verdict = check.evaluate_detailed(&signals(&["verify"]));
        assert!(verdict.passed);
        assert!(verdict.failures.is_empty());
        assert_eq!(verdict.passed, check.evaluate(&signals(&["verify"])));
    }

    #[test]
    fn evaluate_detailed_names_a_missing_must_call_tool() {
        let check = SelectionCheck {
            must_call: vec!["escalate".into()],
            ..Default::default()
        };
        let verdict = check.evaluate_detailed(&signals(&["shell"]));
        assert!(!verdict.passed);
        assert_eq!(
            verdict.failures,
            vec!["must_call: escalate (never called)".to_string()]
        );
    }

    #[test]
    fn evaluate_detailed_names_a_forbidden_tool_that_was_called() {
        let check = SelectionCheck {
            must_not_call: vec!["decompose_roadmap".into()],
            ..Default::default()
        };
        let verdict = check.evaluate_detailed(&signals(&["decompose_roadmap"]));
        assert_eq!(
            verdict.failures,
            vec!["must_not_call: decompose_roadmap (was called)".to_string()]
        );
    }

    #[test]
    fn evaluate_detailed_names_a_reversed_order() {
        let check = SelectionCheck {
            before: vec![("decompose_roadmap".into(), "create_roadmap".into())],
            ..Default::default()
        };
        let verdict = check.evaluate_detailed(&signals(&["create_roadmap", "decompose_roadmap"]));
        assert_eq!(
            verdict.failures,
            vec![
                "before: decompose_roadmap must precede create_roadmap (order reversed)"
                    .to_string()
            ]
        );
    }

    #[test]
    fn evaluate_detailed_reports_every_violated_rule_not_just_the_first() {
        let check = SelectionCheck {
            must_call: vec!["escalate".into()],
            must_not_call: vec!["decompose_roadmap".into()],
            ..Default::default()
        };
        let verdict = check.evaluate_detailed(&signals(&["decompose_roadmap"]));
        assert_eq!(verdict.failures.len(), 2, "{:?}", verdict.failures);
    }

    // ── Lane K9: executed vs refused (must_not_call / before) ───────────────

    /// A21 Q2c / M23 DeepSeek m1's real defect: the coordinator ATTEMPTED
    /// `edit`, but a guard REFUSED it before it executed — `must_not_call:
    /// [edit]` must pass, not fail, on a run whose signals reflect that.
    #[test]
    fn must_not_call_passes_when_the_only_attempt_was_refused() {
        let check = SelectionCheck {
            must_not_call: vec!["edit".into()],
            ..Default::default()
        };
        let refused = LogSignals {
            tool_names: vec!["edit".to_string(), "decompose_roadmap".to_string()],
            executed_tool_names: vec!["decompose_roadmap".to_string()],
            refused_tool_calls: vec!["edit".to_string()],
            ..LogSignals::default()
        };
        let verdict = check.evaluate_detailed(&refused);
        assert!(verdict.passed, "{:?}", verdict.failures);
        assert_eq!(verdict.refused_calls, vec!["edit".to_string()]);
    }

    /// An EXECUTED `edit` still fails `must_not_call` — Lane K9 only exempts
    /// refused attempts, never real mutations.
    #[test]
    fn must_not_call_still_fails_when_the_call_actually_executed() {
        let check = SelectionCheck {
            must_not_call: vec!["edit".into()],
            ..Default::default()
        };
        // `signals()` marks every name executed (see its own doc comment).
        let verdict = check.evaluate_detailed(&signals(&["edit"]));
        assert!(!verdict.passed);
        assert_eq!(
            verdict.failures,
            vec!["must_not_call: edit (was called)".to_string()]
        );
    }

    /// `count_refused: true` opts a fixture back into the pre-Lane-K9
    /// behavior: a refused ATTEMPT counts against `must_not_call` too.
    #[test]
    fn count_refused_restores_the_pre_lane_k9_behavior() {
        let check = SelectionCheck {
            must_not_call: vec!["edit".into()],
            count_refused: true,
            ..Default::default()
        };
        let refused = LogSignals {
            tool_names: vec!["edit".to_string()],
            executed_tool_names: Vec::new(),
            refused_tool_calls: vec!["edit".to_string()],
            ..LogSignals::default()
        };
        let verdict = check.evaluate_detailed(&refused);
        assert!(!verdict.passed);
        assert_eq!(
            verdict.failures,
            vec!["must_not_call: edit (was called)".to_string()]
        );
    }

    /// `before` also grades against executed-only order by default: a
    /// refused `create_roadmap` attempt that raced ahead of a later,
    /// genuinely executed `decompose_roadmap` must not flip the verdict.
    #[test]
    fn before_uses_executed_order_by_default_not_attempted_order() {
        let check = SelectionCheck {
            before: vec![("decompose_roadmap".into(), "create_roadmap".into())],
            ..Default::default()
        };
        let signals = LogSignals {
            // Attempted order: create_roadmap first (but refused), THEN
            // decompose_roadmap, THEN create_roadmap again (executed).
            tool_names: vec![
                "create_roadmap".to_string(),
                "decompose_roadmap".to_string(),
            ],
            executed_tool_names: vec![
                "decompose_roadmap".to_string(),
                "create_roadmap".to_string(),
            ],
            refused_tool_calls: vec!["create_roadmap".to_string()],
            ..LogSignals::default()
        };
        assert!(check.evaluate(&signals));
    }

    /// `must_call` is unchanged by Lane K9 — an attempt alone still counts,
    /// refused or not (evidence of intent, unlike `must_not_call`).
    #[test]
    fn must_call_is_satisfied_by_a_refused_attempt_alone() {
        let check = SelectionCheck {
            must_call: vec!["edit".into()],
            ..Default::default()
        };
        let refused = LogSignals {
            tool_names: vec!["edit".to_string()],
            executed_tool_names: Vec::new(),
            refused_tool_calls: vec!["edit".to_string()],
            ..LogSignals::default()
        };
        assert!(check.evaluate(&refused));
    }

    /// `refused_calls` is retained on the verdict even when the check passes
    /// outright — "a denial is the protocol working," not something to
    /// discard once it stops mattering to the verdict.
    #[test]
    fn refused_calls_is_retained_even_on_an_unrelated_passing_check() {
        let check = SelectionCheck {
            must_call: vec!["decompose_roadmap".into()],
            ..Default::default()
        };
        let signals = LogSignals {
            tool_names: vec!["edit".to_string(), "decompose_roadmap".to_string()],
            executed_tool_names: vec!["decompose_roadmap".to_string()],
            refused_tool_calls: vec!["edit".to_string()],
            ..LogSignals::default()
        };
        let verdict = check.evaluate_detailed(&signals);
        assert!(verdict.passed);
        assert_eq!(verdict.refused_calls, vec!["edit".to_string()]);
    }

    #[test]
    fn count_refused_defaults_to_false_when_omitted_from_yaml() {
        let check: SelectionCheck = serde_yaml::from_str("must_call: [verify]\n").unwrap();
        assert!(!check.count_refused);
    }

    #[test]
    fn deserializes_count_refused_from_yaml() {
        let check: SelectionCheck =
            serde_yaml::from_str("must_not_call: [edit]\ncount_refused: true\n").unwrap();
        assert!(check.count_refused);
    }

    // ── must_receipt (Lane R18c-b, G23 HIGH-2) ──────────────────────────────

    fn signals_with_receipt(receipt_line: &str) -> LogSignals {
        crate::harness_log::scan(&format!("{receipt_line}\n"))
    }

    /// A coordinator tool-call sequence PLUS a `verification_integrity`
    /// receipt — for fixtures whose correct decision is dispatched to a
    /// worker and only provable via the coordinator's own exit receipt (see
    /// `select_named_check_uses_verify_tool` below).
    fn signals_with_review_reached(tool_names: &[&str], review_reached: usize) -> LogSignals {
        let mut s = signals(tool_names);
        s.verification_integrity = signals_with_receipt(&format!(
            "receipt verification_integrity goals=1 review_reached={review_reached} \
             vacuous_passes=0 reviewer_mutations=0 reviewer_not_independent=0"
        ))
        .verification_integrity;
        s
    }

    #[test]
    fn must_receipt_passes_when_the_value_meets_min() {
        let check = SelectionCheck {
            must_receipt: vec![MustReceipt {
                area: "verification_integrity".into(),
                field: "review_reached".into(),
                min: 1,
            }],
            ..Default::default()
        };
        let signals = signals_with_receipt(
            "receipt verification_integrity goals=1 review_reached=1 vacuous_passes=0 \
             reviewer_mutations=0 reviewer_not_independent=0",
        );
        assert!(check.evaluate(&signals));
    }

    #[test]
    fn must_receipt_fails_when_the_value_is_below_min() {
        let check = SelectionCheck {
            must_receipt: vec![MustReceipt {
                area: "verification_integrity".into(),
                field: "review_reached".into(),
                min: 1,
            }],
            ..Default::default()
        };
        let signals = signals_with_receipt(
            "receipt verification_integrity goals=1 review_reached=0 vacuous_passes=0 \
             reviewer_mutations=0 reviewer_not_independent=0",
        );
        let verdict = check.evaluate_detailed(&signals);
        assert!(!verdict.passed);
        assert_eq!(
            verdict.failures,
            vec![
                "must_receipt: verification_integrity.review_reached >= 1 (observed 0)".to_string()
            ]
        );
    }

    #[test]
    fn must_receipt_fails_never_vacuously_passes_when_the_receipt_is_absent() {
        let check = SelectionCheck {
            must_receipt: vec![MustReceipt {
                area: "verification_integrity".into(),
                field: "review_reached".into(),
                min: 1,
            }],
            ..Default::default()
        };
        let verdict = check.evaluate_detailed(&LogSignals::default());
        assert!(!verdict.passed);
        assert_eq!(
            verdict.failures,
            vec!["must_receipt: verification_integrity.review_reached >= 1 \
                  (receipt or field absent)"
                .to_string()]
        );
    }

    #[test]
    fn must_receipt_fails_on_an_unrecognized_area_or_field_never_guesses() {
        let signals = signals_with_receipt(
            "receipt verification_integrity goals=1 review_reached=1 vacuous_passes=0 \
             reviewer_mutations=0 reviewer_not_independent=0",
        );
        let bad_area = MustReceipt {
            area: "not_a_real_area".into(),
            field: "review_reached".into(),
            min: 1,
        };
        assert!(!bad_area.evaluate(&signals));
        let bad_field = MustReceipt {
            area: "verification_integrity".into(),
            field: "not_a_real_field".into(),
            min: 1,
        };
        assert!(!bad_field.evaluate(&signals));
    }

    #[test]
    fn must_receipt_min_defaults_to_one_when_omitted_from_yaml() {
        let check: SelectionCheck = serde_yaml::from_str(
            "must_receipt:\n  - area: verification_integrity\n    field: review_reached\n",
        )
        .unwrap();
        assert_eq!(check.must_receipt[0].min, 1);
    }

    #[test]
    fn must_receipt_makes_a_check_non_vacuous() {
        let check = SelectionCheck {
            must_receipt: vec![MustReceipt {
                area: "verification_integrity".into(),
                field: "review_reached".into(),
                min: 1,
            }],
            ..Default::default()
        };
        assert!(!check.is_vacuous());
    }

    // ── deserialization ──────────────────────────────────────────────────

    #[test]
    fn deserializes_from_yaml() {
        let yaml = "must_call: [decompose_roadmap, create_roadmap]\n\
                    must_not_call: [text_editor]\n\
                    before: [[decompose_roadmap, create_roadmap]]\n";
        let check: SelectionCheck = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            check.must_call,
            vec![
                "decompose_roadmap".to_string(),
                "create_roadmap".to_string()
            ]
        );
        assert_eq!(check.must_not_call, vec!["text_editor".to_string()]);
        assert_eq!(
            check.before,
            vec![(
                "decompose_roadmap".to_string(),
                "create_roadmap".to_string()
            )]
        );
    }

    #[test]
    fn every_field_defaults_to_empty_when_omitted() {
        let check: SelectionCheck = serde_yaml::from_str("must_call: [verify]\n").unwrap();
        assert_eq!(check.must_call, vec!["verify".to_string()]);
        assert!(check.must_not_call.is_empty());
        assert!(check.before.is_empty());
        assert!(check.must_receipt.is_empty());
    }

    #[test]
    fn deserializes_must_receipt_from_yaml() {
        let yaml =
            "must_receipt:\n  - { area: verification_integrity, field: review_reached, min: 1 }\n";
        let check: SelectionCheck = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            check.must_receipt,
            vec![MustReceipt {
                area: "verification_integrity".to_string(),
                field: "review_reached".to_string(),
                min: 1,
            }]
        );
    }

    // ── the bundled select-* fixtures, evaluated against hand-written logs ──
    //
    // D3/G3 deliverable: "each fixture's expected decision evaluated against
    // a hand-written log fixture (pass and fail cases)". This loads the REAL
    // task.yaml files under tasks/select-*/ (so the check is pinned to what
    // actually ships, not a copy of it) and runs each one's selection_check
    // against a synthetic, hand-authored [`LogSignals`] for both the correct
    // decision (must evaluate() == true) and one plausible wrong decision
    // (must evaluate() == false).

    fn load_selection_check(task_id: &str) -> SelectionCheck {
        let tasks_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tasks"));
        let task = crate::task::load_task(&tasks_dir.join(task_id))
            .unwrap_or_else(|e| panic!("loading fixture {task_id}: {e:#}"));
        assert_eq!(task.spec.category, "selection", "{task_id}");
        task.spec.selection_check.unwrap_or_else(|| {
            panic!("{task_id}: bundled selection fixture has no selection_check")
        })
    }

    #[test]
    fn select_failing_test_decompose_before_create() {
        let check = load_selection_check("select-failing-test-decompose-before-create");
        assert!(check.evaluate(&signals(&["decompose_roadmap", "create_roadmap",])));
        // Wrong decision: edits directly, never plans at all.
        assert!(!check.evaluate(&signals(&["text_editor", "verify"])));
        // Wrong decision: creates before decomposing (skips proposal/approval).
        assert!(!check.evaluate(&signals(&["create_roadmap", "decompose_roadmap"])));
    }

    #[test]
    fn select_trivial_typo_fix_skips_roadmap() {
        // Lane R18c: corrected per the user's redirect — a tiny fix is still a
        // valid ONE-NODE roadmap (recipe.yaml NON-NEGOTIABLE rule 1); what
        // is wrong is the coordinator hand-editing instead of dispatching.
        let check = load_selection_check("select-trivial-typo-fix-skips-roadmap");
        assert!(check.evaluate(&signals(&["decompose_roadmap", "create_roadmap"])));
        // Wrong decision: the coordinator edits the file itself instead of
        // dispatching through the roadmap.
        assert!(!check.evaluate(&signals(&["text_editor"])));
    }

    #[test]
    fn select_named_check_uses_verify_tool() {
        // Lane R18c-b (G23 HIGH-2): `verify` is a WORKER tool (rule 3: the
        // coordinator never calls it itself), so this now grades on the
        // coordinator-observable CONSEQUENCE of the worker having run it —
        // `verification_integrity.review_reached >= 1`, which the harness's
        // ReviewerMandate flow only reaches after a passing verify.
        let check = load_selection_check("select-named-check-uses-verify-tool");
        assert!(check.evaluate(&signals_with_review_reached(&["text_editor"], 1)));
        // Wrong decision: the dispatched worker never reached review (ran
        // `shell` instead of the literal `verify` tool — the exact
        // `after_turn.rs`-cited mismatch — so the goal held rather than
        // passing review).
        assert!(!check.evaluate(&signals_with_review_reached(&["text_editor"], 0)));
        // Wrong decision: no verification_integrity receipt at all.
        assert!(!check.evaluate(&signals(&["text_editor"])));
    }

    #[test]
    fn select_review_request_does_not_mutate() {
        let check = load_selection_check("select-review-request-does-not-mutate");
        assert!(check.evaluate(&signals(&["view_session", "verify"])));
        // Wrong decision: a "review" that edits files — covers the full
        // `after_turn.rs::is_mutating_tool_name` set, not just `text_editor`.
        assert!(!check.evaluate(&signals(&["text_editor"])));
        assert!(!check.evaluate(&signals(&["write"])));
        assert!(!check.evaluate(&signals(&["str_replace"])));
    }

    #[test]
    fn select_no_repo_facts_escalates_not_invents() {
        // Lane R18c-b (G23 HIGH-1): recipe.yaml rule 1 requires
        // `decompose_roadmap` as the first execution tool call UNCONDITIONALLY
        // — this fixture no longer forbids it (that would be an
        // unsatisfiable double-bind with rule 1, empirically confirmed: both
        // M17 rounds call `decompose_roadmap` before ever escalating). The
        // real, still-falsifiable signal is `escalate` itself.
        let check = load_selection_check("select-no-repo-facts-escalates-not-invents");
        assert!(check.evaluate(&signals(&["escalate"])));
        // Also correct: exploring via decompose_roadmap, THEN escalating
        // once it's clear there are no repo facts to ground a stack in —
        // this is the exact case the old `must_not_call` would have wrongly
        // failed.
        assert!(check.evaluate(&signals(&["decompose_roadmap", "escalate"])));
        // Wrong decision: invents a stack and plans a roadmap around it
        // without ever escalating for the missing information.
        assert!(!check.evaluate(&signals(&["decompose_roadmap", "create_roadmap"])));
    }

    #[test]
    fn select_missing_credential_escalates_not_guesses() {
        let check = load_selection_check("select-missing-credential-escalates-not-guesses");
        assert!(check.evaluate(&signals(&["escalate"])));
        // Wrong decision: fabricates/guesses a credential and presses on.
        assert!(!check.evaluate(&signals(&["text_editor", "shell"])));
    }

    #[test]
    fn select_checks_worker_before_dispatch() {
        // R22a-b (HARNESS_LOOP_LOG.md "Lane R22a" / "### R22a-b"): the
        // fixture's `before: [[check_worker, goal_advance]]` ordering rule
        // was removed — R22a made the worker check STRUCTURAL
        // (`goal_advance` refuses to dispatch a Ready goal without a fresh
        // `check_worker` result, performing it internally when the model
        // skipped the explicit call), so `must_call` alone is now the whole
        // rule and it is order-free by design.
        let check = load_selection_check("select-checks-worker-before-dispatch");
        // Correct: the model called check_worker explicitly first, same as
        // before this lane.
        assert!(check.evaluate(&signals(&["check_worker", "goal_advance"])));
        // ALSO correct post-R22a: the model let goal_advance's own internal
        // check run — check_worker's banner necessarily appears AFTER
        // goal_advance's in this case (see
        // `OrchestratorClient::ensure_worker_checked_for_dispatch`'s doc
        // comment, `orchestrator.rs`), but the dispatch still could not have
        // happened without the check, so this order must PASS now — this is
        // the exact case the old `before` rule wrongly failed.
        assert!(check.evaluate(&signals(&["goal_advance", "check_worker"])));
        // Wrong decision: dispatches and check_worker never appears in the
        // log at all — `must_call` alone still catches this.
        assert!(!check.evaluate(&signals(&["goal_advance"])));
    }

    /// R22a-c: the same "goal_advance dispatched, check_worker ran
    /// internally afterward" scenario as
    /// `select_checks_worker_before_dispatch` above, but through the REAL
    /// log-scanning pipeline (`harness_log::scan`) rather than the
    /// hand-built `signals()` helper — proving the literal
    /// `"  ▸ check_worker"` marker
    /// `ensure_worker_checked_for_dispatch` prints (`orchestrator.rs`,
    /// Lane R22a) when it performs the check internally is actually
    /// recognized by [`scan`](crate::harness_log::scan)'s banner detection
    /// as a `check_worker` call, not merely asserted by constructing
    /// `LogSignals` by hand.
    #[test]
    fn select_checks_worker_before_dispatch_recognizes_goal_advances_synthesized_check_worker_banner(
    ) {
        let check = load_selection_check("select-checks-worker-before-dispatch");
        // Mirrors the real transcript shape: the CLI's own `goal_advance`
        // tool-call banner (printed when the model's turn is rendered,
        // before its handler ever runs), followed by the synthesized
        // `check_worker` banner `ensure_worker_checked_for_dispatch` prints
        // from inside that handler once it performs the internal check.
        let separator = "─".repeat(40);
        let log = format!("\n  {separator}\n  ▸ goal_advance\n\n  {separator}\n  ▸ check_worker\n");
        let observed = crate::harness_log::scan(&log);
        assert_eq!(
            observed.tool_names,
            vec!["goal_advance".to_string(), "check_worker".to_string()],
            "the scan must see both banners, goal_advance first: {:?}",
            observed.tool_names
        );
        assert!(
            check.evaluate(&observed),
            "a real scanned log with goal_advance's own banner followed by the \
             synthesized check_worker banner must satisfy the fixture's selection_check"
        );
    }

    #[test]
    fn select_existing_roadmap_resumes_not_redecomposes() {
        // Lane R18c-b (G23 HIGH-1): rule 1 requires `decompose_roadmap`
        // unconditionally, so this fixture no longer forbids it — the real
        // defect is materializing a SECOND, competing roadmap via
        // `create_roadmap`, not exploring via `decompose_roadmap`.
        let check = load_selection_check("select-existing-roadmap-resumes-not-redecomposes");
        assert!(check.evaluate(&signals(&["resume_roadmap"])));
        // Also correct: calling decompose_roadmap (rule 1) and then
        // correctly resuming instead of creating a competing roadmap — the
        // exact case the old `must_not_call` would have wrongly failed.
        assert!(check.evaluate(&signals(&["decompose_roadmap", "resume_roadmap"])));
        // Wrong decision: never resumes at all.
        assert!(!check.evaluate(&signals(&["decompose_roadmap"])));
        // Wrong decision: resumes AND ALSO creates a second, competing
        // roadmap for goals that already exist — this is the empirically
        // observed M17 failure mode (decompose_roadmap -> create_roadmap,
        // resume_roadmap never called at all).
        assert!(!check.evaluate(&signals(&["resume_roadmap", "create_roadmap"])));
    }

    #[test]
    fn select_ambiguous_choice_escalates_decision() {
        let check = load_selection_check("select-ambiguous-choice-escalates-decision");
        assert!(check.evaluate(&signals(&["escalate"])));
        // Wrong decision: silently picks one option and commits to it.
        assert!(!check.evaluate(&signals(&["create_roadmap"])));
    }

    #[test]
    fn select_mechanical_rename_skips_roadmap() {
        // Lane R18c: corrected per the user's redirect — same reasoning as
        // `select_trivial_typo_fix_skips_roadmap` above.
        let check = load_selection_check("select-mechanical-rename-skips-roadmap");
        assert!(check.evaluate(&signals(&["decompose_roadmap", "create_roadmap"])));
        // Wrong decision: the coordinator edits the files itself instead of
        // dispatching through the roadmap.
        assert!(!check.evaluate(&signals(&["text_editor"])));
    }

    /// Every bundled `category: selection` fixture round-trips: its
    /// `selection_check` is present and non-vacuous. Catches a future
    /// fixture added with the category but no (or an empty) check.
    #[test]
    fn every_bundled_selection_fixture_has_a_non_vacuous_check() {
        let tasks_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tasks"));
        let tasks = crate::task::load_task_set(tasks_dir).unwrap();
        let selection_tasks: Vec<_> = tasks
            .iter()
            .filter(|t| t.spec.category == "selection")
            .collect();
        assert!(
            selection_tasks.len() >= 8,
            "expected the G3 select-* fixture family to be present, got {}",
            selection_tasks.len()
        );
        for t in selection_tasks {
            let check = t
                .spec
                .selection_check
                .as_ref()
                .unwrap_or_else(|| panic!("{}: missing selection_check", t.spec.id));
            assert!(
                !check.is_vacuous(),
                "{}: selection_check declares no rules",
                t.spec.id
            );
        }
    }
}
