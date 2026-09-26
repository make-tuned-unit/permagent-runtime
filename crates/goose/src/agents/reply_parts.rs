use anyhow::Result;
use regex::Regex;
use std::sync::Arc;

use async_stream::try_stream;
use futures::stream::StreamExt;
use serde_json::{json, Value};
use tracing::debug;

use super::super::agents::Agent;
#[cfg(feature = "code-mode")]
use crate::agents::platform_extensions::code_execution;
use crate::config::GooseMode;
use crate::conversation::message::{Message, MessageContent, ToolRequest};
use crate::conversation::Conversation;
use crate::cost_router::cache::SystemPromptParts;
#[cfg(test)]
use crate::providers::base::stream_from_single_message;
use crate::providers::base::{MessageStream, Provider, ProviderUsage};
use crate::providers::canonical::{
    cache_hit_rate_of, cache_savings_of, canonical_name, cost_breakdown, maybe_get_pricing,
    worst_case_pricing,
};
use crate::providers::errors::ProviderError;
use crate::providers::toolshim::{
    augment_message_with_tool_calls, convert_tool_messages_to_text,
    modify_system_prompt_for_tool_json, OllamaInterpreter,
};
use crate::session::session_manager::{CallOutcome, CallProvenance, CallPurpose};
use crate::session::{
    budget_task_id, goal_id, CostLedgerRow, CostTier, Session, SessionManager, SessionType,
};
use rmcp::model::Tool;

async fn enhance_model_error(error: ProviderError, provider: &Arc<dyn Provider>) -> ProviderError {
    let ProviderError::RequestFailed(ref msg) = error else {
        return error;
    };

    let re = Regex::new(r"(?i)\b4\d{2}\b.*model|model.*\b4\d{2}\b").unwrap();
    if !re.is_match(msg) {
        return error;
    }

    let Ok(models) = provider.fetch_recommended_models().await else {
        return error;
    };
    if models.is_empty() {
        return error;
    }

    ProviderError::RequestFailed(format!(
        "{}. Available models for this provider: {}",
        msg,
        models.join(", ")
    ))
}

/// Overrides `model_config`'s own `max_tokens` with `reserved_output_tokens`
/// when the call took a paid hold, so the request actually sent to the
/// provider can never ask for more output than the reservation priced.
///
/// Without this, `RESERVED_OUTPUT_TOKENS` would only shrink the dollar hold
/// while the wire request still carried the model's raw (often
/// multi-hundred-thousand-token) `max_tokens` — a reservation that prices
/// one number while authorizing a request for a bigger one. `None` (unpaid
/// work; see `cost_router::reservation::is_free_local_pair`) leaves the
/// model's own config untouched.
fn apply_reserved_output_cap(
    model_config: crate::model::ModelConfig,
    reserved_output_tokens: Option<usize>,
) -> crate::model::ModelConfig {
    match reserved_output_tokens {
        Some(cap) => model_config.with_max_tokens(Some(cap as i32)),
        None => model_config,
    }
}

fn coerce_value(s: &str, schema: &Value) -> Value {
    let type_str = schema.get("type");

    match type_str {
        Some(Value::String(t)) => match t.as_str() {
            "number" | "integer" => try_coerce_number(s),
            "boolean" => try_coerce_boolean(s),
            _ => Value::String(s.to_string()),
        },
        Some(Value::Array(types)) => {
            // Try each type in order
            for t in types {
                if let Value::String(type_name) = t {
                    match type_name.as_str() {
                        "number" | "integer" if s.parse::<f64>().is_ok() => {
                            return try_coerce_number(s);
                        }
                        "boolean" if matches!(s.to_lowercase().as_str(), "true" | "false") => {
                            return try_coerce_boolean(s);
                        }
                        _ => continue,
                    }
                }
            }
            Value::String(s.to_string())
        }
        _ => Value::String(s.to_string()),
    }
}

fn try_coerce_number(s: &str) -> Value {
    if let Ok(n) = s.parse::<f64>() {
        if n.fract() == 0.0 && n >= i64::MIN as f64 && n <= i64::MAX as f64 {
            json!(n as i64)
        } else {
            json!(n)
        }
    } else {
        Value::String(s.to_string())
    }
}

fn try_coerce_boolean(s: &str) -> Value {
    match s.to_lowercase().as_str() {
        "true" => json!(true),
        "false" => json!(false),
        _ => Value::String(s.to_string()),
    }
}

pub(crate) fn coerce_tool_arguments(
    arguments: Option<serde_json::Map<String, Value>>,
    tool_schema: &Value,
) -> Option<serde_json::Map<String, Value>> {
    let args = arguments?;

    let properties = tool_schema.get("properties").and_then(|p| p.as_object())?;

    let mut coerced = serde_json::Map::new();

    for (key, value) in args.iter() {
        let coerced_value =
            if let (Value::String(s), Some(prop_schema)) = (value, properties.get(key)) {
                coerce_value(s, prop_schema)
            } else {
                value.clone()
            };
        coerced.insert(key.clone(), coerced_value);
    }

    Some(coerced)
}

async fn toolshim_postprocess(
    response: Message,
    toolshim_tools: &[Tool],
) -> Result<Message, ProviderError> {
    let interpreter = OllamaInterpreter::new().map_err(|e| {
        ProviderError::ExecutionError(format!("Failed to create OllamaInterpreter: {}", e))
    })?;

    augment_message_with_tool_calls(&interpreter, response, toolshim_tools)
        .await
        .map_err(|e| ProviderError::ExecutionError(format!("Failed to augment message: {}", e)))
}

/// What [`Agent::handle_provider_authorization_failure`] did about a blocked
/// provider call.
///
/// Y4a (harness loop, 2026-09-10, from Lane A12's M12 forensics): the handler
/// used to return a bare `detail` string, so its caller could not tell a gate
/// that had just been auto-granted from one that was parked for a human — it
/// emitted "provider spend could not be authorized" and ended the turn in
/// BOTH cases. For a coordinator that is fatal: the run ends with the turn, so
/// the `granted_via_gate=1, state='pending'` reservation the handler had just
/// paid for was never consumed by anyone (A12: fx01's correct worker commit
/// never surfaced, fx09 the same, fx07 truncated — the goal-scoped grant in
/// the *worker* session settled the same second, because a worker does get a
/// later turn). `gate_grant_minted` is that missing bit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderAuthorizationOutcome {
    /// The human-readable failure detail, unchanged from
    /// `ProviderAuthorizationFailure::to_string()` (plus any context the
    /// handler could add). Always safe to show the model.
    pub(crate) detail: String,
    /// `true` iff a headless gate auto-answer minted a spendable reservation
    /// under the SAME `invocation_id` this call already failed under, so the
    /// caller may retry `reserve_provider_invocation` ONCE with that id and
    /// take its idempotent `AlreadyReserved` fast path. `false` for every
    /// parked, refused, non-gate or non-headless outcome.
    pub(crate) gate_grant_minted: bool,
}

impl ProviderAuthorizationOutcome {
    /// The unchanged pre-Y4a outcome: report `detail`, authorize no retry.
    fn parked(detail: String) -> Self {
        Self {
            detail,
            gate_grant_minted: false,
        }
    }
}

/// Lane R18a-b (G21 HIGH-1): call once per provider call `agents::agent`'s
/// turn loop is about to let proceed, at the exact point `invocation` is
/// finalized (either the direct `reserve_provider_invocation` success, or
/// the retried call after a real `gate_grant_minted` grant) — a per-run
/// observation of `safety_and_trust`'s `approval_bypasses`, not a
/// compile-time literal. `authorized` is always `true` at both of the
/// turn loop's call sites today, because both are only reachable after a
/// real `ProviderAuthorizationOutcome`/reservation success (see the
/// `FIELDS` doc comment in `receipts.rs` for the full argument); this
/// function exists so a future call site that proceeds WITHOUT one shows
/// up as a non-zero receipt instead of nothing. Factored out as a pure,
/// directly-testable function because the turn loop itself (`agents::
/// agent`'s big streaming `reply`) has no lightweight test harness — see
/// `record_provider_call_authorized_increments_only_when_not_authorized`
/// below.
pub(crate) fn record_provider_call_authorized(authorized: bool) {
    if authorized {
        crate::receipts::record_count("safety_and_trust", "approval_bypasses", 0);
    } else {
        crate::receipts::record("safety_and_trust", "approval_bypasses");
    }
}

/// The gate-decision-resolved half of
/// [`Agent::handle_provider_authorization_failure`]'s success path, factored
/// out as a free function taking `headless` as an explicit parameter
/// instead of consulting the process-global headless dispatch policy slot
/// itself.
///
/// E1 (harness loop, Round E, from Lane F9 forensics 2026-09-10): D1's own
/// R2 gate-results notes explicitly flag that global as a real shared-
/// state hazard for `--lib` unit tests in this same binary ("ask future
/// lanes not to compound it") — it was a one-shot, process-wide `OnceLock`
/// with no reset between tests (Lane T7 has since given it an `RwLock`
/// slot and a `#[cfg(test)]` restore guard), so any new test that both installs a
/// headless policy AND asserts on the *absence* of one elsewhere in this
/// binary is a real, order-dependent flakiness risk. Taking `headless` as a
/// plain argument here lets this function's behavior be pinned directly and
/// deterministically, with zero risk to any other test in the binary.
///
/// Returns `Some(outcome)` when the gate was headlessly auto-answered — the
/// caller should retry in place (this session keeps its goal assignment
/// unchanged; never a `requeue_goal` call). Returns `None` when the caller
/// must fall through to the unchanged park path (not headless, the
/// auto-answer itself refused, or a race already resolved the decision).
#[allow(clippy::too_many_arguments)]
async fn try_auto_answer_reservation_gate(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    card_id: &str,
    session: &Session,
    invocation_id: &str,
    failure: &ProviderAuthorizationFailure,
    decision_id: &str,
    headless: bool,
    session_manager: &Arc<SessionManager>,
    detail: &str,
    // F3 (harness loop, Round F, from Lane G9's HIGH finding): the exact
    // provider/model of the call that just failed and got gated — stored on
    // the grant so a later reuse attempt can require an exact match instead
    // of trusting whatever provider/model the reusing call happens to be
    // using.
    provider: &str,
    model: &str,
) -> Option<ProviderAuthorizationOutcome> {
    if !headless {
        return None;
    }
    match crate::decisions::auto_approve_reservation_gate(pool, decision_id).await {
        Ok(_) => {
            let mut gate_grant_minted = false;
            if let ProviderAuthorizationFailure::NeedsGate { requested_usd, .. } = failure {
                let task_id = budget_task_id(&session.extension_data);
                let lease_until = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
                match task_id {
                    Some(task_id) => {
                        match session_manager
                            .grant_reservation_after_gate_approval(
                                invocation_id,
                                &session.id,
                                &task_id,
                                *requested_usd,
                                &lease_until,
                                provider,
                                model,
                            )
                            .await
                        {
                            // Y4a: ONLY this arm authorizes an in-turn retry.
                            // The grant is a real `granted_via_gate=1`,
                            // `state='pending'` row keyed on THIS call's
                            // `invocation_id`, so re-calling
                            // `reserve_provider_invocation` with that same id
                            // takes `reserve_provider_invocation`'s early
                            // idempotent lookup and returns `AlreadyReserved`
                            // without re-running any threshold math. Every
                            // other `Ok` shape (`AlreadySettled`, `Unknown`)
                            // has no spendable hold behind it, so it must
                            // keep the unchanged park path.
                            Ok(outcome) => {
                                gate_grant_minted = matches!(
                                    outcome,
                                    crate::session::CostReservationOutcome::Granted { .. }
                                        | crate::session::CostReservationOutcome::AlreadyReserved { .. }
                                );
                                if !gate_grant_minted {
                                    tracing::warn!(
                                        goal_id = %card_id,
                                        decision_id,
                                        "headless reservation-gate auto-answer succeeded but the \
                                         grant is not spendable ({outcome:?}); not retrying in turn"
                                    );
                                }
                            }
                            Err(error) => {
                                tracing::warn!(
                                    goal_id = %card_id,
                                    decision_id,
                                    "headless reservation-gate auto-answer succeeded but the \
                                     reservation could not be granted ({error:?}); the session's \
                                     next attempt will just re-ask the gate"
                                );
                            }
                        }
                    }
                    None => {
                        tracing::warn!(
                            goal_id = %card_id,
                            decision_id,
                            "headless reservation-gate auto-answer succeeded but this session \
                             has no durable task identity; cannot pre-grant a reservation"
                        );
                    }
                }
            }
            // Retry in place (E1): the goal keeps its current dispatched
            // session unchanged. No `requeue_goal` call here — see
            // `Agent::handle_provider_authorization_failure`'s doc comment.
            Some(ProviderAuthorizationOutcome {
                detail: detail.to_string(),
                gate_grant_minted,
            })
        }
        Err(error) => {
            tracing::info!(
                goal_id = %card_id,
                decision_id,
                "headless reservation-gate auto-answer did not apply ({error:?}); leaving the \
                 gate for a human"
            );
            None
        }
    }
}

/// If `session_id` has an unconsumed reservation grant from a prior headless
/// gate auto-answer (see
/// [`SessionManager::grant_reservation_after_gate_approval`]), return its
/// `invocation_id` so the caller reuses it for the upcoming call instead of
/// minting a fresh one — `reserve_provider_invocation`'s own early
/// idempotent lookup then reports `AlreadyReserved` immediately, never
/// re-running the gate/budget threshold math for a request that already got
/// a yes.
///
/// I1 (harness loop, 2026-09-10, wiring F1b + F10's forensics into the live
/// turn loop): F10 traced every provider call minting a brand-new
/// `invocation_id` (`Uuid::new_v4()`, in `agents::agent`'s turn loop) as the
/// exact reason E1's grant could never be matched by a retry — this closes
/// that gap at its one call site. Consumption is TTL-bounded
/// ([`SessionManager::GRANT_CONSUME_TTL_SECS_DEFAULT`]): an unconsumed grant
/// older than the TTL is released by
/// [`SessionManager::consume_granted_reservation`] itself and this returns
/// `None`, so a stale grant re-gates rather than authorizing a call that
/// never came in time. Belongs to exactly the session that requests it —
/// `consume_granted_reservation` refuses a different session's grant
/// outright, and `find_pending_gate_grant` only ever looks up `session_id`'s
/// own rows in the first place.
///
/// F3 (harness loop, Round F, from Lane G9's HIGH and MEDIUM findings):
/// `provider`/`model` must be the CURRENT call's own identity (its caller's
/// exact live provider/model, captured before dispatch) — a grant minted
/// under a different provider/model is invisible to `find_pending_gate_
/// grant` and is never returned here (H1: a provider/role switch between
/// the gated call and this retry no longer silently reuses the OLD call's
/// dollar bound). A grant already consumed once by an earlier call is
/// likewise refused by `consume_granted_reservation` (M1): it authorizes
/// exactly one retry, not every later call in the session.
pub(crate) async fn reusable_gate_grant_invocation_id(
    manager: &Arc<SessionManager>,
    session_id: &str,
    provider: &str,
    model: &str,
) -> Option<String> {
    let pending_invocation_id = manager
        .find_pending_gate_grant(session_id, provider, model)
        .await
        .ok()
        .flatten()?;
    let outcome = manager
        .consume_granted_reservation(
            &pending_invocation_id,
            session_id,
            provider,
            model,
            SessionManager::GRANT_CONSUME_TTL_SECS_DEFAULT,
            chrono::Utc::now(),
        )
        .await
        .ok()
        .flatten()?;
    matches!(
        outcome,
        crate::session::CostReservationOutcome::AlreadyReserved { .. }
    )
    .then_some(pending_invocation_id)
}

/// I1 (harness loop, 2026-09-10, from M10's forensics): the SESSION-scoped
/// counterpart of [`Agent::handle_provider_authorization_failure`]'s
/// goal-scoped gate, for a session with no `goal_id` — the coordinator's own
/// provider calls. M10 found the coordinator's calls hit the exact same
/// reservation gate as a worker's (bounded by the same task/session budget
/// rules) but `handle_provider_authorization_failure`'s entire gate-
/// decision/auto-answer/retry machinery was a structural no-op for it —
/// `goal_id(&session.extension_data)` is `None`, so it returned `detail`
/// immediately with no decision ever raised, no auto-answer ever attempted
/// (10/10 M10 coordinators hit this; 8/10 never got past it). This gives it
/// the SAME shape/budget-rule auto-answer under a headless Auto run
/// (`headless: true`, the same flag the goal-scoped path already checks),
/// never a goal card to park (there is none) and never a non-gate failure
/// (nothing useful to auto-answer for a session with no goal to route to).
///
/// `headless` is a plain argument, not read from the process-global
/// headless dispatch policy slot here, for the same reason
/// `try_auto_answer_reservation_gate` takes it as one: a `--lib` test that
/// both installs a headless policy AND asserts its absence elsewhere in this
/// binary is an order-dependent flakiness risk unless it holds Lane T7's
/// `take_headless_dispatch_policy_for_test` guard under the shared
/// `serial(headless_dispatch_policy)` key.
///
/// `provider`/`model` (F3, harness loop, Round F, from Lane G9's HIGH
/// finding) are the exact live identity of the call that just failed, so a
/// grant minted here can only ever be reused by a retry under that SAME
/// provider/model (see `try_auto_answer_reservation_gate`'s own doc
/// comment).
#[allow(clippy::too_many_arguments)]
async fn session_scoped_provider_authorization_failure(
    session_manager: &Arc<SessionManager>,
    session: &Session,
    invocation_id: &str,
    failure: &ProviderAuthorizationFailure,
    detail: String,
    headless: bool,
    provider: &str,
    model: &str,
) -> ProviderAuthorizationOutcome {
    let ProviderAuthorizationFailure::NeedsGate {
        scope,
        spent_usd,
        held_usd,
        requested_usd,
        ceiling_usd,
    } = failure
    else {
        // Only a spend gate has an auto-answerable shape; every other
        // failure kind has no goal card to park either, so it simply ends
        // the call exactly as it did before this lane.
        return ProviderAuthorizationOutcome::parked(detail);
    };
    if !headless {
        return ProviderAuthorizationOutcome::parked(detail);
    }
    let pool = match session_manager.pool_clone().await {
        Ok(pool) => pool,
        Err(error) => {
            return ProviderAuthorizationOutcome::parked(format!(
                "{detail}; could not open Decision Inbox: {error}"
            ))
        }
    };

    // Dedupe: a coordinator that keeps hitting the same gate on successive
    // calls must not file a fresh Decision Inbox card every time — same
    // purpose as the goal-scoped path's `find_open_decision_for_goal`, keyed
    // on the session instead since this decision carries no `goal_id`.
    let decision_id =
        match crate::decisions::find_open_reservation_gate_decision_for_session(&pool, &session.id)
            .await
        {
            Ok(Some(existing)) => existing.id,
            Ok(None) => {
                let cfg = crate::cost_router::budget::load_budget_config();
                let request = crate::cost_router::budget::with_requesting_session_id(
                    crate::cost_router::budget::reservation_gate_decision_request(
                        *scope,
                        *spent_usd,
                        *held_usd,
                        *requested_usd,
                        *ceiling_usd,
                        match scope {
                            crate::cost_router::budget::BudgetScope::Task => cfg.task.gate,
                            crate::cost_router::budget::BudgetScope::Session => cfg.session.gate,
                        },
                        None, // no goal_id — this is the coordinator's own call
                        None,
                    ),
                    &session.id,
                );
                match crate::decisions::create_decision(&pool, request).await {
                    Ok(created) => created.id,
                    Err(error) => {
                        tracing::error!(
                            session_id = %session.id,
                            "could not create session-scoped provider budget decision: {error}"
                        );
                        return ProviderAuthorizationOutcome::parked(detail);
                    }
                }
            }
            Err(error) => {
                tracing::warn!(
                    session_id = %session.id,
                    "could not look up an existing session-scoped gate decision ({error}); \
                     proceeding without one this call"
                );
                return ProviderAuthorizationOutcome::parked(detail);
            }
        };

    // M2 (harness loop, Round F, Lane F3, from Lane G9's MEDIUM finding):
    // `NeedsGate` is pure arithmetic over `cost_ledger`/`cost_reservations`
    // totals — answering the decision changes nothing about those totals,
    // so a long-running headless session can hit this gate, get
    // auto-answered, spend more, hit it again, and so on repeatedly with no
    // cap other than the hard ceiling eventually refusing outright (bounded
    // only by `(hard - gate) / amount_usd`, not by a fixed count). Count
    // this session's own PAST session-scoped auto-answers (`kind = 'choice'
    // AND goal_id IS NULL` — excludes the goal-scoped path's decisions,
    // which use the SAME `acted_by` value but always carry a `goal_id`) and
    // stop auto-answering once the cap is reached: the decision this call
    // just resolved onto (`decision_id`) stays open, honestly escalated to
    // a human via the ordinary Decision Inbox, instead of being silently
    // rubber-stamped again.
    let auto_answer_count = session_scoped_auto_answer_count(&pool, &session.id).await;
    let cap = session_gate_auto_answer_cap();
    if auto_answer_count >= cap {
        tracing::warn!(
            session_id = %session.id,
            decision_id,
            auto_answer_count,
            cap,
            "session-scoped reservation gate auto-answer cap reached; escalating to a human \
             instead of auto-answering again"
        );
        // Y4a: the cap is reached, so nothing was auto-answered and no
        // reservation was minted — `gate_grant_minted: false` is what tells
        // the coordinator NOT to retry this call in place.
        return ProviderAuthorizationOutcome::parked(detail);
    }

    if let Some(retry_outcome) = try_auto_answer_reservation_gate(
        &pool,
        &session.id,
        session,
        invocation_id,
        failure,
        &decision_id,
        headless,
        session_manager,
        &detail,
        provider,
        model,
    )
    .await
    {
        return retry_outcome;
    }
    ProviderAuthorizationOutcome::parked(detail)
}

/// M2 (harness loop, Round F, Lane F3, from Lane G9's MEDIUM finding):
/// default cap on how many times a single session's reservation gate may be
/// headlessly auto-answered before honesty requires escalating to a human
/// instead. Overridable via `PERMAGENT_SESSION_GATE_AUTO_ANSWER_CAP`
/// (any non-positive or unparsable value falls back to the default) so an
/// operator — or a test — can tighten it without recompiling.
const SESSION_GATE_AUTO_ANSWER_CAP_DEFAULT: u32 = 5;

fn session_gate_auto_answer_cap() -> u32 {
    std::env::var("PERMAGENT_SESSION_GATE_AUTO_ANSWER_CAP")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(SESSION_GATE_AUTO_ANSWER_CAP_DEFAULT)
}

/// How many session-scoped reservation-gate decisions for `session_id` have
/// already been headlessly auto-answered (`acted_by = ACTOR_HEADLESS_
/// POLICY`, `status = 'answered'`, `kind = 'choice'`, `goal_id IS NULL`) —
/// see [`session_scoped_provider_authorization_failure`]'s own cap check.
/// `goal_id IS NULL` scopes this strictly to the session-scoped path: the
/// goal-scoped gate (`Agent::handle_provider_authorization_failure`) shares
/// the SAME `try_auto_answer_reservation_gate`/`ACTOR_HEADLESS_POLICY`
/// machinery but always stamps a `goal_id`, so it can never inflate this
/// session's own count. A query failure counts as zero rather than refusing
/// the call outright — this cap is a courtesy backstop, not a new failure
/// mode on top of the existing hard-ceiling one.
async fn session_scoped_auto_answer_count(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    session_id: &str,
) -> u32 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM decisions
         WHERE kind = 'choice' AND goal_id IS NULL AND status = 'answered'
           AND acted_by = ?
           AND json_extract(payload_json, '$.reservation_gate.requesting_session_id') = ?",
    )
    .bind(crate::decisions::ACTOR_HEADLESS_POLICY)
    .bind(session_id)
    .fetch_one(pool)
    .await
    .map(|count| count.max(0) as u32)
    .unwrap_or(0)
}

/// Per-goal counterpart of the session cap, shared by all workers for a goal.
const GOAL_GATE_AUTO_ANSWER_CAP_DEFAULT: u32 = 8;

fn goal_gate_auto_answer_cap() -> u32 {
    std::env::var("PERMAGENT_GOAL_GATE_AUTO_ANSWER_CAP")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(GOAL_GATE_AUTO_ANSWER_CAP_DEFAULT)
}

/// NULL goal_id rows cannot contribute. Match the session counter's fallback
/// on query failure; the hard ceiling remains independently enforced.
async fn goal_scoped_auto_answer_count(pool: &sqlx::Pool<sqlx::Sqlite>, goal_id: &str) -> u32 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM decisions
         WHERE kind = 'choice' AND goal_id = ? AND status = 'answered'
           AND acted_by = ?
           AND json_extract(payload_json, '$.reservation_gate.requesting_session_id') IS NOT NULL",
    )
    .bind(goal_id)
    .bind(crate::decisions::ACTOR_HEADLESS_POLICY)
    .fetch_one(pool)
    .await
    .map(|count| count.max(0) as u32)
    .unwrap_or(0)
}

/// Check the goal cap immediately before auto-answering, with an explicit
/// headless seam so tests need no process-global policy.
#[allow(clippy::too_many_arguments)]
async fn try_auto_answer_goal_scoped_reservation_gate(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    goal_id: &str,
    session: &Session,
    invocation_id: &str,
    failure: &ProviderAuthorizationFailure,
    decision_id: &str,
    headless: bool,
    session_manager: &Arc<SessionManager>,
    detail: &str,
    provider: &str,
    model: &str,
) -> Option<ProviderAuthorizationOutcome> {
    if headless {
        let auto_answer_count = goal_scoped_auto_answer_count(pool, goal_id).await;
        let cap = goal_gate_auto_answer_cap();
        if auto_answer_count >= cap {
            tracing::warn!(
                goal_id,
                decision_id,
                auto_answer_count,
                cap,
                "goal-scoped reservation gate auto-answer cap reached; escalating to a human \
                 instead of auto-answering again"
            );
            return Some(ProviderAuthorizationOutcome::parked(detail.to_string()));
        }
    }
    try_auto_answer_reservation_gate(
        pool,
        goal_id,
        session,
        invocation_id,
        failure,
        decision_id,
        headless,
        session_manager,
        detail,
        provider,
        model,
    )
    .await
}

impl Agent {
    /// Immutable identity captured immediately before a provider dispatch.
    /// Keeping these fields outside the mutable session/provider state prevents
    /// a provider switch on the next turn from rewriting this invocation's
    /// attribution.
    ///
    /// The request's system prompt, messages and tools are known here, so the
    /// hold is sized to the prompt actually being sent (see
    /// [`counted_prompt_tokens`]) instead of the whole context window, which is
    /// what refused the harness's first call ("requested bound $1.56, ceiling
    /// $1.00" for a 30k-token prompt).
    // This is the only Agent-level entry point. The request-blind
    // `reserve_provider_invocation` that used to sit beside it was deleted once
    // both streamed-turn call sites moved here: left in place it had no caller
    // (clippy `-D warnings` fails on the dead_code), and any new caller would
    // have silently reserved the whole context window again.
    // Called by the streamed turn in `agents::agent` (first reservation and
    // the Y4a single retry), with the same request it then streams.
    // `provenance` (v60, node A4) is the loop's pre-dispatch account of why
    // this call is made; see `turn_call_provenance`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn reserve_provider_invocation_for_request(
        &self,
        session: &Session,
        provider: &Arc<dyn Provider>,
        invocation_id: String,
        system_prompt: &SystemPromptParts,
        messages: &[Message],
        tools: &[Tool],
        provenance: CallProvenance,
    ) -> std::result::Result<ProviderInvocationContext, ProviderAuthorizationFailure> {
        let counted = counted_prompt_tokens(
            &[
                system_prompt.stable_prefix(),
                system_prompt.volatile_suffix(),
            ],
            messages,
            tools,
        )
        .await;
        reserve_provider_invocation_for_model(
            Arc::clone(&self.config.session_manager),
            session,
            provider.get_name(),
            provider.cost_tier(),
            provider.get_model_config(),
            provider.retry_config().max_physical_attempts(),
            invocation_id,
            counted,
            provenance,
        )
        .await
    }

    /// Make a budget denial durable for a goal worker, then park and stop it.
    /// The requested bound is authorization-only and is intentionally never
    /// presented as already-spent money.
    ///
    /// Y4a: the returned [`ProviderAuthorizationOutcome::gate_grant_minted`]
    /// tells the caller whether a headless auto-answer left a spendable
    /// reservation behind under `invocation_id` — see that type's doc comment
    /// for why the caller must retry once on `true` rather than ending the
    /// turn.
    pub(crate) async fn handle_provider_authorization_failure(
        &self,
        session: &Session,
        // E1 (harness loop, Round E, from Lane F9 forensics 2026-09-10): the
        // exact reservation identity of the call that failed, so a
        // successful gate auto-answer below can grant a reservation under
        // THIS SAME invocation_id — a retry that reuses it then skips the
        // gate's threshold math entirely (see
        // `SessionManager::grant_reservation_after_gate_approval`) instead
        // of re-asking it for a request that already got a yes.
        invocation_id: &str,
        failure: &ProviderAuthorizationFailure,
        // F3 (harness loop, Round F, from Lane G9's HIGH finding): the exact
        // live provider/model of the call that just failed (captured at the
        // mint site in `agents::agent`, independent of any later provider
        // switch) — threaded down to the grant so a reuse attempt can be
        // checked against it.
        provider: &str,
        model: &str,
    ) -> ProviderAuthorizationOutcome {
        let detail = failure.to_string();
        let Some(card_id) = goal_id(&session.extension_data) else {
            // I1 (harness loop, 2026-09-10, from M10's forensics): a session
            // with no goal_id — the coordinator's own provider calls, never
            // a dispatched worker's — has no goal card to park through the
            // rest of this function, but under a headless Auto run it still
            // deserves the SAME shape/budget-rule auto-answer a worker's
            // gate gets, so its own blocked call can retry in place instead
            // of silently ending its turn (M10: 10/10 coordinators hit
            // exactly this, 8/10 never got past it).
            return self
                .handle_session_scoped_provider_authorization_failure(
                    session,
                    invocation_id,
                    failure,
                    detail,
                    provider,
                    model,
                )
                .await;
        };
        let pool = match self.config.session_manager.pool_clone().await {
            Ok(pool) => pool,
            Err(error) => {
                return ProviderAuthorizationOutcome::parked(format!(
                    "{detail}; could not open Decision Inbox: {error}"
                ))
            }
        };

        // Refused/unknown outcomes have no truthful BudgetVerdict (in
        // particular, an unknown provider result is not zero dollars). Use the
        // existing unblock decision shape and the sole guarded park path. A
        // gate fallback remains a choice so the user can actually authorize
        // the next attempt if the worker/session mapping raced with dispatch.
        let is_gate = matches!(failure, ProviderAuthorizationFailure::NeedsGate { .. });
        let kind = if is_gate { "choice" } else { "unblock" };
        let already_open = crate::decisions::find_open_decision_for_goal(&pool, &card_id, kind)
            .await
            .ok()
            .flatten()
            .is_some_and(|decision| {
                !is_gate || decision.headline.contains("authorization needs approval")
            });
        // R2 (harness loop, Lane D1, from Lane F8 forensics): the decision id
        // this call actually resolved onto — freshly created, or the one
        // already open from a raced prior attempt — so a headless run can
        // try to auto-answer it below. `None` whenever nothing gate-shaped
        // exists yet (a non-gate `unblock` decision, or the create failed).
        let mut gate_decision_id: Option<String> = None;
        if !already_open {
            let request = if let ProviderAuthorizationFailure::NeedsGate {
                scope,
                spent_usd,
                held_usd,
                requested_usd,
                ceiling_usd,
            } = failure
            {
                let cfg = crate::cost_router::budget::load_budget_config();
                crate::cost_router::budget::reservation_gate_decision_request(
                    *scope,
                    *spent_usd,
                    *held_usd,
                    *requested_usd,
                    *ceiling_usd,
                    match scope {
                        crate::cost_router::budget::BudgetScope::Task => cfg.task.gate,
                        crate::cost_router::budget::BudgetScope::Session => cfg.session.gate,
                    },
                    Some(card_id.clone()),
                    None,
                )
            } else {
                let reason = match failure {
                    ProviderAuthorizationFailure::Refused { .. } => {
                        crate::decisions::UnblockReason::TokenBudget
                    }
                    _ => crate::decisions::UnblockReason::Stuck,
                };
                let payload = serde_json::to_value(crate::decisions::UnblockPayload {
                    reason,
                    spent: None,
                    cap: None,
                    cause: Some(crate::decisions::UnblockCause::SpendBlocked),
                    salvage: None,
                })
                .unwrap_or_default();
                crate::decisions::NewDecision {
                    kind: kind.to_string(),
                    goal_id: Some(card_id.clone()),
                    project_id: None,
                    headline: Some(crate::decisions::truncate_for_headline(
                        "Provider spend authorization blocked this goal",
                    )),
                    detail: Some(detail.clone()),
                    payload,
                    ..Default::default()
                }
            };
            match crate::decisions::create_decision(&pool, request).await {
                Ok(created) => {
                    if is_gate {
                        gate_decision_id = Some(created.id);
                    }
                }
                Err(error) => {
                    tracing::error!(goal_id = %card_id, "could not create provider budget decision: {error}");
                }
            }
        } else if is_gate {
            gate_decision_id =
                crate::decisions::find_open_decision_for_goal(&pool, &card_id, "choice")
                    .await
                    .ok()
                    .flatten()
                    .filter(|decision| decision.headline.contains("authorization needs approval"))
                    .map(|decision| decision.id);
        }

        // R2 (harness loop, Lane D1, from Lane F8 forensics 2026-09-10): fx01
        // stalled behind exactly this gate forever with no auto-answer path
        // — only `roadmap_create`/`approve_review` had one. Give a headless
        // Auto run (a `HeadlessDispatchPolicy` installed — checked via the
        // same public seam the CLI's own drain summary uses,
        // `headless_auto_land_status`, which is `Some` iff a headless policy
        // exists for this process, whatever its `auto_land` value) one
        // bounded, shape-scoped chance to sail through before parking.
        // `auto_approve_reservation_gate` re-validates the request against
        // the run's real hard ceiling itself — this call site trusts nothing
        // beyond "try it, see if it was accepted".
        //
        // E1 (harness loop, Round E, from Lane F9 forensics 2026-09-10): on
        // success this USED TO requeue the goal (`goal_transition::
        // requeue_goal`), reassigning it to a brand-new session while THIS
        // session — the one that just got its call authorized — kept
        // running under the old assignment. F9 traced this to 4/6 of
        // Round D's M9 failures: the abandoned session often finished real,
        // correct, committed work that could then never land, because the
        // goal's own bookkeeping had already moved on. The fix: never touch
        // the goal's dispatch at all here. This session IS the goal's live
        // dispatched worker (it is, definitionally, the one making this
        // call) — it keeps that assignment and simply retries in place.
        // `grant_reservation_after_gate_approval` grants the reservation
        // directly under `invocation_id` — the SAME identity this exact
        // call already failed under — so a retry that reuses it (rather
        // than minting a fresh one) skips `reserve_provider_invocation`'s
        // gate/hard threshold math entirely instead of re-asking the gate
        // for a request that already got a yes (see that function's own
        // doc comment). Any refusal (not headless, past budget, already
        // resolved by a human) falls straight through to the unchanged
        // park path below, exactly as before.
        if let Some(decision_id) = gate_decision_id.as_deref() {
            let headless =
                crate::agents::platform_extensions::orchestrator::headless_auto_land_status(
                    &self.config.session_manager,
                )
                .await
                .is_some();
            if let Some(retry_outcome) = try_auto_answer_goal_scoped_reservation_gate(
                &pool,
                &card_id,
                session,
                invocation_id,
                failure,
                decision_id,
                headless,
                &self.config.session_manager,
                &detail,
                provider,
                model,
            )
            .await
            {
                return retry_outcome;
            }
        }
        if let Err(error) = crate::goal_transition::park_goal(
            &pool,
            &card_id,
            crate::decisions::ACTOR_SYSTEM,
            &detail,
        )
        .await
        {
            tracing::warn!(goal_id = %card_id, "could not park blocked provider goal: {error}");
        }
        if let Some(kill) =
            crate::agents::platform_extensions::orchestrator::take_goal_worker(&card_id)
        {
            kill.kill();
        }
        ProviderAuthorizationOutcome::parked(detail)
    }

    /// I1 (harness loop, 2026-09-10, from M10's forensics): the SESSION-
    /// scoped counterpart of [`Agent::handle_provider_authorization_failure`]'s
    /// goal-scoped gate for a session with no `goal_id` — the coordinator's
    /// own provider calls. M10 found the coordinator's calls hit the exact
    /// same reservation gate as a worker's (bounded by the same task/session
    /// budget rules) but `handle_provider_authorization_failure`'s entire
    /// gate-decision/auto-answer/retry machinery was a structural no-op for
    /// it — `goal_id(&session.extension_data)` is `None`, so it returned
    /// `detail` immediately with no decision ever raised, no auto-answer
    /// ever attempted (10/10 M10 coordinators hit this; 8/10 never got past
    /// it). This gives it the SAME shape/budget-rule auto-answer under a
    /// headless Auto run (`HeadlessDispatchPolicy` installed — the same
    /// `headless` flag the goal-scoped path already checks), never a goal
    /// card to park (there is none) and never a non-gate failure (nothing
    /// useful to auto-answer for a session with no goal to route to).
    async fn handle_session_scoped_provider_authorization_failure(
        &self,
        session: &Session,
        invocation_id: &str,
        failure: &ProviderAuthorizationFailure,
        detail: String,
        provider: &str,
        model: &str,
    ) -> ProviderAuthorizationOutcome {
        // Read via the SAME public seam the goal-scoped path already uses
        // (`headless_auto_land_status(...).is_some()` — `true` iff a
        // headless dispatch policy is installed for this process, whatever
        // its own `auto_land` value). Kept in this thin wrapper, not the
        // testable function below, for the exact reason
        // `try_auto_answer_reservation_gate`'s own doc comment gives: the
        // backing `OnceLock` is a real shared-state hazard across this
        // binary's `--lib` unit tests, so the actual decision logic takes
        // `headless` as a plain argument and this is the only call site that
        // ever reads the global.
        let headless = crate::agents::platform_extensions::orchestrator::headless_auto_land_status(
            &self.config.session_manager,
        )
        .await
        .is_some();
        session_scoped_provider_authorization_failure(
            &self.config.session_manager,
            session,
            invocation_id,
            failure,
            detail,
            headless,
            provider,
            model,
        )
        .await
    }

    pub async fn prepare_tools_and_prompt(
        &self,
        session_id: &str,
        working_dir: &std::path::Path,
    ) -> Result<(Vec<Tool>, Vec<Tool>, SystemPromptParts)> {
        // Get tools from extension manager
        let mut tools = self.list_tools(session_id, None).await;

        // Add frontend tools
        let frontend_tools = self.frontend_tools.lock().await;
        for frontend_tool in frontend_tools.values() {
            tools.push(frontend_tool.tool.clone());
        }

        #[cfg(feature = "code-mode")]
        let code_execution_active = self
            .extension_manager
            .is_extension_enabled(code_execution::EXTENSION_NAME)
            .await;
        #[cfg(not(feature = "code-mode"))]
        let code_execution_active = false;
        #[cfg(feature = "code-mode")]
        if code_execution_active {
            let disclosure_style =
                crate::agents::platform_extensions::code_execution::get_tool_disclosure();

            tools = tools
                .into_iter()
                .filter_map(|mut t| match disclosure_style {
                    pctx_code_mode::config::ToolDisclosure::Catalog
                    | pctx_code_mode::config::ToolDisclosure::Filesystem => {
                        // in catalog & filesystem styles, progressive search is handled
                        // by pctx, so we want to omit all non-first-class extensions
                        // from the standard tool list
                        if crate::agents::extension_manager::get_tool_owner(&t).is_some_and(|o| {
                            crate::agents::extension_manager::is_first_class_extension(&o)
                        }) {
                            Some(t)
                        } else {
                            None
                        }
                    }
                    pctx_code_mode::config::ToolDisclosure::Sidecar => {
                        // in sidecar style there is no progressive search, just a way to chain tools
                        // together with typescript
                        // add output schema to description since many model providers drop the
                        // output schema when presenting tools to the model
                        let output_schema = t
                            .output_schema
                            .as_ref()
                            .map(|s| serde_json::json!(s).to_string())
                            .unwrap_or("unknown".to_string());
                        let description_extension = format!(
                            "The successful return schema of this tool is:\n{output_schema}"
                        );

                        t.description = Some(
                            t.description
                                .map(|t| format!("{t}\n{description_extension}"))
                                .unwrap_or(description_extension)
                                .into(),
                        );

                        Some(t)
                    }
                })
                .collect();
        }

        // Filter out tools not visible to the model per MCP Apps visibility spec.
        // Tools with `_meta.ui.visibility` that doesn't include "model" are app-only.
        tools.retain(is_tool_visible_to_model);

        // Stable tool ordering is important for multi session prompt caching.
        tools.sort_by(|a, b| a.name.cmp(&b.name));

        // Inject saved skills into system prompt (Hermes feedback loop)
        if let Ok(pool) = self.config.session_manager.pool_clone().await {
            if let Ok(Some(skills_prompt)) = crate::skills::build_skills_prompt(&pool).await {
                let mut pm = self.prompt_manager.lock().await;
                pm.add_system_prompt_extra("saved_skills".to_string(), skills_prompt);
            }
        }

        // First-run guided-tour offer (once). Suppressed once the user engages or
        // declines (the `load_feature_lesson` tool sets `tour_completed`).
        let tour_completed = crate::config::Config::global()
            .get_param::<bool>(crate::agents::self_knowledge::TOUR_COMPLETED_KEY)
            .unwrap_or(false);
        // Lane R62f (A61 miss 5): nobody reads a headless run's replies, so it
        // never gets the tour offer or a learn-next hint — the coordinator
        // offered a codebase tour in 12 headless harness logs, a free-text
        // ending Rule 5 and R58a then have to police. The per-agent headless
        // flag is checked first; the headless CLI harness does not set it
        // (only the scheduler does), so the headless dispatch policy that CLI
        // installs — never installed by a daemon or interactive session —
        // covers it.
        let headless = self.is_headless()
            || crate::agents::platform_extensions::orchestrator::is_headless_dispatch_policy_installed();
        if headless {
            tracing::debug!(
                target: "permagent::agents::reply_parts",
                session_id,
                "onboarding offers suppressed for a headless session"
            );
        } else if !tour_completed {
            let mut pm = self.prompt_manager.lock().await;
            pm.add_system_prompt_extra(
                "tour_offer".to_string(),
                "This user has not been shown around yet. Early in the conversation, once, \
                 warmly offer a short guided tour of what you can do. If they accept, run it \
                 with the `tour` skill (call `load_feature_lesson` per feature). If they \
                 decline, call `load_feature_lesson` with feature_id \"decline\" so you don't \
                 ask again. Offer only once — never nag."
                    .to_string(),
            );
        } else {
            // Past first-run: the onboarding coach may gently surface ONE feature
            // the user hasn't tried yet — at most once a day (its own config
            // cooldown), through this same in-context seam, not a new channel.
            if let Some(hint) =
                crate::agents::self_knowledge::teachable::proactive_learn_next_hint()
            {
                let mut pm = self.prompt_manager.lock().await;
                pm.add_system_prompt_extra("learn_next_offer".to_string(), hint);
            }
        }

        // Prepare system prompt
        let worker_key = {
            let pm = self.prompt_manager.lock().await;
            pm.worker_key().map(str::to_owned)
        };
        let mut extensions_info = self
            .extension_manager
            .get_extensions_info(working_dir)
            .await;
        for ext in &mut extensions_info {
            if crate::public_apis::is_public_apis_extension(&ext.name) {
                ext.instructions =
                    crate::public_apis::instructions_for_agent(worker_key.as_deref());
            }
        }

        // The self-knowledge capability inventory is scoped to this list only
        // for a session that DECLARED an explicit extension set — recipe/CLI
        // runs (`GoosePlatform::GooseCli`, e.g. `permagent run --recipe`),
        // which load a fixed, small roster and never add to it. The daemon's
        // resident chat sessions (`GoosePlatform::GooseDesktop` — Aria) always
        // pass `None` here regardless of how many extensions happen to be
        // active this turn: describing everything Permagent can do is a
        // product contract for that agent, not something a smaller loaded set
        // should narrow. See #1090 — a coding-harness session with 2
        // extensions got a 69KB inventory of all 33 registered ones.
        let declared_extensions = matches!(
            self.config.goose_platform,
            crate::agents::GoosePlatform::GooseCli
        )
        .then(|| extensions_info.iter().map(|e| e.name.clone()).collect());

        let (extension_count, tool_count) = self
            .extension_manager
            .get_extension_and_tool_counts(session_id)
            .await;

        // Get model name from provider
        let provider = self.provider().await?;
        let model_config = provider.get_model_config();

        // M3: tell the model which model it is. Read from the live provider
        // handle every turn — the SAME one the composer footer reads — so a
        // mid-session failover moves this line with it instead of leaving the
        // model to guess (and, on 2026-08-31, to guess wrong).
        {
            let mut pm = self.prompt_manager.lock().await;
            pm.add_system_prompt_extra(
                crate::agents::prompt_manager::MODEL_IDENTITY_KEY.to_string(),
                crate::agents::prompt_manager::model_identity_line(
                    provider.get_name(),
                    &model_config.model_name,
                ),
            );
        }

        let goose_mode = *self.current_goose_mode.lock().await;

        // Live scheduled-job count for the self-knowledge brief (Queryable). The
        // scheduler is only reachable here (async, with the service handle), not
        // inside the synchronous prompt builder.
        let scheduled_job_count = match self.config.scheduler_service.as_ref() {
            Some(scheduler) => Some(scheduler.list_scheduled_jobs().await.len()),
            None => None,
        };

        // Dispatchable-worker list for the self-knowledge brief, gated on the
        // orchestrator being active. The availability probe may block
        // (`model_loaded:` does HTTP), so it runs off the async runtime.
        let dispatchable_workers = if crate::agents::self_knowledge::orchestrator_dispatch_active()
        {
            tokio::task::spawn_blocking(|| {
                let config = crate::config::agent_identity::load_agent_config();
                crate::agents::self_knowledge::dispatchable_workers_from_config(&config)
            })
            .await
            .unwrap_or_default()
        } else {
            Vec::new()
        };

        // Unread briefings from the worker agents. NOT gated on the
        // orchestrator: the agents that file these (Steward, Watcher) run on
        // their own schedules whether or not orchestration is enabled, so
        // gating this would silently mute them on the default profile — which
        // is exactly the profile this machine runs.
        //
        // Capped: the brief rides in every turn's system prompt, so an
        // unbounded list would grow the prompt without bound. `unacknowledged`
        // orders by severity first, so the cap drops routine chatter rather
        // than the thing waiting on a decision.
        const MAX_BRIEFINGS_IN_PROMPT: i64 = 5;
        let agent_briefings = match self.config.session_manager.pool_clone().await {
            Ok(pool) => {
                let unread = crate::briefings::unacknowledged(&pool, MAX_BRIEFINGS_IN_PROMPT).await;

                // Acknowledge the FYI ones now that they have been put in front
                // of Henry — otherwise they ride in every subsequent prompt
                // forever and crowd out newer reports.
                //
                // Attention/ActionRequired deliberately survive: those are
                // waiting on something (usually a human decision on a card),
                // and dropping one because Henry happened to see it once is how
                // a pending force-push quietly stops being mentioned. They
                // clear when the underlying thing resolves — see the resolve
                // path in `crate::briefings`.
                let fyi: Vec<String> = unread
                    .iter()
                    .filter(|b| b.severity == crate::briefings::Severity::Info)
                    .map(|b| b.id.clone())
                    .collect();
                if !fyi.is_empty() {
                    let _ = crate::briefings::acknowledge(&pool, &fyi).await;
                }

                Some(
                    unread
                        .into_iter()
                        .map(|b| crate::agents::self_knowledge::BriefingLine {
                            from: crate::briefings::display_name_for(&b.from_agent),
                            severity: b.severity.render().to_string(),
                            summary: b.summary,
                        })
                        .collect(),
                )
            }
            // No pool (tests, degraded boot). `None`, NOT an empty list — the
            // brief must omit the section rather than tell Henry his agents
            // have nothing pending on the strength of a read that never ran.
            Err(_) => None,
        };

        let prompt_manager = self.prompt_manager.lock().await;
        let mut system_prompt = prompt_manager
            .builder()
            .with_extensions(extensions_info.into_iter())
            .with_frontend_instructions(self.frontend_instructions.lock().await.clone())
            .with_extension_and_tool_counts(extension_count, tool_count)
            .with_code_execution_mode(code_execution_active)
            .with_hints(working_dir)
            .with_goose_mode(goose_mode)
            .with_scheduled_job_count(scheduled_job_count)
            .with_dispatchable_workers(dispatchable_workers)
            .with_agent_briefings(agent_briefings)
            .with_declared_extensions(declared_extensions)
            // Tool-calling discipline is a per-family concern, so the family
            // that will actually answer picks its own short overlay rather than
            // every model paying for the weakest reader's patches.
            .with_model_family_from(provider.get_name(), &model_config.model_name)
            .build_parts();

        // Handle toolshim if enabled
        let mut toolshim_tools = vec![];
        if model_config.toolshim {
            // If tool interpretation is enabled, modify the system prompt. The
            // tool-JSON instructions are as stable as the tool list itself, so
            // they belong in the cached prefix — `map_stable` keeps the volatile
            // tail behind them rather than folding it in.
            system_prompt =
                system_prompt.map_stable(|s| modify_system_prompt_for_tool_json(&s, &tools));
            // Make a copy of tools before emptying
            toolshim_tools = tools.clone();
            // Empty the tools vector for provider completion
            tools = vec![];
        }

        Ok((tools, toolshim_tools, system_prompt))
    }

    #[tracing::instrument(
        skip(provider, session_id, system_prompt, messages, tools, toolshim_tools),
        fields(session.id = %session_id)
    )]
    pub(crate) async fn stream_response_from_provider(
        provider: Arc<dyn Provider>,
        session_id: &str,
        system_prompt: &SystemPromptParts,
        messages: &[Message],
        tools: &[Tool],
        toolshim_tools: &[Tool],
        // The output cap `reserve_provider_invocation_for_request` priced
        // this call's hold against
        // (`ProviderInvocationContext::reserved_output_tokens`), or `None`
        // for unpaid work. Applied to the request's own model config below
        // so the provider is asked to respect the same number the
        // reservation charged for — see `apply_reserved_output_cap`.
        reserved_output_tokens: Option<usize>,
    ) -> Result<MessageStream, ProviderError> {
        let config = provider.get_model_config();

        let filtered_messages: Vec<Message> = messages
            .iter()
            .filter(|m| m.is_agent_visible())
            .map(|m| m.agent_visible_content())
            .collect();

        // Observation masking: tool-result bodies older than the configured
        // window are swapped for a placeholder in what the MODEL sees (the
        // stored transcript keeps them). Block-quantized so the masked prefix
        // changes only once every window's worth of turns — see
        // `observation_mask` for the measured cache trade-off.
        let keep_turns = crate::agents::observation_mask::configured_keep_turns();
        let filtered_messages =
            crate::agents::observation_mask::mask_old_observations(&filtered_messages, keep_turns);

        // Convert tool messages to text if toolshim is enabled
        let messages_for_provider = if config.toolshim {
            convert_tool_messages_to_text(&filtered_messages)
        } else {
            Conversation::new_unvalidated(filtered_messages)
        };

        // Clone owned data to move into the async stream
        let system_prompt = system_prompt.to_owned();
        let tools = tools.to_owned();
        let toolshim_tools = toolshim_tools.to_owned();
        let provider = provider.clone();

        // Capture errors during stream creation and return them as part of the stream
        // so they can be handled by the existing error handling logic in the agent
        //
        // The reservation is only honest if the request cannot ask for more
        // than it priced: `apply_reserved_output_cap` overrides this
        // dispatch's own `max_tokens` with the reserved cap (never the
        // model's raw, possibly much larger, `max_tokens`) before it is
        // handed to the provider, which is what turns into
        // `max_output_tokens` on the wire
        // (`providers::formats::openai_responses::create_responses_request`).
        let model_config =
            apply_reserved_output_cap(provider.get_model_config(), reserved_output_tokens);
        debug!("WAITING_LLM_STREAM_START");
        let stream_result = provider
            // permagent-dispatch: seam=agent_primary_stream_transport_v1 class=excluded reason=caller_reservation_settlement authority=agent_primary_stream
            .stream_split(
                &model_config,
                session_id,
                &system_prompt,
                messages_for_provider.messages(),
                &tools,
            )
            .await;
        debug!("WAITING_LLM_STREAM_END");

        // If there was an error creating the stream, return a stream that yields that error
        let mut stream = match stream_result {
            Ok(s) => s,
            Err(e) => {
                let enhanced_error = enhance_model_error(e, &provider).await;
                // Return a stream that immediately yields the error
                // This allows the error to be caught by existing error handling in agent.rs
                return Ok(Box::pin(try_stream! {
                    yield Err(enhanced_error)?;
                }));
            }
        };

        Ok(Box::pin(try_stream! {
            while let Some(result) = stream.next().await {
                let (mut message, usage) = result?;

                // Store the model information in the global store
                if let Some(usage) = usage.as_ref() {
                    crate::providers::base::set_current_model(&usage.model);
                }

                // Post-process / structure the response only if tool interpretation is enabled
                if message.is_some() && config.toolshim {
                    message = Some(toolshim_postprocess(message.unwrap(), &toolshim_tools).await?);
                }

                yield (message, usage);
            }
        }))
    }

    /// Categorize tool requests from the response into different types
    /// Returns:
    /// - frontend_requests: Tool requests that should be handled by the frontend
    /// - other_requests: All other tool requests (including requests to enable extensions)
    /// - filtered_message: The original message with frontend tool requests removed
    pub(crate) async fn categorize_tool_requests(
        &self,
        response: &Message,
        tools: &[Tool],
        suppress_replayed_thinking: bool,
    ) -> (Vec<ToolRequest>, Vec<ToolRequest>, Message) {
        // First collect all tool requests with coercion applied
        let tool_requests: Vec<ToolRequest> = response
            .content
            .iter()
            .filter_map(|content| {
                if let MessageContent::ToolRequest(req) = content {
                    let mut coerced_req = req.clone();

                    if let Ok(ref mut tool_call) = coerced_req.tool_call {
                        if let Some(tool) = tools.iter().find(|t| t.name == tool_call.name) {
                            let schema_value = Value::Object(tool.input_schema.as_ref().clone());
                            tool_call.arguments =
                                coerce_tool_arguments(tool_call.arguments.clone(), &schema_value);

                            if let Some(ref meta) = tool.meta {
                                // Merge registry meta into existing tool_meta;
                                // existing keys win so provider markers (e.g.
                                // goose.external_dispatch) survive coercion.
                                let new_meta = serde_json::to_value(meta).ok();
                                coerced_req.tool_meta =
                                    match (coerced_req.tool_meta.take(), new_meta) {
                                        (
                                            Some(Value::Object(mut existing)),
                                            Some(Value::Object(new)),
                                        ) => {
                                            for (k, v) in new {
                                                existing.entry(k).or_insert(v);
                                            }
                                            Some(Value::Object(existing))
                                        }
                                        (None, new) => new,
                                        (existing, _) => existing,
                                    };
                            }
                        }
                    }

                    Some(coerced_req)
                } else {
                    None
                }
            })
            .collect();

        let has_tool_requests = !tool_requests.is_empty();
        let should_suppress_replayed_thinking = suppress_replayed_thinking && has_tool_requests;

        // Create a filtered message with frontend tool requests removed.
        // When a response contains tool calls, keep reasoning in the original
        // message for provider/state purposes but only suppress it from the
        // user-visible filtered message if the caller already surfaced
        // thinking earlier in this provider turn. That avoids replaying full
        // accumulated reasoning after streamed thought chunks while still
        // preserving final-only non-streaming thoughts.
        let mut filtered_content = Vec::new();
        let mut tool_request_index = 0;

        for content in &response.content {
            match content {
                MessageContent::ToolRequest(_) => {
                    if tool_request_index < tool_requests.len() {
                        let coerced_req = &tool_requests[tool_request_index];
                        tool_request_index += 1;

                        // Always keep externally-dispatched requests visible, even if
                        // their name happens to overlap a registered frontend tool —
                        // they're observation-only and must not be removed from history.
                        let should_include = if coerced_req.is_externally_dispatched() {
                            true
                        } else if let Ok(tool_call) = &coerced_req.tool_call {
                            !self.is_frontend_tool(&tool_call.name).await
                        } else {
                            true
                        };

                        if should_include {
                            filtered_content.push(MessageContent::ToolRequest(coerced_req.clone()));
                        }
                    }
                }
                MessageContent::Thinking(_) | MessageContent::RedactedThinking(_)
                    if should_suppress_replayed_thinking => {}
                _ => {
                    filtered_content.push(content.clone());
                }
            }
        }

        let mut filtered_message =
            Message::new(response.role.clone(), response.created, filtered_content);

        // Preserve the ID if it exists
        if let Some(id) = response.id.clone() {
            filtered_message = filtered_message.with_id(id);
        }

        // Categorize tool requests
        let mut frontend_requests = Vec::new();
        let mut other_requests = Vec::new();

        for request in tool_requests {
            // Skip externally-dispatched requests (e.g. claude-acp); the
            // provider already executed the tool. Stays in filtered_message.
            if request.is_externally_dispatched() {
                continue;
            }
            if let Ok(tool_call) = &request.tool_call {
                if self.is_frontend_tool(&tool_call.name).await {
                    frontend_requests.push(request);
                } else {
                    other_requests.push(request);
                }
            } else {
                // If there's an error in the tool call, add it to other_requests
                other_requests.push(request);
            }
        }

        (frontend_requests, other_requests, filtered_message)
    }

    pub(crate) async fn update_session_metrics_for_invocation(
        &self,
        session_id: &str,
        schedule_id: Option<String>,
        usage: &ProviderUsage,
        invocation: &ProviderInvocationContext,
        recognition_retrieval_id: Option<&str>,
        outcome: CallOutcome,
    ) -> Result<()> {
        record_provider_usage(
            Arc::clone(&self.config.session_manager),
            session_id,
            schedule_id,
            usage,
            false,
            Some(invocation),
            recognition_retrieval_id,
            outcome,
        )
        .await
    }
}

/// Append one provider response and all derived token/cost rollups.  This is
/// deliberately independent from `Agent` so background context maintenance
/// can use the same single Spectral transaction instead of creating a side
/// ledger or a best-effort metric path.
///
/// v60 (node A4): `outcome` is what response processing observed about this
/// call (tool-request count, completion kind); pass `CallOutcome::default()`
/// when the response shape was not observed. The call's pre-dispatch
/// provenance comes from `invocation` and is never inferred here.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn record_provider_usage(
    manager: Arc<SessionManager>,
    session_id: &str,
    schedule_id: Option<String>,
    usage: &ProviderUsage,
    is_compaction_usage: bool,
    invocation: Option<&ProviderInvocationContext>,
    recognition_retrieval_id: Option<&str>,
    outcome: CallOutcome,
) -> Result<()> {
    let session = manager.get_session(session_id, false).await?;

    // Accumulate in the DATABASE, not here.
    //
    // This used to read `session.accumulated_*`, add the new usage in Rust,
    // and write the absolute result. Two turns sharing a session — a
    // subagent alongside the main loop, or concurrent tool calls — both
    // read the same starting value and both write their own total, so the
    // second commit silently discards the first's tokens. That undercounts
    // spend, and the accumulated figures feed the spend caps.
    let (delta_total, delta_input, delta_output) = (
        usage.usage.total_tokens.unwrap_or(0),
        usage.usage.input_tokens.unwrap_or(0),
        usage.usage.output_tokens.unwrap_or(0),
    );

    let (current_total, current_input, current_output) = if is_compaction_usage {
        // After compaction: summary output becomes new input context
        let new_input = usage.usage.output_tokens;
        (new_input, new_input, None)
    } else {
        (
            usage.usage.total_tokens,
            usage.usage.input_tokens,
            usage.usage.output_tokens,
        )
    };

    // ── Per-call cost ledger (single source of truth for spend/attribution) ─
    // One row per provider response. Every money field is folded by the ONE
    // canonical `cost_of` (via `cost_breakdown`), so the ledger, the live
    // meter, and the verification digest can never disagree. For an
    // invocation-authorized call this write is fail-closed: accounting
    // failure aborts the turn instead of silently losing spend.
    let provider = invocation
        .map(|call| Some(call.provider.clone()))
        .unwrap_or_else(|| session.provider_name.clone());
    let model = invocation
        .map(|call| call.model.clone())
        .unwrap_or_else(|| usage.model.clone());
    // `maybe_get_pricing`, not `maybe_get_canonical_model(..).cost`: the
    // generated registry has no ROW at all for a newly-selectable model, so
    // the canonical lookup returns `None` and there is nothing to read a
    // price off. That is how 128 `deepseek-v4-flash` calls billed as $0.00
    // on 2026-08-23 — see `providers::canonical::published_prices`.
    let pricing = provider
        .as_deref()
        .and_then(|p| maybe_get_pricing(p, &model));
    let breakdown = pricing
        .as_ref()
        .and_then(|p| cost_breakdown(&usage.usage, p));

    // Ollama et al. run locally — not chargeable. Subscription/quota detection
    // is not yet wired (no per-provider plan signal here), so a priced remote
    // provider records as `paid_api`.
    let is_local = provider.as_deref().is_some_and(is_local_provider);
    let cost_tier = invocation.map(|call| call.cost_tier).unwrap_or_else(|| {
        if is_local {
            CostTier::LocalFree
        } else {
            CostTier::PaidApi
        }
    });

    // Not chargeable → cost 0 (never bill local/in-quota). Chargeable with a
    // known price → the folded `cost_of` total.
    //
    // Chargeable but UNPRICED → billed at the worst case we know of for that
    // provider, still flagged `is_estimated`. It used to record $0.00, and
    // because nothing downstream read the flag, the budget ceilings summed
    // 211 real calls as zero and could never fire (2026-08-24 health review).
    // An unknown price now fails CLOSED: an unmeasurable call can only make
    // the cap fire early, never late. `is_estimated` still marks it, so the
    // figure is never mistaken for a measured cost.
    let estimated_breakdown = if cost_tier.is_chargeable() && breakdown.is_none() {
        provider
            .as_deref()
            .and_then(worst_case_pricing)
            .and_then(|p| cost_breakdown(&usage.usage, &p))
    } else {
        None
    };
    if let Some(est) = &estimated_breakdown {
        tracing::warn!(
            model = %model,
            provider = provider.as_deref().unwrap_or("unknown"),
            estimated_cost_usd = est.total_cost,
            "no published price for this model — billing it at the provider's most \
             expensive known rate so the spend cap still applies"
        );
    }

    let (cost_usd, input_cost, output_cost, cache_read_cost, cache_write_cost, is_estimated) =
        match (cost_tier.is_chargeable(), &breakdown, &estimated_breakdown) {
            (false, _, _) => (0.0, 0.0, 0.0, 0.0, 0.0, false),
            (true, Some(b), _) => (
                b.total_cost,
                b.input_cost,
                b.output_cost,
                b.cache_read_cost,
                b.cache_write_cost,
                false,
            ),
            (true, None, Some(b)) => (
                b.total_cost,
                b.input_cost,
                b.output_cost,
                b.cache_read_cost,
                b.cache_write_cost,
                true,
            ),
            // Nothing priced anywhere in the registry — keep $0 rather than
            // inventing a number, and let `is_estimated` carry the warning.
            (true, None, None) => (0.0, 0.0, 0.0, 0.0, 0.0, true),
        };
    let cache_savings_usd = if cost_tier.is_chargeable() {
        pricing
            .as_ref()
            .map(|p| cache_savings_of(&usage.usage, p))
            .unwrap_or(0.0)
    } else {
        0.0
    };

    // Surface the prompt-cache hit rate for this response — reads /
    // (reads + creation), the day-one measurable signal for the cache
    // discipline (#717/#727). Emitted per call alongside the ledger's cost so
    // it is observable without a schema change; `None` (no log) when nothing
    // went through the cache this call.
    if let Some(cache_hit_rate) = cache_hit_rate_of(&usage.usage) {
        tracing::debug!(
            cache_hit_rate,
            cache_savings_usd,
            cost_usd,
            model = %model,
            "prompt-cache hit rate for provider response"
        );
    }

    // Interactive surfaces are User/Terminal; everything else (SubAgent,
    // Scheduled, Hidden, Gateway, Acp) is background/headless. One
    // definition, on the type itself.
    let is_headless = !session.session_type.is_interactive();

    let tok = |t: Option<i32>| t.unwrap_or(0).max(0) as i64;
    // `subagent_id`: a SubAgent session IS the subagent — its own id is the
    // identity a "cost run inside a subagent" query needs.
    // `parent_session_id` comes from the session row (set at spawn via
    // `create_session_with_parent`); ledger rows copy it so parent rollups
    // do not need a join back to `sessions`.
    let subagent_id =
        matches!(session.session_type, SessionType::SubAgent).then(|| session.id.clone());
    let row = CostLedgerRow {
        // ProviderUsage carries the invocation identity assigned by the
        // Agent. A missing identity is retained for legacy/manual callers;
        // it gets a fresh key and therefore cannot claim replay safety.
        call_id: invocation
            .map(|call| call.invocation_id.clone())
            .or_else(|| usage.invocation_id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        ts: chrono::Utc::now().to_rfc3339(),
        session_id: session_id.to_string(),
        parent_session_id: session.parent_session_id.clone(),
        task_id: budget_task_id(&session.extension_data),
        goal_id: goal_id(&session.extension_data),
        subagent_id,
        provider,
        model: Some(model),
        cost_tier,
        is_headless,
        input_tokens: tok(usage.usage.input_tokens),
        output_tokens: tok(usage.usage.output_tokens),
        cache_read_tokens: tok(usage.usage.cache_read_input_tokens),
        cache_write_tokens: tok(usage.usage.cache_write_input_tokens),
        input_cost,
        output_cost,
        cache_read_cost,
        cache_write_cost,
        cost_usd,
        cache_savings_usd,
        is_estimated,
        // Provenance was fixed before dispatch by the caller that knew why the
        // call was made; a call without an invocation context has none.
        provenance: invocation
            .map(|call| call.provenance.clone())
            .unwrap_or_default(),
        outcome,
    };
    // Token and money accounting are one durable operation. An unavailable
    // ledger must stop the turn rather than allow paid work to continue
    // with silently missing spend; duplicate invocation keys are handled as
    // an explicit successful no-op by the storage layer.
    if let Some(invocation) = invocation.filter(|call| call.cost_tier.is_chargeable()) {
        let reservation_id = invocation.reservation_id.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "paid provider invocation has no reservation; refusing to settle silently"
            )
        })?;
        manager
            .settle_provider_invocation(
                reservation_id,
                &row,
                schedule_id,
                current_total,
                current_input,
                current_output,
                delta_total,
                delta_input,
                delta_output,
            )
            .await?;
    } else {
        manager
            .append_usage_and_rollup(
                &row,
                schedule_id,
                current_total,
                current_input,
                current_output,
                delta_total,
                delta_input,
                delta_output,
            )
            .await?;
    }

    // Provider identity is copied by reference only after the authoritative
    // cost row is durable. The recognition writer resolves that call in
    // cost_ledger and enforces the recall/session match; attribution gaps do
    // not fail the ordinary reply.
    if let Some(retrieval_id) = recognition_retrieval_id {
        if let Some(invocation_id) = invocation
            .map(|call| call.invocation_id.as_str())
            .or(usage.invocation_id.as_deref())
        {
            match manager.storage().pool().await {
                Ok(pool) => match tokio::time::timeout(
                    std::time::Duration::from_millis(250),
                    crate::recognition::record_provider_invocation_reference(
                        pool,
                        retrieval_id,
                        session_id,
                        invocation_id,
                    ),
                )
                .await
                {
                    Ok(Ok(crate::recognition::ProviderAttributionWrite::Recorded))
                    | Ok(Ok(crate::recognition::ProviderAttributionWrite::Duplicate)) => {}
                    Ok(Ok(status)) => debug!(
                        target: "permagent::recognition",
                        retrieval_id = %retrieval_id,
                        ?status,
                        "provider attribution remains partial or unavailable"
                    ),
                    Ok(Err(error)) => debug!(
                        target: "permagent::recognition",
                        retrieval_id = %retrieval_id,
                        ?error,
                        "provider attribution write failed open"
                    ),
                    Err(_) => debug!(
                        target: "permagent::recognition",
                        retrieval_id = %retrieval_id,
                        "provider attribution write timed out; settlement remains durable"
                    ),
                },
                Err(error) => debug!(
                    target: "permagent::recognition",
                    retrieval_id = %retrieval_id,
                    ?error,
                    "provider attribution pool unavailable"
                ),
            }
        }
    }

    Ok(())
}

/// Count, with the local o200k tokenizer, an upper estimate of the prompt a
/// provider call is about to send: every system prompt part, every message and
/// the tool schemas. The result feeds
/// `cost_router::reservation::plan_reservation_bound_for_prompt`, which pads
/// it by `prompt_token_margin` before it bounds a paid hold.
///
/// Why: the reservation used to price the model's whole context window for
/// every call, so the harness's first call — ~30k tokens, 7% of a 400k window,
/// $0.12 actually spent — was refused with "task budget gate required before
/// provider call (spent $0.12, authorization holds $0.00, requested bound
/// $1.56, ceiling $1.00)".
///
/// Every choice here errs toward counting MORE than the provider will bill,
/// because an under-stated hold would authorize spend nobody approved:
/// - each message is counted as its full JSON serialization (ids, timestamps,
///   metadata, JSON escaping, and messages the provider later filters out as
///   not agent-visible), which over-covers the per-message role/turn tokens a
///   chat template adds;
/// - tool schemas are counted as their complete JSON, nested properties and
///   all, not the partial walk `TokenCounter::count_tokens_for_tools` does.
///
/// Returns `None` — and the caller then reserves the full window — whenever
/// the count cannot be trusted: the tokenizer is unavailable, a message will
/// not serialize, or the request carries an image, audio or binary resource,
/// whose billed tokens depend on pixel dimensions or duration rather than on
/// anything a text tokenizer can see.
pub(crate) async fn counted_prompt_tokens(
    system_parts: &[&str],
    messages: &[Message],
    tools: &[Tool],
) -> Option<usize> {
    let counter = crate::token_counter::create_token_counter().await.ok()?;
    let mut pieces: Vec<String> = Vec::with_capacity(system_parts.len() + messages.len() + 1);
    pieces.extend(system_parts.iter().map(|part| (*part).to_string()));
    for message in messages {
        let value = serde_json::to_value(message).ok()?;
        if carries_binary_media(&value) {
            return None;
        }
        pieces.push(value.to_string());
    }
    if !tools.is_empty() {
        pieces.push(serde_json::to_string(tools).ok()?);
    }
    // A long session can be megabytes of text; tokenizing it is CPU work that
    // must not stall the async runtime the reply stream is running on.
    tokio::task::spawn_blocking(move || {
        pieces
            .iter()
            .map(|piece| counter.count_tokens(piece))
            .fold(0usize, usize::saturating_add)
    })
    .await
    .ok()
}

/// Whether a serialized message carries content a text tokenizer cannot
/// price: an image or audio block (as `MessageContent::Image` or inside a tool
/// result) or an embedded binary resource (`blob`).
fn carries_binary_media(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            let media_type = map
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| matches!(kind, "image" | "audio"));
            media_type || map.contains_key("blob") || map.values().any(carries_binary_media)
        }
        Value::Array(items) => items.iter().any(carries_binary_media),
        _ => false,
    }
}

/// Reserve exactly one provider invocation using immutable transport/model
/// facts captured before dispatch.  This is shared by streamed turns and the
/// explicit physical attempts behind `complete_fast`; neither path may infer
/// a free tier from a provider name or fabricate a task identity.
///
/// `counted_prompt_tokens` is [`counted_prompt_tokens`]' count of the request
/// being authorized, or `None` when the caller cannot see it; `None` reserves
/// the whole context window, so an uncounted call fails closed onto the worst
/// case rather than onto a guess.
///
/// `provenance` (v60, node A4) is the caller's own account of why this call is
/// being made, captured before dispatch and carried unchanged on the returned
/// context to the ledger row. Callers that do not know pass
/// `CallProvenance::default()` (all unknown).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn reserve_provider_invocation_for_model(
    manager: Arc<SessionManager>,
    session: &Session,
    provider: &str,
    cost_tier: CostTier,
    model_config: crate::model::ModelConfig,
    max_physical_attempts: u32,
    invocation_id: String,
    counted_prompt_tokens: Option<usize>,
    provenance: CallProvenance,
) -> std::result::Result<ProviderInvocationContext, ProviderAuthorizationFailure> {
    // Y7 (harness loop, from Lane A12's Q6 measurement): reserve for ONE
    // physical attempt by default instead of the provider's whole retry
    // envelope. `max_physical_attempts` stays the envelope — it is the
    // ceiling this knob clamps to, and the number the retry policy will
    // actually honour if the reservation is extended to cover it. See
    // `cost_router::reservation::RESERVE_ATTEMPTS_ENV` for the measured
    // reason (holds of $1.66 against settled calls of $0.005).
    let attempts_reserved = crate::cost_router::reserved_physical_attempts(max_physical_attempts);
    // A declarative local runtime is built on a shared engine (the
    // `qwen38_split` two-machine split speaks OpenAI over a socket, so it IS
    // an `OpenAiProvider`) and therefore inherits `Provider::cost_tier`'s
    // fail-closed `PaidApi` default. Resolving the effective tier here — once,
    // from the curated knowledge row for this exact provider/model pair — is
    // what stops a free fallback being refused as an unauthorized paid call
    // (see `cost_router::reservation::is_free_local_pair` for the 2026-09-18
    // failure). It is recorded on the context as well as used for the bound,
    // because settlement below refuses a chargeable invocation carrying no
    // reservation id; authorization and settlement must agree on one answer.
    let cost_tier =
        crate::cost_router::effective_cost_tier(provider, &model_config.model_name, cost_tier);
    let bound = crate::cost_router::reservation::plan_reservation_bound_for_prompt(
        provider,
        &model_config.model_name,
        cost_tier,
        &model_config,
        attempts_reserved,
        counted_prompt_tokens,
    )
    .map_err(|e| ProviderAuthorizationFailure::Unknown {
        reason: format!("cannot authorize provider call: {e}"),
    })?;

    let mut context = ProviderInvocationContext {
        invocation_id,
        provider: provider.to_string(),
        model: model_config.model_name,
        cost_tier,
        reservation_id: None,
        // Only a paid hold constrains how many physical attempts may be made:
        // `plan_reservation_bound` returns `None` for local/subscription work,
        // which holds no dollars and therefore keeps the retry policy's own
        // budget (see `cost_router::authorized_attempts`).
        attempts_reserved: bound.as_ref().map(|bound| bound.max_physical_attempts),
        // The exact output cap the hold below (if any) is about to be priced
        // against, so the dispatch seam can enforce the same number on the
        // request instead of merely pricing it (see the field's own doc
        // comment).
        reserved_output_tokens: bound.as_ref().map(|bound| bound.output_tokens),
        provenance,
    };

    // Local and subscription calls retain typed attribution, but never create
    // a paid-dollar hold.
    let Some(bound) = bound else {
        // Say so. QA looked for "reservation bound computed" after the
        // prompt-sized hold shipped (2026-09-19) and found none since the
        // redeploy: the one run after it had failed over to
        // `ollama`/`qwen2.5:7b` before its first turn, and a free call took
        // this exit without a word, so "no paid hold was needed" and "the new
        // reservation code never ran" looked the same in the log. This line
        // tells them apart, and shows the prompt count the seam received,
        // without anyone having to pay for a call to find out.
        tracing::info!(
            target: "permagent::cost_router::reservation",
            provider = %context.provider,
            model = %context.model,
            cost_tier = ?context.cost_tier,
            counted_prompt_tokens = ?counted_prompt_tokens,
            "reservation skipped: unpaid tier holds no dollars"
        );
        return Ok(context);
    };
    // F2 (harness loop, Round F, from the I1/M11 forensics chain): F8, D1
    // and I1 each spent real time and money trying to explain a flat
    // $1.662976 bound on every zai/glm-4.7 reservation, because nothing
    // recorded which pricing source produced it — `cost_reservations` has no
    // provider/model/source columns, so a lane could only re-derive the
    // arithmetic by hand from raw DB rows (see this lane's own report,
    // `HARNESS_LOOP_LOG.md` `## Lane F2`: it was `CanonicalExact` pricing ×
    // 5 physical attempts the whole time, never `WorstCase`). This line is
    // the fix for THAT gap: every reservation now names its own live
    // provider/model, canonical mapping, dollar bound and pricing source at
    // the moment it is computed, so the next lane can `grep` the log instead
    // of reconstructing it from a database.
    tracing::info!(
        target: "permagent::cost_router::reservation",
        provider = %context.provider,
        model = %context.model,
        canonical = %canonical_name(&context.provider, &context.model),
        bound_usd = bound.amount_usd,
        pricing_source = crate::cost_router::describe_pricing_source(bound.pricing_provenance),
        // Y7: `attempts_reserved` is what this hold actually pays for (1 by
        // default); `retry_envelope_attempts` is what the provider's retry
        // policy would spend if every attempt were authorized. They were the
        // same number — and the reason a single call held $1.66 — until Y7.
        attempts_reserved = bound.max_physical_attempts,
        retry_envelope_attempts = max_physical_attempts,
        // Whether the hold was sized to the counted prompt or to the whole
        // window (`counted_prompt_tokens` absent), and the input it covers.
        counted_prompt_tokens = ?counted_prompt_tokens,
        reserved_input_tokens = bound.input_tokens,
        "reservation bound computed"
    );
    let task_id = budget_task_id(&session.extension_data);
    let lease_until = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
    let outcome = manager
        .reserve_bounded_provider_invocation(
            &context.invocation_id,
            &session.id,
            task_id.as_deref(),
            bound,
            &lease_until,
            &crate::cost_router::budget::load_budget_config(),
        )
        .await
        .map_err(|e| ProviderAuthorizationFailure::Unknown {
            reason: format!("cannot authorize provider call: {e}"),
        })?;

    match outcome {
        crate::session::CostReservationOutcome::Granted { reservation_id }
        | crate::session::CostReservationOutcome::AlreadyReserved { reservation_id } => {
            context.reservation_id = Some(reservation_id);
            Ok(context)
        }
        crate::session::CostReservationOutcome::AlreadySettled { .. } => {
            Err(ProviderAuthorizationFailure::Unknown {
                reason: "provider invocation identity was already settled; refusing replay"
                    .to_string(),
            })
        }
        crate::session::CostReservationOutcome::NeedsGate {
            scope,
            spent_usd,
            held_usd,
            requested_usd,
            ceiling_usd,
        } => Err(ProviderAuthorizationFailure::NeedsGate {
            scope,
            spent_usd,
            held_usd,
            requested_usd,
            ceiling_usd,
        }),
        crate::session::CostReservationOutcome::Refused {
            scope,
            spent_usd,
            held_usd,
            requested_usd,
            ceiling_usd,
        } => Err(ProviderAuthorizationFailure::Refused {
            scope,
            spent_usd,
            held_usd,
            requested_usd,
            ceiling_usd,
        }),
        crate::session::CostReservationOutcome::Unknown { reason } => {
            Err(ProviderAuthorizationFailure::Unknown { reason })
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderInvocationContext {
    pub invocation_id: String,
    pub provider: String,
    pub model: String,
    pub cost_tier: CostTier,
    pub reservation_id: Option<String>,
    /// Y7: physical provider attempts this invocation's hold pays for, or
    /// `None` when the call took no paid hold. The dispatch seam installs it
    /// with an extension authorizer so retries beyond this count require a
    /// new one-attempt reservation grant.
    pub attempts_reserved: Option<u32>,
    /// The output-token cap this invocation's hold was priced against
    /// ([`ReservationBound::output_tokens`](crate::cost_router::reservation::ReservationBound::output_tokens),
    /// `cost_router::reservation::output_reservation_cap` — larger for a
    /// reasoning model — clamped to the model's own `max_tokens`), or `None`
    /// for unpaid work that took no
    /// hold at all. `stream_response_from_provider` applies this to the
    /// request's own model config before dispatch, so the provider is asked
    /// to respect the SAME number the reservation priced — a reservation
    /// that prices one number while the request permits a bigger one is a
    /// lie (see that constant's doc comment for the measured evidence this
    /// closes: 63 reservations held $81.35 and settled at $1.23).
    pub reserved_output_tokens: Option<usize>,
    /// v60 (node A4): why this call was made, as the dispatching caller knew
    /// it before dispatch. Copied verbatim onto the `cost_ledger` row.
    pub provenance: CallProvenance,
}

/// v60 (node A4): the provenance the streamed reply loop can honestly claim
/// for one provider call, from facts it holds before dispatch.
///
/// - `retry_of_failed_call` is `Some` when this loop iteration re-runs the
///   turn because the previous iteration's invocation failed (provider
///   failover or recovery compaction): the call is a `retry` of that id.
/// - Otherwise, when the newest message is a user-role message carrying tool
///   results, the call continues after those results: `tool_continuation`,
///   triggered by the last tool result in it.
/// - Otherwise, on the first iteration of a reply whose newest message is the
///   user's own message: `user_response`, triggered by that message's id when
///   it has one.
/// - Anything else (a later iteration answering a harness-injected nudge or a
///   parse-recovery note, an assistant-last history, an empty conversation)
///   stays `unknown`. The trigger is still recorded where one is identifiable.
pub(crate) fn turn_call_provenance(
    messages: &[Message],
    turns_taken: u32,
    retry_of_failed_call: Option<String>,
) -> CallProvenance {
    use crate::conversation::message::MessageContent;

    let newest_user = messages
        .last()
        .filter(|message| message.role == rmcp::model::Role::User);
    let last_tool_result = newest_user.and_then(|message| {
        message
            .content
            .iter()
            .rev()
            .find_map(|content| match content {
                MessageContent::ToolResponse(response) => Some(response.id.clone()),
                _ => None,
            })
    });
    let trigger = match (&last_tool_result, newest_user) {
        (Some(tool_call_id), _) => Some(format!("tool_result:{tool_call_id}")),
        (None, Some(message)) => message.id.as_ref().map(|id| format!("message:{id}")),
        (None, None) => None,
    };

    let mut provenance = match retry_of_failed_call {
        Some(failed_call_id) => CallProvenance::retry_of(failed_call_id),
        None if last_tool_result.is_some() => {
            CallProvenance::with_purpose(CallPurpose::ToolContinuation)
        }
        None if newest_user.is_some() && turns_taken == 1 => {
            CallProvenance::with_purpose(CallPurpose::UserResponse)
        }
        None => CallProvenance::default(),
    };
    if let Some(trigger) = trigger {
        provenance = provenance.with_trigger(trigger);
    }
    provenance
}

/// One reserved `complete_fast` attempt. It owns an immutable provider and
/// session snapshot so background context work cannot accidentally inherit a
/// model switch or a new task on the next turn.
#[derive(Clone)]
pub(crate) struct AccountedFastCompletion {
    manager: Arc<SessionManager>,
    session: Session,
    provider: Arc<dyn Provider>,
    is_compaction_usage: bool,
    /// v60 (node A4): the purpose every call through this handle is made for,
    /// when the creator genuinely knows it; default (unknown) otherwise.
    provenance: CallProvenance,
}

impl Agent {
    /// The accounted completion handle for the Agent's own context upkeep.
    /// Every caller (reply-loop compaction, recovery compaction, `/compact`,
    /// and tool-pair summarisation) is context maintenance, so its ledger rows
    /// are labelled `context_maintenance` (v60, node A4). A caller making a
    /// call for any other reason must not use this handle.
    pub(crate) fn accounted_fast_completion(
        &self,
        session: &Session,
        provider: Arc<dyn Provider>,
        is_compaction_usage: bool,
    ) -> Arc<dyn crate::context_mgmt::AccountedFastCompletion> {
        Arc::new(AccountedFastCompletion {
            manager: Arc::clone(&self.config.session_manager),
            session: session.clone(),
            provider,
            is_compaction_usage,
            provenance: CallProvenance::with_purpose(CallPurpose::ContextMaintenance),
        })
    }
}

struct FastReservationUnknownGuard {
    manager: Arc<SessionManager>,
    reservation_id: Option<String>,
    armed: bool,
}

impl FastReservationUnknownGuard {
    fn new(manager: Arc<SessionManager>, reservation_id: Option<String>) -> Self {
        Self {
            manager,
            armed: reservation_id.is_some(),
            reservation_id,
        }
    }

    async fn mark_unknown(&mut self) -> Result<()> {
        if let Some(id) = self.reservation_id.as_deref().filter(|_| self.armed) {
            self.manager.mark_provider_invocation_unknown(id).await?;
        }
        self.armed = false;
        Ok(())
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for FastReservationUnknownGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Some(reservation_id) = self.reservation_id.clone() else {
            return;
        };
        let manager = Arc::clone(&self.manager);
        // Cancellation cannot await. Preserve the paid hold as unknown; the
        // durable lease is the crash-safe fallback if the runtime is exiting.
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Err(error) = manager
                    .mark_provider_invocation_unknown(&reservation_id)
                    .await
                {
                    tracing::warn!(
                        reservation_id = %reservation_id,
                        "could not mark cancelled complete_fast reservation unknown: {error}"
                    );
                }
            });
        }
    }
}

impl AccountedFastCompletion {
    /// Resolve the durable hidden session used by background model work. This
    /// keeps internal completions on the same Spectral ledger without inventing
    /// a transcript-side budget or process-local accounting store.
    pub(crate) async fn ensure_background_session(
        manager: Arc<SessionManager>,
        name: &str,
    ) -> Result<Session> {
        if let Some(session) = manager
            .list_sessions_by_types(&[SessionType::Hidden])
            .await?
            .into_iter()
            .find(|session| session.name == name)
        {
            return Ok(session);
        }
        manager
            .create_session(
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
                name.to_string(),
                SessionType::Hidden,
                GooseMode::Auto,
            )
            .await
    }

    /// Run a fast/full completion through the shared reservation and Spectral
    /// settlement seam without requiring an `Agent` instance. Background
    /// orchestrator work uses this entry point so it cannot bypass the same
    /// paid-dispatch boundary as interactive turns.
    pub(crate) async fn complete_fast_accounted(
        manager: Arc<SessionManager>,
        session: Session,
        provider: Arc<dyn Provider>,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
        is_compaction_usage: bool,
    ) -> Result<(Message, ProviderUsage)> {
        Self::complete_fast_accounted_with_purpose(
            manager,
            session,
            provider,
            system,
            messages,
            tools,
            is_compaction_usage,
            CallPurpose::Unknown,
        )
        .await
    }

    /// Same as [`Self::complete_fast_accounted`] but tags the ledger row with
    /// a genuine [`CallPurpose`] instead of the honest-default `Unknown`.
    /// 2026-09-25 DAG K cost fix (HARNESS_COST_TRACE.md Q1): added so
    /// internal callers that DO know why they are calling — the planner, the
    /// reviewer, the summarizer — can say so, instead of every one of them
    /// falling through `CallProvenance::default()` and showing up as
    /// `unknown` in the cost ledger.
    // The existing seven accounting inputs plus the purpose tag; bundling them
    // into a struct is a wider refactor of every accounted caller.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn complete_fast_accounted_with_purpose(
        manager: Arc<SessionManager>,
        session: Session,
        provider: Arc<dyn Provider>,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
        is_compaction_usage: bool,
        purpose: CallPurpose,
    ) -> Result<(Message, ProviderUsage)> {
        let completion = Self {
            manager,
            session,
            provider,
            is_compaction_usage,
            provenance: CallProvenance::with_purpose(purpose),
        };
        completion
            .complete_fast_inner(system, messages, tools)
            .await
    }

    /// Run one full-model completion through the same reservation and
    /// Spectral settlement boundary. Background callers that intentionally
    /// select the provider's actor/lead model use this instead of bypassing
    /// accounting with `Provider::complete`.
    pub(crate) async fn complete_accounted(
        manager: Arc<SessionManager>,
        session: Session,
        provider: Arc<dyn Provider>,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
        is_compaction_usage: bool,
    ) -> Result<(Message, ProviderUsage)> {
        Self::complete_accounted_with_purpose(
            manager,
            session,
            provider,
            system,
            messages,
            tools,
            is_compaction_usage,
            CallPurpose::Unknown,
        )
        .await
    }

    /// Same as [`Self::complete_accounted`] but tags the ledger row with a
    /// genuine [`CallPurpose`] instead of the honest-default `Unknown`. See
    /// [`Self::complete_fast_accounted_with_purpose`]'s doc comment for why
    /// this exists.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn complete_accounted_with_purpose(
        manager: Arc<SessionManager>,
        session: Session,
        provider: Arc<dyn Provider>,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
        is_compaction_usage: bool,
        purpose: CallPurpose,
    ) -> Result<(Message, ProviderUsage)> {
        let model_config = provider.get_model_config();
        let completion = Self {
            manager,
            session,
            provider,
            is_compaction_usage,
            provenance: CallProvenance::with_purpose(purpose),
        };
        completion
            .complete_one(
                model_config,
                system,
                messages,
                tools,
                uuid::Uuid::new_v4().to_string(),
                completion.provenance.clone(),
            )
            .await
    }

    /// `invocation_id` is minted by the caller so a fallback after a failed
    /// attempt can name it in `retry_of_call_id`.
    async fn complete_one(
        &self,
        model_config: crate::model::ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
        invocation_id: String,
        provenance: CallProvenance,
    ) -> Result<(Message, ProviderUsage)> {
        // The exact request is in hand here, so the hold is sized to it rather
        // than to the model's whole context window.
        let counted = counted_prompt_tokens(&[system], messages, tools).await;
        let invocation = reserve_provider_invocation_for_model(
            Arc::clone(&self.manager),
            &self.session,
            self.provider.get_name(),
            self.provider.cost_tier(),
            model_config.clone(),
            self.provider.retry_config().max_physical_attempts(),
            invocation_id,
            counted,
            provenance,
        )
        .await
        .map_err(|failure| anyhow::anyhow!(failure.to_string()))?;
        let mut unknown_guard = FastReservationUnknownGuard::new(
            Arc::clone(&self.manager),
            invocation.reservation_id.clone(),
        );

        // Physical retries beyond the hold's count require a one-attempt
        // extension grant. Unpaid work keeps the retry policy unchanged.
        let (response, mut usage) =
            match crate::cost_router::authorized_attempts::with_reservation_attempts(
                Arc::clone(&self.manager),
                invocation.invocation_id.clone(),
                invocation.attempts_reserved,
                self.provider
                    // permagent-dispatch: seam=complete_fast_attempt_v1 class=wrapped contract=reservation_settlement
                    .complete(&model_config, &self.session.id, system, messages, tools),
            )
            .await
            {
                Ok(result) => result,
                Err(error) => {
                    unknown_guard.mark_unknown().await?;
                    return Err(anyhow::anyhow!(
                        "complete_fast physical attempt failed: {error}"
                    ));
                }
            };

        let has_authoritative_usage = usage.usage.input_tokens.is_some()
            || usage.usage.output_tokens.is_some()
            || usage.usage.total_tokens.is_some()
            || usage.usage.cache_read_input_tokens.is_some()
            || usage.usage.cache_write_input_tokens.is_some();
        if invocation.cost_tier.is_chargeable() && !has_authoritative_usage {
            unknown_guard.mark_unknown().await?;
            return Err(anyhow::anyhow!(
                "complete_fast physical attempt returned no authoritative provider usage"
            ));
        }
        // Free/quota work has no paid hold. Preserve its existing token-meter
        // behavior even when a local adapter omitted a usage frame.
        if !has_authoritative_usage {
            usage
                .ensure_tokens(system, messages, &response, tools)
                .await
                .map_err(|error| anyhow::anyhow!("could not estimate non-paid usage: {error}"))?;
        }
        usage = usage.with_invocation_id(invocation.invocation_id.clone());
        if let Err(error) = record_provider_usage(
            Arc::clone(&self.manager),
            &self.session.id,
            self.session.schedule_id.clone(),
            &usage,
            self.is_compaction_usage,
            Some(&invocation),
            None,
            // A complete (non-streamed) response: its shape is fully observed.
            CallOutcome::from_message(&response),
        )
        .await
        {
            unknown_guard.mark_unknown().await?;
            return Err(anyhow::anyhow!(
                "complete_fast usage settlement failed: {error}"
            ));
        }
        unknown_guard.disarm();
        Ok((response, usage))
    }

    async fn complete_fast_inner(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage)> {
        let regular = self.provider.get_model_config();
        let fast = regular.use_fast_model();
        let paid = self.provider.cost_tier().is_chargeable();
        let first_invocation_id = uuid::Uuid::new_v4().to_string();
        match self
            .complete_one(
                fast.clone(),
                system,
                messages,
                tools,
                first_invocation_id.clone(),
                self.provenance.clone(),
            )
            .await
        {
            Ok(result) => Ok(result),
            // A paid fast failure may have reached the remote provider. Its
            // reservation is now unknown, so a regular fallback would be an
            // unauthorized second dispatch. The user can resolve the existing
            // gate/reconciliation before retrying.
            Err(error) if paid || fast.model_name == regular.model_name => Err(error),
            // Local/subscription calls retain the provider's normal fast→full
            // fallback semantics, with a distinct invocation/ledger row. That
            // row is a re-dispatch after a failed call, so it is labelled a
            // retry of the fast attempt (v60, node A4), keeping the original
            // trigger reference.
            Err(_) => {
                let mut retry = CallProvenance::retry_of(first_invocation_id);
                retry.trigger_ref = self.provenance.trigger_ref.clone();
                self.complete_one(
                    regular,
                    system,
                    messages,
                    tools,
                    uuid::Uuid::new_v4().to_string(),
                    retry,
                )
                .await
            }
        }
    }
}

#[async_trait::async_trait]
impl crate::context_mgmt::AccountedFastCompletion for AccountedFastCompletion {
    async fn complete_fast(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage)> {
        self.complete_fast_inner(system, messages, tools).await
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ProviderAuthorizationFailure {
    NeedsGate {
        scope: crate::cost_router::budget::BudgetScope,
        spent_usd: f64,
        held_usd: f64,
        requested_usd: f64,
        ceiling_usd: f64,
    },
    Refused {
        scope: crate::cost_router::budget::BudgetScope,
        spent_usd: f64,
        held_usd: f64,
        requested_usd: f64,
        ceiling_usd: f64,
    },
    Unknown {
        reason: String,
    },
}

impl std::fmt::Display for ProviderAuthorizationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NeedsGate {
                scope,
                spent_usd,
                held_usd,
                requested_usd,
                ceiling_usd,
            } => write!(
                f,
                "{} budget gate required before provider call (spent ${spent_usd:.2}, authorization holds ${held_usd:.2}, requested bound ${requested_usd:.2}, ceiling ${ceiling_usd:.2})",
                scope.word()
            ),
            Self::Refused {
                scope,
                spent_usd,
                held_usd,
                requested_usd,
                ceiling_usd,
            } => write!(
                f,
                "{} budget refused provider call (spent ${spent_usd:.2}, authorization holds ${held_usd:.2}, requested bound ${requested_usd:.2}, ceiling ${ceiling_usd:.2})",
                scope.word()
            ),
            Self::Unknown { reason } => {
                write!(f, "provider budget is unknown; refusing call: {reason}")
            }
        }
    }
}

fn is_local_provider(provider: &str) -> bool {
    let provider = provider.to_ascii_lowercase();
    provider == "local"
        || provider.contains("ollama")
        || provider == "lmstudio"
        || provider == "llama_swap"
        || provider == "qwen38_split"
}

/// Check whether a tool should be callable by an app based on MCP Apps visibility metadata.
///
/// Per the MCP Apps spec (2026-01-26), if `_meta.ui.visibility` is present and does not
/// include `"app"`, the tool is model-only and must not be callable by app UIs.
/// If the field is absent, the tool defaults to visible to both model and app.
pub fn is_tool_visible_to_app(tool: &Tool) -> bool {
    let Some(meta) = &tool.meta else {
        return true;
    };
    let Some(ui) = meta.0.get("ui") else {
        return true;
    };
    let Some(visibility) = ui.get("visibility") else {
        return true;
    };
    let Some(arr) = visibility.as_array() else {
        return true;
    };
    arr.iter().any(|v| v.as_str() == Some("app"))
}

/// Check whether a tool should be visible to the model based on MCP Apps visibility metadata.
///
/// Per the MCP Apps spec (2026-01-26), tools may declare `_meta.ui.visibility` as an array
/// of `"model"` and/or `"app"`. If the field is absent, the tool defaults to visible to both.
/// If present and does not include `"model"`, the tool is app-only and must not be sent to the LLM.
pub fn is_tool_visible_to_model(tool: &Tool) -> bool {
    let Some(meta) = &tool.meta else {
        return true;
    };
    let Some(ui) = meta.0.get("ui") else {
        return true;
    };
    let Some(visibility) = ui.get("visibility") else {
        return true;
    };
    let Some(arr) = visibility.as_array() else {
        return true;
    };
    arr.iter().any(|v| v.as_str() == Some("model"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GooseMode;
    use crate::conversation::message::Message;
    use crate::model::ModelConfig;
    use crate::providers::base::{Provider, ProviderUsage, Usage};
    use crate::providers::errors::ProviderError;
    use crate::session::session_manager::SessionType;
    use async_trait::async_trait;
    use rmcp::object;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;
    use tempfile::TempDir;

    // ── R18a-b (G21 HIGH-1): the approval_bypasses producer ──────────────

    /// `record_provider_call_authorized` is the pure function both of
    /// `agents::agent`'s turn-loop call sites use — tested directly because
    /// the turn loop itself (`Agent::reply`'s big streaming state machine)
    /// has no lightweight harness to drive end to end. `#[serial]` because
    /// it drives the process-wide `REGISTRY`/`ACTIVE` statics, same shared
    /// key as `receipts.rs`'s own global-API tests.
    #[test]
    #[serial_test::serial(receipts_global_registry)]
    fn record_provider_call_authorized_increments_only_when_not_authorized() {
        crate::receipts::begin_run();

        // Present grant (`authorized = true`, both call sites' shape today):
        // must NOT increment.
        record_provider_call_authorized(true);
        let line = crate::receipts::snapshot().lines()[2].clone();
        assert!(
            line.contains("approval_bypasses=0"),
            "an authorized call must not count as a bypass: {line}"
        );

        // Absent grant: a future call site that proceeds without one must
        // increment.
        record_provider_call_authorized(false);
        let line = crate::receipts::snapshot().lines()[2].clone();
        crate::receipts::end_run();
        assert!(
            line.contains("approval_bypasses=1"),
            "a call that proceeds without authorization must count as one bypass: {line}"
        );
    }

    #[derive(Clone)]
    struct MockProvider {
        model_config: ModelConfig,
    }

    #[async_trait]
    impl Provider for MockProvider {
        fn get_name(&self) -> &str {
            "mock"
        }

        fn get_model_config(&self) -> ModelConfig {
            self.model_config.clone()
        }

        async fn stream(
            &self,
            _model_config: &ModelConfig,
            _session_id: &str,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<MessageStream, ProviderError> {
            let message = Message::assistant().with_text("ok");
            let usage = ProviderUsage::new("mock".to_string(), Usage::default());
            Ok(stream_from_single_message(message, usage))
        }
    }

    /// Records the `ModelConfig` it was actually dispatched with, so a test
    /// can assert on the exact request-level output cap the provider was
    /// handed — not merely on the reservation's own bookkeeping.
    #[derive(Clone)]
    struct CapturingModelConfigProvider {
        model_config: ModelConfig,
        seen: Arc<Mutex<Option<ModelConfig>>>,
    }

    #[async_trait]
    impl Provider for CapturingModelConfigProvider {
        fn get_name(&self) -> &str {
            "mock"
        }

        fn get_model_config(&self) -> ModelConfig {
            self.model_config.clone()
        }

        async fn stream(
            &self,
            model_config: &ModelConfig,
            _session_id: &str,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<MessageStream, ProviderError> {
            *self.seen.lock().unwrap() = Some(model_config.clone());
            let message = Message::assistant().with_text("ok");
            let usage = ProviderUsage::new("mock".to_string(), Usage::default());
            Ok(stream_from_single_message(message, usage))
        }
    }

    /// The seam this whole fix runs through
    /// (`Agent::stream_response_from_provider`): the reservation's
    /// `reserved_output_tokens` must be the SAME number that ends up on the
    /// request-level `ModelConfig` handed to the provider's `stream`/
    /// `stream_split` call, not merely a smaller number priced in the
    /// dollar hold while the wire request still carries the model's raw,
    /// much larger `max_tokens`. A structured assertion on the config the
    /// mock provider actually received, not a comment.
    #[tokio::test]
    async fn stream_response_from_provider_sends_the_reserved_cap_to_the_provider() {
        let seen = Arc::new(Mutex::new(None));
        let provider: Arc<dyn Provider> = Arc::new(CapturingModelConfigProvider {
            model_config: ModelConfig {
                model_name: "glm-5.2".to_string(),
                context_limit: Some(1_000_000),
                // The model's own raw ceiling is far above the reserved cap;
                // the request must carry the cap, not this number.
                max_tokens: Some(128_000),
                ..ModelConfig::default()
            },
            seen: Arc::clone(&seen),
        });
        let system_prompt = SystemPromptParts::all_stable(String::new());

        let mut stream = Agent::stream_response_from_provider(
            provider,
            "session-id",
            &system_prompt,
            &[],
            &[],
            &[],
            Some(4_096),
        )
        .await
        .unwrap();
        while stream.next().await.is_some() {}

        let captured = seen
            .lock()
            .unwrap()
            .clone()
            .expect("provider was dispatched");
        assert_eq!(
            captured.max_tokens,
            Some(4_096),
            "the request must carry the reserved cap, not the model's raw max_tokens"
        );
        assert_ne!(captured.max_tokens, Some(128_000));

        // The same value becomes the wire-level `max_output_tokens` the
        // Responses API request actually serializes — the seam named in the
        // task (`create_responses_request`, ~line 517).
        let payload = crate::providers::formats::openai_responses::create_responses_request(
            &captured,
            "",
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(payload["max_output_tokens"], serde_json::json!(4_096));
    }

    /// `None` (unpaid work — see `cost_router::reservation::is_free_local_pair`)
    /// must leave the model's own config untouched: an unpaid call is never
    /// capped by a paid-only mechanism.
    #[tokio::test]
    async fn stream_response_from_provider_leaves_unpaid_calls_uncapped() {
        let seen = Arc::new(Mutex::new(None));
        let provider: Arc<dyn Provider> = Arc::new(CapturingModelConfigProvider {
            model_config: ModelConfig {
                model_name: "local-model".to_string(),
                context_limit: Some(4_096),
                max_tokens: Some(128),
                ..ModelConfig::default()
            },
            seen: Arc::clone(&seen),
        });
        let system_prompt = SystemPromptParts::all_stable(String::new());

        let mut stream = Agent::stream_response_from_provider(
            provider,
            "session-id",
            &system_prompt,
            &[],
            &[],
            &[],
            None,
        )
        .await
        .unwrap();
        while stream.next().await.is_some() {}

        let captured = seen
            .lock()
            .unwrap()
            .clone()
            .expect("provider was dispatched");
        assert_eq!(captured.max_tokens, Some(128));
    }

    /// A mock whose provider name is settable, so a test can actually switch
    /// providers mid-session the way a failover does.
    #[derive(Clone)]
    struct NamedMockProvider {
        name: &'static str,
        model_config: ModelConfig,
    }

    #[derive(Clone)]
    struct FastAccountingProvider {
        usage: Usage,
        fail: bool,
        calls: Arc<AtomicUsize>,
        model_config: ModelConfig,
    }

    impl FastAccountingProvider {
        fn paid(usage: Usage) -> Self {
            Self {
                usage,
                fail: false,
                calls: Arc::new(AtomicUsize::new(0)),
                model_config: ModelConfig::new("claude-haiku-4-5")
                    .unwrap()
                    .with_canonical_limits("anthropic"),
            }
        }
    }

    #[async_trait]
    impl Provider for FastAccountingProvider {
        fn get_name(&self) -> &str {
            "anthropic"
        }

        fn get_model_config(&self) -> ModelConfig {
            self.model_config.clone()
        }

        async fn stream(
            &self,
            _model_config: &ModelConfig,
            _session_id: &str,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<MessageStream, ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(ProviderError::RequestFailed(
                    "fixture dispatch failed".to_string(),
                ));
            }
            Ok(stream_from_single_message(
                Message::assistant().with_text("summary"),
                ProviderUsage::new("claude-haiku-4-5".to_string(), self.usage),
            ))
        }
    }

    /// Records the model selected for every physical attempt. The first fast
    /// attempt fails; a non-chargeable provider is permitted to retry once
    /// with the regular model.
    #[derive(Clone)]
    struct LocalFastFallbackProvider {
        calls: Arc<AtomicUsize>,
        selected_models: Arc<Mutex<Vec<String>>>,
        model_config: ModelConfig,
    }

    impl LocalFastFallbackProvider {
        fn new() -> Self {
            Self {
                calls: Arc::new(AtomicUsize::new(0)),
                selected_models: Arc::new(Mutex::new(Vec::new())),
                model_config: ModelConfig {
                    model_name: "full-local-model".to_string(),
                    context_limit: Some(4_096),
                    max_tokens: Some(128),
                    fast_model_config: Some(Box::new(ModelConfig {
                        model_name: "fast-local-model".to_string(),
                        context_limit: Some(4_096),
                        max_tokens: Some(128),
                        ..ModelConfig::default()
                    })),
                    ..ModelConfig::default()
                },
            }
        }
    }

    #[async_trait]
    impl Provider for LocalFastFallbackProvider {
        fn get_name(&self) -> &str {
            "local-fixture"
        }

        fn cost_tier(&self) -> CostTier {
            CostTier::LocalFree
        }

        fn get_model_config(&self) -> ModelConfig {
            self.model_config.clone()
        }

        async fn stream(
            &self,
            model_config: &ModelConfig,
            _session_id: &str,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<MessageStream, ProviderError> {
            self.selected_models
                .lock()
                .unwrap()
                .push(model_config.model_name.clone());
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(ProviderError::RequestFailed(
                    "fast local fixture dispatch failed".to_string(),
                ));
            }
            Ok(stream_from_single_message(
                Message::assistant().with_text("regular summary"),
                ProviderUsage::new(
                    model_config.model_name.clone(),
                    Usage {
                        input_tokens: Some(8),
                        output_tokens: Some(3),
                        total_tokens: Some(11),
                        ..Usage::default()
                    },
                ),
            ))
        }
    }

    /// A paid attempt which has definitely crossed the dispatch boundary but
    /// never returns. Dropping its future must turn the durable hold unknown.
    #[derive(Clone)]
    struct PendingPaidProvider {
        calls: Arc<AtomicUsize>,
        model_config: ModelConfig,
    }

    #[async_trait]
    impl Provider for PendingPaidProvider {
        fn get_name(&self) -> &str {
            "anthropic"
        }

        fn get_model_config(&self) -> ModelConfig {
            self.model_config.clone()
        }

        async fn stream(
            &self,
            _model_config: &ModelConfig,
            _session_id: &str,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<MessageStream, ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            std::future::pending::<()>().await;
            unreachable!("the cancellation test must abort the pending dispatch")
        }
    }

    async fn fast_accounting_fixture(
        provider: Arc<dyn Provider>,
        begin_task: bool,
    ) -> (
        TempDir,
        Arc<SessionManager>,
        Session,
        AccountedFastCompletion,
    ) {
        let directory = TempDir::new().unwrap();
        let manager = Arc::new(SessionManager::new(directory.path().to_path_buf()));
        let session = manager
            .create_session(
                directory.path().to_path_buf(),
                "fast-accounting".to_string(),
                SessionType::Hidden,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        if begin_task {
            manager.begin_budget_task(&session.id).await.unwrap();
        }
        let session = manager.get_session(&session.id, false).await.unwrap();
        let completion = AccountedFastCompletion {
            manager: Arc::clone(&manager),
            session: session.clone(),
            provider,
            is_compaction_usage: false,
            provenance: CallProvenance::default(),
        };
        (directory, manager, session, completion)
    }

    #[tokio::test]
    async fn accounted_fast_success_settles_once_with_a_durable_task() {
        let fixture = FastAccountingProvider::paid(Usage {
            input_tokens: Some(12),
            output_tokens: Some(5),
            total_tokens: Some(17),
            ..Usage::default()
        });
        let calls = Arc::clone(&fixture.calls);
        let (_directory, manager, session, completion) =
            fast_accounting_fixture(Arc::new(fixture), true).await;

        crate::context_mgmt::AccountedFastCompletion::complete_fast(
            &completion,
            "summarize",
            &[Message::user().with_text("hello")],
            &[],
        )
        .await
        .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(manager
            .last_call_facts(&session.id)
            .await
            .unwrap()
            .is_some());
        let pool = manager.storage().pool().await.unwrap();
        let settled: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cost_reservations WHERE session_id = ? AND state = 'settled'",
        )
        .bind(&session.id)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(settled, 1);
    }

    /// 2026-09-25 DAG K cost fix (HARNESS_COST_TRACE.md Q1): a caller that
    /// genuinely knows why it is calling — the planner, via
    /// `complete_accounted_with_purpose` — must see that purpose land on the
    /// ledger row instead of falling through to `CallProvenance::default()`
    /// (`unknown`), which is what `complete_accounted`/`complete_fast_accounted`
    /// did unconditionally before this lane. This is the same plumbing
    /// `run_decomposition` (`orchestrator.rs`) uses.
    #[tokio::test]
    async fn complete_accounted_with_purpose_records_planning_on_the_ledger() {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage {
            input_tokens: Some(10),
            output_tokens: Some(4),
            total_tokens: Some(14),
            ..Usage::default()
        }));
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(fixture.clone(), true).await;

        AccountedFastCompletion::complete_accounted_with_purpose(
            Arc::clone(&manager),
            session.clone(),
            fixture,
            "plan",
            &[Message::user().with_text("decompose this")],
            &[],
            false,
            CallPurpose::Planning,
        )
        .await
        .unwrap();

        let pool = manager.storage().pool().await.unwrap();
        let purpose: Option<String> = sqlx::query_scalar(
            "SELECT call_purpose FROM cost_ledger WHERE session_id = ? AND model = 'claude-haiku-4-5'",
        )
        .bind(&session.id)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(purpose.as_deref(), Some("planning"));
    }

    #[tokio::test]
    async fn primary_stream_reservation_refuses_paid_dispatch_without_a_task() {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage::default()));
        let calls = Arc::clone(&fixture.calls);
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(fixture.clone(), false).await;

        let result = reserve_provider_invocation_for_model(
            Arc::clone(&manager),
            &session,
            fixture.get_name(),
            fixture.cost_tier(),
            fixture.get_model_config(),
            fixture.retry_config().max_physical_attempts(),
            uuid::Uuid::new_v4().to_string(),
            None,
            CallProvenance::default(),
        )
        .await;

        assert!(matches!(
            result,
            Err(ProviderAuthorizationFailure::Unknown { .. })
        ));
        // Authorization happens before the stream transport is ever called.
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn primary_stream_settles_one_authoritative_snapshot_per_invocation() {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage::default()));
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(fixture.clone(), true).await;
        let invocation = reserve_provider_invocation_for_model(
            Arc::clone(&manager),
            &session,
            fixture.get_name(),
            fixture.cost_tier(),
            fixture.get_model_config(),
            fixture.retry_config().max_physical_attempts(),
            uuid::Uuid::new_v4().to_string(),
            None,
            CallProvenance::default(),
        )
        .await
        .expect("durable task should authorize the paid stream");
        let usage = ProviderUsage::new(
            fixture.get_model_config().model_name,
            Usage {
                input_tokens: Some(12),
                output_tokens: Some(5),
                total_tokens: Some(17),
                ..Usage::default()
            },
        )
        .with_invocation_id(invocation.invocation_id.clone());

        let pool = manager.storage().pool().await.unwrap();
        sqlx::query(
            "INSERT INTO recognition_events
                (retrieval_id, session_id, query, retrieved_at, rc_persona, strategy)
             VALUES ('reply-attribution', ?, 'fixture', '2026-09-05T12:00:00Z', 'unknown', 'cascade')",
        )
        .bind(&session.id)
        .execute(pool)
        .await
        .unwrap();

        record_provider_usage(
            Arc::clone(&manager),
            &session.id,
            session.schedule_id.clone(),
            &usage,
            false,
            Some(&invocation),
            Some("reply-attribution"),
            CallOutcome::default(),
        )
        .await
        .expect("authoritative terminal usage should settle the hold");

        let pool = manager.storage().pool().await.unwrap();
        let ledger_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cost_ledger WHERE session_id = ? AND call_id = ?",
        )
        .bind(&session.id)
        .bind(&invocation.invocation_id)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(ledger_rows, 1);
        let settled: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cost_reservations WHERE session_id = ? AND reservation_id = ? AND state = 'settled'",
        )
        .bind(&session.id)
        .bind(invocation.reservation_id.as_deref().unwrap())
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(settled, 1);
        let (ids, status): (String, String) = sqlx::query_as(
            "SELECT provider_invocation_ids, attribution_status
               FROM recognition_events WHERE retrieval_id = 'reply-attribution'",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(ids, format!("[\"{}\"]", invocation.invocation_id));
        assert_eq!(status, "observed");
    }

    /// A goal card "dispatched" to `session_id`, mimicking real dispatch
    /// bookkeeping (`worker_session_id`/`worker_session_ids`) in the
    /// `in_progress` column, so a test can assert the card is untouched by
    /// [`try_auto_answer_reservation_gate`].
    async fn dispatched_goal_card(
        pool: &sqlx::Pool<sqlx::Sqlite>,
        session_id: &str,
    ) -> crate::cards::Card {
        crate::cards::seed_goal_columns(pool, crate::projects::PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        let in_progress = crate::cards::get_goal_column(
            pool,
            crate::projects::PERSONAL_PROJECT_ID,
            "in_progress",
        )
        .await
        .unwrap()
        .unwrap();
        crate::cards::create_card(
            pool,
            crate::cards::CreateCard {
                project_id: crate::projects::PERSONAL_PROJECT_ID.to_string(),
                title: "E1 pin test goal".to_string(),
                description: Some("test".to_string()),
                card_type: Some("goal".to_string()),
                column_id: Some(in_progress.id),
                created_by: None,
                metadata_json: Some(serde_json::json!({
                    "attempt_count": 0,
                    "goal_state": "in_progress",
                    "worker_session_id": session_id,
                    "worker_session_ids": [session_id],
                })),
            },
        )
        .await
        .unwrap()
    }

    /// E1 pin (a): a headless auto-answered reservation gate must let the
    /// SAME session retry in place — the goal keeps its current dispatched
    /// session unchanged (same `worker_session_id`, `worker_session_ids`
    /// still a single entry, still `in_progress`) and no `requeue_goal` call
    /// ever fires. Pre-fix, this exact code path called
    /// `goal_transition::requeue_goal`, which would have bounced the card to
    /// `ready` and left `worker_session_id` stale — this test would fail on
    /// the `state_binding`/`worker_session_id` assertions below against that
    /// old behavior.
    #[tokio::test]
    async fn gate_auto_answer_retries_in_place_without_requeuing_the_goal() {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage::default()));
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(fixture.clone(), true).await;
        let pool = manager.storage().pool().await.unwrap().clone();
        let card = dispatched_goal_card(&pool, &session.id).await;

        let failure = ProviderAuthorizationFailure::NeedsGate {
            scope: crate::cost_router::budget::BudgetScope::Task,
            spent_usd: 5.0,
            held_usd: 0.0,
            requested_usd: 0.20,
            ceiling_usd: 5.0,
        };
        let request = crate::cost_router::budget::reservation_gate_decision_request(
            crate::cost_router::budget::BudgetScope::Task,
            5.0,
            0.0,
            0.20,
            5.0,
            5.0,
            Some(card.id.clone()),
            None,
        );
        let decision = crate::decisions::create_decision(&pool, request)
            .await
            .unwrap();

        let invocation_id = uuid::Uuid::new_v4().to_string();
        let outcome = try_auto_answer_goal_scoped_reservation_gate(
            &pool,
            &card.id,
            &session,
            &invocation_id,
            &failure,
            &decision.id,
            true, // headless
            &manager,
            "budget gate detail",
            "zai",
            "glm-4.7",
        )
        .await;
        assert_eq!(
            outcome,
            Some(ProviderAuthorizationOutcome {
                detail: "budget gate detail".to_string(),
                // Y4a: the same call must also report that it left a spendable
                // grant behind, so the turn loop retries in place instead of
                // ending the turn on top of a paid-for authorization.
                gate_grant_minted: true,
            }),
            "a headless auto-answer must report retry-in-place, not fall through to park"
        );

        // (a) goal stays assigned to the SAME session; no second session
        // dispatched; still in_progress (never bounced to ready).
        let after = crate::cards::get_card(&pool, &card.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            after
                .metadata_json
                .get("worker_session_id")
                .and_then(|v| v.as_str()),
            Some(session.id.as_str()),
            "the goal must stay assigned to the session whose call was just authorized"
        );
        assert_eq!(
            after
                .metadata_json
                .get("worker_session_ids")
                .and_then(|v| v.as_array())
                .map(|a| a.len()),
            Some(1),
            "no second session may ever be created by this path"
        );
        let column = crate::cards::get_column(&pool, &after.column_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            column.state_binding.as_deref(),
            Some("in_progress"),
            "a retry-in-place must never move the goal out of in_progress"
        );

        // (c) the retry does not re-ask the gate for the same reservation:
        // reusing `invocation_id` grants immediately (AlreadyReserved via
        // the early invocation-id lookup), with no second decision created.
        let decisions_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM decisions")
            .fetch_one(&pool)
            .await
            .unwrap();
        let retry = reserve_provider_invocation_for_model(
            Arc::clone(&manager),
            &session,
            fixture.get_name(),
            fixture.cost_tier(),
            fixture.get_model_config(),
            fixture.retry_config().max_physical_attempts(),
            invocation_id.clone(),
            None,
            CallProvenance::default(),
        )
        .await;
        assert!(
            retry.is_ok(),
            "the retry must reuse the already-granted reservation, not re-hit the gate: {retry:?}"
        );
        let decisions_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM decisions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            decisions_before, decisions_after,
            "the retry must not create a second gate decision for the same reservation"
        );
    }

    /// E1 pin: a NON-headless run (no `HeadlessDispatchPolicy` installed)
    /// must fall straight through to the caller's park path — this function
    /// returns `None` and touches nothing. Exercises the `headless` flag
    /// directly rather than the process-global headless dispatch policy
    /// slot (see this function's own doc comment for why).
    #[tokio::test]
    async fn gate_auto_answer_is_a_no_op_when_not_headless() {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage::default()));
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(fixture.clone(), true).await;
        let pool = manager.storage().pool().await.unwrap().clone();
        let card = dispatched_goal_card(&pool, &session.id).await;

        let failure = ProviderAuthorizationFailure::NeedsGate {
            scope: crate::cost_router::budget::BudgetScope::Task,
            spent_usd: 5.0,
            held_usd: 0.0,
            requested_usd: 0.20,
            ceiling_usd: 5.0,
        };
        let request = crate::cost_router::budget::reservation_gate_decision_request(
            crate::cost_router::budget::BudgetScope::Task,
            5.0,
            0.0,
            0.20,
            5.0,
            5.0,
            Some(card.id.clone()),
            None,
        );
        let decision = crate::decisions::create_decision(&pool, request)
            .await
            .unwrap();

        let invocation_id = uuid::Uuid::new_v4().to_string();
        let outcome = try_auto_answer_goal_scoped_reservation_gate(
            &pool,
            &card.id,
            &session,
            &invocation_id,
            &failure,
            &decision.id,
            false, // NOT headless
            &manager,
            "budget gate detail",
            "zai",
            "glm-4.7",
        )
        .await;
        assert_eq!(outcome, None);

        let reloaded = crate::decisions::get_decision(&pool, &decision.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            reloaded.status, "open",
            "a non-headless run must leave the gate open for a human"
        );
    }

    /// I1 pin (harness loop, 2026-09-10, from M10's forensics): a coordinator
    /// session — one with no `goal_id` at all, since `fast_accounting_
    /// fixture` never stamps one — that hits the reservation gate under a
    /// headless Auto run must get the SAME shape/budget-rule auto-answer a
    /// worker's gate gets, and continue in the SAME session (a reservation
    /// granted under `invocation_id`, no goal card touched — there is none).
    /// Pre-fix, `handle_provider_authorization_failure` returned bare
    /// `detail` immediately for any goal-less session — this pins the
    /// replacement path directly.
    #[tokio::test]
    async fn session_scoped_gate_auto_answer_lets_the_coordinator_continue_without_a_goal() {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage::default()));
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(fixture.clone(), true).await;
        assert_eq!(
            goal_id(&session.extension_data),
            None,
            "setup: a coordinator session carries no goal_id"
        );

        let failure = ProviderAuthorizationFailure::NeedsGate {
            scope: crate::cost_router::budget::BudgetScope::Task,
            spent_usd: 5.0,
            held_usd: 0.0,
            requested_usd: 0.20,
            ceiling_usd: 5.0,
        };
        let invocation_id = uuid::Uuid::new_v4().to_string();
        let outcome = session_scoped_provider_authorization_failure(
            &manager,
            &session,
            &invocation_id,
            &failure,
            "budget gate detail".to_string(),
            true, // headless
            "zai",
            "glm-4.7",
        )
        .await;
        assert_eq!(
            outcome.detail, "budget gate detail",
            "a headless auto-answer must report retry-in-place, not fall through unanswered"
        );
        // Y4a: the coordinator's own turn loop only retries when this is set.
        assert!(
            outcome.gate_grant_minted,
            "a coordinator whose gate was auto-granted must be told a spendable \
             reservation now exists, so its CURRENT turn can retry"
        );

        let pool = manager.storage().pool().await.unwrap().clone();
        let decision =
            crate::decisions::find_open_reservation_gate_decision_for_session(&pool, &session.id)
                .await
                .unwrap();
        // The decision was auto-answered (ACTOR_HEADLESS_POLICY), so it is no
        // longer OPEN — confirm it exists and was answered, not left open.
        assert!(
            decision.is_none(),
            "an auto-answered decision must no longer be open"
        );
        let answered: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM decisions WHERE goal_id IS NULL AND kind = 'choice' \
             AND status = 'answered' AND acted_by = ?",
        )
        .bind(crate::decisions::ACTOR_HEADLESS_POLICY)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(answered, 1);

        // The SAME session got its reservation granted under its own call's
        // invocation_id — a real reservation, not merely a decision.
        let state: String =
            sqlx::query_scalar("SELECT state FROM cost_reservations WHERE invocation_id = ?")
                .bind(&invocation_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(state, "pending");
    }

    /// Y4a pin (harness loop, 2026-09-10, from Lane A12's M12 forensics): a
    /// coordinator's auto-granted gate must be reported as `gate_grant_minted`
    /// AND that grant must actually authorize a retry under the SAME
    /// `invocation_id`, within the SAME turn.
    ///
    /// This is the session-scoped mirror of part (c) of
    /// `gate_auto_answer_retries_in_place_without_requeuing_the_goal`, and it
    /// is the coordinator's case that A12 traced: a worker gets a later turn
    /// in which `reusable_gate_grant_invocation_id` can pick the grant up, but
    /// a coordinator's turn ending IS the run ending, so a grant it never
    /// consumes in-turn is money paid for nothing (fx01's correct worker
    /// commit never surfaced; fx09 the same; fx07 truncated). Pre-Y4a
    /// `session_scoped_provider_authorization_failure` returned a bare
    /// `String`, so the turn loop could not distinguish this case from a park
    /// and always emitted "provider spend could not be authorized" —
    /// the `gate_grant_minted` assertion below would not compile against it.
    #[tokio::test]
    async fn session_scoped_gate_grant_authorizes_a_retry_under_the_same_invocation_id() {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage::default()));
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(fixture.clone(), true).await;
        let pool = manager.storage().pool().await.unwrap().clone();
        assert_eq!(
            goal_id(&session.extension_data),
            None,
            "setup: a coordinator session carries no goal_id"
        );

        let failure = ProviderAuthorizationFailure::NeedsGate {
            scope: crate::cost_router::budget::BudgetScope::Task,
            spent_usd: 5.0,
            held_usd: 0.0,
            requested_usd: 0.20,
            ceiling_usd: 5.0,
        };
        let invocation_id = uuid::Uuid::new_v4().to_string();
        let outcome = session_scoped_provider_authorization_failure(
            &manager,
            &session,
            &invocation_id,
            &failure,
            "budget gate detail".to_string(),
            true, // headless
            // F3: the grant is minted for THIS provider/model — the same
            // identity the retry below reserves under.
            fixture.get_name(),
            &fixture.get_model_config().model_name,
        )
        .await;
        assert!(
            outcome.gate_grant_minted,
            "the handler must report the grant it just minted, or the turn loop \
             has no way to know an in-turn retry is authorized"
        );

        // The retry the turn loop now performs: the SAME invocation_id, exactly
        // once. It must succeed off the idempotent `AlreadyReserved` path and
        // raise no second gate decision.
        let decisions_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM decisions")
            .fetch_one(&pool)
            .await
            .unwrap();
        let retry = reserve_provider_invocation_for_model(
            Arc::clone(&manager),
            &session,
            fixture.get_name(),
            fixture.cost_tier(),
            fixture.get_model_config(),
            fixture.retry_config().max_physical_attempts(),
            invocation_id.clone(),
            None,
            CallProvenance::default(),
        )
        .await;
        assert!(
            retry.is_ok(),
            "the single retry must reuse the granted reservation, not re-hit the gate: {retry:?}"
        );
        assert_eq!(
            retry.unwrap().reservation_id.as_deref(),
            Some(format!("invocation:{invocation_id}").as_str()),
            "the retry must land on the very reservation the gate paid for"
        );
        let decisions_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM decisions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            decisions_before, decisions_after,
            "the retry must not create a second gate decision for the same reservation"
        );
    }

    /// I1 pin: a coordinator that keeps hitting the same gate on successive
    /// calls must not file a second Decision Inbox card — the session-scoped
    /// dedupe (`find_open_reservation_gate_decision_for_session`) must find
    /// and reuse the first still-open one.
    #[tokio::test]
    async fn session_scoped_gate_auto_answer_does_not_duplicate_the_decision_on_a_retry() {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage::default()));
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(fixture.clone(), true).await;
        let pool = manager.storage().pool().await.unwrap().clone();

        let failure = ProviderAuthorizationFailure::NeedsGate {
            scope: crate::cost_router::budget::BudgetScope::Task,
            spent_usd: 5.0,
            held_usd: 0.0,
            requested_usd: 0.20,
            ceiling_usd: 5.0,
        };

        // First call: not headless yet, so the decision is created and left
        // open (mirrors the non-headless pin above) rather than immediately
        // auto-answered — isolates the dedupe assertion from the auto-answer
        // path itself.
        let first = session_scoped_provider_authorization_failure(
            &manager,
            &session,
            &uuid::Uuid::new_v4().to_string(),
            &failure,
            "budget gate detail".to_string(),
            false, // NOT headless — leaves the decision open
            "zai",
            "glm-4.7",
        )
        .await;
        assert_eq!(first.detail, "budget gate detail");
        // Y4a: nothing was granted, so nothing may be retried.
        assert!(!first.gate_grant_minted);
        let open_after_first: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM decisions WHERE goal_id IS NULL AND kind = 'choice' \
             AND status = 'open'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            open_after_first, 0,
            "a non-headless call creates no decision at all — nothing to dedupe against yet"
        );

        // Manually raise the decision the way a headless run would, then
        // confirm a second call finds and reuses it rather than filing a
        // duplicate.
        let cfg = crate::cost_router::budget::load_budget_config();
        let request = crate::cost_router::budget::with_requesting_session_id(
            crate::cost_router::budget::reservation_gate_decision_request(
                crate::cost_router::budget::BudgetScope::Task,
                5.0,
                0.0,
                0.20,
                5.0,
                cfg.task.gate,
                None,
                None,
            ),
            &session.id,
        );
        crate::decisions::create_decision(&pool, request)
            .await
            .unwrap();
        let open_after_manual: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM decisions WHERE goal_id IS NULL AND kind = 'choice' \
             AND status = 'open'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(open_after_manual, 1);

        let second = session_scoped_provider_authorization_failure(
            &manager,
            &session,
            &uuid::Uuid::new_v4().to_string(),
            &failure,
            "budget gate detail".to_string(),
            true, // headless this time — auto-answers the existing decision
            "zai",
            "glm-4.7",
        )
        .await;
        assert_eq!(second.detail, "budget gate detail");

        let total_reservation_gate_decisions: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM decisions WHERE goal_id IS NULL AND kind = 'choice' \
             AND json_extract(payload_json, '$.reservation_gate.requesting_session_id') = ?",
        )
        .bind(&session.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            total_reservation_gate_decisions, 1,
            "the second call must reuse the existing decision, never file a duplicate"
        );
    }

    /// I1 pin: the mirror of `gate_auto_answer_is_a_no_op_when_not_headless`
    /// for the session-scoped path — a NON-headless coordinator call must
    /// leave any decision it raised open for a human, never auto-answer.
    #[tokio::test]
    async fn session_scoped_gate_auto_answer_is_a_no_op_when_not_headless() {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage::default()));
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(fixture.clone(), true).await;

        let failure = ProviderAuthorizationFailure::NeedsGate {
            scope: crate::cost_router::budget::BudgetScope::Task,
            spent_usd: 5.0,
            held_usd: 0.0,
            requested_usd: 0.20,
            ceiling_usd: 5.0,
        };
        let outcome = session_scoped_provider_authorization_failure(
            &manager,
            &session,
            &uuid::Uuid::new_v4().to_string(),
            &failure,
            "budget gate detail".to_string(),
            false, // NOT headless
            "zai",
            "glm-4.7",
        )
        .await;
        assert_eq!(outcome.detail, "budget gate detail");
        // Y4a: a non-headless gate is for a human — never an in-turn retry.
        assert!(!outcome.gate_grant_minted);

        let pool = manager.storage().pool().await.unwrap().clone();
        let any_decision: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM decisions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            any_decision, 0,
            "a non-headless coordinator call must not even raise a decision"
        );
    }

    /// M2 pin (harness loop, Round F, Lane F3, from Lane G9's MEDIUM
    /// finding): a session that has already been auto-answered
    /// `SESSION_GATE_AUTO_ANSWER_CAP_DEFAULT` (5) times must be escalated
    /// honestly to a human on the next gate hit, not auto-answered again.
    /// Fakes the counter directly (5 already-answered `ACTOR_HEADLESS_
    /// POLICY` decisions inserted for this session) rather than driving 5
    /// real gate cycles, so this test is fast and exercises the counting
    /// query in isolation from the rest of the auto-answer machinery.
    #[tokio::test]
    async fn session_scoped_gate_auto_answer_escalates_once_the_cap_is_reached() {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage::default()));
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(fixture.clone(), true).await;
        let pool = manager.storage().pool().await.unwrap().clone();

        for i in 0..SESSION_GATE_AUTO_ANSWER_CAP_DEFAULT {
            let payload = serde_json::json!({
                "reservation_gate": { "requesting_session_id": session.id },
            });
            sqlx::query(
                "INSERT INTO decisions
                    (id, kind, goal_id, tier, headline, detail, payload_json,
                     status, answer, acted_by, resolved_at)
                 VALUES (?, 'choice', NULL, 1, 'fake auto-answer', 'fake auto-answer detail', ?,
                         'answered', 'choice', ?, ?)",
            )
            .bind(format!("fake-auto-answer-{i}"))
            .bind(payload.to_string())
            .bind(crate::decisions::ACTOR_HEADLESS_POLICY)
            .bind(chrono::Utc::now().to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();
        }
        let count_before = session_scoped_auto_answer_count(&pool, &session.id).await;
        assert_eq!(
            count_before, SESSION_GATE_AUTO_ANSWER_CAP_DEFAULT,
            "setup: the fake counter must read back exactly the cap"
        );

        let failure = ProviderAuthorizationFailure::NeedsGate {
            scope: crate::cost_router::budget::BudgetScope::Task,
            spent_usd: 5.0,
            held_usd: 0.0,
            requested_usd: 0.20,
            ceiling_usd: 5.0,
        };
        let invocation_id = uuid::Uuid::new_v4().to_string();
        let outcome = session_scoped_provider_authorization_failure(
            &manager,
            &session,
            &invocation_id,
            &failure,
            "budget gate detail".to_string(),
            true, // headless — would normally auto-answer
            "zai",
            "glm-4.7",
        )
        .await;
        assert_eq!(
            outcome.detail, "budget gate detail",
            "the call still ends the turn honestly, it just must not auto-answer"
        );
        // Y4a: the cap escalates instead of auto-answering, so no reservation
        // was minted and the coordinator must NOT retry in place.
        assert!(
            !outcome.gate_grant_minted,
            "a capped session got no grant, so nothing authorizes an in-turn retry"
        );

        // No reservation was granted under this call's own invocation_id —
        // the auto-answer never ran.
        let reservation_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM cost_reservations WHERE invocation_id = ?")
                .bind(&invocation_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            reservation_count, 0,
            "a session past its auto-answer cap must not get a fresh grant"
        );

        // The decision this call raised (or reused) stays open — escalated
        // to a human, never silently answered — ACTOR_HEADLESS_POLICY is
        // never attributed to it.
        let decision =
            crate::decisions::find_open_reservation_gate_decision_for_session(&pool, &session.id)
                .await
                .unwrap();
        assert!(
            decision.is_some(),
            "a decision must exist and stay open for a human once the cap is reached"
        );
    }

    async fn insert_answered_gate_decision(
        pool: &sqlx::Pool<sqlx::Sqlite>,
        goal_id: Option<&str>,
        session_id: &str,
    ) {
        let payload = serde_json::json!({
            "reservation_gate": { "requesting_session_id": session_id },
        });
        sqlx::query(
            "INSERT INTO decisions
                (id, kind, goal_id, tier, headline, detail, payload_json,
                 status, answer, acted_by, resolved_at)
             VALUES (?, 'choice', ?, 1, 'fake auto-answer', 'fake auto-answer detail', ?,
                     'answered', 'choice', ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(goal_id)
        .bind(payload.to_string())
        .bind(crate::decisions::ACTOR_HEADLESS_POLICY)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn goal_scoped_gate_auto_answer_escalates_once_the_cap_is_reached() {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage::default()));
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(fixture, true).await;
        let pool = manager.storage().pool().await.unwrap().clone();
        let card = dispatched_goal_card(&pool, &session.id).await;

        for _ in 0..GOAL_GATE_AUTO_ANSWER_CAP_DEFAULT {
            insert_answered_gate_decision(&pool, Some(&card.id), &session.id).await;
        }
        assert_eq!(
            goal_scoped_auto_answer_count(&pool, &card.id).await,
            GOAL_GATE_AUTO_ANSWER_CAP_DEFAULT
        );

        let failure = ProviderAuthorizationFailure::NeedsGate {
            scope: crate::cost_router::budget::BudgetScope::Task,
            spent_usd: 5.0,
            held_usd: 0.0,
            requested_usd: 0.20,
            ceiling_usd: 5.0,
        };
        let request = crate::cost_router::budget::reservation_gate_decision_request(
            crate::cost_router::budget::BudgetScope::Task,
            5.0,
            0.0,
            0.20,
            5.0,
            5.0,
            Some(card.id.clone()),
            None,
        );
        let decision = crate::decisions::create_decision(&pool, request)
            .await
            .unwrap();
        let invocation_id = uuid::Uuid::new_v4().to_string();
        let outcome = try_auto_answer_goal_scoped_reservation_gate(
            &pool,
            &card.id,
            &session,
            &invocation_id,
            &failure,
            &decision.id,
            true,
            &manager,
            "budget gate detail",
            "zai",
            "glm-4.7",
        )
        .await
        .expect("the cap must report an explicit not-minted outcome");
        assert_eq!(outcome.detail, "budget gate detail");
        assert!(!outcome.gate_grant_minted);

        let reservation_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM cost_reservations WHERE invocation_id = ?")
                .bind(&invocation_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(reservation_count, 0, "the ninth gate must not mint a grant");
        let reloaded = crate::decisions::get_decision(&pool, &decision.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reloaded.status, "open");
        assert!(reloaded.acted_by.is_none());
        assert_eq!(
            goal_scoped_auto_answer_count(&pool, &card.id).await,
            GOAL_GATE_AUTO_ANSWER_CAP_DEFAULT,
            "the ninth gate must stay unanswered for a human"
        );
    }

    #[tokio::test]
    async fn goal_scoped_gate_count_excludes_headless_escalation_answers() {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage::default()));
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(fixture, true).await;
        let pool = manager.storage().pool().await.unwrap().clone();
        let card = dispatched_goal_card(&pool, &session.id).await;

        sqlx::query(
            "INSERT INTO decisions
                (id, kind, goal_id, tier, headline, detail, payload_json,
                 status, answer, answer_note, acted_by, resolved_at)
             VALUES (?, 'choice', ?, 1, 'escalation', 'retry requested', '{}',
                     'answered', 'choice', 'headless escalation retry', ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&card.id)
        .bind(crate::decisions::ACTOR_HEADLESS_POLICY)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            goal_scoped_auto_answer_count(&pool, &card.id).await,
            0,
            "headless escalation answers must not consume the reservation-gate cap"
        );

        insert_answered_gate_decision(&pool, Some(&card.id), &session.id).await;
        assert_eq!(goal_scoped_auto_answer_count(&pool, &card.id).await, 1);
    }

    #[tokio::test]
    async fn goal_scoped_gate_and_session_scoped_gate_counts_are_independent() {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage::default()));
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(fixture, true).await;
        let pool = manager.storage().pool().await.unwrap().clone();
        let card = dispatched_goal_card(&pool, &session.id).await;
        let other_card = dispatched_goal_card(&pool, &session.id).await;

        insert_answered_gate_decision(&pool, None, &session.id).await;
        assert_eq!(
            session_scoped_auto_answer_count(&pool, &session.id).await,
            1
        );
        assert_eq!(goal_scoped_auto_answer_count(&pool, &card.id).await, 0);

        // Even matching requesting_session_id values cannot inflate the
        // session counter. The goal counter spans worker sessions instead.
        insert_answered_gate_decision(&pool, Some(&card.id), &session.id).await;
        insert_answered_gate_decision(&pool, Some(&card.id), "another-worker").await;
        insert_answered_gate_decision(&pool, Some(&other_card.id), &session.id).await;
        assert_eq!(
            session_scoped_auto_answer_count(&pool, &session.id).await,
            1
        );
        assert_eq!(goal_scoped_auto_answer_count(&pool, &card.id).await, 2);
        assert_eq!(
            goal_scoped_auto_answer_count(&pool, &other_card.id).await,
            1
        );

        insert_answered_gate_decision(&pool, None, &session.id).await;
        assert_eq!(
            session_scoped_auto_answer_count(&pool, &session.id).await,
            2
        );
        assert_eq!(goal_scoped_auto_answer_count(&pool, &card.id).await, 2);
    }

    /// Grants a reservation for `session` via a headless gate auto-answer
    /// (the exact path `try_auto_answer_reservation_gate` takes on success),
    /// returning the `invocation_id` it was granted under.
    async fn grant_a_gate_reservation(
        pool: &sqlx::Pool<sqlx::Sqlite>,
        manager: &Arc<SessionManager>,
        session: &Session,
        card_id: &str,
        provider: &str,
        model: &str,
    ) -> String {
        let failure = ProviderAuthorizationFailure::NeedsGate {
            scope: crate::cost_router::budget::BudgetScope::Task,
            spent_usd: 5.0,
            held_usd: 0.0,
            requested_usd: 0.20,
            ceiling_usd: 5.0,
        };
        let request = crate::cost_router::budget::reservation_gate_decision_request(
            crate::cost_router::budget::BudgetScope::Task,
            5.0,
            0.0,
            0.20,
            5.0,
            5.0,
            Some(card_id.to_string()),
            None,
        );
        let decision = crate::decisions::create_decision(pool, request)
            .await
            .unwrap();
        let invocation_id = uuid::Uuid::new_v4().to_string();
        let outcome = try_auto_answer_reservation_gate(
            pool,
            card_id,
            session,
            &invocation_id,
            &failure,
            &decision.id,
            true, // headless
            manager,
            "budget gate detail",
            provider,
            model,
        )
        .await;
        assert_eq!(
            outcome,
            Some(ProviderAuthorizationOutcome {
                detail: "budget gate detail".to_string(),
                gate_grant_minted: true,
            }),
            "setup: the gate must have been headlessly auto-answered"
        );
        invocation_id
    }

    /// I1 pin (a): a session with a pending grant makes its next call
    /// without a new gate decision. `reusable_gate_grant_invocation_id`
    /// hands back the ORIGINAL `invocation_id`, and reusing it for the next
    /// `reserve_provider_invocation_for_model` call takes the idempotent
    /// `AlreadyReserved` fast path — no second gate decision is ever
    /// created. Pre-fix (no `granted_via_gate` marker / lookup), this
    /// function does not exist and the retry would have minted a fresh
    /// `invocation_id`, re-hitting the gate exactly as F10 traced.
    #[tokio::test]
    async fn reusable_gate_grant_invocation_id_lets_the_same_session_retry_without_a_new_gate_decision(
    ) {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage::default()));
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(fixture.clone(), true).await;
        let pool = manager.storage().pool().await.unwrap().clone();
        let card = dispatched_goal_card(&pool, &session.id).await;

        let granted_invocation_id = grant_a_gate_reservation(
            &pool,
            &manager,
            &session,
            &card.id,
            fixture.get_name(),
            &fixture.get_model_config().model_name,
        )
        .await;

        let reused = reusable_gate_grant_invocation_id(
            &manager,
            &session.id,
            fixture.get_name(),
            &fixture.get_model_config().model_name,
        )
        .await;
        assert_eq!(
            reused,
            Some(granted_invocation_id.clone()),
            "the session's own pending grant must be reused verbatim"
        );

        let decisions_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM decisions")
            .fetch_one(&pool)
            .await
            .unwrap();
        let retry = reserve_provider_invocation_for_model(
            Arc::clone(&manager),
            &session,
            fixture.get_name(),
            fixture.cost_tier(),
            fixture.get_model_config(),
            fixture.retry_config().max_physical_attempts(),
            reused.unwrap(),
            None,
            CallProvenance::default(),
        )
        .await;
        assert!(
            retry.is_ok(),
            "reusing the granted invocation_id must skip the gate: {retry:?}"
        );
        let decisions_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM decisions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            decisions_before, decisions_after,
            "no new gate decision may be created for a call that reuses a granted invocation_id"
        );
    }

    /// I1 pin (b): a different session cannot consume another session's
    /// grant. Looking it up under session B's id (never session A's, the one
    /// the grant actually belongs to) returns `None`, and the grant is left
    /// completely untouched — session A can still consume it afterward.
    #[tokio::test]
    async fn reusable_gate_grant_invocation_id_is_none_for_a_different_session() {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage::default()));
        let (_directory_a, manager, session_a, _completion_a) =
            fast_accounting_fixture(fixture.clone(), true).await;
        let session_b = manager
            .create_session(
                std::env::temp_dir(),
                "fast-accounting-b".to_string(),
                SessionType::Hidden,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        manager.begin_budget_task(&session_b.id).await.unwrap();
        let session_b = manager.get_session(&session_b.id, false).await.unwrap();

        let pool = manager.storage().pool().await.unwrap().clone();
        let card = dispatched_goal_card(&pool, &session_a.id).await;
        let granted_invocation_id = grant_a_gate_reservation(
            &pool,
            &manager,
            &session_a,
            &card.id,
            fixture.get_name(),
            &fixture.get_model_config().model_name,
        )
        .await;

        let reused_by_b = reusable_gate_grant_invocation_id(
            &manager,
            &session_b.id,
            fixture.get_name(),
            &fixture.get_model_config().model_name,
        )
        .await;
        assert_eq!(
            reused_by_b, None,
            "a grant belongs to the session whose failed call it authorized, never to a different session"
        );

        // Untouched: session A can still find and reuse it.
        let reused_by_a = reusable_gate_grant_invocation_id(
            &manager,
            &session_a.id,
            fixture.get_name(),
            &fixture.get_model_config().model_name,
        )
        .await;
        assert_eq!(reused_by_a, Some(granted_invocation_id));
    }

    /// I1 pin (c): an expired grant re-gates. A grant older than
    /// `SessionManager::GRANT_CONSUME_TTL_SECS_DEFAULT` is released, not
    /// reused — the caller falls back to minting a fresh `invocation_id` and
    /// the ordinary gate/budget path runs again for it.
    #[tokio::test]
    async fn reusable_gate_grant_invocation_id_is_none_once_the_grant_has_expired() {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage::default()));
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(fixture.clone(), true).await;
        let pool = manager.storage().pool().await.unwrap().clone();
        let card = dispatched_goal_card(&pool, &session.id).await;
        let granted_invocation_id = grant_a_gate_reservation(
            &pool,
            &manager,
            &session,
            &card.id,
            fixture.get_name(),
            &fixture.get_model_config().model_name,
        )
        .await;

        // Backdate the grant's created_at past the TTL default (120s).
        let stale = (chrono::Utc::now() - chrono::Duration::seconds(200)).to_rfc3339();
        sqlx::query("UPDATE cost_reservations SET created_at = ? WHERE invocation_id = ?")
            .bind(&stale)
            .bind(&granted_invocation_id)
            .execute(&pool)
            .await
            .unwrap();

        let reused = reusable_gate_grant_invocation_id(
            &manager,
            &session.id,
            fixture.get_name(),
            &fixture.get_model_config().model_name,
        )
        .await;
        assert_eq!(reused, None, "a stale grant must re-gate, not be reused");

        let state: String =
            sqlx::query_scalar("SELECT state FROM cost_reservations WHERE invocation_id = ?")
                .bind(&granted_invocation_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            state, "released",
            "the expired grant must be released, not left pending forever"
        );
    }

    #[tokio::test]
    async fn accounted_fast_refuses_paid_dispatch_without_a_durable_task() {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage::default()));
        let calls = Arc::clone(&fixture.calls);
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(fixture.clone(), false).await;

        assert!(AccountedFastCompletion::complete_fast_accounted(
            Arc::clone(&manager),
            session,
            fixture,
            "summarize",
            &[Message::user().with_text("hello")],
            &[],
            false,
        )
        .await
        .is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn accounted_full_refuses_paid_dispatch_without_a_durable_task() {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage::default()));
        let calls = Arc::clone(&fixture.calls);
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(fixture.clone(), false).await;

        assert!(AccountedFastCompletion::complete_accounted(
            Arc::clone(&manager),
            session,
            fixture,
            "summarize",
            &[Message::user().with_text("hello")],
            &[],
            false,
        )
        .await
        .is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn accounted_fast_missing_usage_keeps_the_paid_hold_unknown() {
        let fixture = FastAccountingProvider::paid(Usage::default());
        let calls = Arc::clone(&fixture.calls);
        let (_directory, manager, session, completion) =
            fast_accounting_fixture(Arc::new(fixture), true).await;

        assert!(crate::context_mgmt::AccountedFastCompletion::complete_fast(
            &completion,
            "summarize",
            &[Message::user().with_text("hello")],
            &[],
        )
        .await
        .is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let pool = manager.storage().pool().await.unwrap();
        let unknown: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cost_reservations WHERE session_id = ? AND state = 'unknown'",
        )
        .bind(&session.id)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(unknown, 1);
    }

    #[tokio::test]
    async fn accounted_fast_paid_failure_does_not_dispatch_the_regular_fallback() {
        let mut fixture = FastAccountingProvider::paid(Usage::default());
        fixture.fail = true;
        fixture.model_config = ModelConfig::new("claude-sonnet-4-5")
            .unwrap()
            .with_canonical_limits("anthropic")
            .with_fast("claude-haiku-4-5", "anthropic")
            .unwrap();
        let calls = Arc::clone(&fixture.calls);
        let (_directory, manager, session, completion) =
            fast_accounting_fixture(Arc::new(fixture), true).await;

        assert!(crate::context_mgmt::AccountedFastCompletion::complete_fast(
            &completion,
            "summarize",
            &[Message::user().with_text("hello")],
            &[],
        )
        .await
        .is_err());

        // A paid fast error may have reached the remote provider. The full
        // model is deliberately not a second unauthorized physical attempt.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let pool = manager.storage().pool().await.unwrap();
        let unknown: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cost_reservations WHERE session_id = ? AND state = 'unknown'",
        )
        .bind(&session.id)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(unknown, 1);
    }

    #[tokio::test]
    async fn accounted_fast_local_failure_retries_the_full_model_as_a_separate_attempt() {
        let fixture = LocalFastFallbackProvider::new();
        let calls = Arc::clone(&fixture.calls);
        let selected_models = Arc::clone(&fixture.selected_models);
        let (_directory, manager, session, completion) =
            fast_accounting_fixture(Arc::new(fixture), false).await;

        crate::context_mgmt::AccountedFastCompletion::complete_fast(
            &completion,
            "summarize",
            &[Message::user().with_text("hello")],
            &[],
        )
        .await
        .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            selected_models.lock().unwrap().as_slice(),
            ["fast-local-model", "full-local-model"]
        );
        let pool = manager.storage().pool().await.unwrap();
        let rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cost_ledger WHERE session_id = ? AND model = 'full-local-model'",
        )
        .bind(&session.id)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(rows, 1);
    }

    /// v60 (node A4): the local fast->regular fallback is a re-dispatch after
    /// a failed call, so its ledger row says `retry`, names a prior call id,
    /// and carries the observed outcome of the complete response.
    #[tokio::test]
    async fn accounted_fast_local_fallback_row_is_labelled_retry_with_outcome() {
        let fixture = LocalFastFallbackProvider::new();
        let (_directory, manager, session, completion) =
            fast_accounting_fixture(Arc::new(fixture), false).await;

        crate::context_mgmt::AccountedFastCompletion::complete_fast(
            &completion,
            "summarize",
            &[Message::user().with_text("hello")],
            &[],
        )
        .await
        .unwrap();

        let pool = manager.storage().pool().await.unwrap();
        type ProvenanceRow = (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<String>,
        );
        let (call_id, purpose, retry_of, trigger, evidence, tool_calls, kind): ProvenanceRow =
            sqlx::query_as(
                "SELECT call_id, call_purpose, retry_of_call_id, trigger_ref, pre_call_evidence,
                    tool_call_count, completion_kind
             FROM cost_ledger WHERE session_id = ? AND model = 'full-local-model'",
            )
            .bind(&session.id)
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(purpose.as_deref(), Some("retry"));
        let retry_of = retry_of.expect("a retry must name the failed invocation");
        assert_ne!(retry_of, call_id, "retry_of_call_id is the EARLIER call");
        assert_eq!(trigger, None, "the fixture handle knew no trigger");
        assert_eq!(evidence, None, "no caller populates pre_call_evidence yet");
        assert_eq!(tool_calls, Some(0));
        assert_eq!(kind.as_deref(), Some("text"));
    }

    /// v60 (node A4): the main loop labels a call only from facts it holds.
    #[test]
    fn turn_call_provenance_labels_only_what_the_loop_knows() {
        use rmcp::model::{CallToolResult, Content};

        let user = Message::user().with_text("do the thing").with_id("m1");
        // First iteration answering the user's own message.
        let first = turn_call_provenance(std::slice::from_ref(&user), 1, None);
        assert_eq!(first.purpose, CallPurpose::UserResponse);
        assert_eq!(first.trigger_ref.as_deref(), Some("message:m1"));
        assert_eq!(first.retry_of_call_id, None);
        assert_eq!(first.pre_call_evidence, None);

        // After tool results: a tool continuation, triggered by the last one.
        let tool_results = Message::user()
            .with_tool_response(
                "tc-1",
                Ok(CallToolResult::success(vec![Content::text("a")])),
            )
            .with_tool_response(
                "tc-2",
                Ok(CallToolResult::success(vec![Content::text("b")])),
            );
        let history = vec![
            user.clone(),
            Message::assistant().with_text("calling tools"),
            tool_results,
        ];
        let cont = turn_call_provenance(&history, 2, None);
        assert_eq!(cont.purpose, CallPurpose::ToolContinuation);
        assert_eq!(cont.trigger_ref.as_deref(), Some("tool_result:tc-2"));

        // A re-run after a failed invocation is a retry of it, trigger kept.
        let retry = turn_call_provenance(&history, 3, Some("failed-call".to_string()));
        assert_eq!(retry.purpose, CallPurpose::Retry);
        assert_eq!(retry.retry_of_call_id.as_deref(), Some("failed-call"));
        assert_eq!(retry.trigger_ref.as_deref(), Some("tool_result:tc-2"));

        // A later iteration answering a harness-injected user note: unknown.
        let nudge = Message::user().with_text("keep going").with_id("n1");
        let later = turn_call_provenance(&[user.clone(), nudge], 4, None);
        assert_eq!(later.purpose, CallPurpose::Unknown);
        assert_eq!(later.trigger_ref.as_deref(), Some("message:n1"));

        // Assistant-last or empty history: nothing is claimed.
        let assistant_last = turn_call_provenance(&[Message::assistant().with_text("hi")], 1, None);
        assert_eq!(assistant_last, CallProvenance::default());
        assert_eq!(
            turn_call_provenance(&[], 1, None),
            CallProvenance::default()
        );
    }

    /// Collects what a scoped `tracing` subscriber writes, so a test can read
    /// the reservation seam's log line back.
    #[derive(Clone, Default)]
    struct CapturedReservationLog(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for CapturedReservationLog {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl CapturedReservationLog {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    fn reservation_log_subscriber(
        log: &CapturedReservationLog,
    ) -> impl tracing::Subscriber + Send + Sync + 'static {
        let writer = log.clone();
        tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .finish()
    }

    // 2026-09-19: QA could not confirm the prompt-sized hold in the live app
    // because the only run after the redeploy failed over to a free local
    // model, and a free call left no reservation line at all. The unpaid exit
    // must now say it ran, and carry the prompt count the seam was handed.
    #[tokio::test]
    async fn an_unpaid_call_logs_that_it_skipped_the_hold_and_what_it_counted() {
        let fixture = LocalFastFallbackProvider::new();
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(Arc::new(fixture.clone()), false).await;
        let log = CapturedReservationLog::default();
        let _guard = tracing::subscriber::set_default(reservation_log_subscriber(&log));

        let invocation = reserve_provider_invocation_for_model(
            Arc::clone(&manager),
            &session,
            fixture.get_name(),
            fixture.cost_tier(),
            fixture.get_model_config(),
            fixture.retry_config().max_physical_attempts(),
            uuid::Uuid::new_v4().to_string(),
            Some(1_234),
            CallProvenance::default(),
        )
        .await
        .expect("a free local call needs no paid authorization");

        assert!(invocation.reservation_id.is_none());
        assert!(invocation.attempts_reserved.is_none());
        let text = log.text();
        assert!(
            text.contains("reservation skipped: unpaid tier holds no dollars"),
            "{text}"
        );
        assert!(text.contains("provider=local-fixture"), "{text}");
        assert!(text.contains("counted_prompt_tokens=Some(1234)"), "{text}");
        assert!(
            !text.contains("reservation bound computed"),
            "a free call must not claim a paid hold: {text}"
        );
    }

    // The paid line is what QA greps for on a real paid run; pin that it names
    // the count it was given and the input the hold actually covers.
    #[tokio::test]
    async fn a_paid_call_logs_the_counted_prompt_and_the_input_it_reserved() {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage::default()));
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(fixture.clone(), true).await;
        let model_config = fixture.get_model_config();
        let window = model_config.context_limit.unwrap()
            - usize::try_from(model_config.max_tokens.unwrap()).unwrap();
        let counted = 1_000usize;
        let expected_input =
            (counted + crate::cost_router::reservation::prompt_token_margin(counted)).min(window);
        let log = CapturedReservationLog::default();
        let _guard = tracing::subscriber::set_default(reservation_log_subscriber(&log));

        // The outcome depends on the configured budget; the log line is
        // written before the budget is consulted, which is what this pins.
        let _ = reserve_provider_invocation_for_model(
            Arc::clone(&manager),
            &session,
            fixture.get_name(),
            fixture.cost_tier(),
            model_config,
            fixture.retry_config().max_physical_attempts(),
            uuid::Uuid::new_v4().to_string(),
            Some(counted),
            CallProvenance::default(),
        )
        .await;

        let text = log.text();
        assert!(text.contains("reservation bound computed"), "{text}");
        assert!(text.contains("counted_prompt_tokens=Some(1000)"), "{text}");
        assert!(
            text.contains(&format!("reserved_input_tokens={expected_input}")),
            "{text}"
        );
        assert!(!text.contains("reservation skipped"), "{text}");
    }

    #[tokio::test]
    async fn accounted_fast_cancelled_paid_dispatch_keeps_the_hold_unknown() {
        let fixture = PendingPaidProvider {
            calls: Arc::new(AtomicUsize::new(0)),
            model_config: ModelConfig::new("claude-haiku-4-5")
                .unwrap()
                .with_canonical_limits("anthropic"),
        };
        let calls = Arc::clone(&fixture.calls);
        let (_directory, manager, session, completion) =
            fast_accounting_fixture(Arc::new(fixture), true).await;
        let completion = Arc::new(completion);
        let task = tokio::spawn({
            let completion = Arc::clone(&completion);
            async move {
                crate::context_mgmt::AccountedFastCompletion::complete_fast(
                    completion.as_ref(),
                    "summarize",
                    &[Message::user().with_text("hello")],
                    &[],
                )
                .await
            }
        });

        tokio::time::timeout(Duration::from_secs(2), async {
            while calls.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("pending provider must reach dispatch before cancellation");
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());

        let pool = manager.storage().pool().await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let unknown: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM cost_reservations WHERE session_id = ? AND state = 'unknown'",
                )
                .bind(&session.id)
                .fetch_one(pool)
                .await
                .unwrap();
                if unknown == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("cancellation guard must mark the paid hold unknown");
    }

    #[async_trait]
    impl Provider for NamedMockProvider {
        fn get_name(&self) -> &str {
            self.name
        }

        fn get_model_config(&self) -> ModelConfig {
            self.model_config.clone()
        }

        async fn stream(
            &self,
            _model_config: &ModelConfig,
            _session_id: &str,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<MessageStream, ProviderError> {
            let message = Message::assistant().with_text("ok");
            let usage = ProviderUsage::new(self.name.to_string(), Usage::default());
            Ok(stream_from_single_message(message, usage))
        }
    }

    /// M3: session 20260831_10 had NO model-identity fact anywhere in its
    /// prompt, so when the harness silently moved it off `qwen38_split` the
    /// model confabulated a third model name entirely. The line must be live
    /// (it follows a switch) and volatile (it must not bust the cached prefix).
    #[tokio::test]
    async fn the_prompt_says_which_model_is_actually_serving() -> anyhow::Result<()> {
        let agent = crate::agents::Agent::new();
        let session = agent
            .config
            .session_manager
            .create_session(
                std::env::current_dir().unwrap(),
                "test-model-identity".to_string(),
                SessionType::Hidden,
                GooseMode::default(),
            )
            .await?;

        agent
            .update_provider(
                std::sync::Arc::new(NamedMockProvider {
                    name: "qwen38_split",
                    model_config: ModelConfig::new("qwen3.8-27b").unwrap(),
                }),
                &session.id,
            )
            .await?;
        let (_, _, parts) = agent
            .prepare_tools_and_prompt(&session.id, session.working_dir.as_path())
            .await?;
        assert!(
            parts
                .volatile_suffix()
                .contains("You are currently served by qwen38_split/qwen3.8-27b."),
            "no live model identity in the volatile tail: {}",
            parts.volatile_suffix()
        );
        assert!(
            !parts
                .stable_prefix()
                .contains("You are currently served by"),
            "a per-turn fact in the cached prefix busts the prompt cache"
        );

        // The failover: same session, different provider AND model.
        agent
            .update_provider(
                std::sync::Arc::new(NamedMockProvider {
                    name: "anthropic",
                    model_config: ModelConfig::new("claude-haiku-4-5").unwrap(),
                }),
                &session.id,
            )
            .await?;
        let (_, _, parts) = agent
            .prepare_tools_and_prompt(&session.id, session.working_dir.as_path())
            .await?;
        assert!(
            parts
                .volatile_suffix()
                .contains("You are currently served by anthropic/claude-haiku-4-5."),
            "identity did not follow the switch: {}",
            parts.volatile_suffix()
        );
        assert!(
            !parts.render().contains("qwen38_split"),
            "the pre-failover identity survived the switch: {}",
            parts.render()
        );
        Ok(())
    }

    /// Lane R62f (A61 miss 5): a headless agent's prompt never carries the
    /// first-run tour offer; an ordinary one still does while the tour is
    /// not completed.
    #[tokio::test]
    async fn a_headless_session_prompt_never_offers_the_tour() -> anyhow::Result<()> {
        const TOUR_TEXT: &str = "This user has not been shown around yet.";
        let tour_completed = crate::config::Config::global()
            .get_param::<bool>(crate::agents::self_knowledge::TOUR_COMPLETED_KEY)
            .unwrap_or(false);

        let headless = crate::agents::Agent::new();
        headless.set_headless(true);
        let session = headless
            .config
            .session_manager
            .create_session(
                std::env::current_dir().unwrap(),
                "test-headless-tour".to_string(),
                SessionType::Hidden,
                GooseMode::default(),
            )
            .await?;
        headless
            .update_provider(
                std::sync::Arc::new(MockProvider {
                    model_config: ModelConfig::new("test-model").unwrap(),
                }),
                &session.id,
            )
            .await?;
        let (_, _, parts) = headless
            .prepare_tools_and_prompt(&session.id, session.working_dir.as_path())
            .await?;
        assert!(
            !parts.render().contains(TOUR_TEXT),
            "a headless session must not be offered the tour"
        );

        if !tour_completed
            && !crate::agents::platform_extensions::orchestrator::is_headless_dispatch_policy_installed()
        {
            let interactive = crate::agents::Agent::new();
            let session = interactive
                .config
                .session_manager
                .create_session(
                    std::env::current_dir().unwrap(),
                    "test-interactive-tour".to_string(),
                    SessionType::Hidden,
                    GooseMode::default(),
                )
                .await?;
            interactive
                .update_provider(
                    std::sync::Arc::new(MockProvider {
                        model_config: ModelConfig::new("test-model").unwrap(),
                    }),
                    &session.id,
                )
                .await?;
            let (_, _, parts) = interactive
                .prepare_tools_and_prompt(&session.id, session.working_dir.as_path())
                .await?;
            assert!(
                parts.render().contains(TOUR_TEXT),
                "control: a non-headless session still gets the tour offer"
            );
        }
        Ok(())
    }

    #[test]
    fn cache_hit_rate_is_surfaced_from_the_canonical_cost_reexport() {
        // The per-call cost path (`update_session_metrics`) surfaces the
        // prompt-cache hit rate via `crate::providers::canonical::cache_hit_rate_of`,
        // re-exported from the canonical cost ledger. Exercise that exact surface.
        // 3 of 4 cacheable tokens served from cache reads → 0.75 hit rate.
        let usage = Usage::default().with_cache_tokens(Some(3), Some(1));
        assert_eq!(cache_hit_rate_of(&usage), Some(0.75));
        // A fully warm prefix → 1.0; no cache activity → None (the log is skipped).
        assert_eq!(
            cache_hit_rate_of(&Usage::default().with_cache_tokens(Some(4), Some(0))),
            Some(1.0)
        );
        assert_eq!(cache_hit_rate_of(&Usage::default()), None);
    }

    #[test]
    fn local_cost_tier_recognizes_both_local_provider_engines() {
        assert!(is_local_provider("local"));
        assert!(is_local_provider("ollama"));
        assert!(is_local_provider("remote-ollama"));
        assert!(is_local_provider("qwen38_split"));
        assert!(!is_local_provider("anthropic"));
    }

    #[tokio::test]
    async fn prepare_tools_returns_sorted_tools_including_frontend() -> anyhow::Result<()> {
        let agent = crate::agents::Agent::new();

        let session = agent
            .config
            .session_manager
            .create_session(
                std::env::current_dir().unwrap(),
                "test-prepare-tools".to_string(),
                SessionType::Hidden,
                GooseMode::default(),
            )
            .await?;

        let model_config = ModelConfig::new("test-model").unwrap();
        let provider = std::sync::Arc::new(MockProvider { model_config });
        agent.update_provider(provider, &session.id).await?;

        // Add unsorted frontend tools
        let frontend_tools = vec![
            Tool::new(
                "frontend__z_tool".to_string(),
                "Z tool".to_string(),
                object!({ "type": "object", "properties": { } }),
            ),
            Tool::new(
                "frontend__a_tool".to_string(),
                "A tool".to_string(),
                object!({ "type": "object", "properties": { } }),
            ),
        ];

        agent
            .add_extension(
                crate::agents::extension::ExtensionConfig::Frontend {
                    name: "frontend".to_string(),
                    description: "desc".to_string(),
                    tools: frontend_tools,
                    instructions: None,
                    bundled: None,
                    available_tools: vec![],
                },
                &session.id,
            )
            .await
            .unwrap();

        let (tools, _toolshim_tools, _system_prompt) = agent
            .prepare_tools_and_prompt(&session.id, session.working_dir.as_path())
            .await?;

        let names: Vec<String> = tools.iter().map(|t| t.name.clone().into_owned()).collect();
        assert!(names.iter().any(|n| n == "frontend__a_tool"));
        assert!(names.iter().any(|n| n == "frontend__z_tool"));

        // Verify the names are sorted ascending
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);

        Ok(())
    }

    #[tokio::test]
    async fn test_stream_error_propagation() {
        use futures::StreamExt;

        type StreamItem = Result<(Option<Message>, Option<ProviderUsage>), ProviderError>;
        let stream = futures::stream::iter(vec![
            Ok((Some(Message::assistant().with_text("chunk1")), None)),
            Ok((Some(Message::assistant().with_text("chunk2")), None)),
            Err(ProviderError::RequestFailed(
                "simulated stream error".to_string(),
            )),
        ] as Vec<StreamItem>);

        let mut pinned = Box::pin(stream);
        let mut results = Vec::new();
        let mut error_seen = false;

        while let Some(result) = pinned.next().await {
            match result {
                Ok((message, _usage)) => {
                    if let Some(msg) = message {
                        results.push(msg.as_concat_text());
                    }
                }
                Err(_e) => {
                    error_seen = true;
                    break;
                }
            }
        }

        assert_eq!(results.len(), 2);
        assert_eq!(results[0], "chunk1");
        assert_eq!(results[1], "chunk2");
        assert!(
            error_seen,
            "Error should have been propagated, not silently ignored"
        );
    }

    #[tokio::test]
    async fn categorize_tool_requests_keeps_thinking_when_not_previously_streamed() {
        let agent = crate::agents::Agent::new();
        let response = Message::assistant()
            .with_thinking("final-only reasoning", "")
            .with_tool_request(
                "tool-1",
                Ok(rmcp::model::CallToolRequestParams::new("test_tool")),
            );

        let (_frontend_requests, other_requests, filtered_message) =
            agent.categorize_tool_requests(&response, &[], false).await;

        assert_eq!(other_requests.len(), 1);
        assert_eq!(filtered_message.content.len(), 2);
        assert!(matches!(
            filtered_message.content[0],
            MessageContent::Thinking(_)
        ));
        assert!(matches!(
            filtered_message.content[1],
            MessageContent::ToolRequest(_)
        ));
    }

    #[tokio::test]
    async fn categorize_tool_requests_drops_replayed_thinking_after_streaming() {
        let agent = crate::agents::Agent::new();
        let response = Message::assistant()
            .with_thinking("replayed reasoning", "")
            .with_tool_request(
                "tool-1",
                Ok(rmcp::model::CallToolRequestParams::new("test_tool")),
            );

        let (_frontend_requests, other_requests, filtered_message) =
            agent.categorize_tool_requests(&response, &[], true).await;

        assert_eq!(other_requests.len(), 1);
        assert_eq!(filtered_message.content.len(), 1);
        assert!(matches!(
            filtered_message.content[0],
            MessageContent::ToolRequest(_)
        ));
    }

    #[tokio::test]
    async fn categorize_tool_requests_skips_externally_dispatched_and_preserves_marker() {
        // External requests must (1) survive coercion with goose.external_dispatch
        // intact, (2) be excluded from both dispatch buckets, (3) stay in
        // filtered_message.
        use crate::conversation::message::TOOL_META_EXTERNAL_DISPATCH_KEY;

        let agent = crate::agents::Agent::new();

        let registry_tool = Tool::new("test_tool", "a test tool", object!({ "type": "object" }))
            .with_meta(rmcp::model::Meta(
                serde_json::json!({ "ui": { "visibility": ["model"] } })
                    .as_object()
                    .unwrap()
                    .clone(),
            ));

        let response = Message::assistant().with_tool_request_with_metadata(
            "tool-1",
            Ok(rmcp::model::CallToolRequestParams::new("test_tool")),
            None,
            Some(serde_json::json!({ TOOL_META_EXTERNAL_DISPATCH_KEY: true })),
        );

        let (frontend_requests, other_requests, filtered_message) = agent
            .categorize_tool_requests(&response, &[registry_tool], false)
            .await;

        assert!(
            frontend_requests.is_empty(),
            "external request leaked into frontend_requests: {frontend_requests:?}"
        );
        assert!(
            other_requests.is_empty(),
            "external request leaked into other_requests: {other_requests:?}"
        );
        assert_eq!(filtered_message.content.len(), 1);
        let tool_req = match &filtered_message.content[0] {
            MessageContent::ToolRequest(req) => req,
            other => panic!("expected ToolRequest, got {other:?}"),
        };
        assert!(
            tool_req.is_externally_dispatched(),
            "goose.external_dispatch marker was clobbered by coercion; merged tool_meta = {:?}",
            tool_req.tool_meta
        );
        let merged = tool_req
            .tool_meta
            .as_ref()
            .and_then(|v| v.as_object())
            .expect("tool_meta should be an object after merge");
        assert!(
            merged.contains_key("ui"),
            "registry tool meta keys were dropped; merged tool_meta = {merged:?}"
        );
    }

    fn make_tool_with_meta(meta_json: Option<serde_json::Value>) -> Tool {
        let mut tool = Tool::new("test_tool", "a test tool", object!({ "type": "object" }));
        if let Some(v) = meta_json {
            let obj = v.as_object().unwrap().clone();
            tool = tool.with_meta(rmcp::model::Meta(obj));
        }
        tool
    }

    #[test]
    fn test_tool_visible_when_no_meta() {
        let tool = make_tool_with_meta(None);
        assert!(is_tool_visible_to_model(&tool));
    }

    #[test]
    fn test_tool_visible_when_meta_has_no_ui() {
        let tool = make_tool_with_meta(Some(serde_json::json!({"other": "stuff"})));
        assert!(is_tool_visible_to_model(&tool));
    }

    #[test]
    fn test_tool_visible_when_ui_has_no_visibility() {
        let tool = make_tool_with_meta(Some(
            serde_json::json!({"ui": {"resourceUri": "ui://foo/bar"}}),
        ));
        assert!(is_tool_visible_to_model(&tool));
    }

    #[test]
    fn test_tool_visible_when_visibility_includes_model() {
        let tool = make_tool_with_meta(Some(
            serde_json::json!({"ui": {"visibility": ["model", "app"]}}),
        ));
        assert!(is_tool_visible_to_model(&tool));
    }

    #[test]
    fn test_tool_visible_when_visibility_is_model_only() {
        let tool = make_tool_with_meta(Some(serde_json::json!({"ui": {"visibility": ["model"]}})));
        assert!(is_tool_visible_to_model(&tool));
    }

    #[test]
    fn test_tool_hidden_when_visibility_is_app_only() {
        let tool = make_tool_with_meta(Some(serde_json::json!({"ui": {"visibility": ["app"]}})));
        assert!(!is_tool_visible_to_model(&tool));
    }

    #[test]
    fn test_tool_hidden_when_visibility_is_empty() {
        let tool = make_tool_with_meta(Some(serde_json::json!({"ui": {"visibility": []}})));
        assert!(!is_tool_visible_to_model(&tool));
    }

    #[test]
    fn test_tool_visible_when_visibility_is_not_array() {
        let tool = make_tool_with_meta(Some(serde_json::json!({"ui": {"visibility": "model"}})));
        assert!(is_tool_visible_to_model(&tool));
    }

    #[test]
    fn test_app_visible_when_no_meta() {
        let tool = make_tool_with_meta(None);
        assert!(is_tool_visible_to_app(&tool));
    }

    #[test]
    fn test_app_visible_when_visibility_includes_app() {
        let tool = make_tool_with_meta(Some(
            serde_json::json!({"ui": {"visibility": ["model", "app"]}}),
        ));
        assert!(is_tool_visible_to_app(&tool));
    }

    #[test]
    fn test_app_visible_when_visibility_is_app_only() {
        let tool = make_tool_with_meta(Some(serde_json::json!({"ui": {"visibility": ["app"]}})));
        assert!(is_tool_visible_to_app(&tool));
    }

    #[test]
    fn test_app_hidden_when_visibility_is_model_only() {
        let tool = make_tool_with_meta(Some(serde_json::json!({"ui": {"visibility": ["model"]}})));
        assert!(!is_tool_visible_to_app(&tool));
    }

    #[test]
    fn test_app_hidden_when_visibility_is_empty() {
        let tool = make_tool_with_meta(Some(serde_json::json!({"ui": {"visibility": []}})));
        assert!(!is_tool_visible_to_app(&tool));
    }

    // --- counted_prompt_tokens: sizing a paid hold to the prompt actually sent.
    // The user-visible failure these guard: the harness's first call (~30k
    // tokens, 7% of a 400k window, $0.12 spent) was refused with "requested
    // bound $1.56, ceiling $1.00" because the hold priced the whole window.

    fn counted_prompt_fixture_tool() -> Tool {
        Tool::new(
            "developer__text_editor".to_string(),
            "Edit a file on disk.".to_string(),
            object!({
                "type": "object",
                "required": ["path", "command"],
                "properties": {
                    "path": { "type": "string", "description": "Absolute path." },
                    "command": {
                        "type": "string",
                        "enum": ["view", "write", "str_replace"],
                        "description": "What to do."
                    },
                    "edits": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "old": { "type": "string" },
                                "new": { "type": "string" }
                            }
                        }
                    }
                }
            }),
        )
    }

    /// The count never falls below the repo's own chat-token estimator for
    /// the same request: it counts whole JSON (message envelopes, nested tool
    /// schema properties) where the estimator walks only part of it. This does
    /// not prove the count bounds a provider's own tokenizer — that is what
    /// `prompt_token_margin` is for — only that nothing sent is left out.
    #[tokio::test]
    async fn counted_prompt_never_undercounts_the_chat_token_estimator() {
        let system = "You are a careful coding agent. Follow the repository rules.";
        let messages = vec![
            Message::user().with_text("Fix the failing test in src/lib.rs and explain the cause."),
            Message::assistant().with_text("Reading the file first."),
        ];
        let tools = vec![counted_prompt_fixture_tool()];

        let counted = counted_prompt_tokens(&[system], &messages, &tools)
            .await
            .expect("a text-only request must be countable");
        let estimator = crate::token_counter::create_token_counter()
            .await
            .unwrap()
            .count_chat_tokens(system, &messages, &tools);
        assert!(
            counted >= estimator,
            "counted {counted} must cover the estimator's {estimator}"
        );

        // Dropping the tools or a system part can only lower the count.
        let without_tools = counted_prompt_tokens(&[system], &messages, &[])
            .await
            .unwrap();
        assert!(without_tools < counted);
        let two_parts = counted_prompt_tokens(&[system, "volatile suffix"], &messages, &tools)
            .await
            .unwrap();
        assert!(two_parts > counted);
    }

    /// End to end through the real tokenizer and the real bound: a ~30k-token
    /// prompt to a 400k-context model reserves input for about that prompt
    /// plus the margin, not the whole window — which is itself now
    /// 367,232 tokens rather than the 272,000 that produced the reported
    /// $1.56 hold, because the reservation caps this reasoning model's raw
    /// 128,000-token `max_tokens` down to 32,768 on the output side
    /// (`REASONING_RESERVED_OUTPUT_TOKENS`), freeing the difference for input
    /// capacity (`context - output`).
    #[tokio::test]
    async fn a_thirty_thousand_token_prompt_is_not_reserved_as_the_whole_window() {
        let body = "word ".repeat(30_000);
        let messages = vec![Message::user().with_text(body)];
        let counted = counted_prompt_tokens(&["system"], &messages, &[])
            .await
            .unwrap();
        assert!(
            (30_000..32_000).contains(&counted),
            "expected about 30k tokens, counted {counted}"
        );

        let cfg = ModelConfig {
            context_limit: Some(400_000),
            max_tokens: Some(128_000),
            model_name: "gpt-5.4-mini".to_string(),
            ..ModelConfig::default()
        };
        let sized = crate::cost_router::reservation::plan_reservation_bound_for_prompt(
            "openai",
            "gpt-5.4-mini",
            CostTier::PaidApi,
            &cfg,
            2,
            Some(counted),
        )
        .unwrap()
        .unwrap();
        let window = crate::cost_router::reservation::plan_reservation_bound_for_prompt(
            "openai",
            "gpt-5.4-mini",
            CostTier::PaidApi,
            &cfg,
            2,
            None,
        )
        .unwrap()
        .unwrap();
        // gpt-5.4-mini is a reasoning model, so its cap is the reasoning one
        // (`output_reservation_cap`), not the plain 4,096.
        assert_eq!(
            window.input_tokens,
            400_000 - crate::cost_router::reservation::output_reservation_cap(&cfg)
        );
        assert!(sized.input_tokens >= counted);
        assert!(
            sized.input_tokens < 60_000,
            "reserved {}",
            sized.input_tokens
        );
        assert!(sized.amount_usd < window.amount_usd);
    }

    /// The user-reported refusal replayed against the real ledger gate: "task
    /// budget gate required before provider call (spent $0.12, authorization
    /// holds $0.00, requested bound $1.56, ceiling $1.00)", under the
    /// first-cut task ceilings ($0.50/$1/$2) that produced it.
    ///
    /// The old path (whole window, two attempts, output priced at this
    /// model's raw 128,000-token `max_tokens`) reproduces that exact gate —
    /// reconstructed here as a literal historical bound, because the CURRENT
    /// `plan_reservation_bound_for_prompt` no longer produces that shape at
    /// all: `RESERVED_OUTPUT_TOKENS` caps the output side at 4,096 regardless
    /// of whether the prompt was counted, so even an uncounted call is no
    /// longer $1.56 today (see `cost_router::reservation::RESERVED_OUTPUT_TOKENS`
    /// for why). The shipped path (the prompt counted by the tokenizer the
    /// streamed turn uses, the default reserved-attempt count resolved from
    /// the same retry envelope, the shipped ceilings, and the enforced output
    /// cap) is granted, and so is one retry's extension; the hold stays well
    /// under the shipped task gate.
    ///
    /// What this does not prove: it plans and reserves exactly as
    /// `reserve_provider_invocation_for_model` does, but passes the ceilings
    /// explicitly because that function reads them from the process config.
    /// It does not run a streamed turn.
    #[tokio::test]
    async fn the_reported_first_call_is_granted_under_the_ceiling_that_refused_it() {
        let directory = TempDir::new().unwrap();
        let manager = SessionManager::new(directory.path().to_path_buf());
        let session = manager
            .create_session(
                directory.path().to_path_buf(),
                "reported-gate".to_string(),
                SessionType::Hidden,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let task = manager.begin_budget_task(&session.id).await.unwrap();
        let pool = manager.storage().pool().await.unwrap();
        sqlx::query(
            "INSERT INTO cost_ledger (call_id, ts, session_id, task_id, cost_usd)
             VALUES ('earlier-call', ?, ?, ?, 0.12)",
        )
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(&session.id)
        .bind(&task)
        .execute(pool)
        .await
        .unwrap();
        let first_cut = crate::cost_router::budget::BudgetConfig {
            task: crate::cost_router::budget::BudgetCeilings {
                soft: 0.5,
                gate: 1.0,
                hard: 2.0,
            },
            session: crate::cost_router::budget::BudgetCeilings {
                soft: 2.0,
                gate: 5.0,
                hard: 10.0,
            },
        };
        let lease = "2099-01-01T00:00:00Z";
        let cfg = ModelConfig {
            context_limit: Some(400_000),
            max_tokens: Some(128_000),
            model_name: "gpt-5.4-mini".to_string(),
            ..ModelConfig::default()
        };
        let plan = |attempts, counted| {
            crate::cost_router::reservation::plan_reservation_bound_for_prompt(
                "openai",
                "gpt-5.4-mini",
                CostTier::PaidApi,
                &cfg,
                attempts,
                counted,
            )
            .unwrap()
            .unwrap()
        };

        // Before: the whole window, two attempts, output priced at this
        // model's raw max_tokens — a literal reconstruction of the reported
        // shape, since today's `plan(2, None)` no longer produces it (see
        // this test's own doc comment).
        let old_bound = crate::cost_router::reservation::ReservationBound {
            amount_usd: 1.56,
            input_tokens: 272_000,
            output_tokens: 128_000,
            max_physical_attempts: 2,
            pricing_provenance: crate::cost_router::reservation::PricingProvenance::CanonicalExact,
        };
        let old = manager
            .reserve_bounded_provider_invocation(
                "reported-old",
                &session.id,
                Some(&task),
                old_bound,
                lease,
                &first_cut,
            )
            .await
            .unwrap();
        match old {
            crate::session::CostReservationOutcome::NeedsGate {
                spent_usd,
                held_usd,
                requested_usd,
                ceiling_usd,
                ..
            } => {
                assert!((spent_usd - 0.12).abs() < 1e-9);
                assert_eq!(held_usd, 0.0);
                assert!((requested_usd - 1.56).abs() < 1e-9, "{requested_usd}");
                assert_eq!(ceiling_usd, 1.0);
            }
            other => panic!("expected the reported gate, got {other:?}"),
        }

        // After: the counted prompt and the shipped attempt count.
        let messages = vec![Message::user().with_text("word ".repeat(30_000))];
        let counted = counted_prompt_tokens(&["system"], &messages, &[]).await;
        assert!(counted.is_some());
        let envelope = crate::providers::RetryConfig::default().max_physical_attempts();
        let attempts = crate::cost_router::reserved_physical_attempts_with(envelope, |_| None);
        let bound = plan(attempts, counted);
        let shipped = crate::cost_router::budget::BudgetConfig::default();
        assert!(
            bound.amount_usd + 0.12 < shipped.task.gate,
            "hold ${:.4}",
            bound.amount_usd
        );
        let granted = manager
            .reserve_bounded_provider_invocation(
                "reported-new",
                &session.id,
                Some(&task),
                bound,
                lease,
                &shipped,
            )
            .await
            .unwrap();
        assert!(
            matches!(
                granted,
                crate::session::CostReservationOutcome::Granted { .. }
            ),
            "the reported first call must go out: {granted:?}"
        );

        // A third attempt's worth still fits under the shipped $3 gate with
        // $0.12 spent, so a retry is not gated either.
        let retry = manager.extend_reservation("reported-new", 1).await.unwrap();
        assert!(
            matches!(
                retry,
                crate::session::CostReservationOutcome::Granted { .. }
            ),
            "{retry:?}"
        );
    }

    /// An image's billed tokens depend on its pixel dimensions, not on any
    /// text a tokenizer sees, so a request carrying one is not counted and the
    /// caller falls back to the full-window hold.
    #[tokio::test]
    async fn a_request_with_an_image_is_not_counted() {
        let messages = vec![Message::user()
            .with_text("what is in this screenshot?")
            .with_image("aGk=", "image/png")];
        assert_eq!(
            counted_prompt_tokens(&["system"], &messages, &[]).await,
            None
        );
    }

    /// The same holds for an image returned inside a tool result, which is
    /// where screenshots usually enter a coding session; a text-only tool
    /// result is still counted.
    #[tokio::test]
    async fn a_tool_result_image_is_not_counted_but_tool_text_is() {
        use rmcp::model::{CallToolResult, Content};
        let with_image = vec![Message::user().with_tool_response(
            "call_1",
            Ok(CallToolResult::success(vec![
                Content::text("captured"),
                Content::image("aGk=", "image/png"),
            ])),
        )];
        assert_eq!(
            counted_prompt_tokens(&["system"], &with_image, &[]).await,
            None
        );

        let text_only = vec![Message::user().with_tool_response(
            "call_1",
            Ok(CallToolResult::success(vec![Content::text(
                "42 tests passed",
            )])),
        )];
        assert!(counted_prompt_tokens(&["system"], &text_only, &[])
            .await
            .is_some());
    }
}
