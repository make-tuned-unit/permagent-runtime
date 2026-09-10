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
}

impl SelectionCheck {
    /// True when this check declares no rules at all — every run would
    /// vacuously "pass" it. `Task::validate` (task.rs) rejects a fixture
    /// whose `selection_check` is vacuous, mirroring the G7 F1 lesson
    /// (`orchestrator.rs`'s `grep_absent`: never let an unfalsifiable check
    /// masquerade as a real one).
    pub fn is_vacuous(&self) -> bool {
        self.must_call.is_empty() && self.must_not_call.is_empty() && self.before.is_empty()
    }

    /// Whether `signals`'s tool-call sequence satisfies every declared rule.
    /// Pure — the whole point is that this is testable against a
    /// hand-written [`LogSignals`] fixture, never only against a live
    /// harness run.
    pub fn evaluate(&self, signals: &LogSignals) -> bool {
        let called = |name: &str| signals.tool_names.iter().any(|t| t == name);

        if self.must_call.iter().any(|t| !called(t)) {
            return false;
        }
        if self.must_not_call.iter().any(|t| called(t)) {
            return false;
        }
        for (before, after) in &self.before {
            let Some(before_pos) = signals.tool_names.iter().position(|t| t == before) else {
                return false;
            };
            let Some(after_pos) = signals.tool_names.iter().position(|t| t == after) else {
                return false;
            };
            if before_pos >= after_pos {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn signals(tool_names: &[&str]) -> LogSignals {
        LogSignals {
            tool_calls: tool_names.len(),
            tool_names: tool_names.iter().map(|s| s.to_string()).collect(),
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
        let check = load_selection_check("select-trivial-typo-fix-skips-roadmap");
        assert!(check.evaluate(&signals(&["text_editor"])));
        // Wrong decision: spins up a roadmap for a one-line typo.
        assert!(!check.evaluate(&signals(&["decompose_roadmap", "create_roadmap"])));
    }

    #[test]
    fn select_named_check_uses_verify_tool() {
        let check = load_selection_check("select-named-check-uses-verify-tool");
        assert!(check.evaluate(&signals(&["text_editor", "verify"])));
        // Wrong decision: runs the test command via `shell` instead of the
        // literal `verify` tool — the exact `after_turn.rs`-cited mismatch.
        assert!(!check.evaluate(&signals(&["text_editor", "shell"])));
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
        let check = load_selection_check("select-no-repo-facts-escalates-not-invents");
        assert!(check.evaluate(&signals(&["escalate"])));
        // Wrong decision: invents a stack and plans a roadmap around it.
        assert!(!check.evaluate(&signals(&["decompose_roadmap"])));
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
        let check = load_selection_check("select-checks-worker-before-dispatch");
        assert!(check.evaluate(&signals(&["check_worker", "goal_advance"])));
        // Wrong decision: dispatches blind, never checks the worker first.
        assert!(!check.evaluate(&signals(&["goal_advance"])));
        assert!(!check.evaluate(&signals(&["goal_advance", "check_worker"])));
    }

    #[test]
    fn select_existing_roadmap_resumes_not_redecomposes() {
        let check = load_selection_check("select-existing-roadmap-resumes-not-redecomposes");
        assert!(check.evaluate(&signals(&["resume_roadmap"])));
        // Wrong decision: re-plans an already-approved roadmap from scratch.
        assert!(!check.evaluate(&signals(&["decompose_roadmap"])));
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
        let check = load_selection_check("select-mechanical-rename-skips-roadmap");
        assert!(check.evaluate(&signals(&["text_editor"])));
        // Wrong decision: over-decomposes a two-file mechanical rename.
        assert!(!check.evaluate(&signals(&["decompose_roadmap", "create_roadmap"])));
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
