//! Agent-to-agent messaging between goal workers (Prime A2A).
//!
//! Payload shape: `{ from_goal, to_goal, body, ts }`. Delivery is refused
//! unless the target is InProgress. The message is written to the target's
//! `a2a_inbox` metadata, the RLM control plane, and (when a live steerable
//! worker exists) `steer_goal`.

use chrono::Utc;
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::{Pool, Sqlite};

use crate::cards;
use crate::rlm;

pub const A2A_INBOX_KEY: &str = "a2a_inbox";
pub const A2A_SENT_KEY: &str = "a2a_sent";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct A2aMessage {
    pub from_goal: String,
    pub to_goal: String,
    pub body: String,
    pub ts: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct A2aDelivery {
    pub steered: bool,
    pub message: A2aMessage,
}

pub async fn send_goal_a2a(
    pool: &Pool<Sqlite>,
    from_goal: &str,
    to_goal: &str,
    body: &str,
) -> Result<A2aDelivery, String> {
    let body = body.trim();
    if body.is_empty() {
        return Err("A2A body is empty".into());
    }
    if from_goal == to_goal {
        return Err("cannot message a goal from itself".into());
    }

    let from = cards::get_card(pool, from_goal)
        .await?
        .ok_or_else(|| format!("from_goal '{from_goal}' not found"))?;
    let to = cards::get_card(pool, to_goal)
        .await?
        .ok_or_else(|| format!("to_goal '{to_goal}' not found"))?;
    if from.card_type != "goal" || to.card_type != "goal" {
        return Err("A2A is only allowed between goal cards".into());
    }

    let to_col = cards::get_column(pool, &to.column_id)
        .await?
        .ok_or_else(|| format!("column for '{to_goal}' not found"))?;
    let binding = to_col.state_binding.as_deref().unwrap_or(&to_col.name);
    if binding != "in_progress" {
        return Err(format!(
            "messaging a non-InProgress target is refused (goal '{to_goal}' is '{binding}')"
        ));
    }

    let msg = A2aMessage {
        from_goal: from_goal.to_string(),
        to_goal: to_goal.to_string(),
        body: body.to_string(),
        ts: Utc::now().to_rfc3339(),
    };
    let value = serde_json::to_value(&msg).map_err(|e| e.to_string())?;

    append_meta_array(pool, to_goal, A2A_INBOX_KEY, value.clone()).await?;
    append_meta_array(pool, from_goal, A2A_SENT_KEY, value.clone()).await?;

    let key = rlm::session_key_for_goal(to_goal);
    rlm::hydrate_from_metadata(&key, &to.metadata_json);
    rlm::set(&key, "a2a_feedback", value.clone());
    persist_rlm_snapshot(pool, to_goal, &key).await?;

    let steered = if let Some(handle) =
        crate::agents::platform_extensions::orchestrator::steer_handle_for(to_goal)
    {
        match handle.steer(&format!("A2A from {from_goal}: {body}")).await {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(
                    target: "permagentd::a2a",
                    to_goal,
                    "steer failed after A2A persist: {e}"
                );
                false
            }
        }
    } else {
        false
    };

    Ok(A2aDelivery {
        steered,
        message: msg,
    })
}

async fn persist_rlm_snapshot(
    pool: &Pool<Sqlite>,
    card_id: &str,
    session_key: &str,
) -> Result<(), String> {
    let card = cards::get_card(pool, card_id)
        .await?
        .ok_or_else(|| format!("card '{card_id}' vanished"))?;
    let mut meta = card.metadata_json.as_object().cloned().unwrap_or_default();
    meta.insert("rlm_state".into(), rlm::snapshot(session_key));
    sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
        .bind(serde_json::to_string(&Value::Object(meta)).map_err(|e| e.to_string())?)
        .bind(card_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

async fn append_meta_array(
    pool: &Pool<Sqlite>,
    card_id: &str,
    key: &str,
    item: Value,
) -> Result<(), String> {
    let card = cards::get_card(pool, card_id)
        .await?
        .ok_or_else(|| format!("card '{card_id}' vanished"))?;
    let mut meta = card.metadata_json.as_object().cloned().unwrap_or_default();
    let mut arr = meta
        .remove(key)
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    arr.push(item);
    const CAP: usize = 20;
    if arr.len() > CAP {
        arr.drain(0..arr.len() - CAP);
    }
    meta.insert(key.to_string(), json!(arr));
    sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
        .bind(serde_json::to_string(&Value::Object(meta)).map_err(|e| e.to_string())?)
        .bind(card_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::PERSONAL_PROJECT_ID;
    use crate::session::spectral_schema::init_spectral_db;

    async fn pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    async fn goal(pool: &Pool<Sqlite>, state: &str) -> cards::Card {
        cards::seed_goal_columns(pool, PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        let col = cards::get_goal_column(pool, PERSONAL_PROJECT_ID, state)
            .await
            .unwrap()
            .unwrap();
        cards::create_card(
            pool,
            cards::CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: format!("goal-{state}"),
                description: Some("t".into()),
                card_type: Some("goal".into()),
                column_id: Some(col.id),
                created_by: None,
                metadata_json: Some(json!({"goal_state": state, "attempt_count": 1})),
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn deliver_to_in_progress_and_refuse_complete() {
        let pool = pool().await;
        let from = goal(&pool, "in_progress").await;
        let to = goal(&pool, "in_progress").await;
        let done = goal(&pool, "complete").await;

        let ok = send_goal_a2a(&pool, &from.id, &to.id, "watch the race")
            .await
            .unwrap();
        assert_eq!(ok.message.from_goal, from.id);
        assert_eq!(ok.message.to_goal, to.id);
        assert!(!ok.steered, "no live CLI worker in this unit test");

        let updated = cards::get_card(&pool, &to.id).await.unwrap().unwrap();
        let inbox = updated
            .metadata_json
            .get(A2A_INBOX_KEY)
            .and_then(|v| v.as_array())
            .expect("inbox");
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0]["body"], "watch the race");
        assert!(updated.metadata_json.get("rlm_state").is_some());
        assert_eq!(
            rlm::get(&rlm::session_key_for_goal(&to.id), "a2a_feedback").unwrap()["body"],
            "watch the race"
        );

        let err = send_goal_a2a(&pool, &from.id, &done.id, "too late")
            .await
            .unwrap_err();
        assert!(
            err.contains("non-InProgress") || err.contains("complete"),
            "{err}"
        );
    }
}
