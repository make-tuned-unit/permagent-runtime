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

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/coding-sessions/summary", post(coding_session_summary))
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
}
