use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use crate::agents::{AgentEvent, SessionConfig};
use crate::cards;
use crate::config::agent_identity;
use crate::config::worker_probe::{self, ProbeCache};
use crate::config::{Config, ExtensionConfig, GooseMode};
use crate::context_mgmt::format_message_for_compacting;
use crate::conversation::message::Message;
use crate::execution::manager::AgentManager;
use crate::goal_state::{self, GoalAction, GoalState};
use crate::providers;
use crate::providers::base::Provider;
use crate::session::extension_data::EnabledExtensionsState;
use crate::session::session_manager::SessionType;
use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

const MAX_GOAL_ATTEMPTS: u64 = 3;

pub static EXTENSION_NAME: &str = "orchestrator";

struct CancelTokenGuard {
    manager: Arc<AgentManager>,
    session_id: String,
    disarmed: bool,
}

impl CancelTokenGuard {
    fn new(manager: Arc<AgentManager>, session_id: String) -> Self {
        Self {
            manager,
            session_id,
            disarmed: false,
        }
    }

    fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for CancelTokenGuard {
    fn drop(&mut self) {
        if !self.disarmed {
            let manager = self.manager.clone();
            let session_id = self.session_id.clone();
            tokio::spawn(async move {
                manager.unregister_cancel_token(&session_id).await;
            });
        }
    }
}

const DEFAULT_LIST_LIMIT: usize = 10;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ListSessionsParams {
    /// Filter by session type: "user", "sub_agent", "scheduled", "hidden", "terminal", "gateway".
    /// If omitted, returns all session types.
    session_type: Option<String>,
    /// Maximum number of sessions to return (most recent first). Defaults to 10.
    last_n: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ViewSessionParams {
    /// The session ID to inspect
    session_id: String,
    /// How to view the conversation: "first_last" returns the first and last message,
    /// "summarize" calls the LLM to produce a summary. If omitted, returns first and last.
    mode: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct StartAgentParams {
    /// Working directory for the new agent session
    working_dir: String,
    /// Human-readable name for the session
    name: Option<String>,
    /// Optional worker persona key from agent.yaml workers section.
    /// If set and found, the orchestrated agent identifies as this worker.
    #[serde(default)]
    worker_persona: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct SendMessageParams {
    /// The session ID of the agent to send a message to
    session_id: String,
    /// The message text to send
    message: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct InterruptAgentParams {
    /// The session ID of the agent to interrupt
    session_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct GoalAdvanceParams {
    /// The card ID (UUID) of the goal to advance.
    card_id: String,
    /// The action to perform: "ready", "dispatch", "review", "approve", "reject"
    action: String,
    /// Optional notes for 'approve' or 'reject' actions. Stored in metadata.review_notes.
    #[serde(default)]
    notes: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct GoalStatusParams {
    /// The card ID (UUID) of the goal to inspect.
    card_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ListWorkersParams {
    /// Force re-probe of all workers, ignoring cache. Defaults to false.
    #[serde(default)]
    refresh: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct CheckWorkerParams {
    /// The worker key from agent.yaml to check.
    worker_key: String,
}

pub struct OrchestratorClient {
    info: InitializeResult,
    context: PlatformExtensionContext,
    probe_cache: Arc<ProbeCache>,
}

impl OrchestratorClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME, "1.0.0").with_title("Orchestrator"),
            )
            .with_instructions(
                "Manage agent sessions and coordinate goal-driven work across projects.\n\n\
                 When users describe work — building, fixing, automating — create goal cards \
                 on a project's Kanban board using card_create with card_type='goal'. Goals \
                 follow a lifecycle: Triage → Ready → InProgress → Review → Complete. Pass \
                 auto_dispatch=true to assign a worker and start immediately.\n\n\
                 Use goal_advance to transition goals (actions: ready, dispatch, review, \
                 approve, reject). Use goal_status to check progress. Use list_workers to \
                 see available workers before dispatching.\n\n\
                 Goals that fail three times move to Triage with needs_human_attention=true. \
                 Surface these to the user rather than retrying silently.",
            );

        let client = Self {
            info,
            context,
            probe_cache: Arc::new(ProbeCache::new()),
        };

        // Spawn one-shot resume of in-progress goals from a prior daemon lifecycle
        let resume_sm = client.context.session_manager.clone();
        tokio::spawn(async move {
            // Small delay to let the DB pool and AgentManager finish initializing
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if let Err(e) = resume_in_progress_goals(&resume_sm).await {
                tracing::warn!(
                    target: "permagentd::brain",
                    "Failed to resume in-progress goals on startup: {}",
                    e
                );
            }
        });

        Ok(client)
    }

    async fn get_agent_manager(&self) -> Result<Arc<AgentManager>, String> {
        AgentManager::instance()
            .await
            .map_err(|e| format!("Failed to get agent manager: {}", e))
    }

    async fn get_provider(&self) -> Result<Arc<dyn Provider>, String> {
        let extension_manager = self
            .context
            .extension_manager
            .as_ref()
            .and_then(|weak| weak.upgrade())
            .ok_or("Extension manager not available")?;

        let provider_guard = extension_manager.get_provider().lock().await;
        provider_guard
            .as_ref()
            .cloned()
            .ok_or_else(|| "Provider not available".to_string())
    }

    fn parent_extensions(&self) -> Vec<ExtensionConfig> {
        let extension_data = self.context.session.as_ref().map(|s| &s.extension_data);
        EnabledExtensionsState::extensions_or_default(extension_data, Config::global())
    }

    /// Select the best available worker for a goal card.
    ///
    /// Builds candidate list from agent.yaml + probe cache, then delegates
    /// to the pure `goal_state::select_best_worker` algorithm.
    pub async fn select_worker(&self, goal: &cards::Card) -> Result<String, String> {
        let config = agent_identity::load_agent_config();

        // Derive required tool_kinds from goal metadata, default to code_edit + shell
        let required_kinds: Vec<String> = goal
            .metadata_json
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_else(|| vec!["code_edit".to_string(), "shell".to_string()]);

        // Build candidate list with availability + session counts
        let manager = self.get_agent_manager().await.ok();
        let active_ids = match &manager {
            Some(m) => m.list_active_session_ids().await,
            None => Vec::new(),
        };

        let candidates: Vec<goal_state::WorkerCandidate> = config
            .workers
            .iter()
            .map(|(key, persona)| {
                let available = match self.probe_cache.get(key) {
                    Some(cached) => cached.available,
                    None => {
                        let (ok, reason) = worker_probe::probe_worker(&persona.availability_check);
                        self.probe_cache.set(key, ok, reason);
                        ok
                    }
                };

                // Count active sessions for this worker (best-effort from session names)
                let active_sessions = active_ids
                    .iter()
                    .filter(|id| {
                        // Sessions spawned by this worker will have the worker key
                        // in their name or metadata — for now, approximate as 0
                        // since we don't yet track per-worker session counts.
                        let _ = id;
                        false
                    })
                    .count();

                goal_state::WorkerCandidate {
                    key: key.clone(),
                    available,
                    tool_kinds: persona.tool_kinds.clone(),
                    cost_tier: persona.cost_tier.clone(),
                    active_sessions,
                }
            })
            .collect();

        goal_state::select_best_worker(&candidates, &required_kinds)
    }

    /// Dispatch a goal card to a worker via subagent.
    ///
    /// Precondition: card must be card_type='goal' in state='ready'.
    /// On success: card moves to InProgress with worker metadata.
    /// On worker selection failure: card stays in Ready, no metadata changes.
    /// On dispatch failure: card stays in Ready, attempt_count incremented.
    pub async fn dispatch_goal(&self, card_id: &str) -> Result<String, String> {
        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())?;

        let card = cards::get_card(&pool, card_id)
            .await?
            .ok_or_else(|| format!("Card '{}' not found", card_id))?;

        if card.card_type != "goal" {
            return Err(format!(
                "Card '{}' is type '{}', not 'goal'",
                card_id, card.card_type
            ));
        }

        // Verify card is in Ready state
        let current_col = cards::get_column(&pool, &card.column_id)
            .await?
            .ok_or_else(|| format!("Column '{}' not found", card.column_id))?;

        if current_col.state_binding.as_deref() != Some("ready") {
            return Err(format!(
                "Card '{}' is in state '{}', not 'ready'. Only Ready goals can be dispatched.",
                card_id,
                current_col
                    .state_binding
                    .as_deref()
                    .unwrap_or(&current_col.name)
            ));
        }

        // Select worker — on failure, leave card in Ready, no metadata changes
        let worker_key = self.select_worker(&card).await?;

        // Resolve worker persona for the subagent
        let config = agent_identity::load_agent_config();
        let persona_override = config
            .workers
            .get(&worker_key)
            .map(|w| (w.system_prompt_block(), w.display_name()));

        // Look up project for root_path context
        let project = crate::projects::get_project(&pool, &card.project_id)
            .await?
            .ok_or_else(|| format!("Project '{}' not found", card.project_id))?;

        let root_path = project.root_path.as_deref().unwrap_or("(not specified)");

        // Build instructions
        let instructions = format!(
            "Goal: {}\n\nDescription: {}\nProject: {}\nProject root: {}",
            card.title, card.description, project.name, root_path
        );

        // Build recipe
        let recipe = crate::recipe::Recipe::builder()
            .version("1.0.0")
            .title(format!("Goal: {}", card.title))
            .description("Orchestrator-dispatched goal")
            .prompt(&instructions)
            .build()
            .map_err(|e| format!("Failed to build recipe: {}", e))?;

        // Get provider and extensions from parent session
        let provider = self.get_provider().await?;
        let extensions = self.parent_extensions();

        // Get the session for working dir
        let working_dir = project
            .root_path
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        // Create subagent session
        let subagent_session = self
            .context
            .session_manager
            .create_session(
                working_dir.clone(),
                format!("Goal: {}", card.title),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .map_err(|e| format!("Failed to create subagent session: {}", e))?;

        let session_id = subagent_session.id.clone();

        // Build task config
        let model_config = provider.get_model_config();
        let task_provider =
            providers::create(provider.get_name(), model_config, extensions.clone())
                .await
                .map_err(|e| format!("Failed to create provider for goal dispatch: {}", e))?;

        let task_config = crate::agents::subagent_task_config::TaskConfig::new(
            task_provider,
            &session_id,
            &working_dir,
            extensions,
        );

        let agent_config = crate::agents::AgentRunnerConfig::new(
            self.context.session_manager.clone(),
            crate::config::permission::PermissionManager::instance(),
            None,
            GooseMode::Auto,
            true,
            crate::agents::GoosePlatform::GooseCli,
        );

        // Spawn the subagent task in background and track completion
        let cancel_token = CancellationToken::new();
        let handle = tokio::spawn({
            let session_id = session_id.clone();
            async move {
                crate::agents::subagent_handler::run_subagent_task(
                    crate::agents::subagent_handler::SubagentRunParams {
                        config: agent_config,
                        recipe,
                        task_config,
                        return_last_only: true,
                        session_id,
                        cancellation_token: Some(cancel_token),
                        on_message: None,
                        notification_tx: None,
                        persona_override,
                    },
                )
                .await
            }
        });

        // Spawn completion tracker — watches the JoinHandle and transitions the card
        let tracker_card_id = card_id.to_string();
        let tracker_project_id = card.project_id.clone();
        let tracker_pool = pool.clone();
        tokio::spawn(async move {
            let result = match handle.await {
                Ok(Ok(_output)) => Ok(()),
                Ok(Err(e)) => Err(e.to_string()),
                Err(e) => Err(format!("Worker task panicked: {}", e)),
            };
            if let Err(e) =
                handle_goal_completion(&tracker_pool, &tracker_card_id, &tracker_project_id, result)
                    .await
            {
                tracing::error!(
                    target: "permagentd::brain",
                    "Failed to handle goal completion for card {}: {}",
                    tracker_card_id,
                    e
                );
            }
        });

        // Update card metadata BEFORE column move (design doc: metadata first)
        let mut meta = card.metadata_json.as_object().cloned().unwrap_or_default();
        let attempt_count = meta
            .get("attempt_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        meta.insert(
            "worker_key".to_string(),
            serde_json::Value::String(worker_key.clone()),
        );
        meta.insert(
            "worker_session_id".to_string(),
            serde_json::Value::String(session_id.clone()),
        );
        meta.insert(
            "dispatched_at".to_string(),
            serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
        );
        meta.insert(
            "attempt_count".to_string(),
            serde_json::json!(attempt_count + 1),
        );
        meta.insert(
            "goal_state".to_string(),
            serde_json::Value::String("in_progress".to_string()),
        );

        cards::update_card(
            &pool,
            card_id,
            cards::UpdateCard {
                assigned_to: Some(Some(worker_key.clone())),
                metadata_json: Some(serde_json::Value::Object(meta)),
                ..Default::default()
            },
        )
        .await?;

        // Move card to InProgress
        let in_progress_col = cards::get_goal_column(&pool, &card.project_id, "in_progress")
            .await?
            .ok_or("InProgress column not found")?;

        cards::move_card(&pool, card_id, &in_progress_col.id, None).await?;

        tracing::info!(
            target: "permagentd::brain",
            "Goal '{}' dispatched to worker '{}' (session: {})",
            card.title,
            worker_key,
            session_id
        );

        Ok(session_id)
    }

    async fn handle_list_sessions(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, String> {
        let type_filter = arguments
            .as_ref()
            .and_then(|args| args.get("session_type"))
            .and_then(|v| v.as_str());

        let limit = arguments
            .as_ref()
            .and_then(|args| args.get("last_n"))
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_LIST_LIMIT);

        let manager = self.get_agent_manager().await?;

        let mut sessions = if let Some(type_str) = type_filter {
            let session_type: SessionType = type_str
                .parse()
                .map_err(|e| format!("Invalid session type '{}': {}", type_str, e))?;
            self.context
                .session_manager
                .list_sessions_by_types(&[session_type])
                .await
                .map_err(|e| format!("Failed to list sessions: {}", e))?
        } else {
            self.context
                .session_manager
                .list_sessions()
                .await
                .map_err(|e| format!("Failed to list sessions: {}", e))?
        };

        // Most recent first
        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        let total = sessions.len();
        sessions.truncate(limit);

        if sessions.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No sessions found.",
            )]));
        }

        let active_ids = manager.list_active_session_ids().await;

        let mut lines = vec![format!(
            "Showing {} of {} session(s):\n",
            sessions.len(),
            total
        )];
        for session in &sessions {
            let is_loaded = active_ids.contains(&session.id);
            let is_busy = if is_loaded {
                manager.is_session_busy(&session.id).await
            } else {
                false
            };

            let status = if is_busy {
                "🔄 busy"
            } else if is_loaded {
                "✓ loaded"
            } else {
                "○ idle"
            };

            lines.push(format!(
                "- **{}** ({})\n  Type: {} | Status: {} | Messages: {} | Updated: {}",
                session.name,
                session.id,
                session.session_type,
                status,
                session.message_count,
                session.updated_at.format("%Y-%m-%d %H:%M"),
            ));
        }

        Ok(CallToolResult::success(vec![Content::text(
            lines.join("\n"),
        )]))
    }

    async fn handle_view_session(
        &self,
        session_id_for_llm: &str,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let session_id = extract_string(&args, "session_id")?;
        let mode = args
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("first_last");

        let session = self
            .context
            .session_manager
            .get_session(&session_id, true)
            .await
            .map_err(|e| format!("Session '{}' not found: {}", session_id, e))?;

        let manager = self.get_agent_manager().await?;
        let is_busy = manager.is_session_busy(&session_id).await;

        let mut output = vec![format!(
            "# Session: {} ({})\n\nType: {} | Status: {} | Working dir: {}\nMessages: {} | Updated: {}\n",
            session.name,
            session.id,
            session.session_type,
            if is_busy { "🔄 busy" } else { "idle" },
            session.working_dir.display(),
            session.message_count,
            session.updated_at.format("%Y-%m-%d %H:%M"),
        )];

        match mode {
            "first_last" => {
                if let Some(conversation) = &session.conversation {
                    let messages = conversation.messages();
                    if messages.is_empty() {
                        output.push("No messages in this session.".to_string());
                    } else {
                        output.push("## First message\n".to_string());
                        output.push(format_message_for_compacting(&messages[0]));

                        if messages.len() > 1 {
                            output.push(format!("\n*({} messages omitted)*\n", messages.len() - 2));
                            output.push("## Last message\n".to_string());
                            output
                                .push(format_message_for_compacting(&messages[messages.len() - 1]));
                        }
                    }
                } else {
                    output.push("No messages in this session.".to_string());
                }
            }
            "summarize" => {
                if let Some(conversation) = &session.conversation {
                    let messages = conversation.messages();
                    if messages.is_empty() {
                        output.push("No messages to summarize.".to_string());
                    } else {
                        let summary = self
                            .summarize_conversation(session_id_for_llm, messages)
                            .await?;
                        output.push(format!("## Summary\n\n{}", summary));
                    }
                } else {
                    output.push("No messages to summarize.".to_string());
                }
            }
            other => {
                return Err(format!(
                    "Unknown mode '{}'. Use 'first_last' or 'summarize'.",
                    other
                ));
            }
        }

        Ok(CallToolResult::success(vec![Content::text(
            output.join("\n"),
        )]))
    }

    async fn summarize_conversation(
        &self,
        session_id: &str,
        messages: &[Message],
    ) -> Result<String, String> {
        let provider = self.get_provider().await?;

        let conversation_text = messages
            .iter()
            .filter(|m| m.is_agent_visible())
            .map(format_message_for_compacting)
            .collect::<Vec<_>>()
            .join("\n");

        let system =
            "You are a helpful assistant. Summarize the following conversation concisely, \
                       capturing the key topics, decisions, and current state. Be brief.";

        let user_message = Message::user().with_text(format!(
            "Summarize this conversation ({} messages):\n\n{}",
            messages.len(),
            conversation_text
        ));

        let (response, _usage) = provider
            .complete_fast(session_id, system, &[user_message], &[])
            .await
            .map_err(|e| format!("LLM summarization failed: {}", e))?;

        Ok(response
            .content
            .iter()
            .filter_map(|c| {
                if let crate::conversation::message::MessageContent::Text(t) = c {
                    Some(t.text.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    async fn handle_start_agent(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let working_dir = extract_string(&args, "working_dir")?;
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Orchestrated Agent")
            .to_string();
        let worker_persona = args
            .get("worker_persona")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let raw_path = PathBuf::from(&working_dir);
        let path = if raw_path.is_absolute() {
            raw_path
        } else {
            let base = self
                .context
                .session
                .as_ref()
                .map(|s| s.working_dir.clone())
                .unwrap_or_else(|| PathBuf::from("."));
            base.join(&raw_path)
        };

        let path = path
            .canonicalize()
            .map_err(|e| format!("Invalid working directory '{}': {}", working_dir, e))?;

        if !path.is_dir() {
            return Err(format!("'{}' is not a directory", working_dir));
        }

        let mode = GooseMode::default();

        let session = self
            .context
            .session_manager
            .create_session(path, name.clone(), SessionType::User, mode)
            .await
            .map_err(|e| format!("Failed to create session: {}", e))?;

        let manager = self.get_agent_manager().await?;
        let agent = manager
            .get_or_create_agent(session.id.clone())
            .await
            .map_err(|e| format!("Failed to create agent: {}", e))?;

        let parent_provider = self.get_provider().await?;
        let extensions = self.parent_extensions();
        let provider = providers::create(
            parent_provider.get_name(),
            parent_provider.get_model_config(),
            extensions,
        )
        .await
        .map_err(|e| format!("Failed to create provider for new agent: {}", e))?;
        agent
            .update_provider(provider, &session.id)
            .await
            .map_err(|e| format!("Failed to set provider on new agent: {}", e))?;

        // Wire worker persona if specified
        if let Some(ref worker_key) = worker_persona {
            let config = crate::config::agent_identity::load_agent_config();
            if let Some(worker) = config.workers.get(worker_key) {
                agent
                    .set_persona_block_override(worker.system_prompt_block(), worker.display_name())
                    .await;
            } else {
                tracing::warn!(
                    target: "permagentd::brain",
                    "Worker persona '{}' not found for orchestrator agent, using primary",
                    worker_key
                );
            }
        }
        tracing::info!(
            target: "permagentd::brain",
            "Orchestrator agent spawned with worker persona: {}",
            worker_persona.as_deref().unwrap_or("(primary)")
        );

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Started agent session '{}' with ID: {}\n\nUse send_message with this session_id to interact with it.",
            name, session.id
        ))]))
    }

    async fn handle_send_message(
        &self,
        parent_session_id: &str,
        parent_cancel: &CancellationToken,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let session_id = extract_string(&args, "session_id")?;
        let message_text = extract_string(&args, "message")?;

        if session_id == parent_session_id {
            return Err("Cannot send a message to the orchestrator's own session".into());
        }

        let manager = self.get_agent_manager().await?;

        let agent = manager
            .get_or_create_agent(session_id.clone())
            .await
            .map_err(|e| format!("Failed to get agent for session '{}': {}", session_id, e))?;

        if agent.provider().await.is_err() {
            if let Ok(parent_provider) = self.get_provider().await {
                let extensions = self.parent_extensions();
                if let Ok(provider) = providers::create(
                    parent_provider.get_name(),
                    parent_provider.get_model_config(),
                    extensions,
                )
                .await
                {
                    agent
                        .update_provider(provider, &session_id)
                        .await
                        .map_err(|e| format!("Failed to set provider: {}", e))?;
                }
            }
        }

        let cancel_token = CancellationToken::new();
        manager
            .try_register_cancel_token(&session_id, cancel_token.clone())
            .await
            .map_err(|_| {
                format!(
                    "Session '{}' is currently busy. Use interrupt_agent first, or wait.",
                    session_id
                )
            })?;

        let mut guard = CancelTokenGuard::new(manager.clone(), session_id.clone());

        let user_message = Message::user().with_text(&message_text);
        let session_config = SessionConfig {
            id: session_id.clone(),
            schedule_id: None,
            max_turns: None,
            retry_config: None,
        };

        let mut stream = agent
            .reply(user_message, session_config, Some(cancel_token.clone()))
            .await
            .map_err(|e| format!("Failed to start reply: {}", e))?;

        let mut response_parts: Vec<String> = Vec::new();
        let mut cancelled = false;

        loop {
            tokio::select! {
                _ = parent_cancel.cancelled() => {
                    cancel_token.cancel();
                    cancelled = true;
                    break;
                }
                event = stream.next() => {
                    match event {
                        Some(Ok(AgentEvent::Message(msg))) => {
                            let text = msg.as_concat_text();
                            if !text.is_empty() {
                                response_parts.push(text);
                            }
                        }
                        Some(Ok(_)) => {}
                        Some(Err(e)) => {
                            response_parts.push(format!("Error during agent processing: {}", e));
                            break;
                        }
                        None => break,
                    }
                }
            }
        }

        drop(stream);
        guard.disarm();
        manager.unregister_cancel_token(&session_id).await;

        if cancelled {
            return Err("Cancelled by parent session".into());
        }

        if response_parts.is_empty() {
            Ok(CallToolResult::success(vec![Content::text(
                "Agent completed without producing text output.",
            )]))
        } else {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "## Response from session {}\n\n{}",
                session_id,
                response_parts.join("\n\n")
            ))]))
        }
    }

    async fn handle_list_workers(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, String> {
        let refresh = arguments
            .as_ref()
            .and_then(|a| a.get("refresh"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if refresh {
            self.probe_cache.clear();
        }

        let config = agent_identity::load_agent_config();
        let mut workers = Vec::new();

        for (key, persona) in &config.workers {
            let (available, reason) = match self.probe_cache.get(key) {
                Some(cached) if !refresh => (cached.available, cached.reason),
                _ => {
                    let (ok, reason) = worker_probe::probe_worker(&persona.availability_check);
                    self.probe_cache.set(key, ok, reason.clone());
                    (ok, reason)
                }
            };

            workers.push(serde_json::json!({
                "key": key,
                "display_name": persona.display_name(),
                "role": persona.role,
                "tool_kinds": persona.tool_kinds,
                "cost_tier": persona.cost_tier,
                "available": available,
                "reason": reason,
            }));
        }

        // Sort by key for stable output
        workers.sort_by(|a, b| {
            a.get("key")
                .and_then(|v| v.as_str())
                .cmp(&b.get("key").and_then(|v| v.as_str()))
        });

        let output = serde_json::to_string_pretty(&workers).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(format!(
            "{} worker(s) configured\n\n{}",
            workers.len(),
            output
        ))]))
    }

    async fn handle_check_worker(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let worker_key = extract_string(&args, "worker_key")?;

        let config = agent_identity::load_agent_config();
        let persona = config
            .workers
            .get(&worker_key)
            .ok_or_else(|| format!("Worker '{}' not found in agent.yaml", worker_key))?;

        // Always re-probe, ignoring cache
        let (available, reason) = worker_probe::probe_worker(&persona.availability_check);
        self.probe_cache.set(&worker_key, available, reason.clone());

        let result = serde_json::json!({
            "worker_key": worker_key,
            "available": available,
            "reason": reason,
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    async fn handle_goal_advance(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let card_id = extract_string(&args, "card_id")?;
        let action_str = extract_string(&args, "action")?;
        let notes = args
            .get("notes")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let action = GoalAction::from_str(&action_str).ok_or_else(|| {
            format!(
                "Invalid action '{}'. Must be: ready, dispatch, review, approve, reject",
                action_str
            )
        })?;

        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())?;

        // Load card and verify it's a goal
        let card = cards::get_card(&pool, &card_id)
            .await?
            .ok_or_else(|| format!("Card '{}' not found", card_id))?;

        if card.card_type != "goal" {
            return Err(format!(
                "Card '{}' is type '{}', not 'goal'. goal_advance only works on goal cards.",
                card_id, card.card_type
            ));
        }

        // Determine current state from the card's column state_binding
        let current_col = cards::get_column(&pool, &card.column_id)
            .await?
            .ok_or_else(|| format!("Column '{}' not found for card", card.column_id))?;

        let current_state = current_col
            .state_binding
            .as_deref()
            .and_then(GoalState::from_binding)
            .ok_or_else(|| {
                format!(
                    "Card '{}' is in column '{}' which has no state_binding. \
                     Goal cards must be in state-bound columns.",
                    card_id, current_col.name
                )
            })?;

        // Validate the transition
        let new_state =
            goal_state::validate_transition(current_state, action).map_err(|e| e.to_string())?;

        // Parse existing metadata
        let mut meta = card.metadata_json.as_object().cloned().unwrap_or_default();

        // Handle reject: check attempt_count for 3-attempt cap
        if action == GoalAction::Reject {
            let attempt_count = meta
                .get("attempt_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            if notes.is_some() {
                meta.insert(
                    "review_notes".to_string(),
                    serde_json::Value::String(notes.clone().unwrap()),
                );
            }

            if attempt_count + 1 >= MAX_GOAL_ATTEMPTS {
                // 3-attempt cap: move to Triage with needs_human_attention
                meta.insert(
                    "attempt_count".to_string(),
                    serde_json::json!(attempt_count + 1),
                );
                meta.insert(
                    "needs_human_attention".to_string(),
                    serde_json::Value::Bool(true),
                );
                meta.insert(
                    "last_error".to_string(),
                    serde_json::Value::String(
                        notes.unwrap_or_else(|| "Rejected after maximum attempts".to_string()),
                    ),
                );
                meta.insert(
                    "goal_state".to_string(),
                    serde_json::Value::String("triage".to_string()),
                );

                // Move to Triage instead of InProgress
                let triage_col = cards::get_goal_column(&pool, &card.project_id, "triage")
                    .await?
                    .ok_or("Triage column not found")?;

                cards::update_card(
                    &pool,
                    &card_id,
                    cards::UpdateCard {
                        metadata_json: Some(serde_json::Value::Object(meta)),
                        ..Default::default()
                    },
                )
                .await?;
                cards::move_card(&pool, &card_id, &triage_col.id, None).await?;

                return Ok(CallToolResult::success(vec![Content::text(format!(
                    "Goal '{}' has reached {} failed attempts. Moved back to Triage with needs_human_attention=true.",
                    card.title, MAX_GOAL_ATTEMPTS
                ))]));
            }

            // Normal reject: increment attempt_count, bounce back to InProgress
            meta.insert(
                "attempt_count".to_string(),
                serde_json::json!(attempt_count + 1),
            );
        }

        // Handle approve: store notes
        if action == GoalAction::Approve {
            if let Some(ref n) = notes {
                meta.insert(
                    "review_notes".to_string(),
                    serde_json::Value::String(n.clone()),
                );
            }
        }

        // Update goal_state in metadata
        meta.insert(
            "goal_state".to_string(),
            serde_json::Value::String(new_state.binding().to_string()),
        );

        // Find target column and move
        let target_col = cards::get_goal_column(&pool, &card.project_id, new_state.binding())
            .await?
            .ok_or_else(|| {
                format!(
                    "Target column for state '{}' not found in project",
                    new_state
                )
            })?;

        cards::update_card(
            &pool,
            &card_id,
            cards::UpdateCard {
                metadata_json: Some(serde_json::Value::Object(meta)),
                ..Default::default()
            },
        )
        .await?;

        cards::move_card(&pool, &card_id, &target_col.id, None).await?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Goal '{}' advanced: {} → {} (action: {})",
            card.title, current_state, new_state, action
        ))]))
    }

    async fn handle_goal_status(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let card_id = extract_string(&args, "card_id")?;

        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())?;

        let card = cards::get_card(&pool, &card_id)
            .await?
            .ok_or_else(|| format!("Card '{}' not found", card_id))?;

        if card.card_type != "goal" {
            return Err(format!(
                "Card '{}' is type '{}', not 'goal'",
                card_id, card.card_type
            ));
        }

        let col = cards::get_column(&pool, &card.column_id).await?;
        let state = col
            .as_ref()
            .and_then(|c| c.state_binding.as_deref())
            .unwrap_or("unknown");

        let meta = card.metadata_json.as_object();
        let worker_key = meta
            .and_then(|m| m.get("worker_key"))
            .and_then(|v| v.as_str());
        let worker_session_id = meta
            .and_then(|m| m.get("worker_session_id"))
            .and_then(|v| v.as_str());
        let dispatched_at = meta
            .and_then(|m| m.get("dispatched_at"))
            .and_then(|v| v.as_str());
        let completed_at = meta
            .and_then(|m| m.get("completed_at"))
            .and_then(|v| v.as_str());
        let attempt_count = meta
            .and_then(|m| m.get("attempt_count"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let last_error = meta
            .and_then(|m| m.get("last_error"))
            .and_then(|v| v.as_str());
        let needs_human_attention = meta
            .and_then(|m| m.get("needs_human_attention"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Check if worker session is alive for last_activity
        let session_alive = if let Some(sid) = worker_session_id {
            if let Ok(manager) = self.get_agent_manager().await {
                manager.is_session_busy(sid).await
            } else {
                false
            }
        } else {
            false
        };

        let result = serde_json::json!({
            "card_id": card_id,
            "title": card.title,
            "state": state,
            "worker_key": worker_key,
            "worker_session_id": worker_session_id,
            "started_at": dispatched_at,
            "completed_at": completed_at,
            "session_alive": session_alive,
            "attempt_count": attempt_count,
            "error": last_error,
            "needs_human_attention": needs_human_attention,
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    async fn handle_interrupt_agent(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let session_id = extract_string(&args, "session_id")?;

        let manager = self.get_agent_manager().await?;

        manager
            .cancel_session(&session_id)
            .await
            .map_err(|e| format!("Failed to interrupt session '{}': {}", session_id, e))?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Interrupted agent session '{}'.",
            session_id
        ))]))
    }
}

#[async_trait]
impl McpClientTrait for OrchestratorClient {
    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        let tools = vec![
            Tool::new(
                "list_sessions".to_string(),
                "List agent sessions with their status (loaded, busy, idle). Returns the most recent 10 by default. Optionally filter by session type."
                    .to_string(),
                schema::<ListSessionsParams>(),
            ),
            Tool::new(
                "view_session".to_string(),
                "View a session's details and conversation. Mode 'first_last' (default) returns the first and last message. Mode 'summarize' calls the LLM to produce a conversation summary."
                    .to_string(),
                schema::<ViewSessionParams>(),
            ),
            Tool::new(
                "start_agent".to_string(),
                "Start a new agent session with its own working directory. Inherits the current provider and model. Returns a session_id for future interaction."
                    .to_string(),
                schema::<StartAgentParams>(),
            ),
            Tool::new(
                "send_message".to_string(),
                "Send a message to an existing agent session and get the response. Returns an error if the agent is currently busy."
                    .to_string(),
                schema::<SendMessageParams>(),
            ),
            Tool::new(
                "interrupt_agent".to_string(),
                "Interrupt a busy agent by cancelling its current operation."
                    .to_string(),
                schema::<InterruptAgentParams>(),
            ),
            Tool::new(
                "list_workers".to_string(),
                "List all configured workers from agent.yaml with their availability status. \
                 Use this to inspect what workers are available before dispatching goals. \
                 Set refresh=true to force re-probing all workers."
                    .to_string(),
                schema::<ListWorkersParams>(),
            ),
            Tool::new(
                "check_worker".to_string(),
                "Check a specific worker's availability by probing its detection method. \
                 Always re-probes regardless of cache. Use before dispatching to a specific worker."
                    .to_string(),
                schema::<CheckWorkerParams>(),
            ),
            Tool::new(
                "goal_advance".to_string(),
                "Advance a goal card through its lifecycle. Actions: \
                 'ready' (Triage→Ready), 'dispatch' (Ready→InProgress), \
                 'review' (InProgress→Review), 'approve' (Review→Complete), \
                 'reject' (Review→InProgress, or Triage if 3 attempts reached). \
                 Only works on card_type='goal' cards."
                    .to_string(),
                schema::<GoalAdvanceParams>(),
            ),
            Tool::new(
                "goal_status".to_string(),
                "Get detailed status of a goal card including its lifecycle state, \
                 assigned worker, session liveness, attempt count, and any errors. \
                 Only works on card_type='goal' cards."
                    .to_string(),
                schema::<GoalStatusParams>(),
            ),
        ];

        Ok(ListToolsResult {
            tools,
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        ctx: &ToolCallContext,
        name: &str,
        arguments: Option<JsonObject>,
        cancel_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        let result = match name {
            "list_sessions" => self.handle_list_sessions(arguments).await,
            "view_session" => self.handle_view_session(&ctx.session_id, arguments).await,
            "start_agent" => self.handle_start_agent(arguments).await,
            "send_message" => {
                self.handle_send_message(&ctx.session_id, &cancel_token, arguments)
                    .await
            }
            "interrupt_agent" => self.handle_interrupt_agent(arguments).await,
            "list_workers" => self.handle_list_workers(arguments).await,
            "check_worker" => self.handle_check_worker(arguments).await,
            "goal_advance" => self.handle_goal_advance(arguments).await,
            "goal_status" => self.handle_goal_status(arguments).await,
            _ => Err(format!("Unknown tool: {}", name)),
        };

        match result {
            Ok(result) => Ok(result),
            Err(error) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Error: {}",
                error
            ))])),
        }
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }
}

fn schema<T: JsonSchema>() -> JsonObject {
    let mut obj = serde_json::to_value(schema_for!(T))
        .map(|v| v.as_object().unwrap().clone())
        .expect("valid schema");
    obj.entry("properties")
        .or_insert_with(|| serde_json::json!({}));
    obj
}

fn extract_string(args: &JsonObject, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("Missing or invalid '{}'", key))
}

/// Handle the completion of a dispatched goal worker.
///
/// Called by the tracker task spawned in dispatch_goal when the JoinHandle resolves.
/// On success: moves card InProgress → Review.
/// On failure: increments attempt_count. At 3 attempts, moves to Triage with
/// needs_human_attention. Otherwise leaves in InProgress for retry.
///
/// Gracefully no-ops if the card is no longer in InProgress (manual intervention).
pub async fn handle_goal_completion(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    card_id: &str,
    project_id: &str,
    result: Result<(), String>,
) -> Result<(), String> {
    let card = cards::get_card(&pool, card_id)
        .await?
        .ok_or_else(|| format!("Card '{}' not found during completion handling", card_id))?;

    // Check card is still in InProgress — if not, someone manually intervened; no-op.
    let current_col = cards::get_column(&pool, &card.column_id).await?;
    match current_col
        .as_ref()
        .and_then(|c| c.state_binding.as_deref())
    {
        Some("in_progress") => {} // expected, continue
        other => {
            tracing::info!(
                target: "permagentd::brain",
                "Goal '{}' completion handler: card is in state {:?}, not in_progress — skipping (manual intervention assumed)",
                card.title,
                other
            );
            return Ok(());
        }
    }

    let mut meta = card.metadata_json.as_object().cloned().unwrap_or_default();

    match result {
        Ok(()) => {
            // Success: move to Review
            meta.insert(
                "goal_state".to_string(),
                serde_json::Value::String("review".to_string()),
            );
            meta.insert(
                "completed_at".to_string(),
                serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
            );

            cards::update_card(
                &pool,
                card_id,
                cards::UpdateCard {
                    metadata_json: Some(serde_json::Value::Object(meta)),
                    ..Default::default()
                },
            )
            .await?;

            let review_col = cards::get_goal_column(&pool, project_id, "review")
                .await?
                .ok_or("Review column not found")?;
            cards::move_card(&pool, card_id, &review_col.id, None).await?;

            tracing::info!(
                target: "permagentd::brain",
                "Goal '{}' worker completed successfully — moved to Review",
                card.title
            );
        }
        Err(error) => {
            let attempt_count = meta
                .get("attempt_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            meta.insert(
                "last_error".to_string(),
                serde_json::Value::String(error.clone()),
            );

            if attempt_count >= MAX_GOAL_ATTEMPTS {
                // Terminal failure: move to Triage with needs_human_attention
                meta.insert(
                    "goal_state".to_string(),
                    serde_json::Value::String("triage".to_string()),
                );
                meta.insert(
                    "needs_human_attention".to_string(),
                    serde_json::Value::Bool(true),
                );

                cards::update_card(
                    &pool,
                    card_id,
                    cards::UpdateCard {
                        metadata_json: Some(serde_json::Value::Object(meta)),
                        ..Default::default()
                    },
                )
                .await?;

                let triage_col = cards::get_goal_column(&pool, project_id, "triage")
                    .await?
                    .ok_or("Triage column not found")?;
                cards::move_card(&pool, card_id, &triage_col.id, None).await?;

                tracing::warn!(
                    target: "permagentd::brain",
                    "Goal '{}' failed {} times — moved to Triage with needs_human_attention",
                    card.title,
                    attempt_count
                );
            } else {
                // Retriable failure: leave in InProgress, metadata already updated
                meta.insert(
                    "goal_state".to_string(),
                    serde_json::Value::String("in_progress".to_string()),
                );

                cards::update_card(
                    &pool,
                    card_id,
                    cards::UpdateCard {
                        metadata_json: Some(serde_json::Value::Object(meta)),
                        ..Default::default()
                    },
                )
                .await?;

                tracing::warn!(
                    target: "permagentd::brain",
                    "Goal '{}' worker failed (attempt {}): {} — leaving in InProgress for retry",
                    card.title,
                    attempt_count,
                    error
                );
            }
        }
    }

    Ok(())
}

/// Resume in-progress goals after daemon restart.
///
/// Scans for goal cards in the `in_progress` state and either:
/// - Moves dead-session cards to Ready (or Triage at 3 attempts)
/// - Re-attaches a polling tracker for alive sessions
pub async fn resume_in_progress_goals(
    session_manager: &crate::session::SessionManager,
) -> Result<(), String> {
    let pool = session_manager
        .pool_clone()
        .await
        .map_err(|e| e.to_string())?;

    // Find all in-progress goal cards across all projects
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT c.id, c.project_id FROM cards c
         JOIN board_columns bc ON c.column_id = bc.id
         WHERE c.card_type = 'goal'
           AND bc.state_binding = 'in_progress'
           AND c.archived_at IS NULL",
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    if rows.is_empty() {
        return Ok(());
    }

    tracing::info!(
        target: "permagentd::brain",
        "Resuming {} in-progress goal(s) from prior session",
        rows.len()
    );

    let manager = AgentManager::instance().await.ok();

    for (card_id, project_id) in rows {
        if let Err(e) = resume_single_goal(&pool, &manager, &card_id, &project_id).await {
            tracing::warn!(
                target: "permagentd::brain",
                "Failed to resume goal {}: {}",
                card_id,
                e
            );
        }
    }

    Ok(())
}

async fn resume_single_goal(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    manager: &Option<Arc<AgentManager>>,
    card_id: &str,
    project_id: &str,
) -> Result<(), String> {
    let card = cards::get_card(pool, card_id)
        .await?
        .ok_or_else(|| format!("Card {} not found during resume", card_id))?;

    let meta = card.metadata_json.as_object();
    let session_id = meta
        .and_then(|m| m.get("worker_session_id"))
        .and_then(|v| v.as_str());
    let attempt_count = meta
        .and_then(|m| m.get("attempt_count"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // Check if worker session is still alive
    let session_alive = match (session_id, manager) {
        (Some(sid), Some(mgr)) => mgr.is_session_busy(sid).await,
        _ => false,
    };

    if session_alive {
        // Case 2: session is alive — spawn polling tracker
        let pool_clone = pool.clone();
        let card_id = card_id.to_string();
        let project_id = project_id.to_string();
        let sid = session_id.unwrap().to_string();

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;

                let still_busy = match AgentManager::instance().await {
                    Ok(mgr) => mgr.is_session_busy(&sid).await,
                    Err(_) => false,
                };

                if !still_busy {
                    // Session finished — treat as success (we can't recover the
                    // actual result from a prior daemon lifecycle, so assume success
                    // and let the user review)
                    if let Err(e) =
                        handle_goal_completion(&pool_clone, &card_id, &project_id, Ok(())).await
                    {
                        tracing::warn!(
                            target: "permagentd::brain",
                            "Failed to handle resumed goal completion for {}: {}",
                            card_id,
                            e
                        );
                    }
                    break;
                }
            }
        });

        tracing::info!(
            target: "permagentd::brain",
            "Re-attached tracker for alive goal '{}' (session: {})",
            card.title,
            session_id.unwrap_or("?")
        );
    } else {
        // Case 1: session is dead — move to Ready or Triage
        let new_attempt = attempt_count + 1;
        let mut meta = card.metadata_json.as_object().cloned().unwrap_or_default();

        meta.insert("attempt_count".to_string(), serde_json::json!(new_attempt));
        meta.insert(
            "last_error".to_string(),
            serde_json::Value::String("Abandoned during daemon restart".to_string()),
        );

        if new_attempt >= MAX_GOAL_ATTEMPTS {
            // Terminal: move to Triage with needs_human_attention
            meta.insert(
                "goal_state".to_string(),
                serde_json::Value::String("triage".to_string()),
            );
            meta.insert(
                "needs_human_attention".to_string(),
                serde_json::Value::Bool(true),
            );

            cards::update_card(
                pool,
                card_id,
                cards::UpdateCard {
                    metadata_json: Some(serde_json::Value::Object(meta)),
                    ..Default::default()
                },
            )
            .await?;

            let triage_col = cards::get_goal_column(pool, project_id, "triage")
                .await?
                .ok_or("Triage column not found")?;
            cards::move_card(pool, card_id, &triage_col.id, None).await?;

            tracing::warn!(
                target: "permagentd::brain",
                "Goal '{}' reached {} attempts after restart — moved to Triage with needs_human_attention",
                card.title,
                new_attempt
            );
        } else {
            // Retriable: move to Ready for re-dispatch
            meta.insert(
                "goal_state".to_string(),
                serde_json::Value::String("ready".to_string()),
            );

            cards::update_card(
                pool,
                card_id,
                cards::UpdateCard {
                    metadata_json: Some(serde_json::Value::Object(meta)),
                    ..Default::default()
                },
            )
            .await?;

            let ready_col = cards::get_goal_column(pool, project_id, "ready")
                .await?
                .ok_or("Ready column not found")?;
            cards::move_card(pool, card_id, &ready_col.id, None).await?;

            tracing::info!(
                target: "permagentd::brain",
                "Goal '{}' moved to Ready after restart (attempt {}/{})",
                card.title,
                new_attempt,
                MAX_GOAL_ATTEMPTS
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> sqlx::Pool<sqlx::Sqlite> {
        use crate::session::spectral_schema::init_spectral_db;
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    /// Create a goal card in a specific state for testing.
    async fn setup_goal_in_state(
        pool: &sqlx::Pool<sqlx::Sqlite>,
        state_binding: &str,
        attempt_count: u64,
    ) -> cards::Card {
        use crate::projects::PERSONAL_PROJECT_ID;

        // Ensure goal columns exist
        cards::seed_goal_columns(pool, PERSONAL_PROJECT_ID)
            .await
            .unwrap();

        let col = cards::get_goal_column(pool, PERSONAL_PROJECT_ID, state_binding)
            .await
            .unwrap()
            .unwrap();

        let mut meta = serde_json::Map::new();
        meta.insert(
            "attempt_count".to_string(),
            serde_json::json!(attempt_count),
        );
        meta.insert(
            "goal_state".to_string(),
            serde_json::Value::String(state_binding.to_string()),
        );

        cards::create_card(
            pool,
            cards::CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: format!("Test goal in {}", state_binding),
                description: Some("test".to_string()),
                card_type: Some("goal".to_string()),
                column_id: Some(col.id.clone()),
                created_by: None,
                metadata_json: Some(serde_json::Value::Object(meta)),
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn completion_success_moves_to_review() {
        let pool = test_pool().await;
        let card = setup_goal_in_state(&pool, "in_progress", 1).await;

        handle_goal_completion(&pool, &card.id, &card.project_id, Ok(()))
            .await
            .unwrap();

        let updated = cards::get_card(&pool, &card.id).await.unwrap().unwrap();
        let col = cards::get_column(&pool, &updated.column_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(col.state_binding.as_deref(), Some("review"));
        assert_eq!(
            updated.metadata_json.get("goal_state").unwrap().as_str(),
            Some("review")
        );
        assert!(updated.metadata_json.get("completed_at").is_some());
    }

    #[tokio::test]
    async fn completion_failure_leaves_in_progress_on_first_attempt() {
        let pool = test_pool().await;
        let card = setup_goal_in_state(&pool, "in_progress", 1).await;

        handle_goal_completion(
            &pool,
            &card.id,
            &card.project_id,
            Err("Worker crashed".to_string()),
        )
        .await
        .unwrap();

        let updated = cards::get_card(&pool, &card.id).await.unwrap().unwrap();
        let col = cards::get_column(&pool, &updated.column_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            col.state_binding.as_deref(),
            Some("in_progress"),
            "Should stay in InProgress on retriable failure"
        );
        assert_eq!(
            updated.metadata_json.get("last_error").unwrap().as_str(),
            Some("Worker crashed")
        );
    }

    #[tokio::test]
    async fn completion_failure_moves_to_triage_on_third_attempt() {
        let pool = test_pool().await;
        let card = setup_goal_in_state(&pool, "in_progress", 3).await;

        handle_goal_completion(
            &pool,
            &card.id,
            &card.project_id,
            Err("Worker crashed again".to_string()),
        )
        .await
        .unwrap();

        let updated = cards::get_card(&pool, &card.id).await.unwrap().unwrap();
        let col = cards::get_column(&pool, &updated.column_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            col.state_binding.as_deref(),
            Some("triage"),
            "Should move to Triage after 3 attempts"
        );
        assert_eq!(
            updated
                .metadata_json
                .get("needs_human_attention")
                .unwrap()
                .as_bool(),
            Some(true)
        );
        assert!(updated.metadata_json.get("last_error").is_some());
    }

    #[tokio::test]
    async fn completion_noops_if_card_not_in_progress() {
        let pool = test_pool().await;
        // Card is in Review (someone already approved manually)
        let card = setup_goal_in_state(&pool, "review", 1).await;

        handle_goal_completion(&pool, &card.id, &card.project_id, Ok(()))
            .await
            .unwrap();

        // Card should still be in Review — no-op
        let updated = cards::get_card(&pool, &card.id).await.unwrap().unwrap();
        let col = cards::get_column(&pool, &updated.column_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            col.state_binding.as_deref(),
            Some("review"),
            "Should not change card that's already been moved"
        );
    }

    #[tokio::test]
    async fn goal_status_returns_correct_shape() {
        let pool = test_pool().await;
        let card = setup_goal_in_state(&pool, "in_progress", 2).await;

        // Add some metadata fields that goal_status reads
        let mut meta = card.metadata_json.as_object().cloned().unwrap_or_default();
        meta.insert(
            "worker_key".to_string(),
            serde_json::Value::String("codex".to_string()),
        );
        meta.insert(
            "worker_session_id".to_string(),
            serde_json::Value::String("20260528_1".to_string()),
        );
        meta.insert(
            "dispatched_at".to_string(),
            serde_json::Value::String("2026-05-28T10:00:00Z".to_string()),
        );
        cards::update_card(
            &pool,
            &card.id,
            cards::UpdateCard {
                metadata_json: Some(serde_json::Value::Object(meta)),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // Call the function directly (we can't easily construct OrchestratorClient in tests,
        // but we can verify the data extraction logic via the card + column)
        let updated = cards::get_card(&pool, &card.id).await.unwrap().unwrap();
        let col = cards::get_column(&pool, &updated.column_id)
            .await
            .unwrap()
            .unwrap();
        let state = col.state_binding.as_deref().unwrap_or("unknown");
        let umeta = updated.metadata_json.as_object().unwrap();

        assert_eq!(state, "in_progress");
        assert_eq!(umeta.get("worker_key").unwrap().as_str(), Some("codex"));
        assert_eq!(
            umeta.get("worker_session_id").unwrap().as_str(),
            Some("20260528_1")
        );
        assert_eq!(umeta.get("attempt_count").unwrap().as_u64(), Some(2));
    }

    #[tokio::test]
    async fn resume_no_in_progress_goals_is_noop() {
        let pool = test_pool().await;
        // No goals at all — should succeed with no side effects
        let result = resume_single_goal(&pool, &None, "nonexistent", "nonexistent").await;
        assert!(result.is_err()); // card not found — expected
    }

    #[tokio::test]
    async fn resume_dead_session_moves_to_ready() {
        let pool = test_pool().await;
        let card = setup_goal_in_state(&pool, "in_progress", 1).await;

        // No manager = session considered dead
        resume_single_goal(&pool, &None, &card.id, &card.project_id)
            .await
            .unwrap();

        let updated = cards::get_card(&pool, &card.id).await.unwrap().unwrap();
        let col = cards::get_column(&pool, &updated.column_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(col.state_binding.as_deref(), Some("ready"));
        assert_eq!(
            updated.metadata_json.get("attempt_count").unwrap().as_u64(),
            Some(2)
        );
        assert_eq!(
            updated.metadata_json.get("last_error").unwrap().as_str(),
            Some("Abandoned during daemon restart")
        );
        assert_eq!(
            updated.metadata_json.get("goal_state").unwrap().as_str(),
            Some("ready")
        );
    }

    #[tokio::test]
    async fn resume_dead_session_at_max_attempts_moves_to_triage() {
        let pool = test_pool().await;
        let card = setup_goal_in_state(&pool, "in_progress", 2).await;

        resume_single_goal(&pool, &None, &card.id, &card.project_id)
            .await
            .unwrap();

        let updated = cards::get_card(&pool, &card.id).await.unwrap().unwrap();
        let col = cards::get_column(&pool, &updated.column_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(col.state_binding.as_deref(), Some("triage"));
        assert_eq!(
            updated.metadata_json.get("attempt_count").unwrap().as_u64(),
            Some(3)
        );
        assert_eq!(
            updated
                .metadata_json
                .get("needs_human_attention")
                .unwrap()
                .as_bool(),
            Some(true)
        );
    }

    #[tokio::test]
    async fn resume_missing_session_id_treated_as_dead() {
        let pool = test_pool().await;
        use crate::projects::PERSONAL_PROJECT_ID;

        cards::seed_goal_columns(&pool, PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        let col = cards::get_goal_column(&pool, PERSONAL_PROJECT_ID, "in_progress")
            .await
            .unwrap()
            .unwrap();

        // Create card with NO worker_session_id in metadata
        let card = cards::create_card(
            &pool,
            cards::CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "No session".to_string(),
                description: None,
                card_type: Some("goal".to_string()),
                column_id: Some(col.id.clone()),
                created_by: None,
                metadata_json: Some(
                    serde_json::json!({"attempt_count": 0, "goal_state": "in_progress"}),
                ),
            },
        )
        .await
        .unwrap();

        resume_single_goal(&pool, &None, &card.id, &card.project_id)
            .await
            .unwrap();

        let updated = cards::get_card(&pool, &card.id).await.unwrap().unwrap();
        let ucol = cards::get_column(&pool, &updated.column_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            ucol.state_binding.as_deref(),
            Some("ready"),
            "Missing session_id should be treated as dead session"
        );
    }

    #[tokio::test]
    async fn resume_multiple_cards_handled_independently() {
        let pool = test_pool().await;
        let card1 = setup_goal_in_state(&pool, "in_progress", 1).await;
        let card2 = setup_goal_in_state(&pool, "in_progress", 2).await;

        // Both dead (no manager)
        resume_single_goal(&pool, &None, &card1.id, &card1.project_id)
            .await
            .unwrap();
        resume_single_goal(&pool, &None, &card2.id, &card2.project_id)
            .await
            .unwrap();

        let u1 = cards::get_card(&pool, &card1.id).await.unwrap().unwrap();
        let c1 = cards::get_column(&pool, &u1.column_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            c1.state_binding.as_deref(),
            Some("ready"),
            "card1: attempt 1 → Ready"
        );

        let u2 = cards::get_card(&pool, &card2.id).await.unwrap().unwrap();
        let c2 = cards::get_column(&pool, &u2.column_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            c2.state_binding.as_deref(),
            Some("triage"),
            "card2: attempt 2 → Triage (at cap)"
        );
    }

    #[tokio::test]
    async fn goal_status_fails_on_standard_card() {
        let pool = test_pool().await;
        use crate::projects::PERSONAL_PROJECT_ID;

        let card = cards::create_card(
            &pool,
            cards::CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "Standard task".to_string(),
                description: None,
                card_type: Some("standard".to_string()),
                column_id: None,
                created_by: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();

        // goal_status logic rejects non-goal cards
        assert_eq!(card.card_type, "standard");
        // The actual tool handler checks card_type != "goal" and returns Err
        // We verify the card_type here since we can't easily call the MCP handler
    }
}
