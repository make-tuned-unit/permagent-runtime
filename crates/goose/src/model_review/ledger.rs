//! Review state for the Model Review findings ledger (W1b).
//!
//! State lives in its own table, keyed by the finding key, and a weekly rescan
//! never touches it: re-persisting a finding updates `model_review_findings`
//! only, so a dismissal or deferral survives every later scan of the same
//! consumer/incumbent/candidate/action (Astra W1 ruling 12–13: one ledger, one
//! finding one decision, never the storage Trash/Keep actions). An open finding
//! has no state row.
use super::findings::Finding;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

/// A single request may act on at most this many findings.
pub const MAX_KEYS_PER_ACTION: usize = 500;
/// A deferral may not reach further than this; past it, dismiss instead.
pub const MAX_DEFER_DAYS: i64 = 180;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingState {
    Open,
    Dismissed,
    Deferred,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub key: String,
    pub report_id: String,
    /// Effective state: a deferral whose `defer_until` has passed reads `open`.
    pub state: FindingState,
    pub defer_until: Option<String>,
    /// Set by W1c when the finding raises a Decision Inbox approval.
    pub decision_id: Option<String>,
    pub updated_at: Option<String>,
    pub finding: Finding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingAction {
    Dismiss,
    Defer,
    Reopen,
}

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct ActionOutcome {
    pub updated: Vec<String>,
    pub unknown: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ActionError {
    /// The request itself is wrong; nothing was written.
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Storage(#[from] sqlx::Error),
}

fn effective_state(
    stored: Option<&str>,
    defer_until: Option<&str>,
    now: DateTime<Utc>,
) -> FindingState {
    match stored {
        Some("dismissed") => FindingState::Dismissed,
        Some("deferred") => {
            let still_deferred = defer_until
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                .is_some_and(|t| t.with_timezone(&Utc) > now);
            if still_deferred {
                FindingState::Deferred
            } else {
                FindingState::Open
            }
        }
        _ => FindingState::Open,
    }
}

fn state_rank(state: FindingState) -> u8 {
    match state {
        FindingState::Open => 0,
        FindingState::Deferred => 1,
        FindingState::Dismissed => 2,
    }
}

/// Every finding in the ledger with its effective review state: open first,
/// then deferred, then dismissed; by consumer, then key, within each group.
pub async fn entries(pool: &SqlitePool, now: DateTime<Utc>) -> anyhow::Result<Vec<LedgerEntry>> {
    let rows = sqlx::query(
        "SELECT f.finding_key, f.report_id, f.consumer, f.finding_json,
                s.state, s.defer_until, s.decision_id, s.updated_at
         FROM model_review_findings f
         LEFT JOIN model_review_finding_states s ON s.finding_key = f.finding_key",
    )
    .fetch_all(pool)
    .await?;
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        let stored: Option<String> = row.try_get("state")?;
        let defer_until: Option<String> = row.try_get("defer_until")?;
        let finding_json: String = row.try_get("finding_json")?;
        entries.push((
            row.try_get::<String, _>("consumer")?,
            LedgerEntry {
                key: row.try_get("finding_key")?,
                report_id: row.try_get("report_id")?,
                state: effective_state(stored.as_deref(), defer_until.as_deref(), now),
                defer_until,
                decision_id: row.try_get("decision_id")?,
                updated_at: row.try_get("updated_at")?,
                finding: serde_json::from_str(&finding_json)?,
            },
        ));
    }
    entries.sort_by(|(ca, a), (cb, b)| {
        (state_rank(a.state), ca, &a.key).cmp(&(state_rank(b.state), cb, &b.key))
    });
    Ok(entries.into_iter().map(|(_, e)| e).collect())
}

/// Apply one action to many findings in a single transaction. Keys that are
/// not in the ledger are reported back, not treated as an error; a malformed
/// request writes nothing. `decision_id` survives every action.
pub async fn apply_action(
    pool: &SqlitePool,
    keys: &[String],
    action: FindingAction,
    defer_until: Option<&str>,
    now: DateTime<Utc>,
) -> Result<ActionOutcome, ActionError> {
    if keys.is_empty() {
        return Err(ActionError::Invalid("keys must not be empty".into()));
    }
    if keys.len() > MAX_KEYS_PER_ACTION {
        return Err(ActionError::Invalid(format!(
            "at most {MAX_KEYS_PER_ACTION} keys per request"
        )));
    }
    let defer_until = match (action, defer_until) {
        (FindingAction::Defer, None) => {
            return Err(ActionError::Invalid("defer requires defer_until".into()))
        }
        (FindingAction::Defer, Some(raw)) => {
            let until = DateTime::parse_from_rfc3339(raw)
                .map_err(|_| ActionError::Invalid("defer_until must be RFC 3339".into()))?
                .with_timezone(&Utc);
            if until <= now {
                return Err(ActionError::Invalid(
                    "defer_until must be in the future".into(),
                ));
            }
            if until > now + Duration::days(MAX_DEFER_DAYS) {
                return Err(ActionError::Invalid(format!(
                    "defer_until must be within {MAX_DEFER_DAYS} days; dismiss instead"
                )));
            }
            Some(until.to_rfc3339())
        }
        _ => None,
    };

    let now_text = now.to_rfc3339();
    let mut outcome = ActionOutcome::default();
    let mut tx = pool.begin().await?;
    for key in keys {
        let known: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM model_review_findings WHERE finding_key = ?")
                .bind(key)
                .fetch_optional(&mut *tx)
                .await?;
        if known.is_none() {
            outcome.unknown.push(key.clone());
            continue;
        }
        match action {
            FindingAction::Reopen => {
                // Keep a decision link even when the finding goes back to open.
                sqlx::query(
                    "UPDATE model_review_finding_states SET state = 'open', defer_until = NULL, updated_at = ?
                     WHERE finding_key = ? AND decision_id IS NOT NULL",
                )
                .bind(&now_text)
                .bind(key)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "DELETE FROM model_review_finding_states WHERE finding_key = ? AND decision_id IS NULL",
                )
                .bind(key)
                .execute(&mut *tx)
                .await?;
            }
            FindingAction::Dismiss | FindingAction::Defer => {
                let state = if action == FindingAction::Dismiss {
                    "dismissed"
                } else {
                    "deferred"
                };
                sqlx::query(
                    "INSERT INTO model_review_finding_states(finding_key, state, defer_until, updated_at)
                     VALUES (?, ?, ?, ?)
                     ON CONFLICT(finding_key) DO UPDATE SET
                       state = excluded.state, defer_until = excluded.defer_until, updated_at = excluded.updated_at",
                )
                .bind(key)
                .bind(state)
                .bind(defer_until.as_deref())
                .bind(&now_text)
                .execute(&mut *tx)
                .await?;
            }
        }
        outcome.updated.push(key.clone());
    }
    tx.commit().await?;
    Ok(outcome)
}
