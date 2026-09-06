//! S2 (#428, epic #399): gate parser + session registry — pure Rust, zero
//! steady-state cost.
//!
//! S1 (#427, `supervised_cli.rs`) launches a gate-ENABLED stream-json Claude
//! Code session into a visible Build-tab PTY. This module is the deterministic
//! consumer of that session's output (spec: `docs/architecture/
//! CONTROL_LOOP_SPEC.md`, Pieces 2+3):
//!
//! - **Gate parser** ([`NdjsonScanner`]): an incremental NDJSON scanner fed
//!   raw `pty_data` chunks. It reassembles lines across chunk boundaries,
//!   strips the PTY's ANSI/OSC noise (zsh precmd hooks, CR/LF, cursor
//!   sequences), and classifies each JSON line into a [`StreamJsonEvent`].
//!   No LLM, no polling — it is idle until a chunk arrives.
//! - **Session registry** ([`register_session`] / [`ingest_output`]): the
//!   bridge the S0 reconciliation demanded — `{supervised session id →
//!   (kind/goal-ness, project, root path, live PTY relay address, pending
//!   gate request_ids)}`. The PTY session id (`pty-<uuid>`) is recorded as
//!   the RELAY ADDRESS for S5 (`write_to_pty` is the only writable handle to
//!   the session's stdin — the daemon holds no process handle).
//!
//! On a `control_request`/`can_use_tool` line the registry emits a structured
//! [`crate::events::terminal_gate_detected`] event to the Permagent bus and
//! records the pending gate. On an observed `control_response` (today: the PTY
//! echo of a hand-typed answer; from S5: the relayed answer) the pending gate
//! is cleared and [`crate::events::terminal_gate_cleared`] is emitted. On the
//! session's `type:"result"` line the registry fulfils S1's completion seam
//! ([`super::supervised_cli::complete_supervised_session`]) so a dispatched
//! goal resolves instead of parking at its timeout.
//!
//! S3 (#429) adds the gate → Decision-Inbox bridge here
//! ([`bridge_report_to_inbox`]): every gate the parser detects becomes a
//! `session_gate` decision (Tier 2 fail-closed — user-only until S4's
//! classification), and a gate that resolves outside the inbox (hand-typed
//! answer observed in the PTY echo, or session end) supersedes its open card
//! so the inbox never shows a zombie.
//!
//! S4 (#429→#430) classifies each gate's tool into a `risk_policy`
//! action_class ([`super::gate_classifier`]) so the bridge files it at the
//! correct tier instead of a blanket Tier 2 — read-only tools at Tier 0,
//! confined edits at Tier 1, shell/network/unrecognized tools at Tier 2
//! (fail-closed). The classification is the ONLY S4 change here; the tier
//! itself is resolved by the existing `risk_policy` machinery.
//!
//! What this module deliberately does NOT do (later slices, do not fold in):
//! - No relay of answers into stdin (S5, #431) — answering the decision
//!   records the ruling and surfaces the exact `control_response` line to
//!   hand-type into the session's visible tab (the S1/S2 escape hatch).

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::supervised_cli::{complete_supervised_session, SupervisedOutcome};

// ── Gate parser ─────────────────────────────────────────────────────────────

/// Upper bound on a buffered partial line. Real protocol lines are small (a
/// gate carries one tool call's input); anything growing past this without a
/// newline is TUI noise or an attack, and is dropped whole rather than letting
/// the buffer grow without bound.
const MAX_LINE_BYTES: usize = 256 * 1024;

/// A classified NDJSON line from a supervised session's output stream.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamJsonEvent {
    /// `{"type":"control_request","request":{"subtype":"can_use_tool",…}}` —
    /// the session is BLOCKED waiting for a permission answer. The heart of
    /// the loop.
    Gate {
        request_id: String,
        tool_name: String,
        /// The tool's input object, verbatim — S3 carries it into the inbox
        /// payload, S4 classifies it, S5 echoes it back in the `allow`.
        input: Value,
        tool_use_id: String,
    },
    /// `{"type":"control_response",…}` observed in the OUTPUT stream — the PTY
    /// echo of an answer written to the session's stdin (hand-typed today,
    /// S5-relayed later). Clears the matching pending gate.
    GateAnswered { request_id: String },
    /// `{"type":"result",…}` with a success subtype — the session finished.
    Completed { summary: String },
    /// `{"type":"result",…}` with `is_error`/an error subtype, or a top-level
    /// `{"type":"error",…}` — the session finished abnormally.
    Failed { reason: String },
}

/// Incremental, chunk-boundary-safe NDJSON scanner over raw PTY output.
///
/// PTY reality this is built for (all covered by tests):
/// - chunks split anywhere, including mid-JSON-line and mid-escape;
/// - `\r\n` line endings (ONLCR) and stray `\r`;
/// - ANSI CSI / OSC sequences interleaved by the shell (the Build-tab zsh
///   emits OSC 7 + OSC 133 precmd hooks) and by `clear`;
/// - non-JSON lines (the echoed launch command, shell prompts) — ignored;
/// - JSON lines not starting at column 0 (cursor-position artifacts) — the
///   scanner falls back to the first `{"type":"` in the line.
#[derive(Debug, Default)]
pub struct NdjsonScanner {
    buf: String,
}

impl NdjsonScanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk of raw PTY output; returns every protocol event completed
    /// by this chunk, in stream order.
    pub fn feed(&mut self, chunk: &str) -> Vec<StreamJsonEvent> {
        let mut events = Vec::new();
        self.buf.push_str(chunk);
        while let Some(pos) = self.buf.find('\n') {
            // `pos` is a char boundary ('\n' is ASCII) — `.get` keeps the
            // workspace `clippy::string_slice` lint honest anyway.
            let line = self.buf.get(..pos).unwrap_or_default().to_string();
            let rest = self.buf.get(pos + 1..).unwrap_or_default().to_string();
            self.buf = rest;
            if let Some(ev) = classify_line(&line) {
                events.push(ev);
            }
        }
        if self.buf.len() > MAX_LINE_BYTES {
            tracing::warn!(
                buffered = self.buf.len(),
                "supervised-session scanner dropped an oversized partial line (no newline within cap)"
            );
            self.buf.clear();
        }
        events
    }
}

/// Strip ANSI escape sequences (CSI, OSC, single-char escapes) and control
/// characters from one line of PTY output. OSC payloads terminate on BEL or
/// ST (`ESC \`). Deterministic char-level state machine — no regex.
fn strip_ansi(line: &str) -> String {
    #[derive(PartialEq)]
    enum St {
        Plain,
        Esc,
        Csi,
        Osc,
        OscEsc,
    }
    let mut out = String::with_capacity(line.len());
    let mut st = St::Plain;
    for c in line.chars() {
        match st {
            St::Plain => match c {
                '\u{1b}' => st = St::Esc,
                c if c.is_control() => {}
                c => out.push(c),
            },
            St::Esc => match c {
                '[' => st = St::Csi,
                ']' => st = St::Osc,
                _ => st = St::Plain,
            },
            St::Csi => {
                // CSI terminates on a "final byte" 0x40–0x7e.
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    st = St::Plain;
                }
            }
            St::Osc => match c {
                '\u{07}' => st = St::Plain,
                '\u{1b}' => st = St::OscEsc,
                _ => {}
            },
            St::OscEsc => {
                // `ESC \` (ST) ends the OSC; anything else returns to the OSC
                // body (defensive — OSC payloads never contain a bare ESC).
                st = if c == '\\' { St::Plain } else { St::Osc };
            }
        }
    }
    out
}

/// Extract the JSON candidate from a cleaned line: the whole line when it
/// starts with `{`, else the suffix from the first `{"type":"` (cursor
/// artifacts can leave a prefix). Returns `None` for plainly non-JSON lines.
fn json_candidate(clean: &str) -> Option<&str> {
    let trimmed = clean.trim();
    if trimmed.starts_with('{') {
        return Some(trimmed);
    }
    clean.find("{\"type\":\"").and_then(|idx| clean.get(idx..))
}

/// Classify one raw PTY line. Returns `None` for anything that is not a
/// protocol event we act on (assistant/system deltas, shell noise, …).
fn classify_line(raw: &str) -> Option<StreamJsonEvent> {
    let clean = strip_ansi(raw);
    let candidate = json_candidate(&clean)?;
    let parsed: Value = serde_json::from_str(candidate).ok()?;
    let msg_type = parsed.get("type")?.as_str()?;
    match msg_type {
        "control_request" => {
            let request = parsed.get("request")?;
            if request.get("subtype")?.as_str()? != "can_use_tool" {
                return None;
            }
            Some(StreamJsonEvent::Gate {
                request_id: parsed.get("request_id")?.as_str()?.to_string(),
                tool_name: request.get("tool_name")?.as_str()?.to_string(),
                input: request
                    .get("input")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(serde_json::Map::new())),
                tool_use_id: request
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            })
        }
        "control_response" => {
            let request_id = parsed
                .get("response")
                .and_then(|r| r.get("request_id"))
                .and_then(Value::as_str)?
                .to_string();
            Some(StreamJsonEvent::GateAnswered { request_id })
        }
        "result" => {
            let is_error = parsed
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let subtype = parsed
                .get("subtype")
                .and_then(Value::as_str)
                .unwrap_or("success");
            let text = parsed
                .get("result")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if is_error || subtype.starts_with("error") {
                let reason = if text.is_empty() {
                    format!("session ended with result subtype '{subtype}'")
                } else {
                    text
                };
                Some(StreamJsonEvent::Failed { reason })
            } else {
                Some(StreamJsonEvent::Completed { summary: text })
            }
        }
        "error" => {
            let reason = parsed
                .get("message")
                .or_else(|| parsed.get("error"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| candidate.to_string());
            Some(StreamJsonEvent::Failed { reason })
        }
        _ => None,
    }
}

// ── Session registry ────────────────────────────────────────────────────────

/// How the supervised session was launched (the "both entry points"
/// ruling). Dispatched goals additionally resolve S1's completion hook so the
/// goal tracker records Success/Failed instead of parking at timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisedSessionKind {
    /// `SupervisedCliEngine::spawn` — worktree + goal card + review. The
    /// session id doubles as the goal's `run_id`/`worker_session_id`.
    DispatchedGoal,
    /// `launch_watched_session` — "just run CC", no goal wrapping.
    Watched,
}

/// Lifecycle of a registered session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisedStatus {
    /// Registered at launch; no PTY output seen yet.
    Launched,
    /// PTY attached (output flowing) — the session is observable and, from
    /// S5 on, addressable.
    Attached,
    /// `type:"result"` (success) observed.
    Completed,
    /// Error result / stream error / PTY closed before a result.
    Failed,
}

/// A gate the session is (or was) blocked on, as recorded in the registry.
#[derive(Debug, Clone, Serialize)]
pub struct PendingGate {
    pub request_id: String,
    pub tool_name: String,
    pub input: Value,
    pub tool_use_id: String,
    pub detected_at: DateTime<Utc>,
}

struct SessionState {
    kind: SupervisedSessionKind,
    project_slug: String,
    root_path: String,
    pty_session_id: Option<String>,
    status: SupervisedStatus,
    pending: Vec<PendingGate>,
    scanner: NdjsonScanner,
    last_summary: Option<String>,
}

// ── Live harness-run projection ───────────────────────────────────────────

/// A bounded, machine-readable projection of one in-flight coding harness
/// run. This deliberately contains no PTY transcript or prompt body: callers
/// get stable operational state, while the terminal remains the place for raw
/// output.
///
/// The projection lives beside supervised sessions because both are
/// short-lived daemon-local execution state. It is not a second source of
/// truth for spend or session history: the daemon route hydrates those values
/// from the canonical session ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessRunStatus {
    Queued,
    Running,
    Verifying,
    WaitingGate,
    Succeeded,
    Failed,
    Cancelled,
}

impl HarnessRunStatus {
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Running | Self::Verifying | Self::WaitingGate
        )
    }
}

/// One declared or observed verification command. Its text is capped at the
/// write boundary so an untrusted harness cannot turn observability into an
/// unbounded log sink.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HarnessVerification {
    pub command: String,
    pub verdict: Option<String>,
}

/// A pending user/action gate without the tool input payload. Full gate input
/// remains in the existing supervised-session/Decision-Inbox paths.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessPendingGate {
    pub request_id: String,
    pub tool_name: String,
    pub tier: Option<String>,
}

/// Write contract for the coding harness. Each request is a full snapshot;
/// `run_id` is its idempotency key. The prompt is deliberately available to
/// the local daemon: Henry and the Council recommender need the user's actual
/// request, not a lossy title, to understand a live Build run. It is bounded,
/// never logged here, and never leaves the local provider boundary except when
/// the user explicitly invokes the Council.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct HarnessRunUpdate {
    pub run_id: String,
    pub session_id: String,
    pub project: String,
    pub prompt_title: String,
    pub prompt_digest: String,
    /// Version of the task/fixture contract used for this run.
    #[serde(default)]
    pub task_version: Option<String>,
    #[serde(default)]
    pub envelope_id: Option<String>,
    #[serde(default)]
    pub prompt_context: Option<String>,
    #[serde(default)]
    pub dag_nodes: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub active_node: Option<String>,
    #[serde(default)]
    pub worker: Option<String>,
    /// Provider selected for the worker (kept separate from the model name).
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub billing_class: Option<String>,
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub routing_reason: Option<String>,
    pub status: HarnessRunStatus,
    #[serde(default)]
    pub declared_verification: Option<HarnessVerification>,
    #[serde(default)]
    pub last_verification: Option<HarnessVerification>,
    #[serde(default)]
    pub verification_attempts: Option<u32>,
    #[serde(default)]
    pub verification_verdict: Option<String>,
    #[serde(default)]
    pub pending_gate: Option<HarnessPendingGate>,
    /// Explicit counters make retries, tool activity, and gate attempts
    /// inspectable without parsing a transcript.
    #[serde(default)]
    pub retry_count: Option<u32>,
    #[serde(default)]
    pub tool_calls: Option<u32>,
    #[serde(default)]
    pub gate_attempts: Option<u32>,
    /// Bounded terminal evidence/result summaries; raw output stays elsewhere.
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default)]
    pub result: Option<String>,
    /// Parent run id for child-node attribution.
    #[serde(default)]
    pub parent_run_id: Option<String>,
    /// Durable parent session identity. Kept distinct from parent_run_id:
    /// sessions and harness runs are different identity domains.
    #[serde(default)]
    pub parent_session_id: Option<String>,
}

/// Read contract for a harness run. `elapsed_ms` is calculated at read time;
/// no ticking task or polling loop is needed to keep it current.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessRunSnapshot {
    pub run_id: String,
    pub session_id: String,
    pub project: String,
    pub prompt_title: String,
    pub prompt_digest: String,
    pub task_version: Option<String>,
    pub envelope_id: Option<String>,
    pub prompt_context: Option<String>,
    pub council_recommendation: CouncilRecommendation,
    pub dag_nodes: Vec<String>,
    pub dependencies: Vec<String>,
    pub active_node: Option<String>,
    pub worker: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub billing_class: Option<String>,
    pub tier: Option<String>,
    pub routing_reason: Option<String>,
    pub status: HarnessRunStatus,
    pub declared_verification: Option<HarnessVerification>,
    pub last_verification: Option<HarnessVerification>,
    pub verification_attempts: Option<u32>,
    pub verification_verdict: Option<String>,
    pub pending_gate: Option<HarnessPendingGate>,
    pub retry_count: Option<u32>,
    pub tool_calls: Option<u32>,
    pub gate_attempts: Option<u32>,
    pub evidence: Option<String>,
    pub result: Option<String>,
    pub parent_run_id: Option<String>,
    pub parent_session_id: Option<String>,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub elapsed_ms: i64,
}

struct HarnessRunState {
    update: HarnessRunUpdate,
    started_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    verification_attempts: Option<u32>,
    council_suggestion_emitted: bool,
}

const MAX_HARNESS_RUNS: usize = 64;
const MAX_DAG_ITEMS: usize = 64;
const MAX_FIELD_CHARS: usize = 512;
const MAX_PROMPT_CONTEXT_CHARS: usize = 24_000;
/// A running projection must prove liveness. The CLI refreshes every 15s;
/// three missed beats removes it from the active view after a crash or kill.
const HARNESS_ACTIVE_TTL_SECS: i64 = 45;

fn bounded_harness_field(value: &str) -> String {
    value.chars().take(MAX_FIELD_CHARS).collect()
}

fn bounded_prompt_context(value: &str) -> String {
    value.chars().take(MAX_PROMPT_CONTEXT_CHARS).collect()
}

/// A deterministic, zero-token preflight. This does not decide to spend; it
/// only decides whether offering the Council is useful. The explicit user
/// click remains the spend/approval boundary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CouncilRecommendation {
    pub recommended: bool,
    pub reason: String,
    pub signals: Vec<String>,
}

pub fn recommend_council(prompt: &str) -> CouncilRecommendation {
    let lower = prompt.to_ascii_lowercase();
    let declines = [
        "do not use the council",
        "don't use the council",
        "without the council",
        "skip the council",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    if declines {
        return CouncilRecommendation {
            recommended: false,
            reason: "The request explicitly declines Council review.".to_string(),
            signals: vec!["explicit-opt-out".to_string()],
        };
    }

    let mut signals = Vec::new();
    let groups: &[(&str, &[&str])] = &[
        (
            "explicit-council",
            &["council", "multiple models", "multi-model"],
        ),
        (
            "architecture",
            &["architecture", "architect", "system design", "migration"],
        ),
        (
            "orchestration",
            &["orchestrat", "dag", "routing", "worker", "provider"],
        ),
        (
            "cross-cutting",
            &[
                "end-to-end",
                "cross-cutting",
                "across the app",
                "backend and",
                "server and",
                "ios and",
                "web and",
            ],
        ),
        (
            "risk",
            &[
                "security",
                "privacy",
                "permission",
                "billing",
                "production",
                "destructive",
                "high trust",
            ],
        ),
        (
            "research",
            &[
                "research",
                "compare approaches",
                "best practice",
                "validate",
            ],
        ),
        (
            "tradeoffs",
            &[
                "tradeoff",
                "trade-off",
                "ambiguous",
                "options",
                "strategy",
                "plan",
            ],
        ),
    ];
    for (name, needles) in groups {
        if needles.iter().any(|needle| lower.contains(needle)) {
            signals.push((*name).to_string());
        }
    }

    let explicit = signals.iter().any(|s| s == "explicit-council");
    let planning = signals.iter().any(|s| {
        matches!(
            s.as_str(),
            "architecture" | "orchestration" | "tradeoffs" | "research"
        )
    });
    let broad = prompt.chars().count() >= 900 || lower.matches('\n').count() >= 5;
    let recommended = explicit || (planning && signals.len() >= 2) || (broad && signals.len() >= 3);
    CouncilRecommendation {
        recommended,
        reason: if recommended {
            format!(
                "Council review may help because this request spans {}.",
                signals.join(", ")
            )
        } else {
            "A single routed worker should handle this request without Council overhead."
                .to_string()
        },
        signals,
    }
}

fn validate_harness_update(update: &HarnessRunUpdate) -> Result<(), String> {
    for (name, value) in [
        ("run_id", update.run_id.as_str()),
        ("session_id", update.session_id.as_str()),
        ("project", update.project.as_str()),
        ("prompt_title", update.prompt_title.as_str()),
        ("prompt_digest", update.prompt_digest.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(format!("harness run requires a non-empty '{name}'"));
        }
        if value.chars().count() > MAX_FIELD_CHARS {
            return Err(format!(
                "harness run field '{name}' exceeds {MAX_FIELD_CHARS} chars"
            ));
        }
    }
    if update.dag_nodes.len() > MAX_DAG_ITEMS || update.dependencies.len() > MAX_DAG_ITEMS {
        return Err(format!(
            "harness DAG is limited to {MAX_DAG_ITEMS} nodes/dependencies"
        ));
    }

    // This endpoint represents harness WORK, not an unstructured terminal
    // process. A one-node graph is valid for a tiny task, but an empty or
    // malformed graph is not: accepting one made the recipe's always-DAG
    // promise advisory and left Build/Henry with nothing to supervise.
    if update.dag_nodes.is_empty() {
        return Err("harness run requires at least one DAG node".to_string());
    }
    let mut node_index = HashMap::new();
    for (index, raw) in update.dag_nodes.iter().enumerate() {
        let node = raw.trim();
        if node.is_empty() {
            return Err("harness DAG nodes must be non-empty".to_string());
        }
        if node.contains("->") {
            return Err("harness DAG node names cannot contain '->'".to_string());
        }
        if node_index.insert(node, index).is_some() {
            return Err(format!("harness DAG contains duplicate node '{node}'"));
        }
    }

    let mut in_degree = vec![0usize; update.dag_nodes.len()];
    let mut adjacency = vec![Vec::<usize>::new(); update.dag_nodes.len()];
    for raw in &update.dependencies {
        let Some((from, to)) = raw.split_once("->") else {
            return Err(format!(
                "harness dependency '{raw}' must use the 'from->to' form"
            ));
        };
        let (from, to) = (from.trim(), to.trim());
        let Some(&from_index) = node_index.get(from) else {
            return Err(format!(
                "harness dependency references unknown node '{from}'"
            ));
        };
        let Some(&to_index) = node_index.get(to) else {
            return Err(format!("harness dependency references unknown node '{to}'"));
        };
        if from_index == to_index {
            return Err(format!("harness DAG node '{from}' depends on itself"));
        }
        adjacency[from_index].push(to_index);
        in_degree[to_index] += 1;
    }

    let mut ready = std::collections::VecDeque::new();
    for (index, degree) in in_degree.iter().enumerate() {
        if *degree == 0 {
            ready.push_back(index);
        }
    }
    let mut visited = 0usize;
    while let Some(node) = ready.pop_front() {
        visited += 1;
        for dependent in &adjacency[node] {
            in_degree[*dependent] -= 1;
            if in_degree[*dependent] == 0 {
                ready.push_back(*dependent);
            }
        }
    }
    if visited != update.dag_nodes.len() {
        return Err("harness DAG contains a dependency cycle".to_string());
    }

    match (update.status.is_active(), update.active_node.as_deref()) {
        (true, Some(active)) if !node_index.contains_key(active.trim()) => {
            return Err(format!("active DAG node '{active}' is not in the graph"));
        }
        (true, None) => return Err("active harness run requires an active DAG node".to_string()),
        (false, Some(_)) => {
            return Err("terminal harness run cannot claim an active DAG node".to_string())
        }
        _ => {}
    }
    Ok(())
}

fn bounded_optional(value: Option<String>) -> Option<String> {
    value.map(|value| bounded_harness_field(&value))
}

fn bounded_verification(value: Option<HarnessVerification>) -> Option<HarnessVerification> {
    value.map(|verification| HarnessVerification {
        command: bounded_harness_field(&verification.command),
        verdict: bounded_optional(verification.verdict),
    })
}

fn snapshot_harness_run(state: &HarnessRunState) -> HarnessRunSnapshot {
    let now = Utc::now();
    HarnessRunSnapshot {
        run_id: state.update.run_id.clone(),
        session_id: state.update.session_id.clone(),
        project: state.update.project.clone(),
        prompt_title: state.update.prompt_title.clone(),
        prompt_digest: state.update.prompt_digest.clone(),
        task_version: state.update.task_version.clone(),
        envelope_id: state.update.envelope_id.clone(),
        prompt_context: state.update.prompt_context.clone(),
        council_recommendation: recommend_council(
            state.update.prompt_context.as_deref().unwrap_or_default(),
        ),
        dag_nodes: state.update.dag_nodes.clone(),
        dependencies: state.update.dependencies.clone(),
        active_node: state.update.active_node.clone(),
        worker: state.update.worker.clone(),
        provider: state.update.provider.clone(),
        model: state.update.model.clone(),
        billing_class: state.update.billing_class.clone(),
        tier: state.update.tier.clone(),
        routing_reason: state.update.routing_reason.clone(),
        status: state.update.status,
        declared_verification: state.update.declared_verification.clone(),
        last_verification: state.update.last_verification.clone(),
        verification_attempts: state.verification_attempts,
        verification_verdict: state.update.verification_verdict.clone(),
        pending_gate: state.update.pending_gate.clone(),
        retry_count: state.update.retry_count,
        tool_calls: state.update.tool_calls,
        gate_attempts: state.update.gate_attempts,
        evidence: state.update.evidence.clone(),
        result: state.update.result.clone(),
        parent_run_id: state.update.parent_run_id.clone(),
        parent_session_id: state.update.parent_session_id.clone(),
        started_at: state.started_at,
        updated_at: state.updated_at,
        elapsed_ms: (now - state.started_at).num_milliseconds().max(0),
    }
}

/// Read-only snapshot of a registry entry (the GET route / tests / S3 view).
#[derive(Debug, Clone, Serialize)]
pub struct SessionSnapshot {
    pub session_id: String,
    pub kind: SupervisedSessionKind,
    pub project_slug: String,
    pub root_path: String,
    pub pty_session_id: Option<String>,
    pub status: SupervisedStatus,
    pub pending_gates: Vec<PendingGate>,
    pub last_summary: Option<String>,
}

/// A gate cleared during one ingest call: the request id plus why it cleared
/// (`answered` — a `control_response` echo was observed; `session_ended` — the
/// session finished with the gate still pending).
#[derive(Debug, Clone, Serialize)]
pub struct ClearedGate {
    pub request_id: String,
    pub reason: String,
}

/// What one [`ingest_output`] call did — the route returns this verbatim so
/// the tee side (and tests) get evidence, not silence. S3 widened it beyond
/// counts: `detected`/`cleared` carry the per-gate substance the inbox bridge
/// ([`bridge_report_to_inbox`]) files/supersedes decisions from, so the bridge
/// acts on exactly what THIS call observed (no racy registry re-read).
#[derive(Debug, Default, Serialize)]
pub struct IngestReport {
    pub gates_detected: usize,
    pub gates_cleared: usize,
    pub completed: bool,
    pub failed: bool,
    pub detected: Vec<PendingGate>,
    pub cleared: Vec<ClearedGate>,
}

/// The process-wide registry. Global like S1's completion hooks and the event
/// bus: supervised sessions are a per-daemon concern, not per-AppState.
#[derive(Default)]
struct Registry {
    sessions: HashMap<String, SessionState>,
    harness_runs: HashMap<String, HarnessRunState>,
}

static REGISTRY: Lazy<Mutex<Registry>> = Lazy::new(|| Mutex::new(Registry::default()));

fn with_registry<R>(f: impl FnOnce(&mut Registry) -> R) -> R {
    // A poisoned lock here would mean a panic while holding the registry —
    // recover the data rather than cascading (same posture as the event bus).
    let mut guard = match REGISTRY.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    f(&mut guard)
}

/// Register a freshly launched supervised session. Called by BOTH S1 launch
/// paths BEFORE the `project_launch` event is emitted, so an attach can never
/// race an unknown session. Re-registering an id is a bug — logged, ignored.
pub fn register_session(
    session_id: &str,
    kind: SupervisedSessionKind,
    project_slug: &str,
    root_path: &str,
) {
    with_registry(|map| {
        if map.sessions.contains_key(session_id) {
            tracing::warn!(
                session_id,
                "supervised session already registered — ignoring"
            );
            return;
        }
        map.sessions.insert(
            session_id.to_string(),
            SessionState {
                kind,
                project_slug: project_slug.to_string(),
                root_path: root_path.to_string(),
                pty_session_id: None,
                status: SupervisedStatus::Launched,
                pending: Vec::new(),
                scanner: NdjsonScanner::new(),
                last_summary: None,
            },
        );
    });
}

/// Record the PTY the frontend spawned for this session — the S5 relay
/// address. Idempotent; returns `false` for an unknown session. A conflicting
/// re-attach (different PTY id) is taken as the tab respawning its PTY and
/// overwrites with a warning — the newest PTY is the live one.
pub fn attach_pty(session_id: &str, pty_session_id: &str) -> bool {
    with_registry(|map| match map.sessions.get_mut(session_id) {
        Some(state) => {
            match &state.pty_session_id {
                Some(existing) if existing != pty_session_id => {
                    tracing::warn!(
                        session_id,
                        old = existing.as_str(),
                        new = pty_session_id,
                        "supervised session re-attached to a different PTY"
                    );
                }
                _ => {}
            }
            state.pty_session_id = Some(pty_session_id.to_string());
            if state.status == SupervisedStatus::Launched {
                state.status = SupervisedStatus::Attached;
            }
            true
        }
        None => false,
    })
}

/// Resolve a registry key from whichever id the caller holds: the supervised
/// session id wins; else reverse-lookup by attached PTY id.
pub fn resolve_session_id(
    supervised_session_id: Option<&str>,
    pty_session_id: Option<&str>,
) -> Option<String> {
    with_registry(|map| {
        if let Some(sid) = supervised_session_id {
            if map.sessions.contains_key(sid) {
                return Some(sid.to_string());
            }
        }
        if let Some(pty) = pty_session_id {
            return map
                .sessions
                .iter()
                .find(|(_, s)| s.pty_session_id.as_deref() == Some(pty))
                .map(|(id, _)| id.clone());
        }
        None
    })
}

/// Feed a chunk of the session's PTY output through the scanner and act on
/// every completed protocol event:
///
/// - gate → record pending + `terminal_gate_detected` on the bus;
/// - observed answer → clear pending + `terminal_gate_cleared` (`answered`);
/// - result → mark Completed/Failed, clear remaining gates
///   (`session_ended`), fulfil S1's completion seam (dispatched goals);
/// - `eof` (PTY closed) without a prior result → Failed via the same seam,
///   so a goal whose tab dies resolves instead of parking at timeout.
///
/// Returns `None` for an unknown session (the caller answers 404 and the tee
/// stops). Steady-state cost when no chunk arrives: zero — nothing polls.
pub fn ingest_output(session_id: &str, data: &str, eof: bool) -> Option<IngestReport> {
    // Parse + mutate under the lock; emit bus events and resolve the
    // completion seam after releasing it (bus subscribers must never be able
    // to re-enter the registry against a held lock).
    struct Actions {
        report: IngestReport,
        detected: Vec<PendingGate>,
        cleared: Vec<(String, &'static str)>,
        outcome: Option<SupervisedOutcome>,
        project_slug: String,
        kind: SupervisedSessionKind,
        root_path: String,
        pty_session_id: Option<String>,
    }

    let actions = with_registry(|map| {
        let state = map.sessions.get_mut(session_id)?;
        let mut acts = Actions {
            report: IngestReport::default(),
            detected: Vec::new(),
            cleared: Vec::new(),
            outcome: None,
            project_slug: state.project_slug.clone(),
            kind: state.kind,
            root_path: state.root_path.clone(),
            pty_session_id: state.pty_session_id.clone(),
        };
        let already_over = matches!(
            state.status,
            SupervisedStatus::Completed | SupervisedStatus::Failed
        );
        for ev in state.scanner.feed(data) {
            match ev {
                StreamJsonEvent::Gate {
                    request_id,
                    tool_name,
                    input,
                    tool_use_id,
                } => {
                    if already_over {
                        // A finished session cannot be waiting on a gate —
                        // late/duplicate tee data must not resurrect it.
                        continue;
                    }
                    if state.pending.iter().any(|g| g.request_id == request_id) {
                        // The same request re-observed (e.g. overlapping tee
                        // delivery) must not double-escalate.
                        continue;
                    }
                    let gate = PendingGate {
                        request_id,
                        tool_name,
                        input,
                        tool_use_id,
                        detected_at: Utc::now(),
                    };
                    state.pending.push(gate.clone());
                    acts.detected.push(gate);
                    acts.report.gates_detected += 1;
                }
                StreamJsonEvent::GateAnswered { request_id } => {
                    let before = state.pending.len();
                    state.pending.retain(|g| g.request_id != request_id);
                    if state.pending.len() < before {
                        acts.cleared.push((request_id, "answered"));
                        acts.report.gates_cleared += 1;
                    }
                }
                StreamJsonEvent::Completed { summary } => {
                    if already_over || acts.outcome.is_some() {
                        continue;
                    }
                    state.status = SupervisedStatus::Completed;
                    state.last_summary = Some(summary.clone());
                    acts.report.completed = true;
                    acts.outcome = Some(SupervisedOutcome::Completed { summary });
                }
                StreamJsonEvent::Failed { reason } => {
                    if already_over || acts.outcome.is_some() {
                        continue;
                    }
                    state.status = SupervisedStatus::Failed;
                    state.last_summary = Some(reason.clone());
                    acts.report.failed = true;
                    acts.outcome = Some(SupervisedOutcome::Failed { reason });
                }
            }
        }
        if eof
            && !already_over
            && acts.outcome.is_none()
            && !matches!(
                state.status,
                SupervisedStatus::Completed | SupervisedStatus::Failed
            )
        {
            let reason = "PTY closed before the session reported a result".to_string();
            state.status = SupervisedStatus::Failed;
            state.last_summary = Some(reason.clone());
            acts.report.failed = true;
            acts.outcome = Some(SupervisedOutcome::Failed { reason });
        }
        // A finished session's remaining gates are moot — clear them so the
        // registry never shows a dead session as "blocked".
        if acts.outcome.is_some() {
            for g in state.pending.drain(..) {
                acts.cleared.push((g.request_id, "session_ended"));
                acts.report.gates_cleared += 1;
            }
        }
        Some(acts)
    })?;

    for gate in &actions.detected {
        crate::events::emit(crate::events::terminal_gate_detected(
            session_id,
            actions.pty_session_id.as_deref(),
            &actions.project_slug,
            actions.kind,
            &actions.root_path,
            gate,
        ));
    }
    for (request_id, reason) in &actions.cleared {
        crate::events::emit(crate::events::terminal_gate_cleared(
            session_id, request_id, reason,
        ));
    }
    if let Some(outcome) = actions.outcome {
        // `false` = nobody was awaiting (watched session, or a goal already
        // timed out and parked) — informational, never an error.
        let delivered = complete_supervised_session(session_id, outcome);
        tracing::info!(
            session_id,
            delivered,
            "supervised session reached a terminal state via the S2 parser"
        );
    }
    let mut report = actions.report;
    report.detected = actions.detected;
    report.cleared = actions
        .cleared
        .into_iter()
        .map(|(request_id, reason)| ClearedGate {
            request_id,
            reason: reason.to_string(),
        })
        .collect();
    Some(report)
}

/// Snapshot one session (S3's context-load and tests).
pub fn session_snapshot(session_id: &str) -> Option<SessionSnapshot> {
    with_registry(|map| {
        map.sessions.get(session_id).map(|s| SessionSnapshot {
            session_id: session_id.to_string(),
            kind: s.kind,
            project_slug: s.project_slug.clone(),
            root_path: s.root_path.clone(),
            pty_session_id: s.pty_session_id.clone(),
            status: s.status,
            pending_gates: s.pending.clone(),
            last_summary: s.last_summary.clone(),
        })
    })
}

/// Snapshot every registered session, newest-registration order not
/// guaranteed (HashMap) — callers sort as needed.
pub fn list_sessions() -> Vec<SessionSnapshot> {
    with_registry(|map| {
        map.sessions
            .iter()
            .map(|(id, s)| SessionSnapshot {
                session_id: id.clone(),
                kind: s.kind,
                project_slug: s.project_slug.clone(),
                root_path: s.root_path.clone(),
                pty_session_id: s.pty_session_id.clone(),
                status: s.status,
                pending_gates: s.pending.clone(),
                last_summary: s.last_summary.clone(),
            })
            .collect()
    })
}

/// Drop a session from the registry (terminal-state sweep / tests).
pub fn remove_session(session_id: &str) -> bool {
    with_registry(|map| map.sessions.remove(session_id).is_some())
}

/// Insert or update a bounded live harness-run projection. Terminal runs are
/// allowed to outlive their reporter; terminal states are retained for the
/// same small registry window so the final verdict remains inspectable.
pub fn update_harness_run(mut update: HarnessRunUpdate) -> Result<HarnessRunSnapshot, String> {
    validate_harness_update(&update)?;
    update.project = bounded_harness_field(&update.project);
    update.prompt_title = bounded_harness_field(&update.prompt_title);
    update.prompt_digest = bounded_harness_field(&update.prompt_digest);
    update.task_version = bounded_optional(update.task_version);
    update.envelope_id = bounded_optional(update.envelope_id);
    update.prompt_context = update.prompt_context.map(|v| bounded_prompt_context(&v));
    update.dag_nodes = update
        .dag_nodes
        .into_iter()
        .map(|v| bounded_harness_field(&v))
        .collect();
    update.dependencies = update
        .dependencies
        .into_iter()
        .map(|v| bounded_harness_field(&v))
        .collect();
    update.active_node = bounded_optional(update.active_node);
    update.worker = bounded_optional(update.worker);
    update.provider = bounded_optional(update.provider);
    update.model = bounded_optional(update.model);
    update.billing_class = bounded_optional(update.billing_class);
    update.tier = bounded_optional(update.tier);
    update.routing_reason = bounded_optional(update.routing_reason);
    update.declared_verification = bounded_verification(update.declared_verification);
    update.last_verification = bounded_verification(update.last_verification);
    update.verification_verdict = bounded_optional(update.verification_verdict);
    update.pending_gate = update.pending_gate.map(|gate| HarnessPendingGate {
        request_id: bounded_harness_field(&gate.request_id),
        tool_name: bounded_harness_field(&gate.tool_name),
        tier: bounded_optional(gate.tier),
    });
    update.evidence = bounded_optional(update.evidence);
    update.result = bounded_optional(update.result);
    update.parent_run_id = bounded_optional(update.parent_run_id);
    update.parent_session_id = bounded_optional(update.parent_session_id);
    let now = Utc::now();
    with_registry(|registry| {
        if let Some(existing) = registry.harness_runs.get_mut(&update.run_id) {
            // A run id is bound to one session. Accepting a later remap would
            // let an accidental client retry merge unrelated runs.
            if existing.update.session_id != update.session_id {
                return Err("harness run id is already bound to another session".to_string());
            }
            // Terminal states are monotonic. In particular, the detached
            // initial announcement may arrive after the awaited completion
            // update on a very fast run; it must not resurrect finished work.
            if !existing.update.status.is_active() {
                return Ok(snapshot_harness_run(existing));
            }
            update.verification_attempts =
                match (existing.verification_attempts, update.verification_attempts) {
                    (Some(old), Some(new)) => Some(old.max(new)),
                    (Some(old), None) => Some(old),
                    (None, value) => value,
                };
            // Telemetry counters are monotonic within a run. A delayed
            // heartbeat must not erase already-observed tool/gate/retry work.
            fn monotonic(old: Option<u32>, new: Option<u32>) -> Option<u32> {
                match (old, new) {
                    (Some(old), Some(new)) => Some(old.max(new)),
                    (Some(old), None) => Some(old),
                    (None, value) => value,
                }
            }
            update.retry_count = monotonic(existing.update.retry_count, update.retry_count);
            update.tool_calls = monotonic(existing.update.tool_calls, update.tool_calls);
            update.gate_attempts = monotonic(existing.update.gate_attempts, update.gate_attempts);

            // Compatibility heartbeats may omit fields. Keep richer evidence
            // already observed for this run instead of turning it unknown.
            macro_rules! preserve {
                ($field:ident) => {
                    if update.$field.is_none() {
                        update.$field = existing.update.$field.clone();
                    }
                };
            }
            preserve!(task_version);
            preserve!(envelope_id);
            preserve!(provider);
            preserve!(billing_class);
            preserve!(model);
            preserve!(tier);
            preserve!(routing_reason);
            preserve!(declared_verification);
            preserve!(last_verification);
            preserve!(verification_verdict);
            preserve!(pending_gate);
            preserve!(evidence);
            preserve!(result);
            preserve!(parent_run_id);
            preserve!(parent_session_id);
            existing.update = update;
            existing.updated_at = now;
            return Ok(snapshot_harness_run(existing));
        }
        if registry.harness_runs.len() >= MAX_HARNESS_RUNS {
            let oldest = registry
                .harness_runs
                .iter()
                .min_by_key(|(_, run)| run.updated_at)
                .map(|(id, _)| id.clone());
            if let Some(oldest) = oldest {
                registry.harness_runs.remove(&oldest);
            }
        }
        let attempts = update.verification_attempts;
        let state = HarnessRunState {
            update,
            started_at: now,
            updated_at: now,
            verification_attempts: attempts,
            council_suggestion_emitted: false,
        };
        let snapshot = snapshot_harness_run(&state);
        registry.harness_runs.insert(snapshot.run_id.clone(), state);
        Ok(snapshot)
    })
}

/// Hydrate the process-local projection from the explicit Spectral-backed
/// store. Rows are accepted only when they are newer than an already-live
/// registry entry, making repeated startup hydration safe and idempotent.
/// Persisted active rows still pass through the normal TTL filter; a daemon
/// restart must not make a stale heartbeat look live.
pub fn hydrate_harness_runs(snapshots: impl IntoIterator<Item = HarnessRunSnapshot>) {
    with_registry(|registry| {
        for snapshot in snapshots {
            if let Some(existing) = registry.harness_runs.get(&snapshot.run_id) {
                if existing.update.session_id != snapshot.session_id {
                    tracing::warn!(
                        run_id = %snapshot.run_id,
                        "ignored persisted harness snapshot bound to another session"
                    );
                    continue;
                }
                // A terminal result is authoritative even if a stale
                // heartbeat reached this process after the result was
                // persisted. Timestamps alone cannot express that ordering:
                // the heartbeat's local arrival time is necessarily newer.
                match (
                    existing.update.status.is_active(),
                    snapshot.status.is_active(),
                ) {
                    (false, true) => continue,
                    (true, false) => {}
                    _ if existing.updated_at >= snapshot.updated_at => continue,
                    _ => {}
                }
            }
            let update = HarnessRunUpdate {
                run_id: snapshot.run_id.clone(),
                session_id: snapshot.session_id.clone(),
                project: snapshot.project.clone(),
                prompt_title: snapshot.prompt_title.clone(),
                prompt_digest: snapshot.prompt_digest.clone(),
                task_version: snapshot.task_version.clone(),
                envelope_id: snapshot.envelope_id.clone(),
                prompt_context: snapshot.prompt_context.clone(),
                dag_nodes: snapshot.dag_nodes.clone(),
                dependencies: snapshot.dependencies.clone(),
                active_node: snapshot.active_node.clone(),
                worker: snapshot.worker.clone(),
                provider: snapshot.provider.clone(),
                model: snapshot.model.clone(),
                billing_class: snapshot.billing_class.clone(),
                tier: snapshot.tier.clone(),
                routing_reason: snapshot.routing_reason.clone(),
                status: snapshot.status,
                declared_verification: snapshot.declared_verification.clone(),
                last_verification: snapshot.last_verification.clone(),
                verification_attempts: snapshot.verification_attempts,
                verification_verdict: snapshot.verification_verdict.clone(),
                pending_gate: snapshot.pending_gate.clone(),
                retry_count: snapshot.retry_count,
                tool_calls: snapshot.tool_calls,
                gate_attempts: snapshot.gate_attempts,
                evidence: snapshot.evidence.clone(),
                result: snapshot.result.clone(),
                parent_run_id: snapshot.parent_run_id.clone(),
                parent_session_id: snapshot.parent_session_id.clone(),
            };
            if registry.harness_runs.len() >= MAX_HARNESS_RUNS {
                let oldest = registry
                    .harness_runs
                    .iter()
                    .min_by_key(|(_, run)| run.updated_at)
                    .map(|(id, _)| id.clone());
                if let Some(oldest) = oldest {
                    registry.harness_runs.remove(&oldest);
                }
            }
            registry.harness_runs.insert(
                snapshot.run_id,
                HarnessRunState {
                    update,
                    started_at: snapshot.started_at,
                    updated_at: snapshot.updated_at,
                    verification_attempts: snapshot.verification_attempts,
                    // A suggestion was already possible before the snapshot
                    // was persisted. Do not nag again after every restart.
                    council_suggestion_emitted: true,
                },
            );
        }
    });
}

/// Atomically claim the one proactive Council suggestion allowed for a run.
/// Heartbeats repeat the same snapshot every 15 seconds, so emitting directly
/// from the HTTP handler without this latch would nag the user indefinitely.
pub fn claim_council_suggestion(run_id: &str) -> Option<CouncilRecommendation> {
    with_registry(|registry| {
        let state = registry.harness_runs.get_mut(run_id)?;
        let recommendation =
            recommend_council(state.update.prompt_context.as_deref().unwrap_or_default());
        if !recommendation.recommended || state.council_suggestion_emitted {
            return None;
        }
        state.council_suggestion_emitted = true;
        Some(recommendation)
    })
}

/// Query the active DAG-1 runs without parsing terminal output. Sorted by
/// latest update, then bounded by the registry's fixed cap.
pub fn list_active_harness_runs() -> Vec<HarnessRunSnapshot> {
    with_registry(|registry| {
        let now = Utc::now();
        let mut runs: Vec<_> = registry
            .harness_runs
            .values()
            .filter(|run| {
                run.update.status.is_active()
                    && (now - run.updated_at).num_seconds() <= HARNESS_ACTIVE_TTL_SECS
            })
            .map(snapshot_harness_run)
            .collect();
        runs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        runs
    })
}

/// Read one run, including its final status when it has already ended.
pub fn harness_run_snapshot(run_id: &str) -> Option<HarnessRunSnapshot> {
    with_registry(|registry| registry.harness_runs.get(run_id).map(snapshot_harness_run))
}

/// Drop a run from the bounded in-memory projection (tests / terminal sweep).
pub fn remove_harness_run(run_id: &str) -> bool {
    with_registry(|registry| registry.harness_runs.remove(run_id).is_some())
}

// ── S3 (#429): gate → Decision Inbox bridge ────────────────────────────────

/// Character cap for the tool-input preview embedded in a `session_gate`
/// decision's `detail` text (same cap and explicit-truncation contract as the
/// `tool_approval` preview in `tool_execution.rs`; the FULL input lives in the
/// payload and is inspectable on the card).
const GATE_INPUT_PREVIEW_MAX_CHARS: usize = 400;

fn gate_input_preview(input: &Value) -> String {
    let json = serde_json::to_string(input).unwrap_or_else(|_| "{}".to_string());
    let total = json.chars().count();
    if total <= GATE_INPUT_PREVIEW_MAX_CHARS {
        return json;
    }
    let clipped: String = json.chars().take(GATE_INPUT_PREVIEW_MAX_CHARS).collect();
    format!(
        "{}… [truncated — {} more chars]",
        clipped,
        total - GATE_INPUT_PREVIEW_MAX_CHARS
    )
}

/// What one [`bridge_report_to_inbox`] call did — evidence for the ingest
/// route's response (and tests), never silence.
#[derive(Debug, Default, Serialize)]
pub struct GateBridgeOutcome {
    /// `session_gate` decisions filed for gates detected by this ingest.
    pub decisions_filed: usize,
    /// Open `session_gate` decisions superseded because their gate resolved
    /// outside the inbox (hand-typed answer, session end).
    pub decisions_superseded: usize,
    /// Best-effort failures (logged too). A bridge failure never fails the
    /// tee: the gate stays VISIBLE in the session's tab (the S1 posture) and
    /// the bus event already went out.
    pub errors: Vec<String>,
}

/// Bridge one ingest call's gate activity into the Decision Inbox (S3, #429;
/// spec Piece 4):
///
/// - each detected gate → a `session_gate` decision (deduped on the
///   (session, request_id) pair — a re-observed gate line, e.g. after a tee
///   reconnect, must not double-escalate). S4 (#430) classifies the gate's
///   tool via [`super::gate_classifier::classify_gate`] into a `risk_policy`
///   action_class, and [`crate::decisions::create_decision`] resolves the tier
///   from it (unrecognized tools → `cc_unclassified`, unseeded → Tier 2
///   fail-closed, user-only).
/// - each cleared gate → the matching open card superseded (status
///   `superseded`, honest note, audit row) so the inbox never shows a gate
///   the session is no longer waiting on.
///
/// Deterministic, zero-LLM. `project_id` is resolved best-effort from the
/// registry's project slug (the decisions table groups by project id);
/// resolution failure files the card without it rather than dropping the
/// escalation.
pub async fn bridge_report_to_inbox(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    session_id: &str,
    report: &IngestReport,
) -> GateBridgeOutcome {
    let mut outcome = GateBridgeOutcome::default();
    if report.detected.is_empty() && report.cleared.is_empty() {
        return outcome;
    }
    // Session context for headline/payload. The registry retains finished
    // sessions (S2), so this resolves for cleared gates too; a missing entry
    // (restart race) degrades to unknown context, never a dropped escalation.
    let snapshot = session_snapshot(session_id);
    let (project_slug, pty_session_id) = match &snapshot {
        Some(s) => (s.project_slug.clone(), s.pty_session_id.clone()),
        None => (String::from("unknown"), None),
    };
    let project_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM projects WHERE slug = ? ORDER BY last_opened_at DESC LIMIT 1",
    )
    .bind(&project_slug)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    for gate in &report.detected {
        match crate::decisions::find_open_session_gate(pool, session_id, &gate.request_id).await {
            Ok(Some(_)) => continue, // already escalated — never double-file
            Ok(None) => {}
            Err(e) => {
                outcome
                    .errors
                    .push(format!("dedupe lookup failed for {}: {e}", gate.request_id));
                continue;
            }
        }
        let question = format!(
            "The Claude Code session in '{}' is asking to run {} — allow it?",
            project_slug, gate.tool_name
        );
        let payload = crate::decisions::SessionGatePayload {
            question: question.clone(),
            target_session_id: session_id.to_string(),
            pty_session_id: pty_session_id.clone(),
            request_id: gate.request_id.clone(),
            tool_name: gate.tool_name.clone(),
            input: gate.input.clone(),
            tool_use_id: if gate.tool_use_id.is_empty() {
                None
            } else {
                Some(gate.tool_use_id.clone())
            },
            options: vec!["allow".to_string(), "deny".to_string()],
        };
        let payload_json = match serde_json::to_value(&payload) {
            Ok(v) => v,
            Err(e) => {
                outcome.errors.push(format!(
                    "payload serialize failed for {}: {e}",
                    gate.request_id
                ));
                continue;
            }
        };
        // Plain-language headline (A1: <= 80 chars, no technical ids); the
        // technical substance goes in `detail`.
        let headline = {
            let h = format!("A terminal session wants to run {}", gate.tool_name);
            if h.chars().count() > crate::decisions::MAX_HEADLINE_CHARS {
                h.chars()
                    .take(crate::decisions::MAX_HEADLINE_CHARS)
                    .collect()
            } else {
                h
            }
        };
        let detail = format!(
            "Supervised session {} (project '{}') is blocked on a can_use_tool gate: \
             {} with input {}. Approving records your ruling; the answer relay into the \
             session ships in S5 (#431) — until then the gate is also answerable in the \
             session's terminal tab.",
            session_id,
            project_slug,
            gate.tool_name,
            gate_input_preview(&gate.input),
        );
        // S4 (#430): classify the gate's tool → a risk_policy action_class; the
        // tier machinery resolves it (unrecognized tools → `cc_unclassified`,
        // which is unseeded → Tier 2 fail-closed). Setting `action_class` is the
        // only S4 wiring here — the tier decision stays in `risk_policy`.
        let action_class =
            super::gate_classifier::classify_gate(&gate.tool_name, &gate.input).to_string();
        match crate::decisions::create_decision(
            pool,
            crate::decisions::NewDecision {
                kind: "session_gate".to_string(),
                project_id: project_id.clone(),
                headline: Some(headline),
                detail: Some(detail),
                payload: payload_json,
                action_class: Some(action_class),
                ..Default::default()
            },
        )
        .await
        {
            Ok(d) => {
                tracing::info!(
                    session_id,
                    request_id = gate.request_id.as_str(),
                    decision_id = d.id.as_str(),
                    tier = d.tier,
                    "session gate escalated to the Decision Inbox"
                );
                outcome.decisions_filed += 1;
            }
            Err(e) => outcome.errors.push(format!(
                "decision create failed for {}: {e}",
                gate.request_id
            )),
        }
    }

    for cleared in &report.cleared {
        let open =
            match crate::decisions::find_open_session_gate(pool, session_id, &cleared.request_id)
                .await
            {
                Ok(Some(d)) => d,
                Ok(None) => continue, // never filed, or already resolved — benign
                Err(e) => {
                    outcome.errors.push(format!(
                        "supersede lookup failed for {}: {e}",
                        cleared.request_id
                    ));
                    continue;
                }
            };
        let note = match cleared.reason.as_str() {
            "answered" => "gate answered in the terminal session".to_string(),
            "session_ended" => "session ended before the gate was answered".to_string(),
            other => format!("gate cleared: {other}"),
        };
        match crate::decisions::supersede_decision(pool, &open.id, &note).await {
            Ok(true) => outcome.decisions_superseded += 1,
            Ok(false) => {} // raced with another resolver — already closed
            Err(e) => outcome
                .errors
                .push(format!("supersede failed for decision {}: {e}", open.id)),
        }
    }

    for e in &outcome.errors {
        tracing::warn!(session_id, "session-gate inbox bridge: {e}");
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::platform_extensions::supervised_cli::register_completion_hook;
    use crate::events::{PermagentEvent, PermagentEventType};

    // ── Fixtures ────────────────────────────────────────────────────────────
    //
    // REAL wire shapes, lifted verbatim from the in-repo protocol
    // implementation's own test suite (`providers/claude_code.rs`, the code
    // that already speaks this protocol to the `claude` CLI in production):
    // GATE_LINE / RESULT_LINE / CONTROL_RESPONSE_ACK. The error-result and
    // system/assistant frames are CONSTRUCTED from the documented stream-json
    // format (the repo has no captured sample of those) — flagged in the PR.

    const GATE_LINE: &str = r#"{"type":"control_request","request_id":"perm_1","request":{"subtype":"can_use_tool","tool_name":"Write","input":{"path":"foo.txt","content":"hello"},"tool_use_id":"tu_1"}}"#;
    const RESULT_LINE: &str =
        r#"{"type":"result","result":"Done","usage":{"input_tokens":10,"output_tokens":5}}"#;
    const CONTROL_RESPONSE_ACK: &str =
        r#"{"type":"control_response","response":{"subtype":"success","request_id":"perm_1"}}"#;
    /// The exact answer shape the spec (and `claude_code.rs`) writes to stdin
    /// — what the PTY echoes back when a human hand-types it today.
    const ANSWER_ECHO_LINE: &str = r#"{"type":"control_response","response":{"subtype":"success","request_id":"perm_1","response":{"behavior":"allow","updatedInput":{"path":"foo.txt","content":"hello"},"toolUseID":"tu_1"}}}"#;
    // Constructed from the documented format (no in-repo capture):
    const ERROR_RESULT_LINE: &str = r#"{"type":"result","subtype":"error_max_turns","is_error":true,"result":"Reached max turns","usage":{"input_tokens":10,"output_tokens":5}}"#;
    const ASSISTANT_LINE: &str = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"working on it"}]},"session_id":"abc"}"#;
    const SYSTEM_INIT_LINE: &str =
        r#"{"type":"system","subtype":"init","session_id":"abc","model":"claude-sonnet-4-5"}"#;

    fn feed_all(scanner: &mut NdjsonScanner, s: &str) -> Vec<StreamJsonEvent> {
        scanner.feed(s)
    }

    fn expect_gate(ev: &StreamJsonEvent) {
        match ev {
            StreamJsonEvent::Gate {
                request_id,
                tool_name,
                input,
                tool_use_id,
            } => {
                assert_eq!(request_id, "perm_1");
                assert_eq!(tool_name, "Write");
                assert_eq!(input["path"], "foo.txt");
                assert_eq!(input["content"], "hello");
                assert_eq!(tool_use_id, "tu_1");
            }
            other => panic!("expected Gate, got {:?}", other),
        }
    }

    // ── Scanner: classification ────────────────────────────────────────────

    #[test]
    fn gate_line_parses_with_all_fields() {
        let mut s = NdjsonScanner::new();
        let evs = feed_all(&mut s, &format!("{GATE_LINE}\n"));
        assert_eq!(evs.len(), 1);
        expect_gate(&evs[0]);
    }

    #[test]
    fn success_result_parses_as_completed() {
        let mut s = NdjsonScanner::new();
        let evs = feed_all(&mut s, &format!("{RESULT_LINE}\n"));
        assert_eq!(
            evs,
            vec![StreamJsonEvent::Completed {
                summary: "Done".to_string()
            }]
        );
    }

    #[test]
    fn error_result_parses_as_failed() {
        let mut s = NdjsonScanner::new();
        let evs = feed_all(&mut s, &format!("{ERROR_RESULT_LINE}\n"));
        assert_eq!(
            evs,
            vec![StreamJsonEvent::Failed {
                reason: "Reached max turns".to_string()
            }]
        );
    }

    #[test]
    fn top_level_error_parses_as_failed() {
        let mut s = NdjsonScanner::new();
        let evs = feed_all(
            &mut s,
            "{\"type\":\"error\",\"message\":\"stream exploded\"}\n",
        );
        assert_eq!(
            evs,
            vec![StreamJsonEvent::Failed {
                reason: "stream exploded".to_string()
            }]
        );
    }

    #[test]
    fn control_response_parses_as_gate_answered() {
        let mut s = NdjsonScanner::new();
        for line in [CONTROL_RESPONSE_ACK, ANSWER_ECHO_LINE] {
            let evs = feed_all(&mut s, &format!("{line}\n"));
            assert_eq!(
                evs,
                vec![StreamJsonEvent::GateAnswered {
                    request_id: "perm_1".to_string()
                }],
                "line: {line}"
            );
        }
    }

    #[test]
    fn non_gate_protocol_lines_and_shell_noise_are_ignored() {
        let mut s = NdjsonScanner::new();
        let noise = [
            ASSISTANT_LINE,
            SYSTEM_INIT_LINE,
            // control_request that is NOT a permission gate:
            r#"{"type":"control_request","request_id":"req_0","request":{"subtype":"initialize"}}"#,
            // the echoed launch command and a shell prompt:
            "cat '/tmp/permagent-supervised/sup-x.prompt.ndjson' - | 'claude' -p --output-format stream-json",
            "jesse@mini permagent %",
            // malformed JSON:
            r#"{"type":"control_request","request_id":"#,
            // JSON that is not an object with a type:
            "[1,2,3]",
            "42",
        ];
        for line in noise {
            assert_eq!(
                feed_all(&mut s, &format!("{line}\n")),
                vec![],
                "line must be ignored: {line}"
            );
        }
    }

    #[test]
    fn gate_with_missing_optional_fields_defaults() {
        // `input` and `tool_use_id` are `#[serde(default)]` in the in-repo
        // protocol structs — mirror that tolerance.
        let mut s = NdjsonScanner::new();
        let evs = feed_all(
            &mut s,
            "{\"type\":\"control_request\",\"request_id\":\"perm_9\",\"request\":{\"subtype\":\"can_use_tool\",\"tool_name\":\"Bash\"}}\n",
        );
        match &evs[..] {
            [StreamJsonEvent::Gate {
                request_id,
                tool_name,
                input,
                tool_use_id,
            }] => {
                assert_eq!(request_id, "perm_9");
                assert_eq!(tool_name, "Bash");
                assert_eq!(input, &serde_json::json!({}));
                assert_eq!(tool_use_id, "");
            }
            other => panic!("expected one Gate, got {:?}", other),
        }
    }

    // ── Scanner: PTY realities ─────────────────────────────────────────────

    #[test]
    fn chunk_splits_mid_line_reassemble() {
        let mut s = NdjsonScanner::new();
        let line = format!("{GATE_LINE}\n");
        // Split mid-JSON, mid-key, and right before the newline.
        let (a, rest) = line.split_at(25);
        let (b, c) = rest.split_at(rest.len() - 3);
        assert_eq!(s.feed(a), vec![]);
        assert_eq!(s.feed(b), vec![]);
        let evs = s.feed(c);
        assert_eq!(evs.len(), 1);
        expect_gate(&evs[0]);
    }

    #[test]
    fn crlf_and_ansi_osc_noise_are_stripped() {
        let mut s = NdjsonScanner::new();
        // The Build-tab zsh emits OSC 133/OSC 7 precmd hooks (terminal.rs
        // injects exactly these), plus SGR color and a CRLF ending.
        let chunk = format!(
            "\u{1b}]133;D;0\u{7}\u{1b}]7;file://host/Users/jesse\u{7}\u{1b}[31m{GATE_LINE}\u{1b}[0m\r\n"
        );
        let evs = s.feed(&chunk);
        assert_eq!(evs.len(), 1);
        expect_gate(&evs[0]);
    }

    #[test]
    fn osc_with_st_terminator_is_stripped() {
        let mut s = NdjsonScanner::new();
        let chunk = format!("\u{1b}]0;title\u{1b}\\{RESULT_LINE}\r\n");
        assert_eq!(
            s.feed(&chunk),
            vec![StreamJsonEvent::Completed {
                summary: "Done".to_string()
            }]
        );
    }

    #[test]
    fn json_not_at_column_zero_is_found() {
        let mut s = NdjsonScanner::new();
        let evs = s.feed(&format!("leftover-echo {GATE_LINE}\n"));
        assert_eq!(evs.len(), 1);
        expect_gate(&evs[0]);
    }

    #[test]
    fn oversized_partial_line_is_dropped_then_stream_recovers() {
        let mut s = NdjsonScanner::new();
        let big = "x".repeat(MAX_LINE_BYTES + 1);
        assert_eq!(s.feed(&big), vec![]);
        // Buffer was dropped; a fresh, complete line still parses.
        let evs = s.feed(&format!("{GATE_LINE}\n"));
        assert_eq!(evs.len(), 1);
        expect_gate(&evs[0]);
    }

    #[test]
    fn multiple_events_in_one_chunk_keep_stream_order() {
        let mut s = NdjsonScanner::new();
        let chunk = format!("{GATE_LINE}\n{ANSWER_ECHO_LINE}\n{RESULT_LINE}\n");
        let evs = s.feed(&chunk);
        assert_eq!(evs.len(), 3);
        expect_gate(&evs[0]);
        assert_eq!(
            evs[1],
            StreamJsonEvent::GateAnswered {
                request_id: "perm_1".to_string()
            }
        );
        assert_eq!(
            evs[2],
            StreamJsonEvent::Completed {
                summary: "Done".to_string()
            }
        );
    }

    #[test]
    fn strip_ansi_removes_csi_osc_and_control_chars() {
        assert_eq!(strip_ansi("\u{1b}[1;32mhi\u{1b}[0m"), "hi");
        assert_eq!(strip_ansi("\u{1b}]7;file://h/p\u{7}hi"), "hi");
        assert_eq!(strip_ansi("\u{1b}]0;t\u{1b}\\hi"), "hi");
        assert_eq!(strip_ansi("a\rb\u{8}c"), "abc");
        assert_eq!(strip_ansi("\u{1b}Mplain"), "plain");
    }

    // ── Registry ───────────────────────────────────────────────────────────

    /// Collect THIS session's supervision events from the bus's replay buffer
    /// into `acc`, deduped by event id (session ids are unique per test, so
    /// cross-test pollution is impossible).
    ///
    /// Deliberately NOT a live `broadcast::Receiver`: under the full parallel
    /// suite the bus floods >1000 events and a receiver `Lagged` past our
    /// frames (both CI legs, deterministic). The replay ring holds the last
    /// 1000 events, and callers scan it immediately after each `ingest_output`
    /// — eviction would need 1000 events between the emit inside ingest and
    /// this scan, which is effectively impossible.
    fn collect_session_events(acc: &mut Vec<PermagentEvent>, session_id: &str) {
        for ev in crate::events::buffered_events() {
            if ev.payload["supervised_session_id"] == session_id
                && !acc.iter().any(|e| e.id == ev.id)
            {
                acc.push(ev);
            }
        }
    }

    fn of_type<'a>(
        acc: &'a [PermagentEvent],
        event_type: &PermagentEventType,
    ) -> Vec<&'a PermagentEvent> {
        acc.iter().filter(|e| &e.event_type == event_type).collect()
    }

    fn unique_id(tag: &str) -> String {
        format!("sup-test-{tag}-{}", uuid::Uuid::new_v4())
    }

    #[test]
    fn register_attach_resolve_roundtrip() {
        let sid = unique_id("resolve");
        register_session(&sid, SupervisedSessionKind::Watched, "proj", "/tmp/p");
        let snap = session_snapshot(&sid).unwrap();
        assert_eq!(snap.status, SupervisedStatus::Launched);
        assert_eq!(snap.kind, SupervisedSessionKind::Watched);
        assert_eq!(snap.project_slug, "proj");
        assert!(snap.pty_session_id.is_none());

        assert!(attach_pty(&sid, "pty-123"));
        let snap = session_snapshot(&sid).unwrap();
        assert_eq!(snap.status, SupervisedStatus::Attached);
        assert_eq!(snap.pty_session_id.as_deref(), Some("pty-123"));

        // Resolution: by supervised id, by pty id, and the miss cases.
        assert_eq!(resolve_session_id(Some(&sid), None).as_deref(), Some(&*sid));
        assert_eq!(
            resolve_session_id(None, Some("pty-123")).as_deref(),
            Some(&*sid)
        );
        assert!(resolve_session_id(Some("sup-nope"), Some("pty-nope")).is_none());
        assert!(!attach_pty("sup-nope", "pty-x"));

        // Re-attach to a NEW pty (tab respawn) overwrites.
        assert!(attach_pty(&sid, "pty-456"));
        assert_eq!(
            session_snapshot(&sid).unwrap().pty_session_id.as_deref(),
            Some("pty-456")
        );

        assert!(remove_session(&sid));
        assert!(session_snapshot(&sid).is_none());
    }

    #[test]
    fn ingest_unknown_session_is_none() {
        assert!(ingest_output("sup-never-registered", "data\n", false).is_none());
    }

    #[tokio::test]
    async fn gate_detection_emits_bus_event_and_records_pending() {
        let sid = unique_id("gate");
        let mut evs = Vec::new();
        register_session(&sid, SupervisedSessionKind::Watched, "proj", "/tmp/p");
        attach_pty(&sid, "pty-gate-1");

        let report = ingest_output(&sid, &format!("{GATE_LINE}\r\n"), false).unwrap();
        collect_session_events(&mut evs, &sid);
        assert_eq!(report.gates_detected, 1);
        assert_eq!(report.gates_cleared, 0);
        assert!(!report.completed && !report.failed);

        // Pending gate recorded with full addressing.
        let snap = session_snapshot(&sid).unwrap();
        assert_eq!(snap.pending_gates.len(), 1);
        assert_eq!(snap.pending_gates[0].request_id, "perm_1");
        assert_eq!(snap.pending_gates[0].tool_name, "Write");

        // Structured bus event carries everything S3 needs.
        let detected = of_type(&evs, &PermagentEventType::TerminalGateDetected);
        assert_eq!(detected.len(), 1);
        let p = &detected[0].payload;
        assert_eq!(p["pty_session_id"], "pty-gate-1");
        assert_eq!(p["project_slug"], "proj");
        assert_eq!(p["session_kind"], "watched");
        assert_eq!(p["root_path"], "/tmp/p");
        assert_eq!(p["request_id"], "perm_1");
        assert_eq!(p["tool_name"], "Write");
        assert_eq!(p["input"]["path"], "foo.txt");
        assert_eq!(p["tool_use_id"], "tu_1");

        // Duplicate gate line (overlapping tee delivery): no double escalation.
        let report = ingest_output(&sid, &format!("{GATE_LINE}\n"), false).unwrap();
        collect_session_events(&mut evs, &sid);
        assert_eq!(report.gates_detected, 0);
        assert_eq!(session_snapshot(&sid).unwrap().pending_gates.len(), 1);
        assert_eq!(
            of_type(&evs, &PermagentEventType::TerminalGateDetected).len(),
            1,
            "duplicate line must not re-emit"
        );

        remove_session(&sid);
    }

    #[tokio::test]
    async fn observed_answer_clears_pending_gate() {
        let sid = unique_id("answer");
        let mut evs = Vec::new();
        register_session(&sid, SupervisedSessionKind::Watched, "proj", "/tmp/p");

        ingest_output(&sid, &format!("{GATE_LINE}\n"), false).unwrap();
        let report = ingest_output(&sid, &format!("{ANSWER_ECHO_LINE}\n"), false).unwrap();
        collect_session_events(&mut evs, &sid);
        assert_eq!(report.gates_cleared, 1);
        assert!(session_snapshot(&sid).unwrap().pending_gates.is_empty());

        let cleared = of_type(&evs, &PermagentEventType::TerminalGateCleared);
        assert_eq!(cleared.len(), 1);
        assert_eq!(cleared[0].payload["request_id"], "perm_1");
        assert_eq!(cleared[0].payload["reason"], "answered");

        // Answer for a request that was never pending: reported, not cleared.
        let report = ingest_output(&sid, &format!("{ANSWER_ECHO_LINE}\n"), false).unwrap();
        assert_eq!(report.gates_cleared, 0);

        remove_session(&sid);
    }

    #[tokio::test]
    async fn result_completes_session_and_fulfils_s1_seam() {
        let sid = unique_id("complete");
        register_session(&sid, SupervisedSessionKind::DispatchedGoal, "proj", "/wt");
        let hook = register_completion_hook(&sid);

        let report = ingest_output(&sid, &format!("{RESULT_LINE}\n"), false).unwrap();
        assert!(report.completed);
        assert_eq!(
            session_snapshot(&sid).unwrap().status,
            SupervisedStatus::Completed
        );
        assert_eq!(
            session_snapshot(&sid).unwrap().last_summary.as_deref(),
            Some("Done")
        );
        match hook.await.unwrap() {
            SupervisedOutcome::Completed { summary } => assert_eq!(summary, "Done"),
            other => panic!("expected Completed, got {:?}", other),
        }

        // Late data after completion never resurrects the session.
        let report = ingest_output(&sid, &format!("{GATE_LINE}\n"), false).unwrap();
        assert_eq!(report.gates_detected, 0);
        assert!(session_snapshot(&sid).unwrap().pending_gates.is_empty());

        remove_session(&sid);
    }

    #[tokio::test]
    async fn error_result_fails_session_via_seam() {
        let sid = unique_id("fail");
        register_session(&sid, SupervisedSessionKind::DispatchedGoal, "proj", "/wt");
        let hook = register_completion_hook(&sid);

        let report = ingest_output(&sid, &format!("{ERROR_RESULT_LINE}\n"), false).unwrap();
        assert!(report.failed);
        assert_eq!(
            session_snapshot(&sid).unwrap().status,
            SupervisedStatus::Failed
        );
        match hook.await.unwrap() {
            SupervisedOutcome::Failed { reason } => assert_eq!(reason, "Reached max turns"),
            other => panic!("expected Failed, got {:?}", other),
        }
        remove_session(&sid);
    }

    #[tokio::test]
    async fn eof_without_result_fails_session_and_clears_gates() {
        let sid = unique_id("eof");
        let mut evs = Vec::new();
        register_session(&sid, SupervisedSessionKind::DispatchedGoal, "proj", "/wt");
        let hook = register_completion_hook(&sid);

        ingest_output(&sid, &format!("{GATE_LINE}\n"), false).unwrap();
        let report = ingest_output(&sid, "", true).unwrap();
        collect_session_events(&mut evs, &sid);
        assert!(report.failed);
        assert_eq!(report.gates_cleared, 1);

        let snap = session_snapshot(&sid).unwrap();
        assert_eq!(snap.status, SupervisedStatus::Failed);
        assert!(snap.pending_gates.is_empty());

        let cleared = of_type(&evs, &PermagentEventType::TerminalGateCleared);
        assert_eq!(cleared.len(), 1);
        assert_eq!(cleared[0].payload["reason"], "session_ended");

        match hook.await.unwrap() {
            SupervisedOutcome::Failed { reason } => {
                assert!(reason.contains("PTY closed"), "{reason}")
            }
            other => panic!("expected Failed, got {:?}", other),
        }

        // A second eof (tee retry) is a no-op — the hook is already consumed.
        let report = ingest_output(&sid, "", true).unwrap();
        assert!(!report.failed);

        remove_session(&sid);
    }

    #[tokio::test]
    async fn full_session_transcript_end_to_end() {
        // A realistic supervised run: init frame, assistant work, a gate,
        // the hand-typed answer echo, more work, the final result — delivered
        // in awkward chunk sizes with PTY noise throughout.
        let sid = unique_id("transcript");
        let mut evs = Vec::new();
        register_session(&sid, SupervisedSessionKind::DispatchedGoal, "proj", "/wt");
        attach_pty(&sid, "pty-tr-1");
        let hook = register_completion_hook(&sid);

        let transcript = format!(
            "\u{1b}]133;D;0\u{7}\u{1b}]7;file://host/wt\u{7}cat '/tmp/x.ndjson' - | 'claude' -p\r\n{SYSTEM_INIT_LINE}\r\n{ASSISTANT_LINE}\r\n{GATE_LINE}\r\n{ANSWER_ECHO_LINE}\r\n{ASSISTANT_LINE}\r\n{RESULT_LINE}\r\n"
        );
        // Feed in ugly fixed-size chunks to exercise reassembly, collecting
        // our bus frames after every chunk (see `collect_session_events`).
        let bytes: Vec<char> = transcript.chars().collect();
        let mut total = IngestReport::default();
        for chunk in bytes.chunks(97) {
            let s: String = chunk.iter().collect();
            let r = ingest_output(&sid, &s, false).unwrap();
            collect_session_events(&mut evs, &sid);
            total.gates_detected += r.gates_detected;
            total.gates_cleared += r.gates_cleared;
            total.completed |= r.completed;
            total.failed |= r.failed;
        }

        assert_eq!(total.gates_detected, 1);
        assert_eq!(total.gates_cleared, 1);
        assert!(total.completed);
        assert!(!total.failed);
        assert_eq!(
            of_type(&evs, &PermagentEventType::TerminalGateDetected).len(),
            1
        );
        assert_eq!(
            of_type(&evs, &PermagentEventType::TerminalGateCleared).len(),
            1
        );
        match hook.await.unwrap() {
            SupervisedOutcome::Completed { summary } => assert_eq!(summary, "Done"),
            other => panic!("expected Completed, got {:?}", other),
        }
        remove_session(&sid);
    }

    #[test]
    fn list_sessions_includes_registered_entry() {
        let sid = unique_id("list");
        register_session(&sid, SupervisedSessionKind::Watched, "proj", "/tmp/p");
        assert!(list_sessions().iter().any(|s| s.session_id == sid));
        remove_session(&sid);
    }

    // ── S3 (#429): gate → Decision Inbox bridge ──
    //
    // In-memory decisions DB per test (no AppState, no PERMAGENT_PATH_ROOT →
    // no #[serial] needed); unique session ids on the shared process-global
    // registry, same as the S2 tests above.

    use crate::decisions;

    async fn memory_pool() -> sqlx::Pool<sqlx::Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::session::spectral_schema::init_spectral_db(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn ingest_report_carries_gate_details_for_the_bridge() {
        let sid = unique_id("report-details");
        register_session(&sid, SupervisedSessionKind::Watched, "proj", "/tmp/p");
        let report = ingest_output(&sid, &format!("{GATE_LINE}\r\n"), false).unwrap();
        assert_eq!(report.gates_detected, 1);
        assert_eq!(report.detected.len(), 1);
        assert_eq!(report.detected[0].request_id, "perm_1");
        assert_eq!(report.detected[0].tool_name, "Write");
        assert_eq!(report.detected[0].input["path"], "foo.txt");

        let report = ingest_output(&sid, &format!("{ANSWER_ECHO_LINE}\r\n"), false).unwrap();
        assert_eq!(report.gates_cleared, 1);
        assert_eq!(report.cleared.len(), 1);
        assert_eq!(report.cleared[0].request_id, "perm_1");
        assert_eq!(report.cleared[0].reason, "answered");
        remove_session(&sid);
    }

    #[tokio::test]
    async fn bridge_files_a_session_gate_decision_classified_by_s4() {
        let pool = memory_pool().await;
        let sid = unique_id("bridge-file");
        register_session(&sid, SupervisedSessionKind::Watched, "proj", "/tmp/p");
        attach_pty(&sid, "pty-br-1");

        let report = ingest_output(&sid, &format!("{GATE_LINE}\r\n"), false).unwrap();
        let outcome = bridge_report_to_inbox(&pool, &sid, &report).await;
        assert_eq!(outcome.decisions_filed, 1);
        assert_eq!(outcome.decisions_superseded, 0);
        assert!(outcome.errors.is_empty(), "errors: {:?}", outcome.errors);

        let d = decisions::find_open_session_gate(&pool, &sid, "perm_1")
            .await
            .unwrap()
            .expect("a session_gate decision must be filed");
        assert_eq!(d.kind, "session_gate");
        // GATE_LINE is a `Write` → S4 classifies it `cc_workspace_edit` (Tier 1:
        // confined, git-reversible edit — Henry-clearable, not user-only).
        assert_eq!(
            d.tier, 1,
            "a Write gate classifies to cc_workspace_edit (Tier 1)"
        );
        assert_eq!(d.payload["target_session_id"], sid.as_str());
        assert_eq!(d.payload["pty_session_id"], "pty-br-1");
        assert_eq!(d.payload["tool_name"], "Write");
        assert_eq!(d.payload["input"]["content"], "hello");
        assert_eq!(d.payload["tool_use_id"], "tu_1");
        assert_eq!(d.payload["options"], serde_json::json!(["allow", "deny"]));
        assert!(
            !d.headline.contains(sid.as_str()),
            "headline must stay plain-language (A1), no technical ids"
        );
        assert!(d.detail.contains("Write"));

        // The typed payload round-trips (deny_unknown_fields) and composes the
        // relay lines — the operator's pre-S5 escape hatch.
        let payload: decisions::SessionGatePayload =
            serde_json::from_value(d.payload.clone()).unwrap();
        let line = decisions::session_gate_relay_line(&payload, true);
        assert!(line.contains("\"behavior\":\"allow\""));
        assert!(line.contains("perm_1"));

        remove_session(&sid);
    }

    #[tokio::test]
    async fn bridge_dedupes_a_reobserved_gate() {
        let pool = memory_pool().await;
        let sid = unique_id("bridge-dedupe");
        register_session(&sid, SupervisedSessionKind::Watched, "proj", "/tmp/p");

        let report = ingest_output(&sid, &format!("{GATE_LINE}\r\n"), false).unwrap();
        assert_eq!(
            bridge_report_to_inbox(&pool, &sid, &report)
                .await
                .decisions_filed,
            1
        );
        // The same report bridged again (tee redelivery) must not double-file.
        assert_eq!(
            bridge_report_to_inbox(&pool, &sid, &report)
                .await
                .decisions_filed,
            0
        );
        remove_session(&sid);
    }

    #[tokio::test]
    async fn bridge_supersedes_when_gate_is_answered_in_the_terminal() {
        let pool = memory_pool().await;
        let sid = unique_id("bridge-answered");
        register_session(&sid, SupervisedSessionKind::Watched, "proj", "/tmp/p");

        let report = ingest_output(&sid, &format!("{GATE_LINE}\r\n"), false).unwrap();
        bridge_report_to_inbox(&pool, &sid, &report).await;
        let open = decisions::find_open_session_gate(&pool, &sid, "perm_1")
            .await
            .unwrap()
            .unwrap();

        // Hand-typed answer echoed in the PTY → gate cleared → card superseded.
        let report = ingest_output(&sid, &format!("{ANSWER_ECHO_LINE}\r\n"), false).unwrap();
        let outcome = bridge_report_to_inbox(&pool, &sid, &report).await;
        assert_eq!(outcome.decisions_superseded, 1);

        let d = decisions::get_decision(&pool, &open.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(d.status, "superseded");
        assert_eq!(
            d.answer_note.as_deref(),
            Some("gate answered in the terminal session")
        );
        assert!(decisions::find_open_session_gate(&pool, &sid, "perm_1")
            .await
            .unwrap()
            .is_none());
        remove_session(&sid);
    }

    #[tokio::test]
    async fn bridge_supersedes_pending_gates_on_session_end() {
        let pool = memory_pool().await;
        let sid = unique_id("bridge-ended");
        register_session(&sid, SupervisedSessionKind::Watched, "proj", "/tmp/p");

        let report = ingest_output(&sid, &format!("{GATE_LINE}\r\n"), false).unwrap();
        bridge_report_to_inbox(&pool, &sid, &report).await;

        // Session finishes with the gate still pending → cleared as
        // session_ended → card superseded with the honest note.
        let report = ingest_output(&sid, &format!("{RESULT_LINE}\r\n"), false).unwrap();
        assert!(report.completed);
        assert_eq!(report.cleared[0].reason, "session_ended");
        let outcome = bridge_report_to_inbox(&pool, &sid, &report).await;
        assert_eq!(outcome.decisions_superseded, 1);

        let history_note = decisions::find_open_session_gate(&pool, &sid, "perm_1")
            .await
            .unwrap();
        assert!(history_note.is_none());
        remove_session(&sid);
    }

    #[tokio::test]
    async fn bridge_is_a_noop_for_an_empty_report() {
        let pool = memory_pool().await;
        let outcome =
            bridge_report_to_inbox(&pool, "sup-never-registered", &IngestReport::default()).await;
        assert_eq!(outcome.decisions_filed, 0);
        assert_eq!(outcome.decisions_superseded, 0);
        assert!(outcome.errors.is_empty());
    }

    // ── S4 (#430): classification → tier, through the bridge ──
    //
    // The classifier itself is unit-tested in `gate_classifier`; these prove the
    // action_class it returns reaches `risk_policy` and resolves to the right
    // tier ON A REAL DECISION filed by the bridge — especially the fail-closed
    // Tier-2 default for an unrecognized tool.

    /// A `can_use_tool` gate line for an arbitrary tool + request id, so a test
    /// can drive a specific S4 classification path.
    fn gate_line(request_id: &str, tool_name: &str) -> String {
        format!(
            r#"{{"type":"control_request","request_id":"{request_id}","request":{{"subtype":"can_use_tool","tool_name":"{tool_name}","input":{{}},"tool_use_id":"tu_{request_id}"}}}}"#
        )
    }

    async fn filed_tier_for_tool(tool_name: &str) -> i64 {
        let pool = memory_pool().await;
        let sid = unique_id("s4");
        register_session(&sid, SupervisedSessionKind::Watched, "proj", "/tmp/p");
        let report = ingest_output(
            &sid,
            &format!("{}\r\n", gate_line("perm_1", tool_name)),
            false,
        )
        .unwrap();
        let outcome = bridge_report_to_inbox(&pool, &sid, &report).await;
        assert_eq!(
            outcome.decisions_filed, 1,
            "a gate for {tool_name} must file exactly one decision"
        );
        let d = decisions::find_open_session_gate(&pool, &sid, "perm_1")
            .await
            .unwrap()
            .expect("decision filed");
        remove_session(&sid);
        d.tier
    }

    #[tokio::test]
    async fn read_only_gate_is_filed_at_tier0() {
        // `Read` → cc_read_only (Tier 0): auto-clearable, no recorded ruling
        // required — the "don't make me babysit" win for pure reads.
        assert_eq!(filed_tier_for_tool("Read").await, 0);
    }

    #[tokio::test]
    async fn edit_gate_is_filed_at_tier1() {
        // `Edit` → cc_workspace_edit (Tier 1): Henry-clearable but recorded.
        assert_eq!(filed_tier_for_tool("Edit").await, 1);
    }

    #[tokio::test]
    async fn shell_gate_is_filed_at_tier2() {
        // `Bash` → cc_shell (Tier 2): the irreversible surface, user-only.
        assert_eq!(filed_tier_for_tool("Bash").await, 2);
    }

    #[tokio::test]
    async fn network_gate_is_filed_at_tier2() {
        // `WebFetch` → network_external (Tier 2, existing seed).
        assert_eq!(filed_tier_for_tool("WebFetch").await, 2);
    }

    #[tokio::test]
    async fn unrecognized_tool_gate_fails_closed_to_tier2() {
        // A tool the classifier does not know (a future CC tool, an MCP tool,
        // a crafted gate) → cc_unclassified, which is UNSEEDED → Tier 2. This
        // is epic #399's escalate-when-unsure guarantee at the decision layer.
        assert_eq!(filed_tier_for_tool("Task").await, 2);
        assert_eq!(
            filed_tier_for_tool("mcp__evil__exfiltrate").await,
            2,
            "an unknown MCP tool must never resolve below Tier 2"
        );
    }

    fn harness_update(run_id: String, status: HarnessRunStatus) -> HarnessRunUpdate {
        let active_node = status.is_active().then(|| "implement".to_string());
        HarnessRunUpdate {
            session_id: format!("session-{run_id}"),
            run_id,
            project: "permagent-runtime".to_string(),
            prompt_title: "Implement DAG-1 observability".to_string(),
            prompt_digest: "a".repeat(64),
            task_version: Some("dag-1/v1".to_string()),
            envelope_id: Some("coding-harness/v1/test".to_string()),
            prompt_context: Some(
                "Plan a cross-cutting architecture DAG with multiple models and security review."
                    .to_string(),
            ),
            dag_nodes: vec!["plan".to_string(), "implement".to_string()],
            dependencies: vec!["plan->implement".to_string()],
            active_node,
            worker: Some("permagent".to_string()),
            provider: Some("local".to_string()),
            model: Some("test-model".to_string()),
            billing_class: Some("local".to_string()),
            tier: Some("harness".to_string()),
            routing_reason: Some("test route".to_string()),
            status,
            declared_verification: Some(HarnessVerification {
                command: "cargo test -p permagent".to_string(),
                verdict: None,
            }),
            last_verification: None,
            verification_attempts: Some(1),
            verification_verdict: None,
            pending_gate: Some(HarnessPendingGate {
                request_id: "gate-1".to_string(),
                tool_name: "Write".to_string(),
                tier: Some("tier_1".to_string()),
            }),
            retry_count: Some(1),
            tool_calls: Some(3),
            gate_attempts: Some(1),
            evidence: Some("cargo test: pass".to_string()),
            result: Some("implemented".to_string()),
            parent_run_id: Some("parent-run".to_string()),
            parent_session_id: None,
        }
    }

    #[test]
    fn harness_run_snapshot_is_structured_and_active_only() {
        let active_id = unique_id("harness-active");
        let terminal_id = unique_id("harness-terminal");
        let active =
            update_harness_run(harness_update(active_id.clone(), HarnessRunStatus::Running))
                .expect("valid run update");
        update_harness_run(harness_update(
            terminal_id.clone(),
            HarnessRunStatus::Succeeded,
        ))
        .expect("valid terminal run update");

        assert_eq!(active.project, "permagent-runtime");
        assert_eq!(active.task_version.as_deref(), Some("dag-1/v1"));
        assert_eq!(active.provider.as_deref(), Some("local"));
        assert_eq!(active.retry_count, Some(1));
        assert_eq!(active.tool_calls, Some(3));
        assert_eq!(active.gate_attempts, Some(1));
        assert_eq!(active.evidence.as_deref(), Some("cargo test: pass"));
        assert_eq!(active.result.as_deref(), Some("implemented"));
        assert_eq!(active.parent_run_id.as_deref(), Some("parent-run"));
        assert_eq!(active.active_node.as_deref(), Some("implement"));
        assert_eq!(active.verification_attempts, Some(1));
        assert_eq!(active.pending_gate.unwrap().request_id, "gate-1");
        assert!(active.elapsed_ms >= 0);
        let active_ids: Vec<_> = list_active_harness_runs()
            .into_iter()
            .map(|run| run.run_id)
            .collect();
        assert!(active_ids.contains(&active_id));
        assert!(!active_ids.contains(&terminal_id));
        assert!(harness_run_snapshot(&terminal_id).is_some());
        remove_harness_run(&active_id);
        remove_harness_run(&terminal_id);
    }

    #[test]
    fn harness_observability_fields_are_bounded_and_old_wire_payloads_default() {
        let mut update = harness_update(unique_id("harness-bounds"), HarnessRunStatus::Succeeded);
        update.evidence = Some("x".repeat(MAX_FIELD_CHARS + 20));
        update.parent_run_id = Some("parent".to_string());
        let snapshot = update_harness_run(update).expect("valid run update");
        assert_eq!(
            snapshot.evidence.as_ref().unwrap().chars().count(),
            MAX_FIELD_CHARS
        );
        assert_eq!(snapshot.parent_run_id.as_deref(), Some("parent"));
        remove_harness_run(&snapshot.run_id);

        let legacy = serde_json::json!({
            "runId": "legacy", "sessionId": "session-legacy",
            "project": "project", "promptTitle": "title", "promptDigest": "digest",
            "dagNodes": ["implement"], "status": "succeeded"
        });
        let decoded: HarnessRunUpdate = serde_json::from_value(legacy).unwrap();
        assert_eq!(decoded.retry_count, None);
        assert_eq!(decoded.tool_calls, None);
        assert_eq!(decoded.gate_attempts, None);
        assert!(decoded.provider.is_none());
        assert!(decoded.parent_run_id.is_none());
    }

    #[test]
    fn harness_run_update_cannot_rebind_a_run_id_to_another_session() {
        let run_id = unique_id("harness-rebind");
        let first = harness_update(run_id.clone(), HarnessRunStatus::Running);
        update_harness_run(first).unwrap();
        let mut conflicting = harness_update(run_id.clone(), HarnessRunStatus::Running);
        conflicting.session_id = "other-session".to_string();
        assert!(update_harness_run(conflicting)
            .unwrap_err()
            .contains("already bound"));
        remove_harness_run(&run_id);
    }

    #[test]
    fn harness_terminal_state_cannot_regress_to_running() {
        let run_id = unique_id("harness-monotonic");
        update_harness_run(harness_update(run_id.clone(), HarnessRunStatus::Succeeded)).unwrap();
        let snapshot =
            update_harness_run(harness_update(run_id.clone(), HarnessRunStatus::Running)).unwrap();
        assert_eq!(snapshot.status, HarnessRunStatus::Succeeded);
        assert!(!list_active_harness_runs()
            .into_iter()
            .any(|run| run.run_id == run_id));
        remove_harness_run(&run_id);
    }

    #[test]
    fn harness_terminal_result_cannot_be_overwritten_or_attempts_decrease() {
        let run_id = unique_id("harness-final");
        let mut running = harness_update(run_id.clone(), HarnessRunStatus::Running);
        running.verification_attempts = Some(4);
        update_harness_run(running).unwrap();

        let mut succeeded = harness_update(run_id.clone(), HarnessRunStatus::Succeeded);
        succeeded.verification_attempts = Some(3);
        let snapshot = update_harness_run(succeeded).unwrap();
        assert_eq!(snapshot.verification_attempts, Some(4));

        let stale_failure = harness_update(run_id.clone(), HarnessRunStatus::Failed);
        let snapshot = update_harness_run(stale_failure).unwrap();
        assert_eq!(snapshot.status, HarnessRunStatus::Succeeded);
        assert_eq!(snapshot.verification_attempts, Some(4));
        remove_harness_run(&run_id);
    }

    #[test]
    fn harness_run_wire_rejects_prompt_bodies() {
        let parsed = serde_json::from_value::<HarnessRunUpdate>(serde_json::json!({
            "runId": "run-1",
            "sessionId": "session-1",
            "project": "project",
            "promptTitle": "safe title",
            "promptDigest": "digest",
            "status": "running",
            "promptBody": "must not enter the registry"
        }));
        assert!(parsed.is_err(), "prompt body must be rejected at the wire");
    }

    #[test]
    fn sparse_heartbeat_preserves_observed_enrichment_and_unknown_stays_unknown() {
        let id = unique_id("harness-enrichment");
        let mut rich = harness_update(id.clone(), HarnessRunStatus::Running);
        update_harness_run(rich.clone()).unwrap();
        rich.task_version = None;
        rich.envelope_id = None;
        rich.provider = None;
        rich.billing_class = None;
        rich.model = None;
        rich.tier = None;
        rich.declared_verification = None;
        rich.evidence = None;
        rich.result = None;
        rich.retry_count = None;
        rich.tool_calls = None;
        rich.gate_attempts = None;
        let snapshot = update_harness_run(rich).unwrap();
        assert_eq!(snapshot.provider.as_deref(), Some("local"));
        assert_eq!(snapshot.billing_class.as_deref(), Some("local"));
        assert_eq!(snapshot.retry_count, Some(1));
        remove_harness_run(&id);

        let unknown = harness_update(
            unique_id("harness-unknown-counters"),
            HarnessRunStatus::Running,
        );
        let unknown = HarnessRunUpdate {
            retry_count: None,
            tool_calls: None,
            gate_attempts: None,
            ..unknown
        };
        let snapshot = update_harness_run(unknown).unwrap();
        assert_eq!(snapshot.retry_count, None);
        assert_eq!(snapshot.tool_calls, None);
        assert_eq!(snapshot.gate_attempts, None);
        remove_harness_run(&snapshot.run_id);
    }

    #[test]
    fn council_recommendation_is_selective_and_respects_opt_out() {
        let broad = recommend_council(
            "Design an architecture DAG across the iOS and server code, compare approaches, and route multiple workers.",
        );
        assert!(broad.recommended);
        assert!(broad.signals.contains(&"architecture".to_string()));
        assert!(!recommend_council("Rename this button to Save.").recommended);
        assert!(
            !recommend_council(
                "Plan the architecture, but do not use the Council for this request."
            )
            .recommended
        );
    }

    #[test]
    fn council_suggestion_is_claimed_once_per_run() {
        let run_id = unique_id("harness-council");
        update_harness_run(harness_update(run_id.clone(), HarnessRunStatus::Running)).unwrap();
        assert!(claim_council_suggestion(&run_id).is_some());
        assert!(claim_council_suggestion(&run_id).is_none());
        remove_harness_run(&run_id);
    }

    #[test]
    fn harness_pending_gate_uses_camel_case_on_the_wire() {
        let parsed = serde_json::from_value::<HarnessRunUpdate>(serde_json::json!({
            "runId": "run-gate",
            "sessionId": "session-gate",
            "project": "project",
            "promptTitle": "safe title",
            "promptDigest": "digest",
            "status": "waiting_gate",
            "dagNodes": ["execute-request"],
            "activeNode": "execute-request",
            "pendingGate": {
                "requestId": "request-1",
                "toolName": "Write",
                "tier": "tier_1"
            }
        }))
        .expect("camel-case nested gate should deserialize");
        assert_eq!(parsed.pending_gate.unwrap().request_id, "request-1");
    }

    #[test]
    fn harness_run_rejects_empty_duplicate_unknown_and_cyclic_graphs() {
        let mut empty = harness_update(unique_id("harness-empty"), HarnessRunStatus::Running);
        empty.dag_nodes.clear();
        empty.dependencies.clear();
        empty.active_node = None;
        assert!(update_harness_run(empty)
            .unwrap_err()
            .contains("at least one DAG node"));

        let mut duplicate =
            harness_update(unique_id("harness-duplicate"), HarnessRunStatus::Running);
        duplicate.dag_nodes = vec!["plan".into(), "plan".into()];
        duplicate.dependencies.clear();
        duplicate.active_node = Some("plan".into());
        assert!(update_harness_run(duplicate)
            .unwrap_err()
            .contains("duplicate node"));

        let mut unknown = harness_update(unique_id("harness-unknown"), HarnessRunStatus::Running);
        unknown.dependencies = vec!["missing->implement".into()];
        assert!(update_harness_run(unknown)
            .unwrap_err()
            .contains("unknown node"));

        let mut cyclic = harness_update(unique_id("harness-cycle"), HarnessRunStatus::Running);
        cyclic.dependencies = vec!["plan->implement".into(), "implement->plan".into()];
        assert!(update_harness_run(cyclic)
            .unwrap_err()
            .contains("dependency cycle"));
    }

    #[test]
    fn harness_run_rejects_impossible_active_node_states() {
        let mut active_without_node =
            harness_update(unique_id("harness-no-active"), HarnessRunStatus::Running);
        active_without_node.active_node = None;
        assert!(update_harness_run(active_without_node)
            .unwrap_err()
            .contains("requires an active DAG node"));

        let mut terminal_with_node = harness_update(
            unique_id("harness-terminal-active"),
            HarnessRunStatus::Succeeded,
        );
        terminal_with_node.active_node = Some("implement".into());
        assert!(update_harness_run(terminal_with_node)
            .unwrap_err()
            .contains("cannot claim an active DAG node"));
    }
}
