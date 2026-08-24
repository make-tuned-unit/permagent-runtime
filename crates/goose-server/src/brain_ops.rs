//! Shared Brain operations extracted from reply.rs and session_events.rs.
//!
//! This module centralises recall injection, chat-turn persistence, and
//! read-only Brain DB connections that were previously duplicated across
//! multiple route handlers.

use std::sync::Arc;

// ── Recall constants & filter ────────────────────────────────────────────

// The spectral_schema.rs historical backfill intentionally pins replicas of
// these values; reconcile any still-NULL injected sets before retuning them.
/// Score floor for recall hits — memories below this are excluded.
pub const RECALL_SCORE_FLOOR: f64 = 0.7;
/// Maximum number of recall hits injected into the system prompt.
pub const RECALL_TOP_K: usize = 3;

/// Filter recall hits by score floor and top-K cap.
/// Input must already be sorted by relevance (upstream guarantee from cascade).
pub fn filter_recall_hits(
    hits: &[spectral::ingest::MemoryHit],
) -> Vec<&spectral::ingest::MemoryHit> {
    hits.iter()
        .filter(|hit| hit.signal_score >= RECALL_SCORE_FLOOR)
        .take(RECALL_TOP_K)
        .collect()
}

/// Result of one recall injection. The pending recognition write is optional
/// because the daemon can run without a session DB pool.
#[derive(Default)]
pub struct RecallInjection {
    pub count: usize,
    pending: Option<permagent::recognition::PendingRecognition>,
    /// Open `Brain::turn` receipt for a SAMPLED turn, awaiting its outcome
    /// report. `None` on the overwhelming majority of turns — see
    /// `permagent::turn_sampling` for why this path is not the default.
    turn: Option<PendingTurn>,
}

/// A sampled `turn` awaiting outcome attribution at end of turn.
struct PendingTurn {
    brain: permagent::brain_handle::SafeBrain,
    receipt: spectral::TurnReceipt,
    /// The delivered set, in the shape the citation rule consumes.
    delivered: Vec<permagent::recognition::InjectedMemory>,
}

impl RecallInjection {
    /// Finish the trace after the reply. Detection and persistence remain
    /// detached/best-effort inside the recognition module.
    pub fn finish(self, assistant_reply: String) {
        if let Some(pending) = self.pending {
            pending.spawn_record_reply_usage(assistant_reply.clone());
        }
        if let Some(turn) = self.turn {
            spawn_record_turn_outcome(turn, assistant_reply);
        }
    }
}

/// Report which delivered memories the reply actually used.
///
/// Detached and best-effort like the recognition write-back: a turn is
/// instrumentation, and instrumentation must never delay or fail a reply. But
/// it MUST happen — an unreported turn leaves memory state unchanged and
/// yields no learning signal, which is the whole reason `turn` exists.
fn spawn_record_turn_outcome(turn: PendingTurn, assistant_reply: String) {
    tokio::spawn(async move {
        // Same citation rule as the recognition path, deliberately: two
        // definitions of "used" would make the corpora incomparable.
        let cited = permagent::recognition::cited_memories_by_content_overlap(
            &turn.delivered,
            &assistant_reply,
        );
        let cited: std::collections::HashSet<&str> = cited.iter().map(String::as_str).collect();

        // Report EVERY delivered hit, not just the used ones. Silence about a
        // delivered-but-unused memory is indistinguishable from a memory that
        // was never delivered, and the negative examples are half the signal.
        let outcomes: Vec<(String, spectral::MemoryOutcome)> = turn
            .delivered
            .iter()
            .map(|hit| {
                let outcome = if cited.contains(hit.id.as_str()) {
                    spectral::MemoryOutcome::Used
                } else {
                    // `Ignored` = delivered but not used. NOT `Wrong` — that
                    // is negative evidence meaning "actively misleading", and
                    // content overlap cannot distinguish unhelpful from unused.
                    // Overstating it would poison the corpus.
                    spectral::MemoryOutcome::Ignored
                };
                (hit.id.clone(), outcome)
            })
            .collect();

        let used = outcomes
            .iter()
            .filter(|(_, o)| matches!(o, spectral::MemoryOutcome::Used))
            .count();
        match turn.brain.record_turn_outcome(turn.receipt, outcomes).await {
            Ok(_) => tracing::debug!(
                target: "permagentd::turn",
                used, "recorded turn outcome"
            ),
            Err(e) => tracing::warn!(
                target: "permagentd::turn",
                "Failed to record turn outcome: {e}"
            ),
        }
    });
}

// ── Recall injection ─────────────────────────────────────────────────────

/// Inject recall results into the agent's system prompt.
/// Filters by `RECALL_SCORE_FLOOR` (0.7), takes `RECALL_TOP_K` (3).
/// Returns the injected count plus an optional turn-end attribution handle.
/// Errors are logged, never propagated.
pub async fn inject_recall(
    brain: &permagent::brain_handle::SafeBrain,
    agent: &Arc<permagent::agents::Agent>,
    user_query: &str,
    recognition_ctx: spectral::graph::RecognitionContext,
    recognition_pool: Option<sqlx::Pool<sqlx::Sqlite>>,
) -> RecallInjection {
    if user_query.is_empty() {
        return RecallInjection::default();
    }

    let query_for_log = user_query.chars().take(80).collect::<String>();

    // Sampled `Brain::turn`, in SHADOW: the reply is still built from
    // recall_cascade below, so a sampled turn cannot change what the user sees
    // — it only contributes a labelled (query, delivered-set, outcome) row to
    // the corpus Spectral needs. Shadowing costs a second retrieval, which is
    // affordable precisely because it is sampled and off by default.
    //
    // Failures are swallowed to None: instrumentation must never break a reply.
    let turn = if permagent::turn_sampling::should_sample_turn() {
        match brain
            .turn(
                user_query,
                spectral::Visibility::Private,
                recognition_ctx.clone(),
            )
            .await
        {
            Ok(result) => {
                // Content lives on `hits`; the receipt's `delivered` carries
                // the KEY that outcomes must be reported against. They are both
                // in rank order, so zip them — and key the InjectedMemory by
                // `key`, not `id`: record_turn_outcome rejects outcomes for
                // keys it did not deliver, and id != key here.
                let delivered: Vec<permagent::recognition::InjectedMemory> = result
                    .receipt
                    .delivered
                    .iter()
                    .zip(result.hits.iter())
                    .map(|(d, hit)| permagent::recognition::InjectedMemory {
                        id: d.key.clone(),
                        content: hit.content.clone(),
                    })
                    .collect();
                // INFO, not DEBUG: this line is the only per-turn record of a
                // sampled turn, and it is what makes the corpus auditable from
                // outside the library — count these lines over a window,
                // count `turn_events` rows over the same window, and at sample
                // rate 1.0 they must be equal. Inequality means turns are
                // being dropped or eligibility changed, which is detectable in
                // the data without reading either implementation (Spectral
                // dispatch 2026-08-06v). At DEBUG it sat below the daemon's
                // INFO floor, so the check required setting RUST_LOG before
                // the measurement window — i.e. it was never run.
                tracing::info!(
                    target: "permagentd::turn",
                    delivered = delivered.len(),
                    "sampled turn opened"
                );
                Some(PendingTurn {
                    brain: brain.clone(),
                    receipt: result.receipt,
                    delivered,
                })
            }
            Err(e) => {
                tracing::warn!(target: "permagentd::turn", "sampled turn failed: {e}");
                None
            }
        }
    } else {
        None
    };

    match brain.recall_cascade(user_query, &recognition_ctx).await {
        Ok(result) => {
            let top_hits = filter_recall_hits(&result.merged_hits);
            let (prefix, injected_ids) = if top_hits.is_empty() {
                (None, Vec::new())
            } else {
                let sources: Vec<permagent::context_layers::AssembleSource<'_>> = top_hits
                    .iter()
                    .map(|hit| permagent::context_layers::AssembleSource {
                        key: hit.key.as_str(),
                        abstract_text: hit.description.as_deref(),
                        content: hit.content.as_str(),
                        score: hit.signal_score,
                    })
                    .collect();
                let layered = permagent::context_layers::assemble(
                    &sources,
                    permagent::context_layers::AssembleBudget::REPLY,
                );
                // `assemble` may omit budget-excluded hits. Receipts and
                // recognition must describe what actually reached the prompt,
                // not the larger pre-budget candidate set.
                let injected_ids = top_hits
                    .iter()
                    .take(layered.len())
                    .map(|hit| hit.id.clone())
                    .collect();
                let rendered = permagent::context_layers::render_prompt(&layered);
                if rendered.is_empty() {
                    (None, Vec::new())
                } else {
                    (Some(rendered), injected_ids)
                }
            };
            let count = injected_ids.len();
            drop(top_hits);

            // Recognition instrumentation: persist the recall event + its WHOLE
            // retrieved set UNCONDITIONALLY (the falsifiable AmbientFrame
            // substrate), minting a retrieval_id for later outcome write-back.
            // Also persist the exact post-filter ids injected into the prompt.
            let pending = recognition_pool.map(|pool| {
                let members: Vec<permagent::recognition::SetMember> = result
                    .merged_hits
                    .iter()
                    .enumerate()
                    .map(|(rank, hit)| (hit.id.clone(), hit.signal_score, rank as i64))
                    .collect();
                // Move the already-filtered hit content into the detached turn
                // handle: citation tracking adds no content clone or overlap scan
                // to the reply hot path.
                let injected_ids: std::collections::HashSet<String> =
                    injected_ids.into_iter().collect();
                let injected = result
                    .merged_hits
                    .into_iter()
                    .filter(|hit| injected_ids.contains(&hit.id))
                    .map(|hit| permagent::recognition::InjectedMemory {
                        id: hit.id,
                        content: hit.content,
                    })
                    .collect();
                permagent::recognition::spawn_persist_recognition(
                    pool,
                    recognition_ctx.clone(),
                    user_query.to_string(),
                    "cascade".to_string(),
                    members,
                    injected,
                )
            });

            // Recognition seam (query mode): the verdict-alongside-recall
            // hook. Spectral's recognize() runs on a DETACHED task inside this
            // call and its verdict is persisted onto the row `pending` just
            // minted (ordered behind that INSERT by the verdict handle). This
            // returns immediately and cannot fail: the reply below is already
            // built, and a recognition error, panic or timeout only drops the
            // verdict. `pending` is kept for turn-end citation detection.
            #[cfg(feature = "spectral-recognition")]
            permagent::recognition_sink::observe_recall_stimulus(
                brain,
                pending.as_ref().map(|p| p.verdict_handle()),
                user_query,
                recognition_ctx.focus_wing.as_deref(),
                recognition_ctx.session_id.as_deref(),
            );

            let Some(prefix) = prefix else {
                tracing::debug!(
                    target: "permagentd::brain",
                    "Recall returned no hits above {} threshold for query: {:?}",
                    RECALL_SCORE_FLOOR,
                    query_for_log
                );
                return RecallInjection {
                    count: 0,
                    pending,
                    turn: None,
                };
            };

            tracing::info!(
                target: "permagentd::brain",
                "Recall injected {} memories into system prompt for query: {:?}",
                count,
                query_for_log
            );

            agent
                .extend_system_prompt("memory_recall".to_string(), prefix)
                .await;
            RecallInjection {
                count,
                pending,
                turn,
            }
        }
        Err(e) => {
            tracing::warn!(
                target: "permagentd::brain",
                "Brain recall failed: {}",
                e
            );
            RecallInjection::default()
        }
    }
}

// ── Chat turn persistence ────────────────────────────────────────────────

/// The `RememberOpts` every chat turn is written with.
///
/// Split out of [`spawn_persist_chat_turn`] so the metadata a chat memory
/// carries can be asserted in a unit test without a Brain, a runtime, or a
/// detached task.
pub fn chat_turn_opts(session_id: &str, device_id: spectral::DeviceId) -> spectral::RememberOpts {
    spectral::RememberOpts {
        source: Some("chat".into()),
        device_id: Some(device_id),
        confidence: Some(1.0),
        visibility: spectral::Visibility::Private,
        // Associate this memory with its originating session so
        // same-session memories co-rank on recall (#131).
        session_id: Some(session_id.to_string()),
        // The chat session IS the episode (R45): every turn of one conversation
        // lands in a single episode, explicitly. Left to Spectral, episode
        // boundaries come from a 30-minute write-gap heuristic per wing, which
        // splits a conversation the user paused mid-way and merges two
        // back-to-back conversations into one.
        episode_id: Some(session_id.to_string()),
        wing: None,
        ..Default::default()
    }
}

/// Persist a chat turn's memories via SafeBrain::remember_with.
/// Spawns a detached background task — fire-and-forget.
pub fn spawn_persist_chat_turn(
    brain: permagent::brain_handle::SafeBrain,
    session_id: String,
    turn_idx: usize,
    user_text: String,
    assistant_text: String,
) {
    tokio::spawn(async move {
        let key = format!("chat-{}-{}", session_id, turn_idx);
        let content = format!("User: {}\nAssistant: {}", user_text, assistant_text);
        let device_id = *brain.device_id();
        let key_for_log = key.clone();

        match brain
            .remember_with(&key, &content, chat_turn_opts(&session_id, device_id))
            .await
        {
            Ok(_) => {
                tracing::info!(
                    target: "permagentd::brain",
                    "Remembered chat turn: {}",
                    key_for_log
                );
            }
            Err(e) => {
                // remember_with returns Err if the session association fails even
                // when the memory itself was committed, so don't claim it was
                // lost. Fire-and-forget: logged, never blocks the reply path.
                tracing::warn!(
                    target: "permagentd::brain",
                    "remember_with returned an error for chat turn {} (the memory may still be persisted; session association or a later step failed): {}",
                    key_for_log,
                    e
                );
            }
        }
    });
}

// ── Ambient context injection ────────────────────────────────────────────

/// Inject ambient context (project focus, probed/recalled memories) into the
/// agent's system prompt via the ContextBuilder.
///
/// Returns the digest on success (callers may use it for ContextAttached events).
/// Returns None if the ContextBuilder is not available or the digest is empty.
/// Errors are logged and swallowed — never blocks the reply path.
pub async fn inject_ambient_context(
    state: &crate::state::AppState,
    agent: &Arc<permagent::agents::Agent>,
) -> Option<permagent::activity::context_builder::Digest> {
    let context_builder = state.context_builder.as_ref()?;

    let focus_wing = state
        .activity_ingester
        .as_ref()
        .and_then(|ing| ing.active_project())
        .map(|ap| ap.wing.clone());

    // NOTE: deliberately probe-only — no `include_recall_query`, so this takes
    // no user text. Every caller follows inject_ambient_context with
    // inject_recall(), which runs the recall_cascade on the user text with the
    // REAL RecognitionContext (this path could only supply an empty one).
    // Letting the digest recall too was a redundant, inferior second cascade —
    // ~5-6s of duplicate Brain work on every reply turn. Recall is owned by
    // inject_recall. See voice pre-stream latency fix.
    let digest_opts = permagent::activity::context_builder::DigestOpts {
        include_probe: true,
        focus_wing,
        ..Default::default()
    };

    let cb = context_builder.clone();
    let digest_result =
        tokio::task::spawn_blocking(move || cb.current_digest_blocking(digest_opts))
            .await
            .unwrap_or_else(|e| Err(anyhow::anyhow!("spawn_blocking: {}", e)));

    match digest_result {
        Ok(digest) => {
            let ambient_block =
                permagent::activity::context_builder::render_ambient_context(&digest);
            if !ambient_block.is_empty() {
                tracing::debug!(
                    target: "permagentd::activity",
                    probed = digest.probed_memories.len(),
                    recalled = digest.recalled_memories.len(),
                    "Injecting ambient context into system prompt"
                );
                agent
                    .extend_system_prompt("ambient_context".to_string(), ambient_block)
                    .await;
            }
            Some(digest)
        }
        Err(e) => {
            tracing::warn!(
                target: "permagentd::activity",
                "ContextBuilder digest failed, proceeding without ambient context: {}",
                e
            );
            None
        }
    }
}

// ── Read-only Brain DB connection ────────────────────────────────────────

/// Open a read-only SQLite connection to the Brain memory.db.
/// Replaces the 4-line boilerplate at 5 call sites.
pub fn read_only_brain_conn() -> Result<rusqlite::Connection, rusqlite::Error> {
    let db_path = permagent::config::paths::Paths::brain_dir().join("memory.db");
    rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R45: a chat turn must name its episode explicitly. The session id is the
    /// episode — stable for every turn of the conversation, and already the
    /// value carried in `session_id`, so no new identifier is minted.
    #[test]
    fn chat_turn_opts_carry_the_session_as_the_episode() {
        let device_id = spectral::DeviceId::from_bytes([7u8; 32]);
        let opts = chat_turn_opts("session-abc", device_id);

        assert_eq!(opts.episode_id.as_deref(), Some("session-abc"));
        assert_eq!(opts.session_id.as_deref(), Some("session-abc"));

        // Stable across turns: the same session yields the same episode, which
        // is the whole point — a per-write id would link nothing.
        let later_turn = chat_turn_opts("session-abc", device_id);
        assert_eq!(opts.episode_id, later_turn.episode_id);
    }
}
