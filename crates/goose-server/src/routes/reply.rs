use crate::routes::errors::ErrorResponse;
use crate::state::AppState;
#[cfg(test)]
use axum::http::StatusCode;
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{self},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use bytes::Bytes;
use futures::{stream::StreamExt, Stream};
use permagent::agents::{AgentEvent, SessionConfig};
use permagent::conversation::message::{Message, MessageContent, TokenState};
use permagent::conversation::Conversation;
use permagent::session::SessionManager;
use rmcp::model::ServerNotification;
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
use tokio_util::sync::CancellationToken;

pub fn track_tool_telemetry(content: &MessageContent, all_messages: &[Message]) {
    match content {
        MessageContent::ToolRequest(tool_request) => {
            if let Ok(tool_call) = &tool_request.tool_call {
                tracing::info!(monotonic_counter.goose.tool_calls = 1,
                    tool_name = %tool_call.name,
                    "Tool call started"
                );
            }
        }
        MessageContent::ToolResponse(tool_response) => {
            let tool_name = all_messages
                .iter()
                .rev()
                .find_map(|msg| {
                    msg.content.iter().find_map(|c| {
                        if let MessageContent::ToolRequest(req) = c {
                            if req.id == tool_response.id {
                                if let Ok(tool_call) = &req.tool_call {
                                    Some(tool_call.name.clone())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or_else(|| "unknown".to_string().into());

            let success = tool_response.tool_result.is_ok();
            let result_status = if success { "success" } else { "error" };

            tracing::info!(
                monotonic_counter.goose.tool_completions = 1,
                tool_name = %tool_name,
                result = %result_status,
                "Tool call completed"
            );
        }
        _ => {}
    }
}

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct ChatRequest {
    user_message: Message,
    /// Override the server's conversation history. Only use this when you need absolute control
    /// over the conversation state (e.g., administrative tools). For normal operations, the server
    /// is the source of truth - use truncate/fork endpoints to modify conversation history instead.
    #[serde(default)]
    override_conversation: Option<Vec<Message>>,
    session_id: String,
    recipe_name: Option<String>,
    recipe_version: Option<String>,
}

pub struct SseResponse {
    rx: ReceiverStream<String>,
}

impl SseResponse {
    fn new(rx: ReceiverStream<String>) -> Self {
        Self { rx }
    }
}

impl Stream for SseResponse {
    type Item = Result<Bytes, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.rx)
            .poll_next(cx)
            .map(|opt| opt.map(|s| Ok(Bytes::from(s))))
    }
}

impl IntoResponse for SseResponse {
    fn into_response(self) -> axum::response::Response {
        let stream = self;
        let body = axum::body::Body::from_stream(stream);

        http::Response::builder()
            .header("Content-Type", "text/event-stream")
            .header("Cache-Control", "no-cache")
            .header("Connection", "keep-alive")
            .body(body)
            .unwrap()
    }
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
#[serde(tag = "type")]
pub enum MessageEvent {
    Message {
        message: Message,
        token_state: TokenState,
    },
    Error {
        error: String,
    },
    Finish {
        reason: String,
        token_state: TokenState,
    },
    Notification {
        request_id: String,
        #[schema(value_type = Object)]
        message: ServerNotification,
    },
    UpdateConversation {
        conversation: Conversation,
    },
    /// Sent at the start of an SSE stream to inform the client about
    /// in-flight requests it can reattach to.
    ActiveRequests {
        request_ids: Vec<String>,
    },
    /// Sent before the model generates a response, carrying references
    /// to the probed/recalled memories that fed the system prompt.
    ContextAttached {
        probed_memories: Vec<ProbedMemoryRef>,
        recalled_memories: Vec<RecalledMemoryRef>,
    },
    Ping,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ProbedMemoryRef {
    pub id: String,
    pub key: String,
    pub content_summary: String,
    pub relevance: f64,
    pub wing: Option<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct RecalledMemoryRef {
    pub id: String,
    pub signal_score: f64,
    pub content_summary: String,
}

pub async fn get_token_state(session_manager: &SessionManager, session_id: &str) -> TokenState {
    session_manager
        .get_session(session_id, false)
        .await
        .map(|session| TokenState {
            input_tokens: session.input_tokens.unwrap_or(0),
            output_tokens: session.output_tokens.unwrap_or(0),
            total_tokens: session.total_tokens.unwrap_or(0),
            accumulated_input_tokens: session.accumulated_input_tokens.unwrap_or(0),
            accumulated_output_tokens: session.accumulated_output_tokens.unwrap_or(0),
            accumulated_total_tokens: session.accumulated_total_tokens.unwrap_or(0),
        })
        .inspect_err(|e| {
            tracing::warn!(
                "Failed to fetch session token state for {}: {}",
                session_id,
                e
            );
        })
        .unwrap_or_default()
}

async fn stream_event(
    event: MessageEvent,
    tx: &mpsc::Sender<String>,
    cancel_token: &CancellationToken,
) {
    let json = serde_json::to_string(&event).unwrap_or_else(|e| {
        format!(
            r#"{{"type":"Error","error":"Failed to serialize event: {}"}}"#,
            e
        )
    });

    if tx.send(format!("data: {}\n\n", json)).await.is_err() {
        tracing::info!("client hung up");
        cancel_token.cancel();
    }
}

#[allow(clippy::too_many_lines)]
#[utoipa::path(
    post,
    path = "/reply",
    request_body = ChatRequest,
    responses(
        (status = 200, description = "Streaming response initiated",
         body = MessageEvent,
         content_type = "text/event-stream"),
        (status = 424, description = "Agent not initialized"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn reply(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ChatRequest>,
) -> Result<SseResponse, ErrorResponse> {
    let session_start = std::time::Instant::now();

    tracing::info!(
        monotonic_counter.goose.session_starts = 1,
        session_type = "app",
        interface = "ui",
        "Session started"
    );

    let session_id = request.session_id.clone();

    // Activity: chat turn started
    permagent::events::activity::emit_activity(
        permagent::events::activity::chat_turn_started(&session_id),
    );

    if let Some(recipe_name) = request.recipe_name.clone() {
        if state.mark_recipe_run_if_absent(&session_id).await {
            let recipe_version = request
                .recipe_version
                .clone()
                .unwrap_or_else(|| "unknown".to_string());

            tracing::info!(
                monotonic_counter.goose.recipe_runs = 1,
                recipe_name = %recipe_name,
                recipe_version = %recipe_version,
                session_type = "app",
                interface = "ui",
                "Recipe execution started"
            );
        }
    }

    let (tx, rx) = mpsc::channel(100);
    let stream = ReceiverStream::new(rx);
    let cancel_token = CancellationToken::new();

    let user_message = request.user_message;
    let override_conversation = request.override_conversation;

    let task_cancel = cancel_token.clone();
    let task_tx = tx.clone();

    drop(tokio::spawn(async move {
        let agent = match state.get_agent(session_id.clone()).await {
            Ok(agent) => agent,
            Err(e) => {
                tracing::error!("Failed to get session agent: {}", e);
                let _ = stream_event(
                    MessageEvent::Error {
                        error: format!("Failed to get session agent: {}", e),
                    },
                    &task_tx,
                    &task_cancel,
                )
                .await;
                return;
            }
        };

        let session = match state.session_manager().get_session(&session_id, true).await {
            Ok(metadata) => metadata,
            Err(e) => {
                tracing::error!("Failed to read session for {}: {}", session_id, e);
                let _ = stream_event(
                    MessageEvent::Error {
                        error: format!("Failed to read session: {}", e),
                    },
                    &task_tx,
                    &cancel_token,
                )
                .await;
                return;
            }
        };

        let session_config = SessionConfig {
            id: session_id.clone(),
            schedule_id: session.schedule_id.clone(),
            max_turns: None,
            retry_config: None,
        };

        let mut all_messages = match override_conversation {
            Some(history) => {
                let conv = Conversation::new_unvalidated(history);
                if let Err(e) = state
                    .session_manager()
                    .replace_conversation(&session_id, &conv)
                    .await
                {
                    tracing::warn!(
                        "Failed to replace session conversation for {}: {}",
                        session_id,
                        e
                    );
                }
                conv
            }
            None => session.conversation.unwrap_or_default(),
        };
        all_messages.push(user_message.clone());

        // ── Phase 3b: Ambient context from ContextBuilder ──
        if let Some(ref context_builder) = state.context_builder {
            let user_text = user_message.as_concat_text();
            let focus_wing = state
                .activity_ingester
                .as_ref()
                .and_then(|ing| ing.active_project())
                .map(|ap| ap.wing.clone());

            let recall_query = if user_text.len() > 20 {
                Some(user_text.clone())
            } else {
                None
            };

            let digest_opts = permagent::activity::context_builder::DigestOpts {
                include_probe: true,
                focus_wing,
                include_recall_query: recall_query,
                ..Default::default()
            };

            let cb = context_builder.clone();
            let digest_result = tokio::task::spawn_blocking(move || cb.current_digest(digest_opts))
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
                            .extend_system_prompt(
                                "ambient_context".to_string(),
                                ambient_block,
                            )
                            .await;
                    }

                    // Emit ContextAttached so the frontend can show citation markers
                    if !digest.probed_memories.is_empty() || !digest.recalled_memories.is_empty() {
                        let probed: Vec<ProbedMemoryRef> = digest.probed_memories.iter().map(|m| {
                            ProbedMemoryRef {
                                id: m.id.clone(),
                                key: m.key.clone(),
                                content_summary: m.content.chars().take(200).collect(),
                                relevance: m.relevance,
                                wing: m.wing.clone(),
                            }
                        }).collect();
                        let recalled: Vec<RecalledMemoryRef> = digest.recalled_memories.iter().map(|m| {
                            RecalledMemoryRef {
                                id: m.source.clone().unwrap_or_default(),
                                signal_score: m.signal_score,
                                content_summary: m.content.chars().take(200).collect(),
                            }
                        }).collect();
                        stream_event(
                            MessageEvent::ContextAttached { probed_memories: probed, recalled_memories: recalled },
                            &task_tx, &task_cancel,
                        ).await;
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        target: "permagentd::activity",
                        "ContextBuilder digest failed, proceeding without ambient context: {}",
                        e
                    );
                }
            }
        }

        // ── Phase 3: Recall from brain before model invocation ──
        const RECALL_SCORE_FLOOR: f64 = 0.7;
        const RECALL_TOP_K: usize = 3;

        if let Some(brain) = state.brain.as_ref() {
            let user_query = user_message.as_concat_text();
            if !user_query.is_empty() {
                let brain = brain.clone();
                let query = user_query.clone();
                let recall_result = tokio::task::spawn_blocking(move || {
                    brain.recall(&query, spectral::Visibility::Private)
                })
                .await;

                match recall_result {
                    Ok(Ok(result)) => {
                        let top_hits: Vec<_> = result
                            .memory_hits
                            .iter()
                            .filter(|hit| hit.signal_score >= RECALL_SCORE_FLOOR)
                            .take(RECALL_TOP_K)
                            .collect();

                        if !top_hits.is_empty() {
                            let mut prefix =
                                String::from("Relevant memories from past context:\n");
                            for hit in &top_hits {
                                prefix.push_str(&format!("- {}\n", hit.content));
                            }

                            tracing::info!(
                                target: "permagentd::brain",
                                "Recall injected {} memories into system prompt for query: {:?}",
                                top_hits.len(),
                                user_query.chars().take(80).collect::<String>()
                            );

                            agent
                                .extend_system_prompt(
                                    "memory_recall".to_string(),
                                    prefix,
                                )
                                .await;
                        } else {
                            tracing::debug!(
                                target: "permagentd::brain",
                                "Recall returned no hits above {} threshold for query: {:?}",
                                RECALL_SCORE_FLOOR,
                                user_query.chars().take(80).collect::<String>()
                            );
                        }
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(
                            target: "permagentd::brain",
                            "Brain recall failed: {}",
                            e
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "permagentd::brain",
                            "Brain recall spawn_blocking panicked: {}",
                            e
                        );
                    }
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
                stream_event(
                    MessageEvent::Error {
                        error: e.to_string(),
                    },
                    &task_tx,
                    &cancel_token,
                )
                .await;
                return;
            }
        };

        let mut heartbeat_interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            tokio::select! {
                _ = task_cancel.cancelled() => {
                    tracing::info!("Agent task cancelled");
                    break;
                }
                _ = heartbeat_interval.tick() => {
                    stream_event(MessageEvent::Ping, &tx, &cancel_token).await;
                }
                response = timeout(Duration::from_millis(500), stream.next()) => {
                    match response {
                        Ok(Some(Ok(AgentEvent::Message(message)))) => {
                            for content in &message.content {
                                track_tool_telemetry(content, all_messages.messages());
                            }

                            all_messages.push(message.clone());

                            let token_state = get_token_state(state.session_manager(), &session_id).await;

                            stream_event(MessageEvent::Message { message, token_state }, &tx, &cancel_token).await;
                        }
                        Ok(Some(Ok(AgentEvent::HistoryReplaced(new_messages)))) => {
                            all_messages = new_messages.clone();
                            stream_event(MessageEvent::UpdateConversation {conversation: new_messages}, &tx, &cancel_token).await;

                        }
                        Ok(Some(Ok(AgentEvent::McpNotification((request_id, n))))) => {
                            stream_event(MessageEvent::Notification{
                                request_id: request_id.clone(),
                                message: n,
                            }, &tx, &cancel_token).await;
                        }

                        Ok(Some(Err(e))) => {
                            tracing::error!("Error processing message: {}", e);
                            stream_event(
                                MessageEvent::Error {
                                    error: e.to_string(),
                                },
                                &tx,
                                &cancel_token,
                            ).await;
                            break;
                        }
                        Ok(None) => {
                            break;
                        }
                        Err(_) => {
                            if tx.is_closed() {
                                break;
                            }
                            continue;
                        }
                    }
                }
            }
        }

        // ── Phase 4: Remember turn after response completes ──
        if let Some(brain) = state.brain.as_ref() {
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
                let brain = brain.clone();
                let remember_session_id = session_id.clone();

                tokio::spawn(async move {
                    let key = format!("chat-{}-{}", remember_session_id, turn_idx);
                    let content =
                        format!("User: {}\nAssistant: {}", user_text, assistant_text);
                    let device_id = brain.device_id().clone();
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
        }

        let session_duration = session_start.elapsed();

        if let Ok(session) = state.session_manager().get_session(&session_id, true).await {
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

        let final_token_state = get_token_state(state.session_manager(), &session_id).await;

        // Activity: chat turn completed
        permagent::events::activity::emit_activity(
            permagent::events::activity::chat_turn_completed(
                &session_id,
                session_start.elapsed().as_millis() as u64,
                final_token_state.input_tokens,
                final_token_state.output_tokens,
            ),
        );

        let _ = stream_event(
            MessageEvent::Finish {
                reason: "stop".to_string(),
                token_state: final_token_state,
            },
            &task_tx,
            &cancel_token,
        )
        .await;
    }));
    Ok(SseResponse::new(stream))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/reply",
            post(reply).layer(DefaultBodyLimit::max(50 * 1024 * 1024)),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    mod integration_tests {
        use super::*;
        use axum::{body::Body, http::Request};
        use permagent::conversation::message::Message;
        use tower::ServiceExt;

        #[tokio::test(flavor = "multi_thread")]
        async fn test_reply_endpoint() {
            let state = AppState::new(true).await.unwrap();

            let app = routes(state);

            let request = Request::builder()
                .uri("/reply")
                .method("POST")
                .header("content-type", "application/json")
                .header("x-secret-key", "test-secret")
                .body(Body::from(
                    serde_json::to_string(&ChatRequest {
                        user_message: Message::user().with_text("test message"),
                        override_conversation: None,
                        session_id: "test-session".to_string(),
                        recipe_name: None,
                        recipe_version: None,
                    })
                    .unwrap(),
                ))
                .unwrap();

            let response = app.oneshot(request).await.unwrap();

            assert_eq!(response.status(), StatusCode::OK);
        }
    }
}
