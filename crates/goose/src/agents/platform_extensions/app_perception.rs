//! App Perception — Henry's read-only sense of the Permagent app.
//!
//! This is deliberately separate from `app_conductor`: perception reads the
//! local data that app surfaces render; it never emits UI events or writes.
//! Results are aggregate answers, never record corpora, because tool output
//! persists in the model conversation.

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use crate::app_views::{self, AnalyticsWindow};
use crate::{activity_journal, briefings, cards, decisions, projects, scheduler, skills};
use anyhow::Result as AnyResult;
use async_trait::async_trait;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Pool, Sqlite};
use std::collections::BTreeMap;
use std::path::Path;
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "app_perception";

/// Every surface `observe_app` can read.
///
/// A list rather than a bare `match` because coverage now has to be checkable.
/// The ruling is that everything in the app is queryable by the agent, and the
/// only way that stays true is if a new surface CANNOT ship without either
/// gaining an aspect here or being explicitly exempted — see
/// `every_shipped_tab_is_observable_or_exempt` in the daemon crate, which reads
/// this list against the shipped app catalog.
///
/// The alternative is what produced the bug this list was added for: the agent
/// could describe the Grow tab from an editorial descriptor while being
/// structurally unable to read a single row in it, and nothing anywhere failed.
pub const OBSERVABLE_SURFACES: &[&str] = &[
    "analytics",
    "projects",
    "goals",
    "cards",
    "spend",
    "sessions",
    "briefings",
    "grow",
    "brain",
    "build",
    "world",
    "settings",
    "overview",
    "inbox",
    "skills",
    "automate",
    "trace",
];
const LIST_LIMIT: usize = 5;

/// A separate descriptor is intentional: the platform-extension descriptor
/// tells Henry a tool exists; this surface descriptor teaches the deeper
/// self-model that app data is his native environment, not a browser page.
pub const SELF_KNOWLEDGE_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "app_awareness",
        display_name: "App awareness",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "You can directly perceive the aggregate data your Permagent home renders by \
             calling observe_app for analytics, projects, goals, cards, spend, sessions, \
             briefings, grow, inbox, skills, automate, trace, brain, build, world, settings, \
             or an overview. This is structured local state, not screenshot vision and not the \
             website in the Build browser. \
             `grow` returns the growth actions YOU recommended for a project, what you predicted \
             each would move, and how the 7/14/28-day sweep judged it — you have no memory of \
             those recommendations, so read them rather than saying you do not know. `brain` is \
             memory counts, recall health, and librarian schedule/phase — never memory contents. \
             `build` is coding-session summaries and the in-app browser bookmarks/tab sets. \
             `world` is the worker roster and availability. `settings` is which providers and \
             extensions are configured/enabled — never secret values, only whether a key is present",
        why_it_matters:
            "Treat the app as your home: answer questions about what is happening here from \
             observe_app without navigating, taking screenshots, or calling browser page tools. \
             The result is deliberately bounded to summaries and small ranked lists so private \
             database rows do not become conversation history",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[],
    };

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ObserveAppParams {
    /// Room of the app to observe: analytics, projects, goals, cards, spend,
    /// sessions, briefings, grow, inbox, skills, automate, trace, brain,
    /// build, world, settings, or overview.
    surface: String,
    /// Narrow scope. Required for analytics/cards; optional project name, slug,
    /// or id for goals. Never returned as a raw join id.
    #[serde(default)]
    scope: Option<String>,
    /// Time window where supported: 7d, 30d, 90d, 365d, or all. Analytics
    /// defaults to 30d; sessions defaults to 7d.
    #[serde(default)]
    window: Option<String>,
}

fn schema<T: JsonSchema>() -> JsonObject {
    let mut obj = serde_json::to_value(schema_for!(T))
        .map(|v| v.as_object().unwrap().clone())
        .expect("valid schema");
    obj.entry("properties")
        .or_insert_with(|| serde_json::json!({}));
    obj
}

fn safe_text(value: &str, max_chars: usize) -> String {
    crate::privacy::redact(value)
        .chars()
        .take(max_chars)
        .collect()
}

/// Defense in depth: every string gets the shared privacy pass immediately
/// before it leaves the tool. Specific views also avoid constructing raw ids,
/// paths, email addresses, and detail bodies in the first place.
fn redact_json(value: &mut Value) {
    match value {
        Value::String(s) => *s = crate::privacy::redact(s),
        Value::Array(items) => items.iter_mut().for_each(redact_json),
        Value::Object(map) => map.values_mut().for_each(redact_json),
        _ => {}
    }
}

fn available(surface: &str, data: Value) -> Value {
    json!({
        "surface": surface,
        "status": "available",
        "queried": true,
        "data": data
    })
}

fn empty(surface: &str, reason: &str, context: Value) -> Value {
    json!({
        "surface": surface,
        "status": "empty",
        "queried": true,
        "reason": reason,
        "data": context
    })
}

fn unavailable(surface: &str, reason: impl AsRef<str>) -> Value {
    json!({
        "surface": surface,
        "status": "unavailable",
        "queried": false,
        "reason": safe_text(reason.as_ref(), 300)
    })
}

fn not_wired(surface: &str, reason: impl AsRef<str>, context: Value) -> Value {
    json!({
        "surface": surface,
        "status": "not_wired",
        "queried": false,
        "reason": safe_text(reason.as_ref(), 300),
        "data": context
    })
}

fn ranked(items: Vec<Value>, total: usize) -> Value {
    json!({
        "items": items,
        "returned": total.min(LIST_LIMIT),
        "total": total,
        "limit": LIST_LIMIT,
        "truncated": total > LIST_LIMIT
    })
}

fn brain_db_path() -> std::path::PathBuf {
    crate::config::paths::Paths::brain_dir().join("memory.db")
}

fn open_brain_db_readonly() -> Result<rusqlite::Connection, String> {
    let db_path = brain_db_path();
    if !db_path.exists() {
        return Err("Brain memory database is not available on disk".to_string());
    }
    rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("could not open Brain memory database: {e}"))
}

fn table_has_column(conn: &rusqlite::Connection, table: &str, column: &str) -> bool {
    let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info({table})")) else {
        return false;
    };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(1)) else {
        return false;
    };
    let found = rows.flatten().any(|name| name == column);
    found
}

/// Aggregate memory counts only — never row contents. Missing DB is an error;
/// a present empty table is a successful zero.
fn query_memory_counts(conn: &rusqlite::Connection) -> Result<(usize, usize), String> {
    let total: usize = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
        .map_err(|e| format!("memory count query failed: {e}"))?;
    let described = if table_has_column(conn, "memories", "description") {
        conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE description IS NOT NULL AND description != ''",
            [],
            |r| r.get(0),
        )
        .map_err(|e| format!("described memory count query failed: {e}"))?
    } else {
        0
    };
    Ok((total, described))
}

fn load_librarian_schedule_summary() -> Value {
    let path = crate::config::paths::Paths::in_data_dir("librarian_schedule.json");
    match std::fs::read_to_string(&path) {
        Ok(contents) => match serde_json::from_str::<Value>(&contents) {
            Ok(schedule) => {
                let enabled = schedule
                    .get("enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let start = schedule
                    .get("start_time")
                    .and_then(|v| v.as_str())
                    .unwrap_or("02:00");
                let duration = schedule
                    .get("duration_minutes")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(240);
                let model = schedule
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("qwen2.5:7b");
                json!({
                    "status": "available",
                    "enabled": enabled,
                    "start_time": safe_text(start, 16),
                    "duration_minutes": duration,
                    "model": safe_text(model, 64)
                })
            }
            Err(e) => json!({
                "status": "unavailable",
                "reason": safe_text(&format!("librarian schedule file is malformed: {e}"), 200)
            }),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({
            "status": "available",
            "enabled": true,
            "start_time": "02:00",
            "duration_minutes": 240,
            "model": "qwen2.5:7b",
            "note": "no schedule file yet; using Librarian defaults"
        }),
        Err(e) => json!({
            "status": "unavailable",
            "reason": safe_text(&format!("could not read librarian schedule: {e}"), 200)
        }),
    }
}

async fn query_recall_health(pool: &Pool<Sqlite>) -> Value {
    // recognition_events lives in the session DB (instrumentation), not memory.db.
    // A missing table is "could not read" — not the same as zero recalls.
    let table_ok: Result<bool, sqlx::Error> = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'recognition_events'",
    )
    .fetch_one(pool)
    .await;
    match table_ok {
        Ok(false) => {
            return json!({
                "status": "unavailable",
                "reason": "recognition_events table is not present in the session database"
            })
        }
        Err(e) => {
            return json!({
                "status": "unavailable",
                "reason": safe_text(&format!("could not check recognition_events: {e}"), 200)
            })
        }
        Ok(true) => {}
    }

    let cutoff = (chrono::Utc::now() - chrono::Duration::days(7))
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    let events_7d: Result<i64, sqlx::Error> =
        sqlx::query_scalar("SELECT COUNT(*) FROM recognition_events WHERE retrieved_at >= ?")
            .bind(&cutoff)
            .fetch_one(pool)
            .await;
    let with_outcome: Result<i64, sqlx::Error> = sqlx::query_scalar(
        "SELECT COUNT(*) FROM recognition_events
          WHERE retrieved_at >= ?
            AND outcome_label IS NOT NULL",
    )
    .bind(&cutoff)
    .fetch_one(pool)
    .await;
    match (events_7d, with_outcome) {
        (Ok(events), Ok(labeled)) => {
            if events == 0 {
                json!({
                    "status": "empty",
                    "window": "7d",
                    "events": 0,
                    "with_outcome_label": 0,
                    "reason": "query succeeded; no recall events in this window"
                })
            } else {
                json!({
                    "status": "available",
                    "window": "7d",
                    "events": events,
                    "with_outcome_label": labeled
                })
            }
        }
        (Err(e), _) | (_, Err(e)) => json!({
            "status": "unavailable",
            "reason": safe_text(&format!("recall health query failed: {e}"), 200)
        }),
    }
}

fn read_json_file_or_default<T: Default + serde::de::DeserializeOwned>(
    path: &std::path::Path,
) -> Result<T, String> {
    match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents)
            .map_err(|e| format!("could not parse {}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(format!("could not read {}: {e}", path.display())),
    }
}

fn config_key_present(name: &str, secret: bool) -> bool {
    let config = crate::config::Config::global();
    if std::env::var(name).is_ok() {
        return true;
    }
    if secret {
        config.get_secret::<serde_json::Value>(name).is_ok()
    } else {
        config.get_param::<serde_json::Value>(name).is_ok() || config.get(name, false).is_ok()
    }
}

fn analytics_ranked(counts: &app_views::RankedCounts, name_key: &str, count_key: &str) -> Value {
    let items: Vec<Value> = counts
        .items
        .iter()
        .map(|item| {
            let mut obj = serde_json::Map::new();
            obj.insert(
                name_key.to_string(),
                Value::String(safe_text(&item.name, 256)),
            );
            obj.insert(count_key.to_string(), Value::from(item.count));
            Value::Object(obj)
        })
        .collect();
    json!({
        "items": items,
        "returned": counts.items.len(),
        "total": counts.total,
        "limit": counts.limit,
        "truncated": counts.truncated
    })
}

fn parse_window(raw: Option<&str>, default_days: u32) -> Result<(AnalyticsWindow, String), String> {
    match raw.unwrap_or_default().trim().to_ascii_lowercase().as_str() {
        "" => Ok((
            AnalyticsWindow::days(default_days),
            format!("{default_days}d"),
        )),
        "7d" => Ok((AnalyticsWindow::days(7), "7d".to_string())),
        "30d" => Ok((AnalyticsWindow::days(30), "30d".to_string())),
        "90d" => Ok((AnalyticsWindow::days(90), "90d".to_string())),
        "365d" => Ok((AnalyticsWindow::days(365), "365d".to_string())),
        "all" | "all_time" | "all-time" => Ok((AnalyticsWindow::AllTime, "all".to_string())),
        other => Err(format!(
            "Unsupported window \"{other}\". Use 7d, 30d, 90d, 365d, or all."
        )),
    }
}

async fn resolve_project(
    pool: &Pool<Sqlite>,
    scope: &str,
) -> std::result::Result<projects::Project, String> {
    if let Some(project) = projects::get_project_by_id_or_slug(pool, scope).await? {
        return Ok(project);
    }
    let matches: Vec<projects::Project> = projects::list_projects(pool, None)
        .await?
        .into_iter()
        .filter(|p| p.name.eq_ignore_ascii_case(scope))
        .collect();
    match matches.as_slice() {
        [project] => Ok(project.clone()),
        [] => Err(format!(
            "No project named \"{}\" exists.",
            safe_text(scope, 120)
        )),
        _ => Err(format!(
            "More than one project is named \"{}\"; use its slug.",
            safe_text(scope, 120)
        )),
    }
}

pub struct AppPerceptionClient {
    info: InitializeResult,
    context: PlatformExtensionContext,
}

impl AppPerceptionClient {
    pub fn new(context: PlatformExtensionContext) -> AnyResult<Self> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME, "1.0.0").with_title("App Perception"),
            )
            .with_instructions(
                "Read the structured data behind the Permagent app. Use observe_app directly \
                 when the user asks what is happening in analytics, projects, goals/cards, \
                 spend, sessions, growth actions you have recommended, agent briefings, the \
                 Decision Inbox, skills, scheduled automations, execution trace, brain memory \
                 health, build coding sessions or browser bookmarks, world workers, settings \
                 configuration, or the overall home. Do not navigate first \
                 and do not use browser snapshots: browser tools see websites, not this app. \
                 This extension is read-only and returns aggregate answers with bounded lists.",
            );
        Ok(Self { info, context })
    }

    /// The growth actions THIS AGENT recommended, and how they are doing.
    ///
    /// Added 2026-08-14 after the user asked their agent about advice it had
    /// given on the Grow tab and got nothing. That was not a memory failure:
    /// `growth_actions` had four real rows and no tool could read them. The
    /// agent could describe the Grow tab from a static self-knowledge
    /// descriptor while being structurally unable to see a single thing in it.
    ///
    /// An agent that recommends a strategy and then cannot recall it can neither
    /// follow up nor learn, which is the whole point of the verify → measure →
    /// learn loop. Predictions are surfaced with the actions so the agent can
    /// see what it CLAIMED as well as what it advised.
    async fn observe_grow(&self, pool: &Pool<Sqlite>, scope: Option<&str>) -> Value {
        let Some(scope) = scope.filter(|s| !s.trim().is_empty()) else {
            return unavailable("grow", "grow requires a project name, slug, or id");
        };
        let project = match resolve_project(pool, scope).await {
            Ok(p) => p,
            Err(e) => return unavailable("grow", e),
        };

        let rows = match crate::growth::store::list_for_project(pool, &project.id).await {
            Ok(r) => r,
            Err(e) => return unavailable("grow", format!("could not read growth actions: {e}")),
        };

        let mut by_status: BTreeMap<String, i64> = BTreeMap::new();
        for row in &rows {
            *by_status.entry(row.status.clone()).or_insert(0) += 1;
        }

        // Bounded like every other aspect: tool output persists in the model
        // conversation, so a full corpus would be paid for on every later turn.
        let mut recent = Vec::new();
        for row in rows.iter().take(LIST_LIMIT) {
            let outcomes = crate::growth::store::outcomes_for(pool, &row.id)
                .await
                .unwrap_or_default();
            recent.push(json!({
                "title": safe_text(&row.title, 120),
                "status": row.status,
                // What the agent predicted. Null means it made no claim, which
                // is different from "no result yet" and must not read the same.
                "predicted": row.target_metric.as_ref().map(|m| json!({
                    "metric": m,
                    "direction": row.target_dir.clone(),
                })),
                "verifiedBy": row.verified_by,
                "verifiedAt": row.verified_at,
                "judged": outcomes.iter().map(|o| json!({
                    "windowDays": o.window_days,
                    "verdict": o.verdict,
                    "deltaPct": o.delta_pct,
                })).collect::<Vec<_>>(),
            }));
        }

        json!({
            "surface": "grow",
            "status": "ok",
            "queried": true,
            "project": project.name,
            "totalActions": rows.len(),
            "byStatus": by_status,
            "recent": recent,
            "truncated": rows.len() > LIST_LIMIT,
        })
    }

    async fn observe_analytics(
        &self,
        pool: &Pool<Sqlite>,
        scope: Option<&str>,
        window: Option<&str>,
    ) -> Value {
        let Some(scope) = scope.filter(|s| !s.trim().is_empty()) else {
            return unavailable(
                "analytics",
                "analytics requires a project name, slug, or id",
            );
        };
        let project = match resolve_project(pool, scope).await {
            Ok(p) => p,
            Err(e) => return unavailable("analytics", e),
        };
        let (window, window_label) = match parse_window(window, 30) {
            Ok(v) => v,
            Err(e) => return unavailable("analytics", e),
        };
        let summary = match app_views::analytics_summary(
            pool,
            &project.id,
            window,
            false,
            LIST_LIMIT,
        )
        .await
        {
            Ok(s) => s,
            Err(e) => return unavailable("analytics", format!("analytics query failed: {e}")),
        };
        let enabled = project.metadata_json.get("first_party_analytics").is_some();
        if summary.human_events + summary.bot_events == 0 {
            if project.metadata_json.get("analytics").is_some() {
                return not_wired(
                    "analytics",
                    "this project uses a third-party analytics provider; provider fetches are not wired into app perception yet",
                    json!({
                        "project": safe_text(&project.name, 120),
                        "local_first_party_queried": true,
                        "local_first_party_events": null,
                        "provider_analytics_queried": false
                    }),
                );
            }
            return empty(
                "analytics",
                "query succeeded; no analytics events were measured in this window",
                json!({
                    "project": safe_text(&project.name, 120),
                    "enabled": enabled,
                    "window": window_label,
                    "unique_visitors": null,
                    "pageviews": null,
                    "events": null,
                    "bots": null,
                    "range": null
                }),
            );
        }

        available(
            "analytics",
            json!({
                "project": safe_text(&project.name, 120),
                "enabled": enabled,
                "window": window_label,
                "range": [summary.range_start, summary.range_end],
                "unique_visitors": summary.unique_visitors,
                "visitor_measurement": "distinct privacy-preserving device signatures; may undercount people",
                "pageviews": summary.pageviews,
                "events": summary.event_count,
                "bots": summary.bot_events,
                "bot_split": {
                    "human_events": summary.human_events,
                    "bot_events": summary.bot_events,
                    "bots_excluded_from_headline": summary.bot_events
                },
                // The day-by-day series. Without this the agent saw only window
                // totals and could not answer "which day did it dip" — the
                // drilldown gap reported 2026-08-04. Capped so a 365-day window
                // cannot flood the context; the totals above stay authoritative.
                "daily": summary.daily.iter().rev().take(90).rev().map(|d| json!({
                    "date": d.date,
                    "pageviews": d.pageviews,
                    "visitors": d.visitors,
                    "events": d.events
                })).collect::<Vec<_>>(),
                "daily_note": "one row per day WITH traffic, ascending; absent days had none",
                "top_paths": analytics_ranked(&summary.top_paths, "path", "views"),
                "utm": {
                    "sources": analytics_ranked(&summary.top_utm_sources, "name", "events"),
                    "mediums": analytics_ranked(&summary.top_utm_mediums, "name", "events"),
                    "campaigns": analytics_ranked(&summary.top_utm_campaigns, "name", "events")
                }
            }),
        )
    }

    async fn observe_projects(&self, pool: &Pool<Sqlite>) -> Value {
        let projects = match projects::list_projects(pool, None).await {
            Ok(rows) => rows,
            Err(e) => return unavailable("projects", format!("projects query failed: {e}")),
        };
        if projects.is_empty() {
            return empty(
                "projects",
                "query succeeded; no projects exist",
                json!({"projects": ranked(Vec::new(), 0)}),
            );
        }
        let total = projects.len();
        let items = projects
            .into_iter()
            .take(LIST_LIMIT)
            .map(|project| {
                json!({
                    "name": safe_text(&project.name, 120),
                    "status": safe_text(&project.status, 40),
                    "description": safe_text(&project.description, 240),
                    "has_site": project.site_url.is_some(),
                    "has_repository": project.repo_url.is_some()
                })
            })
            .collect();
        available("projects", json!({"projects": ranked(items, total)}))
    }

    async fn observe_board(
        &self,
        pool: &Pool<Sqlite>,
        scope: Option<&str>,
        goals_only: bool,
    ) -> Value {
        let surface = if goals_only { "goals" } else { "cards" };
        let Some(scope) = scope.filter(|s| !s.trim().is_empty()) else {
            if goals_only {
                return self.observe_active_goals(pool).await;
            }
            return unavailable("cards", "cards requires a project name, slug, or id");
        };
        let project = match resolve_project(pool, scope).await {
            Ok(p) => p,
            Err(e) => return unavailable(surface, e),
        };
        let columns = match cards::list_columns(pool, &project.id).await {
            Ok(rows) => rows,
            Err(e) => return unavailable(surface, format!("board columns query failed: {e}")),
        };
        let cards =
            match cards::list_cards(pool, &project.id, goals_only.then_some("goal"), None).await {
                Ok(rows) => rows,
                Err(e) => return unavailable(surface, format!("cards query failed: {e}")),
            };
        let column_names: BTreeMap<String, String> = columns
            .iter()
            .map(|column| (column.id.clone(), column.name.clone()))
            .collect();
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for card in &cards {
            let name = column_names
                .get(&card.column_id)
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());
            *counts.entry(name).or_default() += 1;
        }
        let columns_total = counts.len();
        let column_items = counts
            .iter()
            .take(LIST_LIMIT)
            .map(|(name, count)| {
                json!({
                    "name": safe_text(name, 80),
                    "cards": count
                })
            })
            .collect();
        let total = cards.len();
        if total == 0 {
            return empty(
                surface,
                "query succeeded; this board has no matching cards",
                json!({
                    "project": safe_text(&project.name, 120),
                    "columns": ranked(column_items, columns_total),
                    "cards": ranked(Vec::new(), 0)
                }),
            );
        }
        let items = cards
            .into_iter()
            .take(LIST_LIMIT)
            .map(|card| {
                json!({
                    "title": safe_text(&card.title, 180),
                    "type": card.card_type,
                    "column": column_names.get(&card.column_id)
                        .map(|s| safe_text(s, 80)).unwrap_or_else(|| "unknown".to_string()),
                    "assigned_to": card.assigned_to.map(|s| safe_text(&s, 80)),
                    "needs_human_attention": card.metadata_json
                        .get("needs_human_attention").and_then(Value::as_bool).unwrap_or(false)
                })
            })
            .collect();
        available(
            surface,
            json!({
                "project": safe_text(&project.name, 120),
                "columns": ranked(column_items, columns_total),
                "cards": ranked(items, total)
            }),
        )
    }

    async fn observe_active_goals(&self, pool: &Pool<Sqlite>) -> Value {
        let goals = match cards::list_active_goals(pool).await {
            Ok(rows) => rows,
            Err(e) => return unavailable("goals", format!("active goals query failed: {e}")),
        };
        let project_names: BTreeMap<String, String> =
            match projects::list_projects(pool, None).await {
                Ok(rows) => rows.into_iter().map(|p| (p.id, p.name)).collect(),
                Err(e) => return unavailable("goals", format!("projects query failed: {e}")),
            };
        let total = goals.len();
        if total == 0 {
            return empty(
                "goals",
                "query succeeded; there are no active goals",
                json!({"goals": ranked(Vec::new(), 0)}),
            );
        }
        let items = goals
            .into_iter()
            .take(LIST_LIMIT)
            .map(|goal| {
                json!({
                    "title": safe_text(&goal.title, 180),
                    "project": project_names.get(&goal.project_id)
                        .map(|s| safe_text(s, 120)).unwrap_or_else(|| "unknown".to_string()),
                    "state": safe_text(&goal.state, 40),
                    "assigned_to": goal.assigned_to.map(|s| safe_text(&s, 80)),
                    "updated_at": goal.updated_at
                })
            })
            .collect();
        available("goals", json!({"goals": ranked(items, total)}))
    }

    async fn observe_spend(&self, pool: &Pool<Sqlite>, scope: Option<&str>) -> Value {
        if scope.is_some_and(|s| !s.trim().is_empty()) {
            return not_wired(
                "spend",
                "scoped spend is not wired; omit scope for the Governance ledger rollup",
                json!({"scope_supported": false, "whole_ledger_available": true}),
            );
        }
        let rows = match app_views::read_spend_rows(pool).await {
            Ok(rows) => rows,
            Err(e) => return unavailable("spend", format!("spend query failed: {e}")),
        };
        let project_names = match app_views::read_project_names(pool).await {
            Ok(names) => names,
            Err(e) => return unavailable("spend", format!("project labels query failed: {e}")),
        };
        let snapshot = app_views::build_spend_snapshot(
            rows,
            &project_names,
            &crate::cost_router::budget::load_budget_config(),
            LIST_LIMIT,
        );
        if snapshot.session_count == 0 {
            return empty(
                "spend",
                "query succeeded; no sessions have measured cost or token usage",
                json!({
                    "running_total_usd": null,
                    "total_tokens": null,
                    "session_count": 0,
                    "sessions": ranked(Vec::new(), 0),
                    "projects": ranked(Vec::new(), 0)
                }),
            );
        }
        let project_total = snapshot.projects.len();
        let project_items = snapshot
            .projects
            .into_iter()
            .take(LIST_LIMIT)
            .map(|project| {
                json!({
                    "project": safe_text(&project.label, 120),
                    "cost_usd": project.cost_usd,
                    "tokens": project.tokens,
                    "session_count": project.session_count
                })
            })
            .collect();
        let session_total = snapshot.session_count;
        let session_items = snapshot
            .sessions
            .into_iter()
            .map(|session| {
                json!({
                    "name": safe_text(&session.name, 160),
                    "type": safe_text(&session.session_type, 40),
                    "cost_usd": session.cost_usd,
                    "tokens": session.tokens,
                    "budget_band": session.band
                })
            })
            .collect();
        available(
            "spend",
            json!({
                "running_total_usd": snapshot.running_total_usd,
                "total_tokens": snapshot.total_tokens,
                "session_count": session_total,
                "budget": snapshot.budget,
                "sessions": ranked(session_items, session_total),
                "projects": ranked(project_items, project_total)
            }),
        )
    }

    async fn observe_sessions(&self, window: Option<&str>) -> Value {
        let (window, window_label) = match parse_window(window, 7) {
            Ok(v) => v,
            Err(e) => return unavailable("sessions", e),
        };
        let sessions = match self.context.session_manager.list_session_summaries().await {
            Ok(rows) => rows,
            Err(e) => return unavailable("sessions", format!("sessions query failed: {e}")),
        };
        let cutoff = match window {
            AnalyticsWindow::Days(days) => {
                Some(chrono::Utc::now() - chrono::Duration::days(i64::from(days)))
            }
            AnalyticsWindow::AllTime => None,
        };
        let sessions: Vec<_> = sessions
            .into_iter()
            .filter(|session| cutoff.is_none_or(|cutoff| session.updated_at >= cutoff))
            .collect();
        let total = sessions.len();
        if total == 0 {
            return empty(
                "sessions",
                "query succeeded; no sessions were active in this window",
                json!({"window": window_label, "sessions": ranked(Vec::new(), 0)}),
            );
        }
        let items = sessions
            .into_iter()
            .take(LIST_LIMIT)
            .map(|session| {
                json!({
                    "name": safe_text(&session.name, 160),
                    "type": session.session_type.to_string(),
                    "updated_at": session.updated_at.to_rfc3339(),
                    "message_count": session.message_count
                })
            })
            .collect();
        available(
            "sessions",
            json!({"window": window_label, "sessions": ranked(items, total)}),
        )
    }

    async fn observe_briefings(&self, pool: &Pool<Sqlite>) -> Value {
        let total = match briefings::try_unacknowledged_count(pool).await {
            Ok(total) => total.max(0) as usize,
            Err(e) => {
                return unavailable("briefings", format!("briefings count query failed: {e}"))
            }
        };
        let rows = match briefings::try_unacknowledged(pool, LIST_LIMIT as i64).await {
            Ok(rows) => rows,
            Err(e) => return unavailable("briefings", format!("briefings query failed: {e}")),
        };
        if total == 0 {
            return empty(
                "briefings",
                "query succeeded; there are no unread agent briefings",
                json!({"briefings": ranked(Vec::new(), 0)}),
            );
        }
        let items = rows
            .into_iter()
            .map(|briefing| {
                json!({
                    "from": safe_text(&briefings::display_name_for(&briefing.from_agent), 80),
                    "kind": safe_text(&briefing.kind, 80),
                    "severity": briefing.severity.render(),
                    "summary": safe_text(&briefing.summary, 280),
                    "created_at": briefing.created_at
                })
            })
            .collect();
        available("briefings", json!({"briefings": ranked(items, total)}))
    }

    async fn observe_inbox(&self, pool: &Pool<Sqlite>) -> Value {
        let summary = match decisions::inbox_summary(pool).await {
            Ok(summary) => summary,
            Err(e) => return unavailable("inbox", format!("decision inbox query failed: {e}")),
        };
        let rows = match decisions::list_open_decisions(pool).await {
            Ok(rows) => rows,
            Err(e) => return unavailable("inbox", format!("open decisions query failed: {e}")),
        };
        let total = rows.len();
        let items = rows
            .into_iter()
            .take(LIST_LIMIT)
            .map(|item| {
                json!({
                    "headline": safe_text(&item.decision.headline, 120),
                    "kind": safe_text(&item.decision.kind, 80),
                    "tier": item.decision.tier,
                    "goal": item.goal_title.map(|v| safe_text(&v, 160)),
                    "created_at": safe_text(&item.decision.created_at, 80),
                })
            })
            .collect();
        let data = json!({
            "pending": summary.total_pending,
            "handled": summary.handled_count,
            "oldest_pending_at": summary.oldest_pending_at.map(|v| safe_text(&v, 80)),
            "goals_in_flight": summary.goals_in_flight,
            "goals_needing_attention": summary.goals_needing_attention,
            "decisions": ranked(items, total),
        });
        if total == 0 {
            empty(
                "inbox",
                "query succeeded; there are no open decisions",
                data,
            )
        } else {
            available("inbox", data)
        }
    }

    async fn observe_skills(&self, pool: &Pool<Sqlite>) -> Value {
        let saved = match skills::list_skills(pool).await {
            Ok(rows) => rows,
            Err(e) => return unavailable("skills", format!("saved skills query failed: {e}")),
        };
        let proposals = match skills::list_proposals(pool, 2).await {
            Ok(rows) => rows,
            Err(e) => return unavailable("skills", format!("skill proposals query failed: {e}")),
        };
        let saved_total = saved.len();
        let proposal_total = proposals.len();
        let saved_items = saved
            .into_iter()
            .take(LIST_LIMIT)
            .map(|skill| {
                json!({
                    "name": safe_text(&skill.name, 120),
                    "description": skill.description.map(|v| safe_text(&v, 240)),
                    "tool": skill.tool_used.map(|v| safe_text(&v, 120)),
                    "status": safe_text(&skill.status, 40),
                    "trigger_count": skill.trigger_count,
                    "usage_count": skill.usage_count,
                    "last_triggered_at": skill.last_triggered_at.map(|v| safe_text(&v, 80)),
                })
            })
            .collect();
        let proposal_items = proposals
            .into_iter()
            .take(LIST_LIMIT)
            .map(|proposal| {
                json!({
                    "tool": safe_text(&proposal.tool_used, 120),
                    "description": safe_text(&proposal.latest_description, 240),
                    "occurrence_count": proposal.occurrence_count,
                })
            })
            .collect();
        let data = json!({
            "saved": ranked(saved_items, saved_total),
            "proposed": ranked(proposal_items, proposal_total),
        });
        if saved_total + proposal_total == 0 {
            empty(
                "skills",
                "query succeeded; there are no saved or proposed skills",
                data,
            )
        } else {
            available("skills", data)
        }
    }

    async fn observe_automate(&self) -> Value {
        // Do not call `get_default_scheduler_storage_path`: that helper creates
        // the parent directory, while perception must remain strictly read-only.
        let path = crate::config::paths::Paths::data_dir().join("schedule.json");
        Self::observe_automate_path(&path).await
    }

    async fn observe_automate_path(path: &Path) -> Value {
        let raw = match tokio::fs::read_to_string(path).await {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return empty(
                    "automate",
                    "query succeeded; no scheduler store exists because no jobs have been created",
                    json!({"jobs": ranked(Vec::new(), 0)}),
                );
            }
            Err(e) => {
                return unavailable("automate", format!("could not read scheduler store: {e}"))
            }
        };
        let jobs: Vec<scheduler::ScheduledJob> = match serde_json::from_str(&raw) {
            Ok(jobs) => jobs,
            Err(e) => {
                return unavailable("automate", format!("scheduler store is unreadable: {e}"))
            }
        };
        let total = jobs.len();
        let failing = jobs
            .iter()
            .filter(|job| matches!(job.last_status, Some(scheduler::ScheduleRunStatus::Error)))
            .count();
        let missed = jobs
            .iter()
            .filter(|job| matches!(job.last_status, Some(scheduler::ScheduleRunStatus::Missed)))
            .count();
        let items = jobs
            .into_iter()
            .take(LIST_LIMIT)
            .map(|job| {
                json!({
                    "name": safe_text(&job.id, 120),
                    "cron": safe_text(&job.cron, 120),
                    "at": job.at.map(|v| v.to_rfc3339()),
                    "every_seconds": job.every_seconds,
                    "timezone": job.tz.map(|v| safe_text(&v, 80)),
                    "last_run": job.last_run.map(|v| v.to_rfc3339()),
                    "last_status": job.last_status,
                    "failing": matches!(job.last_status, Some(scheduler::ScheduleRunStatus::Error)),
                    "missed": matches!(job.last_status, Some(scheduler::ScheduleRunStatus::Missed)),
                    "paused": job.paused,
                    "currently_running": job.currently_running,
                })
            })
            .collect();
        let data = json!({"failing": failing, "missed": missed, "jobs": ranked(items, total)});
        if total == 0 {
            empty(
                "automate",
                "query succeeded; there are no scheduled jobs",
                data,
            )
        } else {
            available("automate", data)
        }
    }

    async fn observe_trace(&self, pool: &Pool<Sqlite>) -> Value {
        let rows = match activity_journal::page(pool, None, (LIST_LIMIT + 1) as i64, None, None)
            .await
        {
            Ok(rows) => rows,
            Err(e) => return unavailable("trace", format!("activity journal query failed: {e}")),
        };
        let truncated = rows.len() > LIST_LIMIT;
        let total = if truncated {
            LIST_LIMIT + 1
        } else {
            rows.len()
        };
        let items: Vec<Value> = rows
            .into_iter()
            .take(LIST_LIMIT)
            .map(|item| {
                json!({
                    "timestamp": safe_text(&item.ts, 80),
                    "kind": safe_text(&item.kind, 80),
                    "actor": safe_text(&item.actor, 80),
                    "title": safe_text(&item.title, 160),
                    "has_detail": item.detail.is_some(),
                    "reference_kind": item.ref_kind.map(|v| safe_text(&v, 80)),
                })
            })
            .collect();
        let trace = json!({"items": items, "returned": total.min(LIST_LIMIT), "total": if truncated { Value::Null } else { Value::from(total) }, "limit": LIST_LIMIT, "truncated": truncated});
        if total == 0 {
            empty(
                "trace",
                "query succeeded; the activity journal is empty",
                json!({"activity": trace}),
            )
        } else {
            available("trace", json!({"activity": trace}))
        }
    }

    /// Memory system health — counts, recall instrumentation, Librarian phase.
    ///
    /// Added so the agent can answer "is my brain working / how big is memory /
    /// is the librarian catching up" without browsing the Brain tab or dumping
    /// memory contents into the conversation (tool output persists forever).
    async fn observe_brain(&self, pool: &Pool<Sqlite>) -> Value {
        let mut partial = false;

        let memories = match open_brain_db_readonly() {
            Ok(conn) => match query_memory_counts(&conn) {
                Ok((total, described)) => {
                    if total == 0 {
                        json!({
                            "status": "empty",
                            "total": 0,
                            "described": 0,
                            "pending": 0,
                            "reason": "query succeeded; Brain has no memories yet"
                        })
                    } else {
                        json!({
                            "status": "available",
                            "total": total,
                            "described": described,
                            "pending": total.saturating_sub(described)
                        })
                    }
                }
                Err(e) => {
                    partial = true;
                    json!({
                        "status": "unavailable",
                        "total": null,
                        "described": null,
                        "pending": null,
                        "reason": safe_text(&e, 200)
                    })
                }
            },
            Err(e) => {
                partial = true;
                json!({
                    "status": "unavailable",
                    "total": null,
                    "described": null,
                    "pending": null,
                    "reason": safe_text(&e, 200)
                })
            }
        };

        let recall = query_recall_health(pool).await;
        if recall.get("status").and_then(|s| s.as_str()) == Some("unavailable") {
            partial = true;
        }

        let librarian_rt = super::librarian_state::get_state();
        let phase = match librarian_rt.phase {
            super::librarian_state::LibrarianPhase::Idle => "idle",
            super::librarian_state::LibrarianPhase::Warming => "warming",
            super::librarian_state::LibrarianPhase::Describing => "describing",
            super::librarian_state::LibrarianPhase::BatchComplete => "batch_complete",
            super::librarian_state::LibrarianPhase::Error => "error",
        };
        let schedule = load_librarian_schedule_summary();
        if schedule.get("status").and_then(|s| s.as_str()) == Some("unavailable") {
            partial = true;
        }
        let librarian = json!({
            "phase": phase,
            "current_task": safe_text(&librarian_rt.current_task, 160),
            "entities_awaiting_context": librarian_rt.entities_awaiting_context,
            "lifetime": {
                "total": librarian_rt.lifetime_stats.total_memories,
                "described": librarian_rt.lifetime_stats.described,
                "pending": librarian_rt.lifetime_stats.pending
            },
            "session_described": librarian_rt.session_stats.memories_described_this_session,
            "schedule": schedule
            // Deliberately omit current_memory content_preview — that is a
            // memory dump, and this aspect never dumps memory contents.
        });

        let memories_empty = memories.get("status").and_then(|s| s.as_str()) == Some("empty");
        let status = if partial {
            "partial"
        } else if memories_empty {
            "empty"
        } else {
            "available"
        };

        json!({
            "surface": "brain",
            "status": status,
            "queried": true,
            "data": {
                "memories": memories,
                "recall": recall,
                "librarian": librarian
            }
        })
    }

    /// Coding-session memory + Build-tab browser bookmarks/tab sets.
    ///
    /// The Build tab is where the user codes and browses; without this aspect
    /// the agent could not answer "what have I been building" or "what is
    /// bookmarked in the in-app browser" from local state.
    async fn observe_build(&self) -> Value {
        let mut partial = false;

        let coding_sessions = match open_brain_db_readonly() {
            Ok(conn) => {
                let has_source = table_has_column(&conn, "memories", "source");
                let count_sql = if has_source {
                    "SELECT COUNT(*) FROM memories WHERE source = 'coding-session' OR key LIKE 'coding-session-%'"
                } else {
                    "SELECT COUNT(*) FROM memories WHERE key LIKE 'coding-session-%'"
                };
                let total: Result<usize, _> = conn.query_row(count_sql, [], |r| r.get(0));
                match total {
                    Ok(0) => json!({
                        "status": "empty",
                        "sessions": ranked(Vec::new(), 0),
                        "reason": "query succeeded; no coding-session memories stored yet"
                    }),
                    Ok(total) => {
                        let has_created = table_has_column(&conn, "memories", "created_at");
                        let list_sql = match (has_source, has_created) {
                            (true, true) => {
                                "SELECT key, created_at FROM memories
                                  WHERE source = 'coding-session' OR key LIKE 'coding-session-%'
                                  ORDER BY created_at DESC LIMIT ?1"
                            }
                            (true, false) => {
                                "SELECT key, NULL FROM memories
                                  WHERE source = 'coding-session' OR key LIKE 'coding-session-%'
                                  ORDER BY rowid DESC LIMIT ?1"
                            }
                            (false, true) => {
                                "SELECT key, created_at FROM memories
                                  WHERE key LIKE 'coding-session-%'
                                  ORDER BY created_at DESC LIMIT ?1"
                            }
                            (false, false) => {
                                "SELECT key, NULL FROM memories
                                  WHERE key LIKE 'coding-session-%'
                                  ORDER BY rowid DESC LIMIT ?1"
                            }
                        };
                        let mut items = Vec::new();
                        match conn.prepare(list_sql).and_then(|mut stmt| {
                            let rows = stmt.query_map([LIST_LIMIT as i64], |row| {
                                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                            })?;
                            for row in rows.flatten() {
                                let (key, created_at) = row;
                                // key shape: coding-session-{project}-{stamp}
                                let project = key
                                    .strip_prefix("coding-session-")
                                    .and_then(|rest| rest.rsplit_once('-'))
                                    .map(|(project, _)| project)
                                    .unwrap_or("unknown");
                                items.push(json!({
                                    "project": safe_text(project, 80),
                                    "created_at": created_at.map(|s| safe_text(&s, 40)),
                                    // Keys are opaque ids; do not echo full keys.
                                    "has_summary": true
                                }));
                            }
                            Ok(())
                        }) {
                            Ok(()) => json!({
                                "status": "available",
                                "sessions": ranked(items, total)
                            }),
                            Err(e) => {
                                partial = true;
                                json!({
                                    "status": "unavailable",
                                    "sessions": null,
                                    "reason": safe_text(
                                        &format!("coding-session list query failed: {e}"),
                                        200
                                    )
                                })
                            }
                        }
                    }
                    Err(e) => {
                        partial = true;
                        json!({
                            "status": "unavailable",
                            "sessions": null,
                            "reason": safe_text(
                                &format!("coding-session count query failed: {e}"),
                                200
                            )
                        })
                    }
                }
            }
            Err(e) => {
                partial = true;
                json!({
                    "status": "unavailable",
                    "sessions": null,
                    "reason": safe_text(&e, 200)
                })
            }
        };

        let bookmarks_path = crate::config::paths::Paths::in_data_dir("browser_bookmarks.json");
        let tab_sets_path = crate::config::paths::Paths::in_data_dir("browser_tab_sets.json");

        let browser = match (
            read_json_file_or_default::<Value>(&bookmarks_path),
            read_json_file_or_default::<Value>(&tab_sets_path),
        ) {
            (Ok(bookmarks_file), Ok(tab_sets_file)) => {
                let bookmarks = bookmarks_file
                    .get("bookmarks")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let tab_sets = tab_sets_file
                    .get("tabSets")
                    .or_else(|| tab_sets_file.get("tab_sets"))
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let bookmark_total = bookmarks.len();
                let bookmark_items: Vec<Value> = bookmarks
                    .into_iter()
                    .take(LIST_LIMIT)
                    .map(|b| {
                        json!({
                            "title": safe_text(
                                b.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                                80
                            ),
                            // Host only — full URLs are private browsing state.
                            "host": b.get("url").and_then(|v| v.as_str()).and_then(|u| {
                                url::Url::parse(u).ok().and_then(|parsed| {
                                    parsed.host_str().map(|h| safe_text(h, 80))
                                })
                            })
                        })
                    })
                    .collect();
                let set_total = tab_sets.len();
                let set_items: Vec<Value> = tab_sets
                    .into_iter()
                    .take(LIST_LIMIT)
                    .map(|s| {
                        json!({
                            "name": safe_text(
                                s.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                                80
                            ),
                            "tab_count": s.get("tabs").and_then(|v| v.as_array()).map(|t| t.len()).unwrap_or(0)
                        })
                    })
                    .collect();
                if bookmark_total == 0 && set_total == 0 {
                    json!({
                        "status": "empty",
                        "bookmarks": ranked(Vec::new(), 0),
                        "tab_sets": ranked(Vec::new(), 0),
                        "reason": "query succeeded; no browser bookmarks or tab sets saved"
                    })
                } else {
                    json!({
                        "status": "available",
                        "bookmarks": ranked(bookmark_items, bookmark_total),
                        "tab_sets": ranked(set_items, set_total)
                    })
                }
            }
            (Err(e), _) | (_, Err(e)) => {
                partial = true;
                json!({
                    "status": "unavailable",
                    "bookmarks": null,
                    "tab_sets": null,
                    "reason": safe_text(&e, 200)
                })
            }
        };

        let coding_empty = coding_sessions.get("status").and_then(|s| s.as_str()) == Some("empty");
        let browser_empty = browser.get("status").and_then(|s| s.as_str()) == Some("empty");
        let status = if partial {
            "partial"
        } else if coding_empty && browser_empty {
            "empty"
        } else {
            "available"
        };

        json!({
            "surface": "build",
            "status": status,
            "queried": true,
            "data": {
                "coding_sessions": coding_sessions,
                "browser": browser
            }
        })
    }

    /// World-view worker roster — who is on stage and whether they can run.
    ///
    /// The World tab is a visual of the agent ecosystem; without this the agent
    /// could describe the scene editorially while being unable to say which
    /// workers exist or are available on this machine.
    async fn observe_world(&self) -> Value {
        let config = crate::config::agent_identity::load_agent_config();
        let mut workers: Vec<(String, crate::config::agent_identity::WorkerPersona)> =
            config.workers.into_iter().collect();
        workers.sort_by(|a, b| a.0.cmp(&b.0));

        if workers.is_empty() {
            return empty(
                "world",
                "query succeeded; no workers are configured",
                json!({"workers": ranked(Vec::new(), 0)}),
            );
        }

        let total = workers.len();
        // Probe off the async runtime — model_loaded does blocking HTTP.
        let probed = tokio::task::spawn_blocking(move || {
            workers
                .into_iter()
                .map(|(key, worker)| {
                    let (available, reason) =
                        crate::config::worker_probe::probe_worker(&worker.availability_check);
                    (key, worker, available, reason)
                })
                .collect::<Vec<_>>()
        })
        .await;

        let probed = match probed {
            Ok(rows) => rows,
            Err(e) => {
                return unavailable("world", format!("worker probe task failed: {e}"));
            }
        };

        let available_count = probed.iter().filter(|(_, _, a, _)| *a).count();
        let items: Vec<Value> = probed
            .iter()
            .take(LIST_LIMIT)
            .map(|(key, worker, available, reason)| {
                json!({
                    "key": safe_text(key, 64),
                    "name": safe_text(&worker.display_name(), 80),
                    "role": safe_text(&worker.role, 80),
                    "engine": worker.engine.label(),
                    "available": available,
                    "unavailable_reason": reason.as_ref().map(|r| safe_text(r, 120))
                })
            })
            .collect();

        available(
            "world",
            json!({
                "worker_count": total,
                "available_count": available_count,
                "workers": ranked(items, total)
            }),
        )
    }

    /// Configuration state — which providers/extensions are set up.
    ///
    /// Never returns a secret value, not even masked: only whether a key is
    /// present. Without this the agent could not answer "do I have Anthropic
    /// configured" without guessing or reading raw config into the chat.
    async fn observe_settings(&self) -> Value {
        let config = crate::config::Config::global();
        // Prove we can read config at all — a broken store must not look like
        // "no providers configured".
        if let Err(e) = config.all_values() {
            return unavailable(
                "settings",
                format!("could not read configuration store: {e}"),
            );
        }

        let default_provider = config.get_goose_provider().ok();
        let default_model = config.get_goose_model().ok();

        let providers_list = crate::providers::providers().await;
        let provider_total = providers_list.len();
        let mut configured_count = 0usize;
        let mut provider_items = Vec::new();
        for (metadata, _) in providers_list {
            let configured = match crate::providers::get_from_registry(&metadata.name).await {
                Ok(entry) => entry.inventory_configured(),
                Err(_) => false,
            };
            if configured {
                configured_count += 1;
            }
            // Only list configured providers in the bounded list — the full
            // catalog is huge and mostly "not set up"; totals stay authoritative.
            if !configured {
                continue;
            }
            if provider_items.len() >= LIST_LIMIT {
                continue;
            }
            let secret_keys: Vec<Value> = metadata
                .config_keys
                .iter()
                .filter(|k| k.secret)
                .map(|k| {
                    json!({
                        "name": safe_text(&k.name, 64),
                        "present": config_key_present(&k.name, true)
                        // NEVER include the value.
                    })
                })
                .collect();
            provider_items.push(json!({
                "name": safe_text(&metadata.name, 64),
                "display_name": safe_text(&metadata.display_name, 80),
                "configured": true,
                "is_default": default_provider.as_deref() == Some(metadata.name.as_str()),
                "secret_keys": secret_keys
            }));
        }

        let extensions = crate::config::get_all_extensions()
            .into_iter()
            .filter(|ext| {
                !crate::agents::extension_manager::is_hidden_extension(&ext.config.name())
            })
            .collect::<Vec<_>>();
        let extension_total = extensions.len();
        let enabled_count = extensions.iter().filter(|e| e.enabled).count();
        let extension_items: Vec<Value> = extensions
            .into_iter()
            .take(LIST_LIMIT)
            .map(|ext| {
                json!({
                    "name": safe_text(&ext.config.name(), 80),
                    "enabled": ext.enabled
                })
            })
            .collect();

        if configured_count == 0 && extension_total == 0 {
            return empty(
                "settings",
                "query succeeded; no providers configured and no extensions registered",
                json!({
                    "default_provider": default_provider.map(|s| safe_text(&s, 64)),
                    "default_model": default_model.map(|s| safe_text(&s, 80)),
                    "providers": ranked(Vec::new(), 0),
                    "extensions": ranked(Vec::new(), 0)
                }),
            );
        }

        available(
            "settings",
            json!({
                "default_provider": default_provider.map(|s| safe_text(&s, 64)),
                "default_model": default_model.map(|s| safe_text(&s, 80)),
                "providers_configured": configured_count,
                "providers_total": provider_total,
                "providers": ranked(provider_items, configured_count),
                "extensions_enabled": enabled_count,
                "extensions": ranked(extension_items, extension_total)
            }),
        )
    }

    async fn observe_overview(&self, pool: &Pool<Sqlite>) -> Value {
        let projects = projects::list_projects(pool, None).await;
        let goals = cards::list_active_goals(pool).await;
        let sessions = self.context.session_manager.list_session_summaries().await;
        let briefings = briefings::try_unacknowledged_count(pool).await;
        let spend = app_views::read_spend_rows(pool).await;

        let mut partial = false;
        let mut count = |result: std::result::Result<usize, String>| match result {
            Ok(value) => json!({"status": "available", "count": value}),
            Err(error) => {
                partial = true;
                json!({"status": "unavailable", "count": null, "reason": safe_text(&error, 200)})
            }
        };
        let projects_view = count(projects.map(|v| v.len()));
        let goals_view = count(goals.map(|v| v.len()));
        let sessions_view = count(sessions.map(|v| v.len()).map_err(|e| e.to_string()));
        let briefings_view = count(
            briefings
                .map(|v| v.max(0) as usize)
                .map_err(|e| e.to_string()),
        );
        let spend_view = match spend {
            Ok(rows) if rows.is_empty() => {
                json!({"status": "empty", "measured": false, "running_total_usd": null})
            }
            Ok(rows) => json!({
                "status": "available",
                "measured": true,
                "running_total_usd": rows.iter().map(|r| r.cost_usd).sum::<f64>()
            }),
            Err(error) => {
                partial = true;
                json!({
                    "status": "unavailable",
                    "measured": null,
                    "running_total_usd": null,
                    "reason": safe_text(&error.to_string(), 200)
                })
            }
        };

        json!({
            "surface": "overview",
            "status": if partial { "partial" } else { "available" },
            "queried": true,
            "data": {
                "projects": projects_view,
                "active_goals": goals_view,
                "sessions": sessions_view,
                "unread_briefings": briefings_view,
                "spend": spend_view
            }
        })
    }

    async fn handle_observe(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args: ObserveAppParams = arguments
            .map(|obj| serde_json::from_value(Value::Object(obj)))
            .transpose()
            .map_err(|e| format!("Invalid arguments: {e}"))?
            .ok_or_else(|| "Missing arguments".to_string())?;
        let surface = args.surface.trim().to_ascii_lowercase();
        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| format!("Could not open the local app database: {e}"))?;

        let mut payload = match surface.as_str() {
            "analytics" => {
                self.observe_analytics(&pool, args.scope.as_deref(), args.window.as_deref())
                    .await
            }
            "projects" => self.observe_projects(&pool).await,
            "goals" => self.observe_board(&pool, args.scope.as_deref(), true).await,
            "cards" => {
                self.observe_board(&pool, args.scope.as_deref(), false)
                    .await
            }
            "spend" => self.observe_spend(&pool, args.scope.as_deref()).await,
            "sessions" => self.observe_sessions(args.window.as_deref()).await,
            "briefings" => self.observe_briefings(&pool).await,
            "grow" => self.observe_grow(&pool, args.scope.as_deref()).await,
            "inbox" => self.observe_inbox(&pool).await,
            "skills" => self.observe_skills(&pool).await,
            "automate" => self.observe_automate().await,
            "trace" => self.observe_trace(&pool).await,
            "brain" => self.observe_brain(&pool).await,
            "build" => self.observe_build().await,
            "world" => self.observe_world().await,
            "settings" => self.observe_settings().await,
            "overview" => self.observe_overview(&pool).await,
            _ => {
                return Err(format!(
                    "Unknown surface \"{}\". Use one of: {}.",
                    safe_text(&args.surface, 80),
                    OBSERVABLE_SURFACES.join(", ")
                ))
            }
        };
        redact_json(&mut payload);
        let text = serde_json::to_string_pretty(&payload)
            .map_err(|e| format!("Could not serialize app observation: {e}"))?;
        let mut result = CallToolResult::success(vec![Content::text(text)]);
        result.structured_content = Some(payload);
        Ok(result)
    }

    pub(crate) fn get_tools() -> Vec<Tool> {
        vec![Tool::new(
            "observe_app".to_string(),
            "Read aggregate state from the data behind the Permagent app. Call this directly \
             when asked about analytics, projects, goals/cards, spend, sessions, growth \
             actions you recommended, agent briefings, Decision Inbox, skills, scheduled \
             automations, execution trace, brain memory health, build coding sessions or \
             browser bookmarks, world workers, settings configuration, or what \
             is happening overall. It does not require navigation and \
             never use get_page_snapshot for this job: browser snapshots describe a website, \
             not the Permagent app.\n\n\
             ANALYTICS IS THE USER'S OWN WEBSITE TRAFFIC, not Permagent usage stats. \
             Permagent self-hosts a first-party analytics collector, so THIS is the first \
             place to look for pageviews, visitors, referrers, UTM campaigns and custom \
             events for a project — check here before suggesting Google Analytics, \
             Plausible, Fathom or any other third-party tool, and before saying analytics \
             are unavailable. If a project returns no events, say the collector is not \
             installed for it rather than that the data does not exist.\n\n\
             surface: analytics | projects | goals | cards | spend | sessions | briefings | \
             grow | inbox | skills | automate | trace | brain | build | world | settings | \
             overview. analytics and cards require \
             scope = project name, slug, or id; goals accepts an optional project scope; grow \
             requires a project scope. window supports 7d, 30d, 90d, 365d, or all. \
             The analytics surface returns window totals AND a per-day series (`daily`), so \
             answer day-level questions — which day dipped, whether a campaign spiked — from \
             that series rather than declining for lack of granularity. \
             brain reports memory counts, recall health, and librarian schedule/phase — never \
             memory contents. build reports coding-session aggregates and browser bookmarks \
             (hosts only). world reports the worker roster and availability. settings reports \
             which providers/extensions are configured or enabled — never secret values, only \
             whether a key is present. \
             Results are privacy-redacted aggregates with lists capped at five and explicit \
             availability, empty, and truncation states."
                .to_string(),
            schema::<ObserveAppParams>(),
        )]
    }
}

#[async_trait]
impl McpClientTrait for AppPerceptionClient {
    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListToolsResult, Error> {
        Ok(ListToolsResult {
            tools: Self::get_tools(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        _ctx: &ToolCallContext,
        name: &str,
        arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<CallToolResult, Error> {
        let result = match name {
            "observe_app" => self.handle_observe(arguments).await,
            _ => Err(format!("Unknown tool: {name}")),
        };
        match result {
            Ok(result) => Ok(result),
            Err(error) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Error: {error}"
            ))])),
        }
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn seed_memory_db(path: &std::path::Path, rows: &[(&str, &str, Option<&str>)]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE memories (
                id TEXT PRIMARY KEY,
                key TEXT NOT NULL,
                content TEXT NOT NULL,
                description TEXT,
                source TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );",
        )
        .unwrap();
        for (i, (key, content, source)) in rows.iter().enumerate() {
            conn.execute(
                "INSERT INTO memories (id, key, content, source, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    format!("m{i}"),
                    key,
                    content,
                    source,
                    format!("2026-08-0{}T12:00:00Z", (i % 9) + 1)
                ],
            )
            .unwrap();
        }
    }

    fn test_client(data_dir: PathBuf) -> AppPerceptionClient {
        let context = PlatformExtensionContext {
            extension_manager: None,
            session_manager: Arc::new(crate::session::SessionManager::new(data_dir)),
            session: None,
        };
        AppPerceptionClient::new(context).unwrap()
    }

    async fn observe(client: &AppPerceptionClient, surface: &str) -> Value {
        let arguments = json!({ "surface": surface }).as_object().unwrap().clone();
        let result = client
            .call_tool(
                &ToolCallContext::new("test".to_string(), None, None),
                "observe_app",
                Some(arguments),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(!result.is_error.unwrap_or(false));
        result
            .structured_content
            .expect("observe_app must return structured JSON")
    }

    async fn memory_pool() -> Pool<Sqlite> {
        sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap()
    }

    #[test]
    fn parses_only_bounded_windows() {
        assert_eq!(parse_window(Some("30d"), 7).unwrap().1, "30d");
        assert_eq!(parse_window(Some("all"), 7).unwrap().1, "all");
        assert!(parse_window(Some("2020-01-01..2030-01-01"), 7).is_err());
    }

    #[test]
    fn final_redaction_catches_free_text_and_join_ids() {
        let mut value = json!({
            "summary": "email me at a@example.com from /Users/alice/work",
            "accidental_id": "550e8400-e29b-41d4-a716-446655440000"
        });
        redact_json(&mut value);
        let encoded = value.to_string();
        assert!(!encoded.contains("a@example.com"));
        assert!(!encoded.contains("/Users/alice"));
        assert!(!encoded.contains("550e8400"));
    }

    #[test]
    fn list_caps_are_explicit() {
        let value = ranked((0..LIST_LIMIT).map(Value::from).collect(), 54);
        assert_eq!(value["limit"], LIST_LIMIT);
        assert_eq!(value["total"], 54);
        assert_eq!(value["truncated"], true);
    }

    #[tokio::test]
    async fn inbox_is_bounded_and_missing_schema_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let client = test_client(dir.path().to_path_buf());
        let pool = memory_pool().await;
        assert_eq!(client.observe_inbox(&pool).await["status"], "unavailable");
        sqlx::query("CREATE TABLE decisions (id TEXT, kind TEXT, goal_id TEXT, project_id TEXT, tier INTEGER, headline TEXT, detail TEXT, payload_json TEXT, rank REAL, status TEXT, answer TEXT, answer_note TEXT, answer_choice_id TEXT, answer_input TEXT, acted_by TEXT, created_at TEXT, resolved_at TEXT)").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE cards (id TEXT, title TEXT, project_id TEXT, card_type TEXT, column_id TEXT, archived_at TEXT, metadata_json TEXT)").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE board_columns (id TEXT, state_binding TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        for i in 0..(LIST_LIMIT + 1) {
            sqlx::query("INSERT INTO decisions VALUES (?, 'choice', NULL, NULL, 1, ?, 'private detail', '{}', 1, 'open', NULL, NULL, NULL, NULL, NULL, '2026-08-14T00:00:00Z', NULL)")
                .bind(format!("d{i}")).bind(format!("Decision {i}"))
                .execute(&pool).await.unwrap();
        }
        let value = client.observe_inbox(&pool).await;
        assert_eq!(value["data"]["decisions"]["returned"], LIST_LIMIT);
        assert_eq!(value["data"]["decisions"]["truncated"], true);
        assert!(!value.to_string().contains("private detail"));
    }

    #[tokio::test]
    async fn skills_are_bounded_and_missing_schema_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let client = test_client(dir.path().to_path_buf());
        let pool = memory_pool().await;
        assert_eq!(client.observe_skills(&pool).await["status"], "unavailable");
        sqlx::query("CREATE TABLE skills (id TEXT, user_id TEXT, name TEXT, description TEXT, trigger_value TEXT, status TEXT, created_at TEXT)").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE skill_triggers (id TEXT, skill_id TEXT, last_triggered_at TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE skill_executions (id TEXT, skill_id TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE repetition_candidates (tool_used TEXT, argument_shape_hash TEXT, occurrence_count INTEGER, latest_description TEXT, user_id TEXT)").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE skill_dismissals (user_id TEXT, argument_shape_hash TEXT, tool_used TEXT, dismissed_at TEXT)").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE tasks (id TEXT, user_id TEXT, tool_used TEXT, argument_shape_hash TEXT, status TEXT, completed_at TEXT)").execute(&pool).await.unwrap();
        for i in 0..(LIST_LIMIT + 1) {
            sqlx::query("INSERT INTO skills VALUES (?, 'default', ?, 'aggregate only', '{}', 'active', '2026-08-14T00:00:00Z')")
                .bind(format!("s{i}")).bind(format!("Skill {i}"))
                .execute(&pool).await.unwrap();
        }
        let value = client.observe_skills(&pool).await;
        assert_eq!(value["data"]["saved"]["returned"], LIST_LIMIT);
        assert_eq!(value["data"]["saved"]["truncated"], true);
    }

    #[tokio::test]
    async fn automate_is_bounded_and_unreadable_store_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("schedule.json");
        tokio::fs::write(&path, "not json").await.unwrap();
        assert_eq!(
            AppPerceptionClient::observe_automate_path(&path).await["status"],
            "unavailable"
        );
        let jobs: Vec<scheduler::ScheduledJob> = (0..(LIST_LIMIT + 1))
            .map(|i| scheduler::ScheduledJob {
                id: format!("Job {i}"),
                cron: "0 * * * * *".to_string(),
                last_status: Some(if i == 0 {
                    scheduler::ScheduleRunStatus::Missed
                } else {
                    scheduler::ScheduleRunStatus::Ok
                }),
                ..Default::default()
            })
            .collect();
        tokio::fs::write(&path, serde_json::to_vec(&jobs).unwrap())
            .await
            .unwrap();
        let value = AppPerceptionClient::observe_automate_path(&path).await;
        assert_eq!(value["data"]["jobs"]["returned"], LIST_LIMIT);
        assert_eq!(value["data"]["jobs"]["truncated"], true);
        assert_eq!(value["data"]["missed"], 1);
    }

    #[tokio::test]
    async fn trace_is_bounded_and_missing_schema_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let client = test_client(dir.path().to_path_buf());
        let pool = memory_pool().await;
        assert_eq!(client.observe_trace(&pool).await["status"], "unavailable");
        sqlx::query("CREATE TABLE activity_journal (id TEXT, ts TEXT, kind TEXT, actor TEXT, title TEXT, detail TEXT, ref_kind TEXT, ref_id TEXT)").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE cards (id TEXT, project_id TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        for i in 0..(LIST_LIMIT + 1) {
            sqlx::query("INSERT INTO activity_journal VALUES (?, ?, 'task_failed', 'henry', ?, 'private trace detail', NULL, NULL)")
                .bind(format!("a{i}")).bind(format!("2026-08-14T00:00:0{i}Z")).bind(format!("Activity {i}"))
                .execute(&pool).await.unwrap();
        }
        let value = client.observe_trace(&pool).await;
        assert_eq!(value["data"]["activity"]["returned"], LIST_LIMIT);
        assert_eq!(value["data"]["activity"]["truncated"], true);
        assert!(!value.to_string().contains("private trace detail"));
    }

    #[test]
    fn missing_brain_db_is_unavailable_not_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().display().to_string();
        let _env = env_lock::lock_env([("PERMAGENT_PATH_ROOT", Some(root.as_str()))]);
        let err = open_brain_db_readonly().unwrap_err();
        assert!(
            err.contains("not available"),
            "missing DB must not look like an empty Brain: {err}"
        );
    }

    #[test]
    fn memory_counts_are_aggregates_only() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().display().to_string();
        let _env = env_lock::lock_env([("PERMAGENT_PATH_ROOT", Some(root.as_str()))]);
        let db = brain_db_path();
        seed_memory_db(
            &db,
            &[
                ("k1", "SECRET memory body one", Some("chat")),
                ("k2", "SECRET memory body two", Some("chat")),
                ("k3", "SECRET memory body three", None),
            ],
        );
        // Mark one described.
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute(
                "UPDATE memories SET description = 'a short card' WHERE key = 'k1'",
                [],
            )
            .unwrap();
        }
        let conn = open_brain_db_readonly().unwrap();
        let (total, described) = query_memory_counts(&conn).unwrap();
        assert_eq!(total, 3);
        assert_eq!(described, 1);
        // The helper must never return contents — only counts.
        let encoded = format!("{total}:{described}");
        assert!(!encoded.contains("SECRET"));
    }

    #[tokio::test]
    async fn observe_brain_degrades_when_memory_db_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().display().to_string();
        let _env = env_lock::lock_env([("PERMAGENT_PATH_ROOT", Some(root.as_str()))]);
        let client = test_client(tmp.path().to_path_buf());
        let payload = observe(&client, "brain").await;
        assert_eq!(payload["surface"], "brain");
        assert_eq!(payload["queried"], true);
        // Missing DB is unavailable for memories — not an empty Brain.
        assert_eq!(payload["data"]["memories"]["status"], "unavailable");
        assert!(payload["data"]["memories"]["reason"].as_str().is_some());
        // Librarian phase is always local; still present.
        assert!(payload["data"]["librarian"]["phase"].is_string());
        let encoded = payload.to_string();
        assert!(!encoded.contains("content"));
        assert_ne!(payload["data"]["memories"]["status"], "empty");
    }

    #[tokio::test]
    async fn observe_brain_bounds_and_hides_memory_bodies() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().display().to_string();
        let _env = env_lock::lock_env([("PERMAGENT_PATH_ROOT", Some(root.as_str()))]);
        seed_memory_db(
            &brain_db_path(),
            &[("k1", "PRIVATE body must never appear", Some("chat"))],
        );
        let client = test_client(tmp.path().to_path_buf());
        let payload = observe(&client, "brain").await;
        assert_eq!(payload["data"]["memories"]["status"], "available");
        assert_eq!(payload["data"]["memories"]["total"], 1);
        let encoded = payload.to_string();
        assert!(
            !encoded.contains("PRIVATE body"),
            "brain aspect must never dump memory contents: {encoded}"
        );
    }

    #[tokio::test]
    async fn observe_build_degrades_when_memory_db_missing_and_bounds_browser() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().display().to_string();
        let _env = env_lock::lock_env([("PERMAGENT_PATH_ROOT", Some(root.as_str()))]);
        // Browser state present; brain DB absent → partial, not empty-as-broken.
        let bookmarks = json!({
            "bookmarks": (0..8).map(|i| json!({
                "url": format!("https://example{i}.com/path"),
                "title": format!("Bookmark {i}"),
                "createdAt": "2026-08-01T00:00:00Z"
            })).collect::<Vec<_>>()
        });
        std::fs::write(
            tmp.path().join("browser_bookmarks.json"),
            serde_json::to_string(&bookmarks).unwrap(),
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("browser_tab_sets.json"),
            r#"{"tabSets":[]}"#,
        )
        .unwrap();

        let client = test_client(tmp.path().to_path_buf());
        let payload = observe(&client, "build").await;
        assert_eq!(payload["surface"], "build");
        assert_eq!(payload["data"]["coding_sessions"]["status"], "unavailable");
        assert_eq!(payload["data"]["browser"]["status"], "available");
        assert_eq!(payload["data"]["browser"]["bookmarks"]["truncated"], true);
        assert_eq!(
            payload["data"]["browser"]["bookmarks"]["returned"],
            LIST_LIMIT
        );
        assert_eq!(payload["data"]["browser"]["bookmarks"]["total"], 8);
        // Host only — no full URLs in the aggregate.
        let encoded = payload.to_string();
        assert!(!encoded.contains("/path"));
        assert!(encoded.contains("example0.com") || encoded.contains("example"));
    }

    #[tokio::test]
    async fn observe_build_reports_empty_coding_sessions_honestly() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().display().to_string();
        let _env = env_lock::lock_env([("PERMAGENT_PATH_ROOT", Some(root.as_str()))]);
        seed_memory_db(
            &brain_db_path(),
            &[("other-key", "not a coding session", None)],
        );
        let client = test_client(tmp.path().to_path_buf());
        let payload = observe(&client, "build").await;
        assert_eq!(payload["data"]["coding_sessions"]["status"], "empty");
        assert_eq!(payload["data"]["coding_sessions"]["sessions"]["total"], 0);
        // Empty is not unavailable.
        assert_ne!(payload["data"]["coding_sessions"]["status"], "unavailable");
    }

    #[tokio::test]
    async fn observe_world_returns_bounded_worker_roster() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().display().to_string();
        let _env = env_lock::lock_env([("PERMAGENT_PATH_ROOT", Some(root.as_str()))]);
        // No agent.yaml → default roster is merged in; still a real aggregate.
        let client = test_client(tmp.path().to_path_buf());
        let payload = observe(&client, "world").await;
        assert_eq!(payload["surface"], "world");
        assert_eq!(payload["status"], "available");
        assert!(payload["data"]["worker_count"].as_u64().unwrap() > 0);
        let workers = &payload["data"]["workers"];
        assert!(workers["returned"].as_u64().unwrap() <= LIST_LIMIT as u64);
        assert_eq!(workers["limit"], LIST_LIMIT);
        if workers["total"].as_u64().unwrap() > LIST_LIMIT as u64 {
            assert_eq!(workers["truncated"], true);
        }
        // No secret-looking fields.
        let encoded = payload.to_string();
        assert!(!encoded.contains("api_key"));
        assert!(!encoded.contains("API_KEY"));
    }

    /// NOT hermetic, so ignored by default: `observe_settings` reads the LIVE
    /// global config, and `Config::global()`'s keyring choice is baked in at
    /// first access — `PERMAGENT_DISABLE_KEYRING` set per-test cannot undo it.
    /// On macOS, `all_secrets` then calls SecKeychainFindGenericPassword, which
    /// blocks forever on an authorization dialog a headless test cannot show
    /// (observed hanging the suite >10 min, and stalling every later env_lock
    /// test behind its mutex). Run explicitly on a machine where the test
    /// binary may read the keychain:
    /// `cargo test -p permagent observe_settings -- --ignored`
    #[tokio::test]
    #[ignore = "reads the live keychain via Config::global(); hangs headless"]
    async fn observe_settings_never_emits_secret_values() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().display().to_string();
        let _env = env_lock::lock_env([("PERMAGENT_PATH_ROOT", Some(root.as_str()))]);
        let client = test_client(tmp.path().to_path_buf());
        let payload = observe(&client, "settings").await;
        assert_eq!(payload["surface"], "settings");
        assert!(payload["queried"].as_bool().unwrap());
        // Status is available or empty — never a fabricated secret dump.
        assert!(
            matches!(
                payload["status"].as_str(),
                Some("available") | Some("empty") | Some("unavailable")
            ),
            "unexpected status: {}",
            payload["status"]
        );
        let encoded = payload.to_string();
        // Secret *names* may appear; values must not. A sk- style token is a
        // value leak; a present:bool is fine.
        assert!(!encoded.contains("sk-"));
        assert!(!encoded.contains("\"value\""));
        if let Some(providers) = payload["data"]["providers"]["items"].as_array() {
            assert!(providers.len() <= LIST_LIMIT);
            for p in providers {
                if let Some(keys) = p["secret_keys"].as_array() {
                    for k in keys {
                        assert!(k.get("value").is_none());
                        assert!(k["present"].is_boolean());
                    }
                }
            }
        }
    }

    /// Reproducible acceptance harness for a copied production database.
    ///
    /// Run with:
    /// `PERMAGENT_ACCEPTANCE_DB_DIR=/path/to/copy cargo test -p permagent
    /// copied_database_tool_acceptance -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "requires PERMAGENT_ACCEPTANCE_DB_DIR containing a copied permagent.db"]
    async fn copied_database_tool_acceptance() {
        let data_dir = PathBuf::from(
            std::env::var("PERMAGENT_ACCEPTANCE_DB_DIR")
                .expect("PERMAGENT_ACCEPTANCE_DB_DIR must point at the copied DB directory"),
        );
        let context = PlatformExtensionContext {
            extension_manager: None,
            session_manager: std::sync::Arc::new(crate::session::SessionManager::new(data_dir)),
            session: None,
        };
        let client = AppPerceptionClient::new(context).unwrap();
        let arguments = json!({
            "surface": "analytics",
            "scope": "Grocery Savers",
            "window": "all"
        })
        .as_object()
        .unwrap()
        .clone();
        let result = client
            .call_tool(
                &ToolCallContext::new("acceptance".to_string(), None, None),
                "observe_app",
                Some(arguments),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(!result.is_error.unwrap_or(false));
        let payload = result
            .structured_content
            .expect("observe_app must return structured JSON");
        println!("{}", serde_json::to_string_pretty(&payload).unwrap());
        assert_eq!(payload["status"], "available");
        assert_eq!(payload["data"]["project"], "Grocery Savers");
        assert!(payload["data"]["unique_visitors"].is_number());
        assert!(payload["data"]["pageviews"].is_number());
        assert!(payload["data"]["events"].is_number());
        assert!(payload["data"]["bots"].is_number());
    }
}
