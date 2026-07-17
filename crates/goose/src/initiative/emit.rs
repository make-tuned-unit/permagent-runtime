//! W5 — proposal-seam emit (REUSE, not novel).
//!
//! An originated automation is a speculative "automate this?" awaiting yes/no —
//! semantically a **decision**, so by default it surfaces on the **Decision
//! Inbox** ([`crate::decisions::create_decision`], `kind = automation_proposal`),
//! the human-in-the-loop approval surface. Declining it records a recognition
//! bounce so it is never re-pitched (the anti-nag guarantee).
//!
//! The legacy **Triage** surface (a goal card via [`crate::cards::create_card`],
//! Steward's contract) is retained and selectable via the `initiative_surface`
//! config toggle, so the surface stays controllable and reversible. The choice
//! is made by the driver and threaded down; this module stays pure and testable.

use crate::cards::{self, CreateCard};
use crate::decisions::{self, NewDecision, MAX_HEADLINE_CHARS};
use crate::initiative::command_counter::CommandPattern;
use sqlx::{Pool, Sqlite};

/// Which surface an originated proposal lands on. Default is [`Self::Inbox`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InitiativeSurface {
    /// The Decision Inbox — the human-in-the-loop approval surface (default).
    #[default]
    Inbox,
    /// The Triage board column — the legacy goal-card surface.
    Triage,
}

impl InitiativeSurface {
    /// Parse the `initiative_surface` config value. Unknown/empty → `Inbox`.
    pub fn from_config_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "triage" => Self::Triage,
            _ => Self::Inbox,
        }
    }

    /// Stable label for logs/telemetry.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Triage => "triage",
        }
    }
}

/// Outcome of surfacing an initiative proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitiativeOutcome {
    /// A decision was created on the Decision Inbox for the user to answer.
    DecisionCreated { decision_id: String },
    /// A goal card was created in Triage for the user to approve.
    CardCreated { card_id: String },
}

/// Surface a repeated-command pattern as an automation proposal on `surface`.
///
/// `draft_*` are the Tier 1 model's rendered proposal (title + body); the
/// pattern carries the deterministic provenance both surfaces record.
pub async fn surface_initiative_proposal(
    pool: &Pool<Sqlite>,
    project_id: &str,
    pattern: &CommandPattern,
    draft_title: &str,
    draft_description: &str,
    surface: InitiativeSurface,
) -> Result<InitiativeOutcome, String> {
    match surface {
        InitiativeSurface::Inbox => {
            // No seeded risk_policy class for this kind → resolves fail-closed to
            // Tier 2, i.e. the user (jesse) approves — exactly right for
            // "automate this?". Provenance is the same deterministic shape the
            // Triage metadata carries.
            let decision = decisions::create_decision(
                pool,
                NewDecision {
                    kind: "automation_proposal".to_string(),
                    project_id: Some(project_id.to_string()),
                    headline: Some(headline(draft_title)),
                    detail: Some(draft_description.to_string()),
                    payload: serde_json::json!({
                        "normalized_command": pattern.normalized,
                        "occurrence_count": pattern.count,
                        "exemplars": pattern.exemplars,
                        // The editable draft surfaced to the user (mirrors
                        // `detail`). Approve-with-edits diffs the user's
                        // revision against this to learn a correction
                        // (edit-as-training, decision_inbox::learn). This is the
                        // one producer wired to exercise the edit path
                        // end-to-end; other decision kinds carry no draft yet.
                        "draft": draft_description,
                    }),
                    ..Default::default()
                },
            )
            .await?;

            tracing::info!(
                target: "initiative",
                decision_id = %decision.id,
                surface = InitiativeSurface::Inbox.as_str(),
                normalized = %pattern.normalized,
                count = pattern.count,
                "originated automation proposal → Decision Inbox"
            );

            Ok(InitiativeOutcome::DecisionCreated {
                decision_id: decision.id,
            })
        }
        InitiativeSurface::Triage => {
            // Everything a human needs to judge, in the Steward card contract.
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
                surface = InitiativeSurface::Triage.as_str(),
                normalized = %pattern.normalized,
                count = pattern.count,
                "originated automation proposal → goal card in Triage"
            );

            Ok(InitiativeOutcome::CardCreated { card_id: card.id })
        }
    }
}

/// Truncate a drafted title to the Decision Inbox headline limit (a long
/// command would otherwise fail `create_decision`'s ≤80-char rule and be stored
/// as `malformed`).
fn headline(draft_title: &str) -> String {
    let trimmed = draft_title.trim();
    if trimmed.chars().count() <= MAX_HEADLINE_CHARS {
        trimmed.to_string()
    } else {
        let head: String = trimmed.chars().take(MAX_HEADLINE_CHARS - 1).collect();
        format!("{}…", head)
    }
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
    async fn inbox_surface_creates_automation_proposal_decision() {
        let pool = test_pool().await;
        let outcome = surface_initiative_proposal(
            &pool,
            PERSONAL_PROJECT_ID,
            &pattern(),
            "Automate your morning git sync?",
            "You've run `git status && git pull` 3 times this week.",
            InitiativeSurface::Inbox,
        )
        .await
        .expect("decision created");

        let decision_id = match outcome {
            InitiativeOutcome::DecisionCreated { decision_id } => decision_id,
            other => panic!("expected DecisionCreated, got {:?}", other),
        };
        let decision = decisions::get_decision(&pool, &decision_id)
            .await
            .unwrap()
            .expect("decision persisted");
        // A real automation_proposal — NOT coerced to malformed.
        assert_eq!(decision.kind, "automation_proposal");
        assert_eq!(decision.status, "open");
        assert_eq!(
            decision.tier, 2,
            "no seeded class → user-only (fail-closed)"
        );
        // Provenance carried into the payload.
        assert_eq!(
            decision.payload["normalized_command"],
            serde_json::json!("git status && git pull")
        );
        assert_eq!(decision.payload["occurrence_count"], serde_json::json!(3));
        // The editable draft is carried so approve-with-edits can diff a
        // revision against it (edit-as-training).
        assert_eq!(
            decision.payload["draft"],
            serde_json::json!("You've run `git status && git pull` 3 times this week.")
        );
        // No card was created on the inbox surface.
        let card_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cards")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(card_count, 0, "inbox surface creates no board card");
    }

    #[tokio::test]
    async fn long_title_is_truncated_to_a_valid_headline() {
        let pool = test_pool().await;
        let long_title = format!("Automate {}", "x".repeat(200));
        let outcome = surface_initiative_proposal(
            &pool,
            PERSONAL_PROJECT_ID,
            &pattern(),
            &long_title,
            "long body",
            InitiativeSurface::Inbox,
        )
        .await
        .expect("decision created");
        let InitiativeOutcome::DecisionCreated { decision_id } = outcome else {
            panic!("expected DecisionCreated");
        };
        let decision = decisions::get_decision(&pool, &decision_id)
            .await
            .unwrap()
            .unwrap();
        // Truncation kept it a real proposal rather than a malformed row.
        assert_eq!(decision.kind, "automation_proposal");
        assert!(decision.headline.chars().count() <= MAX_HEADLINE_CHARS);
    }

    #[tokio::test]
    async fn triage_surface_still_creates_goal_card() {
        let pool = test_pool().await;
        let outcome = surface_initiative_proposal(
            &pool,
            PERSONAL_PROJECT_ID,
            &pattern(),
            "Automate your morning git sync?",
            "You've run `git status && git pull` 3 times this week.",
            InitiativeSurface::Triage,
        )
        .await
        .expect("card created");

        let card_id = match outcome {
            InitiativeOutcome::CardCreated { card_id } => card_id,
            other => panic!("expected CardCreated, got {:?}", other),
        };
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
