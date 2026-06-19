//! Recognition instrumentation — the AmbientFrame emit side.
//!
//! At every recall we mint a `retrieval_id` and persist a `recognition_events`
//! row plus its retrieved-set members UNCONDITIONALLY (the falsifiable
//! substrate). The retrieval-set memory IDs are not in scope at turn
//! resolution, so we never hold them to turn-end: association is by the
//! persisted key, and the outcome is written back later (seconds-to-minutes)
//! keyed on `retrieval_id`.
//!
//! Two outcome proxies feed the write-back:
//!   - PRIMARY: task resolution (`tasks::log_task_completed`), joined via
//!     `tasks.session_id` → `recognition_events.session_id`.
//!   - SECONDARY: decision approve/bounce (`routes/decisions::execute_effect`),
//!     joined 2-hop `goal_id` → `cards.metadata_json.worker_session_id` →
//!     `recognition_events.session_id`. ~0% volume until the orchestrator is
//!     enabled.
//!
//! All writes are best-effort: failures are logged, never propagated. These
//! tables live in permagent.db (the SessionStorage pool), distinct from
//! Spectral's own precursor `retrieval_events`.

use chrono::Utc;
use sqlx::{Pool, Sqlite};
use tracing::{debug, warn};
use uuid::Uuid;

/// A single retrieved-set member: `(memory_id, signal_score, rank)`.
pub type SetMember = (String, f64, i64);

fn now_iso() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// Persist a recognition (recall) event plus its retrieved set, fire-and-forget.
///
/// Mints the `retrieval_id` join key, writes one `recognition_events` row and
/// one `recognition_set_members` row per `member`, UNCONDITIONALLY (even an
/// empty set). Spawns a detached task so it never blocks the reply path; all
/// errors are logged.
pub fn spawn_persist_recognition(
    pool: Pool<Sqlite>,
    recognition_ctx: spectral::graph::RecognitionContext,
    query: String,
    strategy: String,
    members: Vec<SetMember>,
) {
    tokio::spawn(async move {
        if let Err(e) =
            persist_recognition(&pool, &recognition_ctx, &query, &strategy, &members).await
        {
            warn!(
                target: "permagent::recognition",
                "Failed to persist recognition event: {}",
                e
            );
        }
    });
}

async fn persist_recognition(
    pool: &Pool<Sqlite>,
    recognition_ctx: &spectral::graph::RecognitionContext,
    query: &str,
    strategy: &str,
    members: &[SetMember],
) -> Result<(), sqlx::Error> {
    let retrieval_id = Uuid::now_v7().to_string();
    let now = now_iso();
    let rc_persona = recognition_ctx.persona.clone().unwrap_or_default();
    let rc_session_id = recognition_ctx.session_id.clone();
    let rc_focus_wing = recognition_ctx.focus_wing.clone();
    // Top-level session_id is NOT NULL; the recognition-context session is the
    // authoritative source, empty string only if a caller recalls session-less.
    let session_id = rc_session_id.clone().unwrap_or_default();

    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO recognition_events
            (retrieval_id, session_id, query, retrieved_at,
             rc_persona, rc_session_id, rc_focus_wing, strategy)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&retrieval_id)
    .bind(&session_id)
    .bind(query)
    .bind(&now)
    .bind(&rc_persona)
    .bind(&rc_session_id)
    .bind(&rc_focus_wing)
    .bind(strategy)
    .execute(&mut *tx)
    .await?;

    for (memory_id, signal_score, rank) in members {
        sqlx::query(
            "INSERT OR IGNORE INTO recognition_set_members
                (retrieval_id, memory_id, signal_score, rank)
             VALUES (?, ?, ?, ?)",
        )
        .bind(&retrieval_id)
        .bind(memory_id)
        .bind(signal_score)
        .bind(rank)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    debug!(
        target: "permagent::recognition",
        "Persisted recognition event {} ({} members, strategy={})",
        retrieval_id,
        members.len(),
        strategy
    );
    Ok(())
}

/// Write back a TASK-resolution outcome onto the still-unattributed recognition
/// events for a session (PRIMARY proxy). Attribute-by-key-later: every recall in
/// the session whose outcome is still NULL is resolved as `TaskResolved` /
/// `Positive`. Best-effort; no-op when the session has no open recognition rows.
pub async fn write_back_task_outcome(pool: &Pool<Sqlite>, session_id: &str) {
    if session_id.is_empty() {
        return;
    }
    write_back_outcome(pool, session_id, "TaskResolved", "Positive", "Task").await;
}

/// Write back a DECISION outcome (SECONDARY proxy) via the 2-hop join
/// `goal_id` → `cards.metadata_json.worker_session_id` → recognition events.
/// `approved` selects `DecisionApproved`/`Positive` vs `DecisionBounced`/
/// `Negative`. ~0% volume until the orchestrator is enabled. Best-effort.
pub async fn write_back_decision_outcome(pool: &Pool<Sqlite>, goal_id: &str, approved: bool) {
    let session_id = match resolve_worker_session_id(pool, goal_id).await {
        Some(sid) if !sid.is_empty() => sid,
        _ => {
            debug!(
                target: "permagent::recognition",
                "No worker_session_id for goal {}; skipping decision write-back",
                goal_id
            );
            return;
        }
    };
    let (kind, polarity) = if approved {
        ("DecisionApproved", "Positive")
    } else {
        ("DecisionBounced", "Negative")
    };
    write_back_outcome(pool, &session_id, kind, polarity, "Decision").await;
}

/// Resolve a goal's worker session id from `cards.metadata_json`, preferring the
/// current `worker_session_id`, then the most recent of `worker_session_ids`.
async fn resolve_worker_session_id(pool: &Pool<Sqlite>, goal_id: &str) -> Option<String> {
    let meta_json: String = sqlx::query_scalar("SELECT metadata_json FROM cards WHERE id = ?")
        .bind(goal_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()?;
    let meta: serde_json::Value = serde_json::from_str(&meta_json).ok()?;
    if let Some(sid) = meta.get("worker_session_id").and_then(|v| v.as_str()) {
        return Some(sid.to_string());
    }
    meta.get("worker_session_ids")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.iter().rev().find_map(|v| v.as_str()))
        .map(String::from)
}

async fn write_back_outcome(
    pool: &Pool<Sqlite>,
    session_id: &str,
    kind: &str,
    polarity: &str,
    source: &str,
) {
    let now = now_iso();
    let result = sqlx::query(
        "UPDATE recognition_events
            SET outcome_kind = ?, outcome_polarity = ?, outcome_source = ?, outcome_observed_at = ?
          WHERE session_id = ? AND outcome_kind IS NULL",
    )
    .bind(kind)
    .bind(polarity)
    .bind(source)
    .bind(&now)
    .bind(session_id)
    .execute(pool)
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            debug!(
                target: "permagent::recognition",
                "Wrote back {} outcome to {} recognition event(s) for session {}",
                kind,
                r.rows_affected(),
                session_id
            );
        }
        Ok(_) => {}
        Err(e) => {
            warn!(
                target: "permagent::recognition",
                "Failed to write back {} outcome for session {}: {}",
                kind,
                session_id,
                e
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Row;

    async fn test_pool() -> Pool<Sqlite> {
        use crate::session::spectral_schema::init_spectral_db;
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    fn ctx(session: &str) -> spectral::graph::RecognitionContext {
        spectral::graph::RecognitionContext::empty()
            .with_persona("henry")
            .with_session(session)
            .with_focus_wing("permagent")
    }

    #[tokio::test]
    async fn persists_event_and_members_unconditionally() {
        let pool = test_pool().await;
        persist_recognition(
            &pool,
            &ctx("sess-1"),
            "how do I configure voice?",
            "cascade",
            &[("mem-a".into(), 0.9, 0), ("mem-b".into(), 0.8, 1)],
        )
        .await
        .unwrap();

        let row = sqlx::query(
            "SELECT retrieval_id, session_id, query, rc_persona, rc_focus_wing, strategy,
                    outcome_kind, cited_memory_ids
               FROM recognition_events",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let retrieval_id: String = row.get("retrieval_id");
        assert!(!retrieval_id.is_empty(), "retrieval_id minted");
        assert_eq!(row.get::<String, _>("session_id"), "sess-1");
        assert_eq!(row.get::<String, _>("rc_persona"), "henry");
        assert_eq!(row.get::<String, _>("rc_focus_wing"), "permagent");
        assert_eq!(row.get::<String, _>("strategy"), "cascade");
        assert!(row.get::<Option<String>, _>("outcome_kind").is_none());
        assert_eq!(row.get::<String, _>("cited_memory_ids"), "[]");

        let members: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM recognition_set_members WHERE retrieval_id = ?",
        )
        .bind(&retrieval_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(members, 2, "whole retrieved set persisted");
    }

    #[tokio::test]
    async fn persists_empty_set() {
        let pool = test_pool().await;
        persist_recognition(&pool, &ctx("sess-empty"), "q", "cascade", &[])
            .await
            .unwrap();
        let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recognition_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(events, 1, "event persists even with no hits");
    }

    #[tokio::test]
    async fn task_write_back_keys_on_session() {
        let pool = test_pool().await;
        persist_recognition(
            &pool,
            &ctx("sess-x"),
            "q",
            "cascade",
            &[("m".into(), 0.9, 0)],
        )
        .await
        .unwrap();
        // Unrelated session must stay untouched.
        persist_recognition(&pool, &ctx("sess-y"), "q2", "cascade", &[])
            .await
            .unwrap();

        write_back_task_outcome(&pool, "sess-x").await;

        let kind: Option<String> = sqlx::query_scalar(
            "SELECT outcome_kind FROM recognition_events WHERE session_id = 'sess-x'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(kind.as_deref(), Some("TaskResolved"));

        let untouched: Option<String> = sqlx::query_scalar(
            "SELECT outcome_kind FROM recognition_events WHERE session_id = 'sess-y'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(untouched.is_none(), "other sessions untouched");
    }

    #[tokio::test]
    async fn decision_write_back_resolves_2hop_join() {
        let pool = test_pool().await;
        let worker_session = "worker-sess-1";
        persist_recognition(&pool, &ctx(worker_session), "q", "cascade", &[])
            .await
            .unwrap();

        // Seed a goal card whose metadata carries the worker session id.
        sqlx::query(
            "INSERT INTO cards (id, project_id, card_type, title, column_id, metadata_json)
             VALUES ('goal-1', '00000000-0000-0000-0000-000000000001', 'goal', 'G',
                     'col-personal-backlog', ?)",
        )
        .bind(serde_json::json!({ "worker_session_id": worker_session }).to_string())
        .execute(&pool)
        .await
        .unwrap();

        write_back_decision_outcome(&pool, "goal-1", false).await;

        let row = sqlx::query(
            "SELECT outcome_kind, outcome_polarity, outcome_source
               FROM recognition_events WHERE session_id = ?",
        )
        .bind(worker_session)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            row.get::<Option<String>, _>("outcome_kind").as_deref(),
            Some("DecisionBounced")
        );
        assert_eq!(
            row.get::<Option<String>, _>("outcome_polarity").as_deref(),
            Some("Negative")
        );
        assert_eq!(
            row.get::<Option<String>, _>("outcome_source").as_deref(),
            Some("Decision")
        );
    }
}
