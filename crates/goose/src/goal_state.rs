//! Goal lifecycle state machine — pure validation logic.
//!
//! The five goal states: Triage → Ready → InProgress → Review → Complete
//! with a bounce-back path from Review → InProgress.
//!
//! This module is deliberately free of async, DB, or IO — it's a pure
//! state machine that can be unit tested without infrastructure.

use std::fmt;

/// The five lifecycle states a goal card can be in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalState {
    Triage,
    Ready,
    InProgress,
    Review,
    Complete,
}

impl GoalState {
    /// Parse a state_binding string (from board_columns.state_binding) into a GoalState.
    pub fn from_binding(s: &str) -> Option<Self> {
        match s {
            "triage" => Some(Self::Triage),
            "ready" => Some(Self::Ready),
            "in_progress" => Some(Self::InProgress),
            "review" => Some(Self::Review),
            "complete" => Some(Self::Complete),
            _ => None,
        }
    }

    /// The state_binding string for this state (matches board_columns.state_binding).
    pub fn binding(&self) -> &'static str {
        match self {
            Self::Triage => "triage",
            Self::Ready => "ready",
            Self::InProgress => "in_progress",
            Self::Review => "review",
            Self::Complete => "complete",
        }
    }
}

impl fmt::Display for GoalState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Triage => write!(f, "Triage"),
            Self::Ready => write!(f, "Ready"),
            Self::InProgress => write!(f, "In Progress"),
            Self::Review => write!(f, "Review"),
            Self::Complete => write!(f, "Complete"),
        }
    }
}

/// Actions that can be performed on a goal card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalAction {
    /// Triage → Ready: goal is well-defined enough to assign
    Ready,
    /// Ready → InProgress: worker selected and dispatch started
    Dispatch,
    /// InProgress → Review: worker reports completion
    Review,
    /// Review → Complete: user accepts the work
    Approve,
    /// Review → InProgress: user rejects, bounce-back for rework
    Reject,
}

impl GoalAction {
    /// Parse an action string from the MCP tool parameter.
    pub fn parse_action(s: &str) -> Option<Self> {
        match s {
            "ready" => Some(Self::Ready),
            "dispatch" => Some(Self::Dispatch),
            "review" => Some(Self::Review),
            "approve" => Some(Self::Approve),
            "reject" => Some(Self::Reject),
            _ => None,
        }
    }
}

impl fmt::Display for GoalAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready => write!(f, "ready"),
            Self::Dispatch => write!(f, "dispatch"),
            Self::Review => write!(f, "review"),
            Self::Approve => write!(f, "approve"),
            Self::Reject => write!(f, "reject"),
        }
    }
}

/// Error returned when a state transition is invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionError {
    pub from: GoalState,
    pub action: GoalAction,
    pub reason: String,
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Cannot perform '{}' on a goal in {}: {}",
            self.action, self.from, self.reason
        )
    }
}

/// Validate a state transition. Pure function — no IO, no DB.
///
/// Returns the new state on success, or a descriptive error on failure.
pub fn validate_transition(
    current: GoalState,
    action: GoalAction,
) -> Result<GoalState, TransitionError> {
    match (current, action) {
        (GoalState::Triage, GoalAction::Ready) => Ok(GoalState::Ready),
        (GoalState::Ready, GoalAction::Dispatch) => Ok(GoalState::InProgress),
        (GoalState::InProgress, GoalAction::Review) => Ok(GoalState::Review),
        (GoalState::Review, GoalAction::Approve) => Ok(GoalState::Complete),
        (GoalState::Review, GoalAction::Reject) => Ok(GoalState::InProgress),
        _ => {
            let valid_actions = match current {
                GoalState::Triage => "'ready'",
                GoalState::Ready => "'dispatch'",
                GoalState::InProgress => "'review'",
                GoalState::Review => "'approve' or 'reject'",
                GoalState::Complete => "none (terminal state)",
            };
            Err(TransitionError {
                from: current,
                action,
                reason: format!("{} only accepts: {}", current, valid_actions),
            })
        }
    }
}

// ── Worker selection (pure logic) ────────────────────────────────────────

/// A worker candidate for selection, carrying the data needed for the algorithm.
#[derive(Debug, Clone)]
pub struct WorkerCandidate {
    pub key: String,
    pub available: bool,
    pub tool_kinds: Vec<String>,
    pub cost_tier: String,
    pub active_sessions: usize,
}

/// Select the best worker from candidates for a goal with the given required tool kinds.
///
/// Algorithm:
/// 1. Filter by available == true
/// 2. Filter by capability: worker.tool_kinds intersects required_kinds
///    (workers with empty tool_kinds match everything)
/// 3. Sort by cost_tier: local_free > subscription > paid_api
/// 4. Within same tier: fewest active_sessions wins
/// 5. Final tie-break: alphabetical by key (deterministic)
pub fn select_best_worker(
    candidates: &[WorkerCandidate],
    required_kinds: &[String],
) -> Result<String, String> {
    let mut eligible: Vec<&WorkerCandidate> = candidates
        .iter()
        .filter(|w| w.available)
        .filter(|w| {
            if w.tool_kinds.is_empty() {
                // Workers with no declared tool_kinds match everything
                true
            } else {
                required_kinds
                    .iter()
                    .any(|req| w.tool_kinds.iter().any(|tk| tk == req))
            }
        })
        .collect();

    if eligible.is_empty() {
        return Err(
            "No suitable worker available: no workers match the required \
             capabilities or all are unavailable"
                .to_string(),
        );
    }

    // Sort by: cost_tier rank, then active_sessions, then key (deterministic)
    eligible.sort_by(|a, b| {
        let tier_a = cost_tier_rank(&a.cost_tier);
        let tier_b = cost_tier_rank(&b.cost_tier);
        tier_a
            .cmp(&tier_b)
            .then(a.active_sessions.cmp(&b.active_sessions))
            .then(a.key.cmp(&b.key))
    });

    Ok(eligible[0].key.clone())
}

fn cost_tier_rank(tier: &str) -> u8 {
    match tier {
        "local_free" => 0,
        "subscription" => 1,
        "paid_api" => 2,
        _ => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Worker selection ──────────────────────────────────────────────

    fn make_worker(
        key: &str,
        available: bool,
        tool_kinds: &[&str],
        cost_tier: &str,
        sessions: usize,
    ) -> WorkerCandidate {
        WorkerCandidate {
            key: key.to_string(),
            available,
            tool_kinds: tool_kinds.iter().map(|s| s.to_string()).collect(),
            cost_tier: cost_tier.to_string(),
            active_sessions: sessions,
        }
    }

    #[test]
    fn select_picks_local_free_when_available() {
        let workers = vec![
            make_worker("codex", true, &["code_edit", "shell"], "subscription", 0),
            make_worker("qwen", true, &["code_edit", "shell"], "local_free", 0),
        ];
        let result = select_best_worker(&workers, &["code_edit".into()]);
        assert_eq!(result.unwrap(), "qwen");
    }

    #[test]
    fn select_falls_through_tiers() {
        let workers = vec![
            make_worker("qwen", false, &["code_edit"], "local_free", 0),
            make_worker("codex", true, &["code_edit", "shell"], "subscription", 0),
            make_worker("gpt4", true, &["code_edit"], "paid_api", 0),
        ];
        let result = select_best_worker(&workers, &["code_edit".into()]);
        assert_eq!(
            result.unwrap(),
            "codex",
            "Should skip unavailable qwen, pick subscription codex"
        );
    }

    #[test]
    fn select_tiebreaker_fewest_sessions() {
        let workers = vec![
            make_worker("codex", true, &["code_edit"], "subscription", 3),
            make_worker("cc", true, &["code_edit"], "subscription", 1),
        ];
        let result = select_best_worker(&workers, &["code_edit".into()]);
        assert_eq!(result.unwrap(), "cc", "Should pick cc with fewer sessions");
    }

    #[test]
    fn select_err_when_no_capability_match() {
        let workers = vec![make_worker(
            "librarian",
            true,
            &["memory_ops"],
            "local_free",
            0,
        )];
        let result = select_best_worker(&workers, &["code_edit".into()]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No suitable worker"));
    }

    #[test]
    fn select_err_when_all_unavailable() {
        let workers = vec![
            make_worker("codex", false, &["code_edit"], "subscription", 0),
            make_worker("qwen", false, &["code_edit"], "local_free", 0),
        ];
        let result = select_best_worker(&workers, &["code_edit".into()]);
        assert!(result.is_err());
    }

    #[test]
    fn select_empty_tool_kinds_matches_everything() {
        let workers = vec![make_worker("general", true, &[], "local_free", 0)];
        let result = select_best_worker(&workers, &["code_edit".into(), "shell".into()]);
        assert_eq!(result.unwrap(), "general");
    }

    #[test]
    fn select_deterministic_alphabetical_tiebreak() {
        let workers = vec![
            make_worker("bravo", true, &["code_edit"], "local_free", 0),
            make_worker("alpha", true, &["code_edit"], "local_free", 0),
        ];
        let result = select_best_worker(&workers, &["code_edit".into()]);
        assert_eq!(result.unwrap(), "alpha", "Alphabetical tie-break");
    }

    // ── Valid transitions ─────────────────────────────────────────────

    #[test]
    fn triage_to_ready() {
        assert_eq!(
            validate_transition(GoalState::Triage, GoalAction::Ready),
            Ok(GoalState::Ready)
        );
    }

    #[test]
    fn ready_to_in_progress() {
        assert_eq!(
            validate_transition(GoalState::Ready, GoalAction::Dispatch),
            Ok(GoalState::InProgress)
        );
    }

    #[test]
    fn in_progress_to_review() {
        assert_eq!(
            validate_transition(GoalState::InProgress, GoalAction::Review),
            Ok(GoalState::Review)
        );
    }

    #[test]
    fn review_to_complete() {
        assert_eq!(
            validate_transition(GoalState::Review, GoalAction::Approve),
            Ok(GoalState::Complete)
        );
    }

    #[test]
    fn review_to_in_progress_bounce_back() {
        assert_eq!(
            validate_transition(GoalState::Review, GoalAction::Reject),
            Ok(GoalState::InProgress)
        );
    }

    // ── Invalid transitions ───────────────────────────────────────────

    #[test]
    fn triage_rejects_dispatch() {
        let err = validate_transition(GoalState::Triage, GoalAction::Dispatch).unwrap_err();
        assert_eq!(err.from, GoalState::Triage);
        assert_eq!(err.action, GoalAction::Dispatch);
        assert!(
            err.reason.contains("'ready'"),
            "Should suggest valid action: {}",
            err.reason
        );
    }

    #[test]
    fn ready_rejects_approve() {
        let err = validate_transition(GoalState::Ready, GoalAction::Approve).unwrap_err();
        assert!(err.reason.contains("'dispatch'"));
    }

    #[test]
    fn in_progress_rejects_ready() {
        let err = validate_transition(GoalState::InProgress, GoalAction::Ready).unwrap_err();
        assert!(err.reason.contains("'review'"));
    }

    #[test]
    fn complete_rejects_all() {
        for action in [
            GoalAction::Ready,
            GoalAction::Dispatch,
            GoalAction::Review,
            GoalAction::Approve,
            GoalAction::Reject,
        ] {
            let err = validate_transition(GoalState::Complete, action).unwrap_err();
            assert!(
                err.reason.contains("terminal state"),
                "Complete + {} should say terminal: {}",
                action,
                err.reason
            );
        }
    }

    #[test]
    fn triage_rejects_reject() {
        let err = validate_transition(GoalState::Triage, GoalAction::Reject).unwrap_err();
        assert_eq!(err.from, GoalState::Triage);
    }

    #[test]
    fn review_rejects_dispatch() {
        let err = validate_transition(GoalState::Review, GoalAction::Dispatch).unwrap_err();
        assert!(err.reason.contains("'approve' or 'reject'"));
    }

    // ── Display and parsing ───────────────────────────────────────────

    #[test]
    fn state_binding_roundtrip() {
        for state in [
            GoalState::Triage,
            GoalState::Ready,
            GoalState::InProgress,
            GoalState::Review,
            GoalState::Complete,
        ] {
            assert_eq!(GoalState::from_binding(state.binding()), Some(state));
        }
    }

    #[test]
    fn unknown_binding_returns_none() {
        assert_eq!(GoalState::from_binding("unknown"), None);
    }

    #[test]
    fn action_parsing() {
        assert_eq!(GoalAction::parse_action("ready"), Some(GoalAction::Ready));
        assert_eq!(GoalAction::parse_action("dispatch"), Some(GoalAction::Dispatch));
        assert_eq!(GoalAction::parse_action("review"), Some(GoalAction::Review));
        assert_eq!(GoalAction::parse_action("approve"), Some(GoalAction::Approve));
        assert_eq!(GoalAction::parse_action("reject"), Some(GoalAction::Reject));
        assert_eq!(GoalAction::parse_action("invalid"), None);
    }

    #[test]
    fn transition_error_display_is_actionable() {
        let err = validate_transition(GoalState::Complete, GoalAction::Reject).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("'reject'"), "Should name the action: {}", msg);
        assert!(msg.contains("Complete"), "Should name the state: {}", msg);
        assert!(msg.contains("terminal"), "Should explain why: {}", msg);
    }
}
