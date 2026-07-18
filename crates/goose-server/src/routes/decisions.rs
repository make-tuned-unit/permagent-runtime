//! Decision Inbox routes.
//!
//! Endpoints (registration in routes/mod.rs is the coordinator's commit):
//!   GET  /api/decisions               — top 10 open items ranked + summary envelope;
//!                                       `?all=1` returns ALL open items (Lane L4
//!                                       "+M more" overflow contract)
//!   POST /api/decisions/{id}/answer   — answer a decision and execute the gated effect
//!   GET  /api/decisions/history       — resolved decisions + audit join, cursor pagination
//!
//! Actor attribution (S5): HTTP answers are attributed to 'jesse' (single
//! operator holds the bearer token today). Henry answers Tier-1 in-process as
//! 'henry-policy'; timers use 'system'. Tier-2 with any other actor → 403.

use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use permagent::decisions::{self, AnswerError, DecisionAnswer};
use permagent::goal_state::GoalAction;
use permagent::goal_transition::{self, GuardError, TransitionEffects};
use permagent::permission::permission_confirmation::PrincipalType;
use permagent::permission::{Permission, PermissionConfirmation};
use permagent::sqlx::{Pool, Sqlite};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ── Request / response types ────────────────────────────────────────────────

/// Default number of open items returned without `?all=1` (Lane L4 shows the
/// top 10 and a "+M more" overflow computed from `summary.total_pending`).
const DEFAULT_INBOX_LIMIT: usize = 10;

#[derive(Deserialize)]
pub struct InboxQuery {
    /// `?all=1` (or `true`) returns every open decision instead of the top 10.
    all: Option<String>,
}

impl InboxQuery {
    fn wants_all(&self) -> bool {
        matches!(self.all.as_deref(), Some("1") | Some("true"))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InboxResponse {
    items: Vec<decisions::OpenDecisionItem>,
    summary: decisions::InboxSummary,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerRequest {
    answer: String,
    note: Option<String>,
    choice_id: Option<String>,
    input_text: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AnswerResponse {
    decision: decisions::Decision,
    /// What the gated effect did (e.g. "goal advanced to complete"), if any.
    effect: Option<String>,
    /// Present when the decision was answered but the effect failed — or when
    /// the effect committed and a follow-on step (dependent promotion) failed
    /// (`effect` is then ALSO set). Either way the failure is recorded in the
    /// audit log.
    effect_error: Option<String>,
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    limit: Option<i64>,
    before: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryResponse {
    items: Vec<decisions::HistoryItem>,
    /// Cursor for the next page (pass as ?before=).
    next_before: Option<i64>,
}

/// Map an AnswerError to an HTTP status. Tier-2 with acted_by != 'jesse' must
/// surface as 403 (required by spec; unit-tested below).
fn status_for_answer_error(err: &AnswerError) -> StatusCode {
    match err {
        AnswerError::NotFound => StatusCode::NOT_FOUND,
        AnswerError::AlreadyResolved(_) => StatusCode::CONFLICT,
        AnswerError::Forbidden(_) => StatusCode::FORBIDDEN,
        AnswerError::Invalid(_) => StatusCode::BAD_REQUEST,
        AnswerError::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

// ── Handlers ────────────────────────────────────────────────────────────────

async fn list_decisions_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<InboxQuery>,
) -> Result<Json<InboxResponse>, (StatusCode, String)> {
    let pool = pool_of(&state).await?;
    // Curation (L3): refresh `rank` for open rows before listing — the query
    // below orders by `rank DESC NULLS LAST`. Failure-tolerant: a rerank
    // error never blocks the inbox.
    if let Err(e) = permagent::decision_inbox::curation::rerank_open_decisions(&pool).await {
        tracing::warn!("Decision rerank failed (non-fatal): {}", e);
    }
    let mut items = decisions::list_open_decisions(&pool)
        .await
        .map_err(internal)?;
    if !query.wants_all() {
        items.truncate(DEFAULT_INBOX_LIMIT);
    }
    let summary = decisions::inbox_summary(&pool).await.map_err(internal)?;
    Ok(Json(InboxResponse { items, summary }))
}

async fn answer_decision_handler(
    State(state): State<Arc<AppState>>,
    Path(decision_id): Path<String>,
    Json(req): Json<AnswerRequest>,
) -> Result<Json<AnswerResponse>, (StatusCode, String)> {
    let pool = pool_of(&state).await?;

    let answer = DecisionAnswer {
        answer: req.answer,
        note: req.note,
        choice_id: req.choice_id,
        input_text: req.input_text,
    };

    // HTTP answers are attributed to 'jesse' (S5).
    let (decision, proof) =
        decisions::answer_decision(&pool, &decision_id, &answer, decisions::ACTOR_JESSE)
            .await
            .map_err(|e| (status_for_answer_error(&e), e.to_string()))?;

    // Execute the gated effect. The decision is already answered and audited;
    // an effect failure — or a follow-on warning after a committed effect —
    // is reported (and audit-logged), not silently dropped.
    let (effect, effect_error) = match execute_effect(&pool, &decision, proof).await {
        Ok((effect, warning)) => {
            if let Some(ref w) = warning {
                record_effect_failure(&pool, &decision, w).await;
            }
            (effect, warning)
        }
        Err(e) => {
            let msg = e.to_string();
            record_effect_failure(&pool, &decision, &msg).await;
            (None, Some(msg))
        }
    };

    // Tool-confirmation bridge: a `tool_approval` decision has no goal-state
    // effect above — its real effect is delivering the approve/reject back to the
    // parked agent turn (the command-center analogue of the legacy
    // /action-required modal). Runs here so the reported `effect` reflects what
    // actually happened: a delivery that reached no live waiter (turn cancelled,
    // daemon restarted, answered via the legacy prompt first) is a FAILED
    // effect — reported honestly and audit-recorded via the same path
    // goal-effects use, never papered over with a fabricated "running" claim.
    let (effect, effect_error) = if decision.kind == "tool_approval" {
        match deliver_tool_confirmation(&state, &decision).await {
            Ok(msg) => (Some(msg), effect_error),
            Err(e) => {
                record_effect_failure(&pool, &decision, &e).await;
                (effect, Some(e))
            }
        }
    } else {
        (effect, effect_error)
    };

    // Learn (L3): jesse-answered decisions become Brain memories. This handler
    // attributes all HTTP answers to 'jesse' (S5); the ingest fns re-check
    // status/acted_by themselves. Failure-tolerant — never breaks the answer
    // path. An `edit` (approve-with-edits) is BOTH an acceptance
    // (ingest_answered_decision, storing what was accepted) AND a correction
    // (ingest_edited_decision, storing the draft→revision delta as training),
    // so both run; each is a no-op when it does not apply. `tool_approval` is
    // skipped: it is an operational confirmation, not a judgment worth
    // memorializing, and would flood the Brain in approve/smart_approve modes.
    if decision.kind != "tool_approval" {
        if let Some(brain) = permagent::agents::platform_extensions::get_global_brain() {
            use permagent::decision_inbox::learn;
            if let Err(e) = learn::ingest_answered_decision(&pool, &brain, &decision).await {
                tracing::warn!(
                    "Decision {} answer ingestion failed (non-fatal): {}",
                    decision.id,
                    e
                );
            }
            if let Err(e) = learn::ingest_edited_decision(&pool, &brain, &decision).await {
                tracing::warn!(
                    "Decision {} correction ingestion failed (non-fatal): {}",
                    decision.id,
                    e
                );
            }
        }
    }

    Ok(Json(AnswerResponse {
        decision,
        effect,
        effect_error,
    }))
}

async fn history_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<HistoryResponse>, (StatusCode, String)> {
    let pool = pool_of(&state).await?;
    let items = decisions::decision_history(&pool, query.limit.unwrap_or(50), query.before)
        .await
        .map_err(internal)?;
    let next_before = items.iter().map(|i| i.seq).min();
    Ok(Json(HistoryResponse { items, next_before }))
}

// ── Gated effects ───────────────────────────────────────────────────────────

/// Execute the state change a freshly answered decision authorizes. The
/// `DecisionProof` is consumed here — one answer, at most one gated effect.
///
/// Returns `(effect, effect_warning)`: the effect message, plus a warning for
/// a follow-on step (dependent promotion) that failed AFTER the effect itself
/// committed. The warning surfaces in the response's `effect_error` and is
/// audit-recorded — the decision-spine rule is that nothing here may discard
/// a `Result` (bug-sweep wave 1).
async fn execute_effect(
    pool: &Pool<Sqlite>,
    decision: &decisions::Decision,
    proof: decisions::DecisionProof,
) -> Result<(Option<String>, Option<String>), GuardError> {
    let acted_by = proof.acted_by().to_string();
    match (decision.kind.as_str(), decision.answer.as_deref()) {
        // Review approved → goal completes; dependents become eligible.
        ("approve_review", Some("approve")) => {
            let goal_id = match decision.goal_id.as_deref() {
                Some(g) => g,
                None => return Ok((None, None)),
            };
            goal_transition::advance_goal_checked(
                pool,
                goal_id,
                GoalAction::Approve,
                &acted_by,
                Some(proof),
                TransitionEffects {
                    review_notes: decision.answer_note.clone(),
                    ..Default::default()
                },
            )
            .await?;
            let promotion_warning = match decision.project_id.as_deref() {
                Some(project_id) => {
                    goal_transition::promote_eligible_dependents_or_warn(pool, project_id, goal_id)
                        .await
                }
                None => None,
            };
            // Recognition write-back (SECONDARY proxy): approval is a positive
            // outcome. 2-hop join goal_id → worker_session_id → recognition events.
            permagent::recognition::write_back_decision_outcome(pool, goal_id, true).await;
            Ok((
                Some("goal approved: Review → Complete".to_string()),
                promotion_warning,
            ))
        }
        // Review rejected → bounce back for rework, or park on attempt exhaustion.
        ("approve_review", Some("reject")) => {
            let goal_id = match decision.goal_id.as_deref() {
                Some(g) => g,
                None => return Ok((None, None)),
            };
            // Recognition write-back (SECONDARY proxy): a bounce is a negative
            // outcome for the goal's worker-session recalls, whether it parks or
            // returns for rework.
            permagent::recognition::write_back_decision_outcome(pool, goal_id, false).await;
            let card = permagent::cards::get_card(pool, goal_id)
                .await
                .map_err(GuardError::Db)?
                .ok_or_else(|| GuardError::NotFound(format!("goal '{}' not found", goal_id)))?;
            let attempt_count = card
                .metadata_json
                .get("attempt_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let budget = goal_transition::goal_budget(&card.metadata_json);

            if attempt_count + 1 >= budget.attempt_cap {
                let reason = decision
                    .answer_note
                    .clone()
                    .unwrap_or_else(|| "Rejected after maximum attempts".to_string());
                let decision_id = goal_transition::exhaust_and_park(
                    pool,
                    goal_id,
                    &card.title,
                    &card.project_id,
                    goal_transition::BudgetExhaustion::AttemptCap {
                        spent: attempt_count + 1,
                        cap: budget.attempt_cap,
                    },
                    Some(&reason),
                )
                .await
                .map_err(GuardError::Db)?;
                Ok((
                    Some(format!(
                        "goal rejected at attempt cap: parked with unblock decision {}",
                        decision_id
                    )),
                    None,
                ))
            } else {
                let mut patch = serde_json::Map::new();
                patch.insert(
                    "attempt_count".to_string(),
                    serde_json::json!(attempt_count + 1),
                );
                goal_transition::advance_goal_checked(
                    pool,
                    goal_id,
                    GoalAction::Reject,
                    &acted_by,
                    Some(proof),
                    TransitionEffects {
                        review_notes: decision.answer_note.clone(),
                        metadata_patch: patch,
                        ..Default::default()
                    },
                )
                .await?;
                Ok((
                    Some("goal rejected: Review → InProgress for rework".to_string()),
                    None,
                ))
            }
        }
        // Unblock approved → unpark: clear attention flag, extend the attempt
        // cap, and requeue Triage → Ready.
        ("unblock", Some("approve")) => {
            let goal_id = match decision.goal_id.as_deref() {
                Some(g) => g,
                None => return Ok((None, None)),
            };
            let card = permagent::cards::get_card(pool, goal_id)
                .await
                .map_err(GuardError::Db)?
                .ok_or_else(|| GuardError::NotFound(format!("goal '{}' not found", goal_id)))?;
            let attempt_count = card
                .metadata_json
                .get("attempt_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let mut budget_obj = card
                .metadata_json
                .get("budget")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            budget_obj.insert(
                "attempt_cap".to_string(),
                serde_json::json!(attempt_count + goal_transition::DEFAULT_ATTEMPT_CAP),
            );

            let mut patch = serde_json::Map::new();
            patch.insert(
                "needs_human_attention".to_string(),
                serde_json::json!(false),
            );
            patch.insert("budget".to_string(), serde_json::Value::Object(budget_obj));

            goal_transition::advance_goal_checked(
                pool,
                goal_id,
                GoalAction::Ready,
                &acted_by,
                Some(proof),
                TransitionEffects {
                    metadata_patch: patch,
                    ..Default::default()
                },
            )
            .await?;
            Ok((
                Some(format!(
                    "goal unparked: Triage → Ready with attempt_cap raised to {}",
                    attempt_count + goal_transition::DEFAULT_ATTEMPT_CAP
                )),
                None,
            ))
        }
        // Approved goal deletion (user_data_deletion risk gate).
        ("risk_gate", Some("approve"))
            if decision
                .payload
                .get("action_class")
                .and_then(|v| v.as_str())
                == Some("user_data_deletion")
                && decision.goal_id.is_some() =>
        {
            let goal_id = decision.goal_id.as_deref().unwrap();
            let deleted = goal_transition::delete_goal_checked(pool, goal_id, proof).await?;
            Ok((
                Some(if deleted {
                    format!("goal {} deleted", goal_id)
                } else {
                    format!("goal {} was already gone", goal_id)
                }),
                None,
            ))
        }
        // Declined automation proposal (Initiative → Decision Inbox). Record a
        // recognition bounce keyed on the observed command so the initiative gate
        // prunes it and never re-pitches — the anti-nag guarantee, carried onto
        // the inbox surface. Provenance lives in the decision payload.
        ("automation_proposal", Some("reject")) => {
            let normalized = decision
                .payload
                .get("normalized_command")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if !normalized.is_empty() {
                permagent::recognition::mark_observation_bounced(pool, normalized).await;
            }
            tracing::info!(
                target: "initiative",
                decision_id = %decision.id,
                normalized,
                "automation proposal declined on Decision Inbox — pruned, will not re-pitch"
            );
            Ok((
                Some("automation proposal declined; will not re-pitch".to_string()),
                None,
            ))
        }
        // Approved automation proposal (with or without edits): recorded now;
        // building the saved recipe is the orchestrator's job (not yet enabled),
        // so there is no effect to run yet — parity with today's Triage card,
        // which is likewise unconsumed until the orchestrator turns on. An
        // `edit` accepts the user's revised command (in answer_input); the
        // draft→revision delta is learned via the Learn ingestion in the handler.
        ("automation_proposal", Some("approve")) | ("automation_proposal", Some("edit")) => {
            let with_edits = decision.answer.as_deref() == Some("edit");
            tracing::info!(
                target: "initiative",
                decision_id = %decision.id,
                with_edits,
                "automation proposal approved on Decision Inbox"
            );
            Ok((
                Some(if with_edits {
                    "automation proposal approved with edits".to_string()
                } else {
                    "automation proposal approved".to_string()
                }),
                None,
            ))
        }
        // Approved enrichment proposal (#495 slice 4): write each proposed field
        // to the person's graph entity with Enriched provenance + source URL.
        // Manual-provenance fields are never overwritten — Spectral's store
        // enforces the rule and reports a suppressed write as `false`, counted
        // here as "protected". The allowlist was validated at decision creation;
        // it is re-checked per field (defense in depth) so a hand-crafted row
        // can never write a manual-only field name.
        ("enrichment_proposal", Some("approve")) => {
            let payload: permagent::decisions::EnrichmentProposalPayload =
                serde_json::from_value(decision.payload.clone()).map_err(|e| {
                    GuardError::Invalid(format!("stored enrichment payload unreadable: {}", e))
                })?;
            let brain =
                permagent::agents::platform_extensions::get_global_brain().ok_or_else(|| {
                    GuardError::Invalid(
                        "Brain is not available — cannot write enriched fields".to_string(),
                    )
                })?;
            let entity_id: spectral::core::entity_id::EntityId =
                payload.graph_entity_id.parse().map_err(|e| {
                    GuardError::Invalid(format!("invalid graph_entity_id in payload: {:?}", e))
                })?;
            let (mut applied, mut protected, mut skipped) = (0usize, 0usize, 0usize);
            for f in &payload.fields {
                if !permagent::people::ENRICHABLE_FIELD_NAMES.contains(&f.field_name.as_str()) {
                    skipped += 1;
                    continue;
                }
                let wrote = brain
                    .set_entity_field(
                        entity_id,
                        &f.field_name,
                        &f.value,
                        spectral::ingest::FieldSource::Enriched,
                        Some(&f.source_url),
                    )
                    .await
                    .map_err(|e| {
                        GuardError::Db(format!("set_entity_field({}): {}", f.field_name, e))
                    })?;
                if wrote {
                    applied += 1;
                } else {
                    protected += 1;
                }
            }
            let mut effect = format!(
                "enrichment applied to \"{}\": {} field(s) written with Enriched provenance, \
                 {} protected by manual provenance",
                payload.person_name, applied, protected
            );
            if skipped > 0 {
                effect.push_str(&format!(", {} skipped (not enrichable)", skipped));
            }
            Ok((Some(effect), None))
        }
        // Declined enrichment proposal: recorded; nothing touches the profile.
        ("enrichment_proposal", Some("reject")) => Ok((
            Some("enrichment proposal declined; nothing was written".to_string()),
            None,
        )),
        // Remaining shapes route through L3's resume:auto — `choice` answers
        // and `unblock` answered with input on a PARKED goal make it
        // re-dispatch eligible (Triage → Ready through the guard). Everything
        // else (rejections of unblock/risk_gate, malformed acks, unparked
        // goals) returns Ok(None): recorded; no state change to execute.
        _ => permagent::decision_inbox::policy::resume_answered_decision(pool, decision, proof)
            .await
            .map(|effect| (effect, None)),
    }
}

/// Audit record of an effect failure (the answer itself already succeeded and
/// was audited). The audit write is the last resort on this spine — if it
/// fails too, that failure is at least logged, never discarded.
async fn record_effect_failure(pool: &Pool<Sqlite>, decision: &decisions::Decision, error: &str) {
    tracing::error!(
        "Decision {} answered but effect failed: {}",
        decision.id,
        error
    );
    if let Err(audit_err) =
        decisions::record_effect_outcome(pool, decision, &format!("effect_error: {}", error)).await
    {
        tracing::error!(
            "Decision {} effect failure could not be audit-recorded: {} (original effect error: {})",
            decision.id,
            audit_err,
            error
        );
    }
}

// ── Tool-confirmation bridge ─────────────────────────────────────────────────

/// Pure mapping from a freshly answered `tool_approval` decision to the routing
/// keys, the `PermissionConfirmation` to deliver, and the human-readable effect
/// message: approve→`AllowOnce` (tool runs), reject→`DenyOnce` (tool skipped).
/// No I/O — this is the risk-bearing logic and is unit-tested directly, without
/// an `AppState` or a database.
fn tool_confirmation_from_decision(
    decision: &decisions::Decision,
) -> Result<(String, String, PermissionConfirmation, String), String> {
    let payload: permagent::decisions::ToolApprovalPayload =
        serde_json::from_value(decision.payload.clone())
            .map_err(|e| format!("tool_approval payload unreadable: {}", e))?;

    let (permission, effect_msg) = match decision.answer.as_deref() {
        Some("approve") => (
            Permission::AllowOnce,
            format!("approved — running '{}'", payload.tool_name),
        ),
        Some("reject") => (
            Permission::DenyOnce,
            format!("denied — skipping '{}'", payload.tool_name),
        ),
        other => {
            return Err(format!(
                "tool_approval answered with unsupported answer {:?}",
                other
            ))
        }
    };

    Ok((
        payload.session_id,
        payload.request_id,
        PermissionConfirmation {
            principal_type: PrincipalType::Tool,
            permission,
        },
        effect_msg,
    ))
}

/// What the inbox reports when a tool confirmation reached no live waiter.
/// Nothing ran, and saying so is the effect — the alternative is fabricating
/// "approved — running '<tool>'" over a delivery that went nowhere.
const NO_WAITER_EFFECT: &str = "no live turn was waiting for this approval — nothing was run; \
     the turn may have been cancelled or the daemon restarted";

/// Pure mapping from the delivery outcome to the reported effect: only a
/// delivery a live waiter actually received may claim the tool ran/was
/// skipped; anything else is the honest no-waiter report (an effect FAILURE,
/// so the caller audit-records it). Unit-tested without an `AppState`.
fn delivery_effect(delivered: bool, effect_msg: String) -> Result<String, String> {
    if delivered {
        Ok(effect_msg)
    } else {
        Err(NO_WAITER_EFFECT.to_string())
    }
}

/// Deliver a freshly answered `tool_approval` decision back to the parked agent
/// turn: resolve the session's `Agent` and hand the confirmation to its
/// `ToolConfirmationRouter` via `Agent::handle_confirmation` — the same channel
/// the legacy `/action-required/tool-confirmation` modal uses — so the awaiting
/// tool call unblocks: approve runs the tool, reject skips it, the turn continues.
///
/// Lifecycle: the parked await lives inside a detached, event-bus-decoupled turn
/// task that keeps the reply stream polled until cancel/completion, so it is
/// still alive to receive this delivery even though the answer arrives on a
/// separate request long after the turn parked. If the turn was cancelled, the
/// daemon restarted, or the legacy endpoint answered first, the router reports
/// no waiter — returned as `Err` here so the effect the user sees is the honest
/// "nothing was run", not a success claim.
async fn deliver_tool_confirmation(
    state: &Arc<AppState>,
    decision: &decisions::Decision,
) -> Result<String, String> {
    let (session_id, request_id, confirmation, effect_msg) =
        tool_confirmation_from_decision(decision)?;

    let agent = state
        .get_agent(session_id.clone())
        .await
        .map_err(|e| format!("no agent for session {}: {}", session_id, e))?;

    let delivered = agent.handle_confirmation(request_id, confirmation).await;

    delivery_effect(delivered, effect_msg)
}

// ── Plumbing ────────────────────────────────────────────────────────────────

async fn pool_of(state: &Arc<AppState>) -> Result<Pool<Sqlite>, (StatusCode, String)> {
    state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

fn internal(e: String) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e)
}

// ── Route registration (merged in routes/mod.rs by the coordinator) ────────

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/decisions", get(list_decisions_handler))
        .route("/api/decisions/history", get(history_handler))
        .route("/api/decisions/{id}/answer", post(answer_decision_handler))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GuardError → HTTP status mapping (documented contract; effect errors
    /// surface in `effect_error` with the answer already 200-committed).
    fn status_for_guard_error(err: &GuardError) -> StatusCode {
        match err {
            GuardError::NotFound(_) => StatusCode::NOT_FOUND,
            GuardError::Invalid(_) => StatusCode::BAD_REQUEST,
            GuardError::Denied(_) => StatusCode::FORBIDDEN,
            GuardError::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// (b) Required unit test: Tier-2 answered by anyone but 'jesse' → 403.
    #[tokio::test]
    async fn tier2_answer_by_non_jesse_maps_to_403() {
        use permagent::session::spectral_schema::init_spectral_db;
        let pool = permagent::sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();

        let d = decisions::create_decision(
            &pool,
            decisions::NewDecision {
                kind: "risk_gate".to_string(),
                headline: Some("Permission to push the release".to_string()),
                detail: Some("merge_to_main risk gate".to_string()),
                payload: serde_json::json!({
                    "action_class": "merge_to_main",
                    "description": "publish",
                    "requested_by": "test"
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(d.tier, 2);

        let answer = DecisionAnswer {
            answer: "approve".to_string(),
            ..Default::default()
        };
        let err = decisions::answer_decision(&pool, &d.id, &answer, decisions::ACTOR_HENRY)
            .await
            .unwrap_err();
        assert_eq!(
            status_for_answer_error(&err),
            StatusCode::FORBIDDEN,
            "Tier-2 with acted_by != 'jesse' must map to 403: {:?}",
            err
        );
    }

    #[tokio::test]
    async fn error_status_mapping_is_exhaustive() {
        assert_eq!(
            status_for_answer_error(&AnswerError::NotFound),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status_for_answer_error(&AnswerError::AlreadyResolved("answered".into())),
            StatusCode::CONFLICT
        );
        assert_eq!(
            status_for_answer_error(&AnswerError::Invalid("x".into())),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status_for_answer_error(&AnswerError::Db("x".into())),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            status_for_guard_error(&GuardError::Denied("x".into())),
            StatusCode::FORBIDDEN
        );
    }

    // ── Approve spine: dependent promotion surfaced, never discarded ──

    /// approve_review through `execute_effect`: the approval commits, the
    /// eligible dependent is actually promoted, and the healthy path carries
    /// no warning (bug-sweep wave 1: the promotion Result used to be
    /// `let _ =` — a failure left dependents blocked with no cause). The
    /// failure seam itself is unit-tested in
    /// `goal_transition::promote_or_warn_failure_returns_warning_with_ids`.
    #[tokio::test]
    async fn approve_effect_promotes_dependents_without_warning() {
        use permagent::projects::PERSONAL_PROJECT_ID;
        let pool = memory_pool().await;
        permagent::cards::seed_goal_columns(&pool, PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        let review_col = permagent::cards::get_goal_column(&pool, PERSONAL_PROJECT_ID, "review")
            .await
            .unwrap()
            .unwrap();
        let triage_col = permagent::cards::get_goal_column(&pool, PERSONAL_PROJECT_ID, "triage")
            .await
            .unwrap()
            .unwrap();

        let goal = permagent::cards::create_card(
            &pool,
            permagent::cards::CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "Goal under review".to_string(),
                description: None,
                card_type: Some("goal".to_string()),
                column_id: Some(review_col.id.clone()),
                created_by: None,
                metadata_json: Some(serde_json::json!({})),
            },
        )
        .await
        .unwrap();
        let dependent = permagent::cards::create_card(
            &pool,
            permagent::cards::CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "Dependent goal waiting on the review".to_string(),
                description: None,
                card_type: Some("goal".to_string()),
                column_id: Some(triage_col.id.clone()),
                created_by: None,
                metadata_json: Some(serde_json::json!({"depends_on": [goal.id]})),
            },
        )
        .await
        .unwrap();

        let d = decisions::create_decision(
            &pool,
            decisions::NewDecision {
                kind: "approve_review".to_string(),
                goal_id: Some(goal.id.clone()),
                project_id: Some(PERSONAL_PROJECT_ID.to_string()),
                headline: Some("Approve the finished work on the test goal".to_string()),
                detail: Some("evidence: tests pass".to_string()),
                payload: serde_json::json!({}),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (answered, proof) = decisions::answer_decision(
            &pool,
            &d.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            decisions::ACTOR_JESSE,
        )
        .await
        .unwrap();

        let (effect, warning) = execute_effect(&pool, &answered, proof).await.unwrap();
        assert_eq!(effect.as_deref(), Some("goal approved: Review → Complete"));
        assert!(
            warning.is_none(),
            "healthy path must not warn: {:?}",
            warning
        );

        let dep_card = permagent::cards::get_card(&pool, &dependent.id)
            .await
            .unwrap()
            .unwrap();
        let dep_col = permagent::cards::get_column(&pool, &dep_card.column_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            dep_col.state_binding.as_deref(),
            Some("ready"),
            "the dependent goal must be promoted once its dependency completes"
        );
    }

    // ── Tool-confirmation bridge round-trip (the real trust-mode fix) ──
    //
    // The risk-bearing logic is `tool_confirmation_from_decision` (answer →
    // routing keys + PermissionConfirmation) and its delivery through
    // `Agent::handle_confirmation` — the same channel a parked
    // `handle_approval_tool_requests` await is suspended on. Both are exercised
    // with an in-memory decisions DB + a real Agent (no AppState / no on-disk DB:
    // the AppState session store is a process singleton whose path is not
    // openable under `cargo test` on every platform). Mirrors the sibling tier2
    // test's in-memory pool and the Agent::new()+handle_confirmation tests in
    // crates/goose/src/agents/agent.rs.

    use permagent::agents::Agent;
    use permagent::session::spectral_schema::init_spectral_db;

    async fn memory_pool() -> Pool<Sqlite> {
        let pool = permagent::sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    fn tool_approval_new(
        session_id: &str,
        request_id: &str,
        command: &str,
    ) -> decisions::NewDecision {
        decisions::NewDecision {
            kind: "tool_approval".to_string(),
            headline: Some("Approve tool call: developer__shell".to_string()),
            detail: Some(format!("run {}", command)),
            payload: serde_json::json!({
                "session_id": session_id,
                "request_id": request_id,
                "tool_name": "developer__shell",
                "arguments": {"command": command},
            }),
            ..Default::default()
        }
    }

    /// Create then answer a tool_approval decision, returning the answered row.
    async fn create_and_answer(
        pool: &Pool<Sqlite>,
        session_id: &str,
        request_id: &str,
        command: &str,
        answer: &str,
    ) -> decisions::Decision {
        let d =
            decisions::create_decision(pool, tool_approval_new(session_id, request_id, command))
                .await
                .unwrap();
        assert_ne!(
            d.kind, "malformed",
            "valid tool_approval must not be malformed"
        );
        assert_eq!(d.tier, 2, "tool_approval is human-only (Tier 2)");
        let (answered, _proof) = decisions::answer_decision(
            pool,
            &d.id,
            &DecisionAnswer {
                answer: answer.to_string(),
                ..Default::default()
            },
            decisions::ACTOR_JESSE,
        )
        .await
        .unwrap();
        answered
    }

    #[tokio::test]
    async fn tool_confirmation_from_answered_decision_maps_approve_and_reject() {
        let pool = memory_pool().await;

        let approved = create_and_answer(&pool, "sess-1", "req-1", "ls", "approve").await;
        let (session_id, request_id, confirmation, effect) =
            tool_confirmation_from_decision(&approved).unwrap();
        assert_eq!(session_id, "sess-1");
        assert_eq!(request_id, "req-1");
        assert_eq!(confirmation.permission, Permission::AllowOnce);
        assert!(effect.contains("approved"), "effect: {}", effect);

        let rejected = create_and_answer(&pool, "sess-2", "req-2", "rm -rf /tmp/x", "reject").await;
        let (_s, _r, confirmation, effect) = tool_confirmation_from_decision(&rejected).unwrap();
        assert_eq!(confirmation.permission, Permission::DenyOnce);
        assert!(effect.contains("denied"), "effect: {}", effect);
    }

    /// End-to-end: an answered tool_approval decision, mapped and delivered
    /// through `Agent::handle_confirmation`, unblocks a parked
    /// `ToolConfirmationRouter` await — approve→AllowOnce (tool runs),
    /// reject→DenyOnce (tool skipped) — the exact await
    /// `handle_approval_tool_requests` suspends the turn on.
    #[tokio::test]
    async fn answered_tool_approval_unblocks_parked_agent_await() {
        let pool = memory_pool().await;

        // Approve → AllowOnce (tool runs).
        {
            let agent = Agent::new();
            let rx = agent
                .tool_confirmation_router
                .register("req-approve".to_string())
                .await;
            let answered = create_and_answer(&pool, "s-a", "req-approve", "ls", "approve").await;
            let (_sid, request_id, confirmation, _msg) =
                tool_confirmation_from_decision(&answered).unwrap();
            assert!(
                agent.handle_confirmation(request_id, confirmation).await,
                "a parked waiter must report delivered=true"
            );
            let delivered = tokio::time::timeout(std::time::Duration::from_secs(5), rx)
                .await
                .expect("deliver must unblock the parked await")
                .expect("confirmation sender must not have been dropped");
            assert_eq!(delivered.permission, Permission::AllowOnce);
        }

        // Reject → DenyOnce (tool skipped, turn continues).
        {
            let agent = Agent::new();
            let rx = agent
                .tool_confirmation_router
                .register("req-deny".to_string())
                .await;
            let answered = create_and_answer(&pool, "s-d", "req-deny", "rm", "reject").await;
            let (_sid, request_id, confirmation, _msg) =
                tool_confirmation_from_decision(&answered).unwrap();
            assert!(
                agent.handle_confirmation(request_id, confirmation).await,
                "a parked waiter must report delivered=true"
            );
            let delivered = tokio::time::timeout(std::time::Duration::from_secs(5), rx)
                .await
                .expect("deliver must unblock the parked await")
                .expect("confirmation sender must not have been dropped");
            assert_eq!(delivered.permission, Permission::DenyOnce);
        }
    }

    // ── Delivery honesty (no live waiter ⇒ no success claim) ──

    /// The effect the user reads must track what the delivery actually did:
    /// delivered=true keeps the mapped message; delivered=false becomes the
    /// honest no-waiter report — which must never contain the success claims.
    #[test]
    fn delivery_effect_is_honest_about_no_waiter() {
        assert_eq!(
            delivery_effect(true, "approved — running 'developer__shell'".into()),
            Ok("approved — running 'developer__shell'".to_string())
        );

        let err = delivery_effect(false, "approved — running 'developer__shell'".into())
            .expect_err("no waiter must be an effect failure");
        assert_eq!(err, NO_WAITER_EFFECT);
        assert!(err.contains("nothing was run"));
        assert!(
            !err.contains("running") && !err.contains("approved —"),
            "honest report must not echo the success claim: {}",
            err
        );
    }

    /// End-to-end for the stale case (turn cancelled / daemon restarted /
    /// answered via the legacy prompt first): the answered decision maps fine,
    /// but no waiter is registered — `handle_confirmation` reports false, the
    /// effect becomes the honest no-waiter report, and the failure lands on the
    /// audit chain via the same `record_effect_failure` path goal-effects use
    /// (exactly what `answer_decision_handler` does on the Err branch).
    #[tokio::test]
    async fn undeliverable_tool_approval_reports_honestly_and_audits() {
        let pool = memory_pool().await;
        let agent = Agent::new(); // nothing registered — no live turn

        let answered = create_and_answer(&pool, "s-gone", "req-gone", "ls", "approve").await;
        let (_sid, request_id, confirmation, effect_msg) =
            tool_confirmation_from_decision(&answered).unwrap();

        let delivered = agent.handle_confirmation(request_id, confirmation).await;
        assert!(!delivered, "no registered waiter must report false");

        let effect = delivery_effect(delivered, effect_msg)
            .expect_err("undelivered confirmation must surface as an effect failure");
        record_effect_failure(&pool, &answered, &effect).await;

        // The fabricated success claim is gone; the audit chain records the truth.
        let outcome: String = permagent::sqlx::query_scalar(
            "SELECT outcome FROM decision_audit WHERE decision_id = ? ORDER BY seq DESC LIMIT 1",
        )
        .bind(&answered.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(outcome, format!("effect_error: {}", NO_WAITER_EFFECT));
    }
}
