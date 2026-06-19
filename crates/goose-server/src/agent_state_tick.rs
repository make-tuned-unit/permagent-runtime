//! Agent runtime-state tick — #288 interim approach (A).
//!
//! Derives Henry's coarse runtime state from the SAME signals as
//! `/api/henry/status` (active sessions + in-flight tools, see
//! [`crate::routes::henry_status::classify_henry_state`]) and emits an
//! `agent_state_changed` event ONLY on transition, so World View reacts live
//! instead of polling. Zero `session_manager` surgery.
//!
//! BOUNDARY (interim A): caps at `available`/`working` — `idle` backend state
//! maps to `available`, matching the frontend `mapHenryState`. It NEVER emits
//! `error`: a real per-agent error signal needs the session lifecycle hooks
//! deferred to #288 follow-up (B). Daemon-unreachable error stays a frontend
//! signal (the status poll failing).

use crate::routes::henry_status::classify_henry_state;
use crate::state::AppState;
use permagent::session::session_manager::SessionType;
use std::sync::Arc;
use std::time::Duration;

const TICK_INTERVAL: Duration = Duration::from_secs(2);
const ACTIVE_WINDOW_SECS: i64 = 120;

/// Spawn the long-lived agent-state tick. Emits `agent_state_changed` for Henry
/// when his derived HUD state changes.
pub fn spawn(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut last: Option<&'static str> = None;
        let mut ticker = tokio::time::interval(TICK_INTERVAL);
        loop {
            ticker.tick().await;
            let hud = derive_henry_hud_state(&state).await;
            if last == Some(hud) {
                continue;
            }
            last = Some(hud);
            let name = {
                let persona = state.persona.read().await;
                if persona.first_name.is_empty() {
                    "Aria".to_string()
                } else {
                    persona.first_name.clone()
                }
            };
            permagent::events::emit(permagent::events::agent_state_changed("henry", &name, hud));
        }
    });
}

/// Derive Henry's HUD state (`working` | `available`) from active sessions and
/// in-flight tools, mirroring `mapHenryState` on the frontend.
async fn derive_henry_hud_state(state: &AppState) -> &'static str {
    let two_min_ago = chrono::Utc::now() - chrono::Duration::seconds(ACTIVE_WINDOW_SECS);
    let sessions = state
        .session_manager()
        .list_sessions()
        .await
        .unwrap_or_default();

    let active_ids: Vec<String> = sessions
        .iter()
        .filter(|s| {
            s.updated_at >= two_min_ago
                && matches!(s.session_type, SessionType::User | SessionType::Scheduled)
        })
        .map(|s| s.id.clone())
        .collect();

    // In-flight tool? Mirror henry_status::find_current_tool (active request ids).
    let mut has_tool = false;
    for id in &active_ids {
        if let Some(bus) = state.get_event_bus(id).await {
            if !bus.active_request_ids().await.is_empty() {
                has_tool = true;
                break;
            }
        }
    }

    match classify_henry_state(has_tool, !active_ids.is_empty()) {
        "tool_call" | "in_conversation" => "working",
        _ => "available",
    }
}
