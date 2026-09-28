//! Durable route evidence for every physical provider call made by a native
//! goal worker, including local and subscription-backed calls with no money
//! reservation. This journal records invocation identity, not retry counts.
use anyhow::{anyhow, bail, Result};
use sqlx::{Pool, Sqlite};

use crate::goal_attempts::WorkerBinding;

pub(crate) const COVERAGE_VERSION: &str = "all_routes_v1";

/// Return the exact active native worker, or reject a session that belongs to
/// a terminal native attempt. A `None` result is reserved for sessions with no
/// native attempt history; terminal native workers may not fall through to
/// legacy/unbound accounting and dispatch after cancellation.
pub(crate) async fn active_worker_binding(
    pool: &Pool<Sqlite>,
    session_id: &str,
) -> Result<Option<WorkerBinding>> {
    if let Some(binding) = crate::goal_attempts::worker_binding(pool, session_id).await? {
        return Ok(Some(binding));
    }
    let native_history: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM goal_attempts a JOIN goal_execution_bindings b
         ON b.goal_id=a.goal_id WHERE a.worker_session_id=? AND b.authority='native'",
    )
    .bind(session_id)
    .fetch_one(pool)
    .await?;
    if native_history > 0 {
        bail!("native worker attempt is no longer active");
    }
    Ok(None)
}

pub(crate) async fn apply_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS goal_attempt_provider_calls (
            invocation_id TEXT PRIMARY KEY,
            goal_attempt_id TEXT NOT NULL REFERENCES goal_attempts(attempt_id),
            goal_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            task_id TEXT NOT NULL,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            coverage_version TEXT NOT NULL,
            dispatch_state TEXT NOT NULL DEFAULT 'not_started'
                CHECK(dispatch_state IN ('not_started','started_possible','not_sent')),
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_goal_attempt_provider_calls_attempt
         ON goal_attempt_provider_calls(goal_attempt_id, dispatch_state)",
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Record the exact route before dispatch. The transaction validates the
/// current journal binding and active worker, so neither session extension
/// metadata nor a stale in-memory claim can mint route evidence.
pub(crate) async fn record(
    pool: &Pool<Sqlite>,
    invocation_id: &str,
    binding: &WorkerBinding,
    provider: &str,
    model: &str,
) -> Result<()> {
    if invocation_id.trim().is_empty() || provider.trim().is_empty() || model.trim().is_empty() {
        bail!("native provider route identity must be complete");
    }
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM goal_attempts a
         JOIN goal_execution_bindings b ON b.goal_id=a.goal_id
         JOIN cards c ON c.id=a.goal_id JOIN board_columns bc ON bc.id=c.column_id
         WHERE a.attempt_id=? AND a.goal_id=? AND a.worker_session_id=?
           AND a.execution_state='started' AND b.authority='native' AND b.task_id=?
           AND json_extract(c.metadata_json,'$.dispatch_claim_id')=a.claim_id
           AND bc.state_binding='in_progress'",
    )
    .bind(&binding.attempt_id)
    .bind(&binding.goal_id)
    .bind(&binding.worker_session_id)
    .bind(&binding.task_id)
    .fetch_one(&mut *tx)
    .await?;
    if active != 1 {
        bail!("native provider route is not bound to the active worker attempt");
    }

    let existing: Option<(String, String, String, String, String, String)> = sqlx::query_as(
        "SELECT goal_attempt_id,goal_id,session_id,task_id,provider,model
         FROM goal_attempt_provider_calls WHERE invocation_id=?",
    )
    .bind(invocation_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(existing) = existing {
        let expected = (
            binding.attempt_id.as_str(),
            binding.goal_id.as_str(),
            binding.worker_session_id.as_str(),
            binding.task_id.as_str(),
            provider,
            model,
        );
        if (
            existing.0.as_str(),
            existing.1.as_str(),
            existing.2.as_str(),
            existing.3.as_str(),
            existing.4.as_str(),
            existing.5.as_str(),
        ) != expected
        {
            bail!("native provider invocation id was reused for another route or worker");
        }
        let state: String = sqlx::query_scalar(
            "SELECT dispatch_state FROM goal_attempt_provider_calls WHERE invocation_id=?",
        )
        .bind(invocation_id)
        .fetch_one(&mut *tx)
        .await?;
        if state != "not_started" {
            bail!("native provider invocation may already have been dispatched");
        }
        tx.commit().await?;
        return Ok(());
    }

    sqlx::query(
        "INSERT INTO goal_attempt_provider_calls
            (invocation_id,goal_attempt_id,goal_id,session_id,task_id,provider,model,coverage_version)
         VALUES (?,?,?,?,?,?,?,?)",
    )
    .bind(invocation_id)
    .bind(&binding.attempt_id)
    .bind(&binding.goal_id)
    .bind(&binding.worker_session_id)
    .bind(&binding.task_id)
    .bind(provider)
    .bind(model)
    .bind(COVERAGE_VERSION)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Mark immediately before entering provider transport. `not_started` is
/// idempotent only for the same active row; repeated dispatch after a marker
/// is refused to prevent replaying a possibly-sent native invocation.
pub(crate) async fn mark_dispatch_possible(
    pool: &Pool<Sqlite>,
    invocation_id: &str,
    binding: &WorkerBinding,
) -> Result<()> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let changed = sqlx::query(
        "UPDATE goal_attempt_provider_calls SET dispatch_state='started_possible',
             updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE invocation_id=? AND goal_attempt_id=? AND goal_id=? AND session_id=? AND task_id=?
           AND coverage_version=? AND dispatch_state='not_started'
           AND EXISTS (SELECT 1 FROM goal_attempts a JOIN goal_execution_bindings b
             ON b.goal_id=a.goal_id JOIN cards c ON c.id=a.goal_id
             JOIN board_columns bc ON bc.id=c.column_id
             WHERE a.attempt_id=? AND a.worker_session_id=?
             AND a.execution_state='started' AND b.authority='native' AND b.task_id=?
             AND json_extract(c.metadata_json,'$.dispatch_claim_id')=a.claim_id
             AND bc.state_binding='in_progress')",
    )
    .bind(invocation_id)
    .bind(&binding.attempt_id)
    .bind(&binding.goal_id)
    .bind(&binding.worker_session_id)
    .bind(&binding.task_id)
    .bind(COVERAGE_VERSION)
    .bind(&binding.attempt_id)
    .bind(&binding.worker_session_id)
    .bind(&binding.task_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed == 1 {
        tx.commit().await?;
        return Ok(());
    }
    let state: Option<String> = sqlx::query_scalar(
        "SELECT dispatch_state FROM goal_attempt_provider_calls WHERE invocation_id=?
         AND goal_attempt_id=? AND goal_id=? AND session_id=? AND task_id=? AND coverage_version=?",
    )
    .bind(invocation_id)
    .bind(&binding.attempt_id)
    .bind(&binding.goal_id)
    .bind(&binding.worker_session_id)
    .bind(&binding.task_id)
    .bind(COVERAGE_VERSION)
    .fetch_optional(&mut *tx)
    .await?;
    if state.as_deref() == Some("started_possible") {
        bail!("native provider invocation may already have been dispatched");
    }
    bail!("native provider route is missing, terminal, or no longer active")
}

/// Clear the dispatch possibility only when the caller has authoritative
/// evidence that the whole invocation was never sent.
pub(crate) async fn mark_not_sent(pool: &Pool<Sqlite>, invocation_id: &str) -> Result<()> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let changed = sqlx::query(
        "UPDATE goal_attempt_provider_calls SET dispatch_state='not_sent',
             updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE invocation_id=? AND dispatch_state IN ('not_started','started_possible')",
    )
    .bind(invocation_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed > 1 {
        return Err(anyhow!("native provider invocation identity is not unique"));
    }
    tx.commit().await?;
    Ok(())
}
