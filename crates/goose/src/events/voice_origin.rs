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
        }
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
                 to ask if they want you to continue — if there is more, just keep talking. \
                 The system will offer a continue cue only when a hard spoken budget hits."
            ),
        }
    }
}

/// Spoken once when the 8-sentence budget runs out. Must never mention a
/// screen the listener cannot see.
pub fn budget_notice(client: VoiceClient) -> &'static str {
    match client {
        VoiceClient::Desktop => "There's more — the rest is in the transcript.",
        // Not a question. Last night the model (and this cue) asked "do you
        // want me to continue" mid-story; saying continue then started a
        // cold turn. The leftover is now stashed; this is only a beat.
        VoiceClient::Ios | VoiceClient::Watch => "There's more when you want it.",
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

/// Drop the binding. Safe when [`begin`] never ran.
pub fn end(session_id: &str) {
    if let Ok(mut reg) = REGISTRY.lock() {
        reg.remove(session_id);
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
    fn budget_notice_never_says_on_screen() {
        for client in [VoiceClient::Desktop, VoiceClient::Ios, VoiceClient::Watch] {
            let n = budget_notice(client);
            assert!(
                !n.to_ascii_lowercase().contains("on screen"),
                "desktop-assuming cue leaked for {client:?}: {n}"
            );
            assert!(!n.is_empty());
        }
    }

    #[test]
    fn phone_budget_notice_is_not_a_question() {
        // 20260821_14: "say continue" / "do you want me to continue" mid-reply
        // derailed the next turn. The cue must not ask.
        for client in [VoiceClient::Ios, VoiceClient::Watch] {
            let n = budget_notice(client);
            assert!(!n.contains('?'), "question cue for {client:?}: {n}");
            assert!(
                !n.to_ascii_lowercase().contains("do you want"),
                "continue-offer question leaked for {client:?}: {n}"
            );
        }
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
}
