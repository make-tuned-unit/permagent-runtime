//! W6 — the initiative tick: the orchestration that ties the stages together.
//!
//! One tick is: Tier 0 gate → (on Proceed) Tier 1 draft w/ recognition prune →
//! (on draft) emit a goal card. It is the consumer of every other module here.
//!
//! TICK DRIVER — a deliberate correction to the Phase-0 plan: the tick is driven
//! NATIVELY (a Rust interval loop), NOT as a scheduler recipe. The scheduler
//! executes a recipe by spinning up an agent, which spends tokens *per run* —
//! that is fundamentally incompatible with the zero-token Tier 0 gate that is
//! this layer's whole cost argument. The scheduler is still reused, but for its
//! correct role: the EXECUTION TARGET of an *approved* proposal (the saved
//! automation becomes a `ScheduledJob`). The driver loop emits the same
//! `automation_job_*` audit semantics so observability is preserved.

use crate::initiative::draft::draft_with_provider;
use crate::initiative::emit::surface_initiative_proposal;
use crate::initiative::gate::{evaluate, GateConfig, GateDecision, GateInputs, SkipReason};
use crate::providers::base::Provider;
use sqlx::{Pool, Sqlite};
use std::sync::Arc;

/// The result of one tick — also the telemetry signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TickOutcome {
    /// Tier 0 stopped the tick for free (the common, cheap case).
    Skipped(SkipReason),
    /// Proceeded but the observation was previously bounced — no token spent
    /// past the prune, nothing surfaced.
    Pruned,
    /// A proposal was originated and surfaced as a goal card in Triage.
    Emitted { card_id: String },
}

/// Run one origination tick. Pure orchestration over already-assembled inputs;
/// the caller (the native driver) gathers `inputs` from the live activity
/// snapshot + command counter and persists the cursors.
///
/// `provider == None` uses the deterministic template draft (cheap routing off
/// or unavailable). The token-spending model call happens ONLY past the gate and
/// the recognition prune.
pub async fn run_initiative_tick(
    pool: &Pool<Sqlite>,
    project_id: &str,
    inputs: &GateInputs,
    cfg: &GateConfig,
    provider: Option<&Arc<dyn Provider>>,
) -> Result<TickOutcome, String> {
    let pattern = match evaluate(inputs, cfg) {
        GateDecision::Skip(reason) => return Ok(TickOutcome::Skipped(reason)),
        GateDecision::Proceed(pattern) => pattern,
    };

    let draft = match draft_with_provider(pool, &pattern, provider).await {
        Some(d) => d,
        None => return Ok(TickOutcome::Pruned),
    };

    let outcome =
        surface_initiative_proposal(pool, project_id, &pattern, &draft.title, &draft.description)
            .await?;
    let crate::initiative::emit::InitiativeOutcome::CardCreated { card_id } = outcome;
    Ok(TickOutcome::Emitted { card_id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards;
    use crate::initiative::command_counter::CommandPattern;
    use crate::projects::PERSONAL_PROJECT_ID;
    use chrono::{DateTime, Utc};

    async fn test_pool() -> Pool<Sqlite> {
        use crate::session::spectral_schema::init_spectral_db;
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    fn pattern() -> CommandPattern {
        CommandPattern {
            normalized: "git status && git pull".into(),
            count: 3,
            exemplars: vec!["git status && git pull".into()],
        }
    }

    fn passing_inputs() -> GateInputs {
        GateInputs {
            now: t(100_000),
            last_activity_at: Some(t(99_000)),
            prev_tick_activity_at: Some(t(98_000)),
            last_engagement_at: Some(t(0)),
            last_proposal_at: None,
            pattern: Some(pattern()),
        }
    }

    async fn card_count(pool: &Pool<Sqlite>) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM cards")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// The cheap-by-default proof: a tick with no new activity exits at Tier 0
    /// and surfaces nothing — zero tokens, zero cards.
    #[tokio::test]
    async fn no_activity_tick_skips_and_emits_nothing() {
        let pool = test_pool().await;
        let mut inputs = passing_inputs();
        inputs.last_activity_at = None; // nothing observed
        inputs.pattern = None;

        let outcome = run_initiative_tick(
            &pool,
            PERSONAL_PROJECT_ID,
            &inputs,
            &GateConfig::default(),
            None,
        )
        .await
        .unwrap();

        assert_eq!(outcome, TickOutcome::Skipped(SkipReason::NoActivityDelta));
        assert_eq!(card_count(&pool).await, 0, "no card on an idle tick");
    }

    /// The origination proof: a real repeated-command pattern past the gate
    /// surfaces a goal card in Triage.
    #[tokio::test]
    async fn repeated_pattern_emits_goal_card_in_triage() {
        let pool = test_pool().await;
        let outcome = run_initiative_tick(
            &pool,
            PERSONAL_PROJECT_ID,
            &passing_inputs(),
            &GateConfig::default(),
            None, // template draft → deterministic, no model needed
        )
        .await
        .unwrap();

        let card_id = match outcome {
            TickOutcome::Emitted { card_id } => card_id,
            other => panic!("expected Emitted, got {:?}", other),
        };
        let card = cards::get_card(&pool, &card_id)
            .await
            .unwrap()
            .expect("card persisted");
        assert_eq!(card.card_type, "goal");
        assert_eq!(card.created_by, "henry");
        let triage = cards::get_goal_column(&pool, PERSONAL_PROJECT_ID, "triage")
            .await
            .unwrap()
            .expect("triage seeded");
        assert_eq!(card.column_id, triage.id, "lands in Triage");
        assert_eq!(card.metadata_json["initiative"], serde_json::json!(true));
        assert_eq!(card_count(&pool).await, 1);
    }

    /// Gate guards still win over a present pattern (engaged user → nothing).
    #[tokio::test]
    async fn engaged_user_tick_emits_nothing() {
        let pool = test_pool().await;
        let mut inputs = passing_inputs();
        inputs.last_engagement_at = Some(t(99_950)); // 50s ago < quiet window

        let outcome = run_initiative_tick(
            &pool,
            PERSONAL_PROJECT_ID,
            &inputs,
            &GateConfig::default(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(outcome, TickOutcome::Skipped(SkipReason::UserEngaged));
        assert_eq!(card_count(&pool).await, 0);
    }
}
