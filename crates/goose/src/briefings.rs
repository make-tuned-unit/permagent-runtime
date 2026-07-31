//! Agent briefings — the reporting line from the worker agents to Henry.
//!
//! Before this, agent output went to surfaces a HUMAN reads and Henry never
//! saw it: the Steward posted destructive proposals and repo-health to the
//! cards board, the Watcher wrote insights into
//! `projects.metadata_json.watcher_insights`. Henry could only *recall* a
//! Steward report if a query happened to retrieve it from the Brain — pull,
//! and by luck.
//!
//! A briefing is the push half. An agent files one; Henry sees it in his
//! `<permagent_self>` brief on his next turn, whether or not he asks. The
//! agents report to Henry; Henry answers to the user.
//!
//! Deliberately NOT a message queue and NOT a second inbox:
//!   * Briefings are summaries, not work items. The card board stays the place
//!     where a human approves or rejects; a briefing about a destructive
//!     proposal carries the card's id in `ref_id` rather than duplicating it.
//!   * `acknowledged_at` marks that Henry has *seen* it, nothing more. It is
//!     not approval and must never be read as one.
//!
//! Writes are best-effort: a failed briefing is logged and dropped, never
//! propagated into the agent's own work. An agent's job does not fail because
//! it could not tell Henry about it.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Row, Sqlite};
use tracing::warn;
use uuid::Uuid;

/// How loudly a briefing asks for attention. Henry renders these differently
/// in his brief; nothing here changes control flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Routine. "Nothing needed from you."
    Info,
    /// Worth mentioning to the user unprompted.
    Attention,
    /// A human decision is pending somewhere (usually a card).
    ActionRequired,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Attention => "attention",
            Severity::ActionRequired => "action_required",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "action_required" => Severity::ActionRequired,
            "attention" => Severity::Attention,
            _ => Severity::Info,
        }
    }

    /// Human-facing form, as it appears in Henry's brief.
    pub fn render(&self) -> &'static str {
        match self {
            Severity::ActionRequired => "action required",
            Severity::Attention => "attention",
            Severity::Info => "info",
        }
    }
}

/// Display name for a reporting agent, resolved from the worker roster so the
/// brief says "Steward" rather than the raw key. Falls back to the key
/// capitalized — an unknown agent still reports under a readable name rather
/// than being dropped or mislabelled as a known one.
pub fn display_name_for(agent_key: &str) -> String {
    let config = crate::config::agent_identity::load_agent_config();
    if let Some(worker) = config.workers.get(agent_key) {
        return worker.display_name();
    }
    let mut chars = agent_key.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => agent_key.to_string(),
    }
}

/// One report from one agent to Henry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Briefing {
    pub id: String,
    /// Worker persona key of the reporting agent ("steward", "watcher", ...).
    /// Matches `agent_identity` roster keys so the display name resolves.
    pub from_agent: String,
    /// Coarse class: "repo_health", "destructive_proposal", "insight", ...
    pub kind: String,
    pub severity: Severity,
    /// One line. This is what lands in Henry's brief, so it must stand alone.
    pub summary: String,
    /// Optional longer body, available on demand rather than in every prompt.
    pub detail: Option<String>,
    /// Link to the thing this is about, when one exists ("card", "memory").
    pub ref_kind: Option<String>,
    pub ref_id: Option<String>,
    pub created_at: String,
    pub acknowledged_at: Option<String>,
}

/// A briefing to file. `id` and `created_at` are minted here.
#[derive(Debug, Clone)]
pub struct NewBriefing {
    pub from_agent: String,
    pub kind: String,
    pub severity: Severity,
    pub summary: String,
    pub detail: Option<String>,
    pub ref_kind: Option<String>,
    pub ref_id: Option<String>,
}

/// File a briefing. Best-effort by contract — see the module docs. Returns the
/// briefing id on success so a caller can correlate its own logs.
pub async fn file_briefing(pool: &Pool<Sqlite>, briefing: NewBriefing) -> Option<String> {
    let id = Uuid::now_v7().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let result = sqlx::query(
        "INSERT INTO agent_briefings
            (id, from_agent, kind, severity, summary, detail, ref_kind, ref_id, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&briefing.from_agent)
    .bind(&briefing.kind)
    .bind(briefing.severity.as_str())
    .bind(&briefing.summary)
    .bind(&briefing.detail)
    .bind(&briefing.ref_kind)
    .bind(&briefing.ref_id)
    .bind(&now)
    .execute(pool)
    .await;

    match result {
        Ok(_) => Some(id),
        Err(e) => {
            warn!(
                target: "permagent::briefings",
                "Failed to file briefing from {}: {}", briefing.from_agent, e
            );
            None
        }
    }
}

/// Unacknowledged briefings, most severe first then newest first, capped.
///
/// The ordering is what makes the cap safe: if Henry can only carry N lines,
/// the ones he loses are the least urgent, not the oldest.
pub async fn unacknowledged(pool: &Pool<Sqlite>, limit: i64) -> Vec<Briefing> {
    match try_unacknowledged(pool, limit).await {
        Ok(rows) => rows,
        Err(e) => {
            warn!(target: "permagent::briefings", "Failed to read briefings: {}", e);
            Vec::new()
        }
    }
}

/// Fallible read for callers that must distinguish "nothing pending" from
/// "the briefing store could not be queried" (notably app perception).
pub async fn try_unacknowledged(pool: &Pool<Sqlite>, limit: i64) -> Result<Vec<Briefing>> {
    let rows = sqlx::query(
        "SELECT id, from_agent, kind, severity, summary, detail, ref_kind, ref_id,
                created_at, acknowledged_at
           FROM agent_briefings
          WHERE acknowledged_at IS NULL
          ORDER BY CASE severity
                     WHEN 'action_required' THEN 0
                     WHEN 'attention'       THEN 1
                     ELSE 2
                   END,
                   created_at DESC
          LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(row_to_briefing).collect())
}

/// How many briefings are waiting, by severity — cheap enough for a HUD poll.
pub async fn unacknowledged_count(pool: &Pool<Sqlite>) -> i64 {
    try_unacknowledged_count(pool).await.unwrap_or(0)
}

/// Fallible count for honest read surfaces.
pub async fn try_unacknowledged_count(pool: &Pool<Sqlite>) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM agent_briefings WHERE acknowledged_at IS NULL",
    )
    .fetch_one(pool)
    .await?)
}

/// Mark briefings seen. Acknowledgement means Henry read it — never that a
/// human approved anything.
pub async fn acknowledge(pool: &Pool<Sqlite>, ids: &[String]) -> Result<u64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let now = chrono::Utc::now().to_rfc3339();
    let placeholders = std::iter::repeat("?")
        .take(ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "UPDATE agent_briefings SET acknowledged_at = ?
          WHERE acknowledged_at IS NULL AND id IN ({placeholders})"
    );
    let mut q = sqlx::query(&sql).bind(now);
    for id in ids {
        q = q.bind(id);
    }
    Ok(q.execute(pool).await?.rows_affected())
}

/// Clear briefings that point at a thing which has now been resolved — a card
/// decided, a project closed. Returns how many were cleared.
///
/// This is the counterpart to the render-time acknowledgement in
/// `reply_parts`: `Info` clears once Henry has seen it, but `Attention` and
/// `ActionRequired` survive being seen, precisely because they are waiting on
/// something. Without this they would wait forever, and Henry would keep
/// reporting a force-push decision that was made last week.
pub async fn resolve_for_ref(pool: &Pool<Sqlite>, ref_kind: &str, ref_id: &str) -> u64 {
    let now = chrono::Utc::now().to_rfc3339();
    match sqlx::query(
        "UPDATE agent_briefings SET acknowledged_at = ?
          WHERE acknowledged_at IS NULL AND ref_kind = ? AND ref_id = ?",
    )
    .bind(now)
    .bind(ref_kind)
    .bind(ref_id)
    .execute(pool)
    .await
    {
        Ok(r) => r.rows_affected(),
        Err(e) => {
            warn!(
                target: "permagent::briefings",
                "Failed to resolve briefings for {ref_kind}/{ref_id}: {e}"
            );
            0
        }
    }
}

fn row_to_briefing(row: sqlx::sqlite::SqliteRow) -> Briefing {
    Briefing {
        id: row.get("id"),
        from_agent: row.get("from_agent"),
        kind: row.get("kind"),
        severity: Severity::from_str(&row.get::<String, _>("severity")),
        summary: row.get("summary"),
        detail: row.get("detail"),
        ref_kind: row.get("ref_kind"),
        ref_id: row.get("ref_id"),
        created_at: row.get("created_at"),
        acknowledged_at: row.get("acknowledged_at"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::spectral_schema::apply_briefings_schema;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        apply_briefings_schema(&pool).await.unwrap();
        pool
    }

    fn new(from: &str, sev: Severity, summary: &str) -> NewBriefing {
        NewBriefing {
            from_agent: from.into(),
            kind: "repo_health".into(),
            severity: sev,
            summary: summary.into(),
            detail: None,
            ref_kind: None,
            ref_id: None,
        }
    }

    #[tokio::test]
    async fn files_and_reads_back_unacknowledged() {
        let pool = pool().await;
        file_briefing(&pool, new("steward", Severity::Info, "all clean")).await;
        let out = unacknowledged(&pool, 10).await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].from_agent, "steward");
        assert_eq!(out[0].severity, Severity::Info);
        assert_eq!(unacknowledged_count(&pool).await, 1);
    }

    /// The cap must drop the least urgent, not the oldest — otherwise a burst
    /// of routine chatter buries the one thing needing a decision.
    #[tokio::test]
    async fn severity_orders_ahead_of_recency() {
        let pool = pool().await;
        file_briefing(&pool, new("watcher", Severity::Info, "routine one")).await;
        file_briefing(&pool, new("watcher", Severity::Info, "routine two")).await;
        file_briefing(
            &pool,
            new("steward", Severity::ActionRequired, "force-push proposed"),
        )
        .await;

        let top = unacknowledged(&pool, 1).await;
        assert_eq!(top.len(), 1);
        assert_eq!(
            top[0].summary, "force-push proposed",
            "the action_required briefing must survive the cap"
        );
    }

    #[tokio::test]
    async fn acknowledge_hides_and_is_idempotent() {
        let pool = pool().await;
        file_briefing(&pool, new("steward", Severity::Attention, "stale branches")).await;
        let ids: Vec<String> = unacknowledged(&pool, 10)
            .await
            .into_iter()
            .map(|b| b.id)
            .collect();

        assert_eq!(acknowledge(&pool, &ids).await.unwrap(), 1);
        assert!(unacknowledged(&pool, 10).await.is_empty());
        // Re-acknowledging affects nothing — the guard is on acknowledged_at.
        assert_eq!(acknowledge(&pool, &ids).await.unwrap(), 0);
    }

    /// An action_required briefing must clear when its card is decided —
    /// otherwise Henry keeps reporting a decision the user already made.
    #[tokio::test]
    async fn resolving_the_referenced_card_clears_the_briefing() {
        let pool = pool().await;
        file_briefing(
            &pool,
            NewBriefing {
                from_agent: "steward".into(),
                kind: "destructive_proposal".into(),
                severity: Severity::ActionRequired,
                summary: "force-push proposed".into(),
                detail: None,
                ref_kind: Some("card".into()),
                ref_id: Some("card-1".into()),
            },
        )
        .await;
        // An unrelated card resolving must not clear it.
        assert_eq!(resolve_for_ref(&pool, "card", "card-2").await, 0);
        assert_eq!(unacknowledged(&pool, 10).await.len(), 1);

        assert_eq!(resolve_for_ref(&pool, "card", "card-1").await, 1);
        assert!(unacknowledged(&pool, 10).await.is_empty());
    }

    #[tokio::test]
    async fn empty_acknowledge_is_a_noop_not_a_full_table_update() {
        let pool = pool().await;
        file_briefing(&pool, new("steward", Severity::Info, "x")).await;
        assert_eq!(acknowledge(&pool, &[]).await.unwrap(), 0);
        assert_eq!(unacknowledged(&pool, 10).await.len(), 1);
    }
}
