//! Task Logger — writes tool invocations to the Spectral `tasks` table and
//! emits corresponding [`PermagentEvent`]s on the event bus.
//!
//! Each tool call in the agent execution loop maps to one task row.
//! The `argument_shape_hash` enables repetition detection for auto-skills (Section F).

use crate::events;
use chrono::Utc;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Pool, Row, Sqlite};
use std::sync::OnceLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Opaque task identifier (UUIDv7 string).
pub type TaskId = String;

// ── Global singleton ────────────────────────────────────────────────────────

static TASK_LOGGER: OnceLock<TaskLogger> = OnceLock::new();

/// Initialize the global TaskLogger with the given pool. Call once at startup.
/// Returns false if already initialized (idempotent).
pub fn init_global(pool: Pool<Sqlite>) -> bool {
    TASK_LOGGER.set(TaskLogger::new(pool)).is_ok()
}

/// Get a reference to the global TaskLogger. Returns None if not initialized.
pub fn global() -> Option<&'static TaskLogger> {
    TASK_LOGGER.get()
}

/// Logs tool invocations to the Spectral `tasks` table.
#[derive(Clone)]
pub struct TaskLogger {
    pool: Pool<Sqlite>,
}

impl TaskLogger {
    pub fn new(pool: Pool<Sqlite>) -> Self {
        Self { pool }
    }

    /// Log a new task as `pending`. Returns the task ID.
    pub async fn log_task_created(&self, description: &str, tool_used: Option<&str>) -> TaskId {
        let task_id = Uuid::now_v7().to_string();
        let now = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();

        let result = sqlx::query(
            "INSERT INTO tasks (id, user_id, description, tool_used, status, created_at)
             VALUES (?, 'default', ?, ?, 'pending', ?)",
        )
        .bind(&task_id)
        .bind(description)
        .bind(tool_used)
        .bind(&now)
        .execute(&self.pool)
        .await;

        if let Err(e) = result {
            warn!("Failed to log task_created: {}", e);
        }

        events::emit(events::task_created(&task_id, description, tool_used));
        debug!("task_created: {} ({})", task_id, description);
        task_id
    }

    /// Mark a task as `running`.
    pub async fn log_task_started(&self, task_id: &str, session_id: &str) {
        let now = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();

        let result = sqlx::query(
            "UPDATE tasks SET status = 'running', session_id = ?, started_at = ? WHERE id = ?",
        )
        .bind(session_id)
        .bind(&now)
        .bind(task_id)
        .execute(&self.pool)
        .await;

        if let Err(e) = result {
            warn!("Failed to log task_started: {}", e);
        }

        events::emit(events::task_started(task_id, session_id));
    }

    /// Mark a task as `completed` with output and duration.
    /// Computes `argument_shape_hash` from the tool arguments if available.
    pub async fn log_task_completed(
        &self,
        task_id: &str,
        tool_used: Option<&str>,
        arguments: Option<&Value>,
        output_json: &Value,
        duration_ms: u64,
    ) {
        let now = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
        let output_str = serde_json::to_string(output_json).unwrap_or_default();
        let shape_hash = compute_argument_shape_hash(tool_used, arguments);

        let result = sqlx::query(
            "UPDATE tasks SET status = 'completed', output_json = ?, argument_shape_hash = ?,
             completed_at = ? WHERE id = ?",
        )
        .bind(&output_str)
        .bind(&shape_hash)
        .bind(&now)
        .bind(task_id)
        .execute(&self.pool)
        .await;

        if let Err(e) = result {
            warn!("Failed to log task_completed: {}", e);
        }

        events::emit(events::task_completed(task_id, output_json, duration_ms));
    }

    /// Mark a task as `failed`.
    pub async fn log_task_failed(&self, task_id: &str, error: &str) {
        let result =
            sqlx::query("UPDATE tasks SET status = 'failed', error_message = ? WHERE id = ?")
                .bind(error)
                .bind(task_id)
                .execute(&self.pool)
                .await;

        if let Err(e) = result {
            warn!("Failed to log task_failed: {}", e);
        }

        events::emit(events::task_failed(task_id, error));
    }

    /// Check for repetition candidates after a task completes.
    /// Queries the `repetition_candidates` view for shapes that appear >= threshold
    /// times within the configured window, then emits `SkillProposed` for new patterns.
    pub async fn check_repetition_candidates(&self, config: &SkillsConfig) {
        if !config.auto_detect {
            return;
        }

        let rows = match sqlx::query(
            "SELECT tool_used, argument_shape_hash, occurrence_count, latest_description
             FROM repetition_candidates
             WHERE user_id = 'default'
               AND occurrence_count >= ?",
        )
        .bind(config.repetition_threshold as i64)
        .fetch_all(&self.pool)
        .await
        {
            Ok(rows) => rows,
            Err(e) => {
                warn!("Failed to query repetition_candidates: {}", e);
                return;
            }
        };

        for row in &rows {
            let tool_used: String = row.get("tool_used");
            let shape_hash: String = row.get("argument_shape_hash");
            let count: i64 = row.get("occurrence_count");
            let description: String = row.get("latest_description");

            // Check if a skill already exists for this shape
            let skill_exists = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (
                    SELECT 1 FROM skills
                    WHERE user_id = 'default'
                      AND trigger_value LIKE '%' || ? || '%'
                      AND status != 'archived'
                )",
            )
            .bind(&shape_hash)
            .fetch_one(&self.pool)
            .await
            .unwrap_or(false);

            if skill_exists {
                continue;
            }

            // Check if dismissed within the last 30 days
            let dismissed = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (
                    SELECT 1 FROM skill_dismissals
                    WHERE user_id = 'default'
                      AND argument_shape_hash = ?
                      AND dismissed_at >= strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-30 days')
                )",
            )
            .bind(&shape_hash)
            .fetch_one(&self.pool)
            .await
            .unwrap_or(false);

            if dismissed {
                continue;
            }

            // Get source task IDs for this pattern
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
            .fetch_all(&self.pool)
            .await
            .unwrap_or_default();

            info!(
                "Skill proposed: '{}' ({} occurrences of {})",
                description, count, tool_used
            );
            events::emit(events::skill_proposed(
                &description,
                &tool_used,
                &shape_hash,
                count,
                &task_ids,
            ));
        }
    }

    /// Get a clone of the database pool (for use by skills routes).
    pub fn pool(&self) -> &Pool<Sqlite> {
        &self.pool
    }
}

/// Configuration for the auto-skills detection engine.
#[derive(Debug, Clone)]
pub struct SkillsConfig {
    pub auto_detect: bool,
    pub repetition_threshold: u32,
    pub repetition_window_days: u32,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            auto_detect: true,
            repetition_threshold: 2,
            repetition_window_days: 7,
        }
    }
}

impl SkillsConfig {
    /// Load skills config from the permagent config system.
    pub fn from_config() -> Self {
        let config = crate::config::Config::global();
        let auto_detect = config
            .get_param::<bool>("skills_auto_detect")
            .unwrap_or(true);
        let threshold = config
            .get_param::<u32>("skills_repetition_threshold")
            .unwrap_or(2);
        let window = config
            .get_param::<u32>("skills_repetition_window_days")
            .unwrap_or(7);
        Self {
            auto_detect,
            repetition_threshold: threshold,
            repetition_window_days: window,
        }
    }
}

/// Compute argument_shape_hash per Section F.1:
/// stable hash of (tool_name, sorted_arg_keys, arg_type_categories).
/// Returns sha256 hex truncated to 16 chars, or None if inputs are missing.
pub fn compute_argument_shape_hash(
    tool_used: Option<&str>,
    arguments: Option<&Value>,
) -> Option<String> {
    let tool = tool_used?;
    let args = arguments?;
    let obj = args.as_object()?;

    let mut keys: Vec<&String> = obj.keys().collect();
    keys.sort();

    let type_categories: Vec<&str> = keys
        .iter()
        .map(|k| categorize_json_value(obj.get(*k).unwrap()))
        .collect();

    let mut hasher = Sha256::new();
    hasher.update(tool.as_bytes());
    hasher.update(b"\0");
    for (key, type_cat) in keys.iter().zip(type_categories.iter()) {
        hasher.update(key.as_bytes());
        hasher.update(b":");
        hasher.update(type_cat.as_bytes());
        hasher.update(b",");
    }
    let hash = hasher.finalize();
    let hex = format!("{:x}", hash);
    Some(hex[..16].to_string())
}

/// Categorize a JSON value into a type string for shape hashing.
fn categorize_json_value(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_argument_shape_hash_deterministic() {
        let args = serde_json::json!({"query": "is:unread", "max_results": 10});
        let h1 = compute_argument_shape_hash(Some("gmail__search"), Some(&args));
        let h2 = compute_argument_shape_hash(Some("gmail__search"), Some(&args));
        assert_eq!(h1, h2);
        assert!(h1.as_ref().unwrap().len() == 16);
    }

    #[test]
    fn test_argument_shape_hash_value_independent() {
        let args1 = serde_json::json!({"query": "is:unread"});
        let args2 = serde_json::json!({"query": "from:boss@example.com"});
        let h1 = compute_argument_shape_hash(Some("gmail__search"), Some(&args1));
        let h2 = compute_argument_shape_hash(Some("gmail__search"), Some(&args2));
        assert_eq!(h1, h2); // Same shape, different values
    }

    #[test]
    fn test_argument_shape_hash_different_tools() {
        let args = serde_json::json!({"query": "test"});
        let h1 = compute_argument_shape_hash(Some("gmail__search"), Some(&args));
        let h2 = compute_argument_shape_hash(Some("slack__search"), Some(&args));
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_argument_shape_hash_none_cases() {
        assert!(compute_argument_shape_hash(None, None).is_none());
        assert!(compute_argument_shape_hash(Some("tool"), None).is_none());
        assert!(compute_argument_shape_hash(None, Some(&serde_json::json!({}))).is_none());
    }
}
