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
use permagent::agents::platform_extensions::terminal_supervision::{
    self as run_registry, HarnessRunSnapshot, HarnessRunUpdate,
};
use permagent::conversation::message::Message;
use permagent::session::{BudgetProjection, ProjectionCompleteness};
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

/// Canonical read model for a live harness run. `budget` is recomputed from
/// Spectral's session/ledger/reservation sources on every read. The two
/// optional scalar fields are retained only for older clients; they are not a
/// budget authority and must never be used to fill an unavailable projection.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessRunView {
    #[serde(flatten)]
    pub run: HarnessRunSnapshot,
    pub budget: BudgetProjection,
    /// Legacy compatibility field. Use `budget.task/session` instead.
    pub tokens: Option<i64>,
    /// Legacy compatibility field. Use `budget.task/session` instead.
    pub spend_usd: Option<f64>,
}

fn project_accumulated_tokens(tokens: Option<i32>) -> Option<i64> {
    tokens.map(i64::from)
}

fn usable_budget_projection(budget: BudgetProjection) -> Result<BudgetProjection, String> {
    if budget.session.completeness == ProjectionCompleteness::Unknown
        || (budget.task_id.is_some() && budget.task.completeness == ProjectionCompleteness::Unknown)
    {
        return Err(budget
            .provenance
            .error
            .clone()
            .unwrap_or_else(|| "budget projection is incomplete for a bound session".to_string()));
    }
    Ok(budget)
}

#[derive(Serialize)]
struct HarnessRunError {
    error: String,
}

async fn harness_run_view(
    state: &AppState,
    run: HarnessRunSnapshot,
) -> Result<HarnessRunView, String> {
    // Read the live config at the same boundary as budget authorization. A
    // projection query error is intentionally propagated: returning a valid
    // looking run with omitted/zero budget would be a false-zero contract.
    let budget = state
        .session_manager()
        .budget_projection(
            &run.session_id,
            permagent::cost_router::budget::load_budget_config(),
        )
        .await
        .map_err(|error| error.to_string())?;
    let budget = usable_budget_projection(budget)?;
    let ledger = state
        .session_manager()
        .get_session(&run.session_id, false)
        .await
        .ok();
    // An unbound task is a legitimate partial projection. It is explicit in
    // the canonical payload (null task id/unknown completeness), rather than
    // being converted into a synthetic task or a zero-spend fallback.
    Ok(HarnessRunView {
        budget,
        tokens: ledger
            .as_ref()
            .and_then(|s| project_accumulated_tokens(s.accumulated_total_tokens)),
        spend_usd: ledger.as_ref().and_then(|s| s.accumulated_cost_usd),
        run,
    })
}

fn projection_unavailable(error: impl std::fmt::Display) -> (StatusCode, Json<HarnessRunError>) {
    tracing::error!(target: "permagentd::harness", %error, "budget projection unavailable");
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(HarnessRunError {
            error: "budget projection unavailable".to_string(),
        }),
    )
}

fn durable_read_unavailable(error: impl std::fmt::Display) -> StatusCode {
    tracing::error!(target: "permagentd::harness", %error, "durable harness read unavailable");
    StatusCode::SERVICE_UNAVAILABLE
}

/// Upsert the bounded operational state of a coding harness run. This is the
/// write half of the DAG-1 observability contract; it does not accept PTY
/// text, raw transcripts, tokens, or cost figures. Bounded prompt context is
/// accepted for the live Council preflight, then redacted by the durable store.
async fn update_harness_run(
    State(state): State<Arc<AppState>>,
    Json(update): Json<HarnessRunUpdate>,
) -> Result<Json<HarnessRunView>, (StatusCode, Json<HarnessRunError>)> {
    let run_id = update.run_id.clone();
    // Keep the exact pre-update projection so a storage outage cannot erase
    // or replace a previously valid live row. Re-querying a bounded history
    // page on failure is insufficient: the run may fall outside that page.
    let previous = run_registry::harness_run_snapshot(&run_id);
    let run = run_registry::update_harness_run(update)
        .map_err(|error| (StatusCode::BAD_REQUEST, Json(HarnessRunError { error })))?;
    let durable = match state
        .session_manager()
        .upsert_harness_run_snapshot(&run)
        .await
    {
        Ok(snapshot) => snapshot,
        Err(error) => {
            // Do not leave an unpersisted update visible as live truth. Remove
            // the speculative local entry first, then restore the exact
            // pre-update projection (terminal precedence handles late races).
            run_registry::remove_harness_run(&run_id);
            if let Some(previous) = previous {
                run_registry::hydrate_harness_runs([previous]);
            }
            tracing::error!(target: "permagentd::harness", %error, "failed to persist harness run snapshot");
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(HarnessRunError {
                    error: "harness run persistence unavailable".to_string(),
                }),
            ));
        }
    };
    // A restarted daemon may receive a stale active update after a terminal
    // row was already persisted. Restore the store's authoritative terminal
    // result into the live projection before returning it.
    let run = if durable.status != run.status || durable.updated_at != run.updated_at {
        run_registry::hydrate_harness_runs([durable.clone()]);
        durable
    } else {
        run
    };
    if let Some(recommendation) = run_registry::claim_council_suggestion(&run_id) {
        permagent::events::emit(permagent::events::proactive_nudge(
            "council_suggestion",
            &run.project,
            &format!(
                "This request may benefit from a Council plan. Approve one Council pass from Build? {}",
                recommendation.reason
            ),
            1,
            &chrono::Utc::now().to_rfc3339(),
            None,
            None,
        ));
    }
    harness_run_view(&state, run)
        .await
        .map(Json)
        .map_err(projection_unavailable)
}

/// Read active in-flight runs as structured state. Henry and other read-only
/// consumers can use this endpoint without scraping PTY text.
async fn active_harness_runs(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<HarnessRunView>>, StatusCode> {
    let persisted = state
        .session_manager()
        .list_harness_run_snapshots(false, 64)
        .await
        .map_err(durable_read_unavailable)?;
    run_registry::hydrate_harness_runs(persisted);
    let runs = run_registry::list_active_harness_runs();
    let mut views = Vec::with_capacity(runs.len());
    for run in runs {
        let view = harness_run_view(&state, run).await.map_err(|error| {
            tracing::error!(target: "permagentd::harness", %error, "failed to project active harness run");
            StatusCode::SERVICE_UNAVAILABLE
        })?;
        views.push(view);
    }
    Ok(Json(views))
}

/// Read terminal evidence from Spectral-backed history. This is deliberately
/// separate from the active TTL projection so a completed run remains
/// inspectable after its in-memory entry is evicted or the daemon restarts.
async fn harness_run_history(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<HarnessRunView>>, StatusCode> {
    let runs = state
        .session_manager()
        .list_harness_run_snapshots(true, 64)
        .await
        .map_err(durable_read_unavailable)?;
    let mut views = Vec::with_capacity(runs.len());
    for run in runs {
        let view = harness_run_view(&state, run).await.map_err(|error| {
            tracing::error!(target: "permagentd::harness", %error, "failed to project historical harness run");
            StatusCode::SERVICE_UNAVAILABLE
        })?;
        views.push(view);
    }
    Ok(Json(views))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingTurnReq {
    pub session_id: String,
    pub turn_idx: usize,
    pub user_text: String,
    pub assistant_text: String,
    pub working_dir: Option<String>,
    /// Original event time in epoch milliseconds when supplied by the
    /// harness; omitted requests use the enqueue time.
    pub event_at: Option<i64>,
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
    if let Err(error) = crate::brain_ops::persist_chat_turn(
        brain,
        pool,
        req.session_id,
        req.turn_idx,
        req.event_at
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis()),
        bounded_text(req.user_text),
        bounded_text(req.assistant_text),
        bounded_text(cwd_evidence),
    )
    .await
    {
        tracing::warn!(target: "permagentd::brain", "chat memory enqueue failed: {error}");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
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
    /// Canonical budget-projection.v1 read model from Spectral. Legacy scalar
    /// fields below are derived compatibility values, never an authority.
    pub budget: BudgetProjection,
    /// Derived from the most recent durable session rollup after the canonical
    /// projection has succeeded. It is retained for old clients.
    pub turn_usd: f64,
    /// Derived from `budget.session.settled_usd` for old clients.
    pub session_usd: f64,
    /// Derived from the authoritative ledger query spanning all sessions.
    pub today_usd: f64,
    /// Derived from the durable session rollup for old clients.
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
) -> Result<Json<SpendAnnounceResp>, (StatusCode, Json<HarnessRunError>)> {
    let manager = state.session_manager();
    let session = manager
        .get_session(&req.session_id, false)
        .await
        .map_err(|_| {
            (
                StatusCode::NOT_FOUND,
                Json(HarnessRunError {
                    error: "coding session not found".to_string(),
                }),
            )
        })?;

    // This endpoint is a notification seam, but its response is also a
    // durable read contract. Query the one canonical Spectral projection
    // before constructing either the response or the event. A failed query
    // must be visible as unavailable; emitting a zero-shaped legacy frame
    // would make an outage indistinguishable from a genuinely free session.
    let budget = manager
        .budget_projection(
            &req.session_id,
            permagent::cost_router::budget::load_budget_config(),
        )
        .await
        .map_err(projection_unavailable)?;
    let budget = usable_budget_projection(budget).map_err(projection_unavailable)?;
    let session_usd = budget
        .session
        .settled_usd
        .ok_or_else(|| projection_unavailable("session settled spend is unavailable"))?;
    let turn_usd = session
        .cost_usd
        .or_else(|| (session_usd == 0.0).then_some(0.0))
        .ok_or_else(|| projection_unavailable("turn spend is unavailable"))?;

    // Midnight UTC, the same boundary `growth::metrics` measures days on.
    let today = chrono::Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|d| d.and_utc().to_rfc3339())
        .unwrap_or_default();
    let today_usd = manager
        .spend_since(&today)
        .await
        .map_err(projection_unavailable)?;
    let budget_json = serde_json::to_value(&budget).map_err(|error| {
        projection_unavailable(format!("budget projection serialization failed: {error}"))
    })?;
    // Copy the evidence needed by the legacy scalar fields before moving the
    // canonical projection into the response. The projection remains the
    // authority; these values are compatibility-only views of that evidence.
    let billing = budget.session_billing.clone();

    let resp = SpendAnnounceResp {
        budget,
        turn_usd,
        session_usd,
        today_usd,
        total_tokens: session.accumulated_total_tokens.unwrap_or(0) as i64,
        estimated: billing.is_estimated.unwrap_or(false),
    };

    permagent::events::emit(permagent::events::session_spend_changed(
        permagent::events::SessionSpend {
            session_id: &req.session_id,
            turn_usd: resp.turn_usd,
            session_usd: resp.session_usd,
            today_usd: resp.today_usd,
            total_tokens: resp.total_tokens,
            provider: billing.provider.as_deref(),
            model: billing.model.as_deref(),
            working_dir: req.working_dir.as_deref(),
            estimated: billing.is_estimated.unwrap_or(false),
            final_turn: req.final_turn,
            budget: Some(&budget_json),
        },
    ));

    Ok(Json(resp))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/coding-sessions/summary", post(coding_session_summary))
        .route("/api/coding-sessions/spend", post(announce_spend))
        .route("/api/coding-sessions/turn", post(remember_coding_turn))
        .route(
            "/api/coding-sessions/harness-runs",
            post(update_harness_run).get(active_harness_runs),
        )
        .route(
            "/api/coding-sessions/harness-runs/history",
            axum::routing::get(harness_run_history),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use permagent::agents::platform_extensions::terminal_supervision::{
        CouncilRecommendation, HarnessRunStatus,
    };
    use permagent::session::{
        BillingEvidence, BudgetScopeProjection, CapTriplet, ProjectionBand, ProjectionProvenance,
        BUDGET_PROJECTION_VERSION,
    };

    fn sample_budget() -> BudgetProjection {
        let cap = CapTriplet {
            soft_usd: Some(2.0),
            gate_usd: Some(5.0),
            hard_usd: Some(10.0),
            source: "current_budget_config".to_string(),
        };
        let scope = || BudgetScopeProjection {
            cap: cap.clone(),
            settled_usd: Some(1.25),
            held_usd: Some(0.0),
            unknown_usd: Some(0.0),
            effective_used_usd: Some(1.25),
            remaining_usd: Some(8.75),
            band: Some(ProjectionBand::Ok),
            completeness: ProjectionCompleteness::Complete,
            error: None,
        };
        let evidence = || BillingEvidence {
            billing_class: Some("paid_api".to_string()),
            provider: Some("fixture".to_string()),
            model: Some("fixture-model".to_string()),
            call_id: Some("call-1".to_string()),
            is_estimated: Some(false),
            observed_at: Some(Utc::now().to_rfc3339()),
            source: "cost_ledger".to_string(),
        };
        BudgetProjection {
            task_id: Some("task-1".to_string()),
            root_session_id: "session-1".to_string(),
            task: scope(),
            session: scope(),
            task_billing: evidence(),
            session_billing: evidence(),
            provenance: ProjectionProvenance {
                version: BUDGET_PROJECTION_VERSION.to_string(),
                as_of: Utc::now().to_rfc3339(),
                completeness: ProjectionCompleteness::Complete,
                sources: vec!["sessions".to_string(), "cost_ledger".to_string()],
                error: None,
            },
        }
    }

    fn sample_run() -> HarnessRunSnapshot {
        let now = Utc::now();
        HarnessRunSnapshot {
            run_id: "run-1".to_string(),
            session_id: "session-1".to_string(),
            project: "project".to_string(),
            prompt_title: "prompt".to_string(),
            prompt_digest: "digest".to_string(),
            task_version: None,
            envelope_id: None,
            prompt_context: None,
            council_recommendation: CouncilRecommendation {
                recommended: false,
                reason: "fixture".to_string(),
                signals: Vec::new(),
            },
            dag_nodes: Vec::new(),
            dependencies: Vec::new(),
            active_node: None,
            worker: None,
            provider: None,
            model: None,
            billing_class: None,
            tier: None,
            routing_reason: None,
            status: HarnessRunStatus::Running,
            declared_verification: None,
            last_verification: None,
            verification_attempts: None,
            verification_verdict: None,
            pending_gate: None,
            retry_count: None,
            tool_calls: None,
            gate_attempts: None,
            evidence: None,
            result: None,
            parent_run_id: None,
            parent_session_id: None,
            started_at: now,
            updated_at: now,
            elapsed_ms: 0,
        }
    }

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

    #[test]
    fn token_projection_preserves_unknown_as_null() {
        assert_eq!(project_accumulated_tokens(None), None);
        assert_eq!(project_accumulated_tokens(Some(12_800)), Some(12_800));
    }

    #[test]
    fn harness_run_view_exposes_versioned_budget_and_legacy_compatibility_fields() {
        let view = HarnessRunView {
            run: sample_run(),
            budget: sample_budget(),
            tokens: Some(12_800),
            spend_usd: Some(1.25),
        };
        let json = serde_json::to_value(view).unwrap();
        assert_eq!(
            json["budget"]["provenance"]["version"],
            BUDGET_PROJECTION_VERSION
        );
        assert_eq!(json["budget"]["task"]["remainingUsd"], 8.75);
        assert_eq!(json["tokens"], 12_800);
        assert_eq!(json["spendUsd"], 1.25);
        assert!(json["budget"]["session"]["remainingUsd"]
            .as_f64()
            .is_some_and(|remaining| remaining >= 0.0));
    }

    #[test]
    fn projection_unavailable_is_explicit_service_unavailable() {
        let (status, body) = projection_unavailable("fixture query failure");
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        let json = serde_json::to_value(body.0).unwrap();
        assert_eq!(json["error"], "budget projection unavailable");
    }

    #[test]
    fn durable_read_failure_is_explicit_service_unavailable() {
        assert_eq!(
            durable_read_unavailable("fixture database failure"),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn bound_unknown_projection_is_not_presented_as_a_zero() {
        let mut bound = sample_budget();
        bound.task.completeness = ProjectionCompleteness::Unknown;
        bound.task.error = Some("invalid cap".to_string());
        bound.provenance.error = Some("task: invalid cap".to_string());
        assert!(usable_budget_projection(bound).is_err());

        let mut unbound = sample_budget();
        unbound.task_id = None;
        unbound.task.completeness = ProjectionCompleteness::Unknown;
        unbound.task.error = Some("session is not bound to a durable task".to_string());
        unbound.provenance.completeness = ProjectionCompleteness::Partial;
        unbound.provenance.error = Some("task: session is not bound".to_string());
        assert!(usable_budget_projection(unbound).is_ok());
    }

    #[test]
    fn authoritative_zero_projection_remains_successful_zero() {
        let mut empty = sample_budget();
        for scope in [&mut empty.task, &mut empty.session] {
            scope.settled_usd = Some(0.0);
            scope.held_usd = Some(0.0);
            scope.unknown_usd = Some(0.0);
            scope.effective_used_usd = Some(0.0);
            scope.remaining_usd = Some(10.0);
            scope.band = Some(ProjectionBand::Ok);
            scope.completeness = ProjectionCompleteness::Complete;
            scope.error = None;
        }
        assert!(usable_budget_projection(empty).is_ok());
    }

    #[test]
    fn pending_and_unknown_holds_are_visible_not_coerced_to_zero() {
        let mut held = sample_budget();
        held.session.settled_usd = Some(0.0);
        held.session.held_usd = Some(0.75);
        held.session.unknown_usd = Some(0.25);
        held.session.effective_used_usd = Some(1.0);
        held.session.remaining_usd = Some(9.0);
        held.session.band = Some(ProjectionBand::Unknown);
        held.session.completeness = ProjectionCompleteness::Partial;
        assert!(usable_budget_projection(held.clone()).is_ok());
        assert_eq!(held.session.settled_usd, Some(0.0));
        assert_eq!(held.session.held_usd, Some(0.75));
        assert_eq!(held.session.unknown_usd, Some(0.25));
        assert_eq!(held.session.band, Some(ProjectionBand::Unknown));
    }

    #[test]
    fn unavailable_projection_is_rejected_before_event_construction() {
        let mut unavailable = sample_budget();
        unavailable.session.completeness = ProjectionCompleteness::Unknown;
        unavailable.session.settled_usd = None;
        unavailable.session.held_usd = None;
        unavailable.session.unknown_usd = None;
        unavailable.session.effective_used_usd = None;
        unavailable.session.remaining_usd = None;
        unavailable.session.band = Some(ProjectionBand::Unknown);
        unavailable.session.error = Some("query failed".to_string());
        unavailable.provenance.completeness = ProjectionCompleteness::Unknown;
        unavailable.provenance.error = Some("session: query failed".to_string());
        assert!(usable_budget_projection(unavailable).is_err());
    }

    #[test]
    fn stale_active_runs_expire_from_live_view_but_remain_addressable_for_history() {
        let mut stale = sample_run();
        stale.run_id = format!("b5-ttl-{}", uuid::Uuid::new_v4());
        stale.updated_at = Utc::now() - chrono::Duration::seconds(46);
        run_registry::hydrate_harness_runs([stale.clone()]);

        assert!(!run_registry::list_active_harness_runs()
            .into_iter()
            .any(|run| run.run_id == stale.run_id));
        assert_eq!(
            run_registry::harness_run_snapshot(&stale.run_id)
                .expect("expired entry remains addressable for durable history lookup")
                .run_id,
            stale.run_id
        );
        run_registry::remove_harness_run(&stale.run_id);
    }
}
