//! Spoken approve/reject while talking to Henry.
//!
//! The voice socket is the user's own authenticated channel — the same as
//! tapping Approve in chat — so a short "yes"/"no" can settle a pending
//! decision without sending the transcript to the model (which would be
//! Henry's word, not the user's hand).

/// Map a short spoken utterance to approve/reject. Longer sentences fall
/// through to the agent so "yes, and also log a meeting" is never swallowed.
pub fn spoken_decision_verdict(transcript: &str) -> Option<&'static str> {
    let t = transcript
        .trim()
        .trim_end_matches(['.', '!', ',', '?'])
        .trim()
        .to_ascii_lowercase();
    match t.as_str() {
        "yes" | "yeah" | "yep" | "yup" | "ok" | "okay" | "sure" | "approve" | "approved"
        | "go ahead" | "do it" | "please" | "yes please" | "yes go ahead" | "sounds good"
        | "that's fine" | "thats fine" | "go for it" | "do that" => Some("approve"),
        "no" | "nope" | "reject" | "denied" | "don't" | "dont" | "skip it" | "send back"
        | "no thanks" | "nah" | "not now" => Some("reject"),
        _ => None,
    }
}

/// Kinds the spoken path can answer with a bare yes/no.
pub fn is_binary_kind(kind: &str) -> bool {
    !matches!(kind, "choice" | "unblock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_short_yes_and_no() {
        assert_eq!(spoken_decision_verdict("Yes."), Some("approve"));
        assert_eq!(spoken_decision_verdict("  go ahead  "), Some("approve"));
        assert_eq!(spoken_decision_verdict("nope"), Some("reject"));
        assert_eq!(spoken_decision_verdict("send back"), Some("reject"));
    }

    #[test]
    fn leaves_real_sentences_to_the_agent() {
        assert_eq!(spoken_decision_verdict("yes, and add a meeting"), None);
        assert_eq!(
            spoken_decision_verdict("approve the enrichment for Jane"),
            None
        );
        assert_eq!(spoken_decision_verdict(""), None);
    }
}
