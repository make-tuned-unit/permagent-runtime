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
//! What S2 deliberately does NOT do (later slices, do not fold in):
//! - No Decision-Inbox rows (S3, #429) — the bus event is the S3 input.
//! - No tier/action_class mapping (S4, #430).
//! - No relay of answers into stdin (S5, #431).

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use serde::Serialize;
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

/// How the supervised session was launched (Jesse's "both entry points"
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

/// What one [`ingest_output`] call did — the route returns this verbatim so
/// the tee side (and tests) get evidence, not silence.
#[derive(Debug, Default, Serialize)]
pub struct IngestReport {
    pub gates_detected: usize,
    pub gates_cleared: usize,
    pub completed: bool,
    pub failed: bool,
}

/// The process-wide registry. Global like S1's completion hooks and the event
/// bus: supervised sessions are a per-daemon concern, not per-AppState.
static REGISTRY: Lazy<Mutex<HashMap<String, SessionState>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn with_registry<R>(f: impl FnOnce(&mut HashMap<String, SessionState>) -> R) -> R {
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
        if map.contains_key(session_id) {
            tracing::warn!(
                session_id,
                "supervised session already registered — ignoring"
            );
            return;
        }
        map.insert(
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
    with_registry(|map| match map.get_mut(session_id) {
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
            if map.contains_key(sid) {
                return Some(sid.to_string());
            }
        }
        if let Some(pty) = pty_session_id {
            return map
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
        let state = map.get_mut(session_id)?;
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
    Some(actions.report)
}

/// Snapshot one session (S3's context-load and tests).
pub fn session_snapshot(session_id: &str) -> Option<SessionSnapshot> {
    with_registry(|map| {
        map.get(session_id).map(|s| SessionSnapshot {
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
        map.iter()
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
    with_registry(|map| map.remove(session_id).is_some())
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
}
