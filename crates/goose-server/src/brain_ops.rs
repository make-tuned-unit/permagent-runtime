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
    brain: &Arc<spectral::Brain>,
    agent: &Arc<permagent::agents::Agent>,
    user_query: &str,
    recognition_ctx: spectral::graph::RecognitionContext,
) -> usize {
    if user_query.is_empty() {
        return 0;
    }

    let brain = brain.clone();
    let query = user_query.to_string();
    let query_for_log = user_query.chars().take(80).collect::<String>();
    let recall_result = tokio::task::spawn_blocking(move || {
        brain.recall_cascade(&query, &recognition_ctx, &Default::default())
    })
    .await;

    match recall_result {
        Ok(Ok(result)) => {
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
        Ok(Err(e)) => {
            tracing::warn!(
                target: "permagentd::brain",
                "Brain recall failed: {}",
                e
            );
            0
        }
        Err(e) => {
            tracing::warn!(
                target: "permagentd::brain",
                "Brain recall spawn_blocking panicked: {}",
                e
            );
            0
        }
    }
}

// ── Chat turn persistence ────────────────────────────────────────────────

/// Persist a chat turn's memories via Brain::remember_with.
/// Spawns a detached background task — fire-and-forget.
/// Wraps spawn_blocking and error handling with tracing.
pub fn spawn_persist_chat_turn(
    brain: Arc<spectral::Brain>,
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

        let result = tokio::task::spawn_blocking(move || {
            brain.remember_with(
                &key,
                &content,
                spectral::RememberOpts {
                    source: Some("chat".into()),
                    device_id: Some(device_id),
                    confidence: Some(1.0),
                    visibility: spectral::Visibility::Private,
                    wing: None,
                    ..Default::default()
                },
            )
        })
        .await;

        match result {
            Ok(Ok(_)) => {
                tracing::info!(
                    target: "permagentd::brain",
                    "Remembered chat turn: {}",
                    key_for_log
                );
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    target: "permagentd::brain",
                    "Failed to remember chat turn {}: {}",
                    key_for_log,
                    e
                );
            }
            Err(e) => {
                tracing::warn!(
                    target: "permagentd::brain",
                    "spawn_blocking panicked for remember {}: {}",
                    key_for_log,
                    e
                );
            }
        }
    });
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
