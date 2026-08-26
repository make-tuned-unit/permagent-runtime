//! Durable activity journal (#619) — one reviewable "what did my agents do"
//! surface.
//!
//! An append-only `activity_journal` table (spectral schema v27) fed by the
//! existing [`crate::events`] emit() seam. A daemon-side consumer task
//! (goose-server `state.rs`) subscribes to the bus and calls [`record_event`]
//! for every event; [`entry_from_event`] selects the journal-worthy kinds
//! (goal transitions, decisions, librarian describe runs, Watcher nudges,
//! task failures) and maps their payloads defensively — a missing field
//! degrades to a generic label, never a dropped row.
//!
//! Discipline: the journal INDEXES the existing durable stores, it does not
//! duplicate them. `ref_kind`/`ref_id` point at the card / decision / memory /
//! task; bodies (decision detail, librarian descriptions, transcripts) stay in
//! their home tables. The only denormalized text is a short display title
//! (card title / decision headline), looked up best-effort at write time so
//! the timeline stays readable after the referent is deleted.
//!
//! The row `id` is the event-bus id (UUIDv7) and inserts are `OR IGNORE`, so
//! recording is idempotent under replay. `ts` is RFC3339 UTC milliseconds —
//! lexicographic order is chronological order, which the `?before=<ts>`
//! keyset page and the retention DELETE both rely on.

use anyhow::Result;
use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use sqlx::{Pool, Row, Sqlite};

use crate::events::{PermagentEvent, PermagentEventType};

/// Self-knowledge descriptor for the Home activity timeline — the user-facing
/// surface this journal feeds. Registered in
/// [`crate::agents::self_knowledge::SURFACE_DESCRIPTORS`].
pub const TIMELINE_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "timeline",
        display_name: "Activity timeline",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "A day-grouped timeline on the Home dashboard reading the durable activity journal — an append-only record of what you and your workers actually did, kept for 90 days and filterable by kind or actor: a goal move is attributed to the worker the goal is assigned to, or to the person or policy that authorized it; decisions name you, librarian describe runs name the librarian, and Watcher nudges name the Watcher; a task failure stays unattributed because the task record carries no worker",
        why_it_matters:
            "It is the user's reviewable answer to 'what did my agents do today and why' — a row points at its evidence (the goal card, the decision, the memory, or the article a news nudge is about) whenever the event that produced it carried one, so when the user asks what happened, point them here instead of reconstructing history from your context window",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[],
    };

/// Journal rows older than this are deleted by the startup retention pass
/// (the #560 standing size-cap discipline).
pub const RETENTION_DAYS: i64 = 90;

/// Display caps — the journal stores labels, not bodies.
const TITLE_MAX: usize = 120;
const DETAIL_MAX: usize = 300;

/// The closed set of journaled kinds. The `kind` filter on [`page`] is
/// validated against this list (unknown values simply match nothing), and the
/// frontend's filter chips are built from the same names.
pub const KNOWN_KINDS: [&str; 7] = [
    "goal_state_changed",
    "decision_created",
    "decision_resolved",
    "librarian_describe_completed",
    "proactive_nudge",
    "task_failed",
    "a2a_message",
];

/// A roster agent id. The inner `String` is private to this module so that
/// [`Actor::Agent`] cannot be built anywhere else from an arbitrary string:
/// [`Actor::resolve`] is the only door, and it admits only ids the /agents
/// roster publishes. That is what stops `actor` drifting back into free text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentId(String);

/// Who a journal row names. A closed vocabulary, because the actor is a JOIN
/// key: `GET /api/agents/{id}/work` filters the journal by exact actor, so a
/// value outside the roster is a row no review can ever reach.
///
/// [`Actor::Assistant`] ("henry") is the one member that is deliberately not a
/// roster id — Henry is the assistant, not a dispatchable agent — so decision
/// rows are readable on the timeline but do not appear under any agent's work
/// review. That is correct, not a gap: no agent did that work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    /// No knowable originator. Never a guess.
    System,
    /// The human operator.
    User,
    /// Henry himself.
    Assistant,
    /// Henry's autonomous decision policy (the `henry-policy` audit actor).
    Policy,
    /// A background worker or dispatch persona from the /agents roster.
    Agent(AgentId),
}

/// The roster ids an [`Actor::Agent`] may name: background-worker descriptor
/// ids plus the seeded dispatch-persona keys. Built once — `default_roster()`
/// allocates a full `WorkerPersona` per entry, and this is consulted on every
/// journaled event.
fn known_agent_ids() -> &'static std::collections::HashSet<String> {
    static IDS: std::sync::OnceLock<std::collections::HashSet<String>> = std::sync::OnceLock::new();
    IDS.get_or_init(|| {
        crate::agents::self_knowledge::WORKER_DESCRIPTORS
            .iter()
            .map(|d| d.id.to_string())
            .chain(crate::config::agent_identity::default_roster().into_keys())
            .collect()
    })
}

impl Actor {
    /// Resolve a producer-supplied actor string into the journal's joinable
    /// vocabulary. Anything unrecognized fails closed to [`Actor::System`]:
    /// an unjoinable actor is worse than an honest "not attributed".
    ///
    /// Known limitation, deliberate: the roster check is against the SEEDED
    /// personas only (no config file read, so this stays pure and hermetic in
    /// tests). A persona the operator adds to `agent.yaml` beyond the seeded
    /// set journals as `system` until it is seeded. Widening this means giving
    /// the journal a live roster read, which is a bigger change than #619's
    /// gap warrants.
    ///
    /// Note what is NOT here: the card-authorship literal `"user"`. Several
    /// producers hardcode `created_by: "user"` for any caller (see
    /// `goal_transition::insert_roadmap_goal`), so honouring it would launder
    /// an agent's insert into the human's name — the same defect that function's
    /// own comment records. Only `"jesse"`, which callers state deliberately as
    /// the decision-audit actor, resolves to a person.
    pub fn resolve(raw: &str) -> Self {
        match raw {
            "system" => Self::System,
            "jesse" => Self::User,
            "henry" => Self::Assistant,
            "henry-policy" => Self::Policy,
            _ => {
                // Producers spell agent ids both ways ("claude-code" on cards,
                // "claude_code" on the roster); normalize before the lookup.
                let id = raw.trim().to_lowercase().replace('-', "_");
                if known_agent_ids().contains(&id) {
                    Self::Agent(AgentId(id))
                } else {
                    Self::System
                }
            }
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::System => "system",
            Self::User => "jesse",
            Self::Assistant => "henry",
            Self::Policy => "henry-policy",
            Self::Agent(id) => &id.0,
        }
    }
}

/// A journal row ready to insert (write side).
#[derive(Debug, Clone)]
pub struct NewEntry {
    pub id: String,
    pub ts: String,
    pub kind: String,
    pub actor: Actor,
    pub title: String,
    pub detail: Option<String>,
    pub ref_kind: Option<String>,
    pub ref_id: Option<String>,
}

/// A journal row as served by `GET /api/activity` (read side).
/// `goal_project_id` is resolved live from `cards` for goal refs so the
/// frontend can deep-link `openGoalDetail(project_id, goal_id)`; it is NULL
/// for non-goal rows and for goals whose card has since been deleted.
#[derive(Debug, Clone, Serialize)]
pub struct JournalItem {
    pub id: String,
    pub ts: String,
    pub kind: String,
    pub actor: String,
    pub title: String,
    pub detail: Option<String>,
    pub ref_kind: Option<String>,
    pub ref_id: Option<String>,
    pub goal_project_id: Option<String>,
}

/// Truncate to at most `max` characters on a char boundary, appending an
/// ellipsis when anything was cut.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

fn payload_str<'a>(event: &'a PermagentEvent, key: &str) -> Option<&'a str> {
    event.payload.get(key).and_then(|v| v.as_str())
}

/// Human label for a goal-lifecycle state binding ("in_progress" → "In Progress").
fn state_label(binding: &str) -> String {
    binding
        .split('_')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Map a bus event to a journal entry, or `None` for kinds the journal skips.
///
/// Selected kinds: `goal_state_changed`, `decision_created`,
/// `decision_resolved`, `librarian_describe_completed`, `proactive_nudge`,
/// `task_failed`.
/// Everything else — including the chatty `stream_chunk` / `activity` firehose
/// and `browser_navigate_requested` (navigation noise) — is filtered here,
/// cheaply, before any DB work.
///
/// Pure and total over malformed payloads: every field read is defensive.
pub fn entry_from_event(event: &PermagentEvent) -> Option<NewEntry> {
    let ts = event.timestamp.to_rfc3339_opts(SecondsFormat::Millis, true);

    let (kind, actor, title, detail, ref_kind, ref_id) = match event.event_type {
        PermagentEventType::GoalStateChanged => {
            let goal_id = payload_str(event, "goal_id");
            let from = payload_str(event, "from");
            let to = payload_str(event, "to").unwrap_or("unknown");
            let detail = match (from, to) {
                (_, "deleted") => "Goal deleted".to_string(),
                (None, to) => format!("Goal created ({})", state_label(to)),
                (Some(f), t) => format!("Goal {} → {}", state_label(f), state_label(t)),
            };
            (
                "goal_state_changed",
                Actor::resolve(payload_str(event, "actor").unwrap_or("system")),
                // Placeholder title; record_event upgrades it to the card
                // title when the card still exists.
                goal_id
                    .map(|id| format!("Goal {}", truncate(id, 8)))
                    .unwrap_or_else(|| "Goal".to_string()),
                Some(detail),
                Some("goal"),
                goal_id.map(str::to_string),
            )
        }
        PermagentEventType::A2aMessage => {
            let from_goal = payload_str(event, "from_goal").unwrap_or("a goal");
            let to_goal = payload_str(event, "to_goal");
            let len = event.payload.get("body_len").and_then(|v| v.as_u64());
            let steered = event
                .payload
                .get("steered")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            // Fingerprint only. The body lives in the recipient's `a2a_inbox`,
            // which is where it was addressed; the journal proves the message
            // happened, it does not republish what one agent told another.
            // A plain prefix, not `truncate`: an ellipsis inside a hash reads
            // as part of the hash and makes the prefix unmatchable.
            let digest = payload_str(event, "body_sha256")
                .map(|h| h.chars().take(12).collect::<String>())
                .unwrap_or_else(|| "unhashed".to_string());
            let detail = format!(
                "A2A from {} · {} chars · sha256 {} · {}",
                truncate(from_goal, 8),
                len.unwrap_or(0),
                digest,
                if steered {
                    "delivered to a live worker"
                } else {
                    "queued in the inbox"
                }
            );
            (
                "a2a_message",
                Actor::resolve(payload_str(event, "actor").unwrap_or("system")),
                // Placeholder; record_event upgrades it to the RECIPIENT card's
                // title, which is the goal a reader wants to open.
                to_goal
                    .map(|id| format!("Goal {}", truncate(id, 8)))
                    .unwrap_or_else(|| "Goal".to_string()),
                Some(detail),
                Some("goal"),
                to_goal.map(str::to_string),
            )
        }
        PermagentEventType::DecisionCreated => {
            let decision_id = payload_str(event, "decision_id");
            let kind = payload_str(event, "kind").unwrap_or("unknown");
            let tier = event.payload.get("tier").and_then(|v| v.as_i64());
            let detail = match tier {
                Some(t) => format!("Decision requested · {kind} · tier {t}"),
                None => format!("Decision requested · {kind}"),
            };
            (
                "decision_created",
                // `NewDecision` carries no requester, so the honest actor is
                // Henry's inbox itself, not whichever caller happened to ask.
                Actor::Assistant,
                "Decision requested".to_string(),
                Some(detail),
                Some("decision"),
                decision_id.map(str::to_string),
            )
        }
        PermagentEventType::DecisionResolved => {
            let decision_id = payload_str(event, "decision_id");
            let answer = payload_str(event, "answer").unwrap_or("resolved");
            let acted_by = payload_str(event, "acted_by").unwrap_or("system");
            (
                "decision_resolved",
                Actor::resolve(acted_by),
                "Decision resolved".to_string(),
                Some(format!("Answered: {answer}")),
                Some("decision"),
                decision_id.map(str::to_string),
            )
        }
        PermagentEventType::LibrarianDescribeCompleted => {
            let memory_key = payload_str(event, "memory_key").unwrap_or("memory");
            let duration_ms = event.payload.get("duration_ms").and_then(|v| v.as_u64());
            // Evidence pointer only — the description body stays in the Brain.
            let detail = match duration_ms {
                Some(ms) => format!("Description written in {}s", (ms as f64 / 1000.0).round()),
                None => "Description written".to_string(),
            };
            (
                "librarian_describe_completed",
                Actor::resolve("librarian"),
                format!("Described '{}'", truncate(memory_key, 80)),
                Some(detail),
                Some("memory"),
                Some(memory_key.to_string()),
            )
        }
        PermagentEventType::TaskFailed => {
            let task_id = payload_str(event, "task_id");
            let error = payload_str(event, "error").unwrap_or("unknown error");
            (
                "task_failed",
                // Not knowable: `tasks` has no worker/agent column, only a
                // session id, and #GAP-B may not add one. Honest "system"
                // beats a guess joined off the session.
                Actor::System,
                "Task failed".to_string(),
                Some(error.to_string()),
                Some("task"),
                task_id.map(str::to_string),
            )
        }
        PermagentEventType::ProactiveNudge => {
            // The Watcher's initiative (#672): it chose to surface something.
            // The article link is the evidence for a news nudge, with the grounded
            // project as fallback when a user-declared topic has no link.
            let nudge_kind = payload_str(event, "kind").unwrap_or("nudge");
            let subject = payload_str(event, "subject").unwrap_or("something");
            let message = payload_str(event, "message");
            let title = match nudge_kind {
                "dormant_thread" => format!("Resurfaced '{}'", truncate(subject, 80)),
                "project_news" => format!("News: {}", truncate(subject, 80)),
                "rsi_heat" => format!("RSI heat: {}", truncate(subject, 80)),
                "sell_signal" => format!("Sell signal: {}", truncate(subject, 80)),
                "daily_pick" if subject == "none" => "No pick tomorrow".to_string(),
                "daily_pick" => format!("Tomorrow: {}", truncate(subject, 80)),
                _ => format!("Nudge: {}", truncate(subject, 80)),
            };
            // Financier scored the lot / named tomorrow's pick; the Watcher
            // delivered the nudge. Actor stays financier so the journal
            // attributes the fact.
            let actor = if matches!(nudge_kind, "rsi_heat" | "sell_signal" | "daily_pick") {
                Actor::resolve("financier")
            } else {
                Actor::resolve("watcher")
            };
            let (ref_kind, ref_id) = if nudge_kind == "dormant_thread" {
                (Some("memory"), Some(subject.to_string()))
            } else if matches!(nudge_kind, "rsi_heat" | "sell_signal")
                || (nudge_kind == "daily_pick" && subject != "none")
            {
                (Some("symbol"), Some(subject.to_string()))
            } else if let Some(url) = payload_str(event, "url").filter(|url| !url.is_empty()) {
                (Some("url"), Some(url.to_string()))
            } else if let Some(project_id) =
                payload_str(event, "project_id").filter(|project_id| !project_id.is_empty())
            {
                (Some("project"), Some(project_id.to_string()))
            } else {
                (None, None)
            };
            (
                "proactive_nudge",
                actor,
                title,
                message.map(str::to_string),
                ref_kind,
                ref_id,
            )
        }
        // Everything else — stream chunks, activity firehose, browser
        // navigation (noise per #619), agent state ticks, … — is not journaled.
        _ => return None,
    };

    Some(NewEntry {
        id: event.id.clone(),
        ts,
        kind: kind.to_string(),
        // No truncation: `Actor` is a closed vocabulary of short ids, so the
        // free-text length cap the `String` field needed is now structural.
        actor,
        title: truncate(&title, TITLE_MAX),
        detail: detail.map(|d| truncate(&d, DETAIL_MAX)),
        ref_kind: ref_kind.map(str::to_string),
        ref_id,
    })
}

/// Best-effort title upgrade from the referenced durable store: card title for
/// goals, decision headline for decisions. Lookup failure (deleted referent,
/// any DB error) keeps the placeholder title — the row must never be lost to
/// enrichment.
async fn enrich_title(pool: &Pool<Sqlite>, entry: &mut NewEntry) {
    let (sql, id) = match (entry.ref_kind.as_deref(), entry.ref_id.as_deref()) {
        (Some("goal"), Some(id)) => ("SELECT title FROM cards WHERE id = ?", id),
        (Some("decision"), Some(id)) => ("SELECT headline FROM decisions WHERE id = ?", id),
        _ => return,
    };
    if let Ok(Some(row)) = sqlx::query(sql).bind(id).fetch_optional(pool).await {
        if let Ok(label) = row.try_get::<String, _>(0) {
            if !label.is_empty() {
                entry.title = truncate(&label, TITLE_MAX);
            }
        }
    }
}

/// Insert a journal row. `OR IGNORE` on the event-id primary key makes this
/// idempotent under bus replay / double delivery.
pub async fn insert_entry(pool: &Pool<Sqlite>, entry: &NewEntry) -> Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO activity_journal
            (id, ts, kind, actor, title, detail, ref_kind, ref_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&entry.id)
    .bind(&entry.ts)
    .bind(&entry.kind)
    .bind(entry.actor.as_str())
    .bind(&entry.title)
    .bind(&entry.detail)
    .bind(&entry.ref_kind)
    .bind(&entry.ref_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// The consumer-task entry point: filter, enrich, insert. Returns whether the
/// event was journaled. Cheap for skipped kinds (no DB touch).
pub async fn record_event(pool: &Pool<Sqlite>, event: &PermagentEvent) -> Result<bool> {
    let Some(mut entry) = entry_from_event(event) else {
        return Ok(false);
    };
    enrich_title(pool, &mut entry).await;
    insert_entry(pool, &entry).await?;
    Ok(true)
}

/// Newest-first keyset page for `GET /api/activity`. `before` is an exclusive
/// `ts` cursor (pass the last row's `ts` to get the next page); `None` starts
/// from the newest row. `kinds` / `actor` filter server-side so pagination
/// stays correct under a filter; `kinds` is validated against [`KNOWN_KINDS`]
/// (an all-unknown filter matches nothing). Goal rows are enriched live with
/// the card's `project_id` for deep-linking. Ties on `ts` break on `id`
/// (UUIDv7, so id-order is emit-order); rows sharing the cursor's exact
/// millisecond are the documented keyset edge and can be skipped across a
/// page boundary.
pub async fn page(
    pool: &Pool<Sqlite>,
    before: Option<&str>,
    limit: i64,
    kinds: Option<&[String]>,
    actor: Option<&str>,
) -> Result<Vec<JournalItem>> {
    // The IN clause is assembled ONLY from KNOWN_KINDS members (static
    // literals) — requested kinds select which constants participate, their
    // bytes never reach the SQL.
    let kind_clause = match kinds {
        Some(requested) if !requested.is_empty() => {
            let valid: Vec<&str> = KNOWN_KINDS
                .iter()
                .copied()
                .filter(|k| requested.iter().any(|r| r.as_str() == *k))
                .collect();
            if valid.is_empty() {
                return Ok(Vec::new()); // filter names no known kind
            }
            format!(
                "AND aj.kind IN ({})",
                valid
                    .iter()
                    .map(|k| format!("'{k}'"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        _ => String::new(),
    };

    let sql = format!(
        "SELECT aj.id, aj.ts, aj.kind, aj.actor, aj.title, aj.detail,
                aj.ref_kind, aj.ref_id,
                CASE WHEN aj.ref_kind = 'goal' THEN c.project_id END AS goal_project_id
         FROM activity_journal aj
         LEFT JOIN cards c ON aj.ref_kind = 'goal' AND c.id = aj.ref_id
         WHERE (?1 IS NULL OR aj.ts < ?1)
           AND (?2 IS NULL OR aj.actor = ?2)
           {kind_clause}
         ORDER BY aj.ts DESC, aj.id DESC
         LIMIT ?3"
    );
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(before)
        .bind(actor)
        .bind(limit.clamp(1, 500))
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|row| JournalItem {
            id: row.get("id"),
            ts: row.get("ts"),
            kind: row.get("kind"),
            actor: row.get("actor"),
            title: row.get("title"),
            detail: row.get("detail"),
            ref_kind: row.get("ref_kind"),
            ref_id: row.get("ref_id"),
            goal_project_id: row.get("goal_project_id"),
        })
        .collect())
}

/// Retention pass: delete rows older than `days`. Returns the deleted count.
/// Run once at daemon startup (see goose-server `state.rs`).
pub async fn prune_older_than_days(pool: &Pool<Sqlite>, days: i64) -> Result<u64> {
    let cutoff =
        (Utc::now() - chrono::Duration::days(days)).to_rfc3339_opts(SecondsFormat::Millis, true);
    let result = sqlx::query("DELETE FROM activity_journal WHERE ts < ?")
        .bind(&cutoff)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events;
    use chrono::Duration;

    async fn test_pool() -> Pool<Sqlite> {
        use crate::session::spectral_schema::init_spectral_db;
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    fn entry(id: &str, ts: &str) -> NewEntry {
        NewEntry {
            id: id.to_string(),
            ts: ts.to_string(),
            kind: "task_failed".to_string(),
            actor: Actor::System,
            title: format!("entry {id}"),
            detail: None,
            ref_kind: None,
            ref_id: None,
        }
    }

    #[tokio::test]
    async fn migrate_v26_to_v27_idempotent() {
        let pool = test_pool().await; // fresh init already created the table
        crate::session::spectral_schema::migrate_v26_to_v27(&pool)
            .await
            .unwrap();
        crate::session::spectral_schema::migrate_v26_to_v27(&pool)
            .await
            .unwrap();
        // Table usable after double application.
        insert_entry(&pool, &entry("e1", "2026-07-10T00:00:00.000Z"))
            .await
            .unwrap();
        assert_eq!(page(&pool, None, 10, None, None).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn insert_and_page_newest_first() {
        let pool = test_pool().await;
        for (id, ts) in [
            ("a", "2026-07-08T10:00:00.000Z"),
            ("b", "2026-07-09T10:00:00.000Z"),
            ("c", "2026-07-10T10:00:00.000Z"),
        ] {
            insert_entry(&pool, &entry(id, ts)).await.unwrap();
        }

        let items = page(&pool, None, 10, None, None).await.unwrap();
        assert_eq!(
            items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            vec!["c", "b", "a"]
        );

        // Keyset cursor: page after the newest row's ts.
        let next = page(&pool, Some(&items[0].ts), 10, None, None)
            .await
            .unwrap();
        assert_eq!(
            next.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            vec!["b", "a"]
        );

        // Limit respected.
        let limited = page(&pool, None, 2, None, None).await.unwrap();
        assert_eq!(limited.len(), 2);
        assert_eq!(limited[0].id, "c");
    }

    #[tokio::test]
    async fn insert_is_idempotent_on_id() {
        let pool = test_pool().await;
        let e = entry("dup", "2026-07-10T10:00:00.000Z");
        insert_entry(&pool, &e).await.unwrap();
        insert_entry(&pool, &e).await.unwrap();
        assert_eq!(page(&pool, None, 10, None, None).await.unwrap().len(), 1);
    }

    /// The other half of the guarantee — that the retention pass is still
    /// allowed through the DELETE guard — is `prune_removes_only_old_rows`.
    #[tokio::test]
    async fn journal_rejects_rewrites_and_in_window_deletes() {
        let pool = test_pool().await;
        let fresh_ts = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        insert_entry(&pool, &entry("fresh", &fresh_ts))
            .await
            .unwrap();

        let update = sqlx::query("UPDATE activity_journal SET title = 'rewritten'")
            .execute(&pool)
            .await;
        assert!(
            update.is_err(),
            "activity journal rows must not be rewritable"
        );

        let delete = sqlx::query("DELETE FROM activity_journal WHERE id = ?")
            .bind("fresh")
            .execute(&pool)
            .await;
        assert!(
            delete.is_err(),
            "activity journal rows inside retention must not be deletable"
        );

        let items = page(&pool, None, 10, None, None).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "entry fresh");
    }

    #[tokio::test]
    async fn prune_removes_only_old_rows() {
        let pool = test_pool().await;
        let old_ts = (Utc::now() - Duration::days(RETENTION_DAYS + 5))
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        let fresh_ts = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        insert_entry(&pool, &entry("old", &old_ts)).await.unwrap();
        insert_entry(&pool, &entry("fresh", &fresh_ts))
            .await
            .unwrap();

        let deleted = prune_older_than_days(&pool, RETENTION_DAYS).await.unwrap();
        assert_eq!(deleted, 1);
        let items = page(&pool, None, 10, None, None).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "fresh");
    }

    #[test]
    fn maps_selected_kinds_and_skips_noise() {
        // Selected: goal transition.
        let e = events::goal_state_changed(
            "goal-1",
            Some("proj-1"),
            Some("ready"),
            "in_progress",
            "codex",
        );
        let entry = entry_from_event(&e).expect("goal_state_changed is journaled");
        assert_eq!(entry.kind, "goal_state_changed");
        assert_eq!(entry.ref_kind.as_deref(), Some("goal"));
        assert_eq!(entry.ref_id.as_deref(), Some("goal-1"));
        assert_eq!(entry.detail.as_deref(), Some("Goal Ready → In Progress"));
        assert_eq!(entry.id, e.id, "row id is the event id");

        // Selected: decisions, librarian, task failure.
        assert!(entry_from_event(&events::decision_created("d-1", "unblock", 1)).is_some());
        let resolved = entry_from_event(&events::decision_resolved(
            "d-1", "unblock", "approve", "jesse", 1,
        ))
        .unwrap();
        assert_eq!(resolved.actor.as_str(), "jesse");
        assert_eq!(resolved.detail.as_deref(), Some("Answered: approve"));
        assert!(entry_from_event(&events::task_failed("t-1", "boom")).is_some());

        // Skipped noise.
        assert!(entry_from_event(&events::stream_chunk("s", "tok", false)).is_none());
        assert!(entry_from_event(&events::memory_recalled("q", 3, "brain")).is_none());
        assert!(entry_from_event(&PermagentEvent::new(
            crate::events::PermagentEventType::BrowserNavigateRequested,
            serde_json::json!({ "url": "https://example.com" }),
        ))
        .is_none());
    }

    #[test]
    fn maps_malformed_payload_defensively() {
        // A goal event with an empty payload still journals with fallbacks.
        let e = PermagentEvent::new(PermagentEventType::GoalStateChanged, serde_json::json!({}));
        let entry = entry_from_event(&e).expect("malformed payload degrades, not drops");
        assert_eq!(entry.title, "Goal");
        assert!(entry.ref_id.is_none());
        let f = PermagentEvent::new(PermagentEventType::TaskFailed, serde_json::json!(null));
        assert!(entry_from_event(&f).is_some());
    }

    /// **The anti-rot guard for GAP-B.** `actor` is a JOIN key, not a label:
    /// `GET /api/agents/{id}/work` filters the journal by exact actor. Free
    /// text is how it rotted the first time, so this asserts the property
    /// directly — every journaled kind, and a bogus producer value.
    #[tokio::test]
    async fn every_journaled_kind_resolves_to_a_joinable_actor() {
        let events = [
            events::goal_state_changed("g", None, None, "ready", "claude-code"),
            events::decision_created("d", "unblock", 1),
            events::decision_resolved("d", "unblock", "approve", "henry-policy", 1),
            events::librarian_describe_completed(
                "m",
                "description",
                10,
                crate::agents::platform_extensions::librarian::DescriptionQuality::Structured,
            ),
            events::task_failed("t", "boom"),
            events::proactive_nudge("project_news", "s", "m", 1, "now", None, None),
            events::a2a_message("g1", "g2", "deadbeef", 12, true, "claude-code"),
        ];
        let allowed: std::collections::HashSet<String> =
            ["system", "jesse", "henry", "henry-policy"]
                .into_iter()
                .map(str::to_string)
                .chain(
                    crate::agents::self_knowledge::WORKER_DESCRIPTORS
                        .iter()
                        .map(|descriptor| descriptor.id.to_string()),
                )
                .chain(crate::config::agent_identity::default_roster().into_keys())
                .collect();
        let entries: Vec<NewEntry> = events
            .iter()
            .map(|event| entry_from_event(event).unwrap())
            .collect();
        // Fails when an eighth kind is journaled without being covered here.
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.kind.as_str())
                .collect::<std::collections::HashSet<_>>(),
            KNOWN_KINDS.into_iter().collect(),
            "the guard must represent every journaled event kind"
        );
        for entry in &entries {
            assert!(
                allowed.contains(entry.actor.as_str()),
                "kind '{}' journals actor '{}', which is neither a roster agent id \
                 nor one of system/jesse/henry/henry-policy — no work review can \
                 reach that row. Resolve it through Actor::resolve.",
                entry.kind,
                entry.actor.as_str()
            );
        }

        // A producer inventing an actor must NOT be able to write it.
        let bogus =
            events::goal_state_changed("bogus", None, None, "ready", "totally-made-up-worker");
        assert_eq!(entry_from_event(&bogus).unwrap().actor, Actor::System);

        // And the whole point: a real worker's row is reachable by its id.
        let pool = test_pool().await;
        let worker = events::goal_state_changed("worker-goal", None, None, "ready", "claude_code");
        record_event(&pool, &worker).await.unwrap();
        let page = page(&pool, None, 10, None, Some("claude_code"))
            .await
            .unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].actor, "claude_code");
    }

    #[tokio::test]
    async fn record_event_enriches_goal_title_and_page_resolves_project() {
        let pool = test_pool().await;
        let card = crate::cards::create_card(
            &pool,
            crate::cards::CreateCard {
                project_id: crate::projects::PERSONAL_PROJECT_ID.to_string(),
                title: "Ship the journal".to_string(),
                description: None,
                card_type: None,
                column_id: None,
                created_by: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();

        let e = events::goal_state_changed(
            &card.id,
            Some(crate::projects::PERSONAL_PROJECT_ID),
            None,
            "triage",
            // `create_card` hardcodes this literal for every caller, so it
            // must NOT resolve to a person — see `Actor::resolve`.
            "user",
        );
        assert_eq!(
            entry_from_event(&e).unwrap().actor,
            Actor::System,
            "card authorship 'user' must not be laundered into the human actor"
        );
        assert!(record_event(&pool, &e).await.unwrap());

        let items = page(&pool, None, 10, None, None).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Ship the journal");
        assert_eq!(
            items[0].goal_project_id.as_deref(),
            Some(crate::projects::PERSONAL_PROJECT_ID),
            "goal rows resolve project_id live for deep-linking"
        );
        assert_eq!(items[0].detail.as_deref(), Some("Goal created (Triage)"));

        // Skipped kinds report false and write nothing.
        let skipped = events::stream_chunk("s", "tok", true);
        assert!(!record_event(&pool, &skipped).await.unwrap());
        assert_eq!(page(&pool, None, 10, None, None).await.unwrap().len(), 1);
    }

    #[test]
    fn maps_proactive_nudges() {
        // Dormant-thread nudge: watcher actor + memory evidence pointer.
        let e = events::proactive_nudge(
            "dormant_thread",
            "Solar shed project",
            "You haven't touched the solar shed thread in three weeks.",
            4,
            "2026-06-19T10:00:00.000Z",
            None,
            None,
        );
        let entry = entry_from_event(&e).expect("proactive_nudge is journaled");
        assert_eq!(entry.kind, "proactive_nudge");
        assert_eq!(entry.actor.as_str(), "watcher");
        assert_eq!(entry.title, "Resurfaced 'Solar shed project'");
        assert_eq!(entry.ref_kind.as_deref(), Some("memory"));
        assert_eq!(entry.ref_id.as_deref(), Some("Solar shed project"));
        assert!(entry.detail.as_deref().unwrap().contains("three weeks"));

        // News nudge: the article URL is its evidence pointer.
        let news = entry_from_event(&events::proactive_nudge(
            "project_news",
            "kuzu 0.11",
            "kuzu 0.11 shipped — relevant to the Brain.",
            1,
            "2026-07-10T10:00:00.000Z",
            Some("https://example.com/kuzu-0-11"),
            Some(("proj-brain", "Brain")),
        ))
        .unwrap();
        assert_eq!(news.title, "News: kuzu 0.11");
        assert_eq!(news.ref_kind.as_deref(), Some("url"));
        assert_eq!(
            news.ref_id.as_deref(),
            Some("https://example.com/kuzu-0-11")
        );

        let project_news = entry_from_event(&events::proactive_nudge(
            "project_news",
            "Brain",
            "Something relevant to the Brain.",
            1,
            "2026-07-10T10:00:00.000Z",
            None,
            Some(("proj-brain", "Brain")),
        ))
        .unwrap();
        assert_eq!(project_news.ref_kind.as_deref(), Some("project"));
        assert_eq!(project_news.ref_id.as_deref(), Some("proj-brain"));

        let rsi = entry_from_event(&events::proactive_nudge(
            "rsi_heat",
            "SHOP",
            "RSI 78 on SHOP — above your 74 threshold",
            1,
            "2026-08-21T10:00:00.000Z",
            None,
            None,
        ))
        .unwrap();
        assert_eq!(rsi.title, "RSI heat: SHOP");
        assert_eq!(rsi.actor.as_str(), "financier");
        assert_eq!(rsi.ref_kind.as_deref(), Some("symbol"));
        assert_eq!(rsi.ref_id.as_deref(), Some("SHOP"));
        assert!(rsi.detail.as_deref().unwrap().contains("above your 74"));

        let sell = entry_from_event(&events::proactive_nudge(
            "sell_signal",
            "SHOP",
            "Sell signal on SHOP — RSI 78 — above your 74 threshold. A signal, not an order.",
            1,
            "2026-08-21T10:00:00.000Z",
            None,
            None,
        ))
        .unwrap();
        assert_eq!(sell.title, "Sell signal: SHOP");
        assert_eq!(sell.actor.as_str(), "financier");
        assert_eq!(sell.ref_kind.as_deref(), Some("symbol"));
        assert!(sell.detail.as_deref().unwrap().contains("not an order"));

        let pick = entry_from_event(&events::proactive_nudge(
            "daily_pick",
            "SHOP",
            "SHOP — loop gate held and the window is tomorrow.",
            1,
            "2026-08-24T19:40:00.000Z",
            None,
            None,
        ))
        .unwrap();
        assert_eq!(pick.title, "Tomorrow: SHOP");
        assert_eq!(pick.actor.as_str(), "financier");
        assert_eq!(pick.ref_kind.as_deref(), Some("symbol"));
        assert_eq!(pick.ref_id.as_deref(), Some("SHOP"));

        let none = entry_from_event(&events::proactive_nudge(
            "daily_pick",
            "none",
            "The scanner finished and no name cleared the loop gate.",
            1,
            "2026-08-24T19:40:00.000Z",
            None,
            None,
        ))
        .unwrap();
        assert_eq!(none.title, "No pick tomorrow");
        assert_eq!(none.actor.as_str(), "financier");
        assert!(none.ref_kind.is_none());
    }

    #[tokio::test]
    async fn page_filters_by_kind_and_actor() {
        let pool = test_pool().await;
        let rows = [
            (
                "g1",
                "2026-07-10T10:00:00.000Z",
                "goal_state_changed",
                "system",
            ),
            (
                "d1",
                "2026-07-10T11:00:00.000Z",
                "decision_created",
                "henry",
            ),
            (
                "d2",
                "2026-07-10T12:00:00.000Z",
                "decision_resolved",
                "jesse",
            ),
            (
                "n1",
                "2026-07-10T13:00:00.000Z",
                "proactive_nudge",
                "watcher",
            ),
        ];
        for (id, ts, kind, actor) in rows {
            let mut e = entry(id, ts);
            e.kind = kind.to_string();
            e.actor = Actor::resolve(actor);
            insert_entry(&pool, &e).await.unwrap();
        }

        // Kind filter: multi-kind IN (the Decisions chip).
        let kinds: &[String] = &[
            "decision_created".to_string(),
            "decision_resolved".to_string(),
        ];
        let items = page(&pool, None, 10, Some(kinds), None).await.unwrap();
        assert_eq!(
            items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            vec!["d2", "d1"]
        );

        // Actor filter composes with the cursor.
        let items = page(&pool, None, 10, None, Some("watcher")).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "n1");
        let items = page(
            &pool,
            Some("2026-07-10T13:00:00.000Z"),
            10,
            None,
            Some("watcher"),
        )
        .await
        .unwrap();
        assert!(items.is_empty(), "cursor excludes the watcher's only row");

        // Unknown kinds match nothing (allowlist, not error).
        let bogus: &[String] = &["'; DROP TABLE activity_journal; --".to_string()];
        assert!(page(&pool, None, 10, Some(bogus), None)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            page(&pool, None, 10, None, None).await.unwrap().len(),
            4,
            "table intact after hostile kind filter"
        );
    }
}
