//! Unknown words Henry had to say — the Settings review queue.
//!
//! Decision D5 (Jesse, 2026-10-14): no live pronunciation teaching. When a
//! reply contains a word speech does not know, the TTS layer substitutes a
//! best-guess reading (`oov_fallback`) and the sentence is spoken anyway. The
//! word lands here so the user can review it later in Settings and, if they
//! want, teach it deliberately through `PUT /voice/pronunciations`.
//!
//! What changed from the old in-memory list (audit V-01 / V-02):
//! * It is NEVER fed from the user's transcript. Mishears ("Upneen",
//!   "Tabouli") used to be recorded here and then injected into every voice
//!   prompt on every device. Only synthesis-time OOV from Henry's own speech
//!   is recorded, and only inside a [`ReplyScope`]. A word that also appears
//!   in the user's transcript for that turn is skipped: it came from STT.
//! * It never reaches a prompt. There is no coaching block any more.
//! * It persists under the profile (`data/voice_unknown_words.json`), is
//!   bounded ([`MAX_WORDS`], least-recently-seen evicted), deduplicated by
//!   lowercased word, and each entry carries timestamps, a count and the
//!   sentence it was heard in.
//!
//! Poisoning is recovered rather than propagated: this is advisory state, and
//! a panic elsewhere must not turn every later synthesis call into a panic.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError, RwLock};

use serde::{Deserialize, Serialize};

/// Cap on distinct queued words. A review list, not an audit trail.
pub const MAX_WORDS: usize = 200;

/// Longest source sentence kept per entry (chars).
pub const MAX_SENTENCE_CHARS: usize = 240;

/// One word awaiting review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownWord {
    /// Lowercased key — what `DELETE /voice/unknown-words/{word}` takes.
    pub word: String,
    /// The word as Henry wrote it in the sentence ("Tavily").
    pub display: String,
    /// Times speech had to substitute it.
    pub count: u32,
    /// Unix milliseconds.
    pub first_seen_ms: i64,
    /// Unix milliseconds.
    pub last_seen_ms: i64,
    /// The most recent sentence it was spoken in (truncated).
    pub sentence: String,
    /// The best-guess reading speech used instead, when there was one.
    #[serde(default)]
    pub substitute_ipa: Option<String>,
}

/// The queue itself. Pure (no I/O) so its rules are testable.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownWordQueue {
    pub words: Vec<UnknownWord>,
}

/// Is this token worth putting in front of a person? Contractions are a
/// folding bug (`fold_apostrophes`), not a word to teach; one- and two-letter
/// fragments and digit junk are noise.
pub fn is_reviewable(word: &str) -> bool {
    let w = word.trim();
    if w.chars().filter(|c| c.is_alphabetic()).count() < 3 {
        return false;
    }
    if !w
        .chars()
        .all(|c| c.is_alphabetic() || c == '-' || c == '\'')
    {
        return false;
    }
    !crate::voice::speech_normalize::looks_like_contraction(w)
}

fn truncate_chars(s: &str, max: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

impl UnknownWordQueue {
    /// Record one substituted word. Returns true when the queue changed.
    pub fn record(
        &mut self,
        word: &str,
        display: &str,
        sentence: &str,
        substitute_ipa: Option<&str>,
        now_ms: i64,
    ) -> bool {
        let key = word.trim().to_lowercase();
        if !is_reviewable(&key) {
            return false;
        }
        let sentence = truncate_chars(sentence, MAX_SENTENCE_CHARS);
        if let Some(entry) = self.words.iter_mut().find(|e| e.word == key) {
            entry.count = entry.count.saturating_add(1);
            entry.last_seen_ms = now_ms;
            entry.sentence = sentence;
            if substitute_ipa.is_some() {
                entry.substitute_ipa = substitute_ipa.map(str::to_string);
            }
            return true;
        }
        if self.words.len() >= MAX_WORDS {
            // Evict the least recently seen — a stale word nobody hears any
            // more is the cheapest one to lose.
            if let Some((idx, _)) = self
                .words
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.last_seen_ms)
            {
                self.words.remove(idx);
            }
        }
        let display = if display.trim().is_empty() {
            key.clone()
        } else {
            display.trim().to_string()
        };
        self.words.push(UnknownWord {
            word: key,
            display,
            count: 1,
            first_seen_ms: now_ms,
            last_seen_ms: now_ms,
            sentence,
            substitute_ipa: substitute_ipa.map(str::to_string),
        });
        true
    }

    /// Remove one word (case-insensitive). Returns true when it was queued.
    pub fn remove(&mut self, word: &str) -> bool {
        let key = word.trim().to_lowercase();
        let before = self.words.len();
        self.words.retain(|e| e.word != key);
        self.words.len() != before
    }

    /// Most recently heard first — the order a review list reads in.
    pub fn sorted(&self) -> Vec<UnknownWord> {
        let mut items = self.words.clone();
        items.sort_by(|a, b| {
            b.last_seen_ms
                .cmp(&a.last_seen_ms)
                .then_with(|| a.word.cmp(&b.word))
        });
        items
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Self>(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let raw = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        // Write-then-rename so a crash mid-write cannot leave half a file.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, raw).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }
}

// ── Process-wide store (one profile per daemon) ─────────────────────────────

static CACHE: RwLock<Option<UnknownWordQueue>> = RwLock::new(None);
/// Serialises disk writes; never held together with a CACHE guard across I/O.
static PERSIST_LOCK: Mutex<()> = Mutex::new(());

fn store_path() -> PathBuf {
    permagent::config::paths::Paths::in_state_dir("data").join("voice_unknown_words.json")
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn with_queue_mut<R>(f: impl FnOnce(&mut UnknownWordQueue) -> R) -> R {
    let mut guard = CACHE.write().unwrap_or_else(PoisonError::into_inner);
    let queue = guard.get_or_insert_with(|| UnknownWordQueue::load_from(&store_path()));
    f(queue)
}

fn persist_current() {
    let _persist = PERSIST_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let snapshot = {
        let guard = CACHE.read().unwrap_or_else(PoisonError::into_inner);
        guard.clone().unwrap_or_default()
    };
    if let Err(error) = snapshot.save_to(&store_path()) {
        tracing::warn!(
            target: "permagentd::voice",
            event = "voice_unknown_word_persist_failed",
            error = %error,
            "unknown-word review queue could not be saved"
        );
    }
}

/// Every queued word, most recently heard first.
pub fn snapshot() -> Vec<UnknownWord> {
    with_queue_mut(|q| q.sorted())
}

/// Drop a word from the review queue (reviewed, dismissed or taught).
pub fn forget(word: &str) -> bool {
    let removed = with_queue_mut(|q| q.remove(word));
    if removed {
        persist_current();
    }
    removed
}

// ── Reply scope: who may record, and what is off limits ─────────────────────

/// Context for one stretch of Henry's speech. Only synthesis that runs inside
/// a scope records unknown words, so a Settings voice preview or a test
/// never writes to the queue, and the user's own transcript words for the
/// turn are excluded (they came from STT and are probably mishears).
#[derive(Debug, Clone, Default)]
pub struct ReplyScope {
    pub turn_id: Option<u64>,
    pub session_id: Option<String>,
    /// `reply`, `canned` …: which path is speaking, for the logs (V-37).
    pub source: &'static str,
    /// Lowercased words of the user's transcript for this turn.
    pub transcript_words: HashSet<String>,
}

impl ReplyScope {
    pub fn new(
        turn_id: Option<u64>,
        session_id: Option<&str>,
        source: &'static str,
        transcript: &str,
    ) -> Self {
        Self {
            turn_id,
            session_id: session_id.map(str::to_string),
            source,
            transcript_words: words_of(transcript),
        }
    }
}

fn words_of(text: &str) -> HashSet<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '-'))
        .flat_map(|t| t.split('-').chain(std::iter::once(t)))
        .map(|t| {
            t.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|t| !t.is_empty())
        .collect()
}

thread_local! {
    static SCOPE: RefCell<Option<ReplyScope>> = const { RefCell::new(None) };
}

/// Run `f` (a synchronous synthesis call) with `scope` active on this thread.
/// The previous scope is restored afterwards, even on panic unwind.
pub fn with_scope<R>(scope: ReplyScope, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<ReplyScope>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let prev = self.0.take();
            SCOPE.with(|s| *s.borrow_mut() = prev);
        }
    }
    let prev = SCOPE.with(|s| s.borrow_mut().replace(scope));
    let _restore = Restore(prev);
    f()
}

/// The active scope's log fields, if any: (turn_id, session_id, source).
#[cfg(test)]
pub fn current_scope_fields() -> Option<(Option<u64>, Option<String>, &'static str)> {
    SCOPE.with(|s| {
        s.borrow()
            .as_ref()
            .map(|sc| (sc.turn_id, sc.session_id.clone(), sc.source))
    })
}

/// Stable short hash of a word for logs: greppable across restarts without
/// writing the word itself into every log line (V-37).
pub fn word_hash(word: &str) -> String {
    // FNV-1a 64 — stable across Rust versions, unlike DefaultHasher.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in word.trim().to_lowercase().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", hash >> 32)
}

/// One word speech substituted while synthesizing `sentence`.
pub struct Substitution<'a> {
    pub word: &'a str,
    pub display: &'a str,
    pub substitute_ipa: Option<&'a str>,
}

/// Pure filter: which substitutions may enter the queue under `scope`.
/// No scope → none. A word from the user's transcript → never.
pub fn queueable<'a>(
    scope: Option<&ReplyScope>,
    subs: &'a [Substitution<'a>],
) -> Vec<&'a Substitution<'a>> {
    let Some(scope) = scope else {
        return Vec::new();
    };
    subs.iter()
        .filter(|s| {
            !scope
                .transcript_words
                .contains(&s.word.trim().to_lowercase())
        })
        .filter(|s| is_reviewable(s.word))
        .collect()
}

/// Called by the TTS backend after it substituted unknown words in one
/// sentence. Logs every substitution; queues the ones [`queueable`] admits.
pub fn record_substitutions(sentence: &str, subs: &[Substitution<'_>]) {
    if subs.is_empty() {
        return;
    }
    let scope = SCOPE.with(|s| s.borrow().clone());
    for sub in subs {
        let from_transcript = scope.as_ref().is_some_and(|sc| {
            sc.transcript_words
                .contains(&sub.word.trim().to_lowercase())
        });
        tracing::info!(
            target: "permagentd::voice",
            event = "voice_oov_substituted",
            turn_id = scope.as_ref().and_then(|s| s.turn_id),
            session_id = scope
                .as_ref()
                .and_then(|s| s.session_id.as_deref())
                .unwrap_or("none"),
            source = scope.as_ref().map(|s| s.source).unwrap_or("unscoped"),
            word_hash = %word_hash(sub.word),
            substitution = if sub.substitute_ipa.is_some() { "phonetic_fallback" } else { "dropped_word" },
            queued = scope.is_some() && !from_transcript && is_reviewable(sub.word),
            skipped_reason = if scope.is_none() {
                "no_reply_scope"
            } else if from_transcript {
                "from_user_transcript"
            } else if !is_reviewable(sub.word) {
                "not_reviewable"
            } else {
                "none"
            },
            "unknown word substituted in speech"
        );
    }
    let admitted = queueable(scope.as_ref(), subs);
    if admitted.is_empty() {
        return;
    }
    let now = now_ms();
    let changed = with_queue_mut(|q| {
        let mut changed = false;
        for sub in &admitted {
            changed |= q.record(sub.word, sub.display, sentence, sub.substitute_ipa, now);
        }
        changed
    });
    if changed {
        persist_current();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupes_counts_and_keeps_latest_sentence() {
        let mut q = UnknownWordQueue::default();
        assert!(q.record("Tavily", "Tavily", "Try Tavily.", Some("tˈævɪli"), 10));
        assert!(q.record("tavily", "tavily", "Tavily again.", None, 20));
        assert_eq!(q.words.len(), 1);
        let e = &q.words[0];
        assert_eq!(e.word, "tavily");
        assert_eq!(e.display, "Tavily");
        assert_eq!(e.count, 2);
        assert_eq!(e.first_seen_ms, 10);
        assert_eq!(e.last_seen_ms, 20);
        assert_eq!(e.sentence, "Tavily again.");
        assert_eq!(e.substitute_ipa.as_deref(), Some("tˈævɪli"));
    }

    #[test]
    fn bounded_and_evicts_the_least_recently_seen() {
        let mut q = UnknownWordQueue::default();
        for i in 0..MAX_WORDS {
            let w = format!("word{}{}", "x".repeat(i % 7 + 1), "a".repeat(i / 7 + 1));
            q.record(&w, &w, "s", None, i as i64 + 100);
        }
        assert_eq!(q.words.len(), MAX_WORDS);
        let oldest = q
            .words
            .iter()
            .min_by_key(|e| e.last_seen_ms)
            .unwrap()
            .word
            .clone();
        q.record("brandnewword", "brandnewword", "s", None, 10_000);
        assert_eq!(q.words.len(), MAX_WORDS);
        assert!(!q.words.iter().any(|e| e.word == oldest), "oldest evicted");
        assert!(q.words.iter().any(|e| e.word == "brandnewword"));
    }

    #[test]
    fn long_sentences_are_truncated() {
        let mut q = UnknownWordQueue::default();
        q.record("zorvane", "Zorvane", &"a ".repeat(400), None, 1);
        assert!(q.words[0].sentence.chars().count() <= MAX_SENTENCE_CHARS);
    }

    #[test]
    fn contractions_fragments_and_junk_are_not_reviewable() {
        assert!(!is_reviewable("shouldn't"));
        assert!(!is_reviewable("I\u{2019}m"));
        assert!(!is_reviewable("ab"));
        assert!(!is_reviewable("gpt4o"));
        assert!(is_reviewable("tavily"));
        assert!(is_reviewable("upneen"));
    }

    #[test]
    fn remove_is_case_insensitive_and_reports_presence() {
        let mut q = UnknownWordQueue::default();
        q.record("kaplin", "Kaplin", "s", None, 1);
        assert!(!q.remove("kapli"));
        assert!(q.remove("KAPLIN"));
        assert!(q.words.is_empty());
    }

    #[test]
    fn sorted_is_most_recent_first() {
        let mut q = UnknownWordQueue::default();
        q.record("alpha", "alpha", "s", None, 1);
        q.record("bravo", "bravo", "s", None, 3);
        q.record("charlie", "charlie", "s", None, 2);
        let order: Vec<_> = q.sorted().into_iter().map(|e| e.word).collect();
        assert_eq!(order, vec!["bravo", "charlie", "alpha"]);
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data").join("voice_unknown_words.json");
        let mut q = UnknownWordQueue::default();
        q.record("tavily", "Tavily", "Try Tavily.", Some("x"), 5);
        q.save_to(&path).unwrap();
        assert_eq!(UnknownWordQueue::load_from(&path), q);
        // Missing or corrupt file → empty queue, never a panic.
        std::fs::write(&path, "{not json").unwrap();
        assert!(UnknownWordQueue::load_from(&path).words.is_empty());
        assert!(UnknownWordQueue::load_from(&dir.path().join("nope.json"))
            .words
            .is_empty());
    }

    /// V-01/V-02: a mishear from the user's transcript never enters the queue,
    /// even when Henry echoes it; nothing is queued outside a reply scope.
    #[test]
    fn transcript_words_and_unscoped_synthesis_never_queue() {
        let subs = [
            Substitution {
                word: "upneen",
                display: "Upneen",
                substitute_ipa: Some("ˈʌpniːn"),
            },
            Substitution {
                word: "tavily",
                display: "Tavily",
                substitute_ipa: Some("x"),
            },
        ];
        assert!(queueable(None, &subs).is_empty(), "no scope, no queue");
        let scope = ReplyScope::new(Some(4), Some("s1"), "reply", "Upneen in the world today?");
        let admitted: Vec<_> = queueable(Some(&scope), &subs)
            .into_iter()
            .map(|s| s.word)
            .collect();
        assert_eq!(admitted, vec!["tavily"]);
    }

    #[test]
    fn scope_is_thread_local_and_restored() {
        assert!(current_scope_fields().is_none());
        let inner = with_scope(ReplyScope::new(Some(7), Some("s"), "reply", ""), || {
            let nested = with_scope(ReplyScope::new(Some(8), None, "canned", ""), || {
                current_scope_fields().map(|f| f.0)
            });
            (nested, current_scope_fields().map(|f| f.0))
        });
        assert_eq!(inner, (Some(Some(8)), Some(Some(7))));
        assert!(current_scope_fields().is_none(), "scope must not leak");
        let other = std::thread::spawn(|| {
            with_scope(ReplyScope::new(Some(1), None, "reply", ""), || ());
            current_scope_fields().is_none()
        })
        .join()
        .unwrap();
        assert!(other);
    }

    #[test]
    fn word_hash_is_stable_and_case_insensitive() {
        assert_eq!(word_hash("Tavily"), word_hash("tavily "));
        assert_ne!(word_hash("tavily"), word_hash("upneen"));
        assert_eq!(word_hash("tavily").len(), 8);
    }

    /// The old module injected "The speech engine does not know these words …
    /// The Orb will show the word" into every voice prompt. Nothing in this
    /// module may produce prompt text any more.
    #[test]
    fn no_prompt_coaching_survives() {
        let src = include_str!("oov_log.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        assert!(!code.contains("fn coaching_prompt"));
        assert!(!code.contains("does not know these words"));
    }
}
