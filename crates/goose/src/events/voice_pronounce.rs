//! Listen-once pronunciation teach — stop, ask, hear, save forever.
//!
//! Last night (2026-08-21 iPhone `20260821_14`) the model kept guessing
//! Elspeth / Taran / Barty, failed the save, and spelled them. Coaching was
//! advice. This registry is the turn intercept: an unknown name parks the
//! request, the next utterance is the pronunciation, and a successful save
//! is durable (`user_lexicon`). One listen. Then it is known.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingTeach {
    /// Surface form to store (lowercased on begin).
    pub word: String,
    /// The user turn we held so we can resume after the save.
    pub held_transcript: Option<String>,
    /// How many ASK_AGAIN round trips this word has already used. Caps at
    /// [`MAX_TEACH_ATTEMPTS`] so a never-savable transcript can't loop
    /// forever — see `next_retry`.
    pub attempts: u32,
    /// When this word was first parked. A registry entry that outlives
    /// [`MAX_PENDING_AGE`] is forgotten on next touch (2026-09-01: a
    /// same-process daemon that never restarts must still self-heal a
    /// parked word from a dead session).
    pub started_at: Instant,
}

/// Attempts on one taught word before we give up and resume whatever was
/// held. Mirrors the client's own zero-sample-listen cap (useVoice.ts) so
/// both loops share the same bound.
pub const MAX_TEACH_ATTEMPTS: u32 = 3;

/// How long a parked word may sit un-answered before it's treated as
/// abandoned. The registry is a process-lifetime static (a daemon restart
/// already clears it for free); this bounds the same-process case — a
/// session that vanished mid-drill (crash, forgotten tab) without ever
/// closing its socket cleanly.
pub const MAX_PENDING_AGE: Duration = Duration::from_secs(10 * 60);

static REGISTRY: LazyLock<Mutex<HashMap<String, PendingTeach>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Pure: has a pending teach aged `age` outlived `max_age`? Extracted so the
/// expiry judgement is testable without touching the registry or sleeping.
pub fn is_expired(age: Duration, max_age: Duration) -> bool {
    age > max_age
}

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
                attempts: 0,
                started_at: Instant::now(),
            },
        );
    }
}

pub fn peek(session_id: &str) -> Option<PendingTeach> {
    let mut reg = REGISTRY.lock().ok()?;
    let pending = reg.get(session_id)?.clone();
    if is_expired(pending.started_at.elapsed(), MAX_PENDING_AGE) {
        reg.remove(session_id);
        return None;
    }
    Some(pending)
}

pub fn take(session_id: &str) -> Option<PendingTeach> {
    let mut reg = REGISTRY.lock().ok()?;
    let pending = reg.remove(session_id)?;
    if is_expired(pending.started_at.elapsed(), MAX_PENDING_AGE) {
        return None;
    }
    Some(pending)
}

pub fn clear(session_id: &str) {
    if let Ok(mut reg) = REGISTRY.lock() {
        reg.remove(session_id);
    }
}

/// Outcome of one failed listen (STT ran but nothing savable came out of it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryOutcome {
    /// Attempts remain — ask again. The registry now holds this bumped copy.
    AskAgain(PendingTeach),
    /// Attempts exhausted — NOT re-parked. Caller should say SKIPPED and
    /// resume whatever was held, exactly like a spoken "never mind".
    GiveUp(PendingTeach),
}

/// Pure: decide AskAgain vs GiveUp from an attempt count alone, so the cap is
/// testable without a registry, a socket, or a TTS.
fn next_retry(pending: PendingTeach) -> RetryOutcome {
    let attempts = pending.attempts + 1;
    let bumped = PendingTeach {
        attempts,
        ..pending
    };
    if attempts >= MAX_TEACH_ATTEMPTS {
        RetryOutcome::GiveUp(bumped)
    } else {
        RetryOutcome::AskAgain(bumped)
    }
}

/// Record one failed listen against `session_id`'s pending word. On
/// `AskAgain` the bumped entry is re-parked (same word, same held transcript,
/// fresh `started_at` so the retry window doesn't inherit the original
/// park's age); on `GiveUp` nothing is re-parked — the caller owns SKIPPED +
/// resume, mirroring `is_skip_cue`.
pub fn record_retry(session_id: &str, pending: PendingTeach) -> RetryOutcome {
    let outcome = next_retry(pending);
    match &outcome {
        RetryOutcome::AskAgain(p) => {
            if let Ok(mut reg) = REGISTRY.lock() {
                reg.insert(
                    session_id.to_string(),
                    PendingTeach {
                        started_at: Instant::now(),
                        ..p.clone()
                    },
                );
            }
        }
        RetryOutcome::GiveUp(_) => clear(session_id),
    }
    outcome
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
        assert_eq!(
            p.attempts, 0,
            "a freshly begun teach starts at zero attempts"
        );
        assert!(take(sid).is_none(), "second take must be empty");
        clear(sid);
    }

    /// FIX 3b (2026-09-01 incident): handle_pronunciation_listen re-began
    /// ASK_AGAIN with no attempt cap when try_save_heard failed — an
    /// un-savable transcript looped forever. This is the fails-today
    /// predicate: before the cap existed, `next_retry` didn't exist and the
    /// caller just called `begin` again unconditionally, so attempts never
    /// grew and the loop had no exit. Verified against the OLD behavior
    /// directly: a hand-rolled loop that just re-begins on every failure
    /// never terminates in 1000 iterations, below.
    #[test]
    fn retry_gives_up_after_max_attempts() {
        let sid = "voice-pronounce-retry";
        clear(sid);
        begin(sid, "Elspeth", Some("Tell Rowan the story.".into()));
        let mut pending = take(sid).expect("pending");

        let mut asked_again = 0;
        loop {
            match record_retry(sid, pending) {
                RetryOutcome::AskAgain(p) => {
                    asked_again += 1;
                    assert!(
                        asked_again < MAX_TEACH_ATTEMPTS,
                        "AskAgain must not outlast the cap"
                    );
                    pending = take(sid).expect("re-parked on AskAgain");
                    assert_eq!(pending.attempts, p.attempts);
                }
                RetryOutcome::GiveUp(p) => {
                    assert_eq!(p.attempts, MAX_TEACH_ATTEMPTS);
                    assert!(take(sid).is_none(), "GiveUp must not re-park the word");
                    break;
                }
            }
        }
        assert_eq!(asked_again, MAX_TEACH_ATTEMPTS - 1);
        clear(sid);
    }

    /// The literal old predicate: unconditional re-begin, no counter, no
    /// exit. This is what handle_pronunciation_listen's ASK_AGAIN branch did
    /// before FIX 3b — demonstrating it never terminates confirms the bug
    /// `record_retry` was written to close.
    #[test]
    fn old_unconditional_rebegin_never_terminates() {
        let sid = "voice-pronounce-old-behavior";
        clear(sid);
        begin(sid, "Elspeth", None);
        for _ in 0..1000 {
            // The pre-fix code: `begin(session_id, &pending.word, pending.held_transcript)`
            // on every failed listen, with no counter anywhere.
            let pending = peek(sid).expect("still parked — this IS the bug");
            begin(sid, &pending.word, pending.held_transcript);
        }
        assert!(
            peek(sid).is_some(),
            "old behavior: still parked after 1000 failed listens"
        );
        clear(sid);
    }

    #[test]
    fn pending_teach_expires_by_age() {
        // Pure predicate — no sleeping, no registry.
        assert!(!is_expired(Duration::from_secs(1), MAX_PENDING_AGE));
        assert!(!is_expired(MAX_PENDING_AGE, MAX_PENDING_AGE));
        assert!(is_expired(
            MAX_PENDING_AGE + Duration::from_secs(1),
            MAX_PENDING_AGE
        ));
    }

    /// The parked "Teenity" entry from 2026-08-29 (session 20260829_3) is
    /// ~3 days old by 2026-09-01 — far past MAX_PENDING_AGE — so the very
    /// next `peek`/`take` on that session id after this fix ships forgets
    /// it, without needing a daemon restart.
    #[test]
    fn a_multi_day_old_pending_teach_self_heals_without_a_restart() {
        let sid = "voice-pronounce-teenity-repro";
        clear(sid);
        begin(sid, "Teenity", None);
        // Reach into the registry the same way `begin` does, but backdate
        // `started_at` the way a 3-day-old entry would actually be found —
        // via the same struct field the registry stores, not a separate
        // test-only clock.
        if let Ok(mut reg) = REGISTRY.lock() {
            if let Some(p) = reg.get_mut(sid) {
                p.started_at = Instant::now() - Duration::from_secs(3 * 24 * 60 * 60);
            }
        }
        assert!(
            peek(sid).is_none(),
            "a multi-day-old pending teach must be forgotten on next touch"
        );
        clear(sid);
    }
}
