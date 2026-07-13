//! Durable activity journal (#619) — one reviewable "what did my agents do"
//! surface.
//!
//! An append-only `activity_journal` table (spectral schema v26) fed by the
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
            "A day-grouped timeline on the Home dashboard reading the durable activity journal — an append-only record of what you and your workers actually did: goal moves, decisions requested and resolved, librarian describe runs, Watcher nudges, and task failures, kept for 90 days and filterable by kind or actor",
        why_it_matters:
            "It is the user's reviewable answer to 'what did my agents do today and why' — every row carries an evidence pointer (the goal card, the decision, the memory), so when the user asks what happened, point them here instead of reconstructing history from your context window",
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
pub const KNOWN_KINDS: [&str; 6] = [
    "goal_state_changed",
    "decision_created",
    "decision_resolved",
    "librarian_describe_completed",
    "proactive_nudge",
    "task_failed",
];

/// A journal row ready to insert (write side).
#[derive(Debug, Clone)]
pub struct NewEntry {
    pub id: String,
    pub ts: String,
    pub kind: String,
    pub actor: String,
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
            // Actor honesty: the event payload carries no actor (transitions
            // can be agent- or user-driven), so the journal says "system"
            // rather than guessing. Follow-up: thread the audit actor through
            // the event.
            let detail = match (from, to) {
                (_, "deleted") => "Goal deleted".to_string(),
                (None, to) => format!("Goal created ({})", state_label(to)),
                (Some(f), t) => format!("Goal {} → {}", state_label(f), state_label(t)),
            };
            (
                "goal_state_changed",
                "system".to_string(),
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
                "henry".to_string(),
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
                acted_by.to_string(),
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
                "librarian".to_string(),
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
                "system".to_string(),
                "Task failed".to_string(),
                Some(error.to_string()),
                Some("task"),
                task_id.map(str::to_string),
            )
        }
        PermagentEventType::ProactiveNudge => {
            // The Watcher's initiative (#672): it chose to surface something.
            // Dormant Brain threads carry a memory pointer; news nudges have
            // no durable referent (the link lives in the notification).
            let nudge_kind = payload_str(event, "kind").unwrap_or("nudge");
            let subject = payload_str(event, "subject").unwrap_or("something");
            let message = payload_str(event, "message");
            let title = match nudge_kind {
                "dormant_thread" => format!("Resurfaced '{}'", truncate(subject, 80)),
                "project_news" => format!("News: {}", truncate(subject, 80)),
                _ => format!("Nudge: {}", truncate(subject, 80)),
            };
            let (ref_kind, ref_id) = if nudge_kind == "dormant_thread" {
                (Some("memory"), Some(subject.to_string()))
            } else {
                (None, None)
            };
            (
                "proactive_nudge",
                "watcher".to_string(),
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
        actor: truncate(&actor, 40),
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
    .bind(&entry.actor)
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
    let rows = sqlx::query(&sql)
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
            actor: "system".to_string(),
            title: format!("entry {id}"),
            detail: None,
            ref_kind: None,
            ref_id: None,
        }
    }

    #[tokio::test]
    async fn migrate_v25_to_v26_idempotent() {
        let pool = test_pool().await; // fresh init already created the table
        crate::session::spectral_schema::migrate_v25_to_v26(&pool)
            .await
            .unwrap();
        crate::session::spectral_schema::migrate_v25_to_v26(&pool)
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
        let e = events::goal_state_changed("goal-1", Some("proj-1"), Some("ready"), "in_progress");
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
        assert_eq!(resolved.actor, "jesse");
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
        );
        let entry = entry_from_event(&e).expect("proactive_nudge is journaled");
        assert_eq!(entry.kind, "proactive_nudge");
        assert_eq!(entry.actor, "watcher");
        assert_eq!(entry.title, "Resurfaced 'Solar shed project'");
        assert_eq!(entry.ref_kind.as_deref(), Some("memory"));
        assert_eq!(entry.ref_id.as_deref(), Some("Solar shed project"));
        assert!(entry.detail.as_deref().unwrap().contains("three weeks"));

        // News nudge: no durable referent, so no evidence pointer.
        let news = entry_from_event(&events::proactive_nudge(
            "project_news",
            "kuzu 0.11",
            "kuzu 0.11 shipped — relevant to the Brain.",
            1,
            "2026-07-10T10:00:00.000Z",
        ))
        .unwrap();
        assert_eq!(news.title, "News: kuzu 0.11");
        assert!(news.ref_kind.is_none());
        assert!(news.ref_id.is_none());
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
            e.actor = actor.to_string();
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
