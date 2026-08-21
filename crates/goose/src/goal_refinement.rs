//! Bounded autonomous refinement after completion-check failure.
//!
//! Distinct from [`crate::goal_transition::goal_budget`] (attempt/token/wallclock).
//! A goal may declare `refinement_budget` (metadata) as a sub-cap on auto-rework
//! after worker success but failed mechanical checks. Within budget the goal
//! returns to Ready with check stdout in `last_error` / `last_check_output`.
//! At exhaustion it parks with an `unblock` decision.

use serde_json::{Value, json};

use crate::decisions;
use crate::goal_transition::{self, BudgetExhaustion};

/// Metadata key: max auto-rework rounds after a failing completion check.
pub const REFINEMENT_BUDGET_KEY: &str = "refinement_budget";
/// Metadata key: how many refinement rounds have been spent.
pub const REFINEMENT_SPENT_KEY: &str = "refinement_spent";
/// Metadata key: last failing check stdout injected into the next brief.
pub const LAST_CHECK_OUTPUT_KEY: &str = "last_check_output";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefinementDecision {
    /// No budget declared (or zero) — caller keeps the existing Fail path.
    Skip,
    /// Still within budget: return to Ready with check output in the brief.
    Requeue { spent: u64, budget: u64 },
    /// Budget exhausted: park with unblock.
    Park { spent: u64, budget: u64 },
}

/// Read the declared refinement budget. Absent or 0 ⇒ no autonomous rework.
pub fn refinement_budget(metadata: &Value) -> u64 {
    metadata
        .get(REFINEMENT_BUDGET_KEY)
        .and_then(|v| v.as_u64())
        .or_else(|| {
            metadata
                .get("budget")
                .and_then(|b| b.get(REFINEMENT_BUDGET_KEY))
                .and_then(|v| v.as_u64())
        })
        .unwrap_or(0)
}

pub fn refinement_spent(metadata: &Value) -> u64 {
    metadata
        .get(REFINEMENT_SPENT_KEY)
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
}

/// Decide the next step given current metadata and a failing-check transcript.
pub fn decide(metadata: &Value, _check_output: &str) -> RefinementDecision {
    let budget = refinement_budget(metadata);
    if budget == 0 {
        return RefinementDecision::Skip;
    }
    let next = refinement_spent(metadata).saturating_add(1);
    if next <= budget {
        RefinementDecision::Requeue {
            spent: next,
            budget,
        }
    } else {
        RefinementDecision::Park {
            spent: next,
            budget,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    Skipped,
    Requeued {
        spent: u64,
        budget: u64,
    },
    Parked {
        spent: u64,
        budget: u64,
        decision_id: String,
    },
}

/// Apply [`decide`] against a live goal card (Review or InProgress).
pub async fn apply(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    card_id: &str,
    project_id: &str,
    goal_title: &str,
    metadata: &Value,
    check_output: &str,
) -> Result<Applied, String> {
    match decide(metadata, check_output) {
        RefinementDecision::Skip => Ok(Applied::Skipped),
        RefinementDecision::Requeue { spent, budget } => {
            let mut extra = serde_json::Map::new();
            extra.insert(REFINEMENT_SPENT_KEY.to_string(), json!(spent));
            extra.insert(
                LAST_CHECK_OUTPUT_KEY.to_string(),
                json!(truncate(check_output, 8_000)),
            );
            goal_transition::return_to_ready(
                pool,
                card_id,
                decisions::ACTOR_SYSTEM,
                &format!("completion checks failed (refinement {spent}/{budget})"),
                None,
                extra,
            )
            .await
            .map_err(|e| e.to_string())?;
            Ok(Applied::Requeued { spent, budget })
        }
        RefinementDecision::Park { spent, budget } => {
            let decision_id = goal_transition::exhaust_and_park(
                pool,
                card_id,
                goal_title,
                project_id,
                BudgetExhaustion::RefinementBudget { spent, cap: budget },
                Some(check_output),
            )
            .await?;
            Ok(Applied::Parked {
                spent,
                budget,
                decision_id,
            })
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards;
    use crate::projects::PERSONAL_PROJECT_ID;
    use crate::session::spectral_schema::init_spectral_db;
    use serde_json::json;

    #[test]
    fn decide_skip_when_no_budget() {
        assert_eq!(decide(&json!({}), "fail"), RefinementDecision::Skip);
        assert_eq!(
            decide(&json!({REFINEMENT_BUDGET_KEY: 0}), "fail"),
            RefinementDecision::Skip
        );
    }

    #[test]
    fn decide_requeue_within_budget() {
        let meta = json!({REFINEMENT_BUDGET_KEY: 2, REFINEMENT_SPENT_KEY: 0});
        assert_eq!(
            decide(&meta, "boom"),
            RefinementDecision::Requeue {
                spent: 1,
                budget: 2
            }
        );
        let meta = json!({REFINEMENT_BUDGET_KEY: 2, REFINEMENT_SPENT_KEY: 1});
        assert_eq!(
            decide(&meta, "boom"),
            RefinementDecision::Requeue {
                spent: 2,
                budget: 2
            }
        );
    }

    #[test]
    fn decide_park_at_cap() {
        let meta = json!({REFINEMENT_BUDGET_KEY: 1, REFINEMENT_SPENT_KEY: 1});
        assert_eq!(
            decide(&meta, "still failing"),
            RefinementDecision::Park {
                spent: 2,
                budget: 1
            }
        );
    }

    async fn test_pool() -> sqlx::Pool<sqlx::Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    async fn goal_in_review(pool: &sqlx::Pool<sqlx::Sqlite>, extra: Value) -> cards::Card {
        cards::seed_goal_columns(pool, PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        let col = cards::get_goal_column(pool, PERSONAL_PROJECT_ID, "review")
            .await
            .unwrap()
            .unwrap();
        let mut meta = extra.as_object().cloned().unwrap_or_default();
        meta.insert("goal_state".into(), json!("review"));
        meta.insert("attempt_count".into(), json!(1));
        cards::create_card(
            pool,
            cards::CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "refine me".into(),
                description: Some("test".into()),
                card_type: Some("goal".into()),
                column_id: Some(col.id),
                created_by: None,
                metadata_json: Some(Value::Object(meta)),
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn apply_requeues_within_budget() {
        let pool = test_pool().await;
        let card = goal_in_review(&pool, json!({REFINEMENT_BUDGET_KEY: 2})).await;
        let applied = apply(
            &pool,
            &card.id,
            PERSONAL_PROJECT_ID,
            &card.title,
            &card.metadata_json,
            "cargo test failed\nassertion exploded",
        )
        .await
        .unwrap();
        assert_eq!(
            applied,
            Applied::Requeued {
                spent: 1,
                budget: 2
            }
        );

        let updated = cards::get_card(&pool, &card.id).await.unwrap().unwrap();
        let col = cards::get_column(&pool, &updated.column_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(col.state_binding.as_deref(), Some("ready"));
        assert_eq!(
            updated
                .metadata_json
                .get(REFINEMENT_SPENT_KEY)
                .and_then(|v| v.as_u64()),
            Some(1)
        );
        assert!(
            updated
                .metadata_json
                .get(LAST_CHECK_OUTPUT_KEY)
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.contains("assertion exploded")),
            "check stdout must land on the card for the next brief"
        );
        assert_eq!(
            updated
                .metadata_json
                .get("attempt_count")
                .and_then(|v| v.as_u64()),
            Some(1),
            "refinement must not consume the ordinary attempt_cap"
        );
    }

    #[tokio::test]
    async fn apply_parks_at_cap() {
        let pool = test_pool().await;
        let card = goal_in_review(
            &pool,
            json!({REFINEMENT_BUDGET_KEY: 1, REFINEMENT_SPENT_KEY: 1}),
        )
        .await;
        let applied = apply(
            &pool,
            &card.id,
            PERSONAL_PROJECT_ID,
            &card.title,
            &card.metadata_json,
            "still failing",
        )
        .await
        .unwrap();
        match applied {
            Applied::Parked { spent, budget, .. } => {
                assert_eq!(spent, 2);
                assert_eq!(budget, 1);
            }
            other => panic!("expected park, got {other:?}"),
        }
        let updated = cards::get_card(&pool, &card.id).await.unwrap().unwrap();
        let col = cards::get_column(&pool, &updated.column_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(col.state_binding.as_deref(), Some("failed"));
        let unblock = decisions::find_open_decision_for_goal(&pool, &card.id, "unblock")
            .await
            .unwrap()
            .expect("park must open unblock");
        assert_eq!(
            unblock.payload.get("reason").and_then(|v| v.as_str()),
            Some("refinement_budget")
        );
    }
}
