//! Strip content that must never be spoken aloud.
//!
//! Observed 2026-08-11, in a live voice session: a worker's escalation payload
//! reached the TTS engine verbatim, and Kokoro dutifully read out
//!
//! ```text
//! "evidence_refs": [
//! "019ef6e4-57d7", "019efa7a-6fa9",
//! "019fd14c-c7c2", "019fd14d-27c7", "019ef5b5-cfd7", "019fee68"
//! ```
//!
//! as **37 seconds of synthesized audio** spelling out UUIDs and a JSON key —
//! a third of that turn's entire speech budget, none of it meaningful, and all
//! of it billed twice: once in synthesis latency and once in the user's time
//! listening to it.
//!
//! A screen can show an identifier harmlessly. A speaker cannot. This module is
//! the boundary where that distinction gets enforced: whatever the agent wrote,
//! only speakable prose reaches the synthesizer.
//!
//! The filter is deliberately conservative — it removes tokens that carry no
//! spoken meaning and leaves everything else exactly as written. When nothing
//! speakable survives, the sentence is dropped entirely rather than synthesized
//! as noise.

use regex::Regex;
use std::sync::LazyLock;

/// A full UUID, or the shortened `019fd14c-c7c2` form the decision inbox uses
/// for evidence refs. Requires at least one hyphen-joined hex group so ordinary
/// hyphenated words can never match.
static ID_LIKE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[0-9a-f]{6,}(?:-[0-9a-f]{4,}){1,4}\b").expect("ID_LIKE regex is valid")
});

/// A bare long hex run: git SHAs, digests, opaque handles. 16+ chars so words
/// like "deadbeef" in prose (8) and hex colours stay untouched.
static LONG_HEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b[0-9a-f]{16,}\b").expect("LONG_HEX regex is valid"));

/// Candidate hex-ish token, 6+ chars. Whether it is actually an identifier is
/// decided by [`is_hex_identifier`] — the `regex` crate has no lookaround, so
/// the digit/letter test happens in code.
///
/// This exists because [`ID_LIKE`] requires a hyphen group and [`LONG_HEX`]
/// requires 16 chars, which let the *last* element of a truncated UUID list
/// (`019fee68` — 8 chars, no hyphen) through to the synthesizer. Caught by the
/// test pinning the exact strings that were spoken aloud.
static HEXISH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b[0-9a-f]{6,}\b").expect("HEXISH regex is valid"));

/// Is this all-hex token an identifier rather than a word or a number?
///
/// Requires BOTH a digit and an a-f letter, which is what separates the three
/// cases: `019fee68` is an id (both), `123456` is a quantity (no letters), and
/// `deadbeef` is a word someone may well have typed on purpose (no digits).
fn is_hex_identifier(token: &str) -> bool {
    let has_digit = token.chars().any(|c| c.is_ascii_digit());
    let has_hex_letter = token.chars().any(|c| c.is_ascii_alphabetic());
    has_digit && has_hex_letter
}

/// A JSON object key at the start of a fragment: `"evidence_refs":`.
static JSON_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""[A-Za-z_][A-Za-z0-9_]*"\s*:"#).expect("JSON_KEY regex is valid")
});

/// Structural punctuation left behind once identifiers are removed.
static STRUCTURAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"[\[\]{}"`]"#).expect("STRUCTURAL regex is valid"));

/// Runs of whitespace and orphaned separators, e.g. ` , ,  , `.
static ORPHAN_SEPARATORS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:\s*[,;:]\s*){2,}").expect("ORPHAN_SEPARATORS regex is valid"));

static WHITESPACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s{2,}").expect("WHITESPACE regex is valid"));

/// `**bold**` / `__bold__` markers. Kokoro will otherwise say "asterisk".
static MD_EMPHASIS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\*\*|__").expect("MD_EMPHASIS regex is valid"));

/// `*italic*` leftover singles after `**` is gone.
static MD_ITALIC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\*([^*]+)\*").expect("MD_ITALIC regex is valid"));

/// ATX headings at the start of a line.
static MD_HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^#{1,6}\s+").expect("MD_HEADING regex is valid"));

/// Markdown bullets at the start of a line.
static MD_BULLET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^[-*]\s+").expect("MD_BULLET regex is valid"));

/// Minimum run of letters for a fragment to be worth speaking. Two letters
/// keeps "OK" and "no"; anything shorter is punctuation noise.
const MIN_ALPHA_RUN: usize = 2;

/// Reduce `text` to what should actually be spoken.
///
/// Returns `None` when nothing speakable remains — the caller skips synthesis
/// entirely rather than voicing leftover punctuation.
pub fn speakable(text: &str) -> Option<String> {
    let folded = crate::voice::speech_normalize::for_speech(text);
    let stripped = MD_EMPHASIS.replace_all(&folded, "");
    let stripped = MD_ITALIC.replace_all(&stripped, "$1");
    let stripped = MD_HEADING.replace_all(&stripped, "");
    let stripped = MD_BULLET.replace_all(&stripped, "");
    let stripped = ID_LIKE.replace_all(&stripped, " ");
    let stripped = LONG_HEX.replace_all(&stripped, " ");
    let stripped = HEXISH.replace_all(&stripped, |caps: &regex::Captures| {
        if is_hex_identifier(&caps[0]) {
            " ".to_string()
        } else {
            caps[0].to_string()
        }
    });
    let stripped = JSON_KEY.replace_all(&stripped, " ");
    let stripped = STRUCTURAL.replace_all(&stripped, " ");
    let stripped = ORPHAN_SEPARATORS.replace_all(&stripped, ", ");
    let stripped = WHITESPACE.replace_all(&stripped, " ");

    let cleaned = stripped
        .trim()
        .trim_start_matches([',', ';', ':', '.', '-'])
        .trim()
        .to_string();

    if !has_speakable_content(&cleaned) {
        return None;
    }
    Some(cleaned)
}

/// Does the text contain a run of letters long enough to be a word?
fn has_speakable_content(text: &str) -> bool {
    let mut run = 0usize;
    for ch in text.chars() {
        if ch.is_alphabetic() {
            run += 1;
            if run >= MIN_ALPHA_RUN {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact fragments Kokoro read aloud on 2026-08-11. Every one of these
    /// must now be dropped before synthesis.
    #[test]
    fn drops_the_fragments_that_were_actually_spoken() {
        for spoken in [
            r#""evidence_refs": ["#,
            r#""019ef6e4-57d7", "019efa7a-6fa9","#,
            r#""019efaf8-7726","#,
            r#""019fd14c-c7c2", "019fd14d-27c7", "019ef5b5-cfd7", "019fee68""#,
            r#"["019fd14d-57bb", "019eff14-443b"]"#,
        ] {
            assert_eq!(
                speakable(spoken),
                None,
                "should never be synthesized: {spoken}"
            );
        }
    }

    /// Ordinary replies must survive completely untouched — the filter is not
    /// allowed to quietly edit what the agent said.
    #[test]
    fn leaves_ordinary_prose_alone() {
        for prose in [
            "It should land cleanly this time with proper options included.",
            "You need to go to the Decision Inbox to approve it.",
            "Sorry for the runaround on this one.",
            "I moved 12 projects and 3 could not be resolved.",
            "Well-formed hyphenated words like state-of-the-art stay intact.",
        ] {
            assert_eq!(speakable(prose), Some(prose.to_string()));
        }
    }

    /// An identifier embedded in a real sentence: drop the ID, keep the sentence.
    #[test]
    fn strips_ids_but_keeps_the_surrounding_sentence() {
        let out = speakable("The goal 019fd14c-c7c2 is stuck in Triage.").unwrap();
        assert!(!out.contains("019fd14c"), "id survived: {out}");
        assert!(out.contains("is stuck in Triage"), "sentence lost: {out}");

        let out = speakable("Commit a1b2c3d4e5f60718293a4b5c6d7e8f90 landed.").unwrap();
        assert!(!out.contains("a1b2c3"), "sha survived: {out}");
        assert!(out.contains("landed"));
    }

    /// Short hex-ish words in prose are not identifiers. `deadbeef` has no
    /// digit and `123456` has no hex letter — only tokens with BOTH are ids.
    #[test]
    fn does_not_eat_short_hex_words_or_numbers() {
        for prose in [
            "The deadbeef case returns 42 rows.",
            "The file is 123456 bytes on disk.",
            "Version 2026 shipped with 100000 users.",
        ] {
            assert_eq!(
                speakable(prose),
                Some(prose.to_string()),
                "mangled: {prose}"
            );
        }
    }

    /// The last element of a truncated UUID list is a bare 8-char hex token
    /// with no hyphen — it slipped past the hyphen-anchored and 16-char rules
    /// and would still have been read aloud.
    #[test]
    fn drops_bare_short_hex_identifiers() {
        assert_eq!(speakable("019fee68"), None);
        let out = speakable("Card 019fee68 is stuck.").unwrap();
        assert!(!out.contains("019fee68"), "id survived: {out}");
        assert!(out.contains("is stuck"));
    }

    #[test]
    fn strips_markdown_emphasis_that_was_spoken_aloud() {
        // 2026-08-21 iPhone voice session: Kokoro read "**Angle one:" and "**One:".
        let out =
            speakable("**Angle one: a marketplace for projects.** People discovering").unwrap();
        assert!(!out.contains('*'), "asterisks survived: {out}");
        assert!(out.starts_with("Angle one:"));
        assert!(out.contains("People discovering"));

        let out = speakable("So here are some directions I'd explore: **One:").unwrap();
        assert!(!out.contains('*'), "asterisks survived: {out}");
        assert!(out.contains("One:"));
    }

    #[test]
    fn folds_curly_apostrophes_in_contractions() {
        let out = speakable("He should\u{2019}ve gone.").unwrap();
        assert!(
            out.contains("should've"),
            "curly apostrophe survived: {out}"
        );
        assert!(!out.contains('\u{2019}'));
    }

    #[test]
    fn last_night_equals_and_caps_hyphens_are_not_spoken() {
        let out = speakable("Elspeth = EL-speth, Prideine = PRID-ayn.").unwrap();
        assert!(!out.contains('='), "equals survived: {out}");
        assert!(!out.contains("EL-"), "caps hyphen survived: {out}");
        assert!(out.contains("Elspeth,"));
        assert!(out.contains("EL speth"));
        assert!(out.contains("PRID ayn"));
    }

    #[test]
    fn empty_and_punctuation_only_are_dropped() {
        for junk in ["", "   ", "{}", "[,,]", "\"\"", " , , , "] {
            assert_eq!(speakable(junk), None, "should be dropped: {junk:?}");
        }
    }
}
