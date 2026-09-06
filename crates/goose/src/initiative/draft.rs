//! W4 — Tier 1 proposal draft (the token-spending stage).
//!
//! Reached only after the Tier 0 gate ([`crate::initiative::gate`]) has proven,
//! for free, that a proposal is worth drafting. Two cost controls live here:
//!
//!   1. RECOGNITION PRUNING (quality flywheel): before spending a token we ask
//!      [`crate::recognition::seen_observation`] whether this exact observation
//!      was proposed before and **bounced**. If so we prune — never re-pitch a
//!      declined automation. Outcomes flow back via the decisions write-back, so
//!      the gate gets cheaper and better with use.
//!   2. CHEAP-BY-DEFAULT MODEL (#249 routing): the draft runs on the cheap
//!      provider via `complete_fast`. If no provider is configured or the model
//!      output can't be parsed, we fall back to a deterministic template — so
//!      origination never depends on model availability and has a zero-token
//!      floor.

use crate::agents::reply_parts::AccountedFastCompletion;
use crate::conversation::message::Message;
use crate::initiative::command_counter::CommandPattern;
use crate::providers::base::Provider;
use crate::recognition;
use crate::session::SessionManager;
use sqlx::{Pool, Sqlite};
use std::sync::Arc;

const DRAFT_SESSION_ID: &str = "initiative-draft";

/// A rendered proposal ready to surface as a goal card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftedProposal {
    pub title: String,
    pub description: String,
}

/// True when this observation was previously surfaced and declined — the caller
/// must suppress re-origination (no token spent). Novel or positively-resolved
/// observations are not pruned.
pub async fn is_pruned(pool: &Pool<Sqlite>, normalized: &str) -> bool {
    match recognition::seen_observation(pool, normalized).await {
        Some(seen) => seen.was_bounced(),
        None => false,
    }
}

/// Deterministic zero-token proposal. The guaranteed floor and the fallback when
/// the cheap model is unavailable/unparseable.
pub fn template_draft(pattern: &CommandPattern) -> DraftedProposal {
    let cmd = &pattern.normalized;
    DraftedProposal {
        title: format!("Automate `{}`?", cmd),
        description: format!(
            "I noticed you've run `{}` {} times recently. Want me to save it as a \
             recipe you can run on demand or on a schedule?\n\n\
             Approve this card to set it up, or archive it to dismiss.",
            cmd, pattern.count
        ),
    }
}

/// Cheap-model draft (#249 routing). Returns `None` on any provider or parse
/// failure so the caller falls back to [`template_draft`].
pub async fn model_draft(
    provider: &Arc<dyn Provider>,
    pattern: &CommandPattern,
) -> Option<DraftedProposal> {
    let system = "You write a single short, friendly automation proposal for a personal agent. \
        The user repeatedly ran a shell command; propose saving it as a reusable recipe. \
        Reply ONLY with a JSON object: {\"title\": \"...\", \"description\": \"...\"}. \
        Title <= 60 chars, no markdown fences, no prose outside the JSON.";
    let user = Message::user().with_text(format!(
        "Command (normalized): {}\nObserved {} times.\nExamples:\n{}",
        pattern.normalized,
        pattern.count,
        pattern.exemplars.join("\n")
    ));

    let manager = std::sync::Arc::new(SessionManager::instance());
    let session = AccountedFastCompletion::ensure_background_session(
        std::sync::Arc::clone(&manager),
        DRAFT_SESSION_ID,
    )
    .await
    .ok()?;
    let (response, _usage) = AccountedFastCompletion::complete_fast_accounted(
        manager,
        session,
        std::sync::Arc::clone(provider),
        system,
        std::slice::from_ref(&user),
        &[],
        false,
    )
    .await
    .ok()?;
    parse_draft(&response.as_concat_text())
}

/// Extract `{title, description}` from a model response, tolerating code fences
/// and surrounding prose.
fn parse_draft(text: &str) -> Option<DraftedProposal> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    let json = text.get(start..=end)?;
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let title = v.get("title")?.as_str()?.trim().to_string();
    let description = v.get("description")?.as_str()?.trim().to_string();
    if title.is_empty() || description.is_empty() {
        return None;
    }
    Some(DraftedProposal { title, description })
}

/// Production entry: prune, then draft on the cheap model with a template
/// fallback. `provider == None` (cheap routing disabled / unavailable) uses the
/// template directly. Returns `None` only when pruned.
pub async fn draft_with_provider(
    pool: &Pool<Sqlite>,
    pattern: &CommandPattern,
    provider: Option<&Arc<dyn Provider>>,
) -> Option<DraftedProposal> {
    if is_pruned(pool, &pattern.normalized).await {
        tracing::debug!(
            target: "initiative",
            normalized = %pattern.normalized,
            "pruned: observation previously proposed and bounced"
        );
        return None;
    }
    let drafted = match provider {
        Some(p) => model_draft(p, pattern)
            .await
            .unwrap_or_else(|| template_draft(pattern)),
        None => template_draft(pattern),
    };
    Some(drafted)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            normalized: "npm run build".into(),
            count: 4,
            exemplars: vec!["npm run build".into()],
        }
    }

    #[test]
    fn template_is_nonempty_and_mentions_command() {
        let d = template_draft(&pattern());
        assert!(d.title.contains("npm run build"));
        assert!(!d.description.is_empty());
    }

    #[test]
    fn parse_tolerates_fences_and_prose() {
        let raw =
            "Sure!\n```json\n{\"title\":\"Automate build\",\"description\":\"Save it.\"}\n```";
        let d = parse_draft(raw).expect("parsed");
        assert_eq!(d.title, "Automate build");
        assert_eq!(d.description, "Save it.");
    }

    #[test]
    fn parse_rejects_empty_fields() {
        assert!(parse_draft(r#"{"title":"","description":"x"}"#).is_none());
        assert!(parse_draft("no json here").is_none());
    }

    #[test]
    fn model_draft_cannot_bypass_the_shared_paid_dispatch_boundary() {
        let source = include_str!("draft.rs");
        let direct_call = [".", "complete_fast("].concat();
        assert!(source.contains("complete_fast_accounted"));
        assert!(
            !source.contains(&direct_call),
            "initiative draft must not call Provider::complete_fast directly"
        );
    }

    #[tokio::test]
    async fn novel_observation_is_not_pruned() {
        let pool = test_pool().await;
        assert!(!is_pruned(&pool, "npm run build").await);
        let d = draft_with_provider(&pool, &pattern(), None)
            .await
            .expect("novel → drafts via template");
        assert!(d.title.contains("npm run build"));
    }

    #[tokio::test]
    async fn bounced_observation_is_pruned_before_any_draft() {
        let pool = test_pool().await;
        let q = "npm run build";
        // Seed a recognition for this exact observation, keyed on the worker
        // session, then bounce it via the decision write-back.
        let worker_session = "ws-bounce";
        sqlx::query(
            "INSERT INTO recognition_events
                (retrieval_id, session_id, query, retrieved_at, rc_persona, strategy)
             VALUES ('rec-x', ?, ?, '2026-01-01T00:00:00.000Z', '', 'cascade')",
        )
        .bind(worker_session)
        .bind(q)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO cards (id, project_id, card_type, title, column_id, metadata_json)
             VALUES ('g-x', '00000000-0000-0000-0000-000000000001', 'goal', 'G',
                     'col-personal-backlog', ?)",
        )
        .bind(serde_json::json!({ "worker_session_id": worker_session }).to_string())
        .execute(&pool)
        .await
        .unwrap();
        recognition::write_back_decision_outcome(&pool, "g-x", false).await;

        assert!(is_pruned(&pool, q).await, "bounced observation prunes");
        assert!(
            draft_with_provider(&pool, &pattern(), None).await.is_none(),
            "pruned → no draft, no token"
        );
    }
}
