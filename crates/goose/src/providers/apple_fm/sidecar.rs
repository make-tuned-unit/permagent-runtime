//! Process management and wire protocol for the `permagent-applefm` sidecar.
//!
//! One long-lived child process per daemon, guarded by an async mutex because
//! FoundationModels rejects overlapping generations on a session. Started
//! lazily on first use; if it dies, the next call starts a fresh one rather
//! than failing permanently.
//!
//! Every failure in here is a *reason to fall back*, never a reason to fail a
//! caller's work. See the module docs in `mod.rs` for that contract.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

/// Name of the compiled sidecar, as produced by `scripts/build-apple-fm.sh`.
const SIDECAR_BIN: &str = "permagent-applefm";

/// A probe does no generation, so it either answers at once or the process is
/// wedged and we want to know quickly.
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// Generous relative to what was measured on an M-series machine (~1.2s warm,
/// ~5.1s on the first call of a cold process with a ~700-token prompt), because
/// the OS can be loading assets underneath us. Long enough not to abandon a
/// call that would have succeeded; short enough that a wedged sidecar cannot
/// stall an overnight archiving pass.
const GENERATE_TIMEOUT: Duration = Duration::from_secs(90);

/// Last context window reported by the sidecar. Populated from every probe and
/// every completed generation, so it tracks the running model rather than a
/// constant compiled in here. Zero means "not yet observed".
static LAST_CONTEXT_SIZE: AtomicUsize = AtomicUsize::new(0);

/// Why the on-device model cannot serve this call.
///
/// The first three arise before the framework is ever reached; the next three
/// are Apple's own `SystemLanguageModel.Availability.UnavailableReason` cases,
/// carried across the wire as stable snake_case strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnavailableReason {
    /// Not macOS. Nothing is spawned and nothing is built.
    UnsupportedPlatform,
    /// Turned off through `PERMAGENT_APPLE_FM_ENABLED`.
    Disabled,
    /// The sidecar was never compiled here (no Xcode, a pre-26 SDK, or a build
    /// that simply did not run `scripts/build-apple-fm.sh`).
    SidecarMissing,
    /// Built against a 26 SDK, running on an older system.
    OsTooOld,
    /// This Mac cannot run Apple Intelligence at all.
    DeviceNotEligible,
    /// The user has Apple Intelligence switched off.
    AppleIntelligenceNotEnabled,
    /// The OS has not finished provisioning the model assets. Inference is
    /// local; the weights still arrive over the network, and until they have,
    /// this is what the framework reports.
    ModelNotReady,
    /// A reason this build does not know about — a newer OS adding a case.
    Other(String),
}

impl UnavailableReason {
    pub fn as_str(&self) -> &str {
        match self {
            Self::UnsupportedPlatform => "unsupported_platform",
            Self::Disabled => "disabled",
            Self::SidecarMissing => "sidecar_missing",
            Self::OsTooOld => "os_too_old",
            Self::DeviceNotEligible => "device_not_eligible",
            Self::AppleIntelligenceNotEnabled => "apple_intelligence_not_enabled",
            Self::ModelNotReady => "model_not_ready",
            Self::Other(s) => s.as_str(),
        }
    }

    fn from_wire(raw: &str) -> Self {
        match raw {
            "unsupported_platform" => Self::UnsupportedPlatform,
            "sidecar_missing" => Self::SidecarMissing,
            "os_too_old" => Self::OsTooOld,
            "device_not_eligible" => Self::DeviceNotEligible,
            "apple_intelligence_not_enabled" => Self::AppleIntelligenceNotEnabled,
            "model_not_ready" => Self::ModelNotReady,
            other => Self::Other(other.to_string()),
        }
    }
}

impl std::fmt::Display for UnavailableReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a runtime probe found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// Ready, with the window the *running* model reports.
    Available {
        context_size: usize,
    },
    Unavailable(UnavailableReason),
}

impl Availability {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available { .. })
    }

    /// A short, stable string for logs: `"available"` or the reason.
    pub fn reason(&self) -> &str {
        match self {
            Self::Available { .. } => "available",
            Self::Unavailable(r) => r.as_str(),
        }
    }
}

/// Anything that stops a call completing.
///
/// [`Self::Unavailable`] and [`Self::Generation`] are both ordinary outcomes —
/// the caller logs the reason and uses another backend.
#[derive(Debug, Clone)]
pub enum AppleFmError {
    /// The model cannot serve requests at all right now.
    Unavailable(UnavailableReason),
    /// The framework reached the model and refused or failed this specific
    /// prompt: a guardrail trip, an over-long prompt, a decoding failure.
    ///
    /// Worth its own variant because availability is *not* sufficient. Observed
    /// on a memory-pressured machine (2026-08-19): `availability` reported
    /// `.available` while the safety subsystem could not load its assets, so
    /// every generation failed. A design that only trusted the probe would have
    /// retried into that wall instead of falling back.
    Generation { kind: String, message: String },
    /// The sidecar itself — missing, unspawnable, timed out, or it died.
    Sidecar(String),
}

impl AppleFmError {
    /// Stable short label for structured logs.
    pub fn reason(&self) -> &str {
        match self {
            Self::Unavailable(r) => r.as_str(),
            Self::Generation { kind, .. } => kind.as_str(),
            Self::Sidecar(_) => "sidecar_error",
        }
    }
}

impl std::fmt::Display for AppleFmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(r) => write!(f, "on-device model unavailable: {}", r),
            Self::Generation { kind, message } => {
                write!(f, "on-device generation failed ({}): {}", kind, message)
            }
            Self::Sidecar(m) => write!(f, "on-device sidecar error: {}", m),
        }
    }
}

/// Where the compiled sidecar lives.
///
/// Mirrors how the system-audio helper is found: next to the running
/// executable in a packaged build, in the source tree during development.
/// Returning `None` is a normal state, not an error — it is what a build that
/// never ran `scripts/build-apple-fm.sh` looks like.
pub fn sidecar_path() -> Option<PathBuf> {
    if let Some(explicit) = crate::config::apple_fm_sidecar_override() {
        let p = PathBuf::from(explicit);
        return p.exists().then_some(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let bundled = dir.join(SIDECAR_BIN);
            if bundled.exists() {
                return Some(bundled);
            }
            // A packaged .app puts executables in Contents/MacOS and staged
            // files in Contents/Resources.
            let resources = dir.join("..").join("Resources").join(SIDECAR_BIN);
            if resources.exists() {
                return Some(resources);
            }
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("applefm")
        .join(SIDECAR_BIN);
    dev.exists().then_some(dev)
}

/// The one reason that can be decided without touching the filesystem or
/// spawning anything: platform and configuration.
///
/// Deliberately a runtime `cfg!` rather than `#[cfg]` blocks. The whole module
/// compiles identically on every target — there is no macOS-only code path to
/// bit-rot and no non-macOS build that can break — and off Darwin every call
/// short-circuits here.
fn preflight() -> Option<UnavailableReason> {
    if !cfg!(target_os = "macos") {
        return Some(UnavailableReason::UnsupportedPlatform);
    }
    if !crate::config::apple_fm_enabled() {
        return Some(UnavailableReason::Disabled);
    }
    None
}

/// The most recent context window observed from the sidecar, if any call has
/// been made yet. Never a compiled-in constant: see [`context_size`].
pub fn last_context_size() -> Option<usize> {
    match LAST_CONTEXT_SIZE.load(Ordering::Relaxed) {
        0 => None,
        n => Some(n),
    }
}

fn record_context_size(value: &Value) {
    if let Some(n) = value.get("context_size").and_then(Value::as_u64) {
        if n > 0 {
            LAST_CONTEXT_SIZE.store(n as usize, Ordering::Relaxed);
        }
    }
}

struct Sidecar {
    child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

impl Sidecar {
    fn spawn() -> Result<Self, AppleFmError> {
        let path =
            sidecar_path().ok_or(AppleFmError::Unavailable(UnavailableReason::SidecarMissing))?;
        let mut child = Command::new(&path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // stderr is the sidecar's own diagnostic channel and is left
            // attached to the daemon's, so a human debugging a silent fallback
            // can see what it said.
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| AppleFmError::Sidecar(format!("could not start {:?}: {}", path, e)))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppleFmError::Sidecar("sidecar stdin unavailable".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppleFmError::Sidecar("sidecar stdout unavailable".into()))?;

        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout).lines(),
            next_id: 1,
        })
    }

    /// Send one request and read until its terminating frame, forwarding any
    /// `delta` frames to `on_delta`.
    async fn exchange<F>(
        &mut self,
        mut request: Value,
        timeout: Duration,
        mut on_delta: F,
    ) -> Result<Value, AppleFmError>
    where
        F: FnMut(&str) + Send,
    {
        let id = self.next_id;
        self.next_id += 1;
        request["id"] = json!(id);

        let mut line = serde_json::to_string(&request)
            .map_err(|e| AppleFmError::Sidecar(format!("could not encode request: {}", e)))?;
        line.push('\n');

        let io = async {
            self.stdin
                .write_all(line.as_bytes())
                .await
                .map_err(|e| AppleFmError::Sidecar(format!("write failed: {}", e)))?;
            self.stdin
                .flush()
                .await
                .map_err(|e| AppleFmError::Sidecar(format!("flush failed: {}", e)))?;

            loop {
                let next = self
                    .stdout
                    .next_line()
                    .await
                    .map_err(|e| AppleFmError::Sidecar(format!("read failed: {}", e)))?;
                let Some(raw) = next else {
                    return Err(AppleFmError::Sidecar("sidecar closed its output".into()));
                };
                let Ok(frame) = serde_json::from_str::<Value>(&raw) else {
                    continue;
                };
                if frame.get("id").and_then(Value::as_u64) != Some(id) {
                    // A frame for an abandoned request (a timed-out call whose
                    // process we chose to keep). Drop it.
                    continue;
                }
                match frame.get("type").and_then(Value::as_str) {
                    Some("delta") => {
                        if let Some(text) = frame.get("text").and_then(Value::as_str) {
                            on_delta(text);
                        }
                    }
                    Some(_) => {
                        record_context_size(&frame);
                        return Ok(frame);
                    }
                    None => continue,
                }
            }
        };

        match tokio::time::timeout(timeout, io).await {
            Ok(result) => result,
            Err(_) => Err(AppleFmError::Sidecar(format!(
                "no response within {}s",
                timeout.as_secs()
            ))),
        }
    }
}

fn slot() -> &'static Mutex<Option<Sidecar>> {
    static SIDECAR: OnceLock<Mutex<Option<Sidecar>>> = OnceLock::new();
    SIDECAR.get_or_init(|| Mutex::new(None))
}

/// The runtime that owns the sidecar process and every pipe attached to it.
///
/// A `tokio::process::Child` and its pipes are registered with the I/O driver
/// of the runtime that created them. Driving them from a *different* runtime
/// does not error loudly — the reads simply never complete, which this design
/// would report as `sidecar_error` and every caller would treat as a reason to
/// fall back. The result would be a feature that silently stopped working,
/// which is the worst failure shape available here.
///
/// That is not hypothetical: it was caught by two `#[tokio::test]` cases, each
/// of which gets its own runtime, racing on the shared child (2026-08-19).
/// Production has one runtime today and would not have shown it.
///
/// So all sidecar I/O is pinned to one runtime of our own, and callers on any
/// runtime await the result over a channel. One worker thread is enough: the
/// framework serves one generation at a time regardless.
fn sidecar_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .thread_name("apple-fm-sidecar")
            .build()
            .expect("failed to build the on-device sidecar runtime")
    })
}

/// Run one exchange against the shared sidecar, starting it if needed and
/// discarding it if the exchange broke the pipe.
///
/// Always executed on [`sidecar_runtime`]. Deltas come back over `delta_tx`
/// rather than through a callback so that nothing borrowed by the caller has to
/// cross the runtime boundary.
async fn with_sidecar(
    request: Value,
    timeout: Duration,
    delta_tx: tokio::sync::mpsc::UnboundedSender<String>,
) -> Result<Value, AppleFmError> {
    if let Some(reason) = preflight() {
        return Err(AppleFmError::Unavailable(reason));
    }

    let mut guard = slot().lock().await;
    if guard.is_none() {
        *guard = Some(Sidecar::spawn()?);
    }

    let sidecar = guard.as_mut().expect("just populated");
    let result = sidecar
        .exchange(request, timeout, |delta| {
            // A dropped receiver means the caller went away mid-response. The
            // exchange still runs to completion so the sidecar is not left
            // half-read for the next call.
            let _ = delta_tx.send(delta.to_string());
        })
        .await;

    if let Err(AppleFmError::Sidecar(ref why)) = result {
        // The process is untrustworthy after a protocol or transport failure —
        // it may be mid-response, wedged, or gone. Drop it; the next call gets
        // a clean one. `kill_on_drop` reaps it.
        tracing::warn!(
            target: "permagent::apple_fm",
            error = %why,
            "on-device sidecar failed; discarding it so the next call starts fresh"
        );
        if let Some(mut dead) = guard.take() {
            let _ = dead.child.start_kill();
        }
    }
    result
}

/// Drive one exchange on the sidecar runtime, delivering deltas to `on_delta`
/// on the caller's runtime as they arrive.
async fn dispatch<F>(
    request: Value,
    timeout: Duration,
    mut on_delta: F,
) -> Result<Value, AppleFmError>
where
    F: FnMut(&str) + Send,
{
    let (delta_tx, mut delta_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let task = sidecar_runtime().spawn(with_sidecar(request, timeout, delta_tx));

    // Drains as the exchange runs — this is what keeps streaming streaming.
    // Ends when the task finishes and drops its sender.
    while let Some(delta) = delta_rx.recv().await {
        on_delta(&delta);
    }

    task.await
        .map_err(|e| AppleFmError::Sidecar(format!("sidecar task failed: {}", e)))?
}

/// Ask the running model whether it can serve requests, and how wide its
/// context is.
///
/// A real round trip every time it is called. Availability is not a static
/// property: a user can switch Apple Intelligence off, and the OS can evict
/// model assets, between one call and the next.
pub async fn availability() -> Availability {
    let frame = match dispatch(json!({"op": "probe"}), PROBE_TIMEOUT, |_| {}).await {
        Ok(f) => f,
        Err(AppleFmError::Unavailable(reason)) => return Availability::Unavailable(reason),
        Err(other) => {
            return Availability::Unavailable(UnavailableReason::Other(other.reason().to_string()))
        }
    };

    if frame.get("available").and_then(Value::as_bool) == Some(true) {
        let context_size = frame
            .get("context_size")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        if context_size == 0 {
            // A probe that says "available" but cannot say how wide the window
            // is has told us nothing we can size a prompt against.
            return Availability::Unavailable(UnavailableReason::Other(
                "probe_missing_context_size".into(),
            ));
        }
        return Availability::Available { context_size };
    }

    let reason = frame
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    Availability::Unavailable(UnavailableReason::from_wire(reason))
}

/// The context window of the running model, read from it rather than assumed.
///
/// It is 4096 on macOS 26.2 — `contextSize` is back-deployed before 26.4 and
/// that shim returns 4096 — and on 26.4 and later the same call returns
/// whatever the installed model actually has. Reading it means this number
/// follows the OS with no change here.
pub async fn context_size() -> Option<usize> {
    match availability().await {
        Availability::Available { context_size } => Some(context_size),
        Availability::Unavailable(_) => None,
    }
}

/// One on-device completion.
///
/// `on_delta` receives incremental text as it is produced. The sidecar has
/// already converted Apple's cumulative snapshots into deltas.
pub async fn generate<F>(
    instructions: &str,
    prompt: &str,
    max_tokens: u32,
    temperature: f32,
    on_delta: F,
) -> Result<String, AppleFmError>
where
    F: FnMut(&str) + Send,
{
    // No pre-probe: the sidecar re-reads availability inside the `generate`
    // handler, so one round trip is both cheaper and more current than a probe
    // followed by a call.
    let request = json!({
        "op": "generate",
        "instructions": instructions,
        "prompt": prompt,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "stream": true,
    });

    let frame = dispatch(request, GENERATE_TIMEOUT, on_delta).await?;

    match frame.get("type").and_then(Value::as_str) {
        Some("done") => Ok(frame
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()),
        Some("error") => {
            let kind = frame
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("generation_failed");
            let message = frame
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            // Unavailability arrives here too, because the sidecar re-probes
            // per request. Keep the distinction the caller cares about.
            match kind {
                "device_not_eligible"
                | "apple_intelligence_not_enabled"
                | "model_not_ready"
                | "os_too_old"
                | "unsupported_platform" => Err(AppleFmError::Unavailable(
                    UnavailableReason::from_wire(kind),
                )),
                _ => Err(AppleFmError::Generation {
                    kind: kind.to_string(),
                    message,
                }),
            }
        }
        _ => Err(AppleFmError::Sidecar(format!(
            "unexpected frame from sidecar: {}",
            frame
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_reasons_round_trip_through_their_wire_names() {
        for reason in [
            UnavailableReason::SidecarMissing,
            UnavailableReason::OsTooOld,
            UnavailableReason::DeviceNotEligible,
            UnavailableReason::AppleIntelligenceNotEnabled,
            UnavailableReason::ModelNotReady,
            UnavailableReason::UnsupportedPlatform,
        ] {
            assert_eq!(UnavailableReason::from_wire(reason.as_str()), reason);
        }
    }

    #[test]
    fn an_unknown_reason_from_a_newer_os_is_carried_through_rather_than_lost() {
        // A future macOS adding an UnavailableReason case must still produce a
        // loggable reason here, not collapse to "unknown".
        let reason = UnavailableReason::from_wire("some_future_case");
        assert_eq!(reason.as_str(), "some_future_case");
    }

    #[test]
    fn preflight_refuses_off_darwin_without_touching_the_filesystem() {
        // The whole point of the runtime `cfg!`: on a non-macOS build this is
        // the only path taken, so no sidecar is ever looked for or spawned.
        if !cfg!(target_os = "macos") {
            assert_eq!(preflight(), Some(UnavailableReason::UnsupportedPlatform));
        }
    }

    /// The sidecar must answer callers on any runtime, not only the one that
    /// happened to start it.
    ///
    /// Regression, caught 2026-08-19: the child process and its pipes were
    /// created on whichever runtime called first, and a second runtime's reads
    /// never completed. That surfaced as `sidecar_error`, which every caller
    /// correctly treats as a reason to fall back — so the feature would have
    /// gone quiet rather than failed, on a machine where it was working.
    ///
    /// A plain `#[test]` so it can build two runtimes of its own, and left to
    /// run in parallel with the other tests here so it also exercises the
    /// shared-process locking.
    #[test]
    fn the_sidecar_answers_callers_on_more_than_one_runtime() {
        let first = tokio::runtime::Runtime::new().expect("first runtime");
        let second = tokio::runtime::Runtime::new().expect("second runtime");

        let from_first = first.block_on(availability());
        let from_second = second.block_on(availability());

        assert_eq!(
            from_first.reason(),
            from_second.reason(),
            "one machine must give one answer, whichever runtime asks"
        );
    }

    #[test]
    fn every_error_has_a_stable_short_reason_for_logs() {
        assert_eq!(
            AppleFmError::Unavailable(UnavailableReason::ModelNotReady).reason(),
            "model_not_ready"
        );
        assert_eq!(
            AppleFmError::Generation {
                kind: "guardrail_violation".into(),
                message: "…".into()
            }
            .reason(),
            "guardrail_violation"
        );
        assert_eq!(
            AppleFmError::Sidecar("boom".into()).reason(),
            "sidecar_error"
        );
    }
}
