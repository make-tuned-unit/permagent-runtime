//! Mesh inference-routing — a light layer that decides *where* an inference
//! runs, so callers never have to.
//!
//! This is the concrete, workable-today seed of the Mesh distributed-inference
//! vision (`docs/vision/MESH_DISTRIBUTED_INFERENCE_VISION.md`, epic #306). It
//! deliberately does the smallest useful thing now — route heavy, latency-
//! tolerant **batch** work to a configured, trusted pool machine — while giving
//! future phases a set of extension points they can grow into **without
//! changing a single caller**. It stacks on #698, which made the batch Ollama
//! endpoint configurable (`config::ollama_host()`); this module turns that raw
//! knob into a centralized, trust-gated routing decision.
//!
//! ## The HARD INVARIANT
//!
//! **Only inference computation ever travels off this device — prompts,
//! activations, and model weights — and only to a peer the user has explicitly
//! marked trusted. The user's Brain, documents, and memories never leave as a
//! federated store.** This is enforced three ways: (1) [`InferencePayload`]
//! cannot even *represent* a Brain store, so a caller can only hand the router
//! inference computation, by construction; (2) [`assert_inference_only`] is the
//! runtime seam a future wire-transport enforces through; (3) content-bearing
//! batch work only routes to a pool when [`trusted_pool_enabled`] is true
//! (default off) — see [`resolve_route`]. Interactive work never routes off the
//! device at all, yet.
//!
//! ## Growth seams — what each extension point grows into (per vision phase)
//!
//! - [`resolve_route`] is the *Inference Routing Interface* (vision Phase 3 —
//!   "the most important thing to build early"). Today it is a two-arm table;
//!   future phases extend the seam (TEE/FHE selection, Chitin peer resolution,
//!   role-aware shard assignment) behind the same signature.
//! - [`PrivacyApproach`] is `#[non_exhaustive]`: `TrustedPool` is today's
//!   practical-trust offload (vision Phase 4, Approach A). `TeeAttested`
//!   (Phase 5, Approach B) and `FullyEncrypted` (Phase 6, Approach C) are
//!   documented on the enum and slot in non-breakingly as the hardware matures.
//! - [`is_trusted_peer`] / [`DeviceRole`] are the Chitin-trust seam (vision
//!   Layer 1). Today an env allowlist / env role; tomorrow SBT-verified
//!   membership and reputation on Base L2, and hardware-derived roles.

pub mod pool;

/// Whether an inference endpoint is provably on this machine. Anything that
/// cannot be parsed as HTTP(S) with a loopback host is treated as remote.
pub(crate) fn endpoint_is_loopback(endpoint: &str) -> bool {
    let Ok(url) = url::Url::parse(endpoint) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    match url.host_str() {
        Some(host) if host.eq_ignore_ascii_case("localhost") => true,
        Some(host) => {
            // `Url::host_str` keeps the brackets on an IPv6 literal (e.g.
            // "[::1]"), which `IpAddr::parse` rejects. Strip a single matched
            // pair before parsing so loopback IPv6 hosts classify correctly.
            let host = host
                .strip_prefix('[')
                .and_then(|h| h.strip_suffix(']'))
                .unwrap_or(host);
            host.parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
        }
        None => false,
    }
}

/// Audit and authorize a mesh inference endpoint before any request bytes are
/// sent. Unlike the configurable provider policy, mesh audit is always
/// fail-closed: an unlogged remote mesh send is never allowed.
pub(crate) async fn audit_and_check_mesh_egress(
    endpoint: &str,
    session_id: Option<&str>,
    model: &str,
    system: Option<&str>,
    prompt: &str,
) -> Result<(), String> {
    if endpoint_is_loopback(endpoint) {
        return Ok(());
    }

    let session_id = session_id.unwrap_or_default();
    let blocked = if session_id.is_empty() {
        crate::sovereignty::global_sovereign_mode()
    } else {
        crate::sovereignty::is_context_sovereign(session_id)
    };
    let rendered = format!("system:\n{}\n\nuser:\n{prompt}", system.unwrap_or_default());
    let audit = crate::sovereignty::record_egress(crate::sovereignty::EgressRecord {
        provider: "mesh".to_string(),
        model: model.to_string(),
        session_id: session_id.to_string(),
        project_id: None,
        kind: crate::sovereignty::EgressKind::Inference,
        blocked,
        content_hash: crate::sovereignty::hash_texts(std::slice::from_ref(&rendered)),
        prompt: crate::sovereignty::capture_prompts_enabled().then_some(rendered),
    })
    .await;

    if blocked {
        return Err(format!(
            "{} mesh inference to non-loopback endpoint '{endpoint}' blocked: this context is sovereign",
            crate::sovereignty::SOVEREIGN_BLOCK_PREFIX
        ));
    }
    if let Err(error) = audit {
        return Err(format!(
            "{} mesh inference to '{endpoint}' refused: egress audit write failed: {error:#}",
            crate::sovereignty::SOVEREIGN_BLOCK_PREFIX
        ));
    }
    Ok(())
}

/// This device's role in a compute pool. Determines what it *contributes*, not
/// what it can *access* (the vision's core inversion). Default [`Self::Client`].
///
/// - `Server` holds the heaviest model layers (e.g. an M4 Max) — hosts.
/// - `Contributor` adds GPU/RAM to the pool (e.g. an M1) — helps host.
/// - `Client` cannot meaningfully contribute (e.g. an Intel Mac) — offloads.
///
/// Growth seam: today the role is read from an env var; a future phase derives
/// it from measured hardware capacity and uses it for shard assignment
/// (vision Phase 4). Role does not gate today's simple batch routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeviceRole {
    /// Holds the heaviest layers; hosts models for the pool.
    Server,
    /// Adds compute/memory to the pool; helps host.
    Contributor,
    /// Cannot contribute; offloads heavy work to the pool.
    #[default]
    Client,
}

/// This device's [`DeviceRole`], from `PERMAGENT_MESH_ROLE`
/// (`server` | `contributor` | `client`). Defaults to [`DeviceRole::Client`].
pub fn device_role() -> DeviceRole {
    parse_device_role(std::env::var("PERMAGENT_MESH_ROLE").ok().as_deref())
}

/// Pure core of [`device_role`] — unit-testable without touching process env.
fn parse_device_role(raw: Option<&str>) -> DeviceRole {
    match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("server") => DeviceRole::Server,
        Some("contributor") => DeviceRole::Contributor,
        _ => DeviceRole::Client,
    }
}

/// How the privacy of an off-device inference is protected. `#[non_exhaustive]`
/// on purpose: the roadmap adds stronger approaches as the tech matures, and
/// adding a variant must never be a breaking change for callers that already
/// `match` on this (they keep a `_ =>` arm).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyApproach {
    /// Runs on this device only — nothing leaves the machine.
    LocalOnly,
    /// Runs on a user-configured, explicitly-trusted pool machine. Practical
    /// trust within a Chitin circle (vision Phase 4, Approach A): the operator
    /// vouches for the endpoint. Not cryptographically proven.
    TrustedPool,
    // future: TeeAttested   — Approach B, Apple Neural Engine attestation (vision Phase 5).
    // future: FullyEncrypted — Approach C, FHE, mathematically private (vision Phase 6).
}

/// The kind of work being routed. Batch is latency-tolerant (Librarian/Reader
/// passes, overnight goal work); Interactive is user-facing chat where latency
/// is on the critical path — and which, per the vision, never leaves the device
/// yet (WAN round-trips per layer-group blow the interactive latency budget).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workload {
    /// Latency-tolerant background work — eligible to offload.
    Batch,
    /// User-facing, latency-sensitive work — always local, for now.
    Interactive,
}

/// A resolved routing decision: where the inference runs and how its privacy is
/// protected. Callers use `.endpoint`; the `approach` is the seam future phases
/// key their transport off (e.g. wrap `TeeAttested` in attestation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferenceRoute {
    /// The endpoint the inference is sent to (an Ollama base URL today).
    pub endpoint: String,
    /// How this route's privacy is protected.
    pub approach: PrivacyApproach,
}

impl InferenceRoute {
    fn new(endpoint: impl Into<String>, approach: PrivacyApproach) -> Self {
        Self {
            endpoint: endpoint.into(),
            approach,
        }
    }

    /// The always-on-device route: this machine's local model endpoint. Used
    /// for interactive work and for any batch work not routed to a trusted pool.
    fn local() -> Self {
        Self::new(LOCAL_ENDPOINT, PrivacyApproach::LocalOnly)
    }
}

/// The classes of data the mesh layer will ever put on the wire. The user's
/// Brain, documents, and memories are deliberately **not** representable here —
/// the HARD INVARIANT holds by construction, because a caller cannot hand the
/// router anything but inference computation. `#[non_exhaustive]` so future
/// transports can add payload kinds (e.g. encrypted shards) non-breakingly.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InferencePayload {
    /// A prompt to be completed (encrypted or split before travel in later phases).
    Prompt,
    /// Intermediate layer activations (vision Phase 4 hybrid split).
    Activations,
    /// Model weights / shards — shared, non-sensitive.
    Weights,
}

/// Runtime guard for the HARD INVARIANT: only inference computation may route to
/// a peer, never the user's Brain/documents/memories. [`InferencePayload`] makes
/// a Brain store unrepresentable at the type level, and since the pool engine
/// landed this seam is **invoked in production**: every wire body leaves through
/// [`pool::InferenceBody`]'s constructors, each of which calls this guard. It
/// remains the single, stable call site a future wire-transport (TEE/FHE) adds a
/// real assertion / audit-log to, without callers changing.
pub fn assert_inference_only(_payload: &InferencePayload) {
    // Intentionally a no-op body: the invariant holds by construction (see the
    // `InferencePayload` and `pool::InferenceBody` docs). This is the
    // enforcement seam every outbound body is built through.
}

/// This machine's local Ollama endpoint. Mirrors the default that
/// [`crate::config::ollama_host`] returns when `PERMAGENT_OLLAMA_HOST` is unset;
/// a batch endpoint that differs from this is treated as a (candidate) pool.
const LOCAL_ENDPOINT: &str = "http://localhost:11434";

/// Resolve where a [`Workload`] should run. **This is the seam** — the whole
/// point of the layer. Callers ask "where does batch inference go?" and never
/// learn about pools, trust, or (later) TEE/FHE.
///
/// Today's routing table:
/// - `Interactive` → always local ([`PrivacyApproach::LocalOnly`]). Interactive
///   work never leaves the device yet.
/// - `Batch` → with the multi-peer engine on (`PERMAGENT_MESH_ENGINE`, default
///   off), the healthiest eligible trusted peer per [`pool`]'s scheduler
///   (health-gated, capacity-gated); otherwise the #702 single-endpoint table:
///   a [`PrivacyApproach::TrustedPool`] route **iff** a pool endpoint is
///   configured *and* [`trusted_pool_enabled`] is true; otherwise local.
///   This is the HARD INVARIANT in code: content-bearing batch work only leaves
///   the machine when the user has explicitly marked a pool trusted.
pub fn resolve_route(workload: Workload) -> InferenceRoute {
    if let Some(engine) = pool::engine() {
        return engine.resolve_route_sync(workload);
    }
    let trusted = trusted_pool_enabled();
    let pool_endpoint = configured_pool_endpoint();
    if workload == Workload::Batch && !trusted {
        if let Some(endpoint) = &pool_endpoint {
            warn_pool_untrusted_once(endpoint);
        }
    }
    resolve_route_inner(workload, trusted, pool_endpoint)
}

/// Once-per-boot observability for the silent-revocation case: a pool is
/// configured but not trusted, so batch work is (correctly, fail-closed)
/// staying local — which a #698-era `PERMAGENT_OLLAMA_HOST` deployment that
/// predates the trust gate will not expect. Warn loudly, name the fix.
pub(crate) fn warn_pool_untrusted_once(detail: &str) {
    static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    tracing::warn!(
        pool = %detail,
        "A batch inference pool is configured but NOT marked trusted, so batch work stays on \
         this device (fail-closed). If you trust that machine, set PERMAGENT_MESH_TRUSTED=1 \
         (and `trusted: true` on config-file peers) to use it."
    );
}

/// Pure core of [`resolve_route`] — the routing table, unit-testable without
/// touching process env (env-mutating tests flake under parallel `cargo test`).
fn resolve_route_inner(workload: Workload, trusted: bool, pool: Option<String>) -> InferenceRoute {
    match workload {
        // Never route interactive off-device yet (vision open-question #1:
        // WAN latency breaks the interactive budget). Unlocks per phase.
        Workload::Interactive => InferenceRoute::local(),
        Workload::Batch => match (trusted, pool) {
            (true, Some(endpoint)) => InferenceRoute::new(endpoint, PrivacyApproach::TrustedPool),
            // No trusted pool ⇒ stay local. This is where #698's configured
            // batch host now requires an explicit trust opt-in to be used for
            // content-bearing work — the invariant guard doing its job.
            _ => InferenceRoute::local(),
        },
    }
}

/// The pool endpoint to offload batch work to, if one is configured. An explicit
/// `PERMAGENT_MESH_POOL` wins; otherwise the #698 batch host
/// (`config::ollama_host()`) counts as a pool when it is not the local default.
fn configured_pool_endpoint() -> Option<String> {
    resolve_pool_endpoint(
        std::env::var("PERMAGENT_MESH_POOL").ok(),
        crate::config::ollama_host(),
    )
}

/// Pure core of [`configured_pool_endpoint`] — unit-testable without env.
fn resolve_pool_endpoint(mesh_pool: Option<String>, ollama_host: String) -> Option<String> {
    if let Some(pool) = mesh_pool
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
    {
        return Some(pool);
    }
    (ollama_host != LOCAL_ENDPOINT).then_some(ollama_host)
}

/// Whether the user has explicitly marked a pool trusted, gating content-bearing
/// batch offload. Env `PERMAGENT_MESH_TRUSTED`, **default false**. Truthy:
/// `1` | `true` | `yes` | `on` (case-insensitive).
pub fn trusted_pool_enabled() -> bool {
    env_flag_enabled(std::env::var("PERMAGENT_MESH_TRUSTED").ok().as_deref())
}

/// Pure core of [`trusted_pool_enabled`] — unit-testable without env.
fn env_flag_enabled(raw: Option<&str>) -> bool {
    matches!(
        raw.map(|s| s.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

/// Whether a peer is trusted enough to route inference to. **Chitin-trust
/// extension point.** Today it reads a static env allowlist
/// (`PERMAGENT_MESH_TRUSTED_PEERS`, comma/whitespace-separated).
///
// future: verify Chitin ID SBT ownership + reputation on Base L2 (see
// ui/command-center/src/services/identity.ts and .../config/chain.ts) instead
// of an env allowlist — the trust check has a home here that grows.
pub fn is_trusted_peer(peer_id: &str) -> bool {
    peer_in_allowlist(
        peer_id,
        std::env::var("PERMAGENT_MESH_TRUSTED_PEERS")
            .ok()
            .as_deref(),
    )
}

/// Pure core of [`is_trusted_peer`] — unit-testable without env.
fn peer_in_allowlist(peer_id: &str, allowlist: Option<&str>) -> bool {
    let peer_id = peer_id.trim();
    if peer_id.is_empty() {
        return false;
    }
    allowlist
        .into_iter()
        .flat_map(|list| list.split(|c: char| c.is_whitespace() || c == ','))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .any(|p| p == peer_id)
}

/// Self-knowledge descriptor for device pooling. Co-located here; aggregated by
/// `crate::agents::self_knowledge`. Static — the routing decision is always-on
/// and we do not claim a live pool status we cannot cheaply observe. The
/// teaching steps let Henry *onboard* the user to pooling their devices.
pub const MESH_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "mesh",
        display_name: "Device Pooling",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "The routing seam that decides where the agent's heavy, latency-tolerant batch work — \
             the Librarian's memory descriptions and the Reader's document summaries — actually \
             runs: on this device by default, or on a larger machine when a trusted pool has been \
             configured for it. Interactive chat always stays on this machine; only batch \
             inference is ever eligible to run elsewhere, and what travels is a prompt plus \
             sampling options — never a Brain, a document store, or a memory database as such. \
             But the prompt itself carries the text being summarised: the memory's key and \
             content, or up to 6000 characters of the document. So a configured pool does see \
             that content",
        why_it_matters:
            "It is the safety model and forward seam for pooling compute across a user's own \
             trusted machines — a stronger device can host a larger model while a lighter one \
             offloads its heavy passes. The wire carries inference requests only — no store is \
             ever synced or handed over — but that is not the same as privacy: the memory or \
             document text being summarised rides inside the prompt, so tell the user plainly \
             that a machine they pool to reads what it summarises for them. It stays local by \
             default and activates only when a trusted pool is configured — there is no in-app \
             pooling to switch on today — and it is the seam that later grows into split, \
             hardware-attested, and fully-encrypted inference",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        // Describe-only: a pool is configured via env today (no in-app control),
        // so this surface carries no onboarding lesson — the agent explains what
        // device pooling is and its inference-only safety model; it does not walk
        // the user through turning it on.
        teaching: &[],
    };

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_loopback_classification_is_fail_closed() {
        for endpoint in [
            "http://localhost:11434",
            "http://LOCALHOST:11434",
            "http://127.0.0.1:11434",
            "http://127.1.2.3:11434",
            "http://[::1]:11434",
        ] {
            assert!(endpoint_is_loopback(endpoint), "{endpoint}");
        }
        for endpoint in [
            "http://192.168.1.50:11434",
            "https://ollama.example.com",
            "http://localhost.example.com",
            "not a url",
        ] {
            assert!(!endpoint_is_loopback(endpoint), "{endpoint}");
        }
    }

    #[test]
    fn interactive_is_always_local_regardless_of_trust_or_pool() {
        for trusted in [false, true] {
            for pool in [None, Some("http://pool.local:11434".to_string())] {
                let route = resolve_route_inner(Workload::Interactive, trusted, pool);
                assert_eq!(
                    route.approach,
                    PrivacyApproach::LocalOnly,
                    "interactive must never leave the device"
                );
                assert_eq!(route.endpoint, LOCAL_ENDPOINT);
            }
        }
    }

    #[test]
    fn batch_is_local_unless_pool_and_trusted() {
        let pool = || Some("http://mini.local:11434".to_string());

        // No pool, no trust → local.
        assert_eq!(
            resolve_route_inner(Workload::Batch, false, None).approach,
            PrivacyApproach::LocalOnly
        );
        // Pool configured but not trusted → local (the invariant guard).
        assert_eq!(
            resolve_route_inner(Workload::Batch, false, pool()).approach,
            PrivacyApproach::LocalOnly
        );
        // Trusted but no pool configured → local.
        assert_eq!(
            resolve_route_inner(Workload::Batch, true, None).approach,
            PrivacyApproach::LocalOnly
        );
        // Pool + trusted → the trusted pool.
        let route = resolve_route_inner(Workload::Batch, true, pool());
        assert_eq!(route.approach, PrivacyApproach::TrustedPool);
        assert_eq!(route.endpoint, "http://mini.local:11434");
    }

    #[test]
    fn pool_endpoint_prefers_explicit_mesh_pool_then_remote_ollama_host() {
        // Explicit mesh pool wins and is trimmed.
        assert_eq!(
            resolve_pool_endpoint(
                Some("http://pool.local:11434/".to_string()),
                "http://mini.local:11434".to_string()
            ),
            Some("http://pool.local:11434".to_string())
        );
        // No mesh pool, but a remote #698 batch host → that host is the pool.
        assert_eq!(
            resolve_pool_endpoint(None, "http://mini.local:11434".to_string()),
            Some("http://mini.local:11434".to_string())
        );
        // No mesh pool, batch host is the local default → no pool.
        assert_eq!(
            resolve_pool_endpoint(None, LOCAL_ENDPOINT.to_string()),
            None
        );
        // Blank mesh pool falls through to the (local) ollama host → no pool.
        assert_eq!(
            resolve_pool_endpoint(Some("   ".to_string()), LOCAL_ENDPOINT.to_string()),
            None
        );
    }

    #[test]
    fn trusted_flag_defaults_off_and_reads_truthy_values() {
        assert!(!env_flag_enabled(None));
        assert!(!env_flag_enabled(Some("")));
        assert!(!env_flag_enabled(Some("0")));
        assert!(!env_flag_enabled(Some("false")));
        for on in ["1", "true", "TRUE", " yes ", "On"] {
            assert!(env_flag_enabled(Some(on)), "{on:?} should be truthy");
        }
    }

    #[test]
    fn peer_allowlist_matches_only_listed_ids() {
        assert!(!peer_in_allowlist("alice", None));
        assert!(!peer_in_allowlist("alice", Some("")));
        assert!(peer_in_allowlist("alice", Some("alice,bob")));
        assert!(peer_in_allowlist("bob", Some("alice, bob  carol")));
        assert!(!peer_in_allowlist("mallory", Some("alice,bob")));
        // An empty query never matches, even against a non-empty list.
        assert!(!peer_in_allowlist("   ", Some("alice,bob")));
    }

    #[test]
    fn device_role_parses_and_defaults_to_client() {
        assert_eq!(parse_device_role(None), DeviceRole::Client);
        assert_eq!(parse_device_role(Some("client")), DeviceRole::Client);
        assert_eq!(parse_device_role(Some(" Server ")), DeviceRole::Server);
        assert_eq!(
            parse_device_role(Some("CONTRIBUTOR")),
            DeviceRole::Contributor
        );
        assert_eq!(parse_device_role(Some("garbage")), DeviceRole::Client);
        assert_eq!(DeviceRole::default(), DeviceRole::Client);
    }

    #[test]
    fn inference_only_guard_accepts_computation_payloads() {
        // The type system makes a Brain store unrepresentable; the guard is the
        // stable seam a future transport enforces through.
        for payload in [
            InferencePayload::Prompt,
            InferencePayload::Activations,
            InferencePayload::Weights,
        ] {
            assert_inference_only(&payload);
        }
    }
}
