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
/// - Orchestrate / Review / Edit after verify → Allow.
/// - Mechanical / Local without verify → Hold (or Park past [`MAX_HOLDS`]).
/// - Spinning without verify also holds.
pub fn decide_hold(
    role: WorkflowRole,
    verify_ran: bool,
    signals: &ToolTranscriptSignals,
    prior_holds: u8,
) -> HoldOutcome {
    let judgment = matches!(
        role,
        WorkflowRole::Orchestrate | WorkflowRole::Review | WorkflowRole::Edit
    );
    if judgment && verify_ran {
        return HoldOutcome::Allow;
    }
    if judgment && !verify_ran {
        // Edit/orchestrate still must verify; treat like a mechanical hold.
    } else if role == WorkflowRole::Mechanical || role == WorkflowRole::Local {
        if verify_ran && signals.spinning < 0.5 {
            return HoldOutcome::Allow;
        }
    } else if verify_ran {
        return HoldOutcome::Allow;
    }

    let needs_hold = !verify_ran || signals.spinning >= 0.5 || signals.severity >= 0.7;
    if !needs_hold {
        return HoldOutcome::Allow;
    }

    let next = prior_holds.saturating_add(1);
    if next > MAX_HOLDS {
        return HoldOutcome::Park {
            reason: "held twice without verify — handing to the Decision Inbox".into(),
        };
    }

    let inject_plan = if !verify_ran {
        "Do not declare this done. Run verify, read the errors, make one focused fix, and run verify again.".into()
    } else {
        "Verify is still failing the same way. Stop retrying the identical command; change the approach, then verify once.".into()
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
        assert!(matches!(
            decide_hold(WorkflowRole::Mechanical, true, &spinning, 0),
            HoldOutcome::Hold { .. }
        ));
    }
}
