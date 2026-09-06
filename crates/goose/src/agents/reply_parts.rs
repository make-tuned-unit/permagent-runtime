use anyhow::Result;
use regex::Regex;
use std::sync::Arc;

use async_stream::try_stream;
use futures::stream::StreamExt;
use serde_json::{json, Value};
use tracing::debug;

use super::super::agents::Agent;
#[cfg(feature = "code-mode")]
use crate::agents::platform_extensions::code_execution;
use crate::config::GooseMode;
use crate::conversation::message::{Message, MessageContent, ToolRequest};
use crate::conversation::Conversation;
use crate::cost_router::cache::SystemPromptParts;
#[cfg(test)]
use crate::providers::base::stream_from_single_message;
use crate::providers::base::{MessageStream, Provider, ProviderUsage};
use crate::providers::canonical::{
    cache_hit_rate_of, cache_savings_of, cost_breakdown, maybe_get_pricing, worst_case_pricing,
};
use crate::providers::errors::ProviderError;
use crate::providers::toolshim::{
    augment_message_with_tool_calls, convert_tool_messages_to_text,
    modify_system_prompt_for_tool_json, OllamaInterpreter,
};
use crate::session::{
    budget_task_id, goal_id, CostLedgerRow, CostTier, Session, SessionManager, SessionType,
};
use rmcp::model::Tool;

async fn enhance_model_error(error: ProviderError, provider: &Arc<dyn Provider>) -> ProviderError {
    let ProviderError::RequestFailed(ref msg) = error else {
        return error;
    };

    let re = Regex::new(r"(?i)\b4\d{2}\b.*model|model.*\b4\d{2}\b").unwrap();
    if !re.is_match(msg) {
        return error;
    }

    let Ok(models) = provider.fetch_recommended_models().await else {
        return error;
    };
    if models.is_empty() {
        return error;
    }

    ProviderError::RequestFailed(format!(
        "{}. Available models for this provider: {}",
        msg,
        models.join(", ")
    ))
}

fn coerce_value(s: &str, schema: &Value) -> Value {
    let type_str = schema.get("type");

    match type_str {
        Some(Value::String(t)) => match t.as_str() {
            "number" | "integer" => try_coerce_number(s),
            "boolean" => try_coerce_boolean(s),
            _ => Value::String(s.to_string()),
        },
        Some(Value::Array(types)) => {
            // Try each type in order
            for t in types {
                if let Value::String(type_name) = t {
                    match type_name.as_str() {
                        "number" | "integer" if s.parse::<f64>().is_ok() => {
                            return try_coerce_number(s);
                        }
                        "boolean" if matches!(s.to_lowercase().as_str(), "true" | "false") => {
                            return try_coerce_boolean(s);
                        }
                        _ => continue,
                    }
                }
            }
            Value::String(s.to_string())
        }
        _ => Value::String(s.to_string()),
    }
}

fn try_coerce_number(s: &str) -> Value {
    if let Ok(n) = s.parse::<f64>() {
        if n.fract() == 0.0 && n >= i64::MIN as f64 && n <= i64::MAX as f64 {
            json!(n as i64)
        } else {
            json!(n)
        }
    } else {
        Value::String(s.to_string())
    }
}

fn try_coerce_boolean(s: &str) -> Value {
    match s.to_lowercase().as_str() {
        "true" => json!(true),
        "false" => json!(false),
        _ => Value::String(s.to_string()),
    }
}

pub(crate) fn coerce_tool_arguments(
    arguments: Option<serde_json::Map<String, Value>>,
    tool_schema: &Value,
) -> Option<serde_json::Map<String, Value>> {
    let args = arguments?;

    let properties = tool_schema.get("properties").and_then(|p| p.as_object())?;

    let mut coerced = serde_json::Map::new();

    for (key, value) in args.iter() {
        let coerced_value =
            if let (Value::String(s), Some(prop_schema)) = (value, properties.get(key)) {
                coerce_value(s, prop_schema)
            } else {
                value.clone()
            };
        coerced.insert(key.clone(), coerced_value);
    }

    Some(coerced)
}

async fn toolshim_postprocess(
    response: Message,
    toolshim_tools: &[Tool],
) -> Result<Message, ProviderError> {
    let interpreter = OllamaInterpreter::new().map_err(|e| {
        ProviderError::ExecutionError(format!("Failed to create OllamaInterpreter: {}", e))
    })?;

    augment_message_with_tool_calls(&interpreter, response, toolshim_tools)
        .await
        .map_err(|e| ProviderError::ExecutionError(format!("Failed to augment message: {}", e)))
}

impl Agent {
    /// Immutable identity captured immediately before a provider dispatch.
    /// Keeping these fields outside the mutable session/provider state prevents
    /// a provider switch on the next turn from rewriting this invocation's
    /// attribution.
    pub(crate) async fn reserve_provider_invocation(
        &self,
        session: &Session,
        provider: &Arc<dyn Provider>,
        invocation_id: String,
    ) -> std::result::Result<ProviderInvocationContext, ProviderAuthorizationFailure> {
        reserve_provider_invocation_for_model(
            Arc::clone(&self.config.session_manager),
            session,
            provider.get_name(),
            provider.cost_tier(),
            provider.get_model_config(),
            provider.retry_config().max_physical_attempts(),
            invocation_id,
        )
        .await
    }

    /// Make a budget denial durable for a goal worker, then park and stop it.
    /// The requested bound is authorization-only and is intentionally never
    /// presented as already-spent money.
    pub(crate) async fn handle_provider_authorization_failure(
        &self,
        session: &Session,
        failure: &ProviderAuthorizationFailure,
    ) -> String {
        let detail = failure.to_string();
        let Some(card_id) = goal_id(&session.extension_data) else {
            return detail;
        };
        let pool = match self.config.session_manager.pool_clone().await {
            Ok(pool) => pool,
            Err(error) => return format!("{detail}; could not open Decision Inbox: {error}"),
        };

        // Refused/unknown outcomes have no truthful BudgetVerdict (in
        // particular, an unknown provider result is not zero dollars). Use the
        // existing unblock decision shape and the sole guarded park path. A
        // gate fallback remains a choice so the user can actually authorize
        // the next attempt if the worker/session mapping raced with dispatch.
        let is_gate = matches!(failure, ProviderAuthorizationFailure::NeedsGate { .. });
        let kind = if is_gate { "choice" } else { "unblock" };
        let already_open = crate::decisions::find_open_decision_for_goal(&pool, &card_id, kind)
            .await
            .ok()
            .flatten()
            .is_some_and(|decision| {
                !is_gate || decision.headline.contains("authorization needs approval")
            });
        if !already_open {
            let request = if let ProviderAuthorizationFailure::NeedsGate {
                scope,
                spent_usd,
                held_usd,
                requested_usd,
                ceiling_usd,
            } = failure
            {
                let cfg = crate::cost_router::budget::load_budget_config();
                crate::cost_router::budget::reservation_gate_decision_request(
                    *scope,
                    *spent_usd,
                    *held_usd,
                    *requested_usd,
                    *ceiling_usd,
                    match scope {
                        crate::cost_router::budget::BudgetScope::Task => cfg.task.gate,
                        crate::cost_router::budget::BudgetScope::Session => cfg.session.gate,
                    },
                    Some(card_id.clone()),
                    None,
                )
            } else {
                let reason = match failure {
                    ProviderAuthorizationFailure::Refused { .. } => {
                        crate::decisions::UnblockReason::TokenBudget
                    }
                    _ => crate::decisions::UnblockReason::Stuck,
                };
                let payload = serde_json::to_value(crate::decisions::UnblockPayload {
                    reason,
                    spent: None,
                    cap: None,
                })
                .unwrap_or_default();
                crate::decisions::NewDecision {
                    kind: kind.to_string(),
                    goal_id: Some(card_id.clone()),
                    project_id: None,
                    headline: Some(crate::decisions::truncate_for_headline(
                        "Provider spend authorization blocked this goal",
                    )),
                    detail: Some(detail.clone()),
                    payload,
                    ..Default::default()
                }
            };
            if let Err(error) = crate::decisions::create_decision(&pool, request).await {
                tracing::error!(goal_id = %card_id, "could not create provider budget decision: {error}");
            }
        }
        if let Err(error) = crate::goal_transition::park_goal(
            &pool,
            &card_id,
            crate::decisions::ACTOR_SYSTEM,
            &detail,
        )
        .await
        {
            tracing::warn!(goal_id = %card_id, "could not park blocked provider goal: {error}");
        }
        if let Some(kill) =
            crate::agents::platform_extensions::orchestrator::take_goal_worker(&card_id)
        {
            kill.kill();
        }
        detail
    }

    pub async fn prepare_tools_and_prompt(
        &self,
        session_id: &str,
        working_dir: &std::path::Path,
    ) -> Result<(Vec<Tool>, Vec<Tool>, SystemPromptParts)> {
        // Get tools from extension manager
        let mut tools = self.list_tools(session_id, None).await;

        // Add frontend tools
        let frontend_tools = self.frontend_tools.lock().await;
        for frontend_tool in frontend_tools.values() {
            tools.push(frontend_tool.tool.clone());
        }

        #[cfg(feature = "code-mode")]
        let code_execution_active = self
            .extension_manager
            .is_extension_enabled(code_execution::EXTENSION_NAME)
            .await;
        #[cfg(not(feature = "code-mode"))]
        let code_execution_active = false;
        #[cfg(feature = "code-mode")]
        if code_execution_active {
            let disclosure_style =
                crate::agents::platform_extensions::code_execution::get_tool_disclosure();

            tools = tools
                .into_iter()
                .filter_map(|mut t| match disclosure_style {
                    pctx_code_mode::config::ToolDisclosure::Catalog
                    | pctx_code_mode::config::ToolDisclosure::Filesystem => {
                        // in catalog & filesystem styles, progressive search is handled
                        // by pctx, so we want to omit all non-first-class extensions
                        // from the standard tool list
                        if crate::agents::extension_manager::get_tool_owner(&t).is_some_and(|o| {
                            crate::agents::extension_manager::is_first_class_extension(&o)
                        }) {
                            Some(t)
                        } else {
                            None
                        }
                    }
                    pctx_code_mode::config::ToolDisclosure::Sidecar => {
                        // in sidecar style there is no progressive search, just a way to chain tools
                        // together with typescript
                        // add output schema to description since many model providers drop the
                        // output schema when presenting tools to the model
                        let output_schema = t
                            .output_schema
                            .as_ref()
                            .map(|s| serde_json::json!(s).to_string())
                            .unwrap_or("unknown".to_string());
                        let description_extension = format!(
                            "The successful return schema of this tool is:\n{output_schema}"
                        );

                        t.description = Some(
                            t.description
                                .map(|t| format!("{t}\n{description_extension}"))
                                .unwrap_or(description_extension)
                                .into(),
                        );

                        Some(t)
                    }
                })
                .collect();
        }

        // Filter out tools not visible to the model per MCP Apps visibility spec.
        // Tools with `_meta.ui.visibility` that doesn't include "model" are app-only.
        tools.retain(is_tool_visible_to_model);

        // Stable tool ordering is important for multi session prompt caching.
        tools.sort_by(|a, b| a.name.cmp(&b.name));

        // Inject saved skills into system prompt (Hermes feedback loop)
        if let Ok(pool) = self.config.session_manager.pool_clone().await {
            if let Ok(Some(skills_prompt)) = crate::skills::build_skills_prompt(&pool).await {
                let mut pm = self.prompt_manager.lock().await;
                pm.add_system_prompt_extra("saved_skills".to_string(), skills_prompt);
            }
        }

        // First-run guided-tour offer (once). Suppressed once the user engages or
        // declines (the `load_feature_lesson` tool sets `tour_completed`).
        let tour_completed = crate::config::Config::global()
            .get_param::<bool>(crate::agents::self_knowledge::TOUR_COMPLETED_KEY)
            .unwrap_or(false);
        if !tour_completed {
            let mut pm = self.prompt_manager.lock().await;
            pm.add_system_prompt_extra(
                "tour_offer".to_string(),
                "This user has not been shown around yet. Early in the conversation, once, \
                 warmly offer a short guided tour of what you can do. If they accept, run it \
                 with the `tour` skill (call `load_feature_lesson` per feature). If they \
                 decline, call `load_feature_lesson` with feature_id \"decline\" so you don't \
                 ask again. Offer only once — never nag."
                    .to_string(),
            );
        } else {
            // Past first-run: the onboarding coach may gently surface ONE feature
            // the user hasn't tried yet — at most once a day (its own config
            // cooldown), through this same in-context seam, not a new channel.
            if let Some(hint) =
                crate::agents::self_knowledge::teachable::proactive_learn_next_hint()
            {
                let mut pm = self.prompt_manager.lock().await;
                pm.add_system_prompt_extra("learn_next_offer".to_string(), hint);
            }
        }

        // Prepare system prompt
        let worker_key = {
            let pm = self.prompt_manager.lock().await;
            pm.worker_key().map(str::to_owned)
        };
        let mut extensions_info = self
            .extension_manager
            .get_extensions_info(working_dir)
            .await;
        for ext in &mut extensions_info {
            if crate::public_apis::is_public_apis_extension(&ext.name) {
                ext.instructions =
                    crate::public_apis::instructions_for_agent(worker_key.as_deref());
            }
        }

        // The self-knowledge capability inventory is scoped to this list only
        // for a session that DECLARED an explicit extension set — recipe/CLI
        // runs (`GoosePlatform::GooseCli`, e.g. `permagent run --recipe`),
        // which load a fixed, small roster and never add to it. The daemon's
        // resident chat sessions (`GoosePlatform::GooseDesktop` — Aria) always
        // pass `None` here regardless of how many extensions happen to be
        // active this turn: describing everything Permagent can do is a
        // product contract for that agent, not something a smaller loaded set
        // should narrow. See #1090 — a coding-harness session with 2
        // extensions got a 69KB inventory of all 33 registered ones.
        let declared_extensions = matches!(
            self.config.goose_platform,
            crate::agents::GoosePlatform::GooseCli
        )
        .then(|| extensions_info.iter().map(|e| e.name.clone()).collect());

        let (extension_count, tool_count) = self
            .extension_manager
            .get_extension_and_tool_counts(session_id)
            .await;

        // Get model name from provider
        let provider = self.provider().await?;
        let model_config = provider.get_model_config();

        // M3: tell the model which model it is. Read from the live provider
        // handle every turn — the SAME one the composer footer reads — so a
        // mid-session failover moves this line with it instead of leaving the
        // model to guess (and, on 2026-08-31, to guess wrong).
        {
            let mut pm = self.prompt_manager.lock().await;
            pm.add_system_prompt_extra(
                crate::agents::prompt_manager::MODEL_IDENTITY_KEY.to_string(),
                crate::agents::prompt_manager::model_identity_line(
                    provider.get_name(),
                    &model_config.model_name,
                ),
            );
        }

        let goose_mode = *self.current_goose_mode.lock().await;

        // Live scheduled-job count for the self-knowledge brief (Queryable). The
        // scheduler is only reachable here (async, with the service handle), not
        // inside the synchronous prompt builder.
        let scheduled_job_count = match self.config.scheduler_service.as_ref() {
            Some(scheduler) => Some(scheduler.list_scheduled_jobs().await.len()),
            None => None,
        };

        // Dispatchable-worker list for the self-knowledge brief, gated on the
        // orchestrator being active. The availability probe may block
        // (`model_loaded:` does HTTP), so it runs off the async runtime.
        let dispatchable_workers = if crate::agents::self_knowledge::orchestrator_dispatch_active()
        {
            tokio::task::spawn_blocking(|| {
                let config = crate::config::agent_identity::load_agent_config();
                crate::agents::self_knowledge::dispatchable_workers_from_config(&config)
            })
            .await
            .unwrap_or_default()
        } else {
            Vec::new()
        };

        // Unread briefings from the worker agents. NOT gated on the
        // orchestrator: the agents that file these (Steward, Watcher) run on
        // their own schedules whether or not orchestration is enabled, so
        // gating this would silently mute them on the default profile — which
        // is exactly the profile this machine runs.
        //
        // Capped: the brief rides in every turn's system prompt, so an
        // unbounded list would grow the prompt without bound. `unacknowledged`
        // orders by severity first, so the cap drops routine chatter rather
        // than the thing waiting on a decision.
        const MAX_BRIEFINGS_IN_PROMPT: i64 = 5;
        let agent_briefings = match self.config.session_manager.pool_clone().await {
            Ok(pool) => {
                let unread = crate::briefings::unacknowledged(&pool, MAX_BRIEFINGS_IN_PROMPT).await;

                // Acknowledge the FYI ones now that they have been put in front
                // of Henry — otherwise they ride in every subsequent prompt
                // forever and crowd out newer reports.
                //
                // Attention/ActionRequired deliberately survive: those are
                // waiting on something (usually a human decision on a card),
                // and dropping one because Henry happened to see it once is how
                // a pending force-push quietly stops being mentioned. They
                // clear when the underlying thing resolves — see the resolve
                // path in `crate::briefings`.
                let fyi: Vec<String> = unread
                    .iter()
                    .filter(|b| b.severity == crate::briefings::Severity::Info)
                    .map(|b| b.id.clone())
                    .collect();
                if !fyi.is_empty() {
                    let _ = crate::briefings::acknowledge(&pool, &fyi).await;
                }

                Some(
                    unread
                        .into_iter()
                        .map(|b| crate::agents::self_knowledge::BriefingLine {
                            from: crate::briefings::display_name_for(&b.from_agent),
                            severity: b.severity.render().to_string(),
                            summary: b.summary,
                        })
                        .collect(),
                )
            }
            // No pool (tests, degraded boot). `None`, NOT an empty list — the
            // brief must omit the section rather than tell Henry his agents
            // have nothing pending on the strength of a read that never ran.
            Err(_) => None,
        };

        let prompt_manager = self.prompt_manager.lock().await;
        let mut system_prompt = prompt_manager
            .builder()
            .with_extensions(extensions_info.into_iter())
            .with_frontend_instructions(self.frontend_instructions.lock().await.clone())
            .with_extension_and_tool_counts(extension_count, tool_count)
            .with_code_execution_mode(code_execution_active)
            .with_hints(working_dir)
            .with_goose_mode(goose_mode)
            .with_scheduled_job_count(scheduled_job_count)
            .with_dispatchable_workers(dispatchable_workers)
            .with_agent_briefings(agent_briefings)
            .with_declared_extensions(declared_extensions)
            // Tool-calling discipline is a per-family concern, so the family
            // that will actually answer picks its own short overlay rather than
            // every model paying for the weakest reader's patches.
            .with_model_family_from(provider.get_name(), &model_config.model_name)
            .build_parts();

        // Handle toolshim if enabled
        let mut toolshim_tools = vec![];
        if model_config.toolshim {
            // If tool interpretation is enabled, modify the system prompt. The
            // tool-JSON instructions are as stable as the tool list itself, so
            // they belong in the cached prefix — `map_stable` keeps the volatile
            // tail behind them rather than folding it in.
            system_prompt =
                system_prompt.map_stable(|s| modify_system_prompt_for_tool_json(&s, &tools));
            // Make a copy of tools before emptying
            toolshim_tools = tools.clone();
            // Empty the tools vector for provider completion
            tools = vec![];
        }

        Ok((tools, toolshim_tools, system_prompt))
    }

    #[tracing::instrument(
        skip(provider, session_id, system_prompt, messages, tools, toolshim_tools),
        fields(session.id = %session_id)
    )]
    pub(crate) async fn stream_response_from_provider(
        provider: Arc<dyn Provider>,
        session_id: &str,
        system_prompt: &SystemPromptParts,
        messages: &[Message],
        tools: &[Tool],
        toolshim_tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        let config = provider.get_model_config();

        let filtered_messages: Vec<Message> = messages
            .iter()
            .filter(|m| m.is_agent_visible())
            .map(|m| m.agent_visible_content())
            .collect();

        // Convert tool messages to text if toolshim is enabled
        let messages_for_provider = if config.toolshim {
            convert_tool_messages_to_text(&filtered_messages)
        } else {
            Conversation::new_unvalidated(filtered_messages)
        };

        // Clone owned data to move into the async stream
        let system_prompt = system_prompt.to_owned();
        let tools = tools.to_owned();
        let toolshim_tools = toolshim_tools.to_owned();
        let provider = provider.clone();

        // Capture errors during stream creation and return them as part of the stream
        // so they can be handled by the existing error handling logic in the agent
        let model_config = provider.get_model_config();
        debug!("WAITING_LLM_STREAM_START");
        let stream_result = provider
            // permagent-dispatch: seam=agent_primary_stream_transport_v1 class=excluded reason=caller_reservation_settlement authority=agent_primary_stream
            .stream_split(
                &model_config,
                session_id,
                &system_prompt,
                messages_for_provider.messages(),
                &tools,
            )
            .await;
        debug!("WAITING_LLM_STREAM_END");

        // If there was an error creating the stream, return a stream that yields that error
        let mut stream = match stream_result {
            Ok(s) => s,
            Err(e) => {
                let enhanced_error = enhance_model_error(e, &provider).await;
                // Return a stream that immediately yields the error
                // This allows the error to be caught by existing error handling in agent.rs
                return Ok(Box::pin(try_stream! {
                    yield Err(enhanced_error)?;
                }));
            }
        };

        Ok(Box::pin(try_stream! {
            while let Some(result) = stream.next().await {
                let (mut message, usage) = result?;

                // Store the model information in the global store
                if let Some(usage) = usage.as_ref() {
                    crate::providers::base::set_current_model(&usage.model);
                }

                // Post-process / structure the response only if tool interpretation is enabled
                if message.is_some() && config.toolshim {
                    message = Some(toolshim_postprocess(message.unwrap(), &toolshim_tools).await?);
                }

                yield (message, usage);
            }
        }))
    }

    /// Categorize tool requests from the response into different types
    /// Returns:
    /// - frontend_requests: Tool requests that should be handled by the frontend
    /// - other_requests: All other tool requests (including requests to enable extensions)
    /// - filtered_message: The original message with frontend tool requests removed
    pub(crate) async fn categorize_tool_requests(
        &self,
        response: &Message,
        tools: &[Tool],
        suppress_replayed_thinking: bool,
    ) -> (Vec<ToolRequest>, Vec<ToolRequest>, Message) {
        // First collect all tool requests with coercion applied
        let tool_requests: Vec<ToolRequest> = response
            .content
            .iter()
            .filter_map(|content| {
                if let MessageContent::ToolRequest(req) = content {
                    let mut coerced_req = req.clone();

                    if let Ok(ref mut tool_call) = coerced_req.tool_call {
                        if let Some(tool) = tools.iter().find(|t| t.name == tool_call.name) {
                            let schema_value = Value::Object(tool.input_schema.as_ref().clone());
                            tool_call.arguments =
                                coerce_tool_arguments(tool_call.arguments.clone(), &schema_value);

                            if let Some(ref meta) = tool.meta {
                                // Merge registry meta into existing tool_meta;
                                // existing keys win so provider markers (e.g.
                                // goose.external_dispatch) survive coercion.
                                let new_meta = serde_json::to_value(meta).ok();
                                coerced_req.tool_meta =
                                    match (coerced_req.tool_meta.take(), new_meta) {
                                        (
                                            Some(Value::Object(mut existing)),
                                            Some(Value::Object(new)),
                                        ) => {
                                            for (k, v) in new {
                                                existing.entry(k).or_insert(v);
                                            }
                                            Some(Value::Object(existing))
                                        }
                                        (None, new) => new,
                                        (existing, _) => existing,
                                    };
                            }
                        }
                    }

                    Some(coerced_req)
                } else {
                    None
                }
            })
            .collect();

        let has_tool_requests = !tool_requests.is_empty();
        let should_suppress_replayed_thinking = suppress_replayed_thinking && has_tool_requests;

        // Create a filtered message with frontend tool requests removed.
        // When a response contains tool calls, keep reasoning in the original
        // message for provider/state purposes but only suppress it from the
        // user-visible filtered message if the caller already surfaced
        // thinking earlier in this provider turn. That avoids replaying full
        // accumulated reasoning after streamed thought chunks while still
        // preserving final-only non-streaming thoughts.
        let mut filtered_content = Vec::new();
        let mut tool_request_index = 0;

        for content in &response.content {
            match content {
                MessageContent::ToolRequest(_) => {
                    if tool_request_index < tool_requests.len() {
                        let coerced_req = &tool_requests[tool_request_index];
                        tool_request_index += 1;

                        // Always keep externally-dispatched requests visible, even if
                        // their name happens to overlap a registered frontend tool —
                        // they're observation-only and must not be removed from history.
                        let should_include = if coerced_req.is_externally_dispatched() {
                            true
                        } else if let Ok(tool_call) = &coerced_req.tool_call {
                            !self.is_frontend_tool(&tool_call.name).await
                        } else {
                            true
                        };

                        if should_include {
                            filtered_content.push(MessageContent::ToolRequest(coerced_req.clone()));
                        }
                    }
                }
                MessageContent::Thinking(_) | MessageContent::RedactedThinking(_)
                    if should_suppress_replayed_thinking => {}
                _ => {
                    filtered_content.push(content.clone());
                }
            }
        }

        let mut filtered_message =
            Message::new(response.role.clone(), response.created, filtered_content);

        // Preserve the ID if it exists
        if let Some(id) = response.id.clone() {
            filtered_message = filtered_message.with_id(id);
        }

        // Categorize tool requests
        let mut frontend_requests = Vec::new();
        let mut other_requests = Vec::new();

        for request in tool_requests {
            // Skip externally-dispatched requests (e.g. claude-acp); the
            // provider already executed the tool. Stays in filtered_message.
            if request.is_externally_dispatched() {
                continue;
            }
            if let Ok(tool_call) = &request.tool_call {
                if self.is_frontend_tool(&tool_call.name).await {
                    frontend_requests.push(request);
                } else {
                    other_requests.push(request);
                }
            } else {
                // If there's an error in the tool call, add it to other_requests
                other_requests.push(request);
            }
        }

        (frontend_requests, other_requests, filtered_message)
    }

    pub(crate) async fn update_session_metrics_for_invocation(
        &self,
        session_id: &str,
        schedule_id: Option<String>,
        usage: &ProviderUsage,
        invocation: &ProviderInvocationContext,
        recognition_retrieval_id: Option<&str>,
    ) -> Result<()> {
        record_provider_usage(
            Arc::clone(&self.config.session_manager),
            session_id,
            schedule_id,
            usage,
            false,
            Some(invocation),
            recognition_retrieval_id,
        )
        .await
    }
}

/// Append one provider response and all derived token/cost rollups.  This is
/// deliberately independent from `Agent` so background context maintenance
/// can use the same single Spectral transaction instead of creating a side
/// ledger or a best-effort metric path.
pub(crate) async fn record_provider_usage(
    manager: Arc<SessionManager>,
    session_id: &str,
    schedule_id: Option<String>,
    usage: &ProviderUsage,
    is_compaction_usage: bool,
    invocation: Option<&ProviderInvocationContext>,
    recognition_retrieval_id: Option<&str>,
) -> Result<()> {
    let session = manager.get_session(session_id, false).await?;

    // Accumulate in the DATABASE, not here.
    //
    // This used to read `session.accumulated_*`, add the new usage in Rust,
    // and write the absolute result. Two turns sharing a session — a
    // subagent alongside the main loop, or concurrent tool calls — both
    // read the same starting value and both write their own total, so the
    // second commit silently discards the first's tokens. That undercounts
    // spend, and the accumulated figures feed the spend caps.
    let (delta_total, delta_input, delta_output) = (
        usage.usage.total_tokens.unwrap_or(0),
        usage.usage.input_tokens.unwrap_or(0),
        usage.usage.output_tokens.unwrap_or(0),
    );

    let (current_total, current_input, current_output) = if is_compaction_usage {
        // After compaction: summary output becomes new input context
        let new_input = usage.usage.output_tokens;
        (new_input, new_input, None)
    } else {
        (
            usage.usage.total_tokens,
            usage.usage.input_tokens,
            usage.usage.output_tokens,
        )
    };

    // ── Per-call cost ledger (single source of truth for spend/attribution) ─
    // One row per provider response. Every money field is folded by the ONE
    // canonical `cost_of` (via `cost_breakdown`), so the ledger, the live
    // meter, and the verification digest can never disagree. For an
    // invocation-authorized call this write is fail-closed: accounting
    // failure aborts the turn instead of silently losing spend.
    let provider = invocation
        .map(|call| Some(call.provider.clone()))
        .unwrap_or_else(|| session.provider_name.clone());
    let model = invocation
        .map(|call| call.model.clone())
        .unwrap_or_else(|| usage.model.clone());
    // `maybe_get_pricing`, not `maybe_get_canonical_model(..).cost`: the
    // generated registry has no ROW at all for a newly-selectable model, so
    // the canonical lookup returns `None` and there is nothing to read a
    // price off. That is how 128 `deepseek-v4-flash` calls billed as $0.00
    // on 2026-08-23 — see `providers::canonical::published_prices`.
    let pricing = provider
        .as_deref()
        .and_then(|p| maybe_get_pricing(p, &model));
    let breakdown = pricing
        .as_ref()
        .and_then(|p| cost_breakdown(&usage.usage, p));

    // Ollama et al. run locally — not chargeable. Subscription/quota detection
    // is not yet wired (no per-provider plan signal here), so a priced remote
    // provider records as `paid_api`.
    let is_local = provider.as_deref().is_some_and(is_local_provider);
    let cost_tier = invocation.map(|call| call.cost_tier).unwrap_or_else(|| {
        if is_local {
            CostTier::LocalFree
        } else {
            CostTier::PaidApi
        }
    });

    // Not chargeable → cost 0 (never bill local/in-quota). Chargeable with a
    // known price → the folded `cost_of` total.
    //
    // Chargeable but UNPRICED → billed at the worst case we know of for that
    // provider, still flagged `is_estimated`. It used to record $0.00, and
    // because nothing downstream read the flag, the budget ceilings summed
    // 211 real calls as zero and could never fire (2026-08-24 health review).
    // An unknown price now fails CLOSED: an unmeasurable call can only make
    // the cap fire early, never late. `is_estimated` still marks it, so the
    // figure is never mistaken for a measured cost.
    let estimated_breakdown = if cost_tier.is_chargeable() && breakdown.is_none() {
        provider
            .as_deref()
            .and_then(worst_case_pricing)
            .and_then(|p| cost_breakdown(&usage.usage, &p))
    } else {
        None
    };
    if let Some(est) = &estimated_breakdown {
        tracing::warn!(
            model = %model,
            provider = provider.as_deref().unwrap_or("unknown"),
            estimated_cost_usd = est.total_cost,
            "no published price for this model — billing it at the provider's most \
             expensive known rate so the spend cap still applies"
        );
    }

    let (cost_usd, input_cost, output_cost, cache_read_cost, cache_write_cost, is_estimated) =
        match (cost_tier.is_chargeable(), &breakdown, &estimated_breakdown) {
            (false, _, _) => (0.0, 0.0, 0.0, 0.0, 0.0, false),
            (true, Some(b), _) => (
                b.total_cost,
                b.input_cost,
                b.output_cost,
                b.cache_read_cost,
                b.cache_write_cost,
                false,
            ),
            (true, None, Some(b)) => (
                b.total_cost,
                b.input_cost,
                b.output_cost,
                b.cache_read_cost,
                b.cache_write_cost,
                true,
            ),
            // Nothing priced anywhere in the registry — keep $0 rather than
            // inventing a number, and let `is_estimated` carry the warning.
            (true, None, None) => (0.0, 0.0, 0.0, 0.0, 0.0, true),
        };
    let cache_savings_usd = if cost_tier.is_chargeable() {
        pricing
            .as_ref()
            .map(|p| cache_savings_of(&usage.usage, p))
            .unwrap_or(0.0)
    } else {
        0.0
    };

    // Surface the prompt-cache hit rate for this response — reads /
    // (reads + creation), the day-one measurable signal for the cache
    // discipline (#717/#727). Emitted per call alongside the ledger's cost so
    // it is observable without a schema change; `None` (no log) when nothing
    // went through the cache this call.
    if let Some(cache_hit_rate) = cache_hit_rate_of(&usage.usage) {
        tracing::debug!(
            cache_hit_rate,
            cache_savings_usd,
            cost_usd,
            model = %model,
            "prompt-cache hit rate for provider response"
        );
    }

    // Interactive surfaces are User/Terminal; everything else (SubAgent,
    // Scheduled, Hidden, Gateway, Acp) is background/headless. One
    // definition, on the type itself.
    let is_headless = !session.session_type.is_interactive();

    let tok = |t: Option<i32>| t.unwrap_or(0).max(0) as i64;
    // `subagent_id`: a SubAgent session IS the subagent — its own id is the
    // identity a "cost run inside a subagent" query needs.
    // `parent_session_id` comes from the session row (set at spawn via
    // `create_session_with_parent`); ledger rows copy it so parent rollups
    // do not need a join back to `sessions`.
    let subagent_id =
        matches!(session.session_type, SessionType::SubAgent).then(|| session.id.clone());
    let row = CostLedgerRow {
        // ProviderUsage carries the invocation identity assigned by the
        // Agent. A missing identity is retained for legacy/manual callers;
        // it gets a fresh key and therefore cannot claim replay safety.
        call_id: invocation
            .map(|call| call.invocation_id.clone())
            .or_else(|| usage.invocation_id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        ts: chrono::Utc::now().to_rfc3339(),
        session_id: session_id.to_string(),
        parent_session_id: session.parent_session_id.clone(),
        task_id: budget_task_id(&session.extension_data),
        goal_id: goal_id(&session.extension_data),
        subagent_id,
        provider,
        model: Some(model),
        cost_tier,
        is_headless,
        input_tokens: tok(usage.usage.input_tokens),
        output_tokens: tok(usage.usage.output_tokens),
        cache_read_tokens: tok(usage.usage.cache_read_input_tokens),
        cache_write_tokens: tok(usage.usage.cache_write_input_tokens),
        input_cost,
        output_cost,
        cache_read_cost,
        cache_write_cost,
        cost_usd,
        cache_savings_usd,
        is_estimated,
    };
    // Token and money accounting are one durable operation. An unavailable
    // ledger must stop the turn rather than allow paid work to continue
    // with silently missing spend; duplicate invocation keys are handled as
    // an explicit successful no-op by the storage layer.
    if let Some(invocation) = invocation.filter(|call| call.cost_tier.is_chargeable()) {
        let reservation_id = invocation.reservation_id.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "paid provider invocation has no reservation; refusing to settle silently"
            )
        })?;
        manager
            .settle_provider_invocation(
                reservation_id,
                &row,
                schedule_id,
                current_total,
                current_input,
                current_output,
                delta_total,
                delta_input,
                delta_output,
            )
            .await?;
    } else {
        manager
            .append_usage_and_rollup(
                &row,
                schedule_id,
                current_total,
                current_input,
                current_output,
                delta_total,
                delta_input,
                delta_output,
            )
            .await?;
    }

    // Provider identity is copied by reference only after the authoritative
    // cost row is durable. The recognition writer resolves that call in
    // cost_ledger and enforces the recall/session match; attribution gaps do
    // not fail the ordinary reply.
    if let Some(retrieval_id) = recognition_retrieval_id {
        if let Some(invocation_id) = invocation
            .map(|call| call.invocation_id.as_str())
            .or(usage.invocation_id.as_deref())
        {
            match manager.storage().pool().await {
                Ok(pool) => match tokio::time::timeout(
                    std::time::Duration::from_millis(250),
                    crate::recognition::record_provider_invocation_reference(
                        pool,
                        retrieval_id,
                        session_id,
                        invocation_id,
                    ),
                )
                .await
                {
                    Ok(Ok(crate::recognition::ProviderAttributionWrite::Recorded))
                    | Ok(Ok(crate::recognition::ProviderAttributionWrite::Duplicate)) => {}
                    Ok(Ok(status)) => debug!(
                        target: "permagent::recognition",
                        retrieval_id = %retrieval_id,
                        ?status,
                        "provider attribution remains partial or unavailable"
                    ),
                    Ok(Err(error)) => debug!(
                        target: "permagent::recognition",
                        retrieval_id = %retrieval_id,
                        ?error,
                        "provider attribution write failed open"
                    ),
                    Err(_) => debug!(
                        target: "permagent::recognition",
                        retrieval_id = %retrieval_id,
                        "provider attribution write timed out; settlement remains durable"
                    ),
                },
                Err(error) => debug!(
                    target: "permagent::recognition",
                    retrieval_id = %retrieval_id,
                    ?error,
                    "provider attribution pool unavailable"
                ),
            }
        }
    }

    Ok(())
}

/// Reserve exactly one provider invocation using immutable transport/model
/// facts captured before dispatch.  This is shared by streamed turns and the
/// explicit physical attempts behind `complete_fast`; neither path may infer
/// a free tier from a provider name or fabricate a task identity.
pub(crate) async fn reserve_provider_invocation_for_model(
    manager: Arc<SessionManager>,
    session: &Session,
    provider: &str,
    cost_tier: CostTier,
    model_config: crate::model::ModelConfig,
    max_physical_attempts: u32,
    invocation_id: String,
) -> std::result::Result<ProviderInvocationContext, ProviderAuthorizationFailure> {
    let bound = crate::cost_router::plan_reservation_bound(
        provider,
        &model_config.model_name,
        cost_tier,
        &model_config,
        max_physical_attempts,
    )
    .map_err(|e| ProviderAuthorizationFailure::Unknown {
        reason: format!("cannot authorize provider call: {e}"),
    })?;

    let mut context = ProviderInvocationContext {
        invocation_id,
        provider: provider.to_string(),
        model: model_config.model_name,
        cost_tier,
        reservation_id: None,
    };

    // Local and subscription calls retain typed attribution, but never create
    // a paid-dollar hold.
    let Some(bound) = bound else {
        return Ok(context);
    };
    let task_id = budget_task_id(&session.extension_data);
    let lease_until = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
    let outcome = manager
        .reserve_provider_invocation(
            &context.invocation_id,
            &session.id,
            task_id.as_deref(),
            bound.amount_usd,
            &lease_until,
            &crate::cost_router::budget::load_budget_config(),
        )
        .await
        .map_err(|e| ProviderAuthorizationFailure::Unknown {
            reason: format!("cannot authorize provider call: {e}"),
        })?;

    match outcome {
        crate::session::CostReservationOutcome::Granted { reservation_id }
        | crate::session::CostReservationOutcome::AlreadyReserved { reservation_id } => {
            context.reservation_id = Some(reservation_id);
            Ok(context)
        }
        crate::session::CostReservationOutcome::AlreadySettled { .. } => {
            Err(ProviderAuthorizationFailure::Unknown {
                reason: "provider invocation identity was already settled; refusing replay"
                    .to_string(),
            })
        }
        crate::session::CostReservationOutcome::NeedsGate {
            scope,
            spent_usd,
            held_usd,
            requested_usd,
            ceiling_usd,
        } => Err(ProviderAuthorizationFailure::NeedsGate {
            scope,
            spent_usd,
            held_usd,
            requested_usd,
            ceiling_usd,
        }),
        crate::session::CostReservationOutcome::Refused {
            scope,
            spent_usd,
            held_usd,
            requested_usd,
            ceiling_usd,
        } => Err(ProviderAuthorizationFailure::Refused {
            scope,
            spent_usd,
            held_usd,
            requested_usd,
            ceiling_usd,
        }),
        crate::session::CostReservationOutcome::Unknown { reason } => {
            Err(ProviderAuthorizationFailure::Unknown { reason })
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderInvocationContext {
    pub invocation_id: String,
    pub provider: String,
    pub model: String,
    pub cost_tier: CostTier,
    pub reservation_id: Option<String>,
}

/// One reserved `complete_fast` attempt. It owns an immutable provider and
/// session snapshot so background context work cannot accidentally inherit a
/// model switch or a new task on the next turn.
#[derive(Clone)]
pub(crate) struct AccountedFastCompletion {
    manager: Arc<SessionManager>,
    session: Session,
    provider: Arc<dyn Provider>,
    is_compaction_usage: bool,
}

impl Agent {
    pub(crate) fn accounted_fast_completion(
        &self,
        session: &Session,
        provider: Arc<dyn Provider>,
        is_compaction_usage: bool,
    ) -> Arc<dyn crate::context_mgmt::AccountedFastCompletion> {
        Arc::new(AccountedFastCompletion {
            manager: Arc::clone(&self.config.session_manager),
            session: session.clone(),
            provider,
            is_compaction_usage,
        })
    }
}

struct FastReservationUnknownGuard {
    manager: Arc<SessionManager>,
    reservation_id: Option<String>,
    armed: bool,
}

impl FastReservationUnknownGuard {
    fn new(manager: Arc<SessionManager>, reservation_id: Option<String>) -> Self {
        Self {
            manager,
            armed: reservation_id.is_some(),
            reservation_id,
        }
    }

    async fn mark_unknown(&mut self) -> Result<()> {
        if let Some(id) = self.reservation_id.as_deref().filter(|_| self.armed) {
            self.manager.mark_provider_invocation_unknown(id).await?;
        }
        self.armed = false;
        Ok(())
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for FastReservationUnknownGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Some(reservation_id) = self.reservation_id.clone() else {
            return;
        };
        let manager = Arc::clone(&self.manager);
        // Cancellation cannot await. Preserve the paid hold as unknown; the
        // durable lease is the crash-safe fallback if the runtime is exiting.
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Err(error) = manager
                    .mark_provider_invocation_unknown(&reservation_id)
                    .await
                {
                    tracing::warn!(
                        reservation_id = %reservation_id,
                        "could not mark cancelled complete_fast reservation unknown: {error}"
                    );
                }
            });
        }
    }
}

impl AccountedFastCompletion {
    /// Resolve the durable hidden session used by background model work. This
    /// keeps internal completions on the same Spectral ledger without inventing
    /// a transcript-side budget or process-local accounting store.
    pub(crate) async fn ensure_background_session(
        manager: Arc<SessionManager>,
        name: &str,
    ) -> Result<Session> {
        if let Some(session) = manager
            .list_sessions_by_types(&[SessionType::Hidden])
            .await?
            .into_iter()
            .find(|session| session.name == name)
        {
            return Ok(session);
        }
        manager
            .create_session(
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
                name.to_string(),
                SessionType::Hidden,
                GooseMode::Auto,
            )
            .await
    }

    /// Run a fast/full completion through the shared reservation and Spectral
    /// settlement seam without requiring an `Agent` instance. Background
    /// orchestrator work uses this entry point so it cannot bypass the same
    /// paid-dispatch boundary as interactive turns.
    pub(crate) async fn complete_fast_accounted(
        manager: Arc<SessionManager>,
        session: Session,
        provider: Arc<dyn Provider>,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
        is_compaction_usage: bool,
    ) -> Result<(Message, ProviderUsage)> {
        let completion = Self {
            manager,
            session,
            provider,
            is_compaction_usage,
        };
        completion
            .complete_fast_inner(system, messages, tools)
            .await
    }

    /// Run one full-model completion through the same reservation and
    /// Spectral settlement boundary. Background callers that intentionally
    /// select the provider's actor/lead model use this instead of bypassing
    /// accounting with `Provider::complete`.
    pub(crate) async fn complete_accounted(
        manager: Arc<SessionManager>,
        session: Session,
        provider: Arc<dyn Provider>,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
        is_compaction_usage: bool,
    ) -> Result<(Message, ProviderUsage)> {
        let model_config = provider.get_model_config();
        let completion = Self {
            manager,
            session,
            provider,
            is_compaction_usage,
        };
        completion
            .complete_one(model_config, system, messages, tools)
            .await
    }

    async fn complete_one(
        &self,
        model_config: crate::model::ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage)> {
        let invocation = reserve_provider_invocation_for_model(
            Arc::clone(&self.manager),
            &self.session,
            self.provider.get_name(),
            self.provider.cost_tier(),
            model_config.clone(),
            self.provider.retry_config().max_physical_attempts(),
            uuid::Uuid::new_v4().to_string(),
        )
        .await
        .map_err(|failure| anyhow::anyhow!(failure.to_string()))?;
        let mut unknown_guard = FastReservationUnknownGuard::new(
            Arc::clone(&self.manager),
            invocation.reservation_id.clone(),
        );

        let (response, mut usage) = match self
            .provider
            // permagent-dispatch: seam=complete_fast_attempt_v1 class=wrapped contract=reservation_settlement
            .complete(&model_config, &self.session.id, system, messages, tools)
            .await
        {
            Ok(result) => result,
            Err(error) => {
                unknown_guard.mark_unknown().await?;
                return Err(anyhow::anyhow!(
                    "complete_fast physical attempt failed: {error}"
                ));
            }
        };

        let has_authoritative_usage = usage.usage.input_tokens.is_some()
            || usage.usage.output_tokens.is_some()
            || usage.usage.total_tokens.is_some()
            || usage.usage.cache_read_input_tokens.is_some()
            || usage.usage.cache_write_input_tokens.is_some();
        if invocation.cost_tier.is_chargeable() && !has_authoritative_usage {
            unknown_guard.mark_unknown().await?;
            return Err(anyhow::anyhow!(
                "complete_fast physical attempt returned no authoritative provider usage"
            ));
        }
        // Free/quota work has no paid hold. Preserve its existing token-meter
        // behavior even when a local adapter omitted a usage frame.
        if !has_authoritative_usage {
            usage
                .ensure_tokens(system, messages, &response, tools)
                .await
                .map_err(|error| anyhow::anyhow!("could not estimate non-paid usage: {error}"))?;
        }
        usage = usage.with_invocation_id(invocation.invocation_id.clone());
        if let Err(error) = record_provider_usage(
            Arc::clone(&self.manager),
            &self.session.id,
            self.session.schedule_id.clone(),
            &usage,
            self.is_compaction_usage,
            Some(&invocation),
            None,
        )
        .await
        {
            unknown_guard.mark_unknown().await?;
            return Err(anyhow::anyhow!(
                "complete_fast usage settlement failed: {error}"
            ));
        }
        unknown_guard.disarm();
        Ok((response, usage))
    }

    async fn complete_fast_inner(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage)> {
        let regular = self.provider.get_model_config();
        let fast = regular.use_fast_model();
        let paid = self.provider.cost_tier().is_chargeable();
        match self
            .complete_one(fast.clone(), system, messages, tools)
            .await
        {
            Ok(result) => Ok(result),
            // A paid fast failure may have reached the remote provider. Its
            // reservation is now unknown, so a regular fallback would be an
            // unauthorized second dispatch. The user can resolve the existing
            // gate/reconciliation before retrying.
            Err(error) if paid || fast.model_name == regular.model_name => Err(error),
            // Local/subscription calls retain the provider's normal fast→full
            // fallback semantics, with a distinct invocation/ledger row.
            Err(_) => self.complete_one(regular, system, messages, tools).await,
        }
    }
}

#[async_trait::async_trait]
impl crate::context_mgmt::AccountedFastCompletion for AccountedFastCompletion {
    async fn complete_fast(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage)> {
        self.complete_fast_inner(system, messages, tools).await
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ProviderAuthorizationFailure {
    NeedsGate {
        scope: crate::cost_router::budget::BudgetScope,
        spent_usd: f64,
        held_usd: f64,
        requested_usd: f64,
        ceiling_usd: f64,
    },
    Refused {
        scope: crate::cost_router::budget::BudgetScope,
        spent_usd: f64,
        held_usd: f64,
        requested_usd: f64,
        ceiling_usd: f64,
    },
    Unknown {
        reason: String,
    },
}

impl std::fmt::Display for ProviderAuthorizationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NeedsGate {
                scope,
                spent_usd,
                held_usd,
                requested_usd,
                ceiling_usd,
            } => write!(
                f,
                "{} budget gate required before provider call (spent ${spent_usd:.2}, authorization holds ${held_usd:.2}, requested bound ${requested_usd:.2}, ceiling ${ceiling_usd:.2})",
                scope.word()
            ),
            Self::Refused {
                scope,
                spent_usd,
                held_usd,
                requested_usd,
                ceiling_usd,
            } => write!(
                f,
                "{} budget refused provider call (spent ${spent_usd:.2}, authorization holds ${held_usd:.2}, requested bound ${requested_usd:.2}, ceiling ${ceiling_usd:.2})",
                scope.word()
            ),
            Self::Unknown { reason } => {
                write!(f, "provider budget is unknown; refusing call: {reason}")
            }
        }
    }
}

fn is_local_provider(provider: &str) -> bool {
    let provider = provider.to_ascii_lowercase();
    provider == "local"
        || provider.contains("ollama")
        || provider == "lmstudio"
        || provider == "llama_swap"
        || provider == "qwen38_split"
}

/// Check whether a tool should be callable by an app based on MCP Apps visibility metadata.
///
/// Per the MCP Apps spec (2026-01-26), if `_meta.ui.visibility` is present and does not
/// include `"app"`, the tool is model-only and must not be callable by app UIs.
/// If the field is absent, the tool defaults to visible to both model and app.
pub fn is_tool_visible_to_app(tool: &Tool) -> bool {
    let Some(meta) = &tool.meta else {
        return true;
    };
    let Some(ui) = meta.0.get("ui") else {
        return true;
    };
    let Some(visibility) = ui.get("visibility") else {
        return true;
    };
    let Some(arr) = visibility.as_array() else {
        return true;
    };
    arr.iter().any(|v| v.as_str() == Some("app"))
}

/// Check whether a tool should be visible to the model based on MCP Apps visibility metadata.
///
/// Per the MCP Apps spec (2026-01-26), tools may declare `_meta.ui.visibility` as an array
/// of `"model"` and/or `"app"`. If the field is absent, the tool defaults to visible to both.
/// If present and does not include `"model"`, the tool is app-only and must not be sent to the LLM.
pub fn is_tool_visible_to_model(tool: &Tool) -> bool {
    let Some(meta) = &tool.meta else {
        return true;
    };
    let Some(ui) = meta.0.get("ui") else {
        return true;
    };
    let Some(visibility) = ui.get("visibility") else {
        return true;
    };
    let Some(arr) = visibility.as_array() else {
        return true;
    };
    arr.iter().any(|v| v.as_str() == Some("model"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GooseMode;
    use crate::conversation::message::Message;
    use crate::model::ModelConfig;
    use crate::providers::base::{Provider, ProviderUsage, Usage};
    use crate::providers::errors::ProviderError;
    use crate::session::session_manager::SessionType;
    use async_trait::async_trait;
    use rmcp::object;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;
    use tempfile::TempDir;

    #[derive(Clone)]
    struct MockProvider {
        model_config: ModelConfig,
    }

    #[async_trait]
    impl Provider for MockProvider {
        fn get_name(&self) -> &str {
            "mock"
        }

        fn get_model_config(&self) -> ModelConfig {
            self.model_config.clone()
        }

        async fn stream(
            &self,
            _model_config: &ModelConfig,
            _session_id: &str,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<MessageStream, ProviderError> {
            let message = Message::assistant().with_text("ok");
            let usage = ProviderUsage::new("mock".to_string(), Usage::default());
            Ok(stream_from_single_message(message, usage))
        }
    }

    /// A mock whose provider name is settable, so a test can actually switch
    /// providers mid-session the way a failover does.
    #[derive(Clone)]
    struct NamedMockProvider {
        name: &'static str,
        model_config: ModelConfig,
    }

    #[derive(Clone)]
    struct FastAccountingProvider {
        usage: Usage,
        fail: bool,
        calls: Arc<AtomicUsize>,
        model_config: ModelConfig,
    }

    impl FastAccountingProvider {
        fn paid(usage: Usage) -> Self {
            Self {
                usage,
                fail: false,
                calls: Arc::new(AtomicUsize::new(0)),
                model_config: ModelConfig::new("claude-haiku-4-5")
                    .unwrap()
                    .with_canonical_limits("anthropic"),
            }
        }
    }

    #[async_trait]
    impl Provider for FastAccountingProvider {
        fn get_name(&self) -> &str {
            "anthropic"
        }

        fn get_model_config(&self) -> ModelConfig {
            self.model_config.clone()
        }

        async fn stream(
            &self,
            _model_config: &ModelConfig,
            _session_id: &str,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<MessageStream, ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(ProviderError::RequestFailed(
                    "fixture dispatch failed".to_string(),
                ));
            }
            Ok(stream_from_single_message(
                Message::assistant().with_text("summary"),
                ProviderUsage::new("claude-haiku-4-5".to_string(), self.usage),
            ))
        }
    }

    /// Records the model selected for every physical attempt. The first fast
    /// attempt fails; a non-chargeable provider is permitted to retry once
    /// with the regular model.
    #[derive(Clone)]
    struct LocalFastFallbackProvider {
        calls: Arc<AtomicUsize>,
        selected_models: Arc<Mutex<Vec<String>>>,
        model_config: ModelConfig,
    }

    impl LocalFastFallbackProvider {
        fn new() -> Self {
            Self {
                calls: Arc::new(AtomicUsize::new(0)),
                selected_models: Arc::new(Mutex::new(Vec::new())),
                model_config: ModelConfig {
                    model_name: "full-local-model".to_string(),
                    context_limit: Some(4_096),
                    max_tokens: Some(128),
                    fast_model_config: Some(Box::new(ModelConfig {
                        model_name: "fast-local-model".to_string(),
                        context_limit: Some(4_096),
                        max_tokens: Some(128),
                        ..ModelConfig::default()
                    })),
                    ..ModelConfig::default()
                },
            }
        }
    }

    #[async_trait]
    impl Provider for LocalFastFallbackProvider {
        fn get_name(&self) -> &str {
            "local-fixture"
        }

        fn cost_tier(&self) -> CostTier {
            CostTier::LocalFree
        }

        fn get_model_config(&self) -> ModelConfig {
            self.model_config.clone()
        }

        async fn stream(
            &self,
            model_config: &ModelConfig,
            _session_id: &str,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<MessageStream, ProviderError> {
            self.selected_models
                .lock()
                .unwrap()
                .push(model_config.model_name.clone());
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(ProviderError::RequestFailed(
                    "fast local fixture dispatch failed".to_string(),
                ));
            }
            Ok(stream_from_single_message(
                Message::assistant().with_text("regular summary"),
                ProviderUsage::new(
                    model_config.model_name.clone(),
                    Usage {
                        input_tokens: Some(8),
                        output_tokens: Some(3),
                        total_tokens: Some(11),
                        ..Usage::default()
                    },
                ),
            ))
        }
    }

    /// A paid attempt which has definitely crossed the dispatch boundary but
    /// never returns. Dropping its future must turn the durable hold unknown.
    #[derive(Clone)]
    struct PendingPaidProvider {
        calls: Arc<AtomicUsize>,
        model_config: ModelConfig,
    }

    #[async_trait]
    impl Provider for PendingPaidProvider {
        fn get_name(&self) -> &str {
            "anthropic"
        }

        fn get_model_config(&self) -> ModelConfig {
            self.model_config.clone()
        }

        async fn stream(
            &self,
            _model_config: &ModelConfig,
            _session_id: &str,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<MessageStream, ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            std::future::pending::<()>().await;
            unreachable!("the cancellation test must abort the pending dispatch")
        }
    }

    async fn fast_accounting_fixture(
        provider: Arc<dyn Provider>,
        begin_task: bool,
    ) -> (
        TempDir,
        Arc<SessionManager>,
        Session,
        AccountedFastCompletion,
    ) {
        let directory = TempDir::new().unwrap();
        let manager = Arc::new(SessionManager::new(directory.path().to_path_buf()));
        let session = manager
            .create_session(
                directory.path().to_path_buf(),
                "fast-accounting".to_string(),
                SessionType::Hidden,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        if begin_task {
            manager.begin_budget_task(&session.id).await.unwrap();
        }
        let session = manager.get_session(&session.id, false).await.unwrap();
        let completion = AccountedFastCompletion {
            manager: Arc::clone(&manager),
            session: session.clone(),
            provider,
            is_compaction_usage: false,
        };
        (directory, manager, session, completion)
    }

    #[tokio::test]
    async fn accounted_fast_success_settles_once_with_a_durable_task() {
        let fixture = FastAccountingProvider::paid(Usage {
            input_tokens: Some(12),
            output_tokens: Some(5),
            total_tokens: Some(17),
            ..Usage::default()
        });
        let calls = Arc::clone(&fixture.calls);
        let (_directory, manager, session, completion) =
            fast_accounting_fixture(Arc::new(fixture), true).await;

        crate::context_mgmt::AccountedFastCompletion::complete_fast(
            &completion,
            "summarize",
            &[Message::user().with_text("hello")],
            &[],
        )
        .await
        .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(manager
            .last_call_facts(&session.id)
            .await
            .unwrap()
            .is_some());
        let pool = manager.storage().pool().await.unwrap();
        let settled: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cost_reservations WHERE session_id = ? AND state = 'settled'",
        )
        .bind(&session.id)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(settled, 1);
    }

    #[tokio::test]
    async fn primary_stream_reservation_refuses_paid_dispatch_without_a_task() {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage::default()));
        let calls = Arc::clone(&fixture.calls);
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(fixture.clone(), false).await;

        let result = reserve_provider_invocation_for_model(
            Arc::clone(&manager),
            &session,
            fixture.get_name(),
            fixture.cost_tier(),
            fixture.get_model_config(),
            fixture.retry_config().max_physical_attempts(),
            uuid::Uuid::new_v4().to_string(),
        )
        .await;

        assert!(matches!(
            result,
            Err(ProviderAuthorizationFailure::Unknown { .. })
        ));
        // Authorization happens before the stream transport is ever called.
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn primary_stream_settles_one_authoritative_snapshot_per_invocation() {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage::default()));
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(fixture.clone(), true).await;
        let invocation = reserve_provider_invocation_for_model(
            Arc::clone(&manager),
            &session,
            fixture.get_name(),
            fixture.cost_tier(),
            fixture.get_model_config(),
            fixture.retry_config().max_physical_attempts(),
            uuid::Uuid::new_v4().to_string(),
        )
        .await
        .expect("durable task should authorize the paid stream");
        let usage = ProviderUsage::new(
            fixture.get_model_config().model_name,
            Usage {
                input_tokens: Some(12),
                output_tokens: Some(5),
                total_tokens: Some(17),
                ..Usage::default()
            },
        )
        .with_invocation_id(invocation.invocation_id.clone());

        let pool = manager.storage().pool().await.unwrap();
        sqlx::query(
            "INSERT INTO recognition_events
                (retrieval_id, session_id, query, retrieved_at, rc_persona, strategy)
             VALUES ('reply-attribution', ?, 'fixture', '2026-09-05T12:00:00Z', 'unknown', 'cascade')",
        )
        .bind(&session.id)
        .execute(pool)
        .await
        .unwrap();

        record_provider_usage(
            Arc::clone(&manager),
            &session.id,
            session.schedule_id.clone(),
            &usage,
            false,
            Some(&invocation),
            Some("reply-attribution"),
        )
        .await
        .expect("authoritative terminal usage should settle the hold");

        let pool = manager.storage().pool().await.unwrap();
        let ledger_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cost_ledger WHERE session_id = ? AND call_id = ?",
        )
        .bind(&session.id)
        .bind(&invocation.invocation_id)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(ledger_rows, 1);
        let settled: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cost_reservations WHERE session_id = ? AND reservation_id = ? AND state = 'settled'",
        )
        .bind(&session.id)
        .bind(invocation.reservation_id.as_deref().unwrap())
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(settled, 1);
        let (ids, status): (String, String) = sqlx::query_as(
            "SELECT provider_invocation_ids, attribution_status
               FROM recognition_events WHERE retrieval_id = 'reply-attribution'",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(ids, format!("[\"{}\"]", invocation.invocation_id));
        assert_eq!(status, "observed");
    }

    #[tokio::test]
    async fn accounted_fast_refuses_paid_dispatch_without_a_durable_task() {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage::default()));
        let calls = Arc::clone(&fixture.calls);
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(fixture.clone(), false).await;

        assert!(AccountedFastCompletion::complete_fast_accounted(
            Arc::clone(&manager),
            session,
            fixture,
            "summarize",
            &[Message::user().with_text("hello")],
            &[],
            false,
        )
        .await
        .is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn accounted_full_refuses_paid_dispatch_without_a_durable_task() {
        let fixture = Arc::new(FastAccountingProvider::paid(Usage::default()));
        let calls = Arc::clone(&fixture.calls);
        let (_directory, manager, session, _completion) =
            fast_accounting_fixture(fixture.clone(), false).await;

        assert!(AccountedFastCompletion::complete_accounted(
            Arc::clone(&manager),
            session,
            fixture,
            "summarize",
            &[Message::user().with_text("hello")],
            &[],
            false,
        )
        .await
        .is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn accounted_fast_missing_usage_keeps_the_paid_hold_unknown() {
        let fixture = FastAccountingProvider::paid(Usage::default());
        let calls = Arc::clone(&fixture.calls);
        let (_directory, manager, session, completion) =
            fast_accounting_fixture(Arc::new(fixture), true).await;

        assert!(crate::context_mgmt::AccountedFastCompletion::complete_fast(
            &completion,
            "summarize",
            &[Message::user().with_text("hello")],
            &[],
        )
        .await
        .is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let pool = manager.storage().pool().await.unwrap();
        let unknown: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cost_reservations WHERE session_id = ? AND state = 'unknown'",
        )
        .bind(&session.id)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(unknown, 1);
    }

    #[tokio::test]
    async fn accounted_fast_paid_failure_does_not_dispatch_the_regular_fallback() {
        let mut fixture = FastAccountingProvider::paid(Usage::default());
        fixture.fail = true;
        fixture.model_config = ModelConfig::new("claude-sonnet-4-5")
            .unwrap()
            .with_canonical_limits("anthropic")
            .with_fast("claude-haiku-4-5", "anthropic")
            .unwrap();
        let calls = Arc::clone(&fixture.calls);
        let (_directory, manager, session, completion) =
            fast_accounting_fixture(Arc::new(fixture), true).await;

        assert!(crate::context_mgmt::AccountedFastCompletion::complete_fast(
            &completion,
            "summarize",
            &[Message::user().with_text("hello")],
            &[],
        )
        .await
        .is_err());

        // A paid fast error may have reached the remote provider. The full
        // model is deliberately not a second unauthorized physical attempt.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let pool = manager.storage().pool().await.unwrap();
        let unknown: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cost_reservations WHERE session_id = ? AND state = 'unknown'",
        )
        .bind(&session.id)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(unknown, 1);
    }

    #[tokio::test]
    async fn accounted_fast_local_failure_retries_the_full_model_as_a_separate_attempt() {
        let fixture = LocalFastFallbackProvider::new();
        let calls = Arc::clone(&fixture.calls);
        let selected_models = Arc::clone(&fixture.selected_models);
        let (_directory, manager, session, completion) =
            fast_accounting_fixture(Arc::new(fixture), false).await;

        crate::context_mgmt::AccountedFastCompletion::complete_fast(
            &completion,
            "summarize",
            &[Message::user().with_text("hello")],
            &[],
        )
        .await
        .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            selected_models.lock().unwrap().as_slice(),
            ["fast-local-model", "full-local-model"]
        );
        let pool = manager.storage().pool().await.unwrap();
        let rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cost_ledger WHERE session_id = ? AND model = 'full-local-model'",
        )
        .bind(&session.id)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(rows, 1);
    }

    #[tokio::test]
    async fn accounted_fast_cancelled_paid_dispatch_keeps_the_hold_unknown() {
        let fixture = PendingPaidProvider {
            calls: Arc::new(AtomicUsize::new(0)),
            model_config: ModelConfig::new("claude-haiku-4-5")
                .unwrap()
                .with_canonical_limits("anthropic"),
        };
        let calls = Arc::clone(&fixture.calls);
        let (_directory, manager, session, completion) =
            fast_accounting_fixture(Arc::new(fixture), true).await;
        let completion = Arc::new(completion);
        let task = tokio::spawn({
            let completion = Arc::clone(&completion);
            async move {
                crate::context_mgmt::AccountedFastCompletion::complete_fast(
                    completion.as_ref(),
                    "summarize",
                    &[Message::user().with_text("hello")],
                    &[],
                )
                .await
            }
        });

        tokio::time::timeout(Duration::from_secs(2), async {
            while calls.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("pending provider must reach dispatch before cancellation");
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());

        let pool = manager.storage().pool().await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let unknown: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM cost_reservations WHERE session_id = ? AND state = 'unknown'",
                )
                .bind(&session.id)
                .fetch_one(pool)
                .await
                .unwrap();
                if unknown == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("cancellation guard must mark the paid hold unknown");
    }

    #[async_trait]
    impl Provider for NamedMockProvider {
        fn get_name(&self) -> &str {
            self.name
        }

        fn get_model_config(&self) -> ModelConfig {
            self.model_config.clone()
        }

        async fn stream(
            &self,
            _model_config: &ModelConfig,
            _session_id: &str,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<MessageStream, ProviderError> {
            let message = Message::assistant().with_text("ok");
            let usage = ProviderUsage::new(self.name.to_string(), Usage::default());
            Ok(stream_from_single_message(message, usage))
        }
    }

    /// M3: session 20260831_10 had NO model-identity fact anywhere in its
    /// prompt, so when the harness silently moved it off `qwen38_split` the
    /// model confabulated a third model name entirely. The line must be live
    /// (it follows a switch) and volatile (it must not bust the cached prefix).
    #[tokio::test]
    async fn the_prompt_says_which_model_is_actually_serving() -> anyhow::Result<()> {
        let agent = crate::agents::Agent::new();
        let session = agent
            .config
            .session_manager
            .create_session(
                std::env::current_dir().unwrap(),
                "test-model-identity".to_string(),
                SessionType::Hidden,
                GooseMode::default(),
            )
            .await?;

        agent
            .update_provider(
                std::sync::Arc::new(NamedMockProvider {
                    name: "qwen38_split",
                    model_config: ModelConfig::new("qwen3.8-27b").unwrap(),
                }),
                &session.id,
            )
            .await?;
        let (_, _, parts) = agent
            .prepare_tools_and_prompt(&session.id, session.working_dir.as_path())
            .await?;
        assert!(
            parts
                .volatile_suffix()
                .contains("You are currently served by qwen38_split/qwen3.8-27b."),
            "no live model identity in the volatile tail: {}",
            parts.volatile_suffix()
        );
        assert!(
            !parts
                .stable_prefix()
                .contains("You are currently served by"),
            "a per-turn fact in the cached prefix busts the prompt cache"
        );

        // The failover: same session, different provider AND model.
        agent
            .update_provider(
                std::sync::Arc::new(NamedMockProvider {
                    name: "anthropic",
                    model_config: ModelConfig::new("claude-haiku-4-5").unwrap(),
                }),
                &session.id,
            )
            .await?;
        let (_, _, parts) = agent
            .prepare_tools_and_prompt(&session.id, session.working_dir.as_path())
            .await?;
        assert!(
            parts
                .volatile_suffix()
                .contains("You are currently served by anthropic/claude-haiku-4-5."),
            "identity did not follow the switch: {}",
            parts.volatile_suffix()
        );
        assert!(
            !parts.render().contains("qwen38_split"),
            "the pre-failover identity survived the switch: {}",
            parts.render()
        );
        Ok(())
    }

    #[test]
    fn cache_hit_rate_is_surfaced_from_the_canonical_cost_reexport() {
        // The per-call cost path (`update_session_metrics`) surfaces the
        // prompt-cache hit rate via `crate::providers::canonical::cache_hit_rate_of`,
        // re-exported from the canonical cost ledger. Exercise that exact surface.
        // 3 of 4 cacheable tokens served from cache reads → 0.75 hit rate.
        let usage = Usage::default().with_cache_tokens(Some(3), Some(1));
        assert_eq!(cache_hit_rate_of(&usage), Some(0.75));
        // A fully warm prefix → 1.0; no cache activity → None (the log is skipped).
        assert_eq!(
            cache_hit_rate_of(&Usage::default().with_cache_tokens(Some(4), Some(0))),
            Some(1.0)
        );
        assert_eq!(cache_hit_rate_of(&Usage::default()), None);
    }

    #[test]
    fn local_cost_tier_recognizes_both_local_provider_engines() {
        assert!(is_local_provider("local"));
        assert!(is_local_provider("ollama"));
        assert!(is_local_provider("remote-ollama"));
        assert!(is_local_provider("qwen38_split"));
        assert!(!is_local_provider("anthropic"));
    }

    #[tokio::test]
    async fn prepare_tools_returns_sorted_tools_including_frontend() -> anyhow::Result<()> {
        let agent = crate::agents::Agent::new();

        let session = agent
            .config
            .session_manager
            .create_session(
                std::env::current_dir().unwrap(),
                "test-prepare-tools".to_string(),
                SessionType::Hidden,
                GooseMode::default(),
            )
            .await?;

        let model_config = ModelConfig::new("test-model").unwrap();
        let provider = std::sync::Arc::new(MockProvider { model_config });
        agent.update_provider(provider, &session.id).await?;

        // Add unsorted frontend tools
        let frontend_tools = vec![
            Tool::new(
                "frontend__z_tool".to_string(),
                "Z tool".to_string(),
                object!({ "type": "object", "properties": { } }),
            ),
            Tool::new(
                "frontend__a_tool".to_string(),
                "A tool".to_string(),
                object!({ "type": "object", "properties": { } }),
            ),
        ];

        agent
            .add_extension(
                crate::agents::extension::ExtensionConfig::Frontend {
                    name: "frontend".to_string(),
                    description: "desc".to_string(),
                    tools: frontend_tools,
                    instructions: None,
                    bundled: None,
                    available_tools: vec![],
                },
                &session.id,
            )
            .await
            .unwrap();

        let (tools, _toolshim_tools, _system_prompt) = agent
            .prepare_tools_and_prompt(&session.id, session.working_dir.as_path())
            .await?;

        let names: Vec<String> = tools.iter().map(|t| t.name.clone().into_owned()).collect();
        assert!(names.iter().any(|n| n == "frontend__a_tool"));
        assert!(names.iter().any(|n| n == "frontend__z_tool"));

        // Verify the names are sorted ascending
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);

        Ok(())
    }

    #[tokio::test]
    async fn test_stream_error_propagation() {
        use futures::StreamExt;

        type StreamItem = Result<(Option<Message>, Option<ProviderUsage>), ProviderError>;
        let stream = futures::stream::iter(vec![
            Ok((Some(Message::assistant().with_text("chunk1")), None)),
            Ok((Some(Message::assistant().with_text("chunk2")), None)),
            Err(ProviderError::RequestFailed(
                "simulated stream error".to_string(),
            )),
        ] as Vec<StreamItem>);

        let mut pinned = Box::pin(stream);
        let mut results = Vec::new();
        let mut error_seen = false;

        while let Some(result) = pinned.next().await {
            match result {
                Ok((message, _usage)) => {
                    if let Some(msg) = message {
                        results.push(msg.as_concat_text());
                    }
                }
                Err(_e) => {
                    error_seen = true;
                    break;
                }
            }
        }

        assert_eq!(results.len(), 2);
        assert_eq!(results[0], "chunk1");
        assert_eq!(results[1], "chunk2");
        assert!(
            error_seen,
            "Error should have been propagated, not silently ignored"
        );
    }

    #[tokio::test]
    async fn categorize_tool_requests_keeps_thinking_when_not_previously_streamed() {
        let agent = crate::agents::Agent::new();
        let response = Message::assistant()
            .with_thinking("final-only reasoning", "")
            .with_tool_request(
                "tool-1",
                Ok(rmcp::model::CallToolRequestParams::new("test_tool")),
            );

        let (_frontend_requests, other_requests, filtered_message) =
            agent.categorize_tool_requests(&response, &[], false).await;

        assert_eq!(other_requests.len(), 1);
        assert_eq!(filtered_message.content.len(), 2);
        assert!(matches!(
            filtered_message.content[0],
            MessageContent::Thinking(_)
        ));
        assert!(matches!(
            filtered_message.content[1],
            MessageContent::ToolRequest(_)
        ));
    }

    #[tokio::test]
    async fn categorize_tool_requests_drops_replayed_thinking_after_streaming() {
        let agent = crate::agents::Agent::new();
        let response = Message::assistant()
            .with_thinking("replayed reasoning", "")
            .with_tool_request(
                "tool-1",
                Ok(rmcp::model::CallToolRequestParams::new("test_tool")),
            );

        let (_frontend_requests, other_requests, filtered_message) =
            agent.categorize_tool_requests(&response, &[], true).await;

        assert_eq!(other_requests.len(), 1);
        assert_eq!(filtered_message.content.len(), 1);
        assert!(matches!(
            filtered_message.content[0],
            MessageContent::ToolRequest(_)
        ));
    }

    #[tokio::test]
    async fn categorize_tool_requests_skips_externally_dispatched_and_preserves_marker() {
        // External requests must (1) survive coercion with goose.external_dispatch
        // intact, (2) be excluded from both dispatch buckets, (3) stay in
        // filtered_message.
        use crate::conversation::message::TOOL_META_EXTERNAL_DISPATCH_KEY;

        let agent = crate::agents::Agent::new();

        let registry_tool = Tool::new("test_tool", "a test tool", object!({ "type": "object" }))
            .with_meta(rmcp::model::Meta(
                serde_json::json!({ "ui": { "visibility": ["model"] } })
                    .as_object()
                    .unwrap()
                    .clone(),
            ));

        let response = Message::assistant().with_tool_request_with_metadata(
            "tool-1",
            Ok(rmcp::model::CallToolRequestParams::new("test_tool")),
            None,
            Some(serde_json::json!({ TOOL_META_EXTERNAL_DISPATCH_KEY: true })),
        );

        let (frontend_requests, other_requests, filtered_message) = agent
            .categorize_tool_requests(&response, &[registry_tool], false)
            .await;

        assert!(
            frontend_requests.is_empty(),
            "external request leaked into frontend_requests: {frontend_requests:?}"
        );
        assert!(
            other_requests.is_empty(),
            "external request leaked into other_requests: {other_requests:?}"
        );
        assert_eq!(filtered_message.content.len(), 1);
        let tool_req = match &filtered_message.content[0] {
            MessageContent::ToolRequest(req) => req,
            other => panic!("expected ToolRequest, got {other:?}"),
        };
        assert!(
            tool_req.is_externally_dispatched(),
            "goose.external_dispatch marker was clobbered by coercion; merged tool_meta = {:?}",
            tool_req.tool_meta
        );
        let merged = tool_req
            .tool_meta
            .as_ref()
            .and_then(|v| v.as_object())
            .expect("tool_meta should be an object after merge");
        assert!(
            merged.contains_key("ui"),
            "registry tool meta keys were dropped; merged tool_meta = {merged:?}"
        );
    }

    fn make_tool_with_meta(meta_json: Option<serde_json::Value>) -> Tool {
        let mut tool = Tool::new("test_tool", "a test tool", object!({ "type": "object" }));
        if let Some(v) = meta_json {
            let obj = v.as_object().unwrap().clone();
            tool = tool.with_meta(rmcp::model::Meta(obj));
        }
        tool
    }

    #[test]
    fn test_tool_visible_when_no_meta() {
        let tool = make_tool_with_meta(None);
        assert!(is_tool_visible_to_model(&tool));
    }

    #[test]
    fn test_tool_visible_when_meta_has_no_ui() {
        let tool = make_tool_with_meta(Some(serde_json::json!({"other": "stuff"})));
        assert!(is_tool_visible_to_model(&tool));
    }

    #[test]
    fn test_tool_visible_when_ui_has_no_visibility() {
        let tool = make_tool_with_meta(Some(
            serde_json::json!({"ui": {"resourceUri": "ui://foo/bar"}}),
        ));
        assert!(is_tool_visible_to_model(&tool));
    }

    #[test]
    fn test_tool_visible_when_visibility_includes_model() {
        let tool = make_tool_with_meta(Some(
            serde_json::json!({"ui": {"visibility": ["model", "app"]}}),
        ));
        assert!(is_tool_visible_to_model(&tool));
    }

    #[test]
    fn test_tool_visible_when_visibility_is_model_only() {
        let tool = make_tool_with_meta(Some(serde_json::json!({"ui": {"visibility": ["model"]}})));
        assert!(is_tool_visible_to_model(&tool));
    }

    #[test]
    fn test_tool_hidden_when_visibility_is_app_only() {
        let tool = make_tool_with_meta(Some(serde_json::json!({"ui": {"visibility": ["app"]}})));
        assert!(!is_tool_visible_to_model(&tool));
    }

    #[test]
    fn test_tool_hidden_when_visibility_is_empty() {
        let tool = make_tool_with_meta(Some(serde_json::json!({"ui": {"visibility": []}})));
        assert!(!is_tool_visible_to_model(&tool));
    }

    #[test]
    fn test_tool_visible_when_visibility_is_not_array() {
        let tool = make_tool_with_meta(Some(serde_json::json!({"ui": {"visibility": "model"}})));
        assert!(is_tool_visible_to_model(&tool));
    }

    #[test]
    fn test_app_visible_when_no_meta() {
        let tool = make_tool_with_meta(None);
        assert!(is_tool_visible_to_app(&tool));
    }

    #[test]
    fn test_app_visible_when_visibility_includes_app() {
        let tool = make_tool_with_meta(Some(
            serde_json::json!({"ui": {"visibility": ["model", "app"]}}),
        ));
        assert!(is_tool_visible_to_app(&tool));
    }

    #[test]
    fn test_app_visible_when_visibility_is_app_only() {
        let tool = make_tool_with_meta(Some(serde_json::json!({"ui": {"visibility": ["app"]}})));
        assert!(is_tool_visible_to_app(&tool));
    }

    #[test]
    fn test_app_hidden_when_visibility_is_model_only() {
        let tool = make_tool_with_meta(Some(serde_json::json!({"ui": {"visibility": ["model"]}})));
        assert!(!is_tool_visible_to_app(&tool));
    }

    #[test]
    fn test_app_hidden_when_visibility_is_empty() {
        let tool = make_tool_with_meta(Some(serde_json::json!({"ui": {"visibility": []}})));
        assert!(!is_tool_visible_to_app(&tool));
    }
}
