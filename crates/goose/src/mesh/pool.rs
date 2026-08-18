//! The pool **engine** behind the `resolve_route` seam (#702) — multi-peer
//! membership, health/capacity gating, trusted-peer scheduling, and the
//! fallback ladder that makes batch offload safe to actually turn on.
//!
//! #702 built the routing *decision* (a single trusted endpoint, trust-gated);
//! #717 built the pure *policy* (the four-condition mesh gate + the
//! mesh→cheap-cloud escalation) with no production inputs provider. This module
//! is the missing middle: the live state those two consult, and the executor
//! that carries a batch request through the ladder. It is **hub-and-spoke
//! request scheduling** — each peer runs an ordinary Ollama (or llama.cpp
//! server / OpenAI-compatible) endpoint and whole requests are placed on the
//! best peer. No tensors are split; layer-splitting stays behind the same
//! seam for a later phase (the M0 verdict: llama.cpp RPC's head aborts on
//! worker death and per-token WAN round-trips sink interactivity — request
//! scheduling is the shippable engine today).
//!
//! ## Trust tiers this engine will serve (and the one it will not)
//!
//! - **Tier 1 — the user's own devices**: private by construction.
//! - **Tier 2 — explicitly-trusted peers**: private by consent — a peer
//!   computing on content can read that content, so a peer is only ever used
//!   when the user marked it `trusted` (and the global
//!   `PERMAGENT_MESH_TRUSTED` switch is on).
//! - **Tier 3 — an open volunteer mesh — is deliberately NOT reachable from
//!   this engine.** Activation-inversion research (USENIX Sec'25, IEEE S&P'25,
//!   CCS'25) reconstructs prompts from split-inference traffic, and request
//!   scheduling sends peers full plaintext prompts outright. There is no
//!   configuration of this module that routes to an untrusted peer.
//!
//! ## The M0 lessons, encoded
//!
//! - **The binding constraint is free capacity at request time** — a busy peer
//!   contributes nothing. Health probes are cheap and periodic (never
//!   per-request-blocking), and a hub-side in-flight cap per peer routes
//!   around a busy node instead of queueing on it.
//! - **Graceful degradation is ours to build** — a dead or slow peer must
//!   never poison the caller. Every rung has a timeout; a peer failure marks
//!   it unhealthy immediately and the request falls to the next rung:
//!   **pool → local → cheap cloud** (the Reader's 60s-timeout-then-degrade
//!   pattern, generalized through [`cost_router::mesh::MESH_TIMEOUT`] and
//!   [`cost_router::mesh::fallback_tier`]).
//! - **Interactive work never waits on the pool** — the #702/#717 workload
//!   wall, enforced here by the gate before any peer is even considered.
//!
//! ## The HARD INVARIANT at the network boundary
//!
//! Only inference computation travels: [`GenerateRequest`] carries a model
//! name, a prompt, an optional system prompt, and sampling options — a Brain
//! store is *unrepresentable* (see [`crate::mesh::InferencePayload`]), and
//! [`build_generate_body`] is the single place request bytes are assembled,
//! unit-tested to carry exactly those fields. [`crate::mesh::assert_inference_only`]
//! is invoked on every dispatch as the stable enforcement seam.
//!
//! ## Flags (everything defaults OFF / local)
//!
//! - `PERMAGENT_MESH_ENGINE` (config key `permagent_mesh_engine`) — master
//!   switch for this engine, default **off**. Off ⇒ the #702 single-endpoint
//!   behavior is preserved byte-for-byte.
//! - `PERMAGENT_MESH_POOL` (config key `permagent_mesh_pool`) — the peers.
//!   Read env-first then `config.yaml` via the existing `get_param` path.
//!   Accepted forms (see [`parse_pool_value`]): a comma/whitespace-separated
//!   endpoint string, or a list whose items are bare endpoint strings or
//!   `{ endpoint, label, trusted }` maps.
//! - `PERMAGENT_MESH_TRUSTED` — the live global consent switch (#702,
//!   unchanged): nothing leaves the machine while it is off, engine or not.
//!
//! The engine flag and peer list are read **once per process** (daemon
//! restart picks up changes); the trust switch stays live per-request so
//! turning it off kills off-device routing instantly.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::cost_router::mesh::{
    gate, MeshGateInputs, MeshIneligible, MeshRoute, MeshWorkload, PoolHealth, MESH_TIMEOUT,
};
use crate::cost_router::tier::Tier;
use crate::mesh::{
    assert_inference_only, trusted_pool_enabled, InferencePayload, InferenceRoute, PrivacyApproach,
    Workload, LOCAL_ENDPOINT,
};

// ── Constants (deliberately constants, not knobs — see the module doc) ──────

/// How often the background prober re-checks every peer.
const PROBE_INTERVAL: Duration = Duration::from_secs(30);
/// Per-probe budget. A peer that cannot answer a model-list request in this
/// window is not a peer that should receive a 60s generation.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// A health verdict older than this is treated as Unknown — fail closed
/// (covers a wedged prober as well as a never-started one).
const STALE_AFTER: Duration = Duration::from_secs(90);
/// Hub-side free-capacity heuristic: at most this many in-flight requests per
/// peer. The M0 lesson is that free RAM at request time is the binding
/// constraint and we cannot observe a peer's RAM from here — so a peer gets
/// one generation at a time, and a busy peer is routed around, never queued
/// on. (A peer-side idle-gate is the real fix and is a contributor feature,
/// not a hub heuristic.)
const MAX_INFLIGHT_PER_PEER: usize = 1;

// ── Pool membership (config) ────────────────────────────────────────────────

/// One configured pool peer: where it is, what to call it, and whether the
/// user has explicitly marked it trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerConfig {
    /// Base URL of the peer's Ollama / OpenAI-compatible server.
    pub endpoint: String,
    /// Short human label for logs and (later) the Settings UI.
    pub label: String,
    /// Per-peer trust. Structured config entries must set this explicitly
    /// (default false — consent is per peer). Bare string entries are `true`
    /// here because their consent switch is the *live global*
    /// `PERMAGENT_MESH_TRUSTED` flag — exactly the #702 single-pool contract.
    /// Routing always requires the global switch AND this flag.
    pub trusted: bool,
}

impl PeerConfig {
    pub fn new(endpoint: impl Into<String>, label: impl Into<String>, trusted: bool) -> Self {
        Self {
            endpoint: endpoint.into(),
            label: label.into(),
            trusted,
        }
    }
}

/// Normalize an endpoint: trim, drop a trailing `/`, require an http(s)
/// scheme. `None` means "not a usable endpoint — skip it".
fn normalize_endpoint(raw: &str) -> Option<String> {
    let e = raw.trim().trim_end_matches('/').to_string();
    if e.is_empty() {
        return None;
    }
    if !(e.starts_with("http://") || e.starts_with("https://")) {
        return None;
    }
    Some(e)
}

/// Default label for an endpoint: the host[:port] with the scheme stripped.
fn label_for_endpoint(endpoint: &str) -> String {
    endpoint
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/')
        .to_string()
}

/// Parse the `permagent_mesh_pool` value into peers. Pure — the config/env
/// read happens in [`configured_peers`].
///
/// Accepted forms:
/// - `String`: comma/whitespace-separated bare endpoints
///   (`"http://a:11434, http://b:11434"`) — the `PERMAGENT_MESH_POOL` env
///   shape, generalized from #702's single endpoint. Bare peers carry
///   `trusted: true` and are gated by the live global trust switch.
/// - `Array`: items are bare endpoint strings (as above) or maps
///   `{ "endpoint": …, "label": …, "trusted": … }` — the `config.yaml` shape.
///   Map entries default to `trusted: false`: marking a peer trusted is an
///   explicit, per-peer act.
///
/// Anything malformed is skipped, never fatal — a config typo must not take
/// batch work down.
fn parse_pool_value(value: Option<&Value>) -> Vec<PeerConfig> {
    let mut peers = Vec::new();
    match value {
        None => {}
        Some(Value::String(s)) => {
            for part in s.split(|c: char| c.is_whitespace() || c == ',') {
                if let Some(endpoint) = normalize_endpoint(part) {
                    let label = label_for_endpoint(&endpoint);
                    peers.push(PeerConfig::new(endpoint, label, true));
                }
            }
        }
        Some(Value::Array(items)) => {
            for item in items {
                match item {
                    Value::String(s) => {
                        if let Some(endpoint) = normalize_endpoint(s) {
                            let label = label_for_endpoint(&endpoint);
                            peers.push(PeerConfig::new(endpoint, label, true));
                        }
                    }
                    Value::Object(map) => {
                        let Some(endpoint) = map
                            .get("endpoint")
                            .and_then(|v| v.as_str())
                            .and_then(normalize_endpoint)
                        else {
                            continue;
                        };
                        let label = map
                            .get("label")
                            .and_then(|v| v.as_str())
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| label_for_endpoint(&endpoint));
                        let trusted = map
                            .get("trusted")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        peers.push(PeerConfig::new(endpoint, label, trusted));
                    }
                    _ => {}
                }
            }
        }
        Some(_) => {}
    }
    peers
}

/// Final membership: dedup explicit peers (first mention wins), and when no
/// explicit peer is configured, fold in the #698 batch host
/// (`config::ollama_host()`) as a single trusted-by-global-flag peer — the
/// same precedence `resolve_pool_endpoint` gave the single-endpoint seam.
fn resolve_peers(explicit: Vec<PeerConfig>, ollama_host: &str) -> Vec<PeerConfig> {
    let mut seen: Vec<PeerConfig> = Vec::new();
    for peer in explicit {
        if !seen.iter().any(|p| p.endpoint == peer.endpoint) {
            seen.push(peer);
        }
    }
    if seen.is_empty() {
        if let Some(host) = normalize_endpoint(ollama_host).filter(|h| h != LOCAL_ENDPOINT) {
            seen.push(PeerConfig::new(host, "ollama-host", true));
        }
    }
    seen
}

/// Truthy check for a flag that may arrive as a YAML bool, an env-parsed
/// number, or a string (`"1" | "true" | "yes" | "on"`, case-insensitive) —
/// the same truthy set as [`crate::mesh::trusted_pool_enabled`].
fn flag_from_value(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64() == Some(1),
        Some(Value::String(s)) => matches!(
            s.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        _ => false,
    }
}

/// Whether the pool engine is switched on (`PERMAGENT_MESH_ENGINE` /
/// `permagent_mesh_engine`). Read through the config layer so it is env-first
/// and config-file-backed; consulted once per process (see [`engine`]).
fn engine_flag_enabled() -> bool {
    let value = crate::config::Config::global()
        .get_param::<Value>("permagent_mesh_engine")
        .ok();
    flag_from_value(value.as_ref())
}

/// The configured pool membership, from `PERMAGENT_MESH_POOL` /
/// `permagent_mesh_pool` (env-first via `get_param`), with the #698 batch
/// host as the no-explicit-config fallback.
fn configured_peers() -> Vec<PeerConfig> {
    let value = crate::config::Config::global()
        .get_param::<Value>("permagent_mesh_pool")
        .ok();
    resolve_peers(
        parse_pool_value(value.as_ref()),
        &crate::config::ollama_host(),
    )
}

// ── Health + capacity state ─────────────────────────────────────────────────

/// A peer's probed health. `Unknown` (never probed, or probe stale) is never
/// routed to — fail closed, mirroring [`PoolHealth::Unknown`] in the #717 gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerHealth {
    Unknown,
    Healthy,
    Unhealthy,
}

/// Mutable per-peer state, guarded by the engine's mutex.
#[derive(Debug, Clone)]
struct PeerState {
    health: PeerHealth,
    /// Model names the peer reported on its last successful probe.
    models: Vec<String>,
    last_probe: Option<Instant>,
    inflight: usize,
}

impl PeerState {
    fn new() -> Self {
        Self {
            health: PeerHealth::Unknown,
            models: Vec::new(),
            last_probe: None,
            inflight: 0,
        }
    }
}

/// An immutable read of one peer's state, as the pure scheduler core sees it.
#[derive(Debug, Clone)]
struct PeerSnapshot {
    health: PeerHealth,
    /// False when never probed or the last probe is older than the staleness
    /// budget — treated exactly like `Unknown` health.
    fresh: bool,
    models: Vec<String>,
    inflight: usize,
}

/// Read-only per-peer status for logs, tests, and the future Settings UI.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PeerStatus {
    pub label: String,
    pub endpoint: String,
    pub trusted: bool,
    pub healthy: bool,
    pub inflight: usize,
    pub models: Vec<String>,
}

/// Does the peer's reported model list serve `want`? Exact match, plus the
/// Ollama shorthand where an untagged name means `:latest`.
fn model_served(models: &[String], want: &str) -> bool {
    let want = want.trim();
    if want.is_empty() {
        return true;
    }
    models
        .iter()
        .any(|m| m == want || (!want.contains(':') && *m == format!("{want}:latest")))
}

// ── The scheduler core (pure) ───────────────────────────────────────────────

/// Pick the peer index a batch unit should run on, or `None` (⇒ stay local).
///
/// Eligibility (every condition required — this IS the trusted-only
/// invariant at selection time):
/// - the live global trust switch is on, AND the peer is per-peer trusted;
/// - the peer's last probe is fresh and Healthy;
/// - the peer serves the requested model (when one is named);
/// - the peer has free capacity (in-flight below the cap) — a busy peer
///   contributes nothing and is routed around.
///
/// Among eligible peers: least-loaded wins; ties rotate round-robin via `rr`.
fn select_idx(
    peers: &[PeerConfig],
    snaps: &[PeerSnapshot],
    globally_trusted: bool,
    model: Option<&str>,
    rr: usize,
    max_inflight: usize,
) -> Option<usize> {
    if !globally_trusted {
        return None;
    }
    let eligible: Vec<usize> = (0..peers.len().min(snaps.len()))
        .filter(|&i| {
            peers[i].trusted
                && snaps[i].fresh
                && snaps[i].health == PeerHealth::Healthy
                && snaps[i].inflight < max_inflight
                && model.is_none_or(|m| model_served(&snaps[i].models, m))
        })
        .collect();
    let min_load = eligible.iter().map(|&i| snaps[i].inflight).min()?;
    let least: Vec<usize> = eligible
        .into_iter()
        .filter(|&i| snaps[i].inflight == min_load)
        .collect();
    Some(least[rr % least.len()])
}

/// Aggregate pool health for the #717 gate: Healthy if any trusted peer is
/// fresh+Healthy; Unknown if no trusted peer has ever been (freshly) probed;
/// otherwise Unhealthy. Capacity is deliberately not part of health — a
/// fully-busy healthy pool gates `UseMesh` and then the scheduler returns
/// `None`, which the ladder treats as "skip the pool rung", not a failure.
fn aggregate_health(peers: &[PeerConfig], snaps: &[PeerSnapshot]) -> PoolHealth {
    let trusted: Vec<usize> = (0..peers.len().min(snaps.len()))
        .filter(|&i| peers[i].trusted)
        .collect();
    if trusted
        .iter()
        .any(|&i| snaps[i].fresh && snaps[i].health == PeerHealth::Healthy)
    {
        return PoolHealth::Healthy;
    }
    if trusted
        .iter()
        .all(|&i| !snaps[i].fresh || snaps[i].health == PeerHealth::Unknown)
    {
        return PoolHealth::Unknown;
    }
    PoolHealth::Unhealthy
}

fn to_gate_workload(workload: Workload) -> MeshWorkload {
    match workload {
        Workload::Batch => MeshWorkload::Batch,
        Workload::Interactive => MeshWorkload::Interactive,
    }
}

// ── Requests, responses, errors ─────────────────────────────────────────────

/// A batch generation request. These fields are the ONLY data this engine can
/// put on the wire — the type-level HARD INVARIANT (a Brain/document/memory
/// store is unrepresentable here), mirroring [`InferencePayload`].
#[derive(Debug, Clone)]
pub struct GenerateRequest {
    /// Session privacy context. `None` denotes autonomous/background work,
    /// which still observes process-wide sovereign mode.
    pub session_id: Option<String>,
    pub model: String,
    pub prompt: String,
    pub system: Option<String>,
    /// Ollama sampling options (temperature, num_predict, …), passed through.
    pub options: Option<Value>,
    /// Ollama model-residency hint (e.g. `"1800s"`) — a load-control knob,
    /// not content. Lets a warm-load ride the same ladder as the work it
    /// warms for (the scheduled Librarian driver).
    pub keep_alive: Option<String>,
    /// Per-request budget override; `None` ⇒ the engine's rung budget
    /// (60s, the Reader's pattern). A cold warm-load of a large model
    /// legitimately needs more.
    pub timeout: Option<Duration>,
    pub workload: Workload,
}

/// The single constructor for bytes this module puts on the wire — the
/// operative form of the HARD INVARIANT. The inner value is private and both
/// constructors accept only inference computation (model/prompt/system/
/// sampling/residency), so a Brain store is unrepresentable at the network
/// boundary *in the code that actually runs*, and
/// [`assert_inference_only`] — the #702 enforcement seam — is now invoked on
/// every production wire-body construction, not just documented.
pub struct InferenceBody(Value);

impl InferenceBody {
    /// Wire body for a non-streaming `/api/generate` (the engine's own
    /// dispatch path).
    pub fn for_generate(req: &GenerateRequest) -> Self {
        assert_inference_only(&InferencePayload::Prompt);
        let mut body = serde_json::Map::new();
        body.insert("model".into(), Value::String(req.model.clone()));
        body.insert("prompt".into(), Value::String(req.prompt.clone()));
        if let Some(system) = &req.system {
            body.insert("system".into(), Value::String(system.clone()));
        }
        body.insert("stream".into(), Value::Bool(false));
        if let Some(options) = &req.options {
            body.insert("options".into(), options.clone());
        }
        if let Some(keep_alive) = &req.keep_alive {
            body.insert("keep_alive".into(), Value::String(keep_alive.clone()));
        }
        Self(Value::Object(body))
    }

    /// Wire body for a streaming `/api/generate` (the Librarian's NDJSON
    /// path builds its request through here, so the leased-endpoint route is
    /// covered by the same choke-point).
    pub fn for_stream(model: &str, prompt: &str, system: &str, options: Value) -> Self {
        assert_inference_only(&InferencePayload::Prompt);
        let mut body = serde_json::Map::new();
        body.insert("model".into(), Value::String(model.to_string()));
        body.insert("prompt".into(), Value::String(prompt.to_string()));
        body.insert("system".into(), Value::String(system.to_string()));
        body.insert("stream".into(), Value::Bool(true));
        body.insert("options".into(), options);
        Self(Value::Object(body))
    }
}

impl InferenceBody {
    /// Wire body for a streaming OpenAI-style `/v1/chat/completions` — the
    /// shape llama-server speaks. Same choke-point, same payload discipline:
    /// a system contract and one prompt, nothing else. Thinking is disabled
    /// per request because the Librarian caps output at `max_tokens`; a
    /// reasoning model would spend the whole budget deliberating and return
    /// no fields.
    pub fn for_chat_stream(
        model: &str,
        prompt: &str,
        system: &str,
        max_tokens: u32,
        temperature: f32,
    ) -> Self {
        assert_inference_only(&InferencePayload::Prompt);
        let mut body = serde_json::Map::new();
        body.insert("model".into(), Value::String(model.to_string()));
        body.insert(
            "messages".into(),
            serde_json::json!([
                {"role": "system", "content": system},
                {"role": "user", "content": prompt},
            ]),
        );
        body.insert("stream".into(), Value::Bool(true));
        body.insert("max_tokens".into(), Value::from(max_tokens));
        body.insert("temperature".into(), Value::from(temperature));
        body.insert("cache_prompt".into(), Value::Bool(true));
        body.insert(
            "chat_template_kwargs".into(),
            serde_json::json!({"enable_thinking": false}),
        );
        Self(Value::Object(body))
    }
}

impl serde::Serialize for InferenceBody {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

/// Which rung actually served a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServedBy {
    /// A trusted pool peer, by label.
    PoolPeer { label: String },
    /// This machine (or the engine-off configured batch endpoint).
    Local,
}

#[derive(Debug, Clone)]
pub struct GenerateResponse {
    pub text: String,
    pub served_by: ServedBy,
}

/// Terminal engine failure: every rung it owns (pool, local) was exhausted.
/// `escalate_to` is the #717 handoff — `Some(Tier::CheapCloud)` for batch
/// work, so a cost-router caller walks the ladder on; callers without a cloud
/// executor treat this as today's plain error.
#[derive(Debug, Clone)]
pub struct PoolError {
    pub message: String,
    pub escalate_to: Option<Tier>,
}

impl std::fmt::Display for PoolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for PoolError {}

/// The tier the ladder hands off to when the engine's own rungs are
/// exhausted: cheap cloud for batch (`cost_router::mesh::fallback_tier`),
/// nothing for interactive (interactive never entered the pool, so there is
/// no mesh ladder to escalate).
fn escalation_for(workload: Workload) -> Option<Tier> {
    match workload {
        Workload::Batch => Some(crate::cost_router::mesh::fallback_tier()),
        Workload::Interactive => None,
    }
}

// ── Engine tuning (constructor injection; prod uses the defaults) ───────────

/// Engine timing/capacity parameters. Production uses [`Tuning::default`];
/// tests inject small values (and a mock `local_endpoint`) so behavior is
/// exercised without env mutation or real waiting — env-mutating tests flake
/// under parallel `cargo test`.
#[derive(Debug, Clone)]
pub struct Tuning {
    pub probe_interval: Duration,
    pub probe_timeout: Duration,
    /// Per-rung request budget — the Reader's 60s pattern, generalized
    /// (`cost_router::mesh::MESH_TIMEOUT`).
    pub request_timeout: Duration,
    pub stale_after: Duration,
    pub max_inflight: usize,
    /// The ladder's safety rung: this machine's own model endpoint.
    pub local_endpoint: String,
    /// Test injection for the live global trust switch; `None` (production)
    /// reads [`trusted_pool_enabled`] per request.
    pub assume_trusted: Option<bool>,
    /// Whether an engine entry point lazily starts the background prober.
    /// True in production (a first request warms the pool with no daemon
    /// startup surface). Tests set it false and drive [`PoolEngine::probe_now`]
    /// themselves, so a request-time `mark_failed` is never racily overwritten
    /// by a concurrent background re-probe.
    pub autostart_prober: bool,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            probe_interval: PROBE_INTERVAL,
            probe_timeout: PROBE_TIMEOUT,
            request_timeout: MESH_TIMEOUT,
            stale_after: STALE_AFTER,
            max_inflight: MAX_INFLIGHT_PER_PEER,
            local_endpoint: LOCAL_ENDPOINT.to_string(),
            assume_trusted: None,
            autostart_prober: true,
        }
    }
}

// ── The engine ──────────────────────────────────────────────────────────────

/// Live pool state + scheduler + dispatcher. One global instance serves the
/// process (see [`engine`]); tests construct their own with explicit peers
/// and tuning.
pub struct PoolEngine {
    peers: Vec<PeerConfig>,
    states: Mutex<Vec<PeerState>>,
    rr: AtomicUsize,
    prober_started: AtomicBool,
    tuning: Tuning,
    client: reqwest::Client,
}

/// RAII in-flight slot on a peer: decrements on drop, so success, failure,
/// timeout, and caller-cancellation all release capacity.
struct InflightGuard {
    engine: Arc<PoolEngine>,
    idx: usize,
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        if let Ok(mut states) = self.engine.states.lock() {
            if let Some(s) = states.get_mut(self.idx) {
                s.inflight = s.inflight.saturating_sub(1);
            }
        }
    }
}

/// A scheduled slot on a specific peer (endpoint + held capacity).
struct PeerLease {
    idx: usize,
    endpoint: String,
    label: String,
    _guard: InflightGuard,
}

impl PoolEngine {
    /// Build an engine over explicit peers. Public for tests and future
    /// daemon wiring; production normally goes through the global [`engine`].
    pub fn with_config(peers: Vec<PeerConfig>, tuning: Tuning) -> Arc<Self> {
        let states = peers.iter().map(|_| PeerState::new()).collect();
        Arc::new(Self {
            peers,
            states: Mutex::new(states),
            rr: AtomicUsize::new(0),
            prober_started: AtomicBool::new(false),
            tuning,
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("reqwest client construction cannot fail without custom TLS"),
        })
    }

    /// The live global trust switch (test-injectable via
    /// [`Tuning::assume_trusted`]).
    fn globally_trusted(&self) -> bool {
        self.tuning
            .assume_trusted
            .unwrap_or_else(trusted_pool_enabled)
    }

    fn snapshots(&self) -> Vec<PeerSnapshot> {
        let states = self.states.lock().expect("pool state lock poisoned");
        states
            .iter()
            .map(|s| PeerSnapshot {
                health: s.health,
                fresh: s
                    .last_probe
                    .is_some_and(|t| t.elapsed() <= self.tuning.stale_after),
                models: s.models.clone(),
                inflight: s.inflight,
            })
            .collect()
    }

    /// Per-peer status for logs/tests/UI.
    pub fn peer_statuses(&self) -> Vec<PeerStatus> {
        let snaps = self.snapshots();
        self.peers
            .iter()
            .zip(snaps.iter())
            .map(|(p, s)| PeerStatus {
                label: p.label.clone(),
                endpoint: p.endpoint.clone(),
                trusted: p.trusted,
                healthy: s.fresh && s.health == PeerHealth::Healthy,
                inflight: s.inflight,
                models: s.models.clone(),
            })
            .collect()
    }

    /// Live inputs for the #717 mesh gate — the production
    /// [`MeshGateInputs`] provider that module was written to receive.
    pub fn gate_inputs(&self, workload: Workload) -> MeshGateInputs {
        let snaps = self.snapshots();
        MeshGateInputs {
            workload: to_gate_workload(workload),
            pool_configured: !self.peers.is_empty(),
            trusted: self.globally_trusted() && self.peers.iter().any(|p| p.trusted),
            health: aggregate_health(&self.peers, &snaps),
        }
    }

    /// Gate a workload, surfacing (once per boot) the silent-revocation case:
    /// peers are configured but none is usable for lack of trust — the user
    /// almost certainly expected offload and is not getting it.
    fn gate_observed(&self, workload: Workload) -> MeshRoute {
        let verdict = gate(self.gate_inputs(workload));
        if matches!(verdict, MeshRoute::Ineligible(MeshIneligible::Untrusted)) {
            crate::mesh::warn_pool_untrusted_once(&format!(
                "{} pool peer(s) configured",
                self.peers.len()
            ));
        }
        verdict
    }

    /// Atomically select the best eligible peer and take an in-flight slot on
    /// it. `None` ⇒ no eligible peer right now (stay local).
    fn try_lease(self: &Arc<Self>, model: Option<&str>) -> Option<PeerLease> {
        let globally_trusted = self.globally_trusted();
        let rr = self.rr.fetch_add(1, Ordering::Relaxed);
        let mut states = self.states.lock().expect("pool state lock poisoned");
        let snaps: Vec<PeerSnapshot> = states
            .iter()
            .map(|s| PeerSnapshot {
                health: s.health,
                fresh: s
                    .last_probe
                    .is_some_and(|t| t.elapsed() <= self.tuning.stale_after),
                models: s.models.clone(),
                inflight: s.inflight,
            })
            .collect();
        let idx = select_idx(
            &self.peers,
            &snaps,
            globally_trusted,
            model,
            rr,
            self.tuning.max_inflight,
        )?;
        states[idx].inflight += 1;
        drop(states);
        Some(PeerLease {
            idx,
            endpoint: self.peers[idx].endpoint.clone(),
            label: self.peers[idx].label.clone(),
            _guard: InflightGuard {
                engine: Arc::clone(self),
                idx,
            },
        })
    }

    /// Fast negative feedback: a request-time failure marks the peer
    /// Unhealthy immediately (the periodic probe re-admits it on recovery).
    fn mark_failed(&self, idx: usize) {
        if let Ok(mut states) = self.states.lock() {
            if let Some(s) = states.get_mut(idx) {
                s.health = PeerHealth::Unhealthy;
                s.last_probe = Some(Instant::now());
            }
        }
    }

    fn record_probe(&self, idx: usize, outcome: Result<Vec<String>, String>) {
        if let Ok(mut states) = self.states.lock() {
            if let Some(s) = states.get_mut(idx) {
                s.last_probe = Some(Instant::now());
                match outcome {
                    Ok(models) => {
                        s.health = PeerHealth::Healthy;
                        s.models = models;
                    }
                    Err(_) => {
                        s.health = PeerHealth::Unhealthy;
                        s.models.clear();
                    }
                }
            }
        }
    }

    /// Probe every peer once, concurrently. Cheap (model-list GETs with a
    /// short budget) and off the request path — tests call this directly for
    /// determinism; production runs it on the [`ensure_prober`] loop.
    pub async fn probe_now(&self) {
        let probes = self.peers.iter().enumerate().map(|(i, p)| {
            let client = self.client.clone();
            let endpoint = p.endpoint.clone();
            let timeout = self.tuning.probe_timeout;
            async move {
                let outcome = if crate::sovereignty::global_sovereign_mode()
                    && !crate::mesh::endpoint_is_loopback(&endpoint)
                {
                    Err(format!(
                        "{} non-loopback mesh probe blocked by sovereign mode",
                        crate::sovereignty::SOVEREIGN_BLOCK_PREFIX
                    ))
                } else {
                    probe_endpoint(&client, &endpoint, timeout).await
                };
                (i, outcome)
            }
        });
        for (i, outcome) in futures::future::join_all(probes).await {
            if let Err(e) = &outcome {
                tracing::debug!(peer = %self.peers[i].label, error = %e, "mesh pool probe failed");
            }
            self.record_probe(i, outcome);
        }
    }

    /// Start the periodic health prober exactly once, if a tokio runtime is
    /// available. Called lazily from every engine entry point — no daemon
    /// startup surface required. A no-op when [`Tuning::autostart_prober`] is
    /// false (tests drive [`Self::probe_now`] directly for determinism).
    pub fn ensure_prober(self: &Arc<Self>) {
        if !self.tuning.autostart_prober {
            return;
        }
        if self.prober_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            // Not on a runtime yet — allow a later (async) caller to start it.
            self.prober_started.store(false, Ordering::SeqCst);
            return;
        };
        let engine = Arc::clone(self);
        handle.spawn(async move {
            loop {
                engine.probe_now().await;
                tokio::time::sleep(engine.tuning.probe_interval).await;
            }
        });
    }

    /// Whether the prober loop has been started (test observability).
    pub fn prober_started(&self) -> bool {
        self.prober_started.load(Ordering::SeqCst)
    }

    /// Synchronous, advisory route for the [`crate::mesh::resolve_route`]
    /// seam: the best eligible peer right now, else local. Reads the cached
    /// health snapshot only — never blocks, never probes inline.
    pub(crate) fn resolve_route_sync(self: &Arc<Self>, workload: Workload) -> InferenceRoute {
        if workload == Workload::Interactive {
            return InferenceRoute::local();
        }
        self.ensure_prober();
        let snaps = self.snapshots();
        let rr = self.rr.load(Ordering::Relaxed);
        match select_idx(
            &self.peers,
            &snaps,
            self.globally_trusted(),
            None,
            rr,
            self.tuning.max_inflight,
        ) {
            Some(idx) => InferenceRoute::new(
                self.peers[idx].endpoint.clone(),
                PrivacyApproach::TrustedPool,
            ),
            None => InferenceRoute::local(),
        }
    }

    /// Take a scheduled endpoint for a caller that runs its own HTTP (the
    /// Librarian's streaming path): a trusted healthy peer when the gate
    /// admits it, else the local endpoint.
    pub fn lease_for(self: &Arc<Self>, model: Option<&str>) -> BatchLease {
        self.ensure_prober();
        let verdict = self.gate_observed(Workload::Batch);
        if matches!(verdict, MeshRoute::UseMesh) {
            if let Some(lease) = self.try_lease(model) {
                return BatchLease {
                    endpoint: lease.endpoint.clone(),
                    pool: Some((Arc::clone(self), lease)),
                };
            }
        }
        BatchLease {
            endpoint: self.tuning.local_endpoint.clone(),
            pool: None,
        }
    }

    /// The full ladder for a non-streaming generation:
    /// **pool → local → Err(escalate: cheap cloud)**.
    ///
    /// - Interactive work (or a gate rejection) skips the pool rung entirely.
    /// - A pool-rung failure marks the peer unhealthy and falls through —
    ///   the caller is never poisoned by a dead peer.
    /// - A local-rung failure returns [`PoolError`] carrying the #717
    ///   escalation tier; the engine itself never talks to cloud providers.
    pub async fn generate_with(
        self: &Arc<Self>,
        req: &GenerateRequest,
    ) -> Result<GenerateResponse, PoolError> {
        assert_inference_only(&InferencePayload::Prompt);
        self.ensure_prober();

        // Pool rung — only when the four-condition gate admits this unit.
        if matches!(self.gate_observed(req.workload), MeshRoute::UseMesh) {
            if let Some(lease) = self.try_lease(Some(&req.model)) {
                match attempt_generate(
                    &self.client,
                    &lease.endpoint,
                    req,
                    self.tuning.request_timeout,
                )
                .await
                {
                    Ok(text) => {
                        return Ok(GenerateResponse {
                            text,
                            served_by: ServedBy::PoolPeer {
                                label: lease.label.clone(),
                            },
                        });
                    }
                    Err(err) => {
                        tracing::warn!(
                            peer = %lease.label,
                            error = %err,
                            "mesh pool peer failed mid-request; falling back to local"
                        );
                        self.mark_failed(lease.idx);
                        // lease (and its in-flight slot) drops here.
                    }
                }
            }
        }

        // Local rung — the safety net; same per-request budget.
        match attempt_generate(
            &self.client,
            &self.tuning.local_endpoint,
            req,
            self.tuning.request_timeout,
        )
        .await
        {
            Ok(text) => Ok(GenerateResponse {
                text,
                served_by: ServedBy::Local,
            }),
            Err(err) => Err(PoolError {
                message: err,
                escalate_to: escalation_for(req.workload),
            }),
        }
    }
}

// ── The wire attempt (single rung) ──────────────────────────────────────────

/// One `/api/generate` attempt against one endpoint, within one budget
/// (`req.timeout` overrides for legitimately-slow work like a cold warm-load).
/// Every failure mode — unreachable, non-2xx, death mid-body, over-budget —
/// comes back as a plain `Err(String)`; nothing here can abort the caller.
async fn attempt_generate(
    client: &reqwest::Client,
    endpoint: &str,
    req: &GenerateRequest,
    default_budget: Duration,
) -> Result<String, String> {
    crate::mesh::audit_and_check_mesh_egress(
        endpoint,
        req.session_id.as_deref(),
        &req.model,
        req.system.as_deref(),
        &req.prompt,
    )
    .await?;
    let budget = req.timeout.unwrap_or(default_budget);
    let body = InferenceBody::for_generate(req);
    let attempt = async {
        let resp = client
            .post(format!("{endpoint}/api/generate"))
            .timeout(budget)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Ollama unreachable: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("Ollama error: {}", resp.status()));
        }
        let v: Value = resp
            .json()
            .await
            .map_err(|e| format!("Ollama response unreadable: {e}"))?;
        Ok(v.get("response")
            .and_then(|r| r.as_str())
            .unwrap_or_default()
            .to_string())
    };
    // Belt-and-braces outer deadline over connect+headers+body.
    match tokio::time::timeout(budget + Duration::from_secs(1), attempt).await {
        Ok(result) => result,
        Err(_) => Err(format!("Ollama timed out after {}s", budget.as_secs())),
    }
}

/// One health probe: `GET /api/tags` (Ollama), falling back to
/// `GET /v1/models` (llama.cpp server / any OpenAI-compatible peer) when the
/// endpoint answers but does not speak the Ollama API. Returns the peer's
/// model list on success.
async fn probe_endpoint(
    client: &reqwest::Client,
    endpoint: &str,
    budget: Duration,
) -> Result<Vec<String>, String> {
    match client
        .get(format!("{endpoint}/api/tags"))
        .timeout(budget)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let v: Value = resp
                .json()
                .await
                .map_err(|e| format!("tags unreadable: {e}"))?;
            return Ok(v
                .get("models")
                .and_then(|m| m.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m.get("name").and_then(|n| n.as_str()))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default());
        }
        // Answered, but not the Ollama API — try the OpenAI-compatible shape.
        Ok(_) => {}
        // Connection-level failure: the peer is down; don't double the wait.
        Err(e) if e.is_connect() => return Err(format!("unreachable: {e}")),
        Err(_) => {}
    }
    let resp = client
        .get(format!("{endpoint}/v1/models"))
        .timeout(budget)
        .send()
        .await
        .map_err(|e| format!("unreachable: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("probe status {}", resp.status()));
    }
    let v: Value = resp
        .json()
        .await
        .map_err(|e| format!("models unreadable: {e}"))?;
    Ok(v.get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(|n| n.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default())
}

// ── Leases for callers that run their own HTTP (streaming) ─────────────────

/// A scheduled batch endpoint for a caller that executes the request itself
/// (the Librarian's streaming NDJSON path). Carries the fallback contract:
/// on failure against a pool peer, [`Self::fail_over_local`] marks the peer
/// unhealthy and hands back the local endpoint for one retry; with no engine
/// (or a local lease) there is no retry — legacy behavior, unchanged.
pub struct BatchLease {
    endpoint: String,
    pool: Option<(Arc<PoolEngine>, PeerLease)>,
}

impl BatchLease {
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// True when this lease points at a trusted pool peer (⇒ a local retry
    /// exists on failure).
    pub fn is_pool_peer(&self) -> bool {
        self.pool.is_some()
    }

    /// The request succeeded — release the slot (drop).
    pub fn succeed(self) {}

    /// The request failed. For a pool-peer lease: mark the peer unhealthy,
    /// release the slot, and return the local endpoint for one transparent
    /// retry. For a local/legacy lease: `None` — the caller surfaces its own
    /// error exactly as before this engine existed.
    pub fn fail_over_local(self) -> Option<String> {
        let (engine, lease) = self.pool?;
        engine.mark_failed(lease.idx);
        let local = engine.tuning.local_endpoint.clone();
        drop(lease);
        Some(local)
    }
}

// ── Global engine + free-function surface ───────────────────────────────────

static ENGINE: OnceLock<Option<Arc<PoolEngine>>> = OnceLock::new();

/// The process-wide engine, if `PERMAGENT_MESH_ENGINE` is on and at least one
/// peer is configured. Membership and the flag are read once per process; the
/// trust switch stays live (see the module doc).
pub fn engine() -> Option<Arc<PoolEngine>> {
    ENGINE
        .get_or_init(|| {
            if !engine_flag_enabled() {
                return None;
            }
            let peers = configured_peers();
            if peers.is_empty() {
                tracing::warn!(
                    "PERMAGENT_MESH_ENGINE is on but no pool peers are configured; \
                     batch inference stays local"
                );
                return None;
            }
            tracing::info!(
                peers = peers.len(),
                "mesh pool engine enabled ({})",
                peers
                    .iter()
                    .map(|p| p.label.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            Some(PoolEngine::with_config(peers, Tuning::default()))
        })
        .clone()
}

/// Whether the multi-peer engine is active this process.
pub fn engine_enabled() -> bool {
    engine().is_some()
}

/// Live [`MeshGateInputs`] for cost-router callers — the production inputs
/// provider #717 was written against. With the engine off, health is
/// `Unknown` (no prober runs), which the gate fails closed — the legacy
/// single-endpoint path through [`crate::mesh::resolve_route`] is unaffected.
pub fn live_gate_inputs(workload: Workload) -> MeshGateInputs {
    match engine() {
        Some(engine) => engine.gate_inputs(workload),
        None => MeshGateInputs {
            workload: to_gate_workload(workload),
            pool_configured: crate::mesh::configured_pool_endpoint().is_some(),
            trusted: trusted_pool_enabled(),
            health: PoolHealth::Unknown,
        },
    }
}

/// Schedule a batch endpoint for a caller that runs its own HTTP. Engine off
/// ⇒ exactly `resolve_route(Batch).endpoint` with no retry contract (legacy).
pub fn lease_batch(model: Option<&str>) -> BatchLease {
    match engine() {
        Some(engine) => engine.lease_for(model),
        None => BatchLease {
            endpoint: crate::mesh::resolve_route(Workload::Batch).endpoint,
            pool: None,
        },
    }
}

/// Run a non-streaming batch generation through the ladder. Engine off ⇒ a
/// single attempt against the #702-resolved endpoint with the same 60s
/// budget the Reader always used — behavior-identical to before this module.
pub async fn generate(req: GenerateRequest) -> Result<GenerateResponse, PoolError> {
    assert_inference_only(&InferencePayload::Prompt);
    match engine() {
        Some(engine) => engine.generate_with(&req).await,
        None => {
            let route = crate::mesh::resolve_route(req.workload);
            let client = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| PoolError {
                    message: e.to_string(),
                    escalate_to: None,
                })?;
            match attempt_generate(&client, &route.endpoint, &req, MESH_TIMEOUT).await {
                Ok(text) => Ok(GenerateResponse {
                    text,
                    served_by: match route.approach {
                        PrivacyApproach::TrustedPool => ServedBy::PoolPeer {
                            label: label_for_endpoint(&route.endpoint),
                        },
                        _ => ServedBy::Local,
                    },
                }),
                Err(message) => Err(PoolError {
                    message,
                    escalate_to: escalation_for(req.workload),
                }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_stream_body_disables_thinking_and_caps_output() {
        let body = InferenceBody::for_chat_stream("qwen3.8-27b", "P", "S", 150, 0.2);
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["stream"], true);
        assert_eq!(v["max_tokens"], 150);
        assert_eq!(v["chat_template_kwargs"]["enable_thinking"], false);
        assert_eq!(v["messages"][0]["role"], "system");
        assert_eq!(v["messages"][0]["content"], "S");
        assert_eq!(v["messages"][1]["role"], "user");
        assert_eq!(v["messages"][1]["content"], "P");
    }

    fn peer(endpoint: &str, trusted: bool) -> PeerConfig {
        PeerConfig::new(endpoint, label_for_endpoint(endpoint), trusted)
    }

    fn snap(health: PeerHealth, fresh: bool, models: &[&str], inflight: usize) -> PeerSnapshot {
        PeerSnapshot {
            health,
            fresh,
            models: models.iter().map(|s| s.to_string()).collect(),
            inflight,
        }
    }

    fn healthy(inflight: usize) -> PeerSnapshot {
        snap(PeerHealth::Healthy, true, &["qwen2.5:7b"], inflight)
    }

    // ── Config parsing (multi-peer) ─────────────────────────────────────────

    #[test]
    fn parses_comma_and_whitespace_separated_endpoint_strings() {
        let v = Value::String("http://a:11434, http://b:11434/  http://c:11434/".into());
        let peers = parse_pool_value(Some(&v));
        assert_eq!(
            peers
                .iter()
                .map(|p| p.endpoint.as_str())
                .collect::<Vec<_>>(),
            vec!["http://a:11434", "http://b:11434", "http://c:11434"],
            "commas and whitespace both separate; trailing slashes trimmed"
        );
        assert!(
            peers.iter().all(|p| p.trusted),
            "bare entries are gated by the live global trust switch"
        );
        assert_eq!(peers[0].label, "a:11434", "label defaults to host:port");
    }

    #[test]
    fn parses_structured_entries_with_labels_and_per_peer_trust() {
        let v = serde_json::json!([
            { "endpoint": "http://mini.local:11434/", "label": "mini", "trusted": true },
            { "endpoint": "http://spare.local:11434" },
            "http://bare.local:11434",
        ]);
        let peers = parse_pool_value(Some(&v));
        assert_eq!(peers.len(), 3);
        assert_eq!(peers[0].endpoint, "http://mini.local:11434");
        assert_eq!(peers[0].label, "mini");
        assert!(peers[0].trusted);
        assert!(
            !peers[1].trusted,
            "structured entries default to UNtrusted — consent is explicit per peer"
        );
        assert_eq!(peers[1].label, "spare.local:11434");
        assert!(
            peers[2].trusted,
            "bare string entries defer to the global switch"
        );
    }

    #[test]
    fn skips_malformed_entries_instead_of_failing() {
        let v = serde_json::json!([
            { "label": "no-endpoint" },
            { "endpoint": "ftp://wrong-scheme" },
            "not-a-url",
            42,
            "http://ok:11434",
        ]);
        let peers = parse_pool_value(Some(&v));
        assert_eq!(peers.len(), 1, "only the valid entry survives");
        assert_eq!(peers[0].endpoint, "http://ok:11434");
        assert!(parse_pool_value(None).is_empty());
        assert!(parse_pool_value(Some(&Value::Bool(true))).is_empty());
    }

    #[test]
    fn resolve_peers_dedups_and_falls_back_to_remote_ollama_host() {
        // First mention wins on duplicates.
        let peers = resolve_peers(
            vec![
                peer("http://a:11434", true),
                PeerConfig::new("http://a:11434", "dup", false),
                peer("http://b:11434", false),
            ],
            LOCAL_ENDPOINT,
        );
        assert_eq!(peers.len(), 2);
        assert!(peers[0].trusted, "first mention of a wins");

        // No explicit peers + remote #698 batch host ⇒ that host is the pool.
        let fallback = resolve_peers(vec![], "http://mini.local:11434/");
        assert_eq!(fallback.len(), 1);
        assert_eq!(fallback[0].endpoint, "http://mini.local:11434");
        assert_eq!(fallback[0].label, "ollama-host");
        assert!(fallback[0].trusted);

        // No explicit peers + local host ⇒ no pool at all.
        assert!(resolve_peers(vec![], LOCAL_ENDPOINT).is_empty());

        // Explicit peers suppress the ollama-host fallback.
        let explicit = resolve_peers(
            vec![peer("http://a:11434", true)],
            "http://mini.local:11434",
        );
        assert_eq!(explicit.len(), 1);
        assert_eq!(explicit[0].endpoint, "http://a:11434");
    }

    #[test]
    fn engine_flag_accepts_bool_number_and_truthy_strings() {
        assert!(!flag_from_value(None));
        assert!(flag_from_value(Some(&Value::Bool(true))));
        assert!(!flag_from_value(Some(&Value::Bool(false))));
        assert!(flag_from_value(Some(&serde_json::json!(1))));
        assert!(!flag_from_value(Some(&serde_json::json!(0))));
        for s in ["1", "true", "TRUE", " yes ", "On"] {
            assert!(
                flag_from_value(Some(&Value::String(s.into()))),
                "{s:?} should be truthy"
            );
        }
        assert!(!flag_from_value(Some(&Value::String("off".into()))));
    }

    // ── The scheduler core ──────────────────────────────────────────────────

    #[test]
    fn scheduler_skips_untrusted_peers_even_when_healthy_and_idle() {
        let peers = vec![peer("http://a:11434", false), peer("http://b:11434", true)];
        let snaps = vec![healthy(0), healthy(0)];
        let picked = select_idx(&peers, &snaps, true, None, 0, 1);
        assert_eq!(picked, Some(1), "the untrusted peer must never be selected");
    }

    #[test]
    fn scheduler_returns_none_when_global_trust_is_off() {
        let peers = vec![peer("http://a:11434", true)];
        let snaps = vec![healthy(0)];
        assert_eq!(
            select_idx(&peers, &snaps, false, None, 0, 1),
            None,
            "the live global switch is the master consent gate"
        );
    }

    #[test]
    fn scheduler_skips_unhealthy_unknown_and_stale_peers() {
        let peers = vec![
            peer("http://a:11434", true),
            peer("http://b:11434", true),
            peer("http://c:11434", true),
            peer("http://d:11434", true),
        ];
        let snaps = vec![
            snap(PeerHealth::Unhealthy, true, &[], 0),
            snap(PeerHealth::Unknown, true, &[], 0),
            snap(PeerHealth::Healthy, false, &["qwen2.5:7b"], 0), // stale probe
            healthy(0),
        ];
        assert_eq!(select_idx(&peers, &snaps, true, None, 0, 1), Some(3));
    }

    #[test]
    fn scheduler_routes_around_busy_peers() {
        let peers = vec![peer("http://a:11434", true), peer("http://b:11434", true)];
        // a is at the in-flight cap; b is free.
        let snaps = vec![healthy(1), healthy(0)];
        assert_eq!(select_idx(&peers, &snaps, true, None, 0, 1), Some(1));
        // Everyone at cap ⇒ nobody — the pool contributes nothing when busy.
        let busy = vec![healthy(1), healthy(1)];
        assert_eq!(select_idx(&peers, &busy, true, None, 0, 1), None);
    }

    #[test]
    fn scheduler_filters_on_requested_model() {
        let peers = vec![peer("http://a:11434", true), peer("http://b:11434", true)];
        let snaps = vec![
            snap(PeerHealth::Healthy, true, &["llama3:8b"], 0),
            snap(PeerHealth::Healthy, true, &["qwen2.5:7b"], 0),
        ];
        assert_eq!(
            select_idx(&peers, &snaps, true, Some("qwen2.5:7b"), 0, 1),
            Some(1)
        );
        assert_eq!(
            select_idx(&peers, &snaps, true, Some("absent:1b"), 0, 1),
            None
        );
    }

    #[test]
    fn scheduler_prefers_least_loaded_then_rotates_ties() {
        let peers = vec![
            peer("http://a:11434", true),
            peer("http://b:11434", true),
            peer("http://c:11434", true),
        ];
        // b is more loaded; a and c tie at 0 with a cap of 2.
        let snaps = vec![healthy(0), healthy(1), healthy(0)];
        assert_eq!(select_idx(&peers, &snaps, true, None, 0, 2), Some(0));
        assert_eq!(
            select_idx(&peers, &snaps, true, None, 1, 2),
            Some(2),
            "round-robin rotates among the least-loaded tie"
        );
        assert_eq!(select_idx(&peers, &snaps, true, None, 2, 2), Some(0));
    }

    #[test]
    fn model_shorthand_matches_latest_tag() {
        let models = vec!["qwen2.5:7b".to_string(), "llama3:latest".to_string()];
        assert!(model_served(&models, "qwen2.5:7b"));
        assert!(
            model_served(&models, "llama3"),
            "untagged name means :latest"
        );
        assert!(
            !model_served(&models, "qwen2.5"),
            "no :latest for qwen2.5 here"
        );
        assert!(!model_served(&models, "mistral:7b"));
        assert!(model_served(&models, ""), "no requirement ⇒ served");
    }

    // ── Gate inputs + aggregate health ──────────────────────────────────────

    #[test]
    fn aggregate_health_reflects_trusted_peers_only() {
        let peers = vec![peer("http://a:11434", false), peer("http://b:11434", true)];
        // Untrusted a is healthy, trusted b unprobed ⇒ Unknown, not Healthy.
        let snaps = vec![healthy(0), snap(PeerHealth::Unknown, false, &[], 0)];
        assert_eq!(aggregate_health(&peers, &snaps), PoolHealth::Unknown);
        // Trusted b probed unhealthy ⇒ Unhealthy.
        let snaps = vec![healthy(0), snap(PeerHealth::Unhealthy, true, &[], 0)];
        assert_eq!(aggregate_health(&peers, &snaps), PoolHealth::Unhealthy);
        // Trusted b healthy ⇒ Healthy.
        let snaps = vec![snap(PeerHealth::Unhealthy, true, &[], 0), healthy(0)];
        assert_eq!(aggregate_health(&peers, &snaps), PoolHealth::Healthy);
    }

    #[test]
    fn engine_gate_inputs_fail_closed_before_any_probe() {
        let engine = PoolEngine::with_config(
            vec![peer("http://mini.local:11434", true)],
            Tuning {
                assume_trusted: Some(true),
                ..Tuning::default()
            },
        );
        let inputs = engine.gate_inputs(Workload::Batch);
        assert!(inputs.pool_configured);
        assert!(inputs.trusted);
        assert_eq!(
            inputs.health,
            PoolHealth::Unknown,
            "never probed ⇒ Unknown ⇒ the #717 gate fails closed"
        );
        assert!(matches!(
            gate(inputs),
            MeshRoute::Ineligible(crate::cost_router::mesh::MeshIneligible::Unhealthy)
        ));
    }

    #[test]
    fn interactive_gate_inputs_are_rejected_whatever_the_pool_state() {
        let engine = PoolEngine::with_config(
            vec![peer("http://mini.local:11434", true)],
            Tuning {
                assume_trusted: Some(true),
                ..Tuning::default()
            },
        );
        assert!(matches!(
            gate(engine.gate_inputs(Workload::Interactive)),
            MeshRoute::Ineligible(crate::cost_router::mesh::MeshIneligible::Interactive)
        ));
    }

    // ── The ladder handoff + the wire-boundary invariant ────────────────────

    #[test]
    fn ladder_exhaustion_escalates_batch_to_cheap_cloud_only() {
        assert_eq!(
            escalation_for(Workload::Batch),
            Some(Tier::CheapCloud),
            "matches cost_router::mesh::fallback_tier()"
        );
        assert_eq!(
            escalation_for(Workload::Batch),
            Some(crate::cost_router::mesh::fallback_tier())
        );
        assert_eq!(
            escalation_for(Workload::Interactive),
            None,
            "interactive never entered the mesh ladder"
        );
    }

    #[test]
    fn wire_body_carries_exactly_inference_computation_and_nothing_else() {
        let req = GenerateRequest {
            session_id: None,
            model: "qwen2.5:7b".into(),
            prompt: "p".into(),
            system: Some("s".into()),
            options: Some(serde_json::json!({ "temperature": 0.2 })),
            keep_alive: Some("1800s".into()),
            timeout: None,
            workload: Workload::Batch,
        };
        let body = InferenceBody::for_generate(&req).0;
        let obj = body.as_object().expect("body is an object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "keep_alive",
                "model",
                "options",
                "prompt",
                "stream",
                "system"
            ],
            "the HARD INVARIANT at the wire: only inference computation \
             (+ the keep_alive residency knob) is representable"
        );
        assert_eq!(obj["stream"], Value::Bool(false));

        // Optional fields drop out rather than serializing as null.
        let bare = GenerateRequest {
            session_id: None,
            model: "m".into(),
            prompt: "p".into(),
            system: None,
            options: None,
            keep_alive: None,
            timeout: None,
            workload: Workload::Batch,
        };
        let obj = InferenceBody::for_generate(&bare).0;
        let mut keys: Vec<&str> = obj
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["model", "prompt", "stream"]);
    }

    #[test]
    fn streaming_wire_body_is_pinned_to_the_same_choke_point() {
        let body =
            InferenceBody::for_stream("m", "p", "s", serde_json::json!({ "num_predict": 150 })).0;
        let obj = body.as_object().expect("body is an object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec!["model", "options", "prompt", "stream", "system"],
            "the Librarian's streaming path builds through the same inference-only constructor"
        );
        assert_eq!(obj["stream"], Value::Bool(true));
    }

    #[test]
    fn pool_error_displays_its_message() {
        let e = PoolError {
            message: "Ollama unreachable: refused".into(),
            escalate_to: Some(Tier::CheapCloud),
        };
        assert_eq!(e.to_string(), "Ollama unreachable: refused");
    }

    // ── Lease bookkeeping ───────────────────────────────────────────────────

    #[test]
    fn lease_holds_and_releases_inflight_capacity() {
        let engine = PoolEngine::with_config(
            vec![peer("http://a:11434", true)],
            Tuning {
                assume_trusted: Some(true),
                ..Tuning::default()
            },
        );
        // Make the peer selectable without a network probe.
        engine.record_probe(0, Ok(vec!["qwen2.5:7b".to_string()]));

        let lease = engine.try_lease(None).expect("peer is eligible");
        assert_eq!(engine.peer_statuses()[0].inflight, 1);
        assert!(
            engine.try_lease(None).is_none(),
            "at the cap of 1, the busy peer is routed around"
        );
        drop(lease);
        assert_eq!(
            engine.peer_statuses()[0].inflight,
            0,
            "guard released the slot"
        );
        assert!(engine.try_lease(None).is_some());
    }

    #[test]
    fn batch_lease_fail_over_marks_peer_unhealthy_and_returns_local() {
        let engine = PoolEngine::with_config(
            vec![peer("http://a:11434", true)],
            Tuning {
                assume_trusted: Some(true),
                local_endpoint: "http://127.0.0.1:9".into(),
                ..Tuning::default()
            },
        );
        engine.record_probe(0, Ok(vec![]));

        let lease = engine.lease_for(None);
        assert!(lease.is_pool_peer());
        assert_eq!(lease.endpoint(), "http://a:11434");
        let local = lease.fail_over_local();
        assert_eq!(local.as_deref(), Some("http://127.0.0.1:9"));
        let status = &engine.peer_statuses()[0];
        assert!(
            !status.healthy,
            "failure marked the peer unhealthy immediately"
        );
        assert_eq!(status.inflight, 0, "the slot was released");

        // With the peer now unhealthy, the next lease is local and carries no retry.
        let lease = engine.lease_for(None);
        assert!(!lease.is_pool_peer());
        assert_eq!(lease.endpoint(), "http://127.0.0.1:9");
        assert!(
            lease.fail_over_local().is_none(),
            "local lease has no further rung here"
        );
    }

    #[test]
    fn untrusted_or_globally_off_lease_stays_local() {
        // Per-peer untrusted.
        let engine = PoolEngine::with_config(
            vec![peer("http://a:11434", false)],
            Tuning {
                assume_trusted: Some(true),
                local_endpoint: "http://127.0.0.1:9".into(),
                ..Tuning::default()
            },
        );
        engine.record_probe(0, Ok(vec![]));
        assert!(!engine.lease_for(None).is_pool_peer());

        // Globally off, peer trusted.
        let engine = PoolEngine::with_config(
            vec![peer("http://a:11434", true)],
            Tuning {
                assume_trusted: Some(false),
                local_endpoint: "http://127.0.0.1:9".into(),
                ..Tuning::default()
            },
        );
        engine.record_probe(0, Ok(vec![]));
        assert!(!engine.lease_for(None).is_pool_peer());
    }
}
