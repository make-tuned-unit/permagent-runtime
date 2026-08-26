//! Durable Council sessions, per-model positions, and chair reports.

use serde::{Deserialize, Serialize};
use sqlx::{Pool, Row, Sqlite};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    Weekly,
    OnDemand,
}

impl Trigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Weekly => "weekly",
            Self::OnDemand => "on_demand",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Running,
    Complete,
    Failed,
    Partial,
}

impl SessionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Complete => "complete",
            Self::Failed => "failed",
            Self::Partial => "partial",
        }
    }

    pub fn from_stored(s: &str) -> Self {
        match s {
            "complete" => Self::Complete,
            "failed" => Self::Failed,
            "partial" => Self::Partial,
            _ => Self::Running,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub trigger: String,
    pub extra_question: Option<String>,
    pub chair_provider: Option<String>,
    pub chair_model: Option<String>,
    pub brief_json: String,
    pub status: SessionStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub id: String,
    pub session_id: String,
    pub round: i64,
    pub provider: String,
    pub model: String,
    pub status: String,
    pub raw_text: Option<String>,
    pub parsed_json: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub id: String,
    pub session_id: String,
    pub generated_at: String,
    pub headline: String,
    pub markdown: String,
    pub consensus: Vec<String>,
    pub dissent: Vec<serde_json::Value>,
    pub actions: Vec<serde_json::Value>,
    pub chair_provider: Option<String>,
    pub chair_model: Option<String>,
}

pub async fn insert_session(
    pool: &Pool<Sqlite>,
    trigger: Trigger,
    extra_question: Option<&str>,
    brief_json: &serde_json::Value,
) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    let brief = serde_json::to_string(brief_json).unwrap_or_else(|_| "{}".to_string());
    sqlx::query(
        "INSERT INTO council_sessions (id, trigger, extra_question, brief_json, status)
         VALUES (?, ?, ?, ?, 'running')",
    )
    .bind(&id)
    .bind(trigger.as_str())
    .bind(extra_question)
    .bind(brief)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(id)
}

pub async fn finish_session(
    pool: &Pool<Sqlite>,
    id: &str,
    status: SessionStatus,
    chair_provider: Option<&str>,
    chair_model: Option<&str>,
    error: Option<&str>,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE council_sessions
         SET finished_at = strftime('%Y-%m-%dT%H:%M:%fZ','now'),
             status = ?, chair_provider = ?, chair_model = ?, error = ?
         WHERE id = ?",
    )
    .bind(status.as_str())
    .bind(chair_provider)
    .bind(chair_model)
    .bind(error)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn insert_position(
    pool: &Pool<Sqlite>,
    session_id: &str,
    round: i64,
    provider: &str,
    model: &str,
    status: &str,
    raw_text: Option<&str>,
    parsed: Option<&serde_json::Value>,
    error: Option<&str>,
) -> Result<(), String> {
    let id = Uuid::new_v4().to_string();
    let parsed_s = parsed.map(|v| serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()));
    sqlx::query(
        "INSERT INTO council_positions
            (id, session_id, round, provider, model, status, raw_text, parsed_json, error)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(session_id)
    .bind(round)
    .bind(provider)
    .bind(model)
    .bind(status)
    .bind(raw_text)
    .bind(parsed_s)
    .bind(error)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn insert_report(
    pool: &Pool<Sqlite>,
    session_id: &str,
    headline: &str,
    markdown: &str,
    consensus: &[String],
    dissent: &[serde_json::Value],
    actions: &[serde_json::Value],
    chair_provider: Option<&str>,
    chair_model: Option<&str>,
) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO council_reports
            (id, session_id, headline, markdown, consensus_json, dissent_json, actions_json,
             chair_provider, chair_model)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(session_id)
    .bind(headline)
    .bind(markdown)
    .bind(serde_json::to_string(consensus).unwrap_or_else(|_| "[]".to_string()))
    .bind(serde_json::to_string(dissent).unwrap_or_else(|_| "[]".to_string()))
    .bind(serde_json::to_string(actions).unwrap_or_else(|_| "[]".to_string()))
    .bind(chair_provider)
    .bind(chair_model)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(id)
}

fn parse_json_list(raw: Option<String>) -> Vec<serde_json::Value> {
    raw.and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn parse_string_list(raw: Option<String>) -> Vec<String> {
    raw.and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn row_to_session(row: &sqlx::sqlite::SqliteRow) -> Session {
    Session {
        id: row.get("id"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
        trigger: row.get("trigger"),
        extra_question: row.get("extra_question"),
        chair_provider: row.get("chair_provider"),
        chair_model: row.get("chair_model"),
        brief_json: row.get("brief_json"),
        status: SessionStatus::from_stored(row.get::<String, _>("status").as_str()),
        error: row.get("error"),
    }
}

fn row_to_report(row: &sqlx::sqlite::SqliteRow) -> Report {
    Report {
        id: row.get("id"),
        session_id: row.get("session_id"),
        generated_at: row.get("generated_at"),
        headline: row.get("headline"),
        markdown: row.get("markdown"),
        consensus: parse_string_list(row.get("consensus_json")),
        dissent: parse_json_list(row.get("dissent_json")),
        actions: parse_json_list(row.get("actions_json")),
        chair_provider: row.get("chair_provider"),
        chair_model: row.get("chair_model"),
    }
}

const SESSION_COLS: &str = "id, started_at, finished_at, trigger, extra_question,
        chair_provider, chair_model, brief_json, status, error";

pub async fn get_session(pool: &Pool<Sqlite>, id: &str) -> Result<Option<Session>, String> {
    let sql = format!("SELECT {SESSION_COLS} FROM council_sessions WHERE id = ?");
    let row = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.as_ref().map(row_to_session))
}

pub async fn latest_finished(
    pool: &Pool<Sqlite>,
) -> Result<Option<(Session, Option<Report>)>, String> {
    let sql = format!(
        "SELECT {SESSION_COLS} FROM council_sessions
         WHERE status IN ('complete','partial')
         ORDER BY started_at DESC LIMIT 1"
    );
    let row = sqlx::query(sqlx::AssertSqlSafe(sql))
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
    let Some(row) = row else {
        return Ok(None);
    };
    let session = row_to_session(&row);
    let report = get_report_for_session(pool, &session.id).await?;
    Ok(Some((session, report)))
}

pub async fn get_report_for_session(
    pool: &Pool<Sqlite>,
    session_id: &str,
) -> Result<Option<Report>, String> {
    let row = sqlx::query(
        "SELECT id, session_id, generated_at, headline, markdown,
                consensus_json, dissent_json, actions_json, chair_provider, chair_model
         FROM council_reports WHERE session_id = ?",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.as_ref().map(row_to_report))
}

pub async fn list_positions(
    pool: &Pool<Sqlite>,
    session_id: &str,
) -> Result<Vec<Position>, String> {
    let rows = sqlx::query(
        "SELECT id, session_id, round, provider, model, status, raw_text, parsed_json, error
         FROM council_positions WHERE session_id = ?
         ORDER BY round ASC, provider ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .map(|r| Position {
            id: r.get("id"),
            session_id: r.get("session_id"),
            round: r.get("round"),
            provider: r.get("provider"),
            model: r.get("model"),
            status: r.get("status"),
            raw_text: r.get("raw_text"),
            parsed_json: r
                .get::<Option<String>, _>("parsed_json")
                .and_then(|s| serde_json::from_str(&s).ok()),
            error: r.get("error"),
        })
        .collect())
}

/// Most recent successful (complete or partial) session started_at, if any.
pub async fn last_success_started_at(pool: &Pool<Sqlite>) -> Result<Option<String>, String> {
    sqlx::query_scalar(
        "SELECT started_at FROM council_sessions
         WHERE status IN ('complete','partial')
         ORDER BY started_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())
}

/// True when another session is currently running.
pub async fn has_running(pool: &Pool<Sqlite>) -> Result<bool, String> {
    let n: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM council_sessions WHERE status = 'running'")
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::spectral_schema::init_spectral_db;

    async fn pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        crate::session::spectral_schema::apply_council_schema(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn round_trip_session_positions_report() {
        let pool = pool().await;
        let id = insert_session(
            &pool,
            Trigger::Weekly,
            None,
            &serde_json::json!({"projects": []}),
        )
        .await
        .unwrap();
        insert_position(
            &pool,
            &id,
            1,
            "anthropic",
            "claude-haiku",
            "ok",
            Some("{\"confidence\":0.7}"),
            Some(&serde_json::json!({"confidence": 0.7})),
            None,
        )
        .await
        .unwrap();
        insert_report(
            &pool,
            &id,
            "Focus on Permagent",
            "# Report\nDo the thing.",
            &["focus".into()],
            &[serde_json::json!({"model": "gpt", "claim": "wait"})],
            &[serde_json::json!({"title": "Ship the council"})],
            Some("anthropic"),
            Some("claude-haiku"),
        )
        .await
        .unwrap();
        finish_session(
            &pool,
            &id,
            SessionStatus::Complete,
            Some("anthropic"),
            Some("claude-haiku"),
            None,
        )
        .await
        .unwrap();

        let (session, report) = latest_finished(&pool).await.unwrap().unwrap();
        assert_eq!(session.id, id);
        assert_eq!(session.status, SessionStatus::Complete);
        let report = report.unwrap();
        assert_eq!(report.headline, "Focus on Permagent");
        assert_eq!(report.consensus, vec!["focus"]);
        assert_eq!(list_positions(&pool, &id).await.unwrap().len(), 1);
        assert!(last_success_started_at(&pool).await.unwrap().is_some());
    }
}
