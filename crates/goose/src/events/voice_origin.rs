//! Per-session voice origin — which device is listening on `/voice`.
//!
//! Auth already knows the paired device (the iPhone token updates last-seen on
//! connect) and then used to throw that principal away. This registry keeps the
//! resolved origin on the session for the duration of a voice turn so:
//!
//! - the reply prompt can say where the user is
//! - `navigate_app` / `app_action` / `open_item` can refuse to drive Command
//!   Center on a phone or watch
//! - the spoken-budget cue can offer to continue instead of pointing at a
//!   desktop screen
//!
//! Text-chat turns never call [`begin`], so [`current`] is `None` and desktop
//! tools behave as before.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// The surface that opened the `/voice` socket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceClient {
    /// Command Center on the hub Mac (or an undeclared client).
    Desktop,
    /// Native iOS voice orb.
    Ios,
    /// watchOS orb, relayed through the phone.
    Watch,
}

impl VoiceClient {
    /// Parse the `client=` query param. Unknown or missing values are desktop
    /// — the phone/watch must opt in, and a paired-device name can still
    /// override via [`VoiceOrigin::resolve`].
    pub fn parse(declared: Option<&str>) -> Self {
        match declared.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            Some("ios_voice" | "ios" | "iphone" | "ipad") => Self::Ios,
            Some("watch_voice" | "watch") => Self::Watch,
            _ => Self::Desktop,
        }
    }

    /// Infer from a pairing-registry display name ("iPhone", "Kitchen Watch").
    pub fn from_device_name(name: &str) -> Option<Self> {
        let n = name.to_ascii_lowercase();
        if n.contains("iphone") || n.contains("ipad") || n == "ios" {
            Some(Self::Ios)
        } else if n.contains("watch") {
            Some(Self::Watch)
        } else {
            None
        }
    }

    pub fn can_drive_desktop_ui(self) -> bool {
        matches!(self, Self::Desktop)
    }

    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Desktop => "desktop_voice",
            Self::Ios => "ios_voice",
            Self::Watch => "watch_voice",
        }
    }
}

/// Who is on the other end of this voice turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceOrigin {
    pub client: VoiceClient,
    pub device_name: Option<String>,
    /// True when the socket was opened with a `voice_only` scoped credential
    /// (standalone watch). Turns carrying it may call only the tools that
    /// [`voice_only_tool_allowed`] admits; the agent enforces that in both
    /// `list_tools` and `dispatch_tool_call`.
    pub voice_only: bool,
}

impl VoiceOrigin {
    /// Declared `client=` wins when it is phone/watch. A missing declaration
    /// still becomes iOS/watch when the pairing record is named that way —
    /// that is how the 2026-08-21 iPhone session was identifiable even though
    /// the socket sent only `session_id` and `token`.
    pub fn resolve(declared: Option<&str>, device_name: Option<&str>) -> Self {
        let parsed = VoiceClient::parse(declared);
        let client = match parsed {
            VoiceClient::Desktop => device_name
                .and_then(VoiceClient::from_device_name)
                .unwrap_or(VoiceClient::Desktop),
            other => other,
        };
        Self {
            client,
            device_name: device_name
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            voice_only: false,
        }
    }

    /// Mark this origin as a `voice_only` scoped credential's turn.
    pub fn with_voice_only(mut self) -> Self {
        self.voice_only = true;
        self
    }

    /// Short place-name for prompts and tool results ("iPhone", "your watch").
    pub fn spoken_place(&self) -> String {
        if let Some(name) = self.device_name.as_deref() {
            return name.to_string();
        }
        match self.client {
            VoiceClient::Desktop => "Mac".to_string(),
            VoiceClient::Ios => "iPhone".to_string(),
            VoiceClient::Watch => "watch".to_string(),
        }
    }

    /// Per-turn system prompt. Injected as `voice_origin`.
    pub fn prompt_block(&self) -> String {
        let place = self.spoken_place();
        match self.client {
            VoiceClient::Desktop => format!(
                "The user is speaking from Command Center on this Mac (voice, device {place}). \
                 A long reply's remainder is in the transcript — never say you put it 'on screen' \
                 as if they were on another device. When you take them somewhere, reply with \
                 exactly ONE short sentence then stop (e.g. 'Brain tab open'). navigate_app \
                 switches their view after you finish speaking."
            ),
            VoiceClient::Ios | VoiceClient::Watch => format!(
                "The user is speaking from their {place} over the voice orb — not Command Center \
                 on the Mac. They cannot see desktop tabs, the hub browser, or anything you \
                 'put on screen.' Speak the answer. Do not say you opened a tab, switched a \
                 view, or put the rest on a screen. Do not call navigate_app, app_action, or \
                 open_item — those drive the Mac they are not looking at. If they want a phone \
                 surface, tell them which one to open (Chat, Decisions, Notes). Browser tools \
                 are for YOU to read; narrate what you found, never 'look at the page.' \
                 copy_to_clipboard writes this {place}'s pasteboard. Never stop mid-answer \
                 to ask if they want you to continue. Keep the spoken answer short; anything \
                 past a few sentences reaches them as text in the chat."
            ),
        }
    }
}

static REGISTRY: LazyLock<Mutex<HashMap<String, VoiceOrigin>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Bind `origin` to this session until [`end`]. Idempotent; last write wins.
pub fn begin(session_id: &str, origin: VoiceOrigin) {
    if let Ok(mut reg) = REGISTRY.lock() {
        reg.insert(session_id.to_string(), origin);
    }
}

pub fn current(session_id: &str) -> Option<VoiceOrigin> {
    REGISTRY
        .lock()
        .ok()
        .and_then(|reg| reg.get(session_id).cloned())
}

/// Tool names a `voice_only` (standalone watch) session may see or call.
///
/// DEFAULT-DENY by extension prefix: platform tools are named
/// `<extension>__<tool>`, so unprefixed tools (shell, file edit, search),
/// `Extension Manager`, `configure`, `desktop_control`, `browser`, `summon`,
/// `orchestrator`, schedules, device or pairing administration, and every
/// extension not listed here are refused. Cross-session readers (chat recall)
/// are deliberately absent: the credential is bound to ONE conversation.
/// Widening this list is a one-line, reviewable change.
const VOICE_ONLY_TOOL_PREFIXES: &[&str] = &["todo__"];

/// Pure predicate behind the `voice_only` tool restriction.
pub fn voice_only_tool_allowed(tool_name: &str) -> bool {
    VOICE_ONLY_TOOL_PREFIXES
        .iter()
        .any(|prefix| tool_name.starts_with(prefix))
}

/// Whether the voice turn currently bound to `session_id` is `voice_only`.
pub fn is_voice_only(session_id: &str) -> bool {
    current(session_id).is_some_and(|origin| origin.voice_only)
}

/// The single gate used by the agent: may `tool_name` be listed/dispatched on
/// `session_id` right now? Unrestricted sessions are unaffected.
pub fn tool_allowed_for_session(session_id: &str, tool_name: &str) -> bool {
    !is_voice_only(session_id) || voice_only_tool_allowed(tool_name)
}

/// Drop the binding. Safe when [`begin`] never ran.
pub fn end(session_id: &str) {
    if let Ok(mut reg) = REGISTRY.lock() {
        reg.remove(session_id);
    }
    // Pending confirms deliberately outlive the turn: the user taps Confirm
    // after the reply ends, and the next turn re-issues the call. They lapse
    // on their own after CONFIRM_TTL_SECS.
}

/// True while a voice turn is bound to `session_id`. This is the single
/// "is this a voice turn" test: coding-harness after-turn guards, the slim
/// voice tool profile, the voice prompt and the side-effect gate all key on it.
pub fn is_voice_turn(session_id: &str) -> bool {
    current(session_id).is_some()
}

// ---------------------------------------------------------------------------
// V-13: voice tool profile (data, not conditionals)
// ---------------------------------------------------------------------------

/// Tools a voice turn may see (and, if they are not side-effecting, call).
///
/// One entry per line. `foo__*` is an extension prefix, anything else an exact
/// tool name (platform tools are named `<extension>__<tool>`). Everything not
/// listed is hidden from the model on a voice turn: no shell, no editor, no
/// orchestrator writes, no dispatch, no extension manager. Widening the voice
/// surface is a one-line change here. Keep the live total at or under
/// [`VOICE_MAX_TOOLS`].
pub const VOICE_TOOL_ALLOWLIST: &[&str] = &[
    // recall / reading
    "chatrecall__chatrecall",
    "inbox__list_pending_decisions",
    "dashboard__read_dashboard",
    "projectmanager__project_list",
    "projectmanager__card_list",
    "projectmanager__card_due_list",
    "projectmanager__board_summary",
    "orchestrator__goal_list",
    "orchestrator__goal_status",
    "orchestrator__check_worker",
    // people, calendar, notes (writes in these still pass the confirm gate)
    "people__*",
    "calendar__*",
    "notes__*",
    "note__*",
    // scratch list + the one web read a quick factual question needs
    "todo__todo_write",
    "browser__read_webpage",
    "public_apis__*",
    // moving the user's view / clipboard (the origin block governs phones)
    "app_conductor__navigate_app",
    "app_conductor__open_item",
    "app_conductor__copy_to_clipboard",
];

/// The most tools a voice turn should ever expose.
pub const VOICE_MAX_TOOLS: usize = 25;

fn pattern_matches(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == pattern,
    }
}

/// Whether `tool_name` is part of the slim voice tool set.
pub fn voice_slim_tool_allowed(tool_name: &str) -> bool {
    VOICE_TOOL_ALLOWLIST
        .iter()
        .any(|pattern| pattern_matches(pattern, tool_name))
}

/// Short voice system-prompt section. Replaces the coding prompt on voice turns.
pub const VOICE_PROMPT_SECTION: &str = "\
# Voice conversation
You are talking out loud with the user. Speak in short, plain sentences: no \
markdown, no lists, no code, no file paths, no ids. Answer first, then stop.

You have a small set of tools for memory, people, calendar, notes and reading \
the board. When the user clearly asks you to add or change a person, an event or \
a note and one of your tools does it, call that tool: it pauses for the user to \
tap Confirm on their screen, so say that in one short sentence and never say it \
is done until it is. If none of your tools does it, say plainly that you cannot \
do it from here; never mention a Confirm card you have not asked for. You are \
not in a coding session here: you cannot run shell commands, edit files, \
dispatch or advance goals or change board state from voice; for those, say you \
will set it up in chat.";

/// Extras (by `system_prompt_extras` key) a voice turn keeps. Everything else
/// (skills catalogue, roadmap, repo map, hints, live status) is left out.
pub const VOICE_PROMPT_EXTRA_KEYS: &[&str] = &[
    "voice_reply_style",
    "voice_origin",
    "memory_recall",
    "model_identity",
];

// ---------------------------------------------------------------------------
// V-12 / D4: side-effect policy
// ---------------------------------------------------------------------------

/// Whole tool families that change the world. Always confirm-gated on voice.
const SIDE_EFFECT_PREFIXES: &[&str] = &[
    "developer__",
    "desktop_control__",
    "Extension Manager__",
    "configure__",
    "summon__",
    "orchestrator__",
    "projectmanager__",
    "apps__",
    "finance__",
    "recipe_author__",
    "model_manager__",
    "file_to_project__",
    "platform__manage_schedule",
];

/// Unprefixed tools that change the world.
const SIDE_EFFECT_EXACT: &[&str] = &[
    "shell",
    "write",
    "edit",
    "text_editor",
    "app_conductor__app_action",
];

/// Any `_`-separated word of the tool's own name that is one of these marks it
/// as a write. Catches calendar/notes/people MCP tools we have never seen.
const WRITE_VERBS: &[&str] = &[
    "create",
    "update",
    "delete",
    "remove",
    "move",
    "merge",
    "save",
    "set",
    "add",
    "send",
    "start",
    "stop",
    "apply",
    "revert",
    "approve",
    "retry",
    "connect",
    "disconnect",
    "launch",
    "record",
    "enrich",
    "propose",
    "log",
    "write",
    "edit",
    "run",
    "dispatch",
    "advance",
    "steer",
    "interrupt",
    "pause",
    "resume",
    "decompose",
    "escalate",
    "install",
    "publish",
    "post",
    "schedule",
    "kill",
    "restart",
    "commit",
    "push",
    "associate",
    "reschedule",
    "cancel",
    "archive",
    "rename",
    "assign",
    "invite",
    "reply",
    "forward",
    "book",
    "buy",
    "sell",
    "close",
];

/// Read-ish allowlisted tools that nevertheless contain a write word.
const SIDE_EFFECT_EXEMPT: &[&str] = &["todo__todo_write"];

/// Does calling `tool_name` change something outside this conversation?
pub fn is_side_effecting(tool_name: &str) -> bool {
    if SIDE_EFFECT_EXEMPT.contains(&tool_name) {
        return false;
    }
    if SIDE_EFFECT_EXACT.contains(&tool_name)
        || SIDE_EFFECT_PREFIXES
            .iter()
            .any(|p| tool_name.starts_with(p))
    {
        // Read-only members of a gated family stay readable when allowlisted.
        return !voice_slim_tool_allowed(tool_name);
    }
    let own = tool_name.rsplit("__").next().unwrap_or(tool_name);
    own.split('_').any(|word| WRITE_VERBS.contains(&word))
}

/// The structured request a UI renders as an on-screen confirm card.
/// Carried as the payload of `voice_confirm_requested` on the `/events` bus and
/// as the structured content of the tool result the model sees.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VoiceConfirmRequest {
    /// Opaque id; the UI sends it back to approve or deny.
    pub confirm_id: String,
    pub session_id: String,
    pub tool: String,
    /// One-line, speakable description ("Run shell command", "Create person").
    pub summary: String,
    /// Redacted/truncated arguments for display only.
    pub args_preview: String,
    /// Unix seconds after which the request lapses.
    pub expires_at: i64,
}

/// How long an on-screen confirm stays valid.
pub const CONFIRM_TTL_SECS: i64 = 120;

struct Pending {
    session_id: String,
    tool: String,
    args_key: String,
    expires_at: i64,
    approved: bool,
}

static PENDING: LazyLock<Mutex<HashMap<String, Pending>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn args_key(args: &serde_json::Value) -> String {
    // serde_json maps are ordered, so this is stable for equal arguments.
    args.to_string()
}

fn preview(args: &serde_json::Value) -> String {
    let s = args.to_string();
    if s.chars().count() > 240 {
        let cut: String = s.chars().take(240).collect();
        format!("{cut}…")
    } else {
        s
    }
}

/// What the dispatcher must do with a tool call on a voice turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VoiceGate {
    /// Run it (not a voice turn, a read tool, or a user-approved call).
    Allow,
    /// Hidden from the voice profile and not a known write.
    Unavailable,
    /// Do not run it; show this confirm request on screen.
    NeedsConfirmation(VoiceConfirmRequest),
}

/// The one gate on a voice turn. Pure apart from the pending-confirm registry.
pub fn gate_tool_call(session_id: &str, tool_name: &str, args: &serde_json::Value) -> VoiceGate {
    if !is_voice_turn(session_id) {
        return VoiceGate::Allow;
    }
    if is_side_effecting(tool_name) {
        let key = args_key(args);
        let now = now_secs();
        if let Ok(mut pending) = PENDING.lock() {
            pending.retain(|_, p| p.expires_at > now);
            // An approved confirm for this exact call is consumed once.
            let hit = pending
                .iter()
                .find(|(_, p)| {
                    p.approved
                        && p.session_id == session_id
                        && p.tool == tool_name
                        && p.args_key == key
                })
                .map(|(id, _)| id.clone());
            if let Some(id) = hit {
                pending.remove(&id);
                return VoiceGate::Allow;
            }
            // Re-asking for the same call re-uses its open request.
            let existing = pending
                .iter()
                .find(|(_, p)| {
                    !p.approved
                        && p.session_id == session_id
                        && p.tool == tool_name
                        && p.args_key == key
                })
                .map(|(id, p)| (id.clone(), p.expires_at));
            let (confirm_id, expires_at) = match existing {
                Some(found) => found,
                None => {
                    let id = uuid::Uuid::now_v7().to_string();
                    let expires_at = now + CONFIRM_TTL_SECS;
                    pending.insert(
                        id.clone(),
                        Pending {
                            session_id: session_id.to_string(),
                            tool: tool_name.to_string(),
                            args_key: key,
                            expires_at,
                            approved: false,
                        },
                    );
                    (id, expires_at)
                }
            };
            return VoiceGate::NeedsConfirmation(VoiceConfirmRequest {
                confirm_id,
                session_id: session_id.to_string(),
                tool: tool_name.to_string(),
                summary: confirm_summary(tool_name),
                args_preview: preview(args),
                expires_at,
            });
        }
        // Poisoned lock: fail closed.
        return VoiceGate::Unavailable;
    }
    if voice_slim_tool_allowed(tool_name) {
        VoiceGate::Allow
    } else {
        VoiceGate::Unavailable
    }
}

/// The user tapped Confirm on screen. Returns false for an unknown, expired or
/// foreign-session id. The model must re-issue the same call; the gate then
/// lets that one call through.
pub fn approve_confirmation(session_id: &str, confirm_id: &str) -> bool {
    let now = now_secs();
    let Ok(mut pending) = PENDING.lock() else {
        return false;
    };
    match pending.get_mut(confirm_id) {
        Some(p) if p.session_id == session_id && p.expires_at > now => {
            p.approved = true;
            true
        }
        _ => false,
    }
}

/// The user dismissed the card.
pub fn deny_confirmation(session_id: &str, confirm_id: &str) -> bool {
    let Ok(mut pending) = PENDING.lock() else {
        return false;
    };
    if pending
        .get(confirm_id)
        .is_some_and(|p| p.session_id == session_id)
    {
        pending.remove(confirm_id);
        return true;
    }
    false
}

/// Plain words for the confirm card: `people__create_person` becomes
/// "Allow create person from voice?". Never names the agent (it is configurable).
pub fn confirm_summary(tool_name: &str) -> String {
    let action = tool_name.rsplit("__").next().unwrap_or(tool_name).trim();
    let action = action.replace(['_', '-'], " ");
    let action = action.split_whitespace().collect::<Vec<_>>().join(" ");
    if action.is_empty() {
        "Allow this action from voice?".to_string()
    } else {
        format!("Allow {action} from voice?")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_wire_names() {
        assert_eq!(VoiceClient::parse(Some("ios_voice")), VoiceClient::Ios);
        assert_eq!(VoiceClient::parse(Some("watch_voice")), VoiceClient::Watch);
        assert_eq!(
            VoiceClient::parse(Some("desktop_voice")),
            VoiceClient::Desktop
        );
        assert_eq!(VoiceClient::parse(None), VoiceClient::Desktop);
        assert_eq!(VoiceClient::parse(Some("nonsense")), VoiceClient::Desktop);
    }

    #[test]
    fn iphone_pairing_name_overrides_missing_client_param() {
        let origin = VoiceOrigin::resolve(None, Some("iPhone"));
        assert_eq!(origin.client, VoiceClient::Ios);
        assert_eq!(origin.spoken_place(), "iPhone");
    }

    #[test]
    fn explicit_ios_wins_over_laptop_device_name() {
        let origin = VoiceOrigin::resolve(Some("ios_voice"), Some("Studio Mac"));
        assert_eq!(origin.client, VoiceClient::Ios);
    }

    #[test]
    fn desktop_stays_desktop_without_a_phone_name() {
        let origin = VoiceOrigin::resolve(None, Some("diag-test"));
        assert_eq!(origin.client, VoiceClient::Desktop);
    }

    #[test]
    fn phone_prompt_forbids_screen_and_navigate() {
        let block = VoiceOrigin::resolve(Some("ios_voice"), Some("iPhone")).prompt_block();
        assert!(block.contains("iPhone"));
        assert!(block.contains("Do not call navigate_app"));
        assert!(block.contains("cannot see"));
        assert!(
            block.contains("Never stop mid-answer"),
            "phone prompt must forbid a mid-reply continue question: {block}"
        );
    }

    #[test]
    fn begin_current_end_roundtrip() {
        let sid = "origin-voice-1";
        end(sid);
        assert!(current(sid).is_none());
        begin(sid, VoiceOrigin::resolve(Some("ios_voice"), Some("iPhone")));
        let got = current(sid).expect("bound");
        assert_eq!(got.client, VoiceClient::Ios);
        end(sid);
        assert!(current(sid).is_none());
    }

    #[test]
    fn voice_only_allows_only_the_listed_extensions() {
        assert!(voice_only_tool_allowed("todo__todo_write"));
        assert!(!voice_only_tool_allowed("pronunciation__teach"));
        for denied in [
            "shell",
            "developer__shell",
            "edit",
            "read",
            "configure__set_config",
            "Extension Manager__manage_extensions",
            "desktop_control__click",
            "browser__navigate",
            "summon__delegate",
            "orchestrator__goal_advance",
            "chatrecall__chatrecall",
            "platform__manage_schedule",
            "platform__load_feature_lesson",
            "",
        ] {
            assert!(!voice_only_tool_allowed(denied), "{denied} must be denied");
        }
    }

    #[test]
    fn restriction_applies_only_to_voice_only_turns() {
        let sid = "origin-voice-only-1";
        end(sid);
        // No voice turn bound: unrestricted (text chat unaffected).
        assert!(tool_allowed_for_session(sid, "shell"));
        begin(sid, VoiceOrigin::resolve(Some("ios_voice"), Some("iPhone")));
        assert!(tool_allowed_for_session(sid, "shell"));
        begin(
            sid,
            VoiceOrigin::resolve(Some("watch_voice"), Some("iPhone Watch")).with_voice_only(),
        );
        assert!(!tool_allowed_for_session(sid, "shell"));
        assert!(tool_allowed_for_session(sid, "todo__todo_write"));
        end(sid);
        assert!(tool_allowed_for_session(sid, "shell"));
    }

    fn bind_voice(sid: &str) {
        end(sid);
        begin(sid, VoiceOrigin::resolve(Some("desktop_voice"), None));
    }

    #[test]
    fn slim_profile_exposes_few_tools_and_no_coding_tools() {
        let catalogue = [
            "developer__shell",
            "developer__text_editor",
            "shell",
            "edit",
            "write",
            "orchestrator__goal_advance",
            "orchestrator__goal_list",
            "orchestrator__dispatch",
            "summon__delegate",
            "Extension Manager__manage_extensions",
            "configure__set_config",
            "desktop_control__click",
            "browser__act_on_page",
            "browser__read_webpage",
            "projectmanager__card_move",
            "projectmanager__card_list",
            "chatrecall__chatrecall",
            "people__create_person",
            "calendar__list_events",
            "notes__create_note",
            "code_execution__execute",
            "analyze__analyze",
            "skills__load_skill",
        ];
        for coding in [
            "developer__shell",
            "developer__text_editor",
            "shell",
            "edit",
            "write",
            "orchestrator__goal_advance",
            "orchestrator__dispatch",
            "summon__delegate",
            "code_execution__execute",
            "analyze__analyze",
            "projectmanager__card_move",
        ] {
            assert!(
                !voice_slim_tool_allowed(coding),
                "{coding} leaked into voice"
            );
        }
        assert!(voice_slim_tool_allowed("chatrecall__chatrecall"));
        assert!(voice_slim_tool_allowed("calendar__list_events"));
        // Even if every catalogue entry were loaded, the profile caps the set.
        let exposed = catalogue
            .iter()
            .filter(|t| voice_slim_tool_allowed(t))
            .count();
        assert!(exposed <= VOICE_MAX_TOOLS, "{exposed} voice tools");
        assert!(VOICE_TOOL_ALLOWLIST.len() <= VOICE_MAX_TOOLS);
    }

    #[test]
    fn writes_need_confirmation_on_a_voice_turn() {
        let sid = "gate-voice-1";
        bind_voice(sid);
        let args = serde_json::json!({"cmd": "ls"});
        for tool in [
            "developer__shell",
            "shell",
            "orchestrator__goal_advance",
            "projectmanager__card_move",
            "people__create_person",
            "calendar__create_event",
            "app_conductor__app_action",
        ] {
            match gate_tool_call(sid, tool, &args) {
                VoiceGate::NeedsConfirmation(req) => {
                    assert_eq!(req.tool, tool);
                    assert_eq!(req.session_id, sid);
                    assert!(!req.confirm_id.is_empty());
                }
                other => panic!("{tool} should need confirmation, got {other:?}"),
            }
        }
        end(sid);
    }

    #[test]
    fn reads_stay_allowed_and_unlisted_tools_are_unavailable() {
        let sid = "gate-voice-2";
        bind_voice(sid);
        let args = serde_json::json!({});
        for tool in [
            "chatrecall__chatrecall",
            "projectmanager__card_list",
            "orchestrator__goal_status",
            "calendar__list_events",
            "todo__todo_write",
            "app_conductor__navigate_app",
        ] {
            assert_eq!(gate_tool_call(sid, tool, &args), VoiceGate::Allow, "{tool}");
        }
        assert_eq!(
            gate_tool_call(sid, "analyze__analyze", &args),
            VoiceGate::Unavailable
        );
        end(sid);
    }

    #[test]
    fn text_chat_turns_are_untouched() {
        let sid = "gate-text-1";
        end(sid);
        let args = serde_json::json!({"cmd": "ls"});
        assert_eq!(
            gate_tool_call(sid, "developer__shell", &args),
            VoiceGate::Allow
        );
        assert_eq!(
            gate_tool_call(sid, "orchestrator__goal_advance", &args),
            VoiceGate::Allow
        );
    }

    #[test]
    fn approved_confirmation_lets_exactly_one_identical_call_through() {
        let sid = "gate-voice-3";
        bind_voice(sid);
        let args = serde_json::json!({"name": "Ada"});
        let VoiceGate::NeedsConfirmation(req) = gate_tool_call(sid, "people__create_person", &args)
        else {
            panic!("expected confirm")
        };
        // Asking again reuses the same open request.
        let VoiceGate::NeedsConfirmation(again) =
            gate_tool_call(sid, "people__create_person", &args)
        else {
            panic!("expected confirm")
        };
        assert_eq!(req.confirm_id, again.confirm_id);
        // A foreign session cannot approve it.
        assert!(!approve_confirmation("other-session", &req.confirm_id));
        assert!(approve_confirmation(sid, &req.confirm_id));
        // Different arguments are not covered by the approval.
        assert!(matches!(
            gate_tool_call(
                sid,
                "people__create_person",
                &serde_json::json!({"name": "Bob"})
            ),
            VoiceGate::NeedsConfirmation(_)
        ));
        assert_eq!(
            gate_tool_call(sid, "people__create_person", &args),
            VoiceGate::Allow
        );
        // Consumed: the next identical call needs a fresh confirm.
        assert!(matches!(
            gate_tool_call(sid, "people__create_person", &args),
            VoiceGate::NeedsConfirmation(_)
        ));
        end(sid);
    }

    #[test]
    fn denied_confirmation_is_dropped() {
        let sid = "gate-voice-4";
        bind_voice(sid);
        let args = serde_json::json!({"cmd": "rm"});
        let VoiceGate::NeedsConfirmation(req) = gate_tool_call(sid, "shell", &args) else {
            panic!("expected confirm")
        };
        assert!(deny_confirmation(sid, &req.confirm_id));
        assert!(!approve_confirmation(sid, &req.confirm_id));
        end(sid);
    }

    #[test]
    fn confirm_summary_is_plain_words_without_an_agent_name() {
        assert_eq!(
            confirm_summary("people__create_person"),
            "Allow create person from voice?"
        );
        assert_eq!(confirm_summary("shell"), "Allow shell from voice?");
        assert_eq!(confirm_summary("__"), "Allow this action from voice?");
    }

    #[test]
    fn voice_prompt_section_is_short_and_not_coding() {
        assert!(VOICE_PROMPT_SECTION.len() < 2_000);
        assert!(!VOICE_PROMPT_SECTION
            .to_ascii_lowercase()
            .contains("worktree"));
    }
}
