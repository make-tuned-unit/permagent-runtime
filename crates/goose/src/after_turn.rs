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
    /// Is the `delegate` tool (any extension prefix — `summon__delegate` or
    /// unprefixed `delegate`) actually loaded in this session's resolved tool
    /// list right now? [`ReviewerMandate`] injects an instruction to call
    /// `delegate`; without this, a recipe that dropped the `summon` extension
    /// (e.g. `extensions: [developer]`) would have the mandate ask for a tool
    /// that can never be called, then fail open through the one-ask backstop.
    /// Computed by the caller (`Agent::reply`), which already holds the
    /// resolved `Vec<Tool>` for this turn — see [`delegate_tool_loaded`].
    pub delegate_tool_available: bool,
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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerifyPositions {
    pub last_mutation: Option<usize>,
    pub last_verify: Option<usize>,
    /// X-verify-2: true when the verify recorded at `last_verify` passed but
    /// ran zero tests/assertions — a syntax-only smoke check, a "nothing
    /// configured yet" no-checks pass, or a real check that reported zero
    /// tests (see `developer::verify::vacuous_pass_reason`, the single
    /// authoritative source this mirrors). Reflects only the LATEST recorded
    /// verify: an earlier vacuous pass superseded by a later genuine one does
    /// not linger here.
    pub vacuous_pass: bool,
    /// Human-readable reason mirrored from `vacuous_pass_reason`, so a
    /// hold/park message can quote WHY the pass did not count instead of
    /// just asserting that it didn't.
    pub vacuous_reason: Option<String>,
    /// Lane R62b-3: the tool-call id of that latest vacuous pass, so a guard
    /// that refuses it can record the refusal against that exact call
    /// (`developer::verify::record_vacuous_pass_judgement`). `None` whenever
    /// `vacuous_pass` is false.
    pub vacuous_call_id: Option<String>,
}

impl VerifyPositions {
    /// True when files were written and no successful, NON-VACUOUS verify has
    /// run since. X-verify-2: a vacuous pass (zero tests/assertions) is
    /// treated exactly like no verify at all — a green "PASS" that proved
    /// nothing must never rubber-stamp an edit the way a genuine test run
    /// does. Gated on there being a recorded mutation AND verify at all, so a
    /// vacuous verify with nothing ever edited (ordinary conversation) still
    /// correctly reads as "nothing to verify", not "unverified".
    pub fn unverified_edits(&self) -> bool {
        match (self.last_mutation, self.last_verify) {
            (None, _) => false,
            (Some(_), None) => true,
            (Some(m), Some(v)) => m > v || self.vacuous_pass,
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
    use crate::conversation::message::{tool_requests_by_id, MessageContent};
    use std::collections::HashMap;

    #[derive(Clone, Copy, PartialEq)]
    enum Kind {
        Mutation,
        Verify,
    }

    // Lane R62a: index every request by tool-call id BEFORE pairing, so a
    // conversation reloaded from the `messages` table with a response ahead
    // of its own request still pairs (see `tool_requests_by_id`). Positions
    // are still the RESPONSE's position, exactly as before.
    let mut pending: HashMap<&str, Kind> =
        tool_requests_by_id(messages.iter().flat_map(|msg| &msg.content))
            .into_iter()
            .filter_map(|(id, req)| {
                let name = req.tool_call.as_ref().ok()?.name.as_ref();
                // Verify wins a name that somehow matches both, so a
                // "verify" is never miscounted as an edit.
                if is_verify_tool_name(name) {
                    Some((id, Kind::Verify))
                } else if is_mutating_tool_name(name) {
                    Some((id, Kind::Mutation))
                } else {
                    None
                }
            })
            .collect();
    let mut positions = VerifyPositions::default();
    let mut index = 0usize;

    for msg in messages {
        for content in &msg.content {
            index += 1;
            let MessageContent::ToolResponse(resp) = content else {
                continue;
            };
            use crate::agents::platform_extensions::developer::verify;

            let Some(kind) = pending.remove(resp.id.as_str()) else {
                continue;
            };
            match kind {
                Kind::Mutation => {
                    let succeeded = resp
                        .tool_result
                        .as_ref()
                        .is_ok_and(|result| result.is_error != Some(true));
                    if succeeded {
                        positions.last_mutation = Some(index);
                    }
                }
                Kind::Verify => {
                    let Ok(result) = resp.tool_result.as_ref() else {
                        continue;
                    };
                    if !verify::is_authoritative_pass(result) {
                        continue;
                    }
                    positions.last_verify = Some(index);
                    // X-verify-2: reflects the LATEST recorded verify only —
                    // overwritten every time, same as `last_verify` itself.
                    positions.vacuous_reason = verify::vacuous_pass_reason(result);
                    positions.vacuous_pass = positions.vacuous_reason.is_some();
                    positions.vacuous_call_id = positions.vacuous_pass.then(|| resp.id.clone());
                }
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
pub fn is_mutating_tool_name(name: &str) -> bool {
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
        let outcome = crate::cost_router::decide_hold(
            crate::cost_router::WorkflowRole::Mechanical,
            verify_ran,
            &signals,
            ctx.prior_holds,
        );
        // Lane R62b-3: holding or parking on a vacuous pass is this guard
        // refusing to credit it as verify evidence.
        if !matches!(outcome, crate::cost_router::HoldOutcome::Allow) {
            if let Some(call_id) = positions.vacuous_call_id.as_deref() {
                crate::agents::platform_extensions::developer::verify::record_vacuous_pass_judgement(
                    ctx.session_id,
                    call_id,
                    false,
                );
            }
        }
        match outcome {
            crate::cost_router::HoldOutcome::Allow => AfterTurnAction::Allow,
            crate::cost_router::HoldOutcome::Hold { inject_plan, .. } => {
                // X-verify-2: a vacuous pass is not "no verify at all" —
                // `decide_hold`'s generic "run verify" text would be actively
                // misleading here (verify DID run and DID report PASS; it
                // just proved nothing). Say so honestly instead.
                let inject = if positions.vacuous_pass {
                    vacuous_pass_hold_text(positions.vacuous_reason.as_deref())
                } else {
                    inject_plan
                };
                let inject = with_closing_reply_note(inject);
                AfterTurnAction::Continue { inject }
            }
            crate::cost_router::HoldOutcome::Park { reason } => {
                let reason = if positions.vacuous_pass {
                    vacuous_pass_park_reason(positions.vacuous_reason.as_deref())
                } else {
                    reason
                };
                AfterTurnAction::Park { reason }
            }
        }
    }
}

/// Appended to every hold this guard injects. The hold is for the harness
/// (verify before done); it must not leak into the user's closing message. A
/// plain chat turn that wrote a file otherwise ends with a narration of each
/// verify attempt instead of the short reply the user's instructions asked for.
/// The discipline (run verify, fix, run again) is unchanged; only the final
/// reply's shape is stated.
const CLOSING_REPLY_NOTE: &str = " When verify passes, close with one or two short sentences \
for the user saying what they now have; do not narrate the verify attempts.";

fn with_closing_reply_note(mut inject: String) -> String {
    inject.push_str(CLOSING_REPLY_NOTE);
    inject
}

/// X-verify-2: honest hold text for a vacuous pass — verify DID run and DID
/// report PASS; it just proved nothing (`reason` names why, e.g. "only a
/// syntax-only smoke check ran" or "the check's own output reports zero
/// tests ran"), so `decide_hold`'s generic "run verify" text (written for
/// the "no verify ran at all" case) would be actively misleading here.
fn vacuous_pass_hold_text(reason: Option<&str>) -> String {
    let reason = reason.unwrap_or("it ran zero tests/assertions");
    format!(
        "Do not declare this done. The last verify PASSED, but {reason} — that does not count \
         as verifying this change. Run (or fix) a check that actually exercises the edited \
         code, then verify again."
    )
}

/// X-verify-2: the honest Park reason once the hold budget is exhausted on a
/// repeatedly-vacuous pass.
fn vacuous_pass_park_reason(reason: Option<&str>) -> String {
    let reason = reason.unwrap_or("it ran zero tests/assertions");
    format!(
        "held twice without a real verify — the last verify PASSED, but {reason} — handing to \
         the Decision Inbox"
    )
}

// ── Second implementor: the independent-reviewer mandate ───────────────────

/// Config key for [`ReviewerMandate`]. Default ON (unlike the opt-in
/// `strix_enabled`-style flags): a coding harness that mandates review in its
/// own recipe prose should not need a second switch to make that real. Read via
/// `Config::global().get_param::<bool>(REVIEWER_MANDATE_KEY)`, which — the house
/// pattern — also honours the `REVIEWER_MANDATE` env var automatically.
pub const REVIEWER_MANDATE_KEY: &str = "reviewer_mandate";

/// Prefix stamped on every [`AfterTurnAction::Park`] reason this hook raises.
/// `goose-cli::session::output::render_review_notice` matches this prefix to
/// render the "no independent review" notice as its own labelled block instead
/// of plain assistant text — see that module. Keep the two in sync.
pub const REVIEW_PARK_PREFIX: &str = "independent review did not run: ";

/// Opening sentence of the injected ask. Written once, then recognised again on
/// the next pass through [`ReviewerMandate::after_turn`], which is how this hook
/// counts ITS OWN holds.
///
/// It cannot use `AfterTurnContext::prior_holds` for that: the reply loop keeps
/// ONE counter for all hooks, so a hold [`PrematureDoneGuard`] spent sending the
/// model back to verify would read here as "I already asked for a review" and
/// the mandate would go quiet for the rest of the turn — reinstating exactly the
/// silence this hook exists to end. The injected message is a user message in
/// the conversation, so looking for it is both hook-local and durable.
const REVIEW_ASK_OPENING: &str =
    "Before finishing: this turn changed files and has not been independently reviewed yet.";

/// Enforces the `permagent-coding` recipe's "summon the reviewer before you
/// finish" mandate — prose the model could, and per a 20-run benchmark DID,
/// simply skip. Where [`PrematureDoneGuard`] refuses to end a turn that edited
/// files without a passing verify, this refuses to end a turn that edited files
/// and verified WITHOUT having asked an independent, cross-family model to
/// review the diff.
///
/// Enforcement is by injection, not by inventing a second execution path: the
/// hook tells the model to call the SAME `delegate` tool the recipe already
/// names, with `worker_persona: "reviewer"`. That routes through
/// `summon::resolve_provider`'s existing `WorkflowRole::Review` handling —
/// `cost_router::reviewer_dispatch` — so the reviewer's provider pick is the
/// same cross-family, cost-capped, fail-closed choice PR #1106 already ships,
/// and the review's own cost lands in the ledger under ITS OWN (subagent)
/// session, not silently folded into the author's. What this hook adds is
/// narrower and honest about its limit: the turn cannot end without the
/// delegation having been MADE, or a [`AfterTurnAction::Park`] that says
/// plainly why it was not. It cannot make the reviewer answer well — a model
/// can still rubber-stamp or hallucinate a verdict — only that asking it
/// happened.
///
/// Mirrors [`PrematureDoneGuard`]'s split: the live async half
/// ([`ReviewerMandate::assess`]) does the one database read and the one
/// best-effort `git diff --shortstat` needed to pick a reviewer and price it;
/// the actual decision ([`decide`]) is a pure function of already-resolved
/// inputs, exercised directly by the unit tests below so they never touch a
/// database — `ReviewerAvailability` there is built from the SAME pure
/// `cost_router::select_reviewer` / `reviewer_spend_gate` the live path calls.
pub struct ReviewerMandate;

impl ReviewerMandate {
    pub fn new() -> Self {
        Self
    }

    fn enabled_by_config() -> bool {
        crate::config::Config::global()
            .get_param::<bool>(REVIEWER_MANDATE_KEY)
            .unwrap_or(true)
    }

    /// E1: is the pending diff trivial enough that the recipe's own contract
    /// (`permagent-coding.yaml:88-91`, "skip trivial / test-only / docs-only /
    /// formatting diffs") says to skip the mandate outright? Reads the
    /// session's working dir to compute the actual `git diff`; the pure
    /// decision is [`trivial_skip_reason_for_dir`], which a test drives
    /// directly against a scratch repo. E2a: routed through
    /// [`trivial_skip_reason_for_dir_cached`] so a hold-recheck loop that
    /// re-consults this hook several times over the SAME, unedited content
    /// does not re-shell-out for `git diff`/`git ls-files` every time.
    async fn trivial_skip_reason(session_id: &str) -> Option<String> {
        if !trivial_skip_enabled_by_env() {
            return None;
        }
        let manager = crate::session::SessionManager::instance();
        let session = manager.get_session(session_id, false).await.ok()?;
        trivial_skip_reason_for_dir_cached(session_id, &session.working_dir).await
    }

    /// The live half: read this session's record for its author (provider,
    /// model) and working dir, size the pending change, then run the SAME
    /// reviewer selection + spend gate the goal path's `select_for_goal` /
    /// `spend_for_goal` compose (`goose-server::verification::review`) — that
    /// module cannot be called from here (this crate does not depend on
    /// `goose-server`, and does not want to: that IS problem #2 this feature
    /// exists to fix for the CLI), so the same small composition of
    /// `cost_router` primitives is repeated here rather than reached for.
    async fn assess(session_id: &str) -> ReviewerAvailability {
        let manager = crate::session::SessionManager::instance();
        let session = match manager.get_session(session_id, false).await {
            Ok(s) => s,
            Err(e) => {
                return ReviewerAvailability::Unavailable {
                    reason: format!(
                        "this session's record could not be read to pick an independent \
                         reviewer ({e})"
                    ),
                }
            }
        };

        let worker = match (
            session.provider_name.as_deref(),
            session.model_config.as_ref().map(|m| m.model_name.as_str()),
        ) {
            (Some(p), Some(m)) if !p.is_empty() && !m.is_empty() => {
                Some((p.to_string(), m.to_string()))
            }
            _ => None,
        };
        let lines = changed_lines(&session.working_dir).await;

        let configured = crate::cost_router::role_model(crate::cost_router::WorkflowRole::Review);
        let derived_map = crate::cost_router::derived_role_map().await;
        let derived = derived_map
            .get(crate::cost_router::WorkflowRole::Review)
            .map(|(rm, _)| rm.clone());
        let available = crate::cost_router::discover_available_models_async().await;

        let selection = crate::cost_router::select_reviewer(
            worker.as_ref().map(|(p, m)| (p.as_str(), m.as_str())),
            configured.as_ref(),
            derived.as_ref(),
            &available,
            lines,
        );

        let pick = match selection {
            crate::cost_router::ReviewerSelection::Unavailable { reason } => {
                return ReviewerAvailability::Unavailable { reason }
            }
            crate::cost_router::ReviewerSelection::Reviewer(pick) => pick,
        };

        // Same-scope spend the goal path uses: the AUTHOR session's own running
        // cost against both ceilings (there is no separate "task" scope for an
        // interactive session, so — like `spend_for_goal` — one figure serves
        // both budget scopes `reviewer_spend_gate` checks, and now also the
        // review's own cross-family escalation gate below).
        let spent = session.accumulated_cost_usd.unwrap_or(0.0);
        let cfg = crate::cost_router::budget::load_budget_config();
        let verdict = crate::cost_router::budget_verdict(spent, spent, &cfg);

        // X-cost-2: what the injected text will SAY, resolved through the
        // IDENTICAL decision function the REAL dispatch uses
        // (`summon::resolve_provider`'s `resolve_review_dispatch`, both
        // calling `cost_router::resolve_actual_review_dispatch`) — reusing
        // `derived_map`/`derived` (already fetched above for `select_reviewer`)
        // so this costs no extra probe, unlike calling `delegate_routing_live`
        // a second time.
        let author = worker.as_ref().map(|(p, m)| (p.as_str(), m.as_str()));
        let routing = crate::cost_router::delegate_routing(
            Some(crate::cost_router::WorkflowRole::Review),
            &derived_map,
            None,
            author,
        );
        let routing_is_role_pin = routing.source == crate::cost_router::DelegateSource::RolePin;
        let review_pin = if routing_is_role_pin {
            None
        } else {
            crate::cost_router::review_pack_pin().map(crate::cost_router::pack_to_role_model)
        };
        // Lane R63c / Lane N3: the SAME cross-family candidate the real
        // dispatch (`summon::resolve_review_dispatch`) escalates to — the
        // objective assignment for this review from the goal's trunk-relative
        // diff (`orchestrator::assign_delegate_reviewer`, ruling 2026-09-17),
        // not the derived map's strongest REVIEW entry nor a `HEAD`-relative
        // size — so this prediction cannot promise a route the dispatch will
        // not take.
        let cross_family_candidate = if routing_is_role_pin || review_pin.is_some() {
            None
        } else {
            let pool = manager.pool_clone().await.ok();
            crate::agents::platform_extensions::orchestrator::assign_delegate_reviewer(
                pool.as_ref(),
                &session,
                author,
                &available,
            )
            .await
            .assignment
            .pick_role_model()
        };
        let dispatch = crate::cost_router::resolve_actual_review_dispatch(
            routing_is_role_pin,
            routing.role_model.clone(),
            author,
            review_pin,
            cross_family_candidate,
            &|p| crate::cost_router::is_provider_configured(p),
            Some(verdict.band),
        );
        let actual_route = actual_reviewer_route(&dispatch, author);

        match crate::cost_router::reviewer_spend_gate(&pick, &verdict) {
            crate::cost_router::SpendDecision::Allow => {
                ReviewerAvailability::Ready(pick, actual_route)
            }
            // `reason` already names the refused reviewer (`reviewer_spend_gate`
            // formats it as "the reviewer (provider/model) has no published
            // price…" / the budget-band message) — nothing else to carry.
            crate::cost_router::SpendDecision::Refuse { reason } => {
                ReviewerAvailability::SpendRefused { reason }
            }
        }
    }
}

impl Default for ReviewerMandate {
    fn default() -> Self {
        Self::new()
    }
}

/// What [`ReviewerMandate::assess`] found when it asked whether an independent
/// review can run right now, and — if it can — whether it may be paid for.
/// Tests build this directly (from the real, pure `select_reviewer` /
/// `reviewer_spend_gate`), so [`decide`] is exercised without a database.
#[derive(Debug, Clone)]
enum ReviewerAvailability {
    /// A reviewer was chosen and its spend is allowed. Carries the pick
    /// (capability/pricing) alongside what the delegate call will ACTUALLY
    /// dispatch to (see [`ActualReviewerRoute`]) — two different resolutions,
    /// because `select_reviewer` and `summon`'s dispatch precedence can (and,
    /// per Lane A's ledger, reliably do) disagree.
    Ready(Box<crate::cost_router::ReviewerPick>, ActualReviewerRoute),
    /// A reviewer was chosen but its spend must be refused (unpriced, or a
    /// Gate/Hard budget band) — never billed as free, never run anyway.
    SpendRefused { reason: String },
    /// No reviewer could be chosen at all (no cross-family model available).
    Unavailable { reason: String },
}

/// What the `delegate` call the mandate is about to inject will ACTUALLY
/// route to — resolved through the IDENTICAL decision function
/// `summon::resolve_provider` calls for `worker_persona: "reviewer"`
/// (`cost_router::resolve_actual_review_dispatch`), not the independent
/// `select_reviewer` pick used for capability/spend gating.
///
/// E3 / defect 1a (original defect): `select_reviewer` has no notion of
/// `PERMAGENT_PACK_*` pins, so it could promise a genuinely cross-family
/// model while the actual dispatch — which honours a `PERMAGENT_PACK_HARD_*`
/// pin at a higher precedence than cross-family escalation
/// (`delegate.rs`'s table, level 4 over level 5) — landed the reviewer on the
/// author's own family. Lane A's ledger showed this 3/3 times the reviewer
/// actually ran. X-cost-2 closed the ROOT cause (the real dispatch now runs
/// review's own precedence — a dedicated `PERMAGENT_PACK_REVIEW_*` pin, else
/// genuine cross-family escalation, else an honestly-labeled same-family
/// fallback — instead of silently inheriting `Role::Hard`'s pin); this struct
/// still exists because `select_reviewer`'s independent pick and the real
/// dispatch's resolution remain two different code paths that COULD diverge
/// again, and the mandate must never promise independence the dispatch will
/// not deliver. `same_family` is `true` whenever the real dispatch will not
/// give an independent, different-family review — either because the
/// resolved model shares the author's family, or because nothing routes it
/// anywhere but the session model — so the injected text can say so plainly
/// instead of promising independence it will not get.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ActualReviewerRoute {
    label: String,
    same_family: bool,
}

/// Pure: derive the actual route from the delegate-side dispatch decision.
/// Exercised directly by tests with a hand-built [`crate::cost_router::ReviewerDispatch`]
/// so this never needs a database or a live pack-pin environment to pin the
/// defect.
fn actual_reviewer_route(
    dispatch: &crate::cost_router::ReviewerDispatch,
    author: Option<(&str, &str)>,
) -> ActualReviewerRoute {
    let label = match &dispatch.role_model {
        Some(rm) => format!("{}/{}", rm.provider, rm.model),
        None => author
            .map(|(p, m)| format!("{p}/{m}"))
            .unwrap_or_else(|| "your model".to_string()),
    };
    ActualReviewerRoute {
        label,
        same_family: dispatch.warning.is_some(),
    }
}

/// Best-effort "how big is the pending change" signal for `select_reviewer`'s
/// capability floor: `git diff --shortstat HEAD` in the session's working dir,
/// summed insertions+deletions. Mirrors `orchestrator::capture_worktree_diff`'s
/// discipline (bounded, best-effort, a non-repo or git error is never fatal) —
/// a failure here yields `0`, the SMALL-diff floor, which is the LENIENT
/// default: worst case the mandate asks for a slightly weaker reviewer than an
/// exact count would, never a reason to block the review outright.
pub(crate) async fn changed_lines(working_dir: &std::path::Path) -> usize {
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(working_dir)
        .args(["diff", "--shortstat", "HEAD"])
        .output()
        .await;
    match output {
        Ok(o) if o.status.success() => parse_shortstat(&String::from_utf8_lossy(&o.stdout)),
        _ => 0,
    }
}

/// Parse `git diff --shortstat`'s one-line summary (e.g. "3 files changed, 12
/// insertions(+), 4 deletions(-)") into a total changed-line count. A clause
/// that does not parse (or a rename-only diff missing one clause entirely)
/// simply contributes 0 rather than failing the whole parse.
fn parse_shortstat(text: &str) -> usize {
    let mut total = 0usize;
    for clause in text.split(',') {
        let clause = clause.trim();
        let number_part = clause
            .strip_suffix("insertions(+)")
            .or_else(|| clause.strip_suffix("insertion(+)"))
            .or_else(|| clause.strip_suffix("deletions(-)"))
            .or_else(|| clause.strip_suffix("deletion(-)"));
        if let Some(n) = number_part.and_then(|n| n.trim().parse::<usize>().ok()) {
            total += n;
        }
    }
    total
}

/// E1 knob: whether the recipe's own trivial/docs-only/test-only/format-only
/// review skip (`cost_router::review_required`) applies on this hook, the
/// same way it already does on the goal-dispatch path
/// (`goose-server::verification::review`). Default ON. A raw `std::env::var`
/// read, not the `Config`/`get_param` house pattern used elsewhere in this
/// file — deliberately, so an eval driver (`permagent-eval ab --knob`) can
/// flip it per child process without touching global config precedence.
pub const REVIEWER_MANDATE_SKIP_TRIVIAL_ENV: &str = "PERMAGENT_REVIEWER_MANDATE_SKIP_TRIVIAL";

fn trivial_skip_enabled_by_env() -> bool {
    match std::env::var(REVIEWER_MANDATE_SKIP_TRIVIAL_ENV) {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

/// Tracked + untracked paths changed relative to `HEAD` in `working_dir`'s
/// repository — mirrors [`changed_lines`]'s discipline exactly: a git failure
/// yields an empty list, which makes [`crate::cost_router::review_required`]
/// answer `Review` (an empty path list looks like "no product source
/// touched," which is also a skip — but an empty list only ever happens on a
/// git error here, since a real trivial-or-skippable diff still names its own
/// paths, so this never manufactures a false skip in practice; the
/// leniency mirrors `changed_lines`' own worst-case of a weaker reviewer
/// rather than a blocked one).
async fn changed_paths(working_dir: &std::path::Path) -> Vec<String> {
    let tracked = tokio::process::Command::new("git")
        .arg("-C")
        .arg(working_dir)
        .args(["diff", "--name-only", "HEAD"])
        .output()
        .await;
    let untracked = tokio::process::Command::new("git")
        .arg("-C")
        .arg(working_dir)
        .args(["ls-files", "--others", "--exclude-standard"])
        .output()
        .await;
    let mut paths = Vec::new();
    if let Ok(o) = &tracked {
        if o.status.success() {
            paths.extend(
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(str::to_string),
            );
        }
    }
    if let Ok(o) = &untracked {
        if o.status.success() {
            paths.extend(
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(str::to_string),
            );
        }
    }
    paths.retain(|p| !p.trim().is_empty());
    paths
}

/// E1: the actual skip decision for a working directory — split out from
/// [`ReviewerMandate::trivial_skip_reason`] so a unit test can drive it with a
/// real scratch git repo instead of a live, database-backed session record.
async fn trivial_skip_reason_for_dir(working_dir: &std::path::Path) -> Option<String> {
    if !trivial_skip_enabled_by_env() {
        return None;
    }
    let paths = changed_paths(working_dir).await;
    let lines = changed_lines(working_dir).await;
    match crate::cost_router::review_required(&paths, lines) {
        crate::cost_router::ReviewTrigger::Skip { reason } => Some(reason),
        crate::cost_router::ReviewTrigger::Review => None,
    }
}

/// E2a: the "hold-recheck" caching wrapper around [`trivial_skip_reason_for_dir`].
///
/// `ReviewerMandate::after_turn` re-runs `trivial_skip_reason` (three `git`
/// subprocess calls: one `--shortstat` diff, one `--name-only` diff, one
/// `ls-files --others`) on EVERY turn it is consulted while a hold is
/// pending — the mandate's own "hold-recheck loop", the same repeated-
/// recheck shape G7's review flagged for the goal-dispatch credit path. A
/// worker can be held/re-prompted several times in a row with no new commit
/// in between (waiting on a reviewer, retrying an unrelated tool call), and
/// each of those turns pays for the same three `git` calls against content
/// that has not moved.
///
/// `AfterTurnContext` carries no `goal_id` (this guard is session-scoped, not
/// goal-scoped) — `session_id` is used as the cache-key dimension instead.
/// This is a sound substitute for this call site specifically: E1's own
/// forensics established that a dispatched worker session maps to at most
/// one goal attempt at a time, so "never cache across goals" is preserved by
/// "never cache across sessions" here.
///
/// The underlying [`trivial_skip_reason_for_dir`] returns `Option<String>`
/// (a reason on skip, `None` on "still needs review") — richer than
/// [`crate::check_cache::CachedCheckResult`]'s bare `passed: bool`. Rather
/// than widen the shared cache-result shape for one caller, the eligibility
/// bit alone is cached (`passed` = "skip is eligible") and the real reason
/// text is captured out-of-band on a genuine (non-cached) run; a cache HIT
/// still returns a real, honest `Some`/`None` — just with a generic reason
/// string on the `Some` side, since the specific wording was already logged
/// on the run that produced it and does not affect `after_turn`'s control
/// flow (Allow-vs-not), which only depends on `Some` vs `None`.
async fn trivial_skip_reason_for_dir_cached(
    session_id: &str,
    working_dir: &std::path::Path,
) -> Option<String> {
    // I1 (harness loop, 2026-09-10, gate fix): `RefCell` is not `Sync`, so a
    // `&RefCell<_>` captured across the `async move` block below is not
    // `Send` — and `AfterTurnHook::after_turn` requires its returned future
    // be `Send`. `std::sync::Mutex` is the `Send`+`Sync` equivalent for this
    // single-threaded-in-practice, briefly-locked use (write once inside the
    // closure, read once after `get_or_run` returns); never contended.
    let captured_reason: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    // A `&Mutex<_>` is `Copy`, so moving it into the closure/future below
    // (needed so the future does not borrow the outer closure's stack frame)
    // leaves `captured_reason` itself fully usable afterward.
    let captured_reason_ref = &captured_reason;
    let (result, was_cache_hit) = crate::check_cache::get_or_run(
        session_id,
        "after_turn::reviewer_mandate::trivial_skip",
        working_dir,
        move || async move {
            let reason = trivial_skip_reason_for_dir(working_dir).await;
            *captured_reason_ref
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = reason.clone();
            crate::check_cache::CachedCheckResult {
                passed: reason.is_some(),
            }
        },
    )
    .await;

    if was_cache_hit {
        result.passed.then(|| {
            "trivial-skip eligible (cached; see the earlier receipt for the exact reason)"
                .to_string()
        })
    } else {
        captured_reason
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// True when a `delegate` (or `*__delegate`) call carrying
/// `worker_persona: "reviewer"` (case-insensitive) appears anywhere after
/// `after_position` — the SAME flattened-conversation position scheme
/// [`scan_verify_positions`] uses, so "after the last mutation" means the same
/// thing here as it does there. A request is not proof: only a successful tool
/// response whose canonical `VERDICT: APPROVE` parses as approval satisfies
/// the mandate.
fn reviewer_approved_after(messages: &[Message], after_position: usize) -> bool {
    use crate::conversation::message::MessageContent;
    use crate::cost_router::review_gate::{parse_review, Verdict};
    use std::collections::HashSet;

    let mut index = 0usize;
    let mut pending = HashSet::new();
    for msg in messages {
        for content in &msg.content {
            index += 1;
            if index <= after_position {
                continue;
            }
            if let MessageContent::ToolRequest(req) = content {
                if let Ok(call) = req.tool_call.as_ref() {
                    if is_reviewer_delegation(call) {
                        pending.insert(req.id.clone());
                    }
                }
            } else if let MessageContent::ToolResponse(response) = content {
                if !pending.remove(&response.id) {
                    continue;
                }
                let Ok(result) = response.tool_result.as_ref() else {
                    continue;
                };
                if result.is_error == Some(true) {
                    continue;
                }
                let text = result
                    .content
                    .iter()
                    .filter_map(|content| content.as_text().map(|text| text.text.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n");
                if parse_review(&text).verdict == Verdict::Approve {
                    return true;
                }
            }
        }
    }
    false
}

/// Does this tool call delegate to the `reviewer` worker persona?
fn is_reviewer_delegation(call: &rmcp::model::CallToolRequestParams) -> bool {
    let base = call.name.rsplit("__").next().unwrap_or(call.name.as_ref());
    if base != "delegate" {
        return false;
    }
    call.arguments
        .as_ref()
        .and_then(|args| args.get("worker_persona"))
        .and_then(|v| v.as_str())
        .is_some_and(|p| p.eq_ignore_ascii_case("reviewer"))
}

/// What the LAST `delegate(worker_persona: "reviewer")` call after
/// `after_position` actually came back with. E5 / defect 1b: Lane A observed a
/// real, billed reviewer subagent call return a completely empty answer (or an
/// error) — no verdict, approving or otherwise — which the mandate then
/// treated identically to a genuine "reviewed and rejected" outcome and parked
/// immediately. This distinguishes the two so an empty/errored call can be
/// retried instead.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ReviewerLastOutcome {
    /// No `delegate(reviewer)` call at all after `after_position`.
    NotCalled,
    /// A call ran and returned real, non-empty content — whatever verdict it
    /// carries (or fails to parse), something was actually reviewed.
    Answered,
    /// A call ran but its result was empty or an error: nothing was actually
    /// reviewed despite the call happening.
    Empty,
    /// X-verify-1: the reviewer mutated the working tree — `summon` detected
    /// this via a before/after filesystem snapshot and stamped the result
    /// with [`REVIEWER_MUTATION_VOID_PREFIX`] instead of trusting whatever
    /// verdict the reviewer's own text carries. Carries the mutated paths so
    /// the re-hold text can name them.
    Void(Vec<String>),
}

fn last_reviewer_outcome(messages: &[Message], after_position: usize) -> ReviewerLastOutcome {
    use crate::conversation::message::MessageContent;
    use std::collections::HashSet;

    let mut index = 0usize;
    let mut pending = HashSet::new();
    let mut outcome = ReviewerLastOutcome::NotCalled;
    for msg in messages {
        for content in &msg.content {
            index += 1;
            if index <= after_position {
                continue;
            }
            if let MessageContent::ToolRequest(req) = content {
                if let Ok(call) = req.tool_call.as_ref() {
                    if is_reviewer_delegation(call) {
                        pending.insert(req.id.clone());
                    }
                }
            } else if let MessageContent::ToolResponse(response) = content {
                if !pending.remove(&response.id) {
                    continue;
                }
                outcome = match response.tool_result.as_ref() {
                    Ok(result) => {
                        let text = result
                            .content
                            .iter()
                            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
                            .collect::<Vec<_>>()
                            .join("\n");
                        if let Some(paths) = reviewer_mutation_void_paths(&text) {
                            ReviewerLastOutcome::Void(paths)
                        } else if result.is_error == Some(true) || text.trim().is_empty() {
                            ReviewerLastOutcome::Empty
                        } else {
                            ReviewerLastOutcome::Answered
                        }
                    }
                    Err(_) => ReviewerLastOutcome::Empty,
                };
            }
        }
    }
    outcome
}

/// X-verify-1: sentinel line `summon::handle_delegate` stamps as the FIRST
/// line of a reviewer's tool-result text once a before/after working-tree
/// snapshot (`reviewer_integrity::snapshot`/`diff`) shows the reviewer
/// subagent mutated files despite its READ-ONLY mandate (F2/H1) — Lane A2's
/// live evidence ("a reviewer subagent called `edit` on `src/cache.rs` and
/// the change shipped") and Lane G finding 2 (the reviewer keeps `shell`,
/// which can mutate via `sed -i`/redirection/`rm` regardless of the tool
/// allowlist). `parse_review`'s "first VERDICT wins, a later stray token
/// cannot override it" rule means stamping this (with its own `VERDICT:
/// UNCERTAIN` line) BEFORE the reviewer's own text always wins, even if that
/// text itself says `VERDICT: APPROVE` — the void is not a paint-over the
/// model could talk itself past.
pub const REVIEWER_MUTATION_VOID_PREFIX: &str = "REVIEWER_MUTATED_FILES_VERDICT_VOID: ";

/// Build the text `summon::handle_delegate` substitutes for a reviewer's real
/// answer once [`reviewer_integrity::diff`] finds a non-empty mutation. Comma-
/// joins `paths` after [`REVIEWER_MUTATION_VOID_PREFIX`] (parsed back out by
/// [`reviewer_mutation_void_paths`]); the original text is kept below a
/// divider for audit purposes but is never trusted for its verdict.
pub fn reviewer_mutation_void_text(paths: &[String], original: &str) -> String {
    let joined = paths.join(", ");
    format!(
        "{REVIEWER_MUTATION_VOID_PREFIX}{joined}\n\
         VERDICT: UNCERTAIN\n\
         CHECKED: nothing — the reviewer modified the working tree instead of only \
         reviewing it, so its verdict is void.\n\
         A read-only reviewer must never edit, write, or otherwise change a file (including \
         through `shell`); call `delegate` again with `worker_persona: \"reviewer\"` for a \
         genuine, non-mutating review.\n\n\
         --- original reviewer output (voided; do not trust its verdict) ---\n{original}"
    )
}

/// Parse the mutated-paths list back out of a reviewer result
/// [`reviewer_mutation_void_text`] stamped. `None` when the sentinel is not
/// the FIRST line — a reviewer's own text merely mentioning the sentinel
/// string elsewhere must never be mistaken for the real marker.
fn reviewer_mutation_void_paths(text: &str) -> Option<Vec<String>> {
    let first_line = text.lines().next()?;
    let rest = first_line.strip_prefix(REVIEWER_MUTATION_VOID_PREFIX)?;
    Some(
        rest.split(',')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect(),
    )
}

/// X-verify-1: injected to retry after a voided (mutated-tree) reviewer call —
/// names the paths and reiterates the read-only mandate.
fn reviewer_mutation_void_retry_text(paths: &[String]) -> String {
    let named = if paths.is_empty() {
        "the working tree".to_string()
    } else {
        paths.join(", ")
    };
    format!(
        "The independent reviewer modified {named} instead of only reviewing it — a \
         READ-ONLY reviewer must never edit, write, or otherwise change a file, so its verdict \
         is void and nothing was actually reviewed. Call the `delegate` tool again with \
         `worker_persona: \"reviewer\"` and make sure it only reads and analyzes; report its \
         genuine, non-mutating verdict before ending the turn."
    )
}

/// Injected when [`last_reviewer_outcome`] finds an empty/errored reviewer
/// call: names the concrete problem (nothing was actually reviewed) and asks
/// for exactly one thing, rather than repeating the full original ask.
const REVIEWER_EMPTY_RETRY_TEXT: &str =
    "The independent reviewer call returned no answer (an empty or errored response) — nothing \
     was actually reviewed. Call the `delegate` tool again with `worker_persona: \"reviewer\"` \
     and report its verdict before ending the turn.";

/// X-verify-3: one real reviewer verdict in chronological order, paired with
/// whether a successful mutating tool call happened since the PREVIOUS entry
/// in the list this is built into — the same "identical across edits" shape
/// `tool_monitor`'s S6 verify fix-loop counter uses (see that module), scoped
/// here to `delegate(reviewer)` round-trips instead of `verify` re-runs.
struct ReviewerVerdictEvent {
    fingerprint: String,
    /// The reviewer's raw answer text, kept only to quote in the escalation
    /// message a human reads — never re-parsed.
    text: String,
    mutated_since_previous: bool,
}

/// Walk the WHOLE conversation — not scoped to a single mutation window, since
/// a repeated round-trip can span several turns and several `last_mutation`
/// positions — and record every REAL (non-empty, non-errored, non-voided)
/// `delegate(reviewer)` verdict in order, each tagged with whether a
/// successful mutating tool call happened since the previous one. A voided
/// (X-verify-1) or empty/errored call is skipped entirely: it was never a
/// real verdict to compare against, the same reasoning
/// [`last_reviewer_outcome`] already applies to the LATEST call alone.
fn reviewer_verdict_history(messages: &[Message]) -> Vec<ReviewerVerdictEvent> {
    use crate::conversation::message::MessageContent;
    use crate::cost_router::review_gate::parse_review;
    use std::collections::HashSet;

    let mut pending_reviewer = HashSet::new();
    let mut pending_mutation = HashSet::new();
    let mut mutated_since_previous = false;
    let mut history = Vec::new();

    for msg in messages {
        for content in &msg.content {
            match content {
                MessageContent::ToolRequest(req) => {
                    let Ok(call) = req.tool_call.as_ref() else {
                        continue;
                    };
                    if is_reviewer_delegation(call) {
                        pending_reviewer.insert(req.id.clone());
                    } else {
                        let base = call.name.rsplit("__").next().unwrap_or(call.name.as_ref());
                        if is_mutating_tool_name(base) {
                            pending_mutation.insert(req.id.clone());
                        }
                    }
                }
                MessageContent::ToolResponse(response) => {
                    if pending_mutation.remove(&response.id) {
                        if response
                            .tool_result
                            .as_ref()
                            .is_ok_and(|r| r.is_error != Some(true))
                        {
                            mutated_since_previous = true;
                        }
                        continue;
                    }
                    if !pending_reviewer.remove(&response.id) {
                        continue;
                    }
                    let Ok(result) = response.tool_result.as_ref() else {
                        continue;
                    };
                    if result.is_error == Some(true) {
                        continue;
                    }
                    let text = result
                        .content
                        .iter()
                        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
                        .collect::<Vec<_>>()
                        .join("\n");
                    if text.trim().is_empty() || reviewer_mutation_void_paths(&text).is_some() {
                        continue;
                    }
                    history.push(ReviewerVerdictEvent {
                        fingerprint: parse_review(&text).fingerprint(),
                        text,
                        mutated_since_previous,
                    });
                    mutated_since_previous = false;
                }
                _ => {}
            }
        }
    }
    history
}

/// X-verify-3: the escalate condition — the last two real reviewer verdicts
/// anywhere in the conversation are byte-identical (fingerprint) AND nothing
/// was successfully mutated between them. Returns the repeated verdict's raw
/// text (for the Park message) when the condition holds.
///
/// A tree change between the two rounds means the model genuinely attempted a
/// fix in response to the first verdict; the mandate must not punish that —
/// it is left to loop (bounded elsewhere: `MAX_HOLDS`, the turn budget). Only
/// the "asked again, changed nothing, got the same answer again" shape — a
/// wasted round-trip a third delegate call cannot fix either, since nothing
/// about the input to review has changed — escalates here, before a third
/// call is ever made.
fn repeated_identical_reviewer_verdict(messages: &[Message]) -> Option<String> {
    let history = reviewer_verdict_history(messages);
    let last = history.last()?;
    if last.mutated_since_previous {
        return None;
    }
    let previous = history.get(history.len().checked_sub(2)?)?;
    if last.fingerprint != previous.fingerprint {
        return None;
    }
    Some(last.text.clone())
}

/// Is a `delegate` tool present in this turn's resolved tool list? Same
/// prefix-stripping as [`is_reviewer_delegation`] (`extension_manager.rs`'s
/// `{extension}__{tool}` convention; `summon` itself registers `delegate`
/// unprefixed via `unprefixed_tools: true`, so both forms must match). This is
/// the live half of the `delegate_available` gate `decide` enforces: a recipe
/// that dropped the `summon` extension (only `developer`, say) has no
/// `delegate` tool at all, and the mandate cannot inject an instruction to
/// call a tool that isn't there.
pub fn delegate_tool_loaded(tools: &[rmcp::model::Tool]) -> bool {
    tools
        .iter()
        .any(|t| t.name.rsplit("__").next() == Some("delegate"))
}

/// True when this hook's own ask has already been injected after
/// `after_position` — one ask per changed-files turn, whether or not the model
/// obliged. Without this the hook would re-ask every time the model finished
/// again, which is a loop.
fn reviewer_ask_pending(messages: &[Message], after_position: usize) -> bool {
    use crate::conversation::message::MessageContent;

    let mut index = 0usize;
    for msg in messages {
        for content in &msg.content {
            index += 1;
            if index <= after_position {
                continue;
            }
            if let MessageContent::Text(text) = content {
                if text.text.contains(REVIEW_ASK_OPENING) {
                    return true;
                }
            }
        }
    }
    false
}

/// G-3 (Lane G review, finding 3): whether the E1 trivial-diff skip is even
/// allowed to run for this mutation. The skip must never fire once a
/// reviewer ask has already gone out — whether it is still awaiting an
/// answer, or was already answered with a real REQUEST_CHANGES/UNCERTAIN
/// verdict — because `trivial_skip_reason` reads the CURRENT `git diff`
/// live, and a diff that shrinks to trivial size between the ask and a later
/// turn (e.g. via a `shell` mutation `is_mutating_tool_name` does not track,
/// H2's own documented residual) would otherwise let the skip unconditionally
/// return `Allow` and silently erase a negative verdict already sitting in
/// the conversation — the exact bug Lane G found: `ask_state` would resolve
/// to `Asked` → `Park` if it were ever consulted, but the skip ran first and
/// returned before `ask_state` was computed at all.
///
/// `reviewer_ask_pending` already reports "an ask went out after this
/// mutation" regardless of whether it was later answered, so it is the
/// correct single signal here: once true, the diff's current triviality must
/// never be consulted again for this mutation.
fn trivial_skip_still_eligible(messages: &[Message], after_position: usize) -> bool {
    !reviewer_ask_pending(messages, after_position)
}

/// The imperative instruction injected to re-enter the model loop: names the
/// concrete reviewer, the tool, the persona, the data-fence format the recipe
/// already specifies, and the strict verdict tokens — so the model has exactly
/// what it needs to make the delegation in one shot.
fn reviewer_inject_text(
    pick: &crate::cost_router::ReviewerPick,
    actual: &ActualReviewerRoute,
) -> String {
    let warning = pick
        .warning
        .as_ref()
        .map(|w| format!("\n\nNote: {w}"))
        .unwrap_or_default();
    // E3: say what the dispatch will ACTUALLY do, not what `select_reviewer`
    // picked independently of the pack pin the dispatch honours.
    let model_note = if actual.same_family {
        format!(
            "it will run on {label} — the same family as you, so this is not an \
             independent, cross-family review",
            label = actual.label
        )
    } else {
        format!(
            "it will run on {label}, a different model from yours",
            label = actual.label
        )
    };
    format!(
        "{REVIEW_ASK_OPENING} Call the `delegate` tool now with `worker_persona: \"reviewer\"` \
         ({model_note}). Give it the task \
         spec, the diff, and the verify output, fenced exactly as the recipe \
         specifies:\n\
         BEGIN_TASK_SPEC\n<what was asked for>\nEND_TASK_SPEC\n\
         BEGIN_DIFF\n<the diff>\nEND_DIFF\n\
         BEGIN_VERIFY_OUTPUT\n<the verify output>\nEND_VERIFY_OUTPUT\n\
         Then report the reviewer's verdict — APPROVE, REQUEST_CHANGES, or \
         UNCERTAIN — before ending the turn. Do not end the turn until the \
         reviewer has answered.{warning}"
    )
}

/// Whether — and WHY — this hook considers itself already asked. Split out
/// (E2) so the Park reason can be honest about which of the two applies: an
/// ask that genuinely went out and got no APPROVE back, versus the shared
/// hold-budget counter running out before this hook ever got to ask at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AskState {
    /// The mandate's own [`REVIEW_ASK_OPENING`] text was found in the
    /// conversation: a real ask went out.
    Asked,
    /// No ask was found, but the shared `prior_holds` counter is already at
    /// its bound anyway (e.g. spent entirely by [`PrematureDoneGuard`] on
    /// verify-fix cycles) — the loop must still stop, but the reviewer was
    /// never actually consulted.
    HoldBudgetExhausted,
    /// Neither of the above: this turn has not been asked yet.
    NotAsked,
}

/// Reason text for a Park raised because the `delegate` tool itself is not
/// loaded — distinct from `ReviewerAvailability::Unavailable` (which means "no
/// reviewer model could be picked"): this means the mandate cannot even ask,
/// because the recipe/session never loaded the tool it would call.
fn delegate_not_loaded_reason() -> String {
    format!(
        "{REVIEW_PARK_PREFIX}the `delegate` tool is not loaded in this session, so the reviewer \
         mandate cannot be enforced — add the `summon` extension to this recipe or session"
    )
}

/// Pure: what the mandate does once mutation/delegation/hold state, tool
/// availability, and reviewer availability are already known. See
/// [`ReviewerMandate::after_turn`] for the live gating order this mirrors —
/// `mutated`, then `delegation_already_ran`, then `ask_state`, then
/// `delegate_available`, and ONLY THEN `availability`, so a turn that never
/// needed a reviewer (or already got one, was already asked once, or has no
/// `delegate` tool to call at all) never has to have `availability` computed
/// at all.
///
/// `delegate_available` gates BEFORE `availability` is even consulted: asking
/// the model to call a tool that is not loaded is not a weaker version of the
/// mandate, it is the mandate silently failing open (the bug this exists to
/// fix) — so a missing tool is always a `Park`, never a `Continue`, regardless
/// of what `availability` says.
fn decide(
    mutated: bool,
    reviewer_approved: bool,
    ask_state: AskState,
    delegate_available: bool,
    availability: &ReviewerAvailability,
) -> AfterTurnAction {
    if !mutated || reviewer_approved {
        return AfterTurnAction::Allow;
    }
    match ask_state {
        AskState::Asked => AfterTurnAction::Park {
            reason: format!(
                "{REVIEW_PARK_PREFIX}the reviewer did not return a successful APPROVE verdict"
            ),
        },
        // E2: honest about WHY the loop is stopping — the shared hold counter
        // ran out, not a reviewer that answered and failed. Never claim a
        // verdict that never came back.
        AskState::HoldBudgetExhausted => AfterTurnAction::Park {
            reason: format!(
                "{REVIEW_PARK_PREFIX}the hold budget was exhausted before the reviewer could be \
                 asked"
            ),
        },
        // CASE B fix #1 (main #1157): a reviewer may be picked and priced and
        // still be unaskable, because the recipe never loaded the `delegate`
        // tool the ask names. Gating here — inside `NotAsked`, after the two
        // already-stopped arms and before `availability` — keeps main's
        // ordering (`already_asked`, then `delegate_available`, then
        // `availability`) while leaving HEAD's honest Park reasons intact.
        AskState::NotAsked if !delegate_available => AfterTurnAction::Park {
            reason: delegate_not_loaded_reason(),
        },
        AskState::NotAsked => match availability {
            ReviewerAvailability::Ready(pick, actual) => AfterTurnAction::Continue {
                inject: reviewer_inject_text(pick, actual),
            },
            ReviewerAvailability::SpendRefused { reason } => AfterTurnAction::Park {
                reason: format!("{REVIEW_PARK_PREFIX}{reason}"),
            },
            ReviewerAvailability::Unavailable { reason } => AfterTurnAction::Park {
                reason: format!("{REVIEW_PARK_PREFIX}{reason}"),
            },
        },
    }
}

#[async_trait]
impl AfterTurn for ReviewerMandate {
    fn name(&self) -> &'static str {
        "reviewer_mandate"
    }

    fn is_enabled(&self) -> bool {
        Self::enabled_by_config()
    }

    async fn after_turn(&self, ctx: &AfterTurnContext<'_>) -> AfterTurnAction {
        let positions = scan_verify_positions(ctx.messages);
        let Some(last_mutation) = positions.last_mutation else {
            // The applicability gate, same as `PrematureDoneGuard`: nothing was
            // written, so there is nothing to have reviewed. No reviewer
            // selection is attempted — ordinary conversation never pays for it.
            return AfterTurnAction::Allow;
        };
        // The recipe mandates review "once verify PASSES on a non-trivial
        // change", and deferring until then is also what keeps the two hooks
        // from fighting. `AfterTurnManager` folds their answers worst-wins
        // (Allow < Continue < Park), so a `Park` raised here because no reviewer
        // is available would OUTRANK `PrematureDoneGuard`'s `Continue` and end
        // the turn in place of sending the model back to verify. While edits are
        // unverified this is the guard's turn to speak; the mandate fires on the
        // pass after verify goes green.
        if positions.unverified_edits() {
            return AfterTurnAction::Allow;
        }

        let reviewer_approved = reviewer_approved_after(ctx.messages, last_mutation);
        if reviewer_approved {
            return AfterTurnAction::Allow;
        }

        // E1: the recipe's own trivial/docs-only/test-only/format-only skip —
        // previously enforced only on the goal-dispatch path
        // (`goose-server::verification::review`), never here. Checked after
        // `reviewer_approved` (an already-approved turn never needs it) and
        // before everything else, so a trivial diff never pays for a reviewer
        // pick or a `delegate` round-trip at all.
        //
        // G-3: but ONLY when no reviewer ask is pending and no reviewer
        // verdict has been recorded since the last mutation
        // (`trivial_skip_still_eligible`, reusing the same
        // `reviewer_ask_pending` `ask_state` will use below) — otherwise a
        // diff that reads as trivial NOW could silently erase a real
        // negative verdict already sitting in the conversation. This also
        // means an already-asked mutation never touches the DB-backed
        // `trivial_skip_reason` at all.
        if trivial_skip_still_eligible(ctx.messages, last_mutation) {
            if let Some(reason) = Self::trivial_skip_reason(ctx.session_id).await {
                tracing::info!(
                    session_id = %ctx.session_id,
                    "independent review skipped: {reason}"
                );
                return AfterTurnAction::Allow;
            }
        }

        // E5: a `delegate(reviewer)` call that came back empty or errored
        // asked nothing real — retry it, bounded by the same shared hold
        // counter every other hook answers to, rather than treating "no
        // answer" as a completed-but-failed review.
        //
        // X-verify-1: a call `summon` VOIDED because the reviewer mutated the
        // working tree gets the identical bounded-retry treatment, but with
        // an honest message naming the paths instead of the generic "no
        // answer" text — Lane A2's live evidence was a reviewer that DID
        // answer (often with a confident APPROVE) while also editing a file;
        // that is not "no answer", so it needs its own branch, not Empty's.
        match last_reviewer_outcome(ctx.messages, last_mutation) {
            ReviewerLastOutcome::Void(paths) => {
                return if ctx.prior_holds < crate::cost_router::MAX_HOLDS {
                    AfterTurnAction::Continue {
                        inject: reviewer_mutation_void_retry_text(&paths),
                    }
                } else {
                    let named = if paths.is_empty() {
                        "the working tree".to_string()
                    } else {
                        paths.join(", ")
                    };
                    AfterTurnAction::Park {
                        reason: format!(
                            "{REVIEW_PARK_PREFIX}the reviewer modified {named} instead of only \
                             reviewing it, and the hold budget was exhausted before a genuine \
                             review could be obtained"
                        ),
                    }
                };
            }
            ReviewerLastOutcome::Empty => {
                return if ctx.prior_holds < crate::cost_router::MAX_HOLDS {
                    AfterTurnAction::Continue {
                        inject: REVIEWER_EMPTY_RETRY_TEXT.to_string(),
                    }
                } else {
                    AfterTurnAction::Park {
                        reason: format!(
                            "{REVIEW_PARK_PREFIX}the reviewer returned no answer and the hold \
                             budget was exhausted before it could be asked again"
                        ),
                    }
                };
            }
            ReviewerLastOutcome::Answered | ReviewerLastOutcome::NotCalled => {}
        }

        // X-verify-3: the model (or a prior injected ask) may have called the
        // reviewer more than once for this mutation. If the last two REAL
        // (non-empty, non-voided) verdicts anywhere in the conversation are
        // byte-identical AND nothing was mutated between them, a third
        // delegate call cannot do anything the first two didn't already show —
        // escalate now instead of injecting another ask.
        if let Some(findings) = repeated_identical_reviewer_verdict(ctx.messages) {
            return AfterTurnAction::Park {
                reason: format!(
                    "{REVIEW_PARK_PREFIX}the reviewer returned the identical verdict twice in \
                     a row with no code change between the two reviews — escalating instead of \
                     asking a third time. Last verdict:\n{findings}"
                ),
            };
        }

        // `prior_holds` is a shared, all-hooks counter and cannot answer "did I
        // already ask?" (see `REVIEW_ASK_OPENING`); it serves only as a hard
        // backstop against an unbounded loop if the injected ask ever stopped
        // being findable. 2, not 1, because the guard may legitimately have
        // spent one hold on verify before this hook ever ran.
        let ask_state = if reviewer_ask_pending(ctx.messages, last_mutation) {
            AskState::Asked
        } else if ctx.prior_holds >= 2 {
            AskState::HoldBudgetExhausted
        } else {
            AskState::NotAsked
        };

        // Gate on tool availability BEFORE paying for `assess` (a session
        // read plus a `git diff --shortstat` and cost-router selection): if
        // `delegate` was never loaded, no amount of reviewer-picking changes
        // the answer, and asking anyway is exactly the fail-open bug this
        // gate exists to close.
        if !ctx.delegate_tool_available {
            return AfterTurnAction::Park {
                reason: delegate_not_loaded_reason(),
            };
        }

        let availability = Self::assess(ctx.session_id).await;
        decide(
            true,
            reviewer_approved,
            ask_state,
            ctx.delegate_tool_available,
            &availability,
        )
    }
}

// ── Third implementor: no "dispatched" claim without a child ───────────────

/// Refuses to let a turn end claiming a roadmap's work is dispatched or in
/// progress when the durable goal-card state says otherwise.
///
/// Lane M3's Defect M3-2 (harness loop): a coordinator ended its turn with
/// *"The root goals are dispatched and will run in isolated worktrees ... I
/// will monitor progress and report results once completed and verified. No
/// further action is needed on this loop until workers report back"* having
/// called neither `goal_advance` nor `delegate` at all that session — the
/// roadmap's only root goal was still sitting Ready. Unlike
/// [`PrematureDoneGuard`] and [`ReviewerMandate`], this is not a
/// transcript-shaped signal (a coordinator's own closing claim is not
/// evidence of anything happening) — it reads the durable card state
/// `create_roadmap` stamps with `metadata_json.created_by_session_id`
/// (X-dag-1, coding-harness excellence iteration) and asks a purely factual
/// question: does ANY goal this session created look stuck?
///
///   - a ROOT goal sitting Ready with no worker session ever recorded (the
///     exact M3-2 shape: dispatch never happened at all), or
///   - ANY goal (root or not) claiming InProgress with no worker session
///     recorded — a state that should be unreachable through the sanctioned
///     `dispatch_goal_fn` transition, which stamps the worker session id in
///     the SAME transaction as the Ready→InProgress move.
///
/// A session that never called `create_roadmap` has no cards attributed to
/// it at all, so this is a no-op for ordinary conversation — the same
/// applicability discipline [`PrematureDoneGuard`] and [`ReviewerMandate`]
/// hold themselves to, just keyed on durable state instead of the
/// transcript. Bounded by the shared `MAX_HOLDS` counter like the other two
/// hooks; `Park`s with an honest, specific reason once exhausted rather than
/// holding forever.
pub struct RoadmapDispatchGuard;

impl RoadmapDispatchGuard {
    pub fn new() -> Self {
        Self
    }

    /// The live half, pool-parameterised so it is directly testable against
    /// an isolated in-memory database — unlike [`ReviewerMandate::assess`],
    /// this does not need the process-global `SessionManager::instance()`
    /// singleton to be exercised by a test; only the trait impl below reaches
    /// for it, keeping that seam as thin as possible.
    async fn undispatched_goal_for(
        pool: &sqlx::Pool<sqlx::Sqlite>,
        session_id: &str,
    ) -> Result<Option<crate::cards::SessionGoalCard>, String> {
        let cards = crate::cards::goal_cards_created_by_session(pool, session_id).await?;
        Ok(crate::cards::first_undispatched_goal(&cards).cloned())
    }

    /// The message injected on hold, and reused verbatim as the `Park`
    /// reason once the hold budget is exhausted — the brief's own wording,
    /// naming the session as the roadmap's identity (this crate has no
    /// separate "roadmap id"; a roadmap IS the set of goal cards one
    /// session's `create_roadmap` call minted) and the stuck card's real id
    /// (never a fabricated one — the same discoverability fix M3-1(b) made
    /// for `create_roadmap`'s own result text).
    fn message(session_id: &str, goal: &crate::cards::SessionGoalCard) -> String {
        format!(
            "roadmap {session_id}: goal {} is not dispatched; call goal_advance dispatch or explain",
            goal.card_id
        )
    }

    /// E3 (harness loop, Round E, from Lane F9 forensics 2026-09-10): fx10
    /// — a headless coordinator called `decompose_roadmap`, its proposal's
    /// approval decision was answered `approve-roadmap`, and the turn simply
    /// ended without ever calling `create_roadmap`: 0 goal cards, no
    /// dispatch, nothing for [`Self::undispatched_goal_for`] to see (there
    /// is nothing to check when there are no cards at all — this is its
    /// complement, "was a roadmap ever materialized").
    ///
    /// Durable-state-only, same discipline as `undispatched_goal_for`: a
    /// session with no cards AND no approved roadmap-creation decision
    /// minted by it is ordinary conversation, a no-op here. Returns the
    /// nudge/park text when this session's approved proposal never produced
    /// a single card.
    async fn undecomposed_roadmap_for(
        pool: &sqlx::Pool<sqlx::Sqlite>,
        session_id: &str,
    ) -> Result<Option<String>, String> {
        let cards = crate::cards::goal_cards_created_by_session(pool, session_id).await?;
        if !cards.is_empty() {
            // create_roadmap succeeded at least once for this session.
            return Ok(None);
        }
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT payload_json FROM decisions
             WHERE kind = 'choice' AND status = 'answered' AND answer_choice_id = 'approve-roadmap'",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
        let minted_this_session = rows.iter().any(|(payload_json,)| {
            serde_json::from_str::<crate::decisions::ChoicePayload>(payload_json)
                .ok()
                .is_some_and(|payload| {
                    payload.proposal.as_deref() == Some(crate::decisions::PROPOSAL_ROADMAP_CREATE)
                        && payload
                            .roadmap_approval
                            .as_ref()
                            .and_then(|r| r.minting_session_id.as_deref())
                            == Some(session_id)
                })
        });
        if !minted_this_session {
            return Ok(None);
        }
        Ok(Some(format!(
            "roadmap {session_id}: decompose_roadmap's proposal was approved but \
             create_roadmap was never called and no goal cards exist; call create_roadmap \
             with the exact goals_json, proposal_hash, and approval_decision_id from \
             decompose_roadmap's own result, or escalate if the roadmap should not be created"
        )))
    }
}

/// Whether any tool call anywhere in `messages` was `escalate` — the honest
/// exit valve for [`RoadmapDispatchGuard::undecomposed_roadmap_for`] (E3,
/// harness loop Round E): a coordinator that decides mid-roadmap the
/// proposal should not be created after all correctly stopped to let a
/// human decide; that is not the fx10 "stuck" shape and must never be held.
fn turn_called_escalate(messages: &[Message]) -> bool {
    turn_called_any_tool(messages, &["escalate"])
}

/// Whether any tool request anywhere in `messages` has one of `names` as its
/// BASE tool name — the part after the extension prefix, so `orchestrator__
/// decompose_roadmap` and a bare `decompose_roadmap` are the same call. The
/// shared spine of [`turn_called_escalate`] and
/// [`NoRoadmapGuard`]'s roadmap check, so the two can never drift on how a
/// tool name is matched.
fn turn_called_any_tool(messages: &[Message], names: &[&str]) -> bool {
    use crate::conversation::message::MessageContent;
    messages
        .iter()
        .flat_map(|m| m.content.iter())
        .any(|content| {
            let MessageContent::ToolRequest(req) = content else {
                return false;
            };
            let Ok(call) = req.tool_call.as_ref() else {
                return false;
            };
            let base = call.name.rsplit("__").next().unwrap_or(call.name.as_ref());
            names.contains(&base)
        })
}

impl Default for RoadmapDispatchGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AfterTurn for RoadmapDispatchGuard {
    fn name(&self) -> &'static str {
        "roadmap_dispatch"
    }

    async fn after_turn(&self, ctx: &AfterTurnContext<'_>) -> AfterTurnAction {
        let manager = crate::session::SessionManager::instance();
        let pool = match manager.pool_clone().await {
            Ok(p) => p,
            // Cannot check — the same storage layer `create_roadmap`/
            // `goal_advance` themselves depend on. Failing open here matches
            // `summon::restrict_delegate_for_active_roadmap`'s own choice:
            // an unrelated DB outage must not turn every turn end into a
            // hold.
            Err(_) => return AfterTurnAction::Allow,
        };
        if let Ok(Some(goal)) = Self::undispatched_goal_for(&pool, ctx.session_id).await {
            let message = Self::message(ctx.session_id, &goal);
            return if ctx.prior_holds < crate::cost_router::MAX_HOLDS {
                AfterTurnAction::Continue { inject: message }
            } else {
                AfterTurnAction::Park { reason: message }
            };
        }

        // E3 (harness loop, Round E, from Lane F9 forensics 2026-09-10):
        // complementary to the dispatch check above — that one is a no-op
        // when a session has zero cards; this catches the shape where it
        // has zero cards because `create_roadmap` was never called at all.
        // An explicit `escalate` anywhere in this session's transcript is
        // an honest way to end the turn without creating the roadmap; never
        // held in that case.
        if !turn_called_escalate(ctx.messages) {
            if let Ok(Some(reason)) = Self::undecomposed_roadmap_for(&pool, ctx.session_id).await {
                return if ctx.prior_holds < crate::cost_router::MAX_HOLDS {
                    AfterTurnAction::Continue { inject: reason }
                } else {
                    AfterTurnAction::Park { reason }
                };
            }
        }

        AfterTurnAction::Allow
    }
}

// ── Fourth implementor: a coordinator turn cannot end without a roadmap ────

/// The orchestrator tools that satisfy rule 1 / exception (a) of
/// `permagent-coding.yaml`'s NON-NEGOTIABLE protocol: `decompose_roadmap`/
/// `create_roadmap` turn a plan into durable goal cards; `resume_roadmap`
/// (Lane R20c-c) resumes dispatch of an EXISTING one — exception (a)'s own
/// wording lists it alongside `check_worker`/`goal_advance` as the correct
/// alternative to re-decomposing when "a roadmap/goal card already exists
/// and is Ready or paused". Before R20c-c only the first two were
/// recognised, so a coordinator that correctly resumed a pre-existing
/// roadmap and then closed with a summary still read as "never satisfied
/// rule 1" (harness loop, "Lane R20c-c":
/// `select-existing-roadmap-resumes-not-redecomposes`, M24 evidence). Any
/// ONE of the three having succeeded is proof the coordinator did the
/// thing the recipe asks for; none having been called/succeeded is
/// [`NoRoadmapGuard`]'s and [`Rule5HonestFailureGuard`]'s whole trigger.
const ROADMAP_TOOL_NAMES: [&str; 3] = ["decompose_roadmap", "create_roadmap", "resume_roadmap"];

/// Lane R20c-c: cards relevant to `session_id`'s OWN roadmap decision — the
/// union of (a) cards this session itself created
/// ([`crate::cards::goal_cards_created_by_session`], unchanged scope — its
/// other callers, `RoadmapDispatchGuard`'s X-dag-1/X-dag-2 checks, keep
/// using it directly and are untouched by this function) and (b) cards
/// belonging to the ACTIVE project whose `root_path` matches `working_dir`
/// ([`crate::cards::goal_cards_for_project_matching_cwd`]) — so a
/// coordinator that RESUMED a pre-existing roadmap it never created still
/// sees it. Deduplicated by `card_id` (the two scopes can overlap once a
/// resumed goal's later dispatch gets attributed to this session). Shared
/// by [`NoRoadmapGuard`] and [`Rule5HonestFailureGuard`] so the two guards'
/// "does a roadmap exist" answer can never disagree.
async fn goal_cards_relevant_to_session(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    session_id: &str,
    working_dir: &std::path::Path,
) -> Result<Vec<crate::cards::SessionGoalCard>, String> {
    let mut cards = crate::cards::goal_cards_created_by_session(pool, session_id).await?;
    let project_cards =
        crate::cards::goal_cards_for_project_matching_cwd(pool, working_dir).await?;
    let mut seen: std::collections::HashSet<String> =
        cards.iter().map(|c| c.card_id.clone()).collect();
    for card in project_cards {
        if seen.insert(card.card_id.clone()) {
            cards.push(card);
        }
    }
    Ok(cards)
}

/// Opening sentence of the injected nudge. Written once, then recognised again
/// on any later pass through [`NoRoadmapGuard::after_turn`], which is how this
/// hook enforces its own once-per-session budget.
///
/// It cannot use `AfterTurnContext::prior_holds` for that — the reply loop
/// keeps ONE counter shared by every hook, so a hold spent by
/// [`PrematureDoneGuard`] or [`ReviewerMandate`] would read here as "I already
/// asked". The injected message is a durable user message in the conversation,
/// so looking for it is both hook-local and survives restarts. Same mechanism,
/// same reasoning, as [`REVIEW_ASK_OPENING`].
const NO_ROADMAP_INJECT_OPENING: &str = "You ended the turn without a roadmap.";

/// The message injected when a coding-harness coordinator's turn ends with no
/// roadmap. Names the rule it is enforcing rather than merely scolding, and
/// closes the two exits the live failures actually took — a prose plan
/// (fx02: *"Say the word and I'll create and dispatch this immediately"*) and
/// a question to the user (fx06, after `CodingDagInspector` denied its
/// `shell` call).
// R20c-b (A21 Q2): the old wording below offered `escalate` only for a task
// that is "genuinely impossible" — narrower than the recipe's own carve-outs
// (b)/(c) (ambiguity, a missing credential/provider, or any question to the
// user, all of which are also `escalate`). A DeepSeek round literally quoted
// this nudge back and followed it into `decompose_roadmap` on a fixture that
// should have escalated instead — the nudge was overriding the carve-out it
// was supposed to reinforce. Restated so `escalate` covers the same three
// cases the recipe does, checked FIRST, matching the recipe's own reading
// order.
const NO_ROADMAP_INJECT_TEXT: &str =
    "You ended the turn without a roadmap. If the request is ambiguous, a needed credential or \
     provider is missing, or the task is genuinely impossible, call `escalate` with the exact \
     question — do not guess, do not ask in prose. Otherwise call `decompose_roadmap` now with \
     the concrete nodes for this task; a tiny fix is a valid one-node roadmap. A plan described \
     in a message, a question, or an offer to act \"once confirmed\" is not an exit from this \
     turn. In unattended mode the operator's launch is already the approval. If a direct tool \
     call was denied, that denial is the protocol working — decompose and let the dispatched \
     worker run it.";

/// Refuses to let a coding-harness COORDINATOR turn end having never asked for
/// a roadmap at all.
///
/// Lane A13's forensics on the M13 slice (harness loop, 2026-09-10) found two
/// fixtures that exited in ten seconds with nothing done and no guard firing:
///
///   - **fx02** made four read-only calls, wrote a full "Proposed roadmap"
///     table in prose, and ended *"Say the word and I'll create and dispatch
///     this immediately."*
///   - **fx06** had its one `shell` call denied by
///     [`CodingDagInspector`](crate::tool_inspection::CodingDagInspector) —
///     correctly — and then asked the user a question instead of decomposing.
///
/// Both violate protocol rule 1 of `permagent-coding.yaml` ("your first
/// execution tool call is `decompose_roadmap`"), and neither existing hook can
/// see it. [`PrematureDoneGuard`] needs a SUCCESSFUL mutation (fx06's denied
/// `shell` is not one, and fx02 never even tried). [`RoadmapDispatchGuard`]
/// needs either goal cards or an approved `decompose_roadmap` proposal — this
/// shape has neither, because the tool was never called. That is the gap: the
/// recipe claims its rules "are enforced by the runtime", and rule 1's
/// *first-call* framing was in fact enforced by nothing.
///
/// Applicability is the identical predicate `CodingDagInspector` uses —
/// [`is_top_level_coding_harness`](crate::tool_inspection::is_top_level_coding_harness):
/// recipe title `"Permagent Coding Harness"` AND `parent_session_id` absent. A
/// dispatched worker (which has a parent) is the thing that MAY edit and must
/// never be told to decompose; any other recipe, and ordinary conversation,
/// never reaches the check at all. Like the other guards keyed on that
/// predicate, no headless/interactive distinction is drawn — interactive rule 2
/// still says "present that exact graph once", and presenting it means calling
/// `decompose_roadmap`, not describing it.
///
/// Fires **at most once per session**: the nudge is a nudge, not a cage. A
/// coordinator that reads it and still declines has made a decision a human
/// should see, and the run's own transcript now carries the reason.
pub struct NoRoadmapGuard;

impl NoRoadmapGuard {
    pub fn new() -> Self {
        Self
    }

    /// The pool-parameterised half, testable against an isolated in-memory
    /// database exactly like [`RoadmapDispatchGuard::undispatched_goal_for`] —
    /// "does this session own OR share a project with any goal card at
    /// all", the complement of that guard's "is one of this session's
    /// cards stuck". Lane R20c-c: widened from "does this session own any
    /// goal card" to also see a RESUMED roadmap's cards via
    /// [`goal_cards_relevant_to_session`] — see that function's doc comment.
    #[cfg(test)]
    async fn has_roadmap_cards(
        pool: &sqlx::Pool<sqlx::Sqlite>,
        session_id: &str,
        working_dir: &std::path::Path,
    ) -> Result<bool, String> {
        Ok(
            !goal_cards_relevant_to_session(pool, session_id, working_dir)
                .await?
                .is_empty(),
        )
    }

    /// True when this hook's own nudge is already in the conversation. One
    /// nudge per session, whether or not the model obliged.
    fn already_nudged(messages: &[Message]) -> bool {
        use crate::conversation::message::MessageContent;
        messages.iter().flat_map(|m| m.content.iter()).any(|c| {
            matches!(c, MessageContent::Text(text) if text.text.contains(NO_ROADMAP_INJECT_OPENING))
        })
    }

    /// Lane R31b: the stranded-Ready-goal nudge's OWN once-per-session
    /// marker — distinct from [`Self::already_nudged`]'s (a different
    /// injected sentence entirely), so the two nudges' budgets can never be
    /// confused with each other.
    fn already_nudged_stranded_ready(messages: &[Message]) -> bool {
        use crate::conversation::message::MessageContent;
        messages.iter().flat_map(|m| m.content.iter()).any(|c| {
            matches!(c, MessageContent::Text(text) if text.text.contains(STRANDED_READY_NUDGE_MARKER))
        })
    }

    /// Lane R31b (A30 analysis): the first (if any) of `cards` that is
    /// `Ready` and carries R26b's `resumed_by_headless_unblock_at` stamp —
    /// the exact shape A30 traced (a goal that finished its one-time
    /// headless-unblock resume with no further redispatch/refusal-audit
    /// cycle left to claim it before the run ended). One extra
    /// `cards::get_card` fetch per Ready candidate only — `SessionGoalCard`
    /// itself carries no `metadata_json` — never a scan of the whole
    /// roadmap; a card read failure is treated the same as "not stranded"
    /// (fail open, same posture as every other lookup in this guard).
    async fn stranded_ready_goal_id(
        pool: &sqlx::Pool<sqlx::Sqlite>,
        cards: &[crate::cards::SessionGoalCard],
    ) -> Option<String> {
        for card in cards {
            if card.state_binding != "ready" {
                continue;
            }
            if let Ok(Some(full)) = crate::cards::get_card(pool, &card.card_id).await {
                if full
                    .metadata_json
                    .get("resumed_by_headless_unblock_at")
                    .is_some()
                {
                    return Some(card.card_id.clone());
                }
            }
        }
        None
    }

    /// The whole policy as one pure fold, so every branch is unit-testable
    /// without a session store or a database — the same shape as
    /// [`decide`], `ReviewerMandate`'s pure half.
    ///
    /// Every "do not fire" reason is a first-class parameter rather than an
    /// early return in the live path, so a test can pin each one
    /// independently.
    fn decide(
        is_coding_coordinator: bool,
        has_roadmap_cards: bool,
        called_roadmap_tool: bool,
        called_escalate: bool,
        already_nudged: bool,
        prior_holds: u8,
    ) -> AfterTurnAction {
        if !is_coding_coordinator
            || has_roadmap_cards
            || called_roadmap_tool
            || called_escalate
            || already_nudged
        {
            return AfterTurnAction::Allow;
        }
        if prior_holds < crate::cost_router::MAX_HOLDS {
            AfterTurnAction::Continue {
                inject: NO_ROADMAP_INJECT_TEXT.to_string(),
            }
        } else {
            // Honest about which budget ran out, like `ReviewerMandate`'s
            // `HoldBudgetExhausted`: the turn is ending with nothing done, and
            // saying so beats exiting silently the way fx02/fx06 did.
            AfterTurnAction::Park {
                reason: NO_ROADMAP_PARK_REASON.to_string(),
            }
        }
    }
}

/// Lane R31b (A30 analysis): the stable substring [`NoRoadmapGuard::
/// already_nudged_stranded_ready`] recognises and [`NoRoadmapGuard::
/// after_turn`]'s own injected text always contains, regardless of which
/// goal id it names — a fixed marker separate from the goal id lets the
/// once-per-session check work without re-parsing the injected sentence.
const STRANDED_READY_NUDGE_MARKER: &str =
    "is Ready with unlanded work; call goal_advance(dispatch)";

/// Surfaced instead of the nudge once the shared hold budget is spent. Never
/// claims a roadmap was refused — only that none was ever requested.
const NO_ROADMAP_PARK_REASON: &str = "the coding coordinator's turn ended without a roadmap: \
     neither decompose_roadmap nor create_roadmap was ever called and no goal cards exist, and \
     the hold budget was exhausted before one could be requested";

impl Default for NoRoadmapGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AfterTurn for NoRoadmapGuard {
    fn name(&self) -> &'static str {
        "no_roadmap"
    }

    async fn after_turn(&self, ctx: &AfterTurnContext<'_>) -> AfterTurnAction {
        // Transcript-only gates first, so the overwhelmingly common case — a
        // turn that DID decompose — costs no session read and no DB round
        // trip. `Self::decide` encodes the same three conditions, and the
        // tests pin them there.
        let called_roadmap_tool = turn_called_any_tool(ctx.messages, &ROADMAP_TOOL_NAMES);
        let called_escalate = turn_called_escalate(ctx.messages);
        let already_nudged = Self::already_nudged(ctx.messages);
        if called_roadmap_tool || called_escalate || already_nudged {
            return AfterTurnAction::Allow;
        }

        let manager = crate::session::SessionManager::instance();
        // Fail open on a degraded session store, the same choice
        // `CodingDagInspector` makes for the identical lookup: this is a
        // quality-policy guard, and an unrelated storage outage must not turn
        // every turn end into a hold.
        let Ok(session) = manager.get_session(ctx.session_id, false).await else {
            return AfterTurnAction::Allow;
        };
        let title = session.recipe.as_ref().map(|recipe| recipe.title.as_str());
        if !crate::tool_inspection::is_top_level_coding_harness(
            title,
            session.parent_session_id.as_deref(),
        ) {
            return AfterTurnAction::Allow;
        }

        let Ok(pool) = manager.pool_clone().await else {
            return AfterTurnAction::Allow;
        };
        let Ok(roadmap_cards) =
            goal_cards_relevant_to_session(&pool, ctx.session_id, &session.working_dir).await
        else {
            return AfterTurnAction::Allow;
        };
        let has_roadmap_cards = !roadmap_cards.is_empty();

        // Lane R31b (A30 analysis): a roadmap already exists — the branch
        // below would otherwise just Allow — so cheaply check whether one
        // of THIS session's own cards is `Ready` carrying R26b's
        // `resumed_by_headless_unblock_at` stamp, the exact "finished its
        // one-time headless-unblock resume with no further redispatch cycle
        // left to claim it" shape A30 traced. Bounded: only the (normally 0
        // or 1) Ready candidates get a follow-up `cards::get_card` fetch —
        // `SessionGoalCard` itself carries no metadata — never the whole
        // roadmap. `run_end_salvage_sweep` (orchestrator.rs) is the
        // guaranteed last-chance backstop regardless of whether this nudge
        // fires or is heeded; this is only a cheaper, earlier chance for the
        // coordinator to act on it itself.
        if has_roadmap_cards && !Self::already_nudged_stranded_ready(ctx.messages) {
            if let Some(goal_id) = Self::stranded_ready_goal_id(&pool, &roadmap_cards).await {
                return AfterTurnAction::Continue {
                    inject: format!(
                        "goal {goal_id} is Ready with unlanded work; call goal_advance(dispatch) \
                         or let the run-end sweep land it"
                    ),
                };
            }
        }

        Self::decide(
            true,
            has_roadmap_cards,
            called_roadmap_tool,
            called_escalate,
            already_nudged,
            ctx.prior_holds,
        )
    }
}

// ── Lane R35a: nudge a verified-but-unlanded goal instead of a prose summary ──

/// Marker every [`VerifiedGoalAwaitingLandGuard`] injection contains,
/// regardless of goal id or which action it names — lets the once-PER-GOAL
/// check below test for "was THIS goal already nudged" without re-parsing the
/// sentence. Distinct from [`STRANDED_READY_NUDGE_MARKER`] (a different
/// shape: no verified commit, only R26b's headless-unblock resume stamp).
const VERIFIED_LANDING_NUDGE_MARKER: &str = "has a verified commit awaiting review";

/// Which real `goal_advance` action closes the gap for a given goal's current
/// state — never a hoped-for "land" action: `goal_advance` only ever parses
/// `ready`/`dispatch`/`review`/`approve`/`reject`
/// ([`crate::goal_state::GoalAction::parse_action`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerifiedLandingAction {
    /// `ready` with `dispatch_evidence.verdict.status == "pass"` already on
    /// the card: a prior attempt verified and committed, but the card cycled
    /// back to Ready (redispatch) instead of ever reaching Review — call
    /// `review` to advance it there directly.
    Review,
    /// `review` with the same verified verdict: the completion tracker's own
    /// automatic InProgress -> Review transition already fired and opened the
    /// `approve_review` decision, but the turn is about to end without
    /// answering it — call `approve`, which lands the goal.
    Approve,
}

impl VerifiedLandingAction {
    fn tool_action(&self) -> &'static str {
        match self {
            Self::Review => "review",
            Self::Approve => "approve",
        }
    }
}

/// The full injected sentence for `goal_id`/`action` — always contains
/// [`VERIFIED_LANDING_NUDGE_MARKER`] so
/// [`VerifiedGoalAwaitingLandGuard::already_nudged_for_goal`] recognises it on
/// a later turn.
fn verified_landing_nudge_text(goal_id: &str, action: &VerifiedLandingAction) -> String {
    format!(
        "goal {goal_id} {VERIFIED_LANDING_NUDGE_MARKER}; call orchestrator__goal_advance action \
         \"{}\" now — do not summarise until every goal is Done or parked.",
        action.tool_action()
    )
}

/// Refuses to let a coding-harness coordinator's turn end in prose while a
/// goal in its roadmap already has a worker's verified, committed PASS
/// sitting unconsumed.
///
/// A32 root cause 1 (HARNESS_LOOP_LOG.md, "A32 analysis — review_reached /
/// vacuous_passes flake on M31"): every `review_reached=0` miss traced ends
/// its harness log with the byte-identical tail `"0 goal(s) finished, 1 still
/// running at exit -> parked"` after 2-3 dispatch cycles (workers go
/// in_progress -> ready, never complete). The clearest case
/// (`m3/select-named-check-uses-verify-tool`) had a worker that diagnosed a
/// broken `python3` interpreter, re-ran `verify` explicitly, got a genuine
/// PASS, and committed real, correct, verified work — but the coordinator's
/// final captured turn was spent narrating a prose summary of the
/// environment issue instead of calling a tool to advance the card, and the
/// run parked before any `approve_review` decision was ever created.
/// `run_end_salvage_sweep` (Lane R31b, `orchestrator.rs`) already rescues
/// this at teardown, but the coordinator should not need rescuing — this
/// hook gives it one more chance to act itself, before that backstop is the
/// only thing left.
///
/// Distinct from [`NoRoadmapGuard`]'s own stranded-Ready nudge (R31b): that
/// one fires on R26b's `resumed_by_headless_unblock_at` stamp (a goal
/// resumed from Held with no verified work yet); this one fires on a real
/// `dispatch_evidence.verdict.status == "pass"` — a worker actually finished
/// and verified, not merely resumed.
///
/// Same applicability gate as [`NoRoadmapGuard`]
/// ([`crate::tool_inspection::is_top_level_coding_harness`]) — a dispatched
/// worker (which has a parent session) never reaches this check. Fires only
/// on a turn whose FINAL message has no tool call at all (the exact "prose
/// summary instead of advancing" shape A32 traced,
/// [`final_message_has_tool_call`]) — a turn that called ANY tool, even an
/// unrelated one, is not this shape and is left to the other guards. At most
/// once PER GOAL, not once per session: a coordinator juggling several goals
/// may need the nudge again for a second one even after acting on the first.
pub struct VerifiedGoalAwaitingLandGuard;

impl VerifiedGoalAwaitingLandGuard {
    pub fn new() -> Self {
        Self
    }

    /// True when [`verified_landing_nudge_text`] for `goal_id` (in either
    /// action shape) is already in the conversation.
    fn already_nudged_for_goal(messages: &[Message], goal_id: &str) -> bool {
        use crate::conversation::message::MessageContent;
        let needle = format!("goal {goal_id} {VERIFIED_LANDING_NUDGE_MARKER}");
        messages
            .iter()
            .flat_map(|m| m.content.iter())
            .any(|c| matches!(c, MessageContent::Text(text) if text.text.contains(&needle)))
    }

    /// The first (if any) of `cards` that is `ready` or `review` and carries
    /// `dispatch_evidence.verdict.status == "pass"` — a real, observed worker
    /// PASS, the same field
    /// [`crate::decisions_effects::approve_review_verification_status`] reads
    /// for the identical "was this goal actually verified" question. One
    /// extra [`crate::cards::get_card`] fetch per ready/review candidate only
    /// ([`crate::cards::SessionGoalCard`] itself carries no `metadata_json`)
    /// — never a scan of the whole roadmap. A card read failure is treated as
    /// "not a candidate" (fail open, same posture as every other lookup in
    /// this file).
    async fn verified_landing_candidate(
        pool: &sqlx::Pool<sqlx::Sqlite>,
        cards: &[crate::cards::SessionGoalCard],
    ) -> Option<(String, VerifiedLandingAction)> {
        for card in cards {
            let action = match card.state_binding.as_str() {
                "ready" => VerifiedLandingAction::Review,
                "review" => VerifiedLandingAction::Approve,
                _ => continue,
            };
            let Ok(Some(full)) = crate::cards::get_card(pool, &card.card_id).await else {
                continue;
            };
            let verified = full
                .metadata_json
                .pointer("/dispatch_evidence/verdict/status")
                .and_then(serde_json::Value::as_str)
                .map(|status| status.eq_ignore_ascii_case("pass"))
                .unwrap_or(false);
            if verified {
                return Some((card.card_id.clone(), action));
            }
        }
        None
    }
}

impl Default for VerifiedGoalAwaitingLandGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AfterTurn for VerifiedGoalAwaitingLandGuard {
    fn name(&self) -> &'static str {
        "verified_goal_awaiting_land"
    }

    async fn after_turn(&self, ctx: &AfterTurnContext<'_>) -> AfterTurnAction {
        // The exact A32 shape: a turn that ends with no tool call at all. A
        // turn that called any tool (even one unrelated to landing) is left
        // to the other guards.
        if final_message_has_tool_call(ctx.messages) {
            return AfterTurnAction::Allow;
        }

        let manager = crate::session::SessionManager::instance();
        // Fail open on a degraded session store — this is a quality-policy
        // nudge, not a correctness gate, same posture as `NoRoadmapGuard`.
        let Ok(session) = manager.get_session(ctx.session_id, false).await else {
            return AfterTurnAction::Allow;
        };
        let title = session.recipe.as_ref().map(|recipe| recipe.title.as_str());
        if !crate::tool_inspection::is_top_level_coding_harness(
            title,
            session.parent_session_id.as_deref(),
        ) {
            return AfterTurnAction::Allow;
        }

        let Ok(pool) = manager.pool_clone().await else {
            return AfterTurnAction::Allow;
        };
        let Ok(roadmap_cards) =
            goal_cards_relevant_to_session(&pool, ctx.session_id, &session.working_dir).await
        else {
            return AfterTurnAction::Allow;
        };

        let Some((goal_id, action)) = Self::verified_landing_candidate(&pool, &roadmap_cards).await
        else {
            return AfterTurnAction::Allow;
        };
        if Self::already_nudged_for_goal(ctx.messages, &goal_id) {
            return AfterTurnAction::Allow;
        }

        AfterTurnAction::Continue {
            inject: verified_landing_nudge_text(&goal_id, &action),
        }
    }
}

// ── Fifth implementor: rule 5, an honest failure for a headless free-text turn ──

/// Whether the turn's FINAL message contains at least one `ToolRequest` — the
/// narrow signature [`Rule5HonestFailureGuard`] cares about. A turn that ends
/// by calling literally anything (even a tool unrelated to the roadmap) is
/// not the Class 4 shape; only a pure free-text ending is.
fn final_message_has_tool_call(messages: &[Message]) -> bool {
    use crate::conversation::message::MessageContent;
    let Some(last) = messages.last() else {
        return false;
    };
    last.content
        .iter()
        .any(|content| matches!(content, MessageContent::ToolRequest(_)))
}

/// Whether any tool call anywhere in `messages` with a base name in `names`
/// actually SUCCEEDED — the complement of merely being attempted.
///
/// Lane R18f (A17 analysis Class 4, HARNESS_LOOP_LOG.md): [`turn_called_any_tool`]
/// cannot distinguish a `decompose_roadmap` call that landed a roadmap from
/// one that came back `-32002: Tool not found` — both are "a `ToolRequest`
/// with this name exists". [`NoRoadmapGuard`] only needs that weaker "was it
/// attempted" reading (a call denied by a DIFFERENT guard is still evidence
/// rule 1 was followed); [`Rule5HonestFailureGuard`] needs the stricter one,
/// because "tried `decompose_roadmap`, it errored, then asked a free-text
/// question" is exactly the shape that left Class 4 with zero decisions
/// recorded and no guard firing.
fn turn_tool_call_succeeded(messages: &[Message], names: &[&str]) -> bool {
    use crate::conversation::message::MessageContent;
    use std::collections::HashSet;

    let mut pending = HashSet::new();
    for msg in messages {
        for content in &msg.content {
            match content {
                MessageContent::ToolRequest(req) => {
                    let Ok(call) = req.tool_call.as_ref() else {
                        continue;
                    };
                    let base = call.name.rsplit("__").next().unwrap_or(call.name.as_ref());
                    if names.contains(&base) {
                        pending.insert(req.id.clone());
                    }
                }
                MessageContent::ToolResponse(response) => {
                    if !pending.remove(&response.id) {
                        continue;
                    }
                    if let Ok(result) = response.tool_result.as_ref() {
                        if result.is_error != Some(true) {
                            return true;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    false
}

/// Lane R20c-b: how many CONSECUTIVE coordinator turns, counting backward
/// from the most recent, ended with no tool call at all — the count
/// [`Rule5HonestFailureGuard`] uses for its own two-strike rule, independent
/// of `prior_holds` (the budget EVERY after-turn hook shares — see
/// [`NO_ROADMAP_INJECT_OPENING`]'s doc comment). Only `Role::Assistant`
/// messages count as turn boundaries; an injected nudge (`Message::user()`)
/// or a tool response is skipped without affecting the count. A turn with
/// ANY tool call — not only a roadmap/escalate call — resets the streak to
/// zero the moment scanning reaches it, the same "any tool call at all"
/// definition [`final_message_has_tool_call`] uses for the CURRENT turn.
///
/// M22 zai round w1 (A21 analysis, HARNESS_LOOP_LOG.md "Lane R20c-b"):
/// before this fix, [`Rule5HonestFailureGuard::decide`] gated its own
/// Continue/Park choice on the SHARED `prior_holds < MAX_HOLDS` (`MAX_HOLDS
/// == 2`). [`NoRoadmapGuard`]'s nudge on the first free-text turn already
/// spends one of those two shared holds, so on the SECOND free-text turn
/// this guard still read `prior_holds (1) < MAX_HOLDS (2)` as true and
/// injected its OWN extra nudge ([`RULE5_INJECT_TEXT`]) instead of parking —
/// a three-attempt cascade (two nudges, then park on the third), not the
/// two-strike rule the guard's own doc comment already claimed. Root cause
/// of THAT specific round's instant exits was unrelated (the zai provider
/// had exhausted its credit balance, confirmed via every
/// `data/logs/llm_request.N.jsonl`'s `error` field reading "Credits
/// exhausted"), but the extra-nudge-before-park defect this uncovered is
/// real and independent of what triggers the free-text turns in the first
/// place — it fires identically for any repeated free-text ending, billing
/// failure or otherwise.
fn consecutive_free_text_coordinator_turns(messages: &[Message]) -> u8 {
    use crate::conversation::message::MessageContent;
    let mut count: u8 = 0;
    for msg in messages.iter().rev() {
        if msg.role != rmcp::model::Role::Assistant {
            continue;
        }
        let has_tool_call = msg
            .content
            .iter()
            .any(|c| matches!(c, MessageContent::ToolRequest(_)));
        if has_tool_call {
            break;
        }
        count = count.saturating_add(1);
    }
    count
}

/// Lane R18f: set the one time [`Rule5HonestFailureGuard`] actually parks a
/// headless coding-harness turn on a rule-5 free-text ending.
/// `AfterTurnAction::Park` on its own only ends the turn with a message in
/// the transcript (see `agent.rs`'s reply loop) — nothing about it makes
/// `permagent run`'s process exit non-zero, so this shape would otherwise
/// still read as a clean exit to the eval harness the way Class 4 originally
/// did. `handle_run_command` (`goose-cli/src/cli.rs`) checks
/// [`take_rule5_free_text_turn`] after `session.headless()` returns and turns
/// an otherwise-`Ok` result into an error so the eval marks the run errored
/// instead of passing silently. Process-global by design, matching
/// `crate::receipts`'s own convention: the CLI runs exactly one headless
/// session per process.
static RULE5_FREE_TEXT_TURN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Read-and-clear. `false` unless [`Rule5HonestFailureGuard`] parked a turn
/// since the last call (or ever, in a process that never ran headless).
pub fn take_rule5_free_text_turn() -> bool {
    RULE5_FREE_TEXT_TURN.swap(false, std::sync::atomic::Ordering::SeqCst)
}

const RULE5_INJECT_TEXT: &str =
    "Your turn is ending in free text with no tool call and no roadmap. Per rule 5: call \
     decompose_roadmap (or create_roadmap) now with concrete nodes, or — if the task is \
     genuinely impossible with the tools available to you — call escalate. A question to the \
     user or a prose plan is not a valid way to end this turn in unattended mode.";

const RULE5_PARK_REASON: &str =
    "rule5_free_text_turn: the coding coordinator's turn ended in free text — no roadmap, no \
     dispatched goal, no escalate, and no tool call at all in the final message — on the second \
     consecutive turn to end this way. Call decompose_roadmap or escalate; a free-text answer \
     is not a valid way to end this turn unattended.";

/// Lane R20c: the mid-session sibling of [`RULE5_INJECT_TEXT`] — a roadmap
/// already exists, at least one goal is still not Done, and the turn ended
/// on a free-text question instead of a tool call.
const RULE5_MIDSESSION_INJECT_TEXT: &str =
    "Your turn is ending in free text that reads as a question or a request for a decision, \
     with an existing roadmap and at least one goal not yet Done. Per rule 5: any question to \
     the user is an escalate call, never prose — call escalate with the exact question, or, if \
     no decision is actually needed, call check_worker, goal_advance, or resume_roadmap to keep \
     the roadmap moving. A closing summary is fine once every goal is Done; this is not that.";

/// Lane R20c: the mid-session sibling of [`RULE5_PARK_REASON`]. The marker
/// `rule5_free_text_question` (distinct from `rule5_free_text_turn`) is the
/// receipt-visible name for this shape — see
/// [`Rule5HonestFailureGuard::after_turn`]'s `harness_error` field.
const RULE5_MIDSESSION_PARK_REASON: &str =
    "rule5_free_text_question: the coding coordinator's turn ended on a free-text question or \
     decision request — an existing roadmap has a goal that is not yet Done, no escalate was \
     called, and no tool call at all appeared in the final message — on the second consecutive \
     turn to end this way. A question to the user must be an escalate call, never prose.";

/// Lane R58a: the nudge for a headless coordinator that ended a turn in free
/// text while a Ready root goal it could dispatch sat undispatched — the
/// `select-checks-worker-before-dispatch` r59 q4 shape, where the coordinator
/// probed `check_worker`/`routing_roster` and then asked the absent user to
/// pick the `worker_tier`. Names the resolution path, never dispatches for
/// the model.
const RULE5_READY_GOAL_INJECT_TEXT: &str =
    "Headless run: no user will answer. A Ready goal is still undispatched and nothing was \
     dispatched or escalated. Make the routing choice yourself — the worker and the cheapest \
     `worker_tier` the `routing_roster` marks capable for the goal — and call `goal_advance` with \
     action `dispatch` (or `resume_roadmap` if the roadmap is paused), or call `escalate` with a \
     concrete blocker. Asking the user to choose a worker, tier, or spend in prose is not a valid \
     way to end this turn.";

/// Lane R58a: the second-strike sibling of [`RULE5_READY_GOAL_INJECT_TEXT`],
/// only surfaced when minting the escalation itself fails. The marker
/// `rule5_ready_goal_undispatched` is the greppable `harness_error` name.
const RULE5_READY_GOAL_PARK_REASON: &str =
    "rule5_ready_goal_undispatched: the headless coding coordinator's turn ended in free text \
     while a Ready goal was still undispatched — no goal_advance dispatch, no escalate, and no \
     tool call at all in the final message — on the second consecutive turn to end this way. \
     Make the routing choice and dispatch, or escalate a concrete blocker.";

/// Lane R58a: the tools a coordinator reaches for while PREPARING a dispatch
/// — any one of them attempted in the transcript is what separates "probed
/// the roster, then asked the user to pick" (r59 q4: `check_worker` +
/// `routing_roster`; r56 m3: `routing_roster` + `list_workers`) from a session
/// about something else entirely that merely shares a cwd with a stale Ready
/// card. Attempted, not succeeded: a refused `goal_advance` still counts.
const DISPATCH_PREP_TOOL_NAMES: [&str; 6] = [
    "check_worker",
    "list_workers",
    "routing_roster",
    "goal_list",
    "goal_status",
    "goal_advance",
];

/// Lane R58a: whether `cards` holds a goal the coordinator could dispatch
/// right now and never has — a ROOT goal (no dependencies to wait on) sitting
/// `ready` with no worker session ever recorded. The same shape as
/// [`crate::cards::first_undispatched_goal`]'s first clause, minus its
/// in-progress clause (which is a different defect). A goal that was already
/// dispatched once (R26b's resumed-from-Held stamp, R35a's verified PASS)
/// carries a worker session and is left to [`NoRoadmapGuard`] and
/// [`VerifiedGoalAwaitingLandGuard`].
fn has_undispatched_ready_root_goal(cards: &[crate::cards::SessionGoalCard]) -> bool {
    cards
        .iter()
        .any(|c| c.is_root && c.state_binding == "ready" && !c.has_worker_session)
}

/// Headless-only backstop: a coding-harness coordinator turn that ends with
/// no roadmap, no dispatched goal, no `escalate`, and NO TOOL CALL AT ALL in
/// its final assistant message is a free-text ending — the exact shape
/// `permagent-coding.yaml` rule 5 forbids ("Never end a turn without a
/// roadmap … If the task is genuinely impossible, call escalate").
///
/// Lane R18f (A17 analysis Class 4, HARNESS_LOOP_LOG.md): a DeepSeek round's
/// first `decompose_roadmap` call came back `-32002`; the model correctly
/// declined to work around the missing tool, but then asked a free-text
/// question instead of calling `escalate`. [`NoRoadmapGuard`] did not fire —
/// its `called_roadmap_tool` check only asks whether a `ToolRequest` with
/// that name exists, not whether it SUCCEEDED, so the failed attempt read as
/// "rule 1 satisfied" and the turn ended with zero decisions recorded. This
/// guard closes that gap with the stricter [`turn_tool_call_succeeded`]
/// check, scoped to headless only (`GooseMode::Auto`, the same signal
/// `build_session::wants_headless_goal_dispatch` reads) — an interactive
/// operator sees the same free-text answer directly and can reply, so
/// nothing here changes [`NoRoadmapGuard`]'s existing interactive behaviour.
///
/// Lane R20c (A20 analysis Q3, HARNESS_LOOP_LOG.md, fixture
/// `select-missing-credential-escalates-not-guesses`): the original guard
/// above only fires on a BARE session — no roadmap cards yet at all. A
/// zai glm-4.7 round showed the same rule-5 violation reached MID-session
/// instead: roadmap/dispatch calls already existed, then the coordinator hit
/// a missing credential and asked the question in free text (`send_message`)
/// rather than calling `escalate`. Outside the original guard's scope by
/// construction (`has_roadmap_cards` was true, so it returned `Allow`). This
/// extends the same guard: once a roadmap already exists, a turn that ends
/// with no tool call AND reads as a question/decision request (see
/// [`final_message_reads_as_question`]) AND still has a goal that is not
/// Done is the identical dishonest-exit shape, just reached from a
/// different starting state. A legitimate closing summary — every goal
/// already Done/Cancelled — never fires, and an explicit `escalate` call
/// anywhere in the turn (the cheapest gate below) still releases both
/// shapes identically.
///
/// Lane R20c-b (A21 analysis, HARNESS_LOOP_LOG.md "Lane R20c-b"): both
/// shapes above are meant to be TWO-STRIKE — the first free-text turn gets
/// a nudge, the second consecutive one parks. Until this fix the Continue-
/// vs-Park choice was gated on `prior_holds < MAX_HOLDS`, a budget SHARED
/// with every other after-turn hook (see [`NoRoadmapGuard`]'s own doc
/// comment on why it cannot use that counter either). Since
/// [`NoRoadmapGuard`]'s turn-1 nudge already spends one of the two shared
/// holds before this guard is even the deciding vote, the shared budget let
/// a SECOND free-text turn still read as "under budget" and get an extra
/// nudge of its own — three attempts before park, not two. `decide` now
/// counts strikes via [`consecutive_free_text_coordinator_turns`] instead,
/// a value derived purely from the transcript and specific to THIS guard's
/// own two-strike rule; `prior_holds >= MAX_HOLDS` stays as an OR'd
/// defensive ceiling only, never a reason to grant an extra nudge.
///
/// Lane R24b (M26 DeepSeek m1, HARNESS_LOOP_LOG.md "Lane R24b"): a second
/// strike whose final message reads as a question is not graded as a
/// dishonest exit anymore — see [`mint_rule5_question_escalation`] and
/// [`rule5_park_should_escalate_instead`]. The park itself is redirected
/// into minting the `escalate` decision the model's own prose already
/// represents, and the turn ends `Allow` (exit 0) instead. A second strike
/// whose final message is NOT a question (a plan in prose, a refusal,
/// silence) is unaffected by this and still parks exactly as before.
///
/// Lane R58a (r59 q4, `select-checks-worker-before-dispatch`): a third shape,
/// keyed on durable state rather than phrasing — a true headless CLI run, a
/// Ready root goal no worker ever took, no successful `goal_advance`, and a
/// free-text ending that none of the shapes above caught (the coordinator
/// asked the absent user to pick a `worker_tier` without a `?` or a listed
/// phrase). Nudge on the first strike, minted escalation on the second — see
/// [`Rule5HonestFailureGuard::decide_undispatched_ready_goal`].
pub struct Rule5HonestFailureGuard;

impl Rule5HonestFailureGuard {
    pub fn new() -> Self {
        Self
    }

    /// The whole policy as one pure fold, mirroring [`NoRoadmapGuard::decide`]'s
    /// shape so both the original bare-session shape and Lane R20c's
    /// mid-session extension are unit-testable without a session store or a
    /// database. `final_message_has_tool_call`/`turn_called_escalate` stay
    /// OUTSIDE this function (transcript-only, cheapest, checked first in the
    /// live path) so they need no DB round trip at all — a call reaching this
    /// function has already established neither is true for the turn.
    ///
    /// R20c-b: `consecutive_free_text_turns` (see
    /// [`consecutive_free_text_coordinator_turns`]) drives the two-strike
    /// Continue/Park choice, NOT the shared `prior_holds`. The FIRST
    /// free-text turn (count == 1) always gets a nudge; the SECOND
    /// consecutive one (count >= 2) always parks — regardless of whether
    /// some other hook already spent a shared hold earlier in this same
    /// reply for an unrelated reason. `prior_holds >= MAX_HOLDS` stays as a
    /// second, OR'd condition: a defensive ceiling so this guard can never
    /// exceed the reply loop's own global hold budget, but it no longer
    /// GRANTS an extra nudge the two-strike count would have refused.
    fn decide(
        is_headless_coding_coordinator: bool,
        roadmap_tool_succeeded_this_turn: bool,
        has_roadmap_cards: bool,
        has_active_roadmap: bool,
        final_message_reads_as_question: bool,
        consecutive_free_text_turns: u8,
        prior_holds: u8,
    ) -> AfterTurnAction {
        if !is_headless_coding_coordinator || roadmap_tool_succeeded_this_turn {
            return AfterTurnAction::Allow;
        }

        // Two distinct shapes fire, sharing everything past this point:
        //   - bare session: no roadmap ever requested (R18f's original).
        //   - mid session: a roadmap exists, at least one goal is not yet
        //     Done, and the final message reads as a question (R20c).
        let bare_session = !has_roadmap_cards;
        let mid_session_question =
            has_roadmap_cards && has_active_roadmap && final_message_reads_as_question;
        if !bare_session && !mid_session_question {
            return AfterTurnAction::Allow;
        }

        let second_strike =
            consecutive_free_text_turns >= 2 || prior_holds >= crate::cost_router::MAX_HOLDS;
        if !second_strike {
            AfterTurnAction::Continue {
                inject: if mid_session_question {
                    RULE5_MIDSESSION_INJECT_TEXT
                } else {
                    RULE5_INJECT_TEXT
                }
                .to_string(),
            }
        } else {
            AfterTurnAction::Park {
                reason: if mid_session_question {
                    RULE5_MIDSESSION_PARK_REASON
                } else {
                    RULE5_PARK_REASON
                }
                .to_string(),
            }
        }
    }

    /// Lane R58a: the undispatched-Ready-goal shape, folded AFTER
    /// [`Self::decide`] and only where it returned `Allow` — so neither of
    /// that function's shapes (nor its tests) changes. [`Self::decide`]'s
    /// mid-session shape needs the final message to read as a question, and
    /// r59 q4's routing ask ("One thing I need from you before I dispatch …
    /// I won't pick that one silently") matched none of
    /// [`RULE5_QUESTION_PHRASES`] and had no trailing `?`, so a seeded Ready
    /// card with nothing dispatched ended the run with `escalated=0` and
    /// `goals=0`. This shape keys on durable state instead of phrasing.
    ///
    /// Narrow by construction: the TRUE headless-CLI signal (not merely Auto
    /// mode — an interactive desktop Auto session has a user who can answer),
    /// a Ready root goal no worker ever took, a coordinator that was preparing
    /// a dispatch ([`DISPATCH_PREP_TOOL_NAMES`]), and no successful
    /// `goal_advance` anywhere in the transcript. Same two-strike rule as [`Self::decide`]:
    /// nudge on the first free-text turn, park on the second — which
    /// `after_turn` redirects into R24b's minted escalation.
    fn decide_undispatched_ready_goal(
        decided: AfterTurnAction,
        is_headless_cli_run: bool,
        has_undispatched_ready_goal: bool,
        prepared_dispatch: bool,
        goal_advance_succeeded: bool,
        consecutive_free_text_turns: u8,
        prior_holds: u8,
    ) -> AfterTurnAction {
        if decided != AfterTurnAction::Allow
            || !is_headless_cli_run
            || !has_undispatched_ready_goal
            || !prepared_dispatch
            || goal_advance_succeeded
        {
            return decided;
        }
        let second_strike =
            consecutive_free_text_turns >= 2 || prior_holds >= crate::cost_router::MAX_HOLDS;
        if second_strike {
            AfterTurnAction::Park {
                reason: RULE5_READY_GOAL_PARK_REASON.to_string(),
            }
        } else {
            AfterTurnAction::Continue {
                inject: RULE5_READY_GOAL_INJECT_TEXT.to_string(),
            }
        }
    }
}

impl Default for Rule5HonestFailureGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// Lane R20c: small, DOCUMENTED phrase list distinguishing a free-text
/// question/decision-request from a legitimate closing summary. Deliberately
/// short: a false negative here only lets a genuine rule-5 violation through
/// (the same fail-open bias every other check in this guard already makes),
/// while a false positive would wrongly park an innocent status update — so
/// the list stays small and specific rather than broad and sensitive.
const RULE5_QUESTION_PHRASES: [&str; 5] = [
    "please provide",
    "which",
    "should i",
    "let me know",
    "confirm",
];

/// Whether `needle` appears in `haystack` as a whole word or phrase, not
/// merely as a substring of a larger word — so `"which"` matches "which file
/// should I edit?" but not "sandwich". Both arguments are expected already
/// lowercased by the caller.
// Every index below is a char boundary by construction: `rel` comes from
// `str::find`, which only ever returns the start of a match, and `after` is
// that start plus the matched needle's own byte length — i.e. the match's end
// boundary. So these slices cannot split a UTF-8 character.
#[allow(clippy::string_slice)]
fn contains_word_or_phrase(haystack: &str, needle: &str) -> bool {
    let mut start = 0usize;
    while let Some(rel) = haystack[start..].find(needle) {
        let pos = start + rel;
        let before_ok = haystack[..pos]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_ascii_alphanumeric());
        let after = pos + needle.len();
        let after_ok = haystack[after..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_ascii_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        start = pos + 1;
    }
    false
}

/// The turn's FINAL message, as trimmed joined text — `None` if the last
/// message has no text content (or only whitespace). Factored out of
/// [`final_message_reads_as_question`] so Lane R24b's escalation minting can
/// reuse the exact same text a question was detected in, rather than
/// re-deriving it (and risking the two readings drifting apart).
fn final_message_text(messages: &[Message]) -> Option<String> {
    use crate::conversation::message::MessageContent;
    let last = messages.last()?;
    let text: String = last
        .content
        .iter()
        .filter_map(|c| match c {
            MessageContent::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Lane R20c: whether the turn's FINAL message reads as a question or a
/// request for information/a decision, per [`RULE5_QUESTION_PHRASES`] plus a
/// trailing `?`. Scoped to the TEXT content of the last message only — by
/// the time the live path calls this, [`final_message_has_tool_call`] has
/// already established that message has no tool call.
fn final_message_reads_as_question(messages: &[Message]) -> bool {
    let Some(trimmed) = final_message_text(messages) else {
        return false;
    };
    if trimmed.ends_with('?') {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    RULE5_QUESTION_PHRASES
        .iter()
        .any(|phrase| contains_word_or_phrase(&lower, phrase))
}

// ── Lane R24b: a free-text question at rule-5 park time is the escalation
// it looks like ─────────────────────────────────────────────────────────

/// The plain-language headline (Amendment A1: max 80 chars, no PR/issue
/// numbers, paths, file counts, or internal IDs) for an escalation the
/// rule-5 guard mints on the model's behalf. Deliberately a FIXED,
/// always-clean string rather than derived from the model's own free text:
/// the model's question can be arbitrary length and may contain exactly the
/// tokens Amendment A1 forbids in a headline (a card id, a path, a PR
/// number), and deriving one from raw prose risks landing the mint on
/// `DecisionKind::Malformed` (Tier 2, generic headline) for what is, in
/// substance, an ordinary answerable ask. The model's actual question is
/// preserved VERBATIM in `why_blocked` (the technical-detail field, only
/// length-capped) — nothing about the ask is lost, only the plain-language
/// summary is templated.
const RULE5_ESCALATION_HEADLINE: &str = "A coordinator question needs a human decision";

/// Deliberately small, documented heuristic — matching
/// [`final_message_reads_as_question`]'s own bias toward simple, checkable
/// rules rather than a full parse of the model's prose: a literal `" or "`
/// between two alternatives reads as the model offering the user a choice
/// (`"decision"`, which [`crate::decision_inbox::escalate::draft_from_payload`]
/// reconstructs into a proceed/decline pair when — as here — no structured
/// `options` list is given; see that function's and
/// `default_decision_options`'s own doc comments). Anything else — including
/// the M26 defect's own "which credential should I use?" shape — is a plain
/// `"information"` ask. A false negative here (a real either/or question
/// phrased without the literal word "or") still mints a perfectly answerable
/// `information` escalation; it only affects which of the two default
/// choice layouts the human sees.
fn rule5_escalation_kind(question_text: &str) -> &'static str {
    if question_text.to_ascii_lowercase().contains(" or ") {
        "decision"
    } else {
        "information"
    }
}

/// Build the JSON payload [`crate::decision_inbox::escalate::draft_from_payload`]
/// expects — the same schema `handle_escalate` validates a live `escalate`
/// tool call against — from the rule-5 guard's own final-message text
/// rather than an actual tool call.
fn rule5_escalation_payload(question_text: &str) -> serde_json::Value {
    serde_json::json!({
        "kind": rule5_escalation_kind(question_text),
        "specific_ask": RULE5_ESCALATION_HEADLINE,
        "why_blocked": question_text,
        "evidence_refs": ["minted_by:rule5_guard"],
        "resume": "auto",
    })
}

/// Lane R24b: mint the escalation a free-text question at rule-5 park time
/// represents, exactly as the `escalate` tool itself would (M26 DeepSeek m1,
/// HARNESS_LOOP_LOG.md "Lane R24b": a coordinator asked "which credential…?"
/// in prose twice and the two-strike rule-5 guard ended the run as a
/// dishonest failure — the question WAS the right escalation, only the form
/// was wrong).
///
/// Takes an INJECTED sink, mirroring `orchestrator::record_escalation`'s own
/// test-friendly design (see that function's doc comment) rather than
/// reading `escalate::global_decision_sink()` directly here — so a unit
/// test can assert on an [`crate::decision_inbox::escalate::InMemoryDecisionSink`]
/// without installing anything into either of `decision_inbox::escalate`'s
/// or `receipts`'s process-wide globals. The live caller
/// ([`Rule5HonestFailureGuard::after_turn`]) passes the real global sink.
///
/// Records through the exact same seam `handle_escalate`/`record_escalation`
/// use — `draft_from_payload` then `DecisionSink::record` — so the minted
/// decision gets the same `decisions`/`decision_audit` rows a real
/// `escalate` call would (`SqlDecisionSink::record`, installed for every
/// headless run, writes both atomically); increments the same
/// `dag_enforcement.escalated` receipt counter `record_escalation` does; and
/// prints the same tool-call-banner shape Lane R22a's
/// `ensure_worker_checked_for_dispatch` synthesizes for `check_worker` — the
/// ANSI-stripped `▸` glyph as the line's first non-whitespace character is
/// all `permagent_eval::harness_log::scan`'s `tool_call_banner_tool` needs
/// to record this as an `escalate` call, which is what lets the eval
/// driver's `dag_enforcement`/selection checks see it.
async fn mint_rule5_question_escalation(
    sink: &dyn crate::decision_inbox::escalate::DecisionSink,
    session_id: &str,
    question_text: &str,
) -> Result<crate::decision_inbox::escalate::RecordedDecision, String> {
    use crate::decision_inbox::escalate as escalate_tool;

    let source = escalate_tool::DraftSource {
        session_id: Some(session_id.to_string()),
        worker_roster_name: None,
    };
    let raw = rule5_escalation_payload(question_text);
    let draft = escalate_tool::draft_from_payload(raw, &source);
    let recorded = sink
        .record(draft)
        .await
        .map_err(|e| format!("Lane R24b: failed to record the rule-5-minted escalation: {e}"))?;

    // Same observation point `record_escalation` uses for a real `escalate`
    // call — see that function's own doc comment. A no-op unless this
    // process is an active headless CLI run collecting receipts
    // (`receipts::is_active`).
    crate::receipts::record_count("dag_enforcement", "escalated", 1);

    // Lane R22a precedent (`ensure_worker_checked_for_dispatch`,
    // `orchestrator.rs`): print the exact banner shape a real tool call's
    // own rendering would produce, so the harness log carries a `▸
    // escalate` line even though no live tool call happened.
    println!();
    println!("  {}", "─".repeat(40));
    println!("  ▸ escalate");

    Ok(recorded)
}

/// Whether a park-bound rule-5 decision should be redirected into a minted
/// escalation instead of an honest failure: the guard is about to `Park`
/// AND the turn's final message reads as a question. Pure predicate, kept
/// separate from [`Rule5HonestFailureGuard::after_turn`] so the override
/// condition itself is unit-testable without a live session.
fn rule5_park_should_escalate_instead(action: &AfterTurnAction, reads_as_question: bool) -> bool {
    matches!(action, AfterTurnAction::Park { .. }) && reads_as_question
}

#[async_trait]
impl AfterTurn for Rule5HonestFailureGuard {
    fn name(&self) -> &'static str {
        "rule5_honest_failure"
    }

    async fn after_turn(&self, ctx: &AfterTurnContext<'_>) -> AfterTurnAction {
        // Cheapest gates first, transcript-only, so the overwhelmingly common
        // case (a turn that ended with SOME tool call, or an escalate) costs
        // no session read and no DB round trip.
        if final_message_has_tool_call(ctx.messages) || turn_called_escalate(ctx.messages) {
            return AfterTurnAction::Allow;
        }

        let manager = crate::session::SessionManager::instance();
        // Fail open on a degraded session store, the same choice every other
        // guard in this file makes for the identical lookup.
        let Ok(session) = manager.get_session(ctx.session_id, false).await else {
            return AfterTurnAction::Allow;
        };
        if session.goose_mode != crate::config::GooseMode::Auto {
            return AfterTurnAction::Allow;
        }
        let title = session.recipe.as_ref().map(|recipe| recipe.title.as_str());
        if !crate::tool_inspection::is_top_level_coding_harness(
            title,
            session.parent_session_id.as_deref(),
        ) {
            return AfterTurnAction::Allow;
        }

        if turn_tool_call_succeeded(ctx.messages, &ROADMAP_TOOL_NAMES) {
            return AfterTurnAction::Allow;
        }

        let Ok(pool) = manager.pool_clone().await else {
            return AfterTurnAction::Allow;
        };
        // R20c: fetch the cards themselves, not just the bare "has any"
        // boolean `NoRoadmapGuard::has_roadmap_cards` returns — the
        // mid-session extension needs each card's own state to answer
        // "is any goal still not Done" via `session_has_an_active_roadmap`.
        // R20c-c: `goal_cards_relevant_to_session` (not the narrower
        // `crate::cards::goal_cards_created_by_session` this used before)
        // so a RESUMED roadmap this session never created is seen too.
        let Ok(cards) =
            goal_cards_relevant_to_session(&pool, ctx.session_id, &session.working_dir).await
        else {
            return AfterTurnAction::Allow;
        };
        let has_roadmap_cards = !cards.is_empty();
        let has_active_roadmap = crate::cards::session_has_an_active_roadmap(&cards);
        let reads_as_question = final_message_reads_as_question(ctx.messages);
        // R20c-b: the two-strike count, not the shared `prior_holds` — see
        // `Self::decide`'s doc comment and
        // `consecutive_free_text_coordinator_turns`'s own doc comment for why.
        let consecutive_free_text_turns = consecutive_free_text_coordinator_turns(ctx.messages);

        let action = Self::decide(
            true,  // is_top_level_coding_harness already confirmed above
            false, // turn_tool_call_succeeded already checked false above
            has_roadmap_cards,
            has_active_roadmap,
            reads_as_question,
            consecutive_free_text_turns,
            ctx.prior_holds,
        );

        // Lane R58a: a seeded/resumed Ready root goal the coordinator never
        // dispatched, with a free-text ending that did not read as a question
        // — see `Self::decide_undispatched_ready_goal`. Only consulted where
        // `decide` allowed the turn, so the shapes never compete.
        use crate::agents::platform_extensions::orchestrator::is_headless_dispatch_policy_installed;
        let ready_goal_shape = action == AfterTurnAction::Allow
            && has_undispatched_ready_root_goal(&cards)
            && is_headless_dispatch_policy_installed();
        let action = if ready_goal_shape {
            Self::decide_undispatched_ready_goal(
                action,
                true, // is_headless_dispatch_policy_installed checked above
                true, // has_undispatched_ready_root_goal checked above
                turn_called_any_tool(ctx.messages, &DISPATCH_PREP_TOOL_NAMES),
                turn_tool_call_succeeded(ctx.messages, &["goal_advance"]),
                consecutive_free_text_turns,
                ctx.prior_holds,
            )
        } else {
            action
        };
        let ready_goal_shape = ready_goal_shape && action != AfterTurnAction::Allow;

        if let AfterTurnAction::Park { .. } = &action {
            // Lane R24b: a free-text turn about to be parked whose final
            // message reads as a question is not a dishonest exit — it is
            // the escalation rule 5 already asks for, just in the wrong
            // form (prose instead of an `escalate` call). Mint that
            // escalation on the model's behalf and end the turn cleanly
            // instead of throwing the whole run away as a rule-5 failure. A
            // free-text turn that is NOT a question (a plan in prose, a
            // refusal, silence) never enters this branch and keeps today's
            // honest failure below unchanged.
            // Lane R58a: the undispatched-Ready-goal shape mints on its
            // second strike whether or not the prose reads as a question —
            // its trigger is durable state, not phrasing, and an honest
            // `escalated=1` beats a silent `goals=0` exit.
            if rule5_park_should_escalate_instead(&action, reads_as_question || ready_goal_shape) {
                if let Some(question_text) = final_message_text(ctx.messages) {
                    let sink = crate::decision_inbox::escalate::global_decision_sink();
                    match mint_rule5_question_escalation(
                        sink.as_ref(),
                        ctx.session_id,
                        &question_text,
                    )
                    .await
                    {
                        Ok(recorded) => {
                            tracing::info!(
                                session_id = %ctx.session_id,
                                harness_note = "rule5_question_escalated",
                                decision_id = %recorded.decision_id,
                                "Lane R24b: rule-5 guard minted the escalation a free-text \
                                 question represents instead of parking the turn"
                            );
                            // Lane R26a: the same headless-information-answer
                            // policy a live `escalate` tool call gets
                            // (`orchestrator::record_escalation`) — call into
                            // it here too, at the ONE seam this lane touches
                            // in this file, so a rule-5-minted `information`
                            // question about an implementation detail does
                            // not sit open forever with no human ever going
                            // to see it (this decision has no goal binding —
                            // `rule5_escalation_payload`'s evidence_refs name
                            // only `minted_by:rule5_guard` — so nothing else
                            // will ever resolve it). Gated on the TRUE
                            // headless-CLI signal, not merely "Auto mode +
                            // top-level coding harness" (this guard's own
                            // gate above, which an interactive desktop-app
                            // Auto session can also satisfy).
                            use crate::agents::platform_extensions::orchestrator as headless_policy;
                            let headless = headless_policy::is_headless_dispatch_policy_installed();
                            let cap = headless_policy::parse_headless_escalate_cap(
                                std::env::var("PERMAGENT_HEADLESS_ESCALATE_CAP")
                                    .ok()
                                    .as_deref(),
                            );
                            match headless_policy::headless_information_answer_policy(
                                &pool,
                                &recorded.decision_id,
                                Some(ctx.session_id),
                                headless,
                                cap,
                            )
                            .await
                            {
                                Ok(Some(answer)) => {
                                    tracing::info!(
                                        session_id = %ctx.session_id,
                                        harness_note = "headless_information_answered",
                                        decision_id = %recorded.decision_id,
                                        "Lane R26a: headless policy auto-answered the rule-5-minted \
                                         information question; continuing the coordinator's turn"
                                    );
                                    return AfterTurnAction::Continue {
                                        inject: format!(
                                            "Your question has been answered automatically (headless \
                                             policy, decision {}): {}",
                                            recorded.decision_id, answer
                                        ),
                                    };
                                }
                                Ok(None) => {}
                                Err(error) => {
                                    tracing::error!(
                                        session_id = %ctx.session_id,
                                        harness_error = "headless_information_answer_failed",
                                        decision_id = %recorded.decision_id,
                                        error = %error,
                                        "Lane R26a: headless information-answer policy errored; \
                                         the decision stays open for a human, same as today"
                                    );
                                }
                            }
                            return AfterTurnAction::Allow;
                        }
                        Err(error) => {
                            // Minting failed (e.g. a degraded session store) —
                            // fall through to the ordinary honest-failure park
                            // below rather than silently claiming a clean exit
                            // for an escalation that was never actually
                            // recorded.
                            tracing::error!(
                                session_id = %ctx.session_id,
                                harness_error = "rule5_question_escalation_mint_failed",
                                error = %error,
                                "Lane R24b: failed to mint the rule-5 question's escalation; \
                                 falling back to the honest-failure park"
                            );
                        }
                    }
                }
            }

            // R18f's original marker for the bare-session shape; R20c's new
            // marker for the mid-session question shape — same mechanism
            // R18f used to make this a receipt-visible, greppable failure
            // rather than a silent hold.
            let mid_session_question = has_roadmap_cards && has_active_roadmap && reads_as_question;
            let harness_error = if ready_goal_shape {
                "rule5_ready_goal_undispatched"
            } else if mid_session_question {
                "rule5_free_text_question"
            } else {
                "rule5_free_text_turn"
            };
            tracing::error!(
                session_id = %ctx.session_id,
                harness_error = harness_error,
                "Lane R18f/R20c: headless coding-harness coordinator ended a turn in free text — \
                 honest failure, not a clean exit"
            );
            RULE5_FREE_TEXT_TURN.store(true, std::sync::atomic::Ordering::SeqCst);
        }

        action
    }
}

// ── R28a: turn-budget nudge for dispatched goal workers ─────────────────────
//
// HARNESS_LOOP_LOG.md "## A27 analysis — zai z1 misses on M27 (16/19)", misses
// 1/2: a dispatched worker writes a correct, on-spec fix in its own goal
// worktree, then spends its whole remaining `max_turns` budget on ad-hoc
// manual shell re-testing and never calls `git commit` — the session ends
// with `agent.rs`'s literal "I've reached the maximum number of actions..."
// message, `maybe_salvage_committed_work` correctly finds nothing to salvage,
// and a substantively-complete fix is lost in full. This is a proactive
// nudge, not an [`AfterTurn`] hook: the existing `AfterTurn` seam only fires
// when the model already believes ITS OWN turn is over (`exit_chat`), which
// never happens here — the model keeps calling tools right up to the hard
// cap. `turn_budget_nudge_text` is instead consulted unconditionally, once
// per session, from `agent.rs`'s own turn-counting loop, the same place the
// S5 monologue nudge (`monologue_nudged`) already lives.

/// How many turns before the hard `max_turns` cap the nudge fires. "≤3 turns
/// left" per the lane brief.
pub const TURN_BUDGET_NUDGE_THRESHOLD: u32 = 3;

/// Whether a dispatched worker session's turn budget is low enough to warn
/// about, and if so, the notice to inject into the next prompt.
///
/// `turns_taken` is the 1-based count `agent.rs` already tracks (incremented
/// before this would be called, matching its own `turns_taken += 1` ordering
/// relative to the `turns_taken > max_turns` cap check). `has_explicit_cap`
/// is `session_config.max_turns.is_some()` — deliberately reused as the
/// "is this a dispatched goal worker" signal instead of a second session-type
/// lookup: `subagent_task_config::TaskConfig` always sets `max_turns` for a
/// dispatched worker ("TaskConfig always sets max_turns"), while an ordinary
/// interactive session leaves `SessionConfig::max_turns` `None` and falls
/// back to the much larger `GOOSE_MAX_TURNS` default — so the two are already
/// distinguishable for free, with no extra DB round trip inside the hot loop.
///
/// Returns `None` before the threshold (so a normal session mid-budget is
/// untouched) and `None` when `has_explicit_cap` is false (an ordinary chat
/// is never told to `git commit`, which would be nonsensical advice outside
/// a goal worktree). The caller is expected to fire this at most once per
/// session — bounding re-entry is the caller's job, the same shape as
/// `monologue_nudged` bounds the S5 nudge in `agent.rs`, so this function
/// stays a pure question of "turns_taken/max_turns," not "have I already
/// said this."
///
/// Lane R62f (A61 miss 2): `has_explicit_cap` alone is not enough — a
/// headless coordinator run with `--max-turns` also has one, and was told to
/// `git commit` NOW. Callers pass [`turn_budget_nudge_applies`] instead.
pub fn turn_budget_nudge_text(
    turns_taken: u32,
    max_turns: u32,
    has_explicit_cap: bool,
) -> Option<String> {
    if !has_explicit_cap {
        return None;
    }
    let remaining = max_turns.saturating_sub(turns_taken);
    if remaining > TURN_BUDGET_NUDGE_THRESHOLD {
        return None;
    }
    let (action_word, verb) = if remaining == 1 {
        ("action", "remains")
    } else {
        ("actions", "remain")
    };
    Some(format!(
        "{remaining} {action_word} {verb}. If your change is complete, run the tests once more \
         only if needed and `git add -A && git commit -m '<summary>'` NOW; uncommitted work is \
         discarded."
    ))
}

/// Lane R62f (A61 miss 2): whether a session is a dispatched goal worker the
/// turn-budget nudge is for — an explicit `max_turns` cap AND a `SubAgent`
/// session (goal workers are always `SubAgent`, see `security::write_jail`'s
/// `JailRole::GoalWorker`). A top-level coordinator (a `User`/`Terminal`
/// session) given a turn cap by the headless CLI is not in a goal worktree
/// and must never be told to `git commit`.
pub fn turn_budget_nudge_applies(
    max_turns: Option<u32>,
    session_type: crate::session::SessionType,
) -> bool {
    max_turns.is_some() && matches!(session_type, crate::session::SessionType::SubAgent)
}

#[cfg(test)]
mod turn_budget_nudge_tests {
    use super::*;

    /// Lane R62f (A61 miss 2): the s3 named-check coordinator, run headless
    /// with a turn cap, received "3 actions remain … git commit NOW".
    #[test]
    fn nudge_applies_only_to_capped_sub_agent_sessions() {
        use crate::session::SessionType;
        assert!(turn_budget_nudge_applies(Some(25), SessionType::SubAgent));
        for coordinator in [
            SessionType::User,
            SessionType::Terminal,
            SessionType::Scheduled,
            SessionType::Hidden,
        ] {
            assert!(
                !turn_budget_nudge_applies(Some(15), coordinator),
                "{coordinator:?} with a turn cap is not a goal worker"
            );
        }
        assert!(!turn_budget_nudge_applies(None, SessionType::SubAgent));
    }

    #[test]
    fn no_nudge_without_an_explicit_cap() {
        // An ordinary interactive session (`max_turns: None` in
        // `SessionConfig`) must never be told to `git commit` — it is not
        // running in a goal worktree at all.
        assert_eq!(turn_budget_nudge_text(22, 25, false), None);
        assert_eq!(turn_budget_nudge_text(25, 25, false), None);
    }

    #[test]
    fn no_nudge_before_the_threshold() {
        // 25 - 21 = 4 remaining: one MORE than the threshold, must not fire.
        assert_eq!(turn_budget_nudge_text(21, 25, true), None);
    }

    #[test]
    fn nudge_fires_exactly_at_the_threshold() {
        // 25 - 22 = 3 remaining: exactly the threshold, must fire.
        let text = turn_budget_nudge_text(22, 25, true).expect("must nudge at remaining == 3");
        assert!(text.starts_with("3 actions remain."), "{text:?}");
        assert!(text.contains("git add -A && git commit -m"), "{text:?}");
        assert!(text.contains("uncommitted work is discarded"), "{text:?}");
    }

    #[test]
    fn nudge_keeps_firing_as_the_count_falls_to_the_very_last_turn() {
        assert!(turn_budget_nudge_text(23, 25, true)
            .unwrap()
            .starts_with("2 actions remain."));
        assert!(turn_budget_nudge_text(24, 25, true)
            .unwrap()
            .starts_with("1 action remains"));
        // turns_taken == max_turns: this is the LAST turn `agent.rs` will
        // run (the very next increment trips `turns_taken > max_turns` and
        // ends the session) — the most urgent case, must still fire, not be
        // excluded as an edge case.
        assert!(turn_budget_nudge_text(25, 25, true)
            .unwrap()
            .starts_with("0 actions remain."));
    }

    #[test]
    fn singular_wording_at_exactly_one_remaining() {
        let text = turn_budget_nudge_text(24, 25, true).unwrap();
        assert!(
            text.starts_with("1 action remains."),
            "expected singular 'action'/'remains', got {text:?}"
        );
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::{Message, MessageContent};
    use rmcp::model::{CallToolRequestParams, CallToolResult, Content, Role};

    fn tool_exchange(name: &str, id: &str, ok: bool) -> Message {
        tool_exchange_with_text(name, id, if ok { "out" } else { "failed" }, ok)
    }

    fn tool_exchange_with_text(name: &str, id: &str, text: &str, ok: bool) -> Message {
        let mut result = if ok {
            CallToolResult::success(vec![Content::text(text)])
        } else {
            CallToolResult::error(vec![Content::text(text)])
        };
        if crate::agents::platform_extensions::orchestrator::is_verify_tool_name(name) {
            result.structured_content = Some(serde_json::json!({
                "kind": crate::agents::platform_extensions::developer::verify::VERIFICATION_OBSERVATION_KIND,
                "command": "test",
                "verdict": if ok { "pass" } else { "fail" },
                "evidence": text,
            }));
        }
        Message::new(
            Role::Assistant,
            0,
            vec![
                MessageContent::tool_request(id, Ok(CallToolRequestParams::new(name.to_string()))),
                MessageContent::tool_response(id, Ok(result)),
            ],
        )
    }

    /// X-verify-2: a verify tool exchange that reports an authoritative PASS
    /// but is marked vacuous — mirrors what `developer::verify`'s real
    /// `pass_result`/`no_checks_result` stamp for a syntax-only smoke, a
    /// no-checks pass, or a zero-tests run.
    fn tool_exchange_vacuous_pass(id: &str, vacuous_reason: &str) -> Message {
        let mut result = CallToolResult::success(vec![Content::text("PASS")]);
        result.structured_content = Some(serde_json::json!({
            "kind": crate::agents::platform_extensions::developer::verify::VERIFICATION_OBSERVATION_KIND,
            "command": "python3 -m py_compile a.py",
            "verdict": "pass",
            "evidence": "PASS",
            "vacuous_reason": vacuous_reason,
        }));
        Message::new(
            Role::Assistant,
            0,
            vec![
                MessageContent::tool_request(
                    id,
                    Ok(CallToolRequestParams::new("developer__verify".to_string())),
                ),
                MessageContent::tool_response(id, Ok(result)),
            ],
        )
    }

    fn ctx<'a>(messages: &'a [Message], prior_holds: u8) -> AfterTurnContext<'a> {
        AfterTurnContext {
            session_id: "sess-1",
            messages,
            prior_holds,
            // Every existing test in this module predates the delegate-tool
            // gate and assumes the tool IS loaded; the dedicated case below
            // (`live_after_turn_parks_when_delegate_tool_is_not_loaded`)
            // builds its own context with this false instead of adding a
            // parameter every one of ~20 call sites would have to thread
            // through unchanged.
            delegate_tool_available: true,
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
                assert!(
                    inject.starts_with("Do not declare this done."),
                    "discipline text must lead, inject was {inject:?}"
                );
                assert!(
                    inject.contains("do not narrate the verify attempts"),
                    "inject was {inject:?}"
                );
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
    async fn a_plaintext_pass_without_structured_evidence_does_not_advance() {
        let guard = PrematureDoneGuard::new();
        let result = CallToolResult::success(vec![Content::text("PASS - all checks passed")]);
        let mut messages = vec![tool_exchange("developer__edit", "e1", true)];
        messages.push(Message::new(
            Role::Assistant,
            0,
            vec![
                MessageContent::tool_request(
                    "v1",
                    Ok(CallToolRequestParams::new("developer__verify")),
                ),
                MessageContent::tool_response("v1", Ok(result)),
            ],
        ));
        assert!(matches!(
            guard.after_turn(&ctx(&messages, 0)).await,
            AfterTurnAction::Continue { .. }
        ));
    }

    /// The live bug, end to end. A passing cargo suite prints `0 failed`;
    /// `"failed"` was in the transcript-signal failure list, so two green runs
    /// scored as a spin and this guard injected "Verify is still failing the
    /// same way" AFTER a pass. Signals now read the wire's `is_error`.
    #[tokio::test]
    async fn a_green_run_whose_output_says_zero_failed_is_allowed_to_finish() {
        let pass = "test result: ok. 42 passed; 0 failed; 0 ignored";
        let guard = PrematureDoneGuard::new();
        let done = vec![
            tool_exchange("developer__edit", "e1", true),
            tool_exchange_with_text("developer__verify", "v1", pass, true),
            tool_exchange_with_text("developer__verify", "v2", pass, true),
        ];
        assert_eq!(
            guard.after_turn(&ctx(&done, 0)).await,
            AfterTurnAction::Allow,
            "a passing suite must never be answered with 'still failing'"
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

    /// Lane R62a (A61 bug A): a conversation reloaded from the `messages`
    /// table can hold a tool response ahead of its own request. The verify
    /// after the edit must still pair and cover it.
    #[test]
    fn scan_positions_pairs_a_response_stored_before_its_request() {
        let swapped = |exchange: Message| {
            let mut content = exchange.content.into_iter();
            let request = content.next().unwrap();
            let response = content.next().unwrap();
            [
                Message::new(Role::User, 0, vec![response]),
                Message::new(Role::Assistant, 0, vec![request]),
            ]
        };
        let messages: Vec<Message> = swapped(tool_exchange("developer__edit", "e1", true))
            .into_iter()
            .chain(swapped(tool_exchange("developer__verify", "v1", true)))
            .collect();
        let p = scan_verify_positions(&messages);
        assert!(p.last_mutation.is_some(), "the swapped edit must pair");
        assert!(p.last_verify.is_some(), "the swapped verify must pair");
        assert!(
            !p.unverified_edits(),
            "a genuine verify after the edit covers it"
        );
    }

    // ── X-verify-2: vacuous passes ─────────────────────────────────────────

    #[test]
    fn a_vacuous_pass_is_recorded_and_treated_as_unverified() {
        let p = scan_verify_positions(&[
            tool_exchange("developer__edit", "e1", true),
            tool_exchange_vacuous_pass("v1", "only a syntax-only smoke check ran"),
        ]);
        assert!(p.last_mutation.is_some());
        // The position IS recorded (a pass did happen)…
        assert!(p.last_verify.is_some());
        // …but it does not count as verifying the edit.
        assert!(p.vacuous_pass);
        assert_eq!(
            p.vacuous_reason.as_deref(),
            Some("only a syntax-only smoke check ran")
        );
        assert!(
            p.unverified_edits(),
            "a vacuous pass must not rubber-stamp the edit"
        );
    }

    #[test]
    fn a_genuine_pass_after_a_vacuous_one_clears_the_flag() {
        let p = scan_verify_positions(&[
            tool_exchange("developer__edit", "e1", true),
            tool_exchange_vacuous_pass("v1", "0 tests run"),
            tool_exchange("developer__verify", "v2", true),
        ]);
        assert!(
            !p.vacuous_pass,
            "the LATEST verify is genuine; an earlier vacuous one must not linger"
        );
        assert!(!p.unverified_edits());
    }

    // Lane R62b-3: holding on a vacuous pass records `vacuous_refused` in the
    // shared receipts registry when a run is active.
    #[tokio::test]
    #[serial_test::serial(receipts_global_registry)]
    async fn a_vacuous_pass_holds_with_an_honest_message_naming_the_reason() {
        let guard = PrematureDoneGuard::new();
        let turn = vec![
            tool_exchange("developer__edit", "e1", true),
            tool_exchange_vacuous_pass("v1", "only a syntax-only smoke check ran"),
        ];
        match guard.after_turn(&ctx(&turn, 0)).await {
            AfterTurnAction::Continue { inject } => {
                assert!(inject.contains("PASSED"), "inject was {inject:?}");
                assert!(
                    inject.contains("only a syntax-only smoke check ran"),
                    "inject was {inject:?}"
                );
            }
            other => panic!("expected a Continue (bounded hold), got {other:?}"),
        }
    }

    // Lane R62b-3: see the hold test above.
    #[tokio::test]
    #[serial_test::serial(receipts_global_registry)]
    async fn a_repeatedly_vacuous_pass_parks_honestly_once_the_hold_budget_is_exhausted() {
        let guard = PrematureDoneGuard::new();
        let turn = vec![
            tool_exchange("developer__edit", "e1", true),
            tool_exchange_vacuous_pass("v1", "0 tests run"),
        ];
        match guard
            .after_turn(&ctx(&turn, crate::cost_router::MAX_HOLDS))
            .await
        {
            AfterTurnAction::Park { reason } => {
                assert!(reason.contains("0 tests run"), "{reason:?}");
                assert!(reason.contains("PASSED"), "{reason:?}");
            }
            other => panic!("expected Park, got {other:?}"),
        }
    }

    /// Lane R62b-3 (coordinator ruling on A61 miss 2): the guard refusing a
    /// vacuous pass counts `vacuous_refused`, once per call, and never the
    /// gated `vacuous_passes`.
    #[tokio::test]
    #[serial_test::serial(receipts_global_registry)]
    async fn refusing_a_vacuous_pass_counts_vacuous_refused_not_vacuous_passes() {
        let guard = PrematureDoneGuard::new();
        let turn = vec![
            tool_exchange("developer__edit", "e1", true),
            tool_exchange_vacuous_pass("v1", "0 tests run"),
        ];
        let positions = scan_verify_positions(&turn);
        assert_eq!(positions.vacuous_call_id.as_deref(), Some("v1"));

        crate::receipts::begin_run();
        let first = guard.after_turn(&ctx(&turn, 0)).await;
        let second = guard.after_turn(&ctx(&turn, 1)).await;
        let lines = crate::receipts::snapshot().lines();
        crate::receipts::end_run();

        assert!(matches!(first, AfterTurnAction::Continue { .. }));
        assert!(!matches!(second, AfterTurnAction::Allow));
        assert!(
            lines[1].contains("vacuous_passes=0") && lines[1].ends_with("vacuous_refused=1"),
            "got: {}",
            lines[1]
        );
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

    // ── ReviewerMandate ──────────────────────────────────────────────────────

    fn delegate_exchange_result(
        id: &str,
        worker_persona: &str,
        verdict: &str,
        ok: bool,
    ) -> Message {
        let call = CallToolRequestParams::new("delegate".to_string()).with_arguments(
            serde_json::Map::from_iter([(
                "worker_persona".to_string(),
                serde_json::Value::String(worker_persona.to_string()),
            )]),
        );
        let output = format!("VERDICT: {verdict}\nCHECKED: the changed files");
        let result = if ok {
            CallToolResult::success(vec![Content::text(output)])
        } else {
            CallToolResult::error(vec![Content::text(output)])
        };
        Message::new(
            Role::Assistant,
            0,
            vec![
                MessageContent::tool_request(id, Ok(call)),
                MessageContent::tool_response(id, Ok(result)),
            ],
        )
    }

    fn delegate_exchange(id: &str, worker_persona: &str) -> Message {
        delegate_exchange_result(id, worker_persona, "APPROVE", true)
    }

    /// A `delegate(worker_persona: reviewer)` call whose response carries NO
    /// text content at all — `ok` controls whether it is a "successful" empty
    /// response or an errored one; [`last_reviewer_outcome`] treats both as
    /// [`ReviewerLastOutcome::Empty`]. Mirrors Lane A's live evidence: a real,
    /// billed subagent call that returned nothing.
    fn delegate_exchange_empty(id: &str, worker_persona: &str, ok: bool) -> Message {
        let call = CallToolRequestParams::new("delegate".to_string()).with_arguments(
            serde_json::Map::from_iter([(
                "worker_persona".to_string(),
                serde_json::Value::String(worker_persona.to_string()),
            )]),
        );
        let result = if ok {
            CallToolResult::success(vec![])
        } else {
            CallToolResult::error(vec![])
        };
        Message::new(
            Role::Assistant,
            0,
            vec![
                MessageContent::tool_request(id, Ok(call)),
                MessageContent::tool_response(id, Ok(result)),
            ],
        )
    }

    /// Two KB rows from different families — mirrors
    /// `cost_router::reviewer_pick`'s own test fixture so this suite never
    /// hard-codes a vendor.
    fn two_families() -> (
        &'static crate::cost_router::ModelKnowledge,
        &'static crate::cost_router::ModelKnowledge,
    ) {
        let a = crate::cost_router::KNOWN_MODELS
            .first()
            .expect("knowledge base is not empty");
        let b = crate::cost_router::KNOWN_MODELS
            .iter()
            .find(|m| m.family != a.family)
            .expect("knowledge base has at least two families");
        (a, b)
    }

    /// A real `Ready` pick over the full knowledge base, for tests that need the
    /// injected text rather than the decision.
    fn ready_pick() -> Box<crate::cost_router::ReviewerPick> {
        let (worker, _) = two_families();
        let available: Vec<crate::cost_router::AvailableModel> = crate::cost_router::KNOWN_MODELS
            .iter()
            .map(|m| crate::cost_router::AvailableModel::new(m.provider, m.model))
            .collect();
        match crate::cost_router::select_reviewer(
            Some((worker.provider, worker.model)),
            None,
            None,
            &available,
            10,
        ) {
            crate::cost_router::ReviewerSelection::Reviewer(p) => p,
            other => panic!("expected a reviewer, got {other:?}"),
        }
    }

    /// A neutral, cross-family `ActualReviewerRoute` for tests that only care
    /// about the fixed parts of the injected text (E3 has its own dedicated
    /// tests for the same-family wording).
    fn cross_family_route(pick: &crate::cost_router::ReviewerPick) -> ActualReviewerRoute {
        ActualReviewerRoute {
            label: pick.label(),
            same_family: false,
        }
    }

    /// The route matching [`ready_pick`], for tests that need to reconstruct
    /// the exact ask text [`ReviewerMandate::after_turn`] would have injected.
    fn ready_route() -> ActualReviewerRoute {
        cross_family_route(&ready_pick())
    }

    /// Case 1 — No mutating tool call → Allow, and no reviewer selection is
    /// attempted (the applicability gate returns before `assess` runs, so this
    /// is safe to exercise through the real, DB-touching `after_turn`).
    #[tokio::test]
    async fn no_mutation_allows_without_attempting_a_reviewer_pick() {
        let mandate = ReviewerMandate::new();
        let plain = vec![Message::assistant().with_text("Here is the answer.")];
        assert_eq!(
            mandate.after_turn(&ctx(&plain, 0)).await,
            AfterTurnAction::Allow
        );

        let read_only = vec![tool_exchange("developer__search", "a", true)];
        assert_eq!(
            mandate.after_turn(&ctx(&read_only, 0)).await,
            AfterTurnAction::Allow
        );
    }

    /// Case 2 — A mutation with no reviewer delegation yet → `Continue`, naming the
    /// `delegate` tool and the `reviewer` persona. Exercises the pure `decide`
    /// directly with a `Ready` availability built from the real
    /// `select_reviewer`, so this never touches a database.
    #[test]
    fn a_mutation_with_no_delegation_yet_asks_for_one() {
        // The full known-models catalog, like `reviewer_pick`'s own
        // `best_fit_picks_the_cheapest_capable_different_family_model` test —
        // a two-candidate list risks neither clearing the capability floor.
        let (worker, _) = two_families();
        let available: Vec<crate::cost_router::AvailableModel> = crate::cost_router::KNOWN_MODELS
            .iter()
            .map(|m| crate::cost_router::AvailableModel::new(m.provider, m.model))
            .collect();
        let selection = crate::cost_router::select_reviewer(
            Some((worker.provider, worker.model)),
            None,
            None,
            &available,
            10,
        );
        let pick = match selection {
            crate::cost_router::ReviewerSelection::Reviewer(p) => p,
            other => panic!("expected a reviewer, got {other:?}"),
        };
        let route = cross_family_route(&pick);
        let availability = ReviewerAvailability::Ready(pick, route);
        match decide(true, false, AskState::NotAsked, true, &availability) {
            AfterTurnAction::Continue { inject } => {
                assert!(inject.contains("delegate"), "inject was {inject:?}");
                let lower = inject.to_ascii_lowercase();
                assert!(lower.contains("worker_persona"), "{inject:?}");
                assert!(lower.contains("reviewer"), "{inject:?}");
            }
            other => panic!("expected Continue, got {other:?}"),
        }
    }

    /// CASE B fix #1 (main #1157): a reviewer WAS chosen and priced (`Ready`)
    /// but the `delegate` tool itself is not loaded — the recipe dropped
    /// `summon`. Before the fix, `decide` never asked this question at all and
    /// the mandate would `Continue`, injecting an instruction to call a tool
    /// that cannot be called; the one-ask backstop then `Allow`s and the turn
    /// ends "normal" with an unreviewed report. `delegate_available` must gate
    /// BEFORE `availability`, so a `Ready` pick never overrides it.
    #[test]
    fn missing_delegate_tool_parks_even_with_a_ready_reviewer() {
        let (worker, _) = two_families();
        let available: Vec<crate::cost_router::AvailableModel> = crate::cost_router::KNOWN_MODELS
            .iter()
            .map(|m| crate::cost_router::AvailableModel::new(m.provider, m.model))
            .collect();
        let selection = crate::cost_router::select_reviewer(
            Some((worker.provider, worker.model)),
            None,
            None,
            &available,
            10,
        );
        let pick = match selection {
            crate::cost_router::ReviewerSelection::Reviewer(p) => p,
            other => panic!("expected a reviewer, got {other:?}"),
        };
        let route = cross_family_route(&pick);
        let availability = ReviewerAvailability::Ready(pick, route);
        match decide(true, false, AskState::NotAsked, false, &availability) {
            AfterTurnAction::Park { reason } => {
                assert!(
                    reason.starts_with(REVIEW_PARK_PREFIX),
                    "reason was {reason:?}"
                );
                assert!(
                    reason.contains("delegate") && reason.contains("not loaded"),
                    "reason must say WHY: {reason:?}"
                );
            }
            other => panic!(
                "expected Park (the bug: Continue with 'Call the `delegate` tool now'), got \
                 {other:?}"
            ),
        }
    }

    /// Same case through the live `after_turn` path: a turn that mutated
    /// files, with `delegate_tool_available: false` on the context, must Park
    /// without ever reaching `assess` (no session need be readable for this
    /// to hold — the gate fires before the database is touched).
    #[tokio::test]
    async fn live_after_turn_parks_when_delegate_tool_is_not_loaded() {
        // Edit THEN a verify — otherwise `unverified_edits()` would return
        // Allow before this hook's delegate-availability check ever runs
        // (that ordering is deliberate: PrematureDoneGuard's turn first).
        let edited = vec![
            tool_exchange("developer__edit", "e1", true),
            tool_exchange("developer__verify", "v1", true),
        ];
        let live_ctx = AfterTurnContext {
            session_id: "sess-nonexistent",
            messages: &edited,
            prior_holds: 0,
            delegate_tool_available: false,
        };
        match ReviewerMandate::new().after_turn(&live_ctx).await {
            AfterTurnAction::Park { reason } => {
                assert!(reason.starts_with(REVIEW_PARK_PREFIX), "{reason:?}");
                assert!(reason.contains("not loaded"), "{reason:?}");
            }
            other => panic!("expected Park, got {other:?}"),
        }
    }

    /// CASE B fix #2 (Rust side of the gate): `delegate_tool_loaded` reads the
    /// resolved tool list the same way `is_reviewer_delegation` reads a tool
    /// call name — prefix-stripped on `__`, so both `summon__delegate` and the
    /// unprefixed `delegate` (summon registers it with `unprefixed_tools:
    /// true`) count, and an unrelated tool does not.
    #[test]
    fn delegate_tool_loaded_matches_prefixed_and_unprefixed_names() {
        use rmcp::model::Tool;
        fn tool(name: &str) -> Tool {
            Tool::new(
                name.to_string(),
                String::new(),
                serde_json::json!({"type": "object", "properties": {}})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
        }
        assert!(delegate_tool_loaded(&[tool("summon__delegate")]));
        assert!(delegate_tool_loaded(&[tool("delegate")]));
        assert!(!delegate_tool_loaded(&[tool("developer__text_editor")]));
        assert!(!delegate_tool_loaded(&[]));
    }

    /// Case 3 — A mutation FOLLOWED BY a `delegate` call carrying
    /// `worker_persona: "reviewer"` → Allow — satisfied, fires once, no loop.
    /// Short-circuits before `assess`, so safe through the real `after_turn`.
    #[tokio::test]
    async fn a_completed_reviewer_delegation_satisfies_the_mandate() {
        let mandate = ReviewerMandate::new();
        let turn = vec![
            tool_exchange("developer__edit", "e1", true),
            tool_exchange("developer__verify", "v1", true),
            delegate_exchange("d1", "reviewer"),
        ];
        assert_eq!(
            mandate.after_turn(&ctx(&turn, 0)).await,
            AfterTurnAction::Allow
        );

        // Case-insensitive, and a persona named after the fact still counts.
        let turn = vec![
            tool_exchange("developer__edit", "e1", true),
            tool_exchange("developer__verify", "v1", true),
            delegate_exchange("d1", "Reviewer"),
        ];
        assert_eq!(
            mandate.after_turn(&ctx(&turn, 0)).await,
            AfterTurnAction::Allow
        );
    }

    /// Case 4 — The ask is made once. Refusal then parks instead of looping or
    /// treating the request itself as a completed independent review.
    #[tokio::test]
    async fn an_ask_already_injected_is_never_repeated() {
        let mandate = ReviewerMandate::new();
        let asked = vec![
            tool_exchange("developer__edit", "e1", true),
            tool_exchange("developer__verify", "v1", true),
            Message::user().with_text(reviewer_inject_text(&ready_pick(), &ready_route())),
            Message::assistant().with_text("I would rather not."),
        ];
        assert!(matches!(
            mandate.after_turn(&ctx(&asked, 0)).await,
            AfterTurnAction::Park { .. }
        ));

        // And the shared counter is a backstop only, at 2 — one hold belongs to
        // `PrematureDoneGuard`.
        let verified = vec![
            tool_exchange("developer__edit", "e1", true),
            tool_exchange("developer__verify", "v1", true),
        ];
        assert!(matches!(
            mandate.after_turn(&ctx(&verified, 2)).await,
            AfterTurnAction::Park { .. }
        ));
    }

    /// A reviewer call that returned REAL content — just not an approval —
    /// still parks once asked. (The is_error/empty case moved to the
    /// dedicated E5 tests below: an errored or empty response is now
    /// retried, not parked, because nothing was actually reviewed.)
    #[tokio::test]
    async fn failed_or_non_approving_reviewer_never_satisfies_the_mandate() {
        let mandate = ReviewerMandate::new();
        for review in [
            delegate_exchange_result("d2", "reviewer", "REQUEST_CHANGES", true),
            delegate_exchange_result("d3", "reviewer", "UNCERTAIN", true),
            delegate_exchange_result("d4", "reviewer", "not a verdict", true),
        ] {
            let turn = vec![
                tool_exchange("developer__edit", "e1", true),
                tool_exchange("developer__verify", "v1", true),
                review,
                Message::user().with_text(reviewer_inject_text(&ready_pick(), &ready_route())),
            ];
            assert!(matches!(
                mandate.after_turn(&ctx(&turn, 0)).await,
                AfterTurnAction::Park { .. }
            ));
        }
    }

    /// E5, pinned two ways: an empty/errored reviewer call is retried
    /// (bounded), and only Parks — honestly — once the hold budget itself is
    /// exhausted; it never claims a verdict came back when nothing was
    /// actually reviewed.
    ///
    /// Before this repair, the failing assertion was:
    /// `assert!(matches!(mandate.after_turn(&ctx(&turn, 0)).await, AfterTurnAction::Continue { .. }))`
    /// — the old code treated ANY non-approving reviewer state (including a
    /// genuinely empty/errored call) as "already asked" and Parked
    /// immediately with "the reviewer did not return a successful APPROVE
    /// verdict," even though no review had actually happened.
    #[tokio::test]
    async fn an_empty_reviewer_response_reopens_the_hold_instead_of_parking() {
        let mandate = ReviewerMandate::new();
        for empty in [
            delegate_exchange_empty("d1", "reviewer", true), // success, but no content
            delegate_exchange_empty("d2", "reviewer", false), // an error
        ] {
            let turn = vec![
                tool_exchange("developer__edit", "e1", true),
                tool_exchange("developer__verify", "v1", true),
                empty,
            ];
            match mandate.after_turn(&ctx(&turn, 0)).await {
                AfterTurnAction::Continue { inject } => {
                    assert!(
                        inject.to_ascii_lowercase().contains("no answer")
                            || inject.to_ascii_lowercase().contains("empty"),
                        "inject was {inject:?}"
                    );
                }
                other => panic!("expected Continue (a bounded retry), got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn an_empty_reviewer_response_parks_honestly_once_the_hold_budget_is_exhausted() {
        let mandate = ReviewerMandate::new();
        let turn = vec![
            tool_exchange("developer__edit", "e1", true),
            tool_exchange("developer__verify", "v1", true),
            delegate_exchange_empty("d1", "reviewer", true),
        ];
        match mandate
            .after_turn(&ctx(&turn, crate::cost_router::MAX_HOLDS))
            .await
        {
            AfterTurnAction::Park { reason } => {
                assert!(reason.starts_with(REVIEW_PARK_PREFIX), "{reason:?}");
                assert!(
                    !reason.contains("did not return a successful APPROVE"),
                    "reason must not claim a verdict came back: {reason:?}"
                );
            }
            other => panic!("expected Park, got {other:?}"),
        }
    }

    // ── X-verify-1: reviewer mutation detector ──────────────────────────────

    /// A `delegate(worker_persona: reviewer)` call whose result `summon` has
    /// already stamped VOID because a before/after tree snapshot showed the
    /// reviewer mutated files (see `reviewer_integrity`). Mirrors Lane A2's
    /// live evidence: the reviewer's OWN text can still say `VERDICT:
    /// APPROVE` — that must never survive the void.
    fn delegate_exchange_voided(id: &str, paths: &[String], original_verdict: &str) -> Message {
        let call = CallToolRequestParams::new("delegate".to_string()).with_arguments(
            serde_json::Map::from_iter([(
                "worker_persona".to_string(),
                serde_json::Value::String("reviewer".to_string()),
            )]),
        );
        let text = reviewer_mutation_void_text(
            paths,
            &format!("VERDICT: {original_verdict}\nCHECKED: looks fine to me"),
        );
        let result = CallToolResult::success(vec![Content::text(text)]);
        Message::new(
            Role::Assistant,
            0,
            vec![
                MessageContent::tool_request(id, Ok(call)),
                MessageContent::tool_response(id, Ok(result)),
            ],
        )
    }

    #[test]
    fn the_void_sentinel_always_wins_over_the_reviewers_own_verdict() {
        use crate::cost_router::review_gate::{parse_review, Verdict};

        let paths = vec!["src/cache.rs".to_string()];
        let text = reviewer_mutation_void_text(&paths, "VERDICT: APPROVE\nCHECKED: fine");
        assert_eq!(
            parse_review(&text).verdict,
            Verdict::Uncertain,
            "a mutated-tree result must never parse as an approval, whatever the \
             reviewer's own text claims"
        );
    }

    #[test]
    fn void_paths_round_trip_and_require_the_sentinel_on_the_first_line() {
        let paths = vec!["src/cache.rs".to_string(), "src/lib.rs".to_string()];
        let text = reviewer_mutation_void_text(&paths, "VERDICT: APPROVE");
        assert_eq!(reviewer_mutation_void_paths(&text), Some(paths));

        // A reviewer's own text merely mentioning the sentinel string
        // somewhere in its body (not as the first line) must never be
        // mistaken for the real marker.
        let innocuous = format!("VERDICT: APPROVE\nCHECKED: I saw the string {REVIEWER_MUTATION_VOID_PREFIX}foo in a comment");
        assert_eq!(reviewer_mutation_void_paths(&innocuous), None);

        assert_eq!(reviewer_mutation_void_paths("no sentinel here"), None);
    }

    #[test]
    fn last_reviewer_outcome_reports_void_for_a_stamped_result() {
        let paths = vec!["src/cache.rs".to_string()];
        let messages = vec![delegate_exchange_voided("d1", &paths, "APPROVE")];
        assert_eq!(
            last_reviewer_outcome(&messages, 0),
            ReviewerLastOutcome::Void(paths)
        );
    }

    #[tokio::test]
    async fn a_voided_reviewer_response_reopens_the_hold_naming_the_paths() {
        let mandate = ReviewerMandate::new();
        let paths = vec!["src/cache.rs".to_string()];
        let turn = vec![
            tool_exchange("developer__edit", "e1", true),
            tool_exchange("developer__verify", "v1", true),
            delegate_exchange_voided("d1", &paths, "APPROVE"),
        ];
        match mandate.after_turn(&ctx(&turn, 0)).await {
            AfterTurnAction::Continue { inject } => {
                assert!(inject.contains("src/cache.rs"), "inject was {inject:?}");
                assert!(
                    inject.to_ascii_lowercase().contains("read-only")
                        || inject.to_ascii_lowercase().contains("modified"),
                    "inject was {inject:?}"
                );
            }
            other => panic!("expected Continue (a bounded retry), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_voided_reviewer_response_parks_honestly_once_the_hold_budget_is_exhausted() {
        let mandate = ReviewerMandate::new();
        let paths = vec!["src/cache.rs".to_string()];
        let turn = vec![
            tool_exchange("developer__edit", "e1", true),
            tool_exchange("developer__verify", "v1", true),
            delegate_exchange_voided("d1", &paths, "APPROVE"),
        ];
        match mandate
            .after_turn(&ctx(&turn, crate::cost_router::MAX_HOLDS))
            .await
        {
            AfterTurnAction::Park { reason } => {
                assert!(reason.starts_with(REVIEW_PARK_PREFIX), "{reason:?}");
                assert!(reason.contains("src/cache.rs"), "{reason:?}");
                assert!(
                    !reason.contains("did not return a successful APPROVE"),
                    "reason must not claim a verdict came back: {reason:?}"
                );
            }
            other => panic!("expected Park, got {other:?}"),
        }
    }

    /// A clean reviewer (no mutation detected) is completely unaffected — the
    /// void machinery never fires on an ordinary approval.
    #[tokio::test]
    async fn a_clean_reviewer_is_unaffected_by_the_void_machinery() {
        let mandate = ReviewerMandate::new();
        let turn = vec![
            tool_exchange("developer__edit", "e1", true),
            tool_exchange("developer__verify", "v1", true),
            delegate_exchange("d1", "reviewer"),
        ];
        assert_eq!(
            mandate.after_turn(&ctx(&turn, 0)).await,
            AfterTurnAction::Allow
        );
    }

    // ── X-verify-3: never repeat an identical failed reviewer gate unchanged ─

    #[test]
    fn repeated_identical_reviewer_verdict_detects_two_in_a_row_with_no_change() {
        let messages = vec![
            delegate_exchange_result("d1", "reviewer", "REQUEST_CHANGES", true),
            delegate_exchange_result("d2", "reviewer", "REQUEST_CHANGES", true),
        ];
        assert!(repeated_identical_reviewer_verdict(&messages).is_some());
    }

    #[test]
    fn repeated_identical_reviewer_verdict_allows_a_tree_change_between_them() {
        let messages = vec![
            delegate_exchange_result("d1", "reviewer", "REQUEST_CHANGES", true),
            tool_exchange("developer__edit", "e1", true),
            delegate_exchange_result("d2", "reviewer", "REQUEST_CHANGES", true),
        ];
        assert!(repeated_identical_reviewer_verdict(&messages).is_none());
    }

    #[test]
    fn repeated_identical_reviewer_verdict_ignores_a_differing_verdict() {
        let messages = vec![
            delegate_exchange_result("d1", "reviewer", "REQUEST_CHANGES", true),
            delegate_exchange_result("d2", "reviewer", "UNCERTAIN", true),
        ];
        assert!(repeated_identical_reviewer_verdict(&messages).is_none());
    }

    #[test]
    fn repeated_identical_reviewer_verdict_needs_at_least_two_rounds() {
        let messages = vec![delegate_exchange_result(
            "d1",
            "reviewer",
            "REQUEST_CHANGES",
            true,
        )];
        assert!(repeated_identical_reviewer_verdict(&messages).is_none());
    }

    /// The gate's own acceptance test, end to end through the real hook: two
    /// identical REQUEST_CHANGES verdicts with nothing edited in between must
    /// escalate (Park, quoting the repeated verdict) instead of injecting a
    /// third `delegate` ask.
    #[tokio::test]
    async fn two_identical_verdicts_with_no_tree_change_escalate_instead_of_a_third_delegate() {
        let mandate = ReviewerMandate::new();
        let turn = vec![
            tool_exchange("developer__edit", "e1", true),
            tool_exchange("developer__verify", "v1", true),
            delegate_exchange_result("d1", "reviewer", "REQUEST_CHANGES", true),
            delegate_exchange_result("d2", "reviewer", "REQUEST_CHANGES", true),
        ];
        match mandate.after_turn(&ctx(&turn, 0)).await {
            AfterTurnAction::Park { reason } => {
                assert!(reason.starts_with(REVIEW_PARK_PREFIX), "{reason:?}");
                assert!(reason.contains("identical verdict"), "{reason:?}");
                assert!(reason.contains("REQUEST_CHANGES"), "{reason:?}");
            }
            other => panic!("expected Park (escalate), got {other:?}"),
        }
    }

    /// The companion half of the same acceptance test: a real edit between the
    /// two rounds means the escalation must NOT fire — whatever else happens
    /// (here, a DB-less test session makes the reviewer pick itself
    /// unavailable), it must not be the "identical verdict" escalation.
    #[tokio::test]
    async fn a_tree_change_between_two_identical_verdicts_does_not_escalate() {
        let mandate = ReviewerMandate::new();
        let turn = vec![
            tool_exchange("developer__edit", "e1", true),
            tool_exchange("developer__verify", "v1", true),
            delegate_exchange_result("d1", "reviewer", "REQUEST_CHANGES", true),
            tool_exchange("developer__edit", "e2", true),
            tool_exchange("developer__verify", "v2", true),
            delegate_exchange_result("d2", "reviewer", "REQUEST_CHANGES", true),
        ];
        match mandate.after_turn(&ctx(&turn, 0)).await {
            AfterTurnAction::Park { reason } => {
                assert!(
                    !reason.contains("identical verdict"),
                    "a genuine edit between the two rounds must not trip the \
                     no-progress escalation: {reason:?}"
                );
            }
            other => panic!("expected Park (from the DB-less test session), got {other:?}"),
        }
    }

    /// The two hooks do not fight. While edits are unverified the mandate stays
    /// silent so `PrematureDoneGuard` can send the model back — a `Park` here
    /// would outrank the guard's `Continue` and end the turn instead.
    #[tokio::test]
    async fn an_unverified_edit_is_left_to_the_verify_guard() {
        let edited = vec![tool_exchange("developer__edit", "e1", true)];
        assert_eq!(
            ReviewerMandate::new().after_turn(&ctx(&edited, 0)).await,
            AfterTurnAction::Allow,
            "the mandate must not pre-empt the verify hold"
        );
        assert!(
            matches!(
                PrematureDoneGuard::new().after_turn(&ctx(&edited, 0)).await,
                AfterTurnAction::Continue { .. }
            ),
            "and the guard must still be the one that speaks"
        );
    }

    /// Case 5 — No reviewer available, or one available but unpriced → `Park`, and
    /// the reason says review did NOT run (via the shared prefix the CLI
    /// renderer matches on).
    #[test]
    fn no_reviewer_or_an_unpriced_one_parks_saying_so() {
        // No cross-family model at all.
        let (worker, _) = two_families();
        let available = vec![crate::cost_router::AvailableModel::new(
            worker.provider,
            worker.model,
        )];
        let selection = crate::cost_router::select_reviewer(
            Some((worker.provider, worker.model)),
            None,
            None,
            &available,
            10,
        );
        let reason = match selection {
            crate::cost_router::ReviewerSelection::Unavailable { reason } => reason,
            other => panic!("expected Unavailable, got {other:?}"),
        };
        let availability = ReviewerAvailability::Unavailable { reason };
        match decide(true, false, AskState::NotAsked, true, &availability) {
            AfterTurnAction::Park { reason } => {
                assert!(
                    reason.starts_with(REVIEW_PARK_PREFIX),
                    "reason was {reason:?}"
                );
            }
            other => panic!("expected Park, got {other:?}"),
        }

        // A reviewer WAS chosen, but it is unpriced — fail closed, never
        // billed as $0.00.
        let pick = Box::new(crate::cost_router::ReviewerPick {
            provider: "someco".into(),
            model: "some-1".into(),
            family: "someco".into(),
            worker_family: "otherco".into(),
            source: crate::cost_router::ReviewerSource::BestFit,
            cross_family: true,
            cost_hint_per_mtok: 1.0,
            input_usd_per_mtok: 1.0,
            output_usd_per_mtok: 3.0,
            priced: false,
            is_local: false,
            why: String::new(),
            warning: None,
        });
        let verdict = crate::cost_router::BudgetVerdict {
            band: crate::cost_router::BudgetBand::Ok,
            scope: crate::cost_router::BudgetScope::Task,
            spent: 0.0,
            crossed: 0.0,
            unpriced_calls: 0,
        };
        let spend_reason = match crate::cost_router::reviewer_spend_gate(&pick, &verdict) {
            crate::cost_router::SpendDecision::Refuse { reason } => reason,
            other => panic!("expected a refusal, got {other:?}"),
        };
        let availability = ReviewerAvailability::SpendRefused {
            reason: spend_reason,
        };
        match decide(true, false, AskState::NotAsked, true, &availability) {
            AfterTurnAction::Park { reason } => {
                assert!(reason.starts_with(REVIEW_PARK_PREFIX), "{reason:?}");
                assert!(reason.contains("no published price"), "{reason:?}");
            }
            other => panic!("expected Park, got {other:?}"),
        }
    }

    /// Case 6 — The config knob, honoured via the `REVIEWER_MANDATE` env var — the
    /// house `get_param::<bool>` pattern, same as `strix_enabled`.
    #[tokio::test]
    async fn the_config_knob_disables_the_mandate_and_defaults_on() {
        {
            let _g = env_lock::lock_env([("REVIEWER_MANDATE", Some("false"))]);
            assert!(!ReviewerMandate::new().is_enabled());
        }
        {
            let _g = env_lock::lock_env([("REVIEWER_MANDATE", Some("true"))]);
            assert!(ReviewerMandate::new().is_enabled());
        }
        {
            let _g = env_lock::lock_env([("REVIEWER_MANDATE", None::<&str>)]);
            assert!(
                ReviewerMandate::new().is_enabled(),
                "unset means on — mandated review should not need a second opt-in"
            );
        }
    }

    // ── E1: honour the recipe's own trivial/docs/test/format-only skip ──────

    /// A real scratch git repo, one commit deep — mirrors
    /// `tool_monitor::inspector_flags_a_test_gaming_pass`'s pattern, since
    /// [`trivial_skip_reason_for_dir`] shells out to `git` and cannot be
    /// exercised without one.
    fn scratch_git_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success());
        };
        git(&["-c", "init.defaultBranch=main", "init", "-q"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "user.name", "Test"]);
        std::fs::write(dir.path().join("lib.rs"), "fn main() {}\n").unwrap();
        git(&["add", "lib.rs"]);
        git(&["commit", "--no-gpg-sign", "-qm", "init"]);
        dir
    }

    /// Docs-only mutation + passing verify → the mandate must be skippable —
    /// the recipe's own contract (`permagent-coding.yaml:88-91`).
    ///
    /// Before this repair, no such function existed: `ReviewerMandate::after_turn`
    /// went straight from "edits verified" to `assess`/delegate for ANY
    /// mutation, so a docs-only diff still asked for a reviewer. Pinned here
    /// against the real `git`-backed helper rather than the DB-coupled
    /// `after_turn`, per the task's own escape hatch.
    #[tokio::test]
    async fn trivial_skip_allows_a_docs_only_diff() {
        let _g = env_lock::lock_env([(REVIEWER_MANDATE_SKIP_TRIVIAL_ENV, None::<&str>)]);
        let repo = scratch_git_repo();
        std::fs::write(
            repo.path().join("NOTES.md"),
            "one\ntwo\nthree\nfour\nfive\n",
        )
        .unwrap();
        let reason = trivial_skip_reason_for_dir(repo.path()).await;
        assert!(reason.is_some(), "a docs-only diff must be skippable");
    }

    /// ≤3 changed lines on a `.rs` file → skippable (`TRIVIAL_MAX_LINES`).
    #[tokio::test]
    async fn trivial_skip_allows_a_tiny_rust_diff() {
        let _g = env_lock::lock_env([(REVIEWER_MANDATE_SKIP_TRIVIAL_ENV, None::<&str>)]);
        let repo = scratch_git_repo();
        std::fs::write(repo.path().join("lib.rs"), "fn main() {}\n// tiny\n").unwrap();
        let reason = trivial_skip_reason_for_dir(repo.path()).await;
        assert!(reason.is_some(), "a <=3-line rust diff must be skippable");
    }

    /// Regression guard: a substantive product-source diff must still require
    /// review — the skip must never over-fire.
    #[tokio::test]
    async fn trivial_skip_still_requires_review_for_a_substantive_rust_diff() {
        let _g = env_lock::lock_env([(REVIEWER_MANDATE_SKIP_TRIVIAL_ENV, None::<&str>)]);
        let repo = scratch_git_repo();
        let big: String = (0..20).map(|i| format!("// line {i}\n")).collect();
        std::fs::write(repo.path().join("lib.rs"), format!("fn main() {{}}\n{big}")).unwrap();
        let reason = trivial_skip_reason_for_dir(repo.path()).await;
        assert!(
            reason.is_none(),
            "a substantive product-source diff must still require review, got {reason:?}"
        );
    }

    /// The knob, off, restores the old always-review behaviour — the A/B
    /// tool's escape hatch.
    #[tokio::test]
    async fn trivial_skip_knob_off_restores_old_behaviour() {
        let _g = env_lock::lock_env([(REVIEWER_MANDATE_SKIP_TRIVIAL_ENV, Some("0"))]);
        let repo = scratch_git_repo();
        std::fs::write(
            repo.path().join("NOTES.md"),
            "one\ntwo\nthree\nfour\nfive\n",
        )
        .unwrap();
        let reason = trivial_skip_reason_for_dir(repo.path()).await;
        assert!(
            reason.is_none(),
            "the knob off must restore always-review behaviour, got {reason:?}"
        );
    }

    // ── E2a: the check-cache wrapper around the trivial-skip computation ──

    /// The cached wrapper's first (real) call must populate the shared
    /// `check_cache` under this session's own key — proof the wiring is
    /// real, not a pass-through that happens to return the right answer.
    #[tokio::test]
    async fn trivial_skip_reason_for_dir_cached_populates_the_shared_cache() {
        let _g = env_lock::lock_env([(REVIEWER_MANDATE_SKIP_TRIVIAL_ENV, None::<&str>)]);
        let repo = scratch_git_repo();
        std::fs::write(
            repo.path().join("NOTES.md"),
            "one\ntwo\nthree\nfour\nfive\n",
        )
        .unwrap();
        let session_id = "session-trivial-skip-cache";
        crate::check_cache::clear_goal(session_id);

        let first = trivial_skip_reason_for_dir_cached(session_id, repo.path()).await;
        assert!(first.is_some(), "a docs-only diff must be skippable");

        let hash = crate::check_cache::content_fingerprint(repo.path())
            .await
            .unwrap();
        let cached = crate::check_cache::lookup(
            session_id,
            "after_turn::reviewer_mandate::trivial_skip",
            &hash,
        );
        assert_eq!(
            cached,
            Some(crate::check_cache::CachedCheckResult { passed: true }),
            "the first (real) call must have populated the shared cache under this session's key"
        );

        // A second call over the SAME, unedited content must still resolve
        // "eligible" — this time served from the cache.
        let second = trivial_skip_reason_for_dir_cached(session_id, repo.path()).await;
        assert!(second.is_some());
    }

    /// An edit between two calls must never be masked by a stale cache
    /// entry: the SAME session, now with a substantive product-source diff,
    /// must get a fresh (non-`Skip`) verdict, not the earlier "eligible"
    /// answer.
    #[tokio::test]
    async fn trivial_skip_reason_for_dir_cached_recomputes_after_an_edit() {
        let _g = env_lock::lock_env([(REVIEWER_MANDATE_SKIP_TRIVIAL_ENV, None::<&str>)]);
        let repo = scratch_git_repo();
        std::fs::write(
            repo.path().join("NOTES.md"),
            "one\ntwo\nthree\nfour\nfive\n",
        )
        .unwrap();
        let session_id = "session-trivial-skip-cache-edit";
        crate::check_cache::clear_goal(session_id);

        let first = trivial_skip_reason_for_dir_cached(session_id, repo.path()).await;
        assert!(first.is_some());

        let big: String = (0..20).map(|i| format!("// line {i}\n")).collect();
        std::fs::write(repo.path().join("lib.rs"), format!("fn main() {{}}\n{big}")).unwrap();
        let second = trivial_skip_reason_for_dir_cached(session_id, repo.path()).await;
        assert!(
            second.is_none(),
            "an edit that makes the diff substantive must never be masked by a stale \
             cache entry, got {second:?}"
        );
    }

    /// The identical, unedited content but a DIFFERENT session never reuses
    /// the other session's cache entry (mirrors E2a's "different goal" pin,
    /// substituting session id for goal id per this guard's own scope).
    #[tokio::test]
    async fn trivial_skip_reason_for_dir_cached_never_shares_across_sessions() {
        let _g = env_lock::lock_env([(REVIEWER_MANDATE_SKIP_TRIVIAL_ENV, None::<&str>)]);
        let repo = scratch_git_repo();
        std::fs::write(
            repo.path().join("NOTES.md"),
            "one\ntwo\nthree\nfour\nfive\n",
        )
        .unwrap();
        crate::check_cache::clear_goal("session-trivial-skip-a");
        crate::check_cache::clear_goal("session-trivial-skip-b");

        let a = trivial_skip_reason_for_dir_cached("session-trivial-skip-a", repo.path()).await;
        let b = trivial_skip_reason_for_dir_cached("session-trivial-skip-b", repo.path()).await;
        assert!(a.is_some());
        assert!(b.is_some());

        let hash = crate::check_cache::content_fingerprint(repo.path())
            .await
            .unwrap();
        assert!(crate::check_cache::lookup(
            "session-trivial-skip-a",
            "after_turn::reviewer_mandate::trivial_skip",
            &hash
        )
        .is_some());
        assert!(crate::check_cache::lookup(
            "session-trivial-skip-b",
            "after_turn::reviewer_mandate::trivial_skip",
            &hash
        )
        .is_some());
    }

    // ── G-3: the trivial-diff skip must never erase a pending/negative
    // reviewer verdict (Lane G review, finding 3) ──────────────────────────

    /// The exact scenario Lane G found: a reviewer ask already went out for
    /// this mutation and came back with a real, negative verdict — but the
    /// diff, checked fresh every call, now reads as trivial (e.g. it shrank
    /// via a `shell` edit `is_mutating_tool_name` cannot see — H2's own
    /// documented residual). The trivial-diff skip must never even be
    /// consulted in that case: `trivial_skip_still_eligible` must be
    /// `false`, so `ReviewerMandate::after_turn` falls through to
    /// `ask_state` — which resolves to `Asked` and `Park`s honestly —
    /// instead of returning `Allow` unconditionally the way the old,
    /// unguarded `trivial_skip_reason` call did.
    ///
    /// Before this repair `trivial_skip_still_eligible` did not exist at all
    /// and `ReviewerMandate::after_turn` consulted `trivial_skip_reason`
    /// unconditionally, ahead of any ask-state check — confirmed failing by
    /// temporarily hard-coding this function's body to `true` (always
    /// eligible, the old behaviour) and rerunning: the first assertion below
    /// fails with "must never leave the trivial-diff skip eligible to fire"
    /// against that reverted body, and passes once the real
    /// `!reviewer_ask_pending(..)` body is restored.
    #[test]
    fn trivial_skip_is_not_eligible_once_a_reviewer_ask_is_already_pending() {
        let already_asked_with_negative_verdict = vec![
            Message::assistant().with_text(REVIEW_ASK_OPENING),
            Message::assistant()
                .with_text("Reviewer's verdict: REQUEST_CHANGES — off-by-one in the range check."),
        ];
        assert!(
            !trivial_skip_still_eligible(&already_asked_with_negative_verdict, 0),
            "an already-asked mutation with a recorded verdict must never \
             leave the trivial-diff skip eligible to fire"
        );

        let never_asked: Vec<Message> = vec![];
        assert!(
            trivial_skip_still_eligible(&never_asked, 0),
            "a mutation nobody has asked about yet must remain eligible for \
             the trivial-diff skip"
        );
    }

    // ── E2: honest Park reason when only the shared hold counter fired ──────

    /// Before this repair, the failing assertion was:
    /// `assert!(!reason.contains("did not return a successful APPROVE"))`
    /// — `already_asked` collapsed "a real ask went unanswered" and "the
    /// shared hold counter ran out before this hook ever asked" into the
    /// same text, so a hold budget spent entirely by `PrematureDoneGuard` on
    /// verify-fix cycles produced a false claim that a reviewer had answered
    /// and failed.
    #[tokio::test]
    async fn hold_budget_exhausted_alone_parks_honestly_not_as_a_failed_review() {
        let verified = vec![
            tool_exchange("developer__edit", "e1", true),
            tool_exchange("developer__verify", "v1", true),
        ];
        match ReviewerMandate::new()
            .after_turn(&ctx(&verified, crate::cost_router::MAX_HOLDS))
            .await
        {
            AfterTurnAction::Park { reason } => {
                assert!(
                    !reason.contains("did not return a successful APPROVE"),
                    "reason was {reason:?}"
                );
                assert!(reason.contains("hold budget"), "reason was {reason:?}");
            }
            other => panic!("expected Park, got {other:?}"),
        }
    }

    // ── E3: the injected text must not promise a model the dispatch won't use ──

    /// Before this repair, `reviewer_inject_text` took only the `ReviewerPick`
    /// from the INDEPENDENT `select_reviewer` and always said "a different
    /// model from yours" — even when that pick's own provider/model are
    /// identical to the author's, exactly the shape a `PERMAGENT_PACK_HARD_*`
    /// pin to the author's own model produces at the real dispatch site
    /// (3/3 in Lane A's ledger). The failing assertion against the OLD
    /// single-argument signature was:
    /// `assert!(!reviewer_inject_text(&same_family_pick).contains("a different model from yours"))`
    /// — which failed because the old function had no way to know the pick
    /// didn't match what would actually be dispatched.
    #[test]
    fn injected_text_admits_same_family_when_the_dispatch_will_share_it() {
        let pick = ready_pick();
        let same_family_route = ActualReviewerRoute {
            label: "zai/glm-4.7".to_string(),
            same_family: true,
        };
        let text = reviewer_inject_text(&pick, &same_family_route);
        assert!(
            !text.contains("a different model from yours"),
            "text was {text:?}"
        );
        assert!(
            text.to_ascii_lowercase().contains("same family"),
            "text was {text:?}"
        );
    }

    /// The honest case still reads as before: a genuinely different-family
    /// route says so.
    #[test]
    fn injected_text_still_says_different_model_for_a_real_cross_family_route() {
        let pick = ready_pick();
        let route = cross_family_route(&pick);
        let text = reviewer_inject_text(&pick, &route);
        assert!(
            text.contains("a different model from yours"),
            "text was {text:?}"
        );
    }

    /// Pins the actual resolution path
    /// (`delegate_routing` + `resolve_actual_review_dispatch` — the SAME
    /// function `summon::resolve_provider`'s real dispatch calls, X-cost-2)
    /// against a live `PERMAGENT_PACK_HARD_*` pin equal to the author's own
    /// model, with NO cross-family candidate offered (an empty derived map) —
    /// the exact shape observed 3/3 in Lane A's ledger — and confirms
    /// [`actual_reviewer_route`] reports it honestly: with nothing else
    /// available, the HARD pin is still the honest same-family answer.
    #[test]
    fn actual_route_is_same_family_when_the_hard_pack_pins_the_authors_own_model() {
        let author_provider = "zai";
        let author_model = "glm-4.7";
        let _g = env_lock::lock_env([
            (
                crate::cost_router::packs::KEY_HARD_PROVIDER,
                Some(author_provider),
            ),
            (
                crate::cost_router::packs::KEY_HARD_MODEL,
                Some(author_model),
            ),
        ]);
        let derived = crate::cost_router::DerivedRoleMap::empty();
        let routing = crate::cost_router::delegate_routing(
            Some(crate::cost_router::WorkflowRole::Review),
            &derived,
            None,
            Some((author_provider, author_model)),
        );
        let routing_is_role_pin = routing.source == crate::cost_router::DelegateSource::RolePin;
        let dispatch = crate::cost_router::resolve_actual_review_dispatch(
            routing_is_role_pin,
            routing.role_model.clone(),
            Some((author_provider, author_model)),
            None, // no dedicated PERMAGENT_PACK_REVIEW_* pin
            None, // no cross-family candidate available (empty derived map)
            &|_: &str| true,
            Some(crate::cost_router::BudgetBand::Ok),
        );
        let route = actual_reviewer_route(&dispatch, Some((author_provider, author_model)));
        assert!(
            route.same_family,
            "no cross-family candidate is available, so a HARD pack pinned to \
             the author's own model must still resolve same-family"
        );
        assert_eq!(route.label, format!("{author_provider}/{author_model}"));

        let text = reviewer_inject_text(&ready_pick(), &route);
        assert!(
            !text.contains("a different model from yours"),
            "text was {text:?}"
        );
    }

    /// X-cost-2's actual fix, exercised from `after_turn`'s own prediction
    /// path: the SAME HARD-pinned-to-the-author's-family shape as the test
    /// above, but NOW with a genuinely cross-family candidate available,
    /// its provider configured, and spend `Ok` — the injected text must
    /// switch to honestly promising independence, because the real dispatch
    /// (`resolve_actual_review_dispatch`) now actually escalates instead of
    /// silently inheriting the HARD pin.
    #[test]
    fn actual_route_escalates_cross_family_when_a_candidate_is_available() {
        let author_provider = "zai";
        let author_model = "glm-4.7";
        let _g = env_lock::lock_env([
            (
                crate::cost_router::packs::KEY_HARD_PROVIDER,
                Some(author_provider),
            ),
            (
                crate::cost_router::packs::KEY_HARD_MODEL,
                Some(author_model),
            ),
        ]);
        let derived = crate::cost_router::DerivedRoleMap::empty();
        let routing = crate::cost_router::delegate_routing(
            Some(crate::cost_router::WorkflowRole::Review),
            &derived,
            None,
            Some((author_provider, author_model)),
        );
        let routing_is_role_pin = routing.source == crate::cost_router::DelegateSource::RolePin;
        let cross_family_candidate = crate::cost_router::RoleModel {
            provider: "openai".to_string(),
            model: "gpt-5.4-mini".to_string(),
        };
        let dispatch = crate::cost_router::resolve_actual_review_dispatch(
            routing_is_role_pin,
            routing.role_model.clone(),
            Some((author_provider, author_model)),
            None,
            Some(cross_family_candidate.clone()),
            &|_: &str| true,
            Some(crate::cost_router::BudgetBand::Ok),
        );
        let route = actual_reviewer_route(&dispatch, Some((author_provider, author_model)));
        assert!(
            !route.same_family,
            "a configured, spend-Ok cross-family candidate must win over the \
             HARD pin"
        );
        assert_eq!(
            route.label,
            format!(
                "{}/{}",
                cross_family_candidate.provider, cross_family_candidate.model
            )
        );

        let text = reviewer_inject_text(&ready_pick(), &route);
        assert!(
            text.contains("a different model from yours"),
            "text was {text:?}"
        );
    }

    // ── RoadmapDispatchGuard (X-dag-2): no "dispatched" claim without a child ──

    /// An isolated in-memory DB, independent of the process-global
    /// `SessionManager::instance()` singleton every other test in this file
    /// shares — `RoadmapDispatchGuard::undispatched_goal_for` takes the pool
    /// as a parameter specifically so this is possible.
    async fn roadmap_guard_test_pool() -> sqlx::Pool<sqlx::Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::session::spectral_schema::init_spectral_db(&pool)
            .await
            .unwrap();
        crate::cards::seed_goal_columns(&pool, crate::projects::PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        pool
    }

    /// Inserts one goal card attributed to `session_id` in the given
    /// lifecycle state, optionally with a worker session already recorded.
    async fn insert_session_goal_card(
        pool: &sqlx::Pool<sqlx::Sqlite>,
        project_id: &str,
        session_id: &str,
        state_binding: &str,
        is_root: bool,
        worker_session_id: Option<&str>,
    ) -> String {
        let column_id: String = sqlx::query_scalar(
            "SELECT id FROM board_columns WHERE project_id = ? AND state_binding = ?",
        )
        .bind(project_id)
        .bind(state_binding)
        .fetch_one(pool)
        .await
        .unwrap();
        let card_id = uuid::Uuid::now_v7().to_string();
        let mut meta = serde_json::json!({
            "depends_on": if is_root { serde_json::json!([]) } else { serde_json::json!(["some-other-card"]) },
            "created_by_session_id": session_id,
        });
        if let Some(worker) = worker_session_id {
            meta.as_object_mut()
                .unwrap()
                .insert("worker_session_id".to_string(), serde_json::json!(worker));
        }
        sqlx::query(
            "INSERT INTO cards (id, project_id, card_type, title, description, column_id, \
                                 position, created_by, metadata_json) \
             VALUES (?, ?, 'goal', 'goal', '', ?, 0, 'user', ?)",
        )
        .bind(&card_id)
        .bind(project_id)
        .bind(&column_id)
        .bind(meta.to_string())
        .execute(pool)
        .await
        .unwrap();
        card_id
    }

    // ── the pool-parameterised query, in isolation ───────────────────────────

    /// Root, Ready, no worker session → flagged. M3-2's exact shape at the
    /// query level: `create_roadmap` succeeded, dispatch never landed.
    #[tokio::test]
    async fn undispatched_goal_for_finds_an_undispatched_root_goal() {
        let pool = roadmap_guard_test_pool().await;
        let session_id = format!("query-m3-2-{}", uuid::Uuid::now_v7());
        let card_id = insert_session_goal_card(
            &pool,
            crate::projects::PERSONAL_PROJECT_ID,
            &session_id,
            "ready",
            true,
            None,
        )
        .await;

        let goal = RoadmapDispatchGuard::undispatched_goal_for(&pool, &session_id)
            .await
            .unwrap();
        assert_eq!(goal.map(|g| g.card_id), Some(card_id));
    }

    /// InProgress with no worker session recorded — a claim with nothing
    /// behind it, the guard's second condition — is flagged even when the
    /// goal is not a root.
    #[tokio::test]
    async fn undispatched_goal_for_finds_an_orphaned_in_progress_goal_even_if_not_root() {
        let pool = roadmap_guard_test_pool().await;
        let session_id = format!("query-orphaned-{}", uuid::Uuid::now_v7());
        let card_id = insert_session_goal_card(
            &pool,
            crate::projects::PERSONAL_PROJECT_ID,
            &session_id,
            "in_progress",
            false,
            None,
        )
        .await;

        let goal = RoadmapDispatchGuard::undispatched_goal_for(&pool, &session_id)
            .await
            .unwrap();
        assert_eq!(goal.map(|g| g.card_id), Some(card_id));
    }

    /// A root goal that actually dispatched (a worker session recorded) is
    /// never flagged.
    #[tokio::test]
    async fn undispatched_goal_for_ignores_a_dispatched_root_goal() {
        let pool = roadmap_guard_test_pool().await;
        let session_id = format!("query-dispatched-{}", uuid::Uuid::now_v7());
        insert_session_goal_card(
            &pool,
            crate::projects::PERSONAL_PROJECT_ID,
            &session_id,
            "in_progress",
            true,
            Some("worker-sess-1"),
        )
        .await;

        let goal = RoadmapDispatchGuard::undispatched_goal_for(&pool, &session_id)
            .await
            .unwrap();
        assert!(
            goal.is_none(),
            "a root goal with a recorded worker session must not be flagged"
        );
    }

    /// A session with no cards at all (never called `create_roadmap`) has
    /// nothing to flag.
    #[tokio::test]
    async fn undispatched_goal_for_ignores_a_session_with_no_cards() {
        let pool = roadmap_guard_test_pool().await;
        let session_id = format!("query-no-roadmap-{}", uuid::Uuid::now_v7());

        let goal = RoadmapDispatchGuard::undispatched_goal_for(&pool, &session_id)
            .await
            .unwrap();
        assert!(goal.is_none());
    }

    // ── the trait impl end to end, via `SessionManager::instance()` ─────────
    //
    // `RoadmapDispatchGuard::after_turn` reads the SAME process-global
    // singleton `ReviewerMandate`'s live tests above already share (there is
    // no injection seam at the trait boundary, matching that precedent) — so,
    // like those, these fixtures go through it directly rather than the
    // isolated pool above. Each test also gets its OWN fresh project (never
    // `PERSONAL_PROJECT_ID`): dozens of other tests across this crate
    // exercise that exact project's `board_columns` concurrently under
    // `cargo test`'s default parallelism, and `seed_goal_columns`'s
    // check-then-insert is not race-safe against concurrent callers on the
    // SAME project — confirmed live (`RowNotFound` on the "ready" column
    // lookup, intermittent). A session id alone is not enough isolation,
    // since the race is over the COLUMNS, not the cards.

    async fn singleton_pool_with_fresh_project() -> (sqlx::Pool<sqlx::Sqlite>, String) {
        let pool = crate::session::SessionManager::instance()
            .pool_clone()
            .await
            .unwrap();
        let project = crate::projects::create_project(
            &pool,
            crate::projects::CreateProject {
                name: format!("x-dag-2-test-{}", uuid::Uuid::now_v7()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        crate::cards::seed_goal_columns(&pool, &project.id)
            .await
            .unwrap();
        (pool, project.id)
    }

    /// M3-2's exact shape end to end: `create_roadmap` succeeded (a root
    /// goal card exists, attributed to this session), no dispatch call ever
    /// landed (still Ready, no worker session recorded), and the model's
    /// own turn claims otherwise. Must hold the turn with the brief's own
    /// message, not let the "dispatched" claim stand.
    #[tokio::test]
    async fn m3_2_shape_an_undispatched_root_goal_holds_the_turn() {
        let (pool, project_id) = singleton_pool_with_fresh_project().await;
        let session_id = format!("m3-2-{}", uuid::Uuid::now_v7());
        let card_id =
            insert_session_goal_card(&pool, &project_id, &session_id, "ready", true, None).await;

        // The false-completion transcript itself (M3-2's own quoted text) —
        // irrelevant to the guard's decision, which is DB-driven, not
        // transcript-driven; included so this test pins the exact scenario
        // the defect report described, not just the underlying query.
        let claim = vec![Message::assistant().with_text(
            "The root goals are dispatched and will run in isolated worktrees. I will \
             monitor progress and report results once completed and verified. No further \
             action is needed on this loop until workers report back.",
        )];
        let guard = RoadmapDispatchGuard::new();
        match guard
            .after_turn(&AfterTurnContext {
                delegate_tool_available: true,
                session_id: &session_id,
                messages: &claim,
                prior_holds: 0,
            })
            .await
        {
            AfterTurnAction::Continue { inject } => {
                assert_eq!(
                    inject,
                    format!(
                        "roadmap {session_id}: goal {card_id} is not dispatched; call \
                         goal_advance dispatch or explain"
                    )
                );
            }
            other => panic!("expected Continue, got {other:?}"),
        }
    }

    /// Bounded like the other two hooks: once the shared hold budget is
    /// exhausted, this Parks with the same honest, specific reason instead
    /// of holding forever.
    #[tokio::test]
    async fn an_undispatched_root_goal_parks_once_the_hold_budget_is_exhausted() {
        let (pool, project_id) = singleton_pool_with_fresh_project().await;
        let session_id = format!("m3-2-exhausted-{}", uuid::Uuid::now_v7());
        let card_id =
            insert_session_goal_card(&pool, &project_id, &session_id, "ready", true, None).await;

        let guard = RoadmapDispatchGuard::new();
        let empty: Vec<Message> = vec![];
        match guard
            .after_turn(&AfterTurnContext {
                delegate_tool_available: true,
                session_id: &session_id,
                messages: &empty,
                prior_holds: crate::cost_router::MAX_HOLDS,
            })
            .await
        {
            AfterTurnAction::Park { reason } => {
                assert_eq!(
                    reason,
                    format!(
                        "roadmap {session_id}: goal {card_id} is not dispatched; call \
                         goal_advance dispatch or explain"
                    )
                );
            }
            other => panic!("expected Park, got {other:?}"),
        }
    }

    /// All roots dispatched (a worker session recorded) → Allow.
    #[tokio::test]
    async fn all_roots_dispatched_allows_the_turn_to_end() {
        let (pool, project_id) = singleton_pool_with_fresh_project().await;
        let session_id = format!("dispatched-{}", uuid::Uuid::now_v7());
        insert_session_goal_card(
            &pool,
            &project_id,
            &session_id,
            "in_progress",
            true,
            Some("worker-sess-1"),
        )
        .await;

        let guard = RoadmapDispatchGuard::new();
        let empty: Vec<Message> = vec![];
        assert_eq!(
            guard
                .after_turn(&AfterTurnContext {
                    delegate_tool_available: true,
                    session_id: &session_id,
                    messages: &empty,
                    prior_holds: 0,
                })
                .await,
            AfterTurnAction::Allow
        );
    }

    /// No roadmap at all (this session never called `create_roadmap`, so it
    /// has no cards) → Allow. The applicability gate: ordinary conversation
    /// must never be held by a guard whose entire premise is "this session
    /// has a roadmap".
    #[tokio::test]
    async fn no_roadmap_allows_the_turn_to_end() {
        let _fresh_project = singleton_pool_with_fresh_project().await;
        let session_id = format!("no-roadmap-{}", uuid::Uuid::now_v7());

        let guard = RoadmapDispatchGuard::new();
        let empty: Vec<Message> = vec![];
        assert_eq!(
            guard
                .after_turn(&AfterTurnContext {
                    delegate_tool_available: true,
                    session_id: &session_id,
                    messages: &empty,
                    prior_holds: 0,
                })
                .await,
            AfterTurnAction::Allow
        );
    }

    // ── RoadmapDispatchGuard's E3 half: decompose approved, never created ──

    /// Insert an ANSWERED, approved roadmap-creation decision minted by
    /// `session_id` — the durable trace `decompose_roadmap` leaves once its
    /// proposal is approved (headlessly or by a human), independent of
    /// whether `create_roadmap` was ever subsequently called.
    async fn insert_answered_roadmap_decision(
        pool: &sqlx::Pool<sqlx::Sqlite>,
        project_id: &str,
        session_id: &str,
    ) {
        let decision = crate::decisions::create_decision(
            pool,
            crate::decisions::NewDecision {
                kind: "choice".to_string(),
                project_id: Some(project_id.to_string()),
                headline: Some("Approve this roadmap?".to_string()),
                detail: Some("Approve exact roadmap".to_string()),
                payload: serde_json::json!({
                    "question": "Create this exact roadmap?",
                    "options": [
                        {"id": "approve-roadmap", "label": "Approve and begin"},
                        {"id": "reject-roadmap", "label": "Reject"}
                    ],
                    "proposal": crate::decisions::PROPOSAL_ROADMAP_CREATE,
                    "roadmap_approval": {
                        "proposal_hash": "deadbeef",
                        "minting_session_id": session_id,
                    }
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        crate::decisions::answer_decision(
            pool,
            &decision.id,
            &crate::decisions::DecisionAnswer {
                answer: "choice".to_string(),
                choice_id: Some("approve-roadmap".to_string()),
                note: None,
                input_text: None,
            },
            crate::decisions::ACTOR_JESSE,
        )
        .await
        .unwrap();
    }

    /// E3 (harness loop, Round E, from Lane F9 forensics 2026-09-10) pin,
    /// fx10's exact shape: `decompose_roadmap`'s proposal was approved but
    /// `create_roadmap` was never called — 0 goal cards. Must hold the turn
    /// with a nudge naming the exact missing step, not Allow. Pre-fix: the
    /// guard has no `undecomposed_roadmap_for` check at all, and
    /// `undispatched_goal_for` is a no-op here (no cards exist to check),
    /// so this call falls through to `AfterTurnAction::Allow` — this test's
    /// `match` panics on the `other => panic!` arm.
    #[tokio::test]
    async fn fx10_shape_decompose_without_create_holds_the_turn() {
        let (pool, project_id) = singleton_pool_with_fresh_project().await;
        let session_id = format!("fx10-{}", uuid::Uuid::now_v7());
        insert_answered_roadmap_decision(&pool, &project_id, &session_id).await;

        let guard = RoadmapDispatchGuard::new();
        let empty: Vec<Message> = vec![];
        match guard
            .after_turn(&AfterTurnContext {
                delegate_tool_available: true,
                session_id: &session_id,
                messages: &empty,
                prior_holds: 0,
            })
            .await
        {
            AfterTurnAction::Continue { inject } => {
                assert!(inject.contains("create_roadmap"), "{inject}");
                assert!(inject.contains(&session_id), "{inject}");
            }
            other => panic!("expected Continue, got {other:?}"),
        }
    }

    /// E3: bounded like every other hook here — once the shared hold budget
    /// is exhausted, Parks with the same reason instead of holding forever
    /// (never an infinite loop).
    #[tokio::test]
    async fn fx10_shape_parks_once_the_hold_budget_is_exhausted() {
        let (pool, project_id) = singleton_pool_with_fresh_project().await;
        let session_id = format!("fx10-exhausted-{}", uuid::Uuid::now_v7());
        insert_answered_roadmap_decision(&pool, &project_id, &session_id).await;

        let guard = RoadmapDispatchGuard::new();
        let empty: Vec<Message> = vec![];
        match guard
            .after_turn(&AfterTurnContext {
                delegate_tool_available: true,
                session_id: &session_id,
                messages: &empty,
                prior_holds: crate::cost_router::MAX_HOLDS,
            })
            .await
        {
            AfterTurnAction::Park { reason } => {
                assert!(reason.contains("create_roadmap"), "{reason}");
            }
            other => panic!("expected Park, got {other:?}"),
        }
    }

    /// E3: after `create_roadmap` succeeds (a goal card now exists,
    /// dispatched so the guard's FIRST check does not also flag it), the
    /// guard releases — the exact "after create → released" pin.
    #[tokio::test]
    async fn after_create_roadmap_the_guard_releases() {
        let (pool, project_id) = singleton_pool_with_fresh_project().await;
        let session_id = format!("fx10-created-{}", uuid::Uuid::now_v7());
        insert_answered_roadmap_decision(&pool, &project_id, &session_id).await;
        insert_session_goal_card(
            &pool,
            &project_id,
            &session_id,
            "in_progress",
            true,
            Some("worker-sess-1"),
        )
        .await;

        let guard = RoadmapDispatchGuard::new();
        let empty: Vec<Message> = vec![];
        assert_eq!(
            guard
                .after_turn(&AfterTurnContext {
                    delegate_tool_available: true,
                    session_id: &session_id,
                    messages: &empty,
                    prior_holds: 0,
                })
                .await,
            AfterTurnAction::Allow
        );
    }

    /// E3: an explicit `escalate` anywhere in the transcript is an honest
    /// way to end the turn without creating the roadmap — never held.
    #[tokio::test]
    async fn an_explicit_escalate_releases_the_fx10_shape_guard() {
        let (pool, project_id) = singleton_pool_with_fresh_project().await;
        let session_id = format!("fx10-escalated-{}", uuid::Uuid::now_v7());
        insert_answered_roadmap_decision(&pool, &project_id, &session_id).await;

        let guard = RoadmapDispatchGuard::new();
        let escalated = vec![tool_exchange("escalate", "call-1", true)];
        assert_eq!(
            guard
                .after_turn(&AfterTurnContext {
                    delegate_tool_available: true,
                    session_id: &session_id,
                    messages: &escalated,
                    prior_holds: 0,
                })
                .await,
            AfterTurnAction::Allow
        );
    }

    // ── NoRoadmapGuard (Lane Y5): a coordinator turn cannot end with no roadmap ──
    //
    // The live path reads the process-global session store to answer "is this
    // a coding-harness coordinator" and the durable card table to answer "does
    // a roadmap exist". `NoRoadmapGuard::decide` takes both answers as
    // parameters precisely so each branch below is a pure, isolated test —
    // the same seam `RoadmapDispatchGuard::undispatched_goal_for` and
    // `ReviewerMandate::decide` already established in this file.

    /// A coordinator that made only read-only calls and then wrote its plan in
    /// prose — fx02's exact shape ("Say the word and I'll create and dispatch
    /// this immediately") — is sent back to call the tool.
    #[test]
    fn no_roadmap_read_only_turn_is_sent_back_to_decompose() {
        let prose = vec![
            tool_exchange("analyze__map_query", "call-1", true),
            tool_exchange("developer__search", "call-2", true),
            Message::assistant().with_text(
                "Proposed roadmap: one node, no dependencies — a single routed worker owns it \
                 end to end. Say the word and I'll create and dispatch this immediately.",
            ),
        ];
        assert!(
            !conversation_mutated_files(&prose),
            "read-only calls must not read as a mutation: PrematureDoneGuard is silent here"
        );
        let called_roadmap_tool = turn_called_any_tool(&prose, &ROADMAP_TOOL_NAMES);
        assert!(
            !called_roadmap_tool,
            "a prose roadmap is not a decompose_roadmap call"
        );

        match NoRoadmapGuard::decide(
            true,
            false,
            called_roadmap_tool,
            turn_called_escalate(&prose),
            NoRoadmapGuard::already_nudged(&prose),
            0,
        ) {
            AfterTurnAction::Continue { inject } => {
                assert!(
                    inject.starts_with(NO_ROADMAP_INJECT_OPENING),
                    "the nudge must be recognisable to `already_nudged`: {inject}"
                );
                assert!(inject.contains("decompose_roadmap"), "{inject}");
                assert!(inject.contains("escalate"), "{inject}");
            }
            other => panic!("expected Continue, got {other:?}"),
        }
    }

    /// R20c-b (A21 Q2): the nudge text itself must not override the recipe's
    /// own carve-outs. Before this fix the nudge only offered `escalate` for
    /// a task that is "genuinely impossible" — narrower than the recipe's
    /// (b)/(c) carve-outs (ambiguity, a missing credential/provider, any
    /// question to the user) — and a DeepSeek round followed the nudge's
    /// narrower framing into `decompose_roadmap` on a fixture that should
    /// have escalated. The nudge must now name all three cases AND put
    /// `escalate` before `decompose_roadmap` in reading order, matching the
    /// recipe's own carve-outs-before-rule-1 structure.
    #[test]
    fn no_roadmap_inject_text_offers_escalate_for_ambiguity_and_missing_credential_too() {
        for phrase in [
            "ambiguous",
            "credential",
            "provider",
            "escalate",
            "decompose_roadmap",
        ] {
            assert!(
                NO_ROADMAP_INJECT_TEXT.to_ascii_lowercase().contains(phrase),
                "nudge text is missing {phrase:?}: {NO_ROADMAP_INJECT_TEXT}"
            );
        }
        let escalate_pos = NO_ROADMAP_INJECT_TEXT.find("escalate").unwrap();
        let decompose_pos = NO_ROADMAP_INJECT_TEXT.find("decompose_roadmap").unwrap();
        assert!(
            escalate_pos < decompose_pos,
            "escalate must be offered BEFORE decompose_roadmap in reading order, matching the \
             recipe's carve-outs-before-rule-1 structure: {NO_ROADMAP_INJECT_TEXT}"
        );
    }

    /// fx06's shape: the coordinator's one `shell` call was DENIED by
    /// `CodingDagInspector` and it asked the user a question instead of
    /// decomposing. A denied call is not a mutation, so `PrematureDoneGuard`
    /// is silent — this guard must not be.
    #[test]
    fn no_roadmap_fires_after_a_denied_shell_and_a_question() {
        let denied = vec![
            tool_exchange_with_text(
                "developer__shell",
                "call-1",
                "The user has declined to run this tool. DO NOT attempt to call this tool again.",
                false,
            ),
            Message::assistant()
                .with_text("Would you like me to fix the Makefile, or should I skip that target?"),
        ];
        assert!(
            !conversation_mutated_files(&denied),
            "a denied shell call must not read as a mutation"
        );
        assert!(matches!(
            NoRoadmapGuard::decide(
                true,
                false,
                turn_called_any_tool(&denied, &ROADMAP_TOOL_NAMES),
                turn_called_escalate(&denied),
                NoRoadmapGuard::already_nudged(&denied),
                0,
            ),
            AfterTurnAction::Continue { .. }
        ));
    }

    /// The tool WAS called — nothing to nudge, whichever of the two names was
    /// used and whichever extension prefixed it.
    #[test]
    fn no_roadmap_does_not_fire_when_decompose_roadmap_was_called() {
        for name in [
            "decompose_roadmap",
            "orchestrator__decompose_roadmap",
            "orchestrator__create_roadmap",
        ] {
            let called = vec![tool_exchange(name, "call-1", true)];
            assert!(
                turn_called_any_tool(&called, &ROADMAP_TOOL_NAMES),
                "{name} must be recognised as a roadmap call"
            );
            assert_eq!(
                NoRoadmapGuard::decide(
                    true,
                    false,
                    turn_called_any_tool(&called, &ROADMAP_TOOL_NAMES),
                    false,
                    false,
                    0,
                ),
                AfterTurnAction::Allow,
                "{name}"
            );
        }
    }

    /// Once per session. The nudge is a durable user message, so the second
    /// pass recognises its own handwriting and stands down — this hook cannot
    /// use the shared `prior_holds` counter for that (see
    /// `NO_ROADMAP_INJECT_OPENING`).
    #[test]
    fn no_roadmap_does_not_fire_a_second_time() {
        let AfterTurnAction::Continue { inject } =
            NoRoadmapGuard::decide(true, false, false, false, false, 0)
        else {
            panic!("the first pass must hold");
        };

        let after = vec![
            Message::assistant().with_text("Here is the plan I propose."),
            Message::user().with_text(inject.as_str()),
            Message::assistant().with_text("Understood — but I would still like your approval."),
        ];
        assert!(NoRoadmapGuard::already_nudged(&after));
        assert_eq!(
            NoRoadmapGuard::decide(
                true,
                false,
                turn_called_any_tool(&after, &ROADMAP_TOOL_NAMES),
                turn_called_escalate(&after),
                NoRoadmapGuard::already_nudged(&after),
                0,
            ),
            AfterTurnAction::Allow,
            "the guard must nudge at most once per session"
        );
    }

    /// A dispatched worker has a parent session and is the thing that MAY
    /// edit; telling it to decompose would be exactly backwards. Same
    /// predicate `CodingDagInspector` gates on, asserted here so the two
    /// halves of the protocol can never disagree about who they apply to.
    #[test]
    fn no_roadmap_does_not_fire_for_a_dispatched_worker() {
        let is_coordinator = crate::tool_inspection::is_top_level_coding_harness(
            Some(crate::tool_inspection::CODING_HARNESS_RECIPE_TITLE),
            Some("parent-session-1"),
        );
        assert!(
            !is_coordinator,
            "a session with a parent is not the coordinator"
        );
        assert_eq!(
            NoRoadmapGuard::decide(is_coordinator, false, false, false, false, 0),
            AfterTurnAction::Allow
        );
    }

    /// Any other recipe — and a session with no recipe at all — is ordinary
    /// conversation this guard must never touch.
    #[test]
    fn no_roadmap_does_not_fire_outside_the_coding_harness() {
        for title in [Some("Some Other Recipe"), None] {
            let is_coordinator = crate::tool_inspection::is_top_level_coding_harness(title, None);
            assert_eq!(
                NoRoadmapGuard::decide(is_coordinator, false, false, false, false, 0),
                AfterTurnAction::Allow,
                "{title:?}"
            );
        }
    }

    /// An explicit `escalate` is the honest exit the nudge itself names: a
    /// coordinator that put the decision in front of a human is not the
    /// fx02/fx06 shape and must never be held.
    #[test]
    fn no_roadmap_releases_a_turn_that_escalated() {
        let escalated = vec![tool_exchange("escalate", "call-1", true)];
        assert_eq!(
            NoRoadmapGuard::decide(
                true,
                false,
                turn_called_any_tool(&escalated, &ROADMAP_TOOL_NAMES),
                turn_called_escalate(&escalated),
                false,
                0,
            ),
            AfterTurnAction::Allow
        );
    }

    /// Bounded like every other hook: once the shared hold budget is spent the
    /// turn ends, but with an honest reason instead of the silent ten-second
    /// exit fx02/fx06 produced.
    #[test]
    fn no_roadmap_parks_once_the_hold_budget_is_exhausted() {
        match NoRoadmapGuard::decide(
            true,
            false,
            false,
            false,
            false,
            crate::cost_router::MAX_HOLDS,
        ) {
            AfterTurnAction::Park { reason } => {
                assert_eq!(reason, NO_ROADMAP_PARK_REASON);
            }
            other => panic!("expected Park, got {other:?}"),
        }
    }

    /// A session that already minted goal cards HAS a roadmap — the durable
    /// half of the applicability gate, against the isolated in-memory pool.
    /// `working_dir` is an arbitrary nonexistent path here: the isolated
    /// pool seeds no `projects` row at all, so the R20c-c project-scoped
    /// half of `has_roadmap_cards` always reads empty regardless of what
    /// path is passed — this test is exercising the session-scoped half
    /// only, unchanged from before R20c-c.
    #[tokio::test]
    async fn no_roadmap_has_roadmap_cards_detects_this_sessions_cards() {
        let pool = roadmap_guard_test_pool().await;
        let session_id = format!("y5-cards-{}", uuid::Uuid::now_v7());
        let working_dir = std::path::Path::new("/nonexistent/r20c-c-test-dir");
        assert!(
            !NoRoadmapGuard::has_roadmap_cards(&pool, &session_id, working_dir)
                .await
                .unwrap(),
            "a session that never called create_roadmap owns no cards"
        );

        insert_session_goal_card(
            &pool,
            crate::projects::PERSONAL_PROJECT_ID,
            &session_id,
            "ready",
            true,
            None,
        )
        .await;
        assert!(
            NoRoadmapGuard::has_roadmap_cards(&pool, &session_id, working_dir)
                .await
                .unwrap()
        );
        assert_eq!(
            NoRoadmapGuard::decide(true, true, false, false, false, 0),
            AfterTurnAction::Allow,
            "a session with a roadmap is never nudged to create one"
        );
    }

    /// The trait impl end to end for the case that matters most for blast
    /// radius: an unknown/non-harness session id (the session store cannot
    /// confirm a coding-harness coordinator) is let through untouched.
    #[tokio::test]
    async fn no_roadmap_guard_allows_a_session_it_cannot_confirm() {
        let session_id = format!("y5-unknown-{}", uuid::Uuid::now_v7());
        let guard = NoRoadmapGuard::new();
        let prose = vec![Message::assistant().with_text("Here is a plan. Say the word.")];
        assert_eq!(
            guard
                .after_turn(&AfterTurnContext {
                    delegate_tool_available: true,
                    session_id: &session_id,
                    messages: &prose,
                    prior_holds: 0,
                })
                .await,
            AfterTurnAction::Allow
        );
    }

    // ── goal_cards_relevant_to_session (Lane R20c-c) ────────────────────────
    //
    // Against the process-global `SessionManager::instance()` pool — same
    // pool `crate::projects::create_project` and the live `after_turn` path
    // both use — because `goal_cards_for_project_matching_cwd`'s cwd match
    // needs a REAL, on-disk `root_path` (`crate::projects::resolve_root_path`
    // rejects a directory that does not exist), which the isolated
    // `roadmap_guard_test_pool()` above has no reason to provide.

    /// The union/dedup itself: a card THIS session created (found via the
    /// unchanged `goal_cards_created_by_session` scope) and a SEPARATE card
    /// in the same project created by a DIFFERENT session entirely (the
    /// resumed-roadmap shape) both show up, exactly once each — proving
    /// neither scope shadows the other and a card visible in both is not
    /// double-counted.
    #[tokio::test]
    async fn goal_cards_relevant_to_session_unions_session_and_project_scoped_cards() {
        let pool = crate::session::SessionManager::instance()
            .pool_clone()
            .await
            .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let project = crate::projects::create_project(
            &pool,
            crate::projects::CreateProject {
                name: format!("r20c-c-union-{}", uuid::Uuid::now_v7()),
                root_path: Some(tmp.path().to_string_lossy().into_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        crate::cards::seed_goal_columns(&pool, &project.id)
            .await
            .unwrap();

        let session_id = format!("r20c-c-union-{}", uuid::Uuid::now_v7());
        let mine =
            insert_session_goal_card(&pool, &project.id, &session_id, "ready", true, None).await;
        let theirs = insert_session_goal_card(
            &pool,
            &project.id,
            "some-earlier-session",
            "in_progress",
            false,
            None,
        )
        .await;

        let cards = goal_cards_relevant_to_session(&pool, &session_id, tmp.path())
            .await
            .unwrap();
        let mut ids: Vec<&str> = cards.iter().map(|c| c.card_id.as_str()).collect();
        ids.sort_unstable();
        let mut expected = vec![mine.as_str(), theirs.as_str()];
        expected.sort_unstable();
        assert_eq!(
            ids, expected,
            "must union BOTH scopes and not double-count the session-created card"
        );
    }

    /// R20c-c's headline requirement, composed from real DB state end to
    /// end: the M24 `select-existing-roadmap-resumes-not-redecomposes`
    /// shape — one goal already Complete, one InProgress, BOTH seeded by a
    /// session that is not the coordinator now closing the turn — must
    /// read as "has a roadmap" for both guards, so an ordinary closing
    /// summary does not park. Composes the real DB-backed halves
    /// (`goal_cards_relevant_to_session`, `session_has_an_active_roadmap`)
    /// with the pure `decide` functions — the same seam every other
    /// live-shaped scenario in this file's tests uses — rather than
    /// standing up a full `SessionManager` session row with an explicit
    /// recipe/`GooseMode`; matches this file's established limit (no
    /// guard's live `after_turn` is exercised against a real coding-harness
    /// session row anywhere in this test module, R18f's/R20c's own notes).
    #[tokio::test]
    async fn resumed_roadmap_with_cards_created_by_another_session_reads_as_having_a_roadmap() {
        let pool = crate::session::SessionManager::instance()
            .pool_clone()
            .await
            .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let project = crate::projects::create_project(
            &pool,
            crate::projects::CreateProject {
                name: format!("r20c-c-resumed-{}", uuid::Uuid::now_v7()),
                root_path: Some(tmp.path().to_string_lossy().into_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        crate::cards::seed_goal_columns(&pool, &project.id)
            .await
            .unwrap();

        insert_session_goal_card(
            &pool,
            &project.id,
            "eval-seed-session",
            "complete",
            true,
            None,
        )
        .await;
        insert_session_goal_card(
            &pool,
            &project.id,
            "eval-seed-session",
            "in_progress",
            false,
            Some("worker-session-2"),
        )
        .await;

        let coordinator_session_id = format!("r20c-c-coordinator-{}", uuid::Uuid::now_v7());
        let cards = goal_cards_relevant_to_session(&pool, &coordinator_session_id, tmp.path())
            .await
            .unwrap();
        assert_eq!(
            cards.len(),
            2,
            "both cards must be visible via the project-scoped union, though this \
             session created neither"
        );

        let has_roadmap_cards = !cards.is_empty();
        let has_active_roadmap = crate::cards::session_has_an_active_roadmap(&cards);
        assert!(has_active_roadmap, "the InProgress goal is not yet Done");

        assert_eq!(
            NoRoadmapGuard::decide(true, has_roadmap_cards, false, false, false, 0),
            AfterTurnAction::Allow,
            "a resumed roadmap this session never created must still read as having one"
        );
        assert_eq!(
            Rule5HonestFailureGuard::decide(
                true,
                false,
                has_roadmap_cards,
                has_active_roadmap,
                false, // an ordinary closing summary, not a question
                1,
                0,
            ),
            AfterTurnAction::Allow,
            "an ordinary closing summary after a resumed roadmap must not park"
        );
    }

    // ── Lane R31b (A30 analysis): stranded-Ready-goal nudge ─────────────────

    /// `stranded_ready_goal_id` finds the one Ready card carrying R26b's
    /// `resumed_by_headless_unblock_at` stamp among a roadmap that also has
    /// an ordinary (unstamped) Ready card and a Complete one — neither of
    /// which may be mistaken for the stranded shape.
    #[tokio::test]
    async fn stranded_ready_goal_id_finds_the_resumed_ready_card_only() {
        let pool = crate::session::SessionManager::instance()
            .pool_clone()
            .await
            .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let project = crate::projects::create_project(
            &pool,
            crate::projects::CreateProject {
                name: format!("r31b-stranded-{}", uuid::Uuid::now_v7()),
                root_path: Some(tmp.path().to_string_lossy().into_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        crate::cards::seed_goal_columns(&pool, &project.id)
            .await
            .unwrap();

        let session_id = format!("r31b-stranded-session-{}", uuid::Uuid::now_v7());
        insert_session_goal_card(&pool, &project.id, &session_id, "complete", true, None).await;
        let ordinary_ready =
            insert_session_goal_card(&pool, &project.id, &session_id, "ready", false, None).await;
        let stranded_ready =
            insert_session_goal_card(&pool, &project.id, &session_id, "ready", false, None).await;
        sqlx::query(
            "UPDATE cards SET metadata_json = json_set(metadata_json, \
             '$.resumed_by_headless_unblock_at', ?) WHERE id = ?",
        )
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(&stranded_ready)
        .execute(&pool)
        .await
        .unwrap();

        let cards = goal_cards_relevant_to_session(&pool, &session_id, tmp.path())
            .await
            .unwrap();
        let found = NoRoadmapGuard::stranded_ready_goal_id(&pool, &cards).await;
        assert_eq!(
            found,
            Some(stranded_ready),
            "must find the stamped Ready card, never the plain Ready one ({ordinary_ready}) or \
             the Complete one"
        );
    }

    /// No card carries the stamp (the ordinary case — nothing stranded) —
    /// `stranded_ready_goal_id` must read `None`, never a false positive off
    /// an unrelated Ready card.
    #[tokio::test]
    async fn stranded_ready_goal_id_is_none_without_the_stamp() {
        let pool = crate::session::SessionManager::instance()
            .pool_clone()
            .await
            .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let project = crate::projects::create_project(
            &pool,
            crate::projects::CreateProject {
                name: format!("r31b-unstranded-{}", uuid::Uuid::now_v7()),
                root_path: Some(tmp.path().to_string_lossy().into_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        crate::cards::seed_goal_columns(&pool, &project.id)
            .await
            .unwrap();

        let session_id = format!("r31b-unstranded-session-{}", uuid::Uuid::now_v7());
        insert_session_goal_card(&pool, &project.id, &session_id, "ready", true, None).await;

        let cards = goal_cards_relevant_to_session(&pool, &session_id, tmp.path())
            .await
            .unwrap();
        assert_eq!(
            NoRoadmapGuard::stranded_ready_goal_id(&pool, &cards).await,
            None
        );
    }

    /// The injected nudge text always contains the fixed marker
    /// `already_nudged_stranded_ready` looks for, and the once-per-session
    /// check correctly recognises it once injected.
    #[test]
    fn stranded_ready_nudge_marker_round_trips_through_already_nudged_check() {
        let injected = "goal some-goal-id is Ready with unlanded work; call \
                         goal_advance(dispatch) or let the run-end sweep land it";
        assert!(injected.contains(STRANDED_READY_NUDGE_MARKER));
        let messages = vec![Message::user().with_text(injected)];
        assert!(NoRoadmapGuard::already_nudged_stranded_ready(&messages));
        assert!(!NoRoadmapGuard::already_nudged_stranded_ready(&[
            Message::assistant().with_text("all done")
        ]));
    }

    // ── Lane R35a (A32 root cause 1): verified-goal-awaiting-land nudge ────

    /// A `ready` card carrying a verified PASS verdict — a prior attempt
    /// committed and verified, but the card cycled back to Ready instead of
    /// ever reaching Review — is the `Review`-action candidate.
    #[tokio::test]
    async fn verified_landing_candidate_finds_a_ready_card_with_a_verified_pass_verdict() {
        let pool = roadmap_guard_test_pool().await;
        let session_id = format!("r35a-ready-{}", uuid::Uuid::now_v7());
        let card_id = insert_session_goal_card(
            &pool,
            crate::projects::PERSONAL_PROJECT_ID,
            &session_id,
            "ready",
            true,
            None,
        )
        .await;
        crate::cards::set_goal_verdict(&pool, &card_id, serde_json::json!({"status": "pass"}))
            .await
            .unwrap();

        let cards = goal_cards_relevant_to_session(
            &pool,
            &session_id,
            std::path::Path::new("/nonexistent"),
        )
        .await
        .unwrap();
        let found = VerifiedGoalAwaitingLandGuard::verified_landing_candidate(&pool, &cards).await;
        assert_eq!(found, Some((card_id, VerifiedLandingAction::Review)));
    }

    /// The same verified PASS verdict on a `review` card is the
    /// `Approve`-action candidate instead — the completion tracker's own
    /// automatic transition already fired and opened the review decision.
    #[tokio::test]
    async fn verified_landing_candidate_finds_a_review_card_with_a_verified_pass_verdict() {
        let pool = roadmap_guard_test_pool().await;
        let session_id = format!("r35a-review-{}", uuid::Uuid::now_v7());
        let card_id = insert_session_goal_card(
            &pool,
            crate::projects::PERSONAL_PROJECT_ID,
            &session_id,
            "review",
            true,
            None,
        )
        .await;
        crate::cards::set_goal_verdict(&pool, &card_id, serde_json::json!({"status": "pass"}))
            .await
            .unwrap();

        let cards = goal_cards_relevant_to_session(
            &pool,
            &session_id,
            std::path::Path::new("/nonexistent"),
        )
        .await
        .unwrap();
        let found = VerifiedGoalAwaitingLandGuard::verified_landing_candidate(&pool, &cards).await;
        assert_eq!(found, Some((card_id, VerifiedLandingAction::Approve)));
    }

    /// A worker still running (`in_progress`) — even one that later gets a
    /// verified verdict stamped onto it (e.g. a checkpoint) — is never a
    /// candidate: the goal is not sitting idle, so nothing should nudge the
    /// coordinator to advance it out from under a live worker.
    #[tokio::test]
    async fn verified_landing_candidate_ignores_an_in_progress_card_even_with_a_verified_verdict() {
        let pool = roadmap_guard_test_pool().await;
        let session_id = format!("r35a-in-progress-{}", uuid::Uuid::now_v7());
        let card_id = insert_session_goal_card(
            &pool,
            crate::projects::PERSONAL_PROJECT_ID,
            &session_id,
            "in_progress",
            true,
            None,
        )
        .await;
        crate::cards::set_goal_verdict(&pool, &card_id, serde_json::json!({"status": "pass"}))
            .await
            .unwrap();

        let cards = goal_cards_relevant_to_session(
            &pool,
            &session_id,
            std::path::Path::new("/nonexistent"),
        )
        .await
        .unwrap();
        let found = VerifiedGoalAwaitingLandGuard::verified_landing_candidate(&pool, &cards).await;
        assert_eq!(found, None, "an in_progress goal must never be a candidate");
    }

    /// A `ready` card with no verdict at all (the ordinary case) is not a
    /// candidate — only an observed PASS makes it one.
    #[tokio::test]
    async fn verified_landing_candidate_ignores_a_ready_card_without_a_verdict() {
        let pool = roadmap_guard_test_pool().await;
        let session_id = format!("r35a-unverified-{}", uuid::Uuid::now_v7());
        insert_session_goal_card(
            &pool,
            crate::projects::PERSONAL_PROJECT_ID,
            &session_id,
            "ready",
            true,
            None,
        )
        .await;

        let cards = goal_cards_relevant_to_session(
            &pool,
            &session_id,
            std::path::Path::new("/nonexistent"),
        )
        .await
        .unwrap();
        let found = VerifiedGoalAwaitingLandGuard::verified_landing_candidate(&pool, &cards).await;
        assert_eq!(found, None);
    }

    /// The injected nudge text always contains the fixed marker
    /// `already_nudged_for_goal` looks for, keyed to the goal id, and the
    /// once-per-goal check correctly recognises it once injected — for
    /// EITHER action shape.
    #[test]
    fn verified_landing_nudge_marker_round_trips_through_already_nudged_check() {
        let review_text = verified_landing_nudge_text("goal-1", &VerifiedLandingAction::Review);
        let approve_text = verified_landing_nudge_text("goal-1", &VerifiedLandingAction::Approve);
        assert!(review_text.contains(VERIFIED_LANDING_NUDGE_MARKER));
        assert!(approve_text.contains(VERIFIED_LANDING_NUDGE_MARKER));
        assert!(review_text.contains("action \"review\""));
        assert!(approve_text.contains("action \"approve\""));

        let messages = vec![Message::user().with_text(review_text)];
        assert!(VerifiedGoalAwaitingLandGuard::already_nudged_for_goal(
            &messages, "goal-1"
        ));
        // A different goal id's nudge must not be mistaken for this one's.
        assert!(!VerifiedGoalAwaitingLandGuard::already_nudged_for_goal(
            &messages, "goal-2"
        ));
        assert!(!VerifiedGoalAwaitingLandGuard::already_nudged_for_goal(
            &[Message::assistant().with_text("all done")],
            "goal-1"
        ));
    }

    // ── Rule5HonestFailureGuard (Lane R18f, A17 analysis Class 4) ──────────
    //
    // The live `after_turn` needs a headless coding-harness session row
    // (recipe title, `GooseMode::Auto`, no parent) to reach its interesting
    // branches — the same DB dependency every other guard's live path has in
    // this file, which is why their own tests exercise the PURE predicates
    // directly rather than standing up that session shape. These tests do
    // the same for the two predicates that make this guard's Class 4 fix
    // correct: distinguishing an ATTEMPTED roadmap call from a SUCCESSFUL
    // one, and a pure free-text ending from any other kind.

    /// The exact Class 4 shape: `decompose_roadmap` was called and errored
    /// (the `-32002` case), so it must NOT read as success — this is what
    /// `turn_called_any_tool` (NoRoadmapGuard's weaker check) gets wrong and
    /// `turn_tool_call_succeeded` exists to fix.
    #[test]
    fn turn_tool_call_succeeded_is_false_for_a_failed_decompose_roadmap_call() {
        let failed = vec![tool_exchange("decompose_roadmap", "call-1", false)];
        assert!(
            turn_called_any_tool(&failed, &ROADMAP_TOOL_NAMES),
            "a failed call is still an ATTEMPT — NoRoadmapGuard's weaker check must see it"
        );
        assert!(
            !turn_tool_call_succeeded(&failed, &ROADMAP_TOOL_NAMES),
            "a failed call must never read as a SUCCESSFUL roadmap request"
        );
    }

    #[test]
    fn turn_tool_call_succeeded_is_true_for_a_successful_create_roadmap_call() {
        let ok = vec![tool_exchange("create_roadmap", "call-1", true)];
        assert!(turn_tool_call_succeeded(&ok, &ROADMAP_TOOL_NAMES));
    }

    #[test]
    fn turn_tool_call_succeeded_is_false_when_never_called() {
        let plain = vec![Message::assistant().with_text("Here is a plan.")];
        assert!(!turn_tool_call_succeeded(&plain, &ROADMAP_TOOL_NAMES));
    }

    /// A turn with SOME later successful call still counts, even if an
    /// earlier attempt at the same tool failed — matches "decompose_roadmap
    /// errored once, the model retried, and the retry landed".
    #[test]
    fn turn_tool_call_succeeded_true_when_a_later_retry_landed() {
        let mut messages = vec![tool_exchange("decompose_roadmap", "call-1", false)];
        messages.push(tool_exchange("decompose_roadmap", "call-2", true));
        assert!(turn_tool_call_succeeded(&messages, &ROADMAP_TOOL_NAMES));
    }

    /// The Class 4 free-text signature itself: after a failed
    /// `decompose_roadmap` and a `search_available_extensions` probe, the
    /// FINAL message is pure text with no tool call at all.
    #[test]
    fn final_message_has_tool_call_is_false_after_a_free_text_ending() {
        let messages =
            vec![
            tool_exchange("decompose_roadmap", "call-1", false),
            tool_exchange("extensionmanager__search_available_extensions", "call-2", true),
            Message::assistant().with_text(
                "The decompose_roadmap tool isn't available in this session — I confirmed that \
                 by probing the extension manager.",
            ),
        ];
        assert!(!final_message_has_tool_call(&messages));
    }

    #[test]
    fn final_message_has_tool_call_is_true_when_the_turn_ends_on_a_call() {
        let messages = vec![
            Message::assistant().with_text("Let me check."),
            tool_exchange("decompose_roadmap", "call-1", true),
        ];
        assert!(final_message_has_tool_call(&messages));
    }

    #[test]
    fn final_message_has_tool_call_is_false_for_an_empty_turn() {
        let empty: Vec<Message> = vec![];
        assert!(!final_message_has_tool_call(&empty));
    }

    /// An explicit `escalate` call anywhere in the turn is rule 5's own
    /// honest exit — the guard's live `after_turn` must never fire on it.
    /// Exercised at the trait-impl level (not just the predicate) because
    /// this short-circuit runs before any session lookup, so it needs no DB.
    #[tokio::test]
    async fn rule5_guard_allows_a_turn_that_escalated_even_with_no_other_tool_call() {
        let guard = Rule5HonestFailureGuard::new();
        let escalated = vec![
            tool_exchange("decompose_roadmap", "call-1", false),
            tool_exchange("escalate", "call-2", true),
        ];
        assert_eq!(
            guard
                .after_turn(&AfterTurnContext {
                    delegate_tool_available: true,
                    session_id: "rule5-escalated",
                    messages: &escalated,
                    prior_holds: 0,
                })
                .await,
            AfterTurnAction::Allow,
            "an explicit escalate call is rule 5's own honest exit"
        );
    }

    /// A turn that ends by calling literally anything (not `escalate`, not a
    /// roadmap tool) is not the Class 4 free-text shape and must not fire —
    /// this is the guard's cheapest, DB-free gate.
    #[tokio::test]
    async fn rule5_guard_allows_a_turn_that_ends_on_any_tool_call() {
        let guard = Rule5HonestFailureGuard::new();
        let messages = vec![tool_exchange("decompose_roadmap", "call-1", false)];
        assert_eq!(
            guard
                .after_turn(&AfterTurnContext {
                    delegate_tool_available: true,
                    session_id: "rule5-other-tool",
                    messages: &messages,
                    prior_holds: 0,
                })
                .await,
            AfterTurnAction::Allow,
            "the final message here DOES have a tool call, so this is not a free-text ending"
        );
    }

    /// An unknown/non-harness session id (the session store cannot confirm a
    /// headless coding-harness coordinator) is let through untouched — same
    /// fail-open contract every other guard's live path makes in this file.
    #[tokio::test]
    async fn rule5_guard_allows_a_session_it_cannot_confirm() {
        let session_id = format!("rule5-unknown-{}", uuid::Uuid::now_v7());
        let guard = Rule5HonestFailureGuard::new();
        let free_text = vec![
            tool_exchange("decompose_roadmap", "call-1", false),
            Message::assistant()
                .with_text("I can't find that tool — what would you like me to do?"),
        ];
        assert_eq!(
            guard
                .after_turn(&AfterTurnContext {
                    delegate_tool_available: true,
                    session_id: &session_id,
                    messages: &free_text,
                    prior_holds: 0,
                })
                .await,
            AfterTurnAction::Allow
        );
    }

    // ── Rule5HonestFailureGuard mid-session extension (Lane R20c, A20 Q3) ──
    //
    // `Rule5HonestFailureGuard::decide` takes every DB-derived answer as a
    // parameter, the same seam `NoRoadmapGuard::decide` established, so each
    // branch below — including R20c's new mid-session shape — is a pure,
    // isolated test with no session store or database.

    /// `final_message_reads_as_question`'s trailing-`?` path.
    #[test]
    fn final_message_reads_as_question_detects_a_trailing_question_mark() {
        let messages = vec![Message::assistant()
            .with_text("The deploy provider isn't configured anywhere in this workspace.")];
        assert!(!final_message_reads_as_question(&messages));

        let messages =
            vec![Message::assistant().with_text("Which cloud provider should I target?")];
        assert!(final_message_reads_as_question(&messages));
    }

    /// Each documented phrase fires on its own, not only the trailing `?`.
    #[test]
    fn final_message_reads_as_question_detects_each_documented_phrase() {
        for text in [
            "Please provide the API key before I continue.",
            "I'm not sure which approach you'd prefer here.",
            "Should I use the cache or pagination approach.",
            "Let me know how you would like to proceed.",
            "I need you to confirm the target environment.",
        ] {
            let messages = vec![Message::assistant().with_text(text)];
            assert!(
                final_message_reads_as_question(&messages),
                "{text:?} should read as a question"
            );
        }

        // A negative control: none of the five phrases, and no trailing `?`.
        let closing = vec![Message::assistant()
            .with_text("There are two valid approaches here and I picked the cache.")];
        assert!(!final_message_reads_as_question(&closing));
    }

    /// `contains_word_or_phrase`'s whole-word guard: `"confirm"` must not
    /// match inside a larger word like "confirmed"/"confirmation" — the
    /// realistic false-positive risk for a bare `.contains()` check.
    #[test]
    fn contains_word_or_phrase_does_not_match_inside_a_larger_word() {
        assert!(!contains_word_or_phrase(
            "the deploy step confirmed all checks passed",
            "confirm"
        ));
        assert!(!contains_word_or_phrase(
            "reconfirmation is not required here",
            "confirm"
        ));
        assert!(contains_word_or_phrase(
            "please confirm the target environment",
            "confirm"
        ));
        assert!(contains_word_or_phrase("which file should i edit", "which"));
    }

    /// A legitimate closing summary — plain prose, no question, all goals
    /// already Done — is not a phrase match and must not fire.
    #[test]
    fn final_message_reads_as_question_is_false_for_an_ordinary_closing_summary() {
        let messages = vec![Message::assistant().with_text(
            "All three goals are Done and the diff has passed review. Nothing further to do.",
        )];
        assert!(!final_message_reads_as_question(&messages));
    }

    /// R20c's headline case: an existing roadmap, at least one goal not yet
    /// Done, and a free-text question — the exact
    /// `select-missing-credential-escalates-not-guesses` shape from the A20
    /// analysis (worker used free-text `send_message` instead of `escalate`
    /// mid-session). The FIRST free-text turn (`consecutive_free_text_turns
    /// == 1`) always gets the mid-session nudge, never a park — R20c-b's
    /// requirement 1.
    #[test]
    fn rule5_decide_fires_for_a_mid_session_question_with_open_goals() {
        match Rule5HonestFailureGuard::decide(
            true,  // headless coding coordinator
            false, // no roadmap tool succeeded this turn
            true,  // has_roadmap_cards
            true,  // has_active_roadmap: a goal is not yet Done
            true,  // final message reads as a question
            1,     // first consecutive free-text turn
            0,
        ) {
            AfterTurnAction::Continue { inject } => {
                assert_eq!(inject, RULE5_MIDSESSION_INJECT_TEXT);
                assert!(inject.contains("escalate"));
            }
            other => panic!("expected Continue, got {other:?}"),
        }
    }

    /// R20c-b's core fix, mid-session shape: the SECOND consecutive
    /// free-text turn parks with the `rule5_free_text_question` marker even
    /// when `prior_holds` is nowhere near `MAX_HOLDS` (0, not 2) — proving
    /// the two-strike count, not the shared hold budget, drives this
    /// decision now. Before this fix the equivalent call
    /// (`prior_holds < MAX_HOLDS` was the only gate) would have returned
    /// `Continue` here instead, the extra-nudge bug the M22 round exposed.
    #[test]
    fn rule5_decide_parks_the_mid_session_question_on_the_second_consecutive_free_text_turn() {
        match Rule5HonestFailureGuard::decide(true, false, true, true, true, 2, 0) {
            AfterTurnAction::Park { reason } => {
                assert_eq!(reason, RULE5_MIDSESSION_PARK_REASON);
                assert!(reason.starts_with("rule5_free_text_question"));
            }
            other => panic!("expected Park, got {other:?}"),
        }
    }

    /// A closing summary reached after every goal is already Done/Cancelled
    /// must never fire, even if `has_roadmap_cards` is true — the exception
    /// the task explicitly calls out.
    #[test]
    fn rule5_decide_allows_a_closing_summary_after_all_goals_are_done() {
        assert_eq!(
            Rule5HonestFailureGuard::decide(
                true,  // headless coding coordinator
                false, // no roadmap tool succeeded this turn
                true,  // has_roadmap_cards
                false, // has_active_roadmap: every card is complete/cancelled
                true,  // even a question-shaped closing message
                1, 0,
            ),
            AfterTurnAction::Allow,
            "every goal already Done is a legitimate closing summary, not a rule-5 failure"
        );
    }

    /// The bare-session shape (R18f, no cards at all): the FIRST free-text
    /// turn nudges, never parks — R20c-b's requirement 1, bare-session side.
    #[test]
    fn rule5_decide_fires_a_nudge_not_a_park_on_the_first_bare_session_free_text_turn() {
        match Rule5HonestFailureGuard::decide(true, false, false, false, false, 1, 0) {
            AfterTurnAction::Continue { inject } => assert_eq!(inject, RULE5_INJECT_TEXT),
            other => panic!("expected Continue, got {other:?}"),
        }
    }

    /// R20c-b's core fix, bare-session shape: the SECOND consecutive
    /// free-text turn parks with `rule5_free_text_turn` even at
    /// `prior_holds == 0` — the two-strike count alone is sufficient, no
    /// shared-budget exhaustion required. This is exactly the M22 shape:
    /// `NoRoadmapGuard`'s own turn-1 nudge already spent one shared hold
    /// elsewhere, so `prior_holds` reaching this guard on turn 2 could be as
    /// low as 1 (still `< MAX_HOLDS == 2`) — the count is what must catch it.
    #[test]
    fn rule5_decide_parks_the_bare_session_shape_on_the_second_consecutive_free_text_turn() {
        match Rule5HonestFailureGuard::decide(true, false, false, false, false, 2, 0) {
            AfterTurnAction::Park { reason } => {
                assert_eq!(reason, RULE5_PARK_REASON);
                assert!(reason.starts_with("rule5_free_text_turn"));
            }
            other => panic!("expected Park, got {other:?}"),
        }
    }

    /// The exact M22 regression, at the `decide` level: a first strike
    /// (`consecutive_free_text_turns == 1`) must still nudge, NEVER park,
    /// even when `prior_holds` is already 1 (as it would be after
    /// `NoRoadmapGuard`'s own turn-1 nudge spent one of the two shared
    /// holds). Before R20c-b, `prior_holds (1) < MAX_HOLDS (2)` was the
    /// ONLY gate, so this exact input already produced `Continue` — this
    /// test does not change behaviour here, it PINS it, so a future edit
    /// cannot silently make a first strike start parking early just because
    /// some other hook happened to spend a hold first.
    #[test]
    fn rule5_decide_first_strike_never_parks_even_if_another_hook_already_spent_a_shared_hold() {
        match Rule5HonestFailureGuard::decide(true, false, false, false, false, 1, 1) {
            AfterTurnAction::Continue { .. } => {}
            other => panic!("expected Continue on a first strike, got {other:?}"),
        }
    }

    /// The defensive ceiling: `prior_holds >= MAX_HOLDS` still parks even on
    /// what would otherwise read as a first strike (`consecutive_free_text_
    /// turns == 1`) — the OR'd second condition in `decide`, so this guard
    /// can never itself exceed the reply loop's own global hold budget no
    /// matter how the two-strike count reads. Matches the task's "or when
    /// no turns remain" clause.
    #[test]
    fn rule5_decide_parks_when_the_shared_hold_budget_is_exhausted_even_on_a_first_strike() {
        match Rule5HonestFailureGuard::decide(
            true,
            false,
            false,
            false,
            false,
            1,
            crate::cost_router::MAX_HOLDS,
        ) {
            AfterTurnAction::Park { reason } => assert_eq!(reason, RULE5_PARK_REASON),
            other => panic!("expected Park, got {other:?}"),
        }
    }

    /// A roadmap that exists but has no goal that is still open (mirrors the
    /// closing-summary case above from the other input) plus a NON-question
    /// free-text ending must not fire either — only a question-shaped ending
    /// with an open goal does.
    #[test]
    fn rule5_decide_allows_mid_session_free_text_that_does_not_read_as_a_question() {
        assert_eq!(
            Rule5HonestFailureGuard::decide(true, false, true, true, false, 1, 0),
            AfterTurnAction::Allow,
            "an open goal alone is not enough — the final message must also read as a question"
        );
    }

    /// Interactive sessions (non-headless) never fire, for either shape —
    /// `is_headless_coding_coordinator` gates the whole function before any
    /// card/question/strike state is even consulted.
    #[test]
    fn rule5_decide_never_fires_for_an_interactive_non_headless_session() {
        assert_eq!(
            Rule5HonestFailureGuard::decide(false, false, false, false, false, 2, 0),
            AfterTurnAction::Allow,
            "bare-session shape, interactive, even at the second strike"
        );
        assert_eq!(
            Rule5HonestFailureGuard::decide(false, false, true, true, true, 2, 0),
            AfterTurnAction::Allow,
            "mid-session question shape, interactive, even at the second strike"
        );
    }

    /// A roadmap tool call that SUCCEEDED this turn releases both shapes —
    /// the coordinator just re-decomposed/resumed, which is never a rule-5
    /// failure regardless of card/question/strike state.
    #[test]
    fn rule5_decide_allows_when_a_roadmap_tool_succeeded_this_turn() {
        assert_eq!(
            Rule5HonestFailureGuard::decide(true, true, true, true, true, 2, 0),
            AfterTurnAction::Allow
        );
    }

    // ── consecutive_free_text_coordinator_turns (Lane R20c-b) ──────────────

    /// The plain count: two trailing assistant messages with no tool call,
    /// separated by an injected user nudge (which must be skipped, not
    /// counted as its own turn) — the exact shape the M22 transcript took.
    #[test]
    fn consecutive_free_text_coordinator_turns_counts_trailing_free_text_assistant_turns() {
        let messages = vec![
            Message::assistant().with_text("Ran into this error: Credits exhausted."),
            Message::user().with_text(NO_ROADMAP_INJECT_TEXT),
            Message::assistant().with_text("Ran into this error: Credits exhausted."),
        ];
        assert_eq!(consecutive_free_text_coordinator_turns(&messages), 2);
    }

    /// A single free-text turn (nothing before it) counts as exactly one —
    /// the first-strike shape.
    #[test]
    fn consecutive_free_text_coordinator_turns_is_one_for_a_lone_free_text_turn() {
        let messages = vec![Message::assistant().with_text("Here is a plan. Say the word.")];
        assert_eq!(consecutive_free_text_coordinator_turns(&messages), 1);
    }

    /// R20c-b's "a tool call in between resets the count" requirement: an
    /// older free-text assistant turn sits behind a LATER assistant turn
    /// that DID call a tool — scanning backward from the most recent
    /// message must stop at that tool call, so only the ONE free-text turn
    /// after it counts, not three.
    #[test]
    fn consecutive_free_text_coordinator_turns_resets_at_the_most_recent_tool_call() {
        let messages = vec![
            Message::assistant().with_text("An older free-text turn."),
            tool_exchange("decompose_roadmap", "call-1", true),
            Message::user().with_text(RULE5_INJECT_TEXT),
            Message::assistant().with_text("A newer free-text turn, after the tool call."),
        ];
        assert_eq!(
            consecutive_free_text_coordinator_turns(&messages),
            1,
            "the decompose_roadmap call must reset the streak — only the turn AFTER it counts"
        );
    }

    /// An empty conversation, and a conversation with no assistant turns at
    /// all, both count as zero — nothing to reset from, nothing to count.
    #[test]
    fn consecutive_free_text_coordinator_turns_is_zero_with_no_assistant_turns() {
        assert_eq!(consecutive_free_text_coordinator_turns(&[]), 0);
        let user_only = vec![Message::user().with_text("do the thing")];
        assert_eq!(consecutive_free_text_coordinator_turns(&user_only), 0);
    }

    /// Trait-impl level, mirroring `rule5_guard_allows_a_turn_that_escalated_
    /// even_with_no_other_tool_call` above but for the mid-session shape
    /// specifically: an `escalate` call anywhere in the turn releases a
    /// question-shaped free-text ending exactly like the bare-session one —
    /// checked before any session/DB lookup, so an unconfirmable session id
    /// is sufficient to prove it without standing up a real roadmap.
    #[tokio::test]
    async fn rule5_guard_allows_a_mid_session_question_when_escalate_was_called_this_turn() {
        let guard = Rule5HonestFailureGuard::new();
        let escalated_question = vec![
            tool_exchange("decompose_roadmap", "call-1", true),
            tool_exchange("escalate", "call-2", true),
            Message::assistant()
                .with_text("Which credential should I use — the staging or prod key?"),
        ];
        assert_eq!(
            guard
                .after_turn(&AfterTurnContext {
                    delegate_tool_available: true,
                    session_id: "rule5-midsession-escalated",
                    messages: &escalated_question,
                    prior_holds: 0,
                })
                .await,
            AfterTurnAction::Allow,
            "an escalate call anywhere in the turn is the honest exit for both shapes"
        );
    }

    // ── Lane R24b: a free-text question at rule-5 park time is minted as an
    // escalation ────────────────────────────────────────────────────────

    #[test]
    fn rule5_park_should_escalate_instead_only_fires_for_a_park_with_a_question() {
        let park = AfterTurnAction::Park {
            reason: "x".to_string(),
        };
        let cont = AfterTurnAction::Continue {
            inject: "x".to_string(),
        };
        assert!(
            rule5_park_should_escalate_instead(&park, true),
            "a park whose final message reads as a question must be redirected"
        );
        assert!(
            !rule5_park_should_escalate_instead(&park, false),
            "a park whose final message is a plan/refusal/silence, NOT a question, \
             must keep today's honest failure"
        );
        assert!(
            !rule5_park_should_escalate_instead(&cont, true),
            "a first-strike Continue must never mint — only a park (second strike) does"
        );
        assert!(!rule5_park_should_escalate_instead(
            &AfterTurnAction::Allow,
            true
        ));
    }

    #[test]
    fn rule5_escalation_kind_is_information_for_an_ordinary_question() {
        assert_eq!(
            rule5_escalation_kind("Which credential should I use for the staging API?"),
            "information"
        );
    }

    #[test]
    fn rule5_escalation_kind_is_decision_for_an_either_or_question() {
        assert_eq!(
            rule5_escalation_kind("Should I use the staging key or the prod key?"),
            "decision"
        );
    }

    #[test]
    fn rule5_escalation_payload_carries_the_question_verbatim_and_a_minted_by_tag() {
        let question = "Which credential should I use for the staging API?";
        let raw = rule5_escalation_payload(question);
        assert_eq!(raw["kind"], "information");
        assert_eq!(raw["specific_ask"], RULE5_ESCALATION_HEADLINE);
        assert_eq!(raw["why_blocked"], question);
        assert_eq!(raw["evidence_refs"][0], "minted_by:rule5_guard");
        assert_eq!(raw["resume"], "auto");
    }

    /// The templated headline must always satisfy Amendment A1's own rules —
    /// this is what keeps a mint from ever falling to `DecisionKind::
    /// Malformed` just because the model's underlying question happened to
    /// contain a path, a PR number, or a long hex id.
    #[test]
    fn rule5_escalation_headline_is_always_amendment_a1_clean() {
        assert!(
            crate::decision_inbox::escalate::headline_violations(RULE5_ESCALATION_HEADLINE)
                .is_empty()
        );
    }

    /// Task requirement: a question-shaped final turn mints a decision
    /// carrying the model's own question text, increments the same
    /// `dag_enforcement.escalated` receipt a real `escalate` tool call
    /// would, and does so through an INJECTED sink — no process-global
    /// installed or read.
    #[tokio::test]
    #[serial_test::serial(receipts_global_registry)]
    async fn mint_rule5_question_escalation_records_the_question_text_and_receipt() {
        use crate::decision_inbox::escalate::{DecisionKind, InMemoryDecisionSink};

        crate::receipts::begin_run();
        let sink = InMemoryDecisionSink::default();
        let question = "Which credential should I use for the staging API?";
        let recorded = mint_rule5_question_escalation(&sink, "sess-r24b-1", question)
            .await
            .expect("mint must succeed against a healthy sink");
        let after = crate::receipts::snapshot().lines();
        crate::receipts::end_run();

        assert!(
            after[0].contains("escalated=1"),
            "expected dag_enforcement.escalated to increment exactly once: {}",
            after[0]
        );

        let drafts = sink.recorded();
        assert_eq!(drafts.len(), 1, "exactly one decision must be minted");
        assert_eq!(drafts[0].0, recorded.decision_id);
        let draft = &drafts[0].1;
        assert_eq!(
            draft.kind,
            DecisionKind::Unblock,
            "an ordinary question maps to an information/unblock ask"
        );
        assert_eq!(
            draft.detail, question,
            "the model's own question text must be preserved verbatim as the decision's detail"
        );
        assert!(draft
            .evidence_refs
            .iter()
            .any(|r| r == "minted_by:rule5_guard"));
        assert!(
            draft.validation_errors.is_empty(),
            "the templated headline must never fall to Malformed"
        );
    }

    /// A sink failure must surface as `Err`, never as a silently-successful
    /// mint — the live caller falls back to the ordinary honest-failure
    /// park in that case rather than claiming a clean exit for an
    /// escalation that was never actually recorded.
    struct FailingDecisionSink;

    #[async_trait]
    impl crate::decision_inbox::escalate::DecisionSink for FailingDecisionSink {
        async fn record(
            &self,
            _draft: crate::decision_inbox::escalate::DecisionDraft,
        ) -> anyhow::Result<crate::decision_inbox::escalate::RecordedDecision> {
            Err(anyhow::anyhow!("sink unavailable"))
        }
    }

    #[tokio::test]
    async fn mint_rule5_question_escalation_surfaces_a_sink_error_instead_of_panicking() {
        let sink = FailingDecisionSink;
        let err = mint_rule5_question_escalation(&sink, "sess-r24b-2", "Which key should I use?")
            .await
            .expect_err("a failing sink must surface as an Err, not panic or silently succeed");
        assert!(err.contains("Lane R24b"));
    }

    /// Task requirement: a question-shaped final message with an `escalate`
    /// call ALREADY present earlier in the same turn must never mint a
    /// second decision. `turn_called_escalate` already short-circuits
    /// `Rule5HonestFailureGuard::after_turn` to `Allow` before `Self::decide`
    /// (and therefore before `mint_rule5_question_escalation`) is ever
    /// reached — see `rule5_guard_allows_a_mid_session_question_when_
    /// escalate_was_called_this_turn` above for the same transcript shape.
    /// This test pins that guarantee observably: the `dag_enforcement.
    /// escalated` receipt this lane's own mint path would increment stays
    /// at zero, proving the mint path never fired (a real, live `escalate`
    /// tool call — not exercised by this transcript, which only mimics one
    /// — is what increments it in production, via `record_escalation`).
    #[tokio::test]
    #[serial_test::serial(receipts_global_registry)]
    async fn rule5_guard_does_not_mint_a_second_escalation_when_one_was_already_called_this_turn() {
        crate::receipts::begin_run();
        let guard = Rule5HonestFailureGuard::new();
        let escalated_question = vec![
            tool_exchange("decompose_roadmap", "call-1", true),
            tool_exchange("escalate", "call-2", true),
            Message::assistant()
                .with_text("Which credential should I use — the staging or prod key?"),
        ];
        let action = guard
            .after_turn(&AfterTurnContext {
                delegate_tool_available: true,
                session_id: "rule5-no-double-mint",
                messages: &escalated_question,
                prior_holds: 0,
            })
            .await;
        let after = crate::receipts::snapshot().lines();
        crate::receipts::end_run();

        assert_eq!(action, AfterTurnAction::Allow);
        assert!(
            after[0].contains("escalated=0"),
            "the rule-5 guard's own mint path must never fire once `escalate` was already \
             called this turn: {}",
            after[0]
        );
    }

    /// Task requirement: a plan-in-prose (not question-shaped) turn keeps
    /// today's rule-5 honest failure unchanged — `decide` still parks, and
    /// this lane's new override never engages because
    /// `final_message_reads_as_question` is false. Pins the exact pre-
    /// existing behaviour `rule5_decide_parks_the_bare_session_shape_on_
    /// the_second_consecutive_free_text_turn` (above) already covers, from
    /// the override predicate's own side.
    #[test]
    fn rule5_decide_still_parks_a_non_question_free_text_turn_on_the_second_strike() {
        let action = Rule5HonestFailureGuard::decide(true, false, false, false, false, 2, 0);
        assert!(matches!(action, AfterTurnAction::Park { .. }));
        assert!(
            !rule5_park_should_escalate_instead(&action, false),
            "a plan-in-prose ending (reads_as_question == false) must never be redirected"
        );
    }

    /// Real-log-shape banner check: the exact `▸ escalate` line this lane's
    /// mint function prints (`  ▸ escalate`, blank line + 40-dash separator
    /// before it, mirroring Lane R22a's `check_worker` banner precedent) is
    /// recognized by the REAL eval-driver log scanner as an `escalate` tool
    /// call — the same proof R22a-c used for its own synthesized
    /// `check_worker` banner (`permagent-eval`'s
    /// `select_checks_worker_before_dispatch_recognizes_goal_advances_
    /// synthesized_check_worker_banner`).
    #[test]
    fn rule5_minted_escalation_banner_is_recognized_by_the_real_log_scanner() {
        let sep = "─".repeat(40);
        let log = format!(
            "some earlier prose\n  {sep}\n  ▸ decompose_roadmap\n\n  {sep}\n  ▸ escalate\n"
        );
        let signals = permagent_eval::harness_log::scan(&log);
        assert_eq!(signals.tool_names, vec!["decompose_roadmap", "escalate"]);
    }

    // ── Lane R58a: a headless free-text turn with a Ready goal undispatched ──

    /// r59 q4's exact final sentence: a routing ask with no trailing `?` and
    /// none of `RULE5_QUESTION_PHRASES` — why `decide`'s mid-session shape
    /// allowed it, and why R58a keys on card state instead.
    fn r59_q4_routing_ask() -> Vec<Message> {
        vec![
            tool_exchange("check_worker", "call-1", true),
            tool_exchange("routing_roster", "call-2", true),
            Message::assistant().with_text(
                "One thing I need from you before I dispatch: the routing choice for this card \
                 is user-visible spend ($0.06/1k on the frontier rung vs $0 on-device), and the \
                 roster's own descriptions conflict on whether a repo-wide rename counts as \
                 \"small.\" I won't pick that one silently.",
            ),
        ]
    }

    #[test]
    fn r58a_the_r59_q4_routing_ask_slips_past_the_mid_session_question_shape() {
        let messages = r59_q4_routing_ask();
        assert!(!final_message_has_tool_call(&messages));
        assert!(!final_message_reads_as_question(&messages));
        assert_eq!(consecutive_free_text_coordinator_turns(&messages), 1);
        assert_eq!(
            Rule5HonestFailureGuard::decide(true, false, true, true, false, 1, 0),
            AfterTurnAction::Allow,
            "the pre-R58a shapes allow this turn — the gap this lane closes"
        );
    }

    /// Strike 1: seeded Ready goal, nothing dispatched, headless → nudge.
    #[test]
    fn r58a_first_free_text_turn_with_an_undispatched_ready_goal_nudges() {
        let messages = r59_q4_routing_ask();
        match Rule5HonestFailureGuard::decide_undispatched_ready_goal(
            AfterTurnAction::Allow,
            true, // headless CLI run
            true, // Ready root goal, no worker session
            turn_called_any_tool(&messages, &DISPATCH_PREP_TOOL_NAMES),
            turn_tool_call_succeeded(&messages, &["goal_advance"]),
            consecutive_free_text_coordinator_turns(&messages),
            0,
        ) {
            AfterTurnAction::Continue { inject } => {
                assert_eq!(inject, RULE5_READY_GOAL_INJECT_TEXT);
                assert!(inject.contains("no user will answer"));
                assert!(inject.contains("goal_advance"));
                assert!(inject.contains("escalate"));
            }
            other => panic!("expected Continue, got {other:?}"),
        }
    }

    /// Strike 2: the same free text after the nudge → park, and the park is
    /// redirected into R24b's minted escalation even though the prose does
    /// not read as a question.
    #[test]
    fn r58a_second_free_text_turn_parks_and_is_redirected_into_a_minted_escalation() {
        let mut messages = r59_q4_routing_ask();
        messages.push(Message::user().with_text(RULE5_READY_GOAL_INJECT_TEXT));
        messages.push(
            Message::assistant()
                .with_text("I still won't pick the routing tier for this card silently."),
        );
        assert_eq!(consecutive_free_text_coordinator_turns(&messages), 2);
        let action = Rule5HonestFailureGuard::decide_undispatched_ready_goal(
            AfterTurnAction::Allow,
            true,
            true,
            turn_called_any_tool(&messages, &DISPATCH_PREP_TOOL_NAMES),
            turn_tool_call_succeeded(&messages, &["goal_advance"]),
            consecutive_free_text_coordinator_turns(&messages),
            1,
        );
        match &action {
            AfterTurnAction::Park { reason } => {
                assert_eq!(reason, RULE5_READY_GOAL_PARK_REASON);
                assert!(reason.starts_with("rule5_ready_goal_undispatched"));
            }
            other => panic!("expected Park, got {other:?}"),
        }
        let reads_as_question = final_message_reads_as_question(&messages);
        assert!(!reads_as_question);
        // The live path's `ready_goal_shape` is true exactly when this park
        // came from `decide_undispatched_ready_goal`, as it did here.
        let ready_goal_shape = matches!(
            &action,
            AfterTurnAction::Park { reason } if reason == RULE5_READY_GOAL_PARK_REASON
        );
        assert!(
            rule5_park_should_escalate_instead(&action, reads_as_question || ready_goal_shape),
            "the ready-goal shape mints on its second strike regardless of phrasing"
        );
    }

    /// An interactive session (no headless dispatch policy installed) has a
    /// user who can answer — the identical free text is never held.
    #[test]
    fn r58a_interactive_session_with_the_same_free_text_is_not_guarded() {
        let messages = r59_q4_routing_ask();
        for strikes in [1, 2] {
            assert_eq!(
                Rule5HonestFailureGuard::decide_undispatched_ready_goal(
                    AfterTurnAction::Allow,
                    false, // interactive: no headless dispatch policy
                    true,
                    true,
                    turn_tool_call_succeeded(&messages, &["goal_advance"]),
                    strikes,
                    0,
                ),
                AfterTurnAction::Allow
            );
        }
    }

    /// A turn that dispatched (a successful `goal_advance`) is not the shape;
    /// a refused one still is. Neither is a session with no undispatched
    /// Ready goal, one that never touched the dispatch path, nor a turn
    /// `decide` already acted on.
    #[test]
    fn r58a_a_turn_that_dispatched_is_not_guarded() {
        let dispatched = vec![
            tool_exchange("check_worker", "call-1", true),
            tool_exchange("orchestrator__goal_advance", "call-2", true),
            Message::assistant().with_text("Dispatched the rename to claude_code on `local`."),
        ];
        assert!(turn_tool_call_succeeded(&dispatched, &["goal_advance"]));
        assert_eq!(
            Rule5HonestFailureGuard::decide_undispatched_ready_goal(
                AfterTurnAction::Allow,
                true,
                true,
                true,
                turn_tool_call_succeeded(&dispatched, &["goal_advance"]),
                consecutive_free_text_coordinator_turns(&dispatched),
                0,
            ),
            AfterTurnAction::Allow
        );
        // A refused dispatch (is_error) did NOT dispatch — still guarded.
        let refused = vec![
            tool_exchange("goal_advance", "call-1", false),
            Message::assistant().with_text("The dispatch was refused."),
        ];
        assert!(!turn_tool_call_succeeded(&refused, &["goal_advance"]));
        assert!(matches!(
            Rule5HonestFailureGuard::decide_undispatched_ready_goal(
                AfterTurnAction::Allow,
                true,
                true,
                turn_called_any_tool(&refused, &DISPATCH_PREP_TOOL_NAMES),
                turn_tool_call_succeeded(&refused, &["goal_advance"]),
                consecutive_free_text_coordinator_turns(&refused),
                0,
            ),
            AfterTurnAction::Continue { .. }
        ));
        // No undispatched Ready goal → nothing to act on.
        assert_eq!(
            Rule5HonestFailureGuard::decide_undispatched_ready_goal(
                AfterTurnAction::Allow,
                true,
                false,
                true,
                false,
                1,
                0,
            ),
            AfterTurnAction::Allow
        );
        // A session that never touched the dispatch path (an unrelated task
        // sharing a cwd with a stale Ready card) is not the shape either.
        let unrelated = vec![
            tool_exchange("search", "call-1", true),
            Message::assistant().with_text("The review found no issues in that module."),
        ];
        assert!(!turn_called_any_tool(&unrelated, &DISPATCH_PREP_TOOL_NAMES));
        assert_eq!(
            Rule5HonestFailureGuard::decide_undispatched_ready_goal(
                AfterTurnAction::Allow,
                true,
                true,
                turn_called_any_tool(&unrelated, &DISPATCH_PREP_TOOL_NAMES),
                false,
                1,
                0,
            ),
            AfterTurnAction::Allow
        );
        // An existing non-Allow decision from `decide` is passed through
        // unchanged — the shapes never compete.
        let mid_session = AfterTurnAction::Continue {
            inject: RULE5_MIDSESSION_INJECT_TEXT.to_string(),
        };
        assert_eq!(
            Rule5HonestFailureGuard::decide_undispatched_ready_goal(
                mid_session.clone(),
                true,
                true,
                true,
                false,
                1,
                0,
            ),
            mid_session
        );
    }

    /// Only a Ready ROOT goal no worker ever took counts as dispatchable:
    /// a dependent Ready goal may be waiting on its dependency, and a goal
    /// with a worker session belongs to R31b/R35a's own nudges.
    #[test]
    fn r58a_has_undispatched_ready_root_goal_only_matches_a_never_dispatched_ready_root() {
        fn card(
            state: &str,
            is_root: bool,
            has_worker_session: bool,
        ) -> crate::cards::SessionGoalCard {
            crate::cards::SessionGoalCard {
                card_id: "c1".to_string(),
                title: "goal".to_string(),
                state_binding: state.to_string(),
                is_root,
                has_worker_session,
            }
        }
        assert!(has_undispatched_ready_root_goal(&[card(
            "ready", true, false
        )]));
        assert!(!has_undispatched_ready_root_goal(&[]));
        assert!(!has_undispatched_ready_root_goal(&[card(
            "ready", false, false
        )]));
        assert!(!has_undispatched_ready_root_goal(&[card(
            "ready", true, true
        )]));
        assert!(!has_undispatched_ready_root_goal(&[
            card("in_progress", true, true),
            card("complete", true, true),
        ]));
    }

    /// The seeded-fixture shape end to end against the store: a project whose
    /// root matches the cwd and one Ready root card this session never
    /// created is seen through `goal_cards_relevant_to_session` and read as
    /// undispatched.
    #[tokio::test]
    async fn r58a_a_seeded_ready_card_reads_as_undispatched_through_the_session_scope() {
        let pool = crate::session::SessionManager::instance()
            .pool_clone()
            .await
            .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let project = crate::projects::create_project(
            &pool,
            crate::projects::CreateProject {
                name: format!("r58a-seeded-{}", uuid::Uuid::now_v7()),
                root_path: Some(tmp.path().to_string_lossy().into_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        crate::cards::seed_goal_columns(&pool, &project.id)
            .await
            .unwrap();
        insert_session_goal_card(&pool, &project.id, "r58a-seeder", "ready", true, None).await;

        let session_id = format!("r58a-coordinator-{}", uuid::Uuid::now_v7());
        let cards = goal_cards_relevant_to_session(&pool, &session_id, tmp.path())
            .await
            .unwrap();
        assert_eq!(cards.len(), 1);
        assert!(has_undispatched_ready_root_goal(&cards));
    }
}
