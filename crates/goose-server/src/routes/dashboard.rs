use crate::state::AppState;
use axum::{extract::State, routing::get, Json, Router};
use chrono::{NaiveTime, Utc};
use permagent::session::session_manager::SessionType;
use serde::Serialize;
use std::sync::Arc;

#[derive(Serialize)]
struct AgentStatus {
    name: String,
    state: String,
    active_count: usize,
    summary: String,
}

#[derive(Serialize)]
struct DashboardStats {
    sessions_today: usize,
    sessions_total: usize,
    memory_count: usize,
    memory_delta_today: usize,
}

#[derive(Serialize)]
struct InFlightSession {
    id: String,
    title: String,
    started_at: String,
    state: String,
    progress: f64,
}

#[derive(Serialize)]
struct RecentSession {
    id: String,
    title: String,
    state: String,
    ended_at: String,
}

#[derive(Serialize)]
struct DashboardResponse {
    agent: AgentStatus,
    stats: DashboardStats,
    in_flight: Vec<InFlightSession>,
    recent: Vec<RecentSession>,
}

async fn get_dashboard(State(state): State<Arc<AppState>>) -> Json<DashboardResponse> {
    let persona = state.persona.read().await;
    let name = persona.first_name.clone();
    drop(persona);

    let now = Utc::now();
    let today_midnight = now.date_naive().and_time(NaiveTime::MIN).and_utc();

    // Fetch all sessions
    let sessions = state
        .session_manager()
        .list_sessions()
        .await
        .unwrap_or_default();

    let sessions_total = sessions.len();
    let sessions_today = sessions
        .iter()
        .filter(|s| s.created_at >= today_midnight)
        .count();

    // Determine active sessions (updated within last 2 minutes, not system type)
    let two_min_ago = now - chrono::Duration::seconds(120);
    let active: Vec<_> = sessions
        .iter()
        .filter(|s| {
            s.updated_at >= two_min_ago
                && matches!(s.session_type, SessionType::User | SessionType::Scheduled)
        })
        .collect();
    let active_count = active.len();

    let agent_state = if active_count > 0 { "thinking" } else { "idle" };
    let summary = match active_count {
        0 => format!("{} is ready", name),
        1 => format!("{} is working on 1 thing for you", name),
        n => format!("{} is working on {} things for you", name, n),
    };

    // Memory stats from brain
    let (memory_count, memory_delta_today) = if let Some(brain) = state.brain.as_ref() {
        let brain = brain.clone();
        let query = name.clone();
        let result = tokio::task::spawn_blocking(move || {
            brain.recall(&query, spectral::Visibility::Private)
        })
        .await
        .ok()
        .and_then(|r| r.ok());

        match result {
            Some(r) => {
                let ent_count = r.graph.neighborhood.entities.len();
                let mem_count = r.memory_hits.len();
                (ent_count + mem_count, 0) // delta requires timestamp tracking we don't have
            }
            None => (0, 0),
        }
    } else {
        (0, 0)
    };

    // In-flight: active sessions (updated within 2 min)
    let in_flight: Vec<InFlightSession> = active
        .iter()
        .take(3)
        .map(|s| {
            let elapsed_min = (now - s.updated_at).num_minutes().max(0) as f64;
            let progress = (elapsed_min / 5.0).min(0.95);
            InFlightSession {
                id: s.id.clone(),
                title: if s.name.is_empty() || s.name == "New Chat" {
                    format!("Session {}", s.id)
                } else {
                    truncate(&s.name, 40)
                },
                started_at: s.created_at.to_rfc3339(),
                state: "thinking".to_string(),
                progress,
            }
        })
        .collect();

    // Recent: last 4 non-active sessions
    let recent: Vec<RecentSession> = sessions
        .iter()
        .filter(|s| {
            s.updated_at < two_min_ago
                && matches!(s.session_type, SessionType::User | SessionType::Scheduled)
        })
        .take(4)
        .map(|s| {
            // Non-active sessions (updated_at > 2 min ago) are completed.
            // We have no explicit pause/stop events so all recent sessions
            // are treated as completed to avoid false "paused" labels.
            let state = "completed";
            RecentSession {
                id: s.id.clone(),
                title: if s.name.is_empty() || s.name == "New Chat" {
                    format!("Session {}", s.id)
                } else {
                    truncate(&s.name, 40)
                },
                state: state.to_string(),
                ended_at: s.updated_at.to_rfc3339(),
            }
        })
        .collect();

    Json(DashboardResponse {
        agent: AgentStatus {
            name,
            state: agent_state.to_string(),
            active_count,
            summary,
        },
        stats: DashboardStats {
            sessions_today,
            sessions_total,
            memory_count,
            memory_delta_today,
        },
        in_flight,
        recent,
    })
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", s.get(..end).unwrap_or(s))
    }
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/dashboard", get(get_dashboard))
        .with_state(state)
}
