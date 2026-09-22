use super::{Answer, FallbackReason};
use chrono::{SecondsFormat, Utc};
use serde_json::Value;
use sqlx::{Pool, Row, Sqlite};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateRef(String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("state id must be 1–64 characters without whitespace or control characters")]
pub struct InvalidStateId;

impl StateRef {
    pub fn from_id(id: impl Into<String>) -> Result<Self, InvalidStateId> {
        let id = id.into();
        if id.is_empty()
            || id.chars().count() > 64
            || id.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(InvalidStateId);
        }
        Ok(Self(id))
    }

    pub fn hashed(state_text: &str) -> Self {
        use sha2::{Digest, Sha256};
        Self(hex::encode(Sha256::digest(state_text.as_bytes())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct DecisionJournalEntry {
    pub site: String,
    pub question_version: String,
    pub state_ref: StateRef,
    pub backend: String,
    pub distribution: Vec<Answer>,
    pub acted: bool,
    pub fallback_reason: Option<FallbackReason>,
    pub latency_ms: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecisionJournalRow {
    pub id: String,
    pub ts: String,
    pub site: String,
    pub question_version: String,
    pub state_ref: String,
    pub backend: String,
    pub distribution_json: String,
    pub acted: bool,
    pub fallback_reason: Option<String>,
    pub latency_ms: i64,
}

#[derive(Clone, Debug)]
pub struct DecisionJournal {
    pool: Arc<Pool<Sqlite>>,
}

impl DecisionJournal {
    pub fn new(pool: Pool<Sqlite>) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }

    pub async fn record(&self, entry: &DecisionJournalEntry) {
        let id = Uuid::now_v7().to_string();
        let ts = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let fallback_reason = entry.fallback_reason.as_ref().map(fallback_reason_code);
        let distribution_json = match serde_json::to_string(&entry.distribution) {
            Ok(json) => json,
            Err(error) => {
                tracing::warn!(
                    site = entry.site.as_str(),
                    error = %format!("{error:#}"),
                    "failed to serialize decision_journal distribution"
                );
                return;
            }
        };
        let result = sqlx::query(
            "INSERT INTO decision_journal (
                id, ts, site, question_version, state_ref, backend,
                distribution_json, acted, fallback_reason, latency_ms
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(ts)
        .bind(entry.site.as_str())
        .bind(entry.question_version.as_str())
        .bind(entry.state_ref.as_str())
        .bind(entry.backend.as_str())
        .bind(distribution_json)
        .bind(i64::from(entry.acted))
        .bind(fallback_reason)
        .bind(entry.latency_ms)
        .execute(self.pool.as_ref())
        .await;
        if let Err(error) = result {
            tracing::warn!(
                site = entry.site.as_str(),
                error = %format!("{error:#}"),
                "failed to persist decision_journal entry"
            );
        }
    }

    pub async fn recent(&self, site: &str, limit: i64) -> anyhow::Result<Vec<DecisionJournalRow>> {
        let rows = sqlx::query(
            "SELECT id, ts, site, question_version, state_ref, backend,
                    distribution_json, acted, fallback_reason, latency_ms
             FROM decision_journal
             WHERE site = ?
             ORDER BY ts DESC, id DESC
             LIMIT ?",
        )
        .bind(site)
        .bind(limit)
        .fetch_all(self.pool.as_ref())
        .await?;

        rows.into_iter()
            .map(|row| -> anyhow::Result<DecisionJournalRow> {
                let distribution_json: String = row.try_get("distribution_json")?;
                let _: Value = serde_json::from_str(&distribution_json)?;
                Ok(DecisionJournalRow {
                    id: row.try_get("id")?,
                    ts: row.try_get("ts")?,
                    site: row.try_get("site")?,
                    question_version: row.try_get("question_version")?,
                    state_ref: row.try_get("state_ref")?,
                    backend: row.try_get("backend")?,
                    distribution_json,
                    acted: row.try_get::<i64, _>("acted")? != 0,
                    fallback_reason: row.try_get("fallback_reason")?,
                    latency_ms: row.try_get("latency_ms")?,
                })
            })
            .collect()
    }
}

pub fn fallback_reason_code(reason: &FallbackReason) -> String {
    match reason {
        FallbackReason::BudgetRefused(super::BudgetRefusal::NoTaskIdentity) => {
            "budget_refused:no_task_identity".to_string()
        }
        FallbackReason::BudgetRefused(super::BudgetRefusal::CeilingExhausted { .. }) => {
            "budget_refused:ceiling_exhausted".to_string()
        }
        FallbackReason::PrimaryAbsent => "primary_absent".to_string(),
        FallbackReason::Auth => "auth".to_string(),
        FallbackReason::Validation(_) => "validation".to_string(),
        FallbackReason::RateLimited => "rate_limited".to_string(),
        FallbackReason::RateLimitedLocally => "rate_limited_locally".to_string(),
        FallbackReason::PrimaryFailed(_) => "primary_failed".to_string(),
    }
}
