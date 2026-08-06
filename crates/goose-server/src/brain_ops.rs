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
    receipt: Option<spectral::TurnReceipt>,
    /// The delivered set, in the shape the citation rule consumes.
    delivered: Vec<permagent::recognition::InjectedMemory>,
}

impl PendingTurn {
    fn take_parts(
        &mut self,
    ) -> Option<(
        permagent::brain_handle::SafeBrain,
        spectral::TurnReceipt,
        Vec<permagent::recognition::InjectedMemory>,
    )> {
        self.receipt.take().map(|receipt| {
            (
                self.brain.clone(),
                receipt,
                std::mem::take(&mut self.delivered),
            )
        })
    }
}

impl Drop for PendingTurn {
    fn drop(&mut self) {
        let Some((brain, receipt, _)) = self.take_parts() else {
            return;
        };
        let turn_id = receipt.id.clone();
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::warn!(target: "permagentd::turn", %turn_id, "Cannot void abandoned turn outside a Tokio runtime");
            return;
        };
        runtime.spawn(async move {
            match brain.void_turn(receipt).await {
                Ok(true) => tracing::debug!(target: "permagentd::turn", %turn_id, "voided abandoned turn"),
                Ok(false) => tracing::debug!(target: "permagentd::turn", %turn_id, "abandoned turn was already voided"),
                Err(e) => tracing::warn!(target: "permagentd::turn", %turn_id, error = %e, "Failed to void abandoned turn"),
            }
        });
    }
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
fn spawn_record_turn_outcome(mut turn: PendingTurn, assistant_reply: String) {
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
        let Some((brain, receipt, _)) = turn.take_parts() else {
            return;
        };
        match brain.record_turn_outcome(receipt, outcomes).await {
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
                tracing::debug!(
                    target: "permagentd::turn",
                    delivered = delivered.len(),
                    "sampled turn opened"
                );
                Some(PendingTurn {
                    brain: brain.clone(),
                    receipt: Some(result.receipt),
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
            if let Some(sampled) = turn.as_ref() {
                if let Some(receipt) = sampled.receipt.as_ref() {
                    let cascade_set: std::collections::HashSet<&str> = result
                        .merged_hits
                        .iter()
                        .map(|hit| hit.id.as_str())
                        .collect();
                    let turn_set: std::collections::HashSet<&str> = receipt
                        .delivered
                        .iter()
                        .map(|hit| hit.id.as_str())
                        .collect();
                    let intersection_size = cascade_set.intersection(&turn_set).count();
                    let cascade_size = cascade_set.len();
                    let turn_size = turn_set.len();
                    let cascade_overlap_ratio = if cascade_size == 0 {
                        0.0
                    } else {
                        intersection_size as f64 / cascade_size as f64
                    };
                    tracing::info!(
                        target: "permagentd::turn_divergence",
                        turn_id = %receipt.id,
                        cascade_size,
                        turn_size,
                        intersection_size,
                        cascade_overlap_ratio,
                        "sampled cascade/turn delivery overlap"
                    );
                }
            }
            let top_hits = filter_recall_hits(&result.merged_hits);
            let count = top_hits.len();
            let prefix = if top_hits.is_empty() {
                None
            } else {
                let mut prefix = String::from("Relevant memories from past context:\n");
                for hit in &top_hits {
                    prefix.push_str(&format!("- {}\n", hit.content));
                }
                Some(prefix)
            };
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
                let injected = result
                    .merged_hits
                    .into_iter()
                    .filter(|hit| hit.signal_score >= RECALL_SCORE_FLOOR)
                    .take(RECALL_TOP_K)
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
            // hook. Today a debug log; when Spectral's recognize() lands, this
            // is where its RecognitionResult is forwarded to the sink and
            // persisted next to this recall's outcome row.
            #[cfg(feature = "spectral-recognition")]
            permagent::recognition_sink::observe_recall_stimulus(
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
                    turn,
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
            .remember_with(
                &key,
                &content,
                spectral::RememberOpts {
                    source: Some("chat".into()),
                    device_id: Some(device_id),
                    confidence: Some(1.0),
                    visibility: spectral::Visibility::Private,
                    // Associate this memory with its originating session so
                    // same-session memories co-rank on recall (#131).
                    session_id: Some(session_id.clone()),
                    wing: None,
                    ..Default::default()
                },
            )
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
    use super::PendingTurn;
    use rusqlite::OptionalExtension;

    #[tokio::test]
    async fn dropping_pending_turn_voids_its_receipt() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut raw = spectral::Brain::open(dir.path()).expect("open brain");
        raw.set_async_turn_delivery(true);
        raw.remember(
            "drop-guard-note",
            "the staging deploy runbook lists rollback steps",
            spectral::Visibility::Private,
        )
        .expect("remember");
        let result = raw
            .turn(&spectral::TurnRequest::query(
                "staging deploy rollback",
                spectral::Visibility::Private,
            ))
            .expect("turn");
        let turn_id = result.receipt.id.clone();
        let pending = PendingTurn {
            brain: permagent::brain_handle::SafeBrain::new(raw),
            receipt: Some(result.receipt),
            delivered: Vec::new(),
        };

        drop(pending);

        for _ in 0..100 {
            let conn =
                rusqlite::Connection::open(dir.path().join("memory.db")).expect("open ledger");
            let voided_at: Option<String> = conn
                .query_row(
                    "SELECT voided_at FROM turn_events WHERE occurrence_id = ?1",
                    [&turn_id],
                    |row| row.get(0),
                )
                .optional()
                .expect("query void state")
                .flatten();
            if voided_at.is_some() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("PendingTurn Drop guard did not void turn {turn_id}");
    }
}
