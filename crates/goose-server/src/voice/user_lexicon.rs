//! User pronunciation lexicon — the never-spell-it-out rule's memory.
//!
//! The TTS backend has always had a per-call [`PronunciationLexicon`] seam
//! (consulted before its built-in technical lexicon and before G2P), but every
//! synthesis site passed `lexicon: None` — user-taught pronunciations had
//! nowhere to live. This module is that home: a durable JSON store the
//! `save_pronunciation` tool writes and every synthesis call reads, so a word
//! the user pronounces once is said correctly forever.
//!
//! Values carry BOTH the IPA the backend speaks and the human "sounds like"
//! respelling for display/confirmation. Keys are lowercased (the lexicon's
//! matching is case-insensitive).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError, RwLock};

use serde::{Deserialize, Serialize};

use super::provider::PronunciationLexicon;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PronunciationEntry {
    /// Misaki/Kokoro-style IPA the backend renders (e.g. "pˈɜːməʤɛnt").
    pub ipa: String,
    /// Human respelling for display and read-back (e.g. "PER-ma-jent").
    pub sounds_like: String,
}

static CACHE: RwLock<Option<HashMap<String, PronunciationEntry>>> = RwLock::new(None);

/// Serializes disk writes so two concurrent saves cannot land an older
/// snapshot after a newer one. The persisted state is re-read from the cache
/// AFTER acquiring this lock, so the writer that runs last always writes the
/// newest state. Never held together with a CACHE guard across file I/O.
static PERSIST_LOCK: Mutex<()> = Mutex::new(());

fn store_path() -> PathBuf {
    permagent::config::paths::Paths::in_state_dir("data").join("pronunciations.json")
}

fn load_from_disk() -> HashMap<String, PronunciationEntry> {
    let path = store_path();
    match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

// Poisoning recovery: this cache is a plain map mirroring a persisted file —
// a panic in another thread cannot leave it half-mutated in any way that
// matters (worst case the next persist rewrites the file), so a poisoned lock
// is safe to reuse. The old `.expect("lexicon cache poisoned")` turned ONE
// panic anywhere in a critical section into a panic on EVERY synthesis call
// until restart, feeding the daemon's panic breaker (bug-sweep wave 1).

fn with_entries<R>(f: impl FnOnce(&HashMap<String, PronunciationEntry>) -> R) -> R {
    {
        let cache = CACHE.read().unwrap_or_else(PoisonError::into_inner);
        if let Some(entries) = cache.as_ref() {
            return f(entries);
        }
    }
    let loaded = load_from_disk();
    let mut cache = CACHE.write().unwrap_or_else(PoisonError::into_inner);
    let entries = cache.get_or_insert(loaded);
    f(entries)
}

/// All saved pronunciations (for the API surface).
pub fn all() -> HashMap<String, PronunciationEntry> {
    with_entries(Clone::clone)
}

/// The lexicon every synthesis call should carry. Empty lexicon → None so
/// backends skip the override pass entirely.
pub fn current() -> Option<PronunciationLexicon> {
    with_entries(|entries| {
        if entries.is_empty() {
            return None;
        }
        Some(PronunciationLexicon::from_pairs(
            entries.iter().map(|(w, e)| (w.clone(), e.ipa.clone())),
        ))
    })
}

/// Upsert a pronunciation and persist. Returns the total entry count.
pub fn save(word: &str, entry: PronunciationEntry) -> Result<usize, String> {
    let word = word.trim().to_lowercase();
    if word.is_empty() {
        return Err("word must not be empty".to_string());
    }
    if entry.ipa.trim().is_empty() {
        return Err("ipa must not be empty".to_string());
    }
    // Mutate under the write guard, but do the file I/O OUTSIDE it: holding
    // the guard across `persist` meant a slow disk stalled every synthesis
    // call, and a panic during I/O poisoned the lock for the process lifetime.
    let count = {
        let mut cache = CACHE.write().unwrap_or_else(PoisonError::into_inner);
        let entries = cache.get_or_insert_with(load_from_disk);
        entries.insert(word, entry);
        entries.len()
    };
    persist_current()?;
    Ok(count)
}

/// Remove a pronunciation. Returns true when something was removed.
pub fn remove(word: &str) -> Result<bool, String> {
    let word = word.trim().to_lowercase();
    let removed = {
        let mut cache = CACHE.write().unwrap_or_else(PoisonError::into_inner);
        let entries = cache.get_or_insert_with(load_from_disk);
        entries.remove(&word).is_some()
    };
    if removed {
        persist_current()?;
    }
    Ok(removed)
}

/// Persist the CURRENT cache state. Snapshots under a short read guard after
/// taking the persist lock, so writes are serialized and the last write always
/// reflects the newest cache state; no CACHE guard is held during file I/O.
fn persist_current() -> Result<(), String> {
    let _persist_guard = PERSIST_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let snapshot = {
        let cache = CACHE.read().unwrap_or_else(PoisonError::into_inner);
        cache.clone().unwrap_or_default()
    };
    persist(&snapshot)
}

fn persist(entries: &HashMap<String, PronunciationEntry>) -> Result<(), String> {
    let path = store_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let raw = serde_json::to_string_pretty(entries).map_err(|e| e.to_string())?;
    std::fs::write(&path, raw).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// Round-trip through the public API: save → current() carries the IPA,
    /// case-insensitively; remove → gone. (Uses the real store path — entries
    /// are namespaced under a test-only word to avoid clobbering user data.)
    /// Serial: the CACHE static and store file are process-global.
    #[test]
    #[serial]
    fn save_feeds_current_and_remove_clears() {
        let word = "zz-test-pronunciation-zz";
        save(
            word,
            PronunciationEntry {
                ipa: "tˈɛst".to_string(),
                sounds_like: "test".to_string(),
            },
        )
        .unwrap();
        let lex = current().expect("lexicon non-empty after save");
        assert_eq!(lex.get("ZZ-Test-Pronunciation-ZZ"), Some("tˈɛst"));
        assert!(remove(word).unwrap());
    }

    /// Concurrent saves/reads stay consistent: every saved word is visible in
    /// both the cache and the persisted file afterwards (the persist lock
    /// guarantees the last disk write carries the newest cache state).
    #[test]
    #[serial]
    fn concurrent_saves_are_consistent() {
        let words: Vec<String> = (0..8)
            .map(|i| format!("zz-test-concurrent-{}-zz", i))
            .collect();

        let handles: Vec<_> = words
            .iter()
            .cloned()
            .map(|word| {
                std::thread::spawn(move || {
                    save(
                        &word,
                        PronunciationEntry {
                            ipa: "tˈɛst".to_string(),
                            sounds_like: "test".to_string(),
                        },
                    )
                    .unwrap();
                    // Interleave reads with writes.
                    let _ = current();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        let entries = all();
        for word in &words {
            assert!(entries.contains_key(word), "cache must contain {}", word);
        }
        let on_disk = load_from_disk();
        for word in &words {
            assert!(on_disk.contains_key(word), "disk must contain {}", word);
        }

        for word in &words {
            assert!(remove(word).unwrap());
        }
    }

    /// A panic inside a critical section must NOT take down every later
    /// synthesis call: the poisoned lock is recovered, not propagated. (The
    /// old `.expect("lexicon cache poisoned")` panicked on every `current()`
    /// after one poisoning, until restart.)
    #[test]
    #[serial]
    fn poisoned_lock_recovers() {
        // Poison CACHE by panicking while holding the write guard.
        let poisoner = std::thread::spawn(|| {
            let _guard = CACHE.write().unwrap_or_else(PoisonError::into_inner);
            panic!("deliberate poison");
        });
        assert!(poisoner.join().is_err(), "poisoner thread must panic");

        // Every public entry point still works.
        let word = "zz-test-poison-zz";
        save(
            word,
            PronunciationEntry {
                ipa: "pˈɔɪzn".to_string(),
                sounds_like: "poison".to_string(),
            },
        )
        .expect("save must survive a poisoned lock");
        let lex = current().expect("lexicon non-empty");
        assert_eq!(lex.get(word), Some("pˈɔɪzn"));
        assert!(all().contains_key(word));
        assert!(remove(word).expect("remove must survive a poisoned lock"));
    }
}
