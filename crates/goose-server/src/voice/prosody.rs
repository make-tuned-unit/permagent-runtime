//! Kokoro-native delivery: tags the model may emit, mapped to speed and pauses.
//!
//! Kokoro has no emotion embedding. What it *does* have is a speed input,
//! punctuation that shapes intonation (`!` `?` `...` `—`), and a willingness
//! to pause on ellipses. This module is the closed tag set the voice prompt
//! is allowed to write, stripped before synthesis so brackets are never spoken.
//!
//! Unknown `[tags]` are dropped too — a model inventing `[whisper]` must not
//! produce the word "whisper" on the way to the speaker (speakable() would
//! otherwise turn `[whisper]` into `whisper` by stripping the brackets).

/// Speed for a sentence with no delivery tag, driven by its closing punctuation.
pub fn speed_from_punctuation(text: &str) -> f32 {
    let trimmed = text.trim();
    if trimmed.ends_with('!') {
        1.08
    } else if trimmed.ends_with('?') {
        1.04
    } else {
        1.0
    }
}

/// Map a recognised delivery tag (lowercase, no brackets) to a speed.
fn speed_for_tag(tag: &str) -> Option<f32> {
    Some(match tag {
        "excited" => 1.12,
        "warm" => 1.02,
        "calm" => 0.92,
        "gentle" | "soft" => 0.88,
        "serious" => 0.94,
        _ => return None,
    })
}

/// A sentence ready for Kokoro: tags gone, pauses turned into ellipses, speed set.
#[derive(Debug, Clone, PartialEq)]
pub struct ProsodyPlan {
    pub speech: String,
    pub speed: f32,
}

/// Strip delivery tags, turn `[pause]` into a Kokoro ellipsis beat, pick speed.
pub fn plan(text: &str) -> ProsodyPlan {
    let mut speed: Option<f32> = None;
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(open) = rest.find('[') {
        // `get` rather than byte-indexing: these offsets come from `find` so
        // they are char boundaries, but clippy::string_slice is denied in CI.
        out.push_str(rest.get(..open).unwrap_or(""));
        let after = rest.get(open + 1..).unwrap_or("");
        match after.find(']') {
            None => {
                // Unclosed bracket — drop it so Kokoro never reads "[".
                rest = after;
            }
            Some(close) => {
                let inner = after.get(..close).unwrap_or("").trim();
                let key = inner.to_ascii_lowercase();
                rest = after.get(close + 1..).unwrap_or("");
                if key == "pause" && !out.ends_with('.') && !out.ends_with('…') {
                    out.push_str("... ");
                } else {
                    speed = speed.or_else(|| speed_for_tag(&key));
                }
                // Unknown tags (and recognised ones) are dropped, not spoken.
            }
        }
    }
    out.push_str(rest);

    let speech = collapse_ws(&out);
    let speed = speed.unwrap_or_else(|| speed_from_punctuation(&speech));
    ProsodyPlan { speech, speed }
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = true;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_a_leading_delivery_tag_and_sets_speed() {
        let p = plan("[excited] We shipped it!");
        assert_eq!(p.speech, "We shipped it!");
        assert_eq!(p.speed, 1.12);
    }

    #[test]
    fn pause_becomes_an_ellipsis_kokoro_already_honours() {
        let p = plan("Wait[pause]there it is.");
        assert_eq!(p.speech, "Wait... there it is.");
        assert_eq!(p.speed, 1.0);
    }

    #[test]
    fn unknown_tags_are_dropped_not_spoken() {
        let p = plan("[whisper] Keep this between us.");
        assert_eq!(p.speech, "Keep this between us.");
        assert!(!p.speech.to_lowercase().contains("whisper"));
    }

    #[test]
    fn unclosed_bracket_is_dropped() {
        let p = plan("Hello [oops there");
        assert_eq!(p.speech, "Hello oops there");
    }

    #[test]
    fn punctuation_drives_speed_when_no_tag() {
        assert_eq!(plan("Yes!").speed, 1.08);
        assert_eq!(plan("Really?").speed, 1.04);
        assert_eq!(plan("Okay.").speed, 1.0);
    }

    #[test]
    fn first_delivery_tag_wins() {
        let p = plan("[calm] [excited] No rush.");
        assert_eq!(p.speech, "No rush.");
        assert_eq!(p.speed, 0.92);
    }

    #[test]
    fn empty_after_tags_is_empty_speech() {
        let p = plan("[excited][pause]");
        assert_eq!(p.speech, "...");
    }
}
