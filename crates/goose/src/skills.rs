//! Skills CRUD — database operations for the auto-skills detection pipeline (Section F).
//!
//! Route handlers in `permagent-daemon` delegate to these functions so the
//! server crate does not need a direct `sqlx` dependency.

use crate::events;
use sqlx::{Pool, Row, Sqlite};
use std::path::PathBuf;
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

    // Write the portable on-disk `SKILL.md` folder (the source-of-truth format)
    // and record its path in the index. Non-fatal: the DB row is the durable
    // index and the on-boot `reconcile_skills_to_disk` re-attempts any skill
    // missing its folder, so a transient filesystem error never loses the skill
    // or breaks the save_skill / repetition loop.
    if let Err(e) = export_skill_to_disk(pool, &skill_id).await {
        tracing::warn!(
            "skill '{}' indexed but on-disk SKILL.md export failed (will retry on boot): {}",
            params.name,
            e
        );
    }

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

/// Update a skill's editable fields (name and/or description). A `None` field is
/// left unchanged; `Some` overwrites it. Returns false when no skill matched (an
/// unknown id or another user's skill) so callers can surface the failure rather
/// than pretend it saved. Bumps `updated_at`.
pub async fn update_skill(
    pool: &Pool<Sqlite>,
    skill_id: &str,
    name: Option<&str>,
    description: Option<&str>,
) -> Result<bool, String> {
    let result = sqlx::query(
        "UPDATE skills
         SET name = COALESCE(?, name),
             description = COALESCE(?, description),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE id = ? AND user_id = 'default'",
    )
    .bind(name)
    .bind(description)
    .bind(skill_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(result.rows_affected() > 0)
}

/// One recorded run of a skill, for the execution-history panel.
pub struct SkillExecution {
    pub id: String,
    pub status: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub error_message: Option<String>,
}

/// List a skill's executions, newest first (capped). Empty when the skill has
/// never run.
pub async fn list_skill_executions(
    pool: &Pool<Sqlite>,
    skill_id: &str,
) -> Result<Vec<SkillExecution>, String> {
    let rows = sqlx::query(
        "SELECT id, status, started_at, completed_at, error_message
         FROM skill_executions
         WHERE skill_id = ? AND user_id = 'default'
         ORDER BY started_at DESC
         LIMIT 100",
    )
    .bind(skill_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| SkillExecution {
            id: r.get("id"),
            status: r.get("status"),
            started_at: r.get("started_at"),
            completed_at: r.get("completed_at"),
            error_message: r.get("error_message"),
        })
        .collect())
}

// ── Evidence-based ranking (graduation) ──────────────────────────────────────

/// A saved skill plus the evidence needed to rank it for prompt injection.
/// `execution_count` is the number of recorded `skill_executions` rows — the
/// proven-usage signal that graduates a skill ahead of never-used ones.
pub struct RankedSkill {
    pub name: String,
    pub description: Option<String>,
    pub definition_json: String,
    pub execution_count: i64,
    pub created_at: String,
    /// Path to the on-disk `SKILL.md` folder (the portable source-of-truth).
    /// `None` for skills not yet exported to disk (falls back to the indexed
    /// name/description).
    pub skill_path: Option<String>,
}

/// Rank saved skills for the limited prompt slots: proven usage first (highest
/// `execution_count`), ties broken by recency (newest `created_at`). Returns at
/// most `limit` skills. Pure and DB-free so the ranking policy is unit-testable
/// in isolation. `created_at` is an ISO-8601 UTC string whose lexicographic
/// order matches chronological order.
pub fn rank_skills_for_prompt(mut skills: Vec<RankedSkill>, limit: usize) -> Vec<RankedSkill> {
    skills.sort_by(|a, b| {
        b.execution_count
            .cmp(&a.execution_count)
            .then_with(|| b.created_at.cmp(&a.created_at))
    });
    skills.truncate(limit);
    skills
}

/// Execution count at which a saved skill is considered "proven" (graduated).
/// Mirrors the command-center's TRUSTED tier (usageCount >= 3) so the backend's
/// graduation notion and the UI's tier badges agree.
pub const PROVEN_EXECUTION_THRESHOLD: i64 = 3;

/// Graduation predicate: a skill is "proven" once it has fired at least
/// [`PROVEN_EXECUTION_THRESHOLD`] times. Pure and unit-testable.
pub fn skill_is_proven(execution_count: i64) -> bool {
    execution_count >= PROVEN_EXECUTION_THRESHOLD
}

/// Build the saved-skills prompt fragment for agent system prompt injection.
/// Skills are ranked by PROVEN USAGE (execution count) first, then recency, so
/// the limited prompt slots go to skills the user actually relies on rather than
/// whatever happened to be created most recently. Returns None if no active
/// skills exist.
pub async fn build_skills_prompt(pool: &Pool<Sqlite>) -> Result<Option<String>, String> {
    let rows = sqlx::query(
        "SELECT s.name, s.description, s.definition_json, s.created_at, s.skill_path,
                (SELECT COUNT(*) FROM skill_executions se WHERE se.skill_id = s.id)
                    AS execution_count
         FROM skills s
         WHERE s.user_id = 'default' AND s.status = 'active'",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    if rows.is_empty() {
        return Ok(None);
    }

    let candidates: Vec<RankedSkill> = rows
        .iter()
        .map(|row| RankedSkill {
            name: row.get("name"),
            description: row.get("description"),
            definition_json: row.get("definition_json"),
            execution_count: row.get("execution_count"),
            created_at: row.get("created_at"),
            skill_path: row.get("skill_path"),
        })
        .collect();

    // Proven-usage-first ordering, capped at the prompt budget.
    let ranked = rank_skills_for_prompt(candidates, 10);

    // Progressive disclosure (agentskills.io): inject only each skill's name +
    // description here; the full body loads on demand via the `load_skill` tool.
    let mut lines = vec![
        "## Saved Skills".to_string(),
        "These are saved approaches from patterns the user has confirmed, stored as portable"
            .to_string(),
        "SKILL.md folders. When the user's request matches one, load its full steps with the"
            .to_string(),
        "`load_skill` tool (name shown below), follow the saved approach, and mention".to_string(),
        "\"Using saved skill: [name]\" so the user knows you're reusing learned behavior."
            .to_string(),
        String::new(),
    ];

    for skill in &ranked {
        // Prefer the on-disk SKILL.md description (the source of truth); fall back
        // to the indexed description when the folder is missing (e.g. a skill not
        // yet exported, or the reconcile hasn't run).
        let on_disk = skill
            .skill_path
            .as_deref()
            .filter(|p| !p.is_empty())
            .and_then(|p| crate::skill_md::read_skill_folder(std::path::Path::new(p)).ok());

        let desc_text = on_disk
            .as_ref()
            .map(|parsed| parsed.meta.description.clone())
            .or_else(|| skill.description.clone())
            .unwrap_or_default();
        // The load_skill handle is the on-disk skill name (its folder/slug), which
        // may differ from the human-facing index name.
        let load_name = on_disk
            .as_ref()
            .map(|parsed| parsed.meta.name.clone())
            .unwrap_or_else(|| skill.name.clone());

        let mut skill_line = format!("- **{}**", skill.name);
        if !desc_text.is_empty() {
            skill_line.push_str(&format!(": {}", desc_text));
        }
        // Graduation marker: signal battle-tested skills so the agent can lean on
        // proven approaches over ones that merely exist.
        if skill_is_proven(skill.execution_count) {
            skill_line.push_str(" [proven]");
        }
        skill_line.push_str(&format!(" — `load_skill(name: \"{}\")`", load_name));
        lines.push(skill_line);
    }

    Ok(Some(lines.join("\n")))
}

// ── Retirement sweep (grow-and-trim) ─────────────────────────────────────────

/// Default grace window (days) before a never-used skill is retired. Mirrors the
/// 30-day window used for `skill_dismissals` (see [`dismiss_skill`] /
/// [`list_proposals`]) so the auto-skills policy stays consistent.
pub const RETIREMENT_GRACE_DAYS: i64 = 30;

/// A skill archived by the retirement sweep.
pub struct RetiredSkill {
    pub id: String,
    pub name: String,
}

/// Retirement predicate: a saved skill is stale when it has NEVER executed AND it
/// was created before the grace-window cutoff. A skill with ANY executions is
/// never retired. Pure and DB-free so the policy is unit-testable. `created_at`
/// and `cutoff` are ISO-8601 UTC strings (lexicographic == chronological order).
pub fn skill_is_stale(execution_count: i64, created_at: &str, cutoff: &str) -> bool {
    execution_count == 0 && created_at < cutoff
}

/// Retirement sweep: archive active skills that have never fired after the grace
/// window, emitting a `SkillRetired` event for each. Never touches a skill with
/// any executions. Returns the skills retired on this pass. Idempotent — an
/// already-archived skill is skipped, so the event fires at most once per skill.
pub async fn retire_stale_skills(
    pool: &Pool<Sqlite>,
    grace_days: i64,
) -> Result<Vec<RetiredSkill>, String> {
    // Compute the cutoff with SQLite's own clock so it matches the stored
    // `created_at` format exactly (mirrors the `-30 days` window in list_proposals).
    let cutoff: String = sqlx::query_scalar("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?)")
        .bind(format!("-{grace_days} days"))
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;

    let rows = sqlx::query(
        "SELECT s.id, s.name, s.created_at,
                (SELECT COUNT(*) FROM skill_executions se WHERE se.skill_id = s.id)
                    AS execution_count
         FROM skills s
         WHERE s.user_id = 'default' AND s.status = 'active'",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut retired = Vec::new();
    for row in &rows {
        let execution_count: i64 = row.get("execution_count");
        let created_at: String = row.get("created_at");
        if !skill_is_stale(execution_count, &created_at, &cutoff) {
            continue;
        }

        let id: String = row.get("id");
        let name: String = row.get("name");
        let result = sqlx::query(
            "UPDATE skills
             SET status = 'archived', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE id = ? AND user_id = 'default' AND status = 'active'",
        )
        .bind(&id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

        if result.rows_affected() > 0 {
            events::emit(events::skill_retired(&id, &name));
            retired.push(RetiredSkill { id, name });
        }
    }

    Ok(retired)
}

// ── On-disk SKILL.md source-of-truth (agentskills.io) ────────────────────────

/// Outcome of exporting one indexed skill to its on-disk `SKILL.md` folder.
pub struct ExportedSkill {
    /// The skill folder (`…/skills/<name>`).
    pub dir: PathBuf,
    /// True when this call wrote a NEW folder; false when an existing on-disk
    /// folder was left untouched (idempotent skip).
    pub written: bool,
}

/// Render a skill's Markdown body from its indexed name/description/definition.
/// The body is the skill's instructions; we lead with the description and, when
/// the stored `definition_json` carries a usable approach, embed it as a fenced
/// JSON block so the saved behavior travels with the portable skill.
fn skill_body_from_definition(
    name: &str,
    description: Option<&str>,
    definition_json: &str,
) -> String {
    let mut body = String::new();
    if let Some(d) = description.map(str::trim).filter(|d| !d.is_empty()) {
        body.push_str(d);
        body.push_str("\n\n");
    }
    body.push_str(&format!(
        "This skill was learned by Permagent from a repeated pattern. When the user's \
         request matches \"{name}\", follow the saved approach below.\n"
    ));
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(definition_json) {
        let is_empty_obj = value.as_object().map(|o| o.is_empty()).unwrap_or(false);
        if !value.is_null() && !is_empty_obj {
            if let Ok(pretty) = serde_json::to_string_pretty(&value) {
                body.push_str("\n## Saved approach\n\n```json\n");
                body.push_str(&pretty);
                body.push_str("\n```\n");
            }
        }
    }
    body
}

/// Export one indexed skill to a portable on-disk `SKILL.md` folder under
/// [`crate::config::paths::Paths::skills_dir`] and record the folder path in the
/// index (`skill_path`).
///
/// Idempotent: if the index already points at a folder whose `SKILL.md` exists,
/// the on-disk copy is treated as authoritative and left untouched (it may have
/// been edited by hand or by another agentskills.io client). A slug collision
/// with a *different* skill is disambiguated with a short id suffix so no skill's
/// folder is ever overwritten.
pub async fn export_skill_to_disk(
    pool: &Pool<Sqlite>,
    skill_id: &str,
) -> Result<ExportedSkill, String> {
    let row = sqlx::query(
        "SELECT name, description, definition_json, version, skill_path
         FROM skills WHERE id = ? AND user_id = 'default'",
    )
    .bind(skill_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("skill {skill_id} not found"))?;

    let name: String = row.get("name");
    let description: Option<String> = row.get("description");
    let definition_json: String = row.get("definition_json");
    let version: i32 = row.get("version");
    let existing_path: Option<String> = row.get("skill_path");

    // Idempotency: an existing on-disk folder is the source of truth.
    if let Some(p) = existing_path.as_deref().filter(|p| !p.is_empty()) {
        if std::path::Path::new(p).join("SKILL.md").is_file() {
            return Ok(ExportedSkill {
                dir: PathBuf::from(p),
                written: false,
            });
        }
    }

    let skills_root = crate::config::paths::Paths::skills_dir();
    let dir = unique_skill_dir(&skills_root, &crate::skill_md::slugify(&name), skill_id);
    let skill_name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("skill")
        .to_string();

    let description = crate::skill_md::sanitize_description(
        description.as_deref().unwrap_or_default(),
        &skill_name,
    );

    let mut extra = std::collections::BTreeMap::new();
    extra.insert("version".to_string(), version.to_string());
    extra.insert("source".to_string(), "permagent".to_string());
    extra.insert("permagent_id".to_string(), skill_id.to_string());
    if name != skill_name {
        extra.insert("display_name".to_string(), name.clone());
    }

    let meta = crate::skill_md::build_meta(&skill_name, &description, extra);
    let body = skill_body_from_definition(&name, Some(description.as_str()), &definition_json);
    crate::skill_md::write_skill_folder(&dir, &meta, &body)?;

    sqlx::query("UPDATE skills SET skill_path = ? WHERE id = ? AND user_id = 'default'")
        .bind(dir.to_string_lossy().to_string())
        .bind(skill_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(ExportedSkill { dir, written: true })
}

/// Pick a folder for a skill under `skills_root`. Reuses `<slug>` when it is free
/// or already belongs to this skill (its `SKILL.md` carries our `permagent_id`);
/// otherwise disambiguates with a short id suffix so a slug collision never
/// overwrites a different skill's folder.
fn unique_skill_dir(skills_root: &std::path::Path, slug: &str, skill_id: &str) -> PathBuf {
    let primary = skills_root.join(slug);
    if !primary.exists() || dir_belongs_to(&primary, skill_id) {
        return primary;
    }
    let short: String = skill_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect();
    let short = if short.is_empty() {
        "x".to_string()
    } else {
        short.to_lowercase()
    };
    // Keep the whole name within the 64-char limit.
    let room = crate::skill_md::MAX_NAME_LEN.saturating_sub(short.len() + 1);
    let base: String = slug.chars().take(room).collect();
    let base = base.trim_end_matches('-');
    skills_root.join(format!("{base}-{short}"))
}

fn dir_belongs_to(dir: &std::path::Path, skill_id: &str) -> bool {
    crate::skill_md::read_skill_folder(dir)
        .ok()
        .and_then(|p| p.meta.permagent_id())
        .map(|id| id == skill_id)
        .unwrap_or(false)
}

/// On-boot migration / reconcile: ensure every indexed skill has an on-disk
/// `SKILL.md` folder (the portable source-of-truth). Idempotent — a skill that
/// already has a folder is skipped — so it is cheap to run on every boot and can
/// never lose or corrupt an existing skill. Per-skill failures are logged and
/// skipped so one bad row can't block the rest. Returns the number of skills
/// freshly exported on this pass.
pub async fn reconcile_skills_to_disk(pool: &Pool<Sqlite>) -> Result<usize, String> {
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM skills WHERE user_id = 'default'")
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;

    let mut exported = 0usize;
    for id in ids {
        match export_skill_to_disk(pool, &id).await {
            Ok(outcome) if outcome.written => exported += 1,
            Ok(_) => {}
            Err(e) => tracing::warn!("skill {id} SKILL.md export skipped: {e}"),
        }
    }
    Ok(exported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    // ── Pure ranking / predicate tests (no DB) ──

    fn ranked(name: &str, execution_count: i64, created_at: &str) -> RankedSkill {
        RankedSkill {
            name: name.to_string(),
            description: None,
            definition_json: "{}".to_string(),
            execution_count,
            created_at: created_at.to_string(),
            skill_path: None,
        }
    }

    #[test]
    fn proven_usage_beats_recency() {
        // A heavily-used old skill outranks a brand-new never-used one.
        let out = rank_skills_for_prompt(
            vec![
                ranked("new_unused", 0, "2026-07-14T00:00:00.000Z"),
                ranked("old_proven", 5, "2020-01-01T00:00:00.000Z"),
            ],
            10,
        );
        assert_eq!(out[0].name, "old_proven");
        assert_eq!(out[1].name, "new_unused");
    }

    #[test]
    fn recency_breaks_ties_at_equal_usage() {
        let out = rank_skills_for_prompt(
            vec![
                ranked("older", 3, "2026-01-01T00:00:00.000Z"),
                ranked("newer", 3, "2026-07-01T00:00:00.000Z"),
            ],
            10,
        );
        assert_eq!(out[0].name, "newer");
        assert_eq!(out[1].name, "older");
    }

    #[test]
    fn ranking_cap_boundary_drops_lowest_usage() {
        // 12 skills (usages 0..=11, same timestamp) capped at 10: the two
        // least-used are dropped and the highest-used leads.
        let out = rank_skills_for_prompt(
            (0..12)
                .map(|i| ranked(&format!("s{i}"), i, "2026-01-01T00:00:00.000Z"))
                .collect(),
            10,
        );
        let names: Vec<&str> = out.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(out.len(), 10);
        assert_eq!(out[0].name, "s11"); // highest usage first
        assert_eq!(out[9].name, "s2"); // lowest surviving usage
        assert!(!names.contains(&"s0"));
        assert!(!names.contains(&"s1"));
    }

    #[test]
    fn ranking_keeps_all_within_limit() {
        // Exactly at the limit → all kept.
        let at = rank_skills_for_prompt(
            (0..10)
                .map(|i| ranked(&format!("s{i}"), i, "2026-01-01T00:00:00.000Z"))
                .collect(),
            10,
        );
        assert_eq!(at.len(), 10);
        // Fewer than the limit → all kept.
        let under = rank_skills_for_prompt(vec![ranked("solo", 1, "2026-01-01T00:00:00.000Z")], 10);
        assert_eq!(under.len(), 1);
    }

    #[test]
    fn ranking_empty_input_yields_empty() {
        assert!(rank_skills_for_prompt(Vec::new(), 10).is_empty());
    }

    #[test]
    fn graduation_threshold_below_at_above() {
        assert_eq!(PROVEN_EXECUTION_THRESHOLD, 3);
        assert!(!skill_is_proven(0));
        assert!(!skill_is_proven(2)); // below
        assert!(skill_is_proven(3)); // at
        assert!(skill_is_proven(4)); // above
        assert!(skill_is_proven(100));
    }

    #[test]
    fn retire_predicate_zero_exec_past_grace() {
        let cutoff = "2026-06-14T00:00:00.000Z";
        assert!(skill_is_stale(0, "2026-01-01T00:00:00.000Z", cutoff));
    }

    #[test]
    fn retire_predicate_never_with_any_execution() {
        // The dangerous edge: a skill with ANY executions is never retired,
        // however ancient.
        let cutoff = "2026-06-14T00:00:00.000Z";
        let ancient = "2000-01-01T00:00:00.000Z";
        assert!(!skill_is_stale(1, ancient, cutoff));
        assert!(!skill_is_stale(2, ancient, cutoff));
        assert!(!skill_is_stale(100, ancient, cutoff));
    }

    #[test]
    fn retire_predicate_never_inside_grace() {
        let cutoff = "2026-06-14T00:00:00.000Z";
        // Created after the cutoff (inside grace) → keep.
        assert!(!skill_is_stale(0, "2026-07-01T00:00:00.000Z", cutoff));
        // Boundary: created exactly at the cutoff → keep (strictly-before retires).
        assert!(!skill_is_stale(0, cutoff, cutoff));
        // One millisecond before the cutoff → retire.
        assert!(skill_is_stale(0, "2026-06-13T23:59:59.999Z", cutoff));
    }

    // ── DB round-trip tests ──

    async fn test_pool() -> Pool<Sqlite> {
        use crate::session::spectral_schema::init_spectral_db;
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    async fn insert_skill(pool: &Pool<Sqlite>, name: &str, created_at: &str) -> String {
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO skills (id, user_id, name, definition_json, status, created_at)
             VALUES (?, 'default', ?, '{}', 'active', ?)",
        )
        .bind(&id)
        .bind(name)
        .bind(created_at)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    async fn add_execution(pool: &Pool<Sqlite>, skill_id: &str) {
        sqlx::query(
            "INSERT INTO skill_executions (id, skill_id, user_id, status)
             VALUES (?, ?, 'default', 'completed')",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(skill_id)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn status_of(pool: &Pool<Sqlite>, skill_id: &str) -> String {
        sqlx::query_scalar("SELECT status FROM skills WHERE id = ?")
            .bind(skill_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn now_iso(pool: &Pool<Sqlite>) -> String {
        sqlx::query_scalar("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn days_ago(pool: &Pool<Sqlite>, n: i64) -> String {
        sqlx::query_scalar("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now', ?)")
            .bind(format!("-{n} days"))
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn build_prompt_returns_none_when_no_active_skills() {
        let pool = test_pool().await;
        // Empty library → nothing to inject.
        assert!(build_skills_prompt(&pool).await.unwrap().is_none());

        // A library of only archived skills → still nothing to inject.
        let id = insert_skill(&pool, "retired_one", "2020-01-01T00:00:00.000Z").await;
        sqlx::query("UPDATE skills SET status = 'archived' WHERE id = ?")
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(build_skills_prompt(&pool).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn build_prompt_ranks_proven_skill_first() {
        let pool = test_pool().await;
        let now = now_iso(&pool).await;
        // old_proven: created long ago but used four times.
        let proven = insert_skill(&pool, "old_proven", "2020-01-01T00:00:00.000Z").await;
        for _ in 0..4 {
            add_execution(&pool, &proven).await;
        }
        // new_unused: created just now, never used.
        insert_skill(&pool, "new_unused", &now).await;

        let prompt = build_skills_prompt(&pool).await.unwrap().unwrap();
        let proven_at = prompt.find("old_proven").unwrap();
        let unused_at = prompt.find("new_unused").unwrap();
        assert!(
            proven_at < unused_at,
            "proven skill must be injected ahead of the newer unused one:\n{prompt}"
        );
    }

    #[tokio::test]
    async fn build_prompt_marks_proven_skills() {
        let pool = test_pool().await;
        let now = now_iso(&pool).await;
        // proven_skill fires exactly the threshold number of times → marked.
        let proven = insert_skill(&pool, "proven_skill", &now).await;
        for _ in 0..PROVEN_EXECUTION_THRESHOLD {
            add_execution(&pool, &proven).await;
        }
        // fledgling_skill fires one short of the threshold → not marked.
        let fledgling = insert_skill(&pool, "fledgling_skill", &now).await;
        for _ in 0..(PROVEN_EXECUTION_THRESHOLD - 1) {
            add_execution(&pool, &fledgling).await;
        }

        let prompt = build_skills_prompt(&pool).await.unwrap().unwrap();
        let proven_line = prompt.lines().find(|l| l.contains("proven_skill")).unwrap();
        assert!(
            proven_line.contains("[proven]"),
            "a skill at the graduation threshold must be marked: {proven_line}"
        );
        let fledgling_line = prompt
            .lines()
            .find(|l| l.contains("fledgling_skill"))
            .unwrap();
        assert!(
            !fledgling_line.contains("[proven]"),
            "a skill below the threshold must not be marked: {fledgling_line}"
        );
    }

    #[tokio::test]
    async fn retirement_archives_only_stale_skills() {
        let pool = test_pool().await;
        let now = now_iso(&pool).await;

        let old_unused = insert_skill(&pool, "old_unused", "2020-01-01T00:00:00.000Z").await;
        // Ancient but has fired five times — the dangerous edge: never retire it.
        let old_used = insert_skill(&pool, "old_used", "2020-01-01T00:00:00.000Z").await;
        for _ in 0..5 {
            add_execution(&pool, &old_used).await;
        }
        let new_unused = insert_skill(&pool, "new_unused", &now).await;

        let retired = retire_stale_skills(&pool, RETIREMENT_GRACE_DAYS)
            .await
            .unwrap();

        assert_eq!(
            retired.len(),
            1,
            "only the old never-used skill should retire"
        );
        assert_eq!(retired[0].id, old_unused);
        assert_eq!(status_of(&pool, &old_unused).await, "archived");
        assert_eq!(
            status_of(&pool, &old_used).await,
            "active",
            "an ancient skill with executions must never be retired"
        );
        assert_eq!(status_of(&pool, &new_unused).await, "active");

        // Idempotent: a second sweep retires nothing (the stale skill is archived).
        let again = retire_stale_skills(&pool, RETIREMENT_GRACE_DAYS)
            .await
            .unwrap();
        assert!(again.is_empty());
    }

    #[tokio::test]
    async fn retirement_respects_grace_window_parameter() {
        let pool = test_pool().await;
        let five_days_ago = days_ago(&pool, 5).await;
        let s = insert_skill(&pool, "five_day_old", &five_days_ago).await;

        // A 10-day grace window: the 5-day-old skill is still inside grace → kept.
        let none = retire_stale_skills(&pool, 10).await.unwrap();
        assert!(none.is_empty());
        assert_eq!(status_of(&pool, &s).await, "active");

        // A 3-day grace window: the 5-day-old skill is now past grace → retired.
        let retired = retire_stale_skills(&pool, 3).await.unwrap();
        assert_eq!(retired.len(), 1);
        assert_eq!(retired[0].id, s);
        assert_eq!(status_of(&pool, &s).await, "archived");
    }

    #[tokio::test]
    async fn update_skill_persists_and_reports_missing() {
        // create_skill now also writes the on-disk SKILL.md folder; pin the path
        // root to a tempdir so the test never touches the real ~/.permagent.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().display().to_string();
        let _env = env_lock::lock_env([
            ("HOME", Some(root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(root.as_str())),
        ]);
        let pool = test_pool().await;
        let created = create_skill(
            &pool,
            CreateSkillParams {
                name: "orig-name".to_string(),
                description: Some("orig desc".to_string()),
                tool_used: "gmail__search".to_string(),
                argument_shape_hash: "shape-1".to_string(),
                definition_json: serde_json::json!({}),
                source_task_id: None,
            },
        )
        .await
        .unwrap();

        // Rename + re-describe → persisted (what the editor sees on reopen).
        assert!(
            update_skill(&pool, &created.id, Some("new-name"), Some("new desc"))
                .await
                .unwrap()
        );
        let detail = get_skill(&pool, &created.id).await.unwrap().unwrap();
        assert_eq!(detail.name, "new-name");
        assert_eq!(detail.description.as_deref(), Some("new desc"));

        // Partial update (description only) leaves the name intact via COALESCE.
        assert!(update_skill(&pool, &created.id, None, Some("only desc"))
            .await
            .unwrap());
        let detail = get_skill(&pool, &created.id).await.unwrap().unwrap();
        assert_eq!(detail.name, "new-name");
        assert_eq!(detail.description.as_deref(), Some("only desc"));

        // Unknown id → false, so the editor surfaces the failure instead of
        // closing as if it saved.
        assert!(!update_skill(&pool, "does-not-exist", Some("x"), None)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn record_execution_shows_up_in_history() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().display().to_string();
        let _env = env_lock::lock_env([
            ("HOME", Some(root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(root.as_str())),
        ]);
        let pool = test_pool().await;
        let created = create_skill(
            &pool,
            CreateSkillParams {
                name: "runner".to_string(),
                description: None,
                tool_used: "gmail__search".to_string(),
                argument_shape_hash: "shape-hist".to_string(),
                definition_json: serde_json::json!({}),
                source_task_id: None,
            },
        )
        .await
        .unwrap();

        // Empty before any run.
        assert!(list_skill_executions(&pool, &created.id)
            .await
            .unwrap()
            .is_empty());

        // record_execution matches the skill by argument_shape_hash and logs a row.
        record_execution(&pool, "gmail__search", "shape-hist", None)
            .await
            .unwrap();

        let execs = list_skill_executions(&pool, &created.id).await.unwrap();
        assert_eq!(execs.len(), 1);
        assert_eq!(execs[0].status, "completed");
        assert!(execs[0].completed_at.is_some());
        assert!(!execs[0].started_at.is_empty());
    }

    // ── On-disk SKILL.md source-of-truth ──

    #[tokio::test]
    async fn create_skill_writes_portable_skill_md_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().display().to_string();
        let _env = env_lock::lock_env([
            ("HOME", Some(root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(root.as_str())),
        ]);
        let pool = test_pool().await;

        let created = create_skill(
            &pool,
            CreateSkillParams {
                name: "Weekly Report".to_string(),
                description: Some("Draft the weekly status report.".to_string()),
                tool_used: "gmail__search".to_string(),
                argument_shape_hash: "shape-wr".to_string(),
                definition_json: serde_json::json!({"steps": ["gather", "summarize"]}),
                source_task_id: None,
            },
        )
        .await
        .unwrap();

        // A standards-valid SKILL.md folder is written under the skills dir.
        let dir = crate::config::paths::Paths::skills_dir().join("weekly-report");
        assert!(
            dir.join("SKILL.md").is_file(),
            "create_skill must write a SKILL.md folder"
        );

        // The index points at it.
        let skill_path: Option<String> =
            sqlx::query_scalar("SELECT skill_path FROM skills WHERE id = ?")
                .bind(&created.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(
            skill_path
                .as_deref()
                .unwrap_or("")
                .ends_with("weekly-report"),
            "skill_path index must point at the on-disk folder"
        );

        // It parses back as a standards-valid skill and round-trips our id.
        let parsed = crate::skill_md::read_skill_folder(&dir).unwrap();
        assert_eq!(parsed.meta.name, "weekly-report");
        assert_eq!(parsed.meta.description, "Draft the weekly status report.");
        assert_eq!(
            parsed.meta.permagent_id().as_deref(),
            Some(created.id.as_str())
        );
        assert_eq!(
            parsed.meta.metadata_str("display_name").as_deref(),
            Some("Weekly Report")
        );
        // The saved approach travels with the portable skill.
        assert!(parsed.body.contains("summarize"));
    }

    #[tokio::test]
    async fn reconcile_exports_legacy_skills_losslessly_and_idempotently() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().display().to_string();
        let _env = env_lock::lock_env([
            ("HOME", Some(root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(root.as_str())),
        ]);
        let pool = test_pool().await;

        // Two legacy definition_json skills with NO on-disk folder (skill_path NULL).
        insert_skill(&pool, "Legacy Alpha", "2024-01-01T00:00:00.000Z").await;
        insert_skill(&pool, "Legacy Beta", "2024-01-02T00:00:00.000Z").await;

        // First pass migrates both to disk.
        let n = reconcile_skills_to_disk(&pool).await.unwrap();
        assert_eq!(n, 2, "both legacy skills export on the first pass");

        let skills_root = crate::config::paths::Paths::skills_dir();
        assert!(skills_root.join("legacy-alpha/SKILL.md").is_file());
        assert!(skills_root.join("legacy-beta/SKILL.md").is_file());

        // Every index row now points at its folder.
        let unindexed: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM skills WHERE skill_path IS NULL OR skill_path = ''",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(unindexed, 0, "reconcile indexes every skill's folder");

        // Idempotent: a second pass exports nothing and loses nothing.
        let again = reconcile_skills_to_disk(&pool).await.unwrap();
        assert_eq!(again, 0, "reconcile is idempotent");
        assert!(skills_root.join("legacy-alpha/SKILL.md").is_file());
        assert!(skills_root.join("legacy-beta/SKILL.md").is_file());
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM skills")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 2, "no skill row lost by the migration");
    }

    #[tokio::test]
    async fn build_prompt_reads_on_disk_skill_md_with_progressive_disclosure() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().display().to_string();
        let _env = env_lock::lock_env([
            ("HOME", Some(root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(root.as_str())),
        ]);
        let pool = test_pool().await;

        create_skill(
            &pool,
            CreateSkillParams {
                name: "Weekly Report".to_string(),
                description: Some("Draft the weekly status report.".to_string()),
                tool_used: "gmail__search".to_string(),
                argument_shape_hash: "shape-wr".to_string(),
                definition_json: serde_json::json!({"secret_internal_key": "xyz"}),
                source_task_id: None,
            },
        )
        .await
        .unwrap();

        let prompt = build_skills_prompt(&pool).await.unwrap().unwrap();
        // Progressive disclosure: the on-disk description + a load_skill handle
        // are injected; the full body/definition is NOT inlined (loads on demand).
        assert!(
            prompt.contains("Draft the weekly status report."),
            "on-disk SKILL.md description must be injected:\n{prompt}"
        );
        assert!(
            prompt.contains("load_skill(name: \"weekly-report\")"),
            "a load_skill handle must be offered for on-demand body loading:\n{prompt}"
        );
        assert!(
            !prompt.contains("secret_internal_key"),
            "the skill body/definition must not be inlined:\n{prompt}"
        );
    }

    #[tokio::test]
    async fn export_is_idempotent_and_does_not_overwrite_edited_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().display().to_string();
        let _env = env_lock::lock_env([
            ("HOME", Some(root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(root.as_str())),
        ]);
        let pool = test_pool().await;

        let id = insert_skill(&pool, "Edited Skill", "2024-01-01T00:00:00.000Z").await;
        let first = export_skill_to_disk(&pool, &id).await.unwrap();
        assert!(first.written);

        // Hand-edit the on-disk SKILL.md (as an external client / the user might).
        let edited = "---\nname: edited-skill\ndescription: Human-edited description.\n---\n\nEdited body.\n";
        std::fs::write(first.dir.join("SKILL.md"), edited).unwrap();

        // A second export is a no-op: the on-disk copy is authoritative.
        let second = export_skill_to_disk(&pool, &id).await.unwrap();
        assert!(
            !second.written,
            "existing on-disk folder must not be overwritten"
        );
        let parsed = crate::skill_md::read_skill_folder(&first.dir).unwrap();
        assert_eq!(parsed.meta.description, "Human-edited description.");
        assert_eq!(parsed.body, "Edited body.");
    }
}
