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
    /// For social_post cards only: RFC-3339 instant (UTC) when the post is scheduled.
    /// Rejected on any other card_type.
    scheduled_for: Option<String>,
    /// For social_post cards only: "draft" | "scheduled" | "posted".
    /// Defaults to "draft" when creating a social_post. Rejected on any other card_type.
    /// Create always stores draft — Approve in Grow is the path to scheduled.
    post_status: Option<String>,
    /// For social_post cards only: text | carousel | reel | compose.
    format: Option<String>,
    /// For social_post cards only: channel slug (ig, li, x, …).
    channel: Option<String>,
    /// For social_post cards only: blog | feature | origin | insight.
    harvest_kind: Option<String>,
    /// Due date as an ISO-8601 calendar date, `YYYY-MM-DD` (e.g. "2026-09-01").
    /// A standard card WITHOUT one never reaches the Home tab's to-do list — set
    /// it whenever the user gives or implies a deadline. Rejected on any
    /// card_type other than 'standard', and on any other date format.
    due_date: Option<String>,
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
struct CardUpdateParams {
    /// Card ID (UUID) to update
    card_id: String,
    /// New title (optional)
    title: Option<String>,
    /// New description / post body (optional)
    description: Option<String>,
    /// For social_post cards only: RFC-3339 instant (UTC) to (re)schedule.
    /// Rejected on any other card_type.
    scheduled_for: Option<String>,
    /// For social_post cards only: "draft" | "scheduled" | "posted".
    /// Rejected on any other card_type.
    post_status: Option<String>,
    /// New due date as an ISO-8601 calendar date, `YYYY-MM-DD`; pass `null` to
    /// clear it. Setting one puts the to-do on the Home tab's list; clearing it
    /// takes it off. Omit the field entirely to leave the due date alone.
    due_date: Option<Option<String>>,
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

/// `card_due_list` reads the whole cross-project list; there is nothing to
/// narrow, so it deliberately takes no arguments — the Home tab passes none
/// either, and a filter here would be a second opinion about scope.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct CardDueListParams {}

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
struct SetProjectStrategyParams {
    /// Project ID, slug, or exact name.
    project: String,
    /// Which GTM strategy pillar to save: audience, value, positioning,
    /// channels, content, or workback (the launch workback schedule).
    pillar: String,
    /// The strategy content for this pillar — a concise summary paragraph,
    /// user-editable, rendered on the Grow tab's Strategy lens.
    content: String,
    /// Labeled bullet points as [{label, detail}] — e.g. each channel with its
    /// fit reason, each persona with where they gather. Strings only.
    #[serde(default)]
    points: Option<serde_json::Value>,
    /// Small stat chips as [{label, value}] — e.g. {"label": "Alternatives",
    /// "value": "3"} or {"label": "Price hypothesis", "value": "$9/mo"}.
    #[serde(default)]
    metrics: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct SetProjectBrandParams {
    /// Project ID, slug, or exact name.
    project: String,
    /// How this project writes. Empty leaves the saved voice alone.
    voice: Option<String>,
    /// Why this project was built, in the founder's words. Empty leaves origin alone.
    origin: Option<String>,
    /// Background hex (#RRGGBB). Empty leaves the saved value alone.
    bg: Option<String>,
    /// Foreground hex (#RRGGBB).
    fg: Option<String>,
    /// Accent hex (#RRGGBB).
    accent: Option<String>,
    /// Optional typeface name for compose overlays.
    typeface: Option<String>,
    /// Things generated media must not do, e.g. "fake product UI".
    donts: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct SocialContentBriefParams {
    /// Project ID, slug, or exact name.
    project: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct RetrySocialMediaParams {
    /// Project ID, slug, or exact name.
    project: String,
    /// social_post card ID.
    card_id: String,
    /// Taste notes for the next still (darker, less type, show the product, …).
    /// Omit to reuse notes already on the card. Never rewrites title or body.
    feedback: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ApproveSocialPostParams {
    /// Project ID, slug, or exact name.
    project: String,
    /// social_post card ID. mediaStatus must already be ready.
    card_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct PublisherStatusParams {
    /// Project ID, slug, or exact name.
    project: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ConnectProjectChannelParams {
    /// Project ID, slug, or exact name.
    project: String,
    /// Network to connect for THIS project only: ig, li, or x.
    channel: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct DisconnectProjectChannelParams {
    /// Project ID, slug, or exact name.
    project: String,
    /// Network to disconnect from THIS project: ig, li, or x.
    channel: String,
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
             proposition, positioning, channels, content) you think through with the user; \
             set_project_brand for THIS project's voice, origin, and palette; a content \
             calendar of drafted posts you run as a loop — social_content_brief, card_create \
             as draft, a still generated from this project's kit, retry_social_media with \
             the user's taste notes when the graphic is off (title and body stay), \
             card_update only for copy, approve_social_post when they say Approve and \
             mediaStatus is ready (if this project has connected that channel, that \
             schedules the post on the connected account via Postiz; otherwise it stays \
             on the calendar — connect_project_channel first, per project, then \
             publisher_status to confirm; never copy \
             another project's Instagram onto this one) — \
             each post shows on its scheduled day as draft, scheduled, or posted; and a growth view with a live analytics lens that \
             shows the project's real visitor and traffic numbers — from the daemon's own \
             first-party collector, or from a provider the user already has (Plausible or \
             GoatCounter) — with any post or outreach you draft written in a crisp human voice, \
             never chatbot boilerplate",
        why_it_matters:
            "It is where the user takes a project to market with you. When they want to reach an \
             audience, plan a launch, or draft a post, bring them here, draft in their voice, \
             and manage the still yourself — if they dislike the graphic, take notes and \
             regenerate; never throw away the copy to get a new image",
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
                body: "Offer to draft a launch post for their project: social_content_brief \
                       first, then card_create as a draft. Show them it landed in the content \
                       calendar with a still generating.",
                open_surface: None,
                confirm: None,
            },
            crate::agents::self_knowledge::TeachingStep {
                title: "Connect this project's accounts",
                body: "Each project logs into its own Instagram, LinkedIn, or X. Call \
                       connect_project_channel for THIS project — a login window opens, they \
                       sign in, and that account binds only here. publisher_status to see \
                       what is connected. Then Approve actually schedules on that account.",
                open_surface: Some(crate::agents::self_knowledge::SurfaceRef {
                    tab: "Grow",
                    section: None,
                }),
                confirm: None,
            },
        ],
    };

/// Self-knowledge descriptor for the daemon's own web-analytics collector
/// (#23, `routes/first_party_analytics.rs` in the daemon; the connector lens in
/// `grow_analytics.rs` remains for people who already have a provider). Lives
/// beside GROW_FEATURE because the Grow tab's analytics lens is where it is
/// switched on and read; the descriptor is in the lib because the
/// self-knowledge registry is. Static surface: editorial, no live claim.
pub const FIRST_PARTY_ANALYTICS_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "first_party_analytics",
        display_name: "First-party analytics",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "The daemon is itself a web-analytics collector for the user's own sites, with no \
             third-party analytics vendor in the data path: per project the user opts in from \
             the Grow tab's analytics lens, the daemon mints a random site key and hands back a \
             snippet (plus a prompt for a coding agent) to drop into the site. Beacons land \
             either directly at the daemon's collect endpoint — which can only ever insert \
             events, is rate-limited, and accepts a fixed whitelist of fields — or, for public \
             sites whose visitors cannot reach a home machine, by relay-and-drain: the site \
             buffers events same-origin in its own database and the daemon pulls them outbound \
             on a timer, so daemon downtime loses nothing. Visitor uniques are \
             privacy-preserving — a daily-rotating hash, no IP address ever stored — and \
             pageviews, referrers, campaigns, funnels and custom events render in Grow",
        why_it_matters:
            "It is how a project gets real visitor numbers without handing a vendor the data. \
             When the user asks how their site is doing, or whether a launch moved anything, \
             this is where the numbers come from — read them with observe_app before \
             suggesting Google Analytics or Plausible or saying analytics are unavailable, and \
             if a project has no collector yet, offer to set this one up",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[],
    };

/// Self-knowledge descriptor for the Projects tab itself (#471). Each project
/// opens into a workspace with two lenses: an Overview dashboard (summary, key
/// facts, links, live task status) and the Kanban board. Static — always-on
/// surface, co-located with the project tools that back it.
/// Self-knowledge for the Devices pairing surface + the agent-driven tailnet
/// runbook (MULTI_DEVICE.md, zero-strain ruling 2026-07-11): Henry sets
/// the tailnet up himself with terminal commands; the user's only step is the
/// Tailscale login click, which Henry opens for them.
pub const DEVICES_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "devices",
        display_name: "Devices",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "Settings → Devices pairs the user's other devices to this machine (the hub): it \
             names the device and mints a pairing URL carrying a one-time claim code (single \
             use, ten-minute expiry) that the device exchanges for its own bearer token, \
             auto-fills the hub's Tailscale MagicDNS name when a tailnet is detected, and any \
             browser on the tailnet that opens the URL becomes a full Permagent client with a \
             token of its own. Paired devices are listed by name \
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

/// Self-knowledge for the native iOS companion (`ios/PermagentMobile`,
/// MULTI_DEVICE.md). There is no Rust module for the app, so the descriptor
/// sits beside DEVICES_FEATURE: the companion is the far end of the Devices
/// pairing flow, and keeping the two adjacent keeps them from contradicting
/// each other. Static surface — the hub cannot cheaply observe the phone.
pub const IOS_COMPANION_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "ios_companion",
        display_name: "iOS companion",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "A native iPhone app that is a pocket client of the hub — paired by pasting the \
             Settings → Devices URL. From it the user chats with you live (the phone and the \
             desktop are the same sessions on the same daemon), sees decisions pending and goals \
             in flight, and dictates a note that the hub transcribes with its own local Whisper \
             (no cloud speech-to-text) and files as a project note. Everything you do from the \
             phone acts on the hub and every connected screen renders it live; the phone keeps \
             only its pairing token in the Keychain and holds no user data",
        why_it_matters:
            "It is how the user reaches you away from the desk: an ask from the phone runs on \
             the Mac, so a request to open a site steers the desktop browser and a dispatched \
             goal moves the desktop board. Because it is remote hands rather than a second \
             brain, nothing syncs and a lost phone leaks one individually revocable device \
             token, zero data",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[],
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
             goal and to-do cards. Clicking any card on the board opens its detail view — \
             title, description, column, type, assignee, due date and timestamps — with the \
             title, description and due date editable in place. A to-do card carries an \
             optional due date, and a dated one ALSO appears on the Home tab's to-do list, \
             cross-project and soonest-first; an undated card stays on the board only. \
             A document dropped into a project is extracted and indexed \
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
             Kanban the working board. Read the user's to-dos with card_due_list — it returns \
             the Home tab's list itself, in the same order — and set or clear a card's due \
             date with card_create / card_update; a card with no due date reaches no to-do \
             list, which is why one you file undated goes unseen",
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
                  - 'social_post': a drafted social media post on the project's content calendar (Grow tab)

                When the user says "add a card", "create a task", "track this", or
                similar, use card_create with card_type='standard'. Ask which project
                if not clear from context — default to the active project if the user
                is currently in one, otherwise Personal.

                ## Content calendar

                social_post cards ARE the Grow tab's content calendar. Run this loop
                yourself — do not send the user to recreate a card:

                  1. social_content_brief for THIS project (brand, origin, top pages,
                     shipped features). Empty lists mean this project has no data yet.
                  2. card_create card_type='social_post'. Always leave post_status as
                     draft. Omit scheduled_for so the daemon picks a send time from this
                     project's occupancy and the user's local clock. Pass format
                     (text, carousel, reel, compose), channel (ig, li, …), and
                     harvest_kind (blog, feature, origin, insight) when you know them.
                  3. A still matching THIS post and THIS project's brand starts on
                     create. Tell the user it is generating. card_list
                     card_type='social_post' to read mediaStatus.
                  4. If the still is off-taste, take their notes and call
                     retry_social_media with feedback. That regenerates the still
                     only — title and description stay. Never card_delete +
                     card_create just to get a new graphic. card_update of title or
                     description is for copy edits; after a copy edit, call
                     retry_social_media so the still matches, unless they only wanted
                     the words changed.
                  5. When mediaStatus is ready AND the user says Approve (or "schedule
                     it"), call approve_social_post. Do not set post_status=scheduled
                     yourself any other way. If this project has connected that channel
                     (connect_project_channel — Instagram login for THIS project, not
                     another project's account), Approve schedules it on that account
                     via Postiz. If Postiz is not configured or the channel is not
                     connected, tell them to Connect Instagram (or LinkedIn) on Grow
                     for this project; do not claim it went live.

                  6. Accounts are per project. Another project's Instagram is not
                     reused here. publisher_status to see this project's bindings;
                     disconnect_project_channel to drop one.

                Do not reuse another project's voice or invent a brand that is not on
                this project. If the brand bag is empty, write in the humanize voice
                and say so; then offer set_project_brand.

                The copy you put in the description is what the user publishes, so
                write it in their voice, the way a sharp person actually writes. Lead
                with the point, stay concrete, keep sentences short, and cut every AI
                tell: no em-dashes, no "I'm excited to announce", no hype words like
                "seamless" or "unlock", no throat-clearing openers. Specifics over
                claims. Apply your "humanize" skill for the full voice spec.

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
    /// Persist one GTM strategy pillar for a project — lands in
    /// `metadata_json.strategy` and renders on the Grow tab's Strategy lens
    /// (#13 follow-up to the Ask-Henry cards). Merge-writes; emits
    /// `project_changed` so open Grow views refresh live.
    async fn handle_set_project_strategy(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let params: SetProjectStrategyParams =
            serde_json::from_value(serde_json::Value::Object(args))
                .map_err(|e| format!("Invalid arguments: {e}"))?;
        let content = params.content.trim();
        if content.is_empty() {
            return Err("content must not be empty".to_string());
        }
        let (project, pool) = self.resolve_intel_project(&params.project).await?;
        let pillar = params.pillar.trim().to_lowercase();
        let updated = crate::projects::set_project_strategy(
            &pool,
            &project.id,
            &pillar,
            content,
            crate::projects::StrategyExtras {
                points: params.points,
                metrics: params.metrics,
            },
        )
        .await?
        .ok_or_else(|| format!("Project {} disappeared mid-write", project.id))?;
        crate::events::emit(crate::events::project_changed(&updated.id, "updated"));
        Ok(vec![Content::text(format!(
            "Saved the {pillar} strategy for \"{}\" — it now shows on the Grow tab's Strategy lens and the user can edit it there.",
            updated.name
        ))])
    }

    async fn handle_set_project_brand(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let params: SetProjectBrandParams = serde_json::from_value(serde_json::Value::Object(args))
            .map_err(|e| format!("Invalid arguments: {e}"))?;
        let (project, pool) = self.resolve_intel_project(&params.project).await?;
        let updated = crate::projects::set_project_brand(
            &pool,
            &project.id,
            crate::projects::ProjectBrand {
                voice: params.voice.unwrap_or_default(),
                origin: params.origin.unwrap_or_default(),
                bg: params.bg.unwrap_or_default(),
                fg: params.fg.unwrap_or_default(),
                accent: params.accent.unwrap_or_default(),
                typeface: params.typeface.unwrap_or_default(),
                donts: params.donts.unwrap_or_default(),
                updated_at: None,
            },
        )
        .await?
        .ok_or_else(|| format!("Project {} disappeared mid-write", project.id))?;
        crate::events::emit(crate::events::project_changed(&updated.id, "updated"));
        Ok(vec![Content::text(format!(
            "Saved the brand kit for \"{}\" — voice, origin, and palette now apply to every social still on this project. Other projects are unchanged.",
            updated.name
        ))])
    }

    async fn handle_social_content_brief(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let params: SocialContentBriefParams =
            serde_json::from_value(serde_json::Value::Object(args))
                .map_err(|e| format!("Invalid arguments: {e}"))?;
        let (project, pool) = self.resolve_intel_project(&params.project).await?;
        let brief = crate::grow_media::content_brief(&pool, &project.id).await?;
        Ok(vec![Content::text(format!(
            "Content brief for \"{}\" (this project only):\n\n{}",
            brief.project_name,
            serde_json::to_string_pretty(&brief).unwrap_or_default()
        ))])
    }

    async fn handle_retry_social_media(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let params: RetrySocialMediaParams =
            serde_json::from_value(serde_json::Value::Object(args))
                .map_err(|e| format!("Invalid arguments: {e}"))?;
        let (project, pool) = self.resolve_intel_project(&params.project).await?;
        let before = cards::get_card(&pool, &params.card_id)
            .await?
            .ok_or_else(|| format!("Card '{}' not found", params.card_id))?;
        let card = crate::grow_media::retry_media(
            &pool,
            &project.id,
            &params.card_id,
            params.feedback.as_deref(),
        )
        .await?;
        Ok(vec![Content::text(format!(
            "Regenerating the still for \"{}\" on {}. Title and body were not changed (still \"{}\" / {}). mediaStatus is queued; tell the user when it is ready they can Approve.\n\n{}",
            card.title,
            project.name,
            before.title,
            before.description,
            serde_json::to_string_pretty(&serde_json::json!({
                "id": card.id,
                "title": card.title,
                "description": card.description,
                "media_status": card.metadata_json.get(cards::POST_MEDIA_STATUS_KEY),
                "media_feedback": card.metadata_json.get(cards::POST_MEDIA_FEEDBACK_KEY),
            })).unwrap_or_default()
        ))])
    }

    async fn handle_approve_social_post(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let params: ApproveSocialPostParams =
            serde_json::from_value(serde_json::Value::Object(args))
                .map_err(|e| format!("Invalid arguments: {e}"))?;
        let (project, pool) = self.resolve_intel_project(&params.project).await?;
        let card = crate::grow_media::approve_post(&pool, &project.id, &params.card_id).await?;
        let published = card
            .metadata_json
            .get(cards::POST_PUBLISHER_POST_ID_KEY)
            .and_then(|v| v.as_str());
        let outcome = if let Some(id) = published {
            format!(
                "Approved \"{}\" on {} — scheduled on this project's connected account via Postiz (publisher post {id}). Copy was not rewritten.",
                card.title, project.name
            )
        } else {
            format!(
                "Approved \"{}\" on {} — status is now scheduled on this project's calendar. Copy was not rewritten. Connect Instagram (or LinkedIn) for this project to send it to the network.",
                card.title, project.name
            )
        };
        Ok(vec![Content::text(format!(
            "{outcome}\n\n{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "id": card.id,
                "title": card.title,
                "post_status": card.metadata_json.get(cards::POST_STATUS_KEY),
                "scheduled_for": card.metadata_json.get(cards::POST_SCHEDULED_FOR_KEY),
                "publisher_post_id": card.metadata_json.get(cards::POST_PUBLISHER_POST_ID_KEY),
            }))
            .unwrap_or_default()
        ))])
    }

    async fn handle_publisher_status(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let params: PublisherStatusParams = serde_json::from_value(serde_json::Value::Object(args))
            .map_err(|e| format!("Invalid arguments: {e}"))?;
        let (project, pool) = self.resolve_intel_project(&params.project).await?;
        let snap = crate::grow_media::publisher_snapshot(&pool, &project.id).await?;
        Ok(vec![Content::text(format!(
            "Publisher status for \"{}\" (this project only):\n\n{}",
            project.name,
            serde_json::to_string_pretty(&snap).unwrap_or_default()
        ))])
    }

    async fn handle_connect_project_channel(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let params: ConnectProjectChannelParams =
            serde_json::from_value(serde_json::Value::Object(args))
                .map_err(|e| format!("Invalid arguments: {e}"))?;
        let (project, pool) = self.resolve_intel_project(&params.project).await?;
        let start = crate::grow_media::start_connect(&pool, &project.id, &params.channel).await?;
        Ok(vec![Content::text(format!(
            "Opened the {} login for \"{}\". That account will bind only to this project after they finish signing in. If a browser did not open, send them this URL:\n{}\n\nCall publisher_status until the channel shows as connected, then they can Approve.",
            start.label, project.name, start.url
        ))])
    }

    async fn handle_disconnect_project_channel(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let params: DisconnectProjectChannelParams =
            serde_json::from_value(serde_json::Value::Object(args))
                .map_err(|e| format!("Invalid arguments: {e}"))?;
        let (project, pool) = self.resolve_intel_project(&params.project).await?;
        let snap =
            crate::grow_media::disconnect_channel(&pool, &project.id, &params.channel).await?;
        Ok(vec![Content::text(format!(
            "Disconnected {} from \"{}\". This project will not post there until they connect again.\n\n{}",
            params.channel,
            project.name,
            serde_json::to_string_pretty(&snap).unwrap_or_default()
        ))])
    }

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
        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from);
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
        let scheduled_for = args.get("scheduled_for").and_then(|v| v.as_str());
        let post_status = args.get("post_status").and_then(|v| v.as_str());
        let due_date = args.get("due_date").and_then(|v| v.as_str());
        let mut metadata_json =
            social_post_metadata_for_create(card_type_str, scheduled_for, post_status)?;
        if card_type_str == "social_post" {
            metadata_json = Some(
                crate::grow_media::enrich_new_social_post(
                    &pool,
                    &project,
                    &title,
                    description.as_deref(),
                    metadata_json.unwrap_or_else(|| serde_json::json!({})),
                    args.get("format").and_then(|v| v.as_str()),
                    args.get("channel").and_then(|v| v.as_str()),
                    args.get("harvest_kind").and_then(|v| v.as_str()),
                )
                .await?,
            );
        }

        let card = create_card_with_due_date(
            &pool,
            cards::CreateCard {
                project_id: project.id.clone(),
                title,
                description,
                card_type: Some(card_type_str.to_string()),
                column_id,
                created_by: Some("user".to_string()),
                metadata_json,
            },
            due_date,
        )
        .await?;

        if card.card_type == "social_post" {
            crate::grow_media::enqueue_after_create(
                pool.clone(),
                project.id.clone(),
                card.id.clone(),
            );
        }

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
            match super::orchestrator::dispatch_goal_fn(&self.context, &probe_cache, &card.id, None)
                .await
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

        // Say plainly whether this card will reach the Home tab. An undated
        // standard card lands on the board and NOWHERE else, which is precisely
        // the "he added it to the Kanban and I don't see it as a to-do"
        // surprise — so the tool result names the omission instead of leaving
        // the agent to assume it worked.
        let stamped_due = card
            .metadata_json
            .get(cards::DUE_DATE_KEY)
            .and_then(|v| v.as_str());
        let json = serde_json::json!({
            "id": card.id, "title": card.title, "card_type": card.card_type,
            "column_id": card.column_id, "position": card.position,
            "project": project.name, "due_date": stamped_due,
            "scheduled_for": card.metadata_json.get(cards::POST_SCHEDULED_FOR_KEY),
            "media_status": card.metadata_json.get(cards::POST_MEDIA_STATUS_KEY),
        });
        let home_note = match (card.card_type.as_str(), stamped_due) {
            ("standard", Some(d)) => {
                format!(" — due {d}, so it shows on the Home tab's to-do list")
            }
            ("standard", None) => " — no due date, so it stays on the board and does NOT appear \
                 on the Home tab's to-do list; set due_date to put it there"
                .to_string(),
            ("social_post", _) => {
                let when = card
                    .metadata_json
                    .get(cards::POST_SCHEDULED_FOR_KEY)
                    .and_then(|v| v.as_str())
                    .unwrap_or("unscheduled");
                format!(
                    " — draft on this project's calendar at {when}. A still matching this post \
                     is generating; tell the user. If they dislike the graphic, retry_social_media \
                     with their taste notes — do not recreate the card. Approve with \
                     approve_social_post when mediaStatus is ready."
                )
            }
            _ => String::new(),
        };
        Ok(vec![Content::text(format!(
            "Created card \"{}\" in {} (column: {}, type: {}){}\n\n{}",
            card.title,
            project.name,
            card.column_id,
            card.card_type,
            home_note,
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

    async fn handle_card_update(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: card_id")?;
        let title = args.get("title").and_then(|v| v.as_str()).map(String::from);
        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from);
        let scheduled_for = args.get("scheduled_for").and_then(|v| v.as_str());
        let post_status = args.get("post_status").and_then(|v| v.as_str());
        // Three states, not two: absent leaves the due date alone, explicit
        // null clears it, a string sets it. `as_str()` alone would collapse
        // "clear it" into "don't touch it".
        let due_date = match args.get("due_date") {
            None => None,
            Some(serde_json::Value::Null) => Some(None),
            Some(serde_json::Value::String(d)) => Some(Some(d.as_str())),
            Some(other) => {
                return Err(format!(
                    "due_date must be an ISO-8601 calendar date string (YYYY-MM-DD) or null, \
                     got {other}"
                ))
            }
        };

        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())?;
        let card = cards::get_card(&pool, card_id)
            .await?
            .ok_or_else(|| format!("Card '{}' not found", card_id))?;

        let metadata_json = if scheduled_for.is_some() || post_status.is_some() {
            let merged = merge_social_post_metadata(
                &card.card_type,
                &card.metadata_json,
                scheduled_for,
                post_status,
            )?;
            if post_status == Some("scheduled") {
                let was = card
                    .metadata_json
                    .get(cards::POST_STATUS_KEY)
                    .and_then(|v| v.as_str());
                if was != Some("scheduled") {
                    cards::assert_ready_to_schedule(&merged)?;
                }
            }
            Some(merged)
        } else {
            None
        };

        if title.is_none() && description.is_none() && metadata_json.is_none() && due_date.is_none()
        {
            return Err(
                "card_update requires at least one of: title, description, due_date, \
                 scheduled_for, post_status"
                    .to_string(),
            );
        }

        // The due date is validated first so a malformed one rejects the WHOLE
        // edit: half-applying a title change and then failing would leave the
        // agent reporting an update it did not fully make.
        if let Some(Some(date)) = due_date {
            validated_due_date(&card.card_type, date)?;
        }

        let mut updated = if title.is_some() || description.is_some() || metadata_json.is_some() {
            cards::update_card(
                &pool,
                card_id,
                cards::UpdateCard {
                    title,
                    description,
                    metadata_json,
                    ..Default::default()
                },
            )
            .await?
            .ok_or("Card not found after update")?
        } else {
            card.clone()
        };
        // Written through the shared setter (which merges rather than replaces
        // metadata) AFTER the general update, so it reads the freshest card.
        if let Some(card) = apply_due_date_update(&pool, &updated, due_date).await? {
            updated = card;
        }

        let due_note = match due_date {
            Some(Some(d)) => format!(" — due {d}, now on the Home tab's to-do list"),
            Some(None) => " — due date cleared, so it leaves the Home tab's to-do list".to_string(),
            None => String::new(),
        };
        Ok(vec![Content::text(format!(
            "Updated card \"{}\" (id: {}){}",
            updated.title, updated.id, due_note
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

    /// Read the user's to-do list — the very list the Home tab renders.
    ///
    /// Takes no arguments and asks no question of its own: it calls
    /// [`due_todos`], which calls `cards::list_due_cards`. If this ever
    /// disagrees with what the user sees on Home, the query is wrong for both
    /// of them, not for one.
    async fn handle_card_due_list(
        &self,
        _arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())?;
        // The user's local day — "overdue" is a fact about their calendar, not
        // about UTC, and the Home tab buckets against the same local today.
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let items = due_todos(&pool, &today).await?;
        let overdue = items
            .iter()
            .filter(|v| v["overdue"].as_bool().unwrap_or(false))
            .count();

        if items.is_empty() {
            return Ok(vec![Content::text(
                "No to-dos are due — the Home tab's list is empty. A standard card only \
                 appears there once it has a due date (set one with card_create or \
                 card_update)."
                    .to_string(),
            )]);
        }
        Ok(vec![Content::text(format!(
            "{} to-do(s) on the Home tab, soonest first ({} overdue as of {})\n\n{}",
            items.len(),
            overdue,
            today,
            serde_json::to_string_pretty(&items).unwrap_or_default()
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
        let card_update_schema = serde_json::to_value(schema_for!(CardUpdateParams)).unwrap();
        let card_delete_schema = serde_json::to_value(schema_for!(CardDeleteParams)).unwrap();
        let card_list_schema = serde_json::to_value(schema_for!(CardListParams)).unwrap();
        let card_due_list_schema = serde_json::to_value(schema_for!(CardDueListParams)).unwrap();
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

                DUE DATES: pass due_date="YYYY-MM-DD" to make it a real to-do. A standard card
                with no due date lives on the board only — it does NOT appear on the Home tab's
                to-do list. If the user gives or implies a deadline ("by Friday", "next week"),
                resolve it to a calendar date and set due_date; if they clearly want it on their
                to-do list but named no date, ask for one rather than filing it undated.

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
                "card_update".to_string(),
                indoc! {r#"
                Edit a card's title, description, due date, or (for social_post) schedule and
                post_status. Use to rewrite or reschedule a drafted Grow-tab post without
                recreating it. Rewriting title/description does NOT regenerate the still —
                call retry_social_media after a copy change if they also want a matching
                graphic. Do not use post_status=scheduled; that is approve_social_post.

                DUE DATES: due_date="YYYY-MM-DD" sets or reschedules one, putting the to-do on
                the Home tab's list (and un-dismissing it); due_date=null clears it, taking it
                off. Omit the field to leave the due date untouched. This is how you fix a card
                the user expected to see on Home but doesn't.
            "#}
                .to_string(),
                card_update_schema.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Update Card".to_string()),
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
            Tool::new(
                "card_due_list".to_string(),
                indoc! {r#"
                Read the user's to-do list — EXACTLY what the Home tab shows, in the same
                order (soonest due first). Use it whenever they ask what's on their plate,
                what's due, what they should do next, or what their to-dos are.

                Returns each to-do with its title, project, board column, due date, and
                whether it is overdue. Only standard cards that HAVE a due date appear —
                a Kanban card without one is invisible here and on Home, so if the user
                expects something that is missing, give it a due date with card_update.
            "#}
                .to_string(),
                card_due_list_schema.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("List To-Dos".to_string()),
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
                "set_project_strategy".to_string(),
                indoc! {r#"
                Save one GTM strategy pillar for a project — audience, value,
                positioning, channels, content, or workback — onto the Grow tab's Strategy
                lens, where the user can read and edit it. Use this whenever you
                define or refine a project's go-to-market strategy (e.g. after a
                strategy card ask or a run-all strategy request): write
                each pillar you produced so the work persists instead of living
                only in chat. Keep content concise and ready to publish; saving a
                pillar overwrites its previous value. ALWAYS include the
                structured extras alongside the summary: points as
                [{label, detail}] (channels with fit reasons, personas with
                watering holes) and metrics as [{label, value}] stat chips —
                the Strategy cards render them as rich content.
            "#}
                .to_string(),
                serde_json::to_value(schema_for!(SetProjectStrategyParams))
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Save Project Strategy".to_string()),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            )),
            Tool::new(
                "set_project_brand".to_string(),
                indoc! {r#"
                Save THIS project's brand kit for Grow posts: voice, origin story
                (why it was built), palette (#RRGGBB bg/fg/accent), typeface, and
                donts. Merge-writes — omit a field to leave it. Use after the user
                describes how this project should look and sound, or when
                social_content_brief shows an empty brand. Never copy another
                project's kit onto this one.
            "#}
                .to_string(),
                serde_json::to_value(schema_for!(SetProjectBrandParams))
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Save Project Brand".to_string()),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            )),
            Tool::new(
                "social_content_brief".to_string(),
                indoc! {r#"
                Load what THIS project can be posted about: its brand (voice,
                origin, whether a palette is saved), top site pages from its own
                analytics, recently completed goals (shipped features), and the
                content strategy pillar. Call this before card_create of a
                social_post. Empty lists mean this project has no data yet — do
                not invent another project's stories or traffic.
            "#}
                .to_string(),
                serde_json::to_value(schema_for!(SocialContentBriefParams))
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Social Content Brief".to_string()),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            )),
            Tool::new(
                "retry_social_media".to_string(),
                indoc! {r#"
                Regenerate the still (and Reel video if this is a reel) for an
                existing social_post WITHOUT changing title or description.
                Pass the user's taste notes as feedback (darker, less type, show
                the product, more space). Use this when they dislike the graphic
                — never delete and recreate the card just to get a new still.
                After a copy-only card_update, call this so the still matches
                unless they asked to leave the graphic. mediaStatus returns to
                queued; wait until ready before approve_social_post.
            "#}
                .to_string(),
                serde_json::to_value(schema_for!(RetrySocialMediaParams))
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Retry Social Still".to_string()),
                Some(false),
                Some(true),
                Some(false),
                Some(false),
            )),
            Tool::new(
                "approve_social_post".to_string(),
                indoc! {r#"
                Move a draft social_post to scheduled. Only when the user explicitly
                asks to Approve or schedule it AND mediaStatus is ready. Does not
                rewrite copy. If this project has connected that channel, this
                schedules the post on the connected account via Postiz. If not,
                it stays on the calendar — tell them to connect_project_channel
                for THIS project first. Do not use card_update post_status=scheduled.
            "#}
                .to_string(),
                serde_json::to_value(schema_for!(ApproveSocialPostParams))
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Approve Social Post".to_string()),
                Some(false),
                Some(true),
                Some(false),
                Some(false),
            )),
            Tool::new(
                "publisher_status".to_string(),
                indoc! {r#"
                Show whether a Postiz API key is saved on this install, and which
                Instagram / LinkedIn / X accounts are connected to THIS project.
                Bindings do not carry over from other projects.
            "#}
                .to_string(),
                serde_json::to_value(schema_for!(PublisherStatusParams))
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Publisher Status".to_string()),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            )),
            Tool::new(
                "connect_project_channel".to_string(),
                indoc! {r#"
                Open a login window so THIS project can connect Instagram (ig),
                LinkedIn (li), or X (x). After they sign in, that account binds
                only to this project and Approve can schedule to it. Requires a
                Postiz API key saved in Grow. Never copy another project's
                connection onto this one.
            "#}
                .to_string(),
                serde_json::to_value(schema_for!(ConnectProjectChannelParams))
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Connect Project Channel".to_string()),
                Some(false),
                Some(true),
                Some(false),
                Some(false),
            )),
            Tool::new(
                "disconnect_project_channel".to_string(),
                indoc! {r#"
                Drop THIS project's binding to Instagram, LinkedIn, or X. Does not
                delete the Postiz account; it just stops this project posting there.
            "#}
                .to_string(),
                serde_json::to_value(schema_for!(DisconnectProjectChannelParams))
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Disconnect Project Channel".to_string()),
                Some(false),
                Some(true),
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
            "card_update" => self.handle_card_update(arguments).await,
            "card_delete" => self.handle_card_delete(arguments).await,
            "card_list" => self.handle_card_list(arguments).await,
            "card_due_list" => self.handle_card_due_list(arguments).await,
            "column_create" => self.handle_column_create(arguments).await,
            "column_delete" => self.handle_column_delete(arguments).await,
            "board_summary" => self.handle_board_summary(arguments).await,
            "research_project_intel" => self.handle_research_project_intel(arguments).await,
            "propose_project_intel" => self.handle_propose_project_intel(arguments).await,
            "dismiss_project_intel" => self.handle_dismiss_project_intel(arguments).await,
            "set_project_strategy" => self.handle_set_project_strategy(arguments).await,
            "set_project_brand" => self.handle_set_project_brand(arguments).await,
            "social_content_brief" => self.handle_social_content_brief(arguments).await,
            "retry_social_media" => self.handle_retry_social_media(arguments).await,
            "approve_social_post" => self.handle_approve_social_post(arguments).await,
            "publisher_status" => self.handle_publisher_status(arguments).await,
            "connect_project_channel" => self.handle_connect_project_channel(arguments).await,
            "disconnect_project_channel" => self.handle_disconnect_project_channel(arguments).await,
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

/// Build `metadata_json` for `card_create`. Post-schedule fields are
/// social_post-only — rejecting (not ignoring) them on other types keeps the
/// tool honest about what it accepts.
fn social_post_metadata_for_create(
    card_type: &str,
    scheduled_for: Option<&str>,
    post_status: Option<&str>,
) -> Result<Option<serde_json::Value>, String> {
    if (scheduled_for.is_some() || post_status.is_some()) && card_type != "social_post" {
        return Err(format!(
            "scheduled_for and post_status are only valid for card_type='social_post' \
             (got '{card_type}')"
        ));
    }
    if card_type != "social_post" {
        return Ok(None);
    }
    let status = post_status.unwrap_or("draft");
    cards::validate_post_metadata(scheduled_for, Some(status))?;
    let mut map = serde_json::Map::new();
    map.insert(
        cards::POST_STATUS_KEY.to_string(),
        serde_json::json!(status),
    );
    if let Some(when) = scheduled_for {
        map.insert(
            cards::POST_SCHEDULED_FOR_KEY.to_string(),
            serde_json::json!(when),
        );
    }
    Ok(Some(serde_json::Value::Object(map)))
}

/// Validate the agent's `due_date` argument for a card write.
///
/// Two ways to be wrong, both refused loudly rather than stored:
/// - a date that is not `YYYY-MM-DD` — [`cards::validate_due_date`] owns that
///   message, and it names the expected shape;
/// - a due date on a card the to-do list can never show. `list_due_cards`
///   filters to `card_type = 'standard'`, so stamping a date on a goal or a
///   social_post would write a field that changes nothing — exactly the silent
///   no-op the user hit from the other direction.
fn validated_due_date<'a>(card_type: &str, due_date: &'a str) -> Result<&'a str, String> {
    if card_type != "standard" {
        return Err(format!(
            "due_date is only valid for card_type='standard' — the Home tab's to-do list \
             shows standard cards only (got '{card_type}')"
        ));
    }
    cards::validate_due_date(due_date)?;
    Ok(due_date)
}

/// Create a card and, when the agent supplied one, stamp its due date.
///
/// The date is validated BEFORE the insert so a malformed one cannot leave an
/// orphan card behind, and it is written through [`cards::set_card_due_date`] —
/// the same function the UI's `PUT …/due-date` route calls — rather than by
/// poking `metadata_json`, so sibling metadata survives and the user's and the
/// agent's edits converge on one implementation.
pub(crate) async fn create_card_with_due_date(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    input: cards::CreateCard,
    due_date: Option<&str>,
) -> Result<cards::Card, String> {
    let card_type = input
        .card_type
        .clone()
        .unwrap_or_else(|| "standard".to_string());
    let due = due_date
        .map(|d| validated_due_date(&card_type, d))
        .transpose()?
        .map(str::to_string);

    let card = cards::create_card(pool, input).await?;
    match due {
        Some(date) => cards::set_card_due_date(pool, &card.id, Some(&date))
            .await?
            .ok_or_else(|| format!("Card '{}' vanished before its due date landed", card.id)),
        None => Ok(card),
    }
}

/// Apply the agent's due-date edit to a card that already exists.
///
/// `None` leaves the due date alone, `Some(None)` clears it, `Some(Some(d))`
/// sets it. Clearing is allowed on any card type: removing a key that the
/// to-do list ignores anyway cannot mislead anyone, while *setting* one there
/// would promise a Home-tab appearance that never comes.
pub(crate) async fn apply_due_date_update(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    card: &cards::Card,
    due_date: Option<Option<&str>>,
) -> Result<Option<cards::Card>, String> {
    let Some(value) = due_date else {
        return Ok(None);
    };
    if let Some(date) = value {
        validated_due_date(&card.card_type, date)?;
    }
    cards::set_card_due_date(pool, &card.id, value)
        .await?
        .ok_or_else(|| format!("Card '{}' not found", card.id))
        .map(Some)
}

/// Render one to-do the way the agent needs to talk about it.
///
/// `overdue` is a plain string comparison, which is exactly right for
/// `YYYY-MM-DD`: the shape is guaranteed by [`cards::validate_due_date`] at
/// every write, and lexical order on a fixed-width ISO date IS chronological
/// order — no date library, no timezone to get wrong beyond `today` itself.
fn due_todo_json(items: &[cards::DueCard], today: &str) -> Vec<serde_json::Value> {
    items
        .iter()
        .map(|c| {
            serde_json::json!({
                "id": c.id,
                "title": c.title,
                "project": c.project_name,
                "project_id": c.project_id,
                "column": c.column_name,
                "due_date": c.due_date,
                "overdue": c.due_date.as_str() < today,
                "assigned_to": c.assigned_to,
            })
        })
        .collect()
}

/// The user's to-do list, read from the one query the Home tab reads.
///
/// It deliberately calls [`cards::list_due_cards`] rather than asking its own
/// question: scope (standard cards only, dated, unarchived, not dismissed, not
/// in a terminal column) and ordering (soonest first) live in exactly one
/// place, so the agent and the Home tab cannot drift into disagreeing about
/// what the user's to-dos are.
pub(crate) async fn due_todos(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    today: &str,
) -> Result<Vec<serde_json::Value>, String> {
    let items = cards::list_due_cards(pool).await?;
    Ok(due_todo_json(&items, today))
}

/// Merge post-schedule keys into existing metadata without clobbering siblings.
fn merge_social_post_metadata(
    card_type: &str,
    existing: &serde_json::Value,
    scheduled_for: Option<&str>,
    post_status: Option<&str>,
) -> Result<serde_json::Value, String> {
    if card_type != "social_post" {
        return Err(format!(
            "scheduled_for and post_status are only valid for card_type='social_post' \
             (got '{card_type}')"
        ));
    }
    cards::validate_post_metadata(scheduled_for, post_status)?;
    let mut map = existing.as_object().cloned().unwrap_or_default();
    if let Some(when) = scheduled_for {
        map.insert(
            cards::POST_SCHEDULED_FOR_KEY.to_string(),
            serde_json::json!(when),
        );
    }
    if let Some(status) = post_status {
        map.insert(
            cards::POST_STATUS_KEY.to_string(),
            serde_json::json!(status),
        );
    }
    Ok(serde_json::Value::Object(map))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_create_rejects_schedule_on_non_social_post() {
        let err = social_post_metadata_for_create("standard", Some("2026-08-15T18:00:00Z"), None)
            .unwrap_err();
        assert!(
            err.contains("social_post") && err.contains("standard"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn card_create_defaults_social_post_status_to_draft() {
        let meta = social_post_metadata_for_create("social_post", None, None)
            .unwrap()
            .unwrap();
        assert_eq!(meta[cards::POST_STATUS_KEY], "draft");
        assert!(meta.get(cards::POST_SCHEDULED_FOR_KEY).is_none());
    }

    #[test]
    fn card_update_merges_metadata_without_clobbering_unrelated_keys() {
        let existing = serde_json::json!({
            "channel": "x",
            cards::POST_STATUS_KEY: "draft",
        });
        let merged = merge_social_post_metadata(
            "social_post",
            &existing,
            Some("2026-08-20T12:00:00Z"),
            Some("scheduled"),
        )
        .unwrap();
        assert_eq!(merged["channel"], "x");
        assert_eq!(
            merged[cards::POST_SCHEDULED_FOR_KEY],
            "2026-08-20T12:00:00Z"
        );
        assert_eq!(merged[cards::POST_STATUS_KEY], "scheduled");
    }

    #[test]
    fn card_update_rejects_schedule_on_non_social_post() {
        let err = merge_social_post_metadata(
            "goal",
            &serde_json::json!({}),
            Some("2026-08-15T18:00:00Z"),
            None,
        )
        .unwrap_err();
        assert!(
            err.contains("social_post") && err.contains("goal"),
            "unexpected: {err}"
        );
    }

    // ── Due dates on the agent's card path ─────────────────────────────────
    //
    // Reported symptoms, all one defect: the orchestrator could not see the
    // user's to-dos, and a card it filed on the Kanban never showed up on the
    // Home tab. The Home tab lists `cards::list_due_cards`, which requires a
    // `dueDate` — and until now no agent tool could write one or read the list.

    use sqlx::{Pool, Sqlite};

    async fn test_pool() -> Pool<Sqlite> {
        use crate::session::spectral_schema::init_spectral_db;
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    fn new_todo(title: &str) -> cards::CreateCard {
        cards::CreateCard {
            project_id: crate::projects::PERSONAL_PROJECT_ID.to_string(),
            title: title.to_string(),
            description: None,
            card_type: Some("standard".to_string()),
            column_id: None,
            created_by: Some("user".to_string()),
            metadata_json: None,
        }
    }

    /// The user's exact report: "he added a new item to the Kanban as a backlog
    /// item and i dont see it as a to do on the Home tab." A card the agent
    /// files WITH a due date reaches the Home list; the identical card without
    /// one does not — and now the difference is a parameter the agent controls.
    #[tokio::test]
    async fn agent_created_card_reaches_the_home_todo_list_only_when_it_has_a_due_date() {
        let pool = test_pool().await;

        let undated = create_card_with_due_date(&pool, new_todo("undated backlog item"), None)
            .await
            .unwrap();
        assert!(
            cards::list_due_cards(&pool).await.unwrap().is_empty(),
            "an undated card must not reach the Home tab's to-do list"
        );

        let dated =
            create_card_with_due_date(&pool, new_todo("dated backlog item"), Some("2026-09-01"))
                .await
                .unwrap();

        let due = cards::list_due_cards(&pool).await.unwrap();
        assert_eq!(due.len(), 1, "exactly the dated card should be listed");
        assert_eq!(due[0].id, dated.id);
        assert_eq!(due[0].due_date, "2026-09-01");
        assert_ne!(due[0].id, undated.id);
    }

    /// `card_due_list` must be the Home tab's list, not a second opinion about
    /// it — so this asserts against `cards::list_due_cards` itself rather than
    /// a hand-written expectation that could drift from the real query.
    #[tokio::test]
    async fn card_due_list_returns_exactly_what_the_home_tab_query_returns() {
        let pool = test_pool().await;
        create_card_with_due_date(&pool, new_todo("later"), Some("2026-09-01"))
            .await
            .unwrap();
        create_card_with_due_date(&pool, new_todo("overdue"), Some("2026-07-01"))
            .await
            .unwrap();
        create_card_with_due_date(&pool, new_todo("soon"), Some("2026-08-20"))
            .await
            .unwrap();
        // Excluded by the shared query, so it must be absent from both sides.
        create_card_with_due_date(&pool, new_todo("undated"), None)
            .await
            .unwrap();

        let expected = cards::list_due_cards(&pool).await.unwrap();
        let rendered = due_todos(&pool, "2026-08-19").await.unwrap();

        assert_eq!(rendered.len(), expected.len());
        for (row, card) in rendered.iter().zip(expected.iter()) {
            assert_eq!(row["id"], card.id);
            assert_eq!(row["title"], card.title);
            assert_eq!(row["project"], card.project_name);
            assert_eq!(row["project_id"], card.project_id);
            assert_eq!(row["column"], card.column_name);
            assert_eq!(row["due_date"], card.due_date);
        }
        // Same order as the shared query: soonest first, overdue naturally on top.
        let titles: Vec<&str> = rendered
            .iter()
            .map(|r| r["title"].as_str().unwrap())
            .collect();
        assert_eq!(titles, vec!["overdue", "soon", "later"]);
        assert_eq!(rendered[0]["overdue"], true);
        assert_eq!(rendered[1]["overdue"], false);
        assert_eq!(rendered[2]["overdue"], false);
    }

    /// Mirrors `cards::setting_a_due_date_preserves_other_metadata`: the agent
    /// writes through the shared setter, so a due date must not flatten the
    /// sibling keys the card was carrying.
    #[tokio::test]
    async fn agent_due_date_update_preserves_unrelated_metadata() {
        let pool = test_pool().await;
        let mut input = new_todo("has meta");
        input.metadata_json = Some(serde_json::json!({ "colour": "blue" }));
        let card = create_card_with_due_date(&pool, input, None).await.unwrap();

        let updated = apply_due_date_update(&pool, &card, Some(Some("2026-08-05")))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.metadata_json["colour"], "blue");
        assert_eq!(updated.metadata_json[cards::DUE_DATE_KEY], "2026-08-05");

        // Clearing likewise leaves the siblings alone.
        let cleared = apply_due_date_update(&pool, &updated, Some(None))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cleared.metadata_json["colour"], "blue");
        assert!(cleared.metadata_json.get(cards::DUE_DATE_KEY).is_none());
        assert!(cards::list_due_cards(&pool).await.unwrap().is_empty());
    }

    /// A date the agent guessed at ("next tuesday", "01/09/2026") is refused
    /// with a message that NAMES the shape it wants, so the retry is informed.
    #[tokio::test]
    async fn a_malformed_due_date_is_rejected_naming_the_expected_format() {
        let pool = test_pool().await;

        for bad in ["next tuesday", "01/09/2026", "2026-9-1", ""] {
            let err = create_card_with_due_date(&pool, new_todo("bad"), Some(bad))
                .await
                .unwrap_err();
            assert!(
                err.contains("YYYY-MM-DD") && err.contains(bad),
                "expected '{bad}' refused by format, got: {err}"
            );
        }
        // Correctly shaped but not a day that exists: a different, equally
        // specific message — the shape is not the complaint here.
        let err = create_card_with_due_date(&pool, new_todo("bad"), Some("2026-02-30"))
            .await
            .unwrap_err();
        assert!(
            err.contains("2026-02-30") && err.contains("calendar date"),
            "unexpected: {err}"
        );
        // Rejected BEFORE the insert — no orphan card is left behind.
        assert_eq!(
            cards::list_cards(&pool, crate::projects::PERSONAL_PROJECT_ID, None, None)
                .await
                .unwrap()
                .len(),
            0,
            "a refused due date must not leave a card behind"
        );

        let card = create_card_with_due_date(&pool, new_todo("good"), None)
            .await
            .unwrap();
        let err = apply_due_date_update(&pool, &card, Some(Some("someday")))
            .await
            .unwrap_err();
        assert!(err.contains("YYYY-MM-DD"), "unexpected: {err}");
    }

    /// A due date on a goal or a social_post would be written and then ignored
    /// by `list_due_cards` — the same silent no-op from the other direction.
    /// Refuse it, and name the type that was passed.
    #[tokio::test]
    async fn a_due_date_on_a_non_standard_card_is_refused() {
        let pool = test_pool().await;
        let mut goal = new_todo("a goal");
        goal.card_type = Some("goal".to_string());
        let err = create_card_with_due_date(&pool, goal, Some("2026-09-01"))
            .await
            .unwrap_err();
        assert!(
            err.contains("standard") && err.contains("goal"),
            "unexpected: {err}"
        );
    }
}
