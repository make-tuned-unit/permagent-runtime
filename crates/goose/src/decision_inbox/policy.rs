//! Henry's deterministic decision policy (Part B).
//!
//! Two flows, both consuming Lane L1's decisions module — never bypassing it:
//!
//! 1. [`henry_approve_on_verifier_pass`] — on a verifier-pass verdict, record
//!    an approve-with-rationale through L1's answer path as `henry-policy`.
//!    The tier gate lives in [`crate::decisions::answer_decision`]: a Tier-2
//!    decision answered by henry-policy is rejected there (Forbidden / HTTP
//!    403) and the goal does not move — tested end-to-end below.
//!
//! 2. [`resume_answered_decision`] — resume:auto for the answer shapes L1's
//!    route-level `execute_effect` does NOT cover: `choice` answers and
//!    `unblock` answered with `input` on a parked goal. The goal becomes
//!    re-dispatch-eligible (Triage → Ready through the guard, attention flag
//!    cleared, attempt cap extended) — dispatching remains the orchestrator's
//!    heartbeat job, same as L1's own unblock-approve effect.
//!
//! Everything here is deterministic Rust. Zero cloud tokens.

use sqlx::{Pool, Sqlite};

use crate::decisions::{self, AnswerError, Decision, DecisionAnswer, DecisionProof};
use crate::goal_state::GoalAction;
use crate::goal_transition::{self, GuardError, TransitionEffects};

/// Outcome of a Henry policy approval: the answered decision plus what the
/// gated effect did (or why it failed — the answer itself is already
/// committed and audited, mirroring L1's route contract).
#[derive(Debug)]
pub struct HenryApproval {
    pub decision: Decision,
    pub effect: Option<String>,
    pub effect_error: Option<String>,
}

/// Tier-1 auto-approval on verifier pass, acted_by `henry-policy`.
///
/// Calls L1's [`decisions::answer_decision`] — the tier gate there is the
/// enforcement: Tier-2 decisions return [`AnswerError::Forbidden`] and
/// nothing else happens. On success the minted [`DecisionProof`] drives the
/// Approve transition through the goal-transition guard.
pub async fn henry_approve_on_verifier_pass(
    pool: &Pool<Sqlite>,
    decision_id: &str,
    rationale: &str,
) -> Result<HenryApproval, AnswerError> {
    let answer = DecisionAnswer {
        answer: "approve".to_string(),
        note: Some(rationale.to_string()),
        ..Default::default()
    };
    let (decision, proof) =
        decisions::answer_decision(pool, decision_id, &answer, decisions::ACTOR_HENRY).await?;

    let (effect, effect_error) = match henry_approve_effect(pool, &decision, proof).await {
        Ok((effect, warning)) => {
            if let Some(ref w) = warning {
                record_effect_error(pool, &decision, w).await;
            }
            (effect, warning)
        }
        Err(e) => {
            let msg = e.to_string();
            record_effect_error(pool, &decision, &msg).await;
            (None, Some(msg))
        }
    };

    Ok(HenryApproval {
        decision,
        effect,
        effect_error,
    })
}

/// Audit-record an effect failure/warning. Last resort on this spine: if the
/// audit write itself fails, that is logged rather than discarded.
async fn record_effect_error(pool: &Pool<Sqlite>, decision: &Decision, error: &str) {
    if let Err(audit_err) =
        decisions::record_effect_outcome(pool, decision, &format!("effect_error: {}", error)).await
    {
        tracing::error!(
            decision_id = %decision.id,
            "henry-policy effect error could not be audit-recorded: {} (original error: {})",
            audit_err,
            error
        );
    }
}

/// The gated effect of a Henry-approved `approve_review`: Review → Complete
/// through the guard, then dependent promotion. Other kinds record only.
///
/// Returns `(effect, warning)` — the warning carries a dependent-promotion
/// failure that happened AFTER the approval committed; it surfaces in
/// [`HenryApproval::effect_error`] and the audit log instead of being
/// discarded (bug-sweep wave 1).
async fn henry_approve_effect(
    pool: &Pool<Sqlite>,
    decision: &Decision,
    proof: DecisionProof,
) -> Result<(Option<String>, Option<String>), GuardError> {
    if decision.kind != "approve_review" {
        return Ok((None, None));
    }
    let goal_id = match decision.goal_id.as_deref() {
        Some(g) => g,
        None => return Ok((None, None)),
    };
    goal_transition::advance_goal_checked(
        pool,
        goal_id,
        GoalAction::Approve,
        decisions::ACTOR_HENRY,
        Some(proof),
        TransitionEffects {
            review_notes: decision.answer_note.clone(),
            ..Default::default()
        },
    )
    .await?;
    let warning = match decision.project_id.as_deref() {
        Some(project_id) => {
            goal_transition::promote_eligible_dependents_or_warn(pool, project_id, goal_id).await
        }
        None => None,
    };
    Ok((
        Some("goal approved: Review → Complete (henry-policy, verifier pass)".to_string()),
        warning,
    ))
}

/// resume:auto for answer shapes L1's `execute_effect` does not cover:
/// `("choice", "choice")` and `("unblock", "input")` on a PARKED goal.
///
/// Re-dispatch eligibility = Triage → Ready through the guard with
/// `needs_human_attention` cleared and the attempt cap extended (same shape
/// as L1's unblock-approve effect). Returns `Ok(None)` when the decision is
/// not one of those shapes, has no goal, or the goal is not parked.
///
/// Call site: the catch-all arm of L1's `execute_effect`
/// (crates/goose-server/src/routes/decisions.rs) — coordinator inserts the
/// one-line call; this module never edits L1's handlers.
pub async fn resume_answered_decision(
    pool: &Pool<Sqlite>,
    decision: &Decision,
    proof: DecisionProof,
) -> Result<Option<String>, GuardError> {
    let resumable = matches!(
        (decision.kind.as_str(), decision.answer.as_deref()),
        ("choice", Some("choice")) | ("unblock", Some("input"))
    );
    if !resumable {
        return Ok(None);
    }
    let goal_id = match decision.goal_id.as_deref() {
        Some(g) => g,
        None => return Ok(None),
    };

    let card = crate::cards::get_card(pool, goal_id)
        .await
        .map_err(GuardError::Db)?
        .ok_or_else(|| GuardError::NotFound(format!("goal '{}' not found", goal_id)))?;

    let parked = card
        .metadata_json
        .get("needs_human_attention")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !parked {
        return Ok(None);
    }

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

    let actor = proof.acted_by().to_string();
    goal_transition::advance_goal_checked(
        pool,
        goal_id,
        GoalAction::Ready,
        &actor,
        Some(proof),
        TransitionEffects {
            metadata_patch: patch,
            ..Default::default()
        },
    )
    .await?;

    Ok(Some(format!(
        "goal resumed: Triage → Ready after answered {} decision (re-dispatch eligible)",
        decision.kind
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
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

    async fn goal_in_state(
        pool: &Pool<Sqlite>,
        state: &str,
        metadata: serde_json::Value,
    ) -> crate::cards::Card {
        crate::cards::seed_goal_columns(pool, PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        let col = crate::cards::get_goal_column(pool, PERSONAL_PROJECT_ID, state)
            .await
            .unwrap()
            .unwrap();
        crate::cards::create_card(
            pool,
            crate::cards::CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: format!("Policy test goal in {}", state),
                description: None,
                card_type: Some("goal".to_string()),
                column_id: Some(col.id.clone()),
                created_by: None,
                metadata_json: Some(metadata),
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

    /// Required Part B test: a Tier-2 decision approved via Henry's flow is
    /// rejected END-TO-END — Forbidden from L1's answer path, decision stays
    /// open, goal does not move.
    #[tokio::test]
    async fn tier2_via_henry_flow_is_rejected_end_to_end() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "review", serde_json::json!({})).await;

        let d = decisions::create_decision(
            &pool,
            decisions::NewDecision {
                kind: "risk_gate".to_string(),
                goal_id: Some(goal.id.clone()),
                project_id: Some(PERSONAL_PROJECT_ID.to_string()),
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

        let err = henry_approve_on_verifier_pass(&pool, &d.id, "verifier passed")
            .await
            .unwrap_err();
        assert!(
            matches!(err, AnswerError::Forbidden(_)),
            "Tier-2 via henry-policy must be Forbidden: {:?}",
            err
        );

        // Decision untouched, goal unmoved — the rejection happened before
        // any state change.
        let d = decisions::get_decision(&pool, &d.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(d.status, "open");
        assert!(d.acted_by.is_none());
        assert_eq!(state_of(&pool, &goal.id).await, "review");
    }

    #[tokio::test]
    async fn tier1_verifier_pass_approves_and_completes_goal() {
        let pool = test_pool().await;
        let goal = goal_in_state(&pool, "review", serde_json::json!({})).await;

        let d = decisions::create_decision(
            &pool,
            decisions::NewDecision {
                kind: "approve_review".to_string(),
                goal_id: Some(goal.id.clone()),
                project_id: Some(PERSONAL_PROJECT_ID.to_string()),
                headline: Some("Approve the finished work on the test goal".to_string()),
                detail: Some("verifier evidence".to_string()),
                payload: serde_json::json!({}),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(d.tier, 1);

        let approval =
            henry_approve_on_verifier_pass(&pool, &d.id, "verifier passed: all checks green")
                .await
                .unwrap();
        assert_eq!(approval.decision.status, "answered");
        assert_eq!(
            approval.decision.acted_by.as_deref(),
            Some(decisions::ACTOR_HENRY)
        );
        assert_eq!(
            approval.decision.answer_note.as_deref(),
            Some("verifier passed: all checks green"),
            "rationale is recorded with the answer"
        );
        assert!(approval.effect_error.is_none());
        assert!(approval.effect.is_some());
        assert_eq!(state_of(&pool, &goal.id).await, "complete");
    }

    #[tokio::test]
    async fn answered_choice_resumes_parked_goal() {
        let pool = test_pool().await;
        // Parked goal: triage + needs_human_attention + exhausted attempts.
        let goal = goal_in_state(
            &pool,
            "triage",
            serde_json::json!({
                "needs_human_attention": true,
                "attempt_count": 3,
                "last_error": "blocked on a choice",
            }),
        )
        .await;

        let d = decisions::create_decision(
            &pool,
            decisions::NewDecision {
                kind: "choice".to_string(),
                goal_id: Some(goal.id.clone()),
                project_id: Some(PERSONAL_PROJECT_ID.to_string()),
                headline: Some("Choose which storage backend to build on".to_string()),
                detail: Some("two viable backends".to_string()),
                payload: serde_json::json!({
                    "question": "Which backend?",
                    "options": [
                        {"id": "sqlite", "label": "SQLite"},
                        {"id": "postgres", "label": "Postgres"}
                    ]
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let (answered, proof) = decisions::answer_decision(
            &pool,
            &d.id,
            &DecisionAnswer {
                answer: "choice".to_string(),
                choice_id: Some("postgres".to_string()),
                note: Some("scale matters here".to_string()),
                ..Default::default()
            },
            decisions::ACTOR_JESSE,
        )
        .await
        .unwrap();

        let effect = resume_answered_decision(&pool, &answered, proof)
            .await
            .unwrap();
        assert!(effect.is_some(), "parked goal must resume");
        assert_eq!(state_of(&pool, &goal.id).await, "ready");

        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            card.metadata_json.get("needs_human_attention"),
            Some(&serde_json::Value::Bool(false))
        );
        // Attempt cap extended so dispatch doesn't immediately re-exhaust.
        assert_eq!(
            card.metadata_json
                .get("budget")
                .and_then(|b| b.get("attempt_cap"))
                .and_then(|v| v.as_u64()),
            Some(3 + goal_transition::DEFAULT_ATTEMPT_CAP)
        );
    }

    #[tokio::test]
    async fn resume_skips_unparked_goals_and_non_resumable_answers() {
        let pool = test_pool().await;

        // Choice answered for a goal that is NOT parked → no-op.
        let goal = goal_in_state(&pool, "in_progress", serde_json::json!({})).await;
        let d = decisions::create_decision(
            &pool,
            decisions::NewDecision {
                kind: "choice".to_string(),
                goal_id: Some(goal.id.clone()),
                project_id: Some(PERSONAL_PROJECT_ID.to_string()),
                headline: Some("Choose a direction for the running goal".to_string()),
                detail: Some("context".to_string()),
                payload: serde_json::json!({
                    "question": "Which?",
                    "options": [
                        {"id": "a", "label": "A"},
                        {"id": "b", "label": "B"}
                    ]
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (answered, proof) = decisions::answer_decision(
            &pool,
            &d.id,
            &DecisionAnswer {
                answer: "choice".to_string(),
                choice_id: Some("a".to_string()),
                ..Default::default()
            },
            decisions::ACTOR_JESSE,
        )
        .await
        .unwrap();
        let effect = resume_answered_decision(&pool, &answered, proof)
            .await
            .unwrap();
        assert!(effect.is_none(), "unparked goals are not touched");
        assert_eq!(state_of(&pool, &goal.id).await, "in_progress");

        // Rejected unblock → stays parked (rejection is not a resume).
        let parked = goal_in_state(
            &pool,
            "triage",
            serde_json::json!({"needs_human_attention": true}),
        )
        .await;
        let d = decisions::create_decision(
            &pool,
            decisions::NewDecision {
                kind: "unblock".to_string(),
                goal_id: Some(parked.id.clone()),
                project_id: Some(PERSONAL_PROJECT_ID.to_string()),
                headline: Some("A goal is stuck and needs your direction".to_string()),
                detail: Some("stuck".to_string()),
                payload: serde_json::json!({"reason": "stuck"}),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (answered, proof) = decisions::answer_decision(
            &pool,
            &d.id,
            &DecisionAnswer {
                answer: "reject".to_string(),
                ..Default::default()
            },
            decisions::ACTOR_JESSE,
        )
        .await
        .unwrap();
        let effect = resume_answered_decision(&pool, &answered, proof)
            .await
            .unwrap();
        assert!(effect.is_none());
        assert_eq!(state_of(&pool, &parked.id).await, "triage");
    }
}
