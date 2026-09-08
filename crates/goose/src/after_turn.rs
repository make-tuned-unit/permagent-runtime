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
                    let succeeded = match kind {
                        Kind::Mutation => resp
                            .tool_result
                            .as_ref()
                            .is_ok_and(|result| result.is_error != Some(true)),
                        Kind::Verify => resp.tool_result.as_ref().is_ok_and(
                            crate::agents::platform_extensions::developer::verify::is_authoritative_pass,
                        ),
                    };
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
    /// directly against a scratch repo.
    async fn trivial_skip_reason(session_id: &str) -> Option<String> {
        if !trivial_skip_enabled_by_env() {
            return None;
        }
        let manager = crate::session::SessionManager::instance();
        let session = manager.get_session(session_id, false).await.ok()?;
        trivial_skip_reason_for_dir(&session.working_dir).await
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

        // E3: what the injected text will SAY, resolved through the same
        // precedence the actual `delegate` dispatch uses — reusing `derived_map`
        // (already fetched above) so this costs no extra probe, unlike calling
        // `delegate_routing_live` a second time.
        let author = worker.as_ref().map(|(p, m)| (p.as_str(), m.as_str()));
        let routing = crate::cost_router::delegate_routing(
            Some(crate::cost_router::WorkflowRole::Review),
            &derived_map,
            None,
            author,
        );
        let dispatch = crate::cost_router::reviewer_dispatch(
            author.map(|(p, _)| p),
            author.map(|(_, m)| m),
            routing.role_model.clone(),
        );
        let actual_route = actual_reviewer_route(&dispatch, author);

        // Same-scope spend the goal path uses: the AUTHOR session's own running
        // cost against both ceilings (there is no separate "task" scope for an
        // interactive session, so — like `spend_for_goal` — one figure serves
        // both budget scopes `reviewer_spend_gate` checks).
        let spent = session.accumulated_cost_usd.unwrap_or(0.0);
        let cfg = crate::cost_router::budget::load_budget_config();
        let verdict = crate::cost_router::budget_verdict(spent, spent, &cfg);
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
/// route to — resolved through the IDENTICAL precedence
/// `summon::resolve_provider` applies for `worker_persona: "reviewer"`
/// (`cost_router::delegate_routing` + `cost_router::reviewer_dispatch`), not
/// the independent `select_reviewer` pick used for capability/spend gating.
///
/// E3 / defect 1a: `select_reviewer` has no notion of `PERMAGENT_PACK_*`
/// pins, so it can promise a genuinely cross-family model while the actual
/// dispatch — which DOES honour a `PERMAGENT_PACK_HARD_*` pin, at a higher
/// precedence than cross-family escalation (`delegate.rs`'s table, level 4
/// over level 5) — lands the reviewer on the author's own family. Lane A's
/// ledger showed this 3/3 times the reviewer actually ran. `same_family` is
/// `true` whenever the real dispatch will not give an independent,
/// different-family review — either because the resolved model shares the
/// author's family, or because nothing routes it anywhere but the session
/// model — so the injected text can say so plainly instead of promising
/// independence it will not get.
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
async fn changed_lines(working_dir: &std::path::Path) -> usize {
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReviewerLastOutcome {
    /// No `delegate(reviewer)` call at all after `after_position`.
    NotCalled,
    /// A call ran and returned real, non-empty content — whatever verdict it
    /// carries (or fails to parse), something was actually reviewed.
    Answered,
    /// A call ran but its result was empty or an error: nothing was actually
    /// reviewed despite the call happening.
    Empty,
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
                let is_empty = match response.tool_result.as_ref() {
                    Ok(result) => {
                        result.is_error == Some(true)
                            || result
                                .content
                                .iter()
                                .filter_map(|c| c.as_text().map(|t| t.text.trim().to_string()))
                                .collect::<String>()
                                .is_empty()
                    }
                    Err(_) => true,
                };
                outcome = if is_empty {
                    ReviewerLastOutcome::Empty
                } else {
                    ReviewerLastOutcome::Answered
                };
            }
        }
    }
    outcome
}

/// Injected when [`last_reviewer_outcome`] finds an empty/errored reviewer
/// call: names the concrete problem (nothing was actually reviewed) and asks
/// for exactly one thing, rather than repeating the full original ask.
const REVIEWER_EMPTY_RETRY_TEXT: &str =
    "The independent reviewer call returned no answer (an empty or errored response) — nothing \
     was actually reviewed. Call the `delegate` tool again with `worker_persona: \"reviewer\"` \
     and report its verdict before ending the turn.";

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

/// Pure: what the mandate does once mutation/delegation/hold state and reviewer
/// availability are already known. See [`ReviewerMandate::after_turn`] for the
/// live gating order this mirrors — `mutated`, then `delegation_already_ran`,
/// then `ask_state`, and ONLY THEN `availability`, so a turn that never
/// needed a reviewer (or already got one, or was already asked once) never has
/// to have `availability` computed at all.
fn decide(
    mutated: bool,
    reviewer_approved: bool,
    ask_state: AskState,
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
        if last_reviewer_outcome(ctx.messages, last_mutation) == ReviewerLastOutcome::Empty {
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

        let availability = Self::assess(ctx.session_id).await;
        decide(true, reviewer_approved, ask_state, &availability)
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
        match decide(true, false, AskState::NotAsked, &availability) {
            AfterTurnAction::Continue { inject } => {
                assert!(inject.contains("delegate"), "inject was {inject:?}");
                let lower = inject.to_ascii_lowercase();
                assert!(lower.contains("worker_persona"), "{inject:?}");
                assert!(lower.contains("reviewer"), "{inject:?}");
            }
            other => panic!("expected Continue, got {other:?}"),
        }
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
        match decide(true, false, AskState::NotAsked, &availability) {
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
        match decide(true, false, AskState::NotAsked, &availability) {
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

    /// Pins the actual resolution path (`delegate_routing` + `reviewer_dispatch`)
    /// against a live `PERMAGENT_PACK_HARD_*` pin equal to the author's own
    /// model — the exact shape observed 3/3 in Lane A's ledger — and confirms
    /// [`actual_reviewer_route`] reports it honestly.
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
        let dispatch = crate::cost_router::reviewer_dispatch(
            Some(author_provider),
            Some(author_model),
            routing.role_model.clone(),
        );
        let route = actual_reviewer_route(&dispatch, Some((author_provider, author_model)));
        assert!(
            route.same_family,
            "a HARD pack pinned to the author's own model must resolve same-family"
        );
        assert_eq!(route.label, format!("{author_provider}/{author_model}"));

        let text = reviewer_inject_text(&ready_pick(), &route);
        assert!(
            !text.contains("a different model from yours"),
            "text was {text:?}"
        );
    }
}
