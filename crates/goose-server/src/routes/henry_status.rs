use crate::state::AppState;
use axum::{extract::State, routing::get, Json, Router};
use chrono::{NaiveTime, Utc};
use permagent::session::session_manager::SessionType;
use serde::Serialize;
use std::sync::Arc;

#[derive(Serialize)]
struct Identity {
    name: String,
    traits: Vec<String>,
    tone: String,
}

#[derive(Serialize)]
struct ActiveSession {
    id: String,
    name: String,
    started_at: String,
}

#[derive(Serialize)]
struct RecentTask {
    id: String,
    description: String,
    status: String,
    tool_used: Option<String>,
    completed_at: Option<String>,
}

#[derive(Serialize)]
struct TodayTotals {
    messages_sent: usize,
    tasks_dispatched: usize,
    scheduled_fires: usize,
    memories_formed: usize,
}

#[derive(Serialize)]
struct LifetimeStats {
    total_memories: usize,
    total_sessions: usize,
    days_active: usize,
    first_active: Option<String>,
}

#[derive(Serialize)]
struct NextScheduled {
    id: String,
    cron: String,
    currently_running: bool,
}

#[derive(Serialize)]
struct HenryStatusResponse {
    identity: Identity,
    current_state: String,
    active_sessions: Vec<ActiveSession>,
    current_tool: Option<String>,
    tasks_in_flight: usize,
    recent_tasks: Vec<RecentTask>,
    today_totals: TodayTotals,
    lifetime_stats: LifetimeStats,
    next_scheduled: Option<NextScheduled>,
}

async fn henry_status(State(state): State<Arc<AppState>>) -> Json<HenryStatusResponse> {
    let now = Utc::now();
    let today_midnight = now.date_naive().and_time(NaiveTime::MIN).and_utc();
    let two_min_ago = now - chrono::Duration::seconds(120);

    // Identity from persona
    let persona = state.persona.read().await;
    let identity = Identity {
        name: persona.first_name.clone(),
        traits: persona.traits.clone(),
        tone: persona.tone.clone(),
    };
    drop(persona);

    // Sessions — active + totals
    let sessions = state
        .session_manager()
        .list_sessions()
        .await
        .unwrap_or_default();

    let active: Vec<_> = sessions
        .iter()
        .filter(|s| {
            s.updated_at >= two_min_ago
                && matches!(s.session_type, SessionType::User | SessionType::Scheduled)
        })
        .collect();

    let active_sessions: Vec<ActiveSession> = active
        .iter()
        .take(5)
        .map(|s| ActiveSession {
            id: s.id.clone(),
            name: if s.name.is_empty() || s.name == "New Chat" {
                format!("Session {}", s.id.get(..8).unwrap_or(&s.id))
            } else {
                s.name.clone()
            },
            started_at: s.created_at.to_rfc3339(),
        })
        .collect();

    // Current tool: check active session event buses for in-flight requests
    let current_tool = find_current_tool(&state, &active_sessions).await;

    // Determine state. A real error from the agent's actual reply lifecycle
    // (#348, latched in the runtime registry) outranks the session-activity
    // derive — otherwise this 2s poll would clobber a real failure back to
    // available/working. Working/available themselves still agree with the
    // session derive (an in-flight reply IS an active session).
    let current_state = if permagent::events::agent_errored("henry") {
        "error".to_string()
    } else {
        classify_henry_state(current_tool.is_some(), !active_sessions.is_empty()).to_string()
    };

    // Spectral DB queries (tasks, messages, session stats)
    let today_str = today_midnight.format("%Y-%m-%dT%H:%M:%S").to_string();
    let (
        tasks_in_flight,
        recent_tasks,
        tasks_today,
        messages_today,
        first_active,
        total_sessions_all,
        days_active,
    ) = tokio::task::spawn_blocking(move || query_spectral_stats(&today_str))
        .await
        .unwrap_or_else(|_| (0, vec![], 0, 0, None, 0, 0));

    // Scheduled jobs
    let jobs = state.scheduler().list_scheduled_jobs().await;
    let scheduled_fires_today = jobs
        .iter()
        .filter(|j| j.last_run.map(|lr| lr >= today_midnight).unwrap_or(false))
        .count();

    let next_scheduled = jobs
        .iter()
        .filter(|j| !j.paused)
        .min_by_key(|j| j.last_run)
        .map(|j| NextScheduled {
            id: j.id.clone(),
            cron: j.cron.clone(),
            currently_running: j.currently_running,
        });

    // Memory stats from Brain SQLite
    let today_str2 = today_midnight.format("%Y-%m-%d").to_string();
    let (total_memories, memories_today) =
        tokio::task::spawn_blocking(move || query_brain_memory_stats(&today_str2))
            .await
            .unwrap_or((0, 0));

    Json(HenryStatusResponse {
        identity,
        current_state,
        active_sessions,
        current_tool,
        tasks_in_flight,
        recent_tasks,
        today_totals: TodayTotals {
            messages_sent: messages_today,
            tasks_dispatched: tasks_today,
            scheduled_fires: scheduled_fires_today,
            memories_formed: memories_today,
        },
        lifetime_stats: LifetimeStats {
            total_memories,
            total_sessions: total_sessions_all,
            days_active,
            first_active,
        },
        next_scheduled,
    })
}

/// Classify Henry's coarse runtime state from whether a tool is in flight and
/// whether any session is active. Shared by this route and the #288 agent-state
/// tick (`crate::agent_state_tick`) so both derive state identically.
pub(crate) fn classify_henry_state(has_tool: bool, has_active_session: bool) -> &'static str {
    if has_tool {
        "tool_call"
    } else if has_active_session {
        "in_conversation"
    } else {
        "idle"
    }
}

/// Check active session buses for in-flight requests, returning the name of the
/// tool currently executing (#84). Falls back to `"active"` when a request is in
/// flight but no specific tool is running yet (e.g. the model is still
/// generating) — preserving the pre-#84 "is Henry busy" signal.
async fn find_current_tool(state: &AppState, active_sessions: &[ActiveSession]) -> Option<String> {
    for session in active_sessions {
        if let Some(bus) = state.get_event_bus(&session.id).await {
            let request_ids = bus.active_request_ids().await;
            if !request_ids.is_empty() {
                return Some(
                    bus.current_tool()
                        .await
                        .unwrap_or_else(|| "active".to_string()),
                );
            }
        }
    }
    None
}

/// Query the Spectral DB (permagent.db) for tasks, messages, and session stats.
/// Runs on a blocking thread because rusqlite is synchronous.
fn query_spectral_stats(
    today_str: &str,
) -> (
    usize,
    Vec<RecentTask>,
    usize,
    usize,
    Option<String>,
    usize,
    usize,
) {
    let db_path = permagent::config::paths::Paths::spectral_db();
    let conn = match rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(c) => c,
        Err(_) => return (0, vec![], 0, 0, None, 0, 0),
    };

    // Tasks in flight
    let tasks_in_flight: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE status = 'running'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // Recent completed tasks
    let recent_tasks = {
        let mut stmt = conn
            .prepare(
                "SELECT id, description, status, tool_used, completed_at FROM tasks \
                 WHERE status IN ('completed', 'failed') \
                 ORDER BY completed_at DESC LIMIT 5",
            )
            .unwrap();
        stmt.query_map([], |row| {
            Ok(RecentTask {
                id: row.get(0)?,
                description: row.get(1)?,
                status: row.get(2)?,
                tool_used: row.get(3)?,
                completed_at: row.get(4)?,
            })
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
        .unwrap_or_default()
    };

    // Tasks dispatched today
    let tasks_today: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE created_at >= ?1",
            [today_str],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // Messages sent today (role = 'assistant')
    let messages_today: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE role = 'assistant' AND created_at >= ?1",
            [today_str],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // First active session
    let first_active: Option<String> = conn
        .query_row(
            "SELECT MIN(created_at) FROM sessions WHERE session_type IN ('user', 'scheduled')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(None);

    // Total sessions
    let total_sessions: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE session_type IN ('user', 'scheduled')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // Days active
    let days_active: usize = conn
        .query_row(
            "SELECT COUNT(DISTINCT date(created_at)) FROM sessions WHERE session_type IN ('user', 'scheduled')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    (
        tasks_in_flight,
        recent_tasks,
        tasks_today,
        messages_today,
        first_active,
        total_sessions,
        days_active,
    )
}

/// Query Brain's memory.db for total memories and memories created today.
fn query_brain_memory_stats(today_str: &str) -> (usize, usize) {
    let conn = match crate::brain_ops::read_only_brain_conn() {
        Ok(c) => c,
        Err(_) => return (0, 0),
    };

    let total: usize = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
        .unwrap_or(0);

    let today: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE created_at >= ?1",
            [today_str],
            |r| r.get(0),
        )
        .unwrap_or(0);

    (total, today)
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/henry/status", get(henry_status))
        .with_state(state)
}
