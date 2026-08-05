//! Recipe Author — lets the chat agent create, list, manage automations and skills.
//!
//! Thin wrapper over existing scheduler and skills infrastructure.
//! Tools: create_recipe, list_recipes, run_recipe, delete_recipe,
//!        pause_recipe, list_skills, save_skill.
//!
//! `create_recipe` authors the full [`crate::recipe::Recipe`] shape — beyond
//! title/prompt/cron it wires input parameters, sub_recipes, retry with success
//! checks, extensions, model settings, and an optional worker_persona.

use crate::agents::extension::{Envs, ExtensionConfig, PlatformExtensionContext};
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use crate::agents::types::{RetryConfig, SuccessCheck};
use crate::recipe::{
    Recipe, RecipeParameter, RecipeParameterInputType, RecipeParameterRequirement, Settings,
    SubRecipe,
};
use crate::scheduler::ScheduledJob;
use crate::scheduler_trait::SchedulerTrait;
use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "recipe_author";

// ── Global scheduler handle ─────────────────────────────────────────────────

static GLOBAL_SCHEDULER: std::sync::OnceLock<Arc<dyn SchedulerTrait>> = std::sync::OnceLock::new();

pub fn set_global_scheduler(scheduler: Arc<dyn SchedulerTrait>) {
    let _ = GLOBAL_SCHEDULER.set(scheduler);
}

fn get_global_scheduler() -> Option<Arc<dyn SchedulerTrait>> {
    GLOBAL_SCHEDULER.get().cloned()
}

// ── Tool parameter schemas ──────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct CreateRecipeParams {
    /// Short name for the automation (e.g. "Weekly Downloads Cleanup").
    title: String,
    /// What the agent should do when this automation runs. Be specific and detailed.
    prompt: String,
    /// Cron expression for the schedule (e.g. "0 19 * * 0" for every Sunday at 7 PM).
    /// Use "0 8 * * 1-5" for weekday mornings, "0 0 * * *" for daily midnight, etc.
    cron: String,

    // ── Richer authoring (all optional) ──────────────────────────────────────
    /// Input parameters the recipe accepts. Each parameter is filled in when the
    /// recipe runs (from defaults, a saved value, or a user prompt) and can be
    /// referenced in the prompt with `{{ key }}` templating.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parameters: Option<Vec<RecipeParameterInput>>,
    /// Sub-recipes this automation can call. Each names a reusable recipe file to
    /// run as a step. Adding sub-recipes automatically enables the `summon` tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sub_recipes: Option<Vec<SubRecipeInput>>,
    /// Automatic retry + success-validation for the run. Re-runs up to
    /// `max_retries` times until every success check passes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    retry: Option<RetryInput>,
    /// Extensions (tools) to enable for this recipe's session. Supports builtin
    /// extensions (by name), stdio command extensions, and streamable_http
    /// extensions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extensions: Option<Vec<ExtensionInput>>,
    /// Model/provider settings for the recipe's session (provider, model,
    /// temperature, max turns).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    settings: Option<SettingsInput>,
    /// Optional worker persona key — runs the automation as a specific persona/
    /// character instead of the default agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    worker_persona: Option<String>,
}

/// Authoring shape for a single recipe parameter. Mirrors
/// [`crate::recipe::RecipeParameter`] with string-typed enums so the tool schema
/// is self-documenting for the model.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct RecipeParameterInput {
    /// The parameter name, referenced in the prompt as `{{ key }}`.
    key: String,
    /// One of: "string", "number", "boolean", "date", "file", "select".
    input_type: String,
    /// One of: "required", "optional", "user_prompt".
    requirement: String,
    /// Human-readable explanation of what this parameter is for.
    description: String,
    /// Default value used when the parameter is optional. Not allowed for "file".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default: Option<String>,
    /// Allowed values — required when `input_type` is "select".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    options: Option<Vec<String>>,
}

/// Authoring shape for a sub-recipe. Mirrors [`crate::recipe::SubRecipe`].
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct SubRecipeInput {
    /// Name used to identify this sub-recipe.
    name: String,
    /// Path to the sub-recipe file (yaml/json).
    path: String,
    /// Fixed parameter values to pass to the sub-recipe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    values: Option<HashMap<String, String>>,
    /// Run sequentially (not in parallel) when this sub-recipe is repeated.
    #[serde(default)]
    sequential_when_repeated: bool,
    /// Human-readable description of what the sub-recipe does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

/// Authoring shape for retry + success validation. Mirrors
/// [`crate::agents::types::RetryConfig`]; `checks` are shell commands that must
/// exit 0 for the run to count as successful.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct RetryInput {
    /// Maximum number of attempts before giving up. Must be greater than 0.
    max_retries: u32,
    /// Shell commands that must each exit 0 for the run to be considered
    /// successful. If any fails, the recipe is retried.
    #[serde(default)]
    checks: Vec<String>,
    /// Optional shell command run on failure (e.g. cleanup) before retrying.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    on_failure: Option<String>,
    /// Timeout in seconds for individual success-check commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_seconds: Option<u64>,
    /// Timeout in seconds for the on_failure command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    on_failure_timeout_seconds: Option<u64>,
}

/// Authoring shape for an extension to enable. Covers the common cases: builtin
/// extensions (by name), stdio command extensions, and streamable_http.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ExtensionInput {
    /// Transport type: "builtin" (default), "stdio", or "streamable_http".
    #[serde(default = "default_extension_type")]
    r#type: String,
    /// Name used to identify the extension (for builtin, the builtin's name).
    name: String,
    /// Short description of the extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    /// For stdio extensions: the command to run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cmd: Option<String>,
    /// For stdio extensions: command arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    args: Option<Vec<String>>,
    /// For streamable_http extensions: the endpoint URI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    uri: Option<String>,
    /// Optional timeout in seconds for the extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout: Option<u64>,
}

fn default_extension_type() -> String {
    "builtin".to_string()
}

/// Authoring shape for recipe settings. Mirrors [`crate::recipe::Settings`].
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct SettingsInput {
    /// Provider to use (e.g. "anthropic", "openai").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    goose_provider: Option<String>,
    /// Model to use (e.g. "claude-sonnet-4").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    goose_model: Option<String>,
    /// Sampling temperature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    /// Maximum number of agent turns per run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_turns: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct RecipeIdParams {
    /// The schedule ID of the automation to operate on.
    id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct SaveSkillParams {
    /// Human-readable name for the skill.
    name: String,
    /// What this skill does — one or two sentences.
    #[serde(default)]
    description: Option<String>,
    /// The tool that this skill wraps (from the detected pattern).
    tool_used: String,
    /// The argument shape hash from the detected pattern.
    argument_shape_hash: String,
    /// JSON definition of the skill behavior.
    definition_json: serde_json::Value,
}

fn schema<T: JsonSchema>() -> JsonObject {
    let mut obj = serde_json::to_value(schema_for!(T))
        .map(|v| v.as_object().unwrap().clone())
        .expect("valid schema");
    obj.entry("properties")
        .or_insert_with(|| serde_json::json!({}));
    obj
}

// ── Client ──────────────────────────────────────────────────────────────────

pub struct RecipeAuthorClient {
    info: InitializeResult,
    context: PlatformExtensionContext,
}

impl RecipeAuthorClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME, "1.0.0").with_title("Recipe Author"),
            )
            .with_instructions(
                "Manage the user's automations (scheduled recipes) and saved skills. \
                 Use create_recipe when the user wants to set up a new scheduled task — it \
                 supports richer authoring beyond title/prompt/cron: input parameters, \
                 sub_recipes, retry with success checks, extensions, model settings, and a \
                 worker_persona. \
                 Use list_recipes to show what automations exist. \
                 Use save_skill when the user asks to save a repeated behavior.",
            );
        Ok(Self { info, context })
    }

    async fn handle_create_recipe(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args = parse_args::<CreateRecipeParams>(arguments)?;
        let scheduler =
            get_global_scheduler().ok_or_else(|| "Scheduler not initialized".to_string())?;

        let id = slugify_title(&args.title)?;
        let recipe = build_recipe(&args)?;
        let rich_summary = describe_rich_fields(&recipe);

        let job = ScheduledJob {
            id: id.clone(),
            source: String::new(), // filled by add_scheduled_job
            cron: args.cron.clone(),
            last_run: None,
            currently_running: false,
            // Agent-created automations never start live: they land paused and
            // the user approves by resuming in Automate. This is what stops a
            // headless session from re-creating a deleted job behind the
            // user's back and having it fire (2026-08-05 credit burn).
            paused: true,
            requires_approval: true,
            current_session_id: None,
            process_start_time: None,
            worker_persona: args
                .worker_persona
                .as_ref()
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty()),
            starter_id: None,
            starter_version: None,
            starter_content_hash: None,
            user_customized: None,
            ..Default::default()
        };

        // Write recipe to disk and register with scheduler
        let recipes_dir = crate::scheduler::get_default_scheduled_recipes_dir()
            .map_err(|e| format!("Failed to resolve recipes dir: {}", e))?;
        std::fs::create_dir_all(&recipes_dir)
            .map_err(|e| format!("Failed to create recipes dir: {}", e))?;
        let recipe_path = recipes_dir.join(format!("{}.yaml", id));
        let yaml = serde_yaml::to_string(&recipe)
            .map_err(|e| format!("Failed to serialize recipe: {}", e))?;
        std::fs::write(&recipe_path, &yaml)
            .map_err(|e| format!("Failed to write recipe: {}", e))?;

        let mut job_with_source = job;
        job_with_source.source = recipe_path.to_string_lossy().to_string();

        scheduler
            .add_scheduled_job(job_with_source, false)
            .await
            .map_err(|e| format!("Failed to schedule: {}", e))?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Created automation \"{}\" with cron \"{}\" — PAUSED, awaiting the user's \
             approval. It will not run until the user resumes it from the Automate tab. \
             Do not attempt to resume it yourself and do not create variants of it; tell \
             the user it is waiting for their approval.{}",
            args.title.trim(),
            args.cron,
            rich_summary,
        ))]))
    }

    async fn handle_list_recipes(
        &self,
        _arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let scheduler =
            get_global_scheduler().ok_or_else(|| "Scheduler not initialized".to_string())?;

        let jobs = scheduler.list_scheduled_jobs().await;
        if jobs.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No automations are configured yet.",
            )]));
        }

        let mut lines = vec![format!("{} automation(s):\n", jobs.len())];
        for job in &jobs {
            let status = if job.currently_running {
                "RUNNING"
            } else if job.paused {
                "PAUSED"
            } else {
                "active"
            };

            // Try to read recipe title from the YAML file
            let title = read_recipe_title(&job.source).unwrap_or_else(|| job.id.clone());
            let last = job
                .last_run
                .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "never".to_string());

            lines.push(format!(
                "- **{}** (id: {}) — cron: `{}`, status: {}, last run: {}",
                title, job.id, job.cron, status, last
            ));
        }

        Ok(CallToolResult::success(vec![Content::text(
            lines.join("\n"),
        )]))
    }

    async fn handle_run_recipe(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args = parse_args::<RecipeIdParams>(arguments)?;
        let scheduler =
            get_global_scheduler().ok_or_else(|| "Scheduler not initialized".to_string())?;

        // An agent may not run a paused automation — paused is either the
        // user's explicit choice or the pending-approval state, and run_now
        // firing regardless was the hole that defeated the approval gate.
        if let Some(job) = scheduler
            .list_scheduled_jobs()
            .await
            .iter()
            .find(|j| j.id == args.id)
        {
            if job.paused {
                return Err(format!(
                    "Automation \"{}\" is paused{} and cannot be run by an agent. \
                     The user can run or resume it from the Automate tab.",
                    args.id,
                    if job.requires_approval {
                        " awaiting the user's approval"
                    } else {
                        ""
                    }
                ));
            }
        }

        let session_id = scheduler
            .run_now(&args.id)
            .await
            .map_err(|e| format!("Failed to run: {}", e))?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Started automation \"{}\" (session: {})",
            args.id, session_id
        ))]))
    }

    async fn handle_delete_recipe(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args = parse_args::<RecipeIdParams>(arguments)?;
        let scheduler =
            get_global_scheduler().ok_or_else(|| "Scheduler not initialized".to_string())?;

        scheduler
            .remove_scheduled_job(&args.id, true)
            .await
            .map_err(|e| format!("Failed to delete: {}", e))?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Deleted automation \"{}\".",
            args.id
        ))]))
    }

    async fn handle_pause_recipe(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args = parse_args::<RecipeIdParams>(arguments)?;
        let scheduler =
            get_global_scheduler().ok_or_else(|| "Scheduler not initialized".to_string())?;

        // Check current state to decide pause vs unpause
        let jobs = scheduler.list_scheduled_jobs().await;
        let job = jobs.iter().find(|j| j.id == args.id).ok_or_else(|| {
            format!(
                "Automation \"{}\" not found. Available: {}",
                args.id,
                jobs.iter()
                    .map(|j| j.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;

        if job.paused {
            if job.requires_approval {
                return Err(format!(
                    "Automation \"{}\" is paused awaiting the USER's approval and cannot be \
                     resumed by an agent. The user must resume it from the Automate tab.",
                    args.id
                ));
            }
            scheduler
                .unpause_schedule(&args.id)
                .await
                .map_err(|e| format!("Failed to unpause: {}", e))?;
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Resumed automation \"{}\".",
                args.id
            ))]))
        } else {
            scheduler
                .pause_schedule(&args.id)
                .await
                .map_err(|e| format!("Failed to pause: {}", e))?;
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Paused automation \"{}\".",
                args.id
            ))]))
        }
    }

    async fn handle_list_skills(
        &self,
        _arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| format!("DB not available: {}", e))?;

        let skills = crate::skills::list_skills(&pool)
            .await
            .map_err(|e| format!("Failed to list skills: {}", e))?;

        if skills.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No saved skills yet. Skills are auto-detected when you repeat similar tasks, \
                 or you can ask me to save a behavior as a skill.",
            )]));
        }

        let mut lines = vec![format!("{} skill(s):\n", skills.len())];
        for s in &skills {
            let desc = s.description.as_deref().unwrap_or("no description");
            lines.push(format!(
                "- **{}** — {} (triggered {} time(s), status: {})",
                s.name, desc, s.trigger_count, s.status
            ));
        }

        Ok(CallToolResult::success(vec![Content::text(
            lines.join("\n"),
        )]))
    }

    async fn handle_save_skill(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args = parse_args::<SaveSkillParams>(arguments)?;
        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| format!("DB not available: {}", e))?;

        let params = crate::skills::CreateSkillParams {
            name: args.name.clone(),
            description: args.description,
            tool_used: args.tool_used,
            argument_shape_hash: args.argument_shape_hash,
            definition_json: args.definition_json,
            source_task_id: None,
        };

        let created = crate::skills::create_skill(&pool, params)
            .await
            .map_err(|e| format!("Failed to save skill: {}", e))?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Saved skill \"{}\" (id: {}).",
            created.name, created.id
        ))]))
    }
}

impl RecipeAuthorClient {
    /// The full, static tool inventory. Extracted from `list_tools` so the
    /// self-knowledge completeness guard derives its inventory from the REAL
    /// list — add a tool here and CI fails until the registry `description`
    /// names it.
    pub(crate) fn get_tools() -> Vec<Tool> {
        vec![
            Tool::new(
                "create_recipe".to_string(),
                "Create a new scheduled automation. You MUST call this when the user asks to \
                 set up, create, or schedule an automation, recurring task, or scheduled job. \
                 Collects title, prompt (agent instructions), and cron schedule, plus optional \
                 richer authoring: input parameters, sub_recipes, automatic retry with success \
                 checks, extensions (tools) to enable, model/provider settings, and a \
                 worker_persona to run as. \
                 This is the ONLY way to create automations — describing them in text does nothing. \
                 New automations are created PAUSED and run only after the user approves them \
                 in Automate — never create a second variant because one appears inactive, and \
                 never recreate an automation the user deleted. Schedules may not fire more often \
                 than every 15 minutes. Headless runs get a minimal toolset unless the recipe \
                 declares `extensions` explicitly."
                    .to_string(),
                schema::<CreateRecipeParams>(),
            ),
            Tool::new(
                "list_recipes".to_string(),
                "List all scheduled automations. You MUST call this when the user asks about \
                 their automations, schedules, or recurring tasks. Returns names, cron schedules, \
                 statuses, and last run times."
                    .to_string(),
                schema::<JsonObject>(),
            ),
            Tool::new(
                "run_recipe".to_string(),
                "Run a scheduled automation immediately. You MUST call this when the user asks \
                 to run, trigger, or execute an automation right now."
                    .to_string(),
                schema::<RecipeIdParams>(),
            ),
            Tool::new(
                "delete_recipe".to_string(),
                "Delete a scheduled automation permanently. You MUST call this when the user \
                 asks to remove or delete an automation. Always confirm with the user before \
                 deleting."
                    .to_string(),
                schema::<RecipeIdParams>(),
            ),
            Tool::new(
                "pause_recipe".to_string(),
                "Toggle pause/resume on a scheduled automation. If paused, it resumes; if \
                 active, it pauses. You MUST call this when the user asks to pause, resume, \
                 stop temporarily, or re-enable an automation."
                    .to_string(),
                schema::<RecipeIdParams>(),
            ),
            Tool::new(
                "list_skills".to_string(),
                "List all saved skills. You MUST call this when the user asks about their \
                 skills, learned behaviors, or saved patterns."
                    .to_string(),
                schema::<JsonObject>(),
            ),
            Tool::new(
                "save_skill".to_string(),
                "Save a detected behavior pattern as a reusable skill. Call this when the \
                 user asks to save, remember, or keep a behavior they've been repeating."
                    .to_string(),
                schema::<SaveSkillParams>(),
            ),
        ]
    }
}

// ── MCP trait implementation ────────────────────────────────────────────────

#[async_trait]
impl McpClientTrait for RecipeAuthorClient {
    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListToolsResult, Error> {
        Ok(ListToolsResult {
            tools: Self::get_tools(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        _ctx: &ToolCallContext,
        name: &str,
        arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<CallToolResult, Error> {
        let result = match name {
            "create_recipe" => self.handle_create_recipe(arguments).await,
            "list_recipes" => self.handle_list_recipes(arguments).await,
            "run_recipe" => self.handle_run_recipe(arguments).await,
            "delete_recipe" => self.handle_delete_recipe(arguments).await,
            "pause_recipe" => self.handle_pause_recipe(arguments).await,
            "list_skills" => self.handle_list_skills(arguments).await,
            "save_skill" => self.handle_save_skill(arguments).await,
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

// ── Helpers ─────────────────────────────────────────────────────────────────

fn parse_args<T: for<'de> Deserialize<'de>>(
    arguments: Option<JsonObject>,
) -> std::result::Result<T, String> {
    arguments
        .map(|obj| serde_json::from_value(serde_json::Value::Object(obj)))
        .transpose()
        .map_err(|e| format!("Invalid arguments: {}", e))?
        .ok_or_else(|| "Missing arguments".to_string())
}

fn read_recipe_title(source: &str) -> Option<String> {
    let content = std::fs::read_to_string(source).ok()?;
    let recipe: crate::recipe::Recipe = serde_yaml::from_str(&content).ok()?;
    Some(recipe.title)
}

/// Derive a filesystem/schedule id slug from a recipe title.
fn slugify_title(title: &str) -> std::result::Result<String, String> {
    let id = title
        .trim()
        .to_lowercase()
        .replace(|c: char| !c.is_ascii_alphanumeric() && c != ' ', "")
        .replace(' ', "-");
    if id.is_empty() {
        return Err("Title must contain at least one alphanumeric character".to_string());
    }
    Ok(id)
}

/// Build a fully-formed [`Recipe`] from the authoring params, wiring each richer
/// field (parameters, sub_recipes, retry, extensions, settings) into its real
/// `Recipe` type. Pure — no scheduler/disk access — so it round-trips in tests.
fn build_recipe(args: &CreateRecipeParams) -> std::result::Result<Recipe, String> {
    let parameters = match &args.parameters {
        Some(params) => Some(
            params
                .iter()
                .map(convert_parameter)
                .collect::<std::result::Result<Vec<_>, _>>()?,
        ),
        None => None,
    };

    let sub_recipes = args.sub_recipes.as_ref().map(|subs| {
        subs.iter()
            .map(|s| SubRecipe {
                name: s.name.clone(),
                path: s.path.clone(),
                values: s.values.clone(),
                sequential_when_repeated: s.sequential_when_repeated,
                description: s.description.clone(),
            })
            .collect::<Vec<_>>()
    });

    let retry = match &args.retry {
        Some(r) => Some(convert_retry(r)?),
        None => None,
    };

    let extensions = match &args.extensions {
        Some(exts) => Some(
            exts.iter()
                .map(convert_extension)
                .collect::<std::result::Result<Vec<_>, _>>()?,
        ),
        None => None,
    };

    let settings = args.settings.as_ref().map(|s| Settings {
        goose_provider: s.goose_provider.clone(),
        goose_model: s.goose_model.clone(),
        temperature: s.temperature,
        max_turns: s.max_turns,
    });

    let mut recipe = Recipe {
        version: "1.0.0".to_string(),
        title: args.title.trim().to_string(),
        description: args.prompt.chars().take(120).collect(),
        prompt: Some(args.prompt.clone()),
        instructions: None,
        extensions,
        settings,
        activities: None,
        author: None,
        parameters,
        response: None,
        sub_recipes,
        retry,
    };

    // Sub-recipes require the `summon` tool to invoke them — mirror the same
    // auto-injection Recipe::from_content applies on read, so the written recipe
    // is already self-consistent.
    if recipe.sub_recipes.is_some() {
        let has_summon = recipe
            .extensions
            .as_ref()
            .is_some_and(|exts| exts.iter().any(|e| e.name() == "summon"));
        if !has_summon {
            let summon = ExtensionConfig::Platform {
                name: "summon".to_string(),
                description: String::new(),
                display_name: None,
                bundled: None,
                available_tools: vec![],
            };
            match &mut recipe.extensions {
                Some(exts) => exts.push(summon),
                None => recipe.extensions = Some(vec![summon]),
            }
        }
    }

    Ok(recipe)
}

fn convert_parameter(p: &RecipeParameterInput) -> std::result::Result<RecipeParameter, String> {
    let input_type: RecipeParameterInputType =
        serde_json::from_value(serde_json::Value::String(p.input_type.clone())).map_err(|_| {
            format!(
                "Invalid input_type \"{}\" for parameter \"{}\": expected one of \
                 string, number, boolean, date, file, select",
                p.input_type, p.key
            )
        })?;
    let requirement: RecipeParameterRequirement =
        serde_json::from_value(serde_json::Value::String(p.requirement.clone())).map_err(|_| {
            format!(
                "Invalid requirement \"{}\" for parameter \"{}\": expected one of \
                 required, optional, user_prompt",
                p.requirement, p.key
            )
        })?;

    if matches!(input_type, RecipeParameterInputType::File) && p.default.is_some() {
        return Err(format!(
            "Parameter \"{}\" is a file parameter and cannot have a default value",
            p.key
        ));
    }
    if matches!(input_type, RecipeParameterInputType::Select)
        && p.options.as_ref().map(|o| o.is_empty()).unwrap_or(true)
    {
        return Err(format!(
            "Parameter \"{}\" is a select parameter and must provide non-empty options",
            p.key
        ));
    }

    Ok(RecipeParameter {
        key: p.key.clone(),
        input_type,
        requirement,
        description: p.description.clone(),
        default: p.default.clone(),
        options: p.options.clone(),
    })
}

fn convert_retry(r: &RetryInput) -> std::result::Result<RetryConfig, String> {
    let config = RetryConfig {
        max_retries: r.max_retries,
        checks: r
            .checks
            .iter()
            .map(|command| SuccessCheck::Shell {
                command: command.clone(),
            })
            .collect(),
        on_failure: r.on_failure.clone(),
        timeout_seconds: r.timeout_seconds,
        on_failure_timeout_seconds: r.on_failure_timeout_seconds,
    };
    config.validate()?;
    Ok(config)
}

fn convert_extension(e: &ExtensionInput) -> std::result::Result<ExtensionConfig, String> {
    let description = e.description.clone().unwrap_or_default();
    match e.r#type.as_str() {
        "builtin" => Ok(ExtensionConfig::Builtin {
            name: e.name.clone(),
            description,
            display_name: None,
            timeout: e.timeout,
            bundled: None,
            available_tools: vec![],
        }),
        "stdio" => {
            let cmd = e
                .cmd
                .clone()
                .ok_or_else(|| format!("stdio extension \"{}\" requires a `cmd`", e.name))?;
            Ok(ExtensionConfig::Stdio {
                name: e.name.clone(),
                description,
                cmd,
                args: e.args.clone().unwrap_or_default(),
                envs: Envs::default(),
                env_keys: vec![],
                timeout: e.timeout,
                bundled: None,
                available_tools: vec![],
            })
        }
        "streamable_http" => {
            let uri = e.uri.clone().ok_or_else(|| {
                format!("streamable_http extension \"{}\" requires a `uri`", e.name)
            })?;
            Ok(ExtensionConfig::StreamableHttp {
                name: e.name.clone(),
                description,
                uri,
                envs: Envs::default(),
                env_keys: vec![],
                headers: HashMap::new(),
                timeout: e.timeout,
                socket: None,
                bundled: None,
                available_tools: vec![],
            })
        }
        other => Err(format!(
            "Unknown extension type \"{}\" for extension \"{}\": expected one of \
             builtin, stdio, streamable_http",
            other, e.name
        )),
    }
}

/// Human-readable trailer summarizing which richer fields were authored, appended
/// to the create_recipe success message.
fn describe_rich_fields(recipe: &Recipe) -> String {
    let mut parts = Vec::new();
    if let Some(p) = &recipe.parameters {
        parts.push(format!("{} parameter(s)", p.len()));
    }
    if let Some(s) = &recipe.sub_recipes {
        parts.push(format!("{} sub-recipe(s)", s.len()));
    }
    if let Some(e) = &recipe.extensions {
        parts.push(format!("{} extension(s)", e.len()));
    }
    if recipe.retry.is_some() {
        parts.push("retry/validation".to_string());
    }
    if recipe.settings.is_some() {
        parts.push("custom settings".to_string());
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" Includes: {}.", parts.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn params_from(value: serde_json::Value) -> CreateRecipeParams {
        serde_json::from_value(value).expect("valid CreateRecipeParams")
    }

    #[test]
    fn test_slugify_title() {
        assert_eq!(
            slugify_title("Weekly Downloads Cleanup!").unwrap(),
            "weekly-downloads-cleanup"
        );
        assert!(slugify_title("   ").is_err());
        assert!(slugify_title("***").is_err());
    }

    #[test]
    fn test_basic_recipe_still_works() {
        let args = params_from(json!({
            "title": "Nightly Backup",
            "prompt": "Back up the important folders.",
            "cron": "0 0 * * *"
        }));
        let recipe = build_recipe(&args).unwrap();
        assert_eq!(recipe.title, "Nightly Backup");
        assert_eq!(
            recipe.prompt.as_deref(),
            Some("Back up the important folders.")
        );
        assert!(recipe.parameters.is_none());
        assert!(recipe.sub_recipes.is_none());
        assert!(recipe.retry.is_none());
        assert!(recipe.extensions.is_none());
        assert!(recipe.settings.is_none());
    }

    /// The headline test: author a recipe WITH parameters, sub_recipes, retry,
    /// extensions, and settings, then round-trip it through YAML and assert every
    /// richer field survives.
    #[test]
    fn test_rich_authoring_round_trips() {
        let args = params_from(json!({
            "title": "Rich Recipe",
            "prompt": "Do the rich thing with {{ target }}.",
            "cron": "0 8 * * 1-5",
            "parameters": [
                {
                    "key": "target",
                    "input_type": "string",
                    "requirement": "required",
                    "description": "What to act on"
                },
                {
                    "key": "mode",
                    "input_type": "select",
                    "requirement": "optional",
                    "description": "Operating mode",
                    "default": "fast",
                    "options": ["fast", "thorough"]
                }
            ],
            "sub_recipes": [
                {
                    "name": "cleanup",
                    "path": "cleanup.yaml",
                    "values": { "scope": "temp" },
                    "sequential_when_repeated": true,
                    "description": "Clean up afterwards"
                }
            ],
            "retry": {
                "max_retries": 3,
                "checks": ["test -f /tmp/done", "grep -q OK /tmp/result"],
                "on_failure": "rm -f /tmp/lock",
                "timeout_seconds": 120,
                "on_failure_timeout_seconds": 60
            },
            "extensions": [
                { "type": "builtin", "name": "developer" },
                {
                    "type": "stdio",
                    "name": "fetch",
                    "cmd": "uvx",
                    "args": ["mcp-server-fetch"]
                }
            ],
            "settings": {
                "goose_provider": "anthropic",
                "goose_model": "claude-sonnet-4",
                "temperature": 0.2,
                "max_turns": 25
            },
            "worker_persona": "the-librarian"
        }));

        let recipe = build_recipe(&args).unwrap();

        // Serialize to YAML and read it back (exact serde round-trip).
        let yaml = serde_yaml::to_string(&recipe).unwrap();
        let back: Recipe = serde_yaml::from_str(&yaml).unwrap();

        // Parameters
        let params = back.parameters.expect("parameters preserved");
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].key, "target");
        assert!(matches!(
            params[0].input_type,
            RecipeParameterInputType::String
        ));
        assert!(matches!(
            params[0].requirement,
            RecipeParameterRequirement::Required
        ));
        assert_eq!(params[1].key, "mode");
        assert!(matches!(
            params[1].input_type,
            RecipeParameterInputType::Select
        ));
        assert_eq!(params[1].default.as_deref(), Some("fast"));
        assert_eq!(
            params[1].options.as_ref().unwrap(),
            &vec!["fast".to_string(), "thorough".to_string()]
        );

        // Sub-recipes
        let subs = back.sub_recipes.expect("sub_recipes preserved");
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].name, "cleanup");
        assert_eq!(subs[0].path, "cleanup.yaml");
        assert!(subs[0].sequential_when_repeated);
        assert_eq!(
            subs[0].values,
            Some(HashMap::from([("scope".to_string(), "temp".to_string())]))
        );

        // Retry — the REAL Recipe.retry shape (max_retries/checks/on_failure)
        let retry = back.retry.expect("retry preserved");
        assert_eq!(retry.max_retries, 3);
        assert_eq!(retry.checks.len(), 2);
        assert!(matches!(
            &retry.checks[0],
            SuccessCheck::Shell { command } if command == "test -f /tmp/done"
        ));
        assert_eq!(retry.on_failure.as_deref(), Some("rm -f /tmp/lock"));
        assert_eq!(retry.timeout_seconds, Some(120));
        assert_eq!(retry.on_failure_timeout_seconds, Some(60));

        // Extensions — developer, fetch, and auto-injected summon (for sub_recipes)
        let exts = back.extensions.expect("extensions preserved");
        let names: Vec<String> = exts.iter().map(|e| e.name()).collect();
        assert!(names.contains(&"developer".to_string()));
        assert!(names.contains(&"fetch".to_string()));
        assert!(names.contains(&"summon".to_string()));
        let fetch = exts.iter().find(|e| e.name() == "fetch").unwrap();
        assert!(matches!(fetch, ExtensionConfig::Stdio { cmd, .. } if cmd == "uvx"));

        // Settings
        let settings = back.settings.expect("settings preserved");
        assert_eq!(settings.goose_provider.as_deref(), Some("anthropic"));
        assert_eq!(settings.goose_model.as_deref(), Some("claude-sonnet-4"));
        assert_eq!(settings.temperature, Some(0.2));
        assert_eq!(settings.max_turns, Some(25));
    }

    #[test]
    fn test_sub_recipes_auto_inject_summon() {
        let args = params_from(json!({
            "title": "Sub Only",
            "prompt": "Run sub-recipes.",
            "cron": "0 0 * * *",
            "sub_recipes": [ { "name": "step", "path": "step.yaml" } ]
        }));
        let recipe = build_recipe(&args).unwrap();
        let exts = recipe.extensions.expect("summon injected");
        assert!(exts.iter().any(|e| e.name() == "summon"));
    }

    #[test]
    fn test_invalid_input_type_errors() {
        let args = params_from(json!({
            "title": "Bad Param",
            "prompt": "x",
            "cron": "0 0 * * *",
            "parameters": [
                { "key": "k", "input_type": "banana", "requirement": "required", "description": "d" }
            ]
        }));
        let err = build_recipe(&args).unwrap_err();
        assert!(err.contains("Invalid input_type"), "got: {err}");
    }

    #[test]
    fn test_file_param_with_default_errors() {
        let args = params_from(json!({
            "title": "File Param",
            "prompt": "x",
            "cron": "0 0 * * *",
            "parameters": [
                {
                    "key": "f",
                    "input_type": "file",
                    "requirement": "optional",
                    "description": "d",
                    "default": "/etc/passwd"
                }
            ]
        }));
        let err = build_recipe(&args).unwrap_err();
        assert!(err.contains("cannot have a default"), "got: {err}");
    }

    #[test]
    fn test_select_param_without_options_errors() {
        let args = params_from(json!({
            "title": "Select Param",
            "prompt": "x",
            "cron": "0 0 * * *",
            "parameters": [
                { "key": "s", "input_type": "select", "requirement": "required", "description": "d" }
            ]
        }));
        let err = build_recipe(&args).unwrap_err();
        assert!(err.contains("non-empty options"), "got: {err}");
    }

    #[test]
    fn test_retry_zero_max_retries_errors() {
        let args = params_from(json!({
            "title": "Bad Retry",
            "prompt": "x",
            "cron": "0 0 * * *",
            "retry": { "max_retries": 0 }
        }));
        let err = build_recipe(&args).unwrap_err();
        assert!(
            err.contains("max_retries must be greater than 0"),
            "got: {err}"
        );
    }

    #[test]
    fn test_stdio_extension_requires_cmd() {
        let args = params_from(json!({
            "title": "Bad Ext",
            "prompt": "x",
            "cron": "0 0 * * *",
            "extensions": [ { "type": "stdio", "name": "broken" } ]
        }));
        let err = build_recipe(&args).unwrap_err();
        assert!(err.contains("requires a `cmd`"), "got: {err}");
    }

    #[test]
    fn test_unknown_extension_type_errors() {
        let args = params_from(json!({
            "title": "Bad Ext Type",
            "prompt": "x",
            "cron": "0 0 * * *",
            "extensions": [ { "type": "carrier_pigeon", "name": "nope" } ]
        }));
        let err = build_recipe(&args).unwrap_err();
        assert!(err.contains("Unknown extension type"), "got: {err}");
    }

    #[test]
    fn test_describe_rich_fields_trailer() {
        let args = params_from(json!({
            "title": "Trailer",
            "prompt": "x",
            "cron": "0 0 * * *",
            "parameters": [
                { "key": "k", "input_type": "string", "requirement": "required", "description": "d" }
            ],
            "retry": { "max_retries": 2 }
        }));
        let recipe = build_recipe(&args).unwrap();
        let trailer = describe_rich_fields(&recipe);
        assert!(trailer.contains("1 parameter(s)"), "got: {trailer}");
        assert!(trailer.contains("retry/validation"), "got: {trailer}");
    }
}
