//! Listen-once pronunciation teach — stop, ask, hear, save forever.
//!
//! Last night (2026-08-21 iPhone `20260821_14`) the model kept guessing
//! Elspeth / Taran / Barty, failed the save, and spelled them. Coaching was
//! advice. This registry is the turn intercept: an unknown name parks the
//! request, the next utterance is the pronunciation, and a successful save
//! is durable (`user_lexicon`). One listen. Then it is known.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingTeach {
    /// Surface form to store (lowercased on begin).
    pub word: String,
    /// The user turn we held so we can resume after the save.
    pub held_transcript: Option<String>,
}

static REGISTRY: LazyLock<Mutex<HashMap<String, PendingTeach>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn begin(session_id: &str, word: &str, held_transcript: Option<String>) {
    if session_id.is_empty() {
        return;
    }
    let word = word.trim().to_ascii_lowercase();
    if word.is_empty() {
        return;
    }
    if let Ok(mut reg) = REGISTRY.lock() {
        reg.insert(
            session_id.to_string(),
            PendingTeach {
                word,
                held_transcript: held_transcript.filter(|s| !s.trim().is_empty()),
            },
        );
    }
}

pub fn peek(session_id: &str) -> Option<PendingTeach> {
    REGISTRY
        .lock()
        .ok()
        .and_then(|reg| reg.get(session_id).cloned())
}

pub fn take(session_id: &str) -> Option<PendingTeach> {
    REGISTRY
        .lock()
        .ok()
        .and_then(|mut reg| reg.remove(session_id))
}

pub fn clear(session_id: &str) {
    if let Ok(mut reg) = REGISTRY.lock() {
        reg.remove(session_id);
    }
}

/// Spoken decline — do not save, resume whatever was held.
pub fn is_skip_cue(transcript: &str) -> bool {
    let t = transcript
        .trim()
        .trim_end_matches(['.', '!', ',', '?'])
        .trim()
        .to_ascii_lowercase();
    matches!(
        t.as_str(),
        "skip"
            | "skip it"
            | "skip that"
            | "never mind"
            | "nevermind"
            | "don't worry"
            | "dont worry"
            | "forget it"
            | "forget that"
            | "doesn't matter"
            | "doesnt matter"
            | "just go on"
            | "just continue"
            | "don't bother"
            | "dont bother"
    )
}

/// Words we will actually stop and ask about. Fragments (`peth`, `ayn`) and
/// function words are not names — last night those were the failed syllables,
/// not the thing the user wanted taught.
pub fn is_teachable_word(word: &str) -> bool {
    let w = word
        .trim()
        .trim_matches(|c: char| !c.is_ascii_alphabetic() && c != '-')
        .to_ascii_lowercase();
    if w.len() < 5 {
        return false;
    }
    if !w.chars().all(|c| c.is_ascii_alphabetic() || c == '-') {
        return false;
    }
    // Respell syllables — we ask about the name, not the bits used to teach it.
    if matches!(
        w.as_str(),
        "peth" | "speth" | "prid" | "ells" | "jent" | "spith" | "spyth"
    ) {
        return false;
    }
    !matches!(
        w.as_str(),
        "that"
            | "this"
            | "with"
            | "from"
            | "have"
            | "been"
            | "they"
            | "them"
            | "your"
            | "what"
            | "when"
            | "just"
            | "like"
            | "than"
            | "then"
            | "some"
            | "more"
            | "very"
            | "into"
            | "over"
            | "also"
            | "only"
            | "back"
            | "well"
            | "even"
            | "here"
            | "there"
            | "about"
            | "would"
            | "could"
            | "should"
            | "their"
            | "which"
            | "these"
            | "those"
            | "other"
            | "after"
            | "before"
            | "because"
            | "something"
            | "anything"
    )
}

/// Longest teachable token — prefer `elspeth` over `peth` when both appear.
pub fn first_teachable(words: &[String]) -> Option<String> {
    words
        .iter()
        .map(|w| {
            w.trim()
                .trim_matches(|c: char| !c.is_ascii_alphabetic() && c != '-')
                .to_ascii_lowercase()
        })
        .filter(|w| is_teachable_word(w))
        .max_by_key(|w| w.len())
}

/// Pull a respelling out of what the user just said.
///
/// "it's like else peth" → "else peth"
/// "Elspeth" → "Elspeth"
pub fn sounds_like_from_listen(transcript: &str) -> String {
    let raw = transcript.trim();
    if raw.is_empty() {
        return String::new();
    }
    let lower = raw.to_ascii_lowercase();
    const PREFIXES: &[&str] = &[
        "it sounds like ",
        "sounds like ",
        "it's pronounced ",
        "its pronounced ",
        "pronounced ",
        "it's like ",
        "its like ",
        "say ",
        "like ",
        "it's ",
        "its ",
    ];
    for prefix in PREFIXES {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let start = raw.len().saturating_sub(rest.len());
            if let Some(slice) = raw.get(start..) {
                return tidy_listen(slice);
            }
        }
    }
    tidy_listen(raw)
}

fn tidy_listen(s: &str) -> String {
    s.trim()
        .trim_end_matches(['.', '!', ',', '?'])
        .trim()
        .to_string()
}

/// Candidate respellings to try, in order, from one listen.
///
/// A listen is a word or a short sounds-like (`else peth`, `pig keeper`).
/// A complaint sentence must not become `sounds_like` — 2026-08-27 kitchen
/// wrote `pinkiepper` → `You can't say the word "pig keeper"`.
pub fn save_candidates(word: &str, transcript: &str) -> Vec<String> {
    let mut out = Vec::new();
    let heard = sounds_like_from_listen(transcript);
    if is_usable_respelling(&heard) {
        out.push(heard);
    }
    let raw = transcript
        .trim()
        .trim_end_matches(['.', '!', ',', '?'])
        .trim();
    if is_usable_respelling(raw) && !out.iter().any(|s| s.eq_ignore_ascii_case(raw)) {
        out.push(raw.to_string());
    }
    // Only fall back to the pending word when the listen was actually a
    // short respelling. A sentence must ASK_AGAIN, not save STT garbage as
    // its own spelling.
    let word = word.trim();
    if !word.is_empty() && !out.is_empty() && !out.iter().any(|s| s.eq_ignore_ascii_case(word)) {
        out.push(word.to_string());
    }
    out
}

fn is_usable_respelling(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    let lower = s.to_ascii_lowercase();
    if lower.contains("can't say")
        || lower.contains("cant say")
        || lower.contains("you can't")
        || lower.contains("you cant")
    {
        return false;
    }
    s.split_whitespace().count() <= 4
}

/// Spoken when a word is placed on the Orb. Never names or spells the word —
/// the Orb is the only place the user should see it.
pub const ASK_FIRST: &str = "I'm not recognizing this next word. I'll place it on the Orb \
    and listen for your pronunciation, to store it in my memory.";
pub const ASK_AGAIN: &str = "It's still on the Orb. Say it once more so I can store it.";
pub const SKIPPED: &str = "Okay, I'll leave it.";

/// Title-case for the Orb. Speech never reads this; the screen does.
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

pub fn saved_confirmation(word: &str) -> String {
    format!(
        "{}. Got it — I'll say it that way from now on.",
        display_word(word)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_night_names_are_teachable_fragments_are_not() {
        assert!(is_teachable_word("Elspeth"));
        assert!(is_teachable_word("Taran"));
        assert!(is_teachable_word("Barty"));
        assert!(is_teachable_word("Prideine"));
        assert!(!is_teachable_word("peth"));
        assert!(!is_teachable_word("ayn"));
        assert!(!is_teachable_word("the"));
        assert!(!is_teachable_word("jent"));
    }

    #[test]
    fn prefers_the_full_name_over_a_syllable() {
        let words = vec!["peth".into(), "elspeth".into(), "speth".into()];
        assert_eq!(first_teachable(&words).as_deref(), Some("elspeth"));
    }

    #[test]
    fn listen_extracts_a_respelling() {
        assert_eq!(sounds_like_from_listen("else peth"), "else peth");
        assert_eq!(sounds_like_from_listen("It's like else peth."), "else peth");
        assert_eq!(sounds_like_from_listen("sounds like tear un"), "tear un");
        assert_eq!(sounds_like_from_listen("Elspeth."), "Elspeth");
    }

    #[test]
    fn save_candidates_try_heard_then_the_word() {
        let c = save_candidates("Elspeth", "it's like else peth");
        assert_eq!(c[0], "else peth");
        assert!(c.iter().any(|s| s.eq_ignore_ascii_case("Elspeth")));
    }

    #[test]
    fn two_word_respelling_of_pigkeeper_still_saves() {
        let c = save_candidates("pigkeeper", "pig keeper");
        assert!(c.iter().any(|s| s.eq_ignore_ascii_case("pig keeper")));
    }

    #[test]
    fn skip_cues() {
        assert!(is_skip_cue("Never mind."));
        assert!(is_skip_cue("skip it"));
        assert!(!is_skip_cue("never mind the troll, say Elspeth"));
    }

    #[test]
    fn ask_never_names_or_spells_the_word() {
        for line in [ASK_FIRST, ASK_AGAIN] {
            let lower = line.to_ascii_lowercase();
            assert!(
                !lower.contains("elspeth") && !lower.contains("spell") && !lower.contains("letter"),
                "ask must not name or spell the word: {line}"
            );
            assert!(
                lower.contains("orb"),
                "ask must point at the Orb, not the speaker: {line}"
            );
        }
    }

    #[test]
    fn orb_shows_the_users_capitalisation() {
        assert_eq!(
            display_form("Tell Rowan about Princess Elspeth.", "elspeth"),
            "Elspeth"
        );
        assert_eq!(display_word("taran"), "Taran");
    }

    /// 2026-08-27 kitchen: STT heard `pinkiepper`, the user said
    /// `You can't say the word "pig keeper"`, and `save_candidates` kept the
    /// whole complaint as a respelling. A listen is a word or a short
    /// sounds-like, not a sentence. Do not "fix" pronunciations.json here —
    /// this tripwire must catch the save path.
    #[test]
    fn kitchen_complaint_is_not_a_respelling_of_pinkiepper() {
        let c = save_candidates("pinkiepper", r#"You can't say the word "pig keeper""#);
        assert!(
            c.iter().all(|s| {
                let lower = s.to_ascii_lowercase();
                !lower.contains("can't say") && !lower.contains("cant say")
            }),
            "complaint leaked into save candidates: {c:?}"
        );
        assert!(
            c.iter().all(|s| s.split_whitespace().count() <= 3),
            "a full sentence is not a respelling: {c:?}"
        );
    }

    #[test]
    fn begin_take_is_one_listen() {
        let sid = "voice-pronounce-1";
        clear(sid);
        begin(sid, "Elspeth", Some("Tell Rowan the story.".into()));
        let p = take(sid).expect("pending");
        assert_eq!(p.word, "elspeth");
        assert_eq!(p.held_transcript.as_deref(), Some("Tell Rowan the story."));
        assert!(take(sid).is_none(), "second take must be empty");
        clear(sid);
    }
}
