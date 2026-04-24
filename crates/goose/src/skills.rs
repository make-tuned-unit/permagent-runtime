//! Skills CRUD — database operations for the auto-skills detection pipeline (Section F).
//!
//! Route handlers in `permagent-daemon` delegate to these functions so the
//! server crate does not need a direct `sqlx` dependency.

use crate::events;
use sqlx::{Pool, Row, Sqlite};
use uuid::Uuid;

// ── Data types ─────────────────────────────────────────────────────────────

/// Request payload for creating a skill from a detected pattern.
pub struct CreateSkillParams {
    pub name: String,
    pub description: Option<String>,
    pub tool_used: String,
    pub argument_shape_hash: String,
    pub definition_json: serde_json::Value,
    pub source_task_id: Option<String>,
}

/// Returned after successfully creating a skill.
pub struct CreatedSkill {
    pub id: String,
    pub name: String,
}

/// Summary for listing skills.
pub struct SkillSummary {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub tool_used: Option<String>,
    pub trigger_count: i64,
    pub last_triggered_at: Option<String>,
    pub status: String,
    pub created_at: String,
}

/// Full detail for a single skill.
pub struct SkillDetail {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub tool_used: Option<String>,
    pub definition_json: serde_json::Value,
    pub trigger_type: String,
    pub trigger_value: Option<String>,
    pub status: String,
    pub version: i32,
    pub source_task_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

// ── Operations ─────────────────────────────────────────────────────────────

/// Create a skill and its trigger row. Emits `SkillSaved` event.
pub async fn create_skill(
    pool: &Pool<Sqlite>,
    params: CreateSkillParams,
) -> Result<CreatedSkill, String> {
    let skill_id = Uuid::now_v7().to_string();
    let trigger_id = Uuid::now_v7().to_string();
    let definition_str =
        serde_json::to_string(&params.definition_json).map_err(|e| e.to_string())?;
    let trigger_value = serde_json::json!({
        "tool_used": params.tool_used,
        "argument_shape_hash": params.argument_shape_hash,
    })
    .to_string();

    sqlx::query(
        "INSERT INTO skills (id, user_id, name, description, definition_json, trigger_type, trigger_value, source_task_id)
         VALUES (?, 'default', ?, ?, ?, 'repetition', ?, ?)",
    )
    .bind(&skill_id)
    .bind(&params.name)
    .bind(&params.description)
    .bind(&definition_str)
    .bind(&trigger_value)
    .bind(&params.source_task_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    let trigger_config = serde_json::json!({
        "tool_used": params.tool_used,
        "argument_shape_hash": params.argument_shape_hash,
    })
    .to_string();

    sqlx::query(
        "INSERT INTO skill_triggers (id, skill_id, trigger_type, trigger_config)
         VALUES (?, ?, 'repetition', ?)",
    )
    .bind(&trigger_id)
    .bind(&skill_id)
    .bind(&trigger_config)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    events::emit(events::skill_saved(&skill_id, &params.name, "repetition"));

    Ok(CreatedSkill {
        id: skill_id,
        name: params.name,
    })
}

/// List all skills for the default user.
pub async fn list_skills(pool: &Pool<Sqlite>) -> Result<Vec<SkillSummary>, String> {
    let rows = sqlx::query(
        "SELECT s.id, s.name, s.description,
                json_extract(s.trigger_value, '$.tool_used') as tool_used,
                s.status, s.created_at,
                COUNT(st.id) as trigger_count,
                MAX(st.last_triggered_at) as last_triggered_at
         FROM skills s
         LEFT JOIN skill_triggers st ON st.skill_id = s.id
         WHERE s.user_id = 'default'
         GROUP BY s.id
         ORDER BY s.created_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|row| SkillSummary {
            id: row.get("id"),
            name: row.get("name"),
            description: row.get("description"),
            tool_used: row.get("tool_used"),
            trigger_count: row.get("trigger_count"),
            last_triggered_at: row.get("last_triggered_at"),
            status: row.get("status"),
            created_at: row.get("created_at"),
        })
        .collect())
}

/// Get full detail for a single skill.
pub async fn get_skill(pool: &Pool<Sqlite>, skill_id: &str) -> Result<Option<SkillDetail>, String> {
    let row = sqlx::query(
        "SELECT id, name, description,
                json_extract(trigger_value, '$.tool_used') as tool_used,
                definition_json, trigger_type,
                trigger_value, status, version, source_task_id, created_at, updated_at
         FROM skills
         WHERE id = ? AND user_id = 'default'",
    )
    .bind(skill_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(row.map(|r| {
        let def_str: String = r.get("definition_json");
        let definition_json: serde_json::Value =
            serde_json::from_str(&def_str).unwrap_or(serde_json::Value::Null);
        SkillDetail {
            id: r.get("id"),
            name: r.get("name"),
            description: r.get("description"),
            tool_used: r.get("tool_used"),
            definition_json,
            trigger_type: r.get("trigger_type"),
            trigger_value: r.get("trigger_value"),
            status: r.get("status"),
            version: r.get("version"),
            source_task_id: r.get("source_task_id"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        }
    }))
}

/// Delete a skill (and cascade to skill_triggers).
pub async fn delete_skill(pool: &Pool<Sqlite>, skill_id: &str) -> Result<bool, String> {
    let result = sqlx::query("DELETE FROM skills WHERE id = ? AND user_id = 'default'")
        .bind(skill_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(result.rows_affected() > 0)
}

/// Record a dismissal for an argument shape hash (prevents re-prompting for 30 days).
pub async fn dismiss_skill(pool: &Pool<Sqlite>, argument_shape_hash: &str) -> Result<(), String> {
    let id = Uuid::now_v7().to_string();

    sqlx::query(
        "INSERT INTO skill_dismissals (id, user_id, argument_shape_hash)
         VALUES (?, 'default', ?)",
    )
    .bind(&id)
    .bind(argument_shape_hash)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}
