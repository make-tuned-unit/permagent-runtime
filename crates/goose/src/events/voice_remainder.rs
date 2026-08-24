//! Unspoken voice remainder — what the 8-sentence budget did not say.
//!
//! Last night (2026-08-21 iPhone session `20260821_14`):
//!
//! - `01:30:37` spoken budget hit after eight sentences of the Rowan story
//! - cue: "There's more — say continue and I'll keep going."
//! - `01:31:21` user said `"Continue."` — a *new* agent turn, leftover gone
//! - same again at `01:35:22` / `01:35:54`
//!
//! The leftover sentences were never stored. "Continue" went to the LLM as a
//! bare user message and it lost the thread. This registry keeps the unspoken
//! prose on the session so a continue cue *speaks that text* instead of
//! starting a cold turn.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

static REGISTRY: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Replace the leftover for this session. Empty `text` clears.
pub fn stash(session_id: &str, text: String) {
    if session_id.is_empty() {
        return;
    }
    if let Ok(mut reg) = REGISTRY.lock() {
        if text.trim().is_empty() {
            reg.remove(session_id);
        } else {
            reg.insert(session_id.to_string(), text);
        }
    }
}

/// Take and clear the leftover. `None` when nothing was stashed.
pub fn take(session_id: &str) -> Option<String> {
    REGISTRY
        .lock()
        .ok()
        .and_then(|mut reg| reg.remove(session_id))
        .filter(|s| !s.trim().is_empty())
}

pub fn peek(session_id: &str) -> Option<String> {
    REGISTRY
        .lock()
        .ok()
        .and_then(|reg| reg.get(session_id).cloned())
        .filter(|s| !s.trim().is_empty())
}

pub fn clear(session_id: &str) {
    if let Ok(mut reg) = REGISTRY.lock() {
        reg.remove(session_id);
    }
}

/// Append a dropped sentence onto the leftover buffer (spoken-budget path).
pub fn append_sentence(leftover: &mut String, sentence: &str) {
    let piece = sentence.trim();
    if piece.is_empty() {
        return;
    }
    if !leftover.is_empty() {
        leftover.push(' ');
    }
    leftover.push_str(piece);
}

/// Short continue / keep-going cues from last night and the obvious variants.
///
/// Longer sentences fall through to the agent so "continue the story about
/// Rowan with a new chapter" is never swallowed as a leftover replay.
pub fn is_continue_cue(transcript: &str) -> bool {
    let t = transcript
        .trim()
        .trim_end_matches(['.', '!', ',', '?'])
        .trim()
        .to_ascii_lowercase();
    matches!(
        t.as_str(),
        "continue"
            | "keep going"
            | "go on"
            | "keep talking"
            | "keep on"
            | "please continue"
            | "yes continue"
            | "yeah continue"
            | "yes keep going"
            | "go ahead"
            | "more"
            | "tell me more"
            | "keep reading"
            | "and then"
            | "what next"
            | "next"
            | "yes"
            | "yeah"
            | "yep"
            | "yup"
            | "please"
    )
}

/// True when a server error string is an empty-turn recovery, not a real fault.
///
/// Last night every empty STT (`transcript: ""`) became
/// `"No speech detected — try again"` and flashed on the orb. Those turns
/// must return to ready silently.
pub fn is_transient_empty_turn(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("no speech detected") || m.contains("recording too short")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_night_continue_is_a_cue() {
        // Exact STT from 20260821_14 at 01:31:21 and 01:35:54.
        assert!(is_continue_cue("Continue."));
        assert!(is_continue_cue("continue"));
        assert!(is_continue_cue("  Keep going  "));
        assert!(is_continue_cue("Yes."));
        assert!(is_continue_cue("go on"));
        assert!(is_continue_cue("tell me more"));
    }

    #[test]
    fn real_requests_are_not_cues() {
        assert!(!is_continue_cue(
            "Continue the story about Rowan with a new chapter"
        ));
        assert!(!is_continue_cue("yes, and also tell me about Taran"));
        assert!(!is_continue_cue(
            "Tell me about the story I'm telling Rowan."
        ));
        assert!(!is_continue_cue(""));
    }

    #[test]
    fn stash_take_clears_so_a_second_continue_does_not_replay() {
        let sid = "voice-remainder-1";
        clear(sid);
        stash(
            sid,
            "He pulled the cloak tighter and kept walking toward the ridge.".into(),
        );
        let first = take(sid).expect("stashed");
        assert!(first.contains("cloak"));
        assert!(take(sid).is_none(), "second take must be empty");
        clear(sid);
    }

    #[test]
    fn empty_stash_clears() {
        let sid = "voice-remainder-2";
        stash(sid, " leftover ".into());
        stash(sid, "   ".into());
        assert!(peek(sid).is_none());
        clear(sid);
    }

    #[test]
    fn append_sentence_joins_dropped_budget_sentences() {
        let mut leftover = String::new();
        append_sentence(&mut leftover, "Sentence nine.");
        append_sentence(&mut leftover, " Sentence ten. ");
        assert_eq!(leftover, "Sentence nine. Sentence ten.");
    }

    #[test]
    fn last_night_empty_stt_error_is_transient() {
        assert!(is_transient_empty_turn("No speech detected — try again"));
        assert!(is_transient_empty_turn(
            "Recording too short — hold longer to speak"
        ));
        assert!(!is_transient_empty_turn("STT failed: model missing"));
        assert!(!is_transient_empty_turn("Voice reply failed: timeout"));
    }
}
