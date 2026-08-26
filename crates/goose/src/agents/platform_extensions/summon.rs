use super::{parse_frontmatter, Source, SourceKind};
use crate::agents::platform_extensions::fanout;
use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::subagent_handler::{run_subagent_task, OnMessageCallback, SubagentRunParams};
use crate::agents::subagent_task_config::{TaskConfig, DEFAULT_SUBAGENT_MAX_TURNS};
use crate::agents::tool_execution::ToolCallContext;
use crate::agents::AgentRunnerConfig;
use crate::config::paths::Paths;
use crate::config::{Config, GooseMode};
use crate::providers;
use crate::recipe::build_recipe::build_recipe_from_template;
use crate::recipe::local_recipes::load_local_recipe_file;
use crate::recipe::{Recipe, Settings, RECIPE_FILE_EXTENSIONS};
use crate::session::extension_data::EnabledExtensionsState;
use crate::session::SessionType;
use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult, Meta,
    ServerCapabilities, ServerNotification, Tool,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, Mutex};

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

pub static EXTENSION_NAME: &str = "summon";

fn kind_plural(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Subrecipe => "Subrecipes",
        SourceKind::Recipe => "Recipes",
        SourceKind::Agent => "Agents",
        _ => "Other",
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else if max_len <= 3 {
        "...".to_string()
    } else {
        let truncated: String = s.chars().take(max_len - 3).collect();
        format!("{}...", truncated)
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct DelegateParams {
    pub instructions: Option<String>,
    pub source: Option<String>,
    pub parameters: Option<HashMap<String, serde_json::Value>>,
    pub extensions: Option<Vec<String>>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub temperature: Option<f32>,
    pub max_turns: Option<usize>,
    #[serde(default)]
    pub r#async: bool,
    /// Optional worker persona key from agent.yaml workers section.
    /// If set and resolvable, the subagent identifies as this worker.
    #[serde(default)]
    pub worker_persona: Option<String>,
}

/// One child of a `delegate_many` fan-out: an ordinary delegate, plus a label
/// so the aggregate can name it something better than "child 2".
#[derive(Debug, Deserialize, Default)]
pub struct FanoutChildParams {
    #[serde(flatten)]
    pub delegate: DelegateParams,
    pub label: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct DelegateManyParams {
    #[serde(default)]
    pub tasks: Vec<FanoutChildParams>,
    /// Children in flight at once. Defaults to
    /// [`fanout::DEFAULT_FANOUT_CONCURRENCY`]; clamped to the configured cap so
    /// a caller cannot talk the machine into running eight agents at once.
    pub max_concurrent: Option<usize>,
}

/// Everything one child needs, resolved on the caller's side — recipe, routing,
/// persona — so the concurrency gate holds nothing but the actual run.
struct PreparedChild {
    label: String,
    working_dir: PathBuf,
    recipe: Recipe,
    task_config: TaskConfig,
    routing_receipt: serde_json::Value,
    persona_override: Option<(String, String)>,
}

pub struct BackgroundTask {
    pub id: String,
    pub description: String,
    pub started_at: Instant,
    pub turns: Arc<AtomicU32>,
    pub last_activity: Arc<AtomicU64>,
    pub handle: JoinHandle<Result<String>>,
    pub cancellation_token: CancellationToken,
    pub notification_buffer: Arc<Mutex<Vec<ServerNotification>>>,
}

pub struct CompletedTask {
    pub id: String,
    pub description: String,
    pub result: Result<String, String>,
    pub turns_taken: u32,
    pub duration: Duration,
}

#[derive(Debug, Deserialize)]
struct AgentMetadata {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

fn parse_agent_content(content: &str, path: &Path) -> Option<Source> {
    let (metadata, body): (AgentMetadata, String) = match parse_frontmatter(content) {
        Ok(Some(parsed)) => parsed,
        Ok(None) => return None,
        Err(e) => {
            // Missing fields means this file has valid YAML but isn't an agent — skip silently.
            // Only warn on actual YAML syntax errors.
            if e.to_string().contains("missing field") {
                return None;
            }
            warn!("Failed to parse agent file {}: {}", path.display(), e);
            return None;
        }
    };

    let description = metadata.description.unwrap_or_else(|| {
        let model_info = metadata
            .model
            .as_ref()
            .map(|m| format!(" ({})", m))
            .unwrap_or_default();
        format!("Agent{}", model_info)
    });

    Some(Source {
        name: metadata.name,
        kind: SourceKind::Agent,
        description,
        path: path.to_path_buf(),
        content: body,
        supporting_files: Vec::new(),
    })
}

fn scan_recipes_from_dir(
    dir: &Path,
    kind: SourceKind,
    sources: &mut Vec<Source>,
    seen: &mut std::collections::HashSet<String>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !RECIPE_FILE_EXTENSIONS.contains(&ext) {
            continue;
        }

        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        if name.is_empty() || seen.contains(&name) {
            continue;
        }

        match Recipe::from_file_path(&path) {
            Ok(recipe) => {
                seen.insert(name.clone());
                sources.push(Source {
                    name,
                    kind,
                    description: recipe.description.clone(),
                    path: path.clone(),
                    content: recipe.instructions.clone().unwrap_or_default(),
                    supporting_files: Vec::new(),
                });
            }
            Err(e) => {
                warn!("Failed to parse recipe {}: {}", path.display(), e);
            }
        }
    }
}

fn scan_agents_from_dir(
    dir: &Path,
    sources: &mut Vec<Source>,
    seen: &mut std::collections::HashSet<String>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "md" {
            continue;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to read agent file {}: {}", path.display(), e);
                continue;
            }
        };

        if let Some(source) = parse_agent_content(&content, &path) {
            if !seen.contains(&source.name) {
                seen.insert(source.name.clone());
                sources.push(source);
            }
        }
    }
}

fn discover_filesystem_sources(working_dir: &Path) -> Vec<Source> {
    let mut sources: Vec<Source> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    let home = dirs::home_dir();
    let config = Paths::config_dir();

    let local_recipe_dirs: Vec<PathBuf> = vec![
        working_dir.to_path_buf(),
        working_dir.join(".goose/recipes"),
        working_dir.join(".agents/recipes"),
    ];

    let global_recipe_dirs: Vec<PathBuf> = std::env::var("GOOSE_RECIPE_PATH")
        .ok()
        .into_iter()
        .flat_map(|p| {
            let sep = if cfg!(windows) { ';' } else { ':' };
            p.split(sep).map(PathBuf::from).collect::<Vec<_>>()
        })
        .chain(
            [
                Some(config.join("recipes")),
                home.as_ref().map(|h| h.join(".agents/recipes")),
            ]
            .into_iter()
            .flatten(),
        )
        .collect();

    let local_agent_dirs: Vec<PathBuf> = vec![
        working_dir.join(".goose/agents"),
        working_dir.join(".claude/agents"),
        working_dir.join(".agents/agents"),
    ];

    let global_agent_dirs: Vec<PathBuf> = [
        home.as_ref().map(|h| h.join(".agents/agents")),
        Some(config.join("agents")),
        home.as_ref().map(|h| h.join(".claude/agents")),
    ]
    .into_iter()
    .flatten()
    .collect();

    for dir in local_recipe_dirs {
        scan_recipes_from_dir(&dir, SourceKind::Recipe, &mut sources, &mut seen);
    }

    for dir in local_agent_dirs {
        scan_agents_from_dir(&dir, &mut sources, &mut seen);
    }

    for dir in global_recipe_dirs {
        scan_recipes_from_dir(&dir, SourceKind::Recipe, &mut sources, &mut seen);
    }

    for dir in global_agent_dirs {
        scan_agents_from_dir(&dir, &mut sources, &mut seen);
    }

    sources
}

fn round_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s", (secs / 10) * 10)
    } else {
        format!("{}m", secs / 60)
    }
}

fn current_epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Get maximum number of concurrent background tasks
fn max_background_tasks() -> usize {
    Config::global()
        .get_param::<usize>("GOOSE_MAX_BACKGROUND_TASKS")
        .unwrap_or(5)
}

fn is_session_id(s: &str) -> bool {
    let parts: Vec<&str> = s.split('_').collect();
    parts.len() == 2 && parts[0].len() == 8 && parts[0].chars().all(|c| c.is_ascii_digit())
}

pub struct SummonClient {
    info: InitializeResult,
    context: PlatformExtensionContext,
    source_cache: Mutex<Option<(Instant, PathBuf, Vec<Source>)>>,
    background_tasks: Mutex<HashMap<String, BackgroundTask>>,
    completed_tasks: Mutex<HashMap<String, CompletedTask>>,
    notification_subscribers: Arc<Mutex<Vec<mpsc::Sender<ServerNotification>>>>,
}

impl Drop for SummonClient {
    fn drop(&mut self) {
        // Best-effort cancellation of running tasks on shutdown
        if let Ok(tasks) = self.background_tasks.try_lock() {
            for task in tasks.values() {
                task.cancellation_token.cancel();
            }
        }
    }
}

impl SummonClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(EXTENSION_NAME, "1.0.0").with_title("Summon"));

        Ok(Self {
            info,
            context,
            source_cache: Mutex::new(None),
            background_tasks: Mutex::new(HashMap::new()),
            completed_tasks: Mutex::new(HashMap::new()),
            notification_subscribers: Arc::new(Mutex::new(Vec::new())),
        })
    }

    fn spawn_notification_bridge(
        mut notif_rx: tokio::sync::mpsc::UnboundedReceiver<ServerNotification>,
        subscribers: Arc<Mutex<Vec<mpsc::Sender<ServerNotification>>>>,
        buffer: Arc<Mutex<Vec<ServerNotification>>>,
    ) {
        tokio::spawn(async move {
            while let Some(notification) = notif_rx.recv().await {
                let mut subs = subscribers.lock().await;
                if subs.is_empty() {
                    drop(subs);
                    buffer.lock().await.push(notification);
                } else {
                    subs.retain(|tx| match tx.try_send(notification.clone()) {
                        Ok(()) => true,
                        Err(mpsc::error::TrySendError::Full(_)) => true,
                        Err(mpsc::error::TrySendError::Closed(_)) => false,
                    });
                }
            }
        });
    }

    /// The full tool SUPERSET this extension can expose. `list_tools` is
    /// genuinely dynamic — `delegate` is hidden from subagent sessions so they
    /// cannot recurse — but it SELECTS from this list, so a tool absent here
    /// cannot ship at all. That makes this the drift-proof inventory for the
    /// self-knowledge completeness guard (the main-session view, which is
    /// where the `permagent_self` brief renders). A constructed-client test in
    /// `self_knowledge::tests` additionally asserts a real `list_tools` run
    /// for a non-subagent session returns exactly these names.
    pub(crate) fn all_possible_tools() -> Vec<Tool> {
        vec![
            Self::create_load_tool(),
            Self::create_delegate_tool(),
            Self::create_delegate_many_tool(),
        ]
    }

    fn create_load_tool() -> Tool {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "source": {
                    "type": "string",
                    "description": "Name of the source to load. If omitted, lists all available sources."
                },
                "cancel": {
                    "type": "boolean",
                    "default": false,
                    "description": "For running background tasks: cancel and return output."
                }
            }
        });

        Tool::new(
            "load",
            "Load knowledge into your current context or discover available sources.\n\n\
             Call with no arguments to list all available sources (subrecipes, recipes, agents).\n\
             Call with a source name to load its content into your context.\n\
             For background tasks: load(source: \"task_id\") waits for the task and returns the result.\n\
             To cancel a running task: load(source: \"task_id\", cancel: true) stops and returns output.\n\n\
             Examples:\n\
             - load() → Lists available sources\n\
             - load(source: \"deploy\") → Loads the deploy recipe\n\
             - load(source: \"20260219_1\") → Waits for background task, then returns result"
                .to_string(),
            schema.as_object().unwrap().clone(),
        )
    }

    fn create_delegate_tool() -> Tool {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "instructions": {
                    "type": "string",
                    "description": "Task instructions. Required for ad-hoc tasks."
                },
                "source": {
                    "type": "string",
                    "description": "Name of a recipe or agent to run."
                },
                "parameters": {
                    "type": "object",
                    "additionalProperties": true,
                    "description": "Parameters for the source (only valid with source)."
                },
                "extensions": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Extensions to enable. Omit to inherit all, empty array for none."
                },
                "provider": {
                    "type": "string",
                    "description": "Override LLM provider."
                },
                "model": {
                    "type": "string",
                    "description": "Override model."
                },
                "temperature": {
                    "type": "number",
                    "description": "Override temperature."
                },
                "max_turns": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Maximum turns for this delegate. Overrides recipe settings.max_turns and GOOSE_SUBAGENT_MAX_TURNS."
                },
                "async": {
                    "type": "boolean",
                    "default": false,
                    "description": "Run in background (default: false)."
                },
                "worker_persona": {
                    "type": "string",
                    "description": "Worker persona key from agent.yaml. If set and found, the subagent identifies as this worker."
                }
            }
        });

        Tool::new(
            "delegate",
            "Delegate a task to a subagent that runs independently with its own context.\n\n\
             Modes:\n\
             1. Ad-hoc: Provide `instructions` for a custom task\n\
             2. Source-based: Provide `source` name to run a subrecipe, recipe, or agent\n\
             3. Combined: Pair a source with a task (e.g., source: \"deploy\", instructions: \"deploy to staging\")\n\n\
             Effective Delegation:\n\
             - Delegates know only instructions + source content\n\
             - Delegates cannot coordinate. Same-file work = conflicts.\n\
             - Parallel: async: true, then load(taskId) to wait and get results. Single: sync.\n\n\
             Research (read-only): parallelize freely - delegates explore and report back.\n\
             Work (writes): partition files strictly - no two delegates touch the same file.\n\n\
             Decompose → async delegates → load(taskId) for each → synthesize."
                .to_string(),
            schema.as_object().unwrap().clone(),
        )
    }

    fn create_delegate_many_tool() -> Tool {
        let child = serde_json::json!({
            "type": "object",
            "properties": {
                "instructions": {"type": "string", "description": "What this child should do."},
                "source": {"type": "string", "description": "Subrecipe / recipe / agent to run."},
                "label": {"type": "string", "description": "Short name for this child in the aggregate (e.g. \"security\")."},
                "parameters": {"type": "object", "description": "Parameters for a source."},
                "extensions": {"type": "array", "items": {"type": "string"}, "description": "Narrow this child's extensions."},
                "provider": {"type": "string", "description": "Override LLM provider for this child."},
                "model": {"type": "string", "description": "Override model for this child."},
                "temperature": {"type": "number"},
                "max_turns": {"type": "integer", "minimum": 1},
                "worker_persona": {"type": "string", "description": "Worker persona key from agent.yaml."}
            }
        });
        let schema = serde_json::json!({
            "type": "object",
            "required": ["tasks"],
            "properties": {
                "tasks": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": fanout::MAX_FANOUT_CHILDREN,
                    "items": child,
                    "description": "The children to run. Results come back in this order."
                },
                "max_concurrent": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "How many children run at once. Defaults to the configured fan-out concurrency."
                }
            }
        });

        Tool::new(
            "delegate_many",
            format!(
                "Fan out to several subagents at once and get every answer back in one call.\n\n\
                 Use this instead of N separate `delegate` calls when the work splits cleanly: \
                 review lenses, audits, independent research questions.\n\n\
                 - At most {} children per call, and only a few run at a time — the rest queue. \
                 The cap protects the machine; asking for more does not make it faster.\n\
                 - Results come back IN THE ORDER you listed them, each naming its own model, \
                 its own subagent id, and what it spent.\n\
                 - Children cannot coordinate. Partition files strictly, or keep them read-only.\n\
                 - Cancelling this call cancels the children.",
                fanout::MAX_FANOUT_CHILDREN
            ),
            schema.as_object().unwrap().clone(),
        )
    }

    /// Fan out to N subagents with a bounded number in flight, join in order.
    ///
    /// Each child is resolved through the SAME path a single `delegate` takes —
    /// `build_delegate_recipe` then `build_task_config` — so `cost_router::
    /// delegate`'s precedence applies per child: an explicit provider/model on
    /// that child wins, then its recipe, then the role pin, then the pack pin,
    /// and escalation stays opt-in. Ten children do not become ten silent
    /// escalations to the most expensive row in the knowledge base.
    async fn handle_delegate_many(
        &self,
        session_id: &str,
        arguments: Option<JsonObject>,
        cancellation_token: CancellationToken,
    ) -> Result<CallToolResult, String> {
        let params: DelegateManyParams = arguments
            .map(|args| serde_json::from_value(serde_json::Value::Object(args)))
            .transpose()
            .map_err(|e| format!("Invalid parameters: {}", e))?
            .unwrap_or_default();

        if params.tasks.is_empty() {
            return Err("'tasks' is empty — delegate_many needs at least one child".to_string());
        }
        if params.tasks.len() > fanout::MAX_FANOUT_CHILDREN {
            return Err(format!(
                "delegate_many takes at most {} children (got {}). More than that is a queue, \
                 not a fan-out — run them in batches.",
                fanout::MAX_FANOUT_CHILDREN,
                params.tasks.len()
            ));
        }

        let session = self
            .context
            .session_manager
            .get_session(session_id, false)
            .await
            .map_err(|e| format!("Failed to get session: {}", e))?;

        if session.session_type == SessionType::SubAgent {
            return Err("Delegated tasks cannot spawn further delegations".to_string());
        }

        // Resolve every child BEFORE any of them runs: a fan-out that fails on
        // child 7's bad parameters after paying for six children is worse than
        // one that refuses up front.
        let working_dir = session.working_dir.clone();
        let mut prepared: Vec<PreparedChild> = Vec::with_capacity(params.tasks.len());
        for (index, task) in params.tasks.into_iter().enumerate() {
            let label = task
                .label
                .clone()
                .unwrap_or_else(|| truncate(&Self::get_task_description(&task.delegate), 40));
            let mut child = task.delegate;
            // `async` is meaningless on a child: the fan-out itself is the
            // asynchrony, and the caller joins here.
            child.r#async = false;
            self.validate_delegate_params(&child)
                .map_err(|e| format!("child {index} ({label}): {e}"))?;

            let recipe = self
                .build_delegate_recipe(&child, session_id, &working_dir)
                .await
                .map_err(|e| format!("child {index} ({label}): {e}"))?;
            let (task_config, routing) = self
                .build_task_config(&child, &recipe, &session)
                .await
                .map_err(|e| format!("child {index} ({label}): failed to build task config: {e}"))?;

            prepared.push(PreparedChild {
                label,
                working_dir: working_dir.clone(),
                recipe,
                task_config,
                routing_receipt: routing.receipt_json(),
                persona_override: Self::resolve_worker_persona(child.worker_persona.as_deref()),
            });
        }

        let concurrency = params
            .max_concurrent
            .unwrap_or_else(fanout::fanout_concurrency)
            .clamp(1, fanout::fanout_concurrency().max(1));

        info!(
            target: "permagentd::brain",
            children = prepared.len(),
            concurrency,
            "delegate_many fan-out starting"
        );

        let session_manager = self.context.session_manager.clone();
        let subscribers = Arc::clone(&self.notification_subscribers);
        let outcomes = fanout::run_bounded(
            prepared,
            concurrency,
            cancellation_token,
            move |index, child, token| {
                let session_manager = session_manager.clone();
                let subscribers = Arc::clone(&subscribers);
                async move {
                    let PreparedChild {
                        label,
                        working_dir,
                        recipe,
                        task_config,
                        routing_receipt,
                        persona_override,
                    } = child;

                    let subagent_session = match session_manager
                        .create_session(
                            working_dir,
                            format!("Fan-out [{index}] {label}"),
                            SessionType::SubAgent,
                            GooseMode::Auto,
                        )
                        .await
                    {
                        Ok(s) => s,
                        Err(e) => {
                            return fanout::ChildOutcome::failed(
                                index,
                                label,
                                format!("could not create the subagent session: {e}"),
                            )
                        }
                    };
                    let subagent_id = subagent_session.id.clone();

                    let (notif_tx, notif_rx) =
                        tokio::sync::mpsc::unbounded_channel::<ServerNotification>();
                    Self::spawn_notification_bridge(
                        notif_rx,
                        subscribers,
                        Arc::new(Mutex::new(Vec::new())),
                    );

                    // Subagents run in Auto for the same reason the single
                    // delegate path does: nothing forwards an approval prompt
                    // to the parent, so any other mode hangs.
                    let agent_config = AgentRunnerConfig::new(
                        session_manager.clone(),
                        crate::config::permission::PermissionManager::instance(),
                        None,
                        GooseMode::Auto,
                        true,
                        crate::agents::GoosePlatform::GooseCli,
                    );

                    let result = run_subagent_task(SubagentRunParams {
                        config: agent_config,
                        recipe,
                        task_config,
                        return_last_only: true,
                        session_id: subagent_session.id,
                        // The token is derived from the caller's: cancel the
                        // fan-out and the running children stop too.
                        cancellation_token: Some(token.clone()),
                        on_message: None,
                        notification_tx: Some(notif_tx),
                        persona_override,
                    })
                    .await;

                    let (status, text) = match result {
                        Ok(text) => (fanout::ChildStatus::Ok, text),
                        Err(e) if token.is_cancelled() => {
                            (fanout::ChildStatus::Cancelled, format!("cancelled: {e}"))
                        }
                        Err(e) => (fanout::ChildStatus::Failed, format!("delegation failed: {e}")),
                    };
                    fanout::ChildOutcome {
                        index,
                        label,
                        status,
                        subagent_id: Some(subagent_id),
                        model_routing: Some(routing_receipt),
                        text,
                    }
                }
            },
        )
        .await;

        // Per-child spend, read back out of the ledger by each child's own id.
        let pool = self.context.session_manager.pool_clone().await.ok();
        let mut costs = Vec::with_capacity(outcomes.len());
        for outcome in &outcomes {
            let cost = match (pool.as_ref(), outcome.subagent_id.as_deref()) {
                (Some(pool), Some(id)) => fanout::subagent_cost(pool, id).await,
                _ => fanout::SubagentCost::default(),
            };
            costs.push(cost);
        }

        let children_meta: Vec<serde_json::Value> = outcomes
            .iter()
            .zip(costs.iter())
            .map(|(o, c)| {
                serde_json::json!({
                    "index": o.index,
                    "label": o.label,
                    "status": o.status.as_str(),
                    "subagent_id": o.subagent_id,
                    "model_routing": o.model_routing,
                    "cost": c,
                })
            })
            .collect();

        let mut meta = Meta::new();
        meta.0.insert(
            "fanout_children".to_string(),
            serde_json::Value::Array(children_meta),
        );

        Ok(
            CallToolResult::success(vec![Content::text(fanout::render_outcomes(
                &outcomes, &costs,
            ))])
            .with_meta(Some(meta)),
        )
    }

    async fn get_working_dir(&self, session_id: &str) -> PathBuf {
        self.context
            .session_manager
            .get_session(session_id, false)
            .await
            .ok()
            .map(|s| s.working_dir)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
    }

    async fn get_sources(&self, session_id: &str, working_dir: &Path) -> Vec<Source> {
        let fs_sources = self.get_filesystem_sources(working_dir).await;

        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut sources: Vec<Source> = Vec::new();

        self.add_subrecipes(session_id, &mut sources, &mut seen)
            .await;

        for source in fs_sources {
            if !seen.contains(&source.name) {
                seen.insert(source.name.clone());
                sources.push(source);
            }
        }

        sources.sort_by(|a, b| (&a.kind, &a.name).cmp(&(&b.kind, &b.name)));
        sources
    }

    async fn get_filesystem_sources(&self, working_dir: &Path) -> Vec<Source> {
        let mut cache = self.source_cache.lock().await;
        if let Some((cached_at, cached_dir, sources)) = cache.as_ref() {
            if cached_dir == working_dir && cached_at.elapsed() < Duration::from_secs(60) {
                return sources.clone();
            }
        }
        let sources = self.discover_filesystem_sources(working_dir);
        *cache = Some((Instant::now(), working_dir.to_path_buf(), sources.clone()));
        sources
    }

    async fn resolve_source(
        &self,
        session_id: &str,
        name: &str,
        working_dir: &Path,
    ) -> Result<Option<Source>, String> {
        let sources = self.get_sources(session_id, working_dir).await;

        if let Some(mut source) = sources.iter().find(|s| s.name == name).cloned() {
            if source.kind == SourceKind::Subrecipe && source.content.is_empty() {
                source.content = self.load_subrecipe_content(session_id, &source.name).await;
            }
            return Ok(Some(source));
        }

        Ok(None)
    }

    async fn load_subrecipe_content(&self, session_id: &str, name: &str) -> String {
        let session = match self
            .context
            .session_manager
            .get_session(session_id, false)
            .await
        {
            Ok(s) => s,
            Err(_) => return String::new(),
        };

        let sub_recipes = match session.recipe.as_ref().and_then(|r| r.sub_recipes.as_ref()) {
            Some(sr) => sr,
            None => return String::new(),
        };

        let sr = match sub_recipes.iter().find(|sr| sr.name == name) {
            Some(sr) => sr,
            None => return String::new(),
        };

        match load_local_recipe_file(&sr.path) {
            Ok(recipe_file) => match Recipe::from_content(&recipe_file.content) {
                Ok(recipe) => recipe.instructions.unwrap_or_default(),
                Err(_) => recipe_file.content,
            },
            Err(_) => String::new(),
        }
    }

    fn discover_filesystem_sources(&self, working_dir: &Path) -> Vec<Source> {
        discover_filesystem_sources(working_dir)
    }

    async fn add_subrecipes(
        &self,
        session_id: &str,
        sources: &mut Vec<Source>,
        seen: &mut std::collections::HashSet<String>,
    ) {
        let session = match self
            .context
            .session_manager
            .get_session(session_id, false)
            .await
        {
            Ok(s) => s,
            Err(_) => return,
        };

        let sub_recipes = match session.recipe.as_ref().and_then(|r| r.sub_recipes.as_ref()) {
            Some(sr) => sr,
            None => return,
        };

        for sr in sub_recipes {
            if seen.contains(&sr.name) {
                continue;
            }
            seen.insert(sr.name.clone());

            let description = self.build_subrecipe_description(sr).await;

            sources.push(Source {
                name: sr.name.clone(),
                kind: SourceKind::Subrecipe,
                description,
                path: PathBuf::from(&sr.path),
                content: String::new(),
                supporting_files: Vec::new(),
            });
        }
    }

    async fn build_subrecipe_description(&self, sr: &crate::recipe::SubRecipe) -> String {
        if let Some(desc) = &sr.description {
            return desc.clone();
        }

        if let Ok(recipe_file) = load_local_recipe_file(&sr.path) {
            if let Ok(recipe) = Recipe::from_content(&recipe_file.content) {
                let mut desc = recipe.description.clone();

                if let Some(params) = &recipe.parameters {
                    let param_names: Vec<&str> = params.iter().map(|p| p.key.as_str()).collect();
                    if !param_names.is_empty() {
                        let params_str = param_names.join(", ");
                        desc = format!("{} (params: {})", desc, params_str);
                    }
                }

                return desc;
            }
        }

        format!("Subrecipe from {}", sr.path)
    }

    async fn handle_load(
        &self,
        session_id: &str,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, String> {
        self.cleanup_completed_tasks().await;

        let source_name = arguments
            .as_ref()
            .and_then(|args| args.get("source"))
            .and_then(|v| v.as_str());

        let cancel = arguments
            .as_ref()
            .and_then(|args| args.get("cancel"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let working_dir = self.get_working_dir(session_id).await;

        if source_name.is_none() {
            return self
                .handle_load_discovery(session_id, &working_dir)
                .await
                .map(CallToolResult::success);
        }

        let name = source_name.unwrap();

        if is_session_id(name) {
            let content = self.handle_load_task_result(name, cancel).await?;
            let mut meta = Meta::new();
            meta.0.insert(
                "subagent_session_id".to_string(),
                serde_json::Value::String(name.to_string()),
            );
            return Ok(CallToolResult::success(content).with_meta(Some(meta)));
        }

        self.handle_load_source(session_id, name, &working_dir)
            .await
            .map(CallToolResult::success)
    }

    async fn handle_load_task_result(
        &self,
        task_id: &str,
        cancel: bool,
    ) -> Result<Vec<Content>, String> {
        let mut completed = self.completed_tasks.lock().await;

        if let Some(task) = completed.remove(task_id) {
            let status = if task.result.is_ok() {
                "✓ Completed"
            } else {
                "✗ Failed"
            };
            let output = match task.result {
                Ok(output) => output,
                Err(error) => format!("Error: {}", error),
            };

            return Ok(vec![Content::text(format!(
                "# Background Task Result: {}\n\n\
                 **Task:** {}\n\
                 **Status:** {}\n\
                 **Duration:** {} ({} turns)\n\n\
                 ## Output\n\n{}",
                task_id,
                task.description,
                status,
                round_duration(task.duration),
                task.turns_taken,
                output
            ))]);
        }

        drop(completed);

        let mut running = self.background_tasks.lock().await;
        if running.contains_key(task_id) {
            if cancel {
                let task = running.remove(task_id).unwrap();
                drop(running);

                task.cancellation_token.cancel();

                let duration = task.started_at.elapsed();
                let turns_taken = task.turns.load(Ordering::Relaxed);

                let mut handle = task.handle;
                let output = tokio::select! {
                    result = &mut handle => {
                        match result {
                            Ok(Ok(s)) => s,
                            Ok(Err(e)) => format!("Error: {}", e),
                            Err(e) => format!("Task panicked: {}", e),
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {
                        handle.abort();
                        "Task did not stop in time (aborted)".to_string()
                    }
                };

                return Ok(vec![Content::text(format!(
                    "# Background Task Result: {}\n\n\
                     **Task:** {}\n\
                     **Status:** ⊘ Cancelled\n\
                     **Duration:** {} ({} turns)\n\n\
                     ## Output\n\n{}",
                    task_id,
                    task.description,
                    round_duration(duration),
                    turns_taken,
                    output
                ))]);
            }

            // Wait for the running task to complete, keeping the tool call
            // alive so notifications (subagent tool calls) stream in real time.
            let mut task = running.remove(task_id).unwrap();
            drop(running);

            let buffered = {
                let mut buf = task.notification_buffer.lock().await;
                std::mem::take(&mut *buf)
            };
            if !buffered.is_empty() {
                let subs = self.notification_subscribers.lock().await;
                for notif in buffered {
                    for tx in subs.iter() {
                        let _ = tx.try_send(notif.clone());
                    }
                }
            }

            tokio::select! {
                result = &mut task.handle => {
                    let output = match result {
                        Ok(Ok(s)) => s,
                        Ok(Err(e)) => format!("Error: {}", e),
                        Err(e) => format!("Task panicked: {}", e),
                    };

                    return Ok(vec![Content::text(format!(
                        "# Background Task Result: {}\n\n\
                         **Task:** {}\n\
                         **Status:** ✓ Completed\n\
                         **Duration:** {} ({} turns)\n\n\
                         ## Output\n\n{}",
                        task_id,
                        task.description,
                        round_duration(task.started_at.elapsed()),
                        task.turns.load(Ordering::Relaxed),
                        output
                    ))]);
                }
                _ = tokio::time::sleep(Duration::from_secs(300)) => {
                    self.background_tasks.lock().await.insert(task_id.to_string(), task);

                    return Err(format!(
                        "Task '{task_id}' is still running after waiting 5 min. \
                         Use load(source: \"{task_id}\") to wait again, or \
                         load(source: \"{task_id}\", cancel: true) to stop."
                    ));
                }
            }
        }

        Err(format!("Task '{}' not found.", task_id))
    }

    async fn handle_load_discovery(
        &self,
        session_id: &str,
        working_dir: &Path,
    ) -> Result<Vec<Content>, String> {
        {
            let mut cache = self.source_cache.lock().await;
            *cache = None;
        }

        let sources = self.get_sources(session_id, working_dir).await;
        let completed = self.completed_tasks.lock().await;

        if sources.is_empty() && completed.is_empty() {
            return Ok(vec![Content::text(
                "No sources available for load/delegate.\n\n\
                 Sources are discovered from:\n\
                 • Current recipe's sub_recipes\n\
                 • .agents/recipes/, .agents/agents/ (project-level)\n\
                 • ~/.agents/agents/ (global)\n\
                 • GOOSE_RECIPE_PATH directories",
            )]);
        }

        let mut output = String::from("Available sources for load/delegate:\n");

        if !completed.is_empty() {
            output.push_str("\nCompleted Tasks (awaiting retrieval):\n");
            let mut sorted_completed: Vec<_> = completed.values().collect();
            sorted_completed.sort_by_key(|t| &t.id);
            for task in sorted_completed {
                let status = if task.result.is_ok() {
                    "completed"
                } else {
                    "failed"
                };
                output.push_str(&format!(
                    "• {} - \"{}\" ({})\n",
                    task.id, task.description, status
                ));
            }
        }

        for kind in [SourceKind::Subrecipe, SourceKind::Recipe, SourceKind::Agent] {
            let kind_sources: Vec<_> = sources.iter().filter(|s| s.kind == kind).collect();
            if !kind_sources.is_empty() {
                output.push_str(&format!("\n{}:\n", kind_plural(kind)));
                for source in kind_sources {
                    output.push_str(&format!(
                        "• {} - {}\n",
                        source.name,
                        truncate(&source.description, 60)
                    ));
                }
            }
        }

        output.push_str("\nUse load(source: \"name\") to load into context.\n");
        output.push_str("Use delegate(source: \"name\") to run as subagent.");

        Ok(vec![Content::text(output)])
    }

    async fn handle_load_source(
        &self,
        session_id: &str,
        name: &str,
        working_dir: &Path,
    ) -> Result<Vec<Content>, String> {
        let source = self.resolve_source(session_id, name, working_dir).await?;

        match source {
            Some(source) => {
                let content = source.to_load_text();

                let output = format!(
                    "# Loaded: {} ({})\n\n{}\n\n---\nThis knowledge is now available in your context.",
                    source.name, source.kind, content
                );

                Ok(vec![Content::text(output)])
            }
            None => {
                let sources = self.get_sources(session_id, working_dir).await;

                let suggestions: Vec<&str> = sources
                    .iter()
                    .filter(|s| {
                        s.name.to_lowercase().contains(&name.to_lowercase())
                            || name.to_lowercase().contains(&s.name.to_lowercase())
                    })
                    .take(3)
                    .map(|s| s.name.as_str())
                    .collect();

                let error_msg = if suggestions.is_empty() {
                    format!(
                        "Source '{}' not found. Use load() to see available sources.",
                        name
                    )
                } else {
                    format!(
                        "Source '{}' not found. Did you mean: {}?",
                        name,
                        suggestions.join(", ")
                    )
                };

                Err(error_msg)
            }
        }
    }

    async fn handle_delegate(
        &self,
        session_id: &str,
        arguments: Option<JsonObject>,
        cancellation_token: CancellationToken,
    ) -> Result<CallToolResult, String> {
        self.cleanup_completed_tasks().await;

        let params: DelegateParams = arguments
            .map(|args| serde_json::from_value(serde_json::Value::Object(args)))
            .transpose()
            .map_err(|e| format!("Invalid parameters: {}", e))?
            .unwrap_or_default();

        self.validate_delegate_params(&params)?;

        let session = self
            .context
            .session_manager
            .get_session(session_id, false)
            .await
            .map_err(|e| format!("Failed to get session: {}", e))?;

        if session.session_type == SessionType::SubAgent {
            return Err("Delegated tasks cannot spawn further delegations".to_string());
        }

        if params.r#async {
            let (content, task_id) = self.handle_async_delegate(session_id, params).await?;
            let mut meta = Meta::new();
            meta.0.insert(
                "subagent_session_id".to_string(),
                serde_json::Value::String(task_id),
            );
            return Ok(CallToolResult::success(content).with_meta(Some(meta)));
        }

        let working_dir = session.working_dir.clone();
        let recipe = self
            .build_delegate_recipe(&params, session_id, &working_dir)
            .await?;

        let (task_config, routing) = self
            .build_task_config(&params, &recipe, &session)
            .await
            .map_err(|e| format!("Failed to build task config: {}", e))?;

        let persona_override = Self::resolve_worker_persona(params.worker_persona.as_deref());
        info!(
            target: "permagentd::brain",
            "Subagent spawned with worker persona: {}",
            params.worker_persona.as_deref().unwrap_or("(primary)")
        );

        // Subagents must use Auto until get_agent_messages forwards
        // ActionRequired messages to the parent. Until then, any mode
        // that requires approval will hang on the subagent's confirmation_rx.
        let agent_config = AgentRunnerConfig::new(
            self.context.session_manager.clone(),
            crate::config::permission::PermissionManager::instance(),
            None,
            GooseMode::Auto,
            true, // disable session naming for subagents
            crate::agents::GoosePlatform::GooseCli,
        );

        let subagent_session = self
            .context
            .session_manager
            .create_session(
                working_dir,
                "Delegated task".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .map_err(|e| format!("Failed to create subagent session: {}", e))?;

        let (notif_tx, notif_rx) = tokio::sync::mpsc::unbounded_channel::<ServerNotification>();
        Self::spawn_notification_bridge(
            notif_rx,
            Arc::clone(&self.notification_subscribers),
            Arc::new(Mutex::new(Vec::new())),
        );

        let subagent_session_id = subagent_session.id.clone();

        let result = run_subagent_task(SubagentRunParams {
            config: agent_config,
            recipe,
            task_config,
            return_last_only: true,
            session_id: subagent_session.id,
            cancellation_token: Some(cancellation_token),
            on_message: None,
            notification_tx: Some(notif_tx),
            persona_override,
        })
        .await;

        let mut meta = Meta::new();
        meta.0.insert(
            "subagent_session_id".to_string(),
            serde_json::Value::String(subagent_session_id),
        );
        // The routing receipt travels with the result so a reader can say which
        // model this subagent's tokens were billed to, and why it was that one.
        meta.0
            .insert("model_routing".to_string(), routing.receipt_json());

        match result {
            Ok(text) => {
                Ok(CallToolResult::success(vec![Content::text(text)]).with_meta(Some(meta)))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Delegation failed: {}",
                e
            ))])
            .with_meta(Some(meta))),
        }
    }

    /// Resolve worker_persona key to a (block, display_name) tuple.
    /// Returns None only when worker_persona is None (no persona injection needed).
    fn resolve_worker_persona(worker_persona: Option<&str>) -> Option<(String, String)> {
        let worker_key = worker_persona?;
        let config = crate::config::agent_identity::load_agent_config();
        if let Some(worker) = config.workers.get(worker_key) {
            Some((worker.system_prompt_block(), worker.display_name()))
        } else {
            tracing::warn!(
                target: "permagentd::brain",
                "Worker persona '{}' not found, falling back to primary",
                worker_key
            );
            let primary = &config.primary;
            Some((primary.system_prompt_block(), primary.display_name()))
        }
    }

    /// Instructions a worker cannot act on.
    ///
    /// Observed live on 2026-08-25: a GLM-5.3 harness session called
    /// `delegate` with `{"async": true, "instructions": "placeholder"}` and the
    /// daemon dutifully started a background task named "placeholder" — a whole
    /// subagent, a whole session, and a share of the same rate-limited API key,
    /// spent on nothing. A worker needs a task, and one word is not one.
    ///
    /// Deliberately narrow: it rejects the literal filler words and anything
    /// too short to be a task, and nothing else. Guessing at whether a real
    /// instruction is *good enough* is not this function's business.
    fn unusable_instructions(instructions: &str) -> Option<String> {
        const FILLER: &[&str] = &[
            "placeholder",
            "tbd",
            "todo",
            "test",
            "n/a",
            "none",
            "...",
            "instructions",
            "your instructions here",
        ];
        /// Shorter than this is never a task a subagent can carry out.
        const MIN_MEANINGFUL_CHARS: usize = 20;

        let trimmed = instructions.trim();
        let normalised = trimmed.trim_matches(|c: char| !c.is_alphanumeric());
        let folded = normalised.to_ascii_lowercase();

        if trimmed.is_empty() {
            return Some("'instructions' is empty".to_string());
        }
        if FILLER.contains(&folded.as_str()) {
            return Some(format!(
                "'instructions' is placeholder text ({trimmed:?}), not a task"
            ));
        }
        if normalised.chars().filter(|c| !c.is_whitespace()).count() < MIN_MEANINGFUL_CHARS {
            return Some(format!(
                "'instructions' is too short to delegate ({trimmed:?}) — \
                 say what the worker should do, in a sentence"
            ));
        }
        None
    }

    fn validate_delegate_params(&self, params: &DelegateParams) -> Result<(), String> {
        if params.instructions.is_none() && params.source.is_none() {
            return Err("Must provide 'instructions' or 'source' (or both)".to_string());
        }

        if params.parameters.is_some() && params.source.is_none() {
            return Err("'parameters' can only be used with 'source'".to_string());
        }

        if let Some(max) = params.max_turns {
            if max < 1 {
                return Err("'max_turns' must be at least 1".to_string());
            }
        }

        // A `source` names a real recipe, which carries its own prompt — only
        // ad-hoc instructions have to stand on their own.
        if params.source.is_none() {
            if let Some(instructions) = params.instructions.as_deref() {
                if let Some(reason) = Self::unusable_instructions(instructions) {
                    return Err(format!(
                        "{reason}. Nothing was started. Send the delegate call again \
                         with the actual task."
                    ));
                }
            }
        }

        Ok(())
    }

    async fn build_delegate_recipe(
        &self,
        params: &DelegateParams,
        session_id: &str,
        working_dir: &Path,
    ) -> Result<Recipe, String> {
        if let Some(source_name) = &params.source {
            self.build_source_recipe(source_name, params, session_id, working_dir)
                .await
        } else {
            self.build_adhoc_recipe(params)
        }
    }

    fn build_adhoc_recipe(&self, params: &DelegateParams) -> Result<Recipe, String> {
        let task = params
            .instructions
            .as_ref()
            .ok_or("Instructions required for ad-hoc task")?;

        Recipe::builder()
            .version("1.0.0")
            .title("Delegated Task")
            .description("Ad-hoc delegated task")
            .prompt(task)
            .build()
            .map_err(|e| format!("Failed to build recipe: {}", e))
    }

    async fn build_source_recipe(
        &self,
        source_name: &str,
        params: &DelegateParams,
        session_id: &str,
        working_dir: &Path,
    ) -> Result<Recipe, String> {
        let source = self
            .resolve_source(session_id, source_name, working_dir)
            .await?
            .ok_or_else(|| format!("Source '{}' not found", source_name))?;

        let mut recipe = match source.kind {
            SourceKind::Recipe | SourceKind::Subrecipe => {
                self.build_recipe_from_source(&source, params, session_id)
                    .await?
            }
            SourceKind::Agent => self.build_recipe_from_agent(&source, params)?,
            _ => {
                return Err(format!(
                    "Source '{}' has kind '{}' which cannot be delegated from summon",
                    source_name, source.kind
                ))
            }
        };

        if let Some(extra_instructions) = &params.instructions {
            if recipe.prompt.is_some() {
                let current_prompt = recipe.prompt.take().unwrap();
                recipe.prompt = Some(format!("{}\n\n{}", current_prompt, extra_instructions));
            } else {
                recipe.prompt = Some(extra_instructions.clone());
            }
        }

        Ok(recipe)
    }

    async fn build_recipe_from_source(
        &self,
        source: &Source,
        params: &DelegateParams,
        session_id: &str,
    ) -> Result<Recipe, String> {
        let session = self
            .context
            .session_manager
            .get_session(session_id, false)
            .await
            .map_err(|e| format!("Failed to get session: {}", e))?;

        if source.kind == SourceKind::Subrecipe {
            let sub_recipes = session.recipe.as_ref().and_then(|r| r.sub_recipes.as_ref());

            if let Some(sub_recipes) = sub_recipes {
                if let Some(sr) = sub_recipes.iter().find(|sr| sr.name == source.name) {
                    let recipe_file = load_local_recipe_file(&sr.path).map_err(|e| {
                        format!("Failed to load subrecipe '{}': {}", source.name, e)
                    })?;

                    let mut merged: HashMap<String, String> = HashMap::new();
                    if let Some(values) = &sr.values {
                        for (k, v) in values {
                            merged.insert(k.clone(), v.clone());
                        }
                    }
                    if let Some(provided_params) = &params.parameters {
                        for (k, v) in provided_params {
                            let value_str = match v {
                                serde_json::Value::String(s) => s.clone(),
                                other => other.to_string(),
                            };
                            merged.insert(k.clone(), value_str);
                        }
                    }
                    let param_values: Vec<(String, String)> = merged.into_iter().collect();

                    return build_recipe_from_template(
                        recipe_file.content,
                        &recipe_file.parent_dir,
                        param_values,
                        None::<fn(&str, &str) -> Result<String, anyhow::Error>>,
                    )
                    .map_err(|e| format!("Failed to build subrecipe: {}", e));
                }
            }
        }

        let recipe_file = load_local_recipe_file(source.path.to_str().unwrap_or(""))
            .map_err(|e| format!("Failed to load recipe '{}': {}", source.name, e))?;

        let param_values: Vec<(String, String)> = params
            .parameters
            .as_ref()
            .map(|p| {
                p.iter()
                    .map(|(k, v)| {
                        let value_str = match v {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        (k.clone(), value_str)
                    })
                    .collect()
            })
            .unwrap_or_default();

        build_recipe_from_template(
            recipe_file.content,
            &recipe_file.parent_dir,
            param_values,
            None::<fn(&str, &str) -> Result<String, anyhow::Error>>,
        )
        .map_err(|e| format!("Failed to build recipe: {}", e))
    }

    fn build_recipe_from_agent(
        &self,
        source: &Source,
        params: &DelegateParams,
    ) -> Result<Recipe, String> {
        let agent_content = if source.path.as_os_str().is_empty() {
            return Err("Agent source has no path".to_string());
        } else {
            std::fs::read_to_string(&source.path)
                .map_err(|e| format!("Failed to read agent file: {}", e))?
        };

        let (metadata, _): (AgentMetadata, String) = parse_frontmatter(&agent_content)
            .map_err(|e| format!("Failed to parse agent frontmatter: {}", e))?
            .ok_or("No frontmatter found in agent file")?;

        let model = metadata.model;

        // max_turns is set later in build_task_config so it can incorporate params.max_turns
        // with the correct priority ordering; setting it here would cause it to be overridden
        // by the parent session's recipe instead.
        let settings = model.map(|m| Settings {
            goose_model: Some(m),
            goose_provider: params.provider.clone(),
            temperature: params.temperature,
            max_turns: None,
        });

        let mut builder = Recipe::builder()
            .version("1.0.0")
            .title(format!("Agent: {}", source.name))
            .description(source.description.clone())
            .instructions(&source.content);

        if let Some(settings) = settings {
            builder = builder.settings(settings);
        }

        if params.instructions.is_none() {
            builder = builder.prompt("Proceed with your expertise to produce a useful result.");
        }

        builder
            .build()
            .map_err(|e| format!("Failed to build recipe from agent: {}", e))
    }

    async fn build_task_config(
        &self,
        params: &DelegateParams,
        recipe: &Recipe,
        session: &crate::session::Session,
    ) -> Result<(TaskConfig, crate::cost_router::DelegateRouting), anyhow::Error> {
        let (provider, routing) = self.resolve_provider(params, recipe, session).await?;

        let mut extensions = EnabledExtensionsState::extensions_or_default(
            Some(&session.extension_data),
            Config::global(),
        );

        if let Some(filter) = &params.extensions {
            if filter.is_empty() {
                extensions = Vec::new();
            } else {
                extensions.retain(|ext| filter.contains(&ext.name()));
            }
        }

        let max_turns = params
            .max_turns
            .or_else(|| recipe.settings.as_ref().and_then(|s| s.max_turns))
            .unwrap_or_else(|| self.resolve_max_turns(session));

        if max_turns == 0 || max_turns > u32::MAX as usize {
            anyhow::bail!(
                "max_turns must be between 1 and {} (got {})",
                u32::MAX,
                max_turns
            );
        }

        let task_config = TaskConfig::new(provider, &session.id, &session.working_dir, extensions)
            .with_max_turns(Some(max_turns));

        Ok((task_config, routing))
    }

    /// The workflow role a delegate's worker persona plays, for cost routing.
    /// `None` when there is no persona or it yields no role signal — dispatch then
    /// stays single-model. See `cost_router::role_map::derive_role`.
    fn role_for_persona(worker_persona: Option<&str>) -> Option<crate::cost_router::WorkflowRole> {
        let key = worker_persona?;
        let config = crate::config::agent_identity::load_agent_config();
        config.workers.get(key).and_then(|w| w.routing_role())
    }

    /// The spend band that gates a delegate ESCALATION.
    ///
    /// FAIL-CLOSED, and deliberately the opposite polarity to
    /// [`crate::tool_monitor`]'s `budget_verdict_for`: that gate STOPS work, so a
    /// transient fault must never fabricate a stop and it fails OPEN. This one
    /// AUTHORIZES extra spend on a pricier model, and a band we cannot read is not
    /// permission. `None` ⇒ no escalation.
    async fn escalation_spend_band(
        &self,
        session: &crate::session::Session,
    ) -> Option<crate::cost_router::BudgetBand> {
        use crate::agents::platform_extensions::orchestrator as orch;
        let pool = self.context.session_manager.pool_clone().await.ok()?;
        let task_spent = orch::task_spent_usd(&pool, &session.id).await;
        let session_spent = orch::session_spent_usd(&pool, &session.id).await;
        let unpriced = orch::unpriced_calls_in_session(&pool, &session.id).await;
        let cfg = crate::cost_router::budget::load_budget_config();
        Some(
            crate::cost_router::budget::budget_verdict_with_unpriced(
                task_spent,
                session_spent,
                unpriced,
                &cfg,
            )
            .band,
        )
    }

    async fn resolve_provider(
        &self,
        params: &DelegateParams,
        recipe: &Recipe,
        session: &crate::session::Session,
    ) -> Result<
        (
            Arc<dyn crate::providers::base::Provider>,
            crate::cost_router::DelegateRouting,
        ),
        anyhow::Error,
    > {
        // The role→model layer. If the delegate carries a worker persona with a
        // workflow role, route to that role's model by the precedence in
        // `cost_router::delegate`: role pin (`PERMAGENT_ROLE_*`) → pack pin
        // (`PERMAGENT_PACK_*`) → cost-router escalation (OPT-IN only) → the
        // session's own pair. Slotted after an explicit param / recipe setting and
        // before the `GOOSE_SUBAGENT_*` fallback, as before.
        //
        // What changed on 2026-08-25 (measured; see the `delegate` module note):
        // the `PERMAGENT_PACK_*` pins were not read here at all, and with no pin
        // the DERIVED best-fit map silently escalated across providers — an
        // `ANTHROPIC_API_KEY` alone was enough to route every EDIT/ORCHESTRATE/
        // REVIEW delegate of a gpt-5.4-mini or glm-5.3 session onto
        // `anthropic/claude-fable-5`, the priciest row in the knowledge base. A
        // subagent must never silently run on a family the operator did not
        // configure for this session, so escalation is now behind
        // `PERMAGENT_DELEGATE_ALLOW_ESCALATION` (default off), bounded to
        // configured providers, gated on the spend caps, and always on the record.
        let role = Self::role_for_persona(params.worker_persona.as_deref());
        let session_pair = session.provider_name.as_deref().map(|p| {
            (
                p,
                session
                    .model_config
                    .as_ref()
                    .map(|m| m.model_name.as_str())
                    .unwrap_or("the session model"),
            )
        });
        // The spend band is only read when it can change the answer: with
        // escalation off the `Disabled` refusal fires first, and a pinned dispatch
        // must not pay a DB round trip for a gate it will not reach.
        let spend = if crate::cost_router::delegate_escalation_allowed() {
            self.escalation_spend_band(session).await
        } else {
            None
        };
        let routing = crate::cost_router::delegate_routing_live(role, spend, session_pair).await;
        let role_model = routing.role_model.clone().map(|rm| (rm, routing.source));

        // The review gate's cross-vendor routing sits HERE for the `reviewer`
        // persona: `reviewer_dispatch` composes `reviewer_routing` with the
        // author's (session) pair. It slots the SAME REVIEW-role model the block
        // above resolved — hand-configured first, else the derived best fit —
        // (neither ⇒ `None` ⇒ the review inherits the session model exactly as
        // before), so the (provider, model) outcome and its provenance are
        // unchanged — what it adds is the diversity warning, logged below on the
        // channel the target reconciliation already uses.
        let review_dispatch = (role == Some(crate::cost_router::WorkflowRole::Review)).then(|| {
            crate::cost_router::reviewer_dispatch(
                session.provider_name.as_deref(),
                session.model_config.as_ref().map(|m| m.model_name.as_str()),
                role_model.as_ref().map(|(rm, _)| rm.clone()),
            )
        });
        let role_model = match &review_dispatch {
            // `reviewer_dispatch` passes a configured/derived pick through
            // unchanged and yields `None` for none, so the source travels with it.
            Some(dispatch) => dispatch
                .role_model
                .clone()
                .and_then(|rm| role_model.as_ref().map(|(_, source)| (rm, *source))),
            None => role_model,
        };
        // The routing receipt (#1090's pattern): one line naming the model and
        // WHY, at info — a delegate that changes model is a spend decision and
        // must not be discoverable only at debug. The same line is returned to the
        // caller in `DelegateRouting::receipt` and rides the tool result's `_meta`
        // as `model_routing`, so the status row and the ledger reader can show it.
        tracing::info!(
            target: "permagentd::brain",
            role = role.map(|r| r.as_str()).unwrap_or("none"),
            source = routing.source.as_str(),
            provider = routing
                .role_model
                .as_ref()
                .map(|rm| rm.provider.as_str())
                .unwrap_or_else(|| session.provider_name.as_deref().unwrap_or("session")),
            model = routing
                .role_model
                .as_ref()
                .map(|rm| rm.model.as_str())
                .unwrap_or_else(|| session
                    .model_config
                    .as_ref()
                    .map(|m| m.model_name.as_str())
                    .unwrap_or("session")),
            refused = routing.refused.as_ref().map(|(_, why)| why.as_str()),
            "{}",
            routing.receipt,
        );
        let role_model = role_model.map(|(rm, _)| rm);

        let recipe_provider = recipe
            .settings
            .as_ref()
            .and_then(|s| s.goose_provider.clone());

        // Resolve a CONSISTENT (provider, model) target across the precedence
        // chain. This never crosses a provider chosen at one level with a model
        // that belongs to a different provider (the cross-provider 404 in
        // robustness-audit F5.2), and it falls back off an explicitly-selected
        // provider whose key is unconfigured (the keyless mid-task dispatch error)
        // — both to the fully-consistent inherited session pair, with a warning.
        let target = reconcile_subagent_target(
            (params.provider.clone(), params.model.clone()),
            (
                recipe_provider.clone(),
                recipe.settings.as_ref().and_then(|s| s.goose_model.clone()),
            ),
            (
                role_model.as_ref().map(|rm| rm.provider.clone()),
                role_model.as_ref().map(|rm| rm.model.clone()),
            ),
            (
                Config::global()
                    .get_param::<String>("GOOSE_SUBAGENT_PROVIDER")
                    .ok(),
                Config::global()
                    .get_param::<String>("GOOSE_SUBAGENT_MODEL")
                    .ok(),
            ),
            session.provider_name.clone(),
            &|p| crate::cost_router::is_provider_configured(p),
        )?;
        if let Some(warning) = &target.warning {
            tracing::warn!(target: "permagentd::brain", "{}", warning);
        }
        // The review gate's diversity warning — only when the review is really
        // going where the warning says: no explicit param/recipe override named
        // the reviewer's provider or model (an operator's deliberate call is not
        // second-guessed), and the reconciled target is what the warning
        // describes — the inherited session model for the unset fallback, the
        // configured provider for the same-family notice.
        if let Some(warning) = review_dispatch.as_ref().and_then(|d| d.warning.as_deref()) {
            let explicit_override = params.provider.is_some()
                || params.model.is_some()
                || recipe_provider.is_some()
                || recipe
                    .settings
                    .as_ref()
                    .is_some_and(|s| s.goose_model.is_some());
            let target_matches = match &role_model {
                None => matches!(target.model, SubagentModel::InheritSession),
                Some(rm) => rm.provider == target.provider,
            };
            if !explicit_override && target_matches {
                tracing::warn!(target: "permagentd::brain", "review gate: {}", warning);
            }
        }
        let provider_name = target.provider.clone();

        // The role provider was applied only when no explicit param/recipe named
        // one AND the final provider is actually the role's (i.e. we did not fall
        // back off it) — the gate for the cache guard below.
        let role_provider_applied = params.provider.is_none()
            && recipe_provider.is_none()
            && role_model
                .as_ref()
                .is_some_and(|rm| rm.provider == provider_name);

        let mut model_config = session.model_config.clone().map(Ok).unwrap_or_else(|| {
            crate::model::ModelConfig::new("default")
                .map(|c| c.with_canonical_limits(&provider_name))
        })?;

        // Apply the reconciled model. `InheritSession` keeps the session model;
        // `Explicit` overrides it; `ProviderDefault` builds a fresh config on the
        // explicit provider's default rather than borrow the session's foreign model.
        match target.model {
            SubagentModel::InheritSession => {}
            SubagentModel::Explicit(model) => {
                model_config.model_name = model;
            }
            SubagentModel::ProviderDefault => {
                model_config = crate::model::ModelConfig::new("default")?
                    .with_canonical_limits(&provider_name);
            }
        }

        if let Some(temp) = params.temperature {
            model_config = model_config.with_temperature(Some(temp));
        } else if let Some(temp) = recipe.settings.as_ref().and_then(|s| s.temperature) {
            model_config = model_config.with_temperature(Some(temp));
        }

        let provider = providers::create(&provider_name, model_config, Vec::new()).await?;

        // Live cache guard (#730): a cache-heavy role (orchestrate/edit) routed by
        // the role map to a provider without prompt caching forfeits the
        // warm-prefix saving that makes the loop cheap. Warn — only when the role
        // map is what selected this provider (an explicit param/recipe override is
        // the operator's deliberate call, not something to second-guess).
        if let Some(role) = role {
            let supports_cache = provider.supports_cache_control().await;
            if crate::cost_router::cache_guard_should_warn(
                role,
                role_provider_applied,
                supports_cache,
            ) {
                tracing::warn!(
                    target: "permagentd::brain",
                    "cost-router cache guard: cache-heavy role '{}' routed to provider '{}' \
                     which has no prompt caching — this loop can't keep a warm cache; prefer a \
                     caching provider for the {} role",
                    role.as_str(),
                    provider_name,
                    role.as_str()
                );
            }
        }

        Ok((provider, routing))
    }

    fn resolve_max_turns(&self, session: &crate::session::Session) -> usize {
        session
            .recipe
            .as_ref()
            .and_then(|r| r.settings.as_ref())
            .and_then(|s| s.max_turns)
            .or_else(|| {
                std::env::var("GOOSE_SUBAGENT_MAX_TURNS")
                    .ok()
                    .and_then(|v| v.parse().ok())
            })
            .or_else(|| {
                Config::global()
                    .get_param::<usize>("GOOSE_SUBAGENT_MAX_TURNS")
                    .ok()
            })
            .unwrap_or(DEFAULT_SUBAGENT_MAX_TURNS)
    }

    async fn cleanup_completed_tasks(&self) {
        let finished: Vec<(String, BackgroundTask)> = {
            let mut tasks = self.background_tasks.lock().await;
            let ids: Vec<String> = tasks
                .iter()
                .filter(|(_, t)| t.handle.is_finished())
                .map(|(id, _)| id.clone())
                .collect();
            ids.into_iter()
                .filter_map(|id| tasks.remove(&id).map(|t| (id, t)))
                .collect()
        };

        let mut completed = self.completed_tasks.lock().await;

        for (id, task) in finished {
            let duration = task.started_at.elapsed();
            let turns_taken = task.turns.load(Ordering::Relaxed);

            let result = match task.handle.await {
                Ok(Ok(output)) => {
                    info!("Background task {} completed successfully", id);
                    Ok(output)
                }
                Ok(Err(e)) => {
                    warn!("Background task {} failed: {}", id, e);
                    Err(e.to_string())
                }
                Err(e) => {
                    warn!("Background task {} panicked: {}", id, e);
                    Err(format!("Task panicked: {}", e))
                }
            };

            completed.insert(
                id.clone(),
                CompletedTask {
                    id,
                    description: task.description,
                    result,
                    turns_taken,
                    duration,
                },
            );
        }
    }

    fn get_task_description(params: &DelegateParams) -> String {
        if let Some(source) = &params.source {
            if let Some(instructions) = &params.instructions {
                format!("{}: {}", source, truncate(instructions, 30))
            } else {
                source.clone()
            }
        } else if let Some(instructions) = &params.instructions {
            truncate(instructions, 40)
        } else {
            "Unknown task".to_string()
        }
    }

    async fn handle_async_delegate(
        &self,
        session_id: &str,
        params: DelegateParams,
    ) -> Result<(Vec<Content>, String), String> {
        let task_count = self.background_tasks.lock().await.len();
        let max_tasks = max_background_tasks();
        if task_count >= max_tasks {
            return Err(format!(
                "Maximum {} background tasks already running. Wait for completion or use sync mode.",
                max_tasks
            ));
        }

        let session = self
            .context
            .session_manager
            .get_session(session_id, false)
            .await
            .map_err(|e| format!("Failed to get session: {}", e))?;

        let working_dir = session.working_dir.clone();
        let recipe = self
            .build_delegate_recipe(&params, session_id, &working_dir)
            .await?;

        let (task_config, routing) = self
            .build_task_config(&params, &recipe, &session)
            .await
            .map_err(|e| format!("Failed to build task config: {}", e))?;

        let description = truncate(&Self::get_task_description(&params), 40);

        let persona_override = Self::resolve_worker_persona(params.worker_persona.as_deref());
        info!(
            target: "permagentd::brain",
            "Subagent spawned with worker persona: {}",
            params.worker_persona.as_deref().unwrap_or("(primary)")
        );

        // Subagents must use Auto until get_agent_messages forwards
        // ActionRequired messages to the parent. Until then, any mode
        // that requires approval will hang on the subagent's confirmation_rx.
        let agent_config = AgentRunnerConfig::new(
            self.context.session_manager.clone(),
            crate::config::permission::PermissionManager::instance(),
            None,
            GooseMode::Auto,
            true, // disable session naming for subagents
            crate::agents::GoosePlatform::GooseCli,
        );

        let subagent_session = self
            .context
            .session_manager
            .create_session(
                working_dir,
                description.clone(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .map_err(|e| format!("Failed to create subagent session: {}", e))?;

        let task_id = subagent_session.id.clone();

        let turns = Arc::new(AtomicU32::new(0));
        let last_activity = Arc::new(AtomicU64::new(current_epoch_millis()));

        let turns_clone = Arc::clone(&turns);
        let last_activity_clone = Arc::clone(&last_activity);

        let on_message: OnMessageCallback = Arc::new(move |_msg| {
            turns_clone.fetch_add(1, Ordering::Relaxed);
            last_activity_clone.store(current_epoch_millis(), Ordering::Relaxed);
        });

        let task_token = CancellationToken::new();
        let task_token_clone = task_token.clone();

        let notification_buffer = Arc::new(Mutex::new(Vec::new()));

        let (notif_tx, notif_rx) = tokio::sync::mpsc::unbounded_channel::<ServerNotification>();
        Self::spawn_notification_bridge(
            notif_rx,
            Arc::clone(&self.notification_subscribers),
            Arc::clone(&notification_buffer),
        );

        let handle = tokio::spawn(async move {
            run_subagent_task(SubagentRunParams {
                config: agent_config,
                recipe,
                task_config,
                return_last_only: true,
                session_id: subagent_session.id,
                cancellation_token: Some(task_token_clone),
                on_message: Some(on_message),
                notification_tx: Some(notif_tx),
                persona_override,
            })
            .await
        });

        let task = BackgroundTask {
            id: task_id.clone(),
            description: description.clone(),
            started_at: Instant::now(),
            turns,
            last_activity,
            handle,
            cancellation_token: task_token,
            notification_buffer,
        };

        self.background_tasks
            .lock()
            .await
            .insert(task_id.clone(), task);

        // The routing receipt on the start line: a background delegate spends money
        // on a model the caller never named, so the caller is told which one.
        let content = vec![Content::text(format!(
            "Task {} started in background: \"{}\"\n\
             {}\n\
             Continue with other work. When you need the result, use load(source: \"{}\").",
            task_id, description, routing.receipt, task_id
        ))];
        Ok((content, task_id))
    }
}

#[async_trait]
impl McpClientTrait for SummonClient {
    async fn list_tools(
        &self,
        session_id: &str,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        self.cleanup_completed_tasks().await;

        let is_subagent = self
            .context
            .session_manager
            .get_session(session_id, false)
            .await
            .map(|s| s.session_type == SessionType::SubAgent)
            .unwrap_or(false);

        // Select from the guarded superset so a tool that is not in
        // `all_possible_tools()` cannot ship at all (mirrors ext_manager).
        let mut tools = Self::all_possible_tools();

        if is_subagent {
            // Subagents must not recurse into further delegation.
            tools.retain(|t| t.name.as_ref() != "delegate" && t.name.as_ref() != "delegate_many");
        }

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
        cancellation_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        let session_id = &ctx.session_id;
        match name {
            "load" => match self.handle_load(session_id, arguments).await {
                Ok(result) => Ok(result),
                Err(error) => Ok(CallToolResult::error(vec![Content::text(format!(
                    "Error: {}",
                    error
                ))])),
            },
            "delegate_many" => {
                match self
                    .handle_delegate_many(session_id, arguments, cancellation_token)
                    .await
                {
                    Ok(result) => Ok(result),
                    Err(error) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {}",
                        error
                    ))])),
                }
            }
            "delegate" => {
                match self
                    .handle_delegate(session_id, arguments, cancellation_token)
                    .await
                {
                    Ok(result) => Ok(result),
                    Err(error) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "Error: {}",
                        error
                    ))])),
                }
            }
            _ => Ok(CallToolResult::error(vec![Content::text(format!(
                "Error: Unknown tool: {}",
                name
            ))])),
        }
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }

    async fn subscribe(&self) -> mpsc::Receiver<ServerNotification> {
        let (tx, rx) = mpsc::channel(16);
        self.notification_subscribers.lock().await.push(tx);
        rx
    }

    async fn get_moim(&self, _session_id: &str) -> Option<String> {
        self.cleanup_completed_tasks().await;

        let running = self.background_tasks.lock().await;
        let completed = self.completed_tasks.lock().await;

        if running.is_empty() && completed.is_empty() {
            return None;
        }

        let mut lines = vec!["Background tasks:".to_string()];
        let now = current_epoch_millis();

        let mut sorted_running: Vec<_> = running.values().collect();
        sorted_running.sort_by_key(|t| &t.id);

        for task in sorted_running {
            let elapsed = task.started_at.elapsed();
            let idle_ms = now.saturating_sub(task.last_activity.load(Ordering::Relaxed));

            lines.push(format!(
                "• {}: \"{}\" - running {}, {} turns, idle {}",
                task.id,
                task.description,
                round_duration(elapsed),
                task.turns.load(Ordering::Relaxed),
                round_duration(Duration::from_millis(idle_ms)),
            ));
        }

        let mut sorted_completed: Vec<_> = completed.values().collect();
        sorted_completed.sort_by_key(|t| &t.id);

        for task in sorted_completed {
            let status = if task.result.is_ok() {
                "completed"
            } else {
                "failed"
            };
            lines.push(format!(
                "• {}: \"{}\" - {} in {} ({} turns) - use load(\"{}\") to get result",
                task.id,
                task.description,
                status,
                round_duration(task.duration),
                task.turns_taken,
                task.id
            ));
        }

        if !running.is_empty() {
            lines.push(
                "\n→ Use load(source: \"<id>\") to wait for a task, or load(source: \"<id>\", cancel: true) to stop it"
                    .to_string(),
            );
        }

        Some(lines.join("\n"))
    }
}

/// Pure subagent PROVIDER precedence: explicit param → recipe setting → role map
/// → `GOOSE_SUBAGENT_PROVIDER` → inherited session provider. The objective role
/// layer (#730) slots after the recipe setting and before the env fallback; first
/// present wins. Extracted from `resolve_provider` so the precedence is
/// unit-testable without building a live session.
fn resolve_subagent_provider(
    param: Option<String>,
    recipe: Option<String>,
    role: Option<String>,
    subagent_env: Option<String>,
    session: Option<String>,
) -> Option<String> {
    param.or(recipe).or(role).or(subagent_env).or(session)
}

/// Pure subagent MODEL precedence: explicit param → recipe setting → role map →
/// `GOOSE_SUBAGENT_MODEL`. `None` ⇒ keep the inherited session model — the
/// single-model fallback, NEVER a baked-in vendor default. First present wins.
fn resolve_subagent_model(
    param: Option<String>,
    recipe: Option<String>,
    role: Option<String>,
    subagent_env: Option<String>,
) -> Option<String> {
    param.or(recipe).or(role).or(subagent_env)
}

/// A model decision for a resolved subagent provider, kept CONSISTENT with it.
#[derive(Debug, PartialEq)]
enum SubagentModel {
    /// Keep the inherited session model unchanged (the provider is the session's).
    InheritSession,
    /// Use this explicit model — declared for, or on, the resolved provider.
    Explicit(String),
    /// Use the resolved provider's DEFAULT model: an explicit provider was chosen
    /// without a model of its own, so borrowing the session's (foreign) model would
    /// 404. Never the session model.
    ProviderDefault,
}

/// A consistent subagent dispatch target: a provider and a model that belongs to
/// it, plus an optional operator-facing warning emitted when we fell back.
#[derive(Debug, PartialEq)]
struct SubagentTarget {
    provider: String,
    model: SubagentModel,
    warning: Option<String>,
}

/// Resolve a CONSISTENT subagent (provider, model) target across the precedence
/// sources — param → recipe → role → `GOOSE_SUBAGENT_*` → session — closing the
/// two robustness-audit F5.2 gaps that surface only mid-task:
///
/// - **Cross-provider model (404):** a provider chosen at one precedence level is
///   never paired with a model that belongs to a different provider. An explicitly
///   selected provider WITHOUT a model of its own resolves to that provider's
///   default ([`SubagentModel::ProviderDefault`]) — not the session's foreign
///   model (an Ollama model id sent to Anthropic, or a Claude id to Ollama).
/// - **Keyless provider:** an explicitly selected provider whose key is not
///   configured (e.g. a leftover `GOOSE_SUBAGENT_PROVIDER`) falls back to the
///   fully-consistent session pair rather than a keyless dispatch failure.
///
/// The provider/model precedence itself is unchanged (it reuses
/// [`resolve_subagent_provider`] / [`resolve_subagent_model`]); this only adds the
/// consistency + key reconciliation on top. Pure over an `is_provider_configured`
/// predicate (the live wrapper passes [`crate::cost_router::is_provider_configured`]),
/// so it is unit-testable without a registry or network.
fn reconcile_subagent_target(
    param: (Option<String>, Option<String>),
    recipe: (Option<String>, Option<String>),
    role: (Option<String>, Option<String>),
    env: (Option<String>, Option<String>),
    session_provider: Option<String>,
    is_provider_configured: &impl Fn(&str) -> bool,
) -> Result<SubagentTarget, anyhow::Error> {
    let provider = resolve_subagent_provider(
        param.0.clone(),
        recipe.0.clone(),
        role.0.clone(),
        env.0.clone(),
        session_provider.clone(),
    )
    .ok_or_else(|| anyhow::anyhow!("No provider configured"))?;

    // The first explicit model override across the sources, in precedence order.
    let explicit_model = || {
        resolve_subagent_model(
            param.1.clone(),
            recipe.1.clone(),
            role.1.clone(),
            env.1.clone(),
        )
    };

    // The highest-precedence source that named a provider, carrying the model THAT
    // source declared. Outer `None` ⇒ no explicit provider ⇒ inherited session.
    let winner_own_model: Option<Option<String>> = if param.0.is_some() {
        Some(param.1.clone())
    } else if recipe.0.is_some() {
        Some(recipe.1.clone())
    } else if role.0.is_some() {
        Some(role.1.clone())
    } else if env.0.is_some() {
        Some(env.1.clone())
    } else {
        None
    };

    let is_session = session_provider.as_deref() == Some(provider.as_str());

    // An explicitly-selected FOREIGN provider whose key is unconfigured → fall back
    // to the consistent session pair (or error if there is no session to fall to).
    if winner_own_model.is_some() && !is_session && !is_provider_configured(&provider) {
        return match session_provider {
            Some(session) => Ok(SubagentTarget {
                warning: Some(format!(
                    "subagent provider '{provider}' was selected but its API key is not configured \
                     — falling back to the session provider '{session}' to avoid a keyless mid-task \
                     dispatch failure"
                )),
                provider: session,
                model: SubagentModel::InheritSession,
            }),
            None => Err(anyhow::anyhow!(
                "subagent provider '{provider}' was selected but its API key is not configured, and \
                 there is no session provider to fall back to"
            )),
        };
    }

    let model = match winner_own_model {
        // No explicit provider → session provider; apply any explicit model override.
        None => explicit_model()
            .map(SubagentModel::Explicit)
            .unwrap_or(SubagentModel::InheritSession),
        // Explicit provider that declared its own model → consistent by construction.
        Some(Some(m)) => SubagentModel::Explicit(m),
        // Explicit provider == session provider, no own model → inherit / override.
        Some(None) if is_session => explicit_model()
            .map(SubagentModel::Explicit)
            .unwrap_or(SubagentModel::InheritSession),
        // Explicit FOREIGN provider, no own model → ITS default, never the session's.
        Some(None) => SubagentModel::ProviderDefault,
    };

    let warning = matches!(model, SubagentModel::ProviderDefault).then(|| {
        format!(
            "subagent provider '{provider}' was selected without a model — using that provider's \
             default model instead of the session model (which belongs to a different provider)"
        )
    });

    Ok(SubagentTarget {
        provider,
        model,
        warning,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::collections::HashSet;
    use std::fs;
    use std::sync::Arc;
    use tempfile::TempDir;

    /// The live call that started this: a whole background subagent spun up on
    /// the word "placeholder".
    #[test]
    fn placeholder_instructions_are_refused() {
        for filler in [
            "placeholder",
            "Placeholder",
            "  placeholder  ",
            "placeholder.",
            "TBD",
            "todo",
            "n/a",
            "...",
            "",
            "   ",
            "fix it",
        ] {
            assert!(
                SummonClient::unusable_instructions(filler).is_some(),
                "{filler:?} should not start a worker"
            );
        }
    }

    /// A real task must still get through, including a short but specific one.
    #[test]
    fn a_real_task_is_delegated() {
        for task in [
            "Run the voice latency benchmark and report the p95",
            "Update crates/goose/src/providers/zai.rs to map cached tokens",
            "Read PR 1101 and summarise what its classifier changed",
        ] {
            assert_eq!(
                SummonClient::unusable_instructions(task),
                None,
                "{task:?} should be delegated"
            );
        }
    }

    /// A recipe named by `source` carries its own prompt, so its instructions
    /// field is free to be a note rather than the whole task.
    #[test]
    fn the_guard_only_applies_to_ad_hoc_instructions() {
        let ext = SummonClient::new(create_test_context()).unwrap();
        let with_source = DelegateParams {
            instructions: Some("placeholder".to_string()),
            source: Some("some-recipe".to_string()),
            ..Default::default()
        };
        assert!(ext.validate_delegate_params(&with_source).is_ok());

        let ad_hoc = DelegateParams {
            instructions: Some("placeholder".to_string()),
            ..Default::default()
        };
        let error = ext
            .validate_delegate_params(&ad_hoc)
            .expect_err("ad-hoc placeholder instructions must be refused");
        assert!(error.contains("placeholder text"), "{error}");
        assert!(error.contains("Nothing was started"), "{error}");
    }

    fn create_test_context() -> PlatformExtensionContext {
        PlatformExtensionContext {
            extension_manager: None,
            session_manager: Arc::new(crate::session::SessionManager::instance()),
            session: None,
        }
    }

    // ── Role-router precedence (#730 wiring) ─────────────────────────────────
    // These exercise the pure precedence the role layer slots into, without a
    // live session. `s` is a terse Option<String> helper.
    fn s(v: &str) -> Option<String> {
        Some(v.to_string())
    }

    #[test]
    fn role_model_applies_when_no_param_or_recipe_names_one() {
        // Nothing explicit, a role mapping present → the role's model/provider win.
        assert_eq!(
            resolve_subagent_model(None, None, s("gpt-5.6"), s("subagent-fallback")),
            s("gpt-5.6"),
            "the role model must be used when no param/recipe overrides it"
        );
        assert_eq!(
            resolve_subagent_provider(None, None, s("openai"), s("anthropic"), s("session")),
            s("openai"),
        );
    }

    #[test]
    fn explicit_param_and_recipe_outrank_the_role_layer() {
        // An explicit delegate param wins over the role map…
        assert_eq!(
            resolve_subagent_model(s("param-model"), None, s("role-model"), None),
            s("param-model"),
        );
        // …and a recipe setting wins over the role map when no param is given.
        assert_eq!(
            resolve_subagent_model(None, s("recipe-model"), s("role-model"), None),
            s("recipe-model"),
        );
        assert_eq!(
            resolve_subagent_provider(None, s("recipe-prov"), s("role-prov"), s("env"), s("sess")),
            s("recipe-prov"),
        );
    }

    #[test]
    fn role_layer_outranks_the_env_subagent_fallback() {
        // The role map is a HIGHER precedence than GOOSE_SUBAGENT_* — the whole
        // point: a configured role routes even when a subagent env default exists.
        assert_eq!(
            resolve_subagent_model(None, None, s("role-model"), s("GOOSE_SUBAGENT_MODEL")),
            s("role-model"),
        );
    }

    #[test]
    fn unset_role_falls_through_to_single_model_not_a_baked_default() {
        // THE load-bearing guarantee at the dispatch precedence: with NO role
        // mapping (role = None), model resolution falls through to the env/session
        // fallback — and when that too is absent, `None` means "keep the inherited
        // session model". At no point does a tier-pack default (Opus/Sonnet/Haiku)
        // enter: it is not in the precedence chain at all.
        //
        // No role, no env → None ⇒ caller keeps the session model.
        assert_eq!(resolve_subagent_model(None, None, None, None), None);
        // No role, but an env subagent default → that env value (still the user's
        // choice), never a vendor pack.
        assert_eq!(
            resolve_subagent_model(None, None, None, s("user-env-default")),
            s("user-env-default"),
        );
        // Provider likewise falls through to the inherited session provider.
        assert_eq!(
            resolve_subagent_provider(None, None, None, None, s("session-provider")),
            s("session-provider"),
        );
        // Prove the negative directly: the pack default model is NEVER produced by
        // the precedence when nothing configures the role.
        let pack_default = crate::cost_router::packs::ModelPacks::default().hard.model;
        assert_eq!(pack_default.as_str(), "claude-opus-4-8"); // the baked pack exists…
        assert_ne!(
            resolve_subagent_model(None, None, None, s("user-env-default")),
            Some(pack_default),
            "unset role must not route to the tier-pack default"
        );
    }

    /// The DERIVED default at the role step (ruling 2026-08-18), asserted
    /// on PROVENANCE rather than a vendor name: with NO available providers the
    /// derived map is empty and the role step yields nothing — the fallthrough
    /// is still the session model; with available providers the derived pick is
    /// one of THEM (whatever they are), and it slots into the precedence exactly
    /// where a hand-configured role does — below param/recipe, above
    /// GOOSE_SUBAGENT_*/session.
    #[test]
    fn unset_role_derives_only_from_available_providers_else_session_model() {
        use crate::cost_router::{derive_role_map, AvailableModel, WorkflowRole};
        // No available (scorable) providers → nothing derived → session model.
        let empty = derive_role_map(&[]);
        let role_step = empty
            .get(WorkflowRole::Edit)
            .map(|(rm, _)| (Some(rm.provider.clone()), Some(rm.model.clone())))
            .unwrap_or((None, None));
        assert_eq!(role_step, (None, None));
        assert_eq!(
            resolve_subagent_provider(None, None, role_step.0, None, s("session-provider")),
            s("session-provider"),
        );
        // Available providers → the derived pick is one of them, by provenance.
        let available = vec![
            AvailableModel::new("acme-cloud", "unknown-to-kb"), // unscorable: never picked
            AvailableModel::new("google", "gemini-3-pro"),
            AvailableModel::new("ollama", "qwen3-coder:30b"),
        ];
        let derived = derive_role_map(&available);
        let (rm, _) = derived
            .get(WorkflowRole::Edit)
            .expect("a floor-clearing available model derives EDIT");
        assert!(
            available
                .iter()
                .any(|a| a.provider == rm.provider && a.model == rm.model),
            "derived {}/{} must be one of the user's available models",
            rm.provider,
            rm.model
        );
        // It slots in at the role step: below an explicit param, above env.
        assert_eq!(
            resolve_subagent_model(
                s("param-model"),
                None,
                s(&rm.model),
                s("GOOSE_SUBAGENT_MODEL")
            ),
            s("param-model"),
        );
        assert_eq!(
            resolve_subagent_model(None, None, s(&rm.model), s("GOOSE_SUBAGENT_MODEL")),
            s(&rm.model),
        );
        // The pack default is not "available" here, so by construction it is
        // not the derived pick — the assertion is on provenance, not the name.
        let pack_default = crate::cost_router::packs::ModelPacks::default().hard;
        assert!(!(rm.provider == pack_default.provider && rm.model == pack_default.model));
    }

    // ── Subagent target consistency (robustness-audit F5.2) ──────────────────
    // The reconciliation layer over the precedence chain: it must never cross a
    // provider from one source with a foreign model, and must fall back off a
    // keyless explicit provider. Pure, so an `is_provider_configured` closure
    // stands in for the registry.
    fn configured_all(_: &str) -> bool {
        true
    }
    fn none() -> (Option<String>, Option<String>) {
        (None, None)
    }

    /// THE finding: a `provider` override WITHOUT a `model` must NOT keep the
    /// session's (foreign) model — that is the cross-provider 404. It resolves to
    /// the override provider's default instead.
    #[test]
    fn provider_override_without_model_does_not_pair_a_foreign_model() {
        let target = reconcile_subagent_target(
            (s("ollama"), None), // param: provider override, no model
            none(),
            none(),
            none(),
            s("anthropic"), // session runs a claude model on anthropic
            &configured_all,
        )
        .unwrap();
        assert_eq!(target.provider, "ollama");
        assert_eq!(
            target.model,
            SubagentModel::ProviderDefault,
            "an override provider without a model must use ITS default, not the session's foreign model"
        );
        assert!(target.warning.is_some(), "the fallback must be surfaced");
    }

    /// The second F5.2 case: a leftover `GOOSE_SUBAGENT_PROVIDER` whose key is not
    /// configured falls back to the consistent session pair instead of a keyless
    /// mid-task dispatch failure.
    #[test]
    fn unconfigured_explicit_provider_falls_back_to_the_session_pair() {
        let target = reconcile_subagent_target(
            none(),
            none(),
            none(),
            (s("minimax"), None), // GOOSE_SUBAGENT_PROVIDER, no key
            s("anthropic"),
            &|p| p == "anthropic", // only the session provider is configured
        )
        .unwrap();
        assert_eq!(target.provider, "anthropic");
        assert_eq!(target.model, SubagentModel::InheritSession);
        assert!(target
            .warning
            .as_deref()
            .unwrap()
            .contains("key is not configured"));
    }

    /// #731 role routing is preserved: a role names BOTH provider and model, so the
    /// consistent pair is used as-is (no fallback, no warning).
    #[test]
    fn role_provider_and_model_stay_a_consistent_pair() {
        let target = reconcile_subagent_target(
            none(),
            none(),
            (s("openai"), s("gpt-5.6")),
            none(),
            s("anthropic"),
            &configured_all,
        )
        .unwrap();
        assert_eq!(target.provider, "openai");
        assert_eq!(target.model, SubagentModel::Explicit("gpt-5.6".to_string()));
        assert!(target.warning.is_none());
    }

    /// No overrides → inherit the exact session (provider, model) pair unchanged.
    #[test]
    fn no_overrides_inherits_the_session_pair() {
        let target = reconcile_subagent_target(
            none(),
            none(),
            none(),
            none(),
            s("anthropic"),
            &configured_all,
        )
        .unwrap();
        assert_eq!(target.provider, "anthropic");
        assert_eq!(target.model, SubagentModel::InheritSession);
        assert!(target.warning.is_none());
    }

    /// A model-only override (no provider) stays on the session provider — that is
    /// consistent, so it is NOT downgraded to a fallback.
    #[test]
    fn model_only_override_applies_to_the_session_provider() {
        let target = reconcile_subagent_target(
            (None, s("claude-opus-4-8")),
            none(),
            none(),
            none(),
            s("anthropic"),
            &configured_all,
        )
        .unwrap();
        assert_eq!(target.provider, "anthropic");
        assert_eq!(
            target.model,
            SubagentModel::Explicit("claude-opus-4-8".to_string())
        );
        assert!(target.warning.is_none());
    }

    /// An explicit provider that supplies its own model is used directly.
    #[test]
    fn explicit_provider_with_its_own_model_is_used_as_is() {
        let target = reconcile_subagent_target(
            (s("openai"), s("gpt-5.6")),
            none(),
            none(),
            none(),
            s("anthropic"),
            &configured_all,
        )
        .unwrap();
        assert_eq!(target.provider, "openai");
        assert_eq!(target.model, SubagentModel::Explicit("gpt-5.6".to_string()));
        assert!(target.warning.is_none());
    }

    #[test]
    fn test_agent_frontmatter_parsing() {
        let agent = r#"---
name: reviewer
model: sonnet
---
You review code."#;
        let source = parse_agent_content(agent, Path::new("")).unwrap();
        assert_eq!(source.name, "reviewer");
        assert!(source.description.contains("sonnet"));
    }

    #[test]
    fn test_agent_scan_skips_non_agent_markdown() {
        let temp_dir = TempDir::new().unwrap();
        let agents_dir = temp_dir.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("README.md"),
            "---\ntitle: Notes\n---\nThis is not an agent.",
        )
        .unwrap();
        fs::write(
            agents_dir.join("notes.md"),
            "---\nauthor: someone\ntags: [docs]\n---\nJust documentation.",
        )
        .unwrap();
        fs::write(
            agents_dir.join("reviewer.md"),
            "---\nname: reviewer\nmodel: sonnet\n---\nYou review code.",
        )
        .unwrap();
        fs::write(agents_dir.join("plain.md"), "No frontmatter at all.").unwrap();
        fs::write(
            agents_dir.join("broken.md"),
            "---\nname: [unterminated\n---\nBroken YAML.",
        )
        .unwrap();

        let mut sources = Vec::new();
        let mut seen = HashSet::new();
        scan_agents_from_dir(&agents_dir, &mut sources, &mut seen);

        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].name, "reviewer");
    }

    #[tokio::test]
    async fn test_discover_recipes_and_agents() {
        let temp_dir = TempDir::new().unwrap();

        let recipes = temp_dir.path().join(".goose/recipes");
        fs::create_dir_all(&recipes).unwrap();
        fs::write(
            recipes.join("deploy.yaml"),
            "title: Deploy\ndescription: Deploy to production\ninstructions: Run deploy steps",
        )
        .unwrap();

        let agents = temp_dir.path().join(".goose/agents");
        fs::create_dir_all(&agents).unwrap();
        fs::write(
            agents.join("reviewer.md"),
            "---\nname: reviewer\nmodel: sonnet\ndescription: Code reviewer\n---\nYou review code.",
        )
        .unwrap();

        let client = SummonClient::new(create_test_context()).unwrap();
        let sources = client.discover_filesystem_sources(temp_dir.path());

        let recipe = sources
            .iter()
            .find(|s| s.name == "deploy" && s.kind == SourceKind::Recipe)
            .unwrap();
        assert_eq!(recipe.description, "Deploy to production");
        assert_eq!(recipe.content, "Run deploy steps");

        let agent = sources
            .iter()
            .find(|s| s.name == "reviewer" && s.kind == SourceKind::Agent)
            .unwrap();
        assert_eq!(agent.description, "Code reviewer");
        assert!(agent.content.contains("You review code"));
    }

    #[tokio::test]
    async fn test_recipe_deduplication_local_wins() {
        let temp_dir = TempDir::new().unwrap();

        let local = temp_dir.path().join(".goose/recipes");
        fs::create_dir_all(&local).unwrap();
        fs::write(
            local.join("deploy.yaml"),
            "title: Deploy\ndescription: Local deploy\ninstructions: local steps",
        )
        .unwrap();

        let also_local = temp_dir.path().join(".agents/recipes");
        fs::create_dir_all(&also_local).unwrap();
        fs::write(
            also_local.join("deploy.yaml"),
            "title: Deploy\ndescription: Agents deploy\ninstructions: agents steps",
        )
        .unwrap();

        let client = SummonClient::new(create_test_context()).unwrap();
        let sources = client.discover_filesystem_sources(temp_dir.path());

        let deploys: Vec<_> = sources.iter().filter(|s| s.name == "deploy").collect();
        assert_eq!(deploys.len(), 1);
    }

    #[tokio::test]
    async fn test_load_recipe_source() {
        let temp_dir = TempDir::new().unwrap();

        let recipes = temp_dir.path().join(".goose/recipes");
        fs::create_dir_all(&recipes).unwrap();
        fs::write(
            recipes.join("deploy.yaml"),
            "title: Deploy\ndescription: Deploy to production\ninstructions: Run deploy steps",
        )
        .unwrap();

        let client = SummonClient::new(create_test_context()).unwrap();
        let result = client
            .handle_load_source("test", "deploy", temp_dir.path())
            .await
            .unwrap();

        let text = &result[0].as_text().expect("expected text content").text;
        assert!(text.contains("deploy"));
        assert!(text.contains("Run deploy steps"));
        assert!(text.contains("now available in your context"));
    }

    #[tokio::test]
    async fn test_load_agent_source() {
        let temp_dir = TempDir::new().unwrap();

        let agents = temp_dir.path().join(".goose/agents");
        fs::create_dir_all(&agents).unwrap();
        fs::write(
            agents.join("reviewer.md"),
            "---\nname: reviewer\nmodel: sonnet\ndescription: Code reviewer\n---\nYou review code carefully.",
        )
        .unwrap();

        let client = SummonClient::new(create_test_context()).unwrap();
        let result = client
            .handle_load_source("test", "reviewer", temp_dir.path())
            .await
            .unwrap();

        let text = &result[0].as_text().expect("expected text content").text;
        assert!(text.contains("reviewer"));
        assert!(text.contains("You review code carefully"));
        assert!(text.contains("now available in your context"));
    }

    #[tokio::test]
    async fn test_load_nonexistent_source_suggests_similar() {
        let temp_dir = TempDir::new().unwrap();

        let recipes = temp_dir.path().join(".goose/recipes");
        fs::create_dir_all(&recipes).unwrap();
        fs::write(
            recipes.join("deploy.yaml"),
            "title: Deploy\ndescription: Deploy to production\ninstructions: steps",
        )
        .unwrap();

        let client = SummonClient::new(create_test_context()).unwrap();
        let err = client
            .handle_load_source("test", "deploy-prod", temp_dir.path())
            .await
            .unwrap_err();

        assert!(err.contains("not found"));
        assert!(err.contains("deploy"), "should suggest 'deploy': {}", err);
    }

    #[tokio::test]
    async fn test_load_completely_unknown_source() {
        let temp_dir = TempDir::new().unwrap();

        let client = SummonClient::new(create_test_context()).unwrap();
        let err = client
            .handle_load_source("test", "zzz-nonexistent", temp_dir.path())
            .await
            .unwrap_err();

        assert!(err.contains("not found"));
        assert!(err.contains("Use load()"));
    }

    #[tokio::test]
    async fn test_client_tools_and_unknown_tool() {
        let client = SummonClient::new(create_test_context()).unwrap();

        let result = client
            .list_tools("test", None, CancellationToken::new())
            .await
            .unwrap();
        let names: Vec<_> = result.tools.iter().map(|t| t.name.as_ref()).collect();
        assert!(names.contains(&"load") && names.contains(&"delegate"));
        assert!(
            names.contains(&"delegate_many"),
            "the bounded fan-out must be offered next to the single delegate"
        );

        let ctx = ToolCallContext::new("test".to_string(), None, None);
        let result = client
            .call_tool(&ctx, "unknown", None, CancellationToken::new())
            .await
            .unwrap();
        assert!(result.is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn delegate_many_refuses_more_children_than_the_cap() {
        // Resolution happens before any child runs, so a fan-out that is too
        // wide is refused without having paid for the ones that fit.
        let client = SummonClient::new(create_test_context()).unwrap();
        let tasks: Vec<serde_json::Value> = (0..(fanout::MAX_FANOUT_CHILDREN + 1))
            .map(|i| {
                serde_json::json!({
                    "instructions": format!("review the {i}th module for swallowed errors"),
                })
            })
            .collect();
        let args = serde_json::json!({ "tasks": tasks })
            .as_object()
            .unwrap()
            .clone();

        let err = client
            .handle_delegate_many("test", Some(args), CancellationToken::new())
            .await
            .expect_err("a fan-out past the cap must be refused");
        assert!(err.contains("at most"), "{err}");
    }

    #[tokio::test]
    async fn delegate_many_refuses_an_empty_fan_out() {
        let client = SummonClient::new(create_test_context()).unwrap();
        let args = serde_json::json!({ "tasks": [] })
            .as_object()
            .unwrap()
            .clone();
        let err = client
            .handle_delegate_many("test", Some(args), CancellationToken::new())
            .await
            .expect_err("zero children is not a fan-out");
        assert!(err.contains("at least one child"), "{err}");
    }

    #[test]
    fn a_child_carries_the_same_routing_knobs_as_a_single_delegate() {
        // The per-child provider/model must survive parsing, because they are
        // level 1 of `cost_router::delegate`'s precedence — the operator's
        // explicit pin, and the thing a fan-out most easily loses.
        let parsed: DelegateManyParams = serde_json::from_value(serde_json::json!({
            "max_concurrent": 2,
            "tasks": [
                {"instructions": "audit the auth paths for authz gaps", "label": "security",
                 "provider": "ollama", "model": "qwen3-coder", "async": true},
                {"instructions": "look for swallowed errors and missing tests", "worker_persona": "debugger"}
            ]
        }))
        .unwrap();

        assert_eq!(parsed.tasks.len(), 2);
        assert_eq!(parsed.max_concurrent, Some(2));
        assert_eq!(parsed.tasks[0].label.as_deref(), Some("security"));
        assert_eq!(parsed.tasks[0].delegate.provider.as_deref(), Some("ollama"));
        assert_eq!(parsed.tasks[0].delegate.model.as_deref(), Some("qwen3-coder"));
        assert_eq!(
            parsed.tasks[1].delegate.worker_persona.as_deref(),
            Some("debugger")
        );
    }

    #[test]
    fn test_duration_rounding_for_moim() {
        assert_eq!(round_duration(Duration::from_secs(5)), "0s");
        assert_eq!(round_duration(Duration::from_secs(15)), "10s");
        assert_eq!(round_duration(Duration::from_secs(59)), "50s");

        assert_eq!(round_duration(Duration::from_secs(60)), "1m");
        assert_eq!(round_duration(Duration::from_secs(90)), "1m");
        assert_eq!(round_duration(Duration::from_secs(120)), "2m");
    }

    #[test]
    fn test_task_description_formatting() {
        let make_params = |source: Option<&str>, instructions: Option<&str>| DelegateParams {
            source: source.map(String::from),
            instructions: instructions.map(String::from),
            ..Default::default()
        };

        assert_eq!(
            SummonClient::get_task_description(&make_params(Some("recipe"), None)),
            "recipe"
        );
        assert_eq!(
            SummonClient::get_task_description(&make_params(None, Some("do stuff"))),
            "do stuff"
        );
        assert_eq!(
            SummonClient::get_task_description(&make_params(Some("r"), Some("task"))),
            "r: task"
        );

        let long = "x".repeat(100);
        let desc = SummonClient::get_task_description(&make_params(None, Some(&long)));
        assert!(desc.len() <= 43 && desc.ends_with("..."));
    }

    #[test]
    fn test_validate_delegate_params_rejects_zero_max_turns() {
        let context = create_test_context();
        let client = SummonClient::new(context).unwrap();

        let params = DelegateParams {
            instructions: Some("run the full test suite and report failures".to_string()),
            max_turns: Some(0),
            ..Default::default()
        };
        let result = client.validate_delegate_params(&params);
        assert_eq!(result, Err("'max_turns' must be at least 1".to_string()));
    }

    #[test]
    fn test_validate_delegate_params_accepts_positive_max_turns() {
        let context = create_test_context();
        let client = SummonClient::new(context).unwrap();

        let params = DelegateParams {
            instructions: Some("run the full test suite and report failures".to_string()),
            max_turns: Some(5),
            ..Default::default()
        };
        assert!(client.validate_delegate_params(&params).is_ok());
    }

    #[test]
    #[serial]
    fn test_resolve_max_turns_recipe_overrides_env_var() {
        let context = create_test_context();
        let client = SummonClient::new(context).unwrap();

        let session = crate::session::Session {
            recipe: Some(crate::recipe::Recipe {
                version: "1.0.0".to_string(),
                title: String::new(),
                description: String::new(),
                instructions: None,
                prompt: None,
                extensions: None,
                settings: Some(crate::recipe::Settings {
                    goose_provider: None,
                    goose_model: None,
                    temperature: None,
                    max_turns: Some(10),
                }),
                activities: None,
                author: None,
                parameters: None,
                response: None,
                sub_recipes: None,
                retry: None,
            }),
            ..Default::default()
        };

        // Set env var to a different value — recipe should still win
        std::env::set_var("GOOSE_SUBAGENT_MAX_TURNS", "99");
        let result = client.resolve_max_turns(&session);
        std::env::remove_var("GOOSE_SUBAGENT_MAX_TURNS");

        assert_eq!(
            result, 10,
            "recipe settings.max_turns should take priority over env var"
        );
    }

    #[test]
    #[serial]
    fn test_resolve_max_turns_falls_back_to_env_var() {
        let context = create_test_context();
        let client = SummonClient::new(context).unwrap();

        let session = crate::session::Session::default(); // no recipe

        std::env::set_var("GOOSE_SUBAGENT_MAX_TURNS", "7");
        let result = client.resolve_max_turns(&session);
        std::env::remove_var("GOOSE_SUBAGENT_MAX_TURNS");

        assert_eq!(
            result, 7,
            "should fall back to GOOSE_SUBAGENT_MAX_TURNS env var"
        );
    }

    #[test]
    #[serial]
    fn test_resolve_max_turns_falls_back_to_default() {
        let context = create_test_context();
        let client = SummonClient::new(context).unwrap();

        let session = crate::session::Session::default(); // no recipe

        std::env::remove_var("GOOSE_SUBAGENT_MAX_TURNS");
        let result = client.resolve_max_turns(&session);

        assert_eq!(
            result,
            crate::agents::subagent_task_config::DEFAULT_SUBAGENT_MAX_TURNS,
            "should fall back to DEFAULT_SUBAGENT_MAX_TURNS"
        );
    }

    fn extract_text(content: &Content) -> &str {
        use rmcp::model::RawContent;
        match &content.raw {
            RawContent::Text(t) => t.text.as_str(),
            _ => panic!("Expected text content"),
        }
    }

    #[test]
    fn test_is_session_id() {
        assert!(is_session_id("20260204_1"));
        assert!(is_session_id("20260204_42"));
        assert!(is_session_id("20260204_999"));
        assert!(!is_session_id("task_12345_0001"));
        assert!(!is_session_id("my-recipe"));
        assert!(!is_session_id("2026020_1"));
        assert!(!is_session_id("20260204"));
    }

    #[tokio::test]
    async fn test_async_task_result_lifecycle() {
        let client = SummonClient::new(create_test_context()).unwrap();
        let temp_dir = TempDir::new().unwrap();

        let result = client.handle_load_task_result("20260204_999", false).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));

        {
            use crate::agents::subagent_handler::create_tool_notification;
            use crate::conversation::message::MessageContent;
            use rmcp::model::CallToolRequestParams;

            let tool_call = CallToolRequestParams::new("developer__shell").with_arguments(
                serde_json::json!({"command": "ls"})
                    .as_object()
                    .unwrap()
                    .clone(),
            );
            let content = MessageContent::tool_request("req1", Ok(tool_call));
            let notif = create_tool_notification(&content, "20260204_1").unwrap();

            let buffer = Arc::new(Mutex::new(vec![notif]));

            let mut running = client.background_tasks.lock().await;
            running.insert(
                "20260204_1".to_string(),
                BackgroundTask {
                    id: "20260204_1".to_string(),
                    description: "Running task".to_string(),
                    started_at: Instant::now(),
                    turns: Arc::new(AtomicU32::new(2)),
                    last_activity: Arc::new(AtomicU64::new(current_epoch_millis())),
                    handle: tokio::spawn(async {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        Ok("done".to_string())
                    }),
                    cancellation_token: CancellationToken::new(),
                    notification_buffer: buffer,
                },
            );
        }

        let mut subscriber = client.subscribe().await;

        let result = client
            .handle_load_task_result("20260204_1", false)
            .await
            .expect("load should wait and return result");
        let text = extract_text(&result[0]);
        assert!(text.contains("Completed"));
        assert!(text.contains("done"));

        let notif = subscriber
            .try_recv()
            .expect("subscriber should receive buffered notification");
        if let ServerNotification::LoggingMessageNotification(log) = notif {
            let data = log.params.data.as_object().unwrap();
            assert_eq!(
                data.get("subagent_id").and_then(|v| v.as_str()),
                Some("20260204_1")
            );
        } else {
            panic!("expected logging notification");
        }

        {
            let mut completed = client.completed_tasks.lock().await;
            completed.insert(
                "20260204_2".to_string(),
                CompletedTask {
                    id: "20260204_2".to_string(),
                    description: "Successful task".to_string(),
                    result: Ok("Task completed successfully with output".to_string()),
                    turns_taken: 5,
                    duration: Duration::from_secs(60),
                },
            );
            completed.insert(
                "20260204_3".to_string(),
                CompletedTask {
                    id: "20260204_3".to_string(),
                    description: "Failed task".to_string(),
                    result: Err("Something went wrong".to_string()),
                    turns_taken: 3,
                    duration: Duration::from_secs(30),
                },
            );
        }

        let moim = client.get_moim("test").await.unwrap();
        assert!(moim.contains("20260204_2"));
        assert!(moim.contains("20260204_3"));
        assert!(moim.contains(r#"use load("20260204_2") to get result"#));
        assert!(moim.contains(r#"use load("20260204_3") to get result"#));

        let discovery = client
            .handle_load_discovery("test", temp_dir.path())
            .await
            .unwrap();
        let discovery_text = extract_text(&discovery[0]);
        assert!(discovery_text.contains("Completed Tasks (awaiting retrieval)"));
        assert!(discovery_text.contains("20260204_2"));
        assert!(discovery_text.contains("20260204_3"));

        let result = client
            .handle_load_task_result("20260204_2", false)
            .await
            .unwrap();
        let text = extract_text(&result[0]);
        assert!(text.contains("20260204_2"));
        assert!(text.contains("Successful task"));
        assert!(text.contains("✓ Completed"));
        assert!(text.contains("1m"));
        assert!(text.contains("5 turns"));
        assert!(text.contains("Task completed successfully with output"));

        assert!(!client
            .completed_tasks
            .lock()
            .await
            .contains_key("20260204_2"));

        let result = client
            .handle_load_task_result("20260204_3", false)
            .await
            .unwrap();
        let text = extract_text(&result[0]);
        assert!(text.contains("✗ Failed"));
        assert!(text.contains("Error: Something went wrong"));

        let result = client.handle_load_task_result("20260204_3", false).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));

        // All tasks consumed -- moim should be empty
        assert!(client.get_moim("test").await.is_none());
    }

    #[tokio::test]
    async fn test_cancel_running_task() {
        let client = SummonClient::new(create_test_context()).unwrap();
        let token = CancellationToken::new();

        {
            let mut running = client.background_tasks.lock().await;
            running.insert(
                "20260204_1".to_string(),
                BackgroundTask {
                    id: "20260204_1".to_string(),
                    description: "Cancellable task".to_string(),
                    started_at: Instant::now(),
                    turns: Arc::new(AtomicU32::new(3)),
                    last_activity: Arc::new(AtomicU64::new(current_epoch_millis())),
                    handle: tokio::spawn(async {
                        tokio::time::sleep(Duration::from_secs(1000)).await;
                        Ok("should not see this".to_string())
                    }),
                    cancellation_token: token.clone(),
                    notification_buffer: Arc::new(Mutex::new(Vec::new())),
                },
            );
        }

        let result = client
            .handle_load_task_result("20260204_1", true)
            .await
            .unwrap();
        let text = extract_text(&result[0]);
        assert!(text.contains("Cancelled"));
        assert!(text.contains("20260204_1"));
        assert!(text.contains("Cancellable task"));
        assert!(token.is_cancelled());
        assert!(!client
            .background_tasks
            .lock()
            .await
            .contains_key("20260204_1"));
    }
}
