//! Shared Brain operations extracted from reply.rs and session_events.rs.
//!
//! This module centralises recall injection, chat-turn persistence, and
//! read-only Brain DB connections that were previously duplicated across
//! multiple route handlers.

use std::sync::Arc;

// ── Recall constants & filter ────────────────────────────────────────────

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

// ── Recall injection ─────────────────────────────────────────────────────

/// Inject recall results into the agent's system prompt.
/// Filters by `RECALL_SCORE_FLOOR` (0.7), takes `RECALL_TOP_K` (3).
/// Returns count of memories injected. Errors are logged, never propagated.
pub async fn inject_recall(
    brain: &permagent::brain_handle::SafeBrain,
    agent: &Arc<permagent::agents::Agent>,
    user_query: &str,
    recognition_ctx: spectral::graph::RecognitionContext,
    recognition_pool: Option<sqlx::Pool<sqlx::Sqlite>>,
) -> usize {
    if user_query.is_empty() {
        return 0;
    }

    let query_for_log = user_query.chars().take(80).collect::<String>();
    match brain.recall_cascade(user_query, &recognition_ctx).await {
        Ok(result) => {
            // Recognition instrumentation: persist the recall event + its WHOLE
            // retrieved set UNCONDITIONALLY (the falsifiable AmbientFrame
            // substrate), minting a retrieval_id for later outcome write-back.
            // Captures the full set before the top-K injection filter narrows it.
            if let Some(pool) = recognition_pool {
                let members: Vec<permagent::recognition::SetMember> = result
                    .merged_hits
                    .iter()
                    .enumerate()
                    .map(|(rank, hit)| (hit.id.clone(), hit.signal_score, rank as i64))
                    .collect();
                permagent::recognition::spawn_persist_recognition(
                    pool,
                    recognition_ctx.clone(),
                    user_query.to_string(),
                    "cascade".to_string(),
                    members,
                );
            }

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

            let top_hits = filter_recall_hits(&result.merged_hits);

            if top_hits.is_empty() {
                tracing::debug!(
                    target: "permagentd::brain",
                    "Recall returned no hits above {} threshold for query: {:?}",
                    RECALL_SCORE_FLOOR,
                    query_for_log
                );
                return 0;
            }

            let mut prefix = String::from("Relevant memories from past context:\n");
            for hit in &top_hits {
                prefix.push_str(&format!("- {}\n", hit.content));
            }

            let count = top_hits.len();
            tracing::info!(
                target: "permagentd::brain",
                "Recall injected {} memories into system prompt for query: {:?}",
                count,
                query_for_log
            );

            agent
                .extend_system_prompt("memory_recall".to_string(), prefix)
                .await;
            count
        }
        Err(e) => {
            tracing::warn!(
                target: "permagentd::brain",
                "Brain recall failed: {}",
                e
            );
            0
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
