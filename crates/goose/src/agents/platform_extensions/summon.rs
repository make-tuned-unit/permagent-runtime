use super::{orchestrator, parse_frontmatter, Source, SourceKind};
use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::platform_extensions::fanout;
use crate::agents::subagent_handler::{run_subagent_task, OnMessageCallback, SubagentRunParams};
use crate::agents::subagent_task_config::{TaskConfig, DEFAULT_SUBAGENT_MAX_TURNS};
use crate::agents::tool_execution::ToolCallContext;
use crate::agents::{AgentRunnerConfig, ExtensionConfig};
use crate::config::paths::Paths;
use crate::config::{narrow_extensions_for_agent, Config, GooseMode};
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
    /// X-dag-1: set when this child's tools were restricted to non-mutating
    /// ones because the fanning-out session has an active roadmap of its
    /// own. Prepended to the child's result text so the restriction is
    /// visible in-band, not only in `_meta`.
    roadmap_gate_note: Option<String>,
    /// G5 finding 2: `worker_persona: "reviewer"` (case-insensitive), same
    /// test `handle_delegate` uses. Drives the fan-out closure's
    /// snapshot-before/diff-after reviewer-mutation backstop — before this
    /// field existed, a `delegate_many` child never got that check at all,
    /// so a coordinator could route a reviewer through the fan-out path to
    /// bypass X-verify-1 entirely.
    is_reviewer_persona: bool,
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

        if let Some(refusal) = self.live_goal_delegate_refusal(&session).await {
            return Err(refusal);
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
            let (task_config, routing_receipt, roadmap_gate_note) = self
                .build_task_config(&child, &recipe, &session)
                .await
                .map_err(|e| {
                    format!("child {index} ({label}): failed to build task config: {e}")
                })?;

            let is_reviewer_persona = child
                .worker_persona
                .as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case("reviewer"));

            prepared.push(PreparedChild {
                label,
                working_dir: working_dir.clone(),
                recipe,
                task_config,
                routing_receipt,
                persona_override: Self::resolve_worker_persona(child.worker_persona.as_deref()),
                roadmap_gate_note,
                is_reviewer_persona,
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
        let parent_session_id = session_id.to_string();
        let outcomes = fanout::run_bounded(
            prepared,
            concurrency,
            cancellation_token,
            move |index, child, token| {
                let session_manager = session_manager.clone();
                let subscribers = Arc::clone(&subscribers);
                let parent_session_id = parent_session_id.clone();
                async move {
                    let PreparedChild {
                        label,
                        working_dir,
                        recipe,
                        task_config,
                        routing_receipt,
                        persona_override,
                        roadmap_gate_note,
                        is_reviewer_persona,
                    } = child;

                    // G5 finding 2: snapshot the working tree BEFORE this
                    // child runs when it is a reviewer persona — the exact
                    // check `handle_delegate` already does for the single-
                    // call path, now shared via
                    // `detect_and_record_reviewer_mutation` so a
                    // `delegate_many` reviewer child cannot route around it.
                    let reviewer_snapshot_dir = working_dir.clone();
                    let reviewer_before_snapshot = if is_reviewer_persona {
                        Some(
                            crate::agents::platform_extensions::reviewer_integrity::snapshot(
                                &reviewer_snapshot_dir,
                            )
                            .await,
                        )
                    } else {
                        None
                    };

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
                    // Lane R63c: counted per child actually spawned, not per
                    // child resolved (a later child's bad parameters refuse
                    // the whole fan-out before any of them runs).
                    record_delegate_dispatch(&routing_receipt);

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

                    // G5 finding 2: the subagent has now run to completion —
                    // diff the post-run snapshot against the pre-run one
                    // captured above (if any) and record the mutation
                    // receipt, exactly what `handle_delegate` does for the
                    // single-call path, via the SAME shared helper.
                    let mutated_paths = if let Some(before) = &reviewer_before_snapshot {
                        Self::detect_and_record_reviewer_mutation(
                            &session_manager,
                            &parent_session_id,
                            &reviewer_snapshot_dir,
                            before,
                            &subagent_id,
                        )
                        .await
                    } else {
                        None
                    };

                    let (status, text) = match result {
                        Ok(text) => {
                            let text = match &roadmap_gate_note {
                                Some(note) => format!("{note}\n\n{text}"),
                                None => text,
                            };
                            (fanout::ChildStatus::Ok, text)
                        }
                        Err(e) if token.is_cancelled() => {
                            (fanout::ChildStatus::Cancelled, format!("cancelled: {e}"))
                        }
                        Err(e) => (
                            fanout::ChildStatus::Failed,
                            format!("delegation failed: {e}"),
                        ),
                    };
                    // A mutation voids the verdict regardless of whether the
                    // child's own call succeeded or errored (same rule as
                    // the single `delegate` path) — the aggregate `status`
                    // is left as-is (matching `handle_delegate`, which
                    // returns `CallToolResult::success` even for a voided
                    // reviewer), and only the returned text carries the void
                    // signal a coordinator actually reads.
                    let text = match &mutated_paths {
                        Some(paths) => crate::after_turn::reviewer_mutation_void_text(paths, &text),
                        None => text,
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

        let aggregate_status = fanout::aggregate_status(&outcomes);
        let mut meta = Meta::new();
        meta.0.insert(
            "status".to_string(),
            serde_json::Value::String(aggregate_status.as_str().to_string()),
        );
        meta.0.insert(
            "fanout_children".to_string(),
            serde_json::Value::Array(children_meta),
        );

        let content = vec![Content::text(fanout::render_outcomes(&outcomes, &costs))];
        let result = match aggregate_status {
            fanout::ChildStatus::Ok => CallToolResult::success(content),
            fanout::ChildStatus::Failed | fanout::ChildStatus::Cancelled => {
                CallToolResult::error(content)
            }
        };
        Ok(result.with_meta(Some(meta)))
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

    // DAG-enforcement gap recorded, not fixed (harness loop R3,
    // HARNESS_LOOP_LOG.md "Lane M2 report" Defect 3): `delegate` dispatches a
    // real subagent unconditionally -- it does not require, and cannot
    // check, an approved `orchestrator` roadmap node the way
    // `create_roadmap` requires an answered Decision Inbox approval (see
    // `orchestrator::handle_create_roadmap`). A coordinator that gives up on
    // the roadmap pipeline can call this directly and it will dispatch real,
    // unreviewed work -- observed live in Lane M2's one PASS
    // (`harness-editapply-duplicate-discount-blocks`): 7 failed
    // `decompose_roadmap` attempts, one denied direct `edit`, then
    // `delegate source: codex` succeeded immediately with no roadmap ever
    // created. Whether `delegate` should also require an approved roadmap
    // node, or stays a deliberate escape hatch for mechanical sub-tasks, is
    // a design decision for the user -- not picked unilaterally here.
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

        if let Some(refusal) = self.live_goal_delegate_refusal(&session).await {
            return Err(refusal);
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

        let (task_config, routing_receipt, roadmap_gate_note) = self
            .build_task_config(&params, &recipe, &session)
            .await
            .map_err(|e| format!("Failed to build task config: {}", e))?;

        // Lane R36b: captured now, before `task_config` moves into the spawn
        // below, so a `worker_persona: "reviewer"` sub-session that fails
        // with a provider-unavailable-class error can retry once on the
        // author's own (provider, model) pair — same extensions/max_turns,
        // only the provider swapped (`TaskConfig` IS the existing profile/
        // override mechanism, never a new spawn path). Cheap even when
        // unused (an `Arc` clone plus a small extension list).
        let dispatch_provider_id = task_config.provider.get_name().to_string();
        let dispatch_model_id = task_config.provider.get_model_config().model_name.clone();
        let task_config_for_retry = task_config.clone();

        // X-verify-1: a `worker_persona: "reviewer"` subagent is supposed to
        // be READ-ONLY (F2/H1's tool-allowlist restriction), but Lane A2's
        // live evidence showed a reviewer subagent calling `edit` anyway, and
        // Lane G's review found the reviewer still keeps `shell` — a tool
        // allowlist alone is not proof. Snapshot the working tree now, before
        // the subagent runs at all, so it can be compared against the
        // post-run snapshot below regardless of which tool the mutation went
        // through.
        let is_reviewer_persona = params
            .worker_persona
            .as_deref()
            .is_some_and(|p| p.eq_ignore_ascii_case("reviewer"));
        let reviewer_snapshot_dir = working_dir.clone();
        let reviewer_before_snapshot = if is_reviewer_persona {
            Some(
                crate::agents::platform_extensions::reviewer_integrity::snapshot(
                    &reviewer_snapshot_dir,
                )
                .await,
            )
        } else {
            None
        };

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

        // Lane R63c: this child is about to run a model — count it on
        // `receipt cost_routing` with the audit `resolve_provider` computed.
        record_delegate_dispatch(&routing_receipt);
        let (result, subagent_session_id) = self
            .dispatch_subagent_once(
                agent_config.clone(),
                recipe.clone(),
                task_config,
                session_id,
                working_dir.clone(),
                cancellation_token.clone(),
                persona_override.clone(),
            )
            .await?;

        // Lane R36b: closes the gap R36a's call-site audit flagged — this
        // real sub-session had no breaker check and no fallback at all when
        // its own completion call failed (the M33 record's second,
        // unrecovered `CreditsExhausted` hit, ~70-100s into a run with no
        // paired "escalating" line). One retry, on the author's own
        // (provider, model) pair, through the SAME `dispatch_subagent_once`
        // spawn path above — never a new one.
        let (result, subagent_session_id) = if is_reviewer_persona {
            self.retry_reviewer_dispatch_if_unavailable(
                result,
                subagent_session_id,
                session_id,
                &session,
                &dispatch_provider_id,
                &dispatch_model_id,
                task_config_for_retry,
                agent_config,
                recipe,
                working_dir,
                cancellation_token,
                persona_override,
            )
            .await
        } else {
            (result, subagent_session_id)
        };

        let mut meta = Meta::new();
        meta.0.insert(
            "subagent_session_id".to_string(),
            serde_json::Value::String(subagent_session_id.clone()),
        );
        // The routing receipt travels with the result so a reader can say which
        // model this subagent's tokens were billed to, and why it was that one.
        meta.0.insert("model_routing".to_string(), routing_receipt);

        // X-verify-1: the subagent has now run to completion — compare the
        // post-run snapshot against the pre-run one captured above. A
        // difference here is independent of whatever tool the reviewer used
        // (the F2 allowlist restriction, or its absence, is irrelevant to
        // this check) and independent of what the reviewer's own text
        // claims. G5 finding 2: this is now the SAME shared helper
        // `handle_delegate_many`'s fan-out closure calls, so the two
        // reviewer-dispatch paths cannot drift apart again.
        let mutated_paths = if let Some(before) = &reviewer_before_snapshot {
            Self::detect_and_record_reviewer_mutation(
                &self.context.session_manager,
                session_id,
                &reviewer_snapshot_dir,
                before,
                &subagent_session_id,
            )
            .await
        } else {
            None
        };

        // (b) A mutation voids the verdict regardless of whether the
        // subagent's own call succeeded or errored — either way its text
        // must not be trusted — so first collapse `result` to one text
        // string (keeping whether it was originally an error), then decide.
        // X-dag-1 kept: a restricted delegate must say so in the text a
        // model actually reads — the routing receipt above only ever rode in
        // `_meta`, which is exactly why the earlier `model_routing` note was
        // invisible to the coordinator in live transcripts (Lane S/T
        // evidence). The roadmap-gate note is prepended first, on the
        // success path only, so a subsequent X-verify-1 void (below) still
        // covers it.
        let (original_ok, original_text) = match result {
            Ok(text) => {
                let text = match &roadmap_gate_note {
                    Some(note) => format!("{note}\n\n{text}"),
                    None => text,
                };
                (true, text)
            }
            Err(e) => (false, format!("Delegation failed: {}", e)),
        };

        match &mutated_paths {
            // (c) The mutated paths must be visible in the returned result
            // text, on top of the receipt already recorded above.
            Some(paths) => {
                let text = crate::after_turn::reviewer_mutation_void_text(paths, &original_text);
                Ok(CallToolResult::success(vec![Content::text(text)]).with_meta(Some(meta)))
            }
            None if original_ok => {
                Ok(CallToolResult::success(vec![Content::text(original_text)])
                    .with_meta(Some(meta)))
            }
            None => {
                Ok(CallToolResult::error(vec![Content::text(original_text)]).with_meta(Some(meta)))
            }
        }
    }

    /// G5 finding 2: diff a reviewer-persona child's pre-run tree snapshot
    /// against a fresh post-run snapshot of the same working directory and,
    /// if anything changed, record the mutation receipt on the PARENT
    /// session. This is the ONE place that decides what "the reviewer
    /// mutated the tree" means and how it gets recorded — shared by
    /// `handle_delegate` (the single-call reviewer path X-verify-1
    /// originally wired this into) and `handle_delegate_many`'s fan-out
    /// closure (which had no such backstop at all before this fix, so a
    /// coordinator could route a `worker_persona: "reviewer"` child through
    /// `delegate_many` to bypass X-verify-1 entirely). Returns the mutated
    /// paths (`None` for an empty diff) so the caller can void the child's
    /// own result text via `after_turn::reviewer_mutation_void_text`.
    async fn detect_and_record_reviewer_mutation(
        session_manager: &crate::session::SessionManager,
        parent_session_id: &str,
        working_dir: &Path,
        before: &crate::agents::platform_extensions::reviewer_integrity::TreeSnapshot,
        subagent_session_id: &str,
    ) -> Option<Vec<String>> {
        let after =
            crate::agents::platform_extensions::reviewer_integrity::snapshot(working_dir).await;
        let paths = crate::agents::platform_extensions::reviewer_integrity::diff(before, &after);
        if paths.is_empty() {
            return None;
        }
        crate::receipts::record_once(
            "verification_integrity",
            "reviewer_mutations",
            subagent_session_id,
        );
        tracing::warn!(
            target: "permagentd::brain",
            paths = ?paths,
            subagent_session_id = %subagent_session_id,
            "reviewer subagent mutated the working tree; voiding its verdict"
        );
        // Record a receipt on the PARENT session — best-effort: a failure to
        // persist it must never block returning the (still correctly
        // voided) result to the caller.
        if let Err(e) = session_manager
            .record_reviewer_mutation(
                parent_session_id,
                paths.clone(),
                Some(subagent_session_id.to_string()),
            )
            .await
        {
            tracing::warn!(
                target: "permagentd::brain",
                error = %e,
                "failed to record the reviewer-mutation receipt on the session (non-fatal)"
            );
        }
        Some(paths)
    }

    /// Resolve worker_persona key to a (block, display_name) tuple.
    /// Returns None only when worker_persona is None (no persona injection needed).
    fn resolve_worker_persona(worker_persona: Option<&str>) -> Option<(String, String)> {
        let worker_key = worker_persona?;
        let config = crate::config::agent_identity::load_agent_config();
        if let Some(worker) = config.workers.get(worker_key) {
            Some((
                crate::public_apis::attach_to_persona_block(
                    &worker.system_prompt_block(),
                    Some(worker_key),
                ),
                worker.display_name(),
            ))
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

    /// The `developer` extension's own tool names, minus whatever
    /// `tool_inspection::requires_dispatched_coding_worker` calls a mutation —
    /// the broader set `CodingDagInspector` already denies the top-level
    /// coordinator for (write tools ∪ `shell`/`bash`/`execute_command`/
    /// `run_command`/`terminal`), not just `after_turn::is_mutating_tool_name`'s
    /// named write tools. Derived from the extension's real tool surface (not
    /// a second hand-kept list), so a new read-only developer tool is granted
    /// automatically and a new mutating (or shell-shaped) one is withheld
    /// automatically.
    ///
    /// R50a: widened from `is_mutating_tool_name` to close H2 (see
    /// `restrict_reviewer_tools`'s doc comment below) — `shell` used to
    /// survive this filter untouched, so every caller of
    /// `restrict_to_non_mutating` (the reviewer persona, and a gated
    /// `delegate`/`delegate_many`/`start_agent` child) kept full shell
    /// access. The `verify` tool (deterministic test/build/lint execution)
    /// remains available, so a scoped session can still run checks without
    /// raw shell.
    fn non_mutating_developer_tools() -> Vec<String> {
        super::developer::DeveloperClient::get_tools()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .filter(|name| !crate::tool_inspection::requires_dispatched_coding_worker(name))
            .collect()
    }

    /// F2: scope the `developer` extension in `extensions` to read-only tools
    /// when the child recipe's worker persona is (case-insensitively) the
    /// literal `reviewer` roster key — the same key `resolve_worker_persona`
    /// and the default roster (`agent_identity::default_roster`) use for the
    /// one persona the recipe documents as read-only. Every other extension
    /// (including `analyze`, which the recipe already grants the reviewer) is
    /// left untouched, and a non-reviewer persona is a no-op.
    ///
    /// This reuses the SAME allowlist mechanism (`ExtensionConfig::
    /// available_tools` / `is_tool_available`) the extension manager already
    /// enforces on every tool dispatch — no new enforcement path, just a new
    /// caller of the existing one.
    ///
    /// G-1 (Lane G review): this used to match ONLY `ExtensionConfig::
    /// Platform`, but the real `permagent-coding` recipe declares `developer`
    /// as `type: builtin` (`permagent-coding.yaml`), which deserializes to
    /// `ExtensionConfig::Builtin` (`recipe_extension_adapter.rs`,
    /// `extension.rs:190`) — a variant that was never covered here, so live
    /// this was a no-op and the reviewer kept full `write`/`edit`. Now uses
    /// `ExtensionConfig::available_tools_mut()`, which covers every variant
    /// that carries an allowlist (matching `is_tool_available`'s own arms),
    /// so a future extension-config shape is included automatically rather
    /// than needing a matching update here.
    ///
    /// H2 (harness loop "A47 analysis"): CLOSED. The reviewer used to keep
    /// `shell` (needed to run verify/tests) with no read-only enforcement on
    /// it at all — a reviewer could mutate the tree via `sed -i`,
    /// redirection, `rm`, or `git commit`/`checkout --` through it, and the
    /// same gap let a `delegate`/`delegate_many`/`start_agent` child under
    /// the coding recipe run `shell` unrestricted too (A47: MiniCPM p5's
    /// `[subagent:2]` wrote the passing file via a `shell` heredoc after
    /// every `write`/`edit` call on it was denied). `restrict_to_non_mutating`
    /// now filters on `tool_inspection::requires_dispatched_coding_worker`'s
    /// broader set (write tools ∪ shell/bash/execute_command/run_command/
    /// terminal — the same set `CodingDagInspector` denies the top-level
    /// coordinator for), so `shell` no longer survives this scoping for the
    /// reviewer OR a gated delegate/start_agent child. The `verify` tool
    /// (deterministic test/build/lint execution, not raw shell) remains
    /// available, so read-only sessions can still run checks. A dispatched
    /// goal worker (`dispatch_goal_fn`'s `GOAL_WORKER_EXTENSION_KEYS` path)
    /// never calls `restrict_to_non_mutating` and keeps shell, since that is
    /// the one path that legitimately owns a reviewed, dispatched goal card.
    fn restrict_reviewer_tools(worker_persona: Option<&str>, extensions: &mut [ExtensionConfig]) {
        let is_reviewer = worker_persona.is_some_and(|p| p.eq_ignore_ascii_case("reviewer"));
        if !is_reviewer {
            return;
        }
        Self::restrict_to_non_mutating(extensions);
    }

    /// X-dag-1 (harness loop, coding-harness excellence iteration): the same
    /// mechanism `restrict_reviewer_tools` uses, generalised so other
    /// callers — `restrict_delegate_for_active_roadmap` below, and
    /// `orchestrator::scoped_child_session_extensions_fn` (R50a) — can scope
    /// a NON-reviewer child down to non-mutating tools too, rather than
    /// growing a second, parallel allowlist implementation.
    ///
    /// Returns `true` when `extensions` carried a `developer` extension that
    /// previously exposed a tool this call just removed (i.e. this scoping
    /// was not a no-op) — callers on a GATED path (a delegate/start_agent
    /// child escaping an active roadmap, not the always-scoped reviewer
    /// persona) use this to record a `bypass_attempts` receipt and a
    /// `child_mutation_denied` trace, matching what `CodingDagInspector`
    /// already records for the coordinator's own denied calls.
    pub(crate) fn restrict_to_non_mutating(extensions: &mut [ExtensionConfig]) -> bool {
        let allowed = Self::non_mutating_developer_tools();
        debug_assert!(
            !allowed.is_empty(),
            "an empty available_tools list means EVERY tool is available \
             (ExtensionConfig::is_tool_available's own definition) — the \
             non-mutating allowlist must never be empty, or scoping becomes a \
             no-op the opposite way"
        );
        // Defensive fail-closed fallback: an empty `available_tools` reads as
        // "every tool available" to `is_tool_available` — the exact opposite
        // of what this function exists to do. If `non_mutating_developer_
        // tools` ever legitimately returns nothing (e.g. every developer
        // tool becomes "mutating"), hand out a list that matches no real
        // tool name instead of an empty one, so the caller loses `developer`
        // entirely rather than silently regaining full access to it.
        const NO_SAFE_TOOLS_SENTINEL: &str = "__no_read_only_developer_tools_available__";
        let mut removed_a_denied_tool = false;
        for ext in extensions.iter_mut() {
            if ext.name() == super::developer::EXTENSION_NAME {
                if let Some(available_tools) = ext.available_tools_mut() {
                    let had_denied_tool = available_tools.is_empty()
                        || available_tools.iter().any(|name| {
                            crate::tool_inspection::requires_dispatched_coding_worker(name)
                        });
                    if had_denied_tool {
                        removed_a_denied_tool = true;
                    }
                    *available_tools = if allowed.is_empty() {
                        vec![NO_SAFE_TOOLS_SENTINEL.to_string()]
                    } else {
                        allowed.clone()
                    };
                }
            }
        }
        removed_a_denied_tool
    }

    /// X-dag-1: while a roadmap THIS session created (`create_roadmap`,
    /// stamping `metadata_json.created_by_session_id` — see
    /// `cards::goal_cards_created_by_session`) still has open work, a
    /// coordinator's `delegate` call must not be usable to run mutating
    /// tools. Dispatching a root goal (`goal_advance`/the auto-dispatch nudge)
    /// is the sanctioned path for real edits once a roadmap exists; a
    /// coordinator that gives up on the DAG dance and calls `delegate`
    /// directly must not get a side door around it — Lane R's Defect 3 /
    /// Lane M3's Defect M3-3, where the ungated `delegate` call was the ONLY
    /// path that landed real work in the whole slice, entirely bypassing the
    /// roadmap it had just been denied for.
    ///
    /// A no-op for:
    ///   - an explicit `worker_persona: "reviewer"` — already read-only via
    ///     `restrict_reviewer_tools` above; scoping it a second time here
    ///     would be redundant, not a behavior change, but skipping it keeps
    ///     "today's behaviour" for that persona legible as ONE code path.
    ///   - a `SessionType::SubAgent` session — `handle_delegate` already
    ///     refuses these outright ("Delegated tasks cannot spawn further
    ///     delegations"), so a goal worker dispatched via the hook can never
    ///     reach this function at all; checked again here so this gate's own
    ///     correctness does not silently depend on staying downstream of
    ///     that unrelated refusal.
    ///   - an ordinary session with no active roadmap of its own and no
    ///     coding-harness identity — nothing to bypass.
    ///
    /// Harness loop, Lane Y10c (A15 Q3): "active roadmap" was too narrow a
    /// window. M15 fx04's coordinator called `delegate` at 02:00:30.513 and
    /// minted its roadmap at 02:00:46.097 — 15 s LATER. With no cards yet
    /// attributed to the session, `session_has_an_active_roadmap` is false
    /// and this gate was a no-op, so a coding-harness coordinator could get a
    /// worker holding `write`/`edit` simply by delegating BEFORE
    /// `create_roadmap`. X-dag-1 closed "inside an active roadmap"; "before
    /// the roadmap exists" stayed open.
    ///
    /// So the gate now fires on EITHER identity: a top-level coding-harness
    /// coordinator (the identical
    /// [`is_top_level_coding_harness`](crate::tool_inspection::is_top_level_coding_harness)
    /// predicate `CodingDagInspector` and `NoRoadmapGuard` use — that session
    /// never mutates, roadmap or not, so there is no window at all), or any
    /// session with an active roadmap (unchanged). The identity check is
    /// first because it costs no query.
    ///
    /// Read-only delegates are still allowed: this strips the MUTATING tools
    /// (`restrict_to_non_mutating`), it does not refuse the call — fx04's own
    /// `analyze`/`search` scope audit would pass through untouched.
    ///
    /// R27b (harness loop, "A26 analysis" section 1 Q2/Q3): the identity/
    /// state check above — EITHER identity, both windows — is pulled out to
    /// this standalone associated function so `orchestrator.rs`'s
    /// `start_agent`/`send_message` handlers (harness loop lever 3: "scope
    /// start_agent/send_message targets to the lean coding profile ...
    /// matching what restrict_delegate_for_active_roadmap already does for
    /// delegate") can share the EXACT same gate rather than growing a
    /// second, parallel predicate that could quietly drift from this one on
    /// WHEN to fire. `restrict_delegate_for_active_roadmap` below is now a
    /// thin wrapper around this — same conditions, same messages, unchanged
    /// behavior; the full rationale above (Lane R/M3, Y10c) still applies.
    ///
    /// Returns `Ok(None)` when nothing should be restricted; `Ok(Some(why))`
    /// with a human-readable reason fragment (no trailing punctuation, meant
    /// to be embedded in a caller's own sentence) otherwise.
    pub(crate) async fn active_coding_roadmap_gate_reason(
        pool: &sqlx::Pool<sqlx::Sqlite>,
        session: &crate::session::Session,
        worker_persona: Option<&str>,
    ) -> Result<Option<&'static str>, String> {
        let is_reviewer = worker_persona.is_some_and(|p| p.eq_ignore_ascii_case("reviewer"));
        if is_reviewer || session.session_type == SessionType::SubAgent {
            return Ok(None);
        }

        let recipe_title = session.recipe.as_ref().map(|recipe| recipe.title.as_str());
        if crate::tool_inspection::is_top_level_coding_harness(
            recipe_title,
            session.parent_session_id.as_deref(),
        ) {
            return Ok(Some(
                "this session is the coding-harness coordinator, which plans and dispatches \
                 rather than edits — decompose the work with `create_roadmap` and dispatch its \
                 goals with `goal_advance` (action: dispatch)",
            ));
        }

        let cards = crate::cards::goal_cards_created_by_session(pool, &session.id).await?;
        if !crate::cards::session_has_an_active_roadmap(&cards) {
            return Ok(None);
        }
        Ok(Some(
            "this session has an active roadmap of its own — dispatch its goals with \
             `goal_advance` (action: dispatch) instead of editing directly through a \
             delegated worker",
        ))
    }

    /// Lane R63c: refuse `delegate`/`delegate_many` for a top-level coding
    /// coordinator while one of its own goals is live (InProgress or Review).
    ///
    /// M47 r63 s5: the headless DeepSeek coordinator polled its running goal
    /// with `goal_status` ×2 and `view_session` ×2, then spent a whole child
    /// session through `delegate` on the same work (five turns, ~300k input
    /// tokens). While a goal is live its worker and the goal's own review gate
    /// own that work; a delegate child only duplicates it. The refusal names
    /// the goal and the resolution (inspect with `goal_status`/`view_session`,
    /// or wait for review) and goes out on the ordinary refused-call path (the
    /// handler's `Err`, surfaced as a tool error). Counted as a
    /// `dag_enforcement` `bypass_attempts` — the same informational counter
    /// the coordinator's other denied calls use.
    ///
    /// Narrow by construction: interactive sessions, dispatched workers and
    /// sub-agents are untouched (`is_top_level_coding_harness`), and once the
    /// goal leaves InProgress/Review (Complete, Failed, Blocked, ...) delegate
    /// is available again. An unreadable store fails OPEN, like
    /// `restrict_delegate_for_active_roadmap`: this is a cost/protocol guard,
    /// not a safety boundary.
    async fn live_goal_delegate_refusal(
        &self,
        session: &crate::session::Session,
    ) -> Option<String> {
        if session.session_type == SessionType::SubAgent {
            return None;
        }
        let recipe_title = session.recipe.as_ref().map(|recipe| recipe.title.as_str());
        if !crate::tool_inspection::is_top_level_coding_harness(
            recipe_title,
            session.parent_session_id.as_deref(),
        ) {
            return None;
        }
        let pool = self.context.session_manager.pool_clone().await.ok()?;
        let cards = match crate::cards::goal_cards_created_by_session(&pool, &session.id).await {
            Ok(cards) => cards,
            Err(error) => {
                tracing::warn!(
                    target: "permagentd::brain",
                    session_id = %session.id,
                    %error,
                    "live-goal delegate gate could not read goal cards; not refusing"
                );
                return None;
            }
        };
        let refusal = live_goal_refusal(&cards)?;
        crate::receipts::record("dag_enforcement", "bypass_attempts");
        tracing::info!(
            target: "permagentd::brain",
            session_id = %session.id,
            "delegate_refused_live_goal"
        );
        Some(refusal)
    }

    /// Restricts `extensions` (see `restrict_to_non_mutating`) when
    /// `active_coding_roadmap_gate_reason` (above — the shared gate) fires
    /// for `session`; otherwise a no-op. The rationale for WHEN it fires
    /// lives on that function now; this keeps the WHAT — strip mutating
    /// developer tools AND (R46a) narrow the extension SET itself, record
    /// the bypass-attempt receipt, return an explanatory note — that is
    /// specific to `delegate`.
    ///
    /// R46a (harness loop, "A43 analysis" levers 1/3): before this lane,
    /// the mutating-tools strip above was the ONLY narrowing a gated
    /// `delegate`/`delegate_many` call got — the spawned sub-session still
    /// inherited every OTHER extension whole (`analyze`/`summon`/
    /// `deliberate`/`extensionmanager`, plus `orchestrator`'s full >20-tool
    /// surface), a 145-tool/~150KB-schema/~34.9k-token-minimum session that
    /// overflows any ≤32k-context tier before a single instruction token —
    /// reproducing, on this code path, the exact defect R30a already fixed
    /// for `goal_advance` dispatch. `extensions` is narrowed to
    /// `orchestrator::GOAL_WORKER_EXTENSION_KEYS` (`developer` + a
    /// per-tool-cut `orchestrator`, via `orchestrator::
    /// narrow_goal_worker_orchestrator_tools`) — the SAME lean worker
    /// profile R30a's dispatched goal worker gets, since a `delegate`
    /// child under an active roadmap is doing the identical job (edit code,
    /// report back), not planning or delegating further. Gated on the exact
    /// same condition as the mutating-tools strip, so the two can never
    /// disagree about WHEN to narrow — only a WIDER base list ever shrinks;
    /// nothing is ever added.
    async fn restrict_delegate_for_active_roadmap(
        pool: &sqlx::Pool<sqlx::Sqlite>,
        session: &crate::session::Session,
        worker_persona: Option<&str>,
        extensions: &mut Vec<ExtensionConfig>,
    ) -> Result<Option<String>, String> {
        let Some(why) =
            Self::active_coding_roadmap_gate_reason(pool, session, worker_persona).await?
        else {
            return Ok(None);
        };

        let lean_worker_grants: Vec<String> = orchestrator::GOAL_WORKER_EXTENSION_KEYS
            .iter()
            .map(|s| s.to_string())
            .collect();
        *extensions = narrow_extensions_for_agent(
            std::mem::take(extensions),
            Some(lean_worker_grants.as_slice()),
        );
        orchestrator::narrow_goal_worker_orchestrator_tools(extensions);

        // R50a: `restrict_to_non_mutating` now reports whether it actually
        // removed a tool this child had (write/edit/... OR shell/bash/
        // execute_command/run_command/terminal — H2's own gap) — this is a
        // GATED narrowing (the child is escaping an active roadmap or a
        // top-level coordinator identity), so a real removal is counted as a
        // bypass attempt via the same `dag_enforcement`/`bypass_attempts`
        // counter `CodingDagInspector` uses for the coordinator's own denied
        // calls, and traced as `child_mutation_denied` for the same reason.
        if Self::restrict_to_non_mutating(extensions) {
            crate::receipts::record("dag_enforcement", "bypass_attempts");
            tracing::info!(
                target: "permagentd::brain",
                session_id = %session.id,
                worker_persona = ?worker_persona,
                "child_mutation_denied"
            );
        }
        Ok(Some(format!(
            "Note: this delegate call was restricted to a lean, non-mutating tool set because \
             {why}."
        )))
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
        // E4: `source: "reviewer"` alongside `worker_persona: "reviewer"` (or
        // any other persona named twice this way) is not a real `source` — a
        // worker persona key is not, in general, a recipe/agent source at
        // all, and `worker_persona` already carries everything `source` would
        // have added. Observed 2/2 producing a total silent no-op: no
        // subagent, no visible error, the model narrating its own "reviewer
        // unavailable" excuse. Treat it as persona-only — exactly as if
        // `source` had never been set — rather than resolving (or failing to
        // resolve) the persona key as a source.
        let source_names_the_persona = match (&params.source, &params.worker_persona) {
            (Some(source), Some(persona)) => source.eq_ignore_ascii_case(persona),
            _ => false,
        };
        if source_names_the_persona {
            return self.build_adhoc_recipe(params);
        }
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
    ) -> Result<(TaskConfig, serde_json::Value, Option<String>), anyhow::Error> {
        let (provider, routing_receipt) = self.resolve_provider(params, recipe, session).await?;

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

        // F2: the recipe promises the `reviewer` persona is "scoped READ-ONLY
        // (developer + analyze, no editing)" (`permagent-coding.yaml`), but
        // until now that was a prompt-only claim — nothing stopped the
        // delegated recipe's `developer` extension from exposing `write`/
        // `edit`. Observed live (HARNESS_LOOP_LOG.md, Lane A2): a reviewer
        // subagent called `edit` on `src/cache.rs` and the change shipped in
        // the graded artifact. Enforce it below the prompt.
        Self::restrict_reviewer_tools(params.worker_persona.as_deref(), &mut extensions);

        // X-dag-1: close the `delegate` side door around an active roadmap.
        // Best-effort: `pool_clone()` only fails when the storage layer
        // cannot be reached at all — a condition that would also break
        // `create_roadmap`/`goal_advance` themselves, not something this
        // check alone can meaningfully guard against. Rather than turn every
        // delegate into a hard failure over an unrelated DB outage, an
        // unreachable pool leaves this call unrestricted; the roadmap system
        // itself (goal-transition guard, worktree isolation) remains the
        // primary boundary this is defense-in-depth for, not a replacement
        // for it.
        let roadmap_gate_note = match self.context.session_manager.pool_clone().await {
            Ok(pool) => Self::restrict_delegate_for_active_roadmap(
                &pool,
                session,
                params.worker_persona.as_deref(),
                &mut extensions,
            )
            .await
            .map_err(|e| anyhow::anyhow!(e))?,
            Err(_) => None,
        };

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

        Ok((task_config, routing_receipt, roadmap_gate_note))
    }

    /// Run ONE real subagent dispatch — the fresh subagent session,
    /// notification bridge, and `run_subagent_task` call `handle_delegate`
    /// always ran inline — extracted (Lane R36b) so the reviewer-persona
    /// breaker retry below calls the SAME spawn path a second time rather
    /// than inventing a new one. Session-creation failure is folded into the
    /// `Result` (instead of the caller's own `?`) so a retry attempt's
    /// creation failure just reads as "this attempt failed", not a second
    /// hard abort of the whole delegate call.
    #[allow(clippy::too_many_arguments)]
    async fn dispatch_subagent_once(
        &self,
        agent_config: AgentRunnerConfig,
        recipe: Recipe,
        task_config: TaskConfig,
        session_id: &str,
        working_dir: PathBuf,
        cancellation_token: CancellationToken,
        persona_override: Option<(String, String)>,
    ) -> Result<(Result<String, anyhow::Error>, String), String> {
        let subagent_session = self
            .context
            .session_manager
            .create_session_with_parent(
                Some(session_id),
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

        Ok((result, subagent_session_id))
    }

    /// Lane R36b: after a `worker_persona: "reviewer"` sub-session's real
    /// dispatch fails, decide (pure `cost_router::reviewer_retry_target`)
    /// whether the failure is provider-unavailable-class and a different
    /// session model exists to retry on; if so, open this session's breaker
    /// — so a LATER reviewer/planner call in the same session skips the
    /// same dead provider outright (R36a's `provider_circuit`) — and run
    /// ONE retry through [`Self::dispatch_subagent_once`], on the author's
    /// own (provider, model) pair, reusing `task_config`'s existing
    /// extensions/max_turns via `retry_task_config_base.clone()` with only
    /// the provider swapped (the existing profile/override mechanism, never
    /// a new spawn path).
    ///
    /// The G-lane independence rule is honored honestly, never by skipping:
    /// a retry on the author's own model IS the mutating session's model,
    /// so it is recorded via the SAME `reviewer_not_independent` receipt
    /// counter the ordinary same-family dispatch fallback already uses (the
    /// warning-driven branch in [`Self::resolve_provider`] does not fire for
    /// this RUNTIME retry — that check only sees the SELECTION-time
    /// dispatch, which already ran and picked the now-failed provider).
    ///
    /// If no retry is attempted, or the retry itself fails, the original
    /// failure and session id are returned unchanged and a `reviewer_
    /// unreachable` line is logged for the audit trail. That failure still
    /// surfaces to the caller as an errored delegate result — this
    /// deliberately does NOT try to make the goal read as reviewed:
    /// `after_turn::ReviewerMandate`'s own `ReviewerLastOutcome::Empty`
    /// handling already never treats a failed/empty delegate call as a
    /// completed review, which is what keeps this goal from being counted
    /// as reviewed.
    #[allow(clippy::too_many_arguments)]
    async fn retry_reviewer_dispatch_if_unavailable(
        &self,
        result: Result<String, anyhow::Error>,
        subagent_session_id: String,
        session_id: &str,
        session: &crate::session::Session,
        failed_provider: &str,
        failed_model: &str,
        retry_task_config_base: TaskConfig,
        agent_config: AgentRunnerConfig,
        recipe: Recipe,
        working_dir: PathBuf,
        cancellation_token: CancellationToken,
        persona_override: Option<(String, String)>,
    ) -> (Result<String, anyhow::Error>, String) {
        // Captured as an owned `String` up front (rather than holding a
        // borrow of `result` across the whole function) so `result` stays
        // free to move into every early-return below without fighting the
        // borrow checker.
        let error_text = match &result {
            Ok(_) => return (result, subagent_session_id),
            Err(e) => e.to_string(),
        };
        let author = session
            .provider_name
            .as_deref()
            .zip(session.model_config.as_ref().map(|m| m.model_name.as_str()));
        let Some((class, fallback)) = crate::cost_router::reviewer_retry_target(
            failed_provider,
            failed_model,
            &error_text,
            author,
        ) else {
            tracing::warn!(
                target: "permagentd::brain",
                session_id = %session_id,
                provider = %failed_provider,
                "reviewer_unreachable: the independent reviewer sub-session failed with no \
                 retry target ({}); the review did not happen this turn",
                error_text,
            );
            return (result, subagent_session_id);
        };

        if crate::cost_router::provider_circuit::open(session_id, failed_provider, class) {
            tracing::info!(
                target: "permagentd::brain",
                provider = %failed_provider,
                reason = %class,
                "provider_circuit_open provider={} reason={}",
                failed_provider,
                class,
            );
        }
        // The fallback is definitionally the author's own (provider,
        // model) — the G-lane independence rule says NEVER skip the review
        // over this, only report it honestly.
        crate::receipts::record("verification_integrity", "reviewer_not_independent");

        let model_config = match crate::model::ModelConfig::new(&fallback.model)
            .map(|c| c.with_canonical_limits(&fallback.provider))
        {
            Ok(c) => c,
            Err(build_err) => {
                tracing::warn!(
                    target: "permagentd::brain",
                    session_id = %session_id,
                    "reviewer_unreachable: could not build the fallback reviewer model config \
                     ({}); the review did not happen this turn",
                    build_err,
                );
                return (result, subagent_session_id);
            }
        };
        let fallback_provider =
            match providers::create(&fallback.provider, model_config, Vec::new()).await {
                Ok(p) => p,
                Err(create_err) => {
                    tracing::warn!(
                        target: "permagentd::brain",
                        session_id = %session_id,
                        "reviewer_unreachable: could not create the fallback reviewer provider \
                         {}/{} ({}); the review did not happen this turn",
                        fallback.provider,
                        fallback.model,
                        create_err,
                    );
                    return (result, subagent_session_id);
                }
            };

        let mut retry_task_config = retry_task_config_base;
        retry_task_config.provider = fallback_provider;

        match self
            .dispatch_subagent_once(
                agent_config,
                recipe,
                retry_task_config,
                session_id,
                working_dir,
                cancellation_token,
                persona_override,
            )
            .await
        {
            Ok((retry_result, retry_subagent_session_id)) => {
                if let Err(retry_err) = &retry_result {
                    tracing::warn!(
                        target: "permagentd::brain",
                        session_id = %session_id,
                        subagent_session_id = %retry_subagent_session_id,
                        "reviewer_unreachable: the session-tier fallback reviewer also failed \
                         ({}); the review did not happen this turn",
                        retry_err,
                    );
                }
                (retry_result, retry_subagent_session_id)
            }
            Err(create_err) => {
                tracing::warn!(
                    target: "permagentd::brain",
                    session_id = %session_id,
                    "reviewer_unreachable: could not spawn the session-tier fallback reviewer \
                     ({}); the review did not happen this turn",
                    create_err,
                );
                (result, subagent_session_id)
            }
        }
    }

    /// The workflow role a delegate's worker persona plays, for cost routing.
    /// `None` when there is no persona or it yields no role signal — dispatch then
    /// stays single-model. See `cost_router::role_map::derive_role`.
    fn role_for_persona(worker_persona: Option<&str>) -> Option<crate::cost_router::WorkflowRole> {
        let key = worker_persona?;
        let config = crate::config::agent_identity::load_agent_config();
        config.workers.get(key).and_then(|w| w.routing_role())
    }

    /// Lane R63c: the `receipt cost_routing` audit for one delegate child, from
    /// the live pins — the same `PERMAGENT_ROLE_*`/`PERMAGENT_PACK_*` reads
    /// `delegate_routing_live` makes (cheap config reads), handed to the pure
    /// [`delegate_audit_targets`] + `cost_router::audit_delegate_dispatch`.
    fn delegate_cost_audit(
        role: Option<crate::cost_router::WorkflowRole>,
        review_inputs: Option<&ReviewAuditInputs>,
        session: &crate::session::Session,
        actual: (&str, &str),
    ) -> crate::cost_router::DelegateCostAudit {
        let role_pin = role.and_then(crate::cost_router::role_model);
        let pack_pin = role
            .and_then(crate::cost_router::pack_role_for_workflow_role)
            .and_then(crate::cost_router::pack_pin)
            .map(crate::cost_router::pack_to_role_model);
        let session_pair = session
            .provider_name
            .as_deref()
            .zip(session.model_config.as_ref().map(|m| m.model_name.as_str()))
            .filter(|(p, m)| !p.is_empty() && !m.is_empty())
            .map(|(provider, model)| crate::cost_router::RoleModel {
                provider: provider.to_string(),
                model: model.to_string(),
            });
        let targets =
            delegate_audit_targets(role_pin, pack_pin, review_inputs.cloned(), session_pair);
        let audit = crate::cost_router::audit_delegate_dispatch(
            actual,
            targets.pin.as_ref(),
            targets.pin_exception.as_ref(),
            targets.reference.as_ref(),
            &crate::cost_router::routing_matrix::nominal_call_cost,
        );
        tracing::info!(
            target: "permagentd::brain",
            role = role.map(|r| r.as_str()).unwrap_or("none"),
            provider = actual.0,
            model = actual.1,
            cheapest_capable = audit.cheapest_capable,
            pack_honored = audit.pack_honored,
            "delegate cost_routing audit",
        );
        audit
    }

    /// The spend band that gates a delegate ESCALATION.
    ///
    /// FAIL-CLOSED, matching [`crate::tool_monitor`]'s budget boundary. This
    /// path AUTHORIZES extra spend on a pricier model, so a band we cannot read
    /// is not permission. `None` means no escalation.
    async fn escalation_spend_band(
        &self,
        session: &crate::session::Session,
    ) -> Option<crate::cost_router::BudgetBand> {
        use crate::agents::platform_extensions::orchestrator as orch;
        let pool = self.context.session_manager.pool_clone().await.ok()?;
        let snapshot = orch::spend_snapshot(&pool, &session.id).await.ok()?;
        let task_spent = snapshot.task_spent_usd;
        let session_spent = snapshot.session_spent_usd;
        let unpriced = snapshot.unpriced_calls;
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

    /// X-cost-2 (HARNESS_LOOP_LOG.md "Lane A report" Defect 1a / "Lane E
    /// report" E3): fetch the live inputs and hand them to
    /// [`crate::cost_router::resolve_actual_review_dispatch`] — the SAME
    /// decision function `after_turn::ReviewerMandate::assess` calls to
    /// predict what this dispatch will do (so the mandate's injected
    /// "honesty" text and this REAL dispatch can never drift apart again).
    ///
    /// `routing` is the ordinary `delegate_routing_live` result already
    /// computed for the Review role (role pin → `Role::Hard` pack pin →
    /// gated escalation → session). When `routing.source` is `RolePin`, the
    /// operator's `PERMAGENT_ROLE_REVIEW_*` mapping already resolved this
    /// correctly and wins unconditionally. For every OTHER source
    /// (`PackPin`/`Escalated`/`Session` — none of them review-aware, since
    /// `PackPin` here means `Role::Hard`'s pin), review's OWN dispatch
    /// precedence takes over: a dedicated `PERMAGENT_PACK_REVIEW_*` pin,
    /// else cross-family escalation to the cheapest capable cross-family
    /// reviewer (Lane R63c, `cost_router::cheapest_cross_family_reviewer` —
    /// previously the derived map's strongest REVIEW entry), else the
    /// ordinary chain's answer honestly labeled same-family.
    ///
    /// Lane R63c: returns the dispatch together with the dedicated review pin
    /// and the cross-family candidate it was decided against, so the
    /// `receipt cost_routing` audit judges the child by the SAME inputs.
    async fn resolve_review_dispatch(
        &self,
        session: &crate::session::Session,
        routing: &crate::cost_router::DelegateRouting,
        spend: Option<crate::cost_router::BudgetBand>,
    ) -> ReviewDispatchResolution {
        let author_provider = session.provider_name.as_deref();
        let author_model = session.model_config.as_ref().map(|m| m.model_name.as_str());
        let routing_is_role_pin = routing.source == crate::cost_router::DelegateSource::RolePin;
        // The dedicated review pin / cross-family candidate / spend band are
        // only fetched when they can change the answer — a `RolePin` source
        // already resolved correctly and `resolve_actual_review_dispatch`
        // passes it straight through, so a pinned dispatch must not pay for
        // an answer it will not use.
        let (review_pin, cross_family_candidate, review_spend) = if routing_is_role_pin {
            (None, None, None)
        } else {
            let review_pin =
                crate::cost_router::review_pack_pin().map(crate::cost_router::pack_to_role_model);
            // Lane R63c (M47 r63 s5): the cross-family escalation target is
            // the CHEAPEST capable different-family model, not the derived
            // map's REVIEW entry — the recommender ranks REVIEW by strength
            // first (`recommend::review_rank`), so that entry was
            // `claude-fable-5` and a one-line DeepSeek diff bought a $1.97
            // frontier review. `after_turn::ReviewerMandate::assess` computes
            // the identical candidate so its prediction cannot drift.
            let cross_family_candidate = if review_pin.is_none() {
                let available = crate::cost_router::discover_available_models_async().await;
                let lines = crate::after_turn::changed_lines(&session.working_dir).await;
                crate::cost_router::cheapest_cross_family_reviewer(
                    author_provider.zip(author_model),
                    &available,
                    lines,
                )
            } else {
                None
            };
            // Review's cross-family escalation is not behind the general
            // `PERMAGENT_DELEGATE_ALLOW_ESCALATION` knob (review's whole
            // value IS the cross-family check), so its spend band is read
            // regardless of whether ordinary escalation is enabled — reusing
            // `spend` when the caller already fetched it (escalation on)
            // avoids a second DB round trip.
            let review_spend = if spend.is_some() {
                spend
            } else {
                self.escalation_spend_band(session).await
            };
            (review_pin, cross_family_candidate, review_spend)
        };
        let dispatch = crate::cost_router::resolve_actual_review_dispatch(
            routing_is_role_pin,
            routing.role_model.clone(),
            author_provider.zip(author_model),
            review_pin.clone(),
            cross_family_candidate.clone(),
            &|p| crate::cost_router::is_provider_configured(p),
            review_spend,
        );
        tracing::info!(
            target: "permagentd::brain",
            role = "review",
            cross_family = dispatch.warning.is_none(),
            provider = dispatch
                .role_model
                .as_ref()
                .map(|rm| rm.provider.as_str())
                .unwrap_or_else(|| author_provider.unwrap_or("session")),
            model = dispatch
                .role_model
                .as_ref()
                .map(|rm| rm.model.as_str())
                .unwrap_or_else(|| author_model.unwrap_or("session")),
            "review dispatch resolved",
        );
        ReviewDispatchResolution {
            dispatch,
            review_pin,
            cross_family_candidate,
        }
    }

    async fn resolve_provider(
        &self,
        params: &DelegateParams,
        recipe: &Recipe,
        session: &crate::session::Session,
    ) -> Result<(Arc<dyn crate::providers::base::Provider>, serde_json::Value), anyhow::Error> {
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
        // persona. X-cost-2: `resolve_review_dispatch` reuses the operator's
        // `PERMAGENT_ROLE_REVIEW_*` pin unchanged (when that is what
        // `routing` resolved to), but for every other source it now runs
        // review's OWN dispatch precedence instead of blindly slotting
        // whatever `Role::Hard`'s pack pin produced — a HARD pin no longer
        // silently swallows review's cross-family requirement.
        //
        // Lane R36b: this is the REAL dispatch seam R36a's call-site audit
        // flagged as the one best/fast-over-session path its breaker never
        // reached — `apply_breaker_to_review_dispatch` skips a resolved
        // reviewer provider this session's breaker already has open (a
        // prior billing/auth/outage-class failure anywhere else this same
        // session) straight to the author's own pair, honestly labeled
        // not-independent, rather than dispatching a second doomed call at
        // the same dead account.
        let mut review_audit_inputs: Option<ReviewAuditInputs> = None;
        let review_dispatch = if role == Some(crate::cost_router::WorkflowRole::Review) {
            let ReviewDispatchResolution {
                dispatch,
                review_pin,
                cross_family_candidate,
            } = self.resolve_review_dispatch(session, &routing, spend).await;
            review_audit_inputs = Some(ReviewAuditInputs {
                review_pack_pin: review_pin,
                cross_family: cross_family_candidate,
            });
            // A clean author pair, not `session_pair` above — that one
            // substitutes the placeholder string "the session model" when
            // `model_config` is unset (fine for `delegate_routing_live`'s
            // own purposes), which must never leak into a real fallback
            // `RoleModel`. Mirrors `resolve_review_dispatch`'s own
            // `author_provider`/`author_model` construction exactly.
            let author_pair = session
                .provider_name
                .as_deref()
                .zip(session.model_config.as_ref().map(|m| m.model_name.as_str()));
            Some(crate::cost_router::apply_breaker_to_review_dispatch(
                dispatch,
                author_pair,
                &|p| crate::cost_router::provider_circuit::reason_if_open(&session.id, p),
            ))
        } else {
            None
        };
        let role_model = match &review_dispatch {
            // The resolved (provider, model) may be genuinely NEW here (a
            // dedicated review pin, or a cross-family escalation) rather
            // than whatever `routing` produced for the generic role, so this
            // is taken directly rather than gated on `role_model` already
            // being `Some` — `routing.source` still labels the receipt.
            Some(dispatch) => dispatch.role_model.clone().map(|rm| (rm, routing.source)),
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
                crate::receipts::record("verification_integrity", "reviewer_not_independent");
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

        // The role router runs before explicit/recipe reconciliation. Preserve
        // its human-readable summary, but attach the final dispatch target so a
        // receipt can never claim "session model" when an explicit child pin
        // was actually honored (or vice versa).
        let effective_provider = provider.get_name().to_string();
        let effective_model = provider.get_model_config().model_name.clone();
        let requested_provider = params.provider.clone().or(recipe_provider.clone());
        let requested_model = params
            .model
            .clone()
            .or_else(|| recipe.settings.as_ref().and_then(|s| s.goose_model.clone()));
        let effective_source = if params.provider.is_some() || params.model.is_some() {
            "explicit_param"
        } else if recipe_provider.is_some()
            || recipe
                .settings
                .as_ref()
                .is_some_and(|s| s.goose_model.is_some())
        {
            "recipe"
        } else if role_model.is_some() {
            routing.source.as_str()
        } else if Config::global()
            .get_param::<String>("GOOSE_SUBAGENT_PROVIDER")
            .ok()
            .is_some()
            || Config::global()
                .get_param::<String>("GOOSE_SUBAGENT_MODEL")
                .ok()
                .is_some()
        {
            "environment"
        } else {
            "session"
        };
        let mut routing_receipt = resolved_routing_receipt(
            &routing,
            requested_provider,
            requested_model,
            &effective_provider,
            &effective_model,
            effective_source,
        );
        // Lane R63c: judge THIS child for `receipt cost_routing` against the
        // same pins/candidates that routed it, on its EFFECTIVE target (so an
        // explicit per-call model is counted too, never invisible). Recorded
        // on the process-wide counters only when the child is actually spawned
        // (`record_delegate_dispatch`), not here.
        let audit = Self::delegate_cost_audit(
            role,
            review_audit_inputs.as_ref(),
            session,
            (effective_provider.as_str(), effective_model.as_str()),
        );
        if let Some(object) = routing_receipt.as_object_mut() {
            object.insert("cost_audit".to_string(), serde_json::json!(audit));
        }

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

        Ok((provider, routing_receipt))
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

        let (task_config, routing_receipt, roadmap_gate_note) = self
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
            .create_session_with_parent(
                Some(session_id),
                working_dir,
                description.clone(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .map_err(|e| format!("Failed to create subagent session: {}", e))?;

        let task_id = subagent_session.id.clone();
        // Lane R63c: a background delegate child is a dispatch too.
        record_delegate_dispatch(&routing_receipt);

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
        // X-dag-1: the roadmap-gate note (when present) rides the same
        // visible line, since this task's own result text is not seen until
        // `load(source: ...)` returns it later.
        let mut ack = format!(
            "Task {} started in background: \"{}\"\n\
             {}\n\
             Continue with other work. When you need the result, use load(source: \"{}\").",
            task_id,
            description,
            routing_receipt
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("routing receipt unavailable"),
            task_id
        );
        if let Some(note) = &roadmap_gate_note {
            ack = format!("{note}\n\n{ack}");
        }
        let content = vec![Content::text(ack)];
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

/// Lane R63c: the refusal text for a coordinator `delegate` while one of its
/// goals is live — `None` when no goal is InProgress or Review. Pure over the
/// same [`crate::cards::SessionGoalCard`] read the X-dag-1 gate uses.
fn live_goal_refusal(cards: &[crate::cards::SessionGoalCard]) -> Option<String> {
    let live = cards
        .iter()
        .find(|card| matches!(card.state_binding.as_str(), "in_progress" | "review"))?;
    Some(format!(
        "delegate refused: goal {} is running; use goal_status or view_session to inspect \
         it, or wait for review. Its worker and the goal's own review gate already own \
         that work, so a delegated child would only duplicate it (and spend a session). \
         Delegate is available again once the goal leaves InProgress/Review.",
        live.card_id
    ))
}

/// Lane R63c: count one spawned delegate child on `receipt cost_routing`, from
/// the `cost_audit` [`SummonClient::resolve_provider`] attached to its routing
/// receipt. A receipt without one (never produced by `resolve_provider`) is
/// not guessed at — it is traced and left uncounted.
fn record_delegate_dispatch(routing_receipt: &serde_json::Value) {
    let audit = routing_receipt.get("cost_audit");
    let flag = |key: &str| {
        audit
            .and_then(|a| a.get(key))
            .and_then(serde_json::Value::as_bool)
    };
    let provider = routing_receipt
        .get("provider")
        .and_then(serde_json::Value::as_str);
    match (flag("cheapest_capable"), flag("pack_honored"), provider) {
        (Some(cheapest_capable), Some(pack_honored), Some(provider)) => {
            crate::cost_router::routing_receipt::record_delegate_dispatch(
                crate::cost_router::DelegateCostAudit {
                    cheapest_capable,
                    pack_honored,
                },
                provider,
            );
        }
        _ => tracing::warn!(
            target: "permagentd::brain",
            "delegate child spawned without a cost_audit on its routing receipt; not counted"
        ),
    }
}

/// Lane R63c: the (pin, pin exception, cheapest-capable reference) a delegate
/// child is audited against — pure, so the receipt judgement is tested without
/// a provider or a database.
///
/// - REVIEW (`review` is `Some`): a dedicated pin (`PERMAGENT_ROLE_REVIEW_*`, else
///   `PERMAGENT_PACK_REVIEW_*`) is both the pin and the reference. Otherwise
///   the pin is the `Role::Hard` pack pin review maps through, with the
///   cheapest cross-family reviewer as its one permitted exception (X-cost-2:
///   the review gate requires independence, and on a single-model tier the
///   HARD pin IS the author's family); the reference is that cheapest
///   cross-family reviewer, else the HARD pin, else the session pair.
/// - Every other role (or none): the pin is the role pin, else the pack pin;
///   the reference is that pin, else the operator's session pair (level 6 —
///   escalation is opt-in, so anything pricier than it was a choice).
fn delegate_audit_targets(
    role_pin: Option<crate::cost_router::RoleModel>,
    pack_pin: Option<crate::cost_router::RoleModel>,
    review: Option<ReviewAuditInputs>,
    session_pair: Option<crate::cost_router::RoleModel>,
) -> DelegateAuditTargets {
    match review {
        Some(ReviewAuditInputs {
            review_pack_pin,
            cross_family,
        }) => {
            if let Some(dedicated) = role_pin.or(review_pack_pin) {
                return DelegateAuditTargets {
                    pin: Some(dedicated.clone()),
                    pin_exception: None,
                    reference: Some(dedicated),
                };
            }
            let reference = cross_family
                .clone()
                .or_else(|| pack_pin.clone())
                .or(session_pair);
            DelegateAuditTargets {
                pin: pack_pin,
                pin_exception: cross_family,
                reference,
            }
        }
        None => {
            let pin = role_pin.or(pack_pin);
            let reference = pin.clone().or(session_pair);
            DelegateAuditTargets {
                pin,
                pin_exception: None,
                reference,
            }
        }
    }
}

/// Lane R63c: the review role's own routing inputs, carried from
/// [`SummonClient::resolve_review_dispatch`] to the receipt audit.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewAuditInputs {
    /// The dedicated `PERMAGENT_PACK_REVIEW_*` pin, when one was read.
    review_pack_pin: Option<crate::cost_router::RoleModel>,
    /// The cheapest capable cross-family reviewer, when one was computed.
    cross_family: Option<crate::cost_router::RoleModel>,
}

/// Lane R63c: what a delegate child is audited against (see
/// [`delegate_audit_targets`] and `cost_router::audit_delegate_dispatch`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct DelegateAuditTargets {
    pin: Option<crate::cost_router::RoleModel>,
    pin_exception: Option<crate::cost_router::RoleModel>,
    reference: Option<crate::cost_router::RoleModel>,
}

/// Lane R63c: what [`SummonClient::resolve_review_dispatch`] decided, plus the
/// inputs it decided against — reused by the `receipt cost_routing` audit.
struct ReviewDispatchResolution {
    dispatch: crate::cost_router::ReviewerDispatch,
    /// The dedicated `PERMAGENT_PACK_REVIEW_*` pin, when one was read.
    review_pin: Option<crate::cost_router::RoleModel>,
    /// The cheapest capable cross-family reviewer, when one was computed.
    cross_family_candidate: Option<crate::cost_router::RoleModel>,
}

/// Preserve the existing role-router summary while attaching the final,
/// post-reconciliation target. The old receipt was produced before explicit
/// and recipe overrides were applied, which made an honored child pin appear
/// to have used the session model.
fn resolved_routing_receipt(
    routing: &crate::cost_router::DelegateRouting,
    requested_provider: Option<String>,
    requested_model: Option<String>,
    effective_provider: &str,
    effective_model: &str,
    effective_source: &str,
) -> serde_json::Value {
    let mut receipt = routing.receipt_json();
    if let Some(object) = receipt.as_object_mut() {
        object.insert(
            "requested".to_string(),
            serde_json::json!({
                "provider": requested_provider,
                "model": requested_model,
            }),
        );
        object.insert(
            "effective".to_string(),
            serde_json::json!({
                "provider": effective_provider,
                "model": effective_model,
                "source": effective_source,
            }),
        );
        // Keep these top-level fields for existing consumers that already read
        // `provider`/`model`, but make them the actual dispatch target.
        object.insert("provider".to_string(), effective_provider.into());
        object.insert("model".to_string(), effective_model.into());
        object.insert("effective_source".to_string(), effective_source.into());
        object.insert(
            "summary".to_string(),
            format!(
                "{}; effective {}/{} ({})",
                routing.receipt, effective_provider, effective_model, effective_source
            )
            .into(),
        );
    }
    receipt
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

    /// Build the same shape `build_task_config` sees: a `developer` platform
    /// extension with no restriction yet, plus one unrelated extension that
    /// must never be touched by reviewer scoping.
    fn sample_extensions() -> Vec<ExtensionConfig> {
        vec![
            ExtensionConfig::Platform {
                name: crate::agents::platform_extensions::developer::EXTENSION_NAME.to_string(),
                description: "".to_string(),
                display_name: None,
                bundled: None,
                available_tools: Vec::new(),
            },
            ExtensionConfig::Platform {
                name: "analyze".to_string(),
                description: "".to_string(),
                display_name: None,
                bundled: None,
                available_tools: Vec::new(),
            },
        ]
    }

    fn developer_available_tools(extensions: &[ExtensionConfig]) -> &[String] {
        extensions
            .iter()
            .find(|ext| ext.name() == crate::agents::platform_extensions::developer::EXTENSION_NAME)
            .expect("developer extension must be present")
            .available_tools()
    }

    /// F2 (Lane A2 evidence: a `worker_persona: reviewer` subagent called
    /// `edit` on `src/cache.rs` and the change shipped). `restrict_reviewer_tools`
    /// must scope `developer` down to read-only tools for the literal
    /// `reviewer` persona, and must not touch any other extension.
    ///
    /// R50a (H2 closure): `shell` moved from "must keep" to "must not be
    /// granted" — the reviewer used to keep unrestricted shell (the
    /// documented H2 residual); `verify` (deterministic check execution)
    /// remains so the reviewer can still run tests without raw shell.
    #[test]
    fn restrict_reviewer_tools_scopes_developer_to_read_only_for_the_reviewer_persona() {
        let mut extensions = sample_extensions();

        SummonClient::restrict_reviewer_tools(Some("reviewer"), &mut extensions);

        let allowed = developer_available_tools(&extensions);
        assert!(!allowed.is_empty(), "reviewer must get a real allowlist");
        for mutating in ["write", "edit", "shell"] {
            assert!(
                !allowed.iter().any(|t| t == mutating),
                "reviewer must not be granted '{mutating}', got: {allowed:?}"
            );
        }
        for read_only in ["read", "tree", "search", "verify"] {
            assert!(
                allowed.iter().any(|t| t == read_only),
                "reviewer must keep '{read_only}', got: {allowed:?}"
            );
        }

        // Case-insensitive, matching `resolve_worker_persona`'s own lookup.
        let mut extensions_upper = sample_extensions();
        SummonClient::restrict_reviewer_tools(Some("Reviewer"), &mut extensions_upper);
        assert!(!developer_available_tools(&extensions_upper).is_empty());

        // Untouched: the `analyze` extension the recipe already grants the
        // reviewer must stay unrestricted (empty allowlist == every tool).
        let analyze = extensions
            .iter()
            .find(|ext| ext.name() == "analyze")
            .unwrap();
        assert!(analyze.available_tools().is_empty());
    }

    /// G-1 (Lane G review, CRITICAL): the test above builds its own
    /// `ExtensionConfig::Platform` fixture, but the REAL `permagent-coding`
    /// recipe declares `developer` as `type: builtin`
    /// (`permagent-coding.yaml:171-172`), which the crate's own
    /// `#[serde(tag = "type")]` deserializer turns into
    /// `ExtensionConfig::Builtin` (`recipe_extension_adapter.rs`,
    /// `extension.rs:190`) — a variant `restrict_reviewer_tools` never
    /// matched before this fix, so live the allowlist was never applied and
    /// a reviewer subagent kept full `write`/`edit` (Lane A2's live
    /// evidence: "a reviewer subagent called `edit` on src/cache.rs and the
    /// change shipped"). This builds the extension list by parsing the
    /// actual shipped recipe file — the same shape `build_task_config`
    /// really produces — not a hand-built fixture, so a regression back to
    /// covering only one `ExtensionConfig` variant fails here even if the
    /// `Platform`-shaped test above still passes.
    #[test]
    fn restrict_reviewer_tools_scopes_the_real_recipes_builtin_developer_extension() {
        const RECIPE_YAML: &str =
            include_str!("../../../../goose-cli/src/recipes/builtin/permagent-coding.yaml");
        let recipe: crate::recipe::Recipe =
            serde_yaml::from_str(RECIPE_YAML).expect("the shipped recipe must parse as YAML");
        let mut extensions = recipe
            .extensions
            .expect("permagent-coding.yaml declares an extensions list");

        let developer = extensions
            .iter()
            .find(|ext| ext.name() == crate::agents::platform_extensions::developer::EXTENSION_NAME)
            .expect("the recipe must declare the developer extension");
        assert!(
            matches!(developer, ExtensionConfig::Builtin { .. }),
            "the recipe's `type: builtin` developer entry must deserialize to \
             ExtensionConfig::Builtin, not {developer:?} — if the recipe's \
             shape ever changes, restrict_reviewer_tools's coverage needs to \
             change with it"
        );

        SummonClient::restrict_reviewer_tools(Some("reviewer"), &mut extensions);

        let allowed = developer_available_tools(&extensions);
        assert!(
            !allowed.is_empty(),
            "reviewer must get a real allowlist on the recipe's own builtin \
             developer extension"
        );
        for mutating in ["write", "edit", "apply_patch", "shell"] {
            assert!(
                !allowed.iter().any(|t| t == mutating),
                "reviewer must not be granted '{mutating}' on the real recipe's \
                 builtin developer extension, got: {allowed:?}"
            );
        }
        for read_only in ["read", "tree", "search", "verify"] {
            assert!(
                allowed.iter().any(|t| t == read_only),
                "reviewer must keep '{read_only}', got: {allowed:?}"
            );
        }
    }

    /// A non-reviewer persona (or no persona at all) must be a complete no-op:
    /// this is scoping for one specific documented persona, not a general
    /// tool restriction.
    #[test]
    fn restrict_reviewer_tools_is_a_no_op_for_a_non_reviewer_persona() {
        for persona in [Some("permagent"), Some("debugger"), None] {
            let mut extensions = sample_extensions();
            SummonClient::restrict_reviewer_tools(persona, &mut extensions);
            assert!(
                developer_available_tools(&extensions).is_empty(),
                "persona {persona:?} must leave developer's tools unrestricted"
            );
        }
    }

    // ── X-dag-1: `delegate` may not bypass an active roadmap ────────────────

    /// A throwaway `SessionManager` + a real ready-goal card attributed to
    /// `session.id` — the exact shape `create_roadmap` leaves behind for a
    /// session whose root goal has not been dispatched yet. Returns the
    /// manager (kept alive for the pool), the coordinator's own session, and
    /// the created card id.
    async fn active_roadmap_fixture(
        session_type: SessionType,
    ) -> (
        TempDir,
        Arc<crate::session::SessionManager>,
        crate::session::Session,
        String,
    ) {
        let tmp = TempDir::new().unwrap();
        let sm = Arc::new(crate::session::SessionManager::new(
            tmp.path().to_path_buf(),
        ));
        let session = sm
            .create_session(
                tmp.path().to_path_buf(),
                "x-dag-1-fixture".to_string(),
                session_type,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let pool = sm.pool_clone().await.unwrap();
        crate::cards::seed_goal_columns(&pool, crate::projects::PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        let ready_column: String = sqlx::query_scalar(
            "SELECT id FROM board_columns WHERE project_id = ? AND state_binding = 'ready'",
        )
        .bind(crate::projects::PERSONAL_PROJECT_ID)
        .fetch_one(&pool)
        .await
        .unwrap();
        let card_id = uuid::Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO cards (id, project_id, card_type, title, description, column_id, \
                                 position, created_by, metadata_json) \
             VALUES (?, ?, 'goal', 'root goal', '', ?, 0, 'user', ?)",
        )
        .bind(&card_id)
        .bind(crate::projects::PERSONAL_PROJECT_ID)
        .bind(&ready_column)
        .bind(
            serde_json::json!({
                "depends_on": [],
                "created_by_session_id": session.id,
            })
            .to_string(),
        )
        .execute(&pool)
        .await
        .unwrap();
        (tmp, sm, session, card_id)
    }

    /// X-dag-1's headline case: a coordinator session whose own roadmap has
    /// a root goal sitting Ready, never dispatched, must have `delegate`
    /// restricted to non-mutating tools — the exact side door Lane M3's
    /// Defect M3-3 found (`delegate` was the ONLY path that landed real
    /// work, entirely bypassing the roadmap).
    #[tokio::test]
    #[serial_test::serial(receipts_global_registry)]
    async fn a_coordinator_delegate_with_an_active_roadmap_is_restricted_to_non_mutating_tools() {
        let (_tmp, sm, session, _card_id) = active_roadmap_fixture(SessionType::User).await;
        let pool = sm.pool_clone().await.unwrap();
        let mut extensions = sample_extensions();

        let note = SummonClient::restrict_delegate_for_active_roadmap(
            &pool,
            &session,
            None,
            &mut extensions,
        )
        .await
        .unwrap();

        assert!(
            note.is_some(),
            "a restricted delegate must return a note explaining why"
        );
        let allowed = developer_available_tools(&extensions);
        for mutating in ["write", "edit", "apply_patch"] {
            assert!(
                !allowed.iter().any(|t| t == mutating),
                "must not be granted '{mutating}', got: {allowed:?}"
            );
        }
    }

    /// No active roadmap (no cards ever attributed to this session) → no
    /// restriction, no note, extensions untouched — "unchanged" per the
    /// brief's own test spec.
    #[tokio::test]
    #[serial_test::serial(receipts_global_registry)]
    async fn a_coordinator_delegate_with_no_active_roadmap_is_unchanged() {
        let tmp = TempDir::new().unwrap();
        let sm = crate::session::SessionManager::new(tmp.path().to_path_buf());
        let session = sm
            .create_session(
                tmp.path().to_path_buf(),
                "no-roadmap-fixture".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let pool = sm.pool_clone().await.unwrap();
        let mut extensions = sample_extensions();

        let note = SummonClient::restrict_delegate_for_active_roadmap(
            &pool,
            &session,
            None,
            &mut extensions,
        )
        .await
        .unwrap();

        assert!(note.is_none());
        assert!(
            developer_available_tools(&extensions).is_empty(),
            "no active roadmap must leave developer's tools unrestricted (empty == every tool)"
        );
    }

    /// Y10c (A15 Q3): M15 fx04's coordinator called `delegate` 15 SECONDS
    /// before it minted its roadmap. With no cards yet attributed to the
    /// session the old `session_has_an_active_roadmap` predicate was false and
    /// this gate was a no-op — so a coding-harness coordinator could get a
    /// worker holding `write`/`edit` just by delegating first. The identity
    /// itself now closes that window.
    #[tokio::test]
    #[serial_test::serial(receipts_global_registry)]
    async fn a_coding_harness_coordinator_with_no_roadmap_yet_is_still_restricted() {
        let tmp = TempDir::new().unwrap();
        let sm = crate::session::SessionManager::new(tmp.path().to_path_buf());
        let mut session = sm
            .create_session(
                tmp.path().to_path_buf(),
                "pre-roadmap-coordinator".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        // Deliberately NO cards: this is the 15-second window before
        // `create_roadmap`.
        session.recipe = Some(
            Recipe::builder()
                .version("1.0.0")
                .title(crate::tool_inspection::CODING_HARNESS_RECIPE_TITLE)
                .description("coding harness")
                .prompt("plan and dispatch")
                .build()
                .unwrap(),
        );
        assert!(session.parent_session_id.is_none(), "top-level by fixture");

        let pool = sm.pool_clone().await.unwrap();
        let mut extensions = sample_extensions();

        let note = SummonClient::restrict_delegate_for_active_roadmap(
            &pool,
            &session,
            None,
            &mut extensions,
        )
        .await
        .unwrap();

        let note = note.expect("a restricted delegate must say why");
        assert!(
            note.contains("coding-harness coordinator"),
            "the note must name the identity that restricted it, got {note:?}"
        );
        let allowed = developer_available_tools(&extensions);
        // R50a (H2 closure): `shell` joins the denied set here — it used to
        // survive this restriction (see `non_mutating_developer_tools`'s
        // doc comment).
        for mutating in ["write", "edit", "apply_patch", "shell"] {
            assert!(
                !allowed.iter().any(|t| t == mutating),
                "must not be granted '{mutating}', got: {allowed:?}"
            );
        }
        // Read-only sources stay available — the gate strips mutation, it does
        // not refuse the delegate (fx04's own `analyze` scope audit).
        for readonly in ["search", "read"] {
            assert!(
                allowed.iter().any(|t| t == readonly),
                "a read-only '{readonly}' must survive the restriction, got: {allowed:?}"
            );
        }
    }

    /// The identity is TOP-LEVEL: a coding-harness session with a parent is a
    /// dispatched worker, the thing that MAY edit. Same boundary
    /// `CodingDagInspector`/`NoRoadmapGuard` draw.
    #[tokio::test]
    #[serial_test::serial(receipts_global_registry)]
    async fn a_coding_harness_session_with_a_parent_is_not_restricted_by_identity() {
        let tmp = TempDir::new().unwrap();
        let sm = crate::session::SessionManager::new(tmp.path().to_path_buf());
        let mut session = sm
            .create_session(
                tmp.path().to_path_buf(),
                "child-coordinator".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        session.recipe = Some(
            Recipe::builder()
                .version("1.0.0")
                .title(crate::tool_inspection::CODING_HARNESS_RECIPE_TITLE)
                .description("coding harness")
                .prompt("plan and dispatch")
                .build()
                .unwrap(),
        );
        session.parent_session_id = Some("parent-session".to_string());

        let pool = sm.pool_clone().await.unwrap();
        let mut extensions = sample_extensions();

        let note = SummonClient::restrict_delegate_for_active_roadmap(
            &pool,
            &session,
            None,
            &mut extensions,
        )
        .await
        .unwrap();

        assert!(note.is_none());
        assert!(
            developer_available_tools(&extensions).is_empty(),
            "a dispatched worker keeps every tool (empty == unrestricted)"
        );
    }

    /// A goal-WORKER session (`SessionType::SubAgent`, the shape
    /// `dispatch_goal_fn`/the auto-dispatch hook creates) must never be
    /// restricted by this gate, even if (hypothetically) it were attributed
    /// its own active-roadmap cards — `handle_delegate` already refuses
    /// SubAgent callers outright, and this gate must not depend on staying
    /// downstream of that refusal to be correct.
    #[tokio::test]
    #[serial_test::serial(receipts_global_registry)]
    async fn a_goal_worker_session_is_unaffected_by_the_active_roadmap_gate() {
        let (_tmp, sm, session, _card_id) = active_roadmap_fixture(SessionType::SubAgent).await;
        let pool = sm.pool_clone().await.unwrap();
        let mut extensions = sample_extensions();

        let note = SummonClient::restrict_delegate_for_active_roadmap(
            &pool,
            &session,
            None,
            &mut extensions,
        )
        .await
        .unwrap();

        assert!(note.is_none());
        assert!(developer_available_tools(&extensions).is_empty());
    }

    /// An explicit `worker_persona: "reviewer"` keeps today's behaviour: this
    /// gate is a no-op (the persona is already read-only via
    /// `restrict_reviewer_tools`), even while a roadmap is active.
    #[tokio::test]
    #[serial_test::serial(receipts_global_registry)]
    async fn an_explicit_reviewer_persona_is_unaffected_by_the_active_roadmap_gate() {
        let (_tmp, sm, session, _card_id) = active_roadmap_fixture(SessionType::User).await;
        let pool = sm.pool_clone().await.unwrap();
        let mut extensions = sample_extensions();

        let note = SummonClient::restrict_delegate_for_active_roadmap(
            &pool,
            &session,
            Some("reviewer"),
            &mut extensions,
        )
        .await
        .unwrap();

        assert!(
            note.is_none(),
            "the reviewer persona is handled by restrict_reviewer_tools, not this gate"
        );
    }

    // ── R46a: delegate/delegate_many get R30a's lean worker profile ─────────

    /// A fixture beyond `sample_extensions()`: adds `orchestrator` (with a
    /// full multi-tool surface, standing in for its real >20-tool roster)
    /// plus `extensionmanager`, mirroring the extras "A43 analysis" lever 1
    /// named as still leaking through to a `delegate` child before this
    /// lane — R27b's own `r27b_lean_profile_plus_unrelated_extras` fixture
    /// does the analogous thing for `start_agent`/`send_message`.
    fn sample_extensions_with_orchestrator_and_extras() -> Vec<ExtensionConfig> {
        let mut extensions = sample_extensions();
        extensions.push(ExtensionConfig::Platform {
            name: orchestrator::EXTENSION_NAME.to_string(),
            description: "".to_string(),
            display_name: None,
            bundled: None,
            available_tools: vec![
                "escalate".to_string(),
                "message_goal".to_string(),
                "start_agent".to_string(),
                "create_roadmap".to_string(),
            ],
        });
        extensions.push(ExtensionConfig::Platform {
            name: "extensionmanager".to_string(),
            description: "".to_string(),
            display_name: None,
            bundled: None,
            available_tools: Vec::new(),
        });
        extensions
    }

    /// R46a headline case (implements "A43 analysis" lever 1): a `delegate`
    /// call gated by an active roadmap must narrow the child's extension SET
    /// down to exactly `orchestrator::GOAL_WORKER_EXTENSION_KEYS` — R30a's
    /// lean worker profile — not just strip mutating tools out of whatever
    /// full roster it started from. A drift guard against R30a's own
    /// constants, mirroring R27b's own
    /// `scoped_child_session_extensions_fn_narrows_to_the_lean_coding_
    /// profile_for_the_top_level_coordinator`.
    #[tokio::test]
    #[serial_test::serial(receipts_global_registry)]
    async fn a_gated_delegate_narrows_the_extension_set_to_r30as_lean_worker_profile() {
        let (_tmp, sm, session, _card_id) = active_roadmap_fixture(SessionType::User).await;
        let pool = sm.pool_clone().await.unwrap();
        let mut extensions = sample_extensions_with_orchestrator_and_extras();

        let note = SummonClient::restrict_delegate_for_active_roadmap(
            &pool,
            &session,
            None,
            &mut extensions,
        )
        .await
        .unwrap();
        assert!(note.is_some());

        let names: HashSet<String> = extensions.iter().map(|ext| ext.name()).collect();
        let expected: HashSet<String> = orchestrator::GOAL_WORKER_EXTENSION_KEYS
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            names, expected,
            "a gated delegate child must get exactly GOAL_WORKER_EXTENSION_KEYS (developer + \
             orchestrator), no analyze/extensionmanager/etc, got: {names:?}"
        );

        let orchestrator_ext = extensions
            .iter()
            .find(|ext| ext.name() == orchestrator::EXTENSION_NAME)
            .expect("orchestrator must survive in the lean worker profile");
        let allowed_tools: HashSet<&str> = orchestrator_ext
            .available_tools()
            .iter()
            .map(String::as_str)
            .collect();
        let expected_tools: HashSet<&str> = orchestrator::GOAL_WORKER_ORCHESTRATOR_TOOL_ALLOWLIST
            .iter()
            .copied()
            .collect();
        assert_eq!(
            allowed_tools, expected_tools,
            "orchestrator's own tool surface must be cut to \
             GOAL_WORKER_ORCHESTRATOR_TOOL_ALLOWLIST (escalate + message_goal), not the full \
             roster it started with, got: {allowed_tools:?}"
        );

        // R50a (H2 closure): `developer` survives set-narrowing above, but
        // its OWN tools must also be non-mutating AND shell-free — a
        // delegate child without a dispatched goal card must not be able to
        // reach `write`/`edit` OR `shell`/`bash`/`execute_command`/
        // `run_command`/`terminal`.
        let developer_ext = extensions
            .iter()
            .find(|ext| ext.name() == crate::agents::platform_extensions::developer::EXTENSION_NAME)
            .expect("developer must survive narrowing to GOAL_WORKER_EXTENSION_KEYS");
        let developer_tools: HashSet<&str> = developer_ext
            .available_tools()
            .iter()
            .map(String::as_str)
            .collect();
        for denied in ["write", "edit", "shell"] {
            assert!(
                !developer_tools.contains(denied),
                "a gated delegate child must not keep '{denied}', got: {developer_tools:?}"
            );
        }
        assert!(
            developer_tools.contains("read") && developer_tools.contains("verify"),
            "read-only developer tools (including verify, for running checks without raw \
             shell) must still be available, got: {developer_tools:?}"
        );
    }

    /// R50a (H2 closure, "A47 analysis"): the smallest repair the analysis
    /// names — a gated delegate child whose pre-narrowed `developer` tools
    /// would have included `shell` (even with no `write`/`edit` at all, the
    /// exact MiniCPM p5 shape: both denied at the top level, `shell` still
    /// reached one level down) must be counted as a `bypass_attempts`
    /// receipt, not a pass — the same counter `CodingDagInspector` uses for
    /// the coordinator's own denied calls, so the qualification gate's
    /// `dag_enforcement` record sees this escape attempt even though it
    /// never reaches the coordinator-only inspector.
    #[tokio::test]
    #[serial_test::serial(receipts_global_registry)]
    async fn a_gated_delegate_childs_denied_shell_call_counts_as_a_bypass_attempt() {
        crate::receipts::begin_run();
        let (_tmp, sm, session, _card_id) = active_roadmap_fixture(SessionType::User).await;
        let pool = sm.pool_clone().await.unwrap();
        let mut extensions = sample_extensions_with_orchestrator_and_extras();

        let note = SummonClient::restrict_delegate_for_active_roadmap(
            &pool,
            &session,
            None,
            &mut extensions,
        )
        .await
        .unwrap();
        assert!(note.is_some());

        let line = crate::receipts::snapshot().lines()[0].clone();
        crate::receipts::end_run();
        assert!(
            line.contains("bypass_attempts=1"),
            "a gated delegate child that would have kept shell/write must count as exactly \
             one bypass attempt: {line}"
        );
        assert!(
            line.contains("bypass_passes=0"),
            "a denied tool is a bypass ATTEMPT, never a bypass_passes count: {line}"
        );
    }

    /// Without an active roadmap the gate is a no-op — per the brief's own
    /// spec, the extension SET must stay completely unchanged, not just the
    /// mutating-tools strip: `extensionmanager`/`orchestrator`'s full tool
    /// surface must both survive untouched.
    #[tokio::test]
    #[serial_test::serial(receipts_global_registry)]
    async fn an_ungated_delegate_leaves_the_extension_set_completely_unchanged() {
        let tmp = TempDir::new().unwrap();
        let sm = crate::session::SessionManager::new(tmp.path().to_path_buf());
        let session = sm
            .create_session(
                tmp.path().to_path_buf(),
                "no-roadmap-fixture-r46a".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let pool = sm.pool_clone().await.unwrap();
        let before = sample_extensions_with_orchestrator_and_extras();
        let mut extensions = before.clone();

        let note = SummonClient::restrict_delegate_for_active_roadmap(
            &pool,
            &session,
            None,
            &mut extensions,
        )
        .await
        .unwrap();

        assert!(note.is_none());
        let before_names: HashSet<String> = before.iter().map(|ext| ext.name()).collect();
        let after_names: HashSet<String> = extensions.iter().map(|ext| ext.name()).collect();
        assert_eq!(
            before_names, after_names,
            "no active roadmap must leave the extension SET untouched, got: {after_names:?}"
        );
        let orchestrator_ext = extensions
            .iter()
            .find(|ext| ext.name() == orchestrator::EXTENSION_NAME)
            .expect("orchestrator must still be present, ungated");
        assert_eq!(
            orchestrator_ext.available_tools().len(),
            4,
            "orchestrator's own tool surface must be untouched when the gate never fires"
        );
    }

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
    #[serial_test::serial(receipts_global_registry)]
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

    /// E4, pinned two ways: `source` equal to the same key as
    /// `worker_persona` is treated as persona-only (the ad-hoc path, using
    /// the caller's own instructions), and a genuinely unknown, unrelated
    /// `source` still yields a visible error.
    ///
    /// Before this repair, the failing assertion here was:
    /// `.expect("source naming the same persona must not fail the delegate call")`
    /// — `build_delegate_recipe` took the `source` branch, called
    /// `resolve_source(session_id, "reviewer", working_dir)` against a fresh
    /// temp dir with no `reviewer` agent file registered, and returned
    /// `Err("Source 'reviewer' not found")` instead of ever building a
    /// recipe — the same silent-failure shape Lane A observed live (2/2): no
    /// subagent spawned, and in the real harness no visible error text
    /// either, with the model narrating its own "reviewer unavailable"
    /// excuse.
    #[tokio::test]
    async fn source_naming_the_same_persona_is_treated_as_persona_only() {
        let ext = SummonClient::new(create_test_context()).unwrap();
        let dir = TempDir::new().unwrap();
        let params = DelegateParams {
            instructions: Some("Look for regressions in the diff".to_string()),
            source: Some("reviewer".to_string()),
            worker_persona: Some("reviewer".to_string()),
            ..Default::default()
        };
        let recipe = ext
            .build_delegate_recipe(&params, "fake-session-e4", dir.path())
            .await
            .expect("source naming the same persona must not fail the delegate call");
        assert_eq!(
            recipe.prompt.as_deref(),
            Some("Look for regressions in the diff"),
            "the ad-hoc path must use the caller's own instructions"
        );
    }

    /// Case-insensitive: `source: "Reviewer"` / `worker_persona: "reviewer"`.
    #[tokio::test]
    async fn source_naming_the_persona_case_insensitively_is_also_persona_only() {
        let ext = SummonClient::new(create_test_context()).unwrap();
        let dir = TempDir::new().unwrap();
        let params = DelegateParams {
            instructions: Some("Look for regressions in the diff".to_string()),
            source: Some("Reviewer".to_string()),
            worker_persona: Some("reviewer".to_string()),
            ..Default::default()
        };
        assert!(ext
            .build_delegate_recipe(&params, "fake-session-e4", dir.path())
            .await
            .is_ok());
    }

    /// Regression pin: a genuinely unknown, unrelated `source` must still
    /// surface a visible error — the repair narrows the special case to
    /// `source == worker_persona`; it must not swallow real "not found"
    /// errors.
    #[tokio::test]
    async fn an_unrelated_unknown_source_still_yields_a_visible_error() {
        let ext = SummonClient::new(create_test_context()).unwrap();
        let dir = TempDir::new().unwrap();
        let params = DelegateParams {
            instructions: Some("do the thing".to_string()),
            source: Some("definitely-bogus-source-xyz".to_string()),
            ..Default::default()
        };
        let error = ext
            .build_delegate_recipe(&params, "fake-session-e4", dir.path())
            .await
            .expect_err("an unknown source must surface a visible error");
        assert!(error.contains("not found"), "{error}");
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
    fn explicit_target_receipt_overrides_session_summary() {
        let routing = crate::cost_router::delegate::decide_delegate_model(
            None,
            None,
            None,
            None,
            false,
            None,
            &|_| true,
            Some(("minimax", "MiniMax-M2.5")),
        );
        let receipt = resolved_routing_receipt(
            &routing,
            Some("zai".to_string()),
            Some("glm-4.5-air".to_string()),
            "zai",
            "glm-4.5-air",
            "explicit_param",
        );

        assert_eq!(receipt["requested"]["provider"], "zai");
        assert_eq!(receipt["requested"]["model"], "glm-4.5-air");
        assert_eq!(receipt["effective"]["provider"], "zai");
        assert_eq!(receipt["effective"]["model"], "glm-4.5-air");
        assert_eq!(receipt["effective_source"], "explicit_param");
        assert!(receipt["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("effective zai/glm-4.5-air")));
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

    #[serial_test::serial(receipts_global_registry)]
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

    #[serial_test::serial(receipts_global_registry)]
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

    // ── G5 finding 2: reviewer mutation detection in `delegate_many` ────────
    //
    // `handle_delegate_many` fans a child out through `run_subagent_task`,
    // which needs a real model provider — there is no in-process fake for it
    // in this crate's unit tests (the same constraint X-verify-1's own tests
    // worked within: it pinned `reviewer_integrity::{snapshot,diff}` and
    // `after_turn::reviewer_mutation_void_text` directly rather than driving
    // a full `handle_delegate` call end to end). So these tests pin the
    // extracted helper `detect_and_record_reviewer_mutation` directly — the
    // EXACT function both `handle_delegate` and `handle_delegate_many`'s
    // fan-out closure now call — proving the fan-out path has the identical
    // detection-and-receipt behavior the single-call path already had.
    // Before this fix, no equivalent of this function was reachable from
    // `handle_delegate_many` at all: a reviewer-persona child's mutation
    // went completely undetected there.

    fn reviewer_git_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        let run = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .current_dir(dir.path())
                .args(args)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        };
        run(&["-c", "init.defaultBranch=main", "init", "-q"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "test"]);
        fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "--no-gpg-sign", "-qm", "init"]);
        dir
    }

    #[tokio::test]
    #[serial_test::serial(receipts_global_registry)]
    async fn delegate_many_reviewer_mutation_is_detected_and_voids_via_the_shared_helper() {
        let repo = reviewer_git_repo();
        let before =
            crate::agents::platform_extensions::reviewer_integrity::snapshot(repo.path()).await;

        // Simulate what a reviewer-persona child in `delegate_many`'s
        // fan-out did during its run: edit a tracked file instead of only
        // reviewing it.
        fs::write(repo.path().join("a.txt"), "two\n").unwrap();

        let temp_dir = TempDir::new().unwrap();
        let session_manager = crate::session::SessionManager::new(temp_dir.path().to_path_buf());
        let parent = session_manager
            .create_session(
                temp_dir.path().to_path_buf(),
                "coordinator".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await
            .unwrap();

        let mutated = SummonClient::detect_and_record_reviewer_mutation(
            &session_manager,
            &parent.id,
            repo.path(),
            &before,
            "delegate-many-child-1",
        )
        .await;

        assert_eq!(
            mutated,
            Some(vec!["a.txt".to_string()]),
            "delegate_many's reviewer child must be caught by the same detector as \
             handle_delegate's single-call path"
        );

        let text = crate::after_turn::reviewer_mutation_void_text(
            mutated.as_deref().unwrap(),
            "VERDICT: APPROVE",
        );
        assert!(
            text.contains("a.txt"),
            "the voided text must name the mutated path: {text}"
        );
        assert!(
            text.starts_with(crate::after_turn::REVIEWER_MUTATION_VOID_PREFIX),
            "the void sentinel must be the first line of the returned text: {text}"
        );

        let reloaded = session_manager
            .get_session(&parent.id, false)
            .await
            .unwrap();
        let receipts = crate::session::reviewer_mutation_receipts(&reloaded.extension_data);
        assert_eq!(
            receipts.len(),
            1,
            "the receipt must land on the PARENT session"
        );
        assert_eq!(receipts[0].paths, vec!["a.txt".to_string()]);
        assert_eq!(
            receipts[0].subagent_session_id.as_deref(),
            Some("delegate-many-child-1")
        );
    }

    #[serial_test::serial(receipts_global_registry)]
    #[tokio::test]
    async fn delegate_many_reviewer_with_a_clean_tree_is_not_voided() {
        let repo = reviewer_git_repo();
        let before =
            crate::agents::platform_extensions::reviewer_integrity::snapshot(repo.path()).await;
        // No mutation this time — a genuine, read-only review.

        let temp_dir = TempDir::new().unwrap();
        let session_manager = crate::session::SessionManager::new(temp_dir.path().to_path_buf());
        let parent = session_manager
            .create_session(
                temp_dir.path().to_path_buf(),
                "coordinator".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await
            .unwrap();

        let mutated = SummonClient::detect_and_record_reviewer_mutation(
            &session_manager,
            &parent.id,
            repo.path(),
            &before,
            "delegate-many-child-2",
        )
        .await;

        assert_eq!(mutated, None, "a clean reviewer child must never be voided");
        let reloaded = session_manager
            .get_session(&parent.id, false)
            .await
            .unwrap();
        assert!(
            crate::session::reviewer_mutation_receipts(&reloaded.extension_data).is_empty(),
            "no receipt may be recorded when nothing changed"
        );
    }

    #[serial_test::serial(receipts_global_registry)]
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
        assert_eq!(
            parsed.tasks[0].delegate.model.as_deref(),
            Some("qwen3-coder")
        );
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

    #[serial_test::serial(receipts_global_registry)]
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

    #[serial_test::serial(receipts_global_registry)]
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

    // ── Lane R63c (M47 r63 s5): delegate cost routing, receipts, live goals ──

    const R63_AUTHOR: (&str, &str) = ("custom_deepseek", "deepseek-chat");

    /// The s5 roster: the DeepSeek tier plus the Anthropic line an
    /// `ANTHROPIC_API_KEY` puts in the pool.
    fn r63_available() -> Vec<crate::cost_router::AvailableModel> {
        use crate::cost_router::AvailableModel;
        vec![
            AvailableModel::new(R63_AUTHOR.0, R63_AUTHOR.1),
            AvailableModel::new("anthropic", "claude-fable-5"),
            AvailableModel::new("anthropic", "claude-sonnet-5"),
            AvailableModel::new("anthropic", "claude-haiku-4-5-20251001"),
        ]
    }

    fn r63_rm(provider: &str, model: &str) -> crate::cost_router::RoleModel {
        crate::cost_router::RoleModel {
            provider: provider.to_string(),
            model: model.to_string(),
        }
    }

    /// Every routing key this lane reads, set to the `dschat` tier's shape
    /// (`pack`: every `PERMAGENT_PACK_{EDIT,HARD,MECHANICAL,LOCAL}_*` pinned to
    /// DeepSeek) or cleared (`None`), with no role pins, no dedicated review
    /// pack pin (unless `review` sets one) and escalation off. ONE
    /// `lock_env` call: its mutex is not re-entrant, so a second nested lock
    /// in the same test would deadlock.
    fn r63_env(
        pack: Option<(&'static str, &'static str)>,
        review: Option<(&'static str, &'static str)>,
    ) -> env_lock::EnvGuard<'static> {
        use crate::cost_router::packs::*;
        let provider = pack.map(|(p, _)| p);
        let model = pack.map(|(_, m)| m);
        env_lock::lock_env([
            (KEY_EDIT_PROVIDER, provider),
            (KEY_EDIT_MODEL, model),
            (KEY_HARD_PROVIDER, provider),
            (KEY_HARD_MODEL, model),
            (KEY_MECHANICAL_PROVIDER, provider),
            (KEY_MECHANICAL_MODEL, model),
            (KEY_LOCAL_PROVIDER, provider),
            (KEY_LOCAL_MODEL, model),
            (KEY_REVIEW_PROVIDER, review.map(|(p, _)| p)),
            (KEY_REVIEW_MODEL, review.map(|(_, m)| m)),
            ("PERMAGENT_ROLE_REVIEW_PROVIDER", None),
            ("PERMAGENT_ROLE_REVIEW_MODEL", None),
            ("PERMAGENT_ROLE_EDIT_PROVIDER", None),
            ("PERMAGENT_ROLE_EDIT_MODEL", None),
            (crate::cost_router::KEY_ALLOW_ESCALATION, None),
        ])
    }

    /// The reviewer delegate's real resolution chain, minus the provider
    /// construction: `delegate_routing` (role pin → pack pin → escalation →
    /// session), then review's own precedence with the SAME cross-family
    /// candidate `resolve_review_dispatch` now computes.
    fn r63_review_dispatch(
        derived: &crate::cost_router::DerivedRoleMap,
        changed_lines: usize,
    ) -> (
        crate::cost_router::DelegateRouting,
        Option<crate::cost_router::RoleModel>,
        Option<crate::cost_router::RoleModel>,
        crate::cost_router::ReviewerDispatch,
    ) {
        use crate::cost_router as cr;
        let routing = cr::delegate_routing(
            Some(cr::WorkflowRole::Review),
            derived,
            Some(cr::BudgetBand::Ok),
            Some(R63_AUTHOR),
        );
        let review_pin = cr::review_pack_pin().map(cr::pack_to_role_model);
        let candidate = if review_pin.is_none() {
            cr::cheapest_cross_family_reviewer(Some(R63_AUTHOR), &r63_available(), changed_lines)
        } else {
            None
        };
        let dispatch = cr::resolve_actual_review_dispatch(
            routing.source == cr::DelegateSource::RolePin,
            routing.role_model.clone(),
            Some(R63_AUTHOR),
            review_pin.clone(),
            candidate.clone(),
            &|_: &str| true,
            Some(cr::BudgetBand::Ok),
        );
        (routing, review_pin, candidate, dispatch)
    }

    /// Pinned packs: an ordinary role delegate lands on the pin even with the
    /// derived map offering the frontier row, and the reviewer delegate —
    /// which keeps its cross-family requirement over the HARD pin — escalates
    /// to the CHEAPEST capable cross-family model, never `claude-fable-5`.
    #[test]
    fn r63c_a_delegate_under_pinned_packs_resolves_to_the_pin_and_a_reviewer_to_the_cheapest_cross_family(
    ) {
        use crate::cost_router as cr;
        let _g = r63_env(Some(R63_AUTHOR), None);
        let derived = cr::derive_role_map(&r63_available());

        let edit = cr::delegate_routing(
            Some(cr::WorkflowRole::Edit),
            &derived,
            Some(cr::BudgetBand::Ok),
            Some(R63_AUTHOR),
        );
        assert_eq!(edit.source, cr::DelegateSource::PackPin);
        assert_eq!(edit.role_model, Some(r63_rm(R63_AUTHOR.0, R63_AUTHOR.1)));

        let (routing, review_pin, candidate, dispatch) = r63_review_dispatch(&derived, 1);
        assert_eq!(routing.source, cr::DelegateSource::PackPin, "HARD pin read");
        assert!(review_pin.is_none());
        let target = dispatch
            .role_model
            .clone()
            .expect("a cross-family reviewer");
        assert_eq!(Some(target.clone()), candidate);
        assert_ne!(target.model, "claude-fable-5", "the s5 defect: {target:?}");
        assert_ne!(cr::family_of(&target.provider, &target.model), "deepseek");
        if let Some((strongest, _)) = derived.get(cr::WorkflowRole::Review) {
            let cost = |rm: &cr::RoleModel| {
                cr::routing_matrix::nominal_call_cost(&rm.provider, &rm.model)
                    .unwrap_or(f64::INFINITY)
            };
            assert!(
                cost(&target) <= cost(strongest),
                "the dispatch must never cost more than the derived strongest REVIEW \
                 entry ({strongest:?}); got {target:?}"
            );
        }

        // Receipt audit for that same child: cheapest and pack-honoured.
        let session_pair = Some(r63_rm(R63_AUTHOR.0, R63_AUTHOR.1));
        let targets = delegate_audit_targets(
            None,
            routing.role_model.clone(),
            Some(ReviewAuditInputs {
                review_pack_pin: review_pin.clone(),
                cross_family: candidate.clone(),
            }),
            session_pair.clone(),
        );
        let audit = cr::audit_delegate_dispatch(
            (target.provider.as_str(), target.model.as_str()),
            targets.pin.as_ref(),
            targets.pin_exception.as_ref(),
            targets.reference.as_ref(),
            &cr::routing_matrix::nominal_call_cost,
        );
        assert!(audit.cheapest_capable && audit.pack_honored, "{audit:?}");

        // The s5 child as it actually ran: neither.
        let s5 = cr::audit_delegate_dispatch(
            ("anthropic", "claude-fable-5"),
            targets.pin.as_ref(),
            targets.pin_exception.as_ref(),
            targets.reference.as_ref(),
            &cr::routing_matrix::nominal_call_cost,
        );
        assert!(!s5.cheapest_capable, "{s5:?}");
        assert!(!s5.pack_honored, "{s5:?}");
    }

    /// A DEDICATED review pin is operator intent: the reviewer delegate runs
    /// on it (honestly same-family when it is), and it is the audit's pin and
    /// reference.
    #[test]
    fn r63c_a_dedicated_review_pack_pin_is_honoured_by_the_reviewer_delegate() {
        use crate::cost_router as cr;
        let _g = r63_env(Some(R63_AUTHOR), Some(("anthropic", "claude-sonnet-5")));
        let derived = cr::derive_role_map(&r63_available());
        let (routing, review_pin, candidate, dispatch) = r63_review_dispatch(&derived, 1);
        let pinned = r63_rm("anthropic", "claude-sonnet-5");
        assert_eq!(review_pin, Some(pinned.clone()));
        assert_eq!(dispatch.role_model, Some(pinned.clone()));
        let targets = delegate_audit_targets(
            None,
            routing.role_model.clone(),
            Some(ReviewAuditInputs {
                review_pack_pin: review_pin,
                cross_family: candidate,
            }),
            Some(r63_rm(R63_AUTHOR.0, R63_AUTHOR.1)),
        );
        assert_eq!(
            targets,
            DelegateAuditTargets {
                pin: Some(pinned.clone()),
                pin_exception: None,
                reference: Some(pinned),
            }
        );
    }

    /// Nothing pinned: a role delegate stays on the operator's session model
    /// (escalation is opt-in) — never the planner/frontier row the derived
    /// map offers — and the reviewer delegate picks the cheapest capable
    /// cross-family model, not the strongest.
    #[test]
    fn r63c_a_delegate_without_pins_picks_the_cheapest_capable_not_the_planner_model() {
        use crate::cost_router as cr;
        let _g = r63_env(None, None);
        let derived = cr::derive_role_map(&r63_available());
        let planner = derived
            .get(cr::WorkflowRole::Orchestrate)
            .map(|(rm, _)| rm.clone());

        for role in [cr::WorkflowRole::Edit, cr::WorkflowRole::Orchestrate] {
            let routing = cr::delegate_routing(
                Some(role),
                &derived,
                Some(cr::BudgetBand::Ok),
                Some(R63_AUTHOR),
            );
            assert_eq!(routing.source, cr::DelegateSource::Session, "{role:?}");
            assert_eq!(routing.role_model, None, "inherits the session pair");
        }

        let (_routing, _pin, candidate, dispatch) = r63_review_dispatch(&derived, 1);
        let target = dispatch.role_model.expect("a cross-family reviewer");
        assert_eq!(Some(target.clone()), candidate);
        assert_ne!(Some(target.clone()), planner, "never the planner model");
        assert_ne!(target.model, "claude-fable-5");
    }

    /// The receipt: a delegate child's routing receipt carries its audit, and
    /// spawning it counts one dispatch — a non-pinned expensive child is
    /// neither cheapest-capable nor pack-honoured on the printed line.
    #[test]
    #[serial_test::serial(receipts_global_registry)]
    fn r63c_a_spawned_delegate_child_is_counted_on_receipt_cost_routing() {
        use crate::cost_router::routing_receipt;
        routing_receipt::reset();
        let receipt = serde_json::json!({
            "provider": "anthropic",
            "model": "claude-fable-5",
            "cost_audit": {"cheapest_capable": false, "pack_honored": false},
        });
        record_delegate_dispatch(&receipt);
        let after = routing_receipt::snapshot();
        assert!(after.dispatches >= 1, "{after:?}");
        assert!(after.cheapest_capable < after.dispatches, "{after:?}");
        assert!(after.pack_honored < after.dispatches, "{after:?}");
        assert!(after.routed_cloud >= 1, "{after:?}");

        // No audit ⇒ not guessed at, not counted.
        let before = routing_receipt::snapshot();
        record_delegate_dispatch(&serde_json::json!({"provider": "anthropic"}));
        assert_eq!(routing_receipt::snapshot().dispatches, before.dispatches);
        routing_receipt::reset();
    }

    #[test]
    fn r63c_live_goal_refusal_fires_only_for_in_progress_or_review() {
        let card = |state: &str| crate::cards::SessionGoalCard {
            card_id: format!("goal-{state}"),
            title: "t".to_string(),
            state_binding: state.to_string(),
            is_root: true,
            has_worker_session: true,
        };
        for state in [
            "ready",
            "complete",
            "cancelled",
            "failed",
            "blocked",
            "triage",
        ] {
            assert_eq!(live_goal_refusal(&[card(state)]), None, "{state}");
        }
        for state in ["in_progress", "review"] {
            let refusal = live_goal_refusal(&[card("complete"), card(state)])
                .unwrap_or_else(|| panic!("{state} must refuse"));
            assert!(
                refusal.contains(&format!("goal goal-{state} is running")),
                "{refusal}"
            );
            assert!(refusal.contains("goal_status or view_session"), "{refusal}");
            assert!(refusal.contains("wait for review"), "{refusal}");
        }
    }

    /// Move the fixture's goal card into the column bound to `state`.
    async fn r63_move_card(pool: &sqlx::Pool<sqlx::Sqlite>, card_id: &str, state: &str) {
        sqlx::query(
            "UPDATE cards SET column_id = (SELECT id FROM board_columns \
             WHERE project_id = ? AND state_binding = ?) WHERE id = ?",
        )
        .bind(crate::projects::PERSONAL_PROJECT_ID)
        .bind(state)
        .bind(card_id)
        .execute(pool)
        .await
        .unwrap();
    }

    fn r63_coding_recipe() -> Recipe {
        Recipe::builder()
            .version("1.0.0")
            .title(crate::tool_inspection::CODING_HARNESS_RECIPE_TITLE)
            .description("coding harness")
            .prompt("plan and dispatch")
            .build()
            .unwrap()
    }

    /// The s5 behaviour, end to end through the real `delegate` handler: a
    /// headless top-level coding coordinator whose goal is InProgress gets a
    /// refused call naming the goal and the resolution, before any recipe,
    /// provider or child session is built. Once the goal completes, the gate
    /// no longer fires.
    #[tokio::test]
    #[serial_test::serial(receipts_global_registry)]
    async fn r63c_a_coordinator_delegate_while_its_goal_is_in_progress_is_refused() {
        let (_tmp, sm, session, card_id) = active_roadmap_fixture(SessionType::User).await;
        let pool = sm.pool_clone().await.unwrap();
        r63_move_card(&pool, &card_id, "in_progress").await;
        sm.update(&session.id)
            .recipe(Some(r63_coding_recipe()))
            .apply()
            .await
            .unwrap();
        let client = SummonClient::new(PlatformExtensionContext {
            extension_manager: None,
            session_manager: sm.clone(),
            session: None,
        })
        .unwrap();

        let args = serde_json::json!({
            "instructions": "Check how the coupon trim worker is getting on and report back."
        });
        let error = client
            .handle_delegate(
                &session.id,
                args.as_object().cloned(),
                CancellationToken::new(),
            )
            .await
            .expect_err("a live goal must refuse the coordinator's delegate");
        assert!(
            error.contains(&format!("goal {card_id} is running")),
            "{error}"
        );
        assert!(error.contains("goal_status or view_session"), "{error}");

        let many = serde_json::json!({"tasks": [{"instructions":
            "Check how the coupon trim worker is getting on and report back."}]});
        let error = client
            .handle_delegate_many(
                &session.id,
                many.as_object().cloned(),
                CancellationToken::new(),
            )
            .await
            .expect_err("delegate_many is gated identically");
        assert!(error.contains("is running"), "{error}");

        let persisted = sm.get_session(&session.id, false).await.unwrap();
        r63_move_card(&pool, &card_id, "review").await;
        assert!(client
            .live_goal_delegate_refusal(&persisted)
            .await
            .is_some());
        // A card cannot enter `complete` without an answered approve decision
        // (trg_goal_complete_guard), and the pure `live_goal_refusal` test
        // above already covers `complete`; here any non-running state proves
        // the handler releases the gate once the goal stops running.
        r63_move_card(&pool, &card_id, "ready").await;
        assert!(
            client
                .live_goal_delegate_refusal(&persisted)
                .await
                .is_none(),
            "a goal that is no longer running releases the gate"
        );
    }

    /// Interactive (non-coding-harness) sessions, and a coding-harness session
    /// with a parent (a dispatched worker), are untouched even with a live goal.
    #[tokio::test]
    #[serial_test::serial(receipts_global_registry)]
    async fn r63c_interactive_sessions_and_workers_are_not_gated_by_a_live_goal() {
        let (_tmp, sm, session, card_id) = active_roadmap_fixture(SessionType::User).await;
        let pool = sm.pool_clone().await.unwrap();
        r63_move_card(&pool, &card_id, "in_progress").await;
        let client = SummonClient::new(PlatformExtensionContext {
            extension_manager: None,
            session_manager: sm.clone(),
            session: None,
        })
        .unwrap();

        // Interactive: no coding-harness recipe.
        assert!(session.recipe.is_none());
        assert!(client.live_goal_delegate_refusal(&session).await.is_none());

        // A coding-harness session WITH a parent is a worker, not the coordinator.
        let mut worker = session.clone();
        worker.recipe = Some(r63_coding_recipe());
        worker.parent_session_id = Some("coordinator-1".to_string());
        assert!(client.live_goal_delegate_refusal(&worker).await.is_none());
    }
}
