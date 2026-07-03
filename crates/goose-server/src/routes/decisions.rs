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
    /// Present when the decision was answered but the effect failed; the
    /// failure is also recorded in the audit log.
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
    // an effect failure is reported (and audit-logged), not silently dropped.
    let (effect, effect_error) = match execute_effect(&pool, &decision, proof).await {
        Ok(effect) => (effect, None),
        Err(e) => {
            let msg = e.to_string();
            record_effect_failure(&pool, &decision, &msg).await;
            (None, Some(msg))
        }
    };

    // Learn (L3): jesse-answered decisions become Brain memories. This
    // handler attributes all HTTP answers to 'jesse' (S5), and
    // `ingest_answered_decision` re-checks status/acted_by itself.
    // Failure-tolerant — never breaks the answer path.
    if let Some(brain) = permagent::agents::platform_extensions::get_global_brain() {
        if let Err(e) =
            permagent::decision_inbox::learn::ingest_answered_decision(&pool, &brain, &decision)
                .await
        {
            tracing::warn!(
                "Decision {} learn ingestion failed (non-fatal): {}",
                decision.id,
                e
            );
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
async fn execute_effect(
    pool: &Pool<Sqlite>,
    decision: &decisions::Decision,
    proof: decisions::DecisionProof,
) -> Result<Option<String>, GuardError> {
    let acted_by = proof.acted_by().to_string();
    match (decision.kind.as_str(), decision.answer.as_deref()) {
        // Review approved → goal completes; dependents become eligible.
        ("approve_review", Some("approve")) => {
            let goal_id = match decision.goal_id.as_deref() {
                Some(g) => g,
                None => return Ok(None),
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
            if let Some(ref project_id) = decision.project_id {
                let _ = goal_transition::promote_eligible_dependents(pool, project_id).await;
            }
            // Recognition write-back (SECONDARY proxy): approval is a positive
            // outcome. 2-hop join goal_id → worker_session_id → recognition events.
            permagent::recognition::write_back_decision_outcome(pool, goal_id, true).await;
            Ok(Some("goal approved: Review → Complete".to_string()))
        }
        // Review rejected → bounce back for rework, or park on attempt exhaustion.
        ("approve_review", Some("reject")) => {
            let goal_id = match decision.goal_id.as_deref() {
                Some(g) => g,
                None => return Ok(None),
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
                Ok(Some(format!(
                    "goal rejected at attempt cap: parked with unblock decision {}",
                    decision_id
                )))
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
                Ok(Some(
                    "goal rejected: Review → InProgress for rework".to_string(),
                ))
            }
        }
        // Unblock approved → unpark: clear attention flag, extend the attempt
        // cap, and requeue Triage → Ready.
        ("unblock", Some("approve")) => {
            let goal_id = match decision.goal_id.as_deref() {
                Some(g) => g,
                None => return Ok(None),
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
            Ok(Some(format!(
                "goal unparked: Triage → Ready with attempt_cap raised to {}",
                attempt_count + goal_transition::DEFAULT_ATTEMPT_CAP
            )))
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
            Ok(Some(if deleted {
                format!("goal {} deleted", goal_id)
            } else {
                format!("goal {} was already gone", goal_id)
            }))
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
            Ok(Some(
                "automation proposal declined; will not re-pitch".to_string(),
            ))
        }
        // Approved automation proposal: recorded now; building the saved recipe
        // is the orchestrator's job (not yet enabled), so there is no effect to
        // run yet — parity with today's Triage card, which is likewise unconsumed
        // until the orchestrator turns on.
        ("automation_proposal", Some("approve")) => {
            tracing::info!(
                target: "initiative",
                decision_id = %decision.id,
                "automation proposal approved on Decision Inbox"
            );
            Ok(Some("automation proposal approved".to_string()))
        }
        // Remaining shapes route through L3's resume:auto — `choice` answers
        // and `unblock` answered with input on a PARKED goal make it
        // re-dispatch eligible (Triage → Ready through the guard). Everything
        // else (rejections of unblock/risk_gate, malformed acks, unparked
        // goals) returns Ok(None): recorded; no state change to execute.
        _ => {
            permagent::decision_inbox::policy::resume_answered_decision(pool, decision, proof).await
        }
    }
}

/// Best-effort audit record of an effect failure (the answer itself already
/// succeeded and was audited).
async fn record_effect_failure(pool: &Pool<Sqlite>, decision: &decisions::Decision, error: &str) {
    tracing::error!(
        "Decision {} answered but effect failed: {}",
        decision.id,
        error
    );
    let _ =
        decisions::record_effect_outcome(pool, decision, &format!("effect_error: {}", error)).await;
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
}
