pub mod analyze;
pub mod app_conductor;
pub mod apps;
pub mod browser;
pub mod chatrecall;
#[cfg(feature = "code-mode")]
pub mod code_execution;
pub mod developer;
pub mod ext_manager;
pub mod librarian;
pub mod librarian_state;
pub mod orchestrator;
pub mod project_manager;
pub mod recipe_author;
pub mod skills;
pub mod steward;
pub mod storage_health;
pub mod summarize;
pub mod summon;
pub mod todo;
pub mod tom;

use std::collections::HashMap;
use std::path::PathBuf;

use crate::agents::mcp_client::McpClientTrait;
use crate::session::Session;
use once_cell::sync::Lazy;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Source {
    pub name: String,
    pub kind: SourceKind,
    pub description: String,
    pub path: PathBuf,
    pub content: String,
    pub supporting_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceKind {
    Subrecipe,
    Recipe,
    Skill,
    Agent,
    BuiltinSkill,
}

impl std::fmt::Display for SourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceKind::Subrecipe => write!(f, "subrecipe"),
            SourceKind::Recipe => write!(f, "recipe"),
            SourceKind::Skill => write!(f, "skill"),
            SourceKind::Agent => write!(f, "agent"),
            SourceKind::BuiltinSkill => write!(f, "builtin skill"),
        }
    }
}

impl Source {
    pub fn to_load_text(&self) -> String {
        format!(
            "## {} ({})\n\n{}\n\n### Content\n\n{}",
            self.name, self.kind, self.description, self.content
        )
    }
}

pub fn parse_frontmatter<T: for<'de> Deserialize<'de>>(
    content: &str,
) -> Result<Option<(T, String)>, serde_yaml::Error> {
    let parts: Vec<&str> = content.split("---").collect();
    if parts.len() < 3 {
        return Ok(None);
    }

    let yaml_content = parts[1].trim();
    let metadata: T = serde_yaml::from_str(yaml_content)?;

    let body = parts[2..].join("---").trim().to_string();
    Ok(Some((metadata, body)))
}

pub use ext_manager::MANAGE_EXTENSIONS_TOOL_NAME_COMPLETE;

// These are used by integration tests in crates/goose/tests/
#[allow(unused_imports)]
pub use ext_manager::MANAGE_EXTENSIONS_TOOL_NAME;
#[allow(unused_imports)]
pub use ext_manager::SEARCH_AVAILABLE_EXTENSIONS_TOOL_NAME;

/// Platform extensions that represent embodied agents in the World View.
/// Used by GET /api/agents to build the agent roster for the picker dropdown.
/// Orchestrator is deliberately excluded — it manages sessions but isn't a character.
pub static AGENT_EXTENSIONS: &[&str] = &[librarian::EXTENSION_NAME];

pub static PLATFORM_EXTENSIONS: Lazy<HashMap<&'static str, PlatformExtensionDef>> = Lazy::new(
    || {
        let mut map = HashMap::new();

        map.insert(
            analyze::EXTENSION_NAME,
            PlatformExtensionDef {
                name: analyze::EXTENSION_NAME,
                display_name: "Analyze",
                description:
                    "Analyze code structure with tree-sitter: directory overviews, file details, symbol call graphs",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Reach for it before editing unfamiliar code — it maps structure and call graphs so you change the right thing.",
                teaching: &[],
                client_factory: |ctx| Box::new(analyze::AnalyzeClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            browser::EXTENSION_NAME,
            PlatformExtensionDef {
                name: browser::EXTENSION_NAME,
                display_name: "Browser",
                description: "Read content from the page currently open in the Permagent browser",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Use it to read what the user is actually looking at instead of guessing or asking them to paste.",
                teaching: &[],
                client_factory: |ctx| Box::new(browser::BrowserClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            todo::EXTENSION_NAME,
            PlatformExtensionDef {
                name: todo::EXTENSION_NAME,
                display_name: "Todo",
                description:
                    "Enable a todo list for the agent so it can keep track of what it is doing",
                default_enabled: true,
                unprefixed_tools: false,
                hidden: false,
                why_it_matters: "Track multi-step work so nothing is dropped across a long task.",
                teaching: &[],
                client_factory: |ctx| Box::new(todo::TodoClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            apps::EXTENSION_NAME,
            PlatformExtensionDef {
                name: apps::EXTENSION_NAME,
                display_name: "Apps",
                description:
                    "Create and manage custom Permagent apps through chat. Apps are HTML/CSS/JavaScript and run in sandboxed windows.",
                default_enabled: true,
                unprefixed_tools: false,
                hidden: false,
                why_it_matters:
                    "Build the user a small interactive tool on the spot instead of just describing one.",
                teaching: &[],
                client_factory: |ctx| Box::new(apps::AppsManagerClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            chatrecall::EXTENSION_NAME,
            PlatformExtensionDef {
                name: chatrecall::EXTENSION_NAME,
                display_name: "Chat Recall",
                description:
                    "Search past conversations and load session summaries for contextual memory",
                default_enabled: false,
                unprefixed_tools: false,
                hidden: false,
                why_it_matters:
                    "Recover context from past conversations so the user does not have to repeat themselves.",
                teaching: &[],
                client_factory: |ctx| Box::new(chatrecall::ChatRecallClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            "extensionmanager",
            PlatformExtensionDef {
                name: ext_manager::EXTENSION_NAME,
                display_name: "Extension Manager",
                description:
                    "Enable extension management tools for discovering, enabling, and disabling extensions",
                default_enabled: true,
                unprefixed_tools: false,
                hidden: false,
                why_it_matters:
                    "Turn capabilities on and off mid-task so you always have the right tools loaded.",
                teaching: &[],
                client_factory: |ctx| Box::new(ext_manager::ExtensionManagerClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            summon::EXTENSION_NAME,
            PlatformExtensionDef {
                name: summon::EXTENSION_NAME,
                display_name: "Summon",
                description: "Load knowledge and delegate tasks to subagents",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Delegate heavy or parallel subtasks to subagents instead of doing everything in one context.",
                teaching: &[],
                client_factory: |ctx| Box::new(summon::SummonClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            summarize::EXTENSION_NAME,
            PlatformExtensionDef {
                name: summarize::EXTENSION_NAME,
                display_name: "Summarize",
                description: "Load files/directories and get an LLM summary in a single call",
                default_enabled: false,
                unprefixed_tools: false,
                hidden: false,
                why_it_matters:
                    "Digest large files or directories in one call rather than reading them piecemeal.",
                teaching: &[],
                client_factory: |ctx| Box::new(summarize::SummarizeClient::new(ctx).unwrap()),
            },
        );

        #[cfg(feature = "code-mode")]
        map.insert(
            code_execution::EXTENSION_NAME,
            PlatformExtensionDef {
                name: code_execution::EXTENSION_NAME,
                display_name: "Code Mode",
                description: "Make extension calls through code execution, saving tokens",
                default_enabled: false,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Batch many tool calls into one code block to save tokens on complex work.",
                teaching: &[],
                client_factory: |ctx| {
                    Box::new(
                        code_execution::CodeExecutionClient::new(
                            ctx,
                            code_execution::get_tool_disclosure(),
                        )
                        .unwrap(),
                    )
                },
            },
        );

        map.insert(
            developer::EXTENSION_NAME,
            PlatformExtensionDef {
                name: developer::EXTENSION_NAME,
                display_name: "Developer",
                description: "Write and edit files, and execute shell commands",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Your primary hands — read, write, and run things on the user's machine.",
                teaching: &[],
                client_factory: |ctx| Box::new(developer::DeveloperClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            orchestrator::EXTENSION_NAME,
            PlatformExtensionDef {
                name: orchestrator::EXTENSION_NAME,
                display_name: "Orchestrator",
                description:
                    "Manage agent sessions: list, view, start, send messages, interrupt, and stop agents",
                default_enabled: true,
                unprefixed_tools: false,
                hidden: false,
                why_it_matters:
                    "Start, watch, and steer other agent sessions when one agent is not enough.",
                teaching: &[],
                client_factory: |ctx| Box::new(orchestrator::OrchestratorClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            tom::EXTENSION_NAME,
            PlatformExtensionDef {
                name: tom::EXTENSION_NAME,
                display_name: "Top Of Mind",
                description:
                    "Inject custom context into every turn via GOOSE_MOIM_MESSAGE_TEXT and GOOSE_MOIM_MESSAGE_FILE environment variables",
                default_enabled: true,
                unprefixed_tools: false,
                hidden: false,
                why_it_matters:
                    "Inject standing context into every turn so persistent facts are never forgotten.",
                teaching: &[],
                client_factory: |ctx| Box::new(tom::TomClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            skills::EXTENSION_NAME,
            PlatformExtensionDef {
                name: skills::EXTENSION_NAME,
                display_name: "Skills",
                description: "Discover and provide skill instructions from filesystem and builtins",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Pull in proven step-by-step procedures instead of improvising a workflow.",
                teaching: &[],
                client_factory: |ctx| Box::new(skills::SkillsClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            librarian::EXTENSION_NAME,
            PlatformExtensionDef {
                name: librarian::EXTENSION_NAME,
                display_name: "Librarian",
                description:
                    "Memory archivist — generates prose descriptions for Brain memories using a local LLM (Ollama)",
                default_enabled: true,
                unprefixed_tools: false,
                hidden: false,
                why_it_matters:
                    "Generates prose descriptions for memories in the background so later recall stays sharp.",
                teaching: &[],
                client_factory: |ctx| Box::new(librarian::LibrarianClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            app_conductor::EXTENSION_NAME,
            PlatformExtensionDef {
                name: app_conductor::EXTENSION_NAME,
                display_name: "App Conductor",
                description: "Navigate the user to specific tabs and views in the Permagent app",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Take the user straight to the right view instead of telling them where to click.",
                teaching: &[],
                client_factory: |ctx| {
                    Box::new(app_conductor::AppConductorClient::new(ctx).unwrap())
                },
            },
        );

        map.insert(
            recipe_author::EXTENSION_NAME,
            PlatformExtensionDef {
                name: recipe_author::EXTENSION_NAME,
                display_name: "Recipe Author",
                description:
                    "Create, list, and manage scheduled automations and saved skills through chat",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Turn a repeatable task into a saved automation or schedule the user can rely on.",
                teaching: &[],
                client_factory: |ctx| {
                    Box::new(recipe_author::RecipeAuthorClient::new(ctx).unwrap())
                },
            },
        );

        map.insert(
            storage_health::EXTENSION_NAME,
            PlatformExtensionDef {
                name: storage_health::EXTENSION_NAME,
                display_name: "Storage Health",
                description:
                    "Scan the filesystem for storage cleanup opportunities — dev caches, app caches, stale downloads, and large files",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Find and reclaim wasted disk before it becomes a problem.",
                teaching: &[],
                client_factory: |ctx| {
                    Box::new(storage_health::StorageHealthClient::new(ctx).unwrap())
                },
            },
        );

        map.insert(
            steward::EXTENSION_NAME,
            PlatformExtensionDef {
                name: steward::EXTENSION_NAME,
                display_name: "Git Steward",
                description:
                    "Safety gate for autonomous repo hygiene — routes destructive git operations (branch delete, history rewrite, force-push) to human approval and hard-refuses protected branches",
                default_enabled: false,
                unprefixed_tools: true,
                hidden: true,
                why_it_matters:
                    "Gates destructive git operations behind human approval so autonomous repo work stays safe.",
                teaching: &[],
                client_factory: |ctx| Box::new(steward::StewardClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            project_manager::EXTENSION_NAME,
            PlatformExtensionDef {
                name: project_manager::EXTENSION_NAME,
                display_name: "Project Manager",
                description:
                    "Create, list, update, and delete projects — named workspaces with paths, URLs, and metadata",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Keep the user's workspaces, paths, and URLs organized so project context is one lookup away.",
                teaching: &[],
                client_factory: |ctx| {
                    Box::new(project_manager::ProjectManagerClient::new(ctx).unwrap())
                },
            },
        );

        map
    },
);

/// Global SafeBrain handle, set once by the daemon at startup.
/// Platform extensions can access this without plumbing Brain through every layer.
static GLOBAL_BRAIN: std::sync::OnceLock<crate::brain_handle::SafeBrain> =
    std::sync::OnceLock::new();

pub fn set_global_brain(brain: crate::brain_handle::SafeBrain) {
    let _ = GLOBAL_BRAIN.set(brain);
}

pub fn get_global_brain() -> Option<crate::brain_handle::SafeBrain> {
    GLOBAL_BRAIN.get().cloned()
}

#[derive(Clone)]
pub struct PlatformExtensionContext {
    pub extension_manager:
        Option<std::sync::Weak<crate::agents::extension_manager::ExtensionManager>>,
    pub session_manager: std::sync::Arc<crate::session::SessionManager>,
    pub session: Option<std::sync::Arc<Session>>,
}

impl PlatformExtensionContext {
    pub fn result_with_platform_notification(
        &self,
        mut result: rmcp::model::CallToolResult,
        extension_name: impl Into<String>,
        event_type: impl Into<String>,
        mut additional_params: serde_json::Map<String, serde_json::Value>,
    ) -> rmcp::model::CallToolResult {
        additional_params.insert("extension".to_string(), extension_name.into().into());
        additional_params.insert("event_type".to_string(), event_type.into().into());

        let meta_value = serde_json::json!({
            "platform_notification": {
                "method": "platform_event",
                "params": additional_params
            }
        });

        if let Some(ref mut meta) = result.meta {
            if let Some(obj) = meta_value.as_object() {
                for (k, v) in obj {
                    meta.0.insert(k.clone(), v.clone());
                }
            }
        } else {
            result.meta = Some(rmcp::model::Meta(meta_value.as_object().unwrap().clone()));
        }

        result
    }
}

/// Definition for a platform extension that runs in-process with direct agent access.
#[derive(Debug, Clone)]
pub struct PlatformExtensionDef {
    pub name: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub default_enabled: bool,
    /// If true, tools are exposed without extension prefix for intuitive first-class use.
    pub unprefixed_tools: bool,
    /// If true, the extension is not shown in the UI or discoverable via search_available_extensions.
    pub hidden: bool,
    /// Self-knowledge: why the agent should reach for this tool. Non-`Option` so
    /// every registry entry must supply it — a missing one is a compile error.
    /// See [`crate::agents::self_knowledge`].
    pub why_it_matters: &'static str,
    /// Self-knowledge Phase-2 hook: how to drive this tool. `&[]` in Phase 1.
    pub teaching: &'static [crate::agents::self_knowledge::TeachingStep],
    pub client_factory: fn(PlatformExtensionContext) -> Box<dyn McpClientTrait>,
}

impl PlatformExtensionDef {
    /// Derive the self-knowledge [`FeatureDescriptor`] for this tool. Tools are
    /// always `Tool`/`Queryable` (their enabled state is checked via
    /// `config::extensions::is_extension_enabled`).
    ///
    /// [`FeatureDescriptor`]: crate::agents::self_knowledge::FeatureDescriptor
    pub fn descriptor(&self) -> crate::agents::self_knowledge::FeatureDescriptor {
        crate::agents::self_knowledge::FeatureDescriptor {
            id: self.name,
            display_name: self.display_name,
            category: crate::agents::self_knowledge::FeatureCategory::Tool,
            what_it_does: self.description,
            why_it_matters: self.why_it_matters,
            state_source: crate::agents::self_knowledge::StateSource::Queryable,
            teaching: self.teaching,
        }
    }
}
