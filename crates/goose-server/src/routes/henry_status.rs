use crate::state::AppState;
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{NaiveTime, Utc};
use permagent::session::session_manager::SessionType;
use serde::{Deserialize, Serialize};
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

/// One unread report from a worker agent, for the HUD. Mirrors what Henry sees
/// in his own brief, so the user and Henry are looking at the same list.
#[derive(Serialize)]
struct BriefingSummary {
    id: String,
    from: String,
    severity: String,
    summary: String,
    created_at: String,
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
    /// What Henry's agents have reported and he has not yet cleared.
    briefings: Vec<BriefingSummary>,
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
    // Lean projection (SessionSummary): this endpoint is polled every 1s by the
    // HenryHUD and only reads id/name/type/created_at/updated_at — never the
    // heavy JSON blobs. The fat list_sessions() ran 1-3.5s on 273 sessions every
    // second, a self-inflicted query storm that contended the shared pool. See
    // #341/#371.
    let sessions = state
        .session_manager()
        .list_session_summaries()
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

    // Determine state from the in-flight signals only. A session that merely
    // exists, or was touched in the last two minutes (a blank "New Chat"), is
    // NOT activity. See `derive_henry_state`.
    let current_state = derive_henry_state(
        permagent::events::agent_runtime_state("henry"),
        current_tool.is_some(),
    )
    .to_string();

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

    // Scheduled jobs. This is HENRY's status, so it reports only jobs Henry
    // owns. A job carrying a `worker_persona` belongs to that agent — the
    // Steward's `git-steward` recipe being the case that surfaced this: with no
    // ownership filter, the globally-oldest job won `next_scheduled` regardless
    // of whose it was, and Henry's HUD announced the Steward's cron as his own.
    let jobs = state.scheduler().list_scheduled_jobs().await;
    let henry_owned = || jobs.iter().filter(|j| j.worker_persona.is_none());

    let scheduled_fires_today = henry_owned()
        .filter(|j| j.last_run.map(|lr| lr >= today_midnight).unwrap_or(false))
        .count();

    let next_scheduled = henry_owned()
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

    // Unread agent reports. Same source and ordering Henry sees in his own
    // brief — severity first — so the HUD and the agent never disagree about
    // what is outstanding.
    let briefings = match state.session_manager().pool_clone().await {
        Ok(pool) => permagent::briefings::unacknowledged(&pool, 20)
            .await
            .into_iter()
            .map(|b| BriefingSummary {
                id: b.id,
                from: permagent::briefings::display_name_for(&b.from_agent),
                severity: b.severity.render().to_string(),
                summary: b.summary,
                created_at: b.created_at,
            })
            .collect(),
        Err(_) => Vec::new(),
    };

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
        briefings,
    })
}

/// Henry's coarse runtime state, derived ONLY from in-flight work.
///
/// `runtime` is the reply-loop registry (`permagent::events`, a Drop-guarded
/// in-flight count plus a real error latch); `has_in_flight_request` is whether
/// any session event bus holds an active request. Session recency is
/// deliberately not an input: creating an empty chat session must not make an
/// idle agent look busy. Shared by this route and the agent-state tick
/// (`crate::agent_state_tick`) so both derive state identically.
///
/// Returns `tool_call` | `in_conversation` | `idle` | `error`.
pub(crate) fn derive_henry_state(
    runtime: Option<permagent::events::AgentRuntimeState>,
    has_in_flight_request: bool,
) -> &'static str {
    use permagent::events::AgentRuntimeState;
    match (runtime, has_in_flight_request) {
        (_, true) => "tool_call",
        (Some(AgentRuntimeState::Working), false) => "in_conversation",
        (Some(AgentRuntimeState::Error), false) => "error",
        (Some(AgentRuntimeState::Available), false) | (None, false) => "idle",
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

#[derive(Deserialize)]
struct AckBriefingsRequest {
    ids: Vec<String>,
}

#[derive(Serialize)]
struct AckBriefingsResponse {
    acknowledged: u64,
}

/// Wave-1 item 5: briefings were un-acknowledgeable — no route, no button —
/// so the unacked list only ever grew. Ack means "seen", never approval
/// (the briefings.rs contract).
async fn ack_briefings(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AckBriefingsRequest>,
) -> Result<Json<AckBriefingsResponse>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let acknowledged = permagent::briefings::acknowledge(&pool, &req.ids)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(AckBriefingsResponse { acknowledged }))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/henry/status", get(henry_status))
        .route("/api/henry/briefings/ack", post(ack_briefings))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::derive_henry_state;
    use permagent::events::AgentRuntimeState;

    #[test]
    fn blank_session_is_not_working() {
        // A freshly created, empty session: no turn ever ran, no request.
        assert_eq!(derive_henry_state(None, false), "idle");
    }

    #[test]
    fn idle_after_a_finished_turn() {
        assert_eq!(
            derive_henry_state(Some(AgentRuntimeState::Available), false),
            "idle"
        );
    }

    #[test]
    fn in_flight_reply_turn_is_working() {
        assert_eq!(
            derive_henry_state(Some(AgentRuntimeState::Working), false),
            "in_conversation"
        );
    }

    #[test]
    fn in_flight_request_is_working_even_without_registry_state() {
        assert_eq!(derive_henry_state(None, true), "tool_call");
        assert_eq!(
            derive_henry_state(Some(AgentRuntimeState::Available), true),
            "tool_call"
        );
    }

    #[test]
    fn real_error_is_reported_until_new_work_starts() {
        assert_eq!(
            derive_henry_state(Some(AgentRuntimeState::Error), false),
            "error"
        );
        assert_eq!(
            derive_henry_state(Some(AgentRuntimeState::Error), true),
            "tool_call"
        );
    }
}
