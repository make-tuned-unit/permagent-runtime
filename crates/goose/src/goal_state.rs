//! Goal lifecycle state machine — pure validation logic.
//!
//! The goal states: Triage → Ready → InProgress → Review → Complete
//! with a bounce-back path from Review → InProgress, plus a Cancelled
//! terminal state reachable from any non-terminal state (the user abandons
//! the goal — see [`GoalAction::Cancel`]) and a Failed holding state (#250)
//! where the system parks exhausted goals (budget/timeout/credential block)
//! so a failure never masquerades as a fresh Triage goal. Failed is not
//! terminal: answering the goal's `unblock` decision retries it (Failed →
//! Ready), and the user can cancel it (Failed → Cancelled).
//!
//! This module is deliberately free of async, DB, or IO — it's a pure
//! state machine that can be unit tested without infrastructure.

use std::fmt;

/// The lifecycle states a goal card can be in. Complete and Cancelled are
/// terminal — no action advances out of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalState {
    Triage,
    Ready,
    InProgress,
    Review,
    Complete,
    /// User abandoned the goal (#490). Terminal: not active, not retried, never
    /// resurrected by resume/reaper. A running worker is killed before the card
    /// lands here.
    Cancelled,
    /// The system parked the goal after exhausting its budget (attempt cap,
    /// token budget, wallclock), a dispatch timeout, or a credential block
    /// (#250). Visibly distinct from Triage so an exhausted goal never reads
    /// as a fresh, un-triaged one. NOT terminal: approving the goal's open
    /// `unblock` decision retries it (Failed → Ready with the cap extended),
    /// and the user can cancel it (Failed → Cancelled).
    Failed,
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
            "cancelled" => Some(Self::Cancelled),
            "failed" => Some(Self::Failed),
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
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    /// True for states where Henry is actively moving the goal: Ready and
    /// InProgress only. Review is NOT active — a goal in Review is waiting on
    /// the USER (approve/reject), not on Henry, so it surfaces in the Decision
    /// Inbox rather than the "in flight" set. Triage (queued, not started) and
    /// Complete (done) are likewise not active. The single source of truth for
    /// the "in flight" set shared by every dashboard surface and the
    /// `/api/goals/active` query.
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Ready | Self::InProgress)
    }

    /// The `state_binding` strings counted as active/"in flight", kept in
    /// lockstep with [`is_active`](Self::is_active) so SQL filters and Rust
    /// agree on one definition. Review is excluded — Review goals belong to the
    /// Decision Inbox (and the Kanban Review column), not the in-flight set.
    pub const ACTIVE_BINDINGS: &'static [&'static str] = &["ready", "in_progress"];
}

impl fmt::Display for GoalState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Triage => write!(f, "Triage"),
            Self::Ready => write!(f, "Ready"),
            Self::InProgress => write!(f, "In Progress"),
            Self::Review => write!(f, "Review"),
            Self::Complete => write!(f, "Complete"),
            Self::Cancelled => write!(f, "Cancelled"),
            Self::Failed => write!(f, "Failed"),
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
    /// {Triage,Ready,InProgress,Review} → Cancelled: user abandons the goal.
    /// Deliberately NOT parseable from the MCP tool string (see
    /// [`parse_action`](Self::parse_action)): cancelling must go through the
    /// dedicated cancel path that also kills a running worker, never the bare
    /// `goal_advance` transition (which would orphan the worker subprocess).
    Cancel,
}

impl GoalAction {
    /// Parse an action string from the MCP tool parameter.
    ///
    /// `cancel` is intentionally absent: it is a user-initiated action wired to
    /// a dedicated endpoint that kills the worker first, not a transition the
    /// orchestrator can drive via `goal_advance`.
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
            Self::Cancel => write!(f, "cancel"),
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
        // Manual retry of an exhausted goal (#250): unparking (the unblock
        // decision's approve/input effect) routes Failed → Ready.
        (GoalState::Failed, GoalAction::Ready) => Ok(GoalState::Ready),
        // Cancel is legal from any non-terminal state (#490), including a
        // Failed goal the user chooses to abandon (#250). Terminal states
        // (Complete, Cancelled) fall through to the rejection arm below.
        (
            GoalState::Triage
            | GoalState::Ready
            | GoalState::InProgress
            | GoalState::Review
            | GoalState::Failed,
            GoalAction::Cancel,
        ) => Ok(GoalState::Cancelled),
        _ => {
            let valid_actions = match current {
                GoalState::Triage => "'ready' or 'cancel'",
                GoalState::Ready => "'dispatch' or 'cancel'",
                GoalState::InProgress => "'review' or 'cancel'",
                GoalState::Review => "'approve', 'reject', or 'cancel'",
                GoalState::Failed => "'ready' (retry via the unblock decision) or 'cancel'",
                GoalState::Complete => "none (terminal state)",
                GoalState::Cancelled => "none (terminal state)",
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

/// Cheapest first, by ACTUAL money spent.
///
/// A flat-rate subscription call costs nothing at the margin, so it outranks a
/// metered API however cheap that API's per-token price is (the owner's call,
/// 2026-08-09). `cheap_api` therefore sits between `subscription` and
/// `paid_api`: it is the tier that runs when the subscription CLIs are absent,
/// rate-limited, or not capable of the goal — not the default for routine work.
///
/// The consequence worth naming: the Permagent harness on Kimi/Grok/MiniMax
/// will NOT win a normal coding goal against an available Claude Code or Codex.
/// Route work to it deliberately, by mapping a role to it
/// ([`crate::cost_router::role_map`]) or by giving the worker a capability the
/// CLIs do not declare.
///
/// Unknown tiers sort last: an unrecognised string must never quietly outrank a
/// known-cheap worker.
fn cost_tier_rank(tier: &str) -> u8 {
    match tier {
        "local_free" => 0,
        "subscription" => 1,
        "cheap_api" => 2,
        "paid_api" => 3,
        _ => 4,
    }
}

// ── Roadmap decomposition (pure logic) ───────────────────────────────────

/// Maximum number of goals in a single roadmap.
pub const MAX_ROADMAP_GOALS: usize = 15;

/// A goal proposed by the LLM decomposition, before card creation.
/// Dependencies are index-based (referencing other goals in the same proposal).
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ProposedGoal {
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<usize>,
}

/// Error from roadmap validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoadmapError {
    pub message: String,
}

impl fmt::Display for RoadmapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// Compute a topological dispatch order for proposed goals.
///
/// Takes goals with index-based dependencies and returns the indices
/// in valid dispatch order (dependencies before dependents).
///
/// Rejects:
/// - More than MAX_ROADMAP_GOALS goals
/// - Dependency indices out of range
/// - Cycles (with actionable error naming the cycle path)
/// - Self-dependencies
pub fn topological_order(goals: &[ProposedGoal]) -> Result<Vec<usize>, RoadmapError> {
    if goals.is_empty() {
        return Ok(vec![]);
    }

    if goals.len() > MAX_ROADMAP_GOALS {
        return Err(RoadmapError {
            message: format!(
                "Roadmap has {} goals, exceeding the maximum of {}. \
                 Please narrow the scope or break it into multiple roadmaps.",
                goals.len(),
                MAX_ROADMAP_GOALS
            ),
        });
    }

    let n = goals.len();

    // Validate dependency indices and check for self-dependencies
    for (i, goal) in goals.iter().enumerate() {
        for &dep in &goal.depends_on {
            if dep >= n {
                return Err(RoadmapError {
                    message: format!(
                        "Goal {} ('{}') depends on index {}, but only {} goals exist (indices 0-{}).",
                        i, goal.title, dep, n, n - 1
                    ),
                });
            }
            if dep == i {
                return Err(RoadmapError {
                    message: format!("Goal {} ('{}') depends on itself.", i, goal.title),
                });
            }
        }
    }

    // Kahn's algorithm for topological sort with cycle detection
    let mut in_degree = vec![0usize; n];
    let mut adjacency: Vec<Vec<usize>> = vec![vec![]; n];

    for (i, goal) in goals.iter().enumerate() {
        for &dep in &goal.depends_on {
            adjacency[dep].push(i);
            in_degree[i] += 1;
        }
    }

    let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for (i, &deg) in in_degree.iter().enumerate() {
        if deg == 0 {
            queue.push_back(i);
        }
    }

    let mut order = Vec::with_capacity(n);
    while let Some(node) = queue.pop_front() {
        order.push(node);
        for &dependent in &adjacency[node] {
            in_degree[dependent] -= 1;
            if in_degree[dependent] == 0 {
                queue.push_back(dependent);
            }
        }
    }

    if order.len() != n {
        // Cycle detected — find nodes in the cycle for an actionable error
        let in_cycle: Vec<String> = in_degree
            .iter()
            .enumerate()
            .filter(|(_, &deg)| deg > 0)
            .map(|(i, _)| format!("'{}' (index {})", goals[i].title, i))
            .collect();

        return Err(RoadmapError {
            message: format!(
                "Dependency cycle detected among goals: {}. \
                 Remove or reorder dependencies to break the cycle.",
                in_cycle.join(", ")
            ),
        });
    }

    Ok(order)
}

// ── Post-creation roadmap graph validation (#251, pure logic) ────────────

/// A goal node in an EXISTING roadmap graph (card-id-based, unlike
/// [`ProposedGoal`]'s index-based pre-creation shape).
#[derive(Debug, Clone)]
pub struct GraphGoal {
    pub id: String,
    pub title: String,
    pub depends_on: Vec<String>,
}

/// Validate an existing roadmap's dependency graph (#251): rejects
/// self-dependencies and cycles with actionable errors naming the goals.
/// Edges to ids NOT present in `nodes` are ignored — they point at cards
/// outside the validated set (e.g. deleted or terminal goals) and cannot
/// participate in a cycle among the given nodes.
pub fn validate_goal_graph(nodes: &[GraphGoal]) -> Result<(), RoadmapError> {
    let n = nodes.len();
    if n == 0 {
        return Ok(());
    }
    let index_of: std::collections::HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, g)| (g.id.as_str(), i))
        .collect();

    let mut in_degree = vec![0usize; n];
    let mut adjacency: Vec<Vec<usize>> = vec![vec![]; n];
    for (i, node) in nodes.iter().enumerate() {
        for dep in &node.depends_on {
            if dep == &node.id {
                return Err(RoadmapError {
                    message: format!("Goal '{}' ({}) depends on itself.", node.title, node.id),
                });
            }
            if let Some(&dep_idx) = index_of.get(dep.as_str()) {
                adjacency[dep_idx].push(i);
                in_degree[i] += 1;
            }
            // Unknown dep id: edge leaves the validated set — ignore.
        }
    }

    let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for (i, &deg) in in_degree.iter().enumerate() {
        if deg == 0 {
            queue.push_back(i);
        }
    }
    let mut visited = 0usize;
    while let Some(node) = queue.pop_front() {
        visited += 1;
        for &dependent in &adjacency[node] {
            in_degree[dependent] -= 1;
            if in_degree[dependent] == 0 {
                queue.push_back(dependent);
            }
        }
    }

    if visited != n {
        let in_cycle: Vec<String> = in_degree
            .iter()
            .enumerate()
            .filter(|(_, &deg)| deg > 0)
            .map(|(i, _)| format!("'{}'", nodes[i].title))
            .collect();
        return Err(RoadmapError {
            message: format!(
                "Dependency cycle detected among goals: {}. \
                 Remove or reorder dependencies to break the cycle.",
                in_cycle.join(", ")
            ),
        });
    }
    Ok(())
}

/// Strip markdown fences and leading/trailing prose from an LLM response
/// to extract the JSON content. Matches the Librarian's resilience pattern.
#[allow(clippy::string_slice)]
pub fn extract_json_from_response(response: &str) -> Option<&str> {
    let trimmed = response.trim();

    // Try to find JSON array or object directly
    if (trimmed.starts_with('{') || trimmed.starts_with('['))
        && (trimmed.ends_with('}') || trimmed.ends_with(']'))
    {
        return Some(trimmed);
    }

    // Strip markdown fences: ```json ... ``` or ``` ... ```
    // Safe: fence markers (```) and newlines are single-byte ASCII
    if let Some(start) = trimmed.find("```") {
        let after_fence = &trimmed[start + 3..];
        let content_start = after_fence.find('\n').map(|i| i + 1).unwrap_or(0);
        let content = &after_fence[content_start..];
        if let Some(end) = content.find("```") {
            let json_str = content[..end].trim();
            if !json_str.is_empty() {
                return Some(json_str);
            }
        }
    }

    // Try to find a JSON object/array embedded in prose
    // Safe: {, }, [, ] are single-byte ASCII
    for (start_char, end_char) in [('{', '}'), ('[', ']')] {
        if let Some(start) = trimmed.find(start_char) {
            if let Some(end) = trimmed.rfind(end_char) {
                if end > start {
                    return Some(&trimmed[start..=end]);
                }
            }
        }
    }

    None
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
            GoalAction::Cancel,
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

    // ── Cancellation (#490) ───────────────────────────────────────────

    #[test]
    fn cancel_from_every_non_terminal_state() {
        for from in [
            GoalState::Triage,
            GoalState::Ready,
            GoalState::InProgress,
            GoalState::Review,
            GoalState::Failed,
        ] {
            assert_eq!(
                validate_transition(from, GoalAction::Cancel),
                Ok(GoalState::Cancelled),
                "cancel must be legal from {}",
                from
            );
        }
    }

    #[test]
    fn cancelled_is_terminal_and_rejects_all() {
        for action in [
            GoalAction::Ready,
            GoalAction::Dispatch,
            GoalAction::Review,
            GoalAction::Approve,
            GoalAction::Reject,
            GoalAction::Cancel,
        ] {
            let err = validate_transition(GoalState::Cancelled, action).unwrap_err();
            assert!(
                err.reason.contains("terminal state"),
                "Cancelled + {} should say terminal: {}",
                action,
                err.reason
            );
        }
    }

    #[test]
    fn complete_cannot_be_cancelled() {
        // Finished work is terminal — you cannot cancel a completed goal.
        let err = validate_transition(GoalState::Complete, GoalAction::Cancel).unwrap_err();
        assert!(err.reason.contains("terminal state"), "{}", err.reason);
    }

    #[test]
    fn cancel_is_not_parseable_from_mcp_string() {
        // Cancel must NOT be drivable via goal_advance's action string — it goes
        // through the dedicated worker-killing cancel path only.
        assert_eq!(GoalAction::parse_action("cancel"), None);
    }

    #[test]
    fn triage_rejects_reject() {
        let err = validate_transition(GoalState::Triage, GoalAction::Reject).unwrap_err();
        assert_eq!(err.from, GoalState::Triage);
    }

    #[test]
    fn review_rejects_dispatch() {
        let err = validate_transition(GoalState::Review, GoalAction::Dispatch).unwrap_err();
        assert!(err.reason.contains("'approve'") && err.reason.contains("'reject'"));
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
            GoalState::Cancelled,
            GoalState::Failed,
        ] {
            assert_eq!(GoalState::from_binding(state.binding()), Some(state));
        }
    }

    // ── Failed state (#250) ───────────────────────────────────────────

    #[test]
    fn failed_to_ready_is_the_manual_retry_path() {
        assert_eq!(
            validate_transition(GoalState::Failed, GoalAction::Ready),
            Ok(GoalState::Ready)
        );
    }

    #[test]
    fn failed_rejects_everything_but_ready_and_cancel() {
        for action in [
            GoalAction::Dispatch,
            GoalAction::Review,
            GoalAction::Approve,
            GoalAction::Reject,
        ] {
            let err = validate_transition(GoalState::Failed, action).unwrap_err();
            assert!(
                err.reason.contains("'ready'") && err.reason.contains("'cancel'"),
                "Failed + {} should name the valid actions: {}",
                action,
                err.reason
            );
        }
    }

    #[test]
    fn failed_is_not_active() {
        assert!(!GoalState::Failed.is_active());
        assert!(!GoalState::ACTIVE_BINDINGS.contains(&"failed"));
    }

    #[test]
    fn unknown_binding_returns_none() {
        assert_eq!(GoalState::from_binding("unknown"), None);
    }

    #[test]
    fn action_parsing() {
        assert_eq!(GoalAction::parse_action("ready"), Some(GoalAction::Ready));
        assert_eq!(
            GoalAction::parse_action("dispatch"),
            Some(GoalAction::Dispatch)
        );
        assert_eq!(GoalAction::parse_action("review"), Some(GoalAction::Review));
        assert_eq!(
            GoalAction::parse_action("approve"),
            Some(GoalAction::Approve)
        );
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

    // ── Topological sort tests ────────────────────────────────────────

    fn goal(title: &str, deps: &[usize]) -> ProposedGoal {
        ProposedGoal {
            title: title.to_string(),
            description: format!("Do {}", title),
            acceptance_criteria: vec![],
            tags: vec![],
            depends_on: deps.to_vec(),
        }
    }

    #[test]
    fn topo_linear_chain() {
        // A → B → C
        let goals = vec![goal("A", &[]), goal("B", &[0]), goal("C", &[1])];
        let order = topological_order(&goals).unwrap();
        assert_eq!(order, vec![0, 1, 2]);
    }

    #[test]
    fn topo_diamond() {
        // A → B, A → C, B → D, C → D
        let goals = vec![
            goal("A", &[]),
            goal("B", &[0]),
            goal("C", &[0]),
            goal("D", &[1, 2]),
        ];
        let order = topological_order(&goals).unwrap();
        // A must be first, D must be last, B and C can be in either order
        assert_eq!(order[0], 0, "A first");
        assert_eq!(order[3], 3, "D last");
        assert!(order[1] == 1 || order[1] == 2);
        assert!(order[2] == 1 || order[2] == 2);
        assert_ne!(order[1], order[2]);
    }

    #[test]
    fn topo_cycle_rejected() {
        // A → B → A
        let goals = vec![goal("A", &[1]), goal("B", &[0])];
        let err = topological_order(&goals).unwrap_err();
        assert!(
            err.message.contains("cycle"),
            "Should mention cycle: {}",
            err.message
        );
        assert!(
            err.message.contains("'A'") && err.message.contains("'B'"),
            "Should name the goals in the cycle: {}",
            err.message
        );
    }

    #[test]
    fn topo_self_dependency_rejected() {
        let goals = vec![goal("A", &[0])];
        let err = topological_order(&goals).unwrap_err();
        assert!(
            err.message.contains("depends on itself"),
            "Should say self-dependency: {}",
            err.message
        );
    }

    #[test]
    fn topo_cap_enforced() {
        let goals: Vec<ProposedGoal> = (0..16).map(|i| goal(&format!("G{}", i), &[])).collect();
        let err = topological_order(&goals).unwrap_err();
        assert!(
            err.message.contains("16") && err.message.contains("15"),
            "Should mention count and cap: {}",
            err.message
        );
        assert!(
            err.message.contains("narrow"),
            "Should suggest narrowing scope: {}",
            err.message
        );
    }

    #[test]
    fn topo_empty_roadmap() {
        let order = topological_order(&[]).unwrap();
        assert!(order.is_empty());
    }

    #[test]
    fn topo_single_goal_no_deps() {
        let goals = vec![goal("Solo", &[])];
        let order = topological_order(&goals).unwrap();
        assert_eq!(order, vec![0]);
    }

    #[test]
    fn topo_out_of_range_index() {
        let goals = vec![goal("A", &[99])];
        let err = topological_order(&goals).unwrap_err();
        assert!(
            err.message.contains("index 99"),
            "Should name the bad index: {}",
            err.message
        );
        assert!(
            err.message.contains("1 goals"),
            "Should say how many exist: {}",
            err.message
        );
    }

    #[test]
    fn topo_multiple_independent_chains() {
        // Chain 1: A → B, Chain 2: C → D (no cross-deps)
        let goals = vec![
            goal("A", &[]),
            goal("B", &[0]),
            goal("C", &[]),
            goal("D", &[2]),
        ];
        let order = topological_order(&goals).unwrap();
        assert_eq!(order.len(), 4);
        // A before B, C before D
        let pos_a = order.iter().position(|&x| x == 0).unwrap();
        let pos_b = order.iter().position(|&x| x == 1).unwrap();
        let pos_c = order.iter().position(|&x| x == 2).unwrap();
        let pos_d = order.iter().position(|&x| x == 3).unwrap();
        assert!(pos_a < pos_b, "A before B");
        assert!(pos_c < pos_d, "C before D");
    }

    #[test]
    fn topo_three_node_cycle() {
        // A → B → C → A
        let goals = vec![goal("A", &[2]), goal("B", &[0]), goal("C", &[1])];
        let err = topological_order(&goals).unwrap_err();
        assert!(err.message.contains("cycle"));
        // All three should be named
        assert!(err.message.contains("'A'"));
        assert!(err.message.contains("'B'"));
        assert!(err.message.contains("'C'"));
    }

    // ── Existing-roadmap graph validation (#251) ──────────────────────

    fn node(id: &str, title: &str, deps: &[&str]) -> GraphGoal {
        GraphGoal {
            id: id.to_string(),
            title: title.to_string(),
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn graph_valid_chain_and_empty() {
        assert!(validate_goal_graph(&[]).is_ok());
        let nodes = vec![
            node("a", "A", &[]),
            node("b", "B", &["a"]),
            node("c", "C", &["b", "a"]),
        ];
        assert!(validate_goal_graph(&nodes).is_ok());
    }

    #[test]
    fn graph_cycle_rejected_with_titles() {
        let nodes = vec![node("a", "Alpha", &["b"]), node("b", "Beta", &["a"])];
        let err = validate_goal_graph(&nodes).unwrap_err();
        assert!(err.message.contains("cycle"), "{}", err.message);
        assert!(
            err.message.contains("'Alpha'") && err.message.contains("'Beta'"),
            "{}",
            err.message
        );
    }

    #[test]
    fn graph_self_dependency_rejected() {
        let nodes = vec![node("a", "Alpha", &["a"])];
        let err = validate_goal_graph(&nodes).unwrap_err();
        assert!(err.message.contains("depends on itself"), "{}", err.message);
    }

    #[test]
    fn graph_unknown_dep_ids_are_ignored() {
        // Edge to a card outside the set (deleted/terminal) can't cycle.
        let nodes = vec![node("a", "A", &["gone"]), node("b", "B", &["a"])];
        assert!(validate_goal_graph(&nodes).is_ok());
    }

    // ── JSON extraction tests ─────────────────────────────────────────

    #[test]
    fn extract_json_bare() {
        let input = r#"{"goals": [{"title": "A"}]}"#;
        assert_eq!(extract_json_from_response(input), Some(input));
    }

    #[test]
    fn extract_json_with_fences() {
        let input = "Here's the roadmap:\n```json\n{\"goals\": []}\n```\nDone.";
        assert_eq!(extract_json_from_response(input), Some("{\"goals\": []}"));
    }

    #[test]
    fn extract_json_with_prose() {
        let input = "Sure! Here it is:\n{\"goals\": []}";
        assert_eq!(extract_json_from_response(input), Some("{\"goals\": []}"));
    }

    #[test]
    fn extract_json_array() {
        let input = "[{\"title\": \"A\"}]";
        assert_eq!(extract_json_from_response(input), Some(input));
    }

    #[test]
    fn extract_json_no_json() {
        assert_eq!(extract_json_from_response("just plain text"), None);
    }

    #[test]
    fn extract_json_fences_win_over_prose_braces() {
        // Prose has incidental braces BEFORE the real fenced JSON
        let input = "I considered {option A} and {option B}.\n\n```json\n{\"goals\": [{\"title\": \"Real\"}]}\n```";
        let result = extract_json_from_response(input).unwrap();
        assert!(
            result.contains("\"Real\""),
            "Should extract the fenced JSON, not the prose braces: {}",
            result
        );
    }
}
