//! File-to-project platform extension — the explicit, per-item consent path for
//! persisting content the user is looking at (call-notes/email MVP 2A).
//!
//! One tool, `file_to_project`: the agent proposes filing content (an email open
//! in the embedded browser, pasted text) onto a project. NOTHING persists at
//! call time — the tool files a `file_to_project` decision in the Decision
//! Inbox, and only the user's approval executes the effect
//! (`goose-server/src/routes/decisions.rs`): a project note through the ONE
//! composed note path ([`crate::project_notes::create_note_indexed`] — durable
//! row, Brain-indexed under the `permagent.note` source, Librarian-enriched)
//! plus the named people added to the project ADDRESS-LESS.
//!
//! Ratified rulings this tool embodies:
//! - Email content is persisted ONLY via this explicit per-email flow. This is
//!   the deliberate, user-approved override of the "browser reads are never
//!   persisted" guarantee — the proposal names its content origin so the user
//!   knows exactly what they are consenting to persist.
//! - People land ADDRESS-LESS: the payload cannot carry email/phone (schema
//!   rejects unknown fields), matching the enrichment hard-forbid — no
//!   exception.
//!
//! Project resolution reuses the People extension's discipline
//! ([`super::people::PeopleClient::resolve_project`]): explicit, never silent —
//! an ambiguous name returns the candidates rather than guessing.

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::platform_extensions::people::PeopleClient;
use crate::agents::tool_execution::ToolCallContext;
use async_trait::async_trait;
use indoc::indoc;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool, ToolAnnotations,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "file_to_project";

/// Longest content preview rendered into the decision's `detail` — the full
/// body travels in the payload; the detail is for the human skimming the inbox.
const DETAIL_PREVIEW_CHARS: usize = 1200;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct FileToProjectParams {
    /// The project to file into: name, slug, or id (resolved like the people
    /// tools; an ambiguous name returns the candidates so you can ask).
    project: String,
    /// The full text to file as the note body — e.g. the email's text as read
    /// from the browser, or the text the user pasted.
    content: String,
    /// Optional note title, e.g. "Email from Dana — pricing question".
    title: Option<String>,
    /// Where the content came from, in plain words the user will recognize
    /// (e.g. "email open in the embedded browser", "text pasted in chat").
    /// Shown verbatim in the proposal — be honest and specific.
    content_origin: String,
    /// Display names of people mentioned who should be added to the project.
    /// They are created/associated ADDRESS-LESS — never pass or research email
    /// addresses or phone numbers.
    people: Option<Vec<String>>,
}

pub struct FileToProjectClient {
    info: InitializeResult,
    context: PlatformExtensionContext,
}

impl FileToProjectClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self, anyhow::Error> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME.to_string(), "1.0.0".to_string())
                    .with_title("File to Project"),
            )
            .with_instructions(
                indoc! {r#"
                File content the user is looking at onto a project — as a
                review-gated proposal, never a direct write.

                Use `file_to_project` when the user says "file this email
                against <project>", "save this to <project>", or asks you to
                capture content they are viewing or pasted. Pass the FULL text
                as content, say honestly where it came from in content_origin,
                and optionally name people to add to the project.

                Nothing persists when you call the tool: the proposal waits in
                the Decision Inbox, and only the user's approval creates the
                project note (indexed into their Brain) and adds the people.
                Browser/email content is otherwise NEVER persisted — this tool
                is the one explicit, per-item consent path. People are added
                ADDRESS-LESS: never pass, research, or propose email addresses
                or phone numbers.
            "#}
                .to_string(),
            );
        Ok(Self { info, context })
    }

    async fn pool(&self) -> Result<Pool<Sqlite>, String> {
        self.context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())
    }

    async fn handle_file_to_project(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let params: FileToProjectParams = serde_json::from_value(serde_json::Value::Object(args))
            .map_err(|e| format!("Invalid arguments: {e}"))?;

        let content = params.content.trim();
        if content.is_empty() {
            return Err("content is empty — nothing to file".to_string());
        }
        let origin = params.content_origin.trim();
        if origin.is_empty() {
            return Err(
                "content_origin is required — say where this content came from (e.g. \
                 \"email open in the embedded browser\")"
                    .to_string(),
            );
        }
        let title = params
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty());
        let people: Vec<String> = params
            .people
            .unwrap_or_default()
            .iter()
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .collect();

        let pool = self.pool().await?;
        let project = PeopleClient::resolve_project(&pool, &params.project).await?;

        let payload = crate::decisions::FileToProjectPayload {
            project_id: project.id.clone(),
            project_name: project.name.clone(),
            title: title.map(str::to_string),
            body: content.to_string(),
            content_origin: origin.to_string(),
            people: people.clone(),
        };

        let label = title.unwrap_or("a note");
        let mut headline = format!("File \"{}\" to project \"{}\"", label, project.name);
        if headline.chars().count() > 80 {
            headline = headline.chars().take(79).collect::<String>() + "…";
        }

        let preview: String = if content.chars().count() > DETAIL_PREVIEW_CHARS {
            let clipped: String = content.chars().take(DETAIL_PREVIEW_CHARS).collect();
            format!(
                "{}\n[truncated — {} more chars; the full text is filed on approval]",
                clipped,
                content.chars().count() - DETAIL_PREVIEW_CHARS
            )
        } else {
            content.to_string()
        };
        let mut detail = format!("Source: {origin}\n");
        if !people.is_empty() {
            detail.push_str(&format!(
                "People to add to \"{}\" (address-less — name only): {}\n",
                project.name,
                people.join(", ")
            ));
        }
        detail.push_str(&format!("\nContent:\n{preview}"));

        let decision = crate::decisions::create_decision(
            &pool,
            crate::decisions::NewDecision {
                kind: "file_to_project".to_string(),
                project_id: Some(project.id.clone()),
                headline: Some(headline),
                detail: Some(detail),
                payload: serde_json::to_value(&payload).map_err(|e| e.to_string())?,
                ..Default::default()
            },
        )
        .await?;

        if decision.kind == "malformed" {
            return Err(format!(
                "The proposal was rejected as malformed: {}",
                decision.detail
            ));
        }

        let people_note = if people.is_empty() {
            String::new()
        } else {
            format!(
                " On approval, {} people ({}) will be added to the project address-less \
                 (display name only).",
                people.len(),
                people.join(", ")
            )
        };
        Ok(vec![Content::text(format!(
            "Proposed filing this content to project \"{}\" — decision {} is waiting in the \
             Decision Inbox. NOTHING is persisted until the user approves it there; on approval \
             it becomes a project note indexed into the Brain.{}",
            project.name, decision.id, people_note
        ))])
    }

    pub(crate) fn get_tools() -> Vec<Tool> {
        vec![Tool::new(
            "file_to_project".to_string(),
            indoc! {r#"
                File content the user is looking at (an email open in the
                embedded browser, or text they pasted) onto a project — as a
                review-gated Decision Inbox proposal, never a direct write. On
                approval it becomes a project note (indexed into the Brain,
                Librarian-enriched) and any named people are added to the
                project ADDRESS-LESS (display name only — never pass email
                addresses or phone numbers). Nothing persists until the user
                approves. Use when the user says "file this email against
                <project>" or "save this to <project>".
            "#}
            .to_string(),
            serde_json::to_value(schema_for!(FileToProjectParams))
                .unwrap()
                .as_object()
                .unwrap()
                .clone(),
        )
        .annotate(ToolAnnotations::from_raw(
            Some("File to Project".to_string()),
            Some(false),
            Some(false),
            Some(false),
            Some(false),
        ))]
    }
}

#[async_trait]
impl McpClientTrait for FileToProjectClient {
    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
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
    ) -> Result<CallToolResult, Error> {
        let content = match name {
            "file_to_project" => self.handle_file_to_project(arguments).await,
            _ => Err(format!("Unknown tool: {name}")),
        };
        match content {
            Ok(content) => Ok(CallToolResult::success(content)),
            Err(error) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Error: {error}"
            ))])),
        }
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }
}
