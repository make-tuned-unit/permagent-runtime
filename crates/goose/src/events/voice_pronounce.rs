//! Display helpers for words speech does not know.
//!
//! This module used to hold the listen-once pronunciation TEACH flow: an
//! unknown word parked the user's request, the server spoke a canned ask, and
//! the next utterance was taken as the pronunciation. On 2026-10-14 that flow
//! fired on the user's own STT mishears ("Upneen", "Tabouli"), hijacked four
//! of five answers and told a World user to look at a surface they could not
//! see (audit V-01, V-03, V-09). Jesse's decision D5 removed live teaching:
//! unknown words Henry says are substituted silently by the TTS layer and
//! queued for review in Settings (`goose-server` `voice::oov_log`,
//! `GET /voice/unknown-words`). What remains here is how such a word is
//! shown on a screen.

/// Title-case for a review list. Speech never reads this; a screen does.
pub fn display_word(word: &str) -> String {
    let w = word.trim();
    if w.is_empty() {
        return String::new();
    }
    let mut chars = w.chars();
    match chars.next() {
        Some(first) => {
            let mut out = first.to_uppercase().collect::<String>();
            out.push_str(&chars.as_str().to_lowercase());
            out
        }
        None => String::new(),
    }
}

/// Prefer the user's own capitalisation from the sentence they just said.
pub fn display_form(source: &str, word: &str) -> String {
    let target = word.trim().to_ascii_lowercase();
    if target.is_empty() {
        return String::new();
    }
    for raw in source.split(|c: char| !c.is_ascii_alphabetic() && c != '-') {
        if raw.eq_ignore_ascii_case(&target) {
            return raw.to_string();
        }
    }
    display_word(&target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_prefers_the_written_capitalisation() {
        assert_eq!(
            display_form("Ask Tavily for the tide data.", "tavily"),
            "Tavily"
        );
        assert_eq!(display_word("taran"), "Taran");
        assert_eq!(display_word(""), "");
    }

    /// D5: the live teach flow is gone. Its copy pointed every surface at "the
    /// Orb" (V-03) and its registry parked mishears (V-01); neither may come
    /// back through this module.
    #[test]
    fn no_live_teach_machinery_remains() {
        let src = include_str!("voice_pronounce.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        for gone in ["ASK_FIRST", "ASK_AGAIN", "fn begin", "REGISTRY", "Orb"] {
            assert!(!code.contains(gone), "{gone} came back");
        }
    }
}
