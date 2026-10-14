//! Standalone-watch voice gateway (Astra ruling 2026-10-13, decisions 1, 3, 5).
//!
//! Lets an Apple Watch talk to the hub's existing `/voice` websocket without
//! the iPhone relay, starting on the home LAN. Four small pieces:
//!
//! 1. **Scope.** A `voice_only` device credential (see
//!    [`crate::device_registry::DeviceScope`]) is minted by a paired phone for
//!    its own watch. [`DeviceRegistry::verify`](crate::device_registry::DeviceRegistry::verify)
//!    never accepts it, so every bearer route, SSE stream and `/events` rejects
//!    it; only `/voice` consults [`admit_voice_only`]. [`route_allowed_for_scope`]
//!    states that rule as a testable predicate.
//! 2. **Conversation binding.** The credential may open `/voice` only with an
//!    explicit `session_id` equal to the conversation it is bound to
//!    ([`authorize_session`]); an unbound credential claims the first id it uses.
//! 3. **Tool restriction.** The admitted socket's [`VoiceOrigin`] is marked
//!    `voice_only`; the agent then lists/dispatches only
//!    `permagent::events::voice_origin::voice_only_tool_allowed` tools.
//! 4. **LAN listener + limits.** An opt-in second listener (config
//!    `daemon.voice_gateway.enabled`, default OFF) serves ONLY `/voice` and
//!    `/voice/health` over TLS with a self-signed certificate kept under the
//!    profile dir; its SHA-256 fingerprint rides the watch-credential response
//!    so the watch can pin it. [`Limiter`] bounds connections, turns, audio.
//!
//! Credentials are never logged: only device ids and counters appear.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::{
    extract::{ConnectInfo, Request},
    http::StatusCode,
    middleware::Next,
    response::Response,
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::device_registry::{DeviceRegistry, DeviceScope, VoiceCredential};
use crate::state::AppState;

// ── Settings ────────────────────────────────────────────────────────────────

/// `daemon.voice_gateway` in config.yaml. Every key optional.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct GatewaySettings {
    /// Opt-in. Default OFF: the hub stays loopback-only.
    pub enabled: bool,
    /// Interface to bind (LAN). `0.0.0.0` = all IPv4 interfaces.
    pub bind: String,
    pub port: u16,
    /// Host or IP the watch should dial. Unset = the primary LAN IPv4.
    pub advertise_host: Option<String>,
    /// Lifetime of a minted watch credential.
    pub credential_ttl_days: i64,
    /// Concurrent sockets per watch credential.
    pub max_connections_per_device: usize,
    /// Requests (including failed-auth attempts) per source IP per minute.
    pub max_attempts_per_ip_per_minute: usize,
    pub max_turns_per_minute: usize,
    pub max_audio_bytes_per_minute: u64,
    /// Hard cap on one capture, applied on top of the hub's own capture limit.
    pub max_turn_secs: u64,
}

impl Default for GatewaySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: "0.0.0.0".to_string(),
            port: 3443,
            advertise_host: None,
            credential_ttl_days: crate::device_registry::VOICE_CREDENTIAL_TTL_DAYS,
            max_connections_per_device: 2,
            max_attempts_per_ip_per_minute: 30,
            max_turns_per_minute: 12,
            max_audio_bytes_per_minute: 8 * 1024 * 1024,
            max_turn_secs: 45,
        }
    }
}

static SETTINGS: OnceLock<GatewaySettings> = OnceLock::new();
static LIMITER: OnceLock<Arc<Limiter>> = OnceLock::new();

/// Record the startup settings (first call wins).
pub fn init(settings: GatewaySettings) {
    let _ = SETTINGS.set(settings);
}

/// The active settings (defaults before [`init`]).
pub fn settings() -> GatewaySettings {
    SETTINGS.get().cloned().unwrap_or_default()
}

/// The process-wide limiter, built from [`settings`] on first use.
pub fn limiter() -> Arc<Limiter> {
    LIMITER
        .get_or_init(|| Arc::new(Limiter::new(Limits::from_settings(&settings()))))
        .clone()
}

// ── Scope predicates (pure) ─────────────────────────────────────────────────

/// May a credential of `scope` call `method path`? Full scope: everything the
/// normal auth layers allow. `voice_only`: ONLY the voice socket and its
/// health probe. Applied by the LAN listener as defence in depth; on the main
/// listener the same outcome follows from `DeviceRegistry::verify` refusing
/// `voice_only` tokens.
pub fn route_allowed_for_scope(scope: DeviceScope, method: &str, path: &str) -> bool {
    match scope {
        DeviceScope::Full => true,
        DeviceScope::VoiceOnly => {
            method.eq_ignore_ascii_case("GET") && (path == "/voice" || path == "/voice/health")
        }
    }
}

/// Why a `/voice` open by a `voice_only` credential was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionReject {
    /// No explicit `session_id` (the "latest session" fallback is not allowed).
    Missing,
    /// Malformed id.
    Invalid,
    /// The credential is bound to a different conversation.
    NotBound,
}

const MAX_SESSION_ID_LEN: usize = 128;

/// Conversation binding rule. `bound` is the credential's conversation (if
/// provisioned or already claimed); `requested` is the `session_id` query
/// parameter. Returns the session to use.
pub fn authorize_session(
    bound: Option<&str>,
    requested: Option<&str>,
) -> Result<String, SessionReject> {
    let requested = requested.map(str::trim).filter(|s| !s.is_empty());
    let requested = requested.ok_or(SessionReject::Missing)?;
    if requested.len() > MAX_SESSION_ID_LEN
        || !requested
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
    {
        return Err(SessionReject::Invalid);
    }
    match bound {
        Some(bound) if bound != requested => Err(SessionReject::NotBound),
        _ => Ok(requested.to_string()),
    }
}

/// Mask credential material in free text before it is logged: `Bearer <x>`
/// and `token=<x>` values become `[redacted]`.
pub fn redact_credentials(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let bearer = rest.find("Bearer ").map(|i| (i, "Bearer ".len()));
        let query = rest.find("token=").map(|i| (i, "token=".len()));
        let next = match (bearer, query) {
            (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
            (a, b) => a.or(b),
        };
        let Some((at, len)) = next else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..at + len]);
        out.push_str("[redacted]");
        let tail = &rest[at + len..];
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '&' | '"' | '\'' | ','))
            .unwrap_or(tail.len());
        rest = &tail[end..];
    }
}

// ── Rate limits ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Limits {
    pub max_connections_per_device: usize,
    pub max_attempts_per_ip_per_minute: usize,
    pub max_turns_per_minute: usize,
    pub max_audio_bytes_per_minute: u64,
    pub max_turn: Duration,
}

impl Limits {
    pub fn from_settings(s: &GatewaySettings) -> Self {
        Self {
            max_connections_per_device: s.max_connections_per_device,
            max_attempts_per_ip_per_minute: s.max_attempts_per_ip_per_minute,
            max_turns_per_minute: s.max_turns_per_minute,
            max_audio_bytes_per_minute: s.max_audio_bytes_per_minute,
            max_turn: Duration::from_secs(s.max_turn_secs),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denied {
    ConnectionsPerDevice,
    AttemptsPerIp,
    TurnsPerMinute,
    AudioBytes,
}

impl Denied {
    /// Spoken/visible reason for the watch (no internals).
    pub fn message(self) -> &'static str {
        match self {
            Denied::ConnectionsPerDevice => "Too many open voice connections.",
            Denied::AttemptsPerIp => "Too many requests. Try again in a minute.",
            Denied::TurnsPerMinute => "Too many voice turns. Try again in a minute.",
            Denied::AudioBytes => "Voice audio limit reached. Try again in a minute.",
        }
    }
}

const WINDOW: Duration = Duration::from_secs(60);

#[derive(Default)]
struct LimiterState {
    active: HashMap<String, usize>,
    attempts: HashMap<IpAddr, VecDeque<Instant>>,
    turns: HashMap<String, VecDeque<Instant>>,
    audio: HashMap<String, VecDeque<(Instant, u64)>>,
}

/// Counters exposed for the log line and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    pub connections_admitted: u64,
    pub turns_admitted: u64,
    pub audio_bytes_admitted: u64,
    pub denied: u64,
}

pub struct Limiter {
    limits: Limits,
    state: Mutex<LimiterState>,
    connections_admitted: AtomicU64,
    turns_admitted: AtomicU64,
    audio_bytes_admitted: AtomicU64,
    denied: AtomicU64,
}

/// Holds one connection slot for a credential; released on drop.
pub struct ConnectionPermit {
    limiter: Arc<Limiter>,
    key: String,
}

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        let mut st = self.limiter.state.lock().unwrap();
        if let Some(n) = st.active.get_mut(&self.key) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                st.active.remove(&self.key);
            }
        }
    }
}

fn prune_instants(q: &mut VecDeque<Instant>, now: Instant) {
    while q
        .front()
        .is_some_and(|t| now.saturating_duration_since(*t) >= WINDOW)
    {
        q.pop_front();
    }
}

impl Limiter {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            state: Mutex::new(LimiterState::default()),
            connections_admitted: AtomicU64::new(0),
            turns_admitted: AtomicU64::new(0),
            audio_bytes_admitted: AtomicU64::new(0),
            denied: AtomicU64::new(0),
        }
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    pub fn counters(&self) -> Counters {
        Counters {
            connections_admitted: self.connections_admitted.load(Ordering::Relaxed),
            turns_admitted: self.turns_admitted.load(Ordering::Relaxed),
            audio_bytes_admitted: self.audio_bytes_admitted.load(Ordering::Relaxed),
            denied: self.denied.load(Ordering::Relaxed),
        }
    }

    fn deny(&self, kind: Denied, key: &str) -> Denied {
        let total = self.denied.fetch_add(1, Ordering::Relaxed) + 1;
        // `key` is a device id or IP — never a credential.
        tracing::warn!(
            target: "permagentd::voice_gateway",
            event = "limit_denied",
            kind = ?kind,
            key,
            denied_total = total,
            "voice gateway limit hit"
        );
        kind
    }

    /// Count one request from `ip`; refuse beyond the per-minute budget.
    pub fn record_attempt(&self, ip: IpAddr, now: Instant) -> Result<(), Denied> {
        let mut st = self.state.lock().unwrap();
        if st.attempts.len() > 4096 {
            st.attempts.retain(|_, q| {
                prune_instants(q, now);
                !q.is_empty()
            });
        }
        let q = st.attempts.entry(ip).or_default();
        prune_instants(q, now);
        if q.len() >= self.limits.max_attempts_per_ip_per_minute {
            drop(st);
            return Err(self.deny(Denied::AttemptsPerIp, &ip.to_string()));
        }
        q.push_back(now);
        Ok(())
    }

    /// Take a connection slot for credential `device_id`.
    pub fn try_connect(self: &Arc<Self>, device_id: &str) -> Result<ConnectionPermit, Denied> {
        let mut st = self.state.lock().unwrap();
        let n = st.active.entry(device_id.to_string()).or_insert(0);
        if *n >= self.limits.max_connections_per_device {
            drop(st);
            return Err(self.deny(Denied::ConnectionsPerDevice, device_id));
        }
        *n += 1;
        drop(st);
        let total = self.connections_admitted.fetch_add(1, Ordering::Relaxed) + 1;
        tracing::info!(
            target: "permagentd::voice_gateway",
            event = "connection_admitted",
            device = device_id,
            connections_admitted = total,
            "voice gateway connection admitted"
        );
        Ok(ConnectionPermit {
            limiter: self.clone(),
            key: device_id.to_string(),
        })
    }

    /// Admit one turn start for `device_id` within the per-minute budget.
    pub fn allow_turn(&self, device_id: &str, now: Instant) -> Result<(), Denied> {
        let mut st = self.state.lock().unwrap();
        let q = st.turns.entry(device_id.to_string()).or_default();
        prune_instants(q, now);
        if q.len() >= self.limits.max_turns_per_minute {
            drop(st);
            return Err(self.deny(Denied::TurnsPerMinute, device_id));
        }
        q.push_back(now);
        drop(st);
        self.turns_admitted.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Charge `bytes` of inbound audio against `device_id`'s per-minute budget.
    pub fn charge_audio(&self, device_id: &str, bytes: usize, now: Instant) -> Result<(), Denied> {
        let bytes = bytes as u64;
        let mut st = self.state.lock().unwrap();
        let q = st.audio.entry(device_id.to_string()).or_default();
        while q
            .front()
            .is_some_and(|(t, _)| now.saturating_duration_since(*t) >= WINDOW)
        {
            q.pop_front();
        }
        let used: u64 = q.iter().map(|(_, b)| *b).sum();
        if used.saturating_add(bytes) > self.limits.max_audio_bytes_per_minute {
            drop(st);
            return Err(self.deny(Denied::AudioBytes, device_id));
        }
        q.push_back((now, bytes));
        drop(st);
        self.audio_bytes_admitted
            .fetch_add(bytes, Ordering::Relaxed);
        Ok(())
    }
}

// ── Per-socket gate (hooks in routes/voice.rs) ──────────────────────────────

/// Everything `handle_voice_socket` needs to enforce a `voice_only` socket.
/// Absent (`None`) for ordinary phone/desktop sockets, which are unaffected.
pub struct VoiceSocketGate {
    device_id: String,
    limiter: Arc<Limiter>,
    _permit: ConnectionPermit,
}

impl VoiceSocketGate {
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Called when a turn starts. `Err` carries the message to show.
    pub fn allow_turn(&self) -> Result<(), &'static str> {
        self.limiter
            .allow_turn(&self.device_id, Instant::now())
            .map_err(Denied::message)
    }

    /// Called for every inbound audio frame.
    pub fn charge_audio(&self, bytes: usize) -> Result<(), &'static str> {
        self.limiter
            .charge_audio(&self.device_id, bytes, Instant::now())
            .map_err(Denied::message)
    }

    /// Clamp a capture deadline to the gateway's maximum turn duration.
    pub fn clamp_deadline(&self, requested: tokio::time::Instant) -> tokio::time::Instant {
        requested.min(tokio::time::Instant::now() + self.limiter.limits().max_turn)
    }

    /// False once the credential is revoked, expired or orphaned — the socket
    /// loop closes when this turns false.
    pub fn still_valid(&self, registry: &DeviceRegistry) -> bool {
        registry.voice_credential_valid(&self.device_id, chrono::Utc::now())
    }
}

/// Everything `voice_ws_handler` needs after a successful `voice_only` auth.
pub struct Admission {
    pub gate: VoiceSocketGate,
    pub session_id: String,
    pub origin: permagent::events::voice_origin::VoiceOrigin,
}

/// Authenticate and authorize a `voice_only` open of `/voice`.
///
/// `bearer` is the `Authorization: Bearer` value, `None` if absent or not a
/// voice credential (the caller then falls through to the ordinary path).
/// Status codes: 401 bad/expired/revoked credential, 403 wrong client or
/// conversation, 429 connection limit.
pub fn admit_voice_only(
    registry: &DeviceRegistry,
    limiter: &Arc<Limiter>,
    bearer: &str,
    declared_client: Option<&str>,
    requested_session: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Admission, StatusCode> {
    use permagent::events::voice_origin::{VoiceClient, VoiceOrigin};
    let cred: VoiceCredential = registry
        .verify_voice(bearer, now)
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if VoiceClient::parse(declared_client) != VoiceClient::Watch {
        return Err(StatusCode::FORBIDDEN);
    }
    let session_id = authorize_session(cred.conversation_id.as_deref(), requested_session)
        .map_err(|_| StatusCode::FORBIDDEN)?;
    if !registry.bind_conversation(&cred.id, &session_id) {
        return Err(StatusCode::FORBIDDEN);
    }
    let permit = limiter
        .try_connect(&cred.id)
        .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
    registry.touch(&cred.id);
    let origin = VoiceOrigin::resolve(Some("watch_voice"), Some(&cred.name)).with_voice_only();
    Ok(Admission {
        gate: VoiceSocketGate {
            device_id: cred.id,
            limiter: limiter.clone(),
            _permit: permit,
        },
        session_id,
        origin,
    })
}

// ── TLS material ────────────────────────────────────────────────────────────

pub struct CertMaterial {
    pub cert_pem: String,
    pub key_pem: String,
    /// SHA-256 of the leaf DER, colon-separated uppercase hex (same format as
    /// `tls::sha256_fingerprint`).
    pub fingerprint: String,
}

/// `<profile>/secrets/voice_gateway`.
pub fn default_cert_dir() -> PathBuf {
    permagent::config::paths::Paths::data_dir()
        .join("secrets")
        .join("voice_gateway")
}

pub fn fingerprint_of_pem(cert_pem: &str) -> anyhow::Result<String> {
    let der = pem::parse(cert_pem.as_bytes())?.into_contents();
    let digest = Sha256::digest(&der);
    Ok(digest
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":"))
}

/// Load the gateway certificate from `dir`, generating a self-signed one
/// (0600 key, private dir) if absent. The cert is reused across restarts so
/// the watch's pinned fingerprint stays valid; delete the files to rotate.
pub fn load_or_create_cert(dir: &Path, extra_hosts: &[String]) -> anyhow::Result<CertMaterial> {
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    if let (Ok(cert_pem), Ok(key_pem)) = (
        std::fs::read_to_string(&cert_path),
        std::fs::read_to_string(&key_path),
    ) {
        let fingerprint = fingerprint_of_pem(&cert_pem)?;
        return Ok(CertMaterial {
            cert_pem,
            key_pem,
            fingerprint,
        });
    }
    use rcgen::{CertificateParams, DnType, KeyPair, SanType};
    let mut params = CertificateParams::default();
    params
        .distinguished_name
        .push(DnType::CommonName, "permagent voice gateway");
    let mut sans = vec![
        SanType::IpAddress(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
        SanType::DnsName("localhost".try_into()?),
    ];
    for host in extra_hosts {
        match host.parse::<IpAddr>() {
            Ok(ip) => sans.push(SanType::IpAddress(ip)),
            Err(_) => sans.push(SanType::DnsName(host.as_str().try_into()?)),
        }
    }
    params.subject_alt_names = sans;
    let key_pair = KeyPair::generate()?;
    let cert = params.self_signed(&key_pair)?;
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();
    permagent::config::secure_fs::ensure_private_dir(dir)?;
    permagent::config::secure_fs::write_private_file(&cert_path, cert_pem.as_bytes())?;
    permagent::config::secure_fs::write_private_file(&key_path, key_pem.as_bytes())?;
    let fingerprint = fingerprint_of_pem(&cert_pem)?;
    Ok(CertMaterial {
        cert_pem,
        key_pem,
        fingerprint,
    })
}

/// Primary LAN IPv4 via a connected (never sent) UDP socket.
fn detect_lan_ip() -> Option<IpAddr> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("192.0.2.1:9").ok()?;
    let ip = sock.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

fn advertised_host(s: &GatewaySettings) -> Option<String> {
    s.advertise_host
        .as_deref()
        .map(str::trim)
        .filter(|h| !h.is_empty())
        .map(str::to_string)
        .or_else(|| detect_lan_ip().map(|ip| ip.to_string()))
}

/// `wss://host:port` for `host` (IPv6 literals bracketed).
pub fn hub_url_for(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("wss://[{host}]:{port}")
    } else {
        format!("wss://{host}:{port}")
    }
}

/// Facts the phone hands to the watch.
pub struct Provisioning {
    pub hub_url_lan: String,
    pub cert_sha256: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProvisionError {
    /// `daemon.voice_gateway.enabled` is false.
    Disabled,
    /// No usable LAN host (set `advertise_host`).
    NoHost,
    /// Certificate could not be loaded or created.
    Cert,
}

pub fn provisioning(s: &GatewaySettings, cert_dir: &Path) -> Result<Provisioning, ProvisionError> {
    if !s.enabled {
        return Err(ProvisionError::Disabled);
    }
    let host = advertised_host(s).ok_or(ProvisionError::NoHost)?;
    let material = load_or_create_cert(cert_dir, std::slice::from_ref(&host)).map_err(|e| {
        tracing::error!(target: "permagentd::voice_gateway", "certificate unavailable: {e}");
        ProvisionError::Cert
    })?;
    Ok(Provisioning {
        hub_url_lan: hub_url_for(&host, s.port),
        cert_sha256: material.fingerprint,
    })
}

// ── LAN listener ────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

/// Defence-in-depth guard in front of the two gateway routes: refuses any
/// other path, refuses browser-origin requests (the watch is a native client),
/// and rate-limits requests per source IP before any auth work happens.
async fn gateway_guard(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let path = request.uri().path().to_owned();
    if !route_allowed_for_scope(DeviceScope::VoiceOnly, request.method().as_str(), &path) {
        return Err(StatusCode::NOT_FOUND);
    }
    if request.headers().contains_key("origin") {
        return Err(StatusCode::FORBIDDEN);
    }
    limiter()
        .record_attempt(peer.ip(), Instant::now())
        .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
    Ok(next.run(request).await)
}

/// Router for the LAN listener: `/voice` + `/voice/health`, nothing else.
pub fn gateway_router(state: Arc<AppState>) -> Router {
    Router::new()
        .merge(crate::routes::voice::routes(state))
        .route("/voice/health", get(health))
        .layer(axum::middleware::from_fn(gateway_guard))
}

/// Record settings and, when enabled, start the LAN listener in the background.
pub fn spawn(state: Arc<AppState>, settings: GatewaySettings) {
    init(settings.clone());
    if !settings.enabled {
        tracing::info!(
            target: "permagentd::voice_gateway",
            "voice gateway disabled (daemon.voice_gateway.enabled=false)"
        );
        return;
    }
    tokio::spawn(async move {
        if let Err(e) = serve(state, settings).await {
            tracing::error!(target: "permagentd::voice_gateway", "voice gateway stopped: {e}");
        }
    });
}

async fn serve(state: Arc<AppState>, settings: GatewaySettings) -> anyhow::Result<()> {
    let host = advertised_host(&settings);
    let hosts: Vec<String> = host.into_iter().collect();
    let material = load_or_create_cert(&default_cert_dir(), &hosts)?;
    let addr: SocketAddr = format!("{}:{}", settings.bind, settings.port).parse()?;
    let app = gateway_router(state);
    tracing::info!(
        target: "permagentd::voice_gateway",
        "voice gateway listening on https://{addr} (cert sha256 {})",
        material.fingerprint
    );

    #[cfg(feature = "rustls-tls")]
    {
        let config = axum_server::tls_rustls::RustlsConfig::from_pem(
            material.cert_pem.into_bytes(),
            material.key_pem.into_bytes(),
        )
        .await?;
        axum_server::bind_rustls(addr, config)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .await?;
    }
    #[cfg(feature = "native-tls")]
    {
        let config = axum_server::tls_openssl::OpenSSLConfig::from_pem(
            material.cert_pem.as_bytes(),
            material.key_pem.as_bytes(),
        )?;
        axum_server::bind_openssl(addr, config)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .await?;
    }
    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> Limits {
        Limits {
            max_connections_per_device: 2,
            max_attempts_per_ip_per_minute: 3,
            max_turns_per_minute: 2,
            max_audio_bytes_per_minute: 1000,
            max_turn: Duration::from_secs(45),
        }
    }

    fn registry() -> (tempfile::TempDir, DeviceRegistry) {
        let dir = tempfile::tempdir().unwrap();
        let reg = DeviceRegistry::load(dir.path().join("device_tokens.json"));
        (dir, reg)
    }

    #[test]
    fn scope_predicate_voice_only_reaches_only_voice() {
        for (m, p) in [("GET", "/voice"), ("GET", "/voice/health")] {
            assert!(route_allowed_for_scope(DeviceScope::VoiceOnly, m, p));
        }
        for (m, p) in [
            ("GET", "/api/devices"),
            ("POST", "/api/devices/x/watch-credential"),
            ("POST", "/api/devices/pair"),
            ("POST", "/pair/claim"),
            ("GET", "/events"),
            ("GET", "/voice/pronunciations"),
            ("POST", "/voice"),
            ("GET", "/voice/"),
            ("GET", "/api/config"),
            ("POST", "/agent/start"),
            ("GET", "/"),
        ] {
            assert!(
                !route_allowed_for_scope(DeviceScope::VoiceOnly, m, p),
                "{m} {p} must be refused for voice_only"
            );
            assert!(route_allowed_for_scope(DeviceScope::Full, m, p));
        }
    }

    #[test]
    fn voice_only_token_is_rejected_by_the_general_auth_core() {
        // This is the rule that makes every non-/voice route refuse it.
        let (_d, reg) = registry();
        let (_pt, phone) = reg.pair("Phone");
        let (tok, _w) = reg
            .mint_voice_credential(
                &phone.id,
                Some("c1"),
                chrono::Duration::days(30),
                chrono::Utc::now(),
            )
            .unwrap();
        let verdict =
            crate::middleware::auth::validate_with_devices(Some("master"), &reg, Some(&tok));
        assert_eq!(verdict, Err(StatusCode::UNAUTHORIZED));
    }

    #[test]
    fn session_binding_rule() {
        assert_eq!(
            authorize_session(Some("c1"), Some("c1")),
            Ok("c1".to_string())
        );
        assert_eq!(authorize_session(None, Some("c9")), Ok("c9".to_string()));
        assert_eq!(
            authorize_session(Some("c1"), Some("c2")),
            Err(SessionReject::NotBound)
        );
        assert_eq!(
            authorize_session(Some("c1"), None),
            Err(SessionReject::Missing)
        );
        assert_eq!(
            authorize_session(None, Some("  ")),
            Err(SessionReject::Missing)
        );
        assert_eq!(
            authorize_session(None, Some("a b")),
            Err(SessionReject::Invalid)
        );
        assert_eq!(
            authorize_session(None, Some(&"x".repeat(MAX_SESSION_ID_LEN + 1))),
            Err(SessionReject::Invalid)
        );
    }

    #[test]
    fn admission_requires_watch_client_session_and_binds() {
        let (_d, reg) = registry();
        let lim = Arc::new(Limiter::new(limits()));
        let (_pt, phone) = reg.pair("Phone");
        let now = chrono::Utc::now();
        let (tok, _w) = reg
            .mint_voice_credential(&phone.id, None, chrono::Duration::days(30), now)
            .unwrap();
        let ok = |client, session| admit_voice_only(&reg, &lim, &tok, client, session, now);
        assert_eq!(
            ok(Some("watch_voice"), None).err(),
            Some(StatusCode::FORBIDDEN)
        );
        assert_eq!(
            ok(Some("ios_voice"), Some("c1")).err(),
            Some(StatusCode::FORBIDDEN)
        );
        assert_eq!(ok(None, Some("c1")).err(), Some(StatusCode::FORBIDDEN));
        let a = ok(Some("watch_voice"), Some("c1")).ok().expect("admitted");
        assert_eq!(a.session_id, "c1");
        assert!(a.origin.voice_only);
        assert_eq!(
            a.origin.client,
            permagent::events::voice_origin::VoiceClient::Watch
        );
        drop(a);
        // Now bound to c1: another conversation is refused.
        assert_eq!(
            ok(Some("watch_voice"), Some("c2")).err(),
            Some(StatusCode::FORBIDDEN)
        );
        assert!(ok(Some("watch_voice"), Some("c1")).is_ok());
        // Bad token.
        assert_eq!(
            admit_voice_only(&reg, &lim, "nope", Some("watch_voice"), Some("c1"), now).err(),
            Some(StatusCode::UNAUTHORIZED)
        );
        // Expired / revoked.
        assert_eq!(
            admit_voice_only(
                &reg,
                &lim,
                &tok,
                Some("watch_voice"),
                Some("c1"),
                now + chrono::Duration::days(31)
            )
            .err(),
            Some(StatusCode::UNAUTHORIZED)
        );
    }

    #[test]
    fn revoked_credential_is_refused_and_live_gate_goes_invalid() {
        let (_d, reg) = registry();
        let lim = Arc::new(Limiter::new(limits()));
        let (_pt, phone) = reg.pair("Phone");
        let now = chrono::Utc::now();
        let (tok, w) = reg
            .mint_voice_credential(&phone.id, Some("c1"), chrono::Duration::days(30), now)
            .unwrap();
        let a = admit_voice_only(&reg, &lim, &tok, Some("watch_voice"), Some("c1"), now)
            .ok()
            .unwrap();
        assert!(a.gate.still_valid(&reg));
        reg.revoke(&w.id).unwrap();
        assert!(!a.gate.still_valid(&reg));
        assert_eq!(
            admit_voice_only(&reg, &lim, &tok, Some("watch_voice"), Some("c1"), now).err(),
            Some(StatusCode::UNAUTHORIZED)
        );
    }

    #[test]
    fn connection_slots_are_released_on_drop() {
        let lim = Arc::new(Limiter::new(limits()));
        let a = lim.try_connect("d1").unwrap();
        let _b = lim.try_connect("d1").unwrap();
        assert_eq!(
            lim.try_connect("d1").err(),
            Some(Denied::ConnectionsPerDevice)
        );
        assert!(lim.try_connect("d2").is_ok(), "per-device, not global");
        drop(a);
        assert!(lim.try_connect("d1").is_ok());
        assert!(lim.counters().denied >= 1);
    }

    #[test]
    fn turn_audio_and_ip_limits_roll_over_a_minute() {
        let lim = Limiter::new(limits());
        let t0 = Instant::now();
        assert!(lim.allow_turn("d", t0).is_ok());
        assert!(lim.allow_turn("d", t0).is_ok());
        assert_eq!(lim.allow_turn("d", t0), Err(Denied::TurnsPerMinute));
        assert!(lim.allow_turn("d", t0 + Duration::from_secs(61)).is_ok());

        assert!(lim.charge_audio("d", 600, t0).is_ok());
        assert_eq!(lim.charge_audio("d", 600, t0), Err(Denied::AudioBytes));
        assert!(lim.charge_audio("d", 400, t0).is_ok());
        assert!(lim
            .charge_audio("d", 600, t0 + Duration::from_secs(61))
            .is_ok());

        let ip: IpAddr = "192.168.1.9".parse().unwrap();
        for _ in 0..3 {
            assert!(lim.record_attempt(ip, t0).is_ok());
        }
        assert_eq!(lim.record_attempt(ip, t0), Err(Denied::AttemptsPerIp));
        assert!(lim.record_attempt(ip, t0 + Duration::from_secs(61)).is_ok());
        let c = lim.counters();
        assert_eq!(c.turns_admitted, 3);
        assert_eq!(c.audio_bytes_admitted, 1600);
        assert_eq!(c.denied, 3);
    }

    #[test]
    fn turn_deadline_is_clamped_to_max_turn() {
        let lim = Arc::new(Limiter::new(limits()));
        let gate_lim = lim.clone();
        let permit = lim.try_connect("d").unwrap();
        let gate = VoiceSocketGate {
            device_id: "d".into(),
            limiter: gate_lim,
            _permit: permit,
        };
        let far = tokio::time::Instant::now() + Duration::from_secs(600);
        assert!(gate.clamp_deadline(far) < far);
        let near = tokio::time::Instant::now() + Duration::from_secs(5);
        assert_eq!(gate.clamp_deadline(near), near);
    }

    #[test]
    fn credentials_are_redacted_in_text() {
        let s = redact_credentials(
            "GET /voice?session_id=a&token=abc123&x=1 Authorization: Bearer s3cret end",
        );
        assert!(!s.contains("abc123"));
        assert!(!s.contains("s3cret"));
        assert!(s.contains("session_id=a"));
        assert!(s.contains("Bearer [redacted] end"));
        assert_eq!(redact_credentials("nothing here"), "nothing here");
    }

    #[test]
    fn hub_url_brackets_ipv6() {
        assert_eq!(hub_url_for("192.168.1.5", 3443), "wss://192.168.1.5:3443");
        assert_eq!(hub_url_for("fe80::1", 3443), "wss://[fe80::1]:3443");
    }

    #[test]
    fn cert_is_generated_once_and_fingerprint_is_stable() {
        let dir = tempfile::tempdir().unwrap();
        let first = load_or_create_cert(dir.path(), &["192.168.1.5".to_string()]).unwrap();
        let second = load_or_create_cert(dir.path(), &["192.168.1.5".to_string()]).unwrap();
        assert_eq!(first.fingerprint, second.fingerprint);
        assert_eq!(first.fingerprint.split(':').count(), 32);
        assert!(dir.path().join("key.pem").exists());
    }

    #[test]
    fn provisioning_refuses_when_disabled_and_reports_pin_when_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = GatewaySettings::default();
        assert_eq!(
            provisioning(&s, dir.path()).err(),
            Some(ProvisionError::Disabled)
        );
        s.enabled = true;
        s.advertise_host = Some("hub.local".into());
        s.port = 4443;
        let p = provisioning(&s, dir.path()).ok().unwrap();
        assert_eq!(p.hub_url_lan, "wss://hub.local:4443");
        assert_eq!(p.cert_sha256.split(':').count(), 32);
    }

    #[test]
    fn settings_default_off_and_parse_from_yaml() {
        assert!(!GatewaySettings::default().enabled);
        let s: GatewaySettings =
            serde_yaml::from_str("enabled: true\nport: 5000\nmax_turns_per_minute: 3\n").unwrap();
        assert!(s.enabled);
        assert_eq!(s.port, 5000);
        assert_eq!(s.max_turns_per_minute, 3);
        assert_eq!(s.max_turn_secs, 45, "unset keys keep defaults");
    }
}
