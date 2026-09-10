//! Deterministic provider-traffic cassette (Lane R1, harness improvement loop).
//!
//! Goal: make mechanism regressions in the coordinator/worker pipeline (a
//! rejected approval payload, an orphaning requeue, a misfired dead-worker
//! detector, ...) catchable in seconds, with no network call and no spend,
//! by recording a real eval run's provider traffic ONCE and replaying it
//! through the REAL pipeline (session builder -> coordinator -> decompose ->
//! roadmap -> approval -> dispatch -> worker -> verify -> review -> landing).
//!
//! [`CassetteProvider`] is a [`Provider`] decorator installed at the single
//! provider-factory choke point (`provider_registry.rs`'s
//! `ProviderEntry::create` / `create_with_default_model`, the same choke
//! point [`super::sovereign_guard::SovereignGuardProvider`] uses) so every
//! session the choke point mints a provider for -- coordinator, worker,
//! reviewer -- is covered without each caller having to know a cassette is
//! in play. It is gated by one env var, read once per provider construction
//! (same "read once, at install time" idiom as `PERMAGENT_HEADLESS_AUTOLAND`,
//! `orchestrator.rs:6701-6708`): unset, the choke point is byte-for-byte
//! unchanged (this module is never even reached).
//!
//! `PERMAGENT_PROVIDER_CASSETTE=record:<dir>`: the real provider is
//! constructed and called exactly as before (recording never changes live
//! behavior for the caller); every `stream()` request/response pair is
//! ALSO appended, one JSON object per line, to `<dir>/cassette.jsonl`,
//! keyed by [`cassette_key`] -- a stable SHA-256 hash of (model name, system
//! prompt, canonicalized messages, canonicalized tools). Secrets are
//! stripped from the stored (not the hashed) request/response before it
//! ever reaches disk (see [`redact_secrets_in_value`]).
//!
//! `PERMAGENT_PROVIDER_CASSETTE=replay:<dir>`: `<dir>/cassette.jsonl` is
//! loaded ONCE, at provider construction, into an in-memory map. No real
//! provider is ever constructed for replay -- [`CassetteProvider::for_replay`]
//! needs no credentials and makes no network call. A `stream()` call looks
//! its request up by the same key; a hit returns the recorded
//! message/usage verbatim. A miss is NEVER fabricated: the full unmatched
//! request (secrets stripped) is appended to `<dir>/misses.jsonl` and the
//! call fails loudly with [`ProviderError::ExecutionError`] naming the
//! cassette dir and the key.
//!
//! ## Why the key strips ids/timestamps but the stored value keeps them
//!
//! Every real run mints fresh, non-deterministic identifiers (`Message::id`,
//! `Message::created`, per-call `invocation_id`s -- confirmed at
//! `agents/agent.rs:1992`, "every provider call mints a fresh invocation_id
//! via `Uuid::new_v4()`") even when the CONTENT of the conversation is
//! identical between a record run and a later replay run. Hashing the raw
//! serialized request would therefore never match twice. [`cassette_key`]
//! strips every JSON object key literally named `id`/`created`/`_meta`/`meta`
//! (recursively, at any depth -- covers `Message.id`, `ToolRequest.id`,
//! `ToolResponse.id`, and similar) and additionally normalizes any embedded
//! ISO-8601 timestamp or UUID found INSIDE text content (e.g. a tool
//! response that prints `"dispatched_at": "..."`) to a stable placeholder
//! before hashing. The value actually written to the cassette line keeps
//! every field -- replay must reproduce byte-for-byte, whereas the KEY only
//! needs to recognize "the same request shape."
//!
//! This is a heuristic, not a proof of determinism: an unrecognized volatile
//! substring can still make a real recording miss on replay. The honest
//! fallback is the miss policy above -- fail loud and inspectable, never
//! silently wrong.
//!
//! ## Known limitations (documented, not silently worked around)
//!
//! - Secret redaction can false-positive on a long hex/base64-looking
//!   string that is not actually a secret, altering the stored (and
//!   therefore replayed) text cosmetically. It never leaks a secret and the
//!   oracle grades real tool/filesystem state, not chat transcript text, so
//!   this is very unlikely to flip a fixture's verdict -- but it is a real
//!   trade-off, not a non-issue.
//! - `cost_tier()`/`data_locality()` in replay mode are REPLAYED, not
//!   guessed: record writes the wrapped provider's real tier/locality into
//!   every cassette line (`provider.cost_tier`/`provider.data_locality`)
//!   and [`CassetteProvider::for_replay`] reports them back. This matters
//!   for more than fidelity -- the accounted-completion path
//!   (`agents/reply_parts.rs`'s `reserve_provider_invocation_for_model`)
//!   asks the provider for its tier BEFORE the call, and a `PaidApi` answer
//!   demands a real dollar reservation (which in turn demands a context
//!   limit and a durable task). Answering `PaidApi` for a recording that
//!   was really a `LocalFree` call would make replay refuse before it ever
//!   reached the cassette -- the same class of bug
//!   `SovereignGuardProvider::cost_tier`'s own doc comment records. A
//!   cassette with no recorded tier (an older file, or a hand-written one)
//!   still falls back to the trait's fail-closed `PaidApi`/`Cloud`: a
//!   recorded paid cloud call replays through the SAME
//!   reservation/approval/budget mechanics a real paid run would, rather
//!   than bypassing them because nothing is actually being spent this time.
//! - One cassette file is shared by every provider construction that shares
//!   its directory. A run where the coordinator, worker, and reviewer use
//!   DIFFERENT provider/model tiers all writes into the same
//!   `cassette.jsonl` -- fine (keys already fold in the model name so they
//!   cannot collide), but there is no per-role manifest; `permagent-eval run
//!   --cassette <dir>` (see `crates/permagent-eval/src/main.rs`) currently
//!   drives a single tier at a time, which is the only shape this lane
//!   proves.
//!
//! ## On-disk format
//!
//! `<dir>/cassette.jsonl` -- UTF-8, one JSON object per line (order does not
//! matter; replay loads the whole file into a `HashMap` keyed by `key`), no
//! trailing-comma / array wrapper. Blank lines are skipped. Each line:
//!
//! ```json
//! {
//!   "key": "<64-hex-char sha256 of the canonicalized request, see cassette_key>",
//!   "provider": {
//!     "name": "<the recorded provider's own get_name()>",
//!     "cost_tier": "local_free" | "subscription" | "paid_api",
//!     "data_locality": "local" | "cloud"
//!   },
//!   "request": {
//!     "model": "<model name, informational + part of what was hashed>",
//!     "system": "<system prompt text, secrets redacted>",
//!     "messages": [ /* Vec<Message> (this crate's wire format), secrets redacted */ ],
//!     "tools": [ /* Vec<rmcp::model::Tool> exactly as offered to the provider */ ]
//!   },
//!   "response": {
//!     "message": { /* the single Message the provider returned */ },
//!     "usage": { /* ProviderUsage: model, usage{input/output/total/cache_*_tokens}, invocation_id */ }
//!   }
//! }
//! ```
//!
//! `request` is NEVER read back for matching (matching is by `key` alone) --
//! it exists purely for a human (or a future analysis tool) to read a
//! cassette file and see what was actually asked, without re-deriving it
//! from the hash. `response.message`/`response.usage` are replayed
//! byte-for-byte on a key hit.
//!
//! `<dir>/misses.jsonl` uses the same per-line shape minus `response`:
//! `{"key": "...", "request": {...}}`, one line per unmatched replay
//! lookup, append-only, written even when the miss error is later
//! propagated and the run aborts.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;
use rmcp::model::Tool;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::base::{stream_from_single_message, MessageStream, Provider, ProviderUsage};
use super::errors::ProviderError;
use super::retry::RetryConfig;
use crate::conversation::message::Message;
use crate::model::ModelConfig;

/// Env var read once per provider construction, at the same choke point
/// [`super::sovereign_guard::SovereignGuardProvider::wrap`] is installed at.
pub const PERMAGENT_PROVIDER_CASSETTE_ENV: &str = "PERMAGENT_PROVIDER_CASSETTE";

const CASSETTE_FILE_NAME: &str = "cassette.jsonl";
const MISSES_FILE_NAME: &str = "misses.jsonl";

/// Parsed value of [`PERMAGENT_PROVIDER_CASSETTE_ENV`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CassetteMode {
    Record(PathBuf),
    Replay(PathBuf),
}

impl CassetteMode {
    /// Reads and parses the env var. `None` when unset (the common case --
    /// the caller should fall through to its unwrapped default behavior).
    /// `Some(Err(_))` when the var is set but malformed, so a typo fails the
    /// run loudly instead of silently running without a cassette.
    pub fn from_env() -> Option<Result<Self, String>> {
        std::env::var(PERMAGENT_PROVIDER_CASSETTE_ENV)
            .ok()
            .map(|raw| Self::parse(&raw))
    }

    fn parse(raw: &str) -> Result<Self, String> {
        let (kind, dir) = raw.trim().split_once(':').ok_or_else(|| {
            format!(
                "{PERMAGENT_PROVIDER_CASSETTE_ENV} must be \"record:<dir>\" or \"replay:<dir>\", got {raw:?}"
            )
        })?;
        if dir.trim().is_empty() {
            return Err(format!(
                "{PERMAGENT_PROVIDER_CASSETTE_ENV} directory is empty in {raw:?}"
            ));
        }
        let dir = PathBuf::from(dir);
        match kind {
            "record" => Ok(CassetteMode::Record(dir)),
            "replay" => Ok(CassetteMode::Replay(dir)),
            other => Err(format!(
                "{PERMAGENT_PROVIDER_CASSETTE_ENV} mode must be \"record\" or \"replay\", got {other:?} (from {raw:?})"
            )),
        }
    }
}

// ── On-disk record shapes ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CassetteRequestRecord {
    model: String,
    system: String,
    messages: Vec<Message>,
    tools: Vec<Tool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CassetteResponseRecord {
    message: Message,
    usage: ProviderUsage,
}

/// Identity of the provider that produced a recorded turn. Replayed rather
/// than guessed -- see the module doc's "Known limitations" for why
/// answering `PaidApi` for a recording that was really `LocalFree` breaks
/// replay before it ever reaches the cassette.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CassetteProviderMeta {
    name: String,
    /// [`crate::session::CostTier::as_str`].
    cost_tier: String,
    /// [`crate::sovereignty::DataLocality::as_str`].
    data_locality: String,
}

impl CassetteProviderMeta {
    fn of(inner: &Arc<dyn Provider>) -> Self {
        Self {
            name: inner.get_name().to_string(),
            cost_tier: inner.cost_tier().as_str().to_string(),
            data_locality: inner.data_locality().as_str().to_string(),
        }
    }

    /// Fail-closed on anything unrecognized or absent: an unknown tier is
    /// chargeable and an unknown locality is cloud, matching the `Provider`
    /// trait's own defaults.
    fn cost_tier(&self) -> crate::session::CostTier {
        match self.cost_tier.as_str() {
            "local_free" => crate::session::CostTier::LocalFree,
            "subscription" => crate::session::CostTier::Subscription,
            _ => crate::session::CostTier::PaidApi,
        }
    }

    fn data_locality(&self) -> crate::sovereignty::DataLocality {
        match self.data_locality.as_str() {
            "local" => crate::sovereignty::DataLocality::Local,
            _ => crate::sovereignty::DataLocality::Cloud,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CassetteTurnLine {
    key: String,
    /// Absent in a cassette recorded before this field existed (or a
    /// hand-written one) -- replay then falls back to the trait's
    /// fail-closed `PaidApi`/`Cloud`.
    #[serde(default)]
    provider: Option<CassetteProviderMeta>,
    /// Kept for human debugging / future analysis. Replay looks records up
    /// by `key` alone; this is never re-hashed.
    request: CassetteRequestRecord,
    response: CassetteResponseRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CassetteMissLine {
    key: String,
    request: CassetteRequestRecord,
}

struct StoredTurn {
    message: Message,
    usage: ProviderUsage,
}

// ── Hashing / redaction: shared recursive JSON walkers ──────────────────────

/// Object keys that carry per-run identity or wall-clock noise rather than
/// semantic content, stripped at any depth before hashing. Deliberately
/// coarse (matches the literal key name anywhere in the tree, e.g. inside a
/// tool response's own JSON payload) -- see the module doc's "Why the key
/// strips ids/timestamps" section for the trade-off this accepts.
const VOLATILE_KEYS: &[&str] = &["id", "created", "_meta", "meta"];

fn strip_volatile_keys(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for key in VOLATILE_KEYS {
                map.remove(*key);
            }
            for v in map.values_mut() {
                strip_volatile_keys(v);
            }
        }
        serde_json::Value::Array(items) => {
            for v in items.iter_mut() {
                strip_volatile_keys(v);
            }
        }
        _ => {}
    }
}

fn walk_strings_mut<F: FnMut(&str) -> String>(value: &mut serde_json::Value, f: &mut F) {
    match value {
        serde_json::Value::String(s) => *s = f(s),
        serde_json::Value::Object(map) => {
            for v in map.values_mut() {
                walk_strings_mut(v, f);
            }
        }
        serde_json::Value::Array(items) => {
            for v in items.iter_mut() {
                walk_strings_mut(v, f);
            }
        }
        _ => {}
    }
}

static ISO8601_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:?\d{2})?")
        .expect("valid regex")
});
static UUID_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
        .expect("valid regex")
});

/// Replaces embedded ISO-8601 timestamps and UUIDs with stable placeholders
/// so the hash key is insensitive to wall-clock/identifier noise a live
/// pipeline mints fresh on every real run but that does not change what the
/// provider is actually being asked to do.
fn normalize_volatile_text(text: &str) -> String {
    let text = ISO8601_RE.replace_all(text, "<ts>");
    UUID_RE.replace_all(&text, "<uuid>").into_owned()
}

fn normalize_volatile_strings(value: &mut serde_json::Value) {
    walk_strings_mut(value, &mut |s: &str| normalize_volatile_text(s));
}

/// Secret-shaped substrings redacted from anything written to disk (never
/// from the hash key, which is discarded after hashing). Deliberately
/// pattern-based rather than an env-var allowlist: the request/response
/// shape carries no credentials by construction (auth lives in each
/// provider's own HTTP client, outside `Provider::stream`'s parameters --
/// confirmed by survey), so this is a defense-in-depth net for anything a
/// tool happened to echo back (e.g. a config file containing a key), not the
/// primary boundary.
static SECRET_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r"(?i)bearer\s+[A-Za-z0-9\-_.]{10,}").expect("valid regex"),
        Regex::new(r"sk-[A-Za-z0-9]{10,}").expect("valid regex"),
        Regex::new(r"AKIA[0-9A-Z]{16}").expect("valid regex"),
        Regex::new(r"gh[pousr]_[A-Za-z0-9]{20,}").expect("valid regex"),
        Regex::new(r#"(?i)(api[_-]?key|secret|password)"?\s*[:=]\s*"?[A-Za-z0-9/+=_\-]{12,}"#)
            .expect("valid regex"),
    ]
});

fn redact_secrets(text: &str) -> String {
    let mut out = text.to_string();
    for re in SECRET_PATTERNS.iter() {
        if re.is_match(&out) {
            out = re.replace_all(&out, "<redacted-secret>").into_owned();
        }
    }
    out
}

fn redact_secrets_in_value(value: &mut serde_json::Value) {
    walk_strings_mut(value, &mut |s: &str| redact_secrets(s));
}

/// The stable cassette key for a request: SHA-256 of (model, system,
/// canonicalized messages, canonicalized tools) after stripping volatile
/// fields. See the module doc for what "canonicalized" strips and why.
pub fn cassette_key(
    model_name: &str,
    system: &str,
    messages: &[Message],
    tools: &[Tool],
) -> String {
    let mut messages_json: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null))
        .collect();
    for m in messages_json.iter_mut() {
        strip_volatile_keys(m);
        normalize_volatile_strings(m);
    }
    let mut tools_json: Vec<serde_json::Value> = tools
        .iter()
        .map(|t| serde_json::to_value(t).unwrap_or(serde_json::Value::Null))
        .collect();
    for t in tools_json.iter_mut() {
        strip_volatile_keys(t);
    }
    let mut system_value = serde_json::Value::String(system.to_string());
    normalize_volatile_strings(&mut system_value);

    let canonical = serde_json::json!({
        "model": model_name,
        "system": system_value,
        "messages": messages_json,
        "tools": tools_json,
    });
    let canonical_str = serde_json::to_string(&canonical).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(canonical_str.as_bytes());
    hex::encode(hasher.finalize())
}

fn redacted_message(message: &Message) -> Message {
    let mut value = serde_json::to_value(message).unwrap_or(serde_json::Value::Null);
    redact_secrets_in_value(&mut value);
    serde_json::from_value(value).unwrap_or_else(|_| message.clone())
}

fn redacted_messages(messages: &[Message]) -> Vec<Message> {
    messages.iter().map(redacted_message).collect()
}

fn request_record(
    model_config: &ModelConfig,
    system: &str,
    messages: &[Message],
    tools: &[Tool],
) -> CassetteRequestRecord {
    CassetteRequestRecord {
        model: model_config.model_name.clone(),
        system: redact_secrets(system),
        messages: redacted_messages(messages),
        tools: tools.to_vec(),
    }
}

fn append_line<T: Serialize>(path: &Path, line: &T) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let json = serde_json::to_string(line)
        .unwrap_or_else(|e| format!("{{\"cassette_serialize_error\":{:?}}}", e.to_string()));
    writeln!(file, "{json}")
}

/// Loads `<dir>/cassette.jsonl` into a key -> turn map, plus the provider
/// identity the recording carried (the FIRST line that has one wins; a
/// cassette shared by several provider tiers is documented in the module
/// doc's "Known limitations", and the tier that matters for a replay slice
/// is the one the run is driven at).
fn load_turns(path: &Path) -> Result<(HashMap<String, StoredTurn>, Option<CassetteProviderMeta>)> {
    if !path.exists() {
        return Ok((HashMap::new(), None));
    }
    let file = std::fs::File::open(path)
        .with_context(|| format!("opening cassette {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut out = HashMap::new();
    let mut meta: Option<CassetteProviderMeta> = None;
    for (line_no, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("reading {}:{}", path.display(), line_no + 1))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parsed: CassetteTurnLine = serde_json::from_str(trimmed)
            .with_context(|| format!("parsing {}:{}", path.display(), line_no + 1))?;
        if meta.is_none() {
            meta = parsed.provider;
        }
        out.insert(
            parsed.key,
            StoredTurn {
                message: parsed.response.message,
                usage: parsed.response.usage,
            },
        );
    }
    Ok((out, meta))
}

// ── The decorator ────────────────────────────────────────────────────────

enum CassetteProviderMode {
    Record {
        inner: Arc<dyn Provider>,
        /// The wrapped provider's identity, resolved once at wrap time (the
        /// same "resolve once at wrap time" idiom
        /// [`super::sovereign_guard::SovereignGuardProvider`] uses for
        /// locality) and stamped onto every recorded line.
        meta: CassetteProviderMeta,
        path: PathBuf,
    },
    Replay {
        records: HashMap<String, StoredTurn>,
        cost_tier: crate::session::CostTier,
        data_locality: crate::sovereignty::DataLocality,
        misses_path: PathBuf,
    },
}

/// Provider decorator implementing record/replay. See the module doc for the
/// full contract.
pub struct CassetteProvider {
    name: String,
    model_config: ModelConfig,
    mode: CassetteProviderMode,
}

impl CassetteProvider {
    /// Wrap an already fully-constructed (and, at the real choke point,
    /// already sovereignty-guarded) provider so every `stream()` call is
    /// ALSO appended to `<dir>/cassette.jsonl`. The call still really
    /// happens -- recording never changes live behavior for the caller.
    pub fn wrap_for_record(inner: Arc<dyn Provider>, dir: PathBuf) -> Result<Arc<dyn Provider>> {
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating cassette dir {}", dir.display()))?;
        let name = inner.get_name().to_string();
        let model_config = inner.get_model_config();
        let meta = CassetteProviderMeta::of(&inner);
        Ok(Arc::new(Self {
            name,
            model_config,
            mode: CassetteProviderMode::Record {
                inner,
                meta,
                path: dir.join(CASSETTE_FILE_NAME),
            },
        }))
    }

    /// Build a replay-only provider directly from `<dir>/cassette.jsonl`.
    /// NEVER constructs or calls a real provider -- replay needs no
    /// credentials, makes no network call, and spends nothing.
    pub fn for_replay(
        provider_name: String,
        model_config: ModelConfig,
        dir: PathBuf,
    ) -> Result<Arc<dyn Provider>> {
        let path = dir.join(CASSETTE_FILE_NAME);
        let (records, meta) = load_turns(&path)?;
        if records.is_empty() {
            anyhow::bail!(
                "cassette replay: {} has no recorded turns -- record one first with \
                 PERMAGENT_PROVIDER_CASSETTE=record:<dir> (never fabricated)",
                path.display()
            );
        }
        // Replayed, not guessed — see the module doc's "Known limitations".
        let cost_tier = meta
            .as_ref()
            .map(CassetteProviderMeta::cost_tier)
            .unwrap_or(crate::session::CostTier::PaidApi);
        let data_locality = meta
            .as_ref()
            .map(CassetteProviderMeta::data_locality)
            .unwrap_or(crate::sovereignty::DataLocality::Cloud);
        Ok(Arc::new(Self {
            name: provider_name,
            model_config,
            mode: CassetteProviderMode::Replay {
                records,
                cost_tier,
                data_locality,
                misses_path: dir.join(MISSES_FILE_NAME),
            },
        }))
    }
}

#[async_trait]
impl Provider for CassetteProvider {
    fn get_name(&self) -> &str {
        &self.name
    }

    fn get_model_config(&self) -> ModelConfig {
        self.model_config.clone()
    }

    /// Record mode forwards the wrapped provider's real billing tier
    /// faithfully (transparent decorator, same discipline
    /// `SovereignGuardProvider::cost_tier` documents). Replay mode reports
    /// the tier the RECORDING carried, falling back to the trait's
    /// fail-closed `PaidApi` only when the cassette does not name one --
    /// see the module doc's "Known limitations" for why guessing here
    /// refuses the call before it can ever reach the cassette.
    fn cost_tier(&self) -> crate::session::CostTier {
        match &self.mode {
            CassetteProviderMode::Record { inner, .. } => inner.cost_tier(),
            CassetteProviderMode::Replay { cost_tier, .. } => *cost_tier,
        }
    }

    fn data_locality(&self) -> crate::sovereignty::DataLocality {
        match &self.mode {
            CassetteProviderMode::Record { inner, .. } => inner.data_locality(),
            CassetteProviderMode::Replay { data_locality, .. } => *data_locality,
        }
    }

    fn retry_config(&self) -> RetryConfig {
        match &self.mode {
            CassetteProviderMode::Record { inner, .. } => inner.retry_config(),
            CassetteProviderMode::Replay { .. } => RetryConfig::default(),
        }
    }

    async fn stream(
        &self,
        model_config: &ModelConfig,
        session_id: &str,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        let key = cassette_key(&model_config.model_name, system, messages, tools);
        match &self.mode {
            CassetteProviderMode::Record { inner, meta, path } => {
                let stream = inner
                    .stream(model_config, session_id, system, messages, tools)
                    .await?;
                let (message, usage) = super::base::collect_stream(stream).await?;
                let line = CassetteTurnLine {
                    key: key.clone(),
                    provider: Some(meta.clone()),
                    request: request_record(model_config, system, messages, tools),
                    response: CassetteResponseRecord {
                        message: redacted_message(&message),
                        usage: usage.clone(),
                    },
                };
                if let Err(err) = append_line(path, &line) {
                    tracing::warn!("cassette: failed to record turn {key}: {err}");
                }
                Ok(stream_from_single_message(message, usage))
            }
            CassetteProviderMode::Replay {
                records,
                misses_path,
                ..
            } => {
                if let Some(stored) = records.get(&key) {
                    Ok(stream_from_single_message(
                        stored.message.clone(),
                        stored.usage.clone(),
                    ))
                } else {
                    let miss = CassetteMissLine {
                        key: key.clone(),
                        request: request_record(model_config, system, messages, tools),
                    };
                    if let Err(err) = append_line(misses_path, &miss) {
                        tracing::warn!("cassette: failed to record miss {key}: {err}");
                    }
                    Err(ProviderError::ExecutionError(format!(
                        "cassette replay miss: no recorded response for key {key} (model {}); \
                         unmatched request appended to {} -- a cassette never fabricates a response",
                        model_config.model_name,
                        misses_path.display(),
                    )))
                }
            }
        }
    }

    // ── Faithful delegation of everything else (record mode) ────────────────
    //
    // Same discipline as `SovereignGuardProvider`'s own delegation block: a
    // decorator that silently answers the trait default for a capability
    // question is a decorator that changes the behavior of the thing it
    // wraps. Without these, `record:<dir>` would tell the agent that a
    // Claude-Code-style provider does NOT manage its own context, that
    // Anthropic does NOT support cache control, and that no provider has a
    // permission routing seam — i.e. recording would change the live run it
    // claims to observe, and the cassette would be a recording of a
    // DIFFERENT pipeline than the one that runs without it.
    //
    // `stream_split` is deliberately NOT overridden: its trait default
    // flattens the split and funnels into `self.stream`, which is what makes
    // one cassette key cover a prompt-cache-splitting provider too. The cost
    // is that a `record:` run loses the wrapped provider's prompt cache for
    // the duration of the recording — accepted, because the alternative is
    // two different key spaces for the same logical turn. Replay mode
    // answers the trait defaults on purpose: there is no inner provider to
    // ask, and every one of these defaults is the fail-closed answer.
    //
    // `complete`/`complete_fast`/`generate_session_name` are likewise left as
    // trait defaults so they funnel through the recorded/replayed `stream`.

    async fn fetch_supported_models(&self) -> Result<Vec<String>, ProviderError> {
        match &self.mode {
            CassetteProviderMode::Record { inner, .. } => inner.fetch_supported_models().await,
            CassetteProviderMode::Replay { .. } => Ok(vec![]),
        }
    }

    async fn fetch_recommended_models(&self) -> Result<Vec<String>, ProviderError> {
        match &self.mode {
            CassetteProviderMode::Record { inner, .. } => inner.fetch_recommended_models().await,
            CassetteProviderMode::Replay { .. } => Ok(vec![]),
        }
    }

    fn skip_canonical_filtering(&self) -> bool {
        match &self.mode {
            CassetteProviderMode::Record { inner, .. } => inner.skip_canonical_filtering(),
            CassetteProviderMode::Replay { .. } => false,
        }
    }

    fn supports_embeddings(&self) -> bool {
        match &self.mode {
            CassetteProviderMode::Record { inner, .. } => inner.supports_embeddings(),
            CassetteProviderMode::Replay { .. } => false,
        }
    }

    /// Embeddings are NOT recorded (this lane records inference turns only).
    /// Record mode therefore delegates them untouched — a real call, exactly
    /// as without the cassette — and replay mode refuses, because serving an
    /// embedding it never recorded would be the fabrication this whole
    /// module exists to refuse.
    async fn create_embeddings(
        &self,
        session_id: &str,
        texts: Vec<String>,
    ) -> Result<Vec<Vec<f32>>, ProviderError> {
        match &self.mode {
            CassetteProviderMode::Record { inner, .. } => {
                inner.create_embeddings(session_id, texts).await
            }
            CassetteProviderMode::Replay { .. } => Err(ProviderError::ExecutionError(
                "cassette replay: embeddings are not recorded by a cassette and are never \
                 fabricated -- run this leg without PERMAGENT_PROVIDER_CASSETTE, or record \
                 the embedding path first"
                    .to_string(),
            )),
        }
    }

    fn manages_own_context(&self) -> bool {
        match &self.mode {
            CassetteProviderMode::Record { inner, .. } => inner.manages_own_context(),
            CassetteProviderMode::Replay { .. } => false,
        }
    }

    async fn supports_cache_control(&self) -> bool {
        match &self.mode {
            CassetteProviderMode::Record { inner, .. } => inner.supports_cache_control().await,
            CassetteProviderMode::Replay { .. } => false,
        }
    }

    async fn configure_oauth(&self) -> Result<(), ProviderError> {
        match &self.mode {
            CassetteProviderMode::Record { inner, .. } => inner.configure_oauth().await,
            CassetteProviderMode::Replay { .. } => Err(ProviderError::ExecutionError(
                "cassette replay has no credentials to configure".to_string(),
            )),
        }
    }

    async fn refresh_credentials(&self) -> Result<(), ProviderError> {
        match &self.mode {
            CassetteProviderMode::Record { inner, .. } => inner.refresh_credentials().await,
            CassetteProviderMode::Replay { .. } => Err(ProviderError::NotImplemented(
                "cassette replay has no credentials to refresh".to_string(),
            )),
        }
    }

    async fn update_mode(
        &self,
        session_id: &str,
        mode: crate::config::GooseMode,
    ) -> Result<(), ProviderError> {
        match &self.mode {
            CassetteProviderMode::Record { inner, .. } => inner.update_mode(session_id, mode).await,
            CassetteProviderMode::Replay { .. } => Ok(()),
        }
    }

    fn permission_routing(&self) -> super::base::PermissionRouting {
        match &self.mode {
            CassetteProviderMode::Record { inner, .. } => inner.permission_routing(),
            CassetteProviderMode::Replay { .. } => super::base::PermissionRouting::Noop,
        }
    }

    async fn handle_permission_confirmation(
        &self,
        request_id: &str,
        confirmation: &crate::permission::PermissionConfirmation,
    ) -> bool {
        match &self.mode {
            CassetteProviderMode::Record { inner, .. } => {
                inner
                    .handle_permission_confirmation(request_id, confirmation)
                    .await
            }
            CassetteProviderMode::Replay { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::{Message, MessageContent};
    use crate::providers::base::Usage;
    use chrono::Utc;
    use rmcp::model::{RawTextContent, Role, TextContent};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    fn text_message(role: Role, text: &str) -> Message {
        Message::new(
            role,
            Utc::now().timestamp(),
            vec![MessageContent::Text(TextContent {
                raw: RawTextContent {
                    text: text.to_string(),
                    meta: None,
                },
                annotations: None,
            })],
        )
    }

    fn user(text: &str) -> Message {
        text_message(Role::User, text)
    }

    #[test]
    fn key_ignores_message_id_and_created_but_not_text() {
        let mut a = user("fix the bug");
        a.id = Some("run-1-abc".to_string());
        a.created = 1_000;
        let mut b = user("fix the bug");
        b.id = Some("run-2-xyz".to_string());
        b.created = 2_000;

        assert_eq!(
            cassette_key("m", "sys", &[a], &[]),
            cassette_key("m", "sys", &[b], &[]),
            "id/created must not perturb the key"
        );

        let c = user("fix a DIFFERENT bug");
        assert_ne!(
            cassette_key("m", "sys", &[user("fix the bug")], &[]),
            cassette_key("m", "sys", &[c], &[]),
            "different text must change the key"
        );
    }

    #[test]
    fn key_ignores_embedded_timestamps_and_uuids_in_text() {
        let a =
            user("goal dispatched_at=2026-09-10T16:00:00Z id=3fa85f64-5717-4562-b3fc-2c963f66afa6");
        let b =
            user("goal dispatched_at=2026-09-10T16:05:32Z id=7c9e6679-7425-40de-944b-e07fc1f90ae7");
        assert_eq!(
            cassette_key("m", "sys", &[a], &[]),
            cassette_key("m", "sys", &[b], &[]),
            "embedded volatile timestamp/uuid text must normalize to the same key"
        );
    }

    #[test]
    fn key_folds_in_model_name_and_system() {
        let msgs = [user("hi")];
        assert_ne!(
            cassette_key("model-a", "sys", &msgs, &[]),
            cassette_key("model-b", "sys", &msgs, &[]),
        );
        assert_ne!(
            cassette_key("m", "sys-a", &msgs, &[]),
            cassette_key("m", "sys-b", &msgs, &[]),
        );
    }

    #[derive(Clone)]
    struct CountingProvider {
        model_config: ModelConfig,
        response: String,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Provider for CountingProvider {
        fn get_name(&self) -> &str {
            "counting-mock"
        }

        fn get_model_config(&self) -> ModelConfig {
            self.model_config.clone()
        }

        // Deliberately NOT the trait's fail-closed defaults, so the
        // round-trip test below can prove replay reports what was RECORDED
        // rather than what the trait would have guessed.
        fn cost_tier(&self) -> crate::session::CostTier {
            crate::session::CostTier::LocalFree
        }

        fn data_locality(&self) -> crate::sovereignty::DataLocality {
            crate::sovereignty::DataLocality::Local
        }

        async fn stream(
            &self,
            _model_config: &ModelConfig,
            _session_id: &str,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<MessageStream, ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let message = text_message(Role::Assistant, &self.response);
            let usage = ProviderUsage::new(self.model_config.model_name.clone(), Usage::default());
            Ok(stream_from_single_message(message, usage))
        }
    }

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cassette-test-{tag}-{}-{}",
            std::process::id(),
            uuid_like()
        ));
        dir
    }

    // Avoids pulling in the `uuid` crate just for test-scratch uniqueness.
    fn uuid_like() -> u64 {
        static COUNTER: Mutex<u64> = Mutex::new(0);
        let mut guard = COUNTER.lock().unwrap();
        *guard += 1;
        (Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64).wrapping_add(*guard)
    }

    #[tokio::test]
    async fn record_then_replay_round_trip_never_calls_the_inner_provider_again() {
        let dir = scratch_dir("roundtrip");
        let calls = Arc::new(AtomicUsize::new(0));
        let inner = Arc::new(CountingProvider {
            model_config: ModelConfig::new_or_fail("mock-model"),
            response: "the fix compiles".to_string(),
            calls: calls.clone(),
        });

        let cfg = inner.model_config.clone();
        let messages = [user("please fix it")];

        {
            let recording = CassetteProvider::wrap_for_record(inner, dir.clone()).unwrap();
            let (message, _) = recording
                .complete(&cfg, "sess-1", "sys", &messages, &[])
                .await
                .unwrap();
            assert!(
                matches!(&message.content[0], MessageContent::Text(t) if t.text == "the fix compiles")
            );
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "record calls the real inner once"
        );

        {
            let replaying =
                CassetteProvider::for_replay("mock".to_string(), cfg.clone(), dir.clone()).unwrap();
            let (message, _) = replaying
                .complete(&cfg, "sess-2", "sys", &messages, &[])
                .await
                .unwrap();
            assert!(
                matches!(&message.content[0], MessageContent::Text(t) if t.text == "the fix compiles")
            );
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "replay must never call a real provider"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn replay_miss_is_never_fabricated_and_is_written_to_misses_jsonl() {
        let dir = scratch_dir("miss");
        let calls = Arc::new(AtomicUsize::new(0));
        let inner = Arc::new(CountingProvider {
            model_config: ModelConfig::new_or_fail("mock-model"),
            response: "recorded answer".to_string(),
            calls: calls.clone(),
        });
        let cfg = inner.model_config.clone();

        {
            let recording = CassetteProvider::wrap_for_record(inner, dir.clone()).unwrap();
            recording
                .complete(&cfg, "sess-1", "sys", &[user("known request")], &[])
                .await
                .unwrap();
        }

        let replaying =
            CassetteProvider::for_replay("mock".to_string(), cfg.clone(), dir.clone()).unwrap();
        let result = replaying
            .complete(
                &cfg,
                "sess-2",
                "sys",
                &[user("a request never recorded")],
                &[],
            )
            .await;

        let err = result.expect_err("an unmatched request must fail, never fabricate");
        assert!(
            err.to_string().contains("cassette replay miss"),
            "got: {err}"
        );
        let misses_path = dir.join(MISSES_FILE_NAME);
        let misses = std::fs::read_to_string(&misses_path).unwrap();
        assert!(misses.contains("a request never recorded"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn replay_of_an_empty_cassette_dir_fails_construction_rather_than_serving_nothing() {
        let dir = scratch_dir("empty");
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = ModelConfig::new_or_fail("mock-model");
        // `expect_err` would need `Debug` on the Ok type, and `Arc<dyn Provider>`
        // has none — match instead.
        let err = match CassetteProvider::for_replay("mock".to_string(), cfg, dir.clone()) {
            Ok(_) => panic!("an empty/missing cassette must refuse to construct"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("no recorded turns"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn record_mode_strips_secret_shaped_text_before_it_reaches_disk() {
        let dir = scratch_dir("secrets");
        let calls = Arc::new(AtomicUsize::new(0));
        let inner = Arc::new(CountingProvider {
            model_config: ModelConfig::new_or_fail("mock-model"),
            response: "Bearer sk-1234567890abcdef1234567890".to_string(),
            calls,
        });
        let cfg = inner.model_config.clone();

        {
            let recording = CassetteProvider::wrap_for_record(inner, dir.clone()).unwrap();
            recording
                .complete(&cfg, "sess-1", "sys", &[user("hi")], &[])
                .await
                .unwrap();
        }

        let on_disk = std::fs::read_to_string(dir.join(CASSETTE_FILE_NAME)).unwrap();
        assert!(
            !on_disk.contains("sk-1234567890abcdef1234567890"),
            "secret-shaped text must never reach disk: {on_disk}"
        );
        assert!(on_disk.contains("<redacted-secret>"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The accounted-completion path asks the provider for its cost tier
    /// BEFORE the call (`agents/reply_parts.rs`'s
    /// `reserve_provider_invocation_for_model`), and a `PaidApi` answer
    /// demands a real dollar reservation — which needs a context limit and a
    /// durable task. A replay provider that guessed `PaidApi` for a
    /// recording that was really a free local call would therefore be
    /// REFUSED before it ever reached the cassette, and the miss would look
    /// like an accounting bug rather than a wiring one. Replay must report
    /// what was recorded.
    #[tokio::test]
    async fn replay_reports_the_recorded_cost_tier_and_locality_not_a_guess() {
        let dir = scratch_dir("tier");
        let inner = Arc::new(CountingProvider {
            model_config: ModelConfig::new_or_fail("mock-model"),
            response: "recorded answer".to_string(),
            calls: Arc::new(AtomicUsize::new(0)),
        });
        let cfg = inner.model_config.clone();

        {
            let recording = CassetteProvider::wrap_for_record(inner, dir.clone()).unwrap();
            assert_eq!(
                recording.cost_tier(),
                crate::session::CostTier::LocalFree,
                "record mode forwards the wrapped provider's real tier"
            );
            recording
                .complete(&cfg, "sess-1", "sys", &[user("hi")], &[])
                .await
                .unwrap();
        }

        let replaying = CassetteProvider::for_replay("mock".to_string(), cfg, dir.clone()).unwrap();
        assert_eq!(
            replaying.cost_tier(),
            crate::session::CostTier::LocalFree,
            "replay must report the RECORDED tier, not the trait's PaidApi default"
        );
        assert_eq!(
            replaying.data_locality(),
            crate::sovereignty::DataLocality::Local,
            "replay must report the RECORDED locality"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The fail-closed half of the branch above: a cassette line that names
    /// no provider (an older or hand-written file) must replay as the
    /// trait's chargeable cloud default, never as "free" by omission.
    #[tokio::test]
    async fn replay_of_a_cassette_without_provider_metadata_fails_closed_to_paid_cloud() {
        let dir = scratch_dir("no-meta");
        std::fs::create_dir_all(&dir).unwrap();
        // Built from the real record types with `provider: None` rather than
        // hand-typed JSON, so this fixture cannot drift out of sync with the
        // on-disk shape the loader actually parses.
        let line = CassetteTurnLine {
            key: "deadbeef".to_string(),
            provider: None,
            request: CassetteRequestRecord {
                model: "mock-model".to_string(),
                system: "sys".to_string(),
                messages: vec![],
                tools: vec![],
            },
            response: CassetteResponseRecord {
                message: Message::assistant().with_text("hi"),
                usage: ProviderUsage::new("mock-model".to_string(), Usage::default()),
            },
        };
        std::fs::write(
            dir.join(CASSETTE_FILE_NAME),
            format!("{}\n", serde_json::to_string(&line).unwrap()),
        )
        .unwrap();

        let replaying = CassetteProvider::for_replay(
            "mock".to_string(),
            ModelConfig::new_or_fail("mock-model"),
            dir.clone(),
        )
        .expect("a cassette with no provider block must still load");
        assert_eq!(replaying.cost_tier(), crate::session::CostTier::PaidApi);
        assert_eq!(
            replaying.data_locality(),
            crate::sovereignty::DataLocality::Cloud
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mode_parses_record_and_replay_and_rejects_garbage() {
        assert_eq!(
            CassetteMode::parse("record:/tmp/x").unwrap(),
            CassetteMode::Record(PathBuf::from("/tmp/x"))
        );
        assert_eq!(
            CassetteMode::parse("replay:/tmp/y").unwrap(),
            CassetteMode::Replay(PathBuf::from("/tmp/y"))
        );
        assert!(CassetteMode::parse("bogus").is_err());
        assert!(CassetteMode::parse("record:").is_err());
        assert!(CassetteMode::parse("rewind:/tmp/z").is_err());
    }
}
