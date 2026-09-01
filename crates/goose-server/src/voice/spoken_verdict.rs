//! Spoken approve/reject while talking to Henry.
//!
//! **Voice proposes; a tap commits.** A short "yes"/"no" is caught here rather
//! than sent to the model (which would be Henry's word, not the user's), but it
//! does not settle anything: it is STAGED against the still-open decision, and
//! the answer happens only when someone taps Commit on the confirm surface.
//!
//! This is not caution about accuracy — it is the standard. NIST SP 800-63B-4
//! §3.2.3.2: "Biometric comparison based on voice SHALL NOT be used"; §3.2.3:
//! a biometric may never stand alone, only alongside a physical authenticator.
//! A microphone cannot tell whose mouth it is, an enrolled voiceprint would not
//! fix that, and a cloned "yes" is a commodity attack (FBI IC3 I-051525-PSA).
//! The tap on an unlocked device IS the second factor, so the authority lives
//! there and nowhere else.

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

/// Kinds a bare yes/no can attach to. `choice` needs an option named and
/// `unblock` invites a written reply, so neither is a binary verdict.
///
/// This is a targeting filter, not an authority gate: what the spoken verdict
/// buys is a staged proposal, which is why it can safely span every tier.
pub fn is_binary_kind(kind: &str) -> bool {
    !matches!(kind, "choice" | "unblock")
}

/// What the socket says back after staging. Never claims the answer happened —
/// the whole point is that it has not.
pub fn staged_reply(verdict: &str, headline: &str) -> String {
    let word = if verdict == "approve" {
        "Approve"
    } else {
        "Reject"
    };
    format!("Staged {word} for {headline}. Confirm on screen to commit it.")
}

/// Record a spoken verdict against an open decision WITHOUT answering it, and
/// return the sentence to speak back.
///
/// The only decision-writing call the voice socket may make. It reaches
/// `stage_answer`, which cannot resolve a decision, run an effect, or append an
/// audit row; `apply_jesse_answer` is deliberately out of reach from here (see
/// `the_voice_route_cannot_reach_the_answer_path` below).
pub async fn stage_spoken_verdict(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    decision_id: &str,
    verdict: &str,
) -> Result<String, String> {
    let decision = permagent::decisions::get_decision(pool, decision_id)
        .await?
        .ok_or_else(|| format!("decision '{decision_id}' no longer exists"))?;
    permagent::decisions::stage_answer(pool, decision_id, verdict, None, "voice")
        .await
        .map_err(|e| e.to_string())?;
    Ok(staged_reply(verdict, &decision.headline))
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

    /// The enforcement pin (D29). The spoken path may reach `stage_answer` and
    /// nothing else; restoring the old `apply_jesse_answer` call — the one that
    /// let a bare "yes" clear a Tier-2 risk gate as jesse, audited to the
    /// literal principal "voice" — fails here.
    #[test]
    fn the_voice_route_cannot_reach_the_answer_path() {
        let route = include_str!("../routes/voice.rs");
        assert!(
            !route.contains("apply_jesse_answer"),
            "the voice socket must never call the answer path — a spoken \
             verdict is a proposal (NIST SP 800-63B-4 §3.2.3.2), and the tap on \
             the confirm surface is what authenticates it"
        );
        assert!(
            route.contains("stage_spoken_verdict"),
            "the spoken verdict must still be captured — staged, not dropped"
        );
    }

    async fn test_pool() -> sqlx::Pool<sqlx::Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        permagent::session::spectral_schema::init_spectral_db(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn a_spoken_yes_stages_a_tier2_decision_and_says_so() {
        let pool = test_pool().await;
        let d = permagent::decisions::create_decision(
            &pool,
            permagent::decisions::NewDecision {
                kind: "risk_gate".to_string(),
                project_id: Some(permagent::projects::PERSONAL_PROJECT_ID.to_string()),
                headline: Some("Allow a shell command to run".to_string()),
                detail: Some("cc_shell: rm -rf ./build".to_string()),
                payload: serde_json::json!({
                    "action_class": "cc_shell",
                    "summary": "remove the build directory",
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(d.tier, 2, "sanity: the tier voice used to be able to clear");

        let spoken = stage_spoken_verdict(&pool, &d.id, "approve").await.unwrap();
        assert!(
            spoken.contains("Staged") && spoken.contains("Confirm on screen"),
            "the reply must not claim the answer happened: {spoken}"
        );
        assert!(
            !spoken.contains("Approved:"),
            "the old copy asserted a completed approval: {spoken}"
        );

        let after = permagent::decisions::get_decision(&pool, &d.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.status, "open", "voice answered nothing");
        assert!(after.acted_by.is_none());
        assert_eq!(
            after.staged_answer.as_ref().map(|s| s.staged_via.as_str()),
            Some("voice")
        );
    }
}
