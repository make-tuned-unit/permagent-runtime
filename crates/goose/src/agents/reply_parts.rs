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
use crate::conversation::message::{Message, MessageContent, ToolRequest};
use crate::conversation::Conversation;
use crate::cost_router::cache::SystemPromptParts;
#[cfg(test)]
use crate::providers::base::stream_from_single_message;
use crate::providers::base::{MessageStream, Provider, ProviderUsage};
use crate::providers::canonical::{
    cache_hit_rate_of, cache_savings_of, cost_breakdown, maybe_get_canonical_model,
};
use crate::providers::errors::ProviderError;
use crate::providers::toolshim::{
    augment_message_with_tool_calls, convert_tool_messages_to_text,
    modify_system_prompt_for_tool_json, OllamaInterpreter,
};
use crate::session::{CostLedgerRow, CostTier, SessionType};
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
                            return try_coerce_number(s)
                        }
                        "boolean" if matches!(s.to_lowercase().as_str(), "true" | "false") => {
                            return try_coerce_boolean(s)
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
        let extensions_info = self
            .extension_manager
            .get_extensions_info(working_dir)
            .await;
        let (extension_count, tool_count) = self
            .extension_manager
            .get_extension_and_tool_counts(session_id)
            .await;

        // Get model name from provider
        let provider = self.provider().await?;
        let model_config = provider.get_model_config();

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

    pub(crate) async fn update_session_metrics(
        &self,
        session_id: &str,
        schedule_id: Option<String>,
        usage: &ProviderUsage,
        is_compaction_usage: bool,
    ) -> Result<()> {
        let manager = self.config.session_manager.clone();
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

        manager
            .update(session_id)
            .schedule_id(schedule_id)
            .total_tokens(current_total)
            .input_tokens(current_input)
            .output_tokens(current_output)
            .accumulate_tokens(delta_total, delta_input, delta_output)
            .apply()
            .await?;

        // ── Per-call cost ledger (single source of truth for spend/attribution) ─
        // One row per provider response. Every money field is folded by the ONE
        // canonical `cost_of` (via `cost_breakdown`), so the ledger, the live
        // meter, and the verification digest can never disagree. Best-effort: a
        // ledger failure must never abort the agent turn — log and move on.
        let provider = session.provider_name.clone();
        let model = usage.model.clone();
        let pricing = provider
            .as_deref()
            .and_then(|p| maybe_get_canonical_model(p, &model))
            .map(|m| m.cost);
        let breakdown = pricing
            .as_ref()
            .and_then(|p| cost_breakdown(&usage.usage, p));

        // Ollama et al. run locally — not chargeable. Subscription/quota detection
        // is not yet wired (no per-provider plan signal here), so a priced remote
        // provider records as `paid_api`.
        let is_local = provider.as_deref().is_some_and(is_local_provider);
        let cost_tier = if is_local {
            CostTier::LocalFree
        } else {
            CostTier::PaidApi
        };

        // Not chargeable → cost 0 (never bill local/in-quota). Chargeable with a
        // known price → the folded `cost_of` total. Chargeable but unpriced → 0,
        // flagged `is_estimated` so it is never mistaken for a real $0.
        let (cost_usd, input_cost, output_cost, cache_read_cost, cache_write_cost, is_estimated) =
            match (cost_tier.is_chargeable(), &breakdown) {
                (false, _) => (0.0, 0.0, 0.0, 0.0, 0.0, false),
                (true, Some(b)) => (
                    b.total_cost,
                    b.input_cost,
                    b.output_cost,
                    b.cache_read_cost,
                    b.cache_write_cost,
                    false,
                ),
                (true, None) => (0.0, 0.0, 0.0, 0.0, 0.0, true),
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
        // Scheduled, Hidden, Gateway, Acp) is background/headless.
        let is_headless = !matches!(
            session.session_type,
            SessionType::User | SessionType::Terminal
        );

        let tok = |t: Option<i32>| t.unwrap_or(0).max(0) as i64;
        let row = CostLedgerRow {
            call_id: uuid::Uuid::new_v4().to_string(),
            ts: chrono::Utc::now().to_rfc3339(),
            session_id: session_id.to_string(),
            // Deeper attribution keys are columns awaiting their wiring seam
            // (goal/task association lives in card metadata, not threaded here).
            parent_session_id: None,
            task_id: None,
            goal_id: None,
            subagent_id: None,
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
        if let Err(e) = manager.append_cost_ledger(&row).await {
            tracing::warn!(
                "cost_ledger append failed for session {}: {}",
                session_id,
                e
            );
        }

        Ok(())
    }
}

fn is_local_provider(provider: &str) -> bool {
    let provider = provider.to_ascii_lowercase();
    provider == "local" || provider.contains("ollama")
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
