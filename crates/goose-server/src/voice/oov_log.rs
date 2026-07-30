//! Words speech could not pronounce — the review queue.
//!
//! Without its espeak fallback, misaki spells any unknown word letter by letter.
//! The compound splitter resolves most of them, but whatever survives WILL be
//! spelled out, and until now the only way to find out which words those were
//! was to hear it happen — in a demo. This records them at synthesis time so the
//! list can be reviewed and taught deliberately, before it matters.
//!
//! In-memory and bounded: this is a worklist, not an audit trail. It costs
//! nothing when empty and never grows without limit.

use std::collections::HashMap;
use std::sync::{PoisonError, RwLock};

/// Cap on distinct tracked words. Far above any realistic session vocabulary;
/// exists so a pathological input stream cannot grow this without bound.
const MAX_WORDS: usize = 500;

/// word → how many times synthesis had to spell it out.
static SEEN: RwLock<Option<HashMap<String, u32>>> = RwLock::new(None);

/// Record unresolved words from one sentence. Cheap and lock-light: the common
/// case (nothing unresolved) never takes the write lock.
///
/// Poisoning is recovered rather than propagated — this is advisory telemetry,
/// and a panic elsewhere must not turn every later synthesis call into a panic
/// (the failure mode that fed the daemon's panic breaker in bug-sweep wave 1).
pub fn record(words: &[String]) {
    if words.is_empty() {
        return;
    }
    let mut guard = SEEN.write().unwrap_or_else(PoisonError::into_inner);
    let map = guard.get_or_insert_with(HashMap::new);
    for word in words {
        if map.len() >= MAX_WORDS && !map.contains_key(word) {
            continue;
        }
        *map.entry(word.clone()).or_insert(0) += 1;
    }
}

/// Every unresolved word with its count, most frequent first — the order to
/// teach them in.
pub fn snapshot() -> Vec<(String, u32)> {
    let guard = SEEN.read().unwrap_or_else(PoisonError::into_inner);
    let mut items: Vec<(String, u32)> = guard
        .as_ref()
        .map(|m| m.iter().map(|(w, c)| (w.clone(), *c)).collect())
        .unwrap_or_default();
    items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    items
}

/// Drop a word once it has been taught, so the queue reflects what is still
/// outstanding.
pub fn forget(word: &str) {
    let lowered = word.trim().to_lowercase();
    let mut guard = SEEN.write().unwrap_or_else(PoisonError::into_inner);
    if let Some(map) = guard.as_mut() {
        map.remove(&lowered);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn reset() {
        *SEEN.write().unwrap_or_else(PoisonError::into_inner) = None;
    }

    #[test]
    #[serial]
    fn counts_repeats_and_orders_by_frequency() {
        reset();
        record(&["proptech".into(), "kuzu".into()]);
        record(&["proptech".into()]);
        let snap = snapshot();
        assert_eq!(snap.first().unwrap(), &("proptech".to_string(), 2));
        assert!(snap.contains(&("kuzu".to_string(), 1)));
    }

    #[test]
    #[serial]
    fn empty_input_is_free() {
        reset();
        record(&[]);
        assert!(snapshot().is_empty());
    }

    #[test]
    #[serial]
    fn teaching_a_word_clears_it_from_the_queue() {
        reset();
        record(&["proptech".into()]);
        forget("ProptTech".trim()); // case-insensitive, but a different word
        assert_eq!(snapshot().len(), 1, "an unrelated word must not clear it");
        forget("PROPTECH");
        assert!(snapshot().is_empty(), "the taught word is cleared");
    }

    #[test]
    #[serial]
    fn bounded_so_a_pathological_stream_cannot_grow_it_forever() {
        reset();
        let many: Vec<String> = (0..MAX_WORDS + 50).map(|i| format!("w{i}")).collect();
        record(&many);
        assert_eq!(snapshot().len(), MAX_WORDS);
        // Existing words still count up after the cap is reached.
        record(&["w0".into()]);
        assert_eq!(snapshot().first().unwrap().1, 2);
    }
}
