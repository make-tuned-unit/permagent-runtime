//! Bounded autonomous refinement after completion-check failure.
//!
//! Distinct from [`crate::goal_transition::goal_budget`] (attempt/token/wallclock)
//! and from the premature-done hold ([`crate::cost_router::hold_done`]): this is
//! the *check-failure rework budget*. The worker said done, the mechanical checks
//! disagreed, and the goal gets a bounded number of rounds to fix it before a
//! person is asked.
//!
//! Within budget the goal returns to Ready with the failing check's stdout/stderr
//! tail (plus the lint and placeholder findings the caller folds in) as the
//! corrective plan — the `retry_context_block` pattern, so the next brief opens
//! with the exact failure. Each requeue leaves a routing-snapshot receipt and
//! appends a round to `refinement_history`. At exhaustion the goal parks with an
//! `unblock` decision carrying the WHOLE history, not just the last round.
//!
//! The cap comes from the goal's own `refinement_budget` metadata when it
//! declares one (an explicit `0` opts out), otherwise from the caller's
//! configured default ([`DEFAULT_REFINEMENT_BUDGET`]).

use serde_json::{json, Map, Value};

use crate::cost_router::snapshot::RoutingSnapshot;
use crate::cost_router::tool_signals::ToolTranscriptSignals;
use crate::decisions;
use crate::goal_transition::{self, BudgetExhaustion};

/// Metadata key: max auto-rework rounds after a failing completion check.
pub const REFINEMENT_BUDGET_KEY: &str = "refinement_budget";
/// Metadata key: how many refinement rounds have been spent.
pub const REFINEMENT_SPENT_KEY: &str = "refinement_spent";
/// Metadata key: last failing check stdout injected into the next brief.
pub const LAST_CHECK_OUTPUT_KEY: &str = "last_check_output";
/// Metadata key: every rework round so far, oldest first.
pub const REFINEMENT_HISTORY_KEY: &str = "refinement_history";

/// Rework rounds allowed when nobody configured otherwise.
pub const DEFAULT_REFINEMENT_BUDGET: u64 = 3;

/// Rounds kept on the card. Older rounds fall off the front.
const MAX_HISTORY_ROUNDS: usize = 12;
/// Per-round output kept in the history (the tail is what debugs).
const ROUND_TAIL_CHARS: usize = 2_000;
/// The corrective plan handed to the next brief.
const CHECK_OUTPUT_CHARS: usize = 8_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefinementDecision {
    /// No budget (goal declared 0, or the configured default is 0) — caller
    /// keeps the existing Fail path.
    Skip,
    /// Still within budget: return to Ready with check output in the brief.
    Requeue { spent: u64, budget: u64 },
    /// Budget exhausted: park with unblock.
    Park { spent: u64, budget: u64 },
}

/// The budget this goal declared for itself, if any. `Some(0)` is a deliberate
/// opt-out and outranks the configured default.
pub fn declared_budget(metadata: &Value) -> Option<u64> {
    metadata
        .get(REFINEMENT_BUDGET_KEY)
        .and_then(|v| v.as_u64())
        .or_else(|| {
            metadata
                .get("budget")
                .and_then(|b| b.get(REFINEMENT_BUDGET_KEY))
                .and_then(|v| v.as_u64())
        })
}

/// The cap actually in force: the goal's own declaration, else the caller's
/// configured default.
pub fn effective_budget(metadata: &Value, default_budget: u64) -> u64 {
    declared_budget(metadata).unwrap_or(default_budget)
}

pub fn refinement_spent(metadata: &Value) -> u64 {
    metadata
        .get(REFINEMENT_SPENT_KEY)
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
}

/// Rounds recorded so far, oldest first.
pub fn refinement_history(metadata: &Value) -> Vec<Value> {
    metadata
        .get(REFINEMENT_HISTORY_KEY)
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

/// Decide the next step given current metadata and a failing-check transcript.
///
/// An empty transcript is never a failure worth spending budget on: the caller
/// only reaches here on a Fail verdict, and a Fail with nothing to say gives the
/// next worker nothing to fix.
pub fn decide(metadata: &Value, check_output: &str, default_budget: u64) -> RefinementDecision {
    if check_output.trim().is_empty() {
        return RefinementDecision::Skip;
    }
    let budget = effective_budget(metadata, default_budget);
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
#[allow(clippy::too_many_arguments)]
pub async fn apply(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    card_id: &str,
    project_id: &str,
    goal_title: &str,
    metadata: &Value,
    check_output: &str,
    default_budget: u64,
) -> Result<Applied, String> {
    match decide(metadata, check_output, default_budget) {
        RefinementDecision::Skip => Ok(Applied::Skipped),
        RefinementDecision::Requeue { spent, budget } => {
            let mut extra = Map::new();
            extra.insert(REFINEMENT_SPENT_KEY.to_string(), json!(spent));
            extra.insert(
                LAST_CHECK_OUTPUT_KEY.to_string(),
                json!(truncate(check_output, CHECK_OUTPUT_CHARS)),
            );
            extra.insert(
                REFINEMENT_HISTORY_KEY.to_string(),
                Value::Array(appended_history(
                    metadata,
                    spent,
                    budget,
                    "requeued",
                    check_output,
                )),
            );
            requeue_snapshot(spent, budget).write_into(&mut extra);
            goal_transition::return_to_ready(
                pool,
                card_id,
                decisions::ACTOR_SYSTEM,
                &format!("completion checks failed (rework {spent}/{budget})"),
                None,
                extra,
            )
            .await
            .map_err(|e| e.to_string())?;
            Ok(Applied::Requeued { spent, budget })
        }
        RefinementDecision::Park { spent, budget } => {
            // The history is what a person needs to answer the unblock: every
            // round that was tried, not just the one that ran out of budget.
            let history = appended_history(metadata, spent, budget, "parked", check_output);
            let mut patch = Map::new();
            patch.insert(
                REFINEMENT_HISTORY_KEY.to_string(),
                Value::Array(history.clone()),
            );
            patch.insert(REFINEMENT_SPENT_KEY.to_string(), json!(spent));
            park_snapshot(spent, budget).write_into(&mut patch);
            // Failure-tolerant: a card that would not take the history still
            // has to park, or a failing goal silently sits in Review forever.
            if let Err(e) =
                crate::cards::merge_card_metadata(pool, card_id, Value::Object(patch), false).await
            {
                tracing::warn!(
                    target: "permagent::goal_refinement",
                    card_id = %card_id,
                    "could not write the rework history before parking: {e}"
                );
            }
            let decision_id = goal_transition::exhaust_and_park(
                pool,
                card_id,
                goal_title,
                project_id,
                BudgetExhaustion::RefinementBudget { spent, cap: budget },
                Some(&render_history(&history)),
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

/// The receipt on the card, in the shape `hold_done`'s routing snapshot uses:
/// one prose sentence the Build meter and the next brief can both read.
fn requeue_snapshot(spent: u64, budget: u64) -> RoutingSnapshot {
    RoutingSnapshot::from_signals(
        &ToolTranscriptSignals::default(),
        Some(&format!(
            "Rework {spent}/{budget}: completion checks failed, so the goal went back to \
             Ready with the failing output as its corrective plan."
        )),
    )
}

fn park_snapshot(spent: u64, budget: u64) -> RoutingSnapshot {
    RoutingSnapshot::from_signals(
        &ToolTranscriptSignals::default(),
        Some(&format!(
            "Rework budget spent ({spent}/{budget}) and the checks still fail — parked with \
             the full check history for a person to answer."
        )),
    )
}

/// Append this round to the card's history, oldest rounds falling off the front.
fn appended_history(
    metadata: &Value,
    round: u64,
    budget: u64,
    outcome: &str,
    check_output: &str,
) -> Vec<Value> {
    let mut history = refinement_history(metadata);
    history.push(json!({
        "round": round,
        "budget": budget,
        "outcome": outcome,
        "at": chrono::Utc::now().to_rfc3339(),
        "output": truncate(check_output, ROUND_TAIL_CHARS),
    }));
    if history.len() > MAX_HISTORY_ROUNDS {
        let drop = history.len() - MAX_HISTORY_ROUNDS;
        history.drain(0..drop);
    }
    history
}

/// Render the rounds as the text that lands on the Decision Inbox card.
pub fn render_history(history: &[Value]) -> String {
    if history.is_empty() {
        return "(no check history recorded)".to_string();
    }
    history
        .iter()
        .map(|r| {
            let round = r.get("round").and_then(|v| v.as_u64()).unwrap_or(0);
            let budget = r.get("budget").and_then(|v| v.as_u64()).unwrap_or(0);
            let at = r.get("at").and_then(|v| v.as_str()).unwrap_or("");
            let out = r.get("output").and_then(|v| v.as_str()).unwrap_or("");
            format!("── rework {round}/{budget} ({at}) ──\n{out}")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
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
    use crate::cost_router::snapshot::ROUTING_SNAPSHOT_KEY;
    use crate::projects::PERSONAL_PROJECT_ID;
    use crate::session::spectral_schema::init_spectral_db;
    use serde_json::json;

    #[test]
    fn config_default_applies_when_the_goal_is_silent() {
        // The primitive is on by default now: a goal that says nothing still
        // gets the configured rework rounds.
        assert_eq!(effective_budget(&json!({}), DEFAULT_REFINEMENT_BUDGET), 3);
        assert_eq!(
            decide(&json!({}), "boom", DEFAULT_REFINEMENT_BUDGET),
            RefinementDecision::Requeue {
                spent: 1,
                budget: 3
            }
        );
    }

    #[test]
    fn an_explicit_zero_on_the_goal_still_opts_out() {
        let meta = json!({REFINEMENT_BUDGET_KEY: 0});
        assert_eq!(declared_budget(&meta), Some(0));
        assert_eq!(effective_budget(&meta, DEFAULT_REFINEMENT_BUDGET), 0);
        assert_eq!(
            decide(&meta, "boom", DEFAULT_REFINEMENT_BUDGET),
            RefinementDecision::Skip
        );
    }

    #[test]
    fn the_goals_own_budget_outranks_the_default() {
        let meta = json!({REFINEMENT_BUDGET_KEY: 1, REFINEMENT_SPENT_KEY: 1});
        assert_eq!(
            decide(&meta, "still failing", 9),
            RefinementDecision::Park {
                spent: 2,
                budget: 1
            }
        );
        let nested = json!({"budget": {REFINEMENT_BUDGET_KEY: 5}});
        assert_eq!(effective_budget(&nested, DEFAULT_REFINEMENT_BUDGET), 5);
    }

    #[test]
    fn decide_skip_when_config_default_is_zero() {
        assert_eq!(decide(&json!({}), "fail", 0), RefinementDecision::Skip);
    }

    #[test]
    fn nothing_to_fix_spends_no_budget() {
        // A Fail with an empty corrective plan gives the next worker nothing;
        // spending a rework round on it would only burn the budget.
        assert_eq!(
            decide(&json!({}), "   \n ", DEFAULT_REFINEMENT_BUDGET),
            RefinementDecision::Skip
        );
    }

    #[test]
    fn decide_requeue_within_budget() {
        let meta = json!({REFINEMENT_BUDGET_KEY: 2, REFINEMENT_SPENT_KEY: 0});
        assert_eq!(
            decide(&meta, "boom", 0),
            RefinementDecision::Requeue {
                spent: 1,
                budget: 2
            }
        );
        let meta = json!({REFINEMENT_BUDGET_KEY: 2, REFINEMENT_SPENT_KEY: 1});
        assert_eq!(
            decide(&meta, "boom", 0),
            RefinementDecision::Requeue {
                spent: 2,
                budget: 2
            }
        );
    }

    #[test]
    fn history_keeps_the_newest_rounds() {
        let mut meta = json!({});
        for round in 1..=(MAX_HISTORY_ROUNDS as u64 + 3) {
            let history = appended_history(&meta, round, 99, "requeued", &format!("round {round}"));
            meta = json!({ REFINEMENT_HISTORY_KEY: history });
        }
        let history = refinement_history(&meta);
        assert_eq!(history.len(), MAX_HISTORY_ROUNDS);
        let rendered = render_history(&history);
        assert!(
            !rendered.contains("round 1\n"),
            "the oldest rounds fall off the front"
        );
        assert!(rendered.contains(&format!("round {}", MAX_HISTORY_ROUNDS + 3)));
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
    async fn requeue_carries_the_check_stdout_and_a_routing_snapshot() {
        let pool = test_pool().await;
        let card = goal_in_review(&pool, json!({REFINEMENT_BUDGET_KEY: 2})).await;
        let applied = apply(
            &pool,
            &card.id,
            PERSONAL_PROJECT_ID,
            &card.title,
            &card.metadata_json,
            "[0] shell Fail\nstdout:\ntest tests::works ... FAILED\nstderr:\nassertion exploded",
            DEFAULT_REFINEMENT_BUDGET,
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
        let plan = updated
            .metadata_json
            .get(LAST_CHECK_OUTPUT_KEY)
            .and_then(|v| v.as_str())
            .expect("check stdout must land on the card for the next brief");
        assert!(plan.contains("test tests::works ... FAILED"), "{plan}");
        assert!(plan.contains("assertion exploded"), "{plan}");
        assert_eq!(
            updated
                .metadata_json
                .get("attempt_count")
                .and_then(|v| v.as_u64()),
            Some(1),
            "rework must not consume the ordinary attempt_cap"
        );

        let snapshot = updated
            .metadata_json
            .get(ROUTING_SNAPSHOT_KEY)
            .and_then(|v| v.get("note"))
            .and_then(|v| v.as_str())
            .expect("every requeue leaves a routing-snapshot receipt");
        assert!(snapshot.contains("Rework 1/2"), "{snapshot}");

        let history = refinement_history(&updated.metadata_json);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0]["outcome"], json!("requeued"));
    }

    #[tokio::test]
    async fn exhaustion_parks_with_the_whole_history() {
        let pool = test_pool().await;
        let card = goal_in_review(
            &pool,
            json!({
                REFINEMENT_BUDGET_KEY: 1,
                REFINEMENT_SPENT_KEY: 1,
                REFINEMENT_HISTORY_KEY: [{
                    "round": 1, "budget": 1, "outcome": "requeued",
                    "at": "2026-08-25T00:00:00Z", "output": "round one: clippy screamed",
                }],
            }),
        )
        .await;
        let applied = apply(
            &pool,
            &card.id,
            PERSONAL_PROJECT_ID,
            &card.title,
            &card.metadata_json,
            "round two: clippy still screams",
            DEFAULT_REFINEMENT_BUDGET,
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
        let detail = unblock.detail;
        assert!(
            detail.contains("round one: clippy screamed"),
            "the card carries every round, not just the last: {detail}"
        );
        assert!(detail.contains("round two: clippy still screams"), "{detail}");

        assert_eq!(refinement_history(&updated.metadata_json).len(), 2);
    }

    #[tokio::test]
    async fn a_passing_check_resets_nothing() {
        // A goal that failed once, was reworked, and then passed keeps its
        // spent counter: the budget is per goal, not per verification run, so
        // a pass must not hand the goal a fresh set of rounds to burn.
        let pool = test_pool().await;
        let card = goal_in_review(
            &pool,
            json!({REFINEMENT_BUDGET_KEY: 3, REFINEMENT_SPENT_KEY: 2}),
        )
        .await;
        let applied = apply(
            &pool,
            &card.id,
            PERSONAL_PROJECT_ID,
            &card.title,
            &card.metadata_json,
            "",
            DEFAULT_REFINEMENT_BUDGET,
        )
        .await
        .unwrap();
        assert_eq!(applied, Applied::Skipped);

        let updated = cards::get_card(&pool, &card.id).await.unwrap().unwrap();
        assert_eq!(refinement_spent(&updated.metadata_json), 2);
        assert!(updated.metadata_json.get(ROUTING_SNAPSHOT_KEY).is_none());
        assert!(refinement_history(&updated.metadata_json).is_empty());
        let col = cards::get_column(&pool, &updated.column_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            col.state_binding.as_deref(),
            Some("review"),
            "a pass leaves the goal exactly where the verifier put it"
        );
    }
}
