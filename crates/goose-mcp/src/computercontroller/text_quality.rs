//! Garble detection for extracted document text (#468).
//!
//! PDF text extraction can silently produce char-shifted junk: a subsetted
//! font whose glyph indices don't map to Unicode renders fine on screen, but
//! reading the raw character codes without the font's ToUnicode CMap yields a
//! fixed-offset shift ("All Rights Reserved" → "$OO5LJKWV5HVHUYHG"). Feeding
//! that downstream is worse than failing: the local summarizer / agent will
//! confidently "summarize" noise. This module is the safety gate — a cheap,
//! dependency-free readability assessment that separates natural-language text
//! (any Latin-script language, ALL-CAPS legal text, code, name lists, part
//! numbers) from extraction garbage (Caesar-shifted text at any offset,
//! glyph-index soup, mojibake, unbroken letter runs).
//!
//! Signals (thresholds tuned on fixture corpora — see unit tests):
//! - **mojibake**: U+FFFD replacement chars / stray C0 controls.
//! - **stopword ratio**: fraction of alphabetic tokens that are very common
//!   function words (multilingual list). Real prose ≈ 0.25–0.45; shifted text
//!   ≈ 0 at every offset (a fixed shift maps every function word off the list).
//! - **vowel-bearing ratio**: fraction of ≥3-letter tokens containing a vowel.
//! - **digit-interleave**: tokens with digits sandwiched between letters
//!   (glyph-index junk like "ZHDOWK1H").
//! - **common-letter share**: fraction of letters in the 12 most frequent
//!   Latin-language letters (etaoinsrhdlu ≈ 80% of English letters, similarly
//!   dominant in French/German/Spanish/Italian). Shift-invariant catch-all: a
//!   nonzero shift maps frequent letters onto infrequent ones, so the share
//!   collapses (readable ≥ ~0.76 on fixtures, shifted ≤ ~0.58).
//!
//! Texts with too few letterful tokens return `Readable` — the Reader's
//! existing sparse-text check owns that case, and a short title can't be
//! meaningfully classified (fail-open on volume, fail-closed on garble).
//!
//! NOTE: mirror of `crates/goose/src/reader/garble.rs` (same heuristics, same
//! thresholds) — this crate cannot depend on the `permagent` crate. Keep the
//! two in sync.

use std::collections::HashSet;
use std::sync::LazyLock;

/// Sample size — enough signal, bounded cost on huge documents.
const SAMPLE_CHARS: usize = 8000;
/// Minimum letterful tokens for a meaningful verdict.
const MIN_TOKENS: usize = 15;
/// Above this fraction of replacement/control chars → mojibake.
const MAX_WEIRD_RATIO: f64 = 0.02;
/// Below this stopword ratio the text has no recognizable function words.
const MIN_STOPWORD_RATIO: f64 = 0.03;
/// Readable Latin text keeps vowel-bearing tokens near 1.0.
const MIN_VOWEL_BEARING: f64 = 0.87;
/// Prose has essentially no letter-digit-letter tokens.
const MAX_DIGIT_INTERLEAVE: f64 = 0.05;
/// Fixture gap: readable ≥ 0.76, shifted/garbled ≤ 0.58.
const MIN_COMMON_LETTER_SHARE: f64 = 0.68;
/// Unbroken-run check: this many letters with almost no word boundaries.
const RUN_MIN_LETTERS: usize = 300;
/// Mean token length no natural-language extraction produces.
const RUN_MEAN_TOKEN_LEN: f64 = 24.0;

/// The 12 highest-frequency letters across major Latin-script languages.
const COMMON_LETTERS: &str = "etaoinsrhdlu";

/// Very common function words for English + French/Spanish/German/Italian/
/// Portuguese/Dutch. Any natural Latin-script prose hits this list heavily;
/// char-shifted text hits it at ~0 for every shift offset.
static STOPWORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    "the of and to a in is it you that he was for on are as with his they i at \
     be this have from or one had by word but not what all were we when your \
     can said there use an each which she do how their if will up other about \
     out many then them these so some her would make like him into time has \
     look two more write go see number no way could people my than first water \
     been call who oil its now find long down day did get come made may part \
     over new sound take only little work know place year live me back give \
     most very after thing our just name good sentence man think say great \
     where help through much before line right too mean old any same tell boy \
     follow came want show also around form three small set put end does \
     another well large must big even such because turn here why ask went men \
     read need land different home us move try kind hand picture again change \
     off play spell air away animal house point page letter mother answer \
     found study still learn should \
     le la et les des un une du au aux ce cette est sont dans pour par sur pas \
     ne se qui que quoi ou \
     el en y los las por con para uno del al lo su es son como mas pero \
     der die das und ist nicht ein eine mit von zu auf fur im den dem sich auch \
     il di che non per una sono da come piu \
     o os um uma nao com mais seu sua \
     de het van te dat op zijn met voor niet"
        .split_whitespace()
        .collect()
});

/// Vowels including common accented Latin forms (so accented-language tokens
/// register as vowel-bearing).
const VOWELS: &str = "aeiouyàáâäèéêëìíîïòóôöùúûüåæøãõ";

/// Verdict on a piece of extracted text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextQuality {
    /// Plausibly real text — safe to ingest/summarize.
    Readable,
    /// Extraction garbage — must NOT be ingested, summarized, or shown as the
    /// document's content.
    Garbled {
        /// Human-readable reason (for logs and error surfaces).
        reason: String,
    },
}

/// Assess whether extracted text is plausibly real language or extraction
/// garbage. Fail-open on short/insufficient text (the sparse-text check owns
/// that), fail-closed on the garble signals.
pub fn assess(text: &str) -> TextQuality {
    let sample: String = text.chars().take(SAMPLE_CHARS).collect();
    if sample.trim().is_empty() {
        return TextQuality::Readable; // emptiness is the sparse check's job
    }

    // Mojibake: replacement chars / stray control chars.
    let total_chars = sample.chars().count();
    let weird = sample
        .chars()
        .filter(|&c| c == '\u{FFFD}' || (c.is_control() && !matches!(c, '\n' | '\r' | '\t')))
        .count();
    let weird_ratio = weird as f64 / total_chars as f64;
    if weird_ratio > MAX_WEIRD_RATIO {
        return TextQuality::Garbled {
            reason: format!(
                "text is {:.0}% replacement/control characters (mojibake)",
                weird_ratio * 100.0
            ),
        };
    }

    // Tokenize: whitespace-split, strip surrounding punctuation/symbols.
    let tokens: Vec<&str> = sample
        .split_whitespace()
        .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|t| !t.is_empty())
        .collect();
    let letterful: Vec<&str> = tokens
        .iter()
        .copied()
        .filter(|t| t.chars().filter(|c| c.is_alphabetic()).count() >= 2)
        .collect();
    let letter_count: usize = letterful
        .iter()
        .map(|t| t.chars().filter(|c| c.is_alphabetic()).count())
        .sum();

    if letterful.len() < MIN_TOKENS {
        // One strong signal still applies: a huge blob of letters with almost
        // no word boundaries is never a real text layer.
        if letter_count > RUN_MIN_LETTERS && !letterful.is_empty() {
            let mean_len = letterful.iter().map(|t| t.chars().count()).sum::<usize>() as f64
                / letterful.len() as f64;
            if mean_len > RUN_MEAN_TOKEN_LEN {
                return TextQuality::Garbled {
                    reason: "unbroken letter runs with no word boundaries".to_string(),
                };
            }
        }
        return TextQuality::Readable; // too little signal to condemn
    }

    // Stopword ratio over purely-alphabetic tokens.
    let alpha: Vec<&str> = letterful
        .iter()
        .copied()
        .filter(|t| t.chars().all(|c| c.is_alphabetic()))
        .collect();
    let stopword_ratio = if alpha.is_empty() {
        0.0
    } else {
        alpha
            .iter()
            .filter(|t| STOPWORDS.contains(t.to_lowercase().as_str()))
            .count() as f64
            / alpha.len() as f64
    };

    // Vowel-bearing ratio over ≥3-char letterful tokens.
    let long_tokens: Vec<&str> = letterful
        .iter()
        .copied()
        .filter(|t| t.chars().count() >= 3)
        .collect();
    let vowel_bearing = if long_tokens.is_empty() {
        1.0
    } else {
        long_tokens
            .iter()
            .filter(|t| {
                t.chars()
                    .any(|c| VOWELS.contains(c.to_lowercase().next().unwrap_or(c)))
            })
            .count() as f64
            / long_tokens.len() as f64
    };

    // Digit-interleave: letters, then digit(s), then letters within one token.
    let interleave_ratio = letterful
        .iter()
        .filter(|t| has_digit_between_letters(t))
        .count() as f64
        / letterful.len() as f64;

    // Common-letter share (shift-invariant frequency check).
    let common_share = if letter_count == 0 {
        1.0
    } else {
        letterful
            .iter()
            .flat_map(|t| t.chars())
            .filter(|c| c.is_alphabetic())
            .filter(|c| {
                c.to_lowercase()
                    .next()
                    .is_some_and(|lc| COMMON_LETTERS.contains(lc))
            })
            .count() as f64
            / letter_count as f64
    };

    if stopword_ratio < MIN_STOPWORD_RATIO
        && (vowel_bearing < MIN_VOWEL_BEARING
            || interleave_ratio > MAX_DIGIT_INTERLEAVE
            || common_share < MIN_COMMON_LETTER_SHARE)
    {
        return TextQuality::Garbled {
            reason: format!(
                "no recognizable words and a character distribution unlike natural language \
                 (stopword ratio {stopword_ratio:.2}, vowel-bearing {vowel_bearing:.2}, \
                 digit-interleave {interleave_ratio:.2}, common-letter share {common_share:.2})"
            ),
        };
    }

    TextQuality::Readable
}

/// True when the token contains a digit strictly between letters ("ZHDOWK1H").
fn has_digit_between_letters(token: &str) -> bool {
    let mut seen_letter = false;
    let mut seen_digit_after_letter = false;
    for c in token.chars() {
        if c.is_alphabetic() {
            if seen_digit_after_letter {
                return true;
            }
            seen_letter = true;
        } else if c.is_ascii_digit() {
            if seen_letter {
                seen_digit_after_letter = true;
            }
        } else {
            seen_letter = false;
            seen_digit_after_letter = false;
        }
    }
    false
}

/// Caesar-shift test helper — reproduces exactly the failure mode from #468
/// (raw char codes read without the ToUnicode CMap = fixed-offset shift).
/// Shared with the reader module tests to build garbled PDF fixtures.
#[cfg(test)]
pub(crate) fn caesar_shift(text: &str, n: u8) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_lowercase() {
                (((c as u8 - b'a' + n) % 26) + b'a') as char
            } else if c.is_ascii_uppercase() {
                (((c as u8 - b'A' + n) % 26) + b'A') as char
            } else {
                c
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shift(text: &str, n: u8) -> String {
        caesar_shift(text, n)
    }

    /// True when [`assess`] flags the text as garbled.
    fn garbled(text: &str) -> bool {
        matches!(assess(text), TextQuality::Garbled { .. })
    }

    const ENGLISH: &str = "Wealthie Family Office overview. All rights reserved. \
        Our platform integrates brokerage services with education savings plans, \
        offering families three revenue streams and a partnership model that \
        reaches schools across the province. The integration is expected to \
        launch in September and will provide access to registered accounts for \
        every student enrolled in the program. This document outlines strategy, \
        market size, competitive landscape, and the projected financial model \
        over the next five years.";

    #[test]
    fn english_prose_is_readable() {
        assert_eq!(assess(ENGLISH), TextQuality::Readable);
    }

    #[test]
    fn char_shifted_text_is_garbled_at_every_offset() {
        // The observed bug was shift +3 ("All Rights Reserved" → "$OO5LJKWV5HVHUYHG"),
        // but a broken glyph→code mapping can land on any offset.
        for n in 1..26u8 {
            let shifted = shift(ENGLISH, n);
            assert!(
                garbled(&shifted),
                "shift +{n} must be detected as garbled: {shifted:.60}"
            );
        }
    }

    #[test]
    fn observed_uppercase_shift3_is_garbled() {
        // The exact style from #468: uppercased source, shift +3.
        let junk = shift(&ENGLISH.to_uppercase(), 3);
        assert!(junk.contains("DOO ULJKWV UHVHUYHG"), "fixture sanity");
        assert!(garbled(&junk));
    }

    #[test]
    fn digit_interleaved_glyph_junk_is_garbled() {
        // Subset-font junk where digits stand in for letters mid-word.
        let junk = "$OO5LJKWV5HVHUYHG ZHDOWK1H IDP1O RII1FH GHFN SUHVHQWDW1RQ \
            VWUDWHJ PDUNHW V1]H F0PSHW1W1YH ODQGVFDSH SURMHFWHG I1QDQF1DO \
            P0GHO RYHU WKH QHAW I1YH HDUV WKUHH UHYHQXH VWUHDPV";
        assert!(garbled(junk));
    }

    #[test]
    fn glyph_soup_is_garbled() {
        let soup = "H4x9 qZ2k 8fLp W0mN3 xY7Qr9 KpL2m8 zXcV4b N5mQ8w E2rT6y \
            U9iO3p A7sD1f G4hJ8k L2zX6c V9bN4m Q8wE2r T6yU9i O3pA7s D1fG4h";
        assert!(garbled(soup));
    }

    #[test]
    fn mojibake_is_garbled() {
        let mojibake = "\u{FFFD}\u{FFFD} he\u{FFFD}lo wor\u{FFFD}d th\u{FFFD}s is \
            bro\u{FFFD}en text \u{FFFD}\u{FFFD} more bro\u{FFFD}en \u{FFFD} stuff \
            here and here \u{FFFD} and more";
        assert!(garbled(mojibake));
    }

    #[test]
    fn unbroken_letter_run_is_garbled() {
        let run = shift(&ENGLISH.replace([' ', '\n'], ""), 3);
        assert!(run.chars().filter(|c| c.is_alphabetic()).count() > 300);
        assert!(garbled(&run));
    }

    #[test]
    fn all_caps_english_is_readable() {
        // Legal boilerplate style must not false-positive.
        assert_eq!(assess(&ENGLISH.to_uppercase()), TextQuality::Readable);
    }

    #[test]
    fn latin_languages_are_readable() {
        let french = "Le bureau familial offre des services de gestion de \
            patrimoine pour les familles. Tous les droits sont réservés. Notre \
            plateforme intègre des services de courtage avec des plans \
            d'épargne-études, offrant aux familles trois sources de revenus et \
            un modèle de partenariat qui atteint les écoles de la province.";
        let german = "Das Family Office bietet Vermögensverwaltung für Familien. \
            Alle Rechte vorbehalten. Unsere Plattform integriert \
            Brokerage-Dienstleistungen mit Bildungssparplänen und bietet \
            Familien drei Einnahmequellen und ein Partnerschaftsmodell, das \
            Schulen in der ganzen Provinz erreicht.";
        let spanish = "La oficina familiar ofrece servicios de gestión de \
            patrimonio para las familias. Todos los derechos reservados. \
            Nuestra plataforma integra servicios de corretaje con planes de \
            ahorro educativo, ofreciendo a las familias tres fuentes de \
            ingresos y un modelo de asociación que llega a las escuelas.";
        assert_eq!(assess(french), TextQuality::Readable);
        assert_eq!(assess(german), TextQuality::Readable);
        assert_eq!(assess(spanish), TextQuality::Readable);
    }

    #[test]
    fn name_lists_and_part_numbers_are_readable() {
        // Zero stopwords but real letters — must not false-positive.
        let names = "John Smith Mary Johnson Robert Williams Patricia Brown \
            Michael Davis Linda Miller William Wilson Elizabeth Moore David \
            Taylor Barbara Anderson Richard Thomas Susan Jackson Joseph White \
            Jessica Harris Christopher Martin";
        let parts = "Part list: A123B C456D E789F G012H bracket assembly using \
            the standard mounting kit with four screws and two washers per \
            unit as described in the manual";
        assert_eq!(assess(names), TextQuality::Readable);
        assert_eq!(assess(parts), TextQuality::Readable);
    }

    #[test]
    fn code_and_invoices_are_readable() {
        let code = r#"fn main() { let x = compute_total(items); if x > threshold {
            return Err(anyhow!("too large")); } for item in items.iter() {
            println!("{}", item.name); } } // the main entry point checks the
            total against the threshold and prints each item name"#;
        let invoice = "Invoice 4823 Acme Corp Total due 1240.00 by 2026-07-01 \
            Payment terms net 30 days. Please remit payment to the account \
            listed below. Thank you for your business. Contact billing at \
            accounts payable for questions about this invoice statement.";
        assert_eq!(assess(code), TextQuality::Readable);
        assert_eq!(assess(invoice), TextQuality::Readable);
    }

    #[test]
    fn short_and_numeric_text_fails_open() {
        // The sparse-text check owns these; garble detection must not condemn.
        assert_eq!(assess("Meeting notes for Tuesday"), TextQuality::Readable);
        assert_eq!(
            assess("1042,2381,99.5,2026-01-04\n2043,1177,88.2,2026-02-11"),
            TextQuality::Readable
        );
        assert_eq!(assess(""), TextQuality::Readable);
        assert_eq!(assess("   \n  "), TextQuality::Readable);
    }
}
