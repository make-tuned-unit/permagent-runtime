//! Agent runtime-state tick — emits `agent_state_changed` on transition.
//!
//! As of #348 the state is sourced from the REAL agent lifecycle registry
//! ([`permagent::events::agent_runtime_state`]) — fed by the actual reply loop —
//! so `working` is a live in-flight turn and `error` is a real latched failure,
//! not the #288 interim-A derived-on-tick guess. This tick remains the single
//! emitter (it holds the persona name) and the reconciling heartbeat: it reads
//! the registry and pushes `agent_state_changed` ONLY on transition, so World
//! View reacts live instead of polling. Before the agent's first reply turn the
//! registry is empty and we fall back to the original session-activity derive,
//! so the HUD always shows something. Daemon-unreachable error stays a frontend
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

/// Henry's HUD state (`working` | `available` | `error`). Real lifecycle state
/// (#348) is authoritative once the agent has run at least one reply turn; the
/// session-activity derive is only a pre-first-turn fallback.
async fn derive_henry_hud_state(state: &AppState) -> &'static str {
    // Authoritative: real reply-loop lifecycle (in-flight ref-count + error latch).
    if let Some(rt) = permagent::events::agent_runtime_state("henry") {
        return rt.as_str();
    }

    // Pre-first-turn fallback: derive from active sessions + in-flight tools,
    // mirroring `mapHenryState` on the frontend.
    let two_min_ago = chrono::Utc::now() - chrono::Duration::seconds(ACTIVE_WINDOW_SECS);
    // Lean projection — this 2s-interval fallback only needs id/type/updated_at.
    let sessions = state
        .session_manager()
        .list_session_summaries()
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
