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

/// Split an interior camelCase / PascalCase token so "OpenAI" is spoken as
/// two words rather than spelled. All-lowercase / ALL-CAPS tokens are left
/// alone — those belong to the dictionary and the compound splitter.
pub fn split_camel(token: &str) -> Option<String> {
    let chars: Vec<char> = token.chars().collect();
    if chars.len() < 3 {
        return None;
    }
    let has_lower = chars.iter().any(|c| c.is_lowercase());
    let has_upper = chars.iter().any(|c| c.is_uppercase());
    if !has_lower || !has_upper {
        return None;
    }

    let mut out = String::with_capacity(token.len() + 4);
    for (i, &ch) in chars.iter().enumerate() {
        if i > 0 && ch.is_uppercase() {
            let prev_lower = chars[i - 1].is_lowercase();
            let next_lower = chars.get(i + 1).is_some_and(|c| c.is_lowercase());
            if prev_lower || next_lower {
                out.push(' ');
            }
        }
        out.push(ch);
    }
    if out.contains(' ') {
        Some(out)
    } else {
        None
    }
}

fn lead_punct(token: &str) -> String {
    token
        .chars()
        .take_while(|c| c.is_ascii_punctuation())
        .collect()
}

fn trail_punct(token: &str) -> String {
    token
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_punctuation())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

/// Rewrite one core token (punctuation already stripped) into speakable parts.
/// Returns the replacement text and any still-unresolved pieces.
fn rewrite_core(dict: &impl WordDict, core: &str) -> (String, Vec<String>) {
    if core.is_empty() || dict.known(&core.to_lowercase()) {
        return (core.to_string(), Vec::new());
    }

    // Hyphen / underscore compounds: "gpt-oss", "co_working".
    if core.contains('-') || core.contains('_') {
        let mut pieces: Vec<String> = Vec::new();
        let mut unresolved: Vec<String> = Vec::new();
        for part in core.split(|c| c == '-' || c == '_') {
            if part.is_empty() {
                continue;
            }
            let (rewritten, mut oov) = rewrite_core(dict, part);
            pieces.push(rewritten);
            unresolved.append(&mut oov);
        }
        return (pieces.join(" "), unresolved);
    }

    // "gpt4" / "web2" — split letters from a trailing/leading digit run.
    if let Some(split) = split_alpha_digits(core) {
        let mut unresolved: Vec<String> = Vec::new();
        let mut pieces: Vec<String> = Vec::new();
        for part in split.split_whitespace() {
            let (rewritten, mut oov) = rewrite_core(dict, part);
            pieces.push(rewritten);
            unresolved.append(&mut oov);
        }
        return (pieces.join(" "), unresolved);
    }

    if let Some(camel) = split_camel(core) {
        let mut unresolved: Vec<String> = Vec::new();
        let mut pieces: Vec<String> = Vec::new();
        for part in camel.split_whitespace() {
            let (rewritten, mut oov) = rewrite_core(dict, part);
            pieces.push(rewritten);
            unresolved.append(&mut oov);
        }
        return (pieces.join(" "), unresolved);
    }

    if !core.chars().all(|c| c.is_alphabetic()) {
        // Mixed junk (paths, versions) is not ours to invent a reading of.
        return (core.to_string(), Vec::new());
    }

    match split_compound(dict, core) {
        Some((a, b)) => (format!("{a} {b}"), Vec::new()),
        None => (core.to_string(), vec![core.to_lowercase()]),
    }
}

/// "gpt4" → "gpt 4", "3js" → "3 js". No split when there are no digits or no
/// letters — those tokens pass through untouched.
fn split_alpha_digits(core: &str) -> Option<String> {
    let has_digit = core.chars().any(|c| c.is_ascii_digit());
    let has_alpha = core.chars().any(|c| c.is_ascii_alphabetic());
    if !has_digit || !has_alpha {
        return None;
    }
    let mut out = String::with_capacity(core.len() + 2);
    let mut prev_alpha: Option<bool> = None;
    for ch in core.chars() {
        let alpha = ch.is_ascii_alphabetic();
        if let Some(was_alpha) = prev_alpha {
            if was_alpha != alpha && (alpha || ch.is_ascii_digit()) {
                out.push(' ');
            }
        }
        out.push(ch);
        if ch.is_ascii_alphabetic() || ch.is_ascii_digit() {
            prev_alpha = Some(alpha);
        }
    }
    if out.contains(' ') {
        Some(out)
    } else {
        None
    }
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
        if core.is_empty() {
            out.push(token.to_string());
            continue;
        }
        let (rewritten, mut oov) = rewrite_core(dict, core);
        unresolved.append(&mut oov);
        if rewritten == core {
            out.push(token.to_string());
        } else {
            out.push(format!(
                "{}{}{}",
                lead_punct(token),
                rewritten,
                trail_punct(token)
            ));
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

    #[test]
    fn splits_camel_case_product_names() {
        assert_eq!(split_camel("OpenAI").as_deref(), Some("Open AI"));
        assert_eq!(split_camel("ChatGPT").as_deref(), Some("Chat GPT"));
        assert_eq!(split_camel("GraphQL").as_deref(), Some("Graph QL"));
        assert_eq!(
            split_camel("openai"),
            None,
            "all-lower is the compound splitter's"
        );
        assert_eq!(
            split_camel("NASA"),
            None,
            "all-caps is an acronym, not camelCase"
        );
    }

    #[test]
    fn expands_hyphenated_and_digit_tokens() {
        let (text, unresolved) = expand_compounds(&dict(), "Ship gpt-oss and web2.");
        assert!(text.contains("gpt oss"), "{text}");
        assert!(text.contains("web 2"), "{text}");
        assert!(unresolved.contains(&"gpt".to_string()), "{unresolved:?}");
        assert!(unresolved.contains(&"oss".to_string()), "{unresolved:?}");
    }

    #[test]
    fn expands_camel_case_in_a_sentence() {
        let (text, _) = expand_compounds(&dict(), "Ask OpenAI.");
        assert!(text.contains("Open AI"), "{text}");
        assert!(text.ends_with('.'), "{text}");
    }
}
