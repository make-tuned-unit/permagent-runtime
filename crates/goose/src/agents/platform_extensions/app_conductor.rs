//! App Conductor — lets the chat agent drive the app's UI for the user.
//!
//! Exposes four tools: `navigate_app` (go to a tab, optionally a sub-view),
//! `app_action` (act within a surface — open/close/detach the chat dock,
//! show/hide the Build tab's browser and terminal panes), `open_item`
//! (carry the user the last mile to a specific item — a goal's detail or a
//! project's Grow planner), and `copy_to_clipboard` (put paste-ready text on
//! the user's local clipboard). Each validates its input, emits the matching
//! event on the global bus (or intercepts it on a voice turn), and returns a
//! confirmation to the agent.

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use crate::app_catalog::get_global_catalog;
use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "app_conductor";

// ── Tool parameter schema ───────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct NavigateAppParams {
    /// User-facing tab name from the catalog (e.g. "Brain", "Settings", "Build").
    tab: String,
    /// Optional sub-section within the tab (e.g. "skills" within Automate).
    #[serde(default)]
    section: Option<String>,
    /// Optional opaque JSON state for the receiving view to interpret.
    #[serde(default)]
    state: Option<serde_json::Value>,
    /// Human-readable explanation shown in chat (e.g. "Taking you to the Brain view
    /// so you can see what I remember about X").
    reason: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct AppActionParams {
    /// The surface to act on: "chat" or "build".
    surface: String,
    /// The action to perform within the surface. Valid pairs:
    /// chat → open | close | detach; build → show_browser | hide_browser |
    /// show_terminal | hide_terminal.
    action: String,
    /// Optional opaque JSON params for the action (reserved; unused today).
    #[serde(default)]
    params: Option<serde_json::Value>,
    /// Human-readable explanation shown in chat (e.g. "Hiding the terminal so
    /// the browser fills the Build tab").
    reason: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct OpenItemParams {
    /// The kind of item to open. Valid: "goal" (a goal card's detail modal) or
    /// "grow" (a project's Grow planner).
    kind: String,
    /// The project the item belongs to — REQUIRED for every kind. Get it from
    /// project_resolve (or project_list).
    project_id: String,
    /// The goal card's id — REQUIRED when kind is "goal", ignored otherwise. Get
    /// it from board_summary or card_list (a card with card_type "goal").
    #[serde(default)]
    card_id: Option<String>,
    /// Human-readable explanation shown in chat (e.g. "Opening the goal so you
    /// can see its checklist and evidence").
    reason: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct CopyToClipboardParams {
    /// The exact paste-ready body. Plain text only — no markdown fences, no
    /// surrounding commentary. This is what lands on the clipboard.
    text: String,
    /// Human-readable explanation shown in chat (e.g. "Copying the caption
    /// so you can paste it into Notes").
    reason: String,
}

/// Hard cap so a runaway dump cannot blow a WebSocket frame or the pasteboard.
const MAX_CLIPBOARD_CHARS: usize = 32_768;

/// Trim, reject empty, reject oversized. Shared by the tool and its tests.
fn clipboard_body(raw: &str) -> std::result::Result<String, String> {
    let text = raw.trim().to_string();
    if text.is_empty() {
        return Err(
            "text is empty — pass the paste-ready body the user asked to copy.".to_string(),
        );
    }
    if text.chars().count() > MAX_CLIPBOARD_CHARS {
        return Err(format!(
            "text is too long (max {MAX_CLIPBOARD_CHARS} characters). Shorten it and call again."
        ));
    }
    Ok(text)
}

/// The catalog of surface → actions the agent may drive. This is the single
/// source of truth the tool validates against (mirrors the tab catalog for
/// `navigate_app`), so an unknown pair is rejected with a helpful list rather
/// than emitting an event the frontend can't handle. The frontend dispatcher
/// mirrors these exact strings.
const ACTION_CATALOG: &[(&str, &[&str])] = &[
    ("chat", &["open", "close", "detach"]),
    (
        "build",
        &[
            "show_browser",
            "hide_browser",
            "show_terminal",
            "hide_terminal",
        ],
    ),
];

fn catalog_lists() -> String {
    ACTION_CATALOG
        .iter()
        .map(|(surface, actions)| format!("{} → {}", surface, actions.join(" | ")))
        .collect::<Vec<_>>()
        .join("; ")
}

fn action_is_valid(surface: &str, action: &str) -> bool {
    ACTION_CATALOG
        .iter()
        .any(|(s, actions)| *s == surface && actions.contains(&action))
}

/// The catalog of item KINDS the agent may open by id — the last-mile seam that
/// carries the user past a tab to a specific thing. Single source of truth the
/// `open_item` tool validates against (mirrors `ACTION_CATALOG` for `app_action`),
/// so an unknown kind is rejected with a helpful list rather than emitting an
/// event the frontend can't handle. The frontend `dispatchOpenItem` maps these
/// exact kinds 1:1 to the store seams that already back the human buttons.
///
/// Each entry is `(kind, id-fields hint)`. Every kind needs `project_id`; the
/// hint documents any second id. Only kinds whose ids the agent can RELIABLY
/// obtain from its own tool-results live here — goal cards come from
/// `board_summary`/`card_list` and the project id from `project_resolve`, so both
/// are reachable. (Decision/person/memory seams exist but are intentionally
/// absent: the agent has no reliable way to reference their ids today — see the
/// tool description.)
const ITEM_CATALOG: &[(&str, &str)] = &[("goal", "project_id + card_id"), ("grow", "project_id")];

fn item_kinds_list() -> String {
    ITEM_CATALOG
        .iter()
        .map(|(kind, ids)| format!("{} (needs {})", kind, ids))
        .collect::<Vec<_>>()
        .join("; ")
}

fn item_is_valid(kind: &str) -> bool {
    ITEM_CATALOG.iter().any(|(k, _)| *k == kind)
}

/// The `open_item` success text — honest about the fire-and-forget contract.
/// The tool validates only the kind and id PRESENCE, then emits to the app
/// bus: it cannot confirm an app window is connected or that the ids are
/// current, so the text must not claim the item is open. (The old "Opening
/// {kind}." read as confirmed success to the agent while a stale id showed
/// the user an error modal — or, with no app window, nothing at all.)
fn open_item_result_text(
    kind: &str,
    project_id: &str,
    card_id: Option<&str>,
    reason: &str,
) -> String {
    let ids = match card_id {
        Some(card) if !card.is_empty() => format!("{} (card {})", project_id, card),
        _ => project_id.to_string(),
    };
    format!(
        "Sent the open request for {} {} to the app. If the app isn't open or \
         the id is stale, nothing will change on screen — project_resolve / \
         board_summary / card_list give current ids. {}",
        kind, ids, reason
    )
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

pub struct AppConductorClient {
    info: InitializeResult,
    #[allow(dead_code)]
    context: PlatformExtensionContext,
}

impl AppConductorClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME, "1.0.0").with_title("App Conductor"),
            )
            .with_instructions(
                "Drive the Permagent app for the user. Use navigate_app to take \
                 them to a specific tab or view. Use app_action to operate a \
                 surface once there — open/close/detach the chat dock, or \
                 show/hide the Build tab's browser and terminal panes. Use \
                 open_item to carry them the LAST MILE past a tab to a specific \
                 thing — a goal's detail (open_item kind \"goal\" with its \
                 project_id + card_id) or a project's Grow planner (open_item \
                 kind \"grow\" with its project_id). Resolve the ids first with \
                 your other tools (project_resolve for the project id, \
                 board_summary / card_list for a goal card id), then open \
                 the item so the user lands ON it rather than hunting for it. \
                 Use copy_to_clipboard when they ask for paste-ready text — a \
                 post, caption, speech, blurb, or \"give me the text\" / \
                 \"copy that\" so they can paste into Notes or another app. \
                 Prefer doing it over describing it: these tools are the only way \
                 to actually change what the user sees or what is on their clipboard.",
            );
        Ok(Self { info, context })
    }

    async fn handle_navigate(
        &self,
        session_id: &str,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args: NavigateAppParams = arguments
            .map(|obj| serde_json::from_value(serde_json::Value::Object(obj)))
            .transpose()
            .map_err(|e| format!("Invalid arguments: {}", e))?
            .ok_or_else(|| "Missing arguments".to_string())?;

        let catalog =
            get_global_catalog().ok_or_else(|| "App catalog not initialized".to_string())?;

        let entry = catalog.find_by_name(&args.tab).ok_or_else(|| {
            format!(
                "I don't recognize the tab \"{}\". Available tabs are: {}",
                args.tab,
                catalog.tab_names().join(", ")
            )
        })?;

        // The agent's explicit section wins; otherwise fall back to the
        // entry's own fixed section (Console-consolidation entries like
        // Sessions/Trace/Inbox are Settings sections and carry it in the
        // catalog so `navigate_app("Sessions")` lands on the right pane).
        let section = args.section.clone().or_else(|| entry.section.clone());

        // Speak-then-act seam: if a transport is intercepting navigations for
        // this session (a voice turn), hand the intent off to it instead of
        // emitting to the global bus — it will sequence the nav after the
        // narration has finished playing. Otherwise (text chat) emit as normal,
        // for instant navigation. `capture` is atomic with this decision.
        let intent = crate::events::nav_intercept::NavIntent {
            tab: entry.name.clone(),
            tool_type: entry.tool_type.clone(),
            panel_type: entry.panel_type.clone(),
            section: section.clone(),
            state: args.state.clone(),
            reason: args.reason.clone(),
        };
        if !crate::events::nav_intercept::capture(session_id, intent) {
            crate::events::emit(crate::events::app_navigate(
                &entry.name,
                &entry.tool_type,
                &entry.panel_type,
                section.as_deref(),
                args.state.as_ref(),
                &args.reason,
            ));
        }

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Navigating to {}. {}",
            entry.name, args.reason
        ))]))
    }

    async fn handle_action(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args: AppActionParams = arguments
            .map(|obj| serde_json::from_value(serde_json::Value::Object(obj)))
            .transpose()
            .map_err(|e| format!("Invalid arguments: {}", e))?
            .ok_or_else(|| "Missing arguments".to_string())?;

        if !action_is_valid(&args.surface, &args.action) {
            return Err(format!(
                "I can't do \"{}\" on \"{}\". Valid actions are: {}",
                args.action,
                args.surface,
                catalog_lists()
            ));
        }

        // Emit to the global bus; the frontend dispatcher calls the matching
        // store action. Unlike navigate_app there's no voice speak-then-act
        // intercept — an in-surface toggle is instantaneous and doesn't compete
        // with narration.
        crate::events::emit(crate::events::app_action(
            &args.surface,
            &args.action,
            args.params.as_ref(),
            &args.reason,
        ));

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Done: {} on {}. {}",
            args.action, args.surface, args.reason
        ))]))
    }

    async fn handle_open_item(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args: OpenItemParams = arguments
            .map(|obj| serde_json::from_value(serde_json::Value::Object(obj)))
            .transpose()
            .map_err(|e| format!("Invalid arguments: {}", e))?
            .ok_or_else(|| "Missing arguments".to_string())?;

        if !item_is_valid(&args.kind) {
            return Err(format!(
                "I can't open a \"{}\". Openable items are: {}",
                args.kind,
                item_kinds_list()
            ));
        }

        // Goal is the one kind that needs a second id; reject early with a
        // pointer to where the agent gets it rather than emitting an event the
        // goal-detail modal can't act on.
        if args.kind == "goal" && args.card_id.as_deref().unwrap_or("").is_empty() {
            return Err(
                "Opening a goal needs its card_id (a card_type \"goal\" card from \
                 board_summary / card_list) alongside the project_id."
                    .to_string(),
            );
        }

        // Emit to the global bus; the frontend dispatcher calls the matching
        // store seam (the same one the human button uses). Like app_action there
        // is no voice speak-then-act intercept — opening an item is instantaneous
        // and doesn't compete with narration.
        crate::events::emit(crate::events::app_open_item(
            &args.kind,
            &args.project_id,
            args.card_id.as_deref(),
            &args.reason,
        ));

        Ok(CallToolResult::success(vec![Content::text(
            open_item_result_text(
                &args.kind,
                &args.project_id,
                args.card_id.as_deref(),
                &args.reason,
            ),
        )]))
    }

    async fn handle_copy(
        &self,
        session_id: &str,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args: CopyToClipboardParams = arguments
            .map(|obj| serde_json::from_value(serde_json::Value::Object(obj)))
            .transpose()
            .map_err(|e| format!("Invalid arguments: {}", e))?
            .ok_or_else(|| "Missing arguments".to_string())?;

        let text = clipboard_body(&args.text)?;
        let intent = crate::events::clipboard_intercept::ClipboardIntent {
            text: text.clone(),
            reason: args.reason.clone(),
        };
        // Voice turn: the /voice socket copies on the listening device after
        // narration. Text chat: emit to the bus so the focused window copies.
        if !crate::events::clipboard_intercept::capture(session_id, intent) {
            crate::events::emit(crate::events::app_clipboard(&text, &args.reason));
        }

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Sent the text to the user's clipboard ({} characters). Confirm briefly \
             — do not read the body back. {}",
            text.chars().count(),
            args.reason
        ))]))
    }
}

impl AppConductorClient {
    /// The full, static tool inventory. Extracted from `list_tools` so the
    /// self-knowledge completeness guard derives its inventory from the REAL
    /// list — add a tool here and CI fails until the registry `description`
    /// names it.
    pub(crate) fn get_tools() -> Vec<Tool> {
        vec![
            Tool::new(
                "navigate_app".to_string(),
                "Navigate the user to a specific tab, optionally drilling into a \
                 sub-view. Call this whenever the user expresses intent to view, open, \
                 visit, or be taken to a tab. This is the ONLY way to actually change \
                 what the user sees — describing navigation in text does nothing.\n\n\
                 To open a specific project's detail/kanban view, navigate to the \
                 \"Projects\" tab with state: { \"project_id\": \"<uuid>\" }. Resolve \
                 the project name first with project_resolve to get the ID."
                    .to_string(),
                schema::<NavigateAppParams>(),
            ),
            Tool::new(
                "app_action".to_string(),
                "Act WITHIN a surface — not just navigate to it. Use this to \
                 operate the app on the user's behalf: open, close, or detach the \
                 chat dock, and show/hide the Build tab's browser or terminal \
                 pane. Like navigate_app, describing the action in text does \
                 nothing — you must call this to actually change the UI.\n\n\
                 Valid surface → action pairs: chat → open | close | detach; \
                 build → show_browser | hide_browser | show_terminal | \
                 hide_terminal. (build actions switch to the Build tab first if \
                 needed.)"
                    .to_string(),
                schema::<AppActionParams>(),
            ),
            Tool::new(
                "open_item".to_string(),
                "Carry the user the LAST MILE — open a SPECIFIC item, not just its \
                 tab. Like navigate_app/app_action, calling this is the only way to \
                 actually open the item; describing it does nothing.\n\n\
                 Valid kinds: \"goal\" — open a goal card's detail (checklist, \
                 evidence, decisions); pass project_id AND card_id. \"grow\" — open \
                 a project's Grow planner; pass project_id.\n\n\
                 Get the ids from your own tools first: project_resolve → \
                 project_id; board_summary or card_list → a card_type \
                 \"goal\" card's id. This sends the open request to the app but \
                 cannot confirm the open: a stale or guessed id (or no app \
                 window) changes nothing on screen, so always use CURRENT ids \
                 from those tools. (To open a project's Kanban/Overview instead, \
                 use navigate_app to the Projects tab with state { project_id }.)"
                    .to_string(),
                schema::<OpenItemParams>(),
            ),
            Tool::new(
                "copy_to_clipboard".to_string(),
                "Put paste-ready text on the user's clipboard. Call this when they \
                 ask for a post, caption, speech, blurb, or any short copy they \
                 want to paste somewhere else (Notes, Messages, a social app) — \
                 \"give me the text\", \"copy that\", \"put it on my clipboard\". \
                 Describing the copy in chat does not put it on the clipboard.\n\n\
                 Pass the EXACT body they will paste: plain text, no markdown \
                 fences, no surrounding commentary. Speak a short confirmation \
                 (\"It's on your clipboard\") instead of reading the body aloud."
                    .to_string(),
                schema::<CopyToClipboardParams>(),
            ),
        ]
    }
}

// ── MCP trait implementation ────────────────────────────────────────────────

#[async_trait]
impl McpClientTrait for AppConductorClient {
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
        ctx: &ToolCallContext,
        name: &str,
        arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<CallToolResult, Error> {
        let result = match name {
            "navigate_app" => self.handle_navigate(&ctx.session_id, arguments).await,
            "app_action" => self.handle_action(arguments).await,
            "open_item" => self.handle_open_item(arguments).await,
            "copy_to_clipboard" => self.handle_copy(&ctx.session_id, arguments).await,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_pairs_accepted() {
        assert!(action_is_valid("chat", "open"));
        assert!(action_is_valid("chat", "detach"));
        assert!(action_is_valid("build", "hide_browser"));
        assert!(action_is_valid("build", "show_terminal"));
    }

    #[test]
    fn unknown_pairs_rejected() {
        assert!(!action_is_valid("chat", "hide_browser")); // right action, wrong surface
        assert!(!action_is_valid("build", "detach")); // right surface, wrong action
        assert!(!action_is_valid("world", "open")); // unknown surface
        assert!(!action_is_valid("chat", "")); // empty action
    }

    #[test]
    fn catalog_lists_names_every_surface() {
        let listed = catalog_lists();
        assert!(listed.contains("chat →"));
        assert!(listed.contains("build →"));
        assert!(listed.contains("hide_browser"));
    }

    #[test]
    fn valid_item_kinds_accepted() {
        assert!(item_is_valid("goal"));
        assert!(item_is_valid("grow"));
    }

    #[test]
    fn unknown_item_kinds_rejected() {
        // Seams that exist but the agent can't reliably feed an id for are
        // intentionally NOT in the catalog — they must be rejected here.
        assert!(!item_is_valid("decision"));
        assert!(!item_is_valid("person"));
        assert!(!item_is_valid("memory"));
        assert!(!item_is_valid("project")); // "grow" is the project kind, not this
        assert!(!item_is_valid(""));
    }

    #[test]
    fn item_kinds_list_names_every_kind_and_its_ids() {
        let listed = item_kinds_list();
        assert!(listed.contains("goal (needs project_id + card_id)"));
        assert!(listed.contains("grow (needs project_id)"));
    }

    #[test]
    fn open_item_result_is_honest_about_delivery() {
        // The tool only emits a bus event — it cannot verify the id or that an
        // app window is listening. The success text must say "request sent",
        // never claim the item is open, and point at the id-refresh tools.
        let text = open_item_result_text("grow", "p1", None, "You asked to plan growth.");
        assert!(text.starts_with("Sent the open request for grow p1"));
        assert!(!text.contains("Opening"), "must not claim the item is open");
        assert!(text.contains("isn't open"));
        assert!(text.contains("stale"));
        assert!(text.contains("project_resolve"));
        assert!(
            text.ends_with("You asked to plan growth."),
            "reason must survive at the end (the established result shape)"
        );
    }

    #[test]
    fn open_item_result_names_both_ids_for_goal() {
        let text = open_item_result_text("goal", "p1", Some("c9"), "r");
        assert!(text.contains("goal p1 (card c9)"));
        // Empty card_id (grow, or a goal caught by earlier validation) must not
        // render a dangling "(card )".
        let text = open_item_result_text("grow", "p1", Some(""), "r");
        assert!(!text.contains("(card"));
    }

    #[test]
    fn item_catalog_matches_frontend_dispatcher_one_to_one() {
        // The ITEM_CATALOG ↔ useAppNavigate `dispatchOpenItem` map is 1:1 (mirrors
        // the ACTION_CATALOG invariant). If a kind is added/removed here without
        // updating the dispatcher (or vice versa), the agent emits an event the
        // frontend silently drops. Keep this list in lockstep with dispatchOpenItem.
        let kinds: Vec<&str> = ITEM_CATALOG.iter().map(|(k, _)| *k).collect();
        assert_eq!(kinds, vec!["goal", "grow"]);
    }

    #[test]
    fn clipboard_body_trims_and_rejects_empty() {
        assert_eq!(clipboard_body("  hello \n").unwrap(), "hello");
        assert!(clipboard_body("   ").is_err());
        assert!(clipboard_body("").is_err());
    }

    #[test]
    fn clipboard_body_rejects_oversized() {
        let too_long: String = "a".repeat(MAX_CLIPBOARD_CHARS + 1);
        assert!(clipboard_body(&too_long).is_err());
        let ok: String = "a".repeat(MAX_CLIPBOARD_CHARS);
        assert_eq!(
            clipboard_body(&ok).unwrap().chars().count(),
            MAX_CLIPBOARD_CHARS
        );
    }
}
