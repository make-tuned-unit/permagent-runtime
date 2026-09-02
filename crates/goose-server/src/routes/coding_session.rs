//! Coding-session memory — the agent knows what you've been building.
//!
//! When a terminal tab that ran a coding harness (Claude Code, Codex, the
//! Permagent CLI) exits, the desktop ships the tail of the PTY transcript
//! here. A fast-model pass distills it into a short work summary and the
//! Brain remembers it (source `coding-session`), so "what am I working on?"
//! is answerable from real session content instead of guesses over browser
//! tabs (reported gap, 2026-08-06).
//!
//! Honesty law: no provider or a refusal ⇒ nothing is stored and the caller
//! is told so — a hollow "a session happened" memory is noise, not memory.

use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use permagent::conversation::message::Message;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::state::AppState;

#[derive(Deserialize)]
pub struct CodingSessionReq {
    /// Plain-text transcript tail (the caller strips ANSI). Bounded here too.
    pub transcript: String,
    pub cwd: Option<String>,
    /// The harness command that ran ("claude", "codex", "permagent run …").
    pub command: Option<String>,
    pub duration_secs: Option<u64>,
}

#[derive(Serialize)]
pub struct CodingSessionResp {
    pub stored: bool,
    pub summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingTurnReq {
    pub session_id: String,
    pub turn_idx: usize,
    pub user_text: String,
    pub assistant_text: String,
    pub working_dir: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CodingTurnResp {
    pub accepted: bool,
}

const MAX_TURN_CHARS: usize = 48_000;

fn bounded_text(value: String) -> String {
    value.chars().take(MAX_TURN_CHARS).collect()
}

/// Accept a completed Harness turn and let the daemon-owned Brain persist it.
///
/// The harness runs in its own process and never mounts a Brain — two writers
/// of one Spectral database is a corruption story — so it posts the turn to the
/// owner instead. This is deliberately the SAME `spawn_persist_chat_turn` a
/// Chat turn takes: same key shape, same wing decision, same metadata, so a
/// coding turn and a chat turn are the same kind of memory and recall does not
/// have to know which surface produced it.
///
/// The key is `(session_id, turn_idx)`, so client retries are idempotent.
async fn remember_coding_turn(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CodingTurnReq>,
) -> Result<Json<CodingTurnResp>, StatusCode> {
    if req.session_id.trim().is_empty()
        || req.user_text.trim().is_empty()
        || req.assistant_text.trim().is_empty()
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let brain = state
        .brain
        .as_ref()
        .cloned()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let pool = state.session_manager().pool_clone().await.ok();
    let cwd_evidence = req
        .working_dir
        .as_deref()
        .map(|cwd| format!("Harness working directory: {cwd}"))
        .unwrap_or_default();
    crate::brain_ops::spawn_persist_chat_turn(
        brain,
        pool,
        req.session_id,
        req.turn_idx,
        bounded_text(req.user_text),
        bounded_text(req.assistant_text),
        bounded_text(cwd_evidence),
    );
    Ok(Json(CodingTurnResp { accepted: true }))
}

/// Keep prompts bounded: the tail is what matters — it holds the final state,
/// the last test run, the wrap-up.
const MAX_TRANSCRIPT_CHARS: usize = 24_000;

fn tail_chars(s: &str, max: usize) -> &str {
    // nth(max-1) from the back is the byte offset of the max-th-from-last
    // char; None means the string is already short enough. `get` keeps the
    // slice lint-provably on a char boundary.
    match s.char_indices().rev().nth(max.saturating_sub(1)) {
        Some((i, _)) => s.get(i..).unwrap_or(s),
        None => s,
    }
}

async fn summarize(req: &CodingSessionReq) -> Option<String> {
    let config = permagent::config::Config::global();
    let provider_name = config.get_goose_provider().ok()?;
    let model_name = config.get_goose_model().ok()?;
    if provider_name.trim().is_empty() || model_name.trim().is_empty() {
        return None;
    }
    let provider =
        permagent::providers::create_with_named_model(&provider_name, &model_name, Vec::new())
            .await
            .ok()?;

    let system = "You summarize a coding-agent terminal session for the user's assistant's \
                  long-term memory. From the transcript tail, write ONE plain-prose summary \
                  (max 120 words) covering: what project/directory, what was worked on, what \
                  was accomplished or decided, and any unresolved next step. Ground every \
                  claim in the transcript — never invent. Reply ONLY as JSON: \
                  {\"summary\": \"<text, or empty if the transcript shows no real work>\"}";
    let user = Message::user().with_text(format!(
        "Directory: {}\nHarness: {}\nDuration: {} min\nTranscript tail:\n{}",
        req.cwd.as_deref().unwrap_or("(unknown)"),
        req.command.as_deref().unwrap_or("(unknown)"),
        req.duration_secs.unwrap_or(0) / 60,
        tail_chars(&req.transcript, MAX_TRANSCRIPT_CHARS),
    ));
    let (response, _usage) = provider
        .complete_fast(
            "coding-session-summary",
            system,
            std::slice::from_ref(&user),
            &[],
        )
        .await
        .ok()?;
    let text = response.as_concat_text();
    let (start, end) = (text.find('{')?, text.rfind('}')?);
    let v: serde_json::Value = serde_json::from_str(text.get(start..=end)?).ok()?;
    let summary = v.get("summary")?.as_str()?.trim().to_string();
    if summary.is_empty() {
        return None;
    }
    Some(summary)
}

async fn coding_session_summary(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CodingSessionReq>,
) -> Result<Json<CodingSessionResp>, (StatusCode, String)> {
    if req.transcript.trim().is_empty() {
        return Ok(Json(CodingSessionResp {
            stored: false,
            summary: None,
        }));
    }
    // Detach from the request's lifetime (the run_now lesson): the terminal
    // that posts this often closes moments later, and axum drops the handler
    // future on disconnect — which aborted the summary + Brain write mid-
    // flight. The spawned task survives; a client that waits gets the same
    // response.
    let task = tokio::spawn(async move { summarize_and_store(state, req).await });
    task.await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
}

async fn summarize_and_store(
    state: Arc<AppState>,
    req: CodingSessionReq,
) -> Result<Json<CodingSessionResp>, (StatusCode, String)> {
    let brain = state.brain.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Brain is not available".to_string(),
    ))?;

    let Some(summary) = summarize(&req).await else {
        return Ok(Json(CodingSessionResp {
            stored: false,
            summary: None,
        }));
    };

    let project = req
        .cwd
        .as_deref()
        .and_then(|c| c.rsplit('/').next())
        .unwrap_or("unknown-project");
    let stamp = chrono::Local::now().format("%Y-%m-%d-%H%M");
    let key = format!("coding-session-{project}-{stamp}");
    let content = format!(
        "Coding session ({}, {} in {}): {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M"),
        req.command.as_deref().unwrap_or("coding harness"),
        req.cwd.as_deref().unwrap_or("unknown directory"),
        summary
    );

    let device_id = *brain.device_id();
    brain
        .remember_with(
            &key,
            &content,
            spectral::RememberOpts {
                source: Some("coding-session".into()),
                device_id: Some(device_id),
                confidence: Some(1.0),
                visibility: spectral::Visibility::Private,
                wing: None,
                // The coding session is the episode (R45). The harness posts
                // one summary per session and sends no session id, so the
                // session's own memory key — project + start stamp — is the
                // stable identifier available here; it is derived from the
                // session, not minted per write, and a re-post of the same
                // session upserts into the same episode. When the harness
                // starts sending a session id, pass that instead.
                episode_id: Some(key.clone()),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("brain write failed: {e}"),
            )
        })?;

    tracing::info!(target: "coding_session", key = %key, "coding-session summary remembered");
    Ok(Json(CodingSessionResp {
        stored: true,
        summary: Some(summary),
    }))
}

/// What the harness announces when a turn ends. No numbers: see
/// [`announce_spend`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendAnnounceReq {
    /// The harness's own session id — the one that owns the `cost_ledger` rows.
    pub session_id: String,
    pub working_dir: Option<String>,
    /// The session is closing; this is its last word.
    #[serde(default)]
    pub final_turn: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendAnnounceResp {
    pub turn_usd: f64,
    pub session_usd: f64,
    pub today_usd: f64,
    pub total_tokens: i64,
    /// The last call had no published rate and was priced at the fail-closed
    /// worst case. Shown, not hidden — see [`permagent::events::SessionSpend`].
    pub estimated: bool,
}

/// "That turn is finished" — from the CLI harness, at the end of every turn.
///
/// ANNOUNCES, never posts. The body carries no tokens and no dollars, and that
/// is the whole design: the harness has ALREADY written its `cost_ledger` row,
/// in-process, through the same `append_cost_ledger` the daemon uses, into the
/// same `permagent.db`. Accepting the figures here and writing them again would
/// double every number in `accumulated_cost_usd` — the exact rollup the meter
/// reads — so the harness sends the one thing the daemon cannot know on its
/// own: that there is something new to look at, and under which session id.
///
/// The id is the point. The harness mints its own session (`cli.rs`'s
/// `get_or_create_session_id`, "CLI Session") and nothing ever told the UI it
/// existed; the Build tab's meter was subscribed to the browser's chat session,
/// which is idle for the entire time the user is coding. That is why it read
/// $0.00 all day while the terminal's own footer, reading the same ledger by
/// the right id, printed real money.
///
/// Emitting rather than answering is what makes the meter live: every open
/// window learns the new total on the same bus every other surface uses, with
/// nothing to poll. The response body repeats the figures for the caller's own
/// use (and so this is testable without a bus subscriber).
async fn announce_spend(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SpendAnnounceReq>,
) -> Result<Json<SpendAnnounceResp>, StatusCode> {
    let manager = state.session_manager();
    let session = manager
        .get_session(&req.session_id, false)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    // Midnight UTC, the same boundary `growth::metrics` measures days on.
    let today = chrono::Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|d| d.and_utc().to_rfc3339())
        .unwrap_or_default();
    // A failed rollup query must not lose the announcement: the session figures
    // are the ones the meter is for, and reporting them with today's total
    // missing beats reporting nothing.
    let today_usd = manager.spend_since(&today).await.unwrap_or(0.0);
    let last_call = manager
        .last_call_facts(&req.session_id)
        .await
        .ok()
        .flatten();

    let resp = SpendAnnounceResp {
        turn_usd: session.cost_usd.unwrap_or(0.0),
        session_usd: session.accumulated_cost_usd.unwrap_or(0.0),
        today_usd,
        total_tokens: session.accumulated_total_tokens.unwrap_or(0) as i64,
        estimated: last_call.as_ref().is_some_and(|c| c.estimated),
    };

    permagent::events::emit(permagent::events::session_spend_changed(
        permagent::events::SessionSpend {
            session_id: &req.session_id,
            turn_usd: resp.turn_usd,
            session_usd: resp.session_usd,
            today_usd: resp.today_usd,
            total_tokens: resp.total_tokens,
            provider: last_call.as_ref().and_then(|c| c.provider.as_deref()),
            model: last_call.as_ref().and_then(|c| c.model.as_deref()),
            working_dir: req.working_dir.as_deref(),
            estimated: last_call.as_ref().is_some_and(|c| c.estimated),
            final_turn: req.final_turn,
        },
    ));

    Ok(Json(resp))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/coding-sessions/summary", post(coding_session_summary))
        .route("/api/coding-sessions/spend", post(announce_spend))
        .route("/api/coding-sessions/turn", post(remember_coding_turn))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_keeps_the_end_and_respects_char_boundaries() {
        assert_eq!(tail_chars("hello", 10), "hello");
        assert_eq!(tail_chars("hello", 3), "llo");
        // Multi-byte safety: no mid-char slice panic.
        assert_eq!(tail_chars("héllo", 3), "llo");
        assert_eq!(tail_chars("naïve✻", 2), "e✻");
    }

    #[test]
    fn turn_payload_is_unicode_safe_and_bounded() {
        let long = "✻".repeat(MAX_TURN_CHARS + 10);
        let bounded = bounded_text(long);
        assert_eq!(bounded.chars().count(), MAX_TURN_CHARS);
        assert!(bounded.is_char_boundary(bounded.len()));
    }
}
