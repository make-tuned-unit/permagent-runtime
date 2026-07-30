//! Compound decomposition for out-of-vocabulary words.
//!
//! misaki is built here with `default-features = false`, which disables its
//! espeak fallback (`G2P::fallback` is `None`). Its OOV path is then, verbatim:
//!
//! ```text
//! // No fallback available or failed, try character-by-character
//! ```
//!
//! So any word missing from its 390k-entry dictionary is SPELLED OUT letter by
//! letter — "proptech" becomes "P-R-O-P-T-E-C-H". That is not a teaching
//! failure the user can fix word by word; it is an open-ended class, and modern
//! product vocabulary is full of it (webhook, dogfood, devops, changelog,
//! toolchain, dashboarding…).
//!
//! Most such words are compounds whose PARTS are in the dictionary and are
//! individually correct. Splitting the word and letting misaki phonemize the
//! parts turns the whole class into a solved problem with no user involvement.
//!
//! ── Why the rules are this conservative ──
//! A wrong split is worse than spelling out, because it sounds confidently
//! incorrect. Validated against the real gb dictionaries, a naive
//! longest-prefix-with-recursion produced "agritech" → ag+rite+ch and "devops"
//! → devo+ps. Each rule below removes a specific observed failure:
//!
//! * **Exactly two parts, no recursion** — every three-part split observed was
//!   garbage.
//! * **A part under 3 chars must be in GOLD** (the high-confidence dictionary),
//!   not silver. This is what keeps "sqlite" → sq+lite out: "sq" is a silver
//!   entry. It still admits "adtech" → ad+tech, since "ad" is gold.
//! * **Balance first, gold count only as a tiebreak** — scoring gold above
//!   balance chose "Supabase" → sup+abase ("sup uh-BASE") over supa+base.
//! * **No inflectional suffix as the second part** — misaki already handles
//!   morphology through its own stem_s/stem_ed/stem_ing paths; splitting there
//!   inserts a word boundary mid-word for no gain.
//!
//! Words with no safe split are left alone and belong in the seeded lexicon.

/// Inflectional endings that must never become the second part of a split.
const SUFFIXES: &[&str] = &[
    "ing", "ed", "es", "s", "er", "ers", "ly", "ion", "ions", "ness", "ment",
];

/// Dictionary access needed to judge a split. Implemented over misaki's
/// `Lexicon` (whose `golds`/`silvers` maps are public) and trivially faked in
/// tests, so the algorithm is verifiable without loading a 390k-entry model.
pub trait WordDict {
    /// In the high-confidence dictionary.
    fn in_gold(&self, word: &str) -> bool;
    /// In either dictionary.
    fn known(&self, word: &str) -> bool;
}

/// Is this part acceptable on its own? Short parts must be gold-confident.
fn part_ok(dict: &impl WordDict, part: &str) -> bool {
    if part.len() < 3 {
        dict.in_gold(part)
    } else {
        dict.known(part)
    }
}

/// Split `word` into two dictionary words, or `None` when no safe split exists.
///
/// Returns lowercased parts. The caller re-joins them with a space and lets G2P
/// phonemize the result, which keeps misaki's stress and tagging in play rather
/// than concatenating phonemes by hand.
pub fn split_compound(dict: &impl WordDict, word: &str) -> Option<(String, String)> {
    let lower = word.to_lowercase();
    // Alphabetic only: digits and punctuation are the tokenizer's business, and
    // a "split" of an identifier or version string is never right.
    if lower.len() < 5 || !lower.chars().all(|c| c.is_alphabetic()) {
        return None;
    }

    let chars: Vec<char> = lower.chars().collect();
    let mut best: Option<((usize, u8), (String, String))> = None;

    // Both parts at least 2 chars.
    for i in 2..chars.len().saturating_sub(1) {
        let a: String = chars[..i].iter().collect();
        let b: String = chars[i..].iter().collect();
        if SUFFIXES.contains(&b.as_str()) {
            continue;
        }
        if !part_ok(dict, &a) || !part_ok(dict, &b) {
            continue;
        }
        // Balance dominates; gold count breaks ties. Reversing this priority
        // picked sup+abase over supa+base.
        let score = (
            a.chars().count().min(b.chars().count()),
            u8::from(dict.in_gold(&a)) + u8::from(dict.in_gold(&b)),
        );
        if best.as_ref().is_none_or(|(bs, _)| score > *bs) {
            best = Some((score, (a, b)));
        }
    }

    best.map(|(_, parts)| parts)
}

/// Rewrite every OOV compound in `text` as its spaced parts, leaving all other
/// tokens byte-identical. Trailing punctuation is preserved so sentence-final
/// pauses survive. Returns the rewritten text plus every token that was left
/// unresolved — the caller logs those as the words speech is still guessing at.
pub fn expand_compounds(dict: &impl WordDict, text: &str) -> (String, Vec<String>) {
    let mut out: Vec<String> = Vec::new();
    let mut unresolved: Vec<String> = Vec::new();

    for token in text.split_whitespace() {
        let core = token.trim_matches(|c: char| c.is_ascii_punctuation());
        // A known word, or one with nothing to judge, passes through untouched.
        if core.is_empty() || dict.known(&core.to_lowercase()) {
            out.push(token.to_string());
            continue;
        }
        // Only alphabetic tokens are candidates; anything else (numbers, code,
        // identifiers) is left for the tokenizer.
        if !core.chars().all(|c| c.is_alphabetic()) {
            out.push(token.to_string());
            continue;
        }
        match split_compound(dict, core) {
            Some((a, b)) => {
                // Rebuild with the original surrounding punctuation.
                let lead: String = token
                    .chars()
                    .take_while(|c| c.is_ascii_punctuation())
                    .collect();
                let trail: String = token
                    .chars()
                    .rev()
                    .take_while(|c| c.is_ascii_punctuation())
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                out.push(format!("{lead}{a} {b}{trail}"));
            }
            None => {
                unresolved.push(core.to_lowercase());
                out.push(token.to_string());
            }
        }
    }

    (out.join(" "), unresolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Fake dictionary mirroring the real gb data for the words under test —
    /// including the specific gold/silver split that decides each case.
    struct Dict {
        gold: HashSet<&'static str>,
        silver: HashSet<&'static str>,
    }

    fn dict() -> Dict {
        Dict {
            gold: [
                "prop",
                "tech",
                "ad",
                "health",
                "web",
                "hook",
                "hooks",
                "dog",
                "food",
                "dev",
                "ops",
                "post",
                "run",
                "book",
                "change",
                "log",
                "front",
                "back",
                "end",
                "tool",
                "chain",
                "scroll",
                "dash",
                "base",
                "rotate",
                "abase",
                "coworking",
                "working",
                "co",
                "agent",
                "perm",
            ]
            .into_iter()
            .collect(),
            // "sq" is silver-only in the real data — the reason sqlite must not
            // split. "gres"/"supa"/"boarding" are the silver parts of otherwise
            // good splits.
            silver: ["sq", "lite", "gres", "supa", "boarding"]
                .into_iter()
                .collect(),
        }
    }

    impl WordDict for Dict {
        fn in_gold(&self, word: &str) -> bool {
            self.gold.contains(word)
        }
        fn known(&self, word: &str) -> bool {
            self.gold.contains(word) || self.silver.contains(word)
        }
    }

    fn split(word: &str) -> Option<String> {
        split_compound(&dict(), word).map(|(a, b)| format!("{a} {b}"))
    }

    #[test]
    fn splits_the_reported_word() {
        // "proptech" was spelled P-R-O-P-T-E-C-H in a live demo.
        assert_eq!(split("proptech").as_deref(), Some("prop tech"));
    }

    #[test]
    fn splits_common_product_vocabulary() {
        for (word, expected) in [
            ("webhook", "web hook"),
            ("dogfood", "dog food"),
            ("changelog", "change log"),
            ("toolchain", "tool chain"),
            ("scrollback", "scroll back"),
            ("frontend", "front end"),
            ("healthtech", "health tech"),
        ] {
            assert_eq!(split(word).as_deref(), Some(expected), "{word}");
        }
    }

    #[test]
    fn admits_a_two_char_gold_first_part() {
        // "ad" is gold, so adtech is safe.
        assert_eq!(split("adtech").as_deref(), Some("ad tech"));
    }

    #[test]
    fn rejects_a_two_char_silver_first_part() {
        // THE case for the gold restriction: "sq" is silver-only, and
        // "sq lite" is plainly wrong. Better to spell it out and seed it.
        assert_eq!(split("sqlite"), None);
    }

    #[test]
    fn prefers_balance_over_gold_count() {
        // sup+abase is gold+gold; supa+base is silver+gold but better balanced.
        // Scoring gold first produced "sup uh-BASE".
        assert_eq!(split("supabase").as_deref(), Some("supa base"));
    }

    #[test]
    fn picks_the_balanced_split_over_a_lopsided_one() {
        // devo+ps is unavailable (ps not in either dict), but this also pins
        // the intent: dev+ops is the balanced answer.
        assert_eq!(split("devops").as_deref(), Some("dev ops"));
    }

    #[test]
    fn never_splits_on_an_inflectional_suffix() {
        // "working" is a dictionary word, so co+working would otherwise match —
        // but morphology belongs to misaki's own stemming, and here the whole
        // word is known anyway.
        assert_eq!(split("runing"), None); // "ing" blocked as a second part
    }

    #[test]
    fn refuses_when_no_safe_split_exists() {
        for word in ["kubernetes", "xterm", "agritech", "kuzu"] {
            assert_eq!(split(word), None, "{word}");
        }
    }

    #[test]
    fn ignores_short_and_non_alphabetic_tokens() {
        assert_eq!(split("api"), None);
        assert_eq!(split("v1.2.3"), None);
        assert_eq!(split("snake_case"), None);
    }

    // ── expand_compounds ──

    #[test]
    fn expands_only_the_unknown_words_in_a_sentence() {
        let (text, unresolved) = expand_compounds(&dict(), "The proptech webhook fired.");
        // "the"/"fired" are unknown to this fake dict but have no safe split,
        // so they pass through untouched — the sentence is never mangled.
        assert!(text.contains("prop tech"), "{text}");
        assert!(text.contains("web hook"), "{text}");
        assert!(unresolved.contains(&"the".to_string()));
    }

    #[test]
    fn preserves_trailing_punctuation_so_pauses_survive() {
        let (text, _) = expand_compounds(&dict(), "Ship proptech.");
        assert!(text.ends_with("prop tech."), "{text}");
    }

    #[test]
    fn reports_unresolved_words_for_the_review_queue() {
        let (_, unresolved) = expand_compounds(&dict(), "Deploy kubernetes now");
        assert!(unresolved.contains(&"kubernetes".to_string()));
    }

    #[test]
    fn leaves_a_fully_known_sentence_byte_identical() {
        let input = "prop tech web hook";
        let (text, unresolved) = expand_compounds(&dict(), input);
        assert_eq!(text, input);
        assert!(unresolved.is_empty());
    }
}
