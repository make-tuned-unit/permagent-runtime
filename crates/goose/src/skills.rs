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
    pub usage_count: i64,
    pub status: String,
    pub created_at: String,
}

/// A pending skill proposal from the repetition_candidates view.
pub struct SkillProposal {
    pub tool_used: String,
    pub argument_shape_hash: String,
    pub occurrence_count: i64,
    pub latest_description: String,
    pub source_task_ids: Vec<String>,
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

/// List all skills for the default user, including post-save usage count.
pub async fn list_skills(pool: &Pool<Sqlite>) -> Result<Vec<SkillSummary>, String> {
    let rows = sqlx::query(
        "SELECT s.id, s.name, s.description,
                json_extract(s.trigger_value, '$.tool_used') as tool_used,
                s.status, s.created_at,
                COUNT(DISTINCT st.id) as trigger_count,
                MAX(st.last_triggered_at) as last_triggered_at,
                (SELECT COUNT(*) FROM skill_executions se WHERE se.skill_id = s.id) as usage_count
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
            usage_count: row.get("usage_count"),
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

/// List pending skill proposals (patterns detected but not yet saved or dismissed).
/// Only returns patterns with 2-10 occurrences — above 10 is generic tool usage.
pub async fn list_proposals(
    pool: &Pool<Sqlite>,
    threshold: i64,
) -> Result<Vec<SkillProposal>, String> {
    let rows = sqlx::query(
        "SELECT rc.tool_used, rc.argument_shape_hash, rc.occurrence_count, rc.latest_description
         FROM repetition_candidates rc
         WHERE rc.user_id = 'default'
           AND rc.occurrence_count >= ?
           AND rc.occurrence_count <= 10
           AND NOT EXISTS (
               SELECT 1 FROM skills s
               WHERE s.user_id = 'default'
                 AND s.trigger_value LIKE '%' || rc.argument_shape_hash || '%'
                 AND s.status != 'archived'
           )
           AND NOT EXISTS (
               SELECT 1 FROM skill_dismissals sd
               WHERE sd.user_id = 'default'
                 AND sd.argument_shape_hash = rc.argument_shape_hash
                 AND sd.tool_used != '__agent_surfaced'
                 AND sd.dismissed_at >= strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-30 days')
           )",
    )
    .bind(threshold)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut proposals = Vec::new();
    for row in &rows {
        let tool_used: String = row.get("tool_used");
        let shape_hash: String = row.get("argument_shape_hash");
        let count: i64 = row.get("occurrence_count");
        let description: String = row.get("latest_description");

        let task_ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM tasks
             WHERE user_id = 'default'
               AND tool_used = ?
               AND argument_shape_hash = ?
               AND status = 'completed'
             ORDER BY completed_at DESC
             LIMIT 5",
        )
        .bind(&tool_used)
        .bind(&shape_hash)
        .fetch_all(pool)
        .await
        .unwrap_or_default();

        proposals.push(SkillProposal {
            tool_used,
            argument_shape_hash: shape_hash,
            occurrence_count: count,
            latest_description: description,
            source_task_ids: task_ids,
        });
    }

    Ok(proposals)
}

/// Record a skill execution (post-hoc match by tool_used + argument_shape_hash).
pub async fn record_execution(
    pool: &Pool<Sqlite>,
    tool_used: &str,
    argument_shape_hash: &str,
    session_id: Option<&str>,
) -> Result<(), String> {
    // Find the matching skill
    let skill_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM skills
         WHERE user_id = 'default'
           AND trigger_value LIKE '%' || ? || '%'
           AND status != 'archived'
         LIMIT 1",
    )
    .bind(argument_shape_hash)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    let skill_id = match skill_id {
        Some(id) => id,
        None => return Ok(()), // No matching skill — nothing to record
    };

    let exec_id = Uuid::now_v7().to_string();
    let input = serde_json::json!({
        "tool_used": tool_used,
        "argument_shape_hash": argument_shape_hash,
    })
    .to_string();

    sqlx::query(
        "INSERT INTO skill_executions (id, skill_id, user_id, session_id, status, input_json, completed_at)
         VALUES (?, ?, 'default', ?, 'completed', ?, strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
    )
    .bind(&exec_id)
    .bind(&skill_id)
    .bind(session_id)
    .bind(&input)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Build the saved-skills prompt fragment for agent system prompt injection.
/// Returns None if no saved skills exist.
pub async fn build_skills_prompt(pool: &Pool<Sqlite>) -> Result<Option<String>, String> {
    let rows = sqlx::query(
        "SELECT s.name, s.description, s.definition_json
         FROM skills s
         WHERE s.user_id = 'default' AND s.status = 'active'
         ORDER BY s.created_at DESC
         LIMIT 10",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    if rows.is_empty() {
        return Ok(None);
    }

    let mut lines = vec![
        "## Saved Skills".to_string(),
        "You have the following saved approaches from patterns the user has confirmed.".to_string(),
        "When the user's request matches one of these skills, use the saved approach and"
            .to_string(),
        "mention \"Using saved skill: [name]\" so the user knows you're reusing learned behavior."
            .to_string(),
        String::new(),
    ];

    for row in &rows {
        let name: String = row.get("name");
        let desc: Option<String> = row.get("description");
        let def_str: String = row.get("definition_json");
        let desc_text = desc.unwrap_or_default();
        // Extract a brief summary from definition_json if it contains useful info
        let def_summary = if let Ok(def) = serde_json::from_str::<serde_json::Value>(&def_str) {
            if let Some(obj) = def.as_object() {
                obj.keys().take(3).cloned().collect::<Vec<_>>().join(", ")
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let mut skill_line = format!("- **{}**", name);
        if !desc_text.is_empty() {
            skill_line.push_str(&format!(": {}", desc_text));
        }
        if !def_summary.is_empty() {
            skill_line.push_str(&format!(" (context: {})", def_summary));
        }
        lines.push(skill_line);
    }

    Ok(Some(lines.join("\n")))
}
