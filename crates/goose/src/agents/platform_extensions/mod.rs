pub mod analyze;
pub mod app_conductor;
pub mod app_perception;
pub mod apps;
pub mod best_of_n_adapter;
pub mod browser;
pub mod chatrecall;
#[cfg(feature = "code-mode")]
pub mod code_execution;
pub(crate) mod code_map;
pub mod dashboard;
pub mod desktop;
pub mod developer;
pub mod dispatch_brief;
pub mod dispatch_scope;
pub mod execution_receipt;
pub mod ext_manager;
pub mod file_to_project;
pub mod finance;
pub mod gate_classifier;
pub mod goal_a2a;
pub mod goal_engine;
pub mod inbox_tools;
pub mod librarian;
pub mod librarian_adjudicator;
pub mod librarian_atoms;
pub mod librarian_context;
pub mod librarian_entities;
pub mod librarian_state;
pub mod listen;
pub mod model_manager;
pub mod orchestrator;
pub mod people;
pub mod project_manager;
pub mod pronunciation;
pub mod publish_sequence;
pub mod recipe_author;
pub mod retrospect;
pub mod review_fanout;
pub mod role_brief;
pub mod skills;
pub mod steward;
pub mod storage_health;
pub mod summarize;
pub mod summon;
pub mod supervised_cli;
pub mod terminal_supervision;
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
                    "Analyze code structure with tree-sitter: directory overviews, file details, symbol call graphs (analyze); query a project's indexed code map for the paths matching a term, with their ancestor directories (map_query)",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Reach for it before editing unfamiliar code — it maps structure and call graphs so you change the right thing.",
                required_secrets: &[],
                teaching: &[],
                client_factory: |ctx| Box::new(analyze::AnalyzeClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            browser::EXTENSION_NAME,
            PlatformExtensionDef {
                name: browser::EXTENSION_NAME,
                display_name: "Browser",
                description: "Drive, read, and act on the web: open any site in the in-app \
                              browser (open_website), fetch a public page's readable text without \
                              a tab (read_webpage), read the page the user currently has open \
                              (read_browser_content), list a page's interactive elements as \
                              stable refs (get_page_snapshot), click, type, or select on them \
                              (act_on_page), and — when the browser is unreachable because the \
                              Mac's app is closed — wake it (wake_desktop_app)",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "When the user says 'go to BBC and read me the homepage', open_website shows \
                     it to them and read_webpage gives you the text to read aloud — no pasting, \
                     no guessing. read_browser_content covers whatever tab they already have \
                     open. And when they need something DONE on a page — fill a form, click a \
                     button, pick an option — get_page_snapshot lists the interactive elements \
                     and act_on_page clicks, types, or selects, so you drive the page instead of \
                     only reading it. open_website also opens a LOCAL dev server \
                     (http://localhost:PORT) in the browser, so after you build or scaffold an \
                     app you can show the user the running result — the coding last mile.",
                required_secrets: &[],
                teaching: &[
                    crate::agents::self_knowledge::TeachingStep {
                        title: "Browse together",
                        body: "Offer it live: open a site the user cares about with open_website, \
                               then read_webpage the same URL and give them the highlights out \
                               loud. Works by voice too — this is the hands-free news flow.",
                        open_surface: Some(crate::agents::self_knowledge::SurfaceRef {
                            tab: "Build",
                            section: Some("browser"),
                        }),
                        confirm: None,
                    },
                    crate::agents::self_knowledge::TeachingStep {
                        title: "Act on the page, don't just read it",
                        body: "When a page needs DOING — a search box, a form, a button — call \
                               get_page_snapshot to see the interactive elements as numbered \
                               refs, then act_on_page with a ref to click, type, or select. Take \
                               a fresh snapshot after each act; the page may have changed.",
                        open_surface: Some(crate::agents::self_knowledge::SurfaceRef {
                            tab: "Build",
                            section: Some("browser"),
                        }),
                        confirm: None,
                    },
                ],
                client_factory: |ctx| Box::new(browser::BrowserClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            listen::EXTENSION_NAME,
            PlatformExtensionDef {
                name: listen::EXTENSION_NAME,
                display_name: "Audience Listening",
                description:
                    "Listen to what an audience is saying about a topic or on a channel (listen_to_audience) — RSS-first and zero-config: a topic reads live news chatter, or point it at a feed URL (a subreddit, blog, or podcast) for a specific channel; it health-probes each source and returns only real recent items (title, snippet, date, link)",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Ground a project's Grow strategy — its Audience and Channels — in what people are really saying, not guesses. When the user wants to understand or reach an audience, listen first: it tries RSS then web_search in order, reports which backend answered, and never fabricates chatter",
                required_secrets: &[],
                teaching: &[
                    crate::agents::self_knowledge::TeachingStep {
                        title: "Open Grow",
                        body: "Bring the user to the project's go-to-market workspace, where \
                               audience and channel strategy live.",
                        open_surface: Some(crate::agents::self_knowledge::SurfaceRef {
                            tab: "Grow",
                            section: None,
                        }),
                        confirm: None,
                    },
                    crate::agents::self_knowledge::TeachingStep {
                        title: "Listen to their audience",
                        body: "Offer to listen on a topic or channel the user cares about — call \
                               listen_to_audience and read back the real, recent items — then use \
                               what you heard to sharpen the Audience and Channels pillars together.",
                        open_surface: None,
                        confirm: None,
                    },
                ],
                client_factory: |ctx| Box::new(listen::ListenClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            dashboard::EXTENSION_NAME,
            PlatformExtensionDef {
                name: dashboard::EXTENSION_NAME,
                display_name: "Dashboard",
                description: "Read the live cards on the user's Home dashboard (read_dashboard) — \
                              local weather, system stats, today's calendar — the same numbers \
                              they are looking at",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "The dashboard already holds answers, localized to this user. Asked for the \
                     weather, read the weather card instead of searching the web — the card knows \
                     where they are, needs no API key, and matches what is on their screen. \
                     Searching for something a card already shows is slower and often wrong.",
                required_secrets: &[],
                teaching: &[crate::agents::self_knowledge::TeachingStep {
                    title: "Read the room",
                    body: "Bring the user Home and read a card back to them — the weather where \
                           they actually are, or how their machine is doing — so they see you \
                           looking at the same screen they are.",
                    open_surface: Some(crate::agents::self_knowledge::SurfaceRef {
                        tab: "Dashboard",
                        section: None,
                    }),
                    confirm: None,
                }],
                client_factory: |ctx| Box::new(dashboard::DashboardClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            inbox_tools::EXTENSION_NAME,
            PlatformExtensionDef {
                name: inbox_tools::EXTENSION_NAME,
                display_name: "Decision Inbox",
                description: "Surface and settle the user's Decision Inbox from chat: read what \
                              is waiting on them (list_pending_decisions) and apply the verdict \
                              they state in conversation (answer_decisions), bundling related \
                              items — five finished goal cards, one approval",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Approvals the user must chase across the app do not get answered. When they \
                     ask what needs them — or when pending decisions are the obvious blocker — \
                     bring the Inbox to them, grouped into bundles they can settle in one \
                     breath. The verdict is ALWAYS the user's words from this conversation; \
                     never answer a decision on your own judgment, and read the bundle back \
                     before applying it",
                required_secrets: &[],
                teaching: &[crate::agents::self_knowledge::TeachingStep {
                    title: "Settle what is waiting",
                    body: "Read the user their open decisions grouped into bundles ('five \
                           reviews of goals we already finished'), let them say the word, and \
                           apply exactly what they said — then confirm what changed.",
                    open_surface: None,
                    confirm: None,
                }],
                client_factory: |ctx| Box::new(inbox_tools::InboxClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            retrospect::EXTENSION_NAME,
            PlatformExtensionDef {
                name: retrospect::EXTENSION_NAME,
                display_name: "Retrospect",
                description: "Review where you struggled in a session (review_struggles), record \
                              a grounded failure without fixing it (report_failure), propose a \
                              regression test that pins it (propose_regression), and ask the \
                              user for a tool you turned out not to have (request_capability) — \
                              Decision Inbox proposals, never builds",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Some failures are you using a tool badly; some are you not having the tool. \
                     review_struggles tells them apart from the recorded outcome of every call — \
                     the same shape retried unchanged reads differently from six different \
                     formulations that all missed. When it is a missing tool, say so and file \
                     request_capability with what you tried, rather than flailing again next \
                     time. When something went wrong, report_failure records the grounded \
                     incident but does not fix it. Asking for a better tool is not a failure; \
                     hiding the gap is.",
                required_secrets: &[],
                teaching: &[crate::agents::self_knowledge::TeachingStep {
                    title: "Look back at a rough patch",
                    body: "After a session that went badly, call review_struggles and read the \
                           result back honestly — including that a confidently wrong answer \
                           leaves no failed call behind, so silence there is not proof it went \
                           well. Record grounded failures with report_failure; if a tool was \
                           missing, file request_capability.",
                    open_surface: None,
                    confirm: None,
                }],
                client_factory: |ctx| Box::new(retrospect::RetrospectClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            pronunciation::EXTENSION_NAME,
            PlatformExtensionDef {
                name: pronunciation::EXTENSION_NAME,
                display_name: "Pronunciation",
                description: "Save how a word is pronounced (save_pronunciation) so your spoken \
                              voice says names like the user does — the fix for coined words \
                              being spelled out letter by letter",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "THE RULE: never spell a word out loud. If you are unsure how a name will \
                     sound — or the user winces at your pronunciation — stop and ask them to \
                     say it. Teaching you a pronunciation REQUIRES a save_pronunciation call in \
                     that same turn: pass the word and a sounds-like respelling made of real \
                     English words (never IPA — the engine derives the phonemes itself). \
                     Answering that you will remember it, without the call, stores nothing. \
                     Then read back what the tool confirms and say it aloud. Saved once, \
                     spoken right forever.",
                required_secrets: &[],
                teaching: &[crate::agents::self_knowledge::TeachingStep {
                    title: "Teach me your words",
                    body: "Invite the user to say any name you might mangle — their company, \
                           their dog, their product. Save each with save_pronunciation and \
                           repeat it back in your voice so they hear it stick.",
                    open_surface: None,
                    confirm: None,
                }],
                client_factory: |ctx| {
                    Box::new(pronunciation::PronunciationClient::new(ctx).unwrap())
                },
            },
        );

        map.insert(
            todo::EXTENSION_NAME,
            PlatformExtensionDef {
                name: todo::EXTENSION_NAME,
                display_name: "Todo",
                description:
                    "Keep a persistent todo list of what you are doing — one tool (todo_write) overwrites its entire content, and it survives across turns and compaction",
                default_enabled: true,
                unprefixed_tools: false,
                hidden: false,
                why_it_matters: "Track multi-step work so nothing is dropped across a long task.",
                required_secrets: &[],
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
                    "Create and manage custom Permagent apps through chat — see what exists (list_apps), generate a new app from a description (create_app), improve one from feedback (iterate_app), or remove one (delete_app). Apps are HTML/CSS/JavaScript and run in sandboxed windows",
                default_enabled: true,
                unprefixed_tools: false,
                hidden: false,
                why_it_matters:
                    "Build the user a small interactive tool on the spot instead of just describing one.",
                required_secrets: &[],
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
                    "Search past conversations and load session summaries for contextual memory, in one tool (chatrecall): search mode takes keywords, load mode returns a session's first and last messages",
                default_enabled: false,
                unprefixed_tools: false,
                hidden: false,
                why_it_matters:
                    "Recover context from past conversations so the user does not have to repeat themselves.",
                required_secrets: &[],
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
                    "Search your long-term Brain memory to recall facts and context (search_memory), discover other extensions you can turn on (search_available_extensions), enable or disable them (manage_extensions), and list or read the resources an extension exposes (list_resources, read_resource)",
                default_enabled: true,
                unprefixed_tools: false,
                hidden: false,
                why_it_matters:
                    "Turn capabilities on and off mid-task so you always have the right tools loaded.",
                required_secrets: &[],
                teaching: &[],
                client_factory: |ctx| Box::new(ext_manager::ExtensionManagerClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            summon::EXTENSION_NAME,
            PlatformExtensionDef {
                name: summon::EXTENSION_NAME,
                display_name: "Summon",
                description: "Load knowledge into your context — subrecipes, recipes, agents, and background-task results (load) — and delegate tasks to subagents that run independently, in parallel or in the background (delegate)",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Delegate heavy or parallel subtasks to subagents instead of doing everything in one context.",
                required_secrets: &[],
                teaching: &[],
                client_factory: |ctx| Box::new(summon::SummonClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            summarize::EXTENSION_NAME,
            PlatformExtensionDef {
                name: summarize::EXTENSION_NAME,
                display_name: "Summarize",
                description: "Load files/directories and get an LLM summary in a single call (summarize)",
                default_enabled: false,
                unprefixed_tools: false,
                hidden: false,
                why_it_matters:
                    "Digest large files or directories in one call rather than reading them piecemeal.",
                required_secrets: &[],
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
                description: "Make extension calls through code execution, saving tokens: discover callable functions (list_functions, get_function_details) and run many calls in one script (execute_typescript, execute_bash) — which of these are exposed depends on the configured disclosure mode",
                default_enabled: false,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Batch many tool calls into one code block to save tokens on complex work.",
                required_secrets: &[],
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
            desktop::EXTENSION_NAME,
            PlatformExtensionDef {
                name: desktop::EXTENSION_NAME,
                display_name: "Desktop Control",
                description:
                    "Ground and act on native desktop apps through the macOS accessibility tree — check permission and flag state (desktop_status), list running apps (desktop_apps), snapshot an app's real UI elements as stable refs (desktop_tree), and press a control (desktop_click) or replace an editable field's text (desktop_type) by element ref — local-first grounding in real UI elements, never cloud screenshots",
                default_enabled: false,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Act on the user's other desktop apps by real UI element, entirely on-device — the local-first alternative to cloud screenshot computer use. Double-gated by design: default-off behind DESKTOP_CONTROL_ENABLED plus the macOS Accessibility permission (desktop_status explains both), every action re-checks the flag, and clicking and typing are approval-gated like shell.",
                required_secrets: &[],
                teaching: &[],
                client_factory: |ctx| Box::new(desktop::DesktopClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            developer::EXTENSION_NAME,
            PlatformExtensionDef {
                name: developer::EXTENSION_NAME,
                display_name: "Developer",
                description: "Write and edit files (write, edit), execute shell commands (shell), list a directory tree with line counts (tree), search file contents for a summarized, token-efficient view of matches (search), and run the project's build/test checks for a structured PASS/FAIL (verify)",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Your primary hands — read, write, and run things on the user's machine.",
                required_secrets: &[],
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
                    "Orchestrate work across agent sessions: list, view, start, message, and interrupt them (list_sessions, view_session, start_agent, send_message, interrupt_agent); inspect available workers (list_workers, check_worker); plan objectives and dispatch roadmap goals to worker agents, optionally narrowing extensions for an in-process dispatch while refusing that scope for CLI workers (decompose_roadmap, create_roadmap, goal_advance, goal_status, steer_goal, pause_roadmap, resume_roadmap); run a packaged executable skill by name (run_executable_skill); send a structured agent-to-agent message between InProgress goal workers (message_goal); and surface decisions in the Decision Inbox for supervised approval (escalate)",
                default_enabled: true,
                unprefixed_tools: false,
                hidden: false,
                why_it_matters:
                    "Run multi-agent work — dispatch goals, track roadmaps, and steer other sessions when one agent is not enough — escalating decisions to the user for approval rather than acting unsupervised.",
                required_secrets: &[],
                teaching: &[
                    crate::agents::self_knowledge::TeachingStep {
                        title: "Give me acceptance criteria",
                        body: "Tell the user that when they hand you a goal's acceptance \
                               criteria — 'the project builds', 'GET /health returns 200', \
                               'docs/guide.md exists', 'no TODO remains in src/lib.rs' — you \
                               compile the mechanically-checkable ones into checks the daemon \
                               runs in the goal's worktree before it can be approved. Ask for \
                               criteria in that measurable, verifiable shape.",
                        open_surface: None,
                        confirm: None,
                    },
                    crate::agents::self_knowledge::TeachingStep {
                        title: "Proof, not a claim",
                        body: "Make the point out loud: with acceptance criteria you verify a \
                               goal is actually done — the goal cannot pass review until its \
                               checks pass — rather than just relaying that a worker reported \
                               success. Offer to add a checkable criterion to a real goal so \
                               they see it enforced.",
                        open_surface: None,
                        confirm: None,
                    },
                ],
                client_factory: |ctx| Box::new(orchestrator::OrchestratorClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            tom::EXTENSION_NAME,
            PlatformExtensionDef {
                name: tom::EXTENSION_NAME,
                display_name: "Top Of Mind",
                description:
                    "Inject custom context into every turn via the PERMAGENT_MOIM_MESSAGE_TEXT and PERMAGENT_MOIM_MESSAGE_FILE environment variables",
                default_enabled: true,
                unprefixed_tools: false,
                hidden: false,
                why_it_matters:
                    "Inject standing context into every turn so persistent facts are never forgotten.",
                required_secrets: &[],
                teaching: &[],
                client_factory: |ctx| Box::new(tom::TomClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            skills::EXTENSION_NAME,
            PlatformExtensionDef {
                name: skills::EXTENSION_NAME,
                display_name: "Skills",
                description: "Discover skills stored as portable SKILL.md folders (the open agentskills.io standard) from the filesystem and builtins, and load one's full instructions into your context (load_skill)",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Pull in proven step-by-step procedures instead of improvising a workflow. Skills are portable SKILL.md folders compatible with Claude Code, Cursor, Codex, and the broader agent ecosystem, so learned capability moves in and out without lock-in.",
                required_secrets: &[],
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
                required_secrets: &[],
                teaching: &[],
                client_factory: |ctx| Box::new(librarian::LibrarianClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            app_conductor::EXTENSION_NAME,
            PlatformExtensionDef {
                name: app_conductor::EXTENSION_NAME,
                display_name: "App Conductor",
                description: "Act on the Permagent app: navigate the user to tabs and views (navigate_app), open/close/detach the chat dock or show/hide the Build tab's browser and terminal panes (app_action), carry them to a specific goal detail or project Grow planner (open_item), and put paste-ready text on their clipboard (copy_to_clipboard). These are action-only tools; read app state with observe_app",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Drive the app for the user — take them to the right view, operate it, open the specific goal or project view they mean, and copy paste-ready text onto their clipboard. Perception stays separate: use observe_app when you need to know what a surface contains.",
                required_secrets: &[],
                teaching: &[],
                client_factory: |ctx| {
                    Box::new(app_conductor::AppConductorClient::new(ctx).unwrap())
                },
            },
        );

        map.insert(
            app_perception::EXTENSION_NAME,
            PlatformExtensionDef {
                name: app_perception::EXTENSION_NAME,
                display_name: "App Perception",
                description:
                    "Read privacy-bounded aggregate state from the data behind the Permagent app \
                     without navigating or taking screenshots (observe_app): analytics, projects, \
                     goals/cards, spend, sessions, growth actions, agent briefings, the decision \
                     inbox, skills, scheduled automations, execution trace, brain, build, world, \
                     settings, and an overall home summary",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "The app is your home, not a UI you visit. Use observe_app to know what is \
                     happening from the same local data its surfaces render, while keeping raw \
                     rows and private join identifiers out of conversation history.",
                required_secrets: &[],
                teaching: &[],
                client_factory: |ctx| {
                    Box::new(app_perception::AppPerceptionClient::new(ctx).unwrap())
                },
            },
        );

        map.insert(
            recipe_author::EXTENSION_NAME,
            PlatformExtensionDef {
                name: recipe_author::EXTENSION_NAME,
                display_name: "Recipe Author",
                description:
                    "Create, list, run, pause, and delete scheduled automations (create_recipe, list_recipes, run_recipe, pause_recipe, delete_recipe) and save or list reusable skills (save_skill, list_skills) through chat. create_recipe supports richer authoring: input parameters, sub_recipes, retry with success checks, extensions, model settings, and a worker_persona",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Turn a repeatable task into a saved automation or schedule the user can rely on. \
                     Saved skills are written as portable SKILL.md folders (the open agentskills.io \
                     standard, shared with Claude Code, Cursor, and Codex); the ones that prove useful \
                     are promoted to the front of what you reach for, and ones that never fire retire \
                     themselves, so the skill library stays honest.",
                required_secrets: &[],
                teaching: &[],
                client_factory: |ctx| {
                    Box::new(recipe_author::RecipeAuthorClient::new(ctx).unwrap())
                },
            },
        );

        map.insert(
            model_manager::EXTENSION_NAME,
            PlatformExtensionDef {
                name: model_manager::EXTENSION_NAME,
                display_name: "Model Manager",
                description:
                    "See and steward the local inference models your sub-agents run: list what is installed — id, quantization, size on disk, source, vision support (list_models); and propose switching the active model to a better installed one, review-gated through the Decision Inbox (propose_model_upgrade)",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "So you can see which models your sub-agents run AND keep them current — proposing a switch to a better, more compact model as the market improves, always with the user's approval.",
                required_secrets: &[],
                teaching: &[],
                client_factory: |ctx| {
                    Box::new(model_manager::ModelManagerClient::new(ctx).unwrap())
                },
            },
        );

        map.insert(
            finance::EXTENSION_NAME,
            PlatformExtensionDef {
                name: finance::EXTENSION_NAME,
                display_name: "The Financier",
                description:
                    "Market research and the user's own finance tooling. research_ticker reads live prices, day and 52-week ranges and volume for any symbol with no key. company_fundamentals retrieves financial statements from financialdatasets.ai with an optional key; everything else works without it. If the user runs their own stock scanner, picker_status/picker_start/picker_scan/picker_top_picks drive it and record_trade/list_trades keep their trade history. Reports numbers; never sizes a position and cannot place an order",
                default_enabled: false,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Ground any claim about a price in a real, timestamped number instead of memory — and, for a user who runs their own picking algorithm, run it and keep the record their performance is measured against.",
                required_secrets: &[RequiredSecretDef {
                    key: crate::market_data::FUNDAMENTALS_KEY,
                    impact: SecretImpact::Degraded,
                    unlocks: "Retrieves company income, balance-sheet, and cash-flow fundamentals from financialdatasets.ai.",
                }],
                teaching: &[],
                client_factory: |ctx| Box::new(finance::FinanceClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            storage_health::EXTENSION_NAME,
            PlatformExtensionDef {
                name: storage_health::EXTENSION_NAME,
                display_name: "Storage Health",
                description:
                    "Scan the filesystem for storage cleanup opportunities (scan_storage_health) — dev caches, app caches, stale downloads, and large files",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Find and reclaim wasted disk before it becomes a problem.",
                required_secrets: &[],
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
                required_secrets: &[],
                teaching: &[],
                client_factory: |ctx| Box::new(steward::StewardClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            file_to_project::EXTENSION_NAME,
            PlatformExtensionDef {
                name: file_to_project::EXTENSION_NAME,
                display_name: "File to Project",
                description:
                    "File content the user is looking at — an email open in the embedded browser, or text they pasted — onto a project as a review-gated Decision Inbox proposal (file_to_project): on approval it becomes a project note indexed into the Brain, and any named people are added to the project address-less",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "When the user says \"file this email against <project>\" or \"save this to <project>\", this is the ONE consent path for persisting what they are viewing — nothing is written until they approve the proposal, and browser reads stay unpersisted otherwise. Never pass email addresses or phone numbers for the people it adds — names only.",
                required_secrets: &[],
                teaching: &[],
                client_factory: |ctx| {
                    Box::new(file_to_project::FileToProjectClient::new(ctx).unwrap())
                },
            },
        );

        map.insert(
            people::EXTENSION_NAME,
            PlatformExtensionDef {
                name: people::EXTENSION_NAME,
                display_name: "People",
                description:
                    "Create people and associate them with projects (create_person, associate_person_with_project) — minting a durable graph entity plus a CRM directory row in one deterministic step — enrich a person's professional details (enrich_person, propose_enrichment), and log a meeting against their profile (log_person_meeting) so it shows on the People tab, optionally on a project, with a dated follow-up, and writes through Apple Calendar onto the Home tab. Read the directory with observe_app surface people; open someone with navigate_app tab People and state { person: \"<display name>\" }",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "When the user says \"add <name>\" or \"associate <name> with <project>\", do it directly — you create and link people, you do not just remember them as a note. When they ask to enrich or refresh a contact's details, start with enrich_person — nothing is written until they approve the proposal. When they had a meeting with someone, call log_person_meeting rather than leaving it as a chat note; pass a follow-up date when they should check in. Open the People tab with navigate_app when they want to see someone.",
                required_secrets: &[],
                teaching: &[
                    crate::agents::self_knowledge::TeachingStep {
                        title: "Open the People tab",
                        body: "Call navigate_app with tab \"People\" so the user sees the \
                               directory graph. To open a specific person, pass state: \
                               { \"person\": \"<display name>\" } — the name observe_app \
                               surface people returned, never an email or UUID.",
                        open_surface: Some(crate::agents::self_knowledge::SurfaceRef {
                            tab: "People",
                            section: None,
                        }),
                        confirm: None,
                    },
                ],
                client_factory: |ctx| Box::new(people::PeopleClient::new(ctx).unwrap()),
            },
        );

        map.insert(
            project_manager::EXTENSION_NAME,
            PlatformExtensionDef {
                name: project_manager::EXTENSION_NAME,
                display_name: "Project Manager",
                description:
                    "Manage projects — named workspaces with paths, URLs, and metadata — including create, update, delete, list, and fuzzy-resolve (project_create, project_update, project_delete, project_list, project_resolve); run a project's Kanban board by creating, moving, updating, deleting, and listing cards (card_create, card_move, card_update, card_delete, card_list) and adding or removing columns (column_create, column_delete); read the user's to-do list exactly as the Home tab shows it, and give a card the due date that puts it there (card_due_list, plus a due-date argument on card_create and card_update); summarize the board across projects (board_summary); open a project-rooted terminal in the Build tab (project_launch); and research a project's ecosystem and competitive landscape, review-gated with cited findings, and remove stale intelligence items (research_project_intel, propose_project_intel, dismiss_project_intel); save GTM strategy pillars onto the Grow tab's Strategy lens (set_project_strategy); save this project's brand kit (set_project_brand); load this project's social brief before drafting a post (social_content_brief); regenerate a social post still from taste notes without rewriting the copy (retry_social_media); approve a ready draft (approve_social_post); connect Instagram, LinkedIn, or X to THIS project only (connect_project_channel); see this project's connected accounts (publisher_status); and disconnect one (disconnect_project_channel)",
                default_enabled: true,
                unprefixed_tools: true,
                hidden: false,
                why_it_matters:
                    "Keep the user's workspaces organized, and run the Grow post loop — brief, draft, still, taste-retry, connect this project's accounts, Approve — without dropping the copy.",
                required_secrets: &[],
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

/// What the capability loses when a declared secret is absent. There is no
/// third variant: a secret whose absence changes nothing is not required.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretImpact {
    /// The extension still works; some tools or fields are missing.
    Degraded,
    /// The extension cannot do its job at all without this.
    Unavailable,
}

/// One secret a capability needs, named by the env var / `Config` secret key
/// it is read from. The key NAME travels; the value never does.
#[derive(Debug, Clone, Copy)]
pub struct RequiredSecretDef {
    pub key: &'static str,
    pub impact: SecretImpact,
    /// One human sentence: what having this key unlocks.
    pub unlocks: &'static str,
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
    /// Secrets this extension reads. Non-`Option` and non-defaulted so a new
    /// registry entry that forgets to think about secrets is a COMPILE ERROR,
    /// exactly like `why_it_matters`. `&[]` is a positive claim — "this
    /// extension needs no secret" — and is NOT the same as the API's
    /// `not_declared`, which means no declaration exists at all.
    pub required_secrets: &'static [RequiredSecretDef],
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

#[cfg(test)]
mod required_secret_tests {
    use super::*;

    /// A declaration that says nothing is not a declaration. Hidden entries are
    /// checked too: hidden from the picker is not exempt from the inventory.
    #[test]
    fn every_secret_declaration_is_meaningful() {
        for extension in PLATFORM_EXTENSIONS.values() {
            let mut seen = std::collections::HashSet::new();
            for secret in extension.required_secrets {
                assert!(
                    !secret.key.trim().is_empty() && !secret.key.contains(char::is_whitespace),
                    "{} declares a secret key that is not a key name: {:?}",
                    extension.name,
                    secret.key
                );
                assert!(
                    !secret.unlocks.trim().is_empty(),
                    "{} declares {} with no sentence saying what it unlocks",
                    extension.name,
                    secret.key
                );
                assert!(
                    seen.insert(secret.key),
                    "{} declares {} twice",
                    extension.name,
                    secret.key
                );
            }
        }
    }

    #[test]
    fn financier_declares_its_optional_fundamentals_key() {
        // Omission on any registry entry is prevented structurally by the
        // non-defaulted `required_secrets` field on `PlatformExtensionDef`.
        let financier = PLATFORM_EXTENSIONS.get(finance::EXTENSION_NAME).unwrap();
        assert!(financier.required_secrets.iter().any(|secret| {
            secret.key == crate::market_data::FUNDAMENTALS_KEY
                && secret.impact == SecretImpact::Degraded
        }));
    }
}
