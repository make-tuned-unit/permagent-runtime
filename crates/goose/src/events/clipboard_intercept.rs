//! Per-session clipboard interception — copy on the device that is listening.
//!
//! `copy_to_clipboard` must write the pasteboard where the user will paste
//! (Notes on iPhone, the Mac they are sitting at), not on whichever machine
//! happens to run the daemon. The `/voice` WebSocket already knows that
//! client; this registry lets a voice turn capture the paste-ready body and
//! forward it down that socket.
//!
//! Unlike [`super::nav_intercept`] (which waits until narration ends so the
//! view does not switch mid-sentence), clipboard is flushed as soon as the
//! tool returns. The user often switches to Notes the moment they hear that
//! a copy is happening; iOS drops pasteboard writes from a backgrounded
//! app, so deferring until after TTS is how "It's on your clipboard" became
//! a lie.
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

/// Take any captured intents without ending interception. The voice loop
/// calls this as soon as the tool fires so the listening device can copy
/// while still in the foreground. Safe when interception never started —
/// returns empty and leaves no session registered.
pub fn drain(session_id: &str) -> Vec<ClipboardIntent> {
    if let Ok(mut reg) = REGISTRY.lock() {
        if let Some(buf) = reg.get_mut(session_id) {
            return std::mem::take(buf);
        }
    }
    Vec::new()
}

/// End interception and return any remaining intents in call order. Safe
/// when interception never started — returns empty.
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

    #[test]
    fn drain_keeps_interception_open() {
        let sid = "clip-voice-drain";
        begin(sid);
        assert!(capture(sid, intent("first")));
        let first = drain(sid);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].text, "first");
        assert!(drain(sid).is_empty());
        assert!(capture(sid, intent("second")));
        let rest = take(sid);
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].text, "second");
        assert!(!capture(sid, intent("late")));
    }

    #[test]
    fn drain_without_begin_is_empty_and_does_not_register() {
        assert!(drain("clip-never").is_empty());
        assert!(!capture("clip-never", intent("nope")));
    }
}
