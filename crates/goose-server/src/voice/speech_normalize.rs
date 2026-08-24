//! Fold speech text into the form Kokoro/misaki can actually say.
//!
//! FluidAudio #774 (KokoroAne, 2026): the English splitter only treats the
//! ASCII apostrophe (U+0027) as word-internal. Typographic apostrophes
//! (U+2019, the iOS / LLM default in `should’ve`) tokenize as `should` + `ve`,
//! and `ve` is spoken like the solfège note. The Misaki lexicon stores
//! contractions under ASCII apostrophes — once the token survives, G2P works.
//!
//! Same class of bug as PocketTTS #584. We fold *before* G2P and before the
//! user lexicon, so `should've` / `should’ve` / `should‘ve` are one word.
//!
//! Kokoro's other human-speech levers are punctuation, not SSML (deAPI /
//! hexgrad 2025–26): commas breathe, em dashes turn, ellipses think, `=` is
//! not in the vocab and is read as "equals". `for_speech` cleans those
//! before synthesis. Ordinary hyphenated prose (`state-of-the-art`) is left
//! alone — FluidAudio #775 keeps those keys reachable.

/// Apostrophe-like marks that must become ASCII `'` before phonemize.
const APOSTROPHE_VARIANTS: &[char] = &[
    '\u{2019}', // right single quotation mark — iOS smart punctuation, LLMs
    '\u{2018}', // left single quotation mark
    '\u{02BC}', // modifier letter apostrophe
    '\u{2032}', // prime
    '\u{FF07}', // fullwidth apostrophe
];

/// Fold typographic apostrophes to ASCII so contractions stay one token.
pub fn fold_apostrophes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if APOSTROPHE_VARIANTS.contains(&ch) {
            out.push('\'');
        } else {
            out.push(ch);
        }
    }
    out
}

/// Pre-G2P cleanup so Kokoro speaks prose, not symbols.
///
/// Apostrophes first (contractions), then marks the model cannot say:
/// `=` → a comma breath, en/minus dashes → the em dash Kokoro already
/// pauses on, and ALL-CAPS hyphen respellings (`EL-speth`) → spaces so
/// he does not say "dash".
pub fn for_speech(text: &str) -> String {
    let folded = fold_apostrophes(text);
    let chars: Vec<char> = folded.chars().collect();
    let mut out = String::with_capacity(folded.len());
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        match ch {
            '=' => {
                // The space BEFORE the sign is already in `out`, so testing
                // `ends_with(' ')` suppressed the very comma this exists to
                // add — "Elspeth = EL-speth" came out as "Elspeth  EL speth",
                // a run-on with no breath. Reclaim that space first, then
                // swallow the one after the sign so the breath is a single
                // ", " rather than a doubled gap.
                while out.ends_with(' ') {
                    out.pop();
                }
                if !out.is_empty() && !out.ends_with(',') {
                    out.push(',');
                }
                if !out.is_empty() {
                    out.push(' ');
                }
                while i + 1 < chars.len() && chars[i + 1] == ' ' {
                    i += 1;
                }
            }
            '\u{2013}' | '\u{2212}' => out.push('\u{2014}'), // – − → —
            '-' if is_caps_respelling_hyphen(&chars, i) => out.push(' '),
            _ => out.push(ch),
        }
        i += 1;
    }
    out
}

/// `EL-speth` / `PRID-ayn`: two or more capitals, then a hyphen, then a letter.
/// Leaves `state-of-the-art` and `twenty-one` untouched.
fn is_caps_respelling_hyphen(chars: &[char], hyphen_at: usize) -> bool {
    let next = chars.get(hyphen_at + 1).copied();
    if !next.is_some_and(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    let mut caps = 0usize;
    for ch in chars[..hyphen_at].iter().rev() {
        if ch.is_ascii_uppercase() {
            caps += 1;
        } else {
            break;
        }
    }
    caps >= 2
}

/// True when `text` is a short contraction that speech must keep intact.
pub fn looks_like_contraction(text: &str) -> bool {
    let t = fold_apostrophes(text).to_ascii_lowercase();
    t.contains('\'')
        && matches!(
            t.trim_matches(|c: char| !c.is_ascii_alphabetic() && c != '\''),
            "should've"
                | "would've"
                | "could've"
                | "might've"
                | "must've"
                | "i've"
                | "you've"
                | "we've"
                | "they've"
                | "i'm"
                | "you're"
                | "we're"
                | "they're"
                | "don't"
                | "doesn't"
                | "didn't"
                | "won't"
                | "can't"
                | "couldn't"
                | "shouldn't"
                | "wouldn't"
                | "isn't"
                | "aren't"
                | "wasn't"
                | "weren't"
                | "it's"
                | "that's"
                | "there's"
                | "here's"
                | "what's"
                | "who's"
                | "let's"
                | "i'll"
                | "i'd"
                | "you'll"
                | "you'd"
                | "he'll"
                | "he'd"
                | "she'll"
                | "she'd"
                | "we'll"
                | "we'd"
                | "they'll"
                | "they'd"
                | "haven't"
                | "hadn't"
                | "hasn't"
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curly_shouldve_becomes_ascii_contraction() {
        let curly = "He should\u{2019}ve stayed.";
        let folded = fold_apostrophes(curly);
        assert_eq!(folded, "He should've stayed.");
        assert!(folded.contains("should've"));
        assert!(!folded.contains('\u{2019}'));
    }

    #[test]
    fn wouldve_and_couldve_fold_too() {
        assert_eq!(
            fold_apostrophes("would\u{2018}ve could\u{02BC}ve"),
            "would've could've"
        );
    }

    #[test]
    fn ascii_contraction_is_unchanged() {
        assert_eq!(fold_apostrophes("I don't know."), "I don't know.");
    }

    #[test]
    fn known_contractions_are_recognised() {
        assert!(looks_like_contraction("should’ve"));
        assert!(looks_like_contraction("Would've"));
        assert!(looks_like_contraction("I'm"));
        assert!(looks_like_contraction("I'll"));
        assert!(looks_like_contraction("haven't"));
        assert!(!looks_like_contraction("Elspeth"));
    }

    #[test]
    fn equals_becomes_a_breath_not_the_word_equals() {
        let out = for_speech("Elspeth = el-speth, Prideine = prid-ayn.");
        assert!(!out.contains('='), "equals survived: {out}");
        assert!(out.contains("Elspeth,"));
        assert!(out.contains("Prideine,"));
    }

    #[test]
    fn caps_hyphen_respelling_loses_the_dash() {
        assert_eq!(for_speech("EL-speth"), "EL speth");
        assert_eq!(for_speech("PRID-ayn"), "PRID ayn");
        assert_eq!(
            for_speech("Well-formed state-of-the-art twenty-one"),
            "Well-formed state-of-the-art twenty-one",
            "ordinary hyphens must stay for lexicon hits"
        );
    }

    #[test]
    fn en_dash_becomes_kokoro_em_dash() {
        assert_eq!(for_speech("wait – then"), "wait — then");
    }
}
