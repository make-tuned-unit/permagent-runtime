use super::supervised_cli;
use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use crate::{cards, projects};
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

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct CardCreateParams {
    /// Project ID (UUID) or slug
    project_id_or_slug: String,
    /// Card title (required)
    title: String,
    /// Card description (optional)
    description: Option<String>,
    /// Card type: standard, goal, or social_post (optional, defaults to standard)
    card_type: Option<String>,
    /// Column name or ID to place the card in (optional, defaults to first column)
    column: Option<String>,
    /// For goal cards only: if true, immediately dispatch to a worker after creation.
    /// The card will be moved from Triage → Ready → InProgress automatically.
    /// Defaults to false.
    #[serde(default)]
    auto_dispatch: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct CardMoveParams {
    /// Card ID (UUID)
    card_id: String,
    /// Target column name or ID
    column: String,
    /// Position within the column (optional, defaults to end)
    position: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct CardDeleteParams {
    /// Card ID (UUID) to delete
    card_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct CardListParams {
    /// Project ID (UUID) or slug
    project_id_or_slug: String,
    /// Filter by card type: standard, goal, or social_post (optional)
    card_type: Option<String>,
    /// Filter by column name or ID (optional)
    column: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct BoardSummaryParams {
    /// Project ID (UUID) or slug. If omitted, returns summary for all active projects.
    project_id_or_slug: Option<String>,
    /// Include standard cards in addition to goals. Defaults to false (goals only).
    #[serde(default)]
    include_standard_cards: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ProjectResolveParams {
    /// The spoken or approximate project name to resolve (e.g. "Kinros", "personal").
    /// Performs fuzzy matching against all project names and slugs.
    query: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ResearchProjectIntelParams {
    /// Project ID, slug, or exact name.
    project: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct DismissProjectIntelParams {
    /// Project ID, slug, or exact name.
    project: String,
    /// The name of the intelligence item to remove (case-insensitive), as shown
    /// by research_project_intel.
    name: String,
    /// Optionally restrict removal to one kind: competitor, partner, or adjacent.
    #[serde(default)]
    kind: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ProposedIntelItemParams {
    /// One of: competitor, partner, adjacent.
    kind: String,
    /// Organization or product name.
    name: String,
    /// Concise explanation of its relationship to the project.
    note: Option<String>,
    /// Page where this finding was verified.
    source_url: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ProposeProjectIntelParams {
    /// Project ID, slug, or exact name.
    project: String,
    /// Cited findings to send to the Decision Inbox.
    items: Vec<ProposedIntelItemParams>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ColumnCreateParams {
    /// Project ID (UUID) or slug
    project_id_or_slug: String,
    /// Column name
    name: String,
    /// Position in column order (optional, defaults to end)
    position: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ColumnDeleteParams {
    /// Column ID (UUID) to delete. Cannot delete columns that contain cards.
    column_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ProjectLaunchParams {
    /// Project ID (UUID) or slug to launch a terminal for. The project must
    /// have a root_path set.
    id_or_slug: String,
    /// Optional command to run in the terminal once it opens (e.g. "claude" to
    /// start Claude Code, "npm run dev", etc). If omitted, an interactive shell
    /// is opened at the project root with no command. Mutually exclusive with
    /// `supervised` — a supervised session composes its own command.
    #[serde(default)]
    command: Option<String>,
    /// Launch a SUPERVISED Claude Code session (#427) instead of a plain
    /// command: Claude Code runs in structured stream-json mode with
    /// permission gates ENABLED, visible in the terminal tab, so its gates can
    /// be watched (and, in later slices, escalated and answered). Use this
    /// when the user wants you to run and watch a Claude Code session rather
    /// than open a plain terminal.
    #[serde(default)]
    supervised: Option<bool>,
    /// Initial instruction for the supervised session — compose a clear,
    /// self-contained goal prompt from what the user asked. Only used with
    /// `supervised: true`. If omitted, the session opens idle, waiting for
    /// input.
    #[serde(default)]
    prompt: Option<String>,
}

/// Self-knowledge descriptor for the Build tab — the project-aware terminal +
/// browser workspace. Co-located with the `project_launch` tool that drives it;
/// aggregated by `crate::agents::self_knowledge`. Static — always-on surface.
pub const BUILD_TAB_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "build",
        display_name: "Build tab",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "A workspace with project-aware terminals and an in-app browser. You can open a \
             terminal rooted at any project's directory — and run a command in it (e.g. start \
             Claude Code) — by calling the project_launch tool",
        why_it_matters:
            "It is your native way to run commands and drive coding work inside a project. Reach \
             for project_launch (not a one-shot shell) when the user wants to launch a project, \
             open a terminal, or run an interactive tool like Claude Code in a project",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[],
    };

/// Self-knowledge descriptor for the **Permagent coding harness** (#719/#720) —
/// the third Build-tab launch option, beside the Claude and Codex CLIs, that
/// runs Permagent's OWN internal agent loop (`permagent run --recipe
/// permagent-coding`) configured for software engineering. Co-located with
/// [`BUILD_TAB_FEATURE`] because the Build tab is where the user launches it;
/// aggregated by `crate::agents::self_knowledge::SURFACE_DESCRIPTORS`. Static —
/// the capability is described without claiming a live session status. The
/// bundled sub-capabilities (reliable edit tool #711, structured search #718,
/// ranked-tags repo-map #712, and tiered routing + cost meter — see
/// [`crate::cost_router`]) are indexed inside this one descriptor rather than as
/// standalone surfaces: they are the harness's internals, not independent views
/// the user opens.
pub const CODING_HARNESS_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "coding_harness",
        display_name: "Permagent coding harness",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "Permagent's own internal coding agent, launched from the Build tab as the third \
             option beside the Claude and Codex CLIs — it runs your own agent loop (permagent run \
             --recipe permagent-coding) configured for software engineering, not an external \
             tool. It is provider-agnostic (it uses whichever model the operator configured) and \
             cost-optimized: it bundles a reliable edit tool that tolerates whitespace drift and \
             refuses any edit that would introduce a syntax error, a token-efficient structured \
             search, the analyze code-structure tool, and a ranked-tags repo-map auto-loaded into \
             its context for cheap orientation, and it verifies its own work by building and \
             running tests; on a substantive change, once its own tests pass an independent, \
             different-model reviewer adversarially checks the diff — for correctness, security, \
             spec-fit, and test-integrity — before it calls the work done, all under \
             runaway-loop safety and a live cost meter always on",
        why_it_matters:
            "It is the answer when the user says 'build this with the Permagent harness': you \
             launch it from the Build tab and it codes with your own loop, keeping the expensive \
             main reasoning on one stable model while offloading mechanical, latency-tolerant \
             sub-work to cheaper tiers — down to a free local model — and escalating a sub-task \
             only when the cheap tier stumbles, so it is economical without you managing any of \
             it. Reach for it, not a one-shot shell, whenever the user wants Permagent itself to \
             write, edit, or fix code in a project: open the Build tab, launch the Permagent \
             option, and let it work in the terminal where the user can watch and take over",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[
            crate::agents::self_knowledge::TeachingStep {
                title: "Launch the harness from the Build tab",
                body: "Take the user to the Build tab and point out the third launch option on a \
                       project's chip — 'Permagent', beside Claude and Codex. Explain that unlike \
                       those two (which drive external CLIs), this one runs Permagent's own \
                       coding loop locally, provider-agnostic and cost-optimized, and opens in \
                       the same project-aware terminal so they can watch and take over.",
                open_surface: Some(crate::agents::self_knowledge::SurfaceRef {
                    tab: "Build",
                    section: None,
                }),
                confirm: None,
            },
            crate::agents::self_knowledge::TeachingStep {
                title: "Hand it a task, let it verify itself",
                body: "Offer to build something small end-to-end with it — 'write a simple \
                       game', fix a failing test, add a function. Explain what it will do on its \
                       own: read a repo-map for orientation, search and analyze before editing, \
                       make the smallest change, then build and run tests to verify — routing the \
                       cheap mechanical steps to cheaper models while the hard reasoning stays on \
                       a strong one, all under runaway-loop safety and a live cost meter.",
                open_surface: None,
                confirm: None,
            },
        ],
    };

/// Self-knowledge descriptor for the Grow tab — the per-project go-to-market
/// workspace. Static surface; teaching steps drive onboarding. Closes the
/// coverage gap where Henry could not describe or guide the user to Grow.
pub const GROW_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "grow",
        display_name: "Grow tab",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does: "A per-project go-to-market workspace: strategy pillars (audience, value \
             proposition, positioning, channels, content) you think through with the user, a \
             content calendar of drafted posts, and a growth view with a live analytics lens — a \
             provider-pluggable stats client (Plausible or GoatCounter) that pulls the project's \
             real visitor and traffic numbers into the view — with any post or outreach you draft \
             written in a crisp human voice, never chatbot boilerplate",
        why_it_matters:
            "It is where the user takes a project to market with you. When they want to reach an \
             audience, plan a launch, or draft a post, bring them here and draft it in their \
             voice — marketing copy the user publishes must not read like AI wrote it",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[
            crate::agents::self_knowledge::TeachingStep {
                title: "Open Grow",
                body: "Show the user the go-to-market workspace for a project — where strategy, \
                       content, and launch planning live.",
                open_surface: Some(crate::agents::self_knowledge::SurfaceRef {
                    tab: "Grow",
                    section: None,
                }),
                confirm: None,
            },
            crate::agents::self_knowledge::TeachingStep {
                title: "Draft something real",
                body: "Offer to draft a launch post or outreach message for their project, in a \
                       sharp human voice, then show them it landed in the content calendar.",
                open_surface: None,
                confirm: None,
            },
        ],
    };

/// Self-knowledge descriptor for the Projects tab itself (#471). Each project
/// opens into a workspace with two lenses: an Overview dashboard (summary, key
/// facts, links, live task status) and the Kanban board. Static — always-on
/// surface, co-located with the project tools that back it.
/// Self-knowledge for the Devices pairing surface + the agent-driven tailnet
/// runbook (MULTI_DEVICE.md, Jesse's zero-strain rule 2026-07-11): Henry sets
/// the tailnet up himself with terminal commands; the user's only step is the
/// Tailscale login click, which Henry opens for them.
pub const DEVICES_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "devices",
        display_name: "Devices",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "Settings → Devices pairs the user's other devices to this machine (the hub): it \
             shows a pairing URL carrying the daemon token, auto-fills the hub's Tailscale \
             MagicDNS name when a tailnet is detected, and any browser on the tailnet that \
             opens the URL becomes a full Permagent client. Paired devices are listed by name \
             with a last-seen time, and each one is revocable — the user can name a device, see \
             when it last connected, and revoke its access from this surface. One Brain on the \
             hub — other devices connect to it, nothing syncs",
        why_it_matters:
            "When the user wants Permagent on their phone or laptop, set the tailnet up FOR \
             them with your terminal: (1) `tailscale status --json` — if it errors, install \
             with `brew install --cask tailscale` and launch the app; (2) run \
             `tailscale up` and when it prints a login URL, open_website it so they just \
             click approve; (3) confirm with `tailscale status --json` (BackendState \
             Running), read Self.DNSName, and tell them it now appears in Settings → \
             Devices; (4) remind them the daemon needs HOST=0.0.0.0 to accept tailnet \
             connections. Their only task is one login click — you do the rest",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[crate::agents::self_knowledge::TeachingStep {
            title: "Put Permagent in their pocket",
            body: "Offer to set up multi-device: run the tailnet steps yourself (install, \
                   up, open the login page for them), then walk them to Settings → \
                   Devices, have them open the pairing URL on their phone, and greet them \
                   there.",
            open_surface: Some(crate::agents::self_knowledge::SurfaceRef {
                tab: "Settings",
                section: Some("devices"),
            }),
            confirm: None,
        }],
    };

pub const PROJECT_WORKSPACE_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "projects",
        display_name: "Projects workspace",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "The Projects tab where each project opens into a workspace with two lenses, toggled \
             in shared chrome: an Overview dashboard (summary, key facts, links, live task \
             status, the People panel with person profile cards, the Documents hub with an \
             in-app viewer for PDFs/images/markdown, a Notes panel, a Stack panel that lists \
             each service the project is built on and which login identity (an email or account \
             label) the user signs in with per service — a reference card only, it never stores \
             passwords or secrets — a Memories panel that \
             lists what your Brain has learned about the project — each with a 'View in Brain' \
             deep-link that focuses that memory in the Brain view — and an Intelligence \
             (Ecosystem) panel where you research and curate the project's ecosystem \
             (partners, adjacent players) and competitive landscape (competitors): findings \
             are review-gated through the Decision Inbox, each cites its source, and a \
             'Refresh intelligence' action re-runs the research) and the Kanban board of \
             goal and to-do cards. A document dropped into a project is extracted and indexed \
             into your Brain and associated with that project; notes the user writes on a \
             project are indexed into your Brain the same way — both recallable and \
             Librarian-enriched, scoped to the project, and both surface back in the Memories \
             panel. A project switcher drives both lenses from the same selected project",
        why_it_matters:
            "It is the user's at-a-glance home for a project — what it is, its links, and the live \
             state of its work. Because dropped documents and written notes land in your Brain \
             scoped to the project, you can recall a project's files and notes by content without \
             the user re-pasting them. Reach for the project tools (project_list, board_summary) \
             to read or change what this surface shows; the Overview is the summary view, the \
             Kanban the working board",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[],
    };

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
            .with_instructions(
                indoc! {r#"
                Manage user projects and their Kanban boards. Projects organize work into
                named workspaces with filesystem paths, URLs, and metadata. Each project
                has a slug (stable identifier), name (display label), and optional
                root_path, site_url, and repo_url.

                The implicit "Personal" project always exists and cannot be deleted.

                ## Cards

                Projects contain cards organized into columns (Kanban-style). Three card
                types exist:
                  - 'standard': manual task or note, user-managed
                  - 'goal': agentic goal routed to worker agents (future)
                  - 'social_post': scheduled social media post (future)

                When the user says "add a card", "create a task", "track this", or
                similar, use card_create with card_type='standard'. Ask which project
                if not clear from context — default to the active project if the user
                is currently in one, otherwise Personal.

                Use `research_project_intel` when the user asks to research or
                refresh a project's ecosystem or competitive landscape. It returns
                a research briefing; research with your own web tools, then file
                cited findings with `propose_project_intel`. Findings wait in the
                Decision Inbox — nothing is stored until the user approves.
            "#}
                .to_string(),
            );

        Ok(Self { info, context })
    }

    async fn handle_create(&self, arguments: Option<JsonObject>) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: name")?
            .to_string();
        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())?;
        let input = projects::CreateProject {
            name,
            slug: None,
            description: args
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from),
            root_path: args
                .get("root_path")
                .and_then(|v| v.as_str())
                .map(String::from),
            site_url: args
                .get("site_url")
                .and_then(|v| v.as_str())
                .map(String::from),
            repo_url: args
                .get("repo_url")
                .and_then(|v| v.as_str())
                .map(String::from),
            notes: None,
            tags: args.get("tags").and_then(|v| v.as_array()).map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
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
            project.name,
            project.slug,
            project.id,
            serde_json::to_string_pretty(&json).unwrap_or_default()
        ))])
    }

    async fn handle_update(&self, arguments: Option<JsonObject>) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let id_or_slug = args
            .get("id_or_slug")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: id_or_slug")?;
        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())?;
        let project = projects::get_project_by_id_or_slug(&pool, id_or_slug)
            .await?
            .ok_or_else(|| format!("Project '{}' not found", id_or_slug))?;
        let input = projects::UpdateProject {
            name: args.get("name").and_then(|v| v.as_str()).map(String::from),
            slug: args.get("slug").and_then(|v| v.as_str()).map(String::from),
            description: args
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from),
            status: args
                .get("status")
                .and_then(|v| v.as_str())
                .map(String::from),
            root_path: args.get("root_path").map(|v| v.as_str().map(String::from)),
            site_url: args.get("site_url").map(|v| v.as_str().map(String::from)),
            repo_url: args.get("repo_url").map(|v| v.as_str().map(String::from)),
            notes: args.get("notes").and_then(|v| v.as_str()).map(String::from),
            metadata_json: None,
        };
        let updated = projects::update_project(&pool, &project.id, input)
            .await?
            .ok_or("Project not found after update")?;
        let json = serde_json::json!({
            "id": updated.id, "slug": updated.slug, "name": updated.name,
            "status": updated.status, "root_path": updated.root_path,
            "site_url": updated.site_url, "repo_url": updated.repo_url,
        });
        Ok(vec![Content::text(format!(
            "Updated project \"{}\" (slug: {})\n\n{}",
            updated.name,
            updated.slug,
            serde_json::to_string_pretty(&json).unwrap_or_default()
        ))])
    }

    async fn handle_delete(&self, arguments: Option<JsonObject>) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let id_or_slug = args
            .get("id_or_slug")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: id_or_slug")?;
        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())?;
        let project = projects::get_project_by_id_or_slug(&pool, id_or_slug)
            .await?
            .ok_or_else(|| format!("Project '{}' not found", id_or_slug))?;
        projects::delete_project(&pool, &project.id).await?;
        Ok(vec![Content::text(format!(
            "Deleted project \"{}\" (slug: {}, id: {})",
            project.name, project.slug, project.id
        ))])
    }

    async fn handle_list(&self, arguments: Option<JsonObject>) -> Result<Vec<Content>, String> {
        let status = arguments
            .as_ref()
            .and_then(|a| a.get("status"))
            .and_then(|v| v.as_str())
            .map(String::from);
        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())?;
        let items = projects::list_projects(&pool, status.as_deref()).await?;
        let json: Vec<serde_json::Value> = items
            .iter()
            .map(|p| {
                serde_json::json!({
                    "id": p.id, "slug": p.slug, "name": p.name, "status": p.status,
                    "root_path": p.root_path, "site_url": p.site_url,
                    "last_opened_at": p.last_opened_at, "tags": p.tags,
                })
            })
            .collect();
        Ok(vec![Content::text(format!(
            "{} project(s)\n\n{}",
            items.len(),
            serde_json::to_string_pretty(&json).unwrap_or_default()
        ))])
    }

    async fn resolve_project(
        &self,
        id_or_slug: &str,
    ) -> Result<(projects::Project, sqlx::Pool<sqlx::Sqlite>), String> {
        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())?;
        let project = projects::get_project_by_id_or_slug(&pool, id_or_slug)
            .await?
            .ok_or_else(|| format!("Project '{}' not found", id_or_slug))?;
        Ok((project, pool))
    }

    async fn resolve_intel_project(
        &self,
        query: &str,
    ) -> Result<(projects::Project, sqlx::Pool<sqlx::Sqlite>), String> {
        if let Ok(found) = self.resolve_project(query).await {
            return Ok(found);
        }
        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())?;
        let all = projects::list_projects(&pool, None).await?;
        let matches: Vec<_> = all
            .into_iter()
            .filter(|p| p.name.eq_ignore_ascii_case(query))
            .collect();
        match matches.as_slice() {
            [project] => Ok((project.clone(), pool)),
            [] => Err(format!("Project '{}' not found", query)),
            _ => Err(format!("Project name '{}' is ambiguous", query)),
        }
    }

    async fn handle_research_project_intel(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let params: ResearchProjectIntelParams =
            serde_json::from_value(serde_json::Value::Object(args))
                .map_err(|e| format!("Invalid arguments: {e}"))?;
        let (project, pool) = self.resolve_intel_project(&params.project).await?;
        let current = sqlx::query_as::<_, (String, String, Option<String>, String)>(
            "SELECT kind, name, note, source_url FROM project_intel
             WHERE project_id = ? ORDER BY kind, name",
        )
        .bind(&project.id)
        .fetch_all(&pool)
        .await
        .map_err(|e| e.to_string())?;
        let current = if current.is_empty() {
            "  (none stored yet)".to_string()
        } else {
            current
                .iter()
                .map(|(kind, name, note, source)| {
                    format!(
                        "  - {}: {}{} [source: {}]",
                        kind,
                        name,
                        note.as_deref()
                            .filter(|v| !v.trim().is_empty())
                            .map(|v| format!(" — {v}"))
                            .unwrap_or_default(),
                        source
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        Ok(vec![Content::text(format!(
            "Project intelligence briefing for \"{}\" (project_id: {}).\n\
             \nCurrent stored intelligence:\n{}\n\
             \nResearch ONLY these kinds: competitor, partner, adjacent.\n\
             \nHow to work:\n\
             1. Use your own web tools to research the project's competitive landscape, \
             partners, and adjacent ecosystem players.\n\
             2. Verify every finding on the page you cite; every item needs source_url.\n\
             3. Prefer primary sources and skip duplicates already stored above.\n\
             4. Keep each note concise and explain why the item matters to this project.\n\
             \nWhen done, call propose_project_intel with project \"{}\" and items: \
             [{{kind, name, note, source_url}}]. Nothing is stored until the user approves \
             the proposal in the Decision Inbox.",
            project.name, project.id, current, project.name
        ))])
    }

    async fn handle_propose_project_intel(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let params: ProposeProjectIntelParams =
            serde_json::from_value(serde_json::Value::Object(args))
                .map_err(|e| format!("Invalid arguments: {e}"))?;
        let (project, pool) = self.resolve_intel_project(&params.project).await?;
        if params.items.is_empty() {
            return Err("No intelligence items proposed — nothing to review.".to_string());
        }
        for item in &params.items {
            if !matches!(item.kind.as_str(), "competitor" | "partner" | "adjacent") {
                return Err(format!(
                    "Kind \"{}\" is invalid. Allowed kinds: competitor, partner, adjacent.",
                    item.kind
                ));
            }
            if item.name.trim().is_empty() {
                return Err("An intelligence item has an empty name.".to_string());
            }
            if item.source_url.trim().is_empty() {
                return Err(format!("Item \"{}\" is missing its source_url.", item.name));
            }
        }
        let payload = crate::decisions::ProjectIntelProposalPayload {
            project_id: project.id.clone(),
            project_name: project.name.clone(),
            items: params
                .items
                .iter()
                .map(|item| crate::decisions::ProposedIntelItem {
                    kind: item.kind.clone(),
                    name: item.name.trim().to_string(),
                    note: item
                        .note
                        .as_deref()
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                        .map(str::to_string),
                    source_url: item.source_url.trim().to_string(),
                })
                .collect(),
        };
        let mut headline = format!("Approve project intelligence for {}", project.name);
        if headline.chars().count() > 80 {
            headline = headline.chars().take(79).collect::<String>() + "…";
        }
        let detail = params
            .items
            .iter()
            .map(|item| {
                format!(
                    "{}: {} (source: {})",
                    item.kind,
                    item.name.trim(),
                    item.source_url.trim()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let decision = crate::decisions::create_decision(
            &pool,
            crate::decisions::NewDecision {
                kind: "project_intel_proposal".to_string(),
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
        Ok(vec![Content::text(format!(
            "Proposed {} intelligence item(s) for \"{}\" — decision {} is waiting in the \
             Decision Inbox. Nothing is stored until the user approves it there.",
            params.items.len(),
            project.name,
            decision.id
        ))])
    }

    /// Remove stored project-intelligence items by name (the inverse of
    /// propose_project_intel). Direct delete — removal is user-directed and
    /// reversible by re-researching, so it is not review-gated. Matches the name
    /// case-insensitively; an optional `kind` narrows the match.
    async fn handle_dismiss_project_intel(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let params: DismissProjectIntelParams =
            serde_json::from_value(serde_json::Value::Object(args))
                .map_err(|e| format!("Invalid arguments: {e}"))?;
        let name = params.name.trim();
        if name.is_empty() {
            return Err("name must not be empty".to_string());
        }
        let (project, pool) = self.resolve_intel_project(&params.project).await?;

        let name_folded = name.to_lowercase();
        let rows = sqlx::query_as::<_, (String, String, String)>(
            "SELECT id, kind, name FROM project_intel WHERE project_id = ?",
        )
        .bind(&project.id)
        .fetch_all(&pool)
        .await
        .map_err(|e| e.to_string())?;

        let mut removed = Vec::new();
        for (id, k, n) in rows {
            let kind_ok = params
                .kind
                .as_deref()
                .map(|kf| kf.trim().eq_ignore_ascii_case(&k))
                .unwrap_or(true);
            // Full Unicode case-fold on the name (mirrors the intel dedup).
            if kind_ok && n.to_lowercase() == name_folded {
                sqlx::query("DELETE FROM project_intel WHERE id = ?")
                    .bind(&id)
                    .execute(&pool)
                    .await
                    .map_err(|e| e.to_string())?;
                removed.push(format!("{k}: {n}"));
            }
        }

        if removed.is_empty() {
            Ok(vec![Content::text(format!(
                "No project intelligence matching \"{name}\" found for \"{}\" — nothing removed.",
                project.name
            ))])
        } else {
            Ok(vec![Content::text(format!(
                "Removed {} intelligence item(s) from \"{}\": {}.",
                removed.len(),
                project.name,
                removed.join(", ")
            ))])
        }
    }

    async fn handle_launch(&self, arguments: Option<JsonObject>) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let id_or_slug = args
            .get("id_or_slug")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: id_or_slug")?;
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty());
        let supervised = args
            .get("supervised")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty());

        if supervised && command.is_some() {
            return Err(
                "`command` and `supervised` are mutually exclusive — a supervised session \
                 composes its own Claude Code invocation. Drop `command` (put the instruction \
                 in `prompt` instead)."
                    .to_string(),
            );
        }

        let (project, _pool) = self.resolve_project(id_or_slug).await?;

        let root_path = project
            .root_path
            .clone()
            .filter(|p| !p.trim().is_empty())
            .ok_or_else(|| {
                format!(
                    "Project \"{}\" has no root_path set, so there is nowhere to open a terminal. \
                 Set one with project_update first.",
                    project.name
                )
            })?;

        // S1 (#427): free-standing SUPERVISED session — the same visible-tab
        // launch path, but running Claude Code in gate-enabled stream-json
        // mode via the supervised launcher. No goal card, no worktree.
        if supervised {
            let label = format!(
                "{} · {} (supervised)",
                project.slug,
                supervised_cli::SUPERVISED_CLI_DEFAULT_BIN
            );
            let reason = format!(
                "Opening a supervised Claude Code session in {}",
                project.name
            );
            let launch = supervised_cli::launch_watched_session(
                &root_path,
                &label,
                &project.slug,
                prompt,
                &reason,
            )
            .await?;
            return Ok(vec![Content::text(format!(
                "Launched a supervised Claude Code session for \"{}\" at {} (session {}). It is \
                 running in a visible Build-tab terminal in stream-json mode with permission \
                 gates enabled{}.",
                project.name,
                root_path,
                launch.session_id,
                if prompt.is_some() {
                    " and has been handed the initial prompt"
                } else {
                    "; it is idle until it receives input"
                }
            ))]);
        }

        let label = match command {
            Some(cmd) => format!("{} · {}", project.slug, cmd),
            None => project.slug.clone(),
        };
        let reason = match command {
            Some(cmd) => format!(
                "Opening a terminal in {} and running `{}`",
                project.name, cmd
            ),
            None => format!("Opening a terminal in {}", project.name),
        };

        crate::events::emit(crate::events::project_launch(
            &root_path,
            &label,
            command,
            &project.slug,
            &reason,
            None,
        ));

        Ok(vec![Content::text(format!(
            "Launched a terminal for \"{}\" at {}{}.",
            project.name,
            root_path,
            command
                .map(|c| format!(" running `{}`", c))
                .unwrap_or_default()
        ))])
    }

    async fn resolve_column(
        pool: &sqlx::Pool<sqlx::Sqlite>,
        project_id: &str,
        col_ref: &str,
    ) -> Result<cards::BoardColumn, String> {
        // Try as ID first, then by name
        if let Some(col) = cards::get_column(pool, col_ref).await? {
            if col.project_id == project_id {
                return Ok(col);
            }
        }
        cards::get_column_by_name(pool, project_id, col_ref)
            .await?
            .ok_or_else(|| format!("Column '{}' not found in project", col_ref))
    }

    async fn handle_resolve(&self, arguments: Option<JsonObject>) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: query")?
            .to_string();
        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())?;
        let items = projects::list_projects(&pool, None).await?;

        let query_lower = query.to_lowercase();

        // Score each project: lower is better
        let mut scored: Vec<(usize, &projects::Project)> = items
            .iter()
            .filter_map(|p| {
                let name_lower = p.name.to_lowercase();
                let slug_lower = p.slug.to_lowercase();

                // Exact match on slug or name
                if slug_lower == query_lower || name_lower == query_lower {
                    return Some((0, p));
                }
                // Substring match
                if name_lower.contains(&query_lower) || slug_lower.contains(&query_lower) {
                    return Some((1, p));
                }
                if query_lower.contains(&name_lower) || query_lower.contains(&slug_lower) {
                    return Some((2, p));
                }
                // Edit distance (simple Levenshtein)
                let dist_name = levenshtein(&query_lower, &name_lower);
                let dist_slug = levenshtein(&query_lower, &slug_lower);
                let best = dist_name.min(dist_slug);
                let threshold = (query_lower.len().max(name_lower.len()) / 3).max(2);
                if best <= threshold {
                    Some((3 + best, p))
                } else {
                    None
                }
            })
            .collect();

        scored.sort_by_key(|(score, _)| *score);

        if scored.is_empty() {
            return Ok(vec![Content::text(format!(
                "No project matches \"{}\". Available projects:\n{}",
                query,
                items
                    .iter()
                    .map(|p| format!("  - {} (slug: {}, id: {})", p.name, p.slug, p.id))
                    .collect::<Vec<_>>()
                    .join("\n")
            ))]);
        }

        let best_score = scored[0].0;
        // If best match is exact or substring, return just that one
        let matches: Vec<_> = if best_score <= 2 {
            vec![scored[0].1]
        } else {
            // Return all within same score tier for disambiguation
            scored
                .iter()
                .filter(|(s, _)| *s == best_score)
                .map(|(_, p)| *p)
                .collect()
        };

        let json: Vec<serde_json::Value> = matches
            .iter()
            .map(|p| {
                serde_json::json!({
                    "id": p.id, "slug": p.slug, "name": p.name,
                    "status": p.status, "root_path": p.root_path,
                })
            })
            .collect();

        let confidence = if best_score == 0 {
            "exact"
        } else if best_score <= 2 {
            "high"
        } else {
            "fuzzy"
        };

        Ok(vec![Content::text(format!(
            "{} match(es) for \"{}\" (confidence: {})\n\n{}{}",
            matches.len(),
            query,
            confidence,
            serde_json::to_string_pretty(&json).unwrap_or_default(),
            if matches.len() > 1 {
                "\n\nMultiple matches found — confirm with the user which project they mean."
            } else {
                ""
            }
        ))])
    }

    async fn handle_card_create(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let id_or_slug = args
            .get("project_id_or_slug")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: project_id_or_slug")?;
        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: title")?
            .to_string();
        let (project, pool) = self.resolve_project(id_or_slug).await?;

        let column_id = if let Some(col_ref) = args.get("column").and_then(|v| v.as_str()) {
            Some(Self::resolve_column(&pool, &project.id, col_ref).await?.id)
        } else {
            None
        };

        let card_type_str = args
            .get("card_type")
            .and_then(|v| v.as_str())
            .unwrap_or("standard");
        let auto_dispatch = args
            .get("auto_dispatch")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let card = cards::create_card(
            &pool,
            cards::CreateCard {
                project_id: project.id.clone(),
                title,
                description: args
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                card_type: Some(card_type_str.to_string()),
                column_id,
                created_by: Some("user".to_string()),
                metadata_json: None,
            },
        )
        .await?;

        // Auto-dispatch: for goal cards, move Triage → Ready → InProgress via Orchestrator
        if card_type_str == "goal" && auto_dispatch {
            // Move to Ready through the goal-transition guard (tier-0 'ready')
            let ready_col = cards::get_goal_column(&pool, &project.id, "ready")
                .await?
                .ok_or("Ready column not found for auto-dispatch")?;
            crate::goal_transition::advance_goal_checked(
                &pool,
                &card.id,
                crate::goal_state::GoalAction::Ready,
                crate::decisions::ACTOR_SYSTEM,
                None,
                crate::goal_transition::TransitionEffects::default(),
            )
            .await
            .map_err(String::from)?;

            // Dispatch via the free-function pipeline (#213) — no throwaway
            // OrchestratorClient, whose `new()` would also spawn a resume +
            // worktree-sweep task on construction. A fresh ProbeCache is cheap
            // (an empty in-memory map); the temp client's own cache was discarded
            // anyway.
            let probe_cache = crate::config::worker_probe::ProbeCache::new();
            match super::orchestrator::dispatch_goal_fn(&self.context, &probe_cache, &card.id).await
            {
                Ok(session_id) => {
                    let updated = cards::get_card(&pool, &card.id)
                        .await?
                        .unwrap_or(card.clone());
                    let json = serde_json::json!({
                        "id": updated.id, "title": updated.title, "card_type": updated.card_type,
                        "column_id": updated.column_id, "assigned_to": updated.assigned_to,
                        "worker_session_id": session_id,
                        "project": project.name, "state": "in_progress",
                    });
                    return Ok(vec![Content::text(format!(
                        "Created goal \"{}\" in {} and dispatched to worker (session: {})\n\n{}",
                        updated.title,
                        project.name,
                        session_id,
                        serde_json::to_string_pretty(&json).unwrap_or_default()
                    ))]);
                }
                Err(e) => {
                    // Dispatch failed — card is in Ready, user can retry manually
                    let json = serde_json::json!({
                        "id": card.id, "title": card.title, "card_type": card.card_type,
                        "column_id": ready_col.id, "project": project.name,
                        "state": "ready", "dispatch_error": e,
                    });
                    return Ok(vec![Content::text(format!(
                        "Created goal \"{}\" in {} (moved to Ready) but dispatch failed: {}\n\n{}",
                        card.title,
                        project.name,
                        e,
                        serde_json::to_string_pretty(&json).unwrap_or_default()
                    ))]);
                }
            }
        }

        let json = serde_json::json!({
            "id": card.id, "title": card.title, "card_type": card.card_type,
            "column_id": card.column_id, "position": card.position,
            "project": project.name,
        });
        Ok(vec![Content::text(format!(
            "Created card \"{}\" in {} (column: {}, type: {})\n\n{}",
            card.title,
            project.name,
            card.column_id,
            card.card_type,
            serde_json::to_string_pretty(&json).unwrap_or_default()
        ))])
    }

    async fn handle_card_move(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: card_id")?;
        let col_ref = args
            .get("column")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: column")?;
        let position = args
            .get("position")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32);

        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())?;
        let card = cards::get_card(&pool, card_id)
            .await?
            .ok_or_else(|| format!("Card '{}' not found", card_id))?;
        let target_col = Self::resolve_column(&pool, &card.project_id, col_ref).await?;

        let moved = cards::move_card(&pool, card_id, &target_col.id, position)
            .await?
            .ok_or("Card not found after move")?;

        Ok(vec![Content::text(format!(
            "Moved card \"{}\" to column \"{}\" (position {})",
            moved.title, target_col.name, moved.position
        ))])
    }

    async fn handle_card_delete(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: card_id")?;
        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())?;
        let card = cards::get_card(&pool, card_id)
            .await?
            .ok_or_else(|| format!("Card '{}' not found", card_id))?;
        cards::delete_card(&pool, card_id).await?;
        Ok(vec![Content::text(format!(
            "Deleted card \"{}\" (id: {})",
            card.title, card.id
        ))])
    }

    async fn handle_card_list(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let id_or_slug = args
            .get("project_id_or_slug")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: project_id_or_slug")?;
        let (project, pool) = self.resolve_project(id_or_slug).await?;

        let card_type = args.get("card_type").and_then(|v| v.as_str());
        let column_id = if let Some(col_ref) = args.get("column").and_then(|v| v.as_str()) {
            Some(Self::resolve_column(&pool, &project.id, col_ref).await?.id)
        } else {
            None
        };

        let items = cards::list_cards(&pool, &project.id, card_type, column_id.as_deref()).await?;
        let json: Vec<serde_json::Value> = items
            .iter()
            .map(|c| {
                serde_json::json!({
                    "id": c.id, "title": c.title, "card_type": c.card_type,
                    "column_id": c.column_id, "position": c.position,
                    "assigned_to": c.assigned_to,
                })
            })
            .collect();

        Ok(vec![Content::text(format!(
            "{} card(s) in {}\n\n{}",
            items.len(),
            project.name,
            serde_json::to_string_pretty(&json).unwrap_or_default()
        ))])
    }

    async fn handle_column_create(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let id_or_slug = args
            .get("project_id_or_slug")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: project_id_or_slug")?;
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: name")?
            .to_string();
        let position = args
            .get("position")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32);

        let (project, pool) = self.resolve_project(id_or_slug).await?;
        let col = cards::create_column(
            &pool,
            cards::CreateColumn {
                project_id: project.id,
                name,
                position,
            },
        )
        .await?;

        Ok(vec![Content::text(format!(
            "Created column \"{}\" at position {} (id: {})",
            col.name, col.position, col.id
        ))])
    }

    async fn handle_column_delete(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let column_id = args
            .get("column_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: column_id")?;
        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())?;
        let col = cards::get_column(&pool, column_id)
            .await?
            .ok_or_else(|| format!("Column '{}' not found", column_id))?;
        cards::delete_column(&pool, column_id).await?;
        Ok(vec![Content::text(format!(
            "Deleted column \"{}\" (id: {})",
            col.name, col.id
        ))])
    }

    async fn handle_board_summary(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let project_filter = arguments
            .as_ref()
            .and_then(|a| a.get("project_id_or_slug"))
            .and_then(|v| v.as_str());
        let include_standard = arguments
            .as_ref()
            .and_then(|a| a.get("include_standard_cards"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())?;

        // Resolve project filter if provided
        let project_id = if let Some(id_or_slug) = project_filter {
            let p = projects::get_project_by_id_or_slug(&pool, id_or_slug)
                .await?
                .ok_or_else(|| format!("Project '{}' not found", id_or_slug))?;
            Some(p.id)
        } else {
            None
        };

        // Build query with optional project filter
        let mut sql = String::from(
            "SELECT c.id, c.title, c.card_type, c.assigned_to, c.metadata_json,
                    bc.name as column_name, bc.state_binding, bc.column_kind,
                    p.name as project_name, p.slug as project_slug
             FROM cards c
             JOIN board_columns bc ON c.column_id = bc.id
             JOIN projects p ON c.project_id = p.id
             WHERE c.archived_at IS NULL AND p.status = 'active'",
        );

        if project_id.is_some() {
            sql.push_str(" AND c.project_id = ?");
        }
        if !include_standard {
            sql.push_str(" AND c.card_type = 'goal'");
        }
        sql.push_str(" ORDER BY p.name, bc.position, c.position");

        let mut query = sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                Option<String>,
                String,
                String,
                Option<String>,
                String,
                String,
                String,
            ),
        >(&sql);
        if let Some(ref pid) = project_id {
            query = query.bind(pid);
        }

        let rows = query.fetch_all(&pool).await.map_err(|e| e.to_string())?;

        // Format as JSON array
        let items: Vec<serde_json::Value> = rows
            .iter()
            .map(|(id, title, card_type, assigned_to, meta_str, col_name, state_binding, col_kind, project_name, project_slug)| {
                let meta: serde_json::Value = serde_json::from_str(meta_str).unwrap_or_default();
                serde_json::json!({
                    "id": id,
                    "title": title,
                    "card_type": card_type,
                    "assigned_to": assigned_to,
                    "column": col_name,
                    "state": state_binding.as_deref().unwrap_or(if col_kind == "state" { "unknown" } else { col_name.as_str() }),
                    "project": project_name,
                    "project_slug": project_slug,
                    "attempt_count": meta.get("attempt_count").and_then(|v| v.as_u64()).unwrap_or(0),
                    "needs_human_attention": meta.get("needs_human_attention").and_then(|v| v.as_bool()).unwrap_or(false),
                    "last_error": meta.get("last_error").and_then(|v| v.as_str()),
                })
            })
            .collect();

        let scope = if let Some(ref slug) = project_filter {
            format!("project '{}'", slug)
        } else {
            "all active projects".to_string()
        };

        Ok(vec![Content::text(format!(
            "{} card(s) across {}\n\n{}",
            items.len(),
            scope,
            serde_json::to_string_pretty(&items).unwrap_or_default()
        ))])
    }

    pub(crate) fn get_tools() -> Vec<Tool> {
        let create_schema = serde_json::to_value(schema_for!(ProjectCreateParams)).unwrap();
        let update_schema = serde_json::to_value(schema_for!(ProjectUpdateParams)).unwrap();
        let delete_schema = serde_json::to_value(schema_for!(ProjectDeleteParams)).unwrap();
        let list_schema = serde_json::to_value(schema_for!(ProjectListParams)).unwrap();
        let resolve_schema = serde_json::to_value(schema_for!(ProjectResolveParams)).unwrap();
        let card_create_schema = serde_json::to_value(schema_for!(CardCreateParams)).unwrap();
        let card_move_schema = serde_json::to_value(schema_for!(CardMoveParams)).unwrap();
        let card_delete_schema = serde_json::to_value(schema_for!(CardDeleteParams)).unwrap();
        let card_list_schema = serde_json::to_value(schema_for!(CardListParams)).unwrap();
        let col_create_schema = serde_json::to_value(schema_for!(ColumnCreateParams)).unwrap();
        let col_delete_schema = serde_json::to_value(schema_for!(ColumnDeleteParams)).unwrap();
        let board_summary_schema = serde_json::to_value(schema_for!(BoardSummaryParams)).unwrap();
        let launch_schema = serde_json::to_value(schema_for!(ProjectLaunchParams)).unwrap();

        vec![
            Tool::new(
                "project_create".to_string(),
                indoc! {r#"
                Create a new project workspace. Use when the user asks to "set up a project",
                "create a project", or similar. Walk the user through the required field (name)
                and optional fields (root_path, site_url, repo_url, description, tags)
                conversationally.
            "#}
                .to_string(),
                create_schema.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Create Project".to_string()),
                Some(false),
                Some(true),
                Some(false),
                Some(false),
            )),
            Tool::new(
                "project_update".to_string(),
                indoc! {r#"
                Update an existing project. Accepts the project ID or slug and any fields
                to change. Use when the user says "update project X", "change the root path
                for Y", etc.
            "#}
                .to_string(),
                update_schema.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Update Project".to_string()),
                Some(false),
                Some(true),
                Some(false),
                Some(false),
            )),
            Tool::new(
                "project_delete".to_string(),
                indoc! {r#"
                Delete a project. Accepts the project ID or slug. The implicit "Personal"
                project cannot be deleted. Confirm with the user before deleting.
            "#}
                .to_string(),
                delete_schema.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Delete Project".to_string()),
                Some(true),
                Some(true),
                Some(false),
                Some(false),
            )),
            Tool::new(
                "project_list".to_string(),
                indoc! {r#"
                List all projects. Optionally filter by status (active, paused, archived).
                Use when the user asks "what projects do I have?", "show my projects", etc.
            "#}
                .to_string(),
                list_schema.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("List Projects".to_string()),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            )),
            Tool::new(
                "project_resolve".to_string(),
                indoc! {r#"
                Resolve a spoken or approximate project name to an exact project.
                Use when the user mentions a project by name (especially via voice)
                and you need to find the matching project ID. Performs fuzzy matching
                against project names and slugs to handle transcription errors
                (e.g. "Kinros" matching "Kinross"). If multiple matches are found,
                confirm with the user before proceeding. Then use navigate_app with
                state: { project_id: "<id>" } to open the project's detail view.
            "#}
                .to_string(),
                resolve_schema.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Resolve Project".to_string()),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            )),
            // ── Card tools ──
            Tool::new(
                "card_create".to_string(),
                indoc! {r#"
                Create a card on a project's Kanban board. Use when the user says "add a card",
                "create a task", "track this", etc. Defaults to card_type='standard' and places
                the card in the first column (Backlog) unless specified.

                For goal cards (card_type='goal'): set auto_dispatch=true to immediately
                assign a worker and begin execution. The card moves Triage → Ready → InProgress
                automatically. If auto_dispatch is false or omitted, the goal stays in Triage.
            "#}
                .to_string(),
                card_create_schema.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Create Card".to_string()),
                Some(false),
                Some(true),
                Some(false),
                Some(false),
            )),
            Tool::new(
                "card_move".to_string(),
                indoc! {r#"
                Move a card to a different column on the Kanban board. Use when the user says
                "move X to Doing", "mark X as done", etc. Accepts column name or ID.
            "#}
                .to_string(),
                card_move_schema.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Move Card".to_string()),
                Some(false),
                Some(true),
                Some(false),
                Some(false),
            )),
            Tool::new(
                "card_delete".to_string(),
                indoc! {r#"
                Delete a card from the Kanban board. Confirm with the user before deleting.
            "#}
                .to_string(),
                card_delete_schema.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Delete Card".to_string()),
                Some(true),
                Some(true),
                Some(false),
                Some(false),
            )),
            Tool::new(
                "card_list".to_string(),
                indoc! {r#"
                List cards in a project. Optionally filter by card_type or column.
                Use when the user asks "what cards are in X?", "show my board", etc.
            "#}
                .to_string(),
                card_list_schema.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("List Cards".to_string()),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            )),
            // ── Column tools ──
            Tool::new(
                "column_create".to_string(),
                indoc! {r#"
                Add a new column to a project's Kanban board. Use when the user wants
                to customize their board layout, e.g. "add a Review column".
            "#}
                .to_string(),
                col_create_schema.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Create Column".to_string()),
                Some(false),
                Some(true),
                Some(false),
                Some(false),
            )),
            Tool::new(
                "column_delete".to_string(),
                indoc! {r#"
                Delete a column from a project's Kanban board. Fails if the column
                still contains cards. Confirm with the user before deleting.
            "#}
                .to_string(),
                col_delete_schema.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Delete Column".to_string()),
                Some(true),
                Some(true),
                Some(false),
                Some(false),
            )),
            // ── Board summary ──
            Tool::new(
                "board_summary".to_string(),
                indoc! {r#"
                Get a full board summary across all active projects (or a specific project).
                Returns detailed card information including state, worker assignment, and errors.
                By default shows goal cards only. Set include_standard_cards=true for all cards.
                Use when you need more detail than the ambient board context provides.
            "#}
                .to_string(),
                board_summary_schema.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Board Summary".to_string()),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            )),
            // ── Project launch (terminal) ──
            Tool::new(
                "research_project_intel".to_string(),
                indoc! {r#"
                Start an ecosystem and competitive-intelligence research pass for
                a project. Returns existing findings and a bounded briefing for
                competitors, partners, and adjacent ecosystem players. Research
                with your web tools, then file cited findings via
                propose_project_intel.
            "#}
                .to_string(),
                serde_json::to_value(schema_for!(ResearchProjectIntelParams))
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Research Project Intelligence".to_string()),
                Some(true),
                Some(false),
                Some(true),
                Some(false),
            )),
            Tool::new(
                "propose_project_intel".to_string(),
                indoc! {r#"
                File cited project-intelligence findings as a review-gated
                Decision Inbox proposal. Each item needs
                {kind, name, note, source_url}; kind is competitor, partner, or
                adjacent. Nothing is stored until the user approves.
            "#}
                .to_string(),
                serde_json::to_value(schema_for!(ProposeProjectIntelParams))
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Propose Project Intelligence".to_string()),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            )),
            Tool::new(
                "dismiss_project_intel".to_string(),
                indoc! {r#"
                Remove a stored project-intelligence item (a competitor, partner,
                or adjacent player) that is stale or wrong, by name — the inverse
                of propose_project_intel. Use it when the user says an item no
                longer belongs. The name is matched case-insensitively (as shown
                by research_project_intel); pass kind to disambiguate. Applied
                directly — removal is user-directed and reversible by re-researching.
            "#}
                .to_string(),
                serde_json::to_value(schema_for!(DismissProjectIntelParams))
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Dismiss Project Intelligence".to_string()),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            )),
            Tool::new(
                "project_launch".to_string(),
                indoc! {r#"
                Open a project-aware terminal in the Build tab, rooted at the project's
                directory, optionally running a command. This is your native way to launch
                a project and run interactive tools inside it.

                Use this — NOT a one-shot shell — whenever the user asks to "launch project X",
                "open a terminal in Y", "start Claude Code in the grocery-saver project", "run
                the dev server for Z", or similar. To start Claude Code, pass command="claude".
                For an interactive shell with no command, omit `command`.

                Resolve the project name with project_resolve first if you only have a spoken
                name. The project must have a root_path set.

                To run a SUPERVISED Claude Code session — visible in the tab, structured
                stream-json output, permission gates enabled so they can be watched — pass
                supervised=true (optionally with `prompt` for the initial instruction)
                instead of command="claude".
            "#}
                .to_string(),
                launch_schema.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Launch Project Terminal".to_string()),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            )),
        ]
    }
}

#[async_trait]
impl McpClientTrait for ProjectManagerClient {
    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
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
        _cancellation_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        let content = match name {
            "project_create" => self.handle_create(arguments).await,
            "project_update" => self.handle_update(arguments).await,
            "project_delete" => self.handle_delete(arguments).await,
            "project_list" => self.handle_list(arguments).await,
            "project_resolve" => self.handle_resolve(arguments).await,
            "card_create" => self.handle_card_create(arguments).await,
            "card_move" => self.handle_card_move(arguments).await,
            "card_delete" => self.handle_card_delete(arguments).await,
            "card_list" => self.handle_card_list(arguments).await,
            "column_create" => self.handle_column_create(arguments).await,
            "column_delete" => self.handle_column_delete(arguments).await,
            "board_summary" => self.handle_board_summary(arguments).await,
            "research_project_intel" => self.handle_research_project_intel(arguments).await,
            "propose_project_intel" => self.handle_propose_project_intel(arguments).await,
            "dismiss_project_intel" => self.handle_dismiss_project_intel(arguments).await,
            "project_launch" => self.handle_launch(arguments).await,
            _ => Err(format!("Unknown tool: {}", name)),
        };
        match content {
            Ok(content) => Ok(CallToolResult::success(content)),
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

/// Simple Levenshtein distance for fuzzy name matching.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());
    let mut prev = (0..=n).collect::<Vec<_>>();
    let mut curr = vec![0; n + 1];
    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}
