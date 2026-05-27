use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use crate::projects;
use anyhow::Result;
use async_trait::async_trait;
use indoc::indoc;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool, ToolAnnotations,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "projectmanager";

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ProjectCreateParams {
    /// The project name (required)
    name: String,
    /// Filesystem path to the project root (optional)
    root_path: Option<String>,
    /// Production site URL (optional)
    site_url: Option<String>,
    /// Git repository URL (optional)
    repo_url: Option<String>,
    /// Short description of the project (optional)
    description: Option<String>,
    /// Tags for categorization (optional)
    tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ProjectUpdateParams {
    /// Project ID (UUID) or slug to identify the project
    id_or_slug: String,
    /// New name (optional)
    name: Option<String>,
    /// New slug (optional)
    slug: Option<String>,
    /// New description (optional)
    description: Option<String>,
    /// New status: active, paused, or archived (optional)
    status: Option<String>,
    /// New root path, or null to clear (optional)
    root_path: Option<Option<String>>,
    /// New site URL, or null to clear (optional)
    site_url: Option<Option<String>>,
    /// New repo URL, or null to clear (optional)
    repo_url: Option<Option<String>>,
    /// New notes (optional)
    notes: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ProjectDeleteParams {
    /// Project ID (UUID) or slug to delete
    id_or_slug: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ProjectListParams {
    /// Filter by status: active, paused, or archived (optional, defaults to all)
    status: Option<String>,
}

pub struct ProjectManagerClient {
    info: InitializeResult,
    context: PlatformExtensionContext,
}

impl ProjectManagerClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME.to_string(), "1.0.0".to_string())
                    .with_title("Project Manager"),
            )
            .with_instructions(indoc! {r#"
                Manage user projects. Projects organize work into named workspaces with
                filesystem paths, URLs, and metadata. Each project has a slug (stable
                identifier), name (display label), and optional root_path, site_url,
                and repo_url.

                The implicit "Personal" project always exists and cannot be deleted.
            "#}.to_string());

        Ok(Self { info, context })
    }

    async fn handle_create(&self, arguments: Option<JsonObject>) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let name = args.get("name").and_then(|v| v.as_str()).ok_or("Missing required parameter: name")?.to_string();
        let pool = self.context.session_manager.pool_clone().await.map_err(|e| e.to_string())?;
        let input = projects::CreateProject {
            name,
            slug: None,
            description: args.get("description").and_then(|v| v.as_str()).map(String::from),
            root_path: args.get("root_path").and_then(|v| v.as_str()).map(String::from),
            site_url: args.get("site_url").and_then(|v| v.as_str()).map(String::from),
            repo_url: args.get("repo_url").and_then(|v| v.as_str()).map(String::from),
            notes: None,
            tags: args.get("tags").and_then(|v| v.as_array()).map(|arr| {
                arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()
            }),
        };
        let project = projects::create_project(&pool, input).await?;
        let json = serde_json::json!({
            "id": project.id, "slug": project.slug, "name": project.name,
            "description": project.description, "status": project.status,
            "root_path": project.root_path, "site_url": project.site_url,
            "repo_url": project.repo_url, "tags": project.tags,
        });
        Ok(vec![Content::text(format!(
            "Created project \"{}\" (slug: {}, id: {})\n\n{}",
            project.name, project.slug, project.id,
            serde_json::to_string_pretty(&json).unwrap_or_default()
        ))])
    }

    async fn handle_update(&self, arguments: Option<JsonObject>) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let id_or_slug = args.get("id_or_slug").and_then(|v| v.as_str()).ok_or("Missing required parameter: id_or_slug")?;
        let pool = self.context.session_manager.pool_clone().await.map_err(|e| e.to_string())?;
        let project = projects::get_project_by_id_or_slug(&pool, id_or_slug).await?.ok_or_else(|| format!("Project '{}' not found", id_or_slug))?;
        let input = projects::UpdateProject {
            name: args.get("name").and_then(|v| v.as_str()).map(String::from),
            slug: args.get("slug").and_then(|v| v.as_str()).map(String::from),
            description: args.get("description").and_then(|v| v.as_str()).map(String::from),
            status: args.get("status").and_then(|v| v.as_str()).map(String::from),
            root_path: args.get("root_path").map(|v| v.as_str().map(String::from)),
            site_url: args.get("site_url").map(|v| v.as_str().map(String::from)),
            repo_url: args.get("repo_url").map(|v| v.as_str().map(String::from)),
            notes: args.get("notes").and_then(|v| v.as_str()).map(String::from),
        };
        let updated = projects::update_project(&pool, &project.id, input).await?.ok_or("Project not found after update")?;
        let json = serde_json::json!({
            "id": updated.id, "slug": updated.slug, "name": updated.name,
            "status": updated.status, "root_path": updated.root_path,
            "site_url": updated.site_url, "repo_url": updated.repo_url,
        });
        Ok(vec![Content::text(format!(
            "Updated project \"{}\" (slug: {})\n\n{}",
            updated.name, updated.slug,
            serde_json::to_string_pretty(&json).unwrap_or_default()
        ))])
    }

    async fn handle_delete(&self, arguments: Option<JsonObject>) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let id_or_slug = args.get("id_or_slug").and_then(|v| v.as_str()).ok_or("Missing required parameter: id_or_slug")?;
        let pool = self.context.session_manager.pool_clone().await.map_err(|e| e.to_string())?;
        let project = projects::get_project_by_id_or_slug(&pool, id_or_slug).await?.ok_or_else(|| format!("Project '{}' not found", id_or_slug))?;
        projects::delete_project(&pool, &project.id).await?;
        Ok(vec![Content::text(format!("Deleted project \"{}\" (slug: {}, id: {})", project.name, project.slug, project.id))])
    }

    async fn handle_list(&self, arguments: Option<JsonObject>) -> Result<Vec<Content>, String> {
        let status = arguments.as_ref().and_then(|a| a.get("status")).and_then(|v| v.as_str()).map(String::from);
        let pool = self.context.session_manager.pool_clone().await.map_err(|e| e.to_string())?;
        let items = projects::list_projects(&pool, status.as_deref()).await?;
        let json: Vec<serde_json::Value> = items.iter().map(|p| serde_json::json!({
            "id": p.id, "slug": p.slug, "name": p.name, "status": p.status,
            "root_path": p.root_path, "site_url": p.site_url,
            "last_opened_at": p.last_opened_at, "tags": p.tags,
        })).collect();
        Ok(vec![Content::text(format!(
            "{} project(s)\n\n{}",
            items.len(),
            serde_json::to_string_pretty(&json).unwrap_or_default()
        ))])
    }

    fn get_tools() -> Vec<Tool> {
        let create_schema = serde_json::to_value(schema_for!(ProjectCreateParams)).unwrap();
        let update_schema = serde_json::to_value(schema_for!(ProjectUpdateParams)).unwrap();
        let delete_schema = serde_json::to_value(schema_for!(ProjectDeleteParams)).unwrap();
        let list_schema = serde_json::to_value(schema_for!(ProjectListParams)).unwrap();

        vec![
            Tool::new("project_create".to_string(), indoc! {r#"
                Create a new project workspace. Use when the user asks to "set up a project",
                "create a project", or similar. Walk the user through the required field (name)
                and optional fields (root_path, site_url, repo_url, description, tags)
                conversationally.
            "#}.to_string(), create_schema.as_object().unwrap().clone())
            .annotate(ToolAnnotations::from_raw(Some("Create Project".to_string()), Some(false), Some(true), Some(false), Some(false))),

            Tool::new("project_update".to_string(), indoc! {r#"
                Update an existing project. Accepts the project ID or slug and any fields
                to change. Use when the user says "update project X", "change the root path
                for Y", etc.
            "#}.to_string(), update_schema.as_object().unwrap().clone())
            .annotate(ToolAnnotations::from_raw(Some("Update Project".to_string()), Some(false), Some(true), Some(false), Some(false))),

            Tool::new("project_delete".to_string(), indoc! {r#"
                Delete a project. Accepts the project ID or slug. The implicit "Personal"
                project cannot be deleted. Confirm with the user before deleting.
            "#}.to_string(), delete_schema.as_object().unwrap().clone())
            .annotate(ToolAnnotations::from_raw(Some("Delete Project".to_string()), Some(true), Some(true), Some(false), Some(false))),

            Tool::new("project_list".to_string(), indoc! {r#"
                List all projects. Optionally filter by status (active, paused, archived).
                Use when the user asks "what projects do I have?", "show my projects", etc.
            "#}.to_string(), list_schema.as_object().unwrap().clone())
            .annotate(ToolAnnotations::from_raw(Some("List Projects".to_string()), Some(false), Some(false), Some(false), Some(false))),
        ]
    }
}

#[async_trait]
impl McpClientTrait for ProjectManagerClient {
    async fn list_tools(&self, _session_id: &str, _next_cursor: Option<String>, _cancellation_token: CancellationToken) -> Result<ListToolsResult, Error> {
        Ok(ListToolsResult { tools: Self::get_tools(), next_cursor: None, meta: None })
    }

    async fn call_tool(&self, _ctx: &ToolCallContext, name: &str, arguments: Option<JsonObject>, _cancellation_token: CancellationToken) -> Result<CallToolResult, Error> {
        let content = match name {
            "project_create" => self.handle_create(arguments).await,
            "project_update" => self.handle_update(arguments).await,
            "project_delete" => self.handle_delete(arguments).await,
            "project_list" => self.handle_list(arguments).await,
            _ => Err(format!("Unknown tool: {}", name)),
        };
        match content {
            Ok(content) => Ok(CallToolResult::success(content)),
            Err(error) => Ok(CallToolResult::error(vec![Content::text(format!("Error: {}", error))])),
        }
    }

    fn get_info(&self) -> Option<&InitializeResult> { Some(&self.info) }
}
