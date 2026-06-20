//! W5 — card-seam emit (REUSE, not novel).
//!
//! Surfacing is deliberately the Steward's proven contract: a card minted via
//! [`crate::cards::create_card`] with `created_by = "henry"` and a
//! machine-readable `metadata_json` staged for the future decision-inbox swap
//! (mirrors `steward::surface_destructive_proposal`). We do NOT build a parallel
//! sink.
//!
//! The one difference from Steward is `card_type = "goal"`: an originated
//! automation IS a goal, so it lands in the project's **Triage** column where
//! the (gated) orchestrator will pick it up once enabled. Until then it simply
//! sits on the always-live board for the user to see/approve. That split is the
//! whole architecture: this layer ORIGINATES the goal; the commodity loop
//! CONSUMES it.
//!
//! Swap (later): once decision-inbox is merged + orchestrator enabled, replace
//! the `cards::create_card(...)` call with `decisions::create_decision(...)`.
//! That is the only line that changes.

use crate::cards::{self, CreateCard};
use crate::initiative::command_counter::CommandPattern;
use sqlx::{Pool, Sqlite};

/// Outcome of surfacing an initiative proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitiativeOutcome {
    /// A goal card was created in Triage for the user to approve.
    CardCreated { card_id: String },
}

/// Surface a repeated-command pattern as an automation-proposal goal card.
///
/// `draft_*` are the Tier 1 model's rendered proposal (title + body); the
/// pattern carries the deterministic provenance the metadata records.
pub async fn surface_initiative_proposal(
    pool: &Pool<Sqlite>,
    project_id: &str,
    pattern: &CommandPattern,
    draft_title: &str,
    draft_description: &str,
) -> Result<InitiativeOutcome, String> {
    // Everything a human needs to judge — and the machine-readable shape the
    // future decision-inbox swap will consume (Steward's contract).
    let metadata_json = serde_json::json!({
        "initiative": true,
        "source": "repeated_command",
        "normalized_command": pattern.normalized,
        "occurrence_count": pattern.count,
        "exemplars": pattern.exemplars,
        "goal_state": "triage",
        "needs_human_attention": true,
    });

    let card = cards::create_card(
        pool,
        CreateCard {
            project_id: project_id.to_string(),
            title: draft_title.to_string(),
            description: Some(draft_description.to_string()),
            card_type: Some("goal".to_string()),
            column_id: None, // goal → Triage by default
            created_by: Some("henry".to_string()),
            metadata_json: Some(metadata_json),
        },
    )
    .await?;

    tracing::info!(
        target: "initiative",
        card_id = %card.id,
        normalized = %pattern.normalized,
        count = pattern.count,
        "originated automation proposal → goal card in Triage"
    );

    Ok(InitiativeOutcome::CardCreated { card_id: card.id })
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

    fn pattern() -> CommandPattern {
        CommandPattern {
            normalized: "git status && git pull".into(),
            count: 3,
            exemplars: vec!["git status && git pull".into()],
        }
    }

    #[tokio::test]
    async fn surfaces_goal_card_in_triage() {
        let pool = test_pool().await;
        let outcome = surface_initiative_proposal(
            &pool,
            PERSONAL_PROJECT_ID,
            &pattern(),
            "Automate your morning git sync?",
            "You've run `git status && git pull` 3 times this week.",
        )
        .await
        .expect("card created");

        let InitiativeOutcome::CardCreated { card_id } = outcome;
        let card = cards::get_card(&pool, &card_id)
            .await
            .unwrap()
            .expect("card persisted");
        assert_eq!(card.card_type, "goal");
        assert_eq!(card.created_by, "henry");
        // Lands in the Triage column.
        let col = cards::get_goal_column(&pool, PERSONAL_PROJECT_ID, "triage")
            .await
            .unwrap()
            .expect("triage column seeded");
        assert_eq!(card.column_id, col.id, "originated goal lands in Triage");
        // Provenance is recorded for the decision-inbox swap.
        assert_eq!(card.metadata_json["initiative"], serde_json::json!(true));
        assert_eq!(
            card.metadata_json["normalized_command"],
            serde_json::json!("git status && git pull")
        );
        assert_eq!(
            card.metadata_json["needs_human_attention"],
            serde_json::json!(true)
        );
    }
}
