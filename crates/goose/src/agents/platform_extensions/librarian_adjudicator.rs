//! Staleness adjudication for the Librarian — the Permagent half of Spectral's
//! supersession seam.
//!
//! Spectral structurally detects `(subject, predicate)` slots holding several
//! LIVE objects and asks a pluggable [`Adjudicator`] a closed question: did the
//! newer value replace the older, or do both hold? This module is the
//! Librarian's adjudicator, backed by the local 7B — **shadow-mode first** and
//! **predicate-gated**.
//!
//! SAFETY (mirrors Spectral's tested properties): the model never touches the
//! read path (recall stays deterministic); nothing is deleted — retirement
//! closes a validity interval and `find_triples_as_of` still answers
//! historically; a verdict naming a value NOT among the candidate objects never
//! becomes a `Supersedes` here (we map the reply to a real candidate object or
//! return `Unknown`), so the model can only CHOOSE among asserted facts, never
//! introduce one or empty a slot; below-threshold verdicts are gated by
//! `apply_adjudications`. We bind to Spectral's shipped [`ADJUDICATION_PROMPT`]
//! so both sides use ONE contract, and we deliberately never ask the 7B to
//! extract triples from prose — adjudication only (the published accuracy
//! bottleneck is extraction, not supersession).
//!
//! ENABLEMENT: OFF by default (`PERMAGENT_LIBRARIAN_ADJUDICATOR` unset). Set it
//! to `shadow` to log detected candidates + verdicts while retiring NOTHING, and
//! only to `apply` after the shadow numbers hold on the mini — per Spectral's
//! confirm-then-enable order. The 7B path only ever runs on **undeclared**
//! predicates; genuinely functional predicates are handled deterministically by
//! Spectral via the ontology's `single_valued_for` (no model involved).

use spectral::graph::supersession::{
    Adjudication, Adjudicator, CandidateObject, SupersessionCandidate, ADJUDICATION_PROMPT,
};

/// Cardinality of a predicate — whether a `(subject, predicate)` slot holds at
/// most one live object (functional) or many (accumulating). This is a
/// belt-and-suspenders guard on top of Spectral's ontology-declared cardinality:
/// the 7B is never asked to retire an accumulating predicate even if detection
/// hands us one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cardinality {
    /// At most one live object per subject; a newer value supersedes the older.
    Functional,
    /// Many objects legitimately hold at once; never retire.
    Accumulating,
}

/// Predicates we are CONFIDENT are single-valued — declared `single_valued_for`
/// in the ontology (deterministic supersession, no model). "primary/secondary/
/// favourite" and config-like slots are inherently one.
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

/// Predicates that are USUALLY single-valued but have real concurrent cases
/// (and whose broad all-type domain makes org-scoped uses multi-valued). Left
/// DISABLED until confirmed — enabling one wrongly would silently retire true
/// facts (the one footgun the library cannot infer).
pub const FUNCTIONAL_NEEDS_CONFIRM: &[&str] = &[
    "employer",
    "manager",
    "spouse",
    "wife",
    "board_chair",
    "location",
];

/// Cardinality for a predicate. Only the CONFIRMED-functional set returns
/// `Functional`; everything else — including the needs-confirm set and core
/// edges like `works_on` — is `Accumulating`, so the 7B never retires them.
pub fn predicate_cardinality(predicate: &str) -> Cardinality {
    if FUNCTIONAL_PREDICATES.contains(&predicate) {
        Cardinality::Functional
    } else {
        Cardinality::Accumulating
    }
}

/// Render Spectral's shared [`ADJUDICATION_PROMPT`] for one candidate.
/// `{objects}` is rendered one per line as `- <canonical> (asserted <rfc3339>)`,
/// exactly as the constant documents.
fn render_prompt(candidate: &SupersessionCandidate) -> String {
    let objects = candidate
        .objects
        .iter()
        .map(|o| {
            format!(
                "- {} (asserted {})",
                o.object_canonical,
                o.asserted_at.to_rfc3339()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    ADJUDICATION_PROMPT
        .replace("{subject}", &candidate.subject_canonical)
        .replace("{subject_type}", &candidate.subject_type)
        .replace("{predicate}", &candidate.predicate)
        .replace("{objects}", &objects)
}

/// The parsed shape of the 7B's one-line reply, resolved against the candidate.
#[derive(Debug, Clone, PartialEq)]
enum ParsedReply {
    /// `REPLACED <value>` matched candidate object at this index.
    Replaced {
        index: usize,
        confidence: f64,
    },
    AllHold,
    /// `UNKNOWN`, a malformed reply, or a `REPLACED <value>` naming a value not
    /// in the candidate list — all safe (nothing retired).
    Unknown,
}

/// Parse the 7B reply against the candidate objects. A `REPLACED <value>` whose
/// value is not among the candidates resolves to `Unknown` (never a retirement)
/// — the model may only choose among asserted facts.
fn parse_reply(reply: &str, objects: &[CandidateObject]) -> ParsedReply {
    let line = reply
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let upper = line.to_ascii_uppercase();
    if upper.starts_with("ALL_HOLD") {
        ParsedReply::AllHold
    } else if upper.starts_with("REPLACED") {
        let rest = line.get("REPLACED".len()..).unwrap_or("").trim();
        // The value may contain spaces (a canonical name); the LAST token is the
        // confidence when it parses as a float, otherwise the whole rest is the
        // value and confidence defaults to 0.
        let (value, confidence) = match rest.rsplit_once(char::is_whitespace) {
            Some((v, c)) => match c.trim().parse::<f64>() {
                Ok(f) => (v.trim(), f),
                Err(_) => (rest, 0.0),
            },
            None => (rest, 0.0),
        };
        match objects
            .iter()
            .position(|o| o.object_canonical.eq_ignore_ascii_case(value))
        {
            Some(index) => ParsedReply::Replaced {
                index,
                confidence: confidence.clamp(0.0, 1.0),
            },
            // Hallucinated a value not in the list → safe no-op.
            None => ParsedReply::Unknown,
        }
    } else {
        // UNKNOWN or anything malformed.
        ParsedReply::Unknown
    }
}

/// The run mode. Default `Off`. `PERMAGENT_LIBRARIAN_ADJUDICATOR` = `shadow`
/// (log candidates + verdicts, apply NOTHING) | `apply` (retire above-threshold).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdjudicatorMode {
    Off,
    Shadow,
    Apply,
}

/// Read the adjudicator mode from env. Default `Off`.
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

/// The Librarian's 7B-backed [`Adjudicator`]. Spectral's `adjudicate` is
/// SYNCHRONOUS, so this bridges to the async Ollama call via a captured runtime
/// [`Handle`](tokio::runtime::Handle) — it MUST therefore be driven from a
/// `spawn_blocking` thread (never a runtime worker), which
/// [`run_adjudication_pass`] guarantees.
pub struct LibrarianAdjudicator {
    handle: tokio::runtime::Handle,
    model: String,
}

impl LibrarianAdjudicator {
    /// Capture the current runtime handle + the Librarian's model. Call from an
    /// async context (before `spawn_blocking`).
    pub fn new() -> Self {
        Self {
            handle: tokio::runtime::Handle::current(),
            model: super::librarian::resolve_model(),
        }
    }
}

impl Adjudicator for LibrarianAdjudicator {
    fn adjudicate(&self, candidate: &SupersessionCandidate) -> anyhow::Result<Adjudication> {
        // Never retire an accumulating predicate, even if detection hands us one.
        if predicate_cardinality(&candidate.predicate) == Cardinality::Accumulating
            || candidate.objects.len() < 2
        {
            return Ok(Adjudication::AllHold);
        }
        let prompt = render_prompt(candidate);
        let system = "You adjudicate whether a newer knowledge-base fact has replaced an older \
                      one. Answer ONLY in the required one-line format. Never invent a value; \
                      prefer UNKNOWN over a guess.";
        // Bridge the sync trait to the async Ollama call — safe from the
        // spawn_blocking thread run_adjudication_pass uses.
        let reply = self
            .handle
            .block_on(super::librarian::call_ollama_streaming_pooled(
                system,
                &prompt,
                &self.model,
                false,
                "librarian-adjudicator",
                None,
            ));
        let reply = match reply {
            Ok(r) => r,
            // A model/transport error is never a retirement.
            Err(_) => return Ok(Adjudication::Unknown),
        };
        Ok(match parse_reply(&reply, &candidate.objects) {
            ParsedReply::Replaced { index, confidence } => Adjudication::Supersedes {
                keep: candidate.objects[index].object,
                confidence,
            },
            ParsedReply::AllHold => Adjudication::AllHold,
            ParsedReply::Unknown => Adjudication::Unknown,
        })
    }
}

// ── The pass runner is intentionally NOT here yet ────────────────────────────
//
// `detect_candidates` / `apply_adjudications` (spectral-graph/src/supersession.rs)
// take `&spectral_graph::brain::Brain` — the GRAPH brain. Our `SafeBrain` holds
// the FACADE `spectral::Brain`, whose inner graph brain is private and which, at
// pin 486459c, does NOT expose the supersession seam (unlike the consolidation
// seam, which the facade delegates via `consolidation_candidates` /
// `consolidate_with`). So the runner cannot be written against the facade.
//
// The adapter ABOVE is complete and fully testable — it only touches the seam's
// value types (`Adjudicator`, `SupersessionCandidate`, `CandidateObject`,
// `ADJUDICATION_PROMPT`), not the graph brain. The runner
// (`run_adjudication_pass`: Off | Shadow=detect+log | Apply=apply_adjudications)
// lands as a follow-up once Spectral exposes the seam on the facade — either
// delegating methods (mirroring consolidation) or a `graph()` accessor. Ask sent.

impl Default for LibrarianAdjudicator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spectral::core::entity_id::entity_id;

    fn obj(canonical: &str) -> CandidateObject {
        CandidateObject {
            rowid: 1,
            object: entity_id("organization", canonical),
            object_canonical: canonical.to_string(),
            asserted_at: chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap(),
        }
    }

    #[test]
    fn cardinality_functional_vs_accumulating() {
        assert_eq!(predicate_cardinality("status"), Cardinality::Functional);
        assert_eq!(
            predicate_cardinality("tier1_model"),
            Cardinality::Functional
        );
        assert_eq!(predicate_cardinality("works_on"), Cardinality::Accumulating);
        assert_eq!(predicate_cardinality("employer"), Cardinality::Accumulating);
    }

    #[test]
    fn parses_replaced_by_value_case_insensitive() {
        let objs = [obj("Acme Corp"), obj("Globex")];
        assert_eq!(
            parse_reply("REPLACED globex 0.9", &objs),
            ParsedReply::Replaced {
                index: 1,
                confidence: 0.9
            }
        );
        // multi-word value
        assert_eq!(
            parse_reply("REPLACED Acme Corp 0.8", &objs),
            ParsedReply::Replaced {
                index: 0,
                confidence: 0.8
            }
        );
    }

    #[test]
    fn value_not_in_list_is_unknown_never_retires() {
        let objs = [obj("Acme Corp"), obj("Globex")];
        assert_eq!(
            parse_reply("REPLACED Initech 0.9", &objs),
            ParsedReply::Unknown
        );
    }

    #[test]
    fn parses_all_hold_unknown_and_garbage() {
        let objs = [obj("Acme Corp"), obj("Globex")];
        assert_eq!(parse_reply("ALL_HOLD", &objs), ParsedReply::AllHold);
        assert_eq!(parse_reply("UNKNOWN", &objs), ParsedReply::Unknown);
        assert_eq!(parse_reply("i think acme", &objs), ParsedReply::Unknown);
    }

    #[test]
    fn prompt_binds_the_shared_contract_and_lists_objects() {
        let candidate = SupersessionCandidate {
            subject: entity_id("person", "Mel"),
            subject_canonical: "Mel".to_string(),
            subject_type: "person".to_string(),
            predicate: "employer".to_string(),
            objects: vec![obj("Acme Corp"), obj("Globex")],
        };
        let p = render_prompt(&candidate);
        assert!(p.contains("Mel") && p.contains("employer"));
        assert!(p.contains("- Acme Corp (asserted") && p.contains("- Globex (asserted"));
        assert!(!p.contains("{objects}"));
    }
}
