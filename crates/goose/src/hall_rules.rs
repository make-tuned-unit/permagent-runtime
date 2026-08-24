//! Hall rules — the structural classifier that files a memory by SHAPE.
//!
//! Halls are Spectral's coarse "what kind of record is this" axis (fact,
//! event, …), separate from the per-project wing axis in [`crate::wing_rules`].
//!
//! ## Why this module exists
//!
//! Measured on the live brain: 82 approval memories were filed into the
//! `event` hall. They are not events — they are a durable record of a question
//! the agent asked and the answer it got, which is a `fact`. A hall is a
//! retrieval axis, so 82 misfiled records are 82 records that the fact-shaped
//! query cannot reach.
//!
//! The rule that fixes them is structural, not lexical:
//!
//! ```text
//! was asked:.*answered:   →   fact
//! ```
//!
//! On the live brain that regex matches exactly those 82 records and nothing
//! else. The obvious alternative — a word match on `approve|rejected` — was
//! measured and REJECTED: it also swallows `Task [completed]` records, which
//! genuinely are events. A shape that only approval records have beats a word
//! that approval records merely tend to contain.
//!
//! ## The trap this module exists to make unrepeatable
//!
//! `BrainConfig::hall_rules` is REPLACE, not merge — Spectral resolves it as
//! `config.hall_rules.unwrap_or_else(default_hall_rule_strings)`. So passing
//! our one rule alone would silently DELETE every default hall rule, and not
//! passing it at all means our rule never runs. Both failure modes are silent.
//!
//! [`hall_rules`] is therefore the only supported way to build the list: it
//! returns our rule FIRST (first match wins) followed by every Spectral
//! default, so the rule can never be supplied without them.

/// The structural shape of an approval record: a question the agent asked and
/// the answer it got back. Deliberately anchored on both halves — `was asked:`
/// alone would match prose that merely narrates having asked something.
pub const APPROVAL_SHAPE_PATTERN: &str = r"was asked:.*answered:";

/// The hall an approval record belongs in. It is durable knowledge of what was
/// decided, not a thing that happened at a moment.
pub const APPROVAL_HALL: &str = "fact";

/// The hall rule set to open the Brain with: our structural rules first, then
/// every Spectral default.
///
/// ALWAYS build the list through this function. Passing a bare `vec![...]` to
/// `BrainBuilder::hall_rules` replaces the defaults entirely (see the module
/// docs) — a silent regression with no compile error and no log line.
pub fn hall_rules() -> Vec<(String, String)> {
    let mut rules = spectral::ingest::default_hall_rule_strings();
    // First match wins, so ours must precede the defaults — one of which
    // currently claims these records for `event`.
    rules.insert(
        0,
        (
            APPROVAL_SHAPE_PATTERN.to_string(),
            APPROVAL_HALL.to_string(),
        ),
    );
    rules
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(pattern: &str, text: &str) -> bool {
        regex::Regex::new(pattern)
            .expect("hall pattern must compile")
            .is_match(&text.to_lowercase())
    }

    /// The shape the live brain's 82 misfiled approvals actually have.
    #[test]
    fn approval_shape_matches_a_real_approval_record() {
        for text in [
            "The user was asked: may I delete the merged worktree? and answered: yes",
            "The user was asked: approve the spend cap raise? — answered: no, hold it",
            "was asked: should the Steward sweep tonight? answered: not tonight",
        ] {
            assert!(
                matches(APPROVAL_SHAPE_PATTERN, text),
                "approval record must classify as {APPROVAL_HALL}: {text}"
            );
        }
    }

    /// THE MEASURED OVER-MATCH. A word rule on approve|rejected also claims
    /// completion records, which are genuinely events. The structural rule
    /// must leave them alone — this is why the pattern is what it is.
    #[test]
    fn approval_shape_does_not_claim_task_completion_records() {
        for text in [
            "Task [completed] — rejected the stale branch cleanup",
            "Task [completed]: approved deploy finished in 4m",
            "Henry was asked to look at the build",
            "answered: yes",
        ] {
            assert!(
                !matches(APPROVAL_SHAPE_PATTERN, text),
                "must NOT be reclassified as {APPROVAL_HALL}: {text}"
            );
        }
    }

    /// `hall_rules` is REPLACE, not merge. If this helper ever returns only our
    /// rule, every Spectral default hall silently stops classifying — with no
    /// error and no log line. Pin the shape: ours first, then all the defaults.
    #[test]
    fn helper_prepends_our_rule_and_keeps_every_default() {
        let defaults = spectral::ingest::default_hall_rule_strings();
        let rules = hall_rules();

        assert_eq!(
            rules.len(),
            defaults.len() + 1,
            "the defaults must survive — hall_rules replaces, it does not merge"
        );
        assert_eq!(
            rules[0],
            (
                APPROVAL_SHAPE_PATTERN.to_string(),
                APPROVAL_HALL.to_string()
            ),
            "first match wins, so our rule must be first"
        );
        assert_eq!(
            &rules[1..],
            defaults.as_slice(),
            "the defaults must follow, in their original order"
        );
    }

    /// A DOCUMENTED LIMIT, not an oversight. `.` does not cross a newline
    /// without the `s` flag, so an approval record whose answer sits on its own
    /// line is NOT reclassified. The pattern is left exactly as measured — it
    /// matched 82 records and over-matched none on the live brain — because
    /// widening it with `(?s)` would change a rule whose precision was measured
    /// rather than argued. If multi-line approval records turn out to exist,
    /// re-measure the `(?s)` variant's match count before adopting it.
    #[test]
    fn newline_separated_answer_is_a_known_gap_in_the_measured_pattern() {
        assert!(!matches(
            APPROVAL_SHAPE_PATTERN,
            "was asked: sweep tonight?\nanswered: not tonight"
        ));
    }

    #[test]
    fn every_rule_compiles() {
        for (pattern, hall) in hall_rules() {
            assert!(
                regex::Regex::new(&pattern).is_ok(),
                "hall rule for {hall} does not compile: {pattern}"
            );
        }
    }
}
