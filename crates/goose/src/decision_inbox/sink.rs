//! Part B: the real [`DecisionSink`] — escalate drafts persist as rows in
//! Lane L1's `decisions` table via [`crate::decisions::create_decision`].
//!
//! Mapping invariants:
//! - Valid drafts map to L1's typed payloads (schema-validated on write by
//!   L1, `deny_unknown_fields`); the tier is computed by L1's risk_policy
//!   lookup (fail-closed Tier 2 for unseeded classes).
//! - Malformed drafts become `kind='malformed'` rows (Tier 2) carrying the
//!   raw payload and the validation error list — never coerced, never dropped.
//! - Headline/detail are REQUIRED and flow through L1's validation
//!   (Amendment A1); evidence refs, attribution, and option consequences are
//!   appended to `detail` (typed payloads deny unknown fields).
//! - A goal binding is recovered from `evidence_refs` entries of the form
//!   `card:<id>` / `goal:<id>`; the goal's project becomes the decision's
//!   project (enables resume:auto and curation fan-out scoring).

use sqlx::{Pool, Sqlite};

use super::escalate::{DecisionDraft, DecisionKind, DecisionSink, RecordedDecision};
use crate::decisions::{self, NewDecision};

/// Unseeded risk_policy class for worker capability requests — resolves
/// fail-closed to Tier 2, which is exactly the Phase 0 floor for
/// `capability` escalations.
pub const CAPABILITY_ACTION_CLASS: &str = "worker_capability_request";

/// SQL-backed decision sink (Part B). Installed at daemon startup via
/// [`super::escalate::install_decision_sink`].
pub struct SqlDecisionSink {
    pool: Pool<Sqlite>,
}

impl SqlDecisionSink {
    pub fn new(pool: Pool<Sqlite>) -> Self {
        Self { pool }
    }
}

/// Extract a goal-card binding from opaque evidence refs (`card:<id>` or
/// `goal:<id>`). First match wins; existence is verified by the caller.
pub fn goal_ref_from_evidence(evidence_refs: &[String]) -> Option<String> {
    evidence_refs.iter().find_map(|r| {
        r.strip_prefix("card:")
            .or_else(|| r.strip_prefix("goal:"))
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
    })
}

/// Compose the decision `detail` from the draft: technical why-blocked text
/// plus attribution, evidence refs, and (for choices) option consequences.
/// All of it is DATA (S2) — stored verbatim, never rendered as markdown.
fn compose_detail(draft: &DecisionDraft) -> String {
    let mut detail = draft.detail.clone();
    detail.push_str(&format!("\n\nRequested by: {}", draft.requested_by_display));
    if let Some(ref sid) = draft.source_session_id {
        detail.push_str(&format!(" (session {})", sid));
    }
    if !draft.evidence_refs.is_empty() {
        detail.push_str(&format!("\nEvidence: {}", draft.evidence_refs.join("; ")));
    }
    if draft.kind == DecisionKind::Choice && !draft.options.is_empty() {
        detail.push_str("\nOptions:");
        for o in &draft.options {
            detail.push_str(&format!("\n- {}: {} — {}", o.id, o.label, o.consequence));
        }
    }
    detail
}

/// Build the typed payload for a valid draft, in L1's schema vocabulary.
fn payload_for(draft: &DecisionDraft) -> serde_json::Value {
    match draft.kind {
        DecisionKind::ApproveReview => serde_json::json!({}),
        DecisionKind::Choice => {
            let options: Vec<serde_json::Value> = draft
                .options
                .iter()
                .map(|o| serde_json::json!({"id": o.id, "label": o.label}))
                .collect();
            // Consequences live in detail; L1's ChoiceOption is {id, label}.
            serde_json::json!({"question": draft.headline, "options": options})
        }
        DecisionKind::Unblock => serde_json::json!({"reason": "stuck"}),
        DecisionKind::RiskGate => serde_json::json!({
            "action_class": CAPABILITY_ACTION_CLASS,
            "description": draft.headline,
            "requested_by": draft.requested_by_display,
        }),
        // Malformed never reaches payload_for (see record()).
        DecisionKind::Malformed => serde_json::Value::Null,
    }
}

#[async_trait::async_trait]
impl DecisionSink for SqlDecisionSink {
    async fn record(&self, draft: DecisionDraft) -> anyhow::Result<RecordedDecision> {
        // Goal binding from evidence refs; the goal's project scopes the
        // decision. create_decision re-verifies card existence itself.
        let goal_id = goal_ref_from_evidence(&draft.evidence_refs);
        let project_id = match goal_id.as_deref() {
            Some(gid) => crate::cards::get_card(&self.pool, gid)
                .await
                .map_err(anyhow::Error::msg)?
                .map(|c| c.project_id),
            None => None,
        };

        let req = match draft.kind {
            DecisionKind::Malformed => NewDecision {
                // 'malformed' is not a creatable kind in L1's vocabulary, so
                // validation fails and L1 stores the fail-closed Tier-2
                // malformed row — raw payload and error list preserved.
                kind: "malformed".to_string(),
                goal_id,
                project_id,
                headline: Some(draft.headline.clone()),
                detail: Some(draft.detail.clone()),
                payload: serde_json::json!({
                    "escalate_raw": draft.raw_payload,
                    "validation_errors": draft.validation_errors,
                }),
                ..Default::default()
            },
            _ => NewDecision {
                kind: draft.kind.as_str().to_string(),
                goal_id,
                project_id,
                headline: Some(draft.headline.clone()),
                detail: Some(compose_detail(&draft)),
                payload: payload_for(&draft),
                ..Default::default()
            },
        };

        let decision = decisions::create_decision(&self.pool, req)
            .await
            .map_err(anyhow::Error::msg)?;

        tracing::info!(
            target: "permagentd::decisions",
            decision_id = %decision.id,
            kind = %decision.kind,
            tier = decision.tier,
            "Escalation persisted as decision row"
        );

        Ok(RecordedDecision {
            decision_id: decision.id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision_inbox::escalate::{draft_from_payload, DraftSource};
    use crate::projects::PERSONAL_PROJECT_ID;
    use serde_json::json;

    async fn test_pool() -> Pool<Sqlite> {
        use crate::session::spectral_schema::init_spectral_db;
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    fn source() -> DraftSource {
        DraftSource {
            session_id: Some("sess-1".to_string()),
            worker_roster_name: Some("codex".to_string()),
        }
    }

    async fn goal_card(pool: &Pool<Sqlite>, state: &str) -> crate::cards::Card {
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
                title: "Sink test goal".to_string(),
                description: None,
                card_type: Some("goal".to_string()),
                column_id: Some(col.id.clone()),
                created_by: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap()
    }

    #[test]
    fn goal_ref_extraction_table() {
        let cases: Vec<(Vec<String>, Option<&str>)> = vec![
            (vec!["card:abc".into()], Some("abc")),
            (vec!["goal:xyz".into()], Some("xyz")),
            (vec!["session:1".into(), "card:abc".into()], Some("abc")),
            (vec!["card:".into()], None),
            (vec!["url:https://x".into()], None),
            (vec![], None),
        ];
        for (refs, expected) in cases {
            assert_eq!(
                goal_ref_from_evidence(&refs).as_deref(),
                expected,
                "refs {:?}",
                refs
            );
        }
    }

    #[tokio::test]
    async fn valid_approval_draft_persists_as_approve_review_row() {
        let pool = test_pool().await;
        let goal = goal_card(&pool, "review").await;
        let draft = draft_from_payload(
            json!({
                "kind": "approval",
                "specific_ask": "Approve the login page redesign so it can ship",
                "why_blocked": "Verifier passed; awaiting sign-off.",
                "evidence_refs": [format!("card:{}", goal.id)],
                "resume": "auto"
            }),
            &source(),
        );
        let sink = SqlDecisionSink::new(pool.clone());
        let recorded = sink.record(draft).await.unwrap();

        let d = decisions::get_decision(&pool, &recorded.decision_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(d.kind, "approve_review");
        assert_eq!(d.tier, 1, "goal_approve_standard seeds at tier 1");
        assert_eq!(d.status, "open");
        assert_eq!(d.goal_id.as_deref(), Some(goal.id.as_str()));
        assert_eq!(d.project_id.as_deref(), Some(PERSONAL_PROJECT_ID));
        assert_eq!(d.headline, "Approve the login page redesign so it can ship");
        assert!(d.detail.contains("Requested by: codex"));
        assert!(d.detail.contains("session sess-1"));
        assert!(d.detail.contains(&format!("card:{}", goal.id)));
    }

    #[tokio::test]
    async fn choice_draft_persists_options_that_validate_answers() {
        let pool = test_pool().await;
        let draft = draft_from_payload(
            json!({
                "kind": "decision",
                "specific_ask": "Choose where new user data should be stored",
                "why_blocked": "Two viable storage backends.",
                "options": [
                    {"id": "sqlite", "label": "Local SQLite", "consequence": "single machine"},
                    {"id": "postgres", "label": "Hosted Postgres", "consequence": "ops burden"}
                ],
                "resume": "auto"
            }),
            &source(),
        );
        let sink = SqlDecisionSink::new(pool.clone());
        let recorded = sink.record(draft).await.unwrap();
        let d = decisions::get_decision(&pool, &recorded.decision_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(d.kind, "choice");
        // Consequences flow into detail; payload carries L1's {id,label} shape.
        assert!(d.detail.contains("sqlite: Local SQLite — single machine"));

        // The persisted options gate the answer path.
        let bad = decisions::DecisionAnswer {
            answer: "choice".to_string(),
            choice_id: Some("mongodb".to_string()),
            ..Default::default()
        };
        let err = decisions::answer_decision(&pool, &d.id, &bad, decisions::ACTOR_JESSE)
            .await
            .unwrap_err();
        assert!(matches!(err, decisions::AnswerError::Invalid(_)));

        let good = decisions::DecisionAnswer {
            answer: "choice".to_string(),
            choice_id: Some("postgres".to_string()),
            ..Default::default()
        };
        let (answered, _proof) =
            decisions::answer_decision(&pool, &d.id, &good, decisions::ACTOR_JESSE)
                .await
                .unwrap();
        assert_eq!(answered.answer_choice_id.as_deref(), Some("postgres"));
    }

    #[tokio::test]
    async fn unblock_and_risk_gate_drafts_get_expected_tiers() {
        let pool = test_pool().await;
        let sink = SqlDecisionSink::new(pool.clone());

        let credential = draft_from_payload(
            json!({
                "kind": "credential",
                "specific_ask": "Add the staging database password",
                "why_blocked": "Deploy step needs it.",
                "resume": "auto"
            }),
            &source(),
        );
        let r = sink.record(credential).await.unwrap();
        let d = decisions::get_decision(&pool, &r.decision_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(d.kind, "unblock");
        assert_eq!(d.tier, 1, "unblock maps to goal_retry_within_budget");

        let capability = draft_from_payload(
            json!({
                "kind": "capability",
                "specific_ask": "Allow workers to send outbound email",
                "why_blocked": "Notification goal needs SMTP.",
                "resume": "auto"
            }),
            &source(),
        );
        let r = sink.record(capability).await.unwrap();
        let d = decisions::get_decision(&pool, &r.decision_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(d.kind, "risk_gate");
        assert_eq!(
            d.tier, 2,
            "unseeded capability class fails closed to Tier 2"
        );
    }

    #[tokio::test]
    async fn malformed_draft_persists_as_malformed_row_with_errors() {
        let pool = test_pool().await;
        let raw = json!({"kind": "vibes", "specific_ask": "??"});
        let draft = draft_from_payload(raw.clone(), &source());
        let sink = SqlDecisionSink::new(pool.clone());
        let recorded = sink.record(draft).await.unwrap();

        let d = decisions::get_decision(&pool, &recorded.decision_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(d.kind, "malformed");
        assert_eq!(d.tier, 2, "malformed rows are Tier-2 fail-closed");
        // The raw escalate payload and our validation errors survive inside
        // L1's malformed envelope.
        let raw_stored = d.payload.get("raw").expect("L1 keeps the raw payload");
        assert_eq!(raw_stored.get("escalate_raw"), Some(&raw));
        assert!(raw_stored
            .get("validation_errors")
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty()));
        // Headline stays the rule-clean draft headline (mentions the worker).
        assert!(d.headline.contains("codex"));
    }
}
