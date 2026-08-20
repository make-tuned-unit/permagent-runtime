//! Per-session clipboard interception — copy on the device that is listening.
//!
//! `copy_to_clipboard` must write the pasteboard where the user will paste
//! (Notes on iPhone, the Mac they are sitting at), not on whichever machine
//! happens to run the daemon. The `/voice` WebSocket already knows that
//! client; this registry lets a voice turn capture the paste-ready body and
//! forward it down that socket, the same way [`super::nav_intercept`] defers
//! `navigate_app`.
//!
//! Text-chat turns never call [`begin`], so [`capture`] returns `false` and
//! the tool emits to the global bus for the Command Center to copy locally.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// Paste-ready body captured during a voice turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardIntent {
    pub text: String,
    pub reason: String,
}

static REGISTRY: LazyLock<Mutex<HashMap<String, Vec<ClipboardIntent>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Begin intercepting clipboard writes for `session_id`. Idempotent; resets
/// any prior buffer for the session.
pub fn begin(session_id: &str) {
    if let Ok(mut reg) = REGISTRY.lock() {
        reg.insert(session_id.to_string(), Vec::new());
    }
}

/// Capture `intent` if interception is active. Returns `true` when captured
/// (caller should NOT emit to the global bus).
pub fn capture(session_id: &str, intent: ClipboardIntent) -> bool {
    if let Ok(mut reg) = REGISTRY.lock() {
        if let Some(buf) = reg.get_mut(session_id) {
            buf.push(intent);
            return true;
        }
    }
    false
}

/// End interception and return captured intents in call order. Safe when
/// interception never started — returns empty.
pub fn take(session_id: &str) -> Vec<ClipboardIntent> {
    REGISTRY
        .lock()
        .ok()
        .and_then(|mut reg| reg.remove(session_id))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent(text: &str) -> ClipboardIntent {
        ClipboardIntent {
            text: text.to_string(),
            reason: "test".to_string(),
        }
    }

    #[test]
    fn capture_returns_false_without_begin() {
        assert!(!capture("clip-none", intent("hello")));
        assert!(take("clip-none").is_empty());
    }

    #[test]
    fn begin_capture_take_roundtrip_last_wins_on_client() {
        let sid = "clip-voice-1";
        begin(sid);
        assert!(capture(sid, intent("draft")));
        assert!(capture(sid, intent("final")));
        let clips = take(sid);
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0].text, "draft");
        assert_eq!(clips[1].text, "final");
        assert!(!capture(sid, intent("late")));
    }

    #[test]
    fn take_is_idempotent() {
        let sid = "clip-voice-2";
        begin(sid);
        assert!(capture(sid, intent("body")));
        assert_eq!(take(sid).len(), 1);
        assert!(take(sid).is_empty());
    }
}
