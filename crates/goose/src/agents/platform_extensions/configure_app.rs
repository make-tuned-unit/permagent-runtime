//! Configure — the agent's first user-facing settings WRITE capability.
//!
//! Until this extension the agent could describe every switch in its own home
//! and change none of them: the only config writes it had were `tour_completed`
//! and `learn_next` bookkeeping. "The agent can set up every element of its
//! home" starts here, and the whole design is about doing that *safely*.
//!
//! Three tools, three postures:
//!
//! * [`CONFIGURE_READ`] — read the current value of any key in the writable or
//!   proposable set. Secrets report PRESENCE ONLY; a secret value never leaves
//!   `Config` through this tool.
//! * [`CONFIGURE_SET`] — a DIRECT write, allowlisted to the low-risk class
//!   ([`WRITABLE_KEYS`]): the six worker feature gates, `dev_roots`, the two
//!   Watcher lists, and the Librarian schedule. Everything else is refused.
//! * [`CONFIGURE_PROPOSE`] — for the sensitive classes ([`PROPOSAL_CLASSES`]):
//!   autonomy, sovereignty, budget ceilings, provider/model. Files a Decision
//!   Inbox card; the APPROVAL applies the write server-side
//!   (`decisions_effects::apply_config_change`), rejection writes nothing.
//!
//! ## One writer, not a second path
//!
//! Every direct write here goes through `Config::set_param` — the same writer
//! `POST /config/upsert` calls for the same key — so the write announces itself
//! with `config_changed` from the writer and an open Settings pane refreshes.
//! There is no new emit site in this module, deliberately: a second announcer
//! is a second source of truth.
//!
//! The Librarian schedule is the one exception in shape, not in principle. It
//! is not a `Config` key: it is `librarian_schedule.json`, written by a private
//! function inside the daemon crate
//! (`goose-server/src/routes/librarian/scheduling.rs::save_schedule`), which
//! `permagent` cannot call — the dependency runs the other way. So the
//! Librarian arm takes the `save_pronunciation` shape and goes over loopback
//! HTTP to `PUT /api/librarian/schedule`, landing on that one writer with its
//! own validation rather than reimplementing the file format here.
//!
//! ## Why these tools are hard-excluded from `/agent/call_tool`
//!
//! `POST /agent/call_tool` dispatches straight into the extension manager and
//! never touches the tool-confirmation router. Its only guard,
//! [`crate::agents::reply_parts::is_tool_visible_to_app`], FAILS OPEN: a tool
//! with no `_meta.ui.visibility` is callable by any app. So every tool here
//! declares `visibility: ["model"]` explicitly — visible to the model, refused
//! (403) for the app path. That is a per-tool hardening, not a fix of the
//! fail-open default, which belongs to its own change.

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use crate::config::Config;
use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult, Meta,
    ServerCapabilities, Tool, ToolAnnotations,
};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "configure";

pub const CONFIGURE_READ: &str = "configure_read";
pub const CONFIGURE_SET: &str = "configure_set";
pub const CONFIGURE_PROPOSE: &str = "configure_propose";

/// The Decision Inbox kind a [`CONFIGURE_PROPOSE`] card is filed under.
pub const CONFIG_CHANGE_PROPOSAL_KIND: &str = "config_change_proposal";

/// Not a `Config` key: the Librarian schedule lives in
/// `librarian_schedule.json` and is written by the daemon route. Named here so
/// the allowlist, the refusals and the tool description all say one thing.
pub const LIBRARIAN_SCHEDULE_KEY: &str = "librarian_schedule";

/// The Watcher's two list keys.
///
/// Duplicated string literals, unavoidably: the canonical constants
/// (`WATCHER_TOPICS_KEY` / `WATCHER_MUTED_KEY`) are private to
/// `goose-server/src/proactive.rs`, in the crate that depends on this one. This
/// is the same admitted drift the `steward_scan_enabled` key already carries
/// between `self_knowledge` and `steward_sweep`. `watcher_keys_match_the_reader`
/// pins the strings so a rename is at least a one-line fix with a test pointing
/// at it.
pub const WATCHER_TOPICS_KEY: &str = "watcher_topics";
pub const WATCHER_MUTED_SUBJECTS_KEY: &str = "watcher_muted_subjects";

/// The four legal `GOOSE_MODE` values, per `Config::get_goose_mode`.
const GOOSE_MODES: &[&str] = &["approve", "chat", "auto", "smart_approve"];

/// What a key holds. A value of the wrong shape is REFUSED, never coerced: a
/// silent coercion is how `strix_enabled: "false"` becomes a truthy string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Bool,
    StringList,
    Number,
    Enum(&'static [&'static str]),
    Text,
    /// A JSON object of named fields — only the Librarian schedule.
    Fields,
}

impl Shape {
    fn describe(self) -> &'static str {
        match self {
            Shape::Bool => "true or false",
            Shape::StringList => "a list of strings",
            Shape::Number => "a number",
            Shape::Enum(options) => match options.first() {
                Some(_) => "one of a fixed set of strings",
                None => "a string",
            },
            Shape::Text => "a string",
            Shape::Fields => "an object of named fields",
        }
    }

    /// Check a proposed value against this shape, returning why it does not fit.
    fn check(self, value: &Value) -> Result<(), String> {
        let fits = match self {
            Shape::Bool => value.is_boolean(),
            Shape::StringList => value
                .as_array()
                .is_some_and(|a| a.iter().all(serde_json::Value::is_string)),
            Shape::Number => value.as_f64().is_some_and(f64::is_finite),
            Shape::Enum(options) => value
                .as_str()
                .is_some_and(|s| options.iter().any(|o| *o == s)),
            Shape::Text => value.is_string(),
            Shape::Fields => value.is_object(),
        };
        if fits {
            return Ok(());
        }
        let expected = match self {
            Shape::Enum(options) => format!("one of {}", options.join(", ")),
            other => other.describe().to_string(),
        };
        Err(format!("expected {expected}, got {}", compact(value)))
    }
}

/// A key the agent may write DIRECTLY. Low-risk by construction: every one of
/// these is a switch or a list the user can flip back in one click, none of
/// them widens what the agent is allowed to do, and none of them can spend
/// money or send data anywhere new.
#[derive(Debug, Clone, Copy)]
pub struct WritableKey {
    pub key: &'static str,
    pub shape: Shape,
    /// Where a human changes the same thing. Quoted back in every message so
    /// the user is never told about a setting they cannot find.
    pub surface: &'static str,
    /// What it does, in one clause, for `configure_read`.
    pub what: &'static str,
}

/// The DIRECT-write allowlist. Nothing outside this list is writable by
/// [`CONFIGURE_SET`], and the refusal for anything outside it is explicit.
///
/// The six feature gates use the canonical key constants each subsystem reads
/// from, not string literals, so they are byte-identical to the `worker_gate`
/// table by construction — `feature_gate_keys_match_worker_gate` proves it.
pub const WRITABLE_KEYS: &[WritableKey] = &[
    WritableKey {
        key: crate::initiative::driver::INITIATIVE_ENABLED_KEY,
        shape: Shape::Bool,
        surface: "Settings → Features",
        what: "whether you start work on your own initiative",
    },
    WritableKey {
        key: crate::playbook::PLAYBOOK_ENABLED_KEY,
        shape: Shape::Bool,
        surface: "Settings → Features",
        what: "whether the Playbook worker runs",
    },
    WritableKey {
        key: crate::concierge::CONCIERGE_ENABLED_KEY,
        shape: Shape::Bool,
        surface: "Settings → Features",
        what: "whether the Concierge worker runs",
    },
    WritableKey {
        key: crate::agents::self_knowledge::STEWARD_SCAN_ENABLED_KEY,
        shape: Shape::Bool,
        surface: "Settings → Agents",
        what: "whether the Git Steward scans repositories",
    },
    WritableKey {
        key: crate::strix::STRIX_ENABLED_KEY,
        shape: Shape::Bool,
        surface: "Settings → Models",
        what: "whether the Guard reviews work",
    },
    WritableKey {
        key: crate::council::ENABLED_KEY,
        shape: Shape::Bool,
        surface: "Settings → Features",
        what: "whether the weekly Council debate runs",
    },
    WritableKey {
        key: crate::config::dev_roots::DEV_ROOTS_KEY,
        shape: Shape::StringList,
        surface: "Settings → Dev roots",
        what: "the directories treated as code roots",
    },
    WritableKey {
        key: WATCHER_TOPICS_KEY,
        shape: Shape::StringList,
        surface: "Settings → Watcher",
        what: "the topics the Watcher brings you news about",
    },
    WritableKey {
        key: WATCHER_MUTED_SUBJECTS_KEY,
        shape: Shape::StringList,
        surface: "Settings → Watcher",
        what: "the subjects the Watcher stays quiet about",
    },
    WritableKey {
        key: LIBRARIAN_SCHEDULE_KEY,
        shape: Shape::Fields,
        surface: "Settings → Librarian",
        what: "the nightly Librarian window (enabled, start_time, duration_minutes, \
               model, run_if_launched_in_window, pruning_enabled)",
    },
];

/// One sensitive key, and the shape an approved write must have.
#[derive(Debug, Clone, Copy)]
pub struct ProposalKey {
    pub key: &'static str,
    pub shape: Shape,
}

/// A class of setting the agent may PROPOSE and never write directly.
#[derive(Debug, Clone, Copy)]
pub struct ProposalClass {
    pub id: &'static str,
    pub keys: &'static [ProposalKey],
    /// Why it is gated — printed on every refusal, so the model learns the
    /// rule rather than just the "no".
    pub why_gated: &'static str,
    pub surface: &'static str,
}

/// The classes [`CONFIGURE_PROPOSE`] accepts. Nothing here is ever written by
/// [`CONFIGURE_SET`], and the write happens only when the user approves the
/// card.
pub const PROPOSAL_CLASSES: &[ProposalClass] = &[
    ProposalClass {
        id: "autonomy",
        keys: &[ProposalKey {
            key: "GOOSE_MODE",
            shape: Shape::Enum(GOOSE_MODES),
        }],
        why_gated: "GOOSE_MODE is your own leash — how much you may do without asking. \
                    Loosening it is exactly the decision you must never make for yourself.",
        surface: "Settings → Chat",
    },
    ProposalClass {
        id: "sovereignty",
        keys: &[
            ProposalKey {
                key: crate::sovereignty::SOVEREIGN_MODE_KEY,
                shape: Shape::Bool,
            },
            ProposalKey {
                key: crate::sovereignty::SOVEREIGN_CAPTURE_PROMPTS_KEY,
                shape: Shape::Bool,
            },
            ProposalKey {
                key: crate::sovereignty::SOVEREIGN_STRICT_AUDIT_KEY,
                shape: Shape::Bool,
            },
        ],
        why_gated: "these decide whether anything may leave this machine, and what the \
                    egress audit records. Turning sovereign mode off is a data-boundary \
                    decision that belongs to the user.",
        surface: "Settings → Sovereignty",
    },
    ProposalClass {
        id: "budget",
        keys: &[
            ProposalKey {
                key: crate::cost_router::budget::KEY_TASK_SOFT,
                shape: Shape::Number,
            },
            ProposalKey {
                key: crate::cost_router::budget::KEY_TASK_GATE,
                shape: Shape::Number,
            },
            ProposalKey {
                key: crate::cost_router::budget::KEY_TASK_HARD,
                shape: Shape::Number,
            },
            ProposalKey {
                key: crate::cost_router::budget::KEY_SESSION_SOFT,
                shape: Shape::Number,
            },
            ProposalKey {
                key: crate::cost_router::budget::KEY_SESSION_GATE,
                shape: Shape::Number,
            },
            ProposalKey {
                key: crate::cost_router::budget::KEY_SESSION_HARD,
                shape: Shape::Number,
            },
        ],
        why_gated: "these are the ceilings that stop you spending the user's money. \
                    Raising your own cap is not a decision you get to make.",
        surface: "Settings → Spend",
    },
    ProposalClass {
        id: "provider_or_model",
        keys: &[
            ProposalKey {
                key: "GOOSE_PROVIDER",
                shape: Shape::Text,
            },
            ProposalKey {
                key: "GOOSE_MODEL",
                shape: Shape::Text,
            },
        ],
        why_gated: "which provider and model serve the user changes cost, latency, \
                    capability and where their words are sent. To switch the active LOCAL \
                    inference model, use propose_model_upgrade instead — it checks the \
                    model is installed first.",
        surface: "Settings → Models",
    },
];

/// Suffixes and prefixes that mark a key as holding a credential. Deliberately
/// broad: a false positive costs a refusal the user can route around in
/// Settings, a false negative would let a credential be written — or read —
/// through a chat tool.
const SECRET_MARKERS: &[&str] = &[
    "API_KEY",
    "APIKEY",
    "_TOKEN",
    "TOKEN_",
    "_SECRET",
    "SECRET_",
    "PASSWORD",
    "PASSPHRASE",
    "PRIVATE_KEY",
    "CREDENTIAL",
];

/// Is this key a credential? Presence may be reported; the value never is.
pub fn is_secret_key(key: &str) -> bool {
    if crate::polybot::POLYMARKET_SECRET_KEYS.contains(&key) {
        return true;
    }
    let upper = key.to_ascii_uppercase();
    SECRET_MARKERS.iter().any(|m| upper.contains(m))
}

/// What [`CONFIGURE_SET`] and [`CONFIGURE_READ`] are allowed to do with a key.
#[derive(Debug, Clone, Copy)]
pub enum KeyClass {
    /// Writable directly.
    Direct(&'static WritableKey),
    /// Proposal-only: the class it belongs to, and its expected shape.
    Proposal(&'static ProposalClass, &'static ProposalKey),
    /// A credential. Never written and never read by value.
    Secret,
    /// Not in any list. Refused by name, with a pointer to the nearest surface.
    Unknown,
}

/// Classify a key. The ONE place the allowlist is consulted, so a refusal and a
/// permission can never disagree.
pub fn classify(key: &str) -> KeyClass {
    if let Some(w) = WRITABLE_KEYS.iter().find(|w| w.key == key) {
        return KeyClass::Direct(w);
    }
    for class in PROPOSAL_CLASSES {
        if let Some(pk) = class.keys.iter().find(|k| k.key == key) {
            return KeyClass::Proposal(class, pk);
        }
    }
    if is_secret_key(key) {
        return KeyClass::Secret;
    }
    KeyClass::Unknown
}

/// The surface a human would change `key` on, for a key we do not manage.
///
/// A refusal that just says "unknown key" leaves the user stuck, and a silent
/// no-op that answers "Updated" is worse — that exact defect (report success,
/// change nothing) is why this function exists rather than a bare `Err(())`.
pub fn nearest_surface(key: &str) -> &'static str {
    let k = key.to_ascii_lowercase();
    if k.contains("sovereign") {
        "Settings → Sovereignty"
    } else if k.contains("budget") || k.contains("spend") || k.contains("cost") {
        "Settings → Spend"
    } else if k.contains("model") || k.contains("provider") || k.contains("goose_mode") {
        "Settings → Models"
    } else if k.contains("voice") || k.contains("tts") || k.contains("speech") {
        "Settings → Voice"
    } else if k.contains("watcher") || k.contains("news") {
        "Settings → Watcher"
    } else if k.contains("librarian") || k.contains("brain") || k.contains("memory") {
        "Settings → Librarian"
    } else if k.contains("extension") || k.contains("mcp") {
        "Settings → Search & tools"
    } else if k.contains("persona") || k.contains("identity") || k.contains("name") {
        "Settings → Persona"
    } else if k.ends_with("_enabled") || k.contains("feature") {
        "Settings → Features"
    } else {
        "Settings"
    }
}

fn compact(value: &Value) -> String {
    let s = serde_json::to_string(value).unwrap_or_else(|_| "<unserializable>".to_string());
    if s.chars().count() > 120 {
        format!("{}…", s.chars().take(120).collect::<String>())
    } else {
        s
    }
}

// ── configure_read ──────────────────────────────────────────────────────────

/// Current value of `key` as a display string, or `None` when it is unset.
fn current_value(key: &str) -> Option<String> {
    if key == LIBRARIAN_SCHEDULE_KEY {
        return read_librarian_schedule().as_ref().map(compact);
    }
    Config::global()
        .get_param::<Value>(key)
        .ok()
        .as_ref()
        .map(compact)
}

/// The Librarian schedule as stored on disk, read the same way
/// `librarian::resolve_model` reads it — by path, as untyped JSON, because the
/// typed struct lives in the daemon crate.
fn read_librarian_schedule() -> Option<Value> {
    let path = crate::config::paths::Paths::in_data_dir("librarian_schedule.json");
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<Value>(&contents).ok()
}

/// Render `configure_read` for the given keys (all managed keys when `None`).
///
/// The production function, not a copy of it: the tool handler is a thin
/// wrapper, so a test that calls this tests what the model calls.
pub fn configure_read_impl(keys: Option<&[String]>) -> String {
    let requested: Vec<String> = match keys {
        Some(k) if !k.is_empty() => k.to_vec(),
        _ => WRITABLE_KEYS
            .iter()
            .map(|w| w.key.to_string())
            .chain(
                PROPOSAL_CLASSES
                    .iter()
                    .flat_map(|c| c.keys.iter().map(|k| k.key.to_string())),
            )
            .collect(),
    };

    let mut out = String::from(
        "Current settings. Keys marked [direct] you can change yourself with configure_set; \
         keys marked [proposal-only] need configure_propose and the user's approval; secrets \
         report presence only — you can never read a secret value.\n\n",
    );
    for key in &requested {
        match classify(key) {
            KeyClass::Direct(w) => {
                let value = current_value(key)
                    .unwrap_or_else(|| "<not set — the default applies>".to_string());
                out.push_str(&format!(
                    "- {key} [direct] = {value} — {} ({}).\n",
                    w.what, w.surface
                ));
            }
            KeyClass::Proposal(class, _) => {
                let value = current_value(key)
                    .unwrap_or_else(|| "<not set — the default applies>".to_string());
                out.push_str(&format!(
                    "- {key} [proposal-only, class \"{}\"] = {value} ({}).\n",
                    class.id, class.surface
                ));
            }
            KeyClass::Secret => {
                // PRESENCE ONLY. Never `get_secret(...)` into the message.
                let present = Config::global().get_secret::<String>(key).is_ok();
                out.push_str(&format!(
                    "- {key} [secret] = {} — the value is never shown to you, and you cannot \
                     set it; the user enters it in {}.\n",
                    if present { "set" } else { "not set" },
                    nearest_surface(key)
                ));
            }
            KeyClass::Unknown => {
                out.push_str(&format!(
                    "- {key} — NOT a setting I manage. It is set in {}; say so rather than \
                     guessing at its value.\n",
                    nearest_surface(key)
                ));
            }
        }
    }
    out
}

// ── configure_set ───────────────────────────────────────────────────────────

/// Apply a direct write. `Err` is ALWAYS a refusal that changed nothing, and
/// every message says so in words the model cannot narrate as success.
///
/// The write itself is `Config::set_param` — the same writer
/// `POST /config/upsert` calls — so `config_changed` fires from the writer and
/// an open Settings pane refreshes. Nothing is emitted here.
pub async fn configure_set_impl(key: &str, value: &Value, reason: &str) -> Result<String, String> {
    if key.trim().is_empty() {
        return Err("NOT CHANGED — key must not be empty.".to_string());
    }
    if reason.trim().is_empty() {
        return Err(format!(
            "NOT CHANGED — reason must not be empty. A settings write on the user's machine \
             has to carry why you made it; say what the user asked for, in their words if you \
             have them. Nothing was written to {key}."
        ));
    }

    let writable = match classify(key) {
        KeyClass::Direct(w) => w,
        KeyClass::Proposal(class, _) => {
            let model_hint = if class.id == "provider_or_model" {
                " To switch the active local inference model specifically, use \
                 propose_model_upgrade."
            } else {
                ""
            };
            return Err(format!(
                "NOT CHANGED — \"{key}\" is in the \"{}\" class, which you may propose but never \
                 set yourself. Why: {} Use configure_propose with key_class \"{}\" to put it in \
                 front of the user; nothing changes until they approve it in the Decision Inbox. \
                 The user can also change it directly in {}.{model_hint}",
                class.id, class.why_gated, class.id, class.surface
            ));
        }
        KeyClass::Secret => {
            return Err(format!(
                "NOT CHANGED — \"{key}\" holds a credential. You can never write one, and a \
                 secret must not travel through a tool call at all. Ask the user to enter it in \
                 {}; you can confirm afterwards with configure_read, which reports presence only.",
                nearest_surface(key)
            ));
        }
        KeyClass::Unknown => {
            return Err(format!(
                "NOT CHANGED — \"{key}\" is not a setting I can write. It is set in {}. Tell the \
                 user that is where it lives; do NOT report this as updated.",
                nearest_surface(key)
            ));
        }
    };

    writable.shape.check(value).map_err(|why| {
        format!(
            "NOT CHANGED — \"{key}\" takes {}: {why}. Nothing was written.",
            writable.shape.describe()
        )
    })?;

    if key == LIBRARIAN_SCHEDULE_KEY {
        return set_librarian_schedule(value, reason).await;
    }

    let before = current_value(key).unwrap_or_else(|| "<not set>".to_string());
    Config::global()
        .set_param(key, value)
        .map_err(|e| format!("NOT CHANGED — the write to \"{key}\" failed: {e}."))?;

    // Read back through the same reader a restarted daemon uses, so the report
    // describes what actually persisted rather than what was sent.
    let after = current_value(key)
        .ok_or_else(|| format!("NOT SAVED — \"{key}\" reads back as unset after the write."))?;

    Ok(format!(
        "Saved: {key} was {before}, is now {after} (reason: {reason}). This went to the same \
         place {} writes, so it survives a restart and any open Settings pane has already \
         refreshed.",
        writable.surface
    ))
}

/// The Librarian arm: merge the requested fields into the stored schedule and
/// PUT the result at the daemon route, so `save_schedule` — with its 15..=720
/// duration check and `HH:MM` validation — stays the one writer.
///
/// Loopback HTTP rather than a direct call is not a preference: the writer is a
/// private function in the crate that depends on this one. This is the shape
/// `save_pronunciation` already uses for the same reason.
async fn set_librarian_schedule(value: &Value, reason: &str) -> Result<String, String> {
    let patch = value
        .as_object()
        .ok_or_else(|| "NOT CHANGED — librarian_schedule takes an object of fields.".to_string())?;
    if patch.is_empty() {
        return Err(
            "NOT CHANGED — no fields given. Pass the fields you want changed, e.g. \
             {\"start_time\": \"01:30\", \"duration_minutes\": 300}."
                .to_string(),
        );
    }
    const FIELDS: &[&str] = &[
        "enabled",
        "start_time",
        "duration_minutes",
        "model",
        "run_if_launched_in_window",
        "pruning_enabled",
    ];
    for name in patch.keys() {
        if !FIELDS.contains(&name.as_str()) {
            return Err(format!(
                "NOT CHANGED — \"{name}\" is not a Librarian schedule field. The fields are: {}.",
                FIELDS.join(", ")
            ));
        }
    }

    let mut merged = read_librarian_schedule()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    for (name, v) in patch {
        merged.insert(name.clone(), v.clone());
    }

    let client = reqwest::Client::new();
    let mut req = client
        .put("http://127.0.0.1:3001/api/librarian/schedule")
        .timeout(std::time::Duration::from_secs(10))
        .json(&Value::Object(merged));
    if let Some(token) = daemon_token().await {
        req = req.bearer_auth(token);
    }
    let resp = req.send().await.map_err(|e| {
        format!(
            "NOT CHANGED — could not reach the Librarian schedule service: {e}. Tell the user \
             the schedule was not changed; do not claim it was."
        )
    })?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "NOT CHANGED — the Librarian schedule service refused the write (HTTP {status}): \
             {body}. The window must start at HH:MM and run 15–720 minutes."
        ));
    }

    let stored = read_librarian_schedule()
        .map(|v| compact(&v))
        .unwrap_or_else(|| "<unreadable>".to_string());
    Ok(format!(
        "Saved: the Librarian schedule is now {stored} (reason: {reason}). This went through the \
         same route Settings → Librarian writes."
    ))
}

/// The daemon's own bearer token, read from disk. Same-trust: this tool runs
/// in-process in the daemon; the token only gets the loopback call past the
/// auth middleware. Copied in shape from `pronunciation::daemon_token`.
async fn daemon_token() -> Option<String> {
    let path = crate::config::paths::Paths::data_dir()
        .join("secrets")
        .join("daemon_token.json");
    let content = tokio::fs::read_to_string(path).await.ok()?;
    let parsed: Value = serde_json::from_str(&content).ok()?;
    Some(parsed.get("token")?.as_str()?.to_string())
}

// ── configure_propose ───────────────────────────────────────────────────────

/// Check that `key` really belongs to `key_class` and that `value` has the
/// shape that class's key takes.
///
/// Called TWICE on purpose: once when the card is filed, and again when the
/// approval applies it. The approval authorises WHAT the card says, not
/// whatever the stored payload happens to contain by then — the same
/// re-validate-at-apply discipline `write_regression_task` uses for paths.
pub fn validate_proposed_change(key_class: &str, key: &str, value: &Value) -> Result<(), String> {
    let class = PROPOSAL_CLASSES
        .iter()
        .find(|c| c.id == key_class)
        .ok_or_else(|| {
            format!(
                "\"{key_class}\" is not a proposable class. The classes are: {}.",
                PROPOSAL_CLASSES
                    .iter()
                    .map(|c| c.id)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
    let pk = class.keys.iter().find(|k| k.key == key).ok_or_else(|| {
        format!(
            "\"{key}\" is not in the \"{key_class}\" class. That class covers: {}.",
            class
                .keys
                .iter()
                .map(|k| k.key)
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;
    pk.shape
        .check(value)
        .map_err(|why| format!("\"{key}\" takes {}: {why}", pk.shape.describe()))
}

/// Apply an APPROVED proposal. Re-validates before writing, then goes through
/// `Config::set_param` — the same writer as every other path.
pub fn apply_proposed_change(key_class: &str, key: &str, value: &Value) -> Result<String, String> {
    validate_proposed_change(key_class, key, value)?;
    let before = current_value(key).unwrap_or_else(|| "<not set>".to_string());
    Config::global()
        .set_param(key, value)
        .map_err(|e| format!("failed to write {key}: {e}"))?;
    let after = current_value(key).unwrap_or_else(|| "<not set>".to_string());
    Ok(format!("{key} changed from {before} to {after}"))
}

/// The headline and detail a `config_change_proposal` card carries.
pub fn proposal_card_text(
    key_class: &str,
    key: &str,
    value: &Value,
    rationale: &str,
) -> (String, String) {
    let before = current_value(key).unwrap_or_else(|| "<not set>".to_string());
    let surface = PROPOSAL_CLASSES
        .iter()
        .find(|c| c.id == key_class)
        .map(|c| c.surface)
        .unwrap_or("Settings");
    (
        format!("Change {key} to {}", compact(value)),
        format!(
            "Your agent proposes changing {key} ({key_class}) from {before} to {}.\n\nWhy: \
             {rationale}\n\nApproving applies the change; rejecting writes nothing. You can also \
             change it yourself in {surface}.",
            compact(value)
        ),
    )
}

// ── the extension ───────────────────────────────────────────────────────────

pub struct ConfigureClient {
    info: InitializeResult,
    context: PlatformExtensionContext,
}

impl ConfigureClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(EXTENSION_NAME, "1.0.0").with_title("Configure"))
            .with_instructions(
                "Set up the user's home. configure_read shows what a setting is now (secrets: \
                 presence only, never the value). configure_set changes the low-risk settings \
                 yourself. configure_propose puts a sensitive one — autonomy, sovereignty, \
                 budget, provider/model — in front of the user as a Decision Inbox card, and \
                 NOTHING changes until they approve. If a key is in neither list, say which \
                 Settings pane it lives on instead of pretending to have changed it.",
            );
        Ok(Self { info, context })
    }

    /// `_meta.ui.visibility` excluding `"app"`.
    ///
    /// `POST /agent/call_tool` bypasses the tool-confirmation router entirely,
    /// and `is_tool_visible_to_app` — its only guard — returns TRUE when this
    /// field is absent. A settings writer that relied on that default would be
    /// callable, unconfirmed, by anything that can reach the daemon's HTTP
    /// surface. Declaring `["model"]` makes the guard refuse it (403) while
    /// leaving the tool where it belongs: in the model's tool list, behind the
    /// normal confirmation path.
    fn model_only() -> Meta {
        Meta(
            serde_json::json!({ "ui": { "visibility": ["model"] } })
                .as_object()
                .expect("visibility meta is an object")
                .clone(),
        )
    }

    fn schema(required: &[&str], properties: Value) -> JsonObject {
        serde_json::json!({
            "type": "object",
            "required": required,
            "properties": properties,
        })
        .as_object()
        .expect("schema is an object")
        .clone()
    }

    /// The full, static tool inventory. `list_tools` returns it verbatim, and
    /// `self_knowledge::extension_tool_inventories` derives from it, so a tool
    /// added here fails CI until the registry description names it.
    pub fn get_tools() -> Vec<Tool> {
        let writable = WRITABLE_KEYS
            .iter()
            .map(|w| w.key)
            .collect::<Vec<_>>()
            .join(", ");
        let classes = PROPOSAL_CLASSES
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>()
            .join(", ");

        vec![
            Tool::new(
                CONFIGURE_READ.to_string(),
                format!(
                    "Read the current value of the settings you can change or propose. Pass keys \
                     to read specific ones, or omit it for all of them. Secrets report PRESENCE \
                     ONLY — you can never read a secret value, and must not claim to. A key that \
                     is neither writable nor proposable is reported with the Settings pane it \
                     lives on. Read-only. Writable keys: {writable}. Proposal-only classes: \
                     {classes}."
                ),
                Self::schema(
                    &[],
                    serde_json::json!({
                        "keys": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Setting keys to read. Omit for all managed keys."
                        }
                    }),
                ),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Read settings".to_string()),
                Some(true),
                Some(false),
                Some(true),
                Some(false),
            ))
            .with_meta(Self::model_only()),
            Tool::new(
                CONFIGURE_SET.to_string(),
                format!(
                    "Change a low-risk setting in the user's home, for real and durably — it \
                     goes to the same place the Settings pane writes, so an open pane refreshes \
                     and the change survives a restart. You may set ONLY these keys: {writable}. \
                     Anything else is refused: sensitive settings ({classes}) need \
                     configure_propose and the user's approval, credentials you can never write, \
                     and an unknown key is refused with the Settings pane it actually lives on. \
                     A refusal always says NOT CHANGED — never report one as if it worked. \
                     reason must say why you are making the change, in the user's terms."
                ),
                Self::schema(
                    &["key", "value", "reason"],
                    serde_json::json!({
                        "key": {
                            "type": "string",
                            "description": "The setting key to write."
                        },
                        "value": {
                            "description": "The new value: true/false for a switch, a list of \
                                            strings for a list, an object of fields for \
                                            librarian_schedule."
                        },
                        "reason": {
                            "type": "string",
                            "description": "Why this change is being made, in the user's terms."
                        }
                    }),
                ),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Change a setting".to_string()),
                Some(false),
                Some(true),
                Some(false),
                Some(false),
            ))
            .with_meta(Self::model_only()),
            Tool::new(
                CONFIGURE_PROPOSE.to_string(),
                format!(
                    "Propose a change to a SENSITIVE setting. It files a Decision Inbox card and \
                     changes NOTHING until the user approves it there; rejecting writes nothing. \
                     Use it for the classes you must never set yourself: {classes} — autonomy is \
                     GOOSE_MODE (your own leash), sovereignty is whether data may leave this \
                     machine, budget is the spending ceilings, provider_or_model is who serves \
                     the user. To switch the active LOCAL inference model, use \
                     propose_model_upgrade instead — it checks the model is installed. \
                     Credentials cannot be proposed at all; ask the user to enter those in \
                     Settings. change is {{key, value}}: the exact key and the value approval \
                     will write."
                ),
                Self::schema(
                    &["key_class", "change", "rationale"],
                    serde_json::json!({
                        "key_class": {
                            "type": "string",
                            "enum": PROPOSAL_CLASSES.iter().map(|c| c.id).collect::<Vec<_>>(),
                            "description": "Which sensitive class the setting belongs to."
                        },
                        "change": {
                            "type": "object",
                            "required": ["key", "value"],
                            "properties": {
                                "key": {
                                    "type": "string",
                                    "description": "The setting key approval will write."
                                },
                                "value": {
                                    "description": "The value approval will write."
                                }
                            },
                            "description": "The exact change being proposed."
                        },
                        "rationale": {
                            "type": "string",
                            "description": "Why the user should approve it. Shown on the card."
                        }
                    }),
                ),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Propose a settings change".to_string()),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            ))
            .with_meta(Self::model_only()),
        ]
    }

    async fn handle_propose(&self, arguments: Option<JsonObject>) -> Result<String, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let key_class = args
            .get("key_class")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: key_class")?
            .trim()
            .to_string();
        let rationale = args
            .get("rationale")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        if rationale.is_empty() {
            return Err(
                "NOTHING PROPOSED — rationale must not be empty; it is what the user reads on \
                 the card."
                    .to_string(),
            );
        }
        let change = args
            .get("change")
            .and_then(|v| v.as_object())
            .ok_or("Missing required parameter: change (an object with key and value)")?;
        let key = change
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or("change.key is required")?
            .trim()
            .to_string();
        let value = change
            .get("value")
            .cloned()
            .ok_or("change.value is required")?;

        if is_secret_key(&key) && !matches!(classify(&key), KeyClass::Proposal(_, _)) {
            return Err(format!(
                "NOTHING PROPOSED — \"{key}\" holds a credential, and a secret must not travel \
                 through a tool call even as a proposal. Ask the user to enter it in {}.",
                nearest_surface(&key)
            ));
        }
        validate_proposed_change(&key_class, &key, &value)
            .map_err(|why| format!("NOTHING PROPOSED — {why}"))?;

        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())?;
        let (headline, detail) = proposal_card_text(&key_class, &key, &value, &rationale);
        let payload = crate::decisions::ConfigChangeProposalPayload {
            key_class: key_class.clone(),
            key: key.clone(),
            value: value.clone(),
            current_value: current_value(&key),
            rationale,
        };
        let decision = crate::decisions::create_decision(
            &pool,
            crate::decisions::NewDecision {
                kind: CONFIG_CHANGE_PROPOSAL_KIND.to_string(),
                headline: Some(crate::decisions::truncate_for_headline(&headline)),
                detail: Some(detail),
                payload: serde_json::to_value(&payload).map_err(|e| e.to_string())?,
                ..Default::default()
            },
        )
        .await
        .map_err(|e| e.to_string())?;
        if decision.kind == "malformed" {
            return Err(format!(
                "NOTHING PROPOSED — the proposal was rejected as malformed: {}",
                decision.detail
            ));
        }
        Ok(format!(
            "Proposed: {key} → {} — decision {} is waiting in the Decision Inbox. NOTHING has \
             changed. It changes only if the user approves it there; if they reject it, nothing \
             is written.",
            compact(&value),
            decision.id
        ))
    }
}

#[async_trait]
impl McpClientTrait for ConfigureClient {
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
        _ctx: &ToolCallContext,
        name: &str,
        arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<CallToolResult, Error> {
        let result: Result<String, String> = match name {
            CONFIGURE_READ => {
                let keys: Option<Vec<String>> = arguments
                    .as_ref()
                    .and_then(|a| a.get("keys"))
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|k| k.as_str().map(str::to_string))
                            .collect()
                    });
                Ok(configure_read_impl(keys.as_deref()))
            }
            CONFIGURE_SET => {
                let args = arguments.unwrap_or_default();
                let key = args.get("key").and_then(|v| v.as_str()).unwrap_or_default();
                let reason = args
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                match args.get("value") {
                    Some(value) => configure_set_impl(key, value, reason).await,
                    None => {
                        Err("NOT CHANGED — value is required. Nothing was written.".to_string())
                    }
                }
            }
            CONFIGURE_PROPOSE => self.handle_propose(arguments).await,
            other => Err(format!("Unknown tool: {other}")),
        };

        match result {
            Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
            Err(error) => Ok(CallToolResult::error(vec![Content::text(error)])),
        }
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::reply_parts::{is_tool_visible_to_app, is_tool_visible_to_model};
    use crate::agents::self_knowledge::{worker_gate, WORKER_DESCRIPTORS};

    fn tool(name: &str) -> Tool {
        ConfigureClient::get_tools()
            .into_iter()
            .find(|t| t.name.as_ref() == name)
            .unwrap_or_else(|| panic!("{name} is declared"))
    }

    /// The write tools must be UNREACHABLE from `POST /agent/call_tool`.
    ///
    /// FAILS BEFORE: with no `_meta.ui.visibility`, `is_tool_visible_to_app`
    /// returns TRUE — the guard fails open — and the route dispatches straight
    /// into the extension manager, skipping the tool-confirmation router
    /// entirely. A settings writer reachable that way is an unconfirmed
    /// settings writer for anything that can reach the daemon's HTTP surface.
    ///
    /// This asserts the exact predicate `routes/agent.rs::call_tool` calls, on
    /// the exact tools it would look up, so a future edit that drops the meta
    /// (or writes `["model", "app"]`) turns this red.
    #[test]
    fn every_tool_here_is_refused_on_the_agent_call_tool_path() {
        for name in [CONFIGURE_READ, CONFIGURE_SET, CONFIGURE_PROPOSE] {
            let t = tool(name);
            assert!(
                !is_tool_visible_to_app(&t),
                "{name} is callable through POST /agent/call_tool, which bypasses tool \
                 confirmation entirely"
            );
            assert!(
                is_tool_visible_to_model(&t),
                "{name} must stay in the model's tool list — hiding it from the model is not \
                 the hardening we wanted"
            );
        }
    }

    /// The hardening must survive the one mutation the pipeline really makes to
    /// `tool.meta`: `ExtensionManager::get_prefixed_tools` merges a
    /// `goose_extension` key into it on the way to `list_tools`, which is the
    /// list the route searches. A visibility that only held before that merge
    /// would be hardening the wrong object.
    #[test]
    fn the_refusal_survives_the_extension_manager_meta_merge() {
        for name in [CONFIGURE_READ, CONFIGURE_SET, CONFIGURE_PROPOSE] {
            let mut t = tool(name);
            let mut merged = t.meta.as_ref().map(|m| m.0.clone()).unwrap_or_default();
            merged.insert(
                "goose_extension".to_string(),
                Value::String(EXTENSION_NAME.to_string()),
            );
            t.meta = Some(Meta(merged));
            assert!(!is_tool_visible_to_app(&t), "{name} lost its app refusal");
            assert!(is_tool_visible_to_model(&t), "{name} lost model visibility");
        }
    }

    /// The write tools must land in the same confirmation class as
    /// `project_update`: `read_only_hint = Some(false)`, which is what
    /// `PermissionManager::apply_tool_annotations` reads to put a tool on the
    /// smart_approve ask-before list, and what keeps
    /// `PermissionInspector::inspect` out of the read-only allow branch.
    #[test]
    fn write_tools_carry_the_same_confirmation_class_as_project_update() {
        let reference =
            crate::agents::platform_extensions::project_manager::ProjectManagerClient::get_tools()
                .into_iter()
                .find(|t| t.name.as_ref() == "project_update")
                .expect("project_update is declared");
        let reference_read_only = reference
            .annotations
            .as_ref()
            .and_then(|a| a.read_only_hint);
        assert_eq!(
            reference_read_only,
            Some(false),
            "the reference write tool changed class; re-derive this test"
        );

        for name in [CONFIGURE_SET, CONFIGURE_PROPOSE] {
            let t = tool(name);
            assert_eq!(
                t.annotations.as_ref().and_then(|a| a.read_only_hint),
                Some(false),
                "{name} is a write and must be annotated as one, or it lands in the read-only \
                 allow branch and is never confirmed"
            );
        }
        assert_eq!(
            tool(CONFIGURE_READ)
                .annotations
                .as_ref()
                .and_then(|a| a.read_only_hint),
            Some(true),
            "configure_read is read-only and should not cost the user a prompt"
        );
    }

    /// The six feature keys in the allowlist must be BYTE-IDENTICAL to the
    /// `worker_gate` keys — the same strings Settings → Features and the
    /// self-knowledge brief switch on. A near-miss ("strix_enable") would write
    /// a key nothing reads and report success, which is the silent-no-op defect
    /// wearing a different hat.
    #[test]
    fn feature_gate_keys_match_worker_gate() {
        let mut gates: Vec<&str> = WORKER_DESCRIPTORS
            .iter()
            .filter_map(|d| worker_gate(d.id).map(|g| g.key))
            .collect();
        gates.sort_unstable();

        let mut mine: Vec<&str> = WRITABLE_KEYS
            .iter()
            .filter(|w| w.shape == Shape::Bool)
            .map(|w| w.key)
            .collect();
        mine.sort_unstable();

        assert_eq!(
            mine, gates,
            "the boolean half of the direct-write allowlist must be exactly the worker gate set"
        );
    }

    /// The Watcher's key constants are private to the daemon crate, so these
    /// two strings are duplicated here. Pin them, so a rename over there fails
    /// with a test that names the file to change.
    #[test]
    fn watcher_keys_match_the_reader() {
        assert_eq!(
            WATCHER_TOPICS_KEY, "watcher_topics",
            "must match WATCHER_TOPICS_KEY in goose-server/src/proactive.rs"
        );
        assert_eq!(
            WATCHER_MUTED_SUBJECTS_KEY, "watcher_muted_subjects",
            "must match WATCHER_MUTED_KEY in goose-server/src/proactive.rs"
        );
    }

    /// The allowlist is exactly what the spec authorised — no more. A key that
    /// creeps in here is a widening of what the agent may do without asking,
    /// so it has to be a deliberate edit to a named list.
    #[test]
    fn the_direct_write_allowlist_is_exactly_the_low_risk_set() {
        let mut keys: Vec<&str> = WRITABLE_KEYS.iter().map(|w| w.key).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "concierge_enabled",
                "council_enabled",
                "dev_roots",
                "initiative_enabled",
                "librarian_schedule",
                "playbook_enabled",
                "steward_scan_enabled",
                "strix_enabled",
                "watcher_muted_subjects",
                "watcher_topics",
            ]
        );
    }

    #[tokio::test]
    async fn set_refuses_goose_mode_and_points_at_the_proposal_path() {
        let before = Config::global().get_param::<Value>("GOOSE_MODE").ok();
        let err = configure_set_impl("GOOSE_MODE", &serde_json::json!("auto"), "faster")
            .await
            .expect_err("the agent must never set its own autonomy mode");

        assert!(err.starts_with("NOT CHANGED"), "got: {err}");
        assert!(err.contains("configure_propose"), "got: {err}");
        assert!(err.contains("autonomy"), "got: {err}");
        for lie in ["Saved:", "has been updated", "I've updated", "Updated "] {
            assert!(
                !err.contains(lie),
                "a refusal must never read as a success ({lie:?}): {err}"
            );
        }
        assert_eq!(
            Config::global().get_param::<Value>("GOOSE_MODE").ok(),
            before,
            "the refused write changed GOOSE_MODE anyway"
        );
    }

    #[tokio::test]
    async fn set_refuses_a_secret_without_ever_touching_its_value() {
        let err = configure_set_impl(
            "OPENAI_API_KEY",
            &serde_json::json!("sk-should-never-be-written"),
            "the user gave me a key",
        )
        .await
        .expect_err("a credential must never be written through a tool call");

        assert!(err.starts_with("NOT CHANGED"), "got: {err}");
        assert!(err.contains("credential"), "got: {err}");
        assert!(
            !err.contains("sk-should-never-be-written"),
            "the refusal echoed the secret back into the transcript: {err}"
        );
        assert!(
            Config::global()
                .get_param::<Value>("OPENAI_API_KEY")
                .is_err(),
            "a refused secret write reached config.yaml"
        );
    }

    #[tokio::test]
    async fn set_refuses_a_sovereignty_key_with_the_proposal_pointer() {
        let before = Config::global()
            .get_param::<Value>(crate::sovereignty::SOVEREIGN_MODE_KEY)
            .ok();
        let err = configure_set_impl(
            crate::sovereignty::SOVEREIGN_MODE_KEY,
            &serde_json::json!(false),
            "cloud models are better",
        )
        .await
        .expect_err("sovereign_mode is a data-boundary decision, not the agent's");

        assert!(err.starts_with("NOT CHANGED"), "got: {err}");
        assert!(err.contains("sovereignty"), "got: {err}");
        assert!(err.contains("configure_propose"), "got: {err}");
        assert_eq!(
            Config::global()
                .get_param::<Value>(crate::sovereignty::SOVEREIGN_MODE_KEY)
                .ok(),
            before
        );
    }

    /// The known past defect this closes: a no-op that answers "Updated".
    ///
    /// A key we do not manage must be refused BY NAME, with the pane it really
    /// lives on, and nothing may be written under it.
    #[tokio::test]
    async fn an_unknown_key_is_refused_by_naming_the_surface_and_writes_nothing() {
        const KEY: &str = "some_setting_we_do_not_manage_enabled";
        let err = configure_set_impl(KEY, &serde_json::json!(true), "user asked")
            .await
            .expect_err("an unmanaged key must be refused, not silently accepted");

        assert!(err.starts_with("NOT CHANGED"), "got: {err}");
        assert!(err.contains(KEY), "the refusal must name the key: {err}");
        assert!(
            err.contains("Settings → Features"),
            "the refusal must name the nearest surface: {err}"
        );
        // The lie this guards against is an AFFIRMATIVE claim, not the word
        // "updated" — the refusal deliberately ends by telling the model not to
        // report it as updated, which is the opposite failure.
        for lie in ["Saved:", "has been updated", "I've updated", "Updated "] {
            assert!(
                !err.contains(lie),
                "the silent-no-op defect is back — the refusal reads as a success ({lie:?}): {err}"
            );
        }
        assert!(
            Config::global().get_param::<Value>(KEY).is_err(),
            "the refused key was written anyway"
        );
    }

    #[tokio::test]
    async fn a_wrong_shaped_value_is_refused_rather_than_coerced() {
        let err = configure_set_impl(
            crate::config::dev_roots::DEV_ROOTS_KEY,
            &serde_json::json!("/Users/me/code"),
            "one root",
        )
        .await
        .expect_err("dev_roots takes a list, not a bare string");
        assert!(err.starts_with("NOT CHANGED"), "got: {err}");
        assert!(err.contains("a list of strings"), "got: {err}");
    }

    #[tokio::test]
    async fn a_write_without_a_reason_is_refused() {
        let err = configure_set_impl(
            crate::strix::STRIX_ENABLED_KEY,
            &serde_json::json!(true),
            "  ",
        )
        .await
        .expect_err("a settings write has to carry why");
        assert!(err.starts_with("NOT CHANGED"), "got: {err}");
        assert!(err.contains("reason"), "got: {err}");
    }

    /// `configure_read` reports a secret's PRESENCE and never its value.
    #[test]
    fn read_reports_secret_presence_and_never_the_value() {
        const SECRET: &str = "CONFIGURE_TEST_API_KEY";
        const VALUE: &str = "sk-configure-must-not-leak-0123456789";
        Config::global().set_secret(SECRET, &VALUE).unwrap();

        let out = configure_read_impl(Some(&[SECRET.to_string()]));
        assert!(out.contains(SECRET), "got: {out}");
        assert!(out.contains("[secret]"), "got: {out}");
        assert!(out.contains("= set"), "presence must be reported: {out}");
        assert!(
            !out.contains(VALUE),
            "configure_read leaked a secret value into the transcript: {out}"
        );
        assert!(
            !out.contains("sk-configure"),
            "configure_read leaked a secret prefix: {out}"
        );
        Config::global().delete_secret(SECRET).ok();
    }

    /// An unmanaged key asked about by name is answered with its real home, not
    /// with a guess at its value.
    #[test]
    fn read_answers_an_unmanaged_key_with_the_pane_it_lives_on() {
        let out = configure_read_impl(Some(&["sovereign_something_else".to_string()]));
        assert!(out.contains("NOT a setting I manage"), "got: {out}");
        assert!(out.contains("Settings → Sovereignty"), "got: {out}");
    }

    #[test]
    fn a_proposal_cannot_smuggle_a_key_from_another_class() {
        let err = validate_proposed_change("budget", "GOOSE_MODE", &serde_json::json!("auto"))
            .expect_err("GOOSE_MODE is not a budget key");
        assert!(err.contains("not in the \"budget\" class"), "got: {err}");

        let err = validate_proposed_change("autonomy", "GOOSE_MODE", &serde_json::json!("yolo"))
            .expect_err("yolo is not a GOOSE_MODE");
        assert!(
            err.contains("one of approve, chat, auto, smart_approve"),
            "got: {err}"
        );

        let err = validate_proposed_change("nonsense", "GOOSE_MODE", &serde_json::json!("auto"))
            .expect_err("an unknown class must be refused");
        assert!(err.contains("not a proposable class"), "got: {err}");

        validate_proposed_change("autonomy", "GOOSE_MODE", &serde_json::json!("approve"))
            .expect("a well-formed autonomy proposal validates");
    }

    /// Every proposal class the tool advertises must have at least one key, or
    /// the model is offered a class it can never file anything under.
    #[test]
    fn every_proposal_class_has_keys_and_a_surface() {
        for class in PROPOSAL_CLASSES {
            assert!(!class.keys.is_empty(), "class {} has no keys", class.id);
            assert!(
                class.surface.starts_with("Settings"),
                "class {} must name where the user does it themselves",
                class.id
            );
            assert!(
                !class.why_gated.is_empty(),
                "class {} says no reason",
                class.id
            );
        }
        assert!(
            PROPOSAL_CLASSES
                .iter()
                .find(|c| c.id == "provider_or_model")
                .expect("the provider/model class exists")
                .why_gated
                .contains("propose_model_upgrade"),
            "the model class must point at the existing model-specific path rather than \
             duplicating it"
        );
    }

    /// The secret detector has to catch the shapes credentials actually take
    /// here, and must not swallow ordinary keys.
    #[test]
    fn secret_detection_catches_credentials_and_leaves_settings_alone() {
        for key in [
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "TAVILY_API_KEY",
            "POLYMARKET_WALLET_PRIVATE_KEY",
            "SOME_SERVICE_TOKEN",
            "DB_PASSWORD",
        ] {
            assert!(is_secret_key(key), "{key} must be treated as a credential");
        }
        for key in [
            "strix_enabled",
            "dev_roots",
            "watcher_topics",
            "GOOSE_MODE",
            "GOOSE_MODEL",
            "sovereign_mode",
        ] {
            assert!(!is_secret_key(key), "{key} is not a credential");
        }
    }

    /// The tool descriptions the model reads must name the real allowlist and
    /// the real proposal classes, and must not promise more than the tool does.
    #[test]
    fn the_tool_descriptions_name_the_allowlist_and_do_not_overclaim() {
        let set = tool(CONFIGURE_SET).description.clone().unwrap_or_default();
        for w in WRITABLE_KEYS {
            assert!(
                set.contains(w.key),
                "configure_set's description must name {}, or the model cannot know it is \
                 writable",
                w.key
            );
        }
        let propose = tool(CONFIGURE_PROPOSE)
            .description
            .clone()
            .unwrap_or_default();
        for class in PROPOSAL_CLASSES {
            assert!(
                propose.contains(class.id),
                "configure_propose's description must name the {} class",
                class.id
            );
        }
        assert!(
            propose.contains("propose_model_upgrade"),
            "configure_propose must point at the existing model-specific path"
        );
        for text in [&set, &propose] {
            let lower = text.to_lowercase();
            for overclaim in ["any setting", "all settings", "every setting"] {
                assert!(
                    !lower.contains(overclaim),
                    "the description overclaims with {overclaim:?}: {text}"
                );
            }
        }
    }
}
