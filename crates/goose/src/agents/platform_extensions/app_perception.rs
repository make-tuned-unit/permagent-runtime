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
use crate::{briefings, cards, projects};
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
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "app_perception";
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
             briefings, or an overview. This is structured local state, not screenshot vision \
             and not the website in the Build browser",
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
    /// sessions, briefings, or overview.
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
                 spend, sessions, agent briefings, or the overall home. Do not navigate first \
                 and do not use browser snapshots: browser tools see websites, not this app. \
                 This extension is read-only and returns aggregate answers with bounded lists.",
            );
        Ok(Self { info, context })
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
            "overview" => self.observe_overview(&pool).await,
            _ => {
                return Err(format!(
                    "Unknown surface \"{}\". Use analytics, projects, goals, cards, spend, \
                     sessions, briefings, or overview.",
                    safe_text(&args.surface, 80)
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
             when asked about analytics, projects, goals/cards, spend, sessions, agent \
             briefings, or what is happening overall. It does not require navigation and \
             never use get_page_snapshot for this job: browser snapshots describe a website, \
             not the Permagent app.\n\n\
             surface: analytics | projects | goals | cards | spend | sessions | briefings | \
             overview. analytics and cards require scope = project name, slug, or id; goals \
             accepts an optional project scope. window supports 7d, 30d, 90d, 365d, or all. \
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
