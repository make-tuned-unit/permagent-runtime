use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use crate::config::get_extension_by_name;
use anyhow::Result;
use async_trait::async_trait;
use indoc::indoc;
use rmcp::model::{
    CallToolResult, Content, ErrorCode, ErrorData, GetPromptResult, Implementation,
    InitializeResult, JsonObject, ListPromptsResult, ListResourcesResult, ListToolsResult,
    ReadResourceResult, ServerCapabilities, ServerNotification, Tool, ToolAnnotations,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "Extension Manager";

/// The durable half of `manage_extensions`: write the enabled flag to the SAME
/// config entry the Settings UI writes through `POST /config/extensions`, then
/// report what actually happened.
///
/// Why this exists: `manage_extensions` used to mutate only the live in-session
/// `ExtensionManager`. The tool reported success, the tools really did appear
/// or vanish for the rest of that conversation, and then the change evaporated
/// at the next daemon restart — while the Settings pane, which reads config.yaml,
/// showed the old state the entire time. Live effect without persistence is the
/// same class of bug as a save that silently fails.
///
/// It writes through `config::set_extension_enabled` rather than looping back
/// over loopback HTTP the way `save_pronunciation` does. That tool has no choice
/// — the voice lexicon lives in the daemon crate, unreachable from here —
/// whereas the extension registry is `crate::config`, one call away. Going
/// direct removes a bearer-token read, a network hop, and a 401 failure mode,
/// and lands on the identical writer the route calls, which is what "one source
/// of truth" was asking for. That write emits `config_changed`, which is what
/// refreshes an open Settings pane.
///
/// Split out from `manage_extensions_impl` so this half is testable without
/// standing up a provider and a session manager to get an `ExtensionManager`.
fn persist_and_report(
    action: ManageExtensionAction,
    extension_name: &str,
    key: &str,
) -> Vec<Content> {
    let enabled = action == ManageExtensionAction::Enable;
    let (verb, past) = if enabled {
        ("enabled", "installed")
    } else {
        ("disabled", "disabled")
    };
    let persisted = crate::config::set_extension_enabled(key, enabled);
    // Say plainly whether the change outlives the session. An extension present
    // at runtime but absent from config.yaml (a recipe-supplied one, say) can be
    // toggled live and cannot be persisted; reporting that as a plain success is
    // what teaches the model to promise durability it did not get.
    let note = if persisted {
        format!(
            " and saved to your configuration — it will still be {verb} after a restart, and \
             the Settings pane now shows it that way."
        )
    } else {
        format!(
            " for THIS SESSION ONLY. '{extension_name}' has no entry in config.yaml, so the \
             change could not be saved and will be gone after a restart. Tell the user that \
             rather than reporting it as saved."
        )
    };
    vec![Content::text(format!(
        "The extension '{extension_name}' has been {past}{note}"
    ))]
}

/// Test seam for [`persist_and_report`] — the durable half of
/// `manage_extensions`, reachable without standing up an `ExtensionManager`
/// (which needs a provider and a session manager). Exercises the production
/// function, not a copy of it.
#[doc(hidden)]
pub fn persist_and_report_for_tests(enable: bool, extension_name: &str, key: &str) -> Vec<Content> {
    let action = if enable {
        ManageExtensionAction::Enable
    } else {
        ManageExtensionAction::Disable
    };
    persist_and_report(action, extension_name, key)
}

#[derive(Debug, thiserror::Error)]
pub enum ExtensionManagerToolError {
    #[error("Unknown tool: {tool_name}")]
    UnknownTool { tool_name: String },

    #[error("Extension manager not available")]
    ManagerUnavailable,

    #[error("Missing required parameter: {param_name}")]
    MissingParameter { param_name: String },

    #[error("Extension operation failed: {message}")]
    OperationFailed { message: String },

    #[error("Failed to deserialize parameters: {0}")]
    DeserializationError(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ManageExtensionAction {
    Enable,
    Disable,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ManageExtensionsParams {
    pub action: ManageExtensionAction,
    pub extension_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReadResourceParams {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListResourcesParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension_name: Option<String>,
}

pub const READ_RESOURCE_TOOL_NAME: &str = "read_resource";
pub const LIST_RESOURCES_TOOL_NAME: &str = "list_resources";
pub const SEARCH_AVAILABLE_EXTENSIONS_TOOL_NAME: &str = "search_available_extensions";
pub const MANAGE_EXTENSIONS_TOOL_NAME: &str = "manage_extensions";
pub const MANAGE_EXTENSIONS_TOOL_NAME_COMPLETE: &str = "extensionmanager__manage_extensions";
pub const SEARCH_MEMORY_TOOL_NAME: &str = "search_memory";
pub const GET_MEMORY_TOOL_NAME: &str = "get_memory";

pub struct ExtensionManagerClient {
    info: InitializeResult,
    #[allow(dead_code)]
    context: PlatformExtensionContext,
}

impl ExtensionManagerClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult::new(
            ServerCapabilities::builder().enable_tools().build(),
        )
        .with_server_info(Implementation::new(EXTENSION_NAME, "1.0.0").with_title(EXTENSION_NAME))
        .with_instructions(indoc! {r#"
            Extension Management

            Use these tools to discover, enable, and disable extensions, as well as review resources.

            Available tools:
            - search_available_extensions: Find extensions available to enable/disable
            - manage_extensions: Enable or disable extensions
            - list_resources: List resources from extensions
            - read_resource: Read specific resources from extensions

            When you lack the tools needed to complete a task, use search_available_extensions first
            to discover what extensions can help.

            Use manage_extensions to enable or disable specific extensions by name.
            Use list_resources and read_resource to work with extension data and resources.
        "#});

        Ok(Self { info, context })
    }

    async fn handle_search_available_extensions(
        &self,
    ) -> Result<Vec<Content>, ExtensionManagerToolError> {
        if let Some(weak_ref) = &self.context.extension_manager {
            if let Some(extension_manager) = weak_ref.upgrade() {
                match extension_manager.search_available_extensions().await {
                    Ok(content) => Ok(content),
                    Err(e) => Err(ExtensionManagerToolError::OperationFailed {
                        message: format!("Failed to search available extensions: {}", e.message),
                    }),
                }
            } else {
                Err(ExtensionManagerToolError::ManagerUnavailable)
            }
        } else {
            Err(ExtensionManagerToolError::ManagerUnavailable)
        }
    }

    async fn handle_manage_extensions(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, ExtensionManagerToolError> {
        let arguments = arguments.ok_or(ExtensionManagerToolError::MissingParameter {
            param_name: "arguments".to_string(),
        })?;

        let params: ManageExtensionsParams =
            serde_json::from_value(serde_json::Value::Object(arguments))?;

        match self
            .manage_extensions_impl(params.action, params.extension_name)
            .await
        {
            Ok(content) => Ok(content),
            Err(error_data) => Err(ExtensionManagerToolError::OperationFailed {
                message: error_data.message.to_string(),
            }),
        }
    }

    async fn manage_extensions_impl(
        &self,
        action: ManageExtensionAction,
        extension_name: String,
    ) -> Result<Vec<Content>, ErrorData> {
        let extension_manager = self
            .context
            .extension_manager
            .as_ref()
            .and_then(|weak| weak.upgrade())
            .ok_or_else(|| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    "Extension manager is no longer available".to_string(),
                    None,
                )
            })?;

        if action == ManageExtensionAction::Disable {
            // Live effect first, durable second: if the in-session removal
            // fails there is nothing to persist, and a config entry saying
            // "disabled" for tools that are still loaded would be the same lie
            // in the other direction.
            extension_manager
                .remove_extension(&extension_name)
                .await
                .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
            return Ok(persist_and_report(
                ManageExtensionAction::Disable,
                &extension_name,
                &crate::config::name_to_key(&extension_name),
            ));
        }

        let config = match get_extension_by_name(&extension_name) {
            Some(config) => config,
            None => {
                return Err(ErrorData::new(
                    ErrorCode::RESOURCE_NOT_FOUND,
                    format!(
                        "Extension '{}' not found. Please check the extension name and try again.",
                        extension_name
                    ),
                    None,
                ));
            }
        };
        let key = config.key();

        extension_manager
            .add_extension(config, None, None, None)
            .await
            .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
        Ok(persist_and_report(
            ManageExtensionAction::Enable,
            &extension_name,
            &key,
        ))
    }

    async fn handle_list_resources(
        &self,
        session_id: &str,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, ExtensionManagerToolError> {
        if let Some(weak_ref) = &self.context.extension_manager {
            if let Some(extension_manager) = weak_ref.upgrade() {
                let params = arguments
                    .map(serde_json::Value::Object)
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

                match extension_manager
                    .list_resources(
                        session_id,
                        params,
                        tokio_util::sync::CancellationToken::default(),
                    )
                    .await
                {
                    Ok(content) => Ok(content),
                    Err(e) => Err(ExtensionManagerToolError::OperationFailed {
                        message: format!("Failed to list resources: {}", e.message),
                    }),
                }
            } else {
                Err(ExtensionManagerToolError::ManagerUnavailable)
            }
        } else {
            Err(ExtensionManagerToolError::ManagerUnavailable)
        }
    }

    async fn handle_read_resource(
        &self,
        session_id: &str,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, ExtensionManagerToolError> {
        if let Some(weak_ref) = &self.context.extension_manager {
            if let Some(extension_manager) = weak_ref.upgrade() {
                let params = arguments
                    .map(serde_json::Value::Object)
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

                match extension_manager
                    .read_resource_tool(
                        session_id,
                        params,
                        tokio_util::sync::CancellationToken::default(),
                    )
                    .await
                {
                    Ok(content) => Ok(content),
                    Err(e) => Err(ExtensionManagerToolError::OperationFailed {
                        message: format!("Failed to read resource: {}", e.message),
                    }),
                }
            } else {
                Err(ExtensionManagerToolError::ManagerUnavailable)
            }
        } else {
            Err(ExtensionManagerToolError::ManagerUnavailable)
        }
    }

    /// The full tool SUPERSET this extension can expose, in exposure order.
    /// `get_tools` is *dynamic* — `list_resources`/`read_resource` appear only
    /// when the extension manager supports resources, `search_memory` only when
    /// the Brain is loaded — but it SELECTS from this list by name, so a tool
    /// that is not constructed here cannot ship at all. That makes this the
    /// drift-proof inventory for the self-knowledge completeness guard
    /// (`self_knowledge::tests::tool_descriptions_name_every_callable_tool`):
    /// add a tool here (the only place one can be added) and CI fails until the
    /// registry `description` names it. A constructed-client test also asserts
    /// a real `list_tools` run returns exactly the ungated prefix of this list,
    /// and that the gated remainder matches the three gate consts.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn all_possible_tools() -> Vec<Tool> {
        vec![
            Tool::new(
                SEARCH_AVAILABLE_EXTENSIONS_TOOL_NAME.to_string(),
                "Searches for additional extensions available to help complete tasks.
        Use this tool when you're unable to find a specific feature or functionality you need to complete your task, or when standard approaches aren't working.
        These extensions might provide the exact tools needed to solve your problem.
        If you find a relevant one, consider using your tools to enable it.".to_string(),
                Arc::new(
                    serde_json::json!({
                        "type": "object",
                        "required": [],
                        "properties": {}
                    })
                    .as_object()
                    .expect("Schema must be an object")
                    .clone()
                ),
            ).annotate(ToolAnnotations::from_raw(
                Some("Discover extensions".to_string()),
                Some(true),
                Some(false),
                Some(false),
                Some(false),
            )),
            Tool::new(
                MANAGE_EXTENSIONS_TOOL_NAME.to_string(),
                "Tool to manage extensions and tools in the agent's context.
            Enable or disable extensions to help complete tasks.
            Enable or disable an extension by providing the extension name.
            ".to_string(),
                Arc::new(
                    serde_json::to_value(schema_for!(ManageExtensionsParams))
                        .expect("Failed to serialize schema")
                        .as_object()
                        .expect("Schema must be an object")
                        .clone()
                ),
            ).annotate(ToolAnnotations::from_raw(
                Some("Enable or disable an extension".to_string()),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            )),
            Tool::new(
                LIST_RESOURCES_TOOL_NAME.to_string(),
                indoc! {r#"
            List resources from an extension(s).

            Resources allow extensions to share data that provide context to LLMs, such as
            files, database schemas, or application-specific information. This tool lists resources
            in the provided extension, and returns a list for the user to browse. If no extension
            is provided, the tool will search all extensions for the resource.
        "#}.to_string(),
                Arc::new(
                    serde_json::to_value(schema_for!(ListResourcesParams))
                        .expect("Failed to serialize schema")
                        .as_object()
                        .expect("Schema must be an object")
                        .clone()
                ),
            ).annotate(ToolAnnotations::from_raw(
                Some("List resources".to_string()),
                Some(true),
                Some(false),
                Some(false),
                Some(false),
            )),
            Tool::new(
                READ_RESOURCE_TOOL_NAME.to_string(),
                indoc! {r#"
            Read a resource from an extension.

            Resources allow extensions to share data that provide context to LLMs, such as
            files, database schemas, or application-specific information. This tool searches for the
            resource URI in the provided extension, and reads in the resource content. If no extension
            is provided, the tool will search all extensions for the resource.
        "#}.to_string(),
                Arc::new(
                    serde_json::to_value(schema_for!(ReadResourceParams))
                        .expect("Failed to serialize schema")
                        .as_object()
                        .expect("Schema must be an object")
                        .clone()
                ),
            ).annotate(ToolAnnotations::from_raw(
                Some("Read a resource".to_string()),
                Some(true),
                Some(false),
                Some(false),
                Some(false),
            )),
            Tool::new(
                SEARCH_MEMORY_TOOL_NAME.to_string(),
                "Search your long-term memory (Brain) for information about a topic. \
                 Returns the most relevant memories matching the query, layered by budget \
                 (abstract, then overview, then full). Use this when you \
                 need to recall facts, events, preferences, or context that you've learned \
                 from past conversations and observations. The query should be a natural \
                 language phrase describing what you're looking for."
                    .to_string(),
                Arc::new(
                    serde_json::json!({
                        "type": "object",
                        "required": ["query"],
                        "properties": {
                            "query": {
                                "type": "string",
                                "description": "Natural language search query for memories"
                            },
                            "limit": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 20,
                                "description": "Maximum number of matched memories (default 8)"
                            },
                            "depth": {
                                "type": "string",
                                "enum": ["budget", "abstract", "overview", "full", "narrative"],
                                "description": "Result detail: budget (default), abstract, overview, full, or bounded narrative neighbors"
                            }
                        }
                    })
                    .as_object()
                    .expect("Schema must be an object")
                    .clone(),
                ),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Search memories".to_string()),
                Some(true),
                Some(false),
                Some(false),
                Some(false),
            )),
            Tool::new(
                GET_MEMORY_TOOL_NAME.to_string(),
                "Load one exact Brain memory by its stable id or key. Returns the stored metadata and exact content; use this after search_memory identifies the memory you need to inspect.".to_string(),
                Arc::new(
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "description": "Stable Spectral memory id"
                            },
                            "key": {
                                "type": "string",
                                "description": "Stable logical memory key"
                            }
                        },
                        "oneOf": [
                            {"required": ["id"]},
                            {"required": ["key"]}
                        ]
                    })
                    .as_object()
                    .expect("Schema must be an object")
                    .clone(),
                ),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Load exact memory".to_string()),
                Some(true),
                Some(false),
                Some(false),
                Some(false),
            )),
        ]
    }

    /// The gated tail of [`Self::all_possible_tools`]: everything after the
    /// always-exposed prefix, each present only when its availability gate is
    /// open. Kept next to `get_tools` so the gate logic and the inventory
    /// cannot silently diverge — the constructed-client test asserts the split.
    /// Test-only consumer, so `#[cfg(test)]` keeps it out of the shipped build.
    #[cfg(test)]
    pub(crate) const GATED_TOOL_NAMES: &[&str] = &[
        LIST_RESOURCES_TOOL_NAME,
        READ_RESOURCE_TOOL_NAME,
        SEARCH_MEMORY_TOOL_NAME,
        GET_MEMORY_TOOL_NAME,
    ];

    async fn get_tools(&self) -> Vec<Tool> {
        let mut names = vec![
            SEARCH_AVAILABLE_EXTENSIONS_TOOL_NAME,
            MANAGE_EXTENSIONS_TOOL_NAME,
        ];

        if let Some(weak_ref) = &self.context.extension_manager {
            if let Some(extension_manager) = weak_ref.upgrade() {
                if extension_manager.supports_resources().await {
                    names.push(LIST_RESOURCES_TOOL_NAME);
                    names.push(READ_RESOURCE_TOOL_NAME);
                }
            }
        }

        // search_memory — active memory search via Brain recall (available when Brain is loaded)
        if super::get_global_brain().is_some() {
            names.push(SEARCH_MEMORY_TOOL_NAME);
            names.push(GET_MEMORY_TOOL_NAME);
        }

        Self::all_possible_tools()
            .into_iter()
            .filter(|t| names.iter().any(|n| t.name.as_ref() == *n))
            .collect()
    }
}

#[async_trait]
impl McpClientTrait for ExtensionManagerClient {
    async fn list_resources(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<ListResourcesResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn read_resource(
        &self,
        _session_id: &str,
        _uri: &str,
        _cancellation_token: CancellationToken,
    ) -> Result<ReadResourceResult, Error> {
        // Extension manager doesn't expose resources directly
        Err(Error::TransportClosed)
    }

    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        Ok(ListToolsResult {
            tools: self.get_tools().await,
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        ctx: &ToolCallContext,
        name: &str,
        arguments: Option<JsonObject>,
        _cancellation_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        let session_id = &ctx.session_id;
        let result = match name {
            SEARCH_AVAILABLE_EXTENSIONS_TOOL_NAME => {
                self.handle_search_available_extensions().await
            }
            MANAGE_EXTENSIONS_TOOL_NAME => self.handle_manage_extensions(arguments).await,
            LIST_RESOURCES_TOOL_NAME => self.handle_list_resources(session_id, arguments).await,
            READ_RESOURCE_TOOL_NAME => self.handle_read_resource(session_id, arguments).await,
            SEARCH_MEMORY_TOOL_NAME => handle_search_memory(session_id, arguments).await,
            GET_MEMORY_TOOL_NAME => handle_get_memory(arguments).await,
            _ => Err(ExtensionManagerToolError::UnknownTool {
                tool_name: name.to_string(),
            }),
        };

        match result {
            Ok(content) => Ok(CallToolResult::success(content)),
            Err(error) => Ok(CallToolResult::error(vec![Content::text(
                error.to_string(),
            )])),
        }
    }

    async fn list_prompts(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<ListPromptsResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn get_prompt(
        &self,
        _session_id: &str,
        _name: &str,
        _arguments: Value,
        _cancellation_token: CancellationToken,
    ) -> Result<GetPromptResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn subscribe(&self) -> mpsc::Receiver<ServerNotification> {
        mpsc::channel(1).1
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchMemoryDepth {
    Budget,
    Abstract,
    Overview,
    Full,
    Narrative,
}

fn parse_search_memory_options(
    arguments: &JsonObject,
) -> Result<(String, usize, SearchMemoryDepth), ExtensionManagerToolError> {
    let query = arguments
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let limit = arguments
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(8)
        .clamp(1, 20);
    let depth = match arguments.get("depth").and_then(|v| v.as_str()) {
        None | Some("budget") => SearchMemoryDepth::Budget,
        Some("abstract") => SearchMemoryDepth::Abstract,
        Some("overview") => SearchMemoryDepth::Overview,
        Some("full") => SearchMemoryDepth::Full,
        Some("narrative") => SearchMemoryDepth::Narrative,
        Some(other) => {
            return Err(ExtensionManagerToolError::OperationFailed {
                message: format!(
                    "Invalid depth '{}'; use budget, abstract, overview, full, or narrative",
                    other
                ),
            })
        }
    };
    Ok((query, limit, depth))
}

fn first_sentence_for_search(text: &str) -> &str {
    let text = text.trim();
    text.find(['.', '!', '?'])
        .and_then(|i| text.get(..=i))
        .map(str::trim)
        .filter(|s| s.chars().count() >= 8)
        .unwrap_or(text)
}

fn take_search_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

/// Return the query terms found in the stored content and a bounded exact
/// excerpt around the first matching term. This is deliberately extractive:
/// search results must give the model evidence it can verify, not a new
/// paraphrase that could be mistaken for the stored memory.
fn exact_search_excerpt(content: &str, query: &str) -> (String, String) {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|term| {
            term.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|term| term.chars().count() >= 2)
        .collect();
    let lowered = content.to_lowercase();
    let matched: Vec<&str> = terms
        .iter()
        .filter(|term| lowered.contains(term.as_str()))
        .map(String::as_str)
        .collect();
    let Some(first_term) = matched.first() else {
        return (String::new(), take_search_chars(content, 320));
    };
    let start = lowered.find(first_term).unwrap_or(0);
    let window = 320usize;
    let start = start.saturating_sub(window / 2);
    let excerpt: String = content
        .char_indices()
        .filter(|(idx, _)| *idx >= start)
        .map(|(_, ch)| ch)
        .take(window)
        .collect();
    (matched.join(", "), excerpt)
}

fn explicit_search_text(
    hit: &spectral::ingest::MemoryHit,
    depth: SearchMemoryDepth,
) -> (&'static str, String) {
    match depth {
        SearchMemoryDepth::Abstract => (
            "abstract",
            hit.description
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| first_sentence_for_search(&hit.content))
                .to_string(),
        ),
        SearchMemoryDepth::Overview => ("overview", take_search_chars(&hit.content, 400)),
        SearchMemoryDepth::Full => ("full", hit.content.clone()),
        SearchMemoryDepth::Narrative => unreachable!("narrative is assembled separately"),
        SearchMemoryDepth::Budget => unreachable!("budget is assembled separately"),
    }
}

fn format_search_memory_hit(
    index: usize,
    hit: &spectral::ingest::MemoryHit,
    text: &str,
    layer: &str,
    query: &str,
) -> String {
    let (matched, excerpt) = exact_search_excerpt(&hit.content, query);
    format!(
        "{}. [id: {}] [key: {}] [source: {}] [score: {:.2}] [layer: {}]\n   rendered {}: {}\n   matched: {}\n   exact excerpt: {}",
        index,
        hit.id,
        hit.key,
        hit.source.as_deref().unwrap_or("unknown"),
        hit.signal_score,
        layer,
        layer,
        text,
        if matched.is_empty() { "(none)" } else { &matched },
        excerpt,
    )
}

/// Handle the search_memory tool — active Brain recall via spawn_blocking.
/// Brain methods use block_on() internally and MUST run off the async executor.
async fn handle_search_memory(
    session_id: &str,
    arguments: Option<JsonObject>,
) -> Result<Vec<Content>, ExtensionManagerToolError> {
    let arguments = arguments.unwrap_or_default();
    let (query, limit, depth) = parse_search_memory_options(&arguments)?;
    if query.is_empty() {
        return Ok(vec![Content::text(
            "Please provide a search query.".to_string(),
        )]);
    }

    let brain = match super::get_global_brain() {
        Some(b) => b,
        None => {
            return Ok(vec![Content::text(
                "Memory search unavailable — Brain not loaded.".to_string(),
            )]);
        }
    };

    // Layered / provenance recall path — gated default-OFF behind the same
    // LIBRARIAN_ATOMS_ENABLED flag as the write-side. When on, surface each
    // consolidation atom together with the raw sources it distilled, framed
    // explicitly as a candidate set to VERIFY (never authoritative). This
    // hint-not-truth framing is the load-bearing difference from the read-time
    // pre-pass that regressed −9.2pp. Until the mini eval flips the flag this
    // branch is never taken, so actor behavior is unchanged.
    if matches!(depth, SearchMemoryDepth::Budget)
        && crate::config::Config::global()
            .get_param::<bool>("LIBRARIAN_ATOMS_ENABLED")
            .unwrap_or(false)
    {
        return handle_search_memory_layered(&brain, &query, limit).await;
    }

    let mut ctx = spectral::graph::RecognitionContext::empty()
        .with_persona(crate::config::agent_identity::DEFAULT_PERSONA_KEY);
    if !session_id.trim().is_empty() {
        // Session is factual caller context; no project/wing is inferred here.
        ctx = ctx.with_session(session_id);
    }
    match brain.recall_cascade(&query, &ctx).await {
        Ok(recall_result) => {
            let hits = &recall_result.merged_hits;
            // Live event for World View (recall-as-river). Counts/query only.
            crate::events::emit(crate::events::memory_recalled(
                &query,
                hits.len(),
                "search_memory",
            ));
            if hits.is_empty() {
                return Ok(vec![Content::text(format!(
                    "No memories found matching \"{}\".",
                    query
                ))]);
            }

            let selected = hits.iter().take(limit).collect::<Vec<_>>();
            if matches!(depth, SearchMemoryDepth::Narrative) {
                const MAX_NARRATIVE_NEIGHBORS: usize = 3;
                let mut output = format!(
                    "Found {} memories matching \"{}\" with bounded narrative context:\n\n",
                    selected.len(),
                    query
                );
                let mut seen = std::collections::HashSet::new();
                for (i, hit) in selected.iter().enumerate() {
                    seen.insert(hit.id.clone());
                    let (_, excerpt) = exact_search_excerpt(&hit.content, &query);
                    output.push_str(&format_search_memory_hit(
                        i + 1,
                        hit,
                        &excerpt,
                        "narrative",
                        &query,
                    ));
                    output.push('\n');
                    if let Some(episode_id) = hit.episode_id.as_deref() {
                        if let Ok(neighbors) = brain.list_memories_by_episode(episode_id).await {
                            let center = neighbors
                                .iter()
                                .position(|neighbor| {
                                    neighbor.id == hit.id || neighbor.key == hit.key
                                })
                                .unwrap_or(0);
                            let start = center.saturating_sub(MAX_NARRATIVE_NEIGHBORS / 2);
                            let mut added = 0;
                            for neighbor in neighbors.into_iter().skip(start) {
                                if added >= MAX_NARRATIVE_NEIGHBORS || seen.contains(&neighbor.id) {
                                    continue;
                                }
                                seen.insert(neighbor.id.clone());
                                output.push_str(&format!(
                                    "   neighbor [id: {}] [key: {}] [source: {}] [episode: {}] [created_at: {}]\n   exact excerpt: {}\n",
                                    neighbor.id,
                                    neighbor.key,
                                    neighbor.source.as_deref().unwrap_or("unknown"),
                                    episode_id,
                                    neighbor.created_at.as_deref().unwrap_or("unknown"),
                                    take_search_chars(&neighbor.content, 320),
                                ));
                                added += 1;
                            }
                        }
                    }
                }
                return Ok(vec![Content::text(output)]);
            }
            if !matches!(depth, SearchMemoryDepth::Budget) {
                let mut output = format!(
                    "Found {} memories matching \"{}\":\n\n",
                    selected.len(),
                    query
                );
                for (i, hit) in selected.iter().enumerate() {
                    let (layer, text) = explicit_search_text(hit, depth);
                    output.push_str(&format_search_memory_hit(i + 1, hit, &text, layer, &query));
                    output.push('\n');
                }
                return Ok(vec![Content::text(output)]);
            }

            let sources: Vec<crate::context_layers::AssembleSource<'_>> = selected
                .iter()
                .map(|hit| crate::context_layers::AssembleSource {
                    key: hit.key.as_str(),
                    abstract_text: hit.description.as_deref(),
                    content: hit.content.as_str(),
                    score: hit.signal_score,
                })
                .collect();
            let layered = crate::context_layers::assemble(
                &sources,
                crate::context_layers::AssembleBudget::SEARCH,
            );
            let mut output = format!(
                "Found {} memories matching \"{}\":\n\n",
                layered.len(),
                query
            );
            for (i, hit) in layered.iter().enumerate() {
                if let Some(original) = selected.get(i) {
                    output.push_str(&format_search_memory_hit(
                        i + 1,
                        original,
                        &hit.text,
                        hit.layer.as_str(),
                        &query,
                    ));
                    output.push('\n');
                }
            }

            Ok(vec![Content::text(output)])
        }
        Err(e) => Ok(vec![Content::text(format!("Memory search failed: {}", e))]),
    }
}

/// Load a single exact memory by stable id or logical key. Unlike search this
/// path does not rank, summarize, or substitute a preview for the stored text.
async fn handle_get_memory(
    arguments: Option<JsonObject>,
) -> Result<Vec<Content>, ExtensionManagerToolError> {
    let args = arguments.unwrap_or_default();
    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let key = args
        .get("key")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    if id.is_some() == key.is_some() {
        return Err(ExtensionManagerToolError::OperationFailed {
            message: "Provide exactly one non-empty memory id or key".to_string(),
        });
    }

    let brain =
        super::get_global_brain().ok_or_else(|| ExtensionManagerToolError::OperationFailed {
            message: "Memory lookup unavailable — Brain not loaded.".to_string(),
        })?;
    let memory = if let Some(id) = id {
        brain.get_memory(id).await
    } else {
        brain
            .get_memory_by_key(key.expect("key checked above"))
            .await
    }
    .map_err(|e| ExtensionManagerToolError::OperationFailed {
        message: format!("Memory lookup failed: {}", e),
    })?;

    let Some(memory) = memory else {
        return Ok(vec![Content::text("Memory not found.".to_string())]);
    };
    let source = memory.source.as_deref().unwrap_or("unknown");
    let output = format!(
        "Memory\n[id: {}]\n[key: {}]\n[source: {}]\n[score: {:.2}]\n[layer: full]\n[episode: {}]\n[created_at: {}]\n\n{}",
        memory.id,
        memory.key,
        source,
        memory.signal_score,
        memory.episode_id.as_deref().unwrap_or("none"),
        memory.created_at.as_deref().unwrap_or("unknown"),
        memory.content,
    );
    Ok(vec![Content::text(output)])
}

/// Layered recall: each hit paired with the raw sources it was distilled from.
/// Gated behind `LIBRARIAN_ATOMS_ENABLED` (default OFF). The consolidation atoms
/// are presented as a candidate set to VERIFY against the sources, never as
/// authoritative — this framing is the load-bearing difference from the
/// read-time consolidation pre-pass that regressed −9.2pp.
async fn handle_search_memory_layered(
    brain: &crate::brain_handle::SafeBrain,
    query: &str,
    limit: usize,
) -> Result<Vec<Content>, ExtensionManagerToolError> {
    const MAX_SOURCES_PER_HIT: usize = 3;

    let hits = match brain
        .recall_with_provenance(
            query.to_string(),
            spectral::Visibility::Private,
            MAX_SOURCES_PER_HIT,
        )
        .await
    {
        Ok(h) => h,
        Err(e) => return Ok(vec![Content::text(format!("Memory search failed: {}", e))]),
    };

    crate::events::emit(crate::events::memory_recalled(
        query,
        hits.len(),
        "search_memory",
    ));

    if hits.is_empty() {
        return Ok(vec![Content::text(format!(
            "No memories found matching \"{}\".",
            query
        ))]);
    }

    // Framing FIRST: the atoms are hints to verify, not ground truth. This
    // instruction is what keeps the strong actor from over-trusting a
    // consolidated summary — the exact failure the read-time A/B measured.
    let mut output = format!(
        "Found {} memories matching \"{}\".\n\n\
         Some results are CONSOLIDATED atoms (a summary distilled from several raw \
         sessions, shown with its sources indented beneath it). Treat every atom as a \
         CANDIDATE SET to VERIFY, not as ground truth: confirm each item against the raw \
         sessions before counting it, and add any items the atoms missed. When an atom \
         and its sources disagree, the raw sources win.\n\n",
        hits.len().min(limit.min(5)),
        query
    );

    for (i, layered) in hits.iter().take(limit.min(5)).enumerate() {
        let hit = &layered.hit;
        if layered.sources.is_empty() {
            output.push_str(&format_search_memory_hit(
                i + 1,
                hit,
                &hit.content,
                "raw",
                query,
            ));
            output.push('\n');
        } else {
            output.push_str(&format_search_memory_hit(
                i + 1,
                hit,
                &hit.content,
                "atom",
                query,
            ));
            output.push_str(&format!(
                "\n   verify vs {} raw source(s) below:\n",
                layered.sources.len()
            ));
            for src in &layered.sources {
                let (_, source_excerpt) = exact_search_excerpt(&src.content, query);
                output.push_str(&format!(
                    "     - source exact excerpt: {}\n",
                    source_excerpt
                ));
            }
        }
    }

    Ok(vec![Content::text(output)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_hit() -> spectral::ingest::MemoryHit {
        serde_json::from_value(serde_json::json!({
            "id": "mem-001",
            "key": "episode:synthetic:beat-1",
            "content": "Alpha beat: the gate opened after the verification step.",
            "wing": null,
            "hall": null,
            "signal_score": 0.875,
            "hits": 2,
            "source": "synthetic_fixture",
            "description": "A concise abstract of the alpha beat."
        }))
        .expect("synthetic MemoryHit should deserialize")
    }

    #[test]
    fn search_options_have_bounded_defaults_and_depths() {
        let args = serde_json::json!({"query": "alpha beat"});
        assert_eq!(
            parse_search_memory_options(args.as_object().unwrap()).unwrap(),
            ("alpha beat".to_string(), 8, SearchMemoryDepth::Budget)
        );

        let args = serde_json::json!({
            "query": "alpha",
            "limit": 999,
            "depth": "narrative"
        });
        assert_eq!(
            parse_search_memory_options(args.as_object().unwrap()).unwrap(),
            ("alpha".to_string(), 20, SearchMemoryDepth::Narrative)
        );
    }

    #[test]
    fn search_hit_has_stable_provenance_and_distinct_exact_excerpt() {
        let hit = synthetic_hit();
        let (matched, excerpt) = exact_search_excerpt(&hit.content, "alpha gate");
        assert_eq!(matched, "alpha, gate");
        assert!(excerpt.contains("Alpha beat"));

        let rendered = format_search_memory_hit(
            1,
            &hit,
            "A concise budget rendering.",
            "abstract",
            "alpha gate",
        );
        assert!(rendered.contains("[id: mem-001]"));
        assert!(rendered.contains("[key: episode:synthetic:beat-1]"));
        assert!(rendered.contains("[source: synthetic_fixture]"));
        assert!(rendered.contains("[score: 0.88]"));
        assert!(rendered.contains("rendered abstract: A concise budget rendering."));
        assert!(rendered.contains("exact excerpt: Alpha beat"));
    }

    #[test]
    fn invalid_search_depth_is_rejected() {
        let args = serde_json::json!({"query": "alpha", "depth": "invented"});
        let error = parse_search_memory_options(args.as_object().unwrap()).unwrap_err();
        assert!(error.to_string().contains("Invalid depth"));
    }

    #[test]
    fn golden_narrative_fixture_renders_every_ordered_beat_with_source_identity() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../goose-server/tests/fixtures/narrative_memory_replay.json"
        ))
        .unwrap();
        let source = &fixture["source"];
        let mut hit = synthetic_hit();
        hit.id = source["memory_id"].as_str().unwrap().to_string();
        hit.key = source["memory_key"].as_str().unwrap().to_string();
        hit.content = source["content"].as_str().unwrap().to_string();
        let rendered = format_search_memory_hit(
            1,
            &hit,
            &hit.content,
            "full",
            fixture["queries"][0]["text"].as_str().unwrap(),
        );

        assert!(rendered.contains(&format!("[id: {}]", hit.id)));
        assert!(rendered.contains(&format!("[key: {}]", hit.key)));
        let mut cursor = 0;
        for beat in fixture["ordered_beats"].as_array().unwrap() {
            let beat = beat.as_str().unwrap();
            let relative = rendered[cursor..]
                .find(beat)
                .unwrap_or_else(|| panic!("missing or out-of-order story beat: {beat}"));
            cursor += relative + beat.len();
        }
        assert!(rendered.contains("remains unknown"));
    }
}
