//! Tell the daemon a turn just finished, so the Build tab's cost meter moves.
//!
//! WHY THIS EXISTS. The harness and the daemon are two processes sharing one
//! `permagent.db`. Cost is written by the harness, in-process, through the same
//! `append_cost_ledger` the daemon uses — so the numbers are already correct
//! and already durable the instant a turn ends. What is missing is not data,
//! it is NOTIFICATION: the daemon's event bus lives in the daemon's process, so
//! a `session_spend_changed` emitted here would reach nobody, and the browser
//! has never been told that this session id exists at all. The harness mints
//! its own session (`get_or_create_session_id`, "CLI Session") and the Build
//! tab's meter subscribes to the browser's chat session, which is idle for the
//! whole time the user is coding. That is the $0.00.
//!
//! So this sends the smallest possible thing: an id and a "look again". The
//! daemon re-reads the rollup the harness just wrote and announces it on the
//! bus. No figures cross this boundary, because a second writer of
//! `accumulated_cost_usd` would double every number on the meter it feeds.
//!
//! BOUNDED AND OBSERVABLE. A bare `permagent run` in a terminal with no daemon
//! running remains usable, but an announcement failure is logged with its
//! bounded cause. The harness's local cost line remains independent; it must
//! never be presented as proof that the daemon accepted the announcement.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::Serialize;
use sha2::{Digest, Sha256};

/// How long a turn's announcement may take before it is abandoned.
///
/// Short on purpose. This runs on a detached task, so it cannot stall the REPL,
/// but an unbounded request against a wedged daemon would leak one task per
/// turn for the life of the session.
const ANNOUNCE_TIMEOUT: Duration = Duration::from_secs(3);

fn classify_billing(provider: &str) -> Option<String> {
    let p = provider.trim().to_ascii_lowercase();
    if ["ollama", "lmstudio", "lm_studio", "apple_foundation_models"]
        .iter()
        .any(|known| p == *known)
    {
        Some("local".to_string())
    } else if ["claude-code", "claude_code", "codex"]
        .iter()
        .any(|known| p == *known)
    {
        Some("subscription_cli".to_string())
    } else {
        None
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct HarnessRunTelemetrySnapshot {
    retry_count: Option<u32>,
    tool_calls: Option<u32>,
    gate_attempts: Option<u32>,
    pending_gate: Option<serde_json::Value>,
    declared_verification: Option<serde_json::Value>,
    last_verification: Option<serde_json::Value>,
    verification_attempts: Option<u32>,
    verification_verdict: Option<String>,
    evidence: Option<String>,
    result: Option<String>,
    #[allow(dead_code)]
    tool_call_ids: HashSet<String>,
    #[allow(dead_code)]
    gate_attempt_ids: HashSet<String>,
    #[allow(dead_code)]
    verification_request_ids: HashSet<String>,
    #[allow(dead_code)]
    verification_response_ids: HashSet<String>,
}

/// Authoritative observations collected from one agent reply stream.
/// Missing observations stay `None`; an untouched accumulator does not mean
/// that the corresponding runtime activity was zero.
#[derive(Debug, Clone, Default)]
pub struct HarnessRunTelemetry {
    state: Arc<Mutex<HarnessRunTelemetrySnapshot>>,
}

impl HarnessRunTelemetry {
    fn lock_state(&self) -> MutexGuard<'_, HarnessRunTelemetrySnapshot> {
        // Telemetry is best-effort and must never take down the coding turn if
        // a detached reporter poisoned its lock during process shutdown.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn snapshot(&self) -> HarnessRunTelemetrySnapshot {
        self.lock_state().clone()
    }

    pub(crate) fn terminal_result(&self) -> Option<String> {
        self.lock_state().result.clone()
    }

    pub fn record_tool_call(&self, request_id: &str) {
        let mut state = self.lock_state();
        if !state.tool_call_ids.insert(request_id.to_string()) {
            return;
        }
        state.tool_calls = Some(state.tool_calls.unwrap_or(0).saturating_add(1));
    }

    pub fn record_gate_attempt(&self, request_id: &str) {
        let mut state = self.lock_state();
        if !state.gate_attempt_ids.insert(request_id.to_string()) {
            return;
        }
        state.gate_attempts = Some(state.gate_attempts.unwrap_or(0).saturating_add(1));
    }

    pub fn set_pending_gate(&self, request_id: &str, tool_name: &str) {
        let mut state = self.lock_state();
        state.pending_gate = Some(serde_json::json!({
            "requestId": request_id,
            "toolName": tool_name,
            "tier": null,
        }));
    }

    pub fn clear_pending_gate(&self) {
        self.lock_state().pending_gate = None;
    }

    /// Record the start of a built-in verifier attempt. The request id is the
    /// event identity, so replayed streamed messages cannot inflate the count.
    /// The verdict/evidence remain unknown until the paired structured result
    /// arrives.
    pub fn record_verification_request(
        &self,
        request_id: &str,
        arguments: Option<&serde_json::Map<String, serde_json::Value>>,
    ) {
        let mut state = self.lock_state();
        if !state
            .verification_request_ids
            .insert(request_id.to_string())
        {
            return;
        }
        state.verification_attempts =
            Some(state.verification_attempts.unwrap_or(0).saturating_add(1));
        if let Some(declaration) = verification_declaration(arguments) {
            state.declared_verification = Some(declaration);
        }
    }

    /// Consume the authoritative structured result emitted by `verify`. Text
    /// content is intentionally ignored: it is a presentation surface, not a
    /// telemetry protocol.
    pub fn record_verification_result(
        &self,
        request_id: &str,
        result: &rmcp::model::CallToolResult,
    ) {
        let Some(observation) = result.structured_content.as_ref() else {
            return;
        };
        let Ok(observation) = serde_json::from_value::<
            permagent::agents::platform_extensions::developer::verify::VerificationObservation,
        >(observation.clone()) else {
            return;
        };
        if observation.kind
            != permagent::agents::platform_extensions::developer::verify::VERIFICATION_OBSERVATION_KIND
        {
            return;
        }
        let mut state = self.lock_state();
        if !state.verification_request_ids.contains(request_id)
            || !state
                .verification_response_ids
                .insert(request_id.to_string())
        {
            return;
        }
        let incoming_verdict = observation
            .verdict
            .as_deref()
            .map(|v| bounded_telemetry_text(v));
        let incoming_pass = incoming_verdict.as_deref() == Some("pass");
        let current_pass = state.verification_verdict.as_deref() == Some("pass");
        // A passing receipt is terminal for this run projection. Detached or
        // out-of-order heartbeats may be sparse, and a later failure from an
        // older attempt must not make durable telemetry regress from pass.
        if current_pass && !incoming_pass {
            return;
        }
        // A structured envelope without a verdict is still not an approval and
        // must not erase a known verdict/evidence pair.
        let Some(verdict) = incoming_verdict else {
            return;
        };
        state.verification_verdict = Some(verdict);
        if incoming_pass || observation.evidence.is_some() {
            state.evidence = observation
                .evidence
                .as_deref()
                .map(bounded_redacted_telemetry_text);
        }
        // `HarnessVerification.command` is required by the wire contract. An
        // unavailable command therefore remains null rather than becoming a
        // made-up label for an unstructured/transport error.
        if let Some(command) = observation.command.filter(|v| !v.trim().is_empty()) {
            state.last_verification = Some(serde_json::json!({
                "command": bounded_redacted_telemetry_text(&command),
                "verdict": state.verification_verdict.clone(),
            }));
        }
    }

    pub fn set_retry_count(&self, count: u32) {
        self.lock_state().retry_count = Some(count);
    }

    fn set_result(&self, result: &'static str) {
        let mut state = self.lock_state();
        // Terminal observations can arrive in either order (for example, a
        // detached heartbeat may race the awaited terminal update). Keep the
        // strongest structured observation instead of making arrival order
        // the source of truth. Cancellation is strongest, followed by an
        // explicit denial, timeout, failure, and finally success.
        fn precedence(result: &str) -> u8 {
            match result {
                "succeeded" => 1,
                "failed" => 2,
                "timeout" => 3,
                "denied" => 4,
                "cancelled" => 5,
                _ => 0,
            }
        }
        if state
            .result
            .as_deref()
            .map_or(true, |current| precedence(result) > precedence(current))
        {
            state.result = Some(result.to_string());
        }
    }

    pub fn mark_cancelled(&self) {
        self.set_result("cancelled");
    }

    pub fn mark_denied(&self) {
        self.set_result("denied");
    }

    /// Mark a timeout observed by a structured runtime producer. Callers must
    /// not use this for a timeout-looking message; absent a typed observation,
    /// the result remains `failed` or unknown.
    pub fn mark_timeout(&self) {
        self.set_result("timeout");
    }

    /// Record the terminal event emitted by this concrete reply stream. The
    /// event is run-scoped; unlike the HUD's process-global runtime state it
    /// cannot be contaminated by another concurrent session.
    pub fn record_agent_outcome(&self, outcome: permagent::agents::AgentRuntimeOutcome) {
        match outcome {
            permagent::agents::AgentRuntimeOutcome::Succeeded => self.set_result("succeeded"),
            permagent::agents::AgentRuntimeOutcome::Failed => self.set_result("failed"),
            permagent::agents::AgentRuntimeOutcome::Cancelled => self.mark_cancelled(),
        }
    }

    /// Record an error delivered by the reply stream. The error event itself
    /// is authoritative failure evidence; only concrete timeout sources are
    /// upgraded to the distinct timeout result.
    pub fn record_error(&self, error: &anyhow::Error) {
        if is_typed_timeout(error) {
            self.mark_timeout();
        } else {
            self.set_result("failed");
        }
    }

    fn terminal_status(&self, requested: &'static str) -> &'static str {
        match self.lock_state().result.as_deref() {
            Some("cancelled") => "cancelled",
            Some("denied") | Some("timeout") => "failed",
            _ => requested,
        }
    }
}

/// Classify a terminal reply from structured runtime observations only.
///
/// The reply stream exposes its per-run typed terminal outcome. Errors escaping
/// that stream retain their concrete source in the anyhow chain. This
/// deliberately does not inspect display text: a message containing the word
/// "timeout" is not evidence that a timeout occurred.
pub(crate) fn classify_terminal_result(
    response: &anyhow::Result<()>,
    observed_result: Option<&str>,
) -> &'static str {
    if let Err(error) = response {
        if is_typed_timeout(error) {
            return "timeout";
        }
        return "failed";
    }

    match observed_result {
        Some("timeout") => "timeout",
        Some("failed") => "failed",
        Some("cancelled") => "cancelled",
        Some("denied") => "denied",
        Some("succeeded") => "succeeded",
        Some(_) => "unknown",
        // No terminal event means the producer did not provide an
        // authoritative result. Do not manufacture success from `Ok(())`.
        None => "unknown",
    }
}

fn is_typed_timeout(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<tokio::time::error::Elapsed>()
            .is_some()
            || cause
                .downcast_ref::<reqwest::Error>()
                .is_some_and(|error| error.is_timeout())
            || cause
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::TimedOut)
    })
}

const MAX_TELEMETRY_TEXT_CHARS: usize = 512;

fn bounded_telemetry_text(value: &str) -> String {
    value.chars().take(MAX_TELEMETRY_TEXT_CHARS).collect()
}

fn bounded_redacted_telemetry_text(value: &str) -> String {
    bounded_telemetry_text(&permagent::privacy::redact(value))
}

/// Build a declaration only from the verifier's structured arguments. An
/// auto-detected command is intentionally left unknown until the verifier's
/// structured result supplies the resolved command.
fn verification_declaration(
    arguments: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Option<serde_json::Value> {
    let args = arguments?;
    let command = args.get("command").and_then(serde_json::Value::as_str);
    let scope = args.get("scope").and_then(serde_json::Value::as_str);
    let path = args.get("path").and_then(serde_json::Value::as_str);
    let label = command
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            if scope.is_none() && path.is_none() {
                return None;
            }
            let mut parts = vec!["verify".to_string()];
            if let Some(scope) = scope.filter(|value| !value.trim().is_empty()) {
                parts.push(format!("scope={scope}"));
            }
            if let Some(path) = path.filter(|value| !value.trim().is_empty()) {
                parts.push(format!("path={path}"));
            }
            Some(parts.join(" "))
        })?;
    Some(serde_json::json!({
        "command": bounded_redacted_telemetry_text(&label),
        "verdict": null,
    }))
}

/// Announce, in the background, that `session_id` has spent more.
///
/// Returns immediately. `final_turn` marks the session's last word, so the
/// meter can hold a finished session's total rather than letting it decay.
pub fn announce(session_id: &str, final_turn: bool) {
    let session_id = session_id.to_string();
    let working_dir = std::env::current_dir()
        .ok()
        .map(|p| p.display().to_string());
    tokio::spawn(async move {
        if let Err(error) = post(&session_id, working_dir.as_deref(), final_turn).await {
            tracing::warn!(
                target: "permagent::spend_announce",
                session_id = %session_id,
                %error,
                "bounded spend announcement failed"
            );
        }
    });
}

/// Announce and WAIT.
///
/// For the closing announcement only: `announce` detaches, and a detached task
/// is dropped when the process exits — which is every time the session ends, so
/// the final total would be the one announcement guaranteed never to arrive.
/// Still bounded by [`ANNOUNCE_TIMEOUT`], so a wedged daemon delays the exit by
/// seconds rather than hanging it; failures are logged for diagnosis.
pub async fn announce_now(session_id: &str, final_turn: bool) {
    let working_dir = std::env::current_dir()
        .ok()
        .map(|p| p.display().to_string());
    if let Err(error) = post(session_id, working_dir.as_deref(), final_turn).await {
        tracing::warn!(
            target: "permagent::spend_announce",
            session_id = %session_id,
            %error,
            "bounded final spend announcement failed"
        );
    }
}

/// Announce one structured harness snapshot without waiting. The run id is
/// created once by [`start_harness_heartbeat`] and carried through every beat
/// and the terminal update, so resumed sessions remain separate invocations.
fn announce_harness_run(
    run_id: &str,
    session_id: &str,
    prompt: &str,
    provider: Option<&str>,
    model: Option<&str>,
    coding_harness: bool,
    status: &'static str,
    parent_session_id: Option<&str>,
    telemetry: Option<&HarnessRunTelemetry>,
) {
    let announcement = HarnessRunAnnouncement::new_with_parent(
        run_id,
        session_id,
        prompt,
        provider,
        model,
        coding_harness,
        status,
        parent_session_id,
        telemetry.map(HarnessRunTelemetry::snapshot),
    );
    tokio::spawn(async move {
        let _ = post_harness_run(&announcement).await;
    });
}

/// The terminal state is awaited during a headless command's normal cleanup,
/// mirroring [`announce_now`]: a detached task is otherwise dropped as the
/// command exits.
async fn announce_harness_run_now(
    run_id: &str,
    session_id: &str,
    prompt: &str,
    provider: Option<&str>,
    model: Option<&str>,
    coding_harness: bool,
    status: &'static str,
    parent_session_id: Option<&str>,
    telemetry: Option<&HarnessRunTelemetry>,
) {
    let announcement = HarnessRunAnnouncement::new_with_parent(
        run_id,
        session_id,
        prompt,
        provider,
        model,
        coding_harness,
        status,
        parent_session_id,
        telemetry.map(HarnessRunTelemetry::snapshot),
    );
    let _ = post_harness_run(&announcement).await;
}

/// Periodically refresh one active harness turn. Dropping or stopping this
/// handle ends the refresh loop; the daemon independently expires missed
/// beats, so a killed client cannot leave a zombie "running" row.
pub struct HarnessHeartbeat {
    run_id: String,
    session_id: String,
    prompt: String,
    provider: Option<String>,
    model: Option<String>,
    coding_harness: bool,
    parent_session_id: Option<String>,
    telemetry: HarnessRunTelemetry,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl HarnessHeartbeat {
    pub fn telemetry(&self) -> HarnessRunTelemetry {
        self.telemetry.clone()
    }

    #[allow(dead_code)]
    pub async fn finish(self, status: &'static str) {
        self.finish_observed(Some(status)).await;
    }

    /// Finish with an optional producer-supplied result. `None` is retained as
    /// unknown in the wire payload; the compatible status is conservatively
    /// terminal-failed unless an earlier cancellation/denial won precedence.
    pub async fn finish_observed(mut self, result: Option<&'static str>) {
        if let Some(result) = result.filter(|result| *result != "unknown") {
            self.telemetry.set_result(result);
        }
        let status = self.telemetry.terminal_status(result.unwrap_or("failed"));
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let _ = self.task.await;
        announce_harness_run_now(
            &self.run_id,
            &self.session_id,
            &self.prompt,
            self.provider.as_deref(),
            self.model.as_deref(),
            self.coding_harness,
            status,
            self.parent_session_id.as_deref(),
            Some(&self.telemetry),
        )
        .await;
    }
}

pub fn start_harness_heartbeat(
    session_id: &str,
    prompt: &str,
    provider: Option<&str>,
    model: Option<&str>,
    coding_harness: bool,
    parent_session_id: Option<&str>,
) -> HarnessHeartbeat {
    let run_id = format!("harness-{}", uuid::Uuid::new_v4());
    let telemetry = HarnessRunTelemetry::default();
    announce_harness_run(
        &run_id,
        session_id,
        prompt,
        provider,
        model,
        coding_harness,
        "running",
        parent_session_id,
        Some(&telemetry),
    );
    let session_id = session_id.to_string();
    let prompt = prompt.to_string();
    let provider = provider.map(str::to_string);
    let model = model.map(str::to_string);
    let heartbeat_run_id = run_id.clone();
    let heartbeat_session_id = session_id.clone();
    let heartbeat_prompt = prompt.clone();
    let heartbeat_provider = provider.clone();
    let heartbeat_model = model.clone();
    let heartbeat_telemetry = telemetry.clone();
    let parent_session_id = parent_session_id.map(str::to_string);
    let heartbeat_parent_session_id = parent_session_id.clone();
    let (stop, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        interval.tick().await;
        loop {
            tokio::select! {
                _ = &mut stop_rx => break,
                _ = interval.tick() => {
                    announce_harness_run_now(
                        &heartbeat_run_id,
                        &heartbeat_session_id,
                        &heartbeat_prompt,
                        heartbeat_provider.as_deref(),
                        heartbeat_model.as_deref(),
                        coding_harness,
                        "running",
                        heartbeat_parent_session_id.as_deref(),
                        Some(&heartbeat_telemetry),
                    ).await;
                }
            }
        }
    });
    HarnessHeartbeat {
        run_id,
        session_id,
        prompt,
        provider,
        model,
        coding_harness,
        parent_session_id,
        telemetry,
        stop: Some(stop),
        task,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HarnessRunAnnouncement {
    run_id: String,
    session_id: String,
    project: String,
    prompt_title: String,
    prompt_digest: String,
    prompt_context: String,
    task_version: String,
    envelope_id: String,
    dag_nodes: Vec<String>,
    dependencies: Vec<String>,
    active_node: Option<String>,
    worker: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    billing_class: Option<String>,
    tier: Option<String>,
    routing_reason: Option<String>,
    status: &'static str,
    declared_verification: Option<serde_json::Value>,
    last_verification: Option<serde_json::Value>,
    verification_attempts: Option<u32>,
    verification_verdict: Option<String>,
    pending_gate: Option<serde_json::Value>,
    retry_count: Option<u32>,
    tool_calls: Option<u32>,
    gate_attempts: Option<u32>,
    evidence: Option<String>,
    result: Option<String>,
    parent_session_id: Option<String>,
}

impl HarnessRunAnnouncement {
    #[cfg(test)]
    fn new(
        run_id: &str,
        session_id: &str,
        prompt: &str,
        provider: Option<&str>,
        model: Option<&str>,
        coding_harness: bool,
        status: &'static str,
    ) -> Self {
        Self::new_with_parent(
            run_id,
            session_id,
            prompt,
            provider,
            model,
            coding_harness,
            status,
            None,
            None,
        )
    }

    fn new_with_parent(
        run_id: &str,
        session_id: &str,
        prompt: &str,
        provider: Option<&str>,
        model: Option<&str>,
        coding_harness: bool,
        status: &'static str,
        parent_session_id: Option<&str>,
        telemetry: Option<HarnessRunTelemetrySnapshot>,
    ) -> Self {
        let project = std::env::current_dir()
            .ok()
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "unknown-project".to_string());
        let prompt_title = prompt
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("coding harness run")
            .trim()
            .chars()
            .take(120)
            .collect();
        // Every run gets a real, deterministic execution envelope before the
        // first token is spent. The coding recipe gets the three mandatory
        // stages; complex requests can expand this into persistent goal cards
        // after the one plan-approval gate. A bare CLI request remains a valid
        // one-node DAG instead of being mislabeled as a coding roadmap.
        let (dag_nodes, dependencies, active_node, routing_reason) = if coding_harness {
            let active = if matches!(status, "running" | "verifying" | "waiting_gate") {
                Some("execute-request".to_string())
            } else {
                None
            };
            (
                vec![
                    "scope-and-route".to_string(),
                    "execute-request".to_string(),
                    "verify-acceptance".to_string(),
                ],
                vec![
                    "scope-and-route->execute-request".to_string(),
                    "execute-request->verify-acceptance".to_string(),
                ],
                active,
                "zero-token coding DAG envelope; complex work expands after one plan approval"
                    .to_string(),
            )
        } else {
            let active = if matches!(status, "running" | "verifying" | "waiting_gate") {
                Some("execute-request".to_string())
            } else {
                None
            };
            (
                vec!["execute-request".to_string()],
                Vec::new(),
                active,
                "single-node direct CLI DAG".to_string(),
            )
        };
        Self {
            run_id: run_id.to_string(),
            session_id: session_id.to_string(),
            project,
            prompt_title,
            prompt_digest: hex::encode(Sha256::digest(prompt.as_bytes())),
            // Local, bearer-authenticated daemon only. Henry and the Council
            // recommender need the live request to offer useful escalation.
            // The daemon enforces the same 24k character bound independently.
            prompt_context: prompt.chars().take(24_000).collect(),
            task_version: if coding_harness {
                "coding-harness/v1".to_string()
            } else {
                "direct-cli/v1".to_string()
            },
            envelope_id: format!(
                "{}/{}",
                if coding_harness {
                    "coding-harness/v1"
                } else {
                    "direct-cli/v1"
                },
                run_id
            ),
            dag_nodes,
            dependencies,
            active_node,
            worker: Some("permagent".to_string()),
            provider: provider.map(str::to_string),
            model: model.map(str::to_string),
            billing_class: provider.and_then(classify_billing),
            // CLI has no cost-router selection record at this seam. `None`
            // is more honest than assigning it a made-up tier.
            tier: None,
            routing_reason: Some(routing_reason),
            status,
            declared_verification: telemetry
                .as_ref()
                .and_then(|t| t.declared_verification.clone()),
            last_verification: telemetry.as_ref().and_then(|t| t.last_verification.clone()),
            verification_attempts: telemetry.as_ref().and_then(|t| t.verification_attempts),
            verification_verdict: telemetry
                .as_ref()
                .and_then(|t| t.verification_verdict.clone()),
            pending_gate: telemetry.as_ref().and_then(|t| t.pending_gate.clone()),
            retry_count: telemetry.as_ref().and_then(|t| t.retry_count),
            tool_calls: telemetry.as_ref().and_then(|t| t.tool_calls),
            gate_attempts: telemetry.as_ref().and_then(|t| t.gate_attempts),
            evidence: telemetry.as_ref().and_then(|t| t.evidence.clone()),
            result: telemetry.and_then(|t| t.result),
            // Parent identity is the durable session id supplied by the
            // existing Session row; no synthetic run graph is introduced.
            parent_session_id: parent_session_id.map(str::to_string),
        }
    }
}

/// The request itself, separated so it can be awaited (and tested) directly.
async fn post(session_id: &str, working_dir: Option<&str>, final_turn: bool) -> anyhow::Result<()> {
    let port = crate::commands::daemon::read_daemon_port();
    let token = crate::commands::daemon::load_daemon_token()?;
    let client = reqwest::Client::builder()
        .timeout(ANNOUNCE_TIMEOUT)
        .build()?;
    client
        .post(format!("http://127.0.0.1:{port}/api/coding-sessions/spend"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "sessionId": session_id,
            "workingDir": working_dir,
            "finalTurn": final_turn,
        }))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn post_harness_run(announcement: &HarnessRunAnnouncement) -> anyhow::Result<()> {
    let port = crate::commands::daemon::read_daemon_port();
    let token = crate::commands::daemon::load_daemon_token()?;
    let client = reqwest::Client::builder()
        .timeout(ANNOUNCE_TIMEOUT)
        .build()?;
    client
        .post(format!(
            "http://127.0.0.1:{port}/api/coding-sessions/harness-runs"
        ))
        .header("Authorization", format!("Bearer {token}"))
        .json(announcement)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

/// The session's closing line: what the whole session cost.
///
/// The per-turn line (`display_session_cost`) already says "$X this turn · $Y
/// session", but it is printed BEFORE the next prompt — so the last turn's copy
/// scrolls away under whatever the user does next, and a session that ends with
/// `/exit` never prints one at all. A session whose total is only recoverable
/// by scrolling back is a session whose total was not reported.
pub fn format_session_total(session_usd: Option<f64>, total_tokens: i64) -> Option<String> {
    let total = session_usd?;
    // A local model spends tokens and no money. Saying "$0.00" for that reads
    // as a broken meter; saying nothing about the money and reporting the
    // tokens is the honest version — the same distinction `format_cost_line`
    // draws for the per-turn line.
    if total == 0.0 && total_tokens > 0 {
        return Some(format!(
            "Session total: {total_tokens} tokens · no API spend"
        ));
    }
    Some(format!(
        "Session total: ${total:.2} · {total_tokens} tokens"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_session_reports_tokens_rather_than_a_zero_dollar_bill() {
        assert_eq!(
            format_session_total(Some(0.0), 12_400).unwrap(),
            "Session total: 12400 tokens · no API spend"
        );
    }

    #[test]
    fn a_paid_session_reports_what_it_cost() {
        assert_eq!(
            format_session_total(Some(1.2345), 98_000).unwrap(),
            "Session total: $1.23 · 98000 tokens"
        );
    }

    /// No ledger reading at all is not "$0.00" — it is nothing to say. A meter
    /// that invents a zero is indistinguishable from one reporting a free run.
    #[test]
    fn an_unknown_total_says_nothing() {
        assert!(format_session_total(None, 0).is_none());
        assert!(format_session_total(None, 500).is_none());
    }

    /// A session that truly spent nothing and used nothing still reports, so a
    /// session that closed immediately does not look like a reporting failure.
    #[test]
    fn an_empty_session_still_reports_zero() {
        assert_eq!(
            format_session_total(Some(0.0), 0).unwrap(),
            "Session total: $0.00 · 0 tokens"
        );
    }

    #[test]
    fn billing_class_is_only_assigned_for_confident_provider_families() {
        assert_eq!(classify_billing("ollama").as_deref(), Some("local"));
        assert_eq!(
            classify_billing("claude-code").as_deref(),
            Some("subscription_cli")
        );
        assert_eq!(classify_billing("mystery-provider"), None);
    }

    #[test]
    fn resumed_session_invocations_can_use_distinct_run_ids() {
        let first = HarnessRunAnnouncement::new(
            "harness-invocation-1",
            "shared-session",
            "fix the verifier",
            None,
            Some("model"),
            true,
            "running",
        );
        let second = HarnessRunAnnouncement::new(
            "harness-invocation-2",
            "shared-session",
            "fix the verifier",
            None,
            Some("model"),
            true,
            "running",
        );
        assert_ne!(first.run_id, second.run_id);
        assert_eq!(first.session_id, second.session_id);

        let child = HarnessRunAnnouncement::new_with_parent(
            "harness-invocation-3",
            "resumed-child",
            "fix the verifier",
            None,
            Some("model"),
            true,
            "running",
            Some("parent-session"),
            None,
        );
        assert_eq!(child.parent_session_id, Some("parent-session".to_string()));
        assert_eq!(child.parent_session_id.as_deref(), Some("parent-session"));
    }

    #[test]
    fn coding_turns_publish_a_bounded_dag_before_the_first_token() {
        let run = HarnessRunAnnouncement::new(
            "run",
            "session",
            "Fix the verifier",
            Some("ollama"),
            Some("model"),
            true,
            "running",
        );
        assert_eq!(
            run.dag_nodes,
            ["scope-and-route", "execute-request", "verify-acceptance"]
        );
        assert_eq!(run.active_node.as_deref(), Some("execute-request"));
        assert_eq!(run.dependencies.len(), 2);
        assert_eq!(run.provider.as_deref(), Some("ollama"));
        assert_eq!(run.billing_class.as_deref(), Some("local"));
        assert_eq!(run.task_version, "coding-harness/v1");
        assert_eq!(run.envelope_id, "coding-harness/v1/run");
        assert_eq!(run.verification_attempts, None);
        assert!(!run.routing_reason.unwrap().contains("no DAG"));
    }

    #[test]
    fn terminal_updates_do_not_claim_an_active_dag_node() {
        let run = HarnessRunAnnouncement::new(
            "run",
            "session",
            "Fix the verifier",
            None,
            Some("model"),
            true,
            "succeeded",
        );
        assert!(run.active_node.is_none());
    }

    #[test]
    fn telemetry_deduplicates_replayed_runtime_events_and_keeps_unknowns_unknown() {
        let telemetry = HarnessRunTelemetry::default();
        let empty = telemetry.snapshot();
        assert_eq!(empty.tool_calls, None);
        assert_eq!(empty.gate_attempts, None);
        assert_eq!(empty.retry_count, None);

        telemetry.record_tool_call("tool-1");
        telemetry.record_tool_call("tool-1");
        telemetry.record_tool_call("tool-2");
        telemetry.record_gate_attempt("gate-1");
        telemetry.record_gate_attempt("gate-1");
        telemetry.set_pending_gate("gate-1", "shell");
        let observed = telemetry.snapshot();
        assert_eq!(observed.tool_calls, Some(2));
        assert_eq!(observed.gate_attempts, Some(1));
        assert_eq!(
            observed.pending_gate,
            Some(serde_json::json!({
                "requestId": "gate-1",
                "toolName": "shell",
                "tier": null,
            }))
        );
    }

    #[test]
    fn poisoned_telemetry_lock_is_fail_soft() {
        let telemetry = HarnessRunTelemetry::default();
        let state = telemetry.state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = state.lock().unwrap();
            panic!("poison only the test lock");
        })
        .join();
        telemetry.record_tool_call("tool-after-poison");
        assert_eq!(telemetry.snapshot().tool_calls, Some(1));
    }

    #[test]
    fn retry_manager_zero_is_an_authoritative_observation_after_reply_reset() {
        let telemetry = HarnessRunTelemetry::default();
        // Agent::reply resets its retry manager at reply entry. Once the CLI
        // samples it after that reply, Some(0) means no recipe retry occurred;
        // before that sample the field remains unknown.
        assert_eq!(telemetry.snapshot().retry_count, None);
        telemetry.set_retry_count(0);
        assert_eq!(telemetry.snapshot().retry_count, Some(0));
    }

    #[test]
    fn cancellation_result_survives_terminal_status_fallback() {
        let telemetry = HarnessRunTelemetry::default();
        telemetry.mark_cancelled();
        telemetry.set_result("succeeded");
        assert_eq!(telemetry.snapshot().result.as_deref(), Some("cancelled"));
    }

    #[test]
    fn denial_result_survives_terminal_status_fallback() {
        let telemetry = HarnessRunTelemetry::default();
        telemetry.mark_denied();
        telemetry.set_result("succeeded");
        assert_eq!(telemetry.snapshot().result.as_deref(), Some("denied"));
    }

    #[test]
    fn terminal_result_precedence_keeps_timeout_and_failure_distinct() {
        let telemetry = HarnessRunTelemetry::default();
        telemetry.set_result("succeeded");
        telemetry.set_result("failed");
        assert_eq!(telemetry.snapshot().result.as_deref(), Some("failed"));

        telemetry.mark_timeout();
        telemetry.set_result("succeeded");
        assert_eq!(telemetry.snapshot().result.as_deref(), Some("timeout"));

        telemetry.mark_denied();
        telemetry.mark_timeout();
        assert_eq!(telemetry.snapshot().result.as_deref(), Some("denied"));

        telemetry.mark_cancelled();
        telemetry.mark_denied();
        assert_eq!(telemetry.snapshot().result.as_deref(), Some("cancelled"));
    }

    #[test]
    fn per_stream_runtime_events_are_projected_without_global_state() {
        let telemetry = HarnessRunTelemetry::default();
        telemetry.record_agent_outcome(permagent::agents::AgentRuntimeOutcome::Failed);
        telemetry.record_agent_outcome(permagent::agents::AgentRuntimeOutcome::Succeeded);
        assert_eq!(telemetry.terminal_result().as_deref(), Some("failed"));
    }

    #[test]
    fn terminal_status_maps_timeout_to_compatible_failed_status() {
        let telemetry = HarnessRunTelemetry::default();
        telemetry.mark_timeout();
        assert_eq!(telemetry.terminal_status("succeeded"), "failed");
    }

    #[test]
    fn classifier_does_not_parse_timeout_words_from_error_text() {
        let response = Err(anyhow::anyhow!("provider request timed out"));
        assert_eq!(classify_terminal_result(&response, None), "failed");
    }

    #[test]
    fn stream_errors_are_failures_even_without_a_typed_timeout() {
        let telemetry = HarnessRunTelemetry::default();
        telemetry.record_error(&anyhow::anyhow!("provider request timed out"));
        assert_eq!(telemetry.terminal_result().as_deref(), Some("failed"));
    }

    #[test]
    fn classifier_uses_the_per_stream_typed_runtime_outcome() {
        let response = Ok(());
        assert_eq!(
            classify_terminal_result(&response, Some("failed")),
            "failed"
        );
        assert_eq!(
            classify_terminal_result(&response, Some("succeeded")),
            "succeeded"
        );
    }

    #[test]
    fn classifier_uses_typed_timed_out_io_errors() {
        let response = Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "not used for classification",
        )
        .into());
        assert_eq!(classify_terminal_result(&response, None), "timeout");
    }

    #[test]
    fn classifier_leaves_missing_runtime_outcomes_unknown() {
        let response = Ok(());
        assert_eq!(classify_terminal_result(&response, None), "unknown");
    }

    #[tokio::test]
    async fn classifier_uses_tokio_elapsed_errors() {
        let elapsed = tokio::time::timeout(Duration::ZERO, std::future::pending::<()>())
            .await
            .expect_err("pending future must hit the zero timeout");
        let response: anyhow::Result<()> = Err(elapsed.into());
        assert_eq!(classify_terminal_result(&response, None), "timeout");
    }

    #[test]
    fn observed_telemetry_is_carried_into_the_heartbeat_payload() {
        let telemetry = HarnessRunTelemetry::default();
        telemetry.record_tool_call("tool-1");
        telemetry.record_gate_attempt("gate-1");
        telemetry.set_retry_count(0);
        telemetry.set_result("succeeded");
        let run = HarnessRunAnnouncement::new_with_parent(
            "run",
            "session",
            "Fix the verifier",
            None,
            Some("model"),
            true,
            "succeeded",
            None,
            Some(telemetry.snapshot()),
        );
        let json = serde_json::to_value(run).unwrap();
        assert_eq!(json["toolCalls"], 1);
        assert_eq!(json["gateAttempts"], 1);
        assert_eq!(json["retryCount"], 0);
        assert_eq!(json["result"], "succeeded");
        assert!(json["evidence"].is_null());
        assert!(json["parentRunId"].is_null());
    }

    #[test]
    fn structured_verifier_events_populate_attempt_verdict_and_evidence_once() {
        let telemetry = HarnessRunTelemetry::default();
        telemetry.record_verification_request("verify-1", None);
        telemetry.record_verification_request("verify-1", None);
        let mut result = rmcp::model::CallToolResult::success(Vec::new());
        result.structured_content = Some(serde_json::json!({
            "kind": "permagent.verification.v1",
            "command": "cargo test -p permagent",
            "verdict": "fail",
            "evidence": "test failed: expected 1, got 2 at /Users/alice/repo token=super-secret-value"
        }));
        result.is_error = Some(true);
        telemetry.record_verification_result("verify-1", &result);
        telemetry.record_verification_result("verify-1", &result);

        let observed = telemetry.snapshot();
        assert_eq!(observed.verification_attempts, Some(1));
        assert_eq!(observed.verification_verdict.as_deref(), Some("fail"));
        assert_eq!(
            observed.evidence.as_deref(),
            Some("test failed: expected 1, got 2 at [REDACTED]/repo [REDACTED]")
        );
        assert_eq!(
            observed.last_verification,
            Some(serde_json::json!({
                "command": "cargo test -p permagent",
                "verdict": "fail"
            }))
        );
    }

    #[test]
    fn verifier_result_without_structured_observation_keeps_evidence_unknown() {
        let telemetry = HarnessRunTelemetry::default();
        telemetry.record_verification_request("verify-transport-error", None);
        let result = rmcp::model::CallToolResult::error(vec![rmcp::model::Content::text(
            "transport failed",
        )]);
        telemetry.record_verification_result("verify-transport-error", &result);
        let observed = telemetry.snapshot();
        assert_eq!(observed.verification_attempts, Some(1));
        assert_eq!(observed.verification_verdict, None);
        assert_eq!(observed.evidence, None);
        assert_eq!(observed.last_verification, None);
    }

    #[test]
    fn passing_verification_is_monotonic_against_later_sparse_or_failed_receipts() {
        let telemetry = HarnessRunTelemetry::default();
        telemetry.record_verification_request("verify-pass", None);
        let mut pass = rmcp::model::CallToolResult::success(Vec::new());
        pass.structured_content = Some(serde_json::json!({
            "kind": "permagent.verification.v1",
            "command": "cargo test",
            "verdict": "pass",
            "evidence": "all checks passed"
        }));
        telemetry.record_verification_result("verify-pass", &pass);

        telemetry.record_verification_request("verify-old", None);
        let mut old = rmcp::model::CallToolResult::error(Vec::new());
        old.structured_content = Some(serde_json::json!({
            "kind": "permagent.verification.v1",
            "command": "cargo test",
            "verdict": "fail",
            "evidence": "old failure"
        }));
        telemetry.record_verification_result("verify-old", &old);

        let observed = telemetry.snapshot();
        assert_eq!(observed.verification_verdict.as_deref(), Some("pass"));
        assert_eq!(observed.evidence.as_deref(), Some("all checks passed"));
        assert_eq!(
            observed.last_verification,
            Some(serde_json::json!({"command": "cargo test", "verdict": "pass"}))
        );

        // A sparse structured heartbeat cannot manufacture or erase a result.
        telemetry.record_verification_request("verify-sparse", None);
        let mut sparse = rmcp::model::CallToolResult::success(Vec::new());
        sparse.structured_content = Some(serde_json::json!({
            "kind": "permagent.verification.v1",
            "command": null,
            "verdict": null,
            "evidence": null
        }));
        telemetry.record_verification_result("verify-sparse", &sparse);
        assert_eq!(
            telemetry.snapshot().verification_verdict.as_deref(),
            Some("pass")
        );
    }

    #[test]
    fn verifier_declaration_comes_from_structured_arguments_and_is_redacted() {
        let telemetry = HarnessRunTelemetry::default();
        let mut args = serde_json::Map::new();
        args.insert(
            "command".to_string(),
            serde_json::json!("cargo test --token=super-secret-value"),
        );
        telemetry.record_verification_request("verify-declared", Some(&args));
        let observed = telemetry.snapshot();
        assert_eq!(
            observed.declared_verification,
            Some(serde_json::json!({
                "command": "cargo test --[REDACTED]",
                "verdict": null
            }))
        );

        let scoped = HarnessRunTelemetry::default();
        let mut scoped_args = serde_json::Map::new();
        scoped_args.insert("scope".to_string(), serde_json::json!("rust"));
        scoped_args.insert(
            "path".to_string(),
            serde_json::json!("/Users/alice/project"),
        );
        scoped.record_verification_request("verify-scoped", Some(&scoped_args));
        assert_eq!(
            scoped.snapshot().declared_verification,
            Some(serde_json::json!({
                "command": "verify scope=rust path=[REDACTED]/project",
                "verdict": null
            }))
        );
    }
}
