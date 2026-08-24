//! After-turn hooks — the seam a guard attaches to when a turn wants to END.
//!
//! [`ToolInspector`](crate::tool_inspection::ToolInspector) covers the moment
//! *before* a tool runs. Nothing covered the moment *after the model decides it
//! is finished* — and that is exactly where "done" gets claimed prematurely.
//!
//! Permagent already had one guard of this shape:
//! [`decide_hold`](crate::cost_router::decide_hold), which refuses a worker's
//! "done" until a verify has actually passed. But it was welded into goal
//! dispatch — `orchestrator::maybe_hold_review`, reachable only on the goal
//! card's InProgress → Review transition. An interactive session could edit six
//! files, never run a check, and finish, with nothing to stop it.
//!
//! deepagents' `after_agent(can_jump_to=["model"])` is the same shape as
//! `HoldOutcome::Hold { inject_plan }` — an after-hook that can *re-enter the
//! model loop* instead of merely observing. This module is that seam:
//! registered on the `Agent` beside the inspector chain, consulted at the one
//! point in the reply loop where the turn would end.
//!
//! The goal path is untouched: `maybe_hold_review` still calls `decide_hold`
//! directly, and its behaviour is byte-identical. This adds a second caller of
//! the same pure decision, for the sessions that never had one.

use async_trait::async_trait;

use crate::conversation::message::Message;

/// What a hook wants to happen now that the model believes it is done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AfterTurnAction {
    /// Let the turn end.
    Allow,
    /// Do not end the turn. Inject this text as a user message and re-enter the
    /// model loop — deepagents' `can_jump_to=["model"]`.
    Continue { inject: String },
    /// Stop, but say why. The reason is surfaced to the user; the turn ends.
    Park { reason: String },
}

impl AfterTurnAction {
    /// Ordering used to fold several hooks' answers. A `Park` beats a
    /// `Continue` beats an `Allow`: the most restrictive answer wins, so one
    /// hook can never talk another out of stopping.
    fn severity(&self) -> u8 {
        match self {
            AfterTurnAction::Allow => 0,
            AfterTurnAction::Continue { .. } => 1,
            AfterTurnAction::Park { .. } => 2,
        }
    }
}

/// Everything a hook is allowed to see. Read-only by construction: a hook
/// decides, it does not mutate the turn.
pub struct AfterTurnContext<'a> {
    pub session_id: &'a str,
    /// The full conversation as it stands, including this turn.
    pub messages: &'a [Message],
    /// How many times a hook has already held THIS reply. Bounds the loop: a
    /// hook that keeps saying `Continue` must eventually be told to stop.
    pub prior_holds: u8,
}

/// A hook consulted when the model believes the turn is over.
#[async_trait]
pub trait AfterTurn: Send + Sync {
    /// Name, for logging.
    fn name(&self) -> &'static str;

    /// Decide whether this turn may end.
    async fn after_turn(&self, ctx: &AfterTurnContext<'_>) -> AfterTurnAction;

    fn is_enabled(&self) -> bool {
        true
    }
}

/// Runs the registered hooks and folds their answers.
#[derive(Default)]
pub struct AfterTurnManager {
    hooks: Vec<Box<dyn AfterTurn>>,
}

impl AfterTurnManager {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    pub fn add_hook(&mut self, hook: Box<dyn AfterTurn>) {
        self.hooks.push(hook);
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    /// Ask every enabled hook, and return the most restrictive answer.
    ///
    /// A hook that panics or errs is not given a veto it did not earn — this
    /// runs every hook and folds; there is no short-circuit, so a later hook's
    /// `Park` is never hidden by an earlier hook's `Allow`.
    pub async fn after_turn(&self, ctx: &AfterTurnContext<'_>) -> AfterTurnAction {
        let mut decision = AfterTurnAction::Allow;
        for hook in &self.hooks {
            if !hook.is_enabled() {
                continue;
            }
            let action = hook.after_turn(ctx).await;
            if action != AfterTurnAction::Allow {
                tracing::info!(
                    hook = hook.name(),
                    session_id = %ctx.session_id,
                    "after-turn hook did not let the turn end: {:?}",
                    action
                );
            }
            if action.severity() > decision.severity() {
                decision = action;
            }
        }
        decision
    }
}

// ── First implementor: the premature-"done" guard ───────────────────────────

/// Environment escape hatch. Set to `0`/`false` to disable the hold entirely.
pub const HOLD_ENV: &str = "PERMAGENT_AFTER_TURN_HOLD";

/// Refuses to end a turn that CHANGED CODE but never verified it.
///
/// The decision itself is [`decide_hold`](crate::cost_router::decide_hold) —
/// the same pure function the goal path uses, not a second copy of the policy.
/// What this adds is an **applicability gate**, and the gate is the whole
/// reason this is safe to run on an interactive session:
///
/// > A turn is only judged if it actually mutated something.
///
/// Ordinary conversation — a question answered, a file read, a search run —
/// never touches this path, because `decide_hold` is never consulted. Without
/// that gate, wiring a "you didn't run verify" hold into every chat turn would
/// hold every chat turn, which is not a guard, it is a bug.
pub struct PrematureDoneGuard;

impl PrematureDoneGuard {
    pub fn new() -> Self {
        Self
    }

    /// Whether the hold is switched on. Default on; `PERMAGENT_AFTER_TURN_HOLD=0`
    /// turns it off for a session that genuinely does not want it.
    fn enabled_by_env() -> bool {
        match std::env::var(HOLD_ENV) {
            Ok(v) => !matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            ),
            Err(_) => true,
        }
    }
}

impl Default for PrematureDoneGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// Where the last successful mutation and the last successful verify happened,
/// as positions in the flattened conversation.
///
/// Positions, not booleans, because "was there a verify?" is the wrong
/// question. A session that edits, verifies, then edits again HAS a passing
/// verify in its history — and the second edit is entirely unchecked. Only
/// "did a verify come *after* the last edit?" is a guard; the boolean is a
/// rubber stamp that any earlier green makes permanent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VerifyPositions {
    pub last_mutation: Option<usize>,
    pub last_verify: Option<usize>,
}

impl VerifyPositions {
    /// True when files were written and no successful verify has run since.
    pub fn unverified_edits(&self) -> bool {
        match (self.last_mutation, self.last_verify) {
            (None, _) => false,
            (Some(_), None) => true,
            (Some(m), Some(v)) => m > v,
        }
    }
}

/// Scan a conversation for the last successful mutating call and the last
/// successful verify call.
///
/// A tool call counts only when its response is present AND succeeded: an
/// attempted write that errored changed nothing, and a verify that came back
/// `is_error` proved nothing. "What counts as a verify" is the orchestrator's
/// own predicate, so the goal path and this one can never disagree.
///
/// Mutation detection is deliberately narrow. `shell` is excluded: `ls` and
/// `cargo test` run through the same tool as `rm`, so treating any shell call
/// as a mutation would hold turns that only looked at things.
pub fn scan_verify_positions(messages: &[Message]) -> VerifyPositions {
    use crate::agents::platform_extensions::orchestrator::is_verify_tool_name;
    use crate::conversation::message::MessageContent;
    use std::collections::HashMap;

    #[derive(Clone, Copy, PartialEq)]
    enum Kind {
        Mutation,
        Verify,
    }

    let mut pending: HashMap<String, Kind> = HashMap::new();
    let mut positions = VerifyPositions::default();
    let mut index = 0usize;

    for msg in messages {
        for content in &msg.content {
            index += 1;
            match content {
                MessageContent::ToolRequest(req) => {
                    let Ok(call) = req.tool_call.as_ref() else {
                        continue;
                    };
                    let name = call.name.as_ref();
                    // Verify wins a name that somehow matches both, so a
                    // "verify" is never miscounted as an edit.
                    if is_verify_tool_name(name) {
                        pending.insert(req.id.clone(), Kind::Verify);
                    } else if is_mutating_tool_name(name) {
                        pending.insert(req.id.clone(), Kind::Mutation);
                    }
                }
                MessageContent::ToolResponse(resp) => {
                    let Some(kind) = pending.remove(&resp.id) else {
                        continue;
                    };
                    let succeeded = resp
                        .tool_result
                        .as_ref()
                        .is_ok_and(|result| result.is_error != Some(true));
                    if !succeeded {
                        continue;
                    }
                    match kind {
                        Kind::Mutation => positions.last_mutation = Some(index),
                        Kind::Verify => positions.last_verify = Some(index),
                    }
                }
                _ => {}
            }
        }
    }
    positions
}

/// True when the conversation contains a SUCCESSFUL call to a tool that changes
/// files. The applicability gate — see [`PrematureDoneGuard`].
pub fn conversation_mutated_files(messages: &[Message]) -> bool {
    scan_verify_positions(messages).last_mutation.is_some()
}

/// Tool names whose purpose is to write to the filesystem.
fn is_mutating_tool_name(name: &str) -> bool {
    let base = name.rsplit("__").next().unwrap_or(name);
    matches!(
        base,
        "write" | "edit" | "text_editor" | "str_replace" | "create_file" | "apply_patch"
    )
}

#[async_trait]
impl AfterTurn for PrematureDoneGuard {
    fn name(&self) -> &'static str {
        "premature_done"
    }

    fn is_enabled(&self) -> bool {
        Self::enabled_by_env()
    }

    async fn after_turn(&self, ctx: &AfterTurnContext<'_>) -> AfterTurnAction {
        let positions = scan_verify_positions(ctx.messages);

        // The gate. Nothing was written, so there is nothing to have verified.
        if positions.last_mutation.is_none() {
            return AfterTurnAction::Allow;
        }

        // Not "was there ever a verify" — "has one run since the last edit".
        let verify_ran = !positions.unverified_edits();
        let signals = crate::cost_router::extract_tool_signals_from_messages(ctx.messages);

        // The SAME decision the goal path makes. An interactive session has no
        // declared workflow role, and a session that edited files without
        // verifying is mechanical work by behaviour whatever it was called.
        match crate::cost_router::decide_hold(
            crate::cost_router::WorkflowRole::Mechanical,
            verify_ran,
            &signals,
            ctx.prior_holds,
        ) {
            crate::cost_router::HoldOutcome::Allow => AfterTurnAction::Allow,
            crate::cost_router::HoldOutcome::Hold { inject_plan, .. } => {
                AfterTurnAction::Continue {
                    inject: inject_plan,
                }
            }
            crate::cost_router::HoldOutcome::Park { reason } => AfterTurnAction::Park { reason },
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::{Message, MessageContent};
    use rmcp::model::{CallToolRequestParams, CallToolResult, Content, Role};

    fn tool_exchange(name: &str, id: &str, ok: bool) -> Message {
        let result = if ok {
            CallToolResult::success(vec![Content::text("out")])
        } else {
            CallToolResult::error(vec![Content::text("failed")])
        };
        Message::new(
            Role::Assistant,
            0,
            vec![
                MessageContent::tool_request(id, Ok(CallToolRequestParams::new(name.to_string()))),
                MessageContent::tool_response(id, Ok(result)),
            ],
        )
    }

    fn ctx<'a>(messages: &'a [Message], prior_holds: u8) -> AfterTurnContext<'a> {
        AfterTurnContext {
            session_id: "sess-1",
            messages,
            prior_holds,
        }
    }

    // ── the applicability gate ──────────────────────────────────────────────

    /// The property that makes this safe to run on every session: ordinary
    /// conversation is never held.
    #[tokio::test]
    async fn a_turn_that_changed_nothing_is_always_allowed() {
        let guard = PrematureDoneGuard::new();
        let plain = vec![Message::assistant().with_text("Here is the answer.")];
        assert_eq!(
            guard.after_turn(&ctx(&plain, 0)).await,
            AfterTurnAction::Allow
        );

        let read_only = vec![
            tool_exchange("developer__search", "a", true),
            tool_exchange("developer__shell", "b", true),
        ];
        assert_eq!(
            guard.after_turn(&ctx(&read_only, 0)).await,
            AfterTurnAction::Allow,
            "reading and shelling out are not mutations"
        );
    }

    #[test]
    fn only_writing_tools_count_as_mutations() {
        for name in [
            "developer__write",
            "developer__edit",
            "text_editor",
            "mcp__x__apply_patch",
        ] {
            assert!(
                conversation_mutated_files(&[tool_exchange(name, "1", true)]),
                "{name} must count"
            );
        }
        for name in ["developer__shell", "developer__search", "read", "tree"] {
            assert!(
                !conversation_mutated_files(&[tool_exchange(name, "1", true)]),
                "{name} must not count"
            );
        }
    }

    #[test]
    fn a_failed_write_did_not_change_anything() {
        assert!(!conversation_mutated_files(&[tool_exchange(
            "developer__write",
            "1",
            false
        )]));
    }

    // ── the hold itself ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn editing_without_verifying_holds_and_re_enters_the_model_loop() {
        let guard = PrematureDoneGuard::new();
        let edited = vec![tool_exchange("developer__edit", "e1", true)];
        match guard.after_turn(&ctx(&edited, 0)).await {
            AfterTurnAction::Continue { inject } => {
                assert!(inject.contains("verify"), "inject was {inject:?}");
            }
            other => panic!("expected a Continue, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn editing_then_verifying_is_allowed_to_finish() {
        let guard = PrematureDoneGuard::new();
        let done = vec![
            tool_exchange("developer__edit", "e1", true),
            tool_exchange("developer__verify", "v1", true),
        ];
        assert_eq!(
            guard.after_turn(&ctx(&done, 0)).await,
            AfterTurnAction::Allow
        );
    }

    #[tokio::test]
    async fn a_failed_verify_does_not_count_as_verifying() {
        let guard = PrematureDoneGuard::new();
        let done = vec![
            tool_exchange("developer__edit", "e1", true),
            tool_exchange("developer__verify", "v1", false),
        ];
        assert!(matches!(
            guard.after_turn(&ctx(&done, 0)).await,
            AfterTurnAction::Continue { .. }
        ));
    }

    /// The reason positions beat a boolean. A session that edits, verifies,
    /// then edits again has a passing verify in its history and a completely
    /// unchecked second edit. The boolean form calls that done.
    #[tokio::test]
    async fn a_verify_does_not_cover_edits_made_after_it() {
        let guard = PrematureDoneGuard::new();
        let edit_verify_edit = vec![
            tool_exchange("developer__edit", "e1", true),
            tool_exchange("developer__verify", "v1", true),
            tool_exchange("developer__edit", "e2", true),
        ];
        assert!(
            scan_verify_positions(&edit_verify_edit).unverified_edits(),
            "the second edit is unverified"
        );
        assert!(
            matches!(
                guard.after_turn(&ctx(&edit_verify_edit, 0)).await,
                AfterTurnAction::Continue { .. }
            ),
            "an earlier green must not rubber-stamp a later edit"
        );

        // …and verifying again closes it.
        let mut closed = edit_verify_edit.clone();
        closed.push(tool_exchange("developer__verify", "v2", true));
        assert!(!scan_verify_positions(&closed).unverified_edits());
        assert_eq!(
            guard.after_turn(&ctx(&closed, 0)).await,
            AfterTurnAction::Allow
        );
    }

    #[test]
    fn scan_positions_reports_both_ends_and_ignores_failures() {
        let p = scan_verify_positions(&[]);
        assert_eq!(p, VerifyPositions::default());
        assert!(!p.unverified_edits());

        // A failed verify leaves the edit uncovered.
        let p = scan_verify_positions(&[
            tool_exchange("developer__edit", "e1", true),
            tool_exchange("developer__verify", "v1", false),
        ]);
        assert!(p.last_mutation.is_some());
        assert_eq!(p.last_verify, None);
        assert!(p.unverified_edits());

        // A verify with no edit at all is not an unverified edit.
        let p = scan_verify_positions(&[tool_exchange("developer__verify", "v1", true)]);
        assert!(!p.unverified_edits());
    }

    /// The loop is bounded: a hook that keeps holding is eventually made to
    /// stop and hand over, rather than pinning the session forever.
    #[tokio::test]
    async fn repeated_holds_park_instead_of_looping_forever() {
        let guard = PrematureDoneGuard::new();
        let edited = vec![tool_exchange("developer__edit", "e1", true)];
        assert!(matches!(
            guard
                .after_turn(&ctx(&edited, crate::cost_router::MAX_HOLDS))
                .await,
            AfterTurnAction::Park { .. }
        ));
    }

    #[tokio::test]
    async fn the_env_escape_hatch_disables_the_guard() {
        {
            let _g = env_lock::lock_env([(HOLD_ENV, Some("0"))]);
            assert!(!PrematureDoneGuard::new().is_enabled());
        }
        {
            let _g = env_lock::lock_env([(HOLD_ENV, Some("1"))]);
            assert!(PrematureDoneGuard::new().is_enabled());
        }
        {
            let _g = env_lock::lock_env([(HOLD_ENV, None::<&str>)]);
            assert!(
                PrematureDoneGuard::new().is_enabled(),
                "unset means on — the guard is default-on"
            );
        }
    }

    // ── the manager's fold ──────────────────────────────────────────────────

    struct Fixed(AfterTurnAction, &'static str);

    #[async_trait]
    impl AfterTurn for Fixed {
        fn name(&self) -> &'static str {
            self.1
        }
        async fn after_turn(&self, _ctx: &AfterTurnContext<'_>) -> AfterTurnAction {
            self.0.clone()
        }
    }

    #[tokio::test]
    async fn an_empty_manager_allows() {
        let m = AfterTurnManager::new();
        assert!(m.is_empty());
        assert_eq!(m.after_turn(&ctx(&[], 0)).await, AfterTurnAction::Allow);
    }

    /// The fold must be worst-wins, and must not short-circuit: an early
    /// `Allow` can never hide a later hook's `Park`.
    #[tokio::test]
    async fn the_most_restrictive_answer_wins_whatever_the_order() {
        let park = || AfterTurnAction::Park {
            reason: "stop".into(),
        };
        let cont = || AfterTurnAction::Continue {
            inject: "more".into(),
        };

        let mut m = AfterTurnManager::new();
        m.add_hook(Box::new(Fixed(AfterTurnAction::Allow, "a")));
        m.add_hook(Box::new(Fixed(cont(), "b")));
        assert_eq!(m.after_turn(&ctx(&[], 0)).await, cont());

        let mut m = AfterTurnManager::new();
        m.add_hook(Box::new(Fixed(AfterTurnAction::Allow, "a")));
        m.add_hook(Box::new(Fixed(park(), "b")));
        m.add_hook(Box::new(Fixed(cont(), "c")));
        assert_eq!(m.after_turn(&ctx(&[], 0)).await, park());

        // Reversed registration order, same verdict.
        let mut m = AfterTurnManager::new();
        m.add_hook(Box::new(Fixed(park(), "b")));
        m.add_hook(Box::new(Fixed(AfterTurnAction::Allow, "a")));
        assert_eq!(m.after_turn(&ctx(&[], 0)).await, park());
    }

    #[tokio::test]
    async fn a_disabled_hook_is_not_consulted() {
        struct Off;
        #[async_trait]
        impl AfterTurn for Off {
            fn name(&self) -> &'static str {
                "off"
            }
            fn is_enabled(&self) -> bool {
                false
            }
            async fn after_turn(&self, _c: &AfterTurnContext<'_>) -> AfterTurnAction {
                AfterTurnAction::Park {
                    reason: "never".into(),
                }
            }
        }
        let mut m = AfterTurnManager::new();
        m.add_hook(Box::new(Off));
        assert_eq!(m.after_turn(&ctx(&[], 0)).await, AfterTurnAction::Allow);
    }
}
