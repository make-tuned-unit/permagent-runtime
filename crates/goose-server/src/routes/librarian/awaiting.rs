//! Answer the Librarian's concrete requests for context.
use crate::{routes::errors::ErrorResponse, state::AppState};
use axum::{
    extract::{Path, State},
    Json,
};
use permagent::agents::platform_extensions::librarian_entities as ledger;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize)]
pub(super) struct AwaitingContext {
    entity_id: String,
    name: String,
    entity_type: String,
    why_thin: String,
    mention_count: usize,
    // The legacy ledger does not record failed-attempt timestamps.
    last_attempt_at: Option<String>,
}

fn awaiting_rows(state: &ledger::EntityLedger) -> Vec<AwaitingContext> {
    state
        .needs_context
        .iter()
        .map(|entry| AwaitingContext {
            entity_id: ledger::needs_context_id(&entry.entity_type, &entry.canonical),
            name: entry.canonical.clone(),
            entity_type: entry.entity_type.clone(),
            why_thin: entry.why_thin.clone(),
            mention_count: entry.mentions,
            last_attempt_at: None,
        })
        .collect()
}

pub(super) async fn list() -> Result<Json<Vec<AwaitingContext>>, ErrorResponse> {
    let ledger =
        ledger::read_awaiting_ledger().map_err(|e| ErrorResponse::internal(e.to_string()))?;
    Ok(Json(awaiting_rows(&ledger)))
}

#[derive(Deserialize)]
pub(super) struct Answer {
    text: String,
}

fn validate_answer(text: &str) -> Result<&str, ErrorResponse> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > 10_000 {
        return Err(ErrorResponse::bad_request(
            "Enter an answer between 1 and 10,000 characters.",
        ));
    }
    Ok(text)
}

pub(super) async fn answer(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(answer): Json<Answer>,
) -> Result<Json<serde_json::Value>, ErrorResponse> {
    let text = validate_answer(&answer.text)?;
    let _guard = ledger::lock_ledger().await;
    let mut ledger =
        ledger::read_awaiting_ledger().map_err(|e| ErrorResponse::internal(e.to_string()))?;
    let entry = ledger
        .needs_context
        .iter()
        .find(|e| ledger::needs_context_id(&e.entity_type, &e.canonical) == id)
        .cloned()
        .ok_or_else(|| {
            ErrorResponse::not_found("This request is no longer waiting for an answer.")
        })?;
    let brain = state.brain.as_ref().ok_or_else(|| {
        ErrorResponse::service_unavailable("Brain is not ready. Your answer has not been saved.")
    })?;
    let content = format!(
        "Context supplied by the user about {} ({}): {}",
        entry.canonical, entry.entity_type, text
    );
    brain
        .remember_with(
            &format!("librarian:context:{id}"),
            &content,
            spectral::RememberOpts {
                source: Some("user.answer".into()),
                visibility: spectral::Visibility::Private,
                ..Default::default()
            },
        )
        .await
        .map_err(|e| ErrorResponse::internal(e.to_string()))?;
    if entry.entity_type == "person" {
        let entity_id = id
            .parse()
            .map_err(|e| ErrorResponse::internal(format!("Invalid entity id: {e}")))?;
        // A separate manual field preserves existing notes and provenance.
        brain
            .set_manual_entity_fields(entity_id, vec![("user_context".into(), text.into())])
            .await
            .map_err(|e| ErrorResponse::internal(e.to_string()))?;
    }
    ledger
        .needs_context
        .retain(|e| ledger::needs_context_id(&e.entity_type, &e.canonical) != id);
    ledger::save_ledger_and_refresh_count(&ledger)
        .map_err(|e| ErrorResponse::internal(e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "saved": true, "remaining": ledger.needs_context.len() }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requests_have_stable_ids_real_reasons_and_no_invented_dates() {
        let mut ledger = ledger::EntityLedger::default();
        ledger.needs_context.push(ledger::NeedsContext {
            canonical: "Ada".into(),
            entity_type: "person".into(),
            mentions: 3,
            why_thin: "Role unknown".into(),
        });
        let rows = awaiting_rows(&ledger);
        assert_eq!(rows[0].entity_id, ledger::needs_context_id("person", "Ada"));
        assert_eq!(rows[0].mention_count, 3);
        assert_eq!(rows[0].why_thin, "Role unknown");
        assert!(rows[0].last_attempt_at.is_none());
    }
    #[test]
    fn blank_and_oversized_answers_are_rejected() {
        assert!(validate_answer(" ").is_err());
        assert!(validate_answer(&"a".repeat(10_001)).is_err());
        assert_eq!(validate_answer("  An advisor  ").unwrap(), "An advisor");
    }
}
