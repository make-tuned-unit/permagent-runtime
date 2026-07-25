//! Durable, retryable application of answered Decision Inbox effects.

use crate::decisions::{self, Decision, DecisionProof};
use crate::goal_state::GoalAction;
use crate::goal_transition::{self, GuardError, TransitionEffects};
use sqlx::{Pool, Row, Sqlite};

const OUTBOX_BATCH_SIZE: i64 = 32;
const RUNNING_LEASE_SECONDS: i64 = 300;

/// The result displayed by the inline answer path.
pub type EffectResult = (Option<String>, Option<String>);

async fn goal_state(pool: &Pool<Sqlite>, goal_id: &str) -> Result<Option<String>, GuardError> {
    sqlx::query_scalar(
        "SELECT col.state_binding
         FROM cards c JOIN board_columns col ON col.id = c.column_id
         WHERE c.id = ?",
    )
    .bind(goal_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| GuardError::Db(e.to_string()))
}

fn already_applied(message: impl Into<String>) -> Result<EffectResult, GuardError> {
    Ok((Some(message.into()), None))
}

/// Apply an outbox-eligible decision effect without daemon `AppState`.
///
/// `tool_approval` and `session_gate` deliberately are not handled here.
pub async fn apply_decision_effect(
    pool: &Pool<Sqlite>,
    decision: &Decision,
    proof: DecisionProof,
    kind: &str,
) -> Result<EffectResult, GuardError> {
    if decision.kind != kind {
        return Err(GuardError::Invalid(format!(
            "outbox kind '{}' does not match decision kind '{}'",
            kind, decision.kind
        )));
    }
    let acted_by = proof.acted_by().to_string();
    match (kind, decision.answer.as_deref()) {
        ("approve_review", Some("approve")) => {
            let Some(goal_id) = decision.goal_id.as_deref() else {
                return Ok((None, None));
            };
            match goal_state(pool, goal_id).await?.as_deref() {
                Some("complete") => return already_applied("goal was already Complete"),
                Some("review") => {}
                None => return already_applied("goal was already gone"),
                Some(state) => {
                    return already_applied(format!(
                        "goal already advanced out of Review (current state: {state})"
                    ))
                }
            }
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
            let warning = match decision.project_id.as_deref() {
                Some(project_id) => {
                    goal_transition::promote_eligible_dependents_or_warn(pool, project_id, goal_id)
                        .await
                }
                None => None,
            };
            crate::recognition::write_back_decision_outcome(pool, goal_id, true).await;
            Ok((
                Some("goal approved: Review → Complete".to_string()),
                warning,
            ))
        }
        ("approve_review", Some("reject")) => {
            let Some(goal_id) = decision.goal_id.as_deref() else {
                return Ok((None, None));
            };
            if goal_state(pool, goal_id).await?.as_deref() != Some("review") {
                return already_applied("goal already advanced out of Review");
            }
            crate::recognition::write_back_decision_outcome(pool, goal_id, false).await;
            let card = crate::cards::get_card(pool, goal_id)
                .await
                .map_err(GuardError::Db)?
                .ok_or_else(|| GuardError::NotFound(format!("goal '{goal_id}' not found")))?;
            let attempt_count = card
                .metadata_json
                .get("attempt_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let budget = goal_transition::goal_budget(&card.metadata_json);
            if attempt_count + 1 >= budget.attempt_cap {
                if let Some(existing) =
                    decisions::find_open_decision_for_goal(pool, goal_id, "unblock")
                        .await
                        .map_err(GuardError::Db)?
                {
                    return already_applied(format!(
                        "goal already parked with unblock decision {}",
                        existing.id
                    ));
                }
                let reason = decision
                    .answer_note
                    .clone()
                    .unwrap_or_else(|| "Rejected after maximum attempts".to_string());
                let already_bumped = card
                    .metadata_json
                    .get("last_rejection_decision_id")
                    .and_then(|value| value.as_str())
                    == Some(decision.id.as_str());
                let spent = attempt_count + u64::from(!already_bumped);
                if !already_bumped {
                    sqlx::query(
                        "UPDATE cards
                         SET metadata_json = json_set(
                             metadata_json,
                             '$.attempt_count', ?,
                             '$.last_rejection_decision_id', ?
                         )
                         WHERE id = ? AND column_id IN (
                             SELECT id FROM board_columns WHERE state_binding = 'review'
                         )",
                    )
                    .bind(spent as i64)
                    .bind(&decision.id)
                    .bind(goal_id)
                    .execute(pool)
                    .await
                    .map_err(|e| GuardError::Db(format!("record rejected attempt: {e}")))?;
                }
                let unblock_id = goal_transition::exhaust_and_park(
                    pool,
                    goal_id,
                    &card.title,
                    &card.project_id,
                    goal_transition::BudgetExhaustion::AttemptCap {
                        spent,
                        cap: budget.attempt_cap,
                    },
                    Some(&reason),
                )
                .await
                .map_err(GuardError::Db)?;
                Ok((
                    Some(format!(
                        "goal rejected at attempt cap: parked with unblock decision {unblock_id}"
                    )),
                    None,
                ))
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
                already_applied("goal rejected: Review → InProgress for rework")
            }
        }
        ("unblock", Some("approve")) => {
            let Some(goal_id) = decision.goal_id.as_deref() else {
                return Ok((None, None));
            };
            match goal_state(pool, goal_id).await?.as_deref() {
                Some("ready" | "in_progress" | "review" | "complete") => {
                    return already_applied("goal was already unparked")
                }
                None => return already_applied("goal was already gone"),
                _ => {}
            }
            let card = crate::cards::get_card(pool, goal_id)
                .await
                .map_err(GuardError::Db)?
                .ok_or_else(|| GuardError::NotFound(format!("goal '{goal_id}' not found")))?;
            let attempt_count = card
                .metadata_json
                .get("attempt_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let mut budget = card
                .metadata_json
                .get("budget")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            let attempt_cap = attempt_count + goal_transition::DEFAULT_ATTEMPT_CAP;
            budget.insert("attempt_cap".to_string(), serde_json::json!(attempt_cap));
            let mut patch = serde_json::Map::new();
            patch.insert(
                "needs_human_attention".to_string(),
                serde_json::json!(false),
            );
            patch.insert("budget".to_string(), serde_json::Value::Object(budget));
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
            already_applied(format!(
                "goal unparked to Ready with attempt_cap raised to {attempt_cap}"
            ))
        }
        ("risk_gate", Some("approve"))
            if decision
                .payload
                .get("action_class")
                .and_then(|v| v.as_str())
                == Some("user_data_deletion")
                && decision.goal_id.is_some() =>
        {
            let goal_id = decision.goal_id.as_deref().expect("checked above");
            if goal_state(pool, goal_id).await?.is_none() {
                return already_applied(format!("goal {goal_id} was already gone"));
            }
            goal_transition::delete_goal_checked(pool, goal_id, proof).await?;
            already_applied(format!("goal {goal_id} deleted"))
        }
        ("automation_proposal", Some("reject")) => {
            let normalized = decision
                .payload
                .get("normalized_command")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if !normalized.is_empty() {
                crate::recognition::mark_observation_bounced(pool, normalized).await;
            }
            already_applied("automation proposal declined; will not re-pitch")
        }
        ("automation_proposal", Some("approve" | "edit")) => {
            already_applied(if decision.answer.as_deref() == Some("edit") {
                "automation proposal approved with edits"
            } else {
                "automation proposal approved"
            })
        }
        ("enrichment_proposal", Some("approve")) => {
            let payload: decisions::EnrichmentProposalPayload =
                serde_json::from_value(decision.payload.clone()).map_err(|e| {
                    GuardError::Invalid(format!("stored enrichment payload unreadable: {e}"))
                })?;
            let brain =
                crate::agents::platform_extensions::get_global_brain().ok_or_else(|| {
                    GuardError::Invalid(
                        "Brain is not available — cannot write enriched fields".to_string(),
                    )
                })?;
            let entity_id: spectral::core::entity_id::EntityId =
                payload.graph_entity_id.parse().map_err(|e| {
                    GuardError::Invalid(format!("invalid graph_entity_id in payload: {e:?}"))
                })?;
            let (mut applied, mut protected, mut skipped) = (0usize, 0usize, 0usize);
            for field in &payload.fields {
                if !crate::people::ENRICHABLE_FIELD_NAMES.contains(&field.field_name.as_str()) {
                    skipped += 1;
                    continue;
                }
                let wrote = brain
                    .set_entity_field(
                        entity_id,
                        &field.field_name,
                        &field.value,
                        spectral::ingest::FieldSource::Enriched,
                        Some(&field.source_url),
                    )
                    .await
                    .map_err(|e| {
                        GuardError::Db(format!("set_entity_field({}): {e}", field.field_name))
                    })?;
                if wrote {
                    applied += 1;
                } else {
                    protected += 1;
                }
            }
            let mut message = format!(
                "enrichment applied to \"{}\": {applied} field(s) written with Enriched \
                 provenance, {protected} protected by manual provenance",
                payload.person_name
            );
            if skipped > 0 {
                message.push_str(&format!(", {skipped} skipped (not enrichable)"));
            }
            already_applied(message)
        }
        ("enrichment_proposal", Some("reject")) => {
            already_applied("enrichment proposal declined; nothing was written")
        }
        ("project_intel_proposal", Some("approve")) => apply_project_intel(pool, decision).await,
        ("project_intel_proposal", Some("reject")) => {
            already_applied("project intelligence proposal declined; nothing was written")
        }
        ("file_to_project", Some("approve")) => apply_file_to_project(pool, decision).await,
        ("file_to_project", Some("reject")) => {
            already_applied("file-to-project proposal declined; nothing was persisted")
        }
        _ => crate::decision_inbox::policy::resume_answered_decision(pool, decision, proof)
            .await
            .map(|effect| (effect, None)),
    }
}

async fn apply_project_intel(
    pool: &Pool<Sqlite>,
    decision: &Decision,
) -> Result<EffectResult, GuardError> {
    let payload: decisions::ProjectIntelProposalPayload =
        serde_json::from_value(decision.payload.clone()).map_err(|e| {
            GuardError::Invalid(format!(
                "stored project intelligence payload unreadable: {e}"
            ))
        })?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| GuardError::Db(e.to_string()))?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?)")
        .bind(&payload.project_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| GuardError::Db(format!("check project exists: {e}")))?;
    if !exists {
        tx.rollback()
            .await
            .map_err(|e| GuardError::Db(e.to_string()))?;
        return already_applied(format!(
            "project \"{}\" no longer exists; nothing was written",
            payload.project_name
        ));
    }
    for item in &payload.items {
        let candidates: Vec<(String, String)> =
            sqlx::query_as("SELECT id, name FROM project_intel WHERE project_id = ? AND kind = ?")
                .bind(&payload.project_id)
                .bind(&item.kind)
                .fetch_all(&mut *tx)
                .await
                .map_err(|e| GuardError::Db(format!("deduplicate project_intel: {e}")))?;
        let item_name_folded = item.name.to_lowercase();
        for (id, name) in candidates {
            // Full Unicode case-fold, not eq_ignore_ascii_case: the latter only
            // folds ASCII A-Z, so accented names ("CAFÉ" vs "café") would slip
            // past and duplicate instead of updating in place.
            if name.to_lowercase() == item_name_folded {
                sqlx::query("DELETE FROM project_intel WHERE id = ?")
                    .bind(id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| GuardError::Db(format!("deduplicate project_intel: {e}")))?;
            }
        }
        sqlx::query(
            "INSERT INTO project_intel
             (id, project_id, kind, name, note, source_url, created_at)
             VALUES (?, ?, ?, ?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&payload.project_id)
        .bind(&item.kind)
        .bind(&item.name)
        .bind(&item.note)
        .bind(&item.source_url)
        .execute(&mut *tx)
        .await
        .map_err(|e| GuardError::Db(format!("insert project_intel: {e}")))?;
    }
    tx.commit()
        .await
        .map_err(|e| GuardError::Db(e.to_string()))?;
    already_applied(format!(
        "project intelligence applied to \"{}\": {} item(s) added/updated with source citations",
        payload.project_name,
        payload.items.len()
    ))
}

enum PersonOutcome {
    Created,
    AssociatedExisting,
}

async fn file_person_addressless(
    pool: &Pool<Sqlite>,
    brain: Option<&crate::brain_handle::SafeBrain>,
    project_id: &str,
    name: &str,
) -> Result<PersonOutcome, String> {
    let candidates = crate::people::list_people(
        pool,
        &crate::people::PeopleFilter {
            company: None,
            role: None,
            query: Some(name.to_string()),
        },
    )
    .await?;
    let exact: Vec<_> = candidates
        .iter()
        .filter(|person| person.display_name.eq_ignore_ascii_case(name))
        .collect();
    match exact.as_slice() {
        [person] => {
            crate::project_association::associate_person(
                pool,
                project_id,
                &person.entity_uuid,
                None,
            )
            .await?;
            Ok(PersonOutcome::AssociatedExisting)
        }
        [] => {
            let brain = brain.ok_or_else(|| {
                "Brain is not available — minting a person writes a graph entity; add them later \
                 with create_person"
                    .to_string()
            })?;
            let person = crate::people_create::create_person(
                pool,
                brain,
                name,
                crate::people_provenance::Provenance::Runtime,
            )
            .await?;
            crate::project_association::associate_person(
                pool,
                project_id,
                &person.entity_uuid,
                None,
            )
            .await?;
            Ok(PersonOutcome::Created)
        }
        people => Err(format!(
            "matches {} people in the directory — not added (never guess an identity)",
            people.len()
        )),
    }
}

async fn apply_file_to_project(
    pool: &Pool<Sqlite>,
    decision: &Decision,
) -> Result<EffectResult, GuardError> {
    let payload: decisions::FileToProjectPayload = serde_json::from_value(decision.payload.clone())
        .map_err(|e| {
            GuardError::Invalid(format!("stored file_to_project payload unreadable: {e}"))
        })?;
    let project = crate::projects::get_project_by_id_or_slug(pool, &payload.project_id)
        .await
        .map_err(GuardError::Db)?
        .ok_or_else(|| {
            GuardError::NotFound(format!(
                "project '{}' ('{}') no longer exists — nothing was filed",
                payload.project_id, payload.project_name
            ))
        })?;
    let note_id = format!("decision-note:{}", decision.id);
    let brain = crate::agents::platform_extensions::get_global_brain();
    let note = if let Some(note) = crate::project_notes::get_note(pool, &project.id, &note_id)
        .await
        .map_err(GuardError::Db)?
    {
        note
    } else {
        crate::project_notes::create_note_indexed_with_id(
            pool,
            brain.as_ref(),
            &note_id,
            &project.id,
            payload.title.as_deref(),
            &payload.body,
        )
        .await
        .map_err(GuardError::Db)?
    };
    let (mut added, mut associated) = (0usize, 0usize);
    let mut warnings = Vec::new();
    for name in &payload.people {
        match file_person_addressless(pool, brain.as_ref(), &project.id, name).await {
            Ok(PersonOutcome::Created) => added += 1,
            Ok(PersonOutcome::AssociatedExisting) => associated += 1,
            Err(error) => warnings.push(format!("{name}: {error}")),
        }
    }
    let mut effect = format!(
        "filed note {} to project \"{}\"{}",
        note.id,
        project.name,
        if note.memory_key.is_some() {
            " (indexed into the Brain)"
        } else {
            " (Brain index skipped — note row persisted)"
        }
    );
    if added + associated > 0 {
        effect.push_str(&format!(
            "; people: {added} added, {associated} already in the directory — all address-less"
        ));
    }
    let warning = (!warnings.is_empty()).then(|| {
        format!(
            "note filed, but {} people step(s) did not apply: {}",
            warnings.len(),
            warnings.join("; ")
        )
    });
    Ok((Some(effect), warning))
}

/// Claim and apply a bounded batch of due effects.
pub async fn drain_effect_outbox(pool: &Pool<Sqlite>) -> Result<(), String> {
    sqlx::query(
        "UPDATE effect_outbox
         SET status = CASE WHEN attempts + 1 >= max_attempts THEN 'dead' ELSE 'pending' END,
             attempts = attempts + 1,
             last_error = 'effect claim lease expired before completion',
             next_attempt_at = strftime(
                 '%Y-%m-%dT%H:%M:%fZ', 'now',
                 '+' || MIN(3600, (1 << MIN(attempts, 12))) || ' seconds'
             ),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE status = 'running'
           AND updated_at <= strftime('%Y-%m-%dT%H:%M:%fZ','now', ?)",
    )
    .bind(format!("-{RUNNING_LEASE_SECONDS} seconds"))
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    let rows = sqlx::query(
        "SELECT id, decision_id, kind FROM effect_outbox
         WHERE status = 'pending' AND next_attempt_at <= strftime('%Y-%m-%dT%H:%M:%fZ','now')
         ORDER BY next_attempt_at, id LIMIT ?",
    )
    .bind(OUTBOX_BATCH_SIZE)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    for row in rows {
        let id: String = row.get("id");
        let decision_id: Option<String> = row.get("decision_id");
        let kind: String = row.get("kind");
        let claimed = sqlx::query(
            "UPDATE effect_outbox SET status = 'running',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE id = ? AND status = 'pending'",
        )
        .bind(&id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
        if claimed.rows_affected() == 0 {
            continue;
        }
        let result = async {
            let decision_id =
                decision_id.ok_or_else(|| "effect outbox row has no decision_id".to_string())?;
            let decision = decisions::get_decision(pool, &decision_id)
                .await?
                .ok_or_else(|| format!("decision '{decision_id}' no longer exists"))?;
            let proof = DecisionProof::from_answered_row(&decision)
                .ok_or_else(|| format!("decision '{decision_id}' is not an answered row"))?;
            apply_decision_effect(pool, &decision, proof, &kind)
                .await
                .map_err(|e| e.to_string())
        }
        .await;
        match result {
            Ok(_) => {
                sqlx::query(
                    "UPDATE effect_outbox SET status = 'applied', attempts = attempts + 1,
                         last_error = NULL,
                         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
                     WHERE id = ? AND status = 'running'",
                )
                .bind(&id)
                .execute(pool)
                .await
                .map_err(|e| e.to_string())?;
            }
            Err(error) => {
                sqlx::query(
                    "UPDATE effect_outbox
                     SET status = CASE
                             WHEN attempts + 1 >= max_attempts THEN 'dead' ELSE 'pending' END,
                         attempts = attempts + 1, last_error = ?,
                         next_attempt_at = strftime(
                             '%Y-%m-%dT%H:%M:%fZ', 'now',
                             '+' || MIN(3600, (1 << MIN(attempts, 12))) || ' seconds'
                         ),
                         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
                     WHERE id = ? AND status = 'running'",
                )
                .bind(&error)
                .bind(&id)
                .execute(pool)
                .await
                .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decisions::{DecisionAnswer, NewDecision, ACTOR_JESSE};
    use crate::projects::PERSONAL_PROJECT_ID;
    use crate::session::spectral_schema::init_spectral_db;

    async fn test_pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    async fn answered_file_decision(pool: &Pool<Sqlite>) -> Decision {
        let decision = decisions::create_decision(
            pool,
            NewDecision {
                kind: "file_to_project".to_string(),
                project_id: Some(PERSONAL_PROJECT_ID.to_string()),
                headline: Some("File these notes to Personal".to_string()),
                detail: Some("One reviewed note".to_string()),
                payload: serde_json::json!({
                    "project_id": PERSONAL_PROJECT_ID,
                    "project_name": "Personal",
                    "title": "Retry-safe note",
                    "body": "This body must be filed exactly once.",
                    "content_origin": "test fixture",
                    "people": []
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(decision.kind, "file_to_project");
        decisions::answer_decision(
            pool,
            &decision.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap()
        .0
    }

    #[tokio::test]
    async fn file_to_project_replay_creates_exactly_one_note() {
        let pool = test_pool().await;
        let decision = answered_file_decision(&pool).await;
        for _ in 0..2 {
            let proof = DecisionProof::from_answered_row(&decision).unwrap();
            apply_decision_effect(&pool, &decision, proof, &decision.kind)
                .await
                .unwrap();
        }
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM project_notes WHERE project_id = ?")
                .bind(PERSONAL_PROJECT_ID)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn worker_claims_and_applies_a_pending_effect() {
        let pool = test_pool().await;
        let decision = answered_file_decision(&pool).await;
        drain_effect_outbox(&pool).await.unwrap();
        let row: (String, i64) =
            sqlx::query_as("SELECT status, attempts FROM effect_outbox WHERE decision_id = ?")
                .bind(&decision.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row, ("applied".to_string(), 1));
        assert_eq!(
            crate::project_notes::list_notes(&pool, PERSONAL_PROJECT_ID)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn transient_failure_is_retried_and_effect_applies() {
        let pool = test_pool().await;
        let missing_project = "retry-project";
        // The `project_id` *column* FK-references projects(id) ON DELETE CASCADE,
        // so it must stay NULL here: the target project does not exist yet. The
        // effect's target lives in the payload, which is exactly what
        // apply_file_to_project reads — so the first drain fails NotFound
        // (retriable → pending) and a later drain applies it once the project
        // appears. This exercises the retry-then-apply loop without the column FK
        // rejecting a decision that references a not-yet-created project.
        let decision = decisions::create_decision(
            &pool,
            NewDecision {
                kind: "file_to_project".to_string(),
                headline: Some("File a retryable note".to_string()),
                detail: Some("The project will appear before retry".to_string()),
                payload: serde_json::json!({
                    "project_id": missing_project,
                    "project_name": "Retry Project",
                    "body": "Persist after the transient failure.",
                    "content_origin": "test fixture",
                    "people": []
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (decision, _) = decisions::answer_decision(
            &pool,
            &decision.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap();

        drain_effect_outbox(&pool).await.unwrap();
        let status: String =
            sqlx::query_scalar("SELECT status FROM effect_outbox WHERE decision_id = ?")
                .bind(&decision.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "pending");

        sqlx::query(
            "INSERT INTO projects (id, user_id, slug, name, description, status)
             VALUES (?, 'default', 'retry-project', 'Retry Project', '', 'active')",
        )
        .bind(missing_project)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE effect_outbox
             SET next_attempt_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE decision_id = ?",
        )
        .bind(&decision.id)
        .execute(&pool)
        .await
        .unwrap();

        drain_effect_outbox(&pool).await.unwrap();
        let status: String =
            sqlx::query_scalar("SELECT status FROM effect_outbox WHERE decision_id = ?")
                .bind(&decision.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "applied");
        assert_eq!(
            crate::project_notes::list_notes(&pool, missing_project)
                .await
                .unwrap()
                .len(),
            1
        );
    }
}
