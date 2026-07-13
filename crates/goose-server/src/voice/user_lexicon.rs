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
use std::sync::RwLock;

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

fn with_entries<R>(f: impl FnOnce(&HashMap<String, PronunciationEntry>) -> R) -> R {
    {
        let cache = CACHE.read().expect("lexicon cache poisoned");
        if let Some(entries) = cache.as_ref() {
            return f(entries);
        }
    }
    let loaded = load_from_disk();
    let mut cache = CACHE.write().expect("lexicon cache poisoned");
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
    let mut cache = CACHE.write().expect("lexicon cache poisoned");
    let entries = cache.get_or_insert_with(load_from_disk);
    entries.insert(word, entry);
    persist(entries)?;
    Ok(entries.len())
}

/// Remove a pronunciation. Returns true when something was removed.
pub fn remove(word: &str) -> Result<bool, String> {
    let word = word.trim().to_lowercase();
    let mut cache = CACHE.write().expect("lexicon cache poisoned");
    let entries = cache.get_or_insert_with(load_from_disk);
    let removed = entries.remove(&word).is_some();
    if removed {
        persist(entries)?;
    }
    Ok(removed)
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

    /// Round-trip through the public API: save → current() carries the IPA,
    /// case-insensitively; remove → gone. (Uses the real store path — entries
    /// are namespaced under a test-only word to avoid clobbering user data.)
    #[test]
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
}
