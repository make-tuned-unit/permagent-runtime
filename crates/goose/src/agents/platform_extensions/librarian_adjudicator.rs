//! Staleness adjudication for the Librarian — the Permagent half of Spectral's
//! supersession seam.
//!
//! Spectral structurally detects `(subject, predicate)` slots holding several
//! LIVE objects and asks a pluggable Adjudicator a closed question: did the
//! newer value replace the older, or do both hold? This module is the
//! Librarian's adjudicator, backed by the local 7B — **shadow-mode first** and
//! **predicate-gated**.
//!
//! WIRING NOTE: Spectral's `Adjudicator` trait / `SupersessionCandidate` /
//! `apply_adjudications` are NOT yet in our pinned Spectral (`362eadb`). This
//! module is the **pin-independent core** — predicate cardinality + the
//! closed-question 7B prompt + a validating verdict parser + the shadow flag —
//! so that once we bump to the Spectral SHA that has the seam, wiring
//! `impl Adjudicator for LibrarianAdjudicator` is a thin adapter
//! (`SupersessionCandidate` → [`adjudicate_candidate`] → `Adjudication`).
//!
//! SAFETY (mirrors Spectral's tested properties): the model never touches the
//! read path; a verdict naming a value NOT among the candidate objects is
//! rejected as [`AdjudicationVerdict::Invalid`] (the model may CHOOSE among
//! asserted facts, never introduce one or empty a slot); below-threshold
//! verdicts are counted, not applied. We deliberately DO NOT ask the 7B to
//! extract triples from prose — adjudication only (per the published accuracy
//! warning: extraction, not supersession, is where 7B accuracy collapses).

/// Cardinality of a predicate — whether a `(subject, predicate)` slot holds at
/// most one live object (functional → supersession applies) or many
/// (accumulating → never retire).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cardinality {
    /// At most one live object per subject; a newer value supersedes the older.
    Functional,
    /// Many objects legitimately hold at once; never retire.
    Accumulating,
}

/// Predicates we are CONFIDENT are single-valued — safe to auto-supersede.
/// "primary/secondary/favourite" and config-like slots are inherently one.
pub const FUNCTIONAL_PREDICATES: &[&str] = &[
    "status",
    "primary_browser",
    "primary_messaging",
    "primary_strategy",
    "secondary_browser",
    "tier1_model",
    "favourite_color",
    "favourite_coffee_shop",
    "favourite_restaurant",
];

/// Predicates that are USUALLY single-valued but have real concurrent cases,
/// and whose broad all-type domain makes org-scoped uses multi-valued
/// (`location` for an org with several offices, etc.). Left DISABLED until Jesse
/// confirms — enabling one wrongly would silently retire true facts (the one
/// footgun Spectral cannot infer).
pub const FUNCTIONAL_NEEDS_CONFIRM: &[&str] = &[
    "employer",
    "manager",
    "spouse",
    "wife",
    "board_chair",
    "location",
];

/// Cardinality for a predicate. Only the CONFIRMED-functional set returns
/// `Functional`; everything else — including the needs-confirm set and our core
/// edges like `works_on` — is `Accumulating`, so nothing is retired by default.
pub fn predicate_cardinality(predicate: &str) -> Cardinality {
    if FUNCTIONAL_PREDICATES.contains(&predicate) {
        Cardinality::Functional
    } else {
        Cardinality::Accumulating
    }
}

/// A closed adjudication question for the 7B. The objects are already-asserted
/// facts sharing one `(subject, predicate)` slot; the model chooses AMONG them
/// and must never invent one.
pub fn build_adjudication_prompt(
    subject: &str,
    predicate: &str,
    objects: &[(String, String)],
) -> String {
    let mut list = String::new();
    for (i, (_id, label)) in objects.iter().enumerate() {
        list.push_str(&format!("{}. {}\n", i + 1, label));
    }
    format!(
        "A knowledge base records that \"{subject}\" has the relationship \"{predicate}\" to \
         MULTIPLE values at once:\n{list}\nThese are all currently marked true. Decide ONE:\n\
         - SUPERSEDES: they describe the SAME slot over time and the most recent has REPLACED the \
         others (a changed job, a moved home). Give the number of the value CURRENTLY true.\n\
         - ALL_HOLD: the values can ALL be true simultaneously (e.g. several colleagues).\n\
         - UNKNOWN: you cannot tell.\n\n\
         Choose ONLY from the numbered values above; never name a value not listed. Reply on ONE \
         line, exactly one of:\nSUPERSEDES <number> <confidence 0-1>\nALL_HOLD\nUNKNOWN"
    )
}

/// The parsed verdict — our local mirror of Spectral's `Adjudication` until the
/// seam is in our pin.
#[derive(Debug, Clone, PartialEq)]
pub enum AdjudicationVerdict {
    /// The object with this id supersedes the others.
    Supersedes { keep: String, confidence: f64 },
    /// All objects legitimately hold — do not retire.
    AllHold,
    /// Cannot determine.
    Unknown,
    /// Malformed, or named a value not among the candidates — rejected, never
    /// applied. Mirrors Spectral's `invalid_verdicts` counter.
    Invalid,
}

/// Parse the 7B reply against the candidate objects. A `SUPERSEDES n` whose `n`
/// is out of range → `Invalid` (the model may only choose among asserted facts).
pub fn parse_adjudication(response: &str, objects: &[(String, String)]) -> AdjudicationVerdict {
    let line = response
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let upper = line.to_ascii_uppercase();
    if upper.starts_with("ALL_HOLD") {
        AdjudicationVerdict::AllHold
    } else if upper.starts_with("SUPERSEDES") {
        let mut parts = line.split_whitespace();
        let _ = parts.next(); // "SUPERSEDES"
        let idx = parts.next().and_then(|s| s.parse::<usize>().ok());
        let conf = parts
            .next()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        match idx {
            Some(n) if n >= 1 && n <= objects.len() => AdjudicationVerdict::Supersedes {
                keep: objects[n - 1].0.clone(),
                confidence: conf.clamp(0.0, 1.0),
            },
            // Out of range → named a value not in the candidate set.
            _ => AdjudicationVerdict::Invalid,
        }
    } else if upper.starts_with("UNKNOWN") {
        AdjudicationVerdict::Unknown
    } else {
        AdjudicationVerdict::Invalid
    }
}

/// The adjudicator's run mode. Default OFF. `PERMAGENT_LIBRARIAN_ADJUDICATOR` =
/// `shadow` (log detected candidates + verdicts, apply NOTHING) | `apply` (apply
/// above-threshold retirements). Shadow is the required first step per Spectral.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdjudicatorMode {
    Off,
    Shadow,
    Apply,
}

/// Read the adjudicator mode from config/env. Default `Off`.
pub fn adjudicator_mode() -> AdjudicatorMode {
    match std::env::var("PERMAGENT_LIBRARIAN_ADJUDICATOR")
        .ok()
        .as_deref()
    {
        Some("shadow") => AdjudicatorMode::Shadow,
        Some("apply") => AdjudicatorMode::Apply,
        _ => AdjudicatorMode::Off,
    }
}

/// Run the local 7B on one detected supersession candidate and return the
/// parsed verdict. This is the reusable core the future `impl Adjudicator`
/// wraps. Accumulating predicates and single-object slots short-circuit to
/// `AllHold` (belt-and-suspenders — Spectral won't hand us these once we declare
/// cardinality, but never retire them regardless).
pub async fn adjudicate_candidate(
    subject: &str,
    predicate: &str,
    objects: &[(String, String)],
) -> AdjudicationVerdict {
    if predicate_cardinality(predicate) == Cardinality::Accumulating || objects.len() < 2 {
        return AdjudicationVerdict::AllHold;
    }
    let model = super::librarian::resolve_model();
    let prompt = build_adjudication_prompt(subject, predicate, objects);
    let system = "You adjudicate whether a newer knowledge-base fact has replaced an older one. \
                  Answer ONLY in the required one-line format. Never invent a value.";
    match super::librarian::call_ollama_streaming_pooled(
        system,
        &prompt,
        &model,
        false,
        "librarian-adjudicator",
        None,
    )
    .await
    {
        Ok(reply) => parse_adjudication(&reply, objects),
        // A model/transport error is never a retirement — fail to Unknown.
        Err(_) => AdjudicationVerdict::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn objs() -> Vec<(String, String)> {
        vec![
            ("id-a".into(), "Acme Corp".into()),
            ("id-b".into(), "Globex".into()),
        ]
    }

    #[test]
    fn cardinality_functional_vs_accumulating() {
        assert_eq!(predicate_cardinality("status"), Cardinality::Functional);
        assert_eq!(
            predicate_cardinality("tier1_model"),
            Cardinality::Functional
        );
        // core edges + needs-confirm default to accumulating (never retired)
        assert_eq!(predicate_cardinality("works_on"), Cardinality::Accumulating);
        assert_eq!(predicate_cardinality("employer"), Cardinality::Accumulating);
        assert_eq!(
            predicate_cardinality("colleague"),
            Cardinality::Accumulating
        );
    }

    #[test]
    fn parses_supersedes_and_maps_to_object_id() {
        match parse_adjudication("SUPERSEDES 2 0.9", &objs()) {
            AdjudicationVerdict::Supersedes { keep, confidence } => {
                assert_eq!(keep, "id-b");
                assert!((confidence - 0.9).abs() < 1e-9);
            }
            other => panic!("expected Supersedes, got {other:?}"),
        }
    }

    #[test]
    fn out_of_range_supersedes_is_invalid_not_applied() {
        // The model must only choose among the candidates; index 5 of 2 is
        // rejected rather than silently retiring or inventing.
        assert_eq!(
            parse_adjudication("SUPERSEDES 5 0.9", &objs()),
            AdjudicationVerdict::Invalid
        );
    }

    #[test]
    fn parses_all_hold_unknown_and_garbage() {
        assert_eq!(
            parse_adjudication("ALL_HOLD", &objs()),
            AdjudicationVerdict::AllHold
        );
        assert_eq!(
            parse_adjudication("UNKNOWN", &objs()),
            AdjudicationVerdict::Unknown
        );
        assert_eq!(
            parse_adjudication("the answer is definitely acme", &objs()),
            AdjudicationVerdict::Invalid
        );
    }

    #[test]
    fn mode_defaults_off() {
        // Note: reads process env; default when unset is Off.
        if std::env::var("PERMAGENT_LIBRARIAN_ADJUDICATOR").is_err() {
            assert_eq!(adjudicator_mode(), AdjudicatorMode::Off);
        }
    }

    #[test]
    fn prompt_lists_candidates_and_forbids_invention() {
        let p = build_adjudication_prompt("Mel", "employer", &objs());
        assert!(p.contains("Acme Corp") && p.contains("Globex"));
        assert!(p.contains("never name a value not listed"));
    }
}
