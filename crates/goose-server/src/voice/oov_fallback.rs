//! Best-guess pronunciation for a word speech does not know.
//!
//! misaki runs here without its espeak fallback, so an out-of-vocabulary word
//! that the compound splitter cannot resolve is SPELLED letter by letter
//! ("U-P-N-E-E-N"). Until 2026-10-14 the backend refused such a sentence
//! instead (`refuse_spell`) and the voice path stopped the reply to ask the
//! user how to say the word. Jesse's decision D5 ended live teaching: an
//! unknown word Henry has to say is now substituted silently with the best
//! phonetic reading we can build, the sentence is always spoken, and the word
//! is queued for review in Settings (`oov_log`).
//!
//! This is a small, conservative English letter-to-sound pass (digraphs,
//! vowel teams, the silent-e rule, soft c/g) that emits the same GB-flavoured
//! IPA the seeded `technical_lexicon` uses. It is not meant to be right for
//! every word; it is meant to sound like a word rather than an alphabet, and
//! every symbol it emits is in the Kokoro vocabulary (pinned by a test in
//! `ort_kokoro_backend`).

/// Short all-caps tokens are acronyms, and an acronym IS said letter by letter
/// ("AWS", "SDK"). Those keep misaki's spelling and are not substituted.
pub fn is_spelled_acronym(original: &str) -> bool {
    let letters: Vec<char> = original.chars().filter(|c| c.is_alphabetic()).collect();
    (2..=5).contains(&letters.len())
        && letters.iter().all(|c| c.is_ascii_uppercase())
        && original
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c.is_ascii_punctuation())
}

fn is_vowel(c: char) -> bool {
    matches!(c, 'a' | 'e' | 'i' | 'o' | 'u')
}

/// Phonemes for a long ("name") vowel.
fn long_vowel(v: char) -> &'static str {
    match v {
        'a' => "eɪ",
        'e' => "iː",
        'i' => "aɪ",
        'o' => "əʊ",
        _ => "uː",
    }
}

/// Phonemes for a short ("checked") vowel.
fn short_vowel(v: char) -> &'static str {
    match v {
        'a' => "æ",
        'e' => "ɛ",
        'i' => "ɪ",
        'o' => "ɒ",
        _ => "ʌ",
    }
}

/// Best-guess IPA for one word, or `None` when the word has nothing a reader
/// could voice (no letters, or no vowel sound at all — "zzqxwv"). Hyphenated
/// input is read part by part.
pub fn fallback_ipa(word: &str) -> Option<String> {
    let parts: Vec<String> = word
        .split(['-', '_', ' '])
        .filter_map(fallback_part)
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

fn fallback_part(part: &str) -> Option<String> {
    let w: Vec<char> = part
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if w.is_empty() || !w.iter().any(|&c| is_vowel(c) || c == 'y') {
        return None;
    }
    let n = w.len();
    let at = |i: usize| w.get(i).copied();
    let starts = |i: usize, s: &str| -> bool {
        let s: Vec<char> = s.chars().collect();
        i + s.len() <= n && w[i..i + s.len()] == s[..]
    };
    // Silent final e after a consonant ("kase", "tane") — it lengthens the
    // vowel before it instead of being said. "be"/"me"-length words keep it.
    let silent_final_e = n >= 3 && at(n - 1) == Some('e') && !is_vowel(w[n - 2]);

    let mut out = String::new();
    let mut stressed = false;
    let mut i = 0;
    while i < n {
        let c = w[i];
        // A vowel nucleus starts here: mark primary stress on the first one.
        let mark_stress = |out: &mut String, stressed: &mut bool| {
            if !*stressed {
                out.push('ˈ');
                *stressed = true;
            }
        };
        if silent_final_e && i == n - 1 {
            break;
        }
        // ── Multi-letter consonant spellings ──
        if i == 0 && (starts(0, "kn") || starts(0, "gn")) {
            out.push('n');
            i += 2;
            continue;
        }
        if i == 0 && starts(0, "wr") {
            out.push('ɹ');
            i += 2;
            continue;
        }
        if starts(i, "tion") {
            out.push_str("ʃən");
            i += 4;
            continue;
        }
        if starts(i, "sion") {
            out.push_str("ʒən");
            i += 4;
            continue;
        }
        if starts(i, "tch") {
            out.push('ʧ');
            i += 3;
            continue;
        }
        if starts(i, "igh") {
            mark_stress(&mut out, &mut stressed);
            out.push_str("aɪ");
            i += 3;
            continue;
        }
        let two = |a: &str| starts(i, a);
        let digraph = if two("ch") {
            Some("ʧ")
        } else if two("sh") {
            Some("ʃ")
        } else if two("th") {
            Some("θ")
        } else if two("ph") {
            Some("f")
        } else if two("ck") {
            Some("k")
        } else if two("ng") {
            Some("ŋ")
        } else if two("qu") {
            Some("kw")
        } else if two("wh") {
            Some("w")
        } else if two("gh") {
            Some(if i == 0 { "ɡ" } else { "" })
        } else {
            None
        };
        if let Some(p) = digraph {
            out.push_str(p);
            i += 2;
            continue;
        }
        // ── Vowel teams ──
        let team = if two("ee") || two("ea") || two("ie") {
            Some("iː")
        } else if two("oo") {
            Some("uː")
        } else if two("ai") || two("ay") || two("ei") || two("ey") {
            Some("eɪ")
        } else if two("oa") || (two("ow") && i + 2 == n) {
            Some("əʊ")
        } else if two("ou") || two("ow") {
            Some("aʊ")
        } else if two("oi") || two("oy") {
            Some("ɔɪ")
        } else if two("au") || two("aw") {
            Some("ɔː")
        } else if two("ue") || two("ew") {
            Some("uː")
        } else if two("ar") && !at(i + 2).is_some_and(is_vowel) {
            Some("ɑː")
        } else if (two("er") || two("ir") || two("ur")) && !at(i + 2).is_some_and(is_vowel) {
            Some(if i + 2 == n && stressed { "ə" } else { "ɜː" })
        } else if two("or") && !at(i + 2).is_some_and(is_vowel) {
            Some("ɔː")
        } else {
            None
        };
        if let Some(p) = team {
            if p != "ə" {
                mark_stress(&mut out, &mut stressed);
            }
            out.push_str(p);
            i += 2;
            continue;
        }
        // ── Single letters ──
        if is_vowel(c) {
            // Open syllable or silent-e: V + one consonant + (vowel | final silent e).
            let next = at(i + 1);
            let after = at(i + 2);
            let lengthened = next.is_some_and(|x| !is_vowel(x) && x != 'y')
                && (after.is_some_and(is_vowel) && !(silent_final_e && i + 2 == n - 1)
                    || silent_final_e && i + 2 == n - 1);
            let is_final = i == n - 1;
            let p = if is_final && c == 'a' {
                "ə"
            } else if is_final || lengthened {
                long_vowel(c)
            } else if stressed {
                // Unstressed checked vowels reduce toward schwa ("-an", "-on").
                if c == 'i' {
                    "ɪ"
                } else {
                    "ə"
                }
            } else {
                short_vowel(c)
            };
            if !(stressed && (p == "ə" || p == "ɪ")) {
                mark_stress(&mut out, &mut stressed);
            }
            out.push_str(p);
            i += 1;
            continue;
        }
        let next = at(i + 1);
        let soft = next.is_some_and(|x| matches!(x, 'e' | 'i' | 'y'));
        let p: &str = match c {
            'c' if soft => "s",
            'c' => "k",
            'g' if soft && (next != Some('e') || i + 2 >= n) => "ʤ",
            'g' => "ɡ",
            'j' => "ʤ",
            'r' => "ɹ",
            'x' if i == 0 => "z",
            'x' => "ks",
            'q' => "k",
            'y' if i == 0 => "j",
            'y' if i == n - 1 => {
                if stressed {
                    "i"
                } else {
                    mark_stress(&mut out, &mut stressed);
                    "aɪ"
                }
            }
            'y' => {
                mark_stress(&mut out, &mut stressed);
                "ɪ"
            }
            other => {
                // Doubled consonants are said once ("ll", "tt").
                if i > 0 && w[i - 1] == other {
                    ""
                } else {
                    match other {
                        'b' => "b",
                        'd' => "d",
                        'f' => "f",
                        'h' => "h",
                        'k' => "k",
                        'l' => "l",
                        'm' => "m",
                        'n' => "n",
                        'p' => "p",
                        's' => "s",
                        't' => "t",
                        'v' => "v",
                        'w' => "w",
                        'z' => "z",
                        _ => "",
                    }
                }
            }
        };
        out.push_str(p);
        i += 1;
    }
    let out = out.trim().to_string();
    if out.is_empty() || out == "ˈ" {
        None
    } else {
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The live mishears and product names from 2026-10-14 must come out as
    /// a word: no letter names, at least one vowel sound, one stress mark.
    #[test]
    fn unknown_words_read_as_words_not_letters() {
        for word in [
            "upneen", "tabouli", "tavily", "taideta", "zorvane", "kaplin",
        ] {
            let ipa = fallback_ipa(word).unwrap_or_else(|| panic!("{word} has no reading"));
            assert_eq!(
                ipa.matches('ˈ').count(),
                1,
                "{word} → {ipa} needs exactly one primary stress"
            );
            assert!(
                ipa.chars().count() < word.len() * 3,
                "{word} → {ipa} looks letter-spelled"
            );
        }
    }

    #[test]
    fn readings_follow_common_spelling_rules() {
        assert_eq!(fallback_ipa("upneen").as_deref(), Some("ˈʌpniːn"));
        // Silent final e lengthens the vowel before it.
        assert_eq!(fallback_ipa("zane").as_deref(), Some("zˈeɪn"));
        // Soft c, ch digraph.
        assert_eq!(fallback_ipa("cinch").as_deref(), Some("sˈɪnʧ"));
    }

    #[test]
    fn nothing_voiceable_is_none_not_noise() {
        assert_eq!(fallback_ipa("zzqxwv"), None);
        assert_eq!(fallback_ipa(""), None);
        assert_eq!(fallback_ipa("---"), None);
    }

    #[test]
    fn hyphenated_words_are_read_part_by_part() {
        let ipa = fallback_ipa("upneen-tavily").unwrap();
        assert_eq!(ipa.split(' ').count(), 2, "{ipa}");
    }

    #[test]
    fn short_all_caps_tokens_are_acronyms_and_stay_spelled() {
        assert!(is_spelled_acronym("AWS"));
        assert!(is_spelled_acronym("SDK."));
        assert!(!is_spelled_acronym("Tavily"));
        assert!(!is_spelled_acronym("upneen"));
        assert!(!is_spelled_acronym("KUBERNETES"));
    }
}
