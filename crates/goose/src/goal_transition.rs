//! Goal transition guard — the sole legal mutator of goal state.
//!
//! Every goal-state change in the daemon funnels through this module:
//! [`advance_goal_checked`] for the five legal state-machine actions, plus
//! the audited system paths [`park_goal`] and [`requeue_goal`], and the
//! Tier-2-gated [`delete_goal_checked`].
//!
//! Each checked advance runs as a single SQLite transaction:
//! re-read → `validate_transition` → `risk_policy` tier lookup → tier gate →
//! write → audit append. Tier-1/Tier-2 transitions demand a
//! [`DecisionProof`](crate::decisions::DecisionProof) — unforgeable outside
//! the decisions module (SafeBrain pattern) — so no handler, including the
//! orchestrator's own tools, can perform a gated transition without an
//! answered decision.
//!
//! Budgets (S4) also live here: attempt/token/wallclock caps stored under the
//! protected `budget` metadata key; exhaustion emits a `kind='unblock'`
//! decision and parks the goal — never a silent retry.

use sqlx::{Pool, Row, Sqlite};
use std::collections::HashSet;

use crate::decisions::{self, DecisionProof};
use crate::goal_state::{validate_transition, GoalAction, GoalState};

/// Default attempt cap when a goal carries no explicit budget
/// (replaces the former hardcoded MAX_GOAL_ATTEMPTS comparisons).
pub const DEFAULT_ATTEMPT_CAP: u64 = 3;

/// Metadata keys that only this guard may write on goal cards.
///
/// `depends_on` (#251): the roadmap dependency graph must stay topologically
/// valid, so edits go through [`set_goal_dependencies`] /
/// [`detach_goal_from_dependents`] (validated + audited), never a raw
/// metadata PATCH.
///
/// `auto_approve` (#252): the per-goal Review-skip opt-in changes the goal's
/// risk posture, so it may only be set through the audited
/// [`set_goal_auto_approve`] path (the user-facing HTTP toggle) — never by a
/// raw metadata PATCH, which the orchestrator's own card tools can reach
/// (S5: the orchestrator must not be able to loosen its own approval gate).
pub const PROTECTED_GOAL_METADATA_KEYS: &[&str] = &[
    HEADLESS_DISPATCH_BUDGET_KEY,
    "goal_state",
    "needs_human_attention",
    "attempt_count",
    "last_error",
    "dispatch_claim_id",
    "budget",
    "completed_at",
    "depends_on",
    "auto_approve",
    // Y16c (harness loop, G19 MED): which worker session's branch a human
    // reviewer explicitly rejected. `prior_goal_branch_tip`
    // (orchestrator.rs) reads this to refuse continuing a re-dispatch from
    // that exact branch; a rejected worker must not be able to clear its
    // own rejection marker via the agent-facing metadata PATCH tool.
    "goal_branch_rejected_session_id",
    // X-dag-1 (harness loop, coding-harness excellence iteration): which
    // session created this roadmap card, stamped once at materialization.
    // Protected so a mutating worker cannot launder a card into (or out of)
    // "this session's active roadmap" after the fact.
    "created_by_session_id",
    // Program registration and transition claims are authoritative roadmap
    // bindings. They may only be written by the program bridge's validated
    // transaction, never by card CRUD or an agent tool.
    "program",
    "program_receipts",
    "program_transition",
    // Lane R18g: how many times a headless run has auto-answered this
    // goal's tier-1 `unblock` decision (`decide_goal_hold`'s Park, a
    // verify/commit-gap hold — never a jail/approval/safety refusal, which
    // never reaches `decide_goal_hold` at all) and resumed it without a
    // human. Protected so a worker cannot reset its own bounded-resume
    // counter via the agent-facing metadata PATCH tool.
    "headless_unblock_count",
    // Lane R20b (G25 MED): how many times `maybe_salvage_committed_work`
    // has attempted to verify/review/land this goal's committed work before
    // a budget/wall park. Protected so a worker cannot reset its own
    // one-salvage-per-goal counter via the agent-facing metadata PATCH tool
    // — same reasoning as `headless_unblock_count` above.
    "salvage_attempted",
    // Lane R21a (A21 Q1): whether this goal's acceptance is a code change
    // (`true`, the default) or a report/review with no change expected
    // (`false`) — decided once by the planner, from its own acceptance
    // criteria, at roadmap-creation time (`hold_done::infer_expects_change`).
    // Protected so a worker cannot flip its own goal to "read-only" via the
    // agent-facing metadata PATCH tool and skip the commit-required rule.
    "expects_change",
    // Lane R21d (A23 Q2 #2): whether the most recent completed attempt made
    // zero progress, and how many CONSECUTIVE attempts have (see
    // `ZERO_PROGRESS_STREAK_METADATA_KEY`, `maybe_forgive_zero_progress_attempt`).
    // Protected so a worker cannot spend its own one-forgiveness-per-goal
    // allowance twice by resetting the streak via the agent-facing metadata
    // PATCH tool — same reasoning as `headless_unblock_count` above.
    "zero_progress",
    ZERO_PROGRESS_STREAK_METADATA_KEY,
    // QA 2026-09-25: the zero-progress circuit breaker's one tier
    // escalation per goal. Protected for the same reason as the streak: a
    // worker must not be able to buy itself a second escalation.
    ZERO_PROGRESS_ESCALATION_USED_METADATA_KEY,
    // Lane R22c-e (G26 MED-1): the receipt an independent cross-family
    // reviewer's approval leaves on a goal salvaged with no automated
    // check to lean on (`orchestrator::land_salvage_approved_by_reviewer`)
    // — its PRESENCE is what makes `auto_land_headless_goal` bypass the
    // ordinary verify-evidence gate and land the goal outright (see that
    // function's own doc comment). Protected so a worker cannot forge its
    // own reviewer sign-off and auto-land unverified, unreviewed work via
    // the agent-facing metadata PATCH tool — this key must only ever be
    // written by the trusted `advance_goal_checked` patch path at the
    // reviewer-approval call site, never by a raw metadata write.
    UNVERIFIED_REVIEWER_APPROVAL_METADATA_KEY,
    // Lane R26b (A25 analysis): stamped by R18g's headless unblock at the
    // moment it resumes a goal Held → Ready without itself redispatching
    // it — the drain's re-dispatch sweep reads this to prioritise the
    // goal (it already burned real attempts getting here) and the audit
    // trail reads it to tell "R18g's own resume, still Ready" apart from
    // an ordinary Ready goal waiting its turn. Protected so a worker
    // cannot forge or clear its own priority/attribution stamp via the
    // agent-facing metadata PATCH tool.
    "resumed_by_headless_unblock_at",
    // Lane R62d (A61 Miss 5): the "already satisfied at baseline" receipt
    // `maybe_hold_review` leaves when a goal's acceptance already passes on a
    // trunk a sibling landed. Its presence lets the landing path complete
    // the goal with nothing to land instead of parking it. Protected so a
    // worker cannot forge one and complete a goal it never verified.
    crate::cost_router::hold_done::ALREADY_SATISFIED_METADATA_KEY,
    // Lane R66a: how many times an independent review sent this goal back
    // to its worker, and when. The count bounds the send-back to once per
    // goal and the stamp gives the drain its one continuation, so a worker
    // cannot reset or forge either via the agent-facing metadata PATCH tool.
    // Written only through `return_to_ready`'s guarded patch.
    crate::agents::platform_extensions::orchestrator::REVIEW_SEND_BACK_COUNT_METADATA_KEY,
    crate::agents::platform_extensions::orchestrator::SENT_BACK_BY_REVIEW_AT_METADATA_KEY,
];

/// Lane R22c-e (G26 MED-1): see [`PROTECTED_GOAL_METADATA_KEYS`]'s own
/// entry for this key.
pub const UNVERIFIED_REVIEWER_APPROVAL_METADATA_KEY: &str = "unverified_reviewer_approval";

/// Lane R21d (A23 Q2 #2): card metadata key tracking CONSECUTIVE
/// zero-progress attempts. Absent/`0`: no streak (either no zero-progress
/// attempt yet, or the last one was followed by a productive attempt, which
/// resets it). `1`: one zero-progress attempt — a finished one spent the
/// goal's one forgiveness (`attempt_count` was decremented back for it); an
/// attempt that ended `Failed`/`TimedOut` counts without being forgiven
/// ([`record_unfinished_attempt_progress`]). `2`: a SECOND consecutive
/// zero-progress attempt, which counted normally against `attempt_count` —
/// [`park_goal_with_meta`] folds this into the park detail (`last_error`) so
/// a human sees the goal spent a real attempt on nothing, not just the
/// ordinary park reason. A human's answer to the goal's `unblock` clears it
/// ([`clear_zero_progress_on_human_unblock`]).
pub const ZERO_PROGRESS_STREAK_METADATA_KEY: &str = "zero_progress_streak";

/// QA 2026-09-25: stamped on the dispatch the zero-progress circuit breaker
/// (`orchestrator::zero_progress_breaker_verdict`) moved to a stronger worker
/// tier, once that worker has actually started (a failed spawn spends
/// nothing). At most one such escalation per goal: a goal that stays at zero
/// progress after it is parked, never escalated again — until a human's
/// answer to its `unblock` clears it
/// ([`clear_zero_progress_on_human_unblock`]).
pub const ZERO_PROGRESS_ESCALATION_USED_METADATA_KEY: &str = "zero_progress_escalation_used";

// ── Errors ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum GuardError {
    /// Card or column not found.
    NotFound(String),
    /// Request shape / state-machine violation.
    Invalid(String),
    /// Tier gate refused the transition (maps to HTTP 403).
    Denied(String),
    /// Database failure.
    Db(String),
}

impl std::fmt::Display for GuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(s) => write!(f, "Not found: {}", s),
            Self::Invalid(s) => write!(f, "Invalid: {}", s),
            Self::Denied(s) => write!(f, "Denied: {}", s),
            Self::Db(s) => write!(f, "Database error: {}", s),
        }
    }
}

impl From<GuardError> for String {
    fn from(e: GuardError) -> Self {
        e.to_string()
    }
}

fn db_err(e: sqlx::Error) -> GuardError {
    GuardError::Db(e.to_string())
}

/// Who the activity journal names for a transition. A human or policy actor
/// always wins. A system-driven move on a worker-owned goal is part of that
/// worker's run, so it is attributed to the worker named in `worker_key` — one
/// of the two fields `GET /api/agents/{id}/work` already joins that agent's
/// goals on, so the two halves of one work review cannot disagree. (`worker_key`
/// alone, not `assigned_to`: dispatch writes both, and only the metadata is in
/// hand here without a second read.)
/// The decision-audit actor is NOT changed by this: the audit answers "who was
/// authorized", the journal answers "whose work was this". They are different
/// questions and the audit chain's meaning must not shift to answer the second.
///
/// Callers pass the POST-patch metadata, so the Dispatch transition that first
/// writes `worker_key` is itself attributed to the worker it hands the goal to
/// — that row opens the worker's run and belongs in its record.
///
/// Nothing here invents an actor: an unowned goal moved by the system stays
/// `system`, and [`crate::activity_journal::Actor::resolve`] fails closed again
/// on anything it does not recognize.
fn journal_actor(
    audit_actor: &str,
    goal_metadata: &serde_json::Map<String, serde_json::Value>,
) -> String {
    if audit_actor != decisions::ACTOR_SYSTEM {
        return audit_actor.to_string();
    }
    goal_metadata
        .get("worker_key")
        .and_then(serde_json::Value::as_str)
        .filter(|worker| !worker.is_empty())
        .unwrap_or(decisions::ACTOR_SYSTEM)
        .to_string()
}

// ── Checked advance ─────────────────────────────────────────────────────────

/// The risk_policy action class governing each goal action.
pub fn action_class_for(action: GoalAction) -> &'static str {
    match action {
        GoalAction::Ready => "goal_ready",
        GoalAction::Dispatch => "goal_dispatch",
        GoalAction::Review => "goal_review",
        GoalAction::Approve => "goal_approve_standard",
        GoalAction::Reject => "goal_retry_within_budget",
        GoalAction::Cancel => "goal_cancel",
    }
}

/// Side payload applied atomically with a checked transition.
///
/// `metadata_patch` may touch protected keys — this guard IS the legal writer.
/// A JSON `null` value removes the key. Internal trusted-code API only; it is
/// never reachable from HTTP or MCP tool input.
#[derive(Debug, Default)]
pub struct TransitionEffects {
    pub review_notes: Option<String>,
    pub metadata_patch: serde_json::Map<String, serde_json::Value>,
    pub assigned_to: Option<String>,
    /// Native headless coordinator policy, never a tool argument.
    pub dispatch_budget: Option<HeadlessDispatchBudget>,
    pub attempt_claim: Option<crate::goal_attempts::ClaimInput>,
    pub expected_attempt: Option<crate::goal_candidates::ExpectedAttempt>,
}

pub const HEADLESS_DISPATCH_BUDGET_KEY: &str = "headless_dispatch_budget";

#[derive(Debug)]
pub struct HeadlessDispatchBudget {
    pub coordinator_session_id: String,
    pub automatic_limit: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct HeadlessDispatchUsage {
    coordinator_session_id: String,
    dispatch_count: u64,
    automatic_limit: u64,
    approved_extra_dispatches: u64,
}

fn scoped_dispatch_usage(
    metadata: &serde_json::Value,
    budget: &HeadlessDispatchBudget,
) -> Result<HeadlessDispatchUsage, String> {
    if budget.coordinator_session_id.is_empty() || budget.automatic_limit == 0 {
        return Err("Headless dispatch policy has no coordinator or initial allowance".into());
    }
    let prior = metadata
        .get(HEADLESS_DISPATCH_BUDGET_KEY)
        .map(|value| serde_json::from_value::<HeadlessDispatchUsage>(value.clone()))
        .transpose()
        .map_err(|_| "Headless dispatch budget evidence is unreadable")?;
    let mut usage = match prior {
        Some(usage) if usage.coordinator_session_id == budget.coordinator_session_id => usage,
        _ => HeadlessDispatchUsage {
            coordinator_session_id: budget.coordinator_session_id.clone(),
            dispatch_count: 0,
            automatic_limit: budget.automatic_limit,
            approved_extra_dispatches: 0,
        },
    };
    usage.automatic_limit = usage.automatic_limit.min(budget.automatic_limit);
    Ok(usage)
}

/// Cheap preflight; the checked transition repeats it under BEGIN IMMEDIATE.
pub fn check_headless_dispatch_budget(
    metadata: &serde_json::Value,
    budget: &HeadlessDispatchBudget,
) -> Result<(), String> {
    let usage = scoped_dispatch_usage(metadata, budget)?;
    let limit = usage
        .automatic_limit
        .saturating_add(usage.approved_extra_dispatches);
    if usage.dispatch_count >= limit {
        return Err(format!("Headless dispatch allowance exhausted: {} of {} attempts claimed; an explicit approved unblock is required", usage.dispatch_count, limit));
    }
    Ok(())
}

fn claim_headless_dispatch_budget(
    metadata: &mut serde_json::Map<String, serde_json::Value>,
    budget: &HeadlessDispatchBudget,
) -> Result<(), GuardError> {
    let value = serde_json::Value::Object(metadata.clone());
    check_headless_dispatch_budget(&value, budget).map_err(GuardError::Invalid)?;
    let mut usage = scoped_dispatch_usage(&value, budget).map_err(GuardError::Invalid)?;
    usage.dispatch_count = usage.dispatch_count.saturating_add(1);
    metadata.insert(
        HEADLESS_DISPATCH_BUDGET_KEY.into(),
        serde_json::to_value(usage).map_err(|e| GuardError::Invalid(e.to_string()))?,
    );
    Ok(())
}

/// Called only from the verified user's unblock decision effect.
pub(crate) fn extend_headless_dispatch_budget(
    metadata: &serde_json::Value,
    patch: &mut serde_json::Map<String, serde_json::Value>,
) -> Result<bool, GuardError> {
    let Some(value) = metadata.get(HEADLESS_DISPATCH_BUDGET_KEY) else {
        return Ok(false);
    };
    let mut usage: HeadlessDispatchUsage = serde_json::from_value(value.clone()).map_err(|_| {
        GuardError::Invalid("Headless dispatch budget evidence is unreadable".into())
    })?;
    usage.approved_extra_dispatches = usage
        .approved_extra_dispatches
        .saturating_add(DEFAULT_ATTEMPT_CAP);
    patch.insert(
        HEADLESS_DISPATCH_BUDGET_KEY.into(),
        serde_json::to_value(usage).map_err(|e| GuardError::Invalid(e.to_string()))?,
    );
    Ok(true)
}

struct GoalRow {
    project_id: String,
    card_type: String,
    column_id: String,
    metadata: serde_json::Map<String, serde_json::Value>,
}

/// The durable part of a checked transition.  Callers that compose several
/// card writes in one transaction use this record to emit live events only
/// after the enclosing transaction commits.
#[derive(Debug)]
pub(crate) struct GoalTransitionRecord {
    pub(crate) card_id: String,
    pub(crate) project_id: String,
    pub(crate) current_state: GoalState,
    pub(crate) new_state: GoalState,
    pub(crate) journal_actor: String,
}

async fn read_goal_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    card_id: &str,
) -> Result<GoalRow, GuardError> {
    let row = sqlx::query(
        "SELECT project_id, card_type, column_id, metadata_json FROM cards WHERE id = ?",
    )
    .bind(card_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db_err)?
    .ok_or_else(|| GuardError::NotFound(format!("Card '{}' not found", card_id)))?;

    let meta_str: String = row.get("metadata_json");
    let metadata = serde_json::from_str::<serde_json::Value>(&meta_str)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();

    Ok(GoalRow {
        project_id: row.get("project_id"),
        card_type: row.get("card_type"),
        column_id: row.get("column_id"),
        metadata,
    })
}

async fn state_binding_of_column_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    column_id: &str,
) -> Result<Option<String>, GuardError> {
    sqlx::query_scalar::<_, Option<String>>("SELECT state_binding FROM board_columns WHERE id = ?")
        .bind(column_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(db_err)?
        .ok_or_else(|| GuardError::NotFound(format!("Column '{}' not found", column_id)))
}

async fn goal_column_id_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    project_id: &str,
    state_binding: &str,
) -> Result<String, GuardError> {
    sqlx::query_scalar::<_, String>(
        "SELECT id FROM board_columns WHERE project_id = ? AND state_binding = ?",
    )
    .bind(project_id)
    .bind(state_binding)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db_err)?
    .ok_or_else(|| {
        GuardError::NotFound(format!(
            "Column for state '{}' not found in project",
            state_binding
        ))
    })
}

async fn next_position_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    column_id: &str,
) -> Result<i32, GuardError> {
    let max: Option<i32> = sqlx::query_scalar(
        "SELECT MAX(position) FROM cards WHERE column_id = ? AND archived_at IS NULL",
    )
    .bind(column_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(db_err)?;
    Ok(max.unwrap_or(-1) + 1)
}

fn apply_patch(
    meta: &mut serde_json::Map<String, serde_json::Value>,
    patch: &serde_json::Map<String, serde_json::Value>,
) {
    for (k, v) in patch {
        if v.is_null() {
            meta.remove(k);
        } else {
            meta.insert(k.clone(), v.clone());
        }
    }
}

/// Validate a proof against a tier gate and the target card/action.
///
/// `auto_land_allowed` / `calling_session_id` (harness loop, Lane V2 —
/// the user's ruling, HARNESS_LOOP_LOG.md "JESSE RULING (2026-09-08 23:55):
/// Tier-1 landing → option 3"): the ONLY way [`decisions::ACTOR_AUTO_LAUNCH`]
/// can ever satisfy the Tier-1 gate. `auto_land_allowed` is an explicit bool
/// the CALLER passes — never read from an env var or global inside this
/// function — so every existing call site keeps it `false` (byte-identical
/// pre-V2 behavior) unless it deliberately opts in. Even when `true`, the
/// proof must ALSO be a single-use `approve_review` decision (the only kind
/// [`decisions::auto_approve_headless_goal_review`] ever mints under this
/// actor) whose `minting_session_id` matches `calling_session_id` — the same
/// minting-session binding Lane R2 added for the roadmap-approval proof
/// (`RoadmapApprovalPayload::minting_session_id`), reused here rather than
/// forked, so an auto-land proof minted for one headless session can never be
/// consumed by presenting it through another. Tier 2 (the `_ =>` arm below)
/// is deliberately untouched: `ACTOR_AUTO_LAUNCH` never satisfies it,
/// whatever `auto_land_allowed` says — landing stays the only Tier-1 action
/// this actor can ever authorize.
fn check_proof(
    tier: i64,
    proof: Option<&DecisionProof>,
    card_id: &str,
    action: GoalAction,
    auto_land_allowed: bool,
    calling_session_id: Option<&str>,
) -> Result<(), GuardError> {
    if let Some(p) = proof {
        if let Some(gid) = p.goal_id() {
            if gid != card_id {
                return Err(GuardError::Denied(format!(
                    "Decision {} is for goal '{}', not '{}'",
                    p.decision_id(),
                    gid,
                    card_id
                )));
            }
        }
        match action {
            GoalAction::Approve if p.answer() != "approve" => {
                return Err(GuardError::Denied(format!(
                    "Action 'approve' requires a decision answered 'approve' (got '{}')",
                    p.answer()
                )));
            }
            GoalAction::Reject if p.answer() != "reject" => {
                return Err(GuardError::Denied(format!(
                    "Action 'reject' requires a decision answered 'reject' (got '{}')",
                    p.answer()
                )));
            }
            _ => {}
        }
    }

    match tier {
        0 => Ok(()),
        1 => match proof {
            Some(p)
                if p.acted_by() == decisions::ACTOR_JESSE
                    || p.acted_by() == decisions::ACTOR_HENRY =>
            {
                Ok(())
            }
            Some(p) if p.acted_by() == decisions::ACTOR_AUTO_LAUNCH => {
                if !auto_land_allowed {
                    return Err(GuardError::Denied(
                        "headless auto-land is off (PERMAGENT_HEADLESS_AUTOLAND)".to_string(),
                    ));
                }
                if p.kind() != "approve_review" {
                    return Err(GuardError::Denied(format!(
                        "headless auto-land only authorizes 'approve_review' decisions, not \
                         '{}'",
                        p.kind()
                    )));
                }
                match (p.minting_session_id(), calling_session_id) {
                    (Some(minted), Some(calling)) if minted == calling => Ok(()),
                    _ => Err(GuardError::Denied(
                        "headless auto-land proof is not bound to the calling session".to_string(),
                    )),
                }
            }
            Some(p) => Err(GuardError::Denied(format!(
                "Tier-1 transition requires a decision answered by 'jesse' or 'henry-policy' \
                 (got '{}')",
                p.acted_by()
            ))),
            None => Err(GuardError::Denied(
                "Tier-1 transition requires an answered decision; none was provided. \
                 Answer the corresponding decision in the inbox."
                    .to_string(),
            )),
        },
        _ => match proof {
            Some(p) if p.acted_by() == decisions::ACTOR_JESSE => Ok(()),
            Some(p) => Err(GuardError::Denied(format!(
                "Tier-2 transition requires a decision answered by 'jesse' (got '{}')",
                p.acted_by()
            ))),
            None => Err(GuardError::Denied(
                "Tier-2 transition requires a decision answered by 'jesse'; none was provided."
                    .to_string(),
            )),
        },
    }
}

/// The sole legal mutator of goal lifecycle state.
///
/// Single transaction: re-read → validate_transition → risk_policy tier
/// lookup → tier gate (proof) → write (metadata + column) → audit append.
/// The proof is consumed by value: one answered decision authorizes at most
/// one transition.
///
/// Thin wrapper over [`advance_goal_checked_with_auto_land`] with headless
/// auto-land off — every pre-existing caller (and there are many, across
/// `goose-server` and `goose`) keeps this exact signature and this exact
/// behavior; none of them needed to change for Lane V2.
pub async fn advance_goal_checked(
    pool: &Pool<Sqlite>,
    card_id: &str,
    action: GoalAction,
    actor: &str,
    proof: Option<DecisionProof>,
    effects: TransitionEffects,
) -> Result<GoalState, GuardError> {
    advance_goal_checked_with_auto_land(pool, card_id, action, actor, proof, effects, false, None)
        .await
}

/// [`advance_goal_checked`], plus the two arguments [`check_proof`] needs to
/// ever accept [`decisions::ACTOR_AUTO_LAUNCH`] at Tier 1 (harness loop, Lane
/// V2 — the user's ruling, HARNESS_LOOP_LOG.md "JESSE RULING (2026-09-08
/// 23:55): Tier-1 landing → option 3"). `auto_land_allowed: false` (what
/// [`advance_goal_checked`] always passes) makes this byte-identical to the
/// pre-V2 function; the ONLY caller that ever passes `true` is
/// `decisions_effects::apply_decision_effect_with_auto_land`'s
/// `approve_review`/"approve" arm, itself reached only from
/// `orchestrator::auto_land_headless_goal`, itself reached only when a
/// headless Auto dispatch policy has `PERMAGENT_HEADLESS_AUTOLAND` on.
#[allow(clippy::too_many_arguments)]
pub async fn advance_goal_checked_with_auto_land(
    pool: &Pool<Sqlite>,
    card_id: &str,
    action: GoalAction,
    actor: &str,
    proof: Option<DecisionProof>,
    effects: TransitionEffects,
    auto_land_allowed: bool,
    calling_session_id: Option<&str>,
) -> Result<GoalState, GuardError> {
    // BEGIN IMMEDIATE, not the default BEGIN DEFERRED. This transaction reads the
    // goal + its column binding + the risk tier, THEN upgrades to a write
    // (UPDATE cards + audit INSERT). Under DEFERRED the read acquires only a
    // snapshot; if a concurrent writer commits between that snapshot and the
    // UPDATE — e.g. the activity-journal consumer reacting to the very
    // decision_resolved / goal_state_changed events this answer→effect path emits
    // — SQLite raises a BUSY lock-upgrade (stale-snapshot) conflict that
    // `busy_timeout` does NOT retry, surfacing as a spurious effect failure (the
    // Decision Inbox router-integration flake). Taking the write lock up front
    // lets `busy_timeout` serialize instead of erroring. Same idiom as
    // workspaces::seed_presets_if_empty and people::upsert_person.
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(db_err)?;

    let goal = read_goal_tx(&mut tx, card_id).await?;
    crate::goal_candidates::check_expected_tx(&mut tx, card_id, effects.expected_attempt.as_ref())
        .await
        .map_err(|e| GuardError::Denied(e.to_string()))?;
    if goal.card_type != "goal" {
        return Err(GuardError::Invalid(format!(
            "Card '{}' is type '{}', not 'goal'",
            card_id, goal.card_type
        )));
    }

    let binding = state_binding_of_column_tx(&mut tx, &goal.column_id).await?;
    let current_state = binding
        .as_deref()
        .and_then(GoalState::from_binding)
        .ok_or_else(|| {
            GuardError::Invalid(format!(
                "Card '{}' is in a column with no state_binding; goal cards must live in \
                 state-bound columns",
                card_id
            ))
        })?;

    let new_state = validate_transition(current_state, action)
        .map_err(|e| GuardError::Invalid(e.to_string()))?;

    // Risk tier (fail-closed for unknown classes).
    let action_class = action_class_for(action);
    let tier: i64 = sqlx::query_scalar("SELECT tier FROM risk_policy WHERE action_class = ?")
        .bind(action_class)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?
        .unwrap_or(2);

    check_proof(
        tier,
        proof.as_ref(),
        card_id,
        action,
        auto_land_allowed,
        calling_session_id,
    )?;

    let (decision_id, audit_actor) = match proof.as_ref() {
        Some(p) => (p.decision_id().to_string(), p.acted_by().to_string()),
        None => ("none".to_string(), actor.to_string()),
    };

    // #504: capture the dispatched worker's run id (== goal worktree dir name)
    // before `meta` is consumed, so a terminal transition can reap the worktree
    // post-commit. Only meaningful for terminal states; `None` otherwise.
    let terminal_run_id = if matches!(new_state, GoalState::Complete | GoalState::Cancelled) {
        goal.metadata
            .get("worker_session_id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    } else {
        None
    };

    if let Some(input) = effects.attempt_claim.as_ref() {
        if action != GoalAction::Dispatch {
            return Err(GuardError::Invalid(
                "Attempt claim requires Dispatch".into(),
            ));
        }
        crate::goal_attempts::claim_tx(&mut tx, card_id, &goal.project_id, input)
            .await
            .map_err(|e| GuardError::Db(e.to_string()))?;
    }

    // Build new metadata.
    let mut meta = goal.metadata;
    apply_patch(&mut meta, &effects.metadata_patch);
    if action == GoalAction::Dispatch {
        if let Some(budget) = effects.dispatch_budget.as_ref() {
            claim_headless_dispatch_budget(&mut meta, budget)?;
        }
    }
    if let Some(ref notes) = effects.review_notes {
        meta.insert(
            "review_notes".to_string(),
            serde_json::Value::String(notes.clone()),
        );
    }
    meta.insert(
        "goal_state".to_string(),
        serde_json::Value::String(new_state.binding().to_string()),
    );
    let journal_actor = journal_actor(&audit_actor, &meta);
    let meta_str = serde_json::to_string(&serde_json::Value::Object(meta))
        .map_err(|e| GuardError::Db(e.to_string()))?;

    let target_col = goal_column_id_tx(&mut tx, &goal.project_id, new_state.binding()).await?;
    let position = next_position_tx(&mut tx, &target_col).await?;

    if let Some(ref assignee) = effects.assigned_to {
        sqlx::query("UPDATE cards SET assigned_to = ? WHERE id = ?")
            .bind(assignee)
            .bind(card_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }

    sqlx::query("UPDATE cards SET metadata_json = ?, column_id = ?, position = ? WHERE id = ?")
        .bind(&meta_str)
        .bind(&target_col)
        .bind(position)
        .bind(card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

    decisions::append_audit_tx(
        &mut tx,
        &decision_id,
        Some(card_id),
        &audit_actor,
        tier,
        &format!("transition:{}", action),
        None,
    )
    .await
    .map_err(GuardError::Db)?;

    tx.commit().await.map_err(db_err)?;

    // Live goal-state push (#454-follow): every transition emits so the
    // dashboard surfaces and Kanban board react within a frame instead of
    // waiting on a poll or a full reload.
    crate::events::emit(crate::events::goal_state_changed(
        card_id,
        Some(&goal.project_id),
        Some(current_state.binding()),
        new_state.binding(),
        &journal_actor,
    ));

    // #504: a goal that just reached a terminal state no longer needs its
    // dispatch worktree — reap it. Cancelled allows reaping unpushed work (the
    // user discarded it); Complete keeps unpushed/unprovable worktrees so
    // unreviewed commits are never destroyed. Best-effort on a detached task:
    // the reap must never block or fail the transition (a lingering dir is the
    // acceptable status quo; a broken transition is not).
    if let Some(run_id) = terminal_run_id {
        let pool = pool.clone();
        let project_id = goal.project_id.clone();
        let allow_unpushed = matches!(new_state, GoalState::Cancelled);
        tokio::spawn(async move {
            crate::agents::platform_extensions::goal_engine::reap_terminal_goal_worktree(
                &pool,
                &project_id,
                &run_id,
                allow_unpushed,
            )
            .await;
        });
    }

    // Feed the Brain: the goal's latest outcome, keyed so repeated
    // transitions upsert one evolving memory instead of piling up.
    crate::brain_feed::feed_for_project(
        pool,
        crate::brain_feed::FeedSource::Goal,
        crate::brain_feed::feed_key("goal", card_id, "outcome"),
        &goal.project_id,
        format!(
            "Goal '{}' moved {:?} -> {:?} via action {:?} (actor {}).",
            card_id, current_state, new_state, action, actor
        ),
    )
    .await;

    Ok(new_state)
}

/// Apply a checked transition to an already-open transaction.  No commit or
/// event is performed here; the caller owns the transaction and must emit any
/// live updates after it commits.  This is used by roadmap materialization so
/// card creation, proposal consumption, and root promotion are one unit.
pub(crate) async fn advance_goal_checked_in_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    card_id: &str,
    action: GoalAction,
    actor: &str,
    proof: Option<DecisionProof>,
    effects: TransitionEffects,
) -> Result<GoalTransitionRecord, GuardError> {
    advance_goal_checked_in_tx_with_auto_land(
        tx, card_id, action, actor, proof, effects, false, None,
    )
    .await
}

pub(crate) async fn advance_goal_checked_in_tx_with_auto_land(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    card_id: &str,
    action: GoalAction,
    actor: &str,
    proof: Option<DecisionProof>,
    effects: TransitionEffects,
    auto_land_allowed: bool,
    calling_session_id: Option<&str>,
) -> Result<GoalTransitionRecord, GuardError> {
    let goal = read_goal_tx(tx, card_id).await?;
    crate::goal_candidates::check_expected_tx(tx, card_id, effects.expected_attempt.as_ref())
        .await
        .map_err(|e| GuardError::Denied(e.to_string()))?;
    if goal.card_type != "goal" {
        return Err(GuardError::Invalid(format!(
            "Card '{}' is type '{}', not 'goal'",
            card_id, goal.card_type
        )));
    }

    let binding = state_binding_of_column_tx(tx, &goal.column_id).await?;
    let current_state = binding
        .as_deref()
        .and_then(GoalState::from_binding)
        .ok_or_else(|| {
            GuardError::Invalid(format!(
                "Card '{}' is in a column with no state_binding; goal cards must live in \
                 state-bound columns",
                card_id
            ))
        })?;
    let new_state = validate_transition(current_state, action)
        .map_err(|e| GuardError::Invalid(e.to_string()))?;

    let tier: i64 = sqlx::query_scalar("SELECT tier FROM risk_policy WHERE action_class = ?")
        .bind(action_class_for(action))
        .fetch_optional(&mut **tx)
        .await
        .map_err(db_err)?
        .unwrap_or(2);
    check_proof(
        tier,
        proof.as_ref(),
        card_id,
        action,
        auto_land_allowed,
        calling_session_id,
    )?;

    let (decision_id, audit_actor) = match proof.as_ref() {
        Some(p) => (p.decision_id().to_string(), p.acted_by().to_string()),
        None => ("none".to_string(), actor.to_string()),
    };
    if let Some(input) = effects.attempt_claim.as_ref() {
        if action != GoalAction::Dispatch {
            return Err(GuardError::Invalid(
                "Attempt claim requires Dispatch".into(),
            ));
        }
        crate::goal_attempts::claim_tx(tx, card_id, &goal.project_id, input)
            .await
            .map_err(|e| GuardError::Db(e.to_string()))?;
    }
    let mut meta = goal.metadata;
    apply_patch(&mut meta, &effects.metadata_patch);
    if action == GoalAction::Dispatch {
        if let Some(budget) = effects.dispatch_budget.as_ref() {
            claim_headless_dispatch_budget(&mut meta, budget)?;
        }
    }
    if let Some(ref notes) = effects.review_notes {
        meta.insert(
            "review_notes".to_string(),
            serde_json::Value::String(notes.clone()),
        );
    }
    meta.insert(
        "goal_state".to_string(),
        serde_json::Value::String(new_state.binding().to_string()),
    );
    let journal_actor = journal_actor(&audit_actor, &meta);
    let meta_str = serde_json::to_string(&serde_json::Value::Object(meta))
        .map_err(|e| GuardError::Db(e.to_string()))?;
    let target_col = goal_column_id_tx(tx, &goal.project_id, new_state.binding()).await?;
    let position = next_position_tx(tx, &target_col).await?;

    if let Some(ref assignee) = effects.assigned_to {
        sqlx::query("UPDATE cards SET assigned_to = ? WHERE id = ?")
            .bind(assignee)
            .bind(card_id)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
    }
    sqlx::query("UPDATE cards SET metadata_json = ?, column_id = ?, position = ? WHERE id = ?")
        .bind(&meta_str)
        .bind(&target_col)
        .bind(position)
        .bind(card_id)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;

    decisions::append_audit_tx(
        tx,
        &decision_id,
        Some(card_id),
        &audit_actor,
        tier,
        &format!("transition:{}", action),
        None,
    )
    .await
    .map_err(GuardError::Db)?;

    Ok(GoalTransitionRecord {
        card_id: card_id.to_string(),
        project_id: goal.project_id,
        current_state,
        new_state,
        journal_actor,
    })
}

// ── Audited system paths (park / requeue / failure record) ─────────────────

/// Park a goal: move it to the Failed column (#250) with
/// `needs_human_attention=true` and the blocking reason in `last_error`.
/// System action (audited, tier 0 semantics: parking restricts, never
/// advances). Before #250 exhausted goals were re-pooled into Triage, where a
/// failure was indistinguishable from a fresh goal; Failed makes exhaustion
/// visible on the board. The retry path is unchanged: answering the goal's
/// `unblock` decision unparks it (Failed → Ready through the guard).
pub async fn park_goal(
    pool: &Pool<Sqlite>,
    card_id: &str,
    actor: &str,
    reason: &str,
) -> Result<(), GuardError> {
    park_goal_with_meta(pool, card_id, actor, reason, &[]).await
}

/// [`park_goal`], additionally merging `extra_meta` into the card's metadata
/// inside the SAME transaction.
///
/// Exists because the attempt that trips the cap was never written down.
/// `requeue_goal` is the only path that persists `attempt_count`, so the final
/// increment — computed in flight, compared against the cap, and quoted in the
/// unblock decision as "3/3 attempts" — died with the local variable. Live
/// cards read `attempt_count: 2` while their own decision text said 3, so
/// anything counting attempts off the card (a reconciler, an amnesty pass, a
/// budget report) undercounted every parked goal by exactly one.
pub async fn park_goal_with_meta(
    pool: &Pool<Sqlite>,
    card_id: &str,
    actor: &str,
    reason: &str,
    extra_meta: &[(&str, serde_json::Value)],
) -> Result<(), GuardError> {
    park_goal_with_meta_filtered(pool, card_id, actor, reason, extra_meta, None, None)
        .await
        .map(|_| ())
}

pub(crate) async fn park_goal_for_attempt(
    pool: &Pool<Sqlite>,
    card_id: &str,
    actor: &str,
    reason: &str,
    expected: Option<&crate::goal_candidates::ExpectedAttempt>,
) -> Result<(), GuardError> {
    park_goal_with_meta_filtered(pool, card_id, actor, reason, &[], None, expected)
        .await
        .map(|_| ())
}

/// Park only a still-Ready, still-exhausted native authorization. A racing
/// authorized dispatch/unblock must not be cancelled by a stale preflight.
pub(crate) async fn park_exhausted_headless_dispatch(
    pool: &Pool<Sqlite>,
    card_id: &str,
    budget: &HeadlessDispatchBudget,
    reason: &str,
) -> Result<bool, GuardError> {
    park_goal_with_meta_filtered(
        pool,
        card_id,
        decisions::ACTOR_SYSTEM,
        reason,
        &[],
        Some(budget),
        None,
    )
    .await
}

async fn park_goal_with_meta_filtered(
    pool: &Pool<Sqlite>,
    card_id: &str,
    actor: &str,
    reason: &str,
    extra_meta: &[(&str, serde_json::Value)],
    exhausted_budget: Option<&HeadlessDispatchBudget>,
    expected_attempt: Option<&crate::goal_candidates::ExpectedAttempt>,
) -> Result<bool, GuardError> {
    // BEGIN IMMEDIATE: this guard reads the goal row / audit-chain head before
    // upgrading to a write; taking the write lock up front avoids the un-retryable
    // BUSY lock-upgrade a concurrent writer would trigger (see advance_goal_checked).
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(db_err)?;

    let project_id = park_goal_with_meta_filtered_tx(
        &mut tx,
        card_id,
        actor,
        reason,
        extra_meta,
        exhausted_budget,
        expected_attempt,
    )
    .await?;
    tx.commit().await.map_err(db_err)?;
    if let Some((project_id, journal_actor)) = project_id {
        crate::events::emit(crate::events::goal_state_changed(
            card_id,
            Some(&project_id),
            None,
            "failed",
            &journal_actor,
        ));
        Ok(true)
    } else {
        Ok(false)
    }
}

async fn park_goal_with_meta_filtered_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    card_id: &str,
    actor: &str,
    reason: &str,
    extra_meta: &[(&str, serde_json::Value)],
    exhausted_budget: Option<&HeadlessDispatchBudget>,
    expected_attempt: Option<&crate::goal_candidates::ExpectedAttempt>,
) -> Result<Option<(String, String)>, GuardError> {
    crate::goal_candidates::check_expected_tx(tx, card_id, expected_attempt)
        .await
        .map_err(|e| GuardError::Denied(e.to_string()))?;
    let goal = read_goal_tx(tx, card_id).await?;
    if goal.card_type != "goal" {
        return Err(GuardError::Invalid(format!(
            "Card '{}' is not a goal",
            card_id
        )));
    }

    if let Some(budget) = exhausted_budget {
        let binding = state_binding_of_column_tx(tx, &goal.column_id).await?;
        if binding.as_deref() != Some("ready")
            || check_headless_dispatch_budget(
                &serde_json::Value::Object(goal.metadata.clone()),
                budget,
            )
            .is_ok()
        {
            return Ok(None);
        }
    }

    let journal_actor = journal_actor(actor, &goal.metadata);
    let mut meta = goal.metadata;
    // Lane R21d (A23 Q2 #2): every park path (Hold-arm Park, budget
    // exhaustion, wall-budget refusal) converges here — fold a SECOND
    // consecutive zero-progress attempt's streak into the park detail so a
    // human reading `last_error` sees the goal spent a real attempt on
    // nothing, not just the ordinary park reason. A single forgiven
    // zero-progress attempt (`streak == 1`) is not noted here — it never
    // counted against the goal, so it left nothing to explain.
    let annotated_reason = match meta
        .get(ZERO_PROGRESS_STREAK_METADATA_KEY)
        .and_then(|v| v.as_u64())
    {
        Some(streak) if streak >= 2 => format!("{reason}; zero_progress_streak={streak}"),
        _ => reason.to_string(),
    };
    meta.insert(
        "goal_state".to_string(),
        serde_json::Value::String("failed".to_string()),
    );
    meta.insert(
        "needs_human_attention".to_string(),
        serde_json::Value::Bool(true),
    );
    meta.insert(
        "last_error".to_string(),
        serde_json::Value::String(annotated_reason),
    );
    for (key, value) in extra_meta {
        meta.insert((*key).to_string(), value.clone());
    }
    let meta_str = serde_json::to_string(&serde_json::Value::Object(meta))
        .map_err(|e| GuardError::Db(e.to_string()))?;

    let failed_col = goal_column_id_tx(tx, &goal.project_id, "failed").await?;
    let position = next_position_tx(tx, &failed_col).await?;

    sqlx::query("UPDATE cards SET metadata_json = ?, column_id = ?, position = ? WHERE id = ?")
        .bind(&meta_str)
        .bind(&failed_col)
        .bind(position)
        .bind(card_id)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;

    decisions::append_audit_tx(tx, "none", Some(card_id), actor, 0, "park", None)
        .await
        .map_err(GuardError::Db)?;

    Ok(Some((goal.project_id, journal_actor)))
}

/// F1c (harness loop, Lane F1 2026-09-10, wired by I1 as a gate fix):
/// annotate a goal card with abandoned work `handle_goal_status`'s R3
/// misfire guard discovered — a session it was about to treat as "exited
/// with no error, safe to redispatch" turns out to have a real commit
/// beyond baseline in its own worktree. Sets `abandoned_work` and flags
/// `needs_human_attention` with a reason, WITHOUT moving the card's column
/// — the goal stays `in_progress`; a human (or a future automated pass) is
/// the one who decides what to do with the discovered commit, not this
/// guard. `needs_human_attention` is a protected metadata key
/// (`PROTECTED_GOAL_METADATA_KEYS`), so — same reasoning as every other
/// function in this module — `cards::merge_card_metadata` refuses this
/// write outright; this is the sanctioned mutator for exactly this case.
pub async fn flag_abandoned_work(
    pool: &Pool<Sqlite>,
    card_id: &str,
    abandoned_work: &str,
    reason: &str,
) -> Result<(), GuardError> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(db_err)?;
    let goal = read_goal_tx(&mut tx, card_id).await?;
    if goal.card_type != "goal" {
        return Err(GuardError::Invalid(format!(
            "Card '{}' is not a goal",
            card_id
        )));
    }
    let mut meta = goal.metadata;
    meta.insert(
        "abandoned_work".to_string(),
        serde_json::Value::String(abandoned_work.to_string()),
    );
    meta.insert(
        "needs_human_attention".to_string(),
        serde_json::Value::Bool(true),
    );
    meta.insert(
        "needs_human_attention_reason".to_string(),
        serde_json::Value::String(reason.to_string()),
    );
    let meta_str = serde_json::to_string(&serde_json::Value::Object(meta))
        .map_err(|e| GuardError::Db(e.to_string()))?;

    sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
        .bind(&meta_str)
        .bind(card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

    tx.commit().await.map_err(db_err)?;
    Ok(())
}

/// Persist an explicit, human-readable refusal reason onto the card when
/// [`requeue_goal_with_meta`]'s guard refuses (E2, harness loop Round E) —
/// the spec requires a refusal to be recorded on the card, not merely
/// returned as an `Err` a caller might drop. Merges into the existing
/// metadata rather than replacing it, and touches neither `goal_state` nor
/// the card's column — a refused requeue changes nothing about where the
/// card lives or what state it is in.
async fn record_requeue_refusal_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    card_id: &str,
    existing_metadata: &serde_json::Map<String, serde_json::Value>,
    actor: &str,
    reason: &str,
) -> Result<(), GuardError> {
    let mut meta = existing_metadata.clone();
    meta.insert(
        "requeue_refused_reason".to_string(),
        serde_json::Value::String(reason.to_string()),
    );
    meta.insert(
        "requeue_refused_at".to_string(),
        serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
    );
    let meta_str = serde_json::to_string(&serde_json::Value::Object(meta))
        .map_err(|e| GuardError::Db(e.to_string()))?;
    sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
        .bind(&meta_str)
        .bind(card_id)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
    decisions::append_audit_tx(tx, "none", Some(card_id), actor, 0, "requeue_refused", None)
        .await
        .map_err(GuardError::Db)?;
    Ok(())
}

/// Requeue a goal whose worker died (e.g. daemon restart): InProgress → Ready
/// with `new_attempt_count` recorded as the attempt count and the abandonment
/// reason recorded as `last_error`.
///
/// The caller decides the count. The restart reconciler passes the CURRENT count
/// unchanged when the interruption was a daemon-lifecycle change rather than a
/// worker failure (R2a) — see `classify_restart_charge`.
pub async fn requeue_goal(
    pool: &Pool<Sqlite>,
    card_id: &str,
    actor: &str,
    new_attempt_count: u64,
    reason: &str,
) -> Result<(), GuardError> {
    // The dead-worker contract this wrapper has always documented (see the
    // doc comment above): every existing caller of the 5-arg form already
    // believes the dispatched session is gone, so `dispatched_session_alive:
    // false` merely makes that belief explicit to the guard below.
    requeue_goal_with_meta(pool, card_id, actor, new_attempt_count, reason, &[], false).await
}

pub(crate) async fn requeue_goal_for_attempt(
    pool: &Pool<Sqlite>,
    card_id: &str,
    actor: &str,
    count: u64,
    reason: &str,
    expected: Option<&crate::goal_candidates::ExpectedAttempt>,
) -> Result<(), GuardError> {
    requeue_goal_with_meta_for_attempt(pool, card_id, actor, count, reason, &[], false, expected)
        .await
}

/// [`requeue_goal`], additionally merging `extra_meta` into the card's metadata
/// inside the SAME transaction.
///
/// Atomicity is the point: the restart exemption's idempotency marker
/// (`restart_forgiven_lifecycle`) must land with the requeue it describes, or a
/// crash between the two would hand the same goal a second free pass for the
/// same interruption.
///
/// `dispatched_session_alive` (E2, harness loop Round E, from Lane F9/M9
/// forensics 2026-09-10): a REQUIRED, caller-attested liveness assertion —
/// this guard cannot see process-local session state on its own (that lives
/// in the in-memory `AgentManager`, not the database this module reads), so
/// the caller must establish death before calling, exactly as R3's own
/// `!session_alive` check and the restart reconciler's "worker session gone"
/// already do. Passing `true` here is refused unconditionally: it describes
/// the one shape that produced F9's dominant failure class (4/6 of Round D's
/// M9 fixtures) — a still-working, often-successful session stranded when
/// the goal's own bookkeeping moved on to a second session dispatched over
/// it. If a requeue is genuinely needed for a live session, the caller must
/// cancel/mark that session first, then call this with `false`.
///
/// M1 (G8 review, harness loop Lane E3 2026-09-10): a caller passing
/// `false` is no longer trusted blindly either — this module still cannot
/// reach the in-memory `AgentManager`/dispatch registry (that cross-check
/// belongs to a caller that CAN, e.g. R3's own `is_session_busy` before it
/// gets here), but it DOES independently cross-check the goal's own
/// `execution_receipt` (a durable record written by a wholly separate code
/// path — the dispatch/heartbeat loop, not whatever is calling this
/// function). A `false` claim that contradicts a live, recently-heartbeated
/// receipt in the CURRENT daemon lifecycle is refused. This is a hint that
/// must agree with the receipt when one exists, not an unconditionally
/// trusted assertion; see the check itself, just below.
pub async fn requeue_goal_with_meta(
    pool: &Pool<Sqlite>,
    card_id: &str,
    actor: &str,
    new_attempt_count: u64,
    reason: &str,
    extra_meta: &[(&str, serde_json::Value)],
    dispatched_session_alive: bool,
) -> Result<(), GuardError> {
    requeue_goal_with_meta_for_attempt(
        pool,
        card_id,
        actor,
        new_attempt_count,
        reason,
        extra_meta,
        dispatched_session_alive,
        None,
    )
    .await
}

pub(crate) async fn requeue_goal_with_meta_for_attempt(
    pool: &Pool<Sqlite>,
    card_id: &str,
    actor: &str,
    new_attempt_count: u64,
    reason: &str,
    extra_meta: &[(&str, serde_json::Value)],
    dispatched_session_alive: bool,
    expected_attempt: Option<&crate::goal_candidates::ExpectedAttempt>,
) -> Result<(), GuardError> {
    // BEGIN IMMEDIATE: this guard reads the goal row / audit-chain head before
    // upgrading to a write; taking the write lock up front avoids the un-retryable
    // BUSY lock-upgrade a concurrent writer would trigger (see advance_goal_checked).
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(db_err)?;

    crate::goal_candidates::check_expected_tx(&mut tx, card_id, expected_attempt)
        .await
        .map_err(|e| GuardError::Denied(e.to_string()))?;
    let goal = read_goal_tx(&mut tx, card_id).await?;
    if goal.card_type != "goal" {
        return Err(GuardError::Invalid(format!(
            "Card '{}' is not a goal",
            card_id
        )));
    }

    let binding = state_binding_of_column_tx(&mut tx, &goal.column_id).await?;
    if binding.as_deref() == Some("ready") {
        // Idempotent retry/completion handling: another caller already
        // requeued this attempt. Do not rewrite metadata or append audit.
        tx.rollback().await.map_err(db_err)?;
        return Ok(());
    }
    if binding.as_deref() != Some("in_progress") {
        return Err(GuardError::Invalid(format!(
            "Only in_progress goals can be requeued (card '{}' is in '{}')",
            card_id,
            binding.as_deref().unwrap_or("?")
        )));
    }

    // E2 (harness loop, Round E, from Lane F9/M9 forensics 2026-09-10): M9
    // found genuinely-solved, landed goals (fx02/fx06 — trunk HEAD verified
    // moved via git log) put right back to Ready by a plain requeue after
    // landing, stranding the completed card in a state that reads as
    // unfinished forever after. A card whose landing receipt already shows
    // the fix reached trunk must never be requeued — there is nothing left
    // to redo, and redoing it risks a second, conflicting attempt.
    let already_landed = goal
        .metadata
        .get("dispatch_evidence")
        .and_then(|v| v.get("landing"))
        .and_then(|v| v.get("trunk_state"))
        .and_then(|v| v.as_str())
        == Some("landed_on_trunk");
    if already_landed {
        let refusal = format!(
            "requeue refused: goal '{card_id}' already landed on trunk \
             (dispatch_evidence.landing.trunk_state = landed_on_trunk) — requeuing would \
             strand completed work, not redo it"
        );
        record_requeue_refusal_tx(&mut tx, card_id, &goal.metadata, actor, &refusal).await?;
        tx.commit().await.map_err(db_err)?;
        return Err(GuardError::Denied(refusal));
    }

    // M1 (G8 review, harness loop Lane E3 2026-09-10): `dispatched_session_
    // alive` is caller-attested and, per this function's own doc comment,
    // this DB-only module cannot independently query the in-memory
    // `AgentManager` the way R3's own `is_session_busy` check does. It CAN
    // be cross-checked against a DIFFERENT independent signal already
    // persisted here: the goal's own `execution_receipt` — the completion
    // tracker's heartbeat record of the SAME attempt, written by a wholly
    // separate code path (the dispatch/heartbeat loop) from whatever caller
    // is requesting this requeue. `ExecutionReceipt::is_live` is exactly
    // the rebind-vs-stale test restart recovery already trusts for this
    // same question. When the receipt itself reports a live,
    // current-lifecycle, recently-heartbeated attempt, a caller claiming
    // `dispatched_session_alive: false` is asserting something the goal's
    // own durable record contradicts — treat that disagreement as
    // untrustworthy and refuse, rather than silently trusting whichever
    // side happens to have called this function. A goal with no
    // `execution_receipt` at all (never dispatched through that path) has
    // nothing to cross-check against and falls through unchanged.
    if !dispatched_session_alive {
        if let Some(receipt) = goal
            .metadata
            .get("execution_receipt")
            .cloned()
            .and_then(|v| {
                serde_json::from_value::<
                    crate::agents::platform_extensions::execution_receipt::ExecutionReceipt,
                >(v)
                .ok()
            })
        {
            let current_lifecycle =
                crate::agents::platform_extensions::orchestrator::daemon_lifecycle_id();
            if receipt.is_live(
                current_lifecycle,
                chrono::Utc::now(),
                crate::agents::platform_extensions::execution_receipt::DEFAULT_STALENESS_SECS,
            ) {
                let refusal = format!(
                    "requeue refused: caller attested goal '{card_id}' has no live dispatched \
                     session, but its own execution_receipt still reports a live, \
                     recently-heartbeated attempt in the current daemon lifecycle — the \
                     caller-attested flag disagrees with the goal's own durable record and is \
                     not trusted"
                );
                record_requeue_refusal_tx(&mut tx, card_id, &goal.metadata, actor, &refusal)
                    .await?;
                tx.commit().await.map_err(db_err)?;
                return Err(GuardError::Denied(refusal));
            }
        }
    }

    // E1/E2 (harness loop, Round E, from Lane F9 forensics 2026-09-10): see
    // this function's doc comment — the caller-attested liveness check.
    if dispatched_session_alive {
        let refusal = format!(
            "requeue refused: goal '{card_id}' has a live dispatched session — a goal may \
             never be requeued out from under a session that is still running; cancel or \
             confirm it dead first"
        );
        record_requeue_refusal_tx(&mut tx, card_id, &goal.metadata, actor, &refusal).await?;
        tx.commit().await.map_err(db_err)?;
        return Err(GuardError::Denied(refusal));
    }

    let journal_actor = journal_actor(actor, &goal.metadata);
    let mut meta = goal.metadata;
    meta.insert(
        "goal_state".to_string(),
        serde_json::Value::String("ready".to_string()),
    );
    meta.insert(
        "attempt_count".to_string(),
        serde_json::json!(new_attempt_count),
    );
    meta.insert(
        "last_error".to_string(),
        serde_json::Value::String(reason.to_string()),
    );
    for (key, value) in extra_meta {
        meta.insert((*key).to_string(), value.clone());
    }
    let meta_str = serde_json::to_string(&serde_json::Value::Object(meta))
        .map_err(|e| GuardError::Db(e.to_string()))?;

    let ready_col = goal_column_id_tx(&mut tx, &goal.project_id, "ready").await?;
    let position = next_position_tx(&mut tx, &ready_col).await?;

    sqlx::query("UPDATE cards SET metadata_json = ?, column_id = ?, position = ? WHERE id = ?")
        .bind(&meta_str)
        .bind(&ready_col)
        .bind(position)
        .bind(card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

    decisions::append_audit_tx(&mut tx, "none", Some(card_id), actor, 0, "requeue", None)
        .await
        .map_err(GuardError::Db)?;

    tx.commit().await.map_err(db_err)?;

    crate::events::emit(crate::events::goal_state_changed(
        card_id,
        Some(&goal.project_id),
        Some("in_progress"),
        "ready",
        &journal_actor,
    ));

    Ok(())
}

/// F1a (harness loop, Lane F1 2026-09-10, from F10 forensics): whether a
/// goal's own `execution_receipt` reports a live, current-lifecycle,
/// recently-heartbeated attempt — the exact rebind-vs-stale test restart
/// recovery already trusts for this question
/// (`execution_receipt::ExecutionReceipt::is_live`). Exposed here as a
/// small, standalone helper (a NEW function, not a change to
/// `requeue_goal_with_meta`'s own body above — that cross-check belongs to
/// Lane E3, a sibling harness-loop branch not merged into this lane's base)
/// so R3's misfire guard
/// (`platform_extensions::orchestrator::OrchestratorClient::handle_goal_status`)
/// can reuse the identical semantics without duplicating `is_live`'s logic
/// inline. A goal with no `execution_receipt` at all has nothing to report
/// live, so this returns `false` — that is not evidence the session is
/// exited, only "this check found nothing to contradict a death claim";
/// callers combine it with their own independent liveness signal (e.g. R3's
/// own `is_session_busy`).
pub fn execution_receipt_is_live(
    metadata: &serde_json::Value,
    current_lifecycle: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    metadata
        .get("execution_receipt")
        .cloned()
        .and_then(|v| {
            serde_json::from_value::<
                crate::agents::platform_extensions::execution_receipt::ExecutionReceipt,
            >(v)
            .ok()
        })
        .map(|receipt| {
            receipt.is_live(
                current_lifecycle,
                now,
                crate::agents::platform_extensions::execution_receipt::DEFAULT_STALENESS_SECS,
            )
        })
        .unwrap_or(false)
}

/// Return a goal to Ready from InProgress **or** Review without consuming the
/// ordinary attempt cap (unless `new_attempt_count` is `Some`). Used by
/// bounded refinement after a completion-check failure.
pub async fn return_to_ready(
    pool: &Pool<Sqlite>,
    card_id: &str,
    actor: &str,
    reason: &str,
    new_attempt_count: Option<u64>,
    extra_patch: serde_json::Map<String, serde_json::Value>,
) -> Result<(), GuardError> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(db_err)?;

    let goal = read_goal_tx(&mut tx, card_id).await?;
    if goal.card_type != "goal" {
        return Err(GuardError::Invalid(format!(
            "Card '{}' is not a goal",
            card_id
        )));
    }

    let binding = state_binding_of_column_tx(&mut tx, &goal.column_id).await?;
    if binding.as_deref() == Some("ready") {
        tx.rollback().await.map_err(db_err)?;
        return Ok(());
    }
    if binding.as_deref() != Some("in_progress") && binding.as_deref() != Some("review") {
        return Err(GuardError::Invalid(format!(
            "Only in_progress or review goals can return to Ready (card '{}' is in '{}')",
            card_id,
            binding.as_deref().unwrap_or("?")
        )));
    }

    let from_binding = binding.clone().unwrap_or_else(|| "unknown".to_string());
    let journal_actor = journal_actor(actor, &goal.metadata);
    let mut meta = goal.metadata;
    meta.insert(
        "goal_state".to_string(),
        serde_json::Value::String("ready".to_string()),
    );
    if let Some(count) = new_attempt_count {
        meta.insert("attempt_count".to_string(), serde_json::json!(count));
    }
    meta.insert(
        "last_error".to_string(),
        serde_json::Value::String(reason.to_string()),
    );
    apply_patch(&mut meta, &extra_patch);
    let meta_str = serde_json::to_string(&serde_json::Value::Object(meta))
        .map_err(|e| GuardError::Db(e.to_string()))?;

    let ready_col = goal_column_id_tx(&mut tx, &goal.project_id, "ready").await?;
    let position = next_position_tx(&mut tx, &ready_col).await?;

    sqlx::query("UPDATE cards SET metadata_json = ?, column_id = ?, position = ? WHERE id = ?")
        .bind(&meta_str)
        .bind(&ready_col)
        .bind(position)
        .bind(card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

    decisions::append_audit_tx(
        &mut tx,
        "none",
        Some(card_id),
        actor,
        0,
        "return_to_ready",
        None,
    )
    .await
    .map_err(GuardError::Db)?;

    tx.commit().await.map_err(db_err)?;

    crate::events::emit(crate::events::goal_state_changed(
        card_id,
        Some(&goal.project_id),
        Some(from_binding.as_str()),
        "ready",
        &journal_actor,
    ));

    Ok(())
}

/// Merge post-spawn metadata into an InProgress goal, but only while the
/// caller's dispatch claim still owns it.
pub async fn finalize_dispatch_claim(
    pool: &Pool<Sqlite>,
    card_id: &str,
    claim_id: &str,
    metadata_patch: serde_json::Map<String, serde_json::Value>,
) -> Result<(), GuardError> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(db_err)?;
    let goal = read_goal_tx(&mut tx, card_id).await?;
    let binding = state_binding_of_column_tx(&mut tx, &goal.column_id).await?;
    let active_claim = goal
        .metadata
        .get("dispatch_claim_id")
        .and_then(|value| value.as_str());
    if binding.as_deref() != Some("in_progress") || active_claim != Some(claim_id) {
        return Err(GuardError::Invalid(format!(
            "Dispatch claim '{}' no longer owns card '{}'",
            claim_id, card_id
        )));
    }

    crate::goal_attempts::finalize_tx(
        &mut tx,
        card_id,
        claim_id,
        metadata_patch
            .get("worker_session_id")
            .and_then(|v| v.as_str()),
        metadata_patch
            .get("execution_receipt")
            .and_then(|v| v.get("process_id"))
            .and_then(|v| v.as_i64()),
    )
    .await
    .map_err(|e| GuardError::Db(e.to_string()))?;
    let mut meta = goal.metadata;
    apply_patch(&mut meta, &metadata_patch);
    let meta_str = serde_json::to_string(&serde_json::Value::Object(meta))
        .map_err(|e| GuardError::Db(e.to_string()))?;
    sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
        .bind(&meta_str)
        .bind(card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(())
}

/// Release an exact dispatch claim after its worker could not be started or
/// registered. The attempt already consumed by the claim is preserved.
pub async fn release_dispatch_claim(
    pool: &Pool<Sqlite>,
    card_id: &str,
    claim_id: &str,
    reason: &str,
) -> Result<(), GuardError> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(db_err)?;
    let goal = read_goal_tx(&mut tx, card_id).await?;
    let binding = state_binding_of_column_tx(&mut tx, &goal.column_id).await?;
    let active_claim = goal
        .metadata
        .get("dispatch_claim_id")
        .and_then(|value| value.as_str());
    if binding.as_deref() != Some("in_progress") || active_claim != Some(claim_id) {
        return Err(GuardError::Invalid(format!(
            "Dispatch claim '{}' no longer owns card '{}'",
            claim_id, card_id
        )));
    }

    crate::goal_attempts::release_tx(&mut tx, card_id, claim_id, reason)
        .await
        .map_err(|e| GuardError::Db(e.to_string()))?;
    let journal_actor = journal_actor(decisions::ACTOR_SYSTEM, &goal.metadata);
    let mut meta = goal.metadata;
    meta.insert(
        "goal_state".to_string(),
        serde_json::Value::String("ready".to_string()),
    );
    meta.insert(
        "last_error".to_string(),
        serde_json::Value::String(reason.to_string()),
    );
    meta.remove("dispatch_claim_id");
    let meta_str = serde_json::to_string(&serde_json::Value::Object(meta))
        .map_err(|e| GuardError::Db(e.to_string()))?;
    let ready_col = goal_column_id_tx(&mut tx, &goal.project_id, "ready").await?;
    let position = next_position_tx(&mut tx, &ready_col).await?;
    sqlx::query("UPDATE cards SET metadata_json = ?, column_id = ?, position = ? WHERE id = ?")
        .bind(&meta_str)
        .bind(&ready_col)
        .bind(position)
        .bind(card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    decisions::append_audit_tx(
        &mut tx,
        "none",
        Some(card_id),
        decisions::ACTOR_SYSTEM,
        0,
        "release_dispatch_claim",
        None,
    )
    .await
    .map_err(GuardError::Db)?;
    tx.commit().await.map_err(db_err)?;

    crate::events::emit(crate::events::goal_state_changed(
        card_id,
        Some(&goal.project_id),
        Some("in_progress"),
        "ready",
        &journal_actor,
    ));
    Ok(())
}

/// Record a retriable worker failure on an in-progress goal without changing
/// its state (`last_error` is a protected key, so this guarded path exists).
pub async fn record_goal_failure(
    pool: &Pool<Sqlite>,
    card_id: &str,
    error: &str,
) -> Result<(), GuardError> {
    // BEGIN IMMEDIATE: this guard reads the goal row / audit-chain head before
    // upgrading to a write; taking the write lock up front avoids the un-retryable
    // BUSY lock-upgrade a concurrent writer would trigger (see advance_goal_checked).
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(db_err)?;
    let goal = read_goal_tx(&mut tx, card_id).await?;
    let journal_actor = journal_actor(decisions::ACTOR_SYSTEM, &goal.metadata);
    let mut meta = goal.metadata;
    meta.insert(
        "last_error".to_string(),
        serde_json::Value::String(error.to_string()),
    );
    meta.insert(
        "goal_state".to_string(),
        serde_json::Value::String("in_progress".to_string()),
    );
    let meta_str = serde_json::to_string(&serde_json::Value::Object(meta))
        .map_err(|e| GuardError::Db(e.to_string()))?;
    sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
        .bind(&meta_str)
        .bind(card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;

    // State is unchanged (in_progress, retriable), but the latched error is
    // new — emit so any error badge on the live surfaces updates.
    crate::events::emit(crate::events::goal_state_changed(
        card_id,
        Some(&goal.project_id),
        Some("in_progress"),
        "in_progress",
        &journal_actor,
    ));

    Ok(())
}

/// Delete a goal card. Tier-2 gated: requires a `risk_gate` decision for this
/// goal answered 'approve' by 'jesse' (action class `user_data_deletion`).
pub async fn delete_goal_checked(
    pool: &Pool<Sqlite>,
    card_id: &str,
    proof: DecisionProof,
) -> Result<bool, GuardError> {
    if proof.kind() != "risk_gate" {
        return Err(GuardError::Denied(format!(
            "Goal deletion requires a risk_gate decision (got kind '{}')",
            proof.kind()
        )));
    }
    if proof.answer() != "approve" {
        return Err(GuardError::Denied(
            "Goal deletion requires an 'approve' answer".to_string(),
        ));
    }
    if proof.acted_by() != decisions::ACTOR_JESSE {
        return Err(GuardError::Denied(format!(
            "Goal deletion is Tier 2 and requires acted_by='jesse' (got '{}')",
            proof.acted_by()
        )));
    }
    if proof.goal_id() != Some(card_id) {
        return Err(GuardError::Denied(format!(
            "Decision {} does not reference goal '{}'",
            proof.decision_id(),
            card_id
        )));
    }

    // BEGIN IMMEDIATE: this guard reads the goal row / audit-chain head before
    // upgrading to a write; taking the write lock up front avoids the un-retryable
    // BUSY lock-upgrade a concurrent writer would trigger (see advance_goal_checked).
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(db_err)?;
    decisions::append_audit_tx(
        &mut tx,
        proof.decision_id(),
        Some(card_id),
        proof.acted_by(),
        2,
        "delete",
        None,
    )
    .await
    .map_err(GuardError::Db)?;
    let result = sqlx::query("DELETE FROM cards WHERE id = ?")
        .bind(card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;

    let deleted = result.rows_affected() > 0;
    if deleted {
        crate::events::emit(crate::events::goal_state_changed(
            card_id,
            None,
            None,
            "deleted",
            proof.acted_by(),
        ));
    }
    Ok(deleted)
}

// ── Budgets (S4) ────────────────────────────────────────────────────────────

/// Per-goal budget, stored under the protected `budget` metadata key.
#[derive(Debug, Clone)]
pub struct GoalBudget {
    pub token_budget: Option<u64>,
    pub attempt_cap: u64,
    pub wallclock_cap_secs: Option<u64>,
}

impl Default for GoalBudget {
    fn default() -> Self {
        Self {
            token_budget: None,
            attempt_cap: DEFAULT_ATTEMPT_CAP,
            wallclock_cap_secs: None,
        }
    }
}

/// Parse a goal card's budget from its metadata (defaults when absent).
pub fn goal_budget(metadata: &serde_json::Value) -> GoalBudget {
    let b = metadata.get("budget");
    GoalBudget {
        token_budget: b
            .and_then(|b| b.get("token_budget"))
            .and_then(|v| v.as_u64()),
        attempt_cap: b
            .and_then(|b| b.get("attempt_cap"))
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_ATTEMPT_CAP),
        wallclock_cap_secs: b
            .and_then(|b| b.get("wallclock_cap_secs"))
            .and_then(|v| v.as_u64()),
    }
}

/// Which budget dimension is exhausted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetExhaustion {
    AttemptCap { spent: u64, cap: u64 },
    TokenBudget { spent: u64, cap: u64 },
    Wallclock { spent_secs: u64, cap_secs: u64 },
    RefinementBudget { spent: u64, cap: u64 },
}

impl BudgetExhaustion {
    pub fn reason(&self) -> decisions::UnblockReason {
        match self {
            Self::AttemptCap { .. } => decisions::UnblockReason::AttemptCap,
            Self::TokenBudget { .. } => decisions::UnblockReason::TokenBudget,
            Self::Wallclock { .. } => decisions::UnblockReason::WallclockCap,
            Self::RefinementBudget { .. } => decisions::UnblockReason::RefinementBudget,
        }
    }

    pub fn spent_and_cap(&self) -> (u64, u64) {
        match *self {
            Self::AttemptCap { spent, cap } => (spent, cap),
            Self::TokenBudget { spent, cap } => (spent, cap),
            Self::Wallclock {
                spent_secs,
                cap_secs,
            } => (spent_secs, cap_secs),
            Self::RefinementBudget { spent, cap } => (spent, cap),
        }
    }

    /// Lane R21d (A23 Q2 #2): a short, stable slug naming which dimension
    /// tripped — `record_headless_unblock_refusal` (orchestrator.rs) used to
    /// log every `check_budget` refusal under the same generic string,
    /// `"cost_budget_exhausted"`, no matter which variant fired; A23 traced a
    /// real `AttemptCap { spent: 3, cap: 3 }` miscategorized that way, costing
    /// real analysis time chasing a dollar cap that was never close. Callers
    /// combine this with `spent_and_cap()` in the evidence digest so the next
    /// analysis reads the dimension and the numbers straight off the audit
    /// row instead of re-deriving them from `decision_audit`/`cards`.
    pub fn gate_label(&self) -> &'static str {
        match self {
            Self::AttemptCap { .. } => "attempt_cap_exhausted",
            Self::TokenBudget { .. } => "token_budget_exhausted",
            Self::Wallclock { .. } => "wallclock_cap_exhausted",
            Self::RefinementBudget { .. } => "refinement_budget_exhausted",
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::AttemptCap { spent, cap } => {
                format!("attempt cap exhausted ({}/{} attempts)", spent, cap)
            }
            Self::TokenBudget { spent, cap } => {
                format!("token budget exhausted ({}/{} tokens)", spent, cap)
            }
            Self::Wallclock {
                spent_secs,
                cap_secs,
            } => format!(
                "wallclock cap exhausted ({}s elapsed of {}s)",
                spent_secs, cap_secs
            ),
            Self::RefinementBudget { spent, cap } => {
                format!("refinement budget exhausted ({spent}/{cap} check-failure reworks)")
            }
        }
    }
}

/// Sum of accumulated total tokens across all worker sessions a goal has used
/// (per-attempt history in `worker_session_ids` plus the current
/// `worker_session_id`).
pub async fn goal_spent_tokens(
    pool: &Pool<Sqlite>,
    metadata: &serde_json::Value,
) -> Result<u64, String> {
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    if let Some(value) = metadata.get("worker_session_ids") {
        let array = value.as_array().ok_or_else(|| {
            "worker_session_ids is not a list; token accounting is unknown".to_string()
        })?;
        for value in array {
            let id = value.as_str().ok_or_else(|| {
                "worker_session_ids contains a non-session value; token accounting is unknown"
                    .to_string()
            })?;
            let id = id.trim();
            if id.is_empty() {
                return Err(
                    "worker_session_ids contains an empty session id; token accounting is unknown"
                        .to_string(),
                );
            }
            if seen.insert(id.to_string()) {
                ids.push(id.to_string());
            }
        }
    }
    if let Some(value) = metadata.get("worker_session_id") {
        let sid = value.as_str().ok_or_else(|| {
            "worker_session_id is not a session id; token accounting is unknown".to_string()
        })?;
        let sid = sid.trim();
        if sid.is_empty() {
            return Err("worker_session_id is empty; token accounting is unknown".to_string());
        }
        if seen.insert(sid.to_string()) {
            ids.push(sid.to_string());
        }
    }
    if ids.is_empty() {
        return Ok(0);
    }

    let placeholders = vec!["?"; ids.len()].join(", ");
    let sql = format!(
        "SELECT id, accumulated_total_tokens FROM sessions WHERE id IN ({})",
        placeholders
    );
    // `placeholders` is only "?" repeated `ids.len()` times; ids are bound below.
    let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
    for id in &ids {
        query = query.bind(id);
    }
    let rows = query.fetch_all(pool).await.map_err(|e| e.to_string())?;
    if rows.len() != ids.len() {
        let found: HashSet<String> = rows
            .iter()
            .filter_map(|row| row.try_get::<String, _>("id").ok())
            .collect();
        let missing = ids
            .iter()
            .filter(|id| !found.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        return Err(format!(
            "worker session(s) do not exist; token accounting is unknown: {}",
            missing.join(", ")
        ));
    }

    let mut total = 0u64;
    for row in rows {
        let id: String = row.try_get("id").map_err(|e| e.to_string())?;
        let tokens: Option<i32> = row
            .try_get("accumulated_total_tokens")
            .map_err(|e| e.to_string())?;
        let tokens = tokens.ok_or_else(|| {
            format!(
                "worker session '{}' has no accumulated token total; token accounting is unknown",
                id
            )
        })?;
        if tokens < 0 {
            return Err(format!(
                "worker session '{}' has a negative accumulated token total; token accounting is unknown",
                id
            ));
        }
        total = total.checked_add(tokens as u64).ok_or_else(|| {
            "worker token total overflow; token accounting is unknown".to_string()
        })?;
    }
    Ok(total)
}

/// Check a goal's budget. Returns the first exhausted dimension, or None when
/// the goal may consume another attempt.
pub async fn check_budget(
    pool: &Pool<Sqlite>,
    metadata: &serde_json::Value,
) -> Result<Option<BudgetExhaustion>, String> {
    let budget = goal_budget(metadata);

    let attempt_count = metadata
        .get("attempt_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if attempt_count >= budget.attempt_cap {
        return Ok(Some(BudgetExhaustion::AttemptCap {
            spent: attempt_count,
            cap: budget.attempt_cap,
        }));
    }

    if let Some(cap) = budget.token_budget {
        let spent = goal_spent_tokens(pool, metadata).await?;
        if spent >= cap {
            return Ok(Some(BudgetExhaustion::TokenBudget { spent, cap }));
        }
    }

    if let Some(cap_secs) = budget.wallclock_cap_secs {
        if let Some(first) = metadata
            .get("first_dispatched_at")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        {
            let elapsed = (chrono::Utc::now() - first.with_timezone(&chrono::Utc)).num_seconds();
            if elapsed >= 0 && elapsed as u64 >= cap_secs {
                return Ok(Some(BudgetExhaustion::Wallclock {
                    spent_secs: elapsed as u64,
                    cap_secs,
                }));
            }
        }
    }

    Ok(None)
}

/// Budget exhaustion handler: emit a `kind='unblock'` decision (deduplicated
/// per goal) and park the goal. NEVER a silent retry (S4).
///
/// Returns the id of the open unblock decision for the goal.
///
/// `goal_title` is no longer written into the headline (Decision Inbox D1):
/// the inbox joins the full title in as `goal_title`, and a quoted title cut
/// at 80 characters was the headline's main defect. It stays a parameter so
/// no caller has to change.
pub async fn exhaust_and_park(
    pool: &Pool<Sqlite>,
    card_id: &str,
    goal_title: &str,
    project_id: &str,
    exhaustion: BudgetExhaustion,
    last_error: Option<&str>,
) -> Result<String, String> {
    exhaust_and_park_with_cause(
        pool, card_id, goal_title, project_id, exhaustion, last_error, None,
    )
    .await
}

/// Title-free headline for a budget park (D1).
pub fn exhaustion_headline(exhaustion: &BudgetExhaustion) -> &'static str {
    match exhaustion {
        BudgetExhaustion::Wallclock { .. } => "Goal ran out of time and needs your direction",
        BudgetExhaustion::AttemptCap { .. } => {
            "Goal used all its attempts and needs your direction"
        }
        BudgetExhaustion::TokenBudget { .. } => "Goal ran out of budget and needs your direction",
        BudgetExhaustion::RefinementBudget { .. } => {
            "Goal kept failing its checks and needs your direction"
        }
    }
}

/// The last-error prefix `orchestrator::handle_goal_timeout` writes when one
/// attempt overran the worker's per-attempt time bound. How that park is told
/// apart from a whole-goal wall budget park: both arrive as
/// `BudgetExhaustion::Wallclock`. The parks the run imposes from outside carry
/// their own prefixes ([`TimeoutPark::error_prefix`]).
const ATTEMPT_TIMEOUT_ERROR_PREFIX: &str = "worker exceeded its time bound";

/// The clause `orchestrator::handle_goal_timeout` appends to a park's salvage
/// note when the credential guard quarantined the worker's uncommitted
/// changes: real work, left uncommitted in the kept worktree for a human.
pub const QUARANTINED_WORK_MARKER: &str = "uncommitted changes NOT checkpointed";

/// Why `orchestrator::handle_goal_timeout_for` is parking a goal that is still
/// running. The handler is one funnel for four different stops, and all four
/// arrive as `BudgetExhaustion::Wallclock` with the handler's `secs` as both
/// spent and cap — but only [`TimeoutPark::AttemptBound`] is the worker
/// running out of its own time. For the others `secs` is the run's elapsed
/// time (or 0 for a shutdown), never a per-attempt budget, and the park must
/// not be explained as one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutPark {
    /// The worker hit its own per-attempt time bound (`GoalOutcome::TimedOut`).
    AttemptBound,
    /// A headless run found the worker stalled and parked the goal.
    Stalled,
    /// A headless run reached its own time cap with the goal still running.
    RunTimeLimit,
    /// The run was shutting down (SIGTERM/SIGINT) with the goal still running.
    Shutdown,
}

impl TimeoutPark {
    /// The last-error prefix each park records. Lowercase ASCII, so it can be
    /// found in lowercased decision text too.
    pub fn error_prefix(self) -> &'static str {
        match self {
            Self::AttemptBound => ATTEMPT_TIMEOUT_ERROR_PREFIX,
            Self::Stalled => "the run parked this goal after its worker stalled",
            Self::RunTimeLimit => {
                "the run reached its own time limit while this goal's worker was still running"
            }
            Self::Shutdown => "the run shut down while this goal's worker was still running",
        }
    }

    /// Which park a recorded last error came from (it starts with the prefix).
    pub fn of_last_error(last_error: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|park| last_error.starts_with(park.error_prefix()))
    }

    /// Which park a decision's headline/detail quotes, anywhere in `text`.
    pub fn in_text(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|park| text.contains(park.error_prefix()))
    }

    const ALL: [TimeoutPark; 4] = [
        Self::AttemptBound,
        Self::Stalled,
        Self::RunTimeLimit,
        Self::Shutdown,
    ];
}

/// Headline for a park the run imposed from outside: the goal did not run out
/// of time, so [`exhaustion_headline`]'s "ran out of time" would be false.
fn stopped_by_run_headline(last_error: Option<&str>) -> Option<&'static str> {
    match last_error.and_then(TimeoutPark::of_last_error)? {
        TimeoutPark::AttemptBound => None,
        TimeoutPark::Stalled | TimeoutPark::RunTimeLimit | TimeoutPark::Shutdown => {
            Some("Goal was stopped before it finished and needs your direction")
        }
    }
}

/// D2: pick the specific cause for a budget park when the caller did not
/// name one. A wall-clock park the run imposed from outside (a stall, the
/// run's own time limit, a shutdown) gets no cause: none describes it, and
/// its last-error prefix is what the inbox explains it from.
pub fn infer_exhaustion_cause(
    exhaustion: &BudgetExhaustion,
    last_error: Option<&str>,
) -> Option<decisions::UnblockCause> {
    match exhaustion {
        BudgetExhaustion::Wallclock { .. } => {
            match last_error.and_then(TimeoutPark::of_last_error) {
                Some(TimeoutPark::AttemptBound) => Some(decisions::UnblockCause::AttemptTimeout),
                Some(TimeoutPark::Stalled | TimeoutPark::RunTimeLimit | TimeoutPark::Shutdown) => {
                    None
                }
                None => Some(decisions::UnblockCause::WallBudget),
            }
        }
        _ => None,
    }
}

/// D2: structured salvage facts for this park, read from the goal's
/// `salvage_receipt` — but only when the park's own last error quotes that
/// receipt's note, so a receipt left by an earlier park is never reused.
///
/// `commits_past_baseline` counts commits NOT yet landed: 0 for both "no
/// commits" and "already on the trunk", which the inbox tells apart by
/// `outcome`. A missing dispatch baseline measured nothing and stays `None`.
/// The quarantine clause `handle_goal_timeout` appends after the receipt's
/// note is carried as its own flag, so a park whose real changes were held
/// back never reads as "no changes".
pub fn salvage_facts_for_park(
    metadata: &serde_json::Value,
    last_error: Option<&str>,
) -> Option<decisions::SalvageFacts> {
    let receipt = metadata.get("salvage_receipt")?;
    let outcome = receipt.get("outcome")?.as_str()?.to_string();
    let note = receipt.get("note").and_then(|v| v.as_str()).unwrap_or("");
    let last_error = last_error?;
    if note.is_empty() || !last_error.contains(note) {
        return None;
    }
    let commits_past_baseline = match outcome.as_str() {
        "salvage_skipped_no_commits" | "salvage_skipped_already_landed" => Some(0),
        _ => note.find(" commit(s)").and_then(|idx| {
            note.get(..idx)?
                .rsplit(|c: char| !c.is_ascii_digit())
                .next()?
                .parse()
                .ok()
        }),
    };
    Some(decisions::SalvageFacts {
        outcome,
        commits_past_baseline,
        uncommitted_work_quarantined: last_error.contains(QUARANTINED_WORK_MARKER),
    })
}

/// [`exhaust_and_park`] with an explicit [`decisions::UnblockCause`] for the
/// callers that know more than the exhaustion dimension (e.g. a review
/// rejected until the attempts ran out). `None` infers what it can.
pub async fn exhaust_and_park_with_cause(
    pool: &Pool<Sqlite>,
    card_id: &str,
    _goal_title: &str,
    project_id: &str,
    exhaustion: BudgetExhaustion,
    last_error: Option<&str>,
    cause: Option<decisions::UnblockCause>,
) -> Result<String, String> {
    exhaust_and_park_for_attempt(
        pool,
        card_id,
        _goal_title,
        project_id,
        exhaustion,
        last_error,
        cause,
        None,
    )
    .await
}

pub(crate) async fn exhaust_and_park_for_attempt(
    pool: &Pool<Sqlite>,
    card_id: &str,
    _goal_title: &str,
    project_id: &str,
    exhaustion: BudgetExhaustion,
    last_error: Option<&str>,
    cause: Option<decisions::UnblockCause>,
    expected_attempt: Option<&crate::goal_candidates::ExpectedAttempt>,
) -> Result<String, String> {
    if let Some(expected) = expected_attempt {
        let mut tx = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|e| e.to_string())?;
        crate::goal_candidates::check_expected_tx(&mut tx, card_id, Some(expected))
            .await
            .map_err(|e| e.to_string())?;
        let goal = read_goal_tx(&mut tx, card_id).await.map_err(String::from)?;
        let (spent, cap) = exhaustion.spent_and_cap();
        let payload = serde_json::to_value(decisions::UnblockPayload {
            reason: exhaustion.reason(),
            spent: Some(spent),
            cap: Some(cap),
            cause: cause.or_else(|| infer_exhaustion_cause(&exhaustion, last_error)),
            salvage: salvage_facts_for_park(&serde_json::Value::Object(goal.metadata), last_error),
        })
        .map_err(|e| e.to_string())?;
        let existing:Option<String>=sqlx::query_scalar("SELECT id FROM decisions WHERE goal_id=? AND kind='unblock' AND status='open' ORDER BY created_at DESC LIMIT 1")
            .bind(card_id).fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?;
        let reason = format!(
            "{}{}",
            exhaustion.describe(),
            last_error
                .map(|e| format!("; last error: {e}"))
                .unwrap_or_default()
        );
        let meta = match &exhaustion {
            BudgetExhaustion::AttemptCap { spent, .. } => {
                vec![("attempt_count", serde_json::json!(*spent))]
            }
            _ => Vec::new(),
        };
        park_goal_with_meta_filtered_tx(
            &mut tx,
            card_id,
            decisions::ACTOR_SYSTEM,
            &reason,
            &meta,
            None,
            Some(expected),
        )
        .await
        .map_err(String::from)?;
        let created = if let Some(id) = existing {
            (id, None)
        } else {
            let (id, tier) = decisions::create_validated_decision_tx(
                &mut tx,
                decisions::NewDecision {
                    kind: "unblock".into(),
                    goal_id: Some(card_id.into()),
                    project_id: Some(project_id.into()),
                    headline: Some(
                        stopped_by_run_headline(last_error)
                            .unwrap_or_else(|| exhaustion_headline(&exhaustion))
                            .into(),
                    ),
                    detail: Some(format!(
                        "Goal {card_id} parked: {}. Last error: {}",
                        exhaustion.describe(),
                        last_error.unwrap_or("(none recorded)")
                    )),
                    payload,
                    ..Default::default()
                },
            )
            .await?;
            (id, Some(tier))
        };
        tx.commit().await.map_err(|e| e.to_string())?;
        if let Some(tier) = created.1 {
            crate::events::emit(crate::events::decision_created(&created.0, "unblock", tier));
        }
        crate::events::emit(crate::events::goal_state_changed(
            card_id,
            Some(project_id),
            None,
            "failed",
            decisions::ACTOR_SYSTEM,
        ));
        return Ok(created.0);
    }
    // Dedupe: one open unblock decision per goal.
    let decision_id = match decisions::find_open_decision_for_goal(pool, card_id, "unblock").await?
    {
        Some(existing) => existing.id,
        None => {
            let (spent, cap) = exhaustion.spent_and_cap();
            let headline = stopped_by_run_headline(last_error)
                .unwrap_or_else(|| exhaustion_headline(&exhaustion))
                .to_string();
            let detail = format!(
                "Goal {} parked: {}. Last error: {}",
                card_id,
                exhaustion.describe(),
                last_error.unwrap_or("(none recorded)")
            );
            // Best-effort read: salvage facts only enrich the explanation.
            let salvage = crate::cards::get_card(pool, card_id)
                .await
                .ok()
                .flatten()
                .and_then(|card| salvage_facts_for_park(&card.metadata_json, last_error));
            let payload = serde_json::to_value(decisions::UnblockPayload {
                reason: exhaustion.reason(),
                spent: Some(spent),
                cap: Some(cap),
                cause: cause.or_else(|| infer_exhaustion_cause(&exhaustion, last_error)),
                salvage,
            })
            .map_err(|e| e.to_string())?;

            decisions::create_decision(
                pool,
                decisions::NewDecision {
                    kind: "unblock".to_string(),
                    goal_id: Some(card_id.to_string()),
                    project_id: Some(project_id.to_string()),
                    headline: Some(headline),
                    detail: Some(detail),
                    payload,
                    ..Default::default()
                },
            )
            .await?
            .id
        }
    };

    // Write down the attempt that tripped the cap. The unblock decision above
    // already quotes it ("3/3 attempts"); until now the card itself did not,
    // so the two disagreed by one on every parked goal.
    let attempt_meta: Vec<(&str, serde_json::Value)> = match &exhaustion {
        BudgetExhaustion::AttemptCap { spent, .. } => {
            vec![("attempt_count", serde_json::json!(*spent))]
        }
        _ => Vec::new(),
    };

    park_goal_with_meta_filtered(
        pool,
        card_id,
        decisions::ACTOR_SYSTEM,
        &format!(
            "{}{}",
            exhaustion.describe(),
            last_error
                .map(|e| format!("; last error: {}", e))
                .unwrap_or_default()
        ),
        &attempt_meta,
        None,
        expected_attempt,
    )
    .await
    .map_err(String::from)?;

    Ok(decision_id)
}

/// Lane R21d (A23 Q2 #2): whether a completed attempt made ANY progress —
/// a commit, a file change, or a successful verify call beyond its own
/// dispatch baseline. `false` for all three is the exact shape A23 found on
/// both DeepSeek m1/m3 misses' 3rd dispatch (`commits: []`,
/// `files_changed: 0`, no verify): a wasted attempt that immediately
/// re-triggered the same hold and only spent the cap.
pub fn attempt_made_zero_progress(
    has_commit: bool,
    has_file_change: bool,
    verify_ran: bool,
) -> bool {
    !has_commit && !has_file_change && !verify_ran
}

/// Lane R21d (A23 Q2 #2): forgive a completed attempt's `attempt_count` when
/// [`attempt_made_zero_progress`] found it produced nothing — A23 traced
/// BOTH DeepSeek m1/m3 misses to exactly this: a zero-progress 3rd dispatch
/// spent the last attempt without moving anything, parking a goal whose
/// correct fix a SIBLING attempt (byte-identical) had already landed with
/// one attempt to spare.
///
/// Called by the dispatch tracker right after this attempt's
/// `dispatch_evidence` is persisted and BEFORE `handle_goal_completion`
/// (`maybe_hold_review` → `check_budget`, an orchestrator.rs concern this
/// module never reaches into) reads the card — the only point at which
/// forgiving this attempt can still stop its OWN Hold/Park decision from
/// spending it, not just the next one.
///
/// `attempt_count` is stamped optimistically at DISPATCH time (the claim
/// patch in `dispatch_goal_fn`), not at completion — many existing tests
/// read it immediately after dispatch and must keep doing so — so
/// forgiveness here DECREMENTS the count already stamped for this attempt
/// rather than skipping an increment that already happened.
///
/// Bounded to at most ONE forgiveness per goal
/// ([`ZERO_PROGRESS_STREAK_METADATA_KEY`]): a card whose immediately prior
/// attempt was ALSO zero-progress (`streak >= 1`) counts this one normally
/// instead and records `streak = 2`, so a stuck worker cannot loop forever
/// on free attempts. Any attempt that DOES make progress resets the streak.
/// Best-effort: the caller only logs an `Err` — a failure here must never
/// block the completion handler that runs right after it.
pub async fn maybe_forgive_zero_progress_attempt(
    pool: &Pool<Sqlite>,
    card_id: &str,
    zero_progress: bool,
) -> Result<(), String> {
    update_zero_progress_streak(pool, card_id, zero_progress, true).await
}

/// QA 2026-09-26: the zero-progress streak for an attempt that ended
/// `Failed` or `TimedOut` rather than finishing. The streak used to move
/// only on a finished attempt, so a failed attempt that committed real work
/// left a stale `2` behind and the circuit breaker parked the goal on its
/// next dispatch ("consecutive attempts made no progress"), while a failed
/// attempt that did nothing never counted at all. Now progress resets the
/// streak and no progress counts toward it, exactly as for a finished
/// attempt — but never forgives `attempt_count`: a failed attempt's count is
/// the failure path's own business (its retry and park decisions read it),
/// and forgiveness stays reserved for a finished attempt.
pub async fn record_unfinished_attempt_progress(
    pool: &Pool<Sqlite>,
    card_id: &str,
    zero_progress: bool,
) -> Result<(), String> {
    update_zero_progress_streak(pool, card_id, zero_progress, false).await
}

/// The shared body of [`maybe_forgive_zero_progress_attempt`] and
/// [`record_unfinished_attempt_progress`]. `forgive_first`: whether the first
/// zero-progress attempt of a streak gets its `attempt_count` back.
async fn update_zero_progress_streak(
    pool: &Pool<Sqlite>,
    card_id: &str,
    zero_progress: bool,
    forgive_first: bool,
) -> Result<(), String> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|e| e.to_string())?;
    let row = sqlx::query(
        "SELECT card_type, metadata_json FROM cards WHERE id = ? AND archived_at IS NULL",
    )
    .bind(card_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    let Some(row) = row else {
        tx.rollback().await.map_err(|e| e.to_string())?;
        return Ok(());
    };
    let card_type: String = row.try_get("card_type").map_err(|e| e.to_string())?;
    if card_type != "goal" {
        tx.rollback().await.map_err(|e| e.to_string())?;
        return Ok(());
    }
    let meta_str: String = row.try_get("metadata_json").map_err(|e| e.to_string())?;
    let mut meta = serde_json::from_str::<serde_json::Value>(&meta_str)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();

    let prior_streak = meta
        .get(ZERO_PROGRESS_STREAK_METADATA_KEY)
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    if !zero_progress {
        if prior_streak == 0 {
            // Nothing to reset — avoid a no-op write.
            tx.rollback().await.map_err(|e| e.to_string())?;
            return Ok(());
        }
        meta.remove(ZERO_PROGRESS_STREAK_METADATA_KEY);
        meta.remove("zero_progress");
    } else if prior_streak >= 1 {
        // Second CONSECUTIVE zero-progress attempt: count it normally
        // (attempt_count is left exactly as the claim patch stamped it) and
        // record the streak so park_goal_with_meta can fold it into the
        // park detail.
        meta.insert(
            ZERO_PROGRESS_STREAK_METADATA_KEY.to_string(),
            serde_json::json!(2),
        );
        meta.insert("zero_progress".to_string(), serde_json::json!(true));
    } else if !forgive_first {
        // First zero-progress attempt this streak, but one that did not
        // finish: it counts toward the streak and is not forgiven.
        meta.insert(
            ZERO_PROGRESS_STREAK_METADATA_KEY.to_string(),
            serde_json::json!(1),
        );
        meta.insert("zero_progress".to_string(), serde_json::json!(true));
    } else {
        // First zero-progress attempt this streak: forgive it — undo the
        // dispatch-time increment for THIS attempt.
        let attempt_count = meta
            .get("attempt_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        meta.insert(
            "attempt_count".to_string(),
            serde_json::json!(attempt_count.saturating_sub(1)),
        );
        meta.insert(
            ZERO_PROGRESS_STREAK_METADATA_KEY.to_string(),
            serde_json::json!(1),
        );
        meta.insert("zero_progress".to_string(), serde_json::json!(true));
    }

    let meta_str =
        serde_json::to_string(&serde_json::Value::Object(meta)).map_err(|e| e.to_string())?;
    sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
        .bind(&meta_str)
        .bind(card_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}

/// QA 2026-09-26: a human's answer to a goal's `unblock` decision (approve,
/// or an `input` answer) clears the zero-progress circuit breaker's state —
/// the streak ([`ZERO_PROGRESS_STREAK_METADATA_KEY`] and `zero_progress`)
/// and the one escalation ([`ZERO_PROGRESS_ESCALATION_USED_METADATA_KEY`]) —
/// by nulling them in `patch`, the guarded metadata patch that unparks the
/// goal. All three are protected keys, so this is the only way they can go:
/// before this the breaker read the same streak on the very next dispatch
/// and parked the goal again before any worker ran, however often the human
/// answered. Returns whether anything was added.
///
/// Human answers only, the same rule as D7's wall-clock reset
/// (`decisions_effects::wall_budget_reset_patch`): an automated actor must
/// not be able to buy a goal more zero-progress runs.
pub fn clear_zero_progress_on_human_unblock(
    patch: &mut serde_json::Map<String, serde_json::Value>,
    acted_by: &str,
) -> bool {
    if acted_by != decisions::ACTOR_JESSE {
        return false;
    }
    for key in [
        ZERO_PROGRESS_STREAK_METADATA_KEY,
        "zero_progress",
        ZERO_PROGRESS_ESCALATION_USED_METADATA_KEY,
    ] {
        patch.insert(key.to_string(), serde_json::Value::Null);
    }
    true
}

// ── Dependency promotion (pool-only, no orchestrator needed) ────────────────

/// Promote Triage goals whose dependencies are all Complete to Ready
/// (tier-0 'ready' transitions through the guard). Dispatching the promoted
/// goals remains the orchestrator's job.
pub async fn promote_eligible_dependents(
    pool: &Pool<Sqlite>,
    project_id: &str,
) -> Result<u32, String> {
    let complete_col: Option<String> = sqlx::query_scalar(
        "SELECT id FROM board_columns WHERE project_id = ? AND state_binding = 'complete'",
    )
    .bind(project_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    let complete_col = match complete_col {
        Some(c) => c,
        None => return Ok(0),
    };

    let triage_goals = sqlx::query_as::<_, (String, String)>(
        "SELECT c.id, c.metadata_json FROM cards c \
         JOIN board_columns bc ON c.column_id = bc.id \
         WHERE c.project_id = ? AND c.card_type = 'goal' \
           AND bc.state_binding = 'triage' AND c.archived_at IS NULL",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut promoted = 0u32;
    for (goal_id, meta_str) in &triage_goals {
        let meta: serde_json::Value = serde_json::from_str(meta_str).unwrap_or_default();

        // Parked goals stay parked until their unblock decision is answered.
        if meta
            .get("needs_human_attention")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            continue;
        }

        let deps: Vec<String> = meta
            .get("depends_on")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        if deps.is_empty() {
            continue;
        }

        let mut all_complete = true;
        for dep_id in &deps {
            let dep_col: Option<String> =
                sqlx::query_scalar("SELECT column_id FROM cards WHERE id = ?")
                    .bind(dep_id)
                    .fetch_optional(pool)
                    .await
                    .map_err(|e| e.to_string())?;
            if dep_col.as_deref() != Some(complete_col.as_str()) {
                all_complete = false;
                break;
            }
        }

        if all_complete {
            advance_goal_checked(
                pool,
                goal_id,
                GoalAction::Ready,
                decisions::ACTOR_SYSTEM,
                None,
                TransitionEffects::default(),
            )
            .await
            .map_err(String::from)?;
            promoted += 1;
        }
    }

    Ok(promoted)
}

/// [`promote_eligible_dependents`] for the post-approval spine: a failure is
/// converted into a logged warning string instead of being discarded (bug-sweep
/// wave 1 — both approve paths used `let _ =`, leaving dependent goals blocked
/// with no cause and no trace). The approval itself has already committed, so
/// the caller surfaces the warning (response `effect_error` / audit note)
/// WITHOUT failing the approval.
pub async fn promote_eligible_dependents_or_warn(
    pool: &Pool<Sqlite>,
    project_id: &str,
    approved_goal_id: &str,
) -> Option<String> {
    match promote_eligible_dependents(pool, project_id).await {
        Ok(_) => None,
        Err(e) => {
            let warning = format!(
                "dependent promotion failed for project {} after approving goal {}: {} — \
                 dependent goals may remain blocked in Triage until the next approval in \
                 the project re-runs promotion",
                project_id, approved_goal_id, e
            );
            tracing::error!(
                goal_id = %approved_goal_id,
                project_id = %project_id,
                "{}",
                warning
            );
            Some(warning)
        }
    }
}

// ── Post-creation roadmap editing (#251) ────────────────────────────────────

/// Input for [`insert_roadmap_goal`].
#[derive(Debug, Default)]
pub struct NewRoadmapGoal {
    pub title: String,
    pub description: Option<String>,
    pub acceptance_criteria: Vec<String>,
    pub tags: Vec<String>,
    /// Card ids of existing goals in the same project this goal depends on.
    pub depends_on: Vec<String>,
}

/// Goal states whose dependency wiring may still be edited (#251): a goal that
/// is already running, under review, or terminal has left the dispatch graph.
const DEP_EDITABLE_STATES: &[&str] = &["triage", "ready", "failed"];

/// Validate that every id in `deps` names an existing goal card in
/// `project_id`, is unique, and is not `self_id`. Returns the deduped list.
async fn validate_dep_ids(
    pool: &Pool<Sqlite>,
    project_id: &str,
    self_id: Option<&str>,
    deps: &[String],
) -> Result<Vec<String>, GuardError> {
    let mut seen: Vec<String> = Vec::new();
    for dep_id in deps {
        if seen.iter().any(|d| d == dep_id) {
            continue; // dedupe silently
        }
        if Some(dep_id.as_str()) == self_id {
            return Err(GuardError::Invalid(
                "A goal cannot depend on itself".to_string(),
            ));
        }
        let row: Option<(String, String)> =
            sqlx::query_as("SELECT card_type, project_id FROM cards WHERE id = ?")
                .bind(dep_id)
                .fetch_optional(pool)
                .await
                .map_err(db_err)?;
        match row {
            None => {
                return Err(GuardError::NotFound(format!(
                    "Dependency '{}' not found",
                    dep_id
                )));
            }
            Some((card_type, _)) if card_type != "goal" => {
                return Err(GuardError::Invalid(format!(
                    "Dependency '{}' is not a goal card",
                    dep_id
                )));
            }
            Some((_, dep_project)) if dep_project != project_id => {
                return Err(GuardError::Invalid(format!(
                    "Dependency '{}' belongs to a different project",
                    dep_id
                )));
            }
            Some(_) => {}
        }
        seen.push(dep_id.clone());
    }
    Ok(seen)
}

/// Load the project's goal cards as a [`GraphGoal`] set for cycle validation.
async fn load_goal_graph(
    pool: &Pool<Sqlite>,
    project_id: &str,
) -> Result<Vec<crate::goal_state::GraphGoal>, GuardError> {
    let cards = crate::cards::list_cards(pool, project_id, Some("goal"), None)
        .await
        .map_err(GuardError::Db)?;
    Ok(cards
        .into_iter()
        .map(|c| crate::goal_state::GraphGoal {
            id: c.id,
            title: c.title,
            depends_on: c
                .metadata_json
                .get("depends_on")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect())
}

/// Guarded write of a goal's `depends_on` metadata + audit append. Internal
/// helper shared by the roadmap edit operations — callers have already
/// validated the graph.
async fn write_depends_on_audited(
    pool: &Pool<Sqlite>,
    card_id: &str,
    deps: &[String],
    actor: &str,
    outcome: &str,
) -> Result<(), GuardError> {
    // BEGIN IMMEDIATE: read-then-write, same idiom as the other guards here.
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(db_err)?;
    let goal = read_goal_tx(&mut tx, card_id).await?;
    let mut meta = goal.metadata;
    meta.insert("depends_on".to_string(), serde_json::json!(deps));
    let meta_str = serde_json::to_string(&serde_json::Value::Object(meta))
        .map_err(|e| GuardError::Db(e.to_string()))?;
    sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
        .bind(&meta_str)
        .bind(card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    decisions::append_audit_tx(&mut tx, "none", Some(card_id), actor, 0, outcome, None)
        .await
        .map_err(GuardError::Db)?;
    tx.commit().await.map_err(db_err)?;
    Ok(())
}

/// Set a goal's dependency edges (#251 reorder/re-parent). Validates that the
/// goal is still dependency-editable (Triage/Ready/Failed), that every dep is
/// an existing goal in the same project, and that the FULL project graph stays
/// acyclic with the new edges — then writes `depends_on` (a protected key;
/// this module is the legal writer) with an audit record.
pub async fn set_goal_dependencies(
    pool: &Pool<Sqlite>,
    card_id: &str,
    deps: &[String],
    actor: &str,
) -> Result<(), GuardError> {
    let card = crate::cards::get_card(pool, card_id)
        .await
        .map_err(GuardError::Db)?
        .ok_or_else(|| GuardError::NotFound(format!("Card '{}' not found", card_id)))?;
    if card.card_type != "goal" {
        return Err(GuardError::Invalid(format!(
            "Card '{}' is type '{}', not 'goal'",
            card_id, card.card_type
        )));
    }
    let col = crate::cards::get_column(pool, &card.column_id)
        .await
        .map_err(GuardError::Db)?
        .ok_or_else(|| GuardError::NotFound(format!("Column '{}' not found", card.column_id)))?;
    let binding = col.state_binding.as_deref().unwrap_or("");
    if !DEP_EDITABLE_STATES.contains(&binding) {
        return Err(GuardError::Invalid(format!(
            "Dependencies of goal '{}' cannot be edited in state '{}'. \
             Editable states: triage, ready, failed.",
            card.title, binding
        )));
    }

    let deps = validate_dep_ids(pool, &card.project_id, Some(card_id), deps).await?;

    // Full-graph cycle check with the proposed edges substituted in.
    let mut graph = load_goal_graph(pool, &card.project_id).await?;
    for node in &mut graph {
        if node.id == card_id {
            node.depends_on = deps.clone();
        }
    }
    crate::goal_state::validate_goal_graph(&graph)
        .map_err(|e| GuardError::Invalid(e.to_string()))?;

    write_depends_on_audited(pool, card_id, &deps, actor, "roadmap:set_deps").await?;

    let journal_actor = card
        .metadata_json
        .as_object()
        .map(|metadata| journal_actor(actor, metadata))
        .unwrap_or_else(|| actor.to_string());

    // Same-state emit so live board surfaces refresh their dependency view.
    crate::events::emit(crate::events::goal_state_changed(
        card_id,
        Some(&card.project_id),
        Some(binding),
        binding,
        &journal_actor,
    ));
    Ok(())
}

/// Splice a goal out of its roadmap graph (#251 remove): every dependent of
/// `card_id` inherits `card_id`'s own dependencies instead (deduped, no
/// self-edges), so the graph stays connected and topologically valid without
/// the removed node. Returns the number of dependents rewired. The caller
/// decides what happens to the removed card itself (cancel for non-terminal
/// goals; terminal cards are left in place).
pub async fn detach_goal_from_dependents(
    pool: &Pool<Sqlite>,
    card_id: &str,
    actor: &str,
) -> Result<u32, GuardError> {
    let card = crate::cards::get_card(pool, card_id)
        .await
        .map_err(GuardError::Db)?
        .ok_or_else(|| GuardError::NotFound(format!("Card '{}' not found", card_id)))?;
    if card.card_type != "goal" {
        return Err(GuardError::Invalid(format!(
            "Card '{}' is type '{}', not 'goal'",
            card_id, card.card_type
        )));
    }
    let inherited: Vec<String> = card
        .metadata_json
        .get("depends_on")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let graph = load_goal_graph(pool, &card.project_id).await?;
    let mut rewired = 0u32;
    for node in &graph {
        if node.id == card_id || !node.depends_on.iter().any(|d| d == card_id) {
            continue;
        }
        let mut new_deps: Vec<String> = node
            .depends_on
            .iter()
            .filter(|d| *d != card_id)
            .cloned()
            .collect();
        for dep in &inherited {
            if dep != &node.id && !new_deps.iter().any(|d| d == dep) {
                new_deps.push(dep.clone());
            }
        }
        write_depends_on_audited(pool, &node.id, &new_deps, actor, "roadmap:detach").await?;
        rewired += 1;
    }
    Ok(rewired)
}

/// Insert a new goal into an existing roadmap (#251). Dependencies are
/// validated (existing goals, same project); a brand-new node with only
/// outgoing edges cannot create a cycle, so no full-graph check is needed.
/// Root inserts (no deps) and inserts whose deps are all Complete advance
/// straight to Ready through the guard so auto-dispatch picks them up.
pub async fn insert_roadmap_goal(
    pool: &Pool<Sqlite>,
    project_id: &str,
    input: NewRoadmapGoal,
    actor: &str,
) -> Result<crate::cards::Card, GuardError> {
    let title = input.title.trim().to_string();
    if title.is_empty() {
        return Err(GuardError::Invalid("Goal title must not be empty".into()));
    }
    let deps = validate_dep_ids(pool, project_id, None, &input.depends_on).await?;

    crate::cards::seed_goal_columns(pool, project_id)
        .await
        .map_err(GuardError::Db)?;
    let triage_col = crate::cards::get_goal_column(pool, project_id, "triage")
        .await
        .map_err(GuardError::Db)?
        .ok_or_else(|| GuardError::NotFound("Triage column not found".to_string()))?;

    let mut meta = serde_json::Map::new();
    meta.insert("depends_on".to_string(), serde_json::json!(deps));
    meta.insert(
        "goal_state".to_string(),
        serde_json::Value::String("triage".to_string()),
    );
    meta.insert("attempt_count".to_string(), serde_json::json!(0));
    if !input.tags.is_empty() {
        meta.insert("tags".to_string(), serde_json::json!(input.tags));
    }
    if !input.acceptance_criteria.is_empty() {
        meta.insert(
            "acceptance_criteria".to_string(),
            serde_json::json!(input.acceptance_criteria),
        );
    }

    let card = crate::cards::create_card(
        pool,
        crate::cards::CreateCard {
            project_id: project_id.to_string(),
            title,
            description: input.description.clone(),
            card_type: Some("goal".to_string()),
            column_id: Some(triage_col.id.clone()),
            created_by: Some("user".to_string()),
            metadata_json: Some(serde_json::Value::Object(meta)),
        },
    )
    .await
    .map_err(GuardError::Db)?;

    // Audit the insert (tier-0 system record), attributed to whoever asked.
    //
    // This used to hardcode `ACTOR_JESSE`. That was correct for the only caller
    // that existed — the UI route — and wrong as a shape: `ACTOR_JESSE` is not
    // a default, it is the literal the Tier-2 human-only gate compares against
    // (see `decisions::risk_gate`). Baking it into a library function means the
    // first non-human caller to appear mints the human's attribution silently,
    // with nothing at the call site to notice. That is the same defect as the
    // `answer_decisions` finding, which relayed a model's verdict as the user.
    // The actor is the caller's fact to state, never this function's to assume.
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(db_err)?;
    decisions::append_audit_tx(
        &mut tx,
        "none",
        Some(&card.id),
        actor,
        0,
        "roadmap:insert",
        None,
    )
    .await
    .map_err(GuardError::Db)?;
    tx.commit().await.map_err(db_err)?;

    // Dependency-satisfied inserts become Ready immediately.
    let mut satisfied = true;
    for dep_id in &deps {
        let dep_binding: Option<String> = sqlx::query_scalar::<_, Option<String>>(
            "SELECT bc.state_binding FROM cards c JOIN board_columns bc ON c.column_id = bc.id
             WHERE c.id = ?",
        )
        .bind(dep_id)
        .fetch_optional(pool)
        .await
        .map_err(db_err)?
        .flatten();
        if dep_binding.as_deref() != Some("complete") {
            satisfied = false;
            break;
        }
    }
    if satisfied {
        advance_goal_checked(
            pool,
            &card.id,
            GoalAction::Ready,
            decisions::ACTOR_SYSTEM,
            None,
            TransitionEffects::default(),
        )
        .await?;
    }

    crate::cards::get_card(pool, &card.id)
        .await
        .map_err(GuardError::Db)?
        .ok_or_else(|| GuardError::NotFound("inserted goal vanished".to_string()))
}

// ── Per-goal auto-approve override (#252) ───────────────────────────────────

/// Set or clear a goal's `auto_approve` flag (#252) — the per-goal opt-in that
/// lets a VERIFIED PASS skip the manual Review answer (henry-policy answers
/// the Tier-1 approve_review decision, exactly like the goal-type allow-list
/// in verifier.json). The flag never blind-completes a goal: a FAIL/Uncertain
/// verdict or a Tier-2-dialed approval still holds in Review, and the DB
/// trigger still demands an answered approve decision.
///
/// `auto_approve` is a protected metadata key; this audited setter is its only
/// legal writer (reached from the user-facing HTTP toggle, not from the
/// orchestrator's card tools).
pub async fn set_goal_auto_approve(
    pool: &Pool<Sqlite>,
    card_id: &str,
    enabled: bool,
    actor: &str,
) -> Result<(), GuardError> {
    // BEGIN IMMEDIATE: read-then-write, same idiom as the other guards here.
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(db_err)?;
    let goal = read_goal_tx(&mut tx, card_id).await?;
    if goal.card_type != "goal" {
        return Err(GuardError::Invalid(format!(
            "Card '{}' is type '{}', not 'goal'",
            card_id, goal.card_type
        )));
    }
    let mut meta = goal.metadata;
    if enabled {
        meta.insert("auto_approve".to_string(), serde_json::Value::Bool(true));
    } else {
        meta.remove("auto_approve");
    }
    let meta_str = serde_json::to_string(&serde_json::Value::Object(meta))
        .map_err(|e| GuardError::Db(e.to_string()))?;
    sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
        .bind(&meta_str)
        .bind(card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    decisions::append_audit_tx(
        &mut tx,
        "none",
        Some(card_id),
        actor,
        0,
        if enabled {
            "auto_approve:enabled"
        } else {
            "auto_approve:disabled"
        },
        None,
    )
    .await
    .map_err(GuardError::Db)?;
    tx.commit().await.map_err(db_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decisions::{answer_decision, create_decision, DecisionAnswer, NewDecision};
    use crate::projects::PERSONAL_PROJECT_ID;

    async fn test_pool() -> Pool<Sqlite> {
        use crate::session::spectral_schema::init_spectral_db;
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    async fn goal_in_state(pool: &Pool<Sqlite>, state: &str, attempts: u64) -> crate::cards::Card {
        crate::cards::seed_goal_columns(pool, PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        let col = crate::cards::get_goal_column(pool, PERSONAL_PROJECT_ID, state)
            .await
            .unwrap()
            .unwrap();
        let mut meta = serde_json::Map::new();
        meta.insert("attempt_count".to_string(), serde_json::json!(attempts));
        meta.insert(
            "goal_state".to_string(),
            serde_json::Value::String(state.to_string()),
        );
        crate::cards::create_card(
            pool,
            crate::cards::CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: format!("Guard test goal in {}", state),
                description: Some("test".to_string()),
                card_type: Some("goal".to_string()),
                column_id: Some(col.id.clone()),
                created_by: None,
                metadata_json: Some(serde_json::Value::Object(meta)),
            },
        )
        .await
        .unwrap()
    }

    async fn state_of(pool: &Pool<Sqlite>, card_id: &str) -> String {
        let card = crate::cards::get_card(pool, card_id)
            .await
            .unwrap()
            .unwrap();
        let col = crate::cards::get_column(pool, &card.column_id)
            .await
            .unwrap()
            .unwrap();
        col.state_binding.unwrap_or_default()
    }

    /// F1a: metadata with no `execution_receipt` at all has nothing to
    /// report live — this is "found no contradiction", not "confirmed dead".
    #[test]
    fn execution_receipt_is_live_is_false_with_no_receipt_at_all() {
        assert!(!execution_receipt_is_live(
            &serde_json::json!({}),
            "life-1",
            chrono::Utc::now(),
        ));
    }

    /// F1a: a fresh, same-lifecycle receipt reads live; the identical receipt
    /// read from a DIFFERENT daemon lifecycle never does (restart recovery's
    /// own rebind-vs-stale contract).
    #[test]
    fn execution_receipt_is_live_true_for_a_fresh_same_lifecycle_receipt() {
        use crate::agents::platform_extensions::execution_receipt::ExecutionReceipt;
        let now = chrono::Utc::now();
        let receipt = ExecutionReceipt::new(
            "permagent",
            "sess-1",
            serde_json::json!({}),
            "life-1",
            now.to_rfc3339(),
            1,
        );
        let metadata = serde_json::json!({ "execution_receipt": receipt });
        assert!(execution_receipt_is_live(&metadata, "life-1", now));
        assert!(
            !execution_receipt_is_live(&metadata, "life-2", now),
            "a receipt dispatched under a different daemon lifecycle must never read as live"
        );
    }

    /// F1a: a terminal receipt (e.g. `StaleWorker`) is never live regardless
    /// of how recently it was touched — mirrors `ExecutionReceipt::is_live`'s
    /// own `rebind_vs_stale` unit test.
    #[test]
    fn execution_receipt_is_live_false_once_terminal() {
        use crate::agents::platform_extensions::execution_receipt::{
            ExecutionReceipt, ReceiptState,
        };
        let now = chrono::Utc::now();
        let mut receipt = ExecutionReceipt::new(
            "permagent",
            "sess-1",
            serde_json::json!({}),
            "life-1",
            now.to_rfc3339(),
            1,
        );
        receipt.finalize(ReceiptState::StaleWorker, now.to_rfc3339());
        let metadata = serde_json::json!({ "execution_receipt": receipt });
        assert!(!execution_receipt_is_live(&metadata, "life-1", now));
    }

    #[tokio::test]
    async fn system_transition_journals_worker_without_changing_audit_actor() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "ready", 0).await;
        let mut metadata = goal.metadata_json.as_object().unwrap().clone();
        metadata.insert("worker_key".to_string(), serde_json::json!("claude_code"));
        sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
            .bind(serde_json::Value::Object(metadata).to_string())
            .bind(&goal.id)
            .execute(&pool)
            .await
            .unwrap();

        let mut events = crate::events::subscribe();
        advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Dispatch,
            decisions::ACTOR_SYSTEM,
            None,
            TransitionEffects::default(),
        )
        .await
        .unwrap();

        let event = loop {
            let event = events.recv().await.unwrap();
            if event.payload.get("goal_id").and_then(|v| v.as_str()) == Some(goal.id.as_str()) {
                break event;
            }
        };
        assert_eq!(
            event.payload.get("actor").and_then(|v| v.as_str()),
            Some("claude_code")
        );
        assert_eq!(
            crate::activity_journal::entry_from_event(&event)
                .unwrap()
                .actor
                .as_str(),
            "claude_code"
        );
        let acted_by: String = sqlx::query_scalar(
            "SELECT acted_by FROM decision_audit WHERE goal_id = ? AND outcome = 'transition:dispatch'",
        )
        .bind(&goal.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(acted_by, decisions::ACTOR_SYSTEM);
    }

    async fn approve_decision_proof(
        pool: &Pool<Sqlite>,
        goal_id: &str,
        actor: &str,
    ) -> crate::decisions::DecisionProof {
        let d = create_decision(
            pool,
            NewDecision {
                kind: "approve_review".to_string(),
                goal_id: Some(goal_id.to_string()),
                project_id: Some(PERSONAL_PROJECT_ID.to_string()),
                headline: Some("Approve the finished work on the test goal".to_string()),
                detail: Some("evidence: tests pass".to_string()),
                payload: serde_json::json!({}),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (_, proof) = answer_decision(
            pool,
            &d.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            actor,
        )
        .await
        .unwrap();
        proof
    }

    // ── Tier gating ──

    #[tokio::test]
    async fn tier0_transitions_need_no_proof() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "triage", 0).await;
        let new = advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Ready,
            "system",
            None,
            TransitionEffects::default(),
        )
        .await
        .unwrap();
        assert_eq!(new, GoalState::Ready);
        assert_eq!(state_of(&pool, &goal.id).await, "ready");
    }

    #[tokio::test]
    async fn tier1_approve_without_decision_is_denied() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "review", 1).await;
        let err = advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Approve,
            "system",
            None,
            TransitionEffects::default(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, GuardError::Denied(_)), "{:?}", err);
        assert_eq!(state_of(&pool, &goal.id).await, "review", "no state change");
    }

    #[tokio::test]
    async fn tier1_approve_with_henry_decision_succeeds() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "review", 1).await;
        let proof = approve_decision_proof(&pool, &goal.id, crate::decisions::ACTOR_HENRY).await;
        let new = advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Approve,
            "henry-policy",
            Some(proof),
            TransitionEffects::default(),
        )
        .await
        .unwrap();
        assert_eq!(new, GoalState::Complete);
        assert_eq!(state_of(&pool, &goal.id).await, "complete");
    }

    #[tokio::test]
    async fn tier2_advance_without_jesse_decision_is_denied() {
        let pool = test_pool().await;
        // Raise approve to Tier 2 via the trust dial.
        sqlx::query("UPDATE risk_policy SET tier = 2 WHERE action_class = 'goal_approve_standard'")
            .execute(&pool)
            .await
            .unwrap();

        let goal = goal_in_state(&pool, "review", 1).await;

        // No decision at all → denied.
        let err = advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Approve,
            "system",
            None,
            TransitionEffects::default(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, GuardError::Denied(_)));

        // Henry-answered decision is NOT enough for Tier 2... but the decisions
        // module already refuses to mint a henry proof for a tier-2 decision,
        // so simulate the only remaining hole: a henry proof minted while the
        // policy said tier 1, replayed after the dial moved to tier 2.
        sqlx::query("UPDATE risk_policy SET tier = 1 WHERE action_class = 'goal_approve_standard'")
            .execute(&pool)
            .await
            .unwrap();
        let henry_proof =
            approve_decision_proof(&pool, &goal.id, crate::decisions::ACTOR_HENRY).await;
        sqlx::query("UPDATE risk_policy SET tier = 2 WHERE action_class = 'goal_approve_standard'")
            .execute(&pool)
            .await
            .unwrap();
        let err = advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Approve,
            "henry-policy",
            Some(henry_proof),
            TransitionEffects::default(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, GuardError::Denied(_)),
            "tier gate must re-check at transition time: {:?}",
            err
        );
        assert_eq!(state_of(&pool, &goal.id).await, "review");

        // user-answered decision passes.
        let jesse_proof =
            approve_decision_proof(&pool, &goal.id, crate::decisions::ACTOR_JESSE).await;
        let new = advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Approve,
            "jesse",
            Some(jesse_proof),
            TransitionEffects::default(),
        )
        .await
        .unwrap();
        assert_eq!(new, GoalState::Complete);
    }

    // ── Harness loop, Lane V2: headless auto-land, ACTOR_AUTO_LAUNCH at Tier 1
    // (the user's ruling, HARNESS_LOOP_LOG.md "JESSE RULING (2026-09-08 23:55):
    // Tier-1 landing → option 3") ──

    /// Mint a genuine `ACTOR_AUTO_LAUNCH` proof for an `approve_review`
    /// decision the same way production does: through
    /// `decisions::auto_approve_headless_goal_review`, never a hand-built
    /// `DecisionProof` — the general `answer_decision` path can never
    /// produce this actor (`VALID_ACTORS` excludes it), so this is the only
    /// legitimate way to obtain one in a test.
    async fn auto_land_decision_proof(
        pool: &Pool<Sqlite>,
        goal_id: &str,
        minting_session_id: Option<&str>,
    ) -> crate::decisions::DecisionProof {
        let payload = match minting_session_id {
            Some(session_id) => serde_json::json!({ "minting_session_id": session_id }),
            None => serde_json::json!({}),
        };
        let d = create_decision(
            pool,
            NewDecision {
                kind: "approve_review".to_string(),
                goal_id: Some(goal_id.to_string()),
                project_id: Some(PERSONAL_PROJECT_ID.to_string()),
                headline: Some("Review the finished work on the test goal".to_string()),
                detail: Some("evidence: durable verifier pass".to_string()),
                payload,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (_, proof) = crate::decisions::auto_approve_headless_goal_review(pool, &d.id, goal_id)
            .await
            .unwrap();
        proof
    }

    #[tokio::test]
    async fn tier1_auto_launch_lands_when_allowed_and_session_matches() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "review", 1).await;
        let proof = auto_land_decision_proof(&pool, &goal.id, Some("coord-session-1")).await;
        let new = advance_goal_checked_with_auto_land(
            &pool,
            &goal.id,
            GoalAction::Approve,
            decisions::ACTOR_AUTO_LAUNCH,
            Some(proof),
            TransitionEffects::default(),
            true,
            Some("coord-session-1"),
        )
        .await
        .unwrap();
        assert_eq!(new, GoalState::Complete);
        assert_eq!(state_of(&pool, &goal.id).await, "complete");
    }

    /// Pre-fix, this exact scenario (an `ACTOR_AUTO_LAUNCH` proof, switch
    /// off) fell through to the generic "requires jesse or henry-policy"
    /// message — dishonest about WHY it was refused, since a headless
    /// auto-land attempt genuinely happened. Post-fix it must name the
    /// switch.
    #[tokio::test]
    async fn tier1_auto_launch_is_denied_when_the_switch_is_off() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "review", 1).await;
        let proof = auto_land_decision_proof(&pool, &goal.id, Some("coord-session-1")).await;
        let err = advance_goal_checked_with_auto_land(
            &pool,
            &goal.id,
            GoalAction::Approve,
            decisions::ACTOR_AUTO_LAUNCH,
            Some(proof),
            TransitionEffects::default(),
            false,
            Some("coord-session-1"),
        )
        .await
        .unwrap_err();
        match err {
            GuardError::Denied(msg) => assert_eq!(
                msg, "headless auto-land is off (PERMAGENT_HEADLESS_AUTOLAND)",
                "the denial must name the switch, not the generic jesse/henry message"
            ),
            other => panic!("expected Denied, got {:?}", other),
        }
        assert_eq!(state_of(&pool, &goal.id).await, "review", "no state change");
    }

    /// Every pre-existing call site — daemon reconciliation included — only
    /// ever calls the plain [`advance_goal_checked`], which always passes
    /// `auto_land_allowed: false, calling_session_id: None`. No daemon code
    /// path can ever produce an `ACTOR_AUTO_LAUNCH` proof in the first place
    /// (only `auto_approve_headless_goal_review` mints one, and only
    /// `orchestrator::auto_land_headless_goal` calls it), but this pins that
    /// even if one were somehow presented here, the daemon's own call shape
    /// refuses it exactly like every other unset switch — unchanged from
    /// what a bare `advance_goal_checked` call has always done.
    #[tokio::test]
    async fn tier1_auto_launch_through_the_plain_advance_goal_checked_is_denied_like_a_daemon() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "review", 1).await;
        let proof = auto_land_decision_proof(&pool, &goal.id, Some("coord-session-1")).await;
        let err = advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Approve,
            decisions::ACTOR_AUTO_LAUNCH,
            Some(proof),
            TransitionEffects::default(),
        )
        .await
        .unwrap_err();
        match err {
            GuardError::Denied(msg) => assert_eq!(
                msg,
                "headless auto-land is off (PERMAGENT_HEADLESS_AUTOLAND)"
            ),
            other => panic!("expected Denied, got {:?}", other),
        }
        assert_eq!(state_of(&pool, &goal.id).await, "review");
    }

    #[tokio::test]
    async fn tier2_auto_launch_is_denied_even_with_the_switch_on() {
        let pool = test_pool().await;
        sqlx::query("UPDATE risk_policy SET tier = 2 WHERE action_class = 'goal_approve_standard'")
            .execute(&pool)
            .await
            .unwrap();
        let goal = goal_in_state(&pool, "review", 1).await;
        let proof = auto_land_decision_proof(&pool, &goal.id, Some("coord-session-1")).await;
        let err = advance_goal_checked_with_auto_land(
            &pool,
            &goal.id,
            GoalAction::Approve,
            decisions::ACTOR_AUTO_LAUNCH,
            Some(proof),
            TransitionEffects::default(),
            true,
            Some("coord-session-1"),
        )
        .await
        .unwrap_err();
        match err {
            GuardError::Denied(msg) => assert_eq!(
                msg,
                "Tier-2 transition requires a decision answered by 'jesse' (got 'auto-mode-launch')",
                "Tier 2 must stay human-only regardless of the auto-land switch"
            ),
            other => panic!("expected Denied, got {:?}", other),
        }
        assert_eq!(state_of(&pool, &goal.id).await, "review");
    }

    #[tokio::test]
    async fn tier1_auto_launch_is_denied_when_the_minting_session_does_not_match() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "review", 1).await;
        let proof = auto_land_decision_proof(&pool, &goal.id, Some("session-A")).await;
        let err = advance_goal_checked_with_auto_land(
            &pool,
            &goal.id,
            GoalAction::Approve,
            decisions::ACTOR_AUTO_LAUNCH,
            Some(proof),
            TransitionEffects::default(),
            true,
            Some("session-B"),
        )
        .await
        .unwrap_err();
        match err {
            GuardError::Denied(msg) => assert_eq!(
                msg,
                "headless auto-land proof is not bound to the calling session"
            ),
            other => panic!("expected Denied, got {:?}", other),
        }
        assert_eq!(state_of(&pool, &goal.id).await, "review");
    }

    /// A proof with no `minting_session_id` at all (e.g. minted before this
    /// field existed) must never be treated as bound to whatever session
    /// happens to call — absence on either side is a mismatch, not a
    /// wildcard match.
    #[tokio::test]
    async fn tier1_auto_launch_is_denied_when_no_minting_session_was_recorded() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "review", 1).await;
        let proof = auto_land_decision_proof(&pool, &goal.id, None).await;
        let err = advance_goal_checked_with_auto_land(
            &pool,
            &goal.id,
            GoalAction::Approve,
            decisions::ACTOR_AUTO_LAUNCH,
            Some(proof),
            TransitionEffects::default(),
            true,
            Some("coord-session-1"),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, GuardError::Denied(_)));
        assert_eq!(state_of(&pool, &goal.id).await, "review");
    }

    // Note: `check_proof`'s "only 'approve_review' decisions" kind check
    // (the third condition in its doc comment) has no dedicated test here.
    // `auto_approve_roadmap_launch` — the only OTHER function that can ever
    // mint an `ACTOR_AUTO_LAUNCH` proof — hardcodes `answer: "choice"`, which
    // already fails this function's earlier, pre-existing
    // "Action 'approve' requires a decision answered 'approve'" check before
    // the tier match is ever reached; no legitimate minter can produce a
    // proof that clears the answer check with any kind other than
    // `approve_review`, and `DecisionProof`'s private fields mean no test
    // outside `decisions.rs` can hand-build one that would. The check is
    // still correct defense-in-depth against a future minter, verified by
    // code review rather than by an executable pin.

    #[tokio::test]
    async fn proof_for_other_goal_is_rejected() {
        let pool = test_pool().await;
        let goal_a = goal_in_state(&pool, "review", 1).await;
        let goal_b = goal_in_state(&pool, "review", 1).await;
        let proof_b =
            approve_decision_proof(&pool, &goal_b.id, crate::decisions::ACTOR_JESSE).await;
        let err = advance_goal_checked(
            &pool,
            &goal_a.id,
            GoalAction::Approve,
            "jesse",
            Some(proof_b),
            TransitionEffects::default(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, GuardError::Denied(_)));
    }

    #[tokio::test]
    async fn reject_answer_cannot_authorize_approve() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "review", 0).await;
        let d = create_decision(
            &pool,
            NewDecision {
                kind: "approve_review".to_string(),
                goal_id: Some(goal.id.clone()),
                headline: Some("Approve the finished work on the test goal".to_string()),
                detail: Some("evidence".to_string()),
                payload: serde_json::json!({}),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (_, proof) = answer_decision(
            &pool,
            &d.id,
            &DecisionAnswer {
                answer: "reject".to_string(),
                ..Default::default()
            },
            crate::decisions::ACTOR_JESSE,
        )
        .await
        .unwrap();
        let err = advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Approve,
            "jesse",
            Some(proof),
            TransitionEffects::default(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, GuardError::Denied(_)));
    }

    #[tokio::test]
    async fn invalid_transition_rejected_before_gates() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "triage", 0).await;
        let err = advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Approve,
            "system",
            None,
            TransitionEffects::default(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, GuardError::Invalid(_)));
    }

    #[tokio::test]
    async fn headless_dispatch_budget_is_atomic_and_not_refunded() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "ready", 0).await;
        let claim = || {
            advance_goal_checked(
                &pool,
                &goal.id,
                GoalAction::Dispatch,
                "system",
                None,
                TransitionEffects {
                    metadata_patch: serde_json::Map::from_iter([(
                        "attempt_count".into(),
                        serde_json::json!(1),
                    )]),
                    dispatch_budget: Some(HeadlessDispatchBudget {
                        coordinator_session_id: "coordinator".into(),
                        automatic_limit: 1,
                    }),
                    ..Default::default()
                },
            )
        };
        let (first, second) = tokio::join!(claim(), claim());
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        // The old goal-attempt counter can be refunded; the native dispatch
        // authorization is consumed by the actual transaction regardless.
        maybe_forgive_zero_progress_attempt(&pool, &goal.id, true)
            .await
            .unwrap();
        requeue_goal(&pool, &goal.id, "system", 0, "zero progress refunded")
            .await
            .unwrap();
        let metadata = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap()
            .metadata_json;
        assert_eq!(metadata[HEADLESS_DISPATCH_BUDGET_KEY]["dispatch_count"], 1);
        assert_eq!(metadata["attempt_count"], 0);
        let failure = claim().await.unwrap_err();
        assert!(failure.to_string().contains("allowance exhausted"));
        assert!(crate::cards::merge_card_metadata(
            &pool,
            &goal.id,
            serde_json::json!({HEADLESS_DISPATCH_BUDGET_KEY: null}),
            false,
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn stale_exhausted_dispatch_park_preserves_authorized_dispatch_and_unblock() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "ready", 0).await;
        let budget = HeadlessDispatchBudget {
            coordinator_session_id: "coordinator".into(),
            automatic_limit: 1,
        };
        let mut metadata = goal.metadata_json.as_object().unwrap().clone();
        claim_headless_dispatch_budget(&mut metadata, &budget).unwrap();
        sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
            .bind(serde_json::Value::Object(metadata).to_string())
            .bind(&goal.id)
            .execute(&pool)
            .await
            .unwrap();
        let exhausted = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap()
            .metadata_json;
        assert!(check_headless_dispatch_budget(&exhausted, &budget).is_err());
        // The approved unblock committed between the preflight and park.
        let mut patch = serde_json::Map::new();
        extend_headless_dispatch_budget(&exhausted, &mut patch).unwrap();
        let mut refreshed = exhausted.as_object().unwrap().clone();
        apply_patch(&mut refreshed, &patch);
        sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
            .bind(serde_json::Value::Object(refreshed).to_string())
            .bind(&goal.id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            !park_exhausted_headless_dispatch(&pool, &goal.id, &budget, "stale cap")
                .await
                .unwrap()
        );
        assert_eq!(state_of(&pool, &goal.id).await, "ready");
        // Its authorized dispatch committed before another stale park.
        advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Dispatch,
            "system",
            None,
            TransitionEffects {
                dispatch_budget: Some(budget),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let stale_budget = HeadlessDispatchBudget {
            coordinator_session_id: "coordinator".into(),
            automatic_limit: 1,
        };
        assert!(
            !park_exhausted_headless_dispatch(&pool, &goal.id, &stale_budget, "stale cap")
                .await
                .unwrap()
        );
        assert_eq!(state_of(&pool, &goal.id).await, "in_progress");
    }

    #[test]
    fn headless_dispatch_budget_is_run_scoped_and_extends_only_explicitly() {
        let budget = HeadlessDispatchBudget {
            coordinator_session_id: "coordinator".into(),
            automatic_limit: 1,
        };
        let mut meta = serde_json::Map::new();
        claim_headless_dispatch_budget(&mut meta, &budget).unwrap();
        let snapshot = serde_json::Value::Object(meta.clone());
        assert!(check_headless_dispatch_budget(&snapshot, &budget).is_err());
        let mut patch = serde_json::Map::new();
        assert!(extend_headless_dispatch_budget(&snapshot, &mut patch).unwrap());
        apply_patch(&mut meta, &patch);
        claim_headless_dispatch_budget(&mut meta, &budget).unwrap();
        assert_eq!(meta[HEADLESS_DISPATCH_BUDGET_KEY]["dispatch_count"], 2);
        let other = HeadlessDispatchBudget {
            coordinator_session_id: "other-coordinator".into(),
            automatic_limit: 1,
        };
        claim_headless_dispatch_budget(&mut meta, &other).unwrap();
        assert_eq!(meta[HEADLESS_DISPATCH_BUDGET_KEY]["dispatch_count"], 1);
        assert_eq!(
            meta[HEADLESS_DISPATCH_BUDGET_KEY]["approved_extra_dispatches"],
            0
        );
        assert!(check_headless_dispatch_budget(
            &serde_json::json!({HEADLESS_DISPATCH_BUDGET_KEY: {}}),
            &budget
        )
        .is_err());
    }

    #[tokio::test]
    async fn concurrent_dispatch_claim_has_exactly_one_winner() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "ready", 0).await;
        let claim = |claim_id: &'static str| {
            let pool = pool.clone();
            let goal_id = goal.id.clone();
            async move {
                let mut patch = serde_json::Map::new();
                patch.insert("dispatch_claim_id".to_string(), serde_json::json!(claim_id));
                advance_goal_checked(
                    &pool,
                    &goal_id,
                    GoalAction::Dispatch,
                    "system",
                    None,
                    TransitionEffects {
                        metadata_patch: patch,
                        ..Default::default()
                    },
                )
                .await
            }
        };

        let (first, second) = tokio::join!(claim("claim-a"), claim("claim-b"));
        assert_eq!(
            usize::from(first.is_ok()) + usize::from(second.is_ok()),
            1,
            "the Ready → InProgress transaction must authorize exactly one dispatcher"
        );

        let updated = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        let winning_claim = updated
            .metadata_json
            .get("dispatch_claim_id")
            .and_then(|value| value.as_str())
            .unwrap();
        assert!(winning_claim == "claim-a" || winning_claim == "claim-b");
    }

    // ── Defense-in-depth trigger ──

    #[tokio::test]
    async fn raw_sql_move_to_complete_without_decision_is_blocked_by_trigger() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "review", 1).await;
        let complete_col = crate::cards::get_goal_column(&pool, PERSONAL_PROJECT_ID, "complete")
            .await
            .unwrap()
            .unwrap();

        let result = sqlx::query("UPDATE cards SET column_id = ? WHERE id = ?")
            .bind(&complete_col.id)
            .bind(&goal.id)
            .execute(&pool)
            .await;
        assert!(
            result.is_err(),
            "raw column update into complete must be blocked by trg_goal_complete_guard"
        );
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("answered approve decision"), "{}", msg);
    }

    // ── Audit ──

    #[tokio::test]
    async fn every_checked_transition_is_audited_and_chain_verifies() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "triage", 0).await;
        advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Ready,
            "system",
            None,
            TransitionEffects::default(),
        )
        .await
        .unwrap();
        advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Dispatch,
            "system",
            None,
            TransitionEffects::default(),
        )
        .await
        .unwrap();

        let outcomes: Vec<String> = sqlx::query_scalar(
            "SELECT outcome FROM decision_audit WHERE goal_id = ? ORDER BY seq ASC",
        )
        .bind(&goal.id)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(outcomes, vec!["transition:ready", "transition:dispatch"]);

        let report = crate::decisions::verify_audit_chain(&pool).await.unwrap();
        assert!(report.intact, "{}", report.detail);
    }

    // ── Budgets ──

    #[tokio::test]
    async fn budget_defaults_and_overrides() {
        let meta = serde_json::json!({});
        let b = goal_budget(&meta);
        assert_eq!(b.attempt_cap, DEFAULT_ATTEMPT_CAP);
        assert!(b.token_budget.is_none());

        let meta = serde_json::json!({
            "budget": {"attempt_cap": 5, "token_budget": 100000, "wallclock_cap_secs": 3600}
        });
        let b = goal_budget(&meta);
        assert_eq!(b.attempt_cap, 5);
        assert_eq!(b.token_budget, Some(100000));
        assert_eq!(b.wallclock_cap_secs, Some(3600));
    }

    #[tokio::test]
    async fn worker_token_accounting_fails_closed_for_unknown_or_null_sessions() {
        let pool = test_pool().await;
        assert_eq!(
            goal_spent_tokens(&pool, &serde_json::json!({}))
                .await
                .unwrap(),
            0
        );

        let unknown = goal_spent_tokens(
            &pool,
            &serde_json::json!({"worker_session_ids": ["missing-worker"]}),
        )
        .await
        .unwrap_err();
        assert!(unknown.contains("do not exist"), "{unknown}");

        sqlx::query("INSERT INTO sessions (id, working_dir) VALUES ('worker-null', '/tmp')")
            .execute(&pool)
            .await
            .unwrap();
        let null_total = goal_spent_tokens(
            &pool,
            &serde_json::json!({"worker_session_ids": ["worker-null"]}),
        )
        .await
        .unwrap_err();
        assert!(
            null_total.contains("no accumulated token total"),
            "{null_total}"
        );

        // A token-budget redispatch must not treat a missing worker total as
        // zero and proceed. The error is intentionally propagated by the
        // budget guard instead of producing a fabricated exhaustion result.
        let budget_error = check_budget(
            &pool,
            &serde_json::json!({
                "attempt_count": 1,
                "budget": {"token_budget": 1000},
                "worker_session_ids": ["worker-null"]
            }),
        )
        .await
        .unwrap_err();
        assert!(budget_error.contains("unknown"), "{budget_error}");

        sqlx::query("UPDATE sessions SET accumulated_total_tokens = 42 WHERE id = 'worker-null'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            goal_spent_tokens(
                &pool,
                &serde_json::json!({"worker_session_ids": ["worker-null"]}),
            )
            .await
            .unwrap(),
            42
        );
    }

    #[tokio::test]
    async fn exhaustion_emits_unblock_decision_and_parks_never_silent_retry() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 3).await;

        let exhaustion = check_budget(&pool, &goal.metadata_json).await.unwrap();
        let exhaustion = exhaustion.expect("3 attempts at default cap 3 is exhausted");
        assert!(matches!(exhaustion, BudgetExhaustion::AttemptCap { .. }));

        let decision_id = exhaust_and_park(
            &pool,
            &goal.id,
            "Guard test goal in in_progress",
            PERSONAL_PROJECT_ID,
            exhaustion,
            Some("worker crashed"),
        )
        .await
        .unwrap();

        // Goal is parked in the visible Failed column (#250), not retried and
        // not re-pooled into Triage.
        assert_eq!(state_of(&pool, &goal.id).await, "failed");
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            card.metadata_json.get("needs_human_attention"),
            Some(&serde_json::Value::Bool(true))
        );

        // An open unblock decision exists.
        let d = crate::decisions::get_decision(&pool, &decision_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(d.kind, "unblock");
        assert_eq!(d.status, "open");
        assert_eq!(d.goal_id.as_deref(), Some(goal.id.as_str()));

        // Re-exhausting dedupes onto the same open decision.
        let again = exhaust_and_park(
            &pool,
            &goal.id,
            "Guard test goal in in_progress",
            PERSONAL_PROJECT_ID,
            BudgetExhaustion::AttemptCap { spent: 3, cap: 3 },
            None,
        )
        .await;
        // park_goal of an already-failed goal is a failed->failed write; it must not error.
        let again = again.unwrap();
        assert_eq!(again, decision_id, "must not create a duplicate decision");
    }

    /// The attempt that trips the cap is computed in flight by the caller
    /// (`new_attempt = attempt_count + 1`) and quoted in the unblock decision as
    /// "3/3 attempts" — but nothing wrote it back, so the card kept saying 2.
    /// Every parked goal undercounted its own attempts by exactly one, and any
    /// reconciler reading the card believed the smaller number.
    #[tokio::test]
    async fn parking_records_the_attempt_that_tripped_the_cap() {
        let pool = test_pool().await;
        // The card as the caller finds it: two attempts persisted, the third
        // only in the caller's local variable.
        let goal = goal_in_state(&pool, "in_progress", 2).await;

        exhaust_and_park(
            &pool,
            &goal.id,
            "Guard test goal in in_progress",
            PERSONAL_PROJECT_ID,
            BudgetExhaustion::AttemptCap { spent: 3, cap: 3 },
            Some("worker session ended before the goal completed"),
        )
        .await
        .unwrap();

        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            card.metadata_json
                .get("attempt_count")
                .and_then(|v| v.as_u64()),
            Some(3),
            "the card must agree with the decision text it was parked with"
        );
        // The rest of the park is unchanged.
        assert_eq!(state_of(&pool, &goal.id).await, "failed");
        assert_eq!(
            card.metadata_json.get("needs_human_attention"),
            Some(&serde_json::Value::Bool(true))
        );
    }

    /// Non-attempt exhaustion (tokens, wallclock) says nothing about attempts,
    /// so it must not rewrite the count with a token total.
    #[tokio::test]
    async fn a_token_budget_park_leaves_the_attempt_count_alone() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 2).await;

        exhaust_and_park(
            &pool,
            &goal.id,
            "Guard test goal in in_progress",
            PERSONAL_PROJECT_ID,
            BudgetExhaustion::TokenBudget {
                spent: 120_000,
                cap: 100_000,
            },
            None,
        )
        .await
        .unwrap();

        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            card.metadata_json
                .get("attempt_count")
                .and_then(|v| v.as_u64()),
            Some(2)
        );
    }

    // ── Decision Inbox modal redesign (D1/D2/D7/D8) ─────────────────────────

    async fn set_meta(pool: &Pool<Sqlite>, card_id: &str, path: &str, value: serde_json::Value) {
        sqlx::query(
            "UPDATE cards SET metadata_json = json_set(metadata_json, ?, json(?)) WHERE id = ?",
        )
        .bind(path)
        .bind(value.to_string())
        .bind(card_id)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn answer_and_apply(
        pool: &Pool<Sqlite>,
        decision_id: &str,
        answer: &str,
        actor: &str,
    ) -> crate::decisions_effects::EffectResult {
        answer_decision(
            pool,
            decision_id,
            &DecisionAnswer {
                answer: answer.to_string(),
                ..Default::default()
            },
            actor,
        )
        .await
        .unwrap();
        let answered = crate::decisions::get_decision(pool, decision_id)
            .await
            .unwrap()
            .unwrap();
        let proof = crate::decisions::DecisionProof::from_answered_row(&answered).unwrap();
        crate::decisions_effects::apply_decision_effect(pool, &answered, proof, "unblock")
            .await
            .unwrap()
    }

    /// D1 + D2: a per-attempt timeout park no longer quotes (or cuts) the goal
    /// title in its headline, and its payload names the cause and carries the
    /// salvage receipt this park quotes — enough for the inbox's summary.
    #[tokio::test]
    async fn attempt_timeout_park_is_title_free_and_structured() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 1).await;
        let note = "no commits past the dispatch baseline — nothing to salvage";
        set_meta(
            &pool,
            &goal.id,
            "$.salvage_receipt",
            serde_json::json!({"outcome": "salvage_skipped_no_commits", "note": note}),
        )
        .await;

        let id = exhaust_and_park(
            &pool,
            &goal.id,
            &goal.title,
            PERSONAL_PROJECT_ID,
            BudgetExhaustion::Wallclock {
                spent_secs: 240,
                cap_secs: 240,
            },
            Some(&format!("worker exceeded its time bound; salvage: {note}")),
        )
        .await
        .unwrap();
        let d = crate::decisions::get_decision(&pool, &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(d.kind, "unblock");
        assert_eq!(d.headline, "Goal ran out of time and needs your direction");
        assert!(!d.headline.contains(&goal.title));
        assert_eq!(d.payload["cause"], "attempt_timeout");
        assert_eq!(
            d.payload["salvage"]["outcome"],
            "salvage_skipped_no_commits"
        );
        assert_eq!(d.payload["salvage"]["commits_past_baseline"], 0);
        assert_eq!(
            crate::decisions::present_decision(&d).summary.as_deref(),
            Some(
                "Ran out of time: each attempt gets 4 min, and this one used all of it. \
                 It made no changes worth keeping."
            )
        );
    }

    /// D2: a receipt left by an EARLIER park is never copied onto a later one.
    #[test]
    fn salvage_facts_only_come_from_the_receipt_this_park_quotes() {
        let meta = serde_json::json!({"salvage_receipt": {
            "outcome": "salvage_verify_failed",
            "note": "2 commit(s) past baseline, verify failed"
        }});
        assert_eq!(salvage_facts_for_park(&meta, Some("worker crashed")), None);
        assert_eq!(salvage_facts_for_park(&meta, None), None);
        let facts = salvage_facts_for_park(
            &meta,
            Some("timed out; salvage: 2 commit(s) past baseline, verify failed"),
        )
        .unwrap();
        assert_eq!(facts.outcome, "salvage_verify_failed");
        assert_eq!(facts.commits_past_baseline, Some(2));
    }

    /// D7: `BudgetExhaustion::Wallclock` from `check_budget` is measured from
    /// `first_dispatched_at` — the goal's FIRST dispatch, not the attempt.
    /// Raising the attempt cap alone therefore re-parks on the very next
    /// dispatch. Approving must also restart that clock.
    #[tokio::test]
    async fn approving_a_wall_budget_park_restarts_the_goal_clock() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 1).await;
        let two_hours_ago = (chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
        set_meta(
            &pool,
            &goal.id,
            "$.first_dispatched_at",
            serde_json::json!(two_hours_ago),
        )
        .await;
        set_meta(
            &pool,
            &goal.id,
            "$.budget",
            serde_json::json!({"wallclock_cap_secs": 600}),
        )
        .await;
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        let exhaustion = check_budget(&pool, &card.metadata_json)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            exhaustion,
            BudgetExhaustion::Wallclock { cap_secs: 600, .. }
        ));

        let id = exhaust_and_park(
            &pool,
            &goal.id,
            &goal.title,
            PERSONAL_PROJECT_ID,
            exhaustion,
            Some("worker crashed"),
        )
        .await
        .unwrap();
        let d = crate::decisions::get_decision(&pool, &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(d.payload["cause"], "wall_budget");

        let (effect, _) =
            answer_and_apply(&pool, &id, "approve", crate::decisions::ACTOR_JESSE).await;
        assert!(effect.unwrap().contains("wall-clock budget restarted"));
        assert_eq!(state_of(&pool, &goal.id).await, "ready");
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert!(card.metadata_json.get("first_dispatched_at").is_none());
        assert!(
            check_budget(&pool, &card.metadata_json)
                .await
                .unwrap()
                .is_none(),
            "the next dispatch must not re-park on the old clock"
        );
    }

    /// D8: rejecting a goal's unblock cancels the goal (it used to leave a
    /// parked goal with no open question). An automated actor's reject never
    /// cancels anything.
    #[tokio::test]
    async fn rejecting_a_goal_unblock_cancels_it_only_for_the_user() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 3).await;
        let id = exhaust_and_park(
            &pool,
            &goal.id,
            &goal.title,
            PERSONAL_PROJECT_ID,
            BudgetExhaustion::AttemptCap { spent: 3, cap: 3 },
            None,
        )
        .await
        .unwrap();
        let (effect, _) =
            answer_and_apply(&pool, &id, "reject", crate::decisions::ACTOR_JESSE).await;
        assert_eq!(effect.as_deref(), Some("goal cancelled at your request"));
        assert_eq!(state_of(&pool, &goal.id).await, "cancelled");

        let other = goal_in_state(&pool, "in_progress", 3).await;
        let id = exhaust_and_park(
            &pool,
            &other.id,
            &other.title,
            PERSONAL_PROJECT_ID,
            BudgetExhaustion::AttemptCap { spent: 3, cap: 3 },
            None,
        )
        .await
        .unwrap();
        let (effect, _) =
            answer_and_apply(&pool, &id, "reject", crate::decisions::ACTOR_HENRY).await;
        assert!(effect.unwrap().contains("stays parked"));
        assert_eq!(state_of(&pool, &other.id).await, "failed");
    }

    /// An open unblock filed for a goal that is still running — an
    /// interactive worker's `escalate` question, a stalled-gate check.
    async fn unblock_for(pool: &Pool<Sqlite>, goal_id: &str) -> String {
        create_decision(
            pool,
            NewDecision {
                kind: "unblock".to_string(),
                goal_id: Some(goal_id.to_string()),
                project_id: Some(PERSONAL_PROJECT_ID.to_string()),
                headline: Some("Should the worker switch the database?".to_string()),
                detail: Some("Should the worker switch the database?".to_string()),
                payload: serde_json::json!({"reason": "stuck"}),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id
    }

    /// Inbox fix group: rejecting a goal-bound unblock cancels the goal ONLY
    /// when the goal is paused on that decision. A running goal's unblock is a
    /// worker's question; "no" is an answer to record, never a reason to
    /// cancel the goal and kill its live worker.
    #[tokio::test]
    async fn rejecting_an_unblock_a_goal_is_not_paused_on_never_cancels_it() {
        let pool = test_pool().await;
        let running = goal_in_state(&pool, "in_progress", 1).await;
        let question = unblock_for(&pool, &running.id).await;
        let (effect, _) =
            answer_and_apply(&pool, &question, "reject", crate::decisions::ACTOR_JESSE).await;
        assert!(
            effect.unwrap().contains("not paused on this decision"),
            "a running goal's reject is recorded only"
        );
        assert_eq!(state_of(&pool, &running.id).await, "in_progress");

        // A parked goal paused on a DIFFERENT, still-open unblock: the stale
        // one's reject is recorded, and the goal waits on the open one.
        let parked = goal_in_state(&pool, "in_progress", 1).await;
        let stale = unblock_for(&pool, &parked.id).await;
        park_goal(&pool, &parked.id, crate::decisions::ACTOR_SYSTEM, "parked")
            .await
            .unwrap();
        let current = unblock_for(&pool, &parked.id).await;
        let (effect, _) =
            answer_and_apply(&pool, &stale, "reject", crate::decisions::ACTOR_JESSE).await;
        assert!(effect.unwrap().contains("not paused on this decision"));
        assert_eq!(state_of(&pool, &parked.id).await, "failed");
        let open = crate::decisions::get_decision(&pool, &current)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(open.status, "open", "the goal's real question stays open");
    }

    /// Answer as the chat `answer_decisions` tool does: the agent actor, the
    /// chat-relay audit principal, a declared session.
    async fn answer_via_chat(
        pool: &Pool<Sqlite>,
        decision_id: &str,
        answer: &str,
    ) -> crate::decisions_effects::EffectResult {
        let (answered, proof) = crate::decisions::answer_decision_as_session(
            pool,
            decision_id,
            &DecisionAnswer {
                answer: answer.to_string(),
                ..Default::default()
            },
            crate::decisions::ACTOR_HENRY,
            crate::decisions_effects::CHAT_RELAY_PRINCIPAL,
            "chat-session-under-test",
        )
        .await
        .unwrap();
        match crate::decisions_effects::apply_answered_effect_now(pool, &answered, proof)
            .await
            .unwrap()
        {
            crate::decisions_effects::AnsweredEffect::Ran(result) => result.unwrap(),
            _ => panic!("an unblock's effect runs inline"),
        }
    }

    /// Inbox fix group: the user's verdict relayed through chat gets exactly
    /// the effect of the same tap in the inbox — the reject-cancel (D8) and
    /// the wall-clock reset (D7) used to key on `acted_by == jesse`, which the
    /// chat tool (acting as the agent) never is.
    #[tokio::test]
    async fn a_verdict_relayed_through_chat_has_the_inbox_effect() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 3).await;
        let id = exhaust_and_park(
            &pool,
            &goal.id,
            &goal.title,
            PERSONAL_PROJECT_ID,
            BudgetExhaustion::AttemptCap { spent: 3, cap: 3 },
            None,
        )
        .await
        .unwrap();
        let (effect, _) = answer_via_chat(&pool, &id, "reject").await;
        assert_eq!(effect.as_deref(), Some("goal cancelled at your request"));
        assert_eq!(state_of(&pool, &goal.id).await, "cancelled");
        let outbox: String =
            sqlx::query_scalar("SELECT status FROM effect_outbox WHERE claim_key = ?")
                .bind(crate::decisions::effect_outbox_claim_key(&id, "unblock").unwrap())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(outbox, "applied", "the shared path settles the outbox row");

        let timed_out = goal_in_state(&pool, "in_progress", 1).await;
        let long_ago = (chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
        set_meta(
            &pool,
            &timed_out.id,
            "$.first_dispatched_at",
            serde_json::json!(long_ago),
        )
        .await;
        set_meta(
            &pool,
            &timed_out.id,
            "$.budget",
            serde_json::json!({"wallclock_cap_secs": 600}),
        )
        .await;
        let id = exhaust_and_park(
            &pool,
            &timed_out.id,
            &timed_out.title,
            PERSONAL_PROJECT_ID,
            BudgetExhaustion::Wallclock {
                spent_secs: 7200,
                cap_secs: 600,
            },
            Some("worker crashed"),
        )
        .await
        .unwrap();
        let (effect, _) = answer_via_chat(&pool, &id, "approve").await;
        assert!(effect.unwrap().contains("wall-clock budget restarted"));
        let card = crate::cards::get_card(&pool, &timed_out.id)
            .await
            .unwrap()
            .unwrap();
        assert!(card.metadata_json.get("first_dispatched_at").is_none());
        assert!(check_budget(&pool, &card.metadata_json)
            .await
            .unwrap()
            .is_none());
    }

    /// Inbox fix group: `exhaust_and_park` dedupes a wall-budget park onto
    /// any open unblock and keeps THAT decision's payload. The reset used to
    /// key on the answered decision's reason, so approving a reused worker
    /// question (reason `stuck`) raised the attempt cap but left the spent
    /// clock — and the next dispatch re-parked the goal at once.
    #[tokio::test]
    async fn approving_a_deduped_wall_budget_park_still_restarts_the_clock() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 1).await;
        let question = unblock_for(&pool, &goal.id).await;
        let long_ago = (chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
        set_meta(
            &pool,
            &goal.id,
            "$.first_dispatched_at",
            serde_json::json!(long_ago),
        )
        .await;
        set_meta(
            &pool,
            &goal.id,
            "$.budget",
            serde_json::json!({"wallclock_cap_secs": 600}),
        )
        .await;
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        let exhaustion = check_budget(&pool, &card.metadata_json)
            .await
            .unwrap()
            .unwrap();
        let id = exhaust_and_park(
            &pool,
            &goal.id,
            &goal.title,
            PERSONAL_PROJECT_ID,
            exhaustion,
            Some("worker crashed"),
        )
        .await
        .unwrap();
        assert_eq!(id, question, "the park reused the open worker question");

        let (effect, _) =
            answer_and_apply(&pool, &id, "approve", crate::decisions::ACTOR_JESSE).await;
        assert!(effect.unwrap().contains("wall-clock budget restarted"));
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            check_budget(&pool, &card.metadata_json)
                .await
                .unwrap()
                .is_none(),
            "the next dispatch must not re-park on the old clock"
        );
    }

    /// Inbox fix group: approving a token-budget park must lift the token
    /// budget. `check_budget` sums every worker session the goal has used, so
    /// raising only the attempt cap re-parked the goal on its next dispatch
    /// while the copy promised it would wake with more attempts.
    #[tokio::test]
    async fn approving_a_token_budget_park_grants_a_fresh_allowance() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 1).await;
        let session = format!("token-park-worker-{}", uuid::Uuid::new_v4());
        sqlx::query(
            "INSERT INTO sessions (id, working_dir, accumulated_total_tokens) \
             VALUES (?, '/tmp', 120000)",
        )
        .bind(&session)
        .execute(&pool)
        .await
        .unwrap();
        set_meta(
            &pool,
            &goal.id,
            "$.worker_session_id",
            serde_json::json!(session),
        )
        .await;
        set_meta(
            &pool,
            &goal.id,
            "$.budget",
            serde_json::json!({"token_budget": 100000}),
        )
        .await;
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        let exhaustion = check_budget(&pool, &card.metadata_json)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(exhaustion, BudgetExhaustion::TokenBudget { .. }));
        let id = exhaust_and_park(
            &pool,
            &goal.id,
            &goal.title,
            PERSONAL_PROJECT_ID,
            exhaustion,
            None,
        )
        .await
        .unwrap();
        let d = crate::decisions::get_decision(&pool, &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            crate::decisions::present_decision(&d).effects["approve"],
            "Wakes the goal with a fresh token allowance and more attempts."
        );

        let (effect, _) =
            answer_and_apply(&pool, &id, "approve", crate::decisions::ACTOR_JESSE).await;
        assert!(effect
            .unwrap()
            .contains("a fresh allowance of 100000 tokens"));
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(goal_budget(&card.metadata_json).token_budget, Some(220000));
        assert!(
            check_budget(&pool, &card.metadata_json)
                .await
                .unwrap()
                .is_none(),
            "the next dispatch must not re-park on the spent tokens"
        );
    }

    /// Inbox fix group: a merge-conflict unblock is filed after the goal
    /// completed. Neither answer wakes or cancels anything, and the copy must
    /// not say they do.
    #[tokio::test]
    async fn a_merge_conflict_unblock_promises_only_what_its_answers_do() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "complete", 1).await;
        let payload = serde_json::json!({"reason": "stuck", "cause": "merge_conflict"});
        let effects = crate::decisions_effects::answer_effects("unblock", &payload, true);
        assert!(
            !effects["approve"].contains("Wakes"),
            "{}",
            effects["approve"]
        );
        assert!(
            !effects["reject"].contains("Cancels"),
            "{}",
            effects["reject"]
        );
        for answer in ["approve", "reject"] {
            let id = create_decision(
                &pool,
                NewDecision {
                    kind: "unblock".to_string(),
                    goal_id: Some(goal.id.clone()),
                    project_id: Some(PERSONAL_PROJECT_ID.to_string()),
                    headline: Some("Goal work could not land and needs a manual merge".to_string()),
                    detail: Some("could not be fast-forwarded onto trunk".to_string()),
                    payload: payload.clone(),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .id;
            answer_and_apply(&pool, &id, answer, crate::decisions::ACTOR_JESSE).await;
            assert_eq!(state_of(&pool, &goal.id).await, "complete", "{answer}");
        }
    }

    /// Inbox fix group: the salvage outcomes that are NOT "no changes" keep
    /// their meaning in the structured facts — already landed stays 0 unlanded
    /// commits under its own outcome, a missing baseline measured nothing, and
    /// a quarantine clause after the receipt note is carried as a flag.
    #[test]
    fn salvage_facts_keep_landed_unmeasured_and_quarantined_work_apart() {
        let landed_note = "3 commit(s) past the rolling redispatch baseline, but the branch \
                           tip is already an ancestor of the project's trunk — nothing left to land";
        let meta = serde_json::json!({"salvage_receipt": {
            "outcome": "salvage_skipped_already_landed", "note": landed_note
        }});
        let facts = salvage_facts_for_park(
            &meta,
            Some(&format!(
                "worker exceeded its time bound; salvage: {landed_note}"
            )),
        )
        .unwrap();
        assert_eq!(facts.outcome, "salvage_skipped_already_landed");
        assert_eq!(facts.commits_past_baseline, Some(0));

        let note = "no dispatch baseline recorded — nothing to salvage";
        let meta = serde_json::json!({"salvage_receipt": {
            "outcome": "salvage_skipped_no_baseline", "note": note
        }});
        let facts =
            salvage_facts_for_park(&meta, Some(&format!("timed out; salvage: {note}"))).unwrap();
        assert_eq!(facts.commits_past_baseline, None, "nothing was measured");

        let note = "no commits past the dispatch baseline — nothing to salvage";
        let meta = serde_json::json!({"salvage_receipt": {
            "outcome": "salvage_skipped_no_commits", "note": note
        }});
        let facts = salvage_facts_for_park(
            &meta,
            Some(&format!(
                "worker exceeded its time bound; salvage: {note}; {QUARANTINED_WORK_MARKER} — \
                 the credential guard flagged them (.env); left uncommitted in /tmp/wt, \
                 worktree kept for a human"
            )),
        )
        .unwrap();
        assert!(facts.uncommitted_work_quarantined);
    }

    /// Inbox fix group: a wall-clock park the run imposed from outside is
    /// classified by its own last-error prefix, never as a per-attempt timeout.
    #[test]
    fn run_imposed_wall_clock_parks_have_no_timeout_cause() {
        let wall = BudgetExhaustion::Wallclock {
            spent_secs: 0,
            cap_secs: 0,
        };
        for park in [
            TimeoutPark::Stalled,
            TimeoutPark::RunTimeLimit,
            TimeoutPark::Shutdown,
        ] {
            let last_error = format!("{}; salvage: nothing", park.error_prefix());
            assert_eq!(infer_exhaustion_cause(&wall, Some(&last_error)), None);
            assert_eq!(TimeoutPark::of_last_error(&last_error), Some(park));
        }
        let attempt = format!(
            "{}; salvage: nothing",
            TimeoutPark::AttemptBound.error_prefix()
        );
        assert_eq!(
            infer_exhaustion_cause(&wall, Some(&attempt)),
            Some(crate::decisions::UnblockCause::AttemptTimeout)
        );
        assert_eq!(
            infer_exhaustion_cause(&wall, Some("worker crashed")),
            Some(crate::decisions::UnblockCause::WallBudget)
        );
    }

    /// Lane R21d (A23 Q2 #2): pure truth table for `attempt_made_zero_progress`
    /// — ANY of a commit, a file change, or a successful verify disqualifies
    /// an attempt from "zero progress".
    #[test]
    fn attempt_made_zero_progress_requires_all_three_signals_absent() {
        assert!(attempt_made_zero_progress(false, false, false));
        assert!(!attempt_made_zero_progress(true, false, false));
        assert!(!attempt_made_zero_progress(false, true, false));
        assert!(!attempt_made_zero_progress(false, false, true));
        assert!(!attempt_made_zero_progress(true, true, true));
    }

    /// Lane R21d (A23 Q2 #2): a goal's first zero-progress attempt is
    /// forgiven — `attempt_count` is decremented back to what it was before
    /// this attempt was dispatched, and the streak is recorded at 1.
    #[tokio::test]
    async fn zero_progress_attempt_is_forgiven_once() {
        let pool = test_pool().await;
        // Attempt #3 was just dispatched (claim patch already stamped
        // attempt_count = 3) and completed with nothing to show for it.
        let goal = goal_in_state(&pool, "in_progress", 3).await;

        maybe_forgive_zero_progress_attempt(&pool, &goal.id, true)
            .await
            .unwrap();

        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            card.metadata_json
                .get("attempt_count")
                .and_then(|v| v.as_u64()),
            Some(2),
            "the zero-progress attempt must not count — attempt_count reverts to what it was \
             before this attempt was dispatched"
        );
        assert_eq!(
            card.metadata_json
                .get(ZERO_PROGRESS_STREAK_METADATA_KEY)
                .and_then(|v| v.as_u64()),
            Some(1)
        );
        assert_eq!(
            card.metadata_json
                .get("zero_progress")
                .and_then(|v| v.as_bool()),
            Some(true)
        );

        // check_budget must now see the forgiven count, not the stamped one
        // — a goal at the default cap of 3 that has 2 real attempts left has
        // NOT exhausted its attempt budget.
        assert!(check_budget(&pool, &card.metadata_json)
            .await
            .unwrap()
            .is_none());
    }

    /// Lane R21d (A23 Q2 #2): a SECOND consecutive zero-progress attempt is
    /// not forgiven — it counts normally against `attempt_count`, and the
    /// streak advances to 2 so the eventual park detail can name it.
    #[tokio::test]
    async fn a_second_consecutive_zero_progress_attempt_counts_normally() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 3).await;

        // First zero-progress attempt: forgiven (3 -> 2), streak = 1.
        maybe_forgive_zero_progress_attempt(&pool, &goal.id, true)
            .await
            .unwrap();

        // The goal gets its forgiven attempt back: redispatch stamps
        // attempt_count = 3 again (mirrors the claim patch's own
        // `attempt_count.saturating_add(1)`).
        sqlx::query(
            "UPDATE cards SET metadata_json = json_set(metadata_json, '$.attempt_count', 3) \
             WHERE id = ?",
        )
        .bind(&goal.id)
        .execute(&pool)
        .await
        .unwrap();

        // Second CONSECUTIVE zero-progress attempt: counts normally.
        maybe_forgive_zero_progress_attempt(&pool, &goal.id, true)
            .await
            .unwrap();

        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            card.metadata_json
                .get("attempt_count")
                .and_then(|v| v.as_u64()),
            Some(3),
            "a second consecutive zero-progress attempt must count — no second forgiveness"
        );
        assert_eq!(
            card.metadata_json
                .get(ZERO_PROGRESS_STREAK_METADATA_KEY)
                .and_then(|v| v.as_u64()),
            Some(2)
        );

        // At the default cap of 3, this now DOES exhaust the attempt budget.
        assert!(matches!(
            check_budget(&pool, &card.metadata_json).await.unwrap(),
            Some(BudgetExhaustion::AttemptCap { spent: 3, cap: 3 })
        ));
    }

    /// Lane R21d (A23 Q2 #2): a productive attempt (a commit, a file change,
    /// or a successful verify) always counts, and resets any zero-progress
    /// streak the goal was carrying.
    #[tokio::test]
    async fn a_productive_attempt_counts_and_resets_the_streak() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 3).await;

        // A prior zero-progress attempt already spent this goal's one
        // forgiveness.
        maybe_forgive_zero_progress_attempt(&pool, &goal.id, true)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE cards SET metadata_json = json_set(metadata_json, '$.attempt_count', 3) \
             WHERE id = ?",
        )
        .bind(&goal.id)
        .execute(&pool)
        .await
        .unwrap();

        // This attempt made real progress.
        maybe_forgive_zero_progress_attempt(&pool, &goal.id, false)
            .await
            .unwrap();

        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            card.metadata_json
                .get("attempt_count")
                .and_then(|v| v.as_u64()),
            Some(3),
            "a productive attempt is never forgiven and must count as stamped"
        );
        assert!(
            card.metadata_json
                .get(ZERO_PROGRESS_STREAK_METADATA_KEY)
                .is_none(),
            "a productive attempt resets the zero-progress streak"
        );
        assert!(card.metadata_json.get("zero_progress").is_none());
    }

    /// QA 2026-09-26: an attempt that ended `Failed`/`TimedOut` moves the
    /// streak too. With nothing to show it counts toward the breaker, but its
    /// `attempt_count` is never forgiven — that stays the failure path's own.
    #[tokio::test]
    async fn an_unfinished_zero_progress_attempt_counts_toward_the_streak_unforgiven() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 3).await;

        record_unfinished_attempt_progress(&pool, &goal.id, true)
            .await
            .unwrap();
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            card.metadata_json
                .get("attempt_count")
                .and_then(|v| v.as_u64()),
            Some(3),
            "a failed attempt is never forgiven"
        );
        assert_eq!(
            card.metadata_json
                .get(ZERO_PROGRESS_STREAK_METADATA_KEY)
                .and_then(|v| v.as_u64()),
            Some(1)
        );

        record_unfinished_attempt_progress(&pool, &goal.id, true)
            .await
            .unwrap();
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            card.metadata_json
                .get(ZERO_PROGRESS_STREAK_METADATA_KEY)
                .and_then(|v| v.as_u64()),
            Some(2),
            "two failed attempts in a row that changed nothing trip the breaker"
        );
        assert_eq!(
            card.metadata_json
                .get("attempt_count")
                .and_then(|v| v.as_u64()),
            Some(3)
        );
    }

    /// QA 2026-09-26: a failed attempt that DID make progress (it committed
    /// before a provider error or a context overflow ended it) clears a
    /// streak left by earlier empty attempts, so the breaker does not park
    /// the goal on its next dispatch for "no progress".
    #[tokio::test]
    async fn an_unfinished_attempt_that_made_progress_clears_a_stale_streak() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 3).await;
        sqlx::query(
            "UPDATE cards SET metadata_json = json_set(metadata_json, \
             '$.zero_progress_streak', 2, '$.zero_progress', json('true')) WHERE id = ?",
        )
        .bind(&goal.id)
        .execute(&pool)
        .await
        .unwrap();

        record_unfinished_attempt_progress(&pool, &goal.id, false)
            .await
            .unwrap();

        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            card.metadata_json
                .get(ZERO_PROGRESS_STREAK_METADATA_KEY)
                .is_none(),
            "progress in a failed attempt resets the streak: {:?}",
            card.metadata_json
        );
        assert!(card.metadata_json.get("zero_progress").is_none());
        assert_eq!(
            card.metadata_json
                .get("attempt_count")
                .and_then(|v| v.as_u64()),
            Some(3)
        );
    }

    /// QA 2026-09-26: only a human's `unblock` answer clears the breaker's
    /// state; an automated actor's never does.
    #[test]
    fn only_a_human_unblock_answer_clears_the_zero_progress_breaker() {
        let mut patch = serde_json::Map::new();
        assert!(clear_zero_progress_on_human_unblock(
            &mut patch,
            crate::decisions::ACTOR_JESSE
        ));
        for key in [
            ZERO_PROGRESS_STREAK_METADATA_KEY,
            "zero_progress",
            ZERO_PROGRESS_ESCALATION_USED_METADATA_KEY,
        ] {
            assert_eq!(
                patch.get(key),
                Some(&serde_json::Value::Null),
                "{key} must be removed by the unpark patch"
            );
        }
        for actor in [
            crate::decisions::ACTOR_SYSTEM,
            crate::decisions::ACTOR_HEADLESS_POLICY,
            crate::decisions::ACTOR_AUTO_LAUNCH,
            crate::decisions::ACTOR_SALVAGE_POLICY,
        ] {
            let mut patch = serde_json::Map::new();
            assert!(!clear_zero_progress_on_human_unblock(&mut patch, actor));
            assert!(patch.is_empty(), "{actor} must not clear the breaker");
        }
    }

    /// Lane R21d (A23 Q2 #2): every park path converges on
    /// `park_goal_with_meta` — a SECOND consecutive zero-progress attempt's
    /// streak must show up in the park detail (`last_error`) so a human
    /// reading it sees the goal spent a real attempt on nothing.
    #[tokio::test]
    async fn park_detail_names_a_second_consecutive_zero_progress_streak() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 3).await;
        sqlx::query(
            "UPDATE cards SET metadata_json = json_set(metadata_json, '$.zero_progress_streak', 2) \
             WHERE id = ?",
        )
        .bind(&goal.id)
        .execute(&pool)
        .await
        .unwrap();

        park_goal(
            &pool,
            &goal.id,
            decisions::ACTOR_SYSTEM,
            "held repeatedly with no verify",
        )
        .await
        .unwrap();

        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        let last_error = card
            .metadata_json
            .get("last_error")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        assert!(
            last_error.contains("zero_progress_streak=2"),
            "park detail must name the streak, got: {last_error}"
        );
        assert!(last_error.starts_with("held repeatedly with no verify"));
    }

    /// Lane R21d (A23 Q2 #2): a single forgiven zero-progress attempt
    /// (streak == 1) never counted against the goal, so the park detail
    /// stays exactly the caller's reason — nothing to explain.
    #[tokio::test]
    async fn park_detail_omits_a_single_forgiven_zero_progress_attempt() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 3).await;
        sqlx::query(
            "UPDATE cards SET metadata_json = json_set(metadata_json, '$.zero_progress_streak', 1) \
             WHERE id = ?",
        )
        .bind(&goal.id)
        .execute(&pool)
        .await
        .unwrap();

        park_goal(
            &pool,
            &goal.id,
            decisions::ACTOR_SYSTEM,
            "held with no verify",
        )
        .await
        .unwrap();

        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            card.metadata_json
                .get("last_error")
                .and_then(|v| v.as_str()),
            Some("held with no verify")
        );
    }

    /// #250: the manual-retry path — a parked (Failed) goal advances back to
    /// Ready through the guard, which is exactly what the answered unblock
    /// decision's effect runs.
    #[tokio::test]
    async fn failed_goal_unparks_to_ready_through_the_guard() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 3).await;
        exhaust_and_park(
            &pool,
            &goal.id,
            "Guard test goal in in_progress",
            PERSONAL_PROJECT_ID,
            BudgetExhaustion::AttemptCap { spent: 3, cap: 3 },
            None,
        )
        .await
        .unwrap();
        assert_eq!(state_of(&pool, &goal.id).await, "failed");

        let mut patch = serde_json::Map::new();
        patch.insert(
            "needs_human_attention".to_string(),
            serde_json::json!(false),
        );
        let new = advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Ready,
            decisions::ACTOR_SYSTEM,
            None,
            TransitionEffects {
                metadata_patch: patch,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(new, GoalState::Ready);
        assert_eq!(state_of(&pool, &goal.id).await, "ready");
    }

    /// #250: a Failed goal can be cancelled (the user abandons the retry).
    #[tokio::test]
    async fn failed_goal_can_be_cancelled() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "failed", 3).await;
        let new = advance_goal_checked(
            &pool,
            &goal.id,
            GoalAction::Cancel,
            decisions::ACTOR_JESSE,
            None,
            TransitionEffects::default(),
        )
        .await
        .unwrap();
        assert_eq!(new, GoalState::Cancelled);
    }

    #[tokio::test]
    async fn within_budget_is_not_exhausted() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 1).await;
        let exhaustion = check_budget(&pool, &goal.metadata_json).await.unwrap();
        assert!(exhaustion.is_none());
    }

    // ── Deletion gate ──

    #[tokio::test]
    async fn goal_deletion_requires_jesse_risk_gate() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "triage", 0).await;

        // A non-risk_gate proof is refused.
        let wrong_kind =
            approve_decision_proof(&pool, &goal.id, crate::decisions::ACTOR_JESSE).await;
        let err = delete_goal_checked(&pool, &goal.id, wrong_kind)
            .await
            .unwrap_err();
        assert!(matches!(err, GuardError::Denied(_)));

        // A proper risk_gate (user_data_deletion) approved by jesse deletes.
        let d = create_decision(
            &pool,
            NewDecision {
                kind: "risk_gate".to_string(),
                goal_id: Some(goal.id.clone()),
                headline: Some("Permission to delete the test goal".to_string()),
                detail: Some("user_data_deletion of a goal card".to_string()),
                payload: serde_json::json!({
                    "action_class": "user_data_deletion",
                    "description": "delete goal",
                    "requested_by": "test"
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (_, proof) = answer_decision(
            &pool,
            &d.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            crate::decisions::ACTOR_JESSE,
        )
        .await
        .unwrap();
        let deleted = delete_goal_checked(&pool, &goal.id, proof).await.unwrap();
        assert!(deleted);
        assert!(crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .is_none());
    }

    // ── Requeue / promote ──

    #[tokio::test]
    async fn requeue_moves_in_progress_to_ready_with_attempts() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 1).await;
        requeue_goal(
            &pool,
            &goal.id,
            "system",
            2,
            // See orchestrator.rs: the tested condition is a dead worker
            // session, not a daemon restart. Keep these two strings identical.
            "Worker session ended before the goal completed",
        )
        .await
        .unwrap();
        assert_eq!(state_of(&pool, &goal.id).await, "ready");
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            card.metadata_json
                .get("attempt_count")
                .and_then(|v| v.as_u64()),
            Some(2)
        );
    }

    /// E2 (harness loop, Round E, from Lane F9 forensics 2026-09-10): F9's
    /// dominant failure class was exactly this call succeeding when it
    /// should have refused — a goal requeued to a fresh session while its
    /// dispatched session was still alive and working. Pre-fix:
    /// `requeue_goal_with_meta` takes no `dispatched_session_alive`
    /// parameter at all (fails to compile against this call).
    #[tokio::test]
    async fn requeue_refuses_when_the_dispatched_session_is_still_alive() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 1).await;

        let err = requeue_goal_with_meta(
            &pool,
            &goal.id,
            "system",
            2,
            "should never apply — the session is alive",
            &[],
            true,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, GuardError::Denied(_)),
            "a live dispatched session must be a refusal, not a silent success: {err:?}"
        );

        // Refused: the card must stay exactly where it was, not move to Ready.
        assert_eq!(state_of(&pool, &goal.id).await, "in_progress");
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            card.metadata_json
                .get("attempt_count")
                .and_then(|v| v.as_u64()),
            Some(1),
            "a refused requeue must never touch attempt_count"
        );
        // And the spec's "explicit reason recorded on the card" requirement.
        assert!(
            card.metadata_json
                .get("requeue_refused_reason")
                .and_then(|v| v.as_str())
                .is_some_and(|r| r.contains("live dispatched session")),
            "the refusal reason must be durably recorded on the card: {:?}",
            card.metadata_json
        );
    }

    /// M1 (G8 review, harness loop Lane E3 2026-09-10): the caller-attested
    /// `dispatched_session_alive: false` must not be trusted blindly when
    /// the goal's OWN `execution_receipt` — written by a wholly separate
    /// code path, the dispatch/heartbeat loop — still reports a live,
    /// recently-heartbeated attempt in the CURRENT daemon lifecycle. Pre-fix:
    /// this cross-check does not exist at all, so this call would silently
    /// succeed and bounce the card to `ready` out from under a receipt that
    /// says the attempt is still running.
    #[tokio::test]
    async fn requeue_refuses_when_caller_says_dead_but_the_execution_receipt_is_still_live() {
        use crate::agents::platform_extensions::execution_receipt::ExecutionReceipt;
        use crate::agents::platform_extensions::orchestrator::daemon_lifecycle_id;

        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 1).await;

        let receipt = ExecutionReceipt::new(
            "permagent",
            "live-session-1",
            serde_json::json!({}),
            daemon_lifecycle_id().to_string(),
            chrono::Utc::now().to_rfc3339(),
            1,
        );
        let mut meta = goal.metadata_json.as_object().cloned().unwrap();
        meta.insert(
            "execution_receipt".to_string(),
            serde_json::to_value(&receipt).unwrap(),
        );
        sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
            .bind(serde_json::Value::Object(meta).to_string())
            .bind(&goal.id)
            .execute(&pool)
            .await
            .unwrap();

        let err = requeue_goal_with_meta(
            &pool,
            &goal.id,
            "system",
            2,
            "caller (wrongly) believes this session is dead",
            &[],
            false,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, GuardError::Denied(_)),
            "a caller-attested death that disagrees with a live execution_receipt must be \
             refused, not silently trusted: {err:?}"
        );

        assert_eq!(
            state_of(&pool, &goal.id).await,
            "in_progress",
            "the card must stay exactly where it was"
        );
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            card.metadata_json
                .get("attempt_count")
                .and_then(|v| v.as_u64()),
            Some(1),
            "a refused requeue must never touch attempt_count"
        );
        assert!(
            card.metadata_json
                .get("requeue_refused_reason")
                .and_then(|v| v.as_str())
                .is_some_and(|r| r.contains("execution_receipt")),
            "the refusal reason must name the disagreement: {:?}",
            card.metadata_json
        );
    }

    /// M1: the cross-check above must not block a GENUINELY dead session —
    /// a terminal (`Completed`) receipt is never "live" regardless of how
    /// recently it was last touched, so a caller-attested death that AGREES
    /// with the receipt still succeeds exactly as before this lane.
    #[tokio::test]
    async fn requeue_still_succeeds_when_caller_says_dead_and_the_execution_receipt_agrees() {
        use crate::agents::platform_extensions::execution_receipt::{
            ExecutionReceipt, ReceiptState,
        };
        use crate::agents::platform_extensions::orchestrator::daemon_lifecycle_id;

        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 1).await;

        let mut receipt = ExecutionReceipt::new(
            "permagent",
            "dead-session-1",
            serde_json::json!({}),
            daemon_lifecycle_id().to_string(),
            chrono::Utc::now().to_rfc3339(),
            1,
        );
        receipt.finalize(ReceiptState::StaleWorker, chrono::Utc::now().to_rfc3339());
        let mut meta = goal.metadata_json.as_object().cloned().unwrap();
        meta.insert(
            "execution_receipt".to_string(),
            serde_json::to_value(&receipt).unwrap(),
        );
        sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
            .bind(serde_json::Value::Object(meta).to_string())
            .bind(&goal.id)
            .execute(&pool)
            .await
            .unwrap();

        requeue_goal_with_meta(
            &pool,
            &goal.id,
            "system",
            2,
            "worker session ended before the goal completed",
            &[],
            false,
        )
        .await
        .unwrap();
        assert_eq!(state_of(&pool, &goal.id).await, "ready");
    }

    /// E2: the guard's OTHER refusal — a goal whose landing receipt already
    /// shows the fix reached trunk must never be requeued back to Ready,
    /// even though its column binding is still `in_progress` (M9's own
    /// observed shape: the requeue is what MOVES it to `ready` in the first
    /// place — this guard has to catch it before that move, while the card
    /// is still `in_progress`). Pre-fix: no `trunk_state` check exists at
    /// all, so this call would have silently succeeded and reset the card.
    #[tokio::test]
    async fn requeue_refuses_a_goal_already_landed_on_trunk() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 0).await;
        let mut meta = goal.metadata_json.as_object().cloned().unwrap();
        meta.insert(
            "dispatch_evidence".to_string(),
            serde_json::json!({
                "landing": { "outcome": "landed", "trunk_state": "landed_on_trunk" },
            }),
        );
        sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
            .bind(serde_json::Value::Object(meta).to_string())
            .bind(&goal.id)
            .execute(&pool)
            .await
            .unwrap();

        let err = requeue_goal(
            &pool,
            &goal.id,
            "system",
            1,
            "should never apply — already landed",
        )
        .await
        .unwrap_err();
        assert!(matches!(err, GuardError::Denied(_)), "{err:?}");
        assert_eq!(
            state_of(&pool, &goal.id).await,
            "in_progress",
            "a landed goal must never be bounced back to ready"
        );
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert!(card
            .metadata_json
            .get("requeue_refused_reason")
            .and_then(|v| v.as_str())
            .is_some_and(|r| r.contains("landed on trunk")));
    }

    /// E2: D1's R3 silent-death redispatch must keep working through this
    /// guard unchanged — it always attests `dispatched_session_alive: false`
    /// (it checks `!session_alive` itself before ever calling requeue; see
    /// `orchestrator.rs`'s `handle_goal_status`), so a real dead-session
    /// requeue succeeds exactly once, same as before this lane.
    #[tokio::test]
    async fn requeue_with_meta_still_succeeds_for_a_genuinely_dead_session() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 0).await;
        requeue_goal_with_meta(
            &pool,
            &goal.id,
            decisions::ACTOR_SYSTEM,
            1,
            "worker exited with no error and no landing; auto-retrying once before escalation",
            &[("retry_count", serde_json::json!(1))],
            false,
        )
        .await
        .unwrap();
        assert_eq!(state_of(&pool, &goal.id).await, "ready");
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            card.metadata_json
                .get("retry_count")
                .and_then(|v| v.as_u64()),
            Some(1)
        );
    }

    #[tokio::test]
    async fn promote_skips_parked_goals() {
        let pool = test_pool().await;
        let dep = goal_in_state(&pool, "complete", 0).await;
        let goal = goal_in_state(&pool, "triage", 3).await;

        // Give the triage goal a satisfied dependency but mark it parked.
        let mut meta = goal.metadata_json.as_object().cloned().unwrap();
        meta.insert("depends_on".to_string(), serde_json::json!([dep.id]));
        meta.insert("needs_human_attention".to_string(), serde_json::json!(true));
        let meta_str = serde_json::to_string(&serde_json::Value::Object(meta)).unwrap();
        sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
            .bind(&meta_str)
            .bind(&goal.id)
            .execute(&pool)
            .await
            .unwrap();

        let promoted = promote_eligible_dependents(&pool, PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        assert_eq!(promoted, 0, "parked goals must not auto-promote");
        assert_eq!(state_of(&pool, &goal.id).await, "triage");
    }

    // ── Post-creation roadmap editing (#251) ────────────────────────────────

    #[tokio::test]
    async fn set_dependencies_validates_and_writes() {
        let pool = test_pool().await;
        let dep = goal_in_state(&pool, "triage", 0).await;
        let goal = goal_in_state(&pool, "triage", 0).await;

        set_goal_dependencies(&pool, &goal.id, std::slice::from_ref(&dep.id), "jesse")
            .await
            .unwrap();
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            card.metadata_json.get("depends_on"),
            Some(&serde_json::json!([dep.id]))
        );

        // A cycle (dep → goal while goal → dep) is rejected.
        let err = set_goal_dependencies(&pool, &dep.id, std::slice::from_ref(&goal.id), "jesse")
            .await
            .unwrap_err();
        assert!(matches!(err, GuardError::Invalid(_)), "{:?}", err);
        assert!(err.to_string().contains("cycle"), "{}", err);

        // Self-dependency is rejected.
        let err = set_goal_dependencies(&pool, &goal.id, std::slice::from_ref(&goal.id), "jesse")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("itself"), "{}", err);

        // A dangling dependency id is rejected.
        let err = set_goal_dependencies(&pool, &goal.id, &["nope".to_string()], "jesse")
            .await
            .unwrap_err();
        assert!(matches!(err, GuardError::NotFound(_)), "{:?}", err);
    }

    #[tokio::test]
    async fn set_dependencies_refused_for_in_progress_goal() {
        let pool = test_pool().await;
        let dep = goal_in_state(&pool, "triage", 0).await;
        let running = goal_in_state(&pool, "in_progress", 1).await;
        let err = set_goal_dependencies(&pool, &running.id, std::slice::from_ref(&dep.id), "jesse")
            .await
            .unwrap_err();
        assert!(matches!(err, GuardError::Invalid(_)), "{:?}", err);
        assert!(err.to_string().contains("in_progress"), "{}", err);
    }

    #[tokio::test]
    async fn depends_on_is_protected_from_raw_metadata_patch() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "triage", 0).await;
        let mut meta = goal.metadata_json.as_object().cloned().unwrap();
        meta.insert("depends_on".to_string(), serde_json::json!(["sneaky"]));
        let err = crate::cards::update_card(
            &pool,
            &goal.id,
            crate::cards::UpdateCard {
                metadata_json: Some(serde_json::Value::Object(meta)),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("depends_on"), "{}", err);
    }

    #[tokio::test]
    async fn detach_splices_dependents_onto_grandparents() {
        let pool = test_pool().await;
        // root ← mid ← leaf; removing mid must leave leaf depending on root.
        let root = goal_in_state(&pool, "triage", 0).await;
        let mid = goal_in_state(&pool, "triage", 0).await;
        let leaf = goal_in_state(&pool, "triage", 0).await;
        set_goal_dependencies(&pool, &mid.id, std::slice::from_ref(&root.id), "jesse")
            .await
            .unwrap();
        set_goal_dependencies(&pool, &leaf.id, std::slice::from_ref(&mid.id), "jesse")
            .await
            .unwrap();

        let rewired = detach_goal_from_dependents(&pool, &mid.id, "jesse")
            .await
            .unwrap();
        assert_eq!(rewired, 1, "leaf rewired");
        let leaf_after = crate::cards::get_card(&pool, &leaf.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            leaf_after.metadata_json.get("depends_on"),
            Some(&serde_json::json!([root.id]))
        );
    }

    #[tokio::test]
    async fn insert_roadmap_goal_root_becomes_ready_and_dependent_stays_triage() {
        let pool = test_pool().await;
        crate::cards::seed_goal_columns(&pool, PERSONAL_PROJECT_ID)
            .await
            .unwrap();

        // Root insert (no deps): straight to Ready for auto-dispatch.
        let root = insert_roadmap_goal(
            &pool,
            PERSONAL_PROJECT_ID,
            NewRoadmapGoal {
                title: "Root insert".to_string(),
                ..Default::default()
            },
            decisions::ACTOR_JESSE,
        )
        .await
        .unwrap();
        assert_eq!(state_of(&pool, &root.id).await, "ready");

        // Dependent insert on an incomplete dep: stays in Triage.
        let child = insert_roadmap_goal(
            &pool,
            PERSONAL_PROJECT_ID,
            NewRoadmapGoal {
                title: "Child insert".to_string(),
                depends_on: vec![root.id.clone()],
                ..Default::default()
            },
            decisions::ACTOR_JESSE,
        )
        .await
        .unwrap();
        assert_eq!(state_of(&pool, &child.id).await, "triage");
        assert_eq!(
            child.metadata_json.get("depends_on"),
            Some(&serde_json::json!([root.id]))
        );

        // Insert on a COMPLETE dep: becomes Ready immediately.
        let done = goal_in_state(&pool, "complete", 0).await;
        let after_done = insert_roadmap_goal(
            &pool,
            PERSONAL_PROJECT_ID,
            NewRoadmapGoal {
                title: "After done".to_string(),
                depends_on: vec![done.id.clone()],
                ..Default::default()
            },
            decisions::ACTOR_JESSE,
        )
        .await
        .unwrap();
        assert_eq!(state_of(&pool, &after_done.id).await, "ready");

        // Empty title refused.
        let err = insert_roadmap_goal(
            &pool,
            PERSONAL_PROJECT_ID,
            NewRoadmapGoal {
                title: "   ".to_string(),
                ..Default::default()
            },
            decisions::ACTOR_JESSE,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, GuardError::Invalid(_)));
    }

    /// The audit row must name whoever actually asked, not a baked-in default.
    ///
    /// `ACTOR_JESSE` is the literal the Tier-2 human-only gate compares
    /// against, so a library function that hardcodes it launders any future
    /// non-human caller into the human's identity — silently, with nothing at
    /// the call site to notice. This asserts a system-originated insert is
    /// attributed to the system and is NOT recorded as the user.
    #[tokio::test]
    async fn roadmap_insert_is_attributed_to_the_caller_not_to_jesse() {
        let pool = test_pool().await;
        crate::cards::seed_goal_columns(&pool, PERSONAL_PROJECT_ID)
            .await
            .unwrap();

        let card = insert_roadmap_goal(
            &pool,
            PERSONAL_PROJECT_ID,
            NewRoadmapGoal {
                title: "System-originated goal".to_string(),
                ..Default::default()
            },
            decisions::ACTOR_SYSTEM,
        )
        .await
        .unwrap();

        let acted_by: String = sqlx::query_scalar(
            "SELECT acted_by FROM decision_audit WHERE goal_id = ? AND outcome = 'roadmap:insert'",
        )
        .bind(&card.id)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(acted_by, decisions::ACTOR_SYSTEM);
        assert_ne!(
            acted_by,
            decisions::ACTOR_JESSE,
            "a system insert must never be laundered into the human-only actor"
        );
    }

    // ── Per-goal auto-approve (#252) ────────────────────────────────────────

    #[tokio::test]
    async fn auto_approve_setter_round_trips_and_audits() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "triage", 0).await;

        set_goal_auto_approve(&pool, &goal.id, true, "jesse")
            .await
            .unwrap();
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            card.metadata_json.get("auto_approve"),
            Some(&serde_json::Value::Bool(true))
        );

        set_goal_auto_approve(&pool, &goal.id, false, "jesse")
            .await
            .unwrap();
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert!(card.metadata_json.get("auto_approve").is_none());

        let outcomes: Vec<String> = sqlx::query_scalar(
            "SELECT outcome FROM decision_audit WHERE goal_id = ? ORDER BY seq ASC",
        )
        .bind(&goal.id)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            outcomes,
            vec!["auto_approve:enabled", "auto_approve:disabled"]
        );
    }

    #[tokio::test]
    async fn auto_approve_is_protected_from_raw_metadata_patch() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "triage", 0).await;
        let mut meta = goal.metadata_json.as_object().cloned().unwrap();
        meta.insert("auto_approve".to_string(), serde_json::json!(true));
        let err = crate::cards::update_card(
            &pool,
            &goal.id,
            crate::cards::UpdateCard {
                metadata_json: Some(serde_json::Value::Object(meta)),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("auto_approve"), "{}", err);
    }

    /// Lane R22c-e (G26 MED-1): `unverified_reviewer_approval`'s PRESENCE is
    /// what makes `auto_land_headless_goal` bypass the ordinary verify-
    /// evidence gate and land a goal outright — a worker must not be able
    /// to forge its own reviewer sign-off via the agent-facing metadata
    /// PATCH tool and auto-land unverified, unreviewed work.
    #[tokio::test]
    async fn unverified_reviewer_approval_is_protected_from_raw_metadata_patch() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "in_progress", 0).await;
        let mut meta = goal.metadata_json.as_object().cloned().unwrap();
        meta.insert(
            "unverified_reviewer_approval".to_string(),
            serde_json::json!({
                "checked": "forged approval",
                "reviewer_provider": "sneaky",
                "reviewer_model": "sneaky-model",
                "note": "forged by the worker, not a real reviewer",
            }),
        );
        let err = crate::cards::update_card(
            &pool,
            &goal.id,
            crate::cards::UpdateCard {
                metadata_json: Some(serde_json::Value::Object(meta)),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("unverified_reviewer_approval"), "{}", err);
    }

    // ── promote_eligible_dependents_or_warn (bug-sweep wave 1) ──────────────

    /// Success path: no warning, and the eligible dependent actually moved —
    /// the approval spine keeps working exactly as before.
    #[tokio::test]
    async fn promote_or_warn_success_promotes_and_returns_none() {
        let pool = test_pool().await;
        let dep = goal_in_state(&pool, "complete", 0).await;
        let goal = goal_in_state(&pool, "triage", 0).await;

        let mut meta = goal.metadata_json.as_object().cloned().unwrap();
        meta.insert("depends_on".to_string(), serde_json::json!([dep.id]));
        let meta_str = serde_json::to_string(&serde_json::Value::Object(meta)).unwrap();
        sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
            .bind(&meta_str)
            .bind(&goal.id)
            .execute(&pool)
            .await
            .unwrap();

        let warning =
            promote_eligible_dependents_or_warn(&pool, PERSONAL_PROJECT_ID, &dep.id).await;
        assert!(warning.is_none(), "healthy promotion must not warn");
        assert_eq!(state_of(&pool, &goal.id).await, "ready");
    }

    /// Failure path: a DB-level promotion failure becomes a warning string
    /// carrying the project and approved-goal ids — never an unwind, never a
    /// discarded Result. (Simulated by dropping the table promotion reads.)
    #[tokio::test]
    async fn promote_or_warn_failure_returns_warning_with_ids() {
        let pool = test_pool().await;
        // Ensure schema exists, then break exactly what promotion needs.
        crate::cards::seed_goal_columns(&pool, PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        sqlx::query("DROP TABLE board_columns")
            .execute(&pool)
            .await
            .unwrap();

        let warning =
            promote_eligible_dependents_or_warn(&pool, PERSONAL_PROJECT_ID, "goal-approved-1")
                .await
                .expect("broken DB must surface as a warning");
        assert!(warning.contains(PERSONAL_PROJECT_ID), "{}", warning);
        assert!(warning.contains("goal-approved-1"), "{}", warning);
        assert!(
            warning.contains("dependent promotion failed"),
            "{}",
            warning
        );
    }
}
