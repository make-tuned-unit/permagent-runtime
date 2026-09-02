//! Retained negatives — a decline is knowledge, not an absence.
//!
//! Adapted from the Omnia Vault review (gavishap/omnia-vault, 2026-08-28): a
//! confident PASS is a win there, and the brief that produced it is *kept*
//! precisely so the same input is not re-litigated next month; a rejected tool
//! keeps its ledger row because "the why is knowledge". We already had half of
//! this — `recognition::mark_observation_bounced` records an initiative
//! automation decline so the same command is never re-pitched. This module
//! extends that one mechanism to the other declinable proposal kinds rather
//! than growing a second store beside it.
//!
//! Two reads, one durable record:
//!
//! * the **gate** — [`was_declined`] asks the recognition tables, under a
//!   namespaced `declined:<kind>:<subject>` key, whether this exact proposal
//!   was already turned down. Used before re-filing a proposal.
//! * the **brief** — [`list_recent`] reads the answered `decisions` rows
//!   themselves (the store the Council brief already reads through
//!   `crate::decisions`), so the chair sees what was declined *and why*. The
//!   user's note lives there and nowhere else, so that is where the reason is
//!   read from; nothing is copied.
//!
//! TEPA: every retained negative carries `created_at`, and [`RetainedNegative::age_days`]
//! turns it into an age. That is deliberately all — an expiry policy can be
//! layered on the key + timestamp without touching this module, and none is
//! imposed here.

use sqlx::{Pool, Row, Sqlite};

use super::learn::sanitize_key_part;

/// Key prefix for retained negatives — the namespace under which declines are
/// recorded in the recognition tables, distinct from the raw normalized
/// commands the Initiative layer bounces.
pub const NEGATIVE_KEY_PREFIX: &str = "declined:";

/// Recognition lane for Decision-Inbox declines (see
/// `recognition::mark_observation_bounced_in_lane`).
pub const NEGATIVE_LANE: &str = "decision-inbox";

/// How many retained negatives a brief carries by default. Small on purpose:
/// this is a "do not re-propose" reminder, not a history tab.
pub const BRIEF_NEGATIVE_LIMIT: i64 = 8;

/// The proposal kinds a Council brief surfaces declines for — the ones whose
/// whole failure mode is being re-proposed verbatim next week.
pub const BRIEF_NEGATIVE_KINDS: &[&str] = &["council_action", "project_intel_proposal"];

/// The key a decline is recorded under. Case-folded and sanitized so
/// "Rewrite the homepage" and "rewrite the homepage" are one negative, and so
/// a colon in either part cannot forge another key.
pub fn negative_key(kind: &str, subject: &str) -> String {
    format!(
        "{}{}:{}",
        NEGATIVE_KEY_PREFIX,
        sanitize_key_part(kind),
        sanitize_key_part(&subject.to_lowercase())
    )
}

/// One declined proposal, as the next brief assembly sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainedNegative {
    /// `declined:<kind>:<subject>` — what [`was_declined`] would look up.
    pub key: String,
    pub kind: String,
    /// The proposal's plain-language headline.
    pub subject: String,
    /// The user's decline note, when they left one — the *why*.
    pub note: Option<String>,
    /// When it was declined (falls back to creation time on old rows). The
    /// TEPA timestamp: enough to expire a negative later, without an expiry
    /// policy being built now.
    pub created_at: String,
}

impl RetainedNegative {
    /// Age in whole days, or `None` when the stored timestamp is unparseable.
    pub fn age_days(&self, now: chrono::DateTime<chrono::Utc>) -> Option<i64> {
        let then = chrono::DateTime::parse_from_rfc3339(&self.created_at).ok()?;
        Some((now - then.with_timezone(&chrono::Utc)).num_days())
    }

    /// One brief line: date, subject, key, and the reason when there is one.
    pub fn render(&self) -> String {
        let day = self
            .created_at
            .split('T')
            .next()
            .unwrap_or(&self.created_at);
        let mut line = format!("- [{day}] \"{}\" (`{}`)", self.subject, self.key);
        match self
            .note
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            Some(note) => line.push_str(&format!(" — declined: {note}")),
            None => line.push_str(" — declined, no reason given"),
        }
        line
    }
}

/// Record a decline against `kind`/`subject`, extending the recognition-bounce
/// precedent under the [`NEGATIVE_KEY_PREFIX`] namespace so
/// `recognition::seen_observation` reports it exactly as it reports an
/// initiative bounce. The *reason* is not copied here — it already lives on the
/// answered decision row, which is what [`list_recent`] reads.
///
/// Best-effort, like the mechanism it extends: a failure is logged inside
/// `recognition`, never propagated into the effect path.
pub async fn record_decline(pool: &Pool<Sqlite>, kind: &str, subject: &str) {
    let subject = subject.trim();
    if subject.is_empty() {
        return;
    }
    crate::recognition::mark_observation_bounced_in_lane(
        pool,
        NEGATIVE_LANE,
        &negative_key(kind, subject),
    )
    .await;
}

/// True when this exact proposal was already declined — the anti-re-pitch gate.
/// A read failure degrades to `false` (propose it), never to an error path.
pub async fn was_declined(pool: &Pool<Sqlite>, kind: &str, subject: &str) -> bool {
    let subject = subject.trim();
    if subject.is_empty() {
        return false;
    }
    crate::recognition::seen_observation(pool, &negative_key(kind, subject))
        .await
        .is_some_and(|seen| seen.was_bounced())
}

/// The most recent declines across `kinds`, newest first, for brief assembly.
///
/// Reads the `decisions` table directly — the store a brief already consults —
/// so the note the user typed at decline time is the note the chair reads.
/// Best-effort: any failure yields an empty list, matching how every other
/// brief section degrades.
pub async fn list_recent(pool: &Pool<Sqlite>, kinds: &[&str], limit: i64) -> Vec<RetainedNegative> {
    if kinds.is_empty() || limit <= 0 {
        return Vec::new();
    }
    let placeholders = vec!["?"; kinds.len()].join(",");
    let sql = format!(
        "SELECT kind, headline, answer_note, \
                COALESCE(resolved_at, created_at) AS declined_at \
           FROM decisions \
          WHERE status = 'answered' AND answer = 'reject' AND kind IN ({placeholders}) \
          ORDER BY declined_at DESC \
          LIMIT ?"
    );
    // Audited: the only interpolation is a run of `?` placeholders derived
    // from `kinds.len()`. Every kind value is bound, never inlined.
    let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
    for kind in kinds {
        query = query.bind(*kind);
    }
    let rows = match query.bind(limit.clamp(1, 100)).fetch_all(pool).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::debug!(
                target: "permagent::decision_inbox",
                "retained negatives unavailable: {e}"
            );
            return Vec::new();
        }
    };
    rows.iter()
        .map(|r| {
            let kind: String = r.get("kind");
            let subject: String = r.get("headline");
            RetainedNegative {
                key: negative_key(&kind, &subject),
                kind,
                subject,
                note: r.get("answer_note"),
                created_at: r.get("declined_at"),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::spectral_schema::init_spectral_db;

    async fn pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    async fn declined(pool: &Pool<Sqlite>, kind: &str, headline: &str, note: Option<&str>) {
        let payload = match kind {
            "council_action" => serde_json::json!({
                "session_id": "sess-1", "title": headline, "description": "d",
            }),
            // Payloads must validate: `create_decision` silently rewrites an
            // invalid one to kind='malformed', which would quietly hollow out
            // this fixture.
            _ => serde_json::json!({
                "project_id": "p-1",
                "project_name": "Permagent",
                "items": [{
                    "kind": "competitor",
                    "name": headline,
                    "source_url": "https://example.com/acme",
                }],
            }),
        };
        let d = crate::decisions::create_decision(
            pool,
            crate::decisions::NewDecision {
                kind: kind.to_string(),
                goal_id: None,
                project_id: None,
                headline: Some(headline.to_string()),
                detail: Some("detail".to_string()),
                payload,
                rank: Some(0.5),
                action_class: Some(kind.to_string()),
            },
        )
        .await
        .unwrap();
        crate::decisions::answer_decision(
            pool,
            &d.id,
            &crate::decisions::DecisionAnswer {
                answer: "reject".to_string(),
                note: note.map(str::to_string),
                choice_id: None,
                input_text: None,
            },
            crate::decisions::ACTOR_JESSE,
        )
        .await
        .unwrap();
    }

    #[test]
    fn key_is_namespaced_case_folded_and_sanitized() {
        assert_eq!(
            negative_key("council_action", "Rewrite the homepage"),
            "declined:council_action:rewrite-the-homepage"
        );
        // Case and surrounding whitespace collapse to one negative.
        assert_eq!(
            negative_key("council_action", "  REWRITE the Homepage "),
            negative_key("council_action", "Rewrite the homepage")
        );
        // Namespaced away from the Initiative layer's raw command keys.
        assert!(negative_key("k", "s").starts_with(NEGATIVE_KEY_PREFIX));
        // A colon in either part cannot forge another key.
        assert_eq!(negative_key("a:b", "c:d"), "declined:a-b:c-d");
    }

    #[tokio::test]
    async fn a_recorded_decline_is_seen_by_the_gate_and_scoped_to_its_subject() {
        let pool = pool().await;
        assert!(!was_declined(&pool, "council_action", "Rewrite the homepage").await);
        record_decline(&pool, "council_action", "Rewrite the homepage").await;
        assert!(was_declined(&pool, "council_action", "Rewrite the homepage").await);
        // Case-insensitive on the same subject...
        assert!(was_declined(&pool, "council_action", "rewrite the HOMEPAGE").await);
        // ...but never bleeds onto another subject or another kind.
        assert!(!was_declined(&pool, "council_action", "Ship the pricing page").await);
        assert!(!was_declined(&pool, "project_intel_proposal", "Rewrite the homepage").await);
        // Empty subjects are not recordable and never gate.
        record_decline(&pool, "council_action", "   ").await;
        assert!(!was_declined(&pool, "council_action", "   ").await);
    }

    /// RED-FIRST (c) as a standing guard: generalizing the bounce into lanes
    /// must leave the Initiative layer's rows exactly as they were.
    #[tokio::test]
    async fn initiative_bounce_rows_are_unchanged_by_the_lane_split() {
        let pool = pool().await;
        crate::recognition::mark_observation_bounced(&pool, "npm run build").await;

        let row = sqlx::query(
            "SELECT retrieval_id, session_id, strategy, rc_persona, outcome_kind, \
                    outcome_polarity, outcome_source, outcome_label \
               FROM recognition_events WHERE query = ?",
        )
        .bind("npm run build")
        .fetch_one(&pool)
        .await
        .unwrap();
        let retrieval_id: String = row.get("retrieval_id");
        assert!(
            retrieval_id.starts_with("initiative-decline:"),
            "{retrieval_id}"
        );
        assert_eq!(row.get::<String, _>("session_id"), "initiative");
        assert_eq!(row.get::<String, _>("strategy"), "initiative");
        assert_eq!(row.get::<String, _>("rc_persona"), "henry");
        assert_eq!(row.get::<String, _>("outcome_kind"), "DecisionBounced");
        assert_eq!(row.get::<String, _>("outcome_polarity"), "Negative");
        assert_eq!(row.get::<String, _>("outcome_source"), "Decision");
        assert_eq!(row.get::<String, _>("outcome_label"), "wrong");
        // And the read side still reports it.
        assert!(crate::recognition::seen_observation(&pool, "npm run build")
            .await
            .unwrap()
            .was_bounced());

        // A Decision-Inbox decline lands in its own lane, not masquerading as
        // the initiative loop.
        record_decline(&pool, "council_action", "Rewrite the homepage").await;
        let lane: String =
            sqlx::query_scalar("SELECT strategy FROM recognition_events WHERE query = ?")
                .bind(negative_key("council_action", "Rewrite the homepage"))
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(lane, NEGATIVE_LANE);
    }

    /// End-to-end through the real effect path: rejecting a `council_action`
    /// on the Decision Inbox is what mints the negative, exactly as rejecting
    /// an `automation_proposal` mints an initiative bounce.
    #[tokio::test]
    async fn rejecting_a_council_action_mints_the_negative_through_the_effect_path() {
        let pool = pool().await;
        let d = crate::decisions::create_decision(
            &pool,
            crate::decisions::NewDecision {
                kind: "council_action".to_string(),
                goal_id: None,
                project_id: None,
                headline: Some("Rewrite the homepage".to_string()),
                detail: Some("detail".to_string()),
                payload: serde_json::json!({
                    "session_id": "sess-1",
                    "title": "Rewrite the homepage",
                    "description": "d",
                }),
                rank: Some(0.6),
                action_class: Some("council_action".to_string()),
            },
        )
        .await
        .unwrap();
        assert!(!was_declined(&pool, "council_action", "Rewrite the homepage").await);

        let (answered, proof) = crate::decisions::answer_decision(
            &pool,
            &d.id,
            &crate::decisions::DecisionAnswer {
                answer: "reject".to_string(),
                note: Some("we already tried this in June".to_string()),
                choice_id: None,
                input_text: None,
            },
            crate::decisions::ACTOR_JESSE,
        )
        .await
        .unwrap();
        crate::decisions_effects::apply_decision_effect(&pool, &answered, proof, "council_action")
            .await
            .unwrap();

        assert!(was_declined(&pool, "council_action", "Rewrite the homepage").await);
        let retained = list_recent(&pool, &["council_action"], 10).await;
        assert_eq!(retained.len(), 1);
        assert_eq!(
            retained[0].note.as_deref(),
            Some("we already tried this in June")
        );
    }

    #[tokio::test]
    async fn list_recent_retains_the_reason_and_ignores_other_outcomes() {
        let pool = pool().await;
        declined(
            &pool,
            "council_action",
            "Rewrite the homepage",
            Some("we already tried this in June"),
        )
        .await;
        declined(
            &pool,
            "project_intel_proposal",
            "Add Acme as a competitor",
            None,
        )
        .await;

        // An OPEN proposal is not a negative.
        crate::decisions::create_decision(
            &pool,
            crate::decisions::NewDecision {
                kind: "council_action".to_string(),
                goal_id: None,
                project_id: None,
                headline: Some("Still open".to_string()),
                detail: Some("d".to_string()),
                payload: serde_json::json!({
                    "session_id": "s", "title": "Still open", "description": "d",
                }),
                rank: None,
                action_class: Some("council_action".to_string()),
            },
        )
        .await
        .unwrap();

        let out = list_recent(&pool, BRIEF_NEGATIVE_KINDS, BRIEF_NEGATIVE_LIMIT).await;
        assert_eq!(out.len(), 2, "{out:#?}");
        assert!(out.iter().all(|n| n.subject != "Still open"));

        let homepage = out
            .iter()
            .find(|n| n.subject == "Rewrite the homepage")
            .unwrap();
        assert_eq!(homepage.kind, "council_action");
        assert_eq!(
            homepage.key,
            negative_key("council_action", "Rewrite the homepage")
        );
        assert_eq!(
            homepage.note.as_deref(),
            Some("we already tried this in June")
        );
        // TEPA: a timestamp is retained, and it reads back as an age.
        assert!(!homepage.created_at.is_empty());
        assert_eq!(homepage.age_days(chrono::Utc::now()), Some(0));
        assert!(homepage.render().contains("we already tried this in June"));

        let intel = out
            .iter()
            .find(|n| n.subject == "Add Acme as a competitor")
            .unwrap();
        assert!(intel.render().contains("no reason given"));

        // Kind filtering is real.
        let only_council = list_recent(&pool, &["council_action"], 10).await;
        assert_eq!(only_council.len(), 1);
        assert!(list_recent(&pool, &[], 10).await.is_empty());
        assert!(list_recent(&pool, BRIEF_NEGATIVE_KINDS, 0).await.is_empty());
    }
}
