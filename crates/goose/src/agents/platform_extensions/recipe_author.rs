//! Recipe Author — lets the chat agent create, list, manage automations and skills.
//!
//! Thin wrapper over existing scheduler and skills infrastructure.
//! Tools: create_recipe, list_recipes, run_recipe, delete_recipe,
//!        pause_recipe, list_skills, save_skill.

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
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
                 Use create_recipe when the user wants to set up a new scheduled task. \
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

        let id = args
            .title
            .trim()
            .to_lowercase()
            .replace(|c: char| !c.is_ascii_alphanumeric() && c != ' ', "")
            .replace(' ', "-");

        if id.is_empty() {
            return Err("Title must contain at least one alphanumeric character".to_string());
        }

        let recipe = crate::recipe::Recipe {
            version: "1.0.0".to_string(),
            title: args.title.trim().to_string(),
            description: args.prompt.chars().take(120).collect(),
            prompt: Some(args.prompt.clone()),
            instructions: None,
            extensions: None,
            settings: None,
            activities: None,
            author: None,
            parameters: None,
            response: None,
            sub_recipes: None,
            retry: None,
        };

        let job = ScheduledJob {
            id: id.clone(),
            source: String::new(), // filled by add_scheduled_job
            cron: args.cron.clone(),
            last_run: None,
            currently_running: false,
            paused: false,
            current_session_id: None,
            process_start_time: None,
            worker_persona: None,
            starter_id: None,
            starter_version: None,
            starter_content_hash: None,
            user_customized: None,
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
            "Created automation \"{}\" scheduled with cron \"{}\".\n\
             Note: This PR supports basic recipe authoring (title, prompt, cron). \
             Richer features (parameters, sub-recipes, retry, extensions) are coming in a follow-up.",
            args.title.trim(),
            args.cron
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

// ── MCP trait implementation ────────────────────────────────────────────────

#[async_trait]
impl McpClientTrait for RecipeAuthorClient {
    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListToolsResult, Error> {
        let tools = vec![
            Tool::new(
                "create_recipe".to_string(),
                "Create a new scheduled automation. You MUST call this when the user asks to \
                 set up, create, or schedule an automation, recurring task, or scheduled job. \
                 Collects title, prompt (agent instructions), and cron schedule. \
                 This is the ONLY way to create automations — describing them in text does nothing."
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
        ];

        Ok(ListToolsResult {
            tools,
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
