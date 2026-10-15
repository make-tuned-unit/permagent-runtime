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

/// `[link text](target)` → `link text`. Runs before anything else: prosody
/// would otherwise drop `[link text]` as an unknown delivery tag.
static MD_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"!?\[([^\]\n]*)\]\([^)\s]*\)").expect("MD_LINK regex is valid"));

/// A backticked code span. Never spoken: a tool name, a path or a snippet is
/// screen material (2026-10-14: "I don't have a `web" was read aloud).
static CODE_SPAN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"```[^`]*(?:```|$)|`[^`\n]*(?:`|$)").expect("CODE_SPAN regex is valid")
});

/// Bare URLs and `www.` hosts.
static URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:https?://|www\.)[^\s)>\]]+").expect("URL regex is valid")
});

/// File-system paths: `~/x`, `./x`, `/a/b`, `a/b/c`, and bare file names with
/// a source/config extension (`voice.rs`, `README.md`).
static PATH_LIKE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:~|\.{1,2})/[\w.\-]+(?:/[\w.\-]+)*/?|/[\w.\-]+(?:/[\w.\-]+)+/?|\b[\w.\-]+(?:/[\w.\-]+){2,}|\b[\w\-]+\.(?:rs|ts|tsx|jsx|py|toml|json|ya?ml|swift|md|sh|lock|sql|css|html)\b",
    )
    .expect("PATH_LIKE regex is valid")
});

/// Code identifiers: `snake_case`, `module::path`, `call()`.
static CODE_IDENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b[A-Za-z][A-Za-z0-9]*(?:_[A-Za-z0-9]+)+\b|\b\w+(?:::\w+)+|\b\w+\(\)")
        .expect("CODE_IDENT regex is valid")
});

/// Branch-like slugs: hyphen-joined tokens carrying a date or a digit-bearing
/// segment, e.g. `ui-polish-2026-09`, `review3-fixes-2026-09`. Ordinary
/// hyphenated prose (`state-of-the-art`, `twenty-one`) has no digits.
static BRANCH_SLUG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[a-z0-9]+(?:-[a-z0-9]+)+\b").expect("BRANCH_SLUG regex is valid")
});

fn is_branch_slug(token: &str) -> bool {
    let segments: Vec<&str> = token.split('-').collect();
    let digit_segments = segments
        .iter()
        .filter(|s| s.chars().any(|c| c.is_ascii_digit()))
        .count();
    let has_alpha = token.chars().any(|c| c.is_ascii_alphabetic());
    // A date-ish tail or a digit-bearing word segment marks a slug; a plain
    // range like "10-12" (no letters) is a quantity and stays.
    has_alpha && segments.len() >= 3 && digit_segments >= 1
}

/// Parentheses emptied by the removals above: `( , , )`.
static EMPTY_PARENS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\(\s*[,;:\s]*\)").expect("EMPTY_PARENS regex is valid"));

/// Numbered list markers at the start of a line (`1. `, `2) `) and `+ `.
static MD_NUMBERED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:\d{1,2}[.)]|\+)\s+").expect("MD_NUMBERED regex is valid")
});

/// Token-spaced punctuation (V-25): `word ,` / `it 's` / `do n't`.
static SPACE_BEFORE_PUNCT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\S)\s+([,.;:!?])").expect("SPACE_BEFORE_PUNCT regex is valid"));
static SPACED_CLITIC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)([a-z])\s+('(?:s|re|ve|ll|d|m)|n't)\b").expect("SPACED_CLITIC regex is valid")
});

/// Remove what must never reach a speaker and was written for a screen:
/// links keep their text; code spans, URLs, paths, code identifiers and
/// branch slugs are omitted. Public so the voice route can run it BEFORE the
/// prosody pass (which treats `[text]` as a delivery tag).
pub fn strip_screen_only(text: &str) -> String {
    let out = MD_LINK.replace_all(text, "$1");
    let out = CODE_SPAN.replace_all(&out, " ");
    let out = URL.replace_all(&out, " ");
    let out = PATH_LIKE.replace_all(&out, " ");
    let out = CODE_IDENT.replace_all(&out, " ");
    let out = BRANCH_SLUG.replace_all(&out, |caps: &regex::Captures| {
        if is_branch_slug(&caps[0]) {
            " ".to_string()
        } else {
            caps[0].to_string()
        }
    });
    let out = EMPTY_PARENS.replace_all(&out, " ");
    out.into_owned()
}

/// Re-join tokens a tokenizer spaced apart (`if it 's up , try`).
fn collapse_token_spacing(text: &str) -> String {
    let out = SPACED_CLITIC.replace_all(text, "$1$2");
    SPACE_BEFORE_PUNCT.replace_all(&out, "$1$2").into_owned()
}

/// Minimum run of letters for a fragment to be worth speaking. Two letters
/// keeps "OK" and "no"; anything shorter is punctuation noise.
const MIN_ALPHA_RUN: usize = 2;

/// Reduce `text` to what should actually be spoken.
///
/// Returns `None` when nothing speakable remains — the caller skips synthesis
/// entirely rather than voicing leftover punctuation.
pub fn speakable(text: &str) -> Option<String> {
    let screen_free = strip_screen_only(text);
    let folded = crate::voice::speech_normalize::for_speech(&screen_free);
    let stripped = MD_EMPHASIS.replace_all(&folded, "");
    let stripped = MD_ITALIC.replace_all(&stripped, "$1");
    let stripped = MD_HEADING.replace_all(&stripped, "");
    let stripped = MD_BULLET.replace_all(&stripped, "");
    let stripped = MD_NUMBERED.replace_all(&stripped, "");
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
    let stripped = collapse_token_spacing(&stripped);
    let stripped = EMPTY_PARENS.replace_all(&stripped, " ");
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

    /// V-24, 2026-10-14 live: a parenthesised list of dated branch slugs was
    /// read aloud, a bullet list was read with its markers, and a reply began
    /// "I don't have a `web".
    #[test]
    fn branch_slugs_backticks_paths_and_urls_are_not_spoken() {
        let out = speakable(
            "You have four branches (ui-polish-2026-09, review3-fixes-2026-09, wq-2026-10-01) in ringsocial.",
        )
        .unwrap();
        assert!(!out.contains("2026"), "slug survived: {out}");
        assert!(!out.contains('('), "empty parens survived: {out}");
        assert!(out.contains("You have four branches"));
        assert!(out.contains("in ringsocial"));

        let out = speakable("I don't have a `web_search` tool here.").unwrap();
        assert!(!out.contains('`') && !out.contains("web_search"), "{out}");
        assert!(out.contains("I don't have a"));
        assert_eq!(
            speakable("I don't have a `web"),
            Some("I don't have a".into())
        );

        let out = speakable("It lives in crates/goose-server/src/routes/voice.rs now.").unwrap();
        assert!(!out.contains('/') && !out.contains("voice.rs"), "{out}");
        assert!(out.contains("now"));

        let out = speakable("See https://example.com/a?b=c for details.").unwrap();
        assert!(!out.contains("http") && !out.contains("example"), "{out}");
        assert!(out.contains("for details"));

        let out = speakable("Call stream_reply_with_tts() via voice::speakable first.").unwrap();
        assert!(!out.contains('_') && !out.contains("::"), "{out}");
    }

    #[test]
    fn markdown_links_keep_their_text_and_lists_lose_markers() {
        assert_eq!(
            speakable("Read [the tide table](https://tides.example/halifax) first."),
            Some("Read the tide table first.".into())
        );
        assert_eq!(speakable("- Buy milk."), Some("Buy milk.".into()));
        assert_eq!(speakable("2. Then bread."), Some("Then bread.".into()));
        assert_eq!(speakable("## Tides today"), Some("Tides today".into()));
        assert_eq!(
            speakable("**Low tide** is at noon."),
            Some("Low tide is at noon.".into())
        );
    }

    /// V-25: "For instance , if it 's up , try" reached TTS token-spaced.
    #[test]
    fn token_spaced_text_is_collapsed() {
        assert_eq!(
            speakable("For instance , if it 's late , we do n't wait ."),
            Some("For instance, if it's late, we don't wait.".into())
        );
    }

    #[test]
    fn product_names_and_hyphenated_prose_survive() {
        for prose in [
            "DeepSeek and MiniMax both answered on the iPhone.",
            "A state-of-the-art twenty-one gun salute.",
            "Low tide is between 10-12 today.",
            "We use Node.js and e.g. the U.S. grid.",
            "Open 24/7, and/or by appointment.",
        ] {
            assert_eq!(
                speakable(prose),
                Some(prose.to_string()),
                "mangled: {prose}"
            );
        }
    }

    #[test]
    fn empty_and_punctuation_only_are_dropped() {
        for junk in ["", "   ", "{}", "[,,]", "\"\"", " , , , "] {
            assert_eq!(speakable(junk), None, "should be dropped: {junk:?}");
        }
    }
}
