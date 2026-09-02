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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldOutcome {
    Allow,
    Hold { inject_plan: String, hold_count: u8 },
    Park { reason: String },
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
/// - Any role after a successful verify → Allow.
/// - Any role without one → Hold (or Park past [`MAX_HOLDS`]).
/// - The one exception to the receipt: a hands-on role that verified green and
///   then went on repeating a failing command still holds.
pub fn decide_hold(
    role: WorkflowRole,
    verify_ran: bool,
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
    if verify_ran && !(hands_on && signals.spinning >= 0.5) {
        return HoldOutcome::Allow;
    }

    let next = prior_holds.saturating_add(1);
    if next > MAX_HOLDS {
        return HoldOutcome::Park {
            reason: "held twice without verify — handing to the Decision Inbox".into(),
        };
    }

    let inject_plan = if verify_ran {
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
}
