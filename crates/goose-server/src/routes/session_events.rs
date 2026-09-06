use crate::routes::errors::ErrorResponse;
use crate::routes::reply::{get_token_state, track_tool_telemetry, MessageEvent};
use crate::session_event_bus::RequestGuard;
use crate::state::AppState;
use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{self, HeaderMap},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use bytes::Bytes;
use futures::{stream::StreamExt, Stream};
use permagent::agents::{AgentEvent, SessionConfig};
use permagent::conversation::message::Message;
use permagent::conversation::Conversation;
use serde::{Deserialize, Serialize};
use std::{
    convert::Infallible,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_stream::wrappers::ReceiverStream;

// ── Request / Response types ────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct SessionReplyRequest {
    /// Client-generated UUIDv7 identifying this request.
    pub request_id: String,
    pub user_message: Message,
    #[serde(default)]
    pub override_conversation: Option<Vec<Message>>,
    /// Current UI state sent by the frontend (tab, selection, etc.).
    #[serde(default)]
    pub app_context: Option<AppContext>,
    /// Durable uploads included in this turn. Each ID is validated against the
    /// current session and linked to the persisted user message/request ID.
    #[serde(default)]
    pub attachment_ids: Vec<String>,
}

/// Snapshot of the frontend's current UI state, sent with each chat message.
#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct AppContext {
    /// Internal tool_type of the active workspace panel (e.g. "memory", "build").
    pub current_tab: String,
    /// If an overlay is open (e.g. "settings"), its panel name.
    #[serde(default)]
    pub active_panel: Option<String>,
    /// ID of a selected item in the current view (memory ID, recipe ID, etc.).
    #[serde(default)]
    pub selected_id: Option<String>,
    /// Opaque per-view state the receiving component may interpret.
    #[serde(default)]
    pub view_state: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SessionReplyResponse {
    pub request_id: String,
}

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct CancelRequest {
    pub request_id: String,
}

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct CancelResponse {
    /// True when a live request was found and its cancellation was triggered —
    /// a terminal `Finish { reason: "stop" }` will follow on the SSE stream.
    /// False when there was nothing to cancel (unknown/stale request_id, e.g.
    /// the turn already finished or the daemon restarted): no terminal frame
    /// will ever arrive for that id, so the client must reconcile its own
    /// streaming state instead of waiting for one.
    pub cancelled: bool,
}

// ── SSE Event Stream Response ───────────────────────────────────────────

/// An SSE response that includes `id:` lines for Last-Event-ID reconnection.
pub struct SseEventStream {
    rx: ReceiverStream<String>,
}

impl SseEventStream {
    fn new(rx: ReceiverStream<String>) -> Self {
        Self { rx }
    }
}

impl Stream for SseEventStream {
    type Item = Result<Bytes, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.rx)
            .poll_next(cx)
            .map(|opt| opt.map(|s| Ok(Bytes::from(s))))
    }
}

impl IntoResponse for SseEventStream {
    fn into_response(self) -> axum::response::Response {
        let body = axum::body::Body::from_stream(self);
        http::Response::builder()
            .header("Content-Type", "text/event-stream")
            .header("Cache-Control", "no-cache")
            .header("Connection", "keep-alive")
            .body(body)
            .unwrap()
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn format_sse_event(seq: u64, json: &str) -> String {
    format!("id: {}\ndata: {}\n\n", seq, json)
}

/// Build the system-prompt-extra block for a "Discuss with {persona}" deep-link
/// (#303). Renders the decision's substance and instructs the agent to open the
/// conversation already knowing it — no persona name is hardcoded here; the
/// agent's identity comes from the global persona prompt, so the opening turn is
/// in character automatically. `detail` already carries attribution, evidence
/// refs, and (for choices) option consequences (decision sink), so it is the
/// richest single field to seed.
fn format_decision_discussion_block(
    d: &permagent::decisions::Decision,
    evidence_block: Option<&str>,
) -> String {
    let mut b = String::from(
        "The user opened this chat from their Decision Inbox to talk through a specific \
decision with you. You already have its full context below — do not ask them to re-explain \
it. Open the conversation in character: briefly recap what the decision is about, what was \
proposed and why, then ask what their question or concern is. Do not give a generic \
\"what would you like to discuss?\" opener.\n\nDecision under discussion:\n",
    );
    b.push_str(&format!("- Summary: {}\n", d.headline));
    b.push_str(&format!("- Type: {}\n", d.kind));
    b.push_str(&format!("- Status: {}\n", d.status));
    if let Some(ref goal) = d.goal_id {
        b.push_str(&format!("- Related goal id: {}\n", goal));
    }
    if !d.detail.trim().is_empty() {
        b.push_str(&format!("- Details: {}\n", d.detail.trim()));
    }
    // S3 (#429): a session_gate decision additionally hydrates the LIVE
    // supervised-session state from the registry (the DB row is a snapshot
    // from filing time; the session may have moved on — answered, completed,
    // died). The #303 context-load itself is reused verbatim; this only adds
    // the registry's current view for the kinds that have one.
    if d.kind == "session_gate" {
        if let Some(target) = d.payload.get("target_session_id").and_then(|v| v.as_str()) {
            match permagent::agents::platform_extensions::terminal_supervision::session_snapshot(
                target,
            ) {
                Some(snap) => b.push_str(&format!(
                    "- Live session state: {:?}, {} pending gate(s) (project '{}')\n",
                    snap.status,
                    snap.pending_gates.len(),
                    snap.project_slug,
                )),
                None => b.push_str(
                    "- Live session state: unknown — the session is not in the supervision \
                     registry (daemon restarted since the gate was filed?)\n",
                ),
            }
        }
    }
    // Layer 3: structured proof-of-work captured at goal completion, so the
    // review is grounded in ground truth (worktree path, commit SHA, push
    // target, diffstat, worker summary) instead of improvised shell commands
    // against a stale local checkout.
    if let Some(block) = evidence_block {
        b.push('\n');
        b.push_str(block);
        b.push('\n');
    }
    b
}

fn serialize_session_event(seq: u64, request_id: Option<&str>, event: &MessageEvent) -> String {
    // Build JSON payload: { request_id?: string, ...event_fields }
    // We flatten request_id into the event JSON.
    let mut event_json = serde_json::to_value(event).unwrap_or_else(
        |e| serde_json::json!({"type": "Error", "error": format!("Serialization error: {}", e)}),
    );

    if let Some(rid) = request_id {
        if let serde_json::Value::Object(ref mut map) = event_json {
            // Always insert chat_request_id for routing (the chat UUID that
            // the frontend registered its listener under).
            map.insert(
                "chat_request_id".to_string(),
                serde_json::Value::String(rid.to_string()),
            );
            // Also set request_id if the event doesn't already carry one
            // (e.g. Notification events have their own request_id for tool-call matching)
            map.entry("request_id")
                .or_insert_with(|| serde_json::Value::String(rid.to_string()));
        }
    }

    let json_str = serde_json::to_string(&event_json).unwrap_or_default();
    format_sse_event(seq, &json_str)
}

// ── GET /sessions/{id}/events ───────────────────────────────────────────

/// Query parameters for the SSE endpoint. `last_event_id` mirrors the
/// `Last-Event-ID` header for EventSource clients, which cannot set request
/// headers when they construct a fresh connection (the browser only sends the
/// header on its own automatic reconnects — not on the manual
/// close-and-reconnect the store's backoff loop performs).
#[derive(Debug, Deserialize)]
pub struct SessionEventsQuery {
    pub last_event_id: Option<String>,
}

#[utoipa::path(
    get,
    path = "/sessions/{id}/events",
    params(
        ("id" = String, Path, description = "Session ID"),
        ("last_event_id" = Option<String>, Query,
         description = "Resume event replay after this sequence number. \
                        Query-param mirror of the Last-Event-ID header for \
                        EventSource clients that cannot set headers."),
    ),
    responses(
        (status = 200, description = "SSE event stream",
         body = MessageEvent,
         content_type = "text/event-stream"),
        (status = 404, description = "Session not found"),
    )
)]
pub async fn session_events(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<SessionEventsQuery>,
    headers: HeaderMap,
) -> Result<SseEventStream, axum::http::StatusCode> {
    // Validate the session exists before creating an event bus.
    state
        .session_manager()
        .get_session(&session_id, false)
        .await
        .map_err(|_| axum::http::StatusCode::NOT_FOUND)?;

    // Header wins over the query param: on a browser-native auto-reconnect the
    // header carries a fresher cursor than the URL captured at construction.
    // Both parse leniently — a malformed cursor degrades to a full replay
    // rather than a 400 on the streaming endpoint.
    let last_event_id: Option<u64> = headers
        .get("Last-Event-ID")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .or_else(|| query.last_event_id.as_deref().and_then(|s| s.parse().ok()));

    let bus = state.get_or_create_event_bus(&session_id).await;

    let (replay, replay_max_seq, mut live_rx) = match bus.subscribe(last_event_id).await {
        Ok(result) => result,
        Err(_) => {
            // Client's Last-Event-ID has been evicted from the replay buffer.
            // Send a single error event so the client knows to reload.
            let (tx, rx) = mpsc::channel::<String>(1);
            let stream = ReceiverStream::new(rx);
            let seq = 0;
            let error_event = MessageEvent::Error {
                error: "Client too far behind — reload conversation".to_string(),
            };
            let frame = serialize_session_event(seq, None, &error_event);
            tokio::spawn(async move {
                let _ = tx.send(frame).await;
            });
            return Ok(SseEventStream::new(stream));
        }
    };

    let (tx, rx) = mpsc::channel::<String>(256);
    let stream = ReceiverStream::new(rx);
    let task_bus = bus.clone();

    tokio::spawn(async move {
        let bus = task_bus;

        // Notify the client about in-flight requests BEFORE replay so it can
        // register event handlers before replayed events arrive. ALWAYS
        // emitted — an EMPTY list is the "nothing is running" reconciliation
        // signal (C1): after a daemon restart mid-turn there is no terminal
        // Finish/Error anywhere (fresh bus, empty replay buffer), and without
        // this frame a reconnecting client that still believes a turn is live
        // stays wedged on "Agent is responding…" forever. Emitted without an
        // SSE `id:` field so it doesn't regress the client's Last-Event-ID
        // cursor.
        let active_ids = bus.active_request_ids().await;
        let event = MessageEvent::ActiveRequests {
            request_ids: active_ids,
        };
        let json_str = serde_json::to_string(&serde_json::to_value(&event).unwrap_or_default())
            .unwrap_or_default();
        let frame = format!("data: {}\n\n", json_str);
        if tx.send(frame).await.is_err() {
            return;
        }

        // Send replayed events
        for event in &replay {
            let frame =
                serialize_session_event(event.seq, event.request_id.as_deref(), &event.event);
            if tx.send(frame).await.is_err() {
                return;
            }
        }

        // Send live events + heartbeat pings
        let mut heartbeat_interval = tokio::time::interval(Duration::from_millis(500));
        // Heartbeat uses a local counter — not stored in the replay buffer
        let mut heartbeat_seq = 0u64;

        loop {
            tokio::select! {
                _ = heartbeat_interval.tick() => {
                    // Send heartbeat directly without publishing to the bus,
                    // so pings don't evict real events from the replay buffer.
                    // Use a comment-style SSE id so it won't interfere with Last-Event-ID.
                    let frame = format!(": ping {}\n\n", heartbeat_seq);
                    heartbeat_seq += 1;
                    if tx.send(frame).await.is_err() {
                        return;
                    }
                }
                result = live_rx.recv() => {
                    match result {
                        Ok(event) => {
                            // Skip events already covered by replay to avoid duplicates
                            // at the replay/live handoff boundary.
                            if event.seq <= replay_max_seq {
                                continue;
                            }
                            let frame = serialize_session_event(
                                event.seq,
                                event.request_id.as_deref(),
                                &event.event,
                            );
                            if tx.send(frame).await.is_err() {
                                return;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("SSE subscriber lagged by {} events, closing stream so client reconnects with Last-Event-ID", n);
                            // Close the stream so the client reconnects and
                            // replays missed events from the buffer.
                            return;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            return;
                        }
                    }
                }
            }
        }
    });

    Ok(SseEventStream::new(stream))
}

// ── POST /sessions/{id}/reply ───────────────────────────────────────────

#[utoipa::path(
    post,
    path = "/sessions/{id}/reply",
    params(
        ("id" = String, Path, description = "Session ID"),
    ),
    request_body = SessionReplyRequest,
    responses(
        (status = 200, description = "Request accepted",
         body = SessionReplyResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 424, description = "Agent not initialized"),
        (status = 500, description = "Internal server error"),
    )
)]
pub async fn session_reply(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(request): Json<SessionReplyRequest>,
) -> Result<Json<SessionReplyResponse>, ErrorResponse> {
    let request_id = request.request_id.clone();

    // Validate request_id is a valid UUID
    if uuid::Uuid::parse_str(&request_id).is_err() {
        return Err(ErrorResponse::bad_request(
            "request_id must be a valid UUID",
        ));
    }

    // Validate session exists before allocating a bus/registering work
    let session_data = state
        .session_manager()
        .get_session(&session_id, false)
        .await
        .map_err(|_| ErrorResponse::not_found(format!("Session {} not found", session_id)))?;

    let session_start = std::time::Instant::now();

    // Activity: chat turn started
    permagent::events::activity::emit_activity(permagent::events::activity::chat_turn_started(
        &session_id,
    ));

    tracing::info!(
        monotonic_counter.goose.session_starts = 1,
        session_type = "app",
        interface = "ui",
        "Session started"
    );

    if let Some(ref recipe) = session_data.recipe {
        if state.mark_recipe_run_if_absent(&session_id).await {
            tracing::info!(
                monotonic_counter.goose.recipe_runs = 1,
                recipe_name = %recipe.title,
                recipe_version = %recipe.version,
                session_type = "app",
                interface = "ui",
                "Recipe execution started"
            );
        }
    }

    let bus = state.get_or_create_event_bus(&session_id).await;

    let cancel_token = bus
        .try_register_request(request_id.clone())
        .await
        .map_err(|_| {
            ErrorResponse::bad_request("Session already has an active request. Cancel it first.")
        })?;

    if !request.attachment_ids.is_empty() {
        let pool =
            state.session_manager().pool_clone().await.map_err(|e| {
                ErrorResponse::internal(format!("Failed to open session store: {e}"))
            })?;
        let linked = permagent::attachments::link_many_to_message_for_session(
            &pool,
            &session_id,
            &request.attachment_ids,
            &request_id,
        )
        .await
        .map_err(|e| ErrorResponse::internal(format!("Failed to link attachments: {e}")))?;
        if !linked {
            return Err(ErrorResponse::bad_request(format!(
                "One or more attachments do not belong to session {}",
                session_id
            )));
        }
    }

    // Sync session provider/model with current global config so stale
    // sessions (created under a previous provider) pick up the user's
    // latest Settings choice.
    //
    // Ordering matters (re-enable-gate epic part B): this must run AFTER
    // `try_register_request`. It evicts the cached agent, and doing that
    // before the active-request check orphaned a live parked/running turn —
    // the turn kept running on the unreachable Arc while a later
    // Decision-Inbox answer reached a freshly recreated agent with no waiter.
    // A session with an active bus request now 400s above with the agent
    // intact, and the swap is deferred to its next idle turn.
    //
    // This sync is AMBIENT: it fires on a session's next turn whenever the
    // process-wide config no longer matches what the session recorded, and the
    // session's own user did nothing to cause it. It used to announce itself in
    // the transcript ("Model switched: this session now uses X (was Y).") — a
    // banner every reopened session printed after any Settings change, in every
    // other open tab, and once per surface whenever anything else wrote the
    // global provider/model. It reads as a fault the user did not cause.
    // An EXPLICIT switch is announced by the surface that performed it: the
    // desktop picker calls POST /agent/update_provider, and the harness `/model`
    // prints its own one-line confirmation (4ef36237). Neither goes through
    // here, so removing the notice costs no user-initiated feedback — it only
    // silences the ambient case, which was the whole of the spam.
    {
        let config = permagent::config::Config::global();
        let current_provider = config.get_goose_provider().ok();
        let current_model = config.get_goose_model().ok();

        let provider_stale = current_provider.is_some()
            && current_provider.as_deref() != session_data.provider_name.as_deref();
        let model_stale = current_model.is_some()
            && current_model.as_deref()
                != session_data
                    .model_config
                    .as_ref()
                    .map(|m| m.model_name.as_str());

        // Holding the bus request slot rules out a concurrent interactive
        // turn, but not turns the bus can't see: an orchestrator-driven turn
        // (registered cancel token on the AgentManager — evicting would cancel
        // it) or a turn parked on a tool-confirmation waiter (e.g. driven via
        // the gateway). Defer the whole sync in those cases — skipping only
        // the eviction would persist the new provider in session metadata and
        // make the session look non-stale on the next turn, so the live agent
        // would never pick up the swap.
        let busy_outside_bus = state.agent_manager.is_session_busy(&session_id).await
            || state
                .agent_manager
                .session_has_pending_confirmation(&session_id)
                .await;

        if (provider_stale || model_stale) && busy_outside_bus {
            tracing::info!(
                session_id = %session_id,
                "Deferring provider/model sync: session has a live turn outside the event bus"
            );
        } else if provider_stale || model_stale {
            let mut update = state.session_manager().update(&session_id);
            if let Some(ref provider) = current_provider {
                update = update.provider_name(provider.clone());
            }
            if let Some(ref model_name) = current_model {
                if let Ok(mc) = permagent::model::ModelConfig::new(model_name) {
                    update = update.model_config(mc);
                }
            }
            if let Err(e) = update.apply().await {
                tracing::warn!("Failed to sync session provider: {}", e);
            } else {
                tracing::info!(
                    "Synced session {} provider {:?} -> {:?}",
                    session_id,
                    session_data.provider_name,
                    current_provider
                );
                // Deliberately not announced in the transcript — see the note
                // above the block. The tracing line is the record.
                // Evict cached agent so it gets recreated with the new provider
                let _ = state.agent_manager.remove_session(&session_id).await;
            }
        }
    }

    let mut user_message = request.user_message;
    // Uploaded attachment messages may not carry their own id. Reuse the
    // request id as a stable target for the persisted message row.
    if user_message.id.is_none() {
        user_message.id = Some(request_id.clone());
    }

    // Diagnostic: log incoming content block types
    {
        let mut text_count = 0u32;
        let mut image_count = 0u32;
        let mut other_count = 0u32;
        for content in &user_message.content {
            match content {
                permagent::conversation::message::MessageContent::Text(_) => text_count += 1,
                permagent::conversation::message::MessageContent::Image(_) => image_count += 1,
                _ => other_count += 1,
            }
        }
        tracing::info!(
            text_blocks = text_count,
            image_blocks = image_count,
            other_blocks = other_count,
            total_blocks = user_message.content.len(),
            "[session_reply] user_message content blocks"
        );
    }

    let override_conversation = request.override_conversation;
    let app_context = request.app_context;

    let task_state = state.clone();
    let task_session_id = session_id.clone();
    let task_request_id = request_id.clone();
    let task_cancel = cancel_token.clone();
    let task_bus = bus.clone();

    drop(tokio::spawn(async move {
        let mut _guard = RequestGuard::new(task_bus.clone(), task_request_id.clone());

        let publish = |rid: Option<String>, event: MessageEvent| {
            let bus = task_bus.clone();
            async move {
                bus.publish(rid, event).await;
            }
        };

        let agent = match task_state.get_agent(task_session_id.clone()).await {
            Ok(agent) => agent,
            Err(e) => {
                tracing::error!("Failed to get session agent: {}", e);
                publish(
                    Some(task_request_id.clone()),
                    MessageEvent::Error {
                        error: format!("Failed to get session agent: {}", e),
                    },
                )
                .await;
                // This Error IS the terminal frame — disarm so the guard's
                // armed-drop handler doesn't publish a duplicate one.
                _guard.disarm();
                task_bus.cleanup_request(&task_request_id).await;
                return;
            }
        };

        // Apply the configured CHAT provider/model to this turn after the
        // request has claimed the event-bus slot. `apply_chat_model` is
        // deliberately best-effort: an invalid or unreachable configured
        // route leaves the session provider in place, so a settings mistake
        // cannot take down the user's next reply.
        crate::chat_model::apply_chat_model(&agent, &task_session_id).await;

        let session = match task_state
            .session_manager()
            .get_session(&task_session_id, true)
            .await
        {
            Ok(metadata) => metadata,
            Err(e) => {
                tracing::error!("Failed to read session for {}: {}", task_session_id, e);
                publish(
                    Some(task_request_id.clone()),
                    MessageEvent::Error {
                        error: format!("Failed to read session: {}", e),
                    },
                )
                .await;
                // Terminal frame published — disarm (see above).
                _guard.disarm();
                task_bus.cleanup_request(&task_request_id).await;
                return;
            }
        };

        let session_config = SessionConfig {
            id: task_session_id.clone(),
            schedule_id: session.schedule_id.clone(),
            max_turns: None,
            retry_config: None,
        };

        let mut all_messages = match override_conversation {
            Some(history) => {
                let conv = Conversation::new_unvalidated(history);
                if let Err(e) = task_state
                    .session_manager()
                    .replace_conversation(&task_session_id, &conv)
                    .await
                {
                    tracing::warn!(
                        "Failed to replace session conversation for {}: {}",
                        task_session_id,
                        e
                    );
                }
                conv
            }
            None => session.conversation.unwrap_or_default(),
        };
        all_messages.push(user_message.clone());

        // ── Phase 3b: Ambient context from ContextBuilder ──
        {
            if let Some(digest) =
                crate::brain_ops::inject_ambient_context(&task_state, &agent).await
            {
                // Emit ContextAttached so the frontend can show citation markers
                if !digest.probed_memories.is_empty() || !digest.recalled_memories.is_empty() {
                    use crate::routes::reply::{ProbedMemoryRef, RecalledMemoryRef};
                    let probed: Vec<ProbedMemoryRef> = digest
                        .probed_memories
                        .iter()
                        .map(|m| ProbedMemoryRef {
                            id: m.id.clone(),
                            key: m.key.clone(),
                            content_summary: m.content.chars().take(200).collect(),
                            relevance: m.relevance,
                            wing: m.wing.clone(),
                        })
                        .collect();
                    let recalled: Vec<RecalledMemoryRef> = digest
                        .recalled_memories
                        .iter()
                        .map(|m| RecalledMemoryRef {
                            id: m.source.clone().unwrap_or_default(),
                            signal_score: m.signal_score,
                            content_summary: m.content.chars().take(200).collect(),
                        })
                        .collect();
                    publish(
                        Some(task_request_id.clone()),
                        MessageEvent::ContextAttached {
                            probed_memories: probed,
                            recalled_memories: recalled,
                        },
                    )
                    .await;
                }
            }
        }

        // ── Phase 3: Recall from brain before model invocation ──
        let recall_trace = if let Some(brain) = task_state.brain.as_ref() {
            let user_query = user_message.as_concat_text();
            let recognition_ctx = task_state.build_recognition_context(Some(&task_session_id));
            let recognition_pool = task_state.session_manager().pool_clone().await.ok();
            crate::brain_ops::inject_recall(
                brain,
                &agent,
                &user_query,
                recognition_ctx,
                recognition_pool,
            )
            .await
        } else {
            crate::brain_ops::RecallInjection::default()
        };

        // ── Phase 3c: Inject app catalog + current UI state ──
        {
            let catalog = &task_state.app_catalog;
            agent
                .extend_system_prompt("app_catalog".to_string(), catalog.to_prompt_block())
                .await;

            if let Some(ref ctx) = app_context {
                let tab_name = catalog
                    .find_by_name(&ctx.current_tab)
                    .or_else(|| catalog.tabs.iter().find(|e| e.tool_type == ctx.current_tab))
                    .map(|e| e.name.as_str())
                    .unwrap_or(&ctx.current_tab);
                let mut block = format!("Current UI state: User is on the {} tab.", tab_name);
                if let Some(ref panel) = ctx.active_panel {
                    if panel != "chat" {
                        block.push_str(&format!(" They have the {} overlay open.", panel));
                    }
                }
                if let Some(ref id) = ctx.selected_id {
                    block.push_str(&format!(" They have selected item {}.", id));
                }
                agent
                    .extend_system_prompt("app_context".to_string(), block)
                    .await;
            }

            // Decision Inbox deep-link (#303): if the user opened this chat to
            // discuss a specific decision, load it authoritatively (never trust
            // the frontend for the substance) and inject its full context so the
            // agent's opening turn already knows the goal, proposal, and reasoning
            // — no re-explaining. The id rides app_context.view_state. Recall over
            // the seed query (Phase 3 inject_recall above) covers Recognition.
            if let Some(decision_id) = app_context
                .as_ref()
                .and_then(|c| c.view_state.as_ref())
                .and_then(|v| v.get("discuss_decision_id"))
                .and_then(|v| v.as_str())
            {
                match task_state.session_manager().pool_clone().await {
                    Ok(pool) => {
                        match permagent::decisions::get_decision(&pool, decision_id).await {
                            Ok(Some(decision)) => {
                                // Load the goal's deterministic completion evidence
                                // (if any) so Henry reviews ground truth, not a
                                // stale local checkout.
                                let evidence_block = match decision.goal_id.as_deref() {
                                    Some(goal_id) => permagent::cards::get_card(&pool, goal_id)
                                        .await
                                        .ok()
                                        .flatten()
                                        .and_then(|c| {
                                            c.metadata_json.get("dispatch_evidence").cloned()
                                        })
                                        .as_ref()
                                        .and_then(permagent::agents::platform_extensions::orchestrator::format_dispatch_evidence_full),
                                    None => None,
                                };
                                agent
                                    .extend_system_prompt(
                                        "discuss_decision".to_string(),
                                        format_decision_discussion_block(
                                            &decision,
                                            evidence_block.as_deref(),
                                        ),
                                    )
                                    .await;
                            }
                            Ok(None) => tracing::warn!(
                                "discuss_decision: decision {} not found",
                                decision_id
                            ),
                            Err(e) => {
                                tracing::warn!("discuss_decision: load failed: {}", e)
                            }
                        }
                    }
                    Err(e) => tracing::warn!("discuss_decision: pool unavailable: {}", e),
                }
            }
        }

        let mut stream = match agent
            .reply(
                user_message.clone(),
                session_config,
                Some(task_cancel.clone()),
            )
            .await
        {
            Ok(stream) => stream,
            Err(e) => {
                tracing::error!("Failed to start reply stream: {:?}", e);
                publish(
                    Some(task_request_id.clone()),
                    MessageEvent::Error {
                        error: e.to_string(),
                    },
                )
                .await;
                // Terminal frame published — disarm (see above).
                _guard.disarm();
                task_bus.cleanup_request(&task_request_id).await;
                return;
            }
        };

        loop {
            tokio::select! {
                _ = task_cancel.cancelled() => {
                    tracing::info!("Agent task cancelled for request {}", task_request_id);
                    break;
                }
                response = timeout(Duration::from_millis(500), stream.next()) => {
                    match response {
                        Ok(Some(Ok(AgentEvent::Message(message)))) => {
                            for content in &message.content {
                                track_tool_telemetry(content, all_messages.messages());
                                // #84: track the in-flight tool name on the bus so
                                // /api/henry/status can surface "working: <tool>".
                                match content {
                                    permagent::conversation::message::MessageContent::ToolRequest(req) => {
                                        if let Ok(tool_call) = &req.tool_call {
                                            task_bus
                                                .set_current_tool(Some(tool_call.name.to_string()))
                                                .await;
                                        }
                                    }
                                    permagent::conversation::message::MessageContent::ToolResponse(_) => {
                                        task_bus.set_current_tool(None).await;
                                    }
                                    _ => {}
                                }
                            }
                            all_messages.push(message.clone());
                            let token_state = get_token_state(
                                task_state.session_manager(),
                                &task_session_id,
                            )
                            .await;
                            publish(
                                Some(task_request_id.clone()),
                                MessageEvent::Message {
                                    message,
                                    token_state,
                                },
                            )
                            .await;
                        }
                        Ok(Some(Ok(AgentEvent::HistoryReplaced(new_messages)))) => {
                            all_messages = new_messages.clone();
                            publish(
                                Some(task_request_id.clone()),
                                MessageEvent::UpdateConversation {
                                    conversation: new_messages,
                                },
                            )
                            .await;
                        }
                        Ok(Some(Ok(AgentEvent::McpNotification((notification_request_id, n))))) => {
                            publish(
                                Some(task_request_id.clone()),
                                MessageEvent::Notification {
                                    request_id: notification_request_id,
                                    message: n,
                                },
                            )
                            .await;
                        }
                        Ok(Some(Ok(AgentEvent::RuntimeOutcome(_)))) => {}
                        Ok(Some(Err(e))) => {
                            tracing::error!("Error processing message: {}", e);
                            publish(
                                Some(task_request_id.clone()),
                                MessageEvent::Error {
                                    error: e.to_string(),
                                },
                            )
                            .await;
                            break;
                        }
                        Ok(None) => {
                            break;
                        }
                        Err(_) => {
                            // Timeout — check if the bus still has subscribers
                            continue;
                        }
                    }
                }
            }
        }

        let traced_assistant_reply = all_messages
            .messages()
            .iter()
            .rev()
            .take_while(|message| message.role != rmcp::model::Role::User)
            .filter(|message| message.role == rmcp::model::Role::Assistant)
            .map(|message| message.as_concat_text())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        recall_trace.finish(traced_assistant_reply);

        // ── Phase 4: Remember turn after response completes ──
        if let Some(brain) = task_state.brain.as_ref() {
            let user_text = user_message.as_concat_text();
            let assistant_text = all_messages
                .messages()
                .iter()
                .rev()
                .find(|m| m.role == rmcp::model::Role::Assistant)
                .map(|m| m.as_concat_text())
                .unwrap_or_default();
            let turn_idx = all_messages.len();

            if !user_text.is_empty() && !assistant_text.is_empty() {
                // See reply.rs: tool-call arguments are corroboration evidence
                // that only exists at write time.
                let tool_text = crate::brain_ops::turn_tool_call_text(all_messages.messages());
                let pool = task_state.session_manager().pool_clone().await.ok();
                if let Err(error) = crate::brain_ops::persist_chat_turn(
                    brain.clone(),
                    pool,
                    task_session_id.clone(),
                    turn_idx,
                    user_message.created,
                    user_text,
                    assistant_text,
                    tool_text,
                )
                .await
                {
                    tracing::warn!(target: "permagentd::brain", "chat memory enqueue failed: {error}");
                }
            }
        }

        // Telemetry
        let session_duration = session_start.elapsed();

        if let Ok(session) = task_state
            .session_manager()
            .get_session(&task_session_id, true)
            .await
        {
            let total_tokens = session.total_tokens.unwrap_or(0);
            tracing::info!(
                monotonic_counter.goose.session_completions = 1,
                session_type = "app",
                interface = "ui",
                exit_type = "normal",
                duration_ms = session_duration.as_millis() as u64,
                total_tokens = total_tokens,
                message_count = session.message_count,
                "Session completed"
            );

            tracing::info!(
                monotonic_counter.goose.session_duration_ms = session_duration.as_millis() as u64,
                session_type = "app",
                interface = "ui",
                "Session duration"
            );

            if total_tokens > 0 {
                tracing::info!(
                    monotonic_counter.goose.session_tokens = total_tokens,
                    session_type = "app",
                    interface = "ui",
                    "Session tokens"
                );
            }
        } else {
            tracing::info!(
                monotonic_counter.goose.session_completions = 1,
                session_type = "app",
                interface = "ui",
                exit_type = "normal",
                duration_ms = session_duration.as_millis() as u64,
                total_tokens = 0u64,
                message_count = all_messages.len(),
                "Session completed"
            );

            tracing::info!(
                monotonic_counter.goose.session_duration_ms = session_duration.as_millis() as u64,
                session_type = "app",
                interface = "ui",
                "Session duration"
            );
        }

        let final_token_state =
            get_token_state(task_state.session_manager(), &task_session_id).await;

        // Activity: chat turn completed
        permagent::events::activity::emit_activity(
            permagent::events::activity::chat_turn_completed(
                &task_session_id,
                session_start.elapsed().as_millis() as u64,
                final_token_state.input_tokens,
                final_token_state.output_tokens,
            ),
        );

        publish(
            Some(task_request_id.clone()),
            MessageEvent::Finish {
                reason: "stop".to_string(),
                token_state: final_token_state,
            },
        )
        .await;

        _guard.disarm();
        task_bus.cleanup_request(&task_request_id).await;
    }));

    Ok(Json(SessionReplyResponse { request_id }))
}

// ── POST /sessions/{id}/cancel ──────────────────────────────────────────

#[utoipa::path(
    post,
    path = "/sessions/{id}/cancel",
    params(
        ("id" = String, Path, description = "Session ID"),
    ),
    request_body = CancelRequest,
    responses(
        (status = 200, description = "Honest cancel result", body = CancelResponse),
    )
)]
pub async fn session_cancel(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(request): Json<CancelRequest>,
) -> Json<CancelResponse> {
    // Always 200 with an honest body. "No bus for this session" and "bus has
    // no such request" both mean the same thing to the caller — nothing is
    // running, nothing was cancelled, and no terminal frame is coming — so
    // they share `cancelled: false` rather than an ambiguous 404 (which the
    // client cannot distinguish from a wrong URL, and which previously hid
    // behind an unconditional 200 that lied about cancelling nothing).
    let cancelled = match state.get_event_bus(&session_id).await {
        Some(bus) => bus.cancel_request(&request.request_id).await,
        None => false,
    };
    Json(CancelResponse { cancelled })
}

// ── Route registration ──────────────────────────────────────────────────

pub fn event_routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/sessions/{id}/events", get(session_events))
        .with_state(state)
}

pub fn control_routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/sessions/{id}/reply",
            post(session_reply).layer(DefaultBodyLimit::max(50 * 1024 * 1024)),
        )
        .route("/sessions/{id}/cancel", post(session_cancel))
        .with_state(state)
}
