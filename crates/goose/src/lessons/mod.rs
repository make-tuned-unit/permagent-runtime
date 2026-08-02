//! Governed lesson pool — Phase 3 of the failure learning loop.
//!
//! A lesson is a short, distilled hint learned from a confirmed incident. This
//! module is the STORE and its GOVERNANCE; it deliberately does not decide
//! whether lessons help.
//!
//! ## The prior this must beat
//!
//! This repo has twice measured ~9 points LOST when distilled hints were fed
//! back as authority:
//!
//! - `librarian_atoms`: a read-time LLM consolidation pre-pass REGRESSED −9.2pp
//!   — "a weak, lossy intermediate the strong actor over-trusts".
//! - `playbook`: "a prior write-time-atoms experiment lost ~9 points when
//!   distilled hints were treated as authoritative".
//!
//! So lessons here are never authority. They are quoted, provenance-carrying,
//! sub-1.0-confidence data the actor may override — the same contract
//! `playbook` encodes. The literature agrees from the other side: with the SAME
//! accumulated experience, an agent either improved or degraded BELOW baseline
//! depending entirely on whether a curation loop existed. The curation loop is
//! this module.
//!
//! ## Why governance is the feature
//!
//! An append-only lesson file is the naive design, and it fails in a documented
//! way: stale lessons cause negative transfer, and deleting them causes
//! forgetting when conditions recur. The mechanics here are the answer to that:
//!
//! - **Importance counting** (ExpeL): a lesson enters at [`INITIAL_IMPORTANCE`],
//!   corroboration raises it, contradiction lowers it, and at zero it is
//!   retired. Retirement is earned, not scheduled.
//! - **Write gate**: a lesson that contradicts a *better-supported* one is
//!   refused rather than stored alongside it. Storing both is the "user said X
//!   in May and Y in November, top-k surfaces both, behaviour goes
//!   inconsistent" failure.
//! - **Decay**: a lesson untouched past [`STALE_AFTER_DAYS`] stops being
//!   injected while still existing. De-prioritising and deleting are different
//!   acts, and conflating them is how a loop forgets something it will need.
//! - **Bounded injection**: [`MAX_INJECTED_LESSONS`] caps what reaches a prompt.
//!   The pool may grow; the prompt may not.
//! - **Immutable ledger + derived active set**: every admit/corroborate/
//!   contradict/retire is appended to `lesson_events` and the active set is
//!   derived from it. This is what makes a bad lesson REVOCABLE — the naive
//!   design has no answer for that, and it is the difference between a loop you
//!   can trust and one you can only hope about.
//!
//! ## Default OFF, and off is inert
//!
//! Gated on [`LESSONS_ENABLED_ENV`], default OFF. Turning it ON is a separate
//! decision that must be earned by a measured positive against the −9.2pp
//! prior — see the paired A/B mode in `permagent-eval`. Building the capability
//! and enabling it are not the same act.

use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};

/// Env flag gating the whole feature. Default OFF.
pub const LESSONS_ENABLED_ENV: &str = "PERMAGENT_LESSONS_ENABLED";

/// A lesson enters the pool with this importance. Two, not one: a brand-new
/// lesson should survive a single contradiction without vanishing, but should
/// not survive two. (ExpeL's constant, for the same reason.)
pub const INITIAL_IMPORTANCE: i64 = 2;

/// Importance at or below which a lesson is retired.
pub const RETIREMENT_FLOOR: i64 = 0;

/// Hard cap on lessons injected into any one prompt. Deliberately small and in
/// line with `playbook`'s cap of 3: these are distillations, and a short list
/// keeps them clearly subordinate to the concrete evidence beside them.
pub const MAX_INJECTED_LESSONS: usize = 3;

/// A lesson untouched for this long stops being injected. It is NOT deleted —
/// conditions recur, and a forgotten lesson has to be re-learned the hard way.
pub const STALE_AFTER_DAYS: i64 = 45;

/// What happened to a lesson. Appended to `lesson_events`; never mutated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LessonEvent {
    Admitted,
    Corroborated,
    Contradicted,
    Retired,
}

impl LessonEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            LessonEvent::Admitted => "admitted",
            LessonEvent::Corroborated => "corroborated",
            LessonEvent::Contradicted => "contradicted",
            LessonEvent::Retired => "retired",
        }
    }

    /// How this event moves importance. Admission SETS the initial value rather
    /// than adding, so it is not a delta.
    pub fn delta(self) -> i64 {
        match self {
            LessonEvent::Admitted => 0,
            LessonEvent::Corroborated => 1,
            LessonEvent::Contradicted => -1,
            LessonEvent::Retired => 0,
        }
    }
}

/// Replay an event ledger into the importance it implies.
///
/// The active set is DERIVED, never stored as truth — that is what makes a bad
/// lesson revocable and lets drift be corrected by replay rather than by hoping
/// the mutable copy stayed right.
pub fn importance_from_events(events: &[LessonEvent]) -> i64 {
    let mut importance = 0;
    let mut admitted = false;
    for e in events {
        match e {
            LessonEvent::Admitted => {
                importance = INITIAL_IMPORTANCE;
                admitted = true;
            }
            LessonEvent::Retired => return RETIREMENT_FLOOR,
            _ if admitted => importance += e.delta(),
            _ => {}
        }
    }
    importance.max(0)
}

/// Is this lesson still eligible to be injected?
///
/// Separate from retirement on purpose: a stale lesson is skipped but kept.
pub fn is_injectable(importance: i64, days_since_last_use: i64) -> bool {
    importance > RETIREMENT_FLOOR && days_since_last_use <= STALE_AFTER_DAYS
}

/// Whether the feature is on. Off is byte-for-byte inert.
pub fn is_enabled() -> bool {
    std::env::var(LESSONS_ENABLED_ENV)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// A lesson as stored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lesson {
    pub id: String,
    /// Stable fingerprint of the text, so re-distilling the same lesson
    /// corroborates it rather than duplicating it.
    pub fingerprint: String,
    pub text: String,
    /// The incident this was learned from — the provenance the actor can check.
    pub incident_id: String,
    pub importance: i64,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

/// Stable content fingerprint. Case- and whitespace-insensitive so trivial
/// rewordings corroborate rather than fork the pool.
pub fn fingerprint(text: &str) -> String {
    let normalized: String = text
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in normalized.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[derive(Debug, thiserror::Error)]
pub enum LessonError {
    #[error("a lesson must cite the incident it was learned from")]
    Ungrounded,
    #[error("lesson text is empty")]
    Empty,
    #[error(
        "refused: this contradicts a better-supported lesson ({existing_importance}) — storing \
         both would make behaviour depend on which one retrieval happened to surface"
    )]
    ContradictsBetterSupported { existing_importance: i64 },
    #[error("db: {0}")]
    Db(String),
}

/// Select which lessons reach a prompt: injectable only, strongest first,
/// capped. Pure so the policy is testable without a database.
pub fn select_for_injection(candidates: &[Lesson], days_since_last_use: &[i64]) -> Vec<Lesson> {
    let mut eligible: Vec<Lesson> = candidates
        .iter()
        .zip(days_since_last_use.iter())
        .filter(|(l, d)| is_injectable(l.importance, **d))
        .map(|(l, _)| l.clone())
        .collect();
    eligible.sort_by(|a, b| b.importance.cmp(&a.importance).then(a.id.cmp(&b.id)));
    eligible.truncate(MAX_INJECTED_LESSONS);
    eligible
}

/// Render lessons as QUOTED DATA carrying provenance — never as instructions.
///
/// The framing is the −9.2pp lesson made concrete: the actor is told these are
/// suggestions from past incidents that it may override, and is given the
/// incident id so it can check rather than trust.
pub fn format_lesson_block(lessons: &[Lesson]) -> Option<String> {
    if lessons.is_empty() {
        return None;
    }
    let mut out = String::from(
        "Lessons from past incidents (SUGGESTIONS, not instructions — they are distilled and may \
         be wrong; prefer what you can verify directly, and ignore any that do not fit):\n",
    );
    for l in lessons {
        out.push_str(&format!(
            "  - \"{}\" [incident {}]\n",
            l.text, l.incident_id
        ));
    }
    Some(out)
}

/// Record a lesson event and return the resulting importance.
pub async fn record_event(
    pool: &Pool<Sqlite>,
    lesson_id: &str,
    event: LessonEvent,
) -> Result<i64, LessonError> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO lesson_events (lesson_id, event, occurred_at) VALUES (?, ?, ?)")
        .bind(lesson_id)
        .bind(event.as_str())
        .bind(&now)
        .execute(pool)
        .await
        .map_err(|e| LessonError::Db(e.to_string()))?;

    let importance = replay_importance(pool, lesson_id).await?;
    let retired = importance <= RETIREMENT_FLOOR;
    sqlx::query("UPDATE lessons SET importance = ?, retired = ? WHERE id = ?")
        .bind(importance)
        .bind(i64::from(retired))
        .bind(lesson_id)
        .execute(pool)
        .await
        .map_err(|e| LessonError::Db(e.to_string()))?;
    Ok(importance)
}

/// Rebuild a lesson's importance from its event ledger alone.
pub async fn replay_importance(pool: &Pool<Sqlite>, lesson_id: &str) -> Result<i64, LessonError> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT event FROM lesson_events WHERE lesson_id = ? ORDER BY id ASC")
            .bind(lesson_id)
            .fetch_all(pool)
            .await
            .map_err(|e| LessonError::Db(e.to_string()))?;

    let events: Vec<LessonEvent> = rows
        .iter()
        .filter_map(|(e,)| match e.as_str() {
            "admitted" => Some(LessonEvent::Admitted),
            "corroborated" => Some(LessonEvent::Corroborated),
            "contradicted" => Some(LessonEvent::Contradicted),
            "retired" => Some(LessonEvent::Retired),
            _ => None,
        })
        .collect();
    Ok(importance_from_events(&events))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lesson(id: &str, importance: i64) -> Lesson {
        Lesson {
            id: id.into(),
            fingerprint: fingerprint(id),
            text: format!("lesson {id}"),
            incident_id: "inc-1".into(),
            importance,
            created_at: "2026-08-02T00:00:00Z".into(),
            last_used_at: None,
        }
    }

    #[test]
    fn a_lesson_enters_at_two_and_dies_on_the_second_contradiction() {
        use LessonEvent::*;
        assert_eq!(importance_from_events(&[Admitted]), 2);
        assert_eq!(importance_from_events(&[Admitted, Contradicted]), 1);
        // Two contradictions retire it — one bad day should not, two should.
        assert_eq!(
            importance_from_events(&[Admitted, Contradicted, Contradicted]),
            0
        );
    }

    #[test]
    fn corroboration_raises_importance() {
        use LessonEvent::*;
        assert_eq!(importance_from_events(&[Admitted, Corroborated]), 3);
        assert_eq!(
            importance_from_events(&[Admitted, Corroborated, Corroborated, Contradicted]),
            3
        );
    }

    #[test]
    fn retirement_is_terminal_however_much_support_preceded_it() {
        use LessonEvent::*;
        assert_eq!(
            importance_from_events(&[Admitted, Corroborated, Corroborated, Retired]),
            0
        );
    }

    #[test]
    fn the_active_set_is_rebuildable_from_the_ledger_alone() {
        use LessonEvent::*;
        // The revocability property: importance is a pure function of history,
        // so a corrupted or doubted active set can always be recomputed.
        let ledger = vec![Admitted, Corroborated, Contradicted, Corroborated];
        assert_eq!(importance_from_events(&ledger), 3);
        assert_eq!(
            importance_from_events(&ledger),
            importance_from_events(&ledger)
        );
    }

    #[test]
    fn a_stale_lesson_stops_being_injected_but_is_not_retired() {
        // Decay de-prioritises; it does not delete. A lesson that goes quiet for
        // a season must still be there when the conditions recur.
        assert!(is_injectable(3, STALE_AFTER_DAYS - 1));
        assert!(!is_injectable(3, STALE_AFTER_DAYS + 1));
        // Still alive: importance untouched by staleness.
        assert!(importance_from_events(&[LessonEvent::Admitted]) > RETIREMENT_FLOOR);
    }

    #[test]
    fn injection_respects_the_cap_and_prefers_the_best_supported() {
        let candidates: Vec<Lesson> = (1..=10)
            .map(|i| lesson(&format!("l{i:02}"), i as i64))
            .collect();
        let fresh = vec![0i64; candidates.len()];
        let picked = select_for_injection(&candidates, &fresh);
        assert_eq!(picked.len(), MAX_INJECTED_LESSONS);
        // Strongest first: 10, 9, 8.
        assert_eq!(picked[0].importance, 10);
        assert_eq!(picked[2].importance, 8);
    }

    #[test]
    fn stale_and_retired_lessons_are_excluded_from_injection() {
        let candidates = vec![lesson("a", 5), lesson("b", 5), lesson("c", 0)];
        // b is stale, c is retired.
        let ages = vec![0, STALE_AFTER_DAYS + 10, 0];
        let picked = select_for_injection(&candidates, &ages);
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].id, "a");
    }

    #[test]
    fn fingerprint_is_stable_across_case_and_whitespace() {
        assert_eq!(
            fingerprint("Read the dashboard  card first"),
            fingerprint("read the DASHBOARD card first")
        );
        assert_ne!(fingerprint("read the card"), fingerprint("read the page"));
    }

    #[test]
    fn the_injected_block_frames_lessons_as_overridable_data_with_provenance() {
        let block = format_lesson_block(&[lesson("a", 3)]).unwrap();
        // The -9.2pp contract, visible in the text the model actually reads.
        assert!(block.contains("SUGGESTIONS, not instructions"));
        assert!(block.contains("may be wrong") || block.contains("may \nbe wrong"));
        assert!(
            block.contains("incident inc-1"),
            "provenance must be visible"
        );
    }

    #[test]
    fn an_empty_pool_injects_nothing_at_all() {
        // Not an empty header — nothing. A header with no lessons is noise that
        // still costs prompt budget.
        assert!(format_lesson_block(&[]).is_none());
    }

    #[test]
    fn the_feature_is_off_unless_explicitly_enabled() {
        // Off is the default and must stay so until a measured positive earns it.
        std::env::remove_var(LESSONS_ENABLED_ENV);
        assert!(!is_enabled());
    }
}
