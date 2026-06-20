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

// ---------------------------------------------------------------------------
// Read side (the #360 initiative-layer glue).
//
// These are the ONLY query accessors over `recognition_events`. They expose the
// novelty + frequency signals the ambient goal-origination loop uses to prune
// timing and suppress already-declined proposals. Both are best-effort: a read
// failure degrades to "no signal" (count 0 / None), never an error path.
// ---------------------------------------------------------------------------

/// A prior recognition of an observation: when it was last seen and how it
/// resolved. `outcome_*` stay `None` until a write-back attributes the row.
#[derive(Debug, Clone)]
pub struct RecognitionSeen {
    pub retrieved_at: String,
    pub outcome_kind: Option<String>,
    pub outcome_polarity: Option<String>,
}

impl RecognitionSeen {
    /// True when a prior proposal for this observation was declined — the
    /// caller should suppress re-origination (the quality half of the flywheel).
    pub fn was_bounced(&self) -> bool {
        self.outcome_polarity.as_deref() == Some("Negative")
    }
}

/// Count recognition events for a session within the last `within_secs` (the
/// frequency signal). `retrieved_at` is fixed-format UTC ISO-8601, which sorts
/// lexically as chronologically, so a string lower-bound is exact. Returns 0
/// on empty session or any error.
pub async fn recent_recognition_count(
    pool: &Pool<Sqlite>,
    session_id: &str,
    within_secs: i64,
) -> i64 {
    if session_id.is_empty() {
        return 0;
    }
    let cutoff = (Utc::now() - chrono::Duration::seconds(within_secs.max(0)))
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM recognition_events
          WHERE session_id = ? AND retrieved_at >= ?",
    )
    .bind(session_id)
    .bind(&cutoff)
    .fetch_one(pool)
    .await
    .unwrap_or(0)
}

/// Look up whether an observation (exact `query` string) has been recognized
/// before, returning the MOST RECENT prior occurrence (the novelty + pruning
/// signal). `None` = never seen → novel, ripe to originate. A returned row with
/// `was_bounced()` means a prior proposal was declined. `None` on error.
pub async fn seen_observation(pool: &Pool<Sqlite>, query: &str) -> Option<RecognitionSeen> {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT retrieved_at, outcome_kind, outcome_polarity
           FROM recognition_events
          WHERE query = ?
          ORDER BY retrieved_at DESC
          LIMIT 1",
    )
    .bind(query)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;
    Some(RecognitionSeen {
        retrieved_at: row.get("retrieved_at"),
        outcome_kind: row.get("outcome_kind"),
        outcome_polarity: row.get("outcome_polarity"),
    })
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

    // ----- read side (#360 glue) -----

    #[tokio::test]
    async fn seen_observation_novel_is_none() {
        let pool = test_pool().await;
        assert!(
            seen_observation(&pool, "git status && git pull")
                .await
                .is_none(),
            "an unseen observation is novel"
        );
    }

    #[tokio::test]
    async fn seen_observation_returns_positive_outcome() {
        let pool = test_pool().await;
        let q = "git status && git pull --ff-only";
        persist_recognition(&pool, &ctx("sess-pos"), q, "cascade", &[])
            .await
            .unwrap();
        write_back_task_outcome(&pool, "sess-pos").await;

        let seen = seen_observation(&pool, q).await.expect("observation seen");
        assert_eq!(seen.outcome_kind.as_deref(), Some("TaskResolved"));
        assert_eq!(seen.outcome_polarity.as_deref(), Some("Positive"));
        assert!(!seen.was_bounced(), "positive outcome is not a bounce");
    }

    #[tokio::test]
    async fn seen_observation_flags_bounced() {
        let pool = test_pool().await;
        let worker_session = "worker-sess-bounce";
        let q = "npm run build:all";
        persist_recognition(&pool, &ctx(worker_session), q, "cascade", &[])
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO cards (id, project_id, card_type, title, column_id, metadata_json)
             VALUES ('goal-b', '00000000-0000-0000-0000-000000000001', 'goal', 'G',
                     'col-personal-backlog', ?)",
        )
        .bind(serde_json::json!({ "worker_session_id": worker_session }).to_string())
        .execute(&pool)
        .await
        .unwrap();
        write_back_decision_outcome(&pool, "goal-b", false).await;

        let seen = seen_observation(&pool, q).await.expect("observation seen");
        assert!(seen.was_bounced(), "declined proposal must read as bounced");
    }

    #[tokio::test]
    async fn seen_observation_returns_most_recent() {
        let pool = test_pool().await;
        let q = "cargo test -p permagent";
        // An older row (resolved) then a newer one (still open).
        sqlx::query(
            "INSERT INTO recognition_events
                (retrieval_id, session_id, query, retrieved_at, rc_persona, strategy, outcome_kind)
             VALUES ('r-old', 'sess-1', ?, '2020-01-01T00:00:00.000Z', '', 'cascade', 'TaskResolved')",
        )
        .bind(q)
        .execute(&pool)
        .await
        .unwrap();
        persist_recognition(&pool, &ctx("sess-1"), q, "cascade", &[])
            .await
            .unwrap();

        let seen = seen_observation(&pool, q).await.expect("seen");
        assert!(
            seen.outcome_kind.is_none(),
            "most-recent row wins (the still-open one)"
        );
    }

    #[tokio::test]
    async fn recent_count_windows_by_time() {
        let pool = test_pool().await;
        // Two fresh events in the session.
        persist_recognition(&pool, &ctx("sess-c"), "q1", "cascade", &[])
            .await
            .unwrap();
        persist_recognition(&pool, &ctx("sess-c"), "q2", "cascade", &[])
            .await
            .unwrap();
        // One ancient event in the same session, outside any sane window.
        sqlx::query(
            "INSERT INTO recognition_events
                (retrieval_id, session_id, query, retrieved_at, rc_persona, strategy)
             VALUES ('r-ancient', 'sess-c', 'q-old', '2020-01-01T00:00:00.000Z', '', 'cascade')",
        )
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(
            recent_recognition_count(&pool, "sess-c", 3600).await,
            2,
            "only the two fresh events fall inside the hour window"
        );
        assert_eq!(
            recent_recognition_count(&pool, "other", 3600).await,
            0,
            "unrelated session has none"
        );
        assert_eq!(
            recent_recognition_count(&pool, "", 3600).await,
            0,
            "empty session short-circuits"
        );
    }
}
