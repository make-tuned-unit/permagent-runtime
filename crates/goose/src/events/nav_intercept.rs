//! Per-session navigation interception — the "speak-then-act" ordering seam.
//!
//! By default `navigate_app` emits an `AppNavigate` event on the global bus the
//! instant the tool call resolves, and the frontend switches views immediately.
//! In a *voice* turn that loses a race: the narration TTS streams and plays with
//! synth + playback latency, so the view switches before the agent has finished
//! saying it will. The two channels (global bus → `/events`, TTS → `/voice`) are
//! fully decoupled, and the only true "narration played" signal lives in the
//! frontend audio queue — so the emit point cannot observe it.
//!
//! This registry lets a transport (the `/voice` WS) *intercept* navigations for
//! the duration of a reply: while interception is active for a session,
//! `navigate_app` captures the intent here instead of emitting to the global bus.
//! The transport drains the captured intents after it has finished streaming the
//! narration and forwards them down its own ordered channel, where the client
//! fires them only once the audio queue has drained.
//!
//! Text-chat turns never call [`begin`], so [`capture`] returns `false` and
//! `navigate_app` emits to the global bus exactly as before — no behavior change.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// A captured navigation, carrying everything the frontend needs to apply it.
#[derive(Clone, Debug, PartialEq)]
pub struct NavIntent {
    pub tab: String,
    pub tool_type: String,
    pub panel_type: String,
    pub section: Option<String>,
    pub state: Option<Value>,
    pub reason: String,
}

static REGISTRY: LazyLock<Mutex<HashMap<String, Vec<NavIntent>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Begin intercepting navigations for `session_id`. Until [`take`] is called,
/// `navigate_app` calls for this session are captured instead of emitted to the
/// global bus. Idempotent; resets any prior buffer for the session.
pub fn begin(session_id: &str) {
    if let Ok(mut reg) = REGISTRY.lock() {
        reg.insert(session_id.to_string(), Vec::new());
    }
}

/// Capture `intent` for `session_id` if interception is active. Returns `true`
/// when captured (caller should NOT emit to the global bus), `false` when no
/// interception is active (caller should emit as normal). Atomic under the lock,
/// so there is no check-then-act gap with the emit decision.
pub fn capture(session_id: &str, intent: NavIntent) -> bool {
    if let Ok(mut reg) = REGISTRY.lock() {
        if let Some(buf) = reg.get_mut(session_id) {
            buf.push(intent);
            return true;
        }
    }
    false
}

/// End interception for `session_id` and return any captured intents (in call
/// order). Safe to call when interception was never started — returns empty.
pub fn take(session_id: &str) -> Vec<NavIntent> {
    REGISTRY
        .lock()
        .ok()
        .and_then(|mut reg| reg.remove(session_id))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent(tab: &str) -> NavIntent {
        NavIntent {
            tab: tab.to_string(),
            tool_type: "memory".to_string(),
            panel_type: "panel".to_string(),
            section: None,
            state: None,
            reason: "test".to_string(),
        }
    }

    #[test]
    fn capture_returns_false_without_begin() {
        // No interception active for this session → caller emits to global bus.
        assert!(!capture("sess-none", intent("Brain")));
        assert!(take("sess-none").is_empty());
    }

    #[test]
    fn begin_capture_take_roundtrip_in_order() {
        let sid = "sess-voice-1";
        begin(sid);
        assert!(capture(sid, intent("Brain")));
        assert!(capture(sid, intent("Automate")));
        let navs = take(sid);
        assert_eq!(navs.len(), 2);
        assert_eq!(navs[0].tab, "Brain");
        assert_eq!(navs[1].tab, "Automate");
        // After take, interception is cleared — further captures fall through.
        assert!(!capture(sid, intent("Build")));
    }

    #[test]
    fn take_is_idempotent() {
        let sid = "sess-voice-2";
        begin(sid);
        assert!(capture(sid, intent("Brain")));
        assert_eq!(take(sid).len(), 1);
        // Second take (e.g. from a Drop guard) returns empty, never panics.
        assert!(take(sid).is_empty());
    }
}
