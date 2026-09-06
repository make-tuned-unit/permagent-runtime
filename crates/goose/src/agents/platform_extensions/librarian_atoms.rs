//! Librarian consolidation atoms — the write-side of the Spectral layered store.
//!
//! The Brain surfaces recurring clusters deterministically
//! ([`SafeBrain::consolidation_candidates`]); this module abstracts each cluster
//! into ONE durable, high-quality atom with a strong model and stores it via
//! [`SafeBrain::consolidate_as`] (provenance-linked). Because the raw sources
//! stay reachable through [`SafeBrain::recall_with_provenance`], the atom is an
//! **additive hint the actor verifies against ground truth**, never an
//! authoritative lossy replacement.
//!
//! ## Why this shape (the load-bearing lesson)
//! On the real eval, retrieval recall is near-ceiling; the gap is the ACTOR
//! mis-counting the same real-world item across sessions. A cheap READ-TIME LLM
//! consolidation pre-pass REGRESSED −9.2pp (a weak, lossy intermediate the strong
//! actor over-trusts). So consolidation here is: (a) write-time, (b) strong-model,
//! (c) durable, (d) gated to recurring clusters, and (e) consumed as a hint the
//! actor verifies against raw sources. The atom quality contract below is the
//! crux — getting it wrong reintroduces the regression.
//!
//! Everything here is invoked only behind the default-OFF `LIBRARIAN_ATOMS_ENABLED`
//! flag (gated on the mac-mini paired eval; see the scheduling seam).

use std::sync::Arc;

use crate::agents::reply_parts::AccountedFastCompletion;
use crate::brain_handle::SafeBrain;
use crate::conversation::message::Message;
use crate::providers::base::Provider;

/// Deterministic recurrence floor for a cluster to be a candidate. `2` is the
/// sensible minimum (an item seen together at least twice), matching Spectral's
/// documented floor.
const MIN_CO_COUNT: u64 = 2;

/// How many unconsolidated memories the deterministic candidate scan walks per
/// sweep. Bounded so the sweep stays cheap; recurring clusters float to the top.
const SCAN_LIMIT: usize = 200;

/// Cap on source members fed to the strong model per atom, so a pathologically
/// large cluster cannot blow the context window. Highest-value members first
/// (the candidate's `member_keys` are already ordered deterministically).
const MAX_SOURCES_PER_ATOM: usize = 24;

/// THE ATOM QUALITY CONTRACT, encoded verbatim in spirit.
///
/// This prompt is the deliverable's crux: it is the difference between the
/// write-time strong-model atom that helps and the weak lossy intermediate that
/// regressed −9.2pp. Do not soften it. Every clause maps to a contract rule:
/// entity-keyed + dedup-correct, inclusion-strict, cite-sources, omission-safe,
/// candidate-set-not-authoritative.
const ATOM_SYSTEM_PROMPT: &str = r#"You are a meticulous archivist building a CONSOLIDATION ATOM: a compact, entity-keyed index over a cluster of memories that keep being retrieved together across different sessions. A downstream actor reads your atom as a HINT and then verifies it against the raw source memories (which it always has). Your atom must make the actor MORE accurate at counting and identifying distinct real-world items across sessions — never less. A lossy or over-merged atom is worse than none.

Output ONE atom: a short list of lines, one line per DISTINCT real-world item. Nothing else — no preamble, no closing commentary.

RULES (follow every one):

1. ENTITY-KEYED, DEDUP-CORRECT. Write exactly one line per distinct real-world item (a person, couple, place, project, purchase, event, appointment, pet, trip — whatever the cluster is about). Key each line by the item's MOST DISTINCTIVE identifier (a name, a date, a unique attribute). MERGE mentions of the SAME item that appear in different sources into that single line — this is the whole point; the actor's failure mode is counting one item as several across sessions. Conversely, NEVER collapse two genuinely different items into one line.

2. INCLUSION-STRICT. Include ONLY items the user actually did, attended, owns, or that actually happened. EXCLUDE: hypotheticals, things merely planned or considered but not done, options weighed, cancelled items, and anything the assistant suggested that the user did not adopt. When a source shows a plan that a later source confirms happened, include it (and cite both); if nothing confirms it happened, leave it out.

3. CITE SOURCES. End every line with the source keys it draws from, in brackets, e.g. "[src: key_a, key_b]". Use the exact keys given in the input. This lets the actor cross-check each claim against ground truth. A line with no citation is invalid.

4. OMISSION-SAFE — WHEN UNSURE, SAY SO; NEVER SILENTLY MERGE OR DROP. If you cannot tell whether two mentions are the same item or two different items, DO NOT guess. Write what you know and flag the ambiguity in the line itself, e.g. "possibly the same as <other item> — sources disagree [src: ...]". Dropping a real item or silently over-merging is the exact error that must not happen; the actor has the raw sources to adjudicate, so hand it the doubt, not a false certainty.

5. YOU ARE A CANDIDATE SET, NOT THE TRUTH. Your atom is additive: the actor treats it as a starting checklist to verify and extend, not as an authoritative replacement for the sources. Prefer being complete and honestly uncertain over being confidently wrong. Do not editorialize, rank, or infer beyond what the sources state.

FORMAT EXAMPLE (illustrative — key by the real items in YOUR cluster):
- Dr. Patel, dermatologist — user had two appointments, 2026-03-04 and 2026-05-19 [src: sess12_turn3, sess40_turn1]
- Kauai trip, June 2026 — booked and taken; note: a separately-mentioned "Hawaii trip" may be this same trip, sources unclear [src: sess18_turn7, sess22_turn2]"#;

/// Run one atom-consolidation sweep over the Brain's recurring clusters.
///
/// For each deterministic candidate (up to `max_atoms` atoms written this
/// sweep): derive a stable `target_key`, skip if that atom already exists
/// (idempotent re-runs), fetch the members' raw contents, generate ONE atom
/// with the strong-model `provider` (or the deterministic extractive fallback
/// when no provider is available), and store it provenance-linked at
/// `CompactionTier::WeeklyRollup`. Best-effort per cluster: one failing cluster
/// logs and continues; the sweep never aborts. Returns the number of atoms
/// written.
pub async fn run_atom_consolidation(
    brain: &SafeBrain,
    max_atoms: usize,
    provider: Option<Arc<dyn Provider>>,
) -> anyhow::Result<usize> {
    let candidates = brain
        .consolidation_candidates(MIN_CO_COUNT, SCAN_LIMIT)
        .await?;

    if candidates.is_empty() {
        tracing::info!(
            target: "permagentd::librarian",
            "atom consolidation: no recurring clusters to abstract"
        );
        return Ok(0);
    }

    let mut written = 0usize;
    for candidate in candidates {
        if written >= max_atoms {
            break;
        }

        let member_keys = candidate.member_keys;
        if member_keys.len() < 2 {
            continue; // spectral guarantees ≥2, but stay defensive
        }

        // Deterministic key over the sorted members → idempotent re-runs.
        let target_key = derive_target_key(&member_keys);

        // Idempotency pre-check: if this exact atom already exists, skip the
        // (expensive) LLM call entirely.
        match brain.get_memory_by_key(&target_key).await {
            Ok(Some(_)) => {
                tracing::debug!(
                    target: "permagentd::librarian",
                    target_key = %target_key,
                    "atom already exists — skipping cluster"
                );
                continue;
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(
                    target: "permagentd::librarian",
                    error = %e,
                    "atom idempotency check failed — skipping cluster"
                );
                continue;
            }
        }

        // Fetch each member's raw content. Skip members we can't fetch; skip the
        // whole cluster if fewer than 2 survive (nothing to consolidate).
        let mut sources: Vec<(String, String)> = Vec::with_capacity(member_keys.len());
        for key in member_keys.iter().take(MAX_SOURCES_PER_ATOM) {
            match brain.get_memory_by_key(key).await {
                Ok(Some(m)) => sources.push((key.clone(), m.content)),
                Ok(None) => {}
                Err(e) => tracing::warn!(
                    target: "permagentd::librarian",
                    key = %key,
                    error = %e,
                    "atom source fetch failed"
                ),
            }
        }
        if sources.len() < 2 {
            tracing::debug!(
                target: "permagentd::librarian",
                target_key = %target_key,
                fetched = sources.len(),
                "atom cluster has <2 fetchable sources — skipping"
            );
            continue;
        }

        // Generate the atom with the strong model; fall back to the
        // deterministic extractive consolidation so layered recall still exists
        // at $0 when the provider is unavailable or the call fails/empties.
        let atom = match provider.as_ref() {
            Some(p) => match generate_atom(p, &sources).await {
                Ok(text) if !text.trim().is_empty() => Some(text),
                Ok(_) => {
                    tracing::warn!(
                        target: "permagentd::librarian",
                        target_key = %target_key,
                        "strong-model atom empty — using extractive fallback"
                    );
                    None
                }
                Err(e) => {
                    tracing::warn!(
                        target: "permagentd::librarian",
                        target_key = %target_key,
                        error = %e,
                        "strong-model atom generation failed — using extractive fallback"
                    );
                    None
                }
            },
            None => None,
        };

        let result = match atom {
            Some(content) => {
                brain
                    .consolidate_as(
                        member_keys.clone(),
                        target_key.clone(),
                        spectral::ingest::CompactionTier::WeeklyRollup,
                        content,
                    )
                    .await
            }
            None => {
                brain
                    .consolidate_extractive(
                        member_keys.clone(),
                        target_key.clone(),
                        spectral::ingest::CompactionTier::WeeklyRollup,
                    )
                    .await
            }
        };

        match result {
            Ok(_) => {
                written += 1;
                tracing::info!(
                    target: "permagentd::librarian",
                    target_key = %target_key,
                    members = member_keys.len(),
                    cohesion = candidate.cohesion,
                    signal = candidate.signal,
                    "consolidation atom written"
                );
            }
            Err(e) => tracing::warn!(
                target: "permagentd::librarian",
                target_key = %target_key,
                error = %e,
                "consolidation atom store failed — continuing sweep"
            ),
        }
    }

    Ok(written)
}

/// One strong-model call → one atom over `sources` (`(key, content)` pairs).
/// Uses the provider's lead/actor model config (`get_model_config`) — NOT the
/// fast model — because the quality contract demands actor-tier or better.
async fn generate_atom(
    provider: &Arc<dyn Provider>,
    sources: &[(String, String)],
) -> anyhow::Result<String> {
    let mut body = String::from(
        "These memories keep being retrieved together across sessions. Consolidate \
         them into ONE atom per the rules. The bracketed key before each memory is \
         its source key — cite these exact keys.\n\n",
    );
    for (key, content) in sources {
        body.push_str(&format!("[{key}]\n{content}\n\n"));
    }
    body.push_str(
        "Now write the atom: one line per distinct real-world item, each citing its \
         source keys. Merge cross-session mentions of the same item; include only \
         what actually happened; flag any uncertainty instead of guessing.",
    );

    let user_message = Message::user().with_text(body);
    let manager = Arc::new(crate::session::SessionManager::instance());
    let session =
        AccountedFastCompletion::ensure_background_session(Arc::clone(&manager), "librarian-atoms")
            .await?;
    let (response, _usage) = AccountedFastCompletion::complete_accounted(
        manager,
        session,
        Arc::clone(provider),
        ATOM_SYSTEM_PROMPT,
        std::slice::from_ref(&user_message),
        &[],
        false,
    )
    .await
    .map_err(|e| anyhow::anyhow!("atom LLM call failed: {e}"))?;

    Ok(response.as_concat_text().trim().to_string())
}

/// Derive a stable, deterministic `target_key` for the atom over `member_keys`.
///
/// Sorts a copy of the members (order-independent) and blake3-hashes the joined
/// keys, so the SAME cluster always maps to the SAME atom key — re-runs are
/// idempotent (an existing atom is detected and skipped). Prefixed `atom:` so
/// atoms are trivially distinguishable from raw memories.
fn derive_target_key(member_keys: &[String]) -> String {
    let mut sorted: Vec<&str> = member_keys.iter().map(|s| s.as_str()).collect();
    sorted.sort_unstable();
    let joined = sorted.join("\n");
    let digest = blake3::hash(joined.as_bytes());
    format!("atom:{}", digest.to_hex())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn librarian_atoms_cannot_bypass_shared_paid_dispatch_boundary() {
        let source = include_str!("librarian_atoms.rs");
        let direct_call = [".", "complete("].concat();
        assert!(source.contains("complete_accounted"));
        assert!(!source.contains(&direct_call));
    }

    #[test]
    fn target_key_is_deterministic_and_order_independent() {
        let a = vec![
            "sess12_turn3".to_string(),
            "sess40_turn1".to_string(),
            "sess18_turn7".to_string(),
        ];
        // Same members, different input order → same key (idempotent re-runs).
        let mut b = a.clone();
        b.reverse();
        assert_eq!(derive_target_key(&a), derive_target_key(&b));

        // Stable across repeated calls.
        assert_eq!(derive_target_key(&a), derive_target_key(&a));

        // Namespaced so atoms are distinguishable from raw memories.
        assert!(derive_target_key(&a).starts_with("atom:"));
    }

    #[test]
    fn different_clusters_get_different_keys() {
        let a = vec!["k1".to_string(), "k2".to_string()];
        let c = vec!["k1".to_string(), "k3".to_string()];
        assert_ne!(derive_target_key(&a), derive_target_key(&c));
    }
}
