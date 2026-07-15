//! Runaway-loop safety — the `ProgressMonitor` tool inspector.
//!
//! Non-negotiable B of the coding harness: *never a loop the user can't get out
//! of, always preserving work*. The old `RepetitionInspector` only caught EXACT
//! consecutive identical tool calls and shipped **disabled** (`new(None)`). This
//! upgrades it into a real progress monitor that is **enabled by default** and
//! escalates a stuck agent through the existing Decision-Inbox path instead of
//! letting it burn turns in a loop.
//!
//! ## The discriminator: state-change
//!
//! The whole design turns on one idea — *identical args AND identical result is
//! a real stall; anything varying is progress*. A call whose arguments or result
//! changes is doing new work and passes untouched; that keeps false-positives
//! low (a legit retry that makes progress is never blocked/escalated).
//!
//! ## Signals (thresholds are exact and unit-tested below)
//!
//! - **S1 repeated identical call** — same `(name,args)` → same `result_hash`.
//!   2 in a window → L1 nudge; **3** → L3 escalate.
//! - **S2 same failure** — S1 where the repeated identical result is an error.
//!   **3** → L3 with reason `Stuck`.
//! - **S3 no-progress cycle** — ≥**4** trailing non-mutating calls whose result
//!   never changes (no new files/edits/observations) → L3.
//! - **S4 oscillation** — two distinct actions A↔B alternating for **6** cycles
//!   (12 calls) → L3.
//! - **S5 monologue** — **3** consecutive assistant turns with 0 tool calls and
//!   no new conclusion → L1 nudge. (Assessed in the agent loop; the pure helper
//!   [`assess_monologue`] lives here.)
//! - **wait/poll/watch tools are EXEMPT from S1–S3** — a legit long-poll must
//!   not be killed (a known real-world false-positive).
//!
//! S7 (the interactive turn budget, 1000 → 50) is enforced in the agent loop.
//! S8 (a $-budget pre-turn stop) is DEFERRED pending the cost ledger — see the
//! `TODO(S8)` seam in `agent.rs`.
//!
//! ## Response ladder (least-disruptive first)
//!
//! - **L1 nudge** (0 human cost): block the offending call and surface a
//!   "try a different approach" message the model self-corrects on.
//! - **L2 ambient** (non-blocking): a `tracing` "possible loop detected" signal
//!   emitted on every detection for observability — never blocks on its own.
//! - **L3 escalate**: raise a *deduplicated* `unblock`/`Stuck` decision via the
//!   existing park path (`orchestrator::escalate_session_loop`).
//! - **L4 hard-stop**: after parking, cancel the worker's `CancellationToken` /
//!   kill it — park-first means the worktree is never reaped, so **work is
//!   preserved**.
//!
//! The pure detection core ([`assess_tool_call`], [`assess_monologue`], and the
//! classifiers) has no I/O so it is directly testable; the `ToolInspector` impl
//! is a thin wrapper that reconstructs the window from the conversation and maps
//! the verdict onto the inspection pipeline.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::agents::self_knowledge::{FeatureCategory, FeatureDescriptor, StateSource};
use crate::config::GooseMode;
use crate::conversation::message::{Message, MessageContent, ToolRequest};
use crate::session::SessionManager;
use crate::tool_inspection::{InspectionAction, InspectionResult, ToolInspector};

/// Inspector name — used by the deny-surfacing path in `agent.rs` to recognize a
/// ProgressMonitor block and show its actionable reason to the model.
pub const PROGRESS_MONITOR_NAME: &str = "progress_monitor";

// ── Thresholds (exact; see the unit tests) ──────────────────────────────────

/// S1: a 2nd identical call+result is nudged (block, tell it to try otherwise).
pub const S1_NUDGE_AT: usize = 2;
/// S1: a 3rd identical call+result escalates.
pub const S1_ESCALATE_AT: usize = 3;
/// S3: 4 trailing non-mutating no-progress calls escalate.
pub const S3_ESCALATE_AT: usize = 4;
/// S4: 6 full A↔B cycles (12 alternating calls) escalate.
pub const S4_ESCALATE_CYCLES: usize = 6;
/// S5: 3 consecutive no-tool, no-new-conclusion turns nudge.
pub const S5_MONOLOGUE_AT: u32 = 3;

/// Rolling per-session window bound — only the most recent N completed tool
/// calls are considered.
pub const WINDOW_MAX: usize = 16;

/// Self-knowledge: the runaway-loop guard, surfaced in the `permagent_self`
/// brief so the agent can describe the constraint it operates under (Guard
/// category, always-on → Static).
pub const SELF_KNOWLEDGE_FEATURE: FeatureDescriptor = FeatureDescriptor {
    id: "runaway_loop_guard",
    display_name: "Runaway-loop guard",
    category: FeatureCategory::Guard,
    what_it_does:
        "A deterministic monitor watches your tool calls over a rolling window and detects when \
         you are stuck — the same call producing the same result, the same failure retried, a \
         no-progress read cycle, or two actions oscillating. It first blocks the offending call \
         and tells you to try a different approach; if you keep looping it escalates to the \
         Decision Inbox and stops the run, always leaving your work in place",
    why_it_matters:
        "It is the safety floor that guarantees you never get trapped in a loop the user can't \
         get out of: identical-and-unchanging work is stopped and handed to the user for \
         direction rather than burning turns and money, and stopping always preserves the diff \
         so nothing you did is thrown away",
    state_source: StateSource::Static,
    teaching: &[],
};

// ── Pure detection core ─────────────────────────────────────────────────────

/// One completed tool call, reconstructed from the conversation.
#[derive(Debug, Clone)]
pub struct ToolEvent {
    pub name: String,
    /// Canonical arguments (`serde_json` compares order-independently).
    pub args: Value,
    /// Hash of the serialized result — the state-change discriminator.
    pub result_hash: u64,
    pub is_error: bool,
    /// Whether the tool can change workspace state (write/edit/shell/…). Used by
    /// S3 to tell "looking around" (no-progress) from "doing work".
    pub is_mutating: bool,
}

/// Which stall signal fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// S1 — same call+result repeated.
    RepeatedCall,
    /// S2 — same call failing the same way.
    SameFailure,
    /// S3 — non-mutating calls with no state change.
    NoProgress,
    /// S4 — two actions oscillating.
    Oscillation,
    /// S5 — consecutive monologue turns.
    Monologue,
}

impl Signal {
    /// Short human label for evidence/headlines.
    pub fn label(self) -> &'static str {
        match self {
            Signal::RepeatedCall => "repeated identical call",
            Signal::SameFailure => "same action failing the same way",
            Signal::NoProgress => "no-progress cycle",
            Signal::Oscillation => "oscillating between two actions",
            Signal::Monologue => "repeating itself without acting",
        }
    }

    /// Finding id for the inspection result.
    fn finding_id(self) -> &'static str {
        match self {
            Signal::RepeatedCall => "LOOP-S1",
            Signal::SameFailure => "LOOP-S2",
            Signal::NoProgress => "LOOP-S3",
            Signal::Oscillation => "LOOP-S4",
            Signal::Monologue => "LOOP-S5",
        }
    }
}

/// The verdict on a single tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopAction {
    /// No loop — allow.
    Pass,
    /// L1 — block this call and nudge the model to change approach.
    Nudge(Signal),
    /// L3 — block, escalate to the Decision Inbox, stop (preserving work).
    Escalate(Signal),
}

impl LoopAction {
    fn signal(self) -> Option<Signal> {
        match self {
            LoopAction::Pass => None,
            LoopAction::Nudge(s) | LoopAction::Escalate(s) => Some(s),
        }
    }
}

/// Tools that legitimately return the same thing many times in a row (long-poll
/// / wait / watch). Exempt from S1–S3 so a real poll loop is never killed.
pub fn is_wait_poll(name: &str) -> bool {
    let n = base_tool_name(name).to_ascii_lowercase();
    // "wait" also covers "await"; keywords are chosen to avoid sloppy substring
    // hits (no bare "tail", which matches "detail").
    ["wait", "poll", "watch", "sleep", "follow_log"]
        .iter()
        .any(|kw| n.contains(kw))
}

/// Tools that can change workspace state. Used by S3: a mutating call is
/// evidence of progress, so a run of them is never a "no-progress cycle".
/// Conservative — anything shell-like counts, so S3 only catches read-only
/// thrash and never a working agent.
pub fn is_mutating(name: &str) -> bool {
    let n = base_tool_name(name).to_ascii_lowercase();
    [
        "write", "edit", "create", "replace", "insert", "patch", "apply", "mkdir", "remove",
        "delete", "rename", "commit", "push", "shell", "bash", "command", "exec", "run", "install",
    ]
    .iter()
    .any(|kw| n.contains(kw))
}

/// Strip an `extension__tool` prefix so heuristics match the bare tool name.
fn base_tool_name(name: &str) -> &str {
    name.rsplit("__").next().unwrap_or(name)
}

/// Hash any string to the `u64` result-hash space.
fn hash_str(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Reconstruct the ordered window of completed tool calls from the conversation.
/// A call is "completed" once its `ToolResponse` (matched by id) has arrived; we
/// emit events in response order (chronological) and keep only the last
/// [`WINDOW_MAX`].
pub fn reconstruct_window(messages: &[Message]) -> Vec<ToolEvent> {
    // id → (name, args) for requests seen so far.
    let mut pending: HashMap<String, (String, Value)> = HashMap::new();
    let mut events: Vec<ToolEvent> = Vec::new();

    for msg in messages {
        for content in &msg.content {
            match content {
                MessageContent::ToolRequest(req) => {
                    if let Ok(call) = &req.tool_call {
                        let args = call
                            .arguments
                            .as_ref()
                            .map(|o| Value::Object(o.clone()))
                            .unwrap_or_default();
                        pending.insert(req.id.clone(), (call.name.to_string(), args));
                    }
                }
                MessageContent::ToolResponse(resp) => {
                    if let Some((name, args)) = pending.remove(&resp.id) {
                        let (is_error, result_hash) = match &resp.tool_result {
                            Ok(r) => {
                                let err = r.is_error.unwrap_or(false);
                                let body = serde_json::to_string(&r.content).unwrap_or_default();
                                (err, hash_str(&body))
                            }
                            Err(e) => {
                                let body = serde_json::to_string(e).unwrap_or_default();
                                (true, hash_str(&body))
                            }
                        };
                        let is_mutating = is_mutating(&name);
                        events.push(ToolEvent {
                            name,
                            args,
                            result_hash,
                            is_error,
                            is_mutating,
                        });
                    }
                }
                _ => {}
            }
        }
    }

    if events.len() > WINDOW_MAX {
        events.drain(0..events.len() - WINDOW_MAX);
    }
    events
}

/// Assess an incoming tool call against the reconstructed window. Pure — no I/O,
/// no state. The whole safety policy lives here so it is directly testable.
pub fn assess_tool_call(window: &[ToolEvent], name: &str, args: &Value) -> LoopAction {
    let win = if window.len() > WINDOW_MAX {
        &window[window.len() - WINDOW_MAX..]
    } else {
        window
    };

    let exempt = is_wait_poll(name);

    // ── S1 / S2: identical (name, args, result) trailing run ────────────────
    // Walk back over calls with this exact fingerprint while the result stays
    // identical. A changed result_hash (progress) or a different fingerprint
    // breaks the run — that is the false-positive guard.
    if !exempt {
        let mut run = 0usize;
        let mut common_hash: Option<u64> = None;
        let mut all_error = true;
        for e in win.iter().rev() {
            if e.name != name || &e.args != args {
                break;
            }
            match common_hash {
                None => common_hash = Some(e.result_hash),
                Some(h) if h == e.result_hash => {}
                Some(_) => break, // result changed → progress → stop the stall run
            }
            run += 1;
            if !e.is_error {
                all_error = false;
            }
        }
        // `run` completed identical calls precede this one; this call is the
        // (run + 1)-th. Escalate at 3 (run ≥ 2), nudge at 2 (run == 1).
        if run + 1 >= S1_ESCALATE_AT {
            let signal = if all_error {
                Signal::SameFailure // S2
            } else {
                Signal::RepeatedCall // S1
            };
            return LoopAction::Escalate(signal);
        }
        if run + 1 >= S1_NUDGE_AT {
            return LoopAction::Nudge(Signal::RepeatedCall);
        }
    }

    // ── S4: oscillation between two distinct actions ────────────────────────
    if let Some(cycles) = trailing_oscillation_cycles(win, name, args) {
        if cycles >= S4_ESCALATE_CYCLES {
            return LoopAction::Escalate(Signal::Oscillation);
        }
    }

    // ── S3: no-progress cycle (read-only thrash, unchanging result) ─────────
    // Only when this call is itself non-mutating — the moment the agent finally
    // acts (a mutating call), we do not escalate.
    if !exempt && !is_mutating(name) {
        let noprog = trailing_no_progress(win);
        if noprog >= S3_ESCALATE_AT {
            return LoopAction::Escalate(Signal::NoProgress);
        }
    }

    LoopAction::Pass
}

/// Length of the trailing run of non-mutating calls whose result never changes
/// (S3). A mutating call or a new result_hash ends the run (that is progress).
fn trailing_no_progress(win: &[ToolEvent]) -> usize {
    let mut count = 0usize;
    let mut common: Option<u64> = None;
    for e in win.iter().rev() {
        if e.is_mutating {
            break;
        }
        match common {
            None => common = Some(e.result_hash),
            Some(h) if h == e.result_hash => {}
            Some(_) => break,
        }
        count += 1;
    }
    count
}

/// If the incoming call `A` continues an A↔B alternation of two *distinct*
/// fingerprints, return the number of full cycles (a cycle == one A/B pair)
/// including this call; otherwise `None`.
fn trailing_oscillation_cycles(win: &[ToolEvent], name: &str, args: &Value) -> Option<usize> {
    let last = win.last()?;
    // The predecessor must be a DIFFERENT fingerprint (otherwise it's S1, not
    // oscillation).
    if last.name == name && &last.args == args {
        return None;
    }
    let b_name = last.name.clone();
    let b_args = last.args.clone();

    // Count the strictly-alternating tail, expecting (from the newest end)
    // B, A, B, A, … The incoming A is counted first.
    let mut events = 1usize; // the incoming A
    let mut expect_b = true;
    for e in win.iter().rev() {
        let is_a = e.name == name && &e.args == args;
        let is_b = e.name == b_name && e.args == b_args;
        if expect_b {
            if is_b {
                events += 1;
                expect_b = false;
            } else {
                break;
            }
        } else if is_a {
            events += 1;
            expect_b = true;
        } else {
            break;
        }
    }
    Some(events / 2)
}

/// S5 — assess a run of consecutive assistant turns that called no tool and
/// produced no new conclusion. Pure; the agent loop maintains the counter.
pub fn assess_monologue(consecutive_no_progress_turns: u32) -> LoopAction {
    if consecutive_no_progress_turns >= S5_MONOLOGUE_AT {
        LoopAction::Nudge(Signal::Monologue)
    } else {
        LoopAction::Pass
    }
}

/// The block message injected as the tool result (L1) — actionable so the model
/// self-corrects rather than repeating.
fn block_message(signal: Signal, name: &str, action: LoopAction) -> String {
    let bare = base_tool_name(name);
    match (signal, action) {
        (Signal::SameFailure, _) => format!(
            "BLOCKED by the runaway-loop guard: `{bare}` has already failed the same way \
             repeatedly. Retrying it identically will not help. Stop and either try a \
             fundamentally different approach, fix the underlying cause, or explain what is \
             blocking you."
        ),
        (_, LoopAction::Escalate(_)) => format!(
            "BLOCKED by the runaway-loop guard: `{bare}` with these exact arguments has already \
             run several times with no change in result. This has been escalated to the user's \
             Decision Inbox. Stop repeating it and try a genuinely different approach."
        ),
        _ => format!(
            "BLOCKED by the runaway-loop guard: `{bare}` with these exact arguments just ran and \
             produced the same result. Repeating it will not make progress — try different \
             arguments, a different tool, or explain what is blocking you."
        ),
    }
}

/// Build a short "last-N actions" evidence block for an escalation decision.
fn evidence_from_window(window: &[ToolEvent], incoming: &str) -> String {
    let mut lines = Vec::new();
    let start = window.len().saturating_sub(5);
    for e in &window[start..] {
        let status = if e.is_error { "error" } else { "ok" };
        lines.push(format!("- {} → {}", e.name, status));
    }
    lines.push(format!("- {incoming} (blocked: about to repeat)"));
    lines.join("\n")
}

// ── ToolInspector wrapper ───────────────────────────────────────────────────

/// The runaway-loop monitor. Reconstructs the per-session window from the
/// conversation, applies the pure policy, blocks looping calls (L1) and, on an
/// L3 verdict, escalates through the Decision Inbox (best-effort, deduplicated).
pub struct ProgressMonitor {
    /// Handle used to reach the daemon pool for L3 escalation. `None` in tests
    /// and in non-daemon contexts → detection + L1 still work, escalation no-ops.
    session_manager: Option<Arc<SessionManager>>,
    /// Sessions already escalated this run — avoids re-spawning the escalation
    /// every turn (the durable dedup is the open decision itself).
    escalated: Mutex<HashSet<String>>,
}

impl std::fmt::Debug for ProgressMonitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProgressMonitor")
            .field("has_session_manager", &self.session_manager.is_some())
            .finish()
    }
}

impl ProgressMonitor {
    /// Enabled-by-default monitor. Pass the session manager in production so L3
    /// can reach the pool; pass `None` in tests.
    pub fn new(session_manager: Option<Arc<SessionManager>>) -> Self {
        Self {
            session_manager,
            escalated: Mutex::new(HashSet::new()),
        }
    }

    /// Clear per-run state (called on retry reset).
    pub fn reset(&self) {
        if let Ok(mut set) = self.escalated.lock() {
            set.clear();
        }
    }

    /// Fire the L3 escalation for a session, once. Best-effort and detached: it
    /// maps the worker session to its goal, raises a deduplicated `unblock`
    /// (`Stuck`) decision with the loop evidence, parks the goal (work is
    /// preserved — park never reaps the worktree) and stops the worker.
    fn escalate(&self, session_id: &str, signal: Signal, evidence: String) {
        let Some(session_manager) = self.session_manager.clone() else {
            return;
        };
        {
            let mut set = match self.escalated.lock() {
                Ok(s) => s,
                Err(_) => return,
            };
            if !set.insert(session_id.to_string()) {
                return; // already escalated this session
            }
        }
        let session_id = session_id.to_string();
        let label = signal.label().to_string();
        tokio::spawn(async move {
            let pool = match session_manager.pool_clone().await {
                Ok(p) => p,
                Err(_) => return,
            };
            if let Err(e) = crate::agents::platform_extensions::orchestrator::escalate_session_loop(
                &pool,
                &session_id,
                &label,
                &evidence,
            )
            .await
            {
                tracing::warn!(
                    target: "permagent::progress_monitor",
                    session_id = %session_id,
                    error = %e,
                    "runaway-loop escalation failed",
                );
            }
        });
    }
}

#[async_trait]
impl ToolInspector for ProgressMonitor {
    fn name(&self) -> &'static str {
        PROGRESS_MONITOR_NAME
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn inspect(
        &self,
        session_id: &str,
        tool_requests: &[ToolRequest],
        messages: &[Message],
        _goose_mode: GooseMode,
    ) -> Result<Vec<InspectionResult>> {
        let window = reconstruct_window(messages);
        let mut results = Vec::new();

        for req in tool_requests {
            let Ok(call) = &req.tool_call else {
                continue;
            };
            let args = call
                .arguments
                .as_ref()
                .map(|o| Value::Object(o.clone()))
                .unwrap_or_default();

            let action = assess_tool_call(&window, call.name.as_ref(), &args);
            let Some(signal) = action.signal() else {
                continue;
            };

            // L2 (ambient, non-blocking): surface every suspected loop to
            // observability/logs regardless of whether we go on to nudge (L1) or
            // escalate (L3). This is the "possible loop" signal — it never blocks
            // on its own.
            tracing::warn!(
                target: "permagent::progress_monitor",
                session_id = %session_id,
                tool = %call.name,
                signal = signal.label(),
                escalating = matches!(action, LoopAction::Escalate(_)),
                "runaway-loop guard: possible loop detected",
            );

            // L3: escalate (once per session) with evidence.
            if matches!(action, LoopAction::Escalate(_)) {
                let evidence = evidence_from_window(&window, call.name.as_ref());
                self.escalate(session_id, signal, evidence);
            }

            // L1/L3 both block the offending call and carry an actionable reason
            // that `agent.rs` surfaces to the model in place of the generic
            // declined text.
            results.push(InspectionResult {
                tool_request_id: req.id.clone(),
                action: InspectionAction::Deny,
                reason: block_message(signal, call.name.as_ref(), action),
                confidence: 1.0,
                inspector_name: PROGRESS_MONITOR_NAME.to_string(),
                finding_id: Some(signal.finding_id().to_string()),
            });
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::CallToolRequestParams;
    use serde_json::json;

    fn ev(name: &str, args: Value, result_hash: u64, is_error: bool) -> ToolEvent {
        ToolEvent {
            name: name.to_string(),
            args,
            result_hash,
            is_error,
            is_mutating: is_mutating(name),
        }
    }

    // ── Classifiers ──

    #[test]
    fn wait_poll_classifier() {
        assert!(is_wait_poll("developer__wait_for_process"));
        assert!(is_wait_poll("poll_status"));
        assert!(is_wait_poll("watch"));
        assert!(!is_wait_poll("developer__text_editor"));
        assert!(!is_wait_poll("read_file"));
    }

    #[test]
    fn mutating_classifier() {
        assert!(is_mutating("developer__text_editor")); // "edit"
        assert!(is_mutating("write_file"));
        assert!(is_mutating("developer__shell"));
        assert!(is_mutating("git_commit"));
        assert!(!is_mutating("read_file"));
        assert!(!is_mutating("list_directory"));
        assert!(!is_mutating("search"));
    }

    // ── S1: repeated identical call ──

    #[test]
    fn s1_nudges_at_two_escalates_at_three() {
        let args = json!({"path": "a.rs"});
        // First repeat (2nd call): nudge, not escalate.
        let win1 = vec![ev("read_file", args.clone(), 1, false)];
        assert_eq!(
            assess_tool_call(&win1, "read_file", &args),
            LoopAction::Nudge(Signal::RepeatedCall),
            "2nd identical call must nudge"
        );
        // Second repeat (3rd call): escalate.
        let win2 = vec![
            ev("read_file", args.clone(), 1, false),
            ev("read_file", args.clone(), 1, false),
        ];
        assert_eq!(
            assess_tool_call(&win2, "read_file", &args),
            LoopAction::Escalate(Signal::RepeatedCall),
            "3rd identical call must escalate (not at 2)"
        );
    }

    #[test]
    fn s1_first_call_passes() {
        let args = json!({"path": "a.rs"});
        assert_eq!(
            assess_tool_call(&[], "read_file", &args),
            LoopAction::Pass,
            "first call ever must pass"
        );
    }

    // ── S2: same action, same failure ──

    #[test]
    fn s2_same_failure_escalates_at_three() {
        let args = json!({"cmd": "cargo build"});
        let win = vec![
            ev("developer__shell", args.clone(), 42, true),
            ev("developer__shell", args.clone(), 42, true),
        ];
        assert_eq!(
            assess_tool_call(&win, "developer__shell", &args),
            LoopAction::Escalate(Signal::SameFailure),
            "3 identical failures must escalate as SameFailure"
        );
    }

    // ── CRITICAL false-positive guard ──

    #[test]
    fn varying_args_never_triggers() {
        // Same tool, different args each time → progress → pass.
        let win = vec![
            ev("read_file", json!({"path": "a.rs"}), 1, false),
            ev("read_file", json!({"path": "b.rs"}), 2, false),
            ev("read_file", json!({"path": "c.rs"}), 3, false),
        ];
        assert_eq!(
            assess_tool_call(&win, "read_file", &json!({"path": "d.rs"})),
            LoopAction::Pass,
            "varying args must pass"
        );
    }

    #[test]
    fn changing_result_hash_never_escalates() {
        // Identical call but the result changes every time → real progress
        // (e.g. a retry that is getting somewhere). Must never escalate.
        let args = json!({"cmd": "poll build"});
        let win = vec![
            ev("check_status", args.clone(), 1, false),
            ev("check_status", args.clone(), 2, false),
            ev("check_status", args.clone(), 3, false),
        ];
        assert_ne!(
            assess_tool_call(&win, "check_status", &args),
            LoopAction::Escalate(Signal::RepeatedCall),
            "a changing result_hash must never escalate"
        );
        // The trailing identical-result run is length 1 → at most a nudge.
        assert_eq!(
            assess_tool_call(&win, "check_status", &args),
            LoopAction::Nudge(Signal::RepeatedCall)
        );
    }

    #[test]
    fn identical_call_with_two_prior_changing_results_only_nudges() {
        // Two identical calls with DIFFERENT results, then repeat: the stall run
        // is broken by the differing result, so no escalation.
        let args = json!({"x": 1});
        let win = vec![
            ev("f", args.clone(), 100, false),
            ev("f", args.clone(), 200, false),
        ];
        assert_eq!(
            assess_tool_call(&win, "f", &args),
            LoopAction::Nudge(Signal::RepeatedCall),
            "differing results cap the response at a nudge"
        );
    }

    // ── wait/poll exemption ──

    #[test]
    fn wait_poll_exempt_from_s1_s3() {
        let args = json!({"pid": 7});
        // Four identical polls that would otherwise trip S1 hard.
        let win = vec![
            ev("developer__wait_for_process", args.clone(), 9, false),
            ev("developer__wait_for_process", args.clone(), 9, false),
            ev("developer__wait_for_process", args.clone(), 9, false),
            ev("developer__wait_for_process", args.clone(), 9, false),
        ];
        assert_eq!(
            assess_tool_call(&win, "developer__wait_for_process", &args),
            LoopAction::Pass,
            "wait/poll tools are exempt from S1–S3"
        );
    }

    // ── S3: no-progress cycle ──

    #[test]
    fn s3_no_progress_escalates_at_four() {
        // Four distinct non-mutating calls (so S1 can't own it) that all return
        // the same result → no new information for four turns.
        let h = 55;
        let win = vec![
            ev("read_file", json!({"path": "a"}), h, false),
            ev("list_dir", json!({"path": "."}), h, false),
            ev("search", json!({"q": "foo"}), h, false),
            ev("read_file", json!({"path": "b"}), h, false),
        ];
        assert_eq!(
            assess_tool_call(&win, "search", &json!({"q": "bar"})),
            LoopAction::Escalate(Signal::NoProgress),
            "4 non-mutating no-progress calls must escalate"
        );
    }

    #[test]
    fn s3_reset_by_new_result_hash() {
        // A changed result_hash mid-run = progress → S3 must not fire.
        let win = vec![
            ev("read_file", json!({"path": "a"}), 55, false),
            ev("list_dir", json!({"path": "."}), 55, false),
            ev("search", json!({"q": "foo"}), 77, false), // new observation
            ev("read_file", json!({"path": "b"}), 55, false),
        ];
        assert_eq!(
            assess_tool_call(&win, "search", &json!({"q": "bar"})),
            LoopAction::Pass,
            "a new result_hash resets the no-progress run"
        );
    }

    #[test]
    fn s3_not_fired_when_incoming_is_mutating() {
        // Even after read-only thrash, if the agent is finally about to act
        // (a mutating call), do not escalate on S3.
        let h = 55;
        let win = vec![
            ev("read_file", json!({"path": "a"}), h, false),
            ev("list_dir", json!({"path": "."}), h, false),
            ev("search", json!({"q": "foo"}), h, false),
            ev("read_file", json!({"path": "b"}), h, false),
        ];
        assert_eq!(
            assess_tool_call(&win, "developer__text_editor", &json!({"path": "a"})),
            LoopAction::Pass,
            "a mutating incoming call must not be S3-escalated"
        );
    }

    // ── S4: oscillation ──

    #[test]
    fn s4_oscillation_escalates_at_six_cycles() {
        let a = json!({"n": "a"});
        let b = json!({"n": "b"});
        // 12 alternating calls ending in B; incoming A → 13 total → 6 cycles.
        let mut win = Vec::new();
        for i in 0..12 {
            if i % 2 == 0 {
                win.push(ev("tool_a", a.clone(), 1, false));
            } else {
                win.push(ev("tool_b", b.clone(), 2, false));
            }
        }
        // window: A B A B A B A B A B A B (ends with B), incoming A.
        assert_eq!(
            assess_tool_call(&win, "tool_a", &a),
            LoopAction::Escalate(Signal::Oscillation),
            "6 full A↔B cycles must escalate"
        );
    }

    #[test]
    fn s4_below_threshold_passes() {
        let a = json!({"n": "a"});
        let b = json!({"n": "b"});
        // Only 2 cycles worth.
        let win = vec![
            ev("tool_a", a.clone(), 1, false),
            ev("tool_b", b.clone(), 2, false),
            ev("tool_a", a.clone(), 1, false),
            ev("tool_b", b.clone(), 2, false),
        ];
        assert_eq!(
            assess_tool_call(&win, "tool_a", &a),
            LoopAction::Pass,
            "a few alternations are normal — must pass"
        );
    }

    // ── S5: monologue ──

    #[test]
    fn s5_monologue_nudges_at_three() {
        assert_eq!(assess_monologue(0), LoopAction::Pass);
        assert_eq!(assess_monologue(1), LoopAction::Pass);
        assert_eq!(assess_monologue(2), LoopAction::Pass);
        assert_eq!(
            assess_monologue(3),
            LoopAction::Nudge(Signal::Monologue),
            "3 consecutive no-progress monologue turns must nudge"
        );
    }

    // ── Window reconstruction ──

    #[test]
    fn window_reconstruction_pairs_requests_and_responses() {
        use rmcp::model::{CallToolResult, Content};

        let call =
            CallToolRequestParams::new("read_file").with_arguments(rmcp::object!({"path": "a.rs"}));
        let req = Message::assistant().with_tool_request("r1", Ok(call));
        let resp = Message::user().with_tool_response(
            "r1",
            Ok(CallToolResult::success(vec![Content::text("hello")])),
        );
        let win = reconstruct_window(&[req, resp]);
        assert_eq!(win.len(), 1);
        assert_eq!(win[0].name, "read_file");
        assert!(!win[0].is_error);
    }

    #[test]
    fn window_bounded_to_max() {
        use rmcp::model::{CallToolResult, Content};
        let mut msgs = Vec::new();
        for i in 0..(WINDOW_MAX + 5) {
            let id = format!("r{i}");
            let call = CallToolRequestParams::new("read_file")
                .with_arguments(rmcp::object!({"i": i as i64}));
            msgs.push(Message::assistant().with_tool_request(&id, Ok(call)));
            msgs.push(
                Message::user()
                    .with_tool_response(&id, Ok(CallToolResult::success(vec![Content::text("x")]))),
            );
        }
        let win = reconstruct_window(&msgs);
        assert_eq!(win.len(), WINDOW_MAX, "window must be bounded");
    }

    // ── Inspector-level: L1 block surfaces an actionable reason ──

    #[tokio::test]
    async fn inspector_denies_repeat_with_progress_monitor_reason() {
        let monitor = ProgressMonitor::new(None); // no escalation in unit tests
        let args = rmcp::object!({"path": "a.rs"});

        // Build a conversation with two prior identical read_file calls.
        let mut messages = Vec::new();
        for i in 0..2 {
            let id = format!("r{i}");
            let call = CallToolRequestParams::new("read_file").with_arguments(args.clone());
            messages.push(Message::assistant().with_tool_request(&id, Ok(call)));
            messages.push(Message::user().with_tool_response(
                &id,
                Ok(rmcp::model::CallToolResult::success(vec![
                    rmcp::model::Content::text("same"),
                ])),
            ));
        }

        // A third identical call is the incoming request.
        let incoming = ToolRequest {
            id: "r_incoming".to_string(),
            tool_call: Ok(CallToolRequestParams::new("read_file").with_arguments(args.clone())),
            metadata: None,
            tool_meta: None,
        };

        let results = monitor
            .inspect("sess-1", &[incoming], &messages, GooseMode::Auto)
            .await
            .unwrap();
        assert_eq!(results.len(), 1, "the repeated call must be flagged");
        assert_eq!(results[0].action, InspectionAction::Deny);
        assert_eq!(results[0].inspector_name, PROGRESS_MONITOR_NAME);
        assert!(
            results[0].reason.contains("runaway-loop guard"),
            "reason must be the actionable block message, got: {}",
            results[0].reason
        );
    }

    #[tokio::test]
    async fn inspector_allows_novel_call() {
        let monitor = ProgressMonitor::new(None);
        let incoming = ToolRequest {
            id: "r1".to_string(),
            tool_call: Ok(CallToolRequestParams::new("read_file")
                .with_arguments(rmcp::object!({"path": "z"}))),
            metadata: None,
            tool_meta: None,
        };
        let results = monitor
            .inspect("sess-1", &[incoming], &[], GooseMode::Auto)
            .await
            .unwrap();
        assert!(results.is_empty(), "a novel call must not be flagged");
    }
}
