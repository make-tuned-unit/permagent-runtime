//! Name-syllable IPA for `save_pronunciation` when a part is not a dictionary word.
//!
//! misaki has no espeak fallback. Any token missing from its 390k dictionary
//! is spelled letter by letter, and `phonemize_text` refuses the save. That is
//! correct for invented junk (`jent`) and fatal for real names (`peth` in
//! Elspeth, `taran`, `barty`) — last night those three could not be stored.
//!
//! A part listed here is spoken as a syllable, not spelled. Keep this table
//! small and name-shaped; product compounds belong in `technical_lexicon`.

/// Lowercased syllable → Kokoro IPA (GB). Stress marks optional on short bits.
pub fn ipa_for(part: &str) -> Option<&'static str> {
    Some(match part.trim().to_ascii_lowercase().as_str() {
        // Princess Elspeth — /ˈɛlspɛθ/
        "elspeth" => "ˈɛlspɛθ",
        "peth" | "speth" => "pɛθ",
        "els" | "ells" => "ɛlz",
        // Taran the Pigkeeper — /ˈtærən/ (Prydain)
        "taran" => "tˈærən",
        "tarun" => "tˈɑːrən",
        // Barty the Troll
        "barty" => "bˈɑːti",
        "pigkeeper" => "pˈɪɡkiːpə",
        // Prydain (STT last night heard "Prideine")
        "prydain" | "prideine" => "prˈɪdaɪn",
        "prid" => "prɪd",
        "ayn" => "aɪn",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_night_names_have_syllables() {
        assert!(ipa_for("Elspeth").is_some());
        assert!(ipa_for("peth").is_some());
        assert!(ipa_for("Taran").is_some());
        assert!(ipa_for("Barty").is_some());
        assert!(ipa_for("Prydain").is_some());
        assert!(ipa_for("Prideine").is_some());
        assert!(
            ipa_for("jent").is_none(),
            "invented junk must still be refused"
        );
    }
}
