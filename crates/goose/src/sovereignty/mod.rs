//! Sovereignty — the data-boundary enforcement layer.
//!
//! Permagent's sovereignty guarantee: when a context is marked *sovereign*,
//! inference for that context is constrained to **local models only**, and any
//! attempt to use a **cloud** provider is **blocked** (fail-closed) — sensitive
//! data provably never leaves this machine for cloud inference. Every cloud
//! inference call (blocked or allowed) is recorded in an append-only local
//! egress audit log. An audit-write failure is logged loudly with full
//! context; with [`SOVEREIGN_STRICT_AUDIT_KEY`] enabled it additionally fails
//! the allowed cloud call itself (fail-closed audit: no unlogged egress).
//!
//! Enforcement is centralized in [`crate::providers::sovereign_guard`], the
//! decorator that wraps *every* provider minted by the factory
//! (`crate::providers::create*`). That is the single choke point: all inference
//! egress in the daemon flows through `Provider::stream` (with `complete` /
//! `complete_fast` / `generate_session_name` funneling into it), so guarding
//! `stream` there guards the whole surface. This module owns the *policy* state
//! (what is sovereign) and the *audit* store (what has left, and when).
//!
//! The local-vs-cloud distinction is [`DataLocality`], surfaced by
//! [`crate::providers::base::Provider::data_locality`]. It is **fail-closed**:
//! any provider that does not *prove* it runs on this machine is treated as
//! cloud egress.

use std::collections::HashSet;
use std::sync::atomic::{AtomicI8, Ordering};
use std::sync::{LazyLock, RwLock};

use sha2::{Digest, Sha256};
use sqlx::{Pool, Row, Sqlite};

use crate::config::{Config, ConfigError};
use crate::conversation::message::Message;

/// Config key for the global sovereign-mode switch. Because `Config::get_param`
/// checks the uppercased env var first, `SOVEREIGN_MODE=true` also works (handy
/// as a kill-switch / for tests).
pub const SOVEREIGN_MODE_KEY: &str = "sovereign_mode";

/// Config key: when true, the egress audit log captures the full prompt text
/// alongside the hash. Default false — only a SHA-256 content hash is stored.
pub const SOVEREIGN_CAPTURE_PROMPTS_KEY: &str = "sovereign_capture_prompts";

/// Config key: when true, an egress-audit **write failure** for an *allowed*
/// cloud call fails the call itself (fail-closed audit — for compliance
/// postures that require no-unlogged-egress). Default false: the failure is
/// logged loudly and the call proceeds. Blocked calls are unaffected either
/// way — they already fail closed with their own refusal. As with the other
/// sovereignty keys, `SOVEREIGN_STRICT_AUDIT=true` also works via env.
pub const SOVEREIGN_STRICT_AUDIT_KEY: &str = "sovereign_strict_audit";

/// Prefix on the block error message so callers, the UI, and tests can
/// recognize a sovereign refusal without a dedicated `ProviderError` variant.
pub const SOVEREIGN_BLOCK_PREFIX: &str = "[sovereign]";

/// Self-knowledge: the sovereignty boundary as a guardrail the agent is
/// *subject to* and must be able to describe — what sovereign mode does, when
/// it engages, and what to do when a cloud call is refused. Aggregated by
/// `crate::agents::self_knowledge::GUARD_DESCRIPTORS`.
///
/// Always-visible (NOT render-gated on the mode), matching the other guards:
/// this machinery is standing policy, not a flag-gated experiment — the egress
/// audit records every cloud call even with the mode off, and sovereignty is
/// per-*context* (`is_context_sovereign` = global mode OR a session mark), so
/// there is no single "off" bit to gate on; gating on the global toggle would
/// hide exactly the knowledge needed to explain a session-sovereign
/// `[sovereign]` refusal, or to offer the boundary when the user asks for it.
/// Static (no live "currently on/off" line): the guard render path is
/// editorial by design, and the honest live truth is per-context, which the
/// brief builder cannot know — a global-only status line would over-claim in a
/// sovereign session. The live signal is the refusal itself, which carries
/// [`SOVEREIGN_BLOCK_PREFIX`] and reaches the agent exactly when it matters.
pub const SELF_KNOWLEDGE_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "sovereignty_guard",
        display_name: "Sovereignty guard",
        category: crate::agents::self_knowledge::FeatureCategory::Guard,
        what_it_does:
            "A fail-closed data boundary at the provider layer. When sovereign mode is on — the Sovereignty section in Settings, or SOVEREIGN_MODE=true in config — or the current session is individually marked sovereign, every cloud inference call is refused before any data leaves this machine, and only local models (the built-in local-inference provider, or Ollama on localhost) may run; the mesh side-path is refused the same way. Independently of the toggle, every cloud call — blocked or allowed — is recorded in an append-only local egress audit log the user can read in Settings, and a strict-audit option (SOVEREIGN_STRICT_AUDIT) refuses an allowed cloud call whose audit write failed rather than let egress go unlogged",
        why_it_matters:
            "A refusal starting with [sovereign] is this boundary working, not an outage: tell the user plainly that this context is sovereign so the request stayed on this machine, and offer the local routes — a local model through Ollama or the built-in local-inference provider — or the Settings toggle if they explicitly choose to allow cloud egress again. Never try to work around the block by re-sending the same content through another provider, session, or worker; the boundary is enforced in code at the single choke point every provider call flows through",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[],
    };

// ── Local vs cloud identity ─────────────────────────────────────────────────

/// Where a model's inference physically runs — the load-bearing local-vs-cloud
/// distinction. **Fail-closed:** anything not *provably* local is [`Cloud`].
///
/// [`Cloud`]: DataLocality::Cloud
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataLocality {
    /// Runs on this machine (in-process llama.cpp, or a loopback Ollama). Data
    /// stays local; using it is never an egress.
    Local,
    /// Runs off this machine (a third-party cloud API, a CLI that ships the
    /// prompt to a vendor, or a non-loopback host). Using it egresses the
    /// prompt.
    Cloud,
}

impl DataLocality {
    pub fn is_local(self) -> bool {
        matches!(self, DataLocality::Local)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            DataLocality::Local => "local",
            DataLocality::Cloud => "cloud",
        }
    }
}

// ── Global sovereign mode (cached) ──────────────────────────────────────────

// -1 = not yet loaded, 0 = off, 1 = on. Cached so the provider hot path never
// re-parses config.yaml per inference call (`get_param` is not cached).
static GLOBAL_MODE_CACHE: AtomicI8 = AtomicI8::new(-1);

/// Is process-wide sovereign mode on? Cheap (cached atomic after first read).
/// When on, *all* cloud inference in the process is blocked and audited.
pub fn global_sovereign_mode() -> bool {
    match GLOBAL_MODE_CACHE.load(Ordering::Relaxed) {
        0 => false,
        1 => true,
        _ => {
            let on = Config::global()
                .get_param::<bool>(SOVEREIGN_MODE_KEY)
                .unwrap_or(false);
            GLOBAL_MODE_CACHE.store(i8::from(on), Ordering::Relaxed);
            on
        }
    }
}

/// Set (and persist) process-wide sovereign mode, updating the cache.
pub fn set_global_sovereign_mode(on: bool) -> Result<(), ConfigError> {
    Config::global().set_param(SOVEREIGN_MODE_KEY, on)?;
    GLOBAL_MODE_CACHE.store(i8::from(on), Ordering::Relaxed);
    Ok(())
}

/// Invalidate the cached global-mode value so the next read reloads from config
/// (used after an external config change, and by tests).
pub fn invalidate_global_mode_cache() {
    GLOBAL_MODE_CACHE.store(-1, Ordering::Relaxed);
}

// ── Session sovereignty registry ────────────────────────────────────────────

// Sessions explicitly marked sovereign (e.g. because their project is
// sovereign) even when global mode is off. Populated at session/agent setup;
// read in the provider hot path by `session_id`.
static SOVEREIGN_SESSIONS: LazyLock<RwLock<HashSet<String>>> =
    LazyLock::new(|| RwLock::new(HashSet::new()));

/// Mark a session as sovereign (local-only). Idempotent.
pub fn mark_session_sovereign(session_id: &str) {
    if let Ok(mut set) = SOVEREIGN_SESSIONS.write() {
        set.insert(session_id.to_string());
    }
}

/// Clear a session's sovereign mark. Idempotent.
pub fn unmark_session_sovereign(session_id: &str) {
    if let Ok(mut set) = SOVEREIGN_SESSIONS.write() {
        set.remove(session_id);
    }
}

/// Is this specific session marked sovereign?
pub fn is_session_sovereign(session_id: &str) -> bool {
    SOVEREIGN_SESSIONS
        .read()
        .map(|s| s.contains(session_id))
        .unwrap_or(false)
}

/// The load-bearing predicate: is inference for this context constrained to
/// local models? True if global sovereign mode is on **or** the session is
/// marked sovereign.
pub fn is_context_sovereign(session_id: &str) -> bool {
    global_sovereign_mode() || is_session_sovereign(session_id)
}

/// The error message returned when a cloud provider is blocked for a sovereign
/// context. Prefixed with [`SOVEREIGN_BLOCK_PREFIX`] for recognition.
pub fn sovereign_block_message(provider: &str, model: &str) -> String {
    format!(
        "{SOVEREIGN_BLOCK_PREFIX} cloud provider '{provider}' (model '{model}') is blocked: \
         this context is sovereign (local-only). No conversation data will leave this machine \
         for cloud inference. Switch to a local model (Ollama on localhost, or the built-in \
         local-inference provider) to continue."
    )
}

// ── Strict (fail-closed) audit mode ─────────────────────────────────────────

/// Is strict audit mode on? Read only on the audit-write *failure* path (cold
/// — failures are exceptional), so an uncached `get_param` is fine here,
/// unlike the per-call `global_sovereign_mode` cache above.
pub fn strict_audit_enabled() -> bool {
    Config::global()
        .get_param::<bool>(SOVEREIGN_STRICT_AUDIT_KEY)
        .unwrap_or(false)
}

/// Pure strict-audit policy: must a failed audit write fail the cloud call
/// itself? Only when strict mode is on AND the call was *allowed* — an
/// unlogged allowed egress is exactly what strict mode exists to prevent,
/// while a blocked call is already fail-closed (no data left the machine)
/// and carries its own, more informative refusal.
pub fn audit_failure_fails_call(strict_audit: bool, blocked: bool) -> bool {
    strict_audit && !blocked
}

/// The error message when strict audit mode refuses an unlogged cloud call.
/// Carries [`SOVEREIGN_BLOCK_PREFIX`] so the UI and callers recognize it as a
/// sovereignty refusal. Honest by construction: the guard writes the audit row
/// *before* delegating to the inner provider, so when this fires no bytes have
/// left the machine.
pub fn strict_audit_block_message(provider: &str, model: &str) -> String {
    format!(
        "{SOVEREIGN_BLOCK_PREFIX} cloud call to provider '{provider}' (model '{model}') refused: \
         the egress audit write failed and `{SOVEREIGN_STRICT_AUDIT_KEY}` is enabled \
         (no-unlogged-egress). The request was not sent. Restore the audit store \
         (spectral db) or disable strict audit to proceed."
    )
}

// ── Egress audit log ────────────────────────────────────────────────────────

/// What kind of content left (or was blocked from leaving) the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EgressKind {
    /// A prompt-completion / streaming inference call.
    Inference,
    /// An embedding request.
    Embedding,
    /// A product-analytics telemetry POST (e.g. the session-started event).
    Telemetry,
    /// A crash-report upload (no ambient path exists today; a future one must
    /// flow through the same boundary — see [`guard_outbound_telemetry`]).
    CrashReport,
}

impl EgressKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EgressKind::Inference => "inference",
            EgressKind::Embedding => "embedding",
            EgressKind::Telemetry => "telemetry",
            EgressKind::CrashReport => "crash_report",
        }
    }
}

/// A single cloud-egress event to record. Built by the guard at the choke
/// point.
#[derive(Debug, Clone)]
pub struct EgressRecord {
    pub provider: String,
    pub model: String,
    pub session_id: String,
    pub project_id: Option<String>,
    pub kind: EgressKind,
    /// True if this egress was *blocked* (sovereign context); false if allowed.
    pub blocked: bool,
    /// SHA-256 hex of the prompt content (always present).
    pub content_hash: String,
    /// Full prompt text, only when `sovereign_capture_prompts` is enabled.
    pub prompt: Option<String>,
}

/// A row read back from the egress audit log (wire-friendly, camelCase JSON).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EgressLogEntry {
    pub id: String,
    pub ts: String,
    pub provider: String,
    pub model: String,
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    pub kind: String,
    pub blocked: bool,
    pub content_hash: String,
    pub prompt: Option<String>,
}

/// Is full-prompt capture enabled? (Default false — hash only.)
pub fn capture_prompts_enabled() -> bool {
    Config::global()
        .get_param::<bool>(SOVEREIGN_CAPTURE_PROMPTS_KEY)
        .unwrap_or(false)
}

/// Render a prompt (system + messages) to a single canonical string for hashing
/// and (optionally) capture.
pub fn render_prompt(system: &str, messages: &[Message]) -> String {
    let mut out = String::with_capacity(system.len() + 32);
    out.push_str("system:\n");
    out.push_str(system);
    for m in messages {
        out.push_str("\n\n");
        // `==` (not `match`) so we never move `role` out of the `&Message`.
        out.push_str(if m.role == rmcp::model::Role::User {
            "user:\n"
        } else {
            "assistant:\n"
        });
        out.push_str(&m.as_concat_text());
    }
    out
}

/// SHA-256 hex of a system+messages prompt.
pub fn hash_prompt(system: &str, messages: &[Message]) -> String {
    hash_str(&render_prompt(system, messages))
}

/// SHA-256 hex of a batch of embedding inputs.
pub fn hash_texts(texts: &[String]) -> String {
    hash_str(&texts.join("\n"))
}

fn hash_str(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

/// Insert one egress event (pure — takes an explicit pool; used by the
/// convenience wrapper and by tests).
pub async fn record_egress_row(pool: &Pool<Sqlite>, rec: &EgressRecord) -> anyhow::Result<()> {
    let id = uuid::Uuid::now_v7().to_string();
    let ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    sqlx::query(
        "INSERT INTO egress_audit \
         (id, ts, provider, model, session_id, project_id, kind, blocked, content_hash, prompt) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(ts)
    .bind(rec.provider.as_str())
    .bind(rec.model.as_str())
    .bind(rec.session_id.as_str())
    .bind(rec.project_id.as_deref())
    .bind(rec.kind.as_str())
    .bind(i64::from(rec.blocked))
    .bind(rec.content_hash.as_str())
    .bind(rec.prompt.as_deref())
    .execute(pool)
    .await?;
    Ok(())
}

/// Read the most recent egress events, newest first (pure — explicit pool).
pub async fn recent_egress_rows(
    pool: &Pool<Sqlite>,
    limit: i64,
) -> anyhow::Result<Vec<EgressLogEntry>> {
    let rows = sqlx::query(
        "SELECT id, ts, provider, model, session_id, project_id, kind, blocked, content_hash, prompt \
         FROM egress_audit ORDER BY ts DESC, id DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| EgressLogEntry {
            id: r.get("id"),
            ts: r.get("ts"),
            provider: r.get("provider"),
            model: r.get("model"),
            session_id: r.get("session_id"),
            project_id: r.get("project_id"),
            kind: r.get("kind"),
            blocked: r.get::<i64, _>("blocked") != 0,
            content_hash: r.get("content_hash"),
            prompt: r.get("prompt"),
        })
        .collect())
}

/// Record a cloud-egress event to the always-on audit log. On write failure
/// (row insert failed, or no db pool at all) the full egress context is logged
/// at `error` under target "sovereignty" — an unlogged cloud call is a lying
/// audit, and it must be loud in the daemon log — and the error is returned so
/// the caller can decide whether it fails the call (the guard consults
/// [`audit_failure_fails_call`] with [`strict_audit_enabled`]; default mode
/// proceeds). Obtains the shared spectral pool via the global session manager,
/// so it is callable from anywhere (including the provider layer).
pub async fn record_egress(rec: EgressRecord) -> anyhow::Result<()> {
    let result = match crate::session::SessionManager::instance()
        .pool_clone()
        .await
    {
        Ok(pool) => record_egress_row(&pool, &rec).await,
        Err(e) => Err(e.context("egress audit unavailable (no db pool)")),
    };
    if let Err(e) = &result {
        // `{:#}` keeps the whole context chain on one log line.
        let error_chain = format!("{e:#}");
        tracing::error!(
            target: "sovereignty",
            provider = %rec.provider,
            model = %rec.model,
            session_id = %rec.session_id,
            kind = rec.kind.as_str(),
            blocked = rec.blocked,
            error = %error_chain,
            "failed to write egress audit row — this cloud call is NOT in the egress audit log"
        );
    }
    result
}

/// Read recent egress events via the shared spectral pool.
pub async fn recent_egress(limit: i64) -> anyhow::Result<Vec<EgressLogEntry>> {
    let pool = crate::session::SessionManager::instance()
        .pool_clone()
        .await?;
    recent_egress_rows(&pool, limit).await
}

// ── Non-inference outbound egress (telemetry / crash-report uploads) ─────────

/// Pure policy for a non-inference outbound POST (telemetry or a crash-report
/// upload) under the sovereign boundary (#327). Given whether the process is in
/// sovereign mode, decide whether the caller MAY send, and build the audit
/// record to write either way. Under sovereign mode the egress is
/// **hard-suppressed** — `allowed = false` and `record.blocked = true` — so
/// telemetry and (any future) crash-report upload fail closed exactly like
/// cloud inference. Split out from [`guard_outbound_telemetry`] so the decision
/// is unit-testable without touching process-global state.
///
/// `destination` is the endpoint URL (recorded as the `provider`), `event` a
/// short label (recorded as the `model`) — both non-sensitive; the audit hashes
/// the destination rather than any payload, so no user content is stored.
pub fn plan_telemetry_egress(
    kind: EgressKind,
    destination: &str,
    event: &str,
    sovereign: bool,
) -> (bool, EgressRecord) {
    let rec = EgressRecord {
        provider: destination.to_string(),
        model: event.to_string(),
        session_id: "-".to_string(),
        project_id: None,
        kind,
        blocked: sovereign,
        content_hash: hash_str(destination),
        prompt: None,
    };
    (!sovereign, rec)
}

/// Guard an outbound telemetry / crash-report POST under the global sovereign
/// boundary and record it to the append-only egress audit log (#327).
///
/// Returns `true` if the caller may send (not sovereign), `false` if it must
/// **hard-suppress** the POST (sovereign mode on). Either way an audit row is
/// written (`blocked = sovereign`), bringing telemetry egress under the same
/// no-unlogged-egress boundary as inference. The suppression decision is
/// independent of whether the audit write succeeds — a failed audit write is
/// logged loudly by [`record_egress`] but never causes a suppressed POST to be
/// sent, and never fails an allowed telemetry POST (unlike strict-audit for
/// inference, telemetry is best-effort and must not break the app).
pub async fn guard_outbound_telemetry(kind: EgressKind, destination: &str, event: &str) -> bool {
    let sovereign = global_sovereign_mode();
    let (allowed, rec) = plan_telemetry_egress(kind, destination, event, sovereign);
    // Best-effort audit: `record_egress` already logs failures loudly. We do not
    // propagate the error — telemetry/crash egress must never crash the caller.
    let _ = record_egress(rec).await;
    allowed
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> Pool<Sqlite> {
        // Mirror the proven in-memory test-pool idiom (see `activity_journal`
        // tests): `init_spectral_db` builds the full schema, which now includes
        // the `egress_audit` table + its append-only triggers.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite pool");
        crate::session::spectral_schema::init_spectral_db(&pool)
            .await
            .expect("spectral schema init");
        pool
    }

    #[test]
    fn data_locality_fail_closed_semantics() {
        assert!(DataLocality::Local.is_local());
        assert!(!DataLocality::Cloud.is_local());
        assert_eq!(DataLocality::Local.as_str(), "local");
        assert_eq!(DataLocality::Cloud.as_str(), "cloud");
    }

    #[test]
    fn session_registry_marks_and_clears() {
        let sid = "sess-registry-unit-test";
        assert!(!is_session_sovereign(sid));
        mark_session_sovereign(sid);
        assert!(is_session_sovereign(sid));
        assert!(is_context_sovereign(sid));
        unmark_session_sovereign(sid);
        assert!(!is_session_sovereign(sid));
    }

    #[test]
    fn content_hash_is_stable_and_sensitive() {
        let a = hash_texts(&["hello".to_string()]);
        let b = hash_texts(&["hello".to_string()]);
        let c = hash_texts(&["world".to_string()]);
        assert_eq!(a, b, "same input hashes identically");
        assert_ne!(a, c, "different input hashes differently");
        assert_eq!(a.len(), 64, "sha256 hex is 64 chars");
    }

    /// The self-knowledge descriptor narrates both halves of the guarantee
    /// (fail-closed local-only inference + always-on egress audit) and teaches
    /// recognition of the REAL refusal prefix — asserted against the constant
    /// itself so the prose cannot drift from the wire format.
    #[test]
    fn self_knowledge_descriptor_narrates_the_boundary() {
        use crate::agents::self_knowledge::{FeatureCategory, StateSource};
        let d = SELF_KNOWLEDGE_FEATURE;
        assert_eq!(d.id, "sovereignty_guard");
        assert_eq!(d.category, FeatureCategory::Guard);
        assert_eq!(
            d.state_source,
            StateSource::Static,
            "guards render editorially; per-context truth cannot be claimed globally"
        );
        assert!(
            d.why_it_matters.contains(SOVEREIGN_BLOCK_PREFIX),
            "the descriptor must teach the agent to recognize a {SOVEREIGN_BLOCK_PREFIX} refusal"
        );
        assert!(
            d.what_it_does.contains("fail-closed") && d.what_it_does.contains("refused"),
            "must narrate fail-closed local-only inference"
        );
        assert!(
            d.what_it_does.contains("egress audit"),
            "must narrate the always-on egress audit"
        );
        assert!(
            d.why_it_matters.contains("Never try to work around"),
            "must instruct the agent to never circumvent the boundary"
        );
    }

    #[test]
    fn block_message_carries_prefix_and_names() {
        let msg = sovereign_block_message("anthropic", "claude-x");
        assert!(msg.starts_with(SOVEREIGN_BLOCK_PREFIX));
        assert!(msg.contains("anthropic"));
        assert!(msg.contains("claude-x"));
    }

    #[test]
    fn strict_audit_message_carries_prefix_names_and_knob() {
        let msg = strict_audit_block_message("openai", "gpt-x");
        assert!(
            msg.starts_with(SOVEREIGN_BLOCK_PREFIX),
            "strict-audit refusal must be recognizable as a sovereignty refusal"
        );
        assert!(msg.contains("openai"));
        assert!(msg.contains("gpt-x"));
        assert!(
            msg.contains(SOVEREIGN_STRICT_AUDIT_KEY),
            "message must name the knob so the user can find/disable it"
        );
    }

    /// The strict-audit policy as pure logic (the guard consults exactly this
    /// with `strict_audit_enabled()` when `record_egress` returns Err). The
    /// end-to-end failure can't be simulated through the guard here because
    /// `record_egress` rides the process-global SessionManager pool, which a
    /// unit test cannot break without racing every other test on it — the
    /// closed-pool test below proves the write-failure path itself errors.
    #[test]
    fn audit_failure_policy_fails_only_strict_allowed_calls() {
        // Strict ON + allowed call: the one case that must fail the call.
        assert!(audit_failure_fails_call(true, false));
        // Strict ON + blocked call: blocked behavior is unchanged (already
        // fail-closed with its own refusal).
        assert!(!audit_failure_fails_call(true, true));
        // Strict OFF: never fails the call, allowed or blocked.
        assert!(!audit_failure_fails_call(false, false));
        assert!(!audit_failure_fails_call(false, true));
    }

    /// A closed pool is the cleanest simulation of "the audit store is gone"
    /// (daemon db wedged/unwritable): the insert must surface an Err — which
    /// the `record_egress` wrapper logs with full context and hands to the
    /// guard — not silently vanish.
    #[tokio::test]
    async fn egress_write_failure_surfaces_an_error() {
        let pool = mem_pool().await;
        pool.close().await;
        let rec = EgressRecord {
            provider: "anthropic".to_string(),
            model: "claude-x".to_string(),
            session_id: "s-closed-pool".to_string(),
            project_id: None,
            kind: EgressKind::Inference,
            blocked: false,
            content_hash: "deadbeef".to_string(),
            prompt: None,
        };
        let result = record_egress_row(&pool, &rec).await;
        assert!(
            result.is_err(),
            "a write against a closed pool must error, not silently succeed"
        );
    }

    #[tokio::test]
    async fn egress_row_roundtrip_and_append_only() {
        let pool = mem_pool().await;
        let rec = EgressRecord {
            provider: "anthropic".to_string(),
            model: "claude-x".to_string(),
            session_id: "s1".to_string(),
            project_id: Some("p1".to_string()),
            kind: EgressKind::Inference,
            blocked: false,
            content_hash: "abc123".to_string(),
            prompt: None,
        };
        record_egress_row(&pool, &rec).await.unwrap();

        let rows = recent_egress_rows(&pool, 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].provider, "anthropic");
        assert_eq!(rows[0].model, "claude-x");
        assert_eq!(rows[0].kind, "inference");
        assert!(!rows[0].blocked);
        assert_eq!(rows[0].content_hash, "abc123");

        // Append-only: the DB itself must reject UPDATE and DELETE so the audit
        // cannot be quietly rewritten (a deletable log is a lying log).
        let upd = sqlx::query("UPDATE egress_audit SET provider = 'x'")
            .execute(&pool)
            .await;
        assert!(upd.is_err(), "egress_audit must reject UPDATE");
        let del = sqlx::query("DELETE FROM egress_audit").execute(&pool).await;
        assert!(del.is_err(), "egress_audit must reject DELETE");
    }

    #[test]
    fn telemetry_and_crash_egress_kinds_render_stable_strings() {
        assert_eq!(EgressKind::Telemetry.as_str(), "telemetry");
        assert_eq!(EgressKind::CrashReport.as_str(), "crash_report");
    }

    /// The load-bearing #327 guarantee at the policy layer: under sovereign mode
    /// a would-be crash-report upload is HARD-SUPPRESSED and recorded as blocked;
    /// with the mode off the same egress is allowed and recorded as not blocked.
    /// Uses the pure planner + an in-memory audit pool so it is race-free (no
    /// process-global sovereign flag to mutate).
    #[tokio::test]
    async fn sovereign_mode_suppresses_and_audits_a_would_be_upload() {
        let pool = mem_pool().await;

        // Sovereign context: a crash-report upload must be suppressed + audited.
        let (allowed, rec) = plan_telemetry_egress(
            EgressKind::CrashReport,
            "https://glitchtip.example/api/store/",
            "crash_report",
            /* sovereign */ true,
        );
        assert!(!allowed, "sovereign mode must hard-suppress the upload");
        assert!(
            rec.blocked,
            "the suppressed upload must be recorded blocked"
        );
        assert!(rec.prompt.is_none(), "no payload/user content is stored");
        record_egress_row(&pool, &rec).await.unwrap();

        // Non-sovereign context: a telemetry POST is allowed + audited (allowed).
        let (allowed2, rec2) = plan_telemetry_egress(
            EgressKind::Telemetry,
            "https://us.i.posthog.com/capture/",
            "session_started",
            /* sovereign */ false,
        );
        assert!(allowed2, "with sovereign mode off the POST is allowed");
        assert!(!rec2.blocked, "an allowed egress is recorded not-blocked");
        record_egress_row(&pool, &rec2).await.unwrap();

        // Both attempts are in the append-only audit log — no unlogged egress.
        let rows = recent_egress_rows(&pool, 10).await.unwrap();
        assert_eq!(rows.len(), 2, "both egress attempts were audited");
        let crash = rows.iter().find(|r| r.kind == "crash_report").unwrap();
        assert!(crash.blocked, "the sovereign-suppressed upload is blocked");
        let telem = rows.iter().find(|r| r.kind == "telemetry").unwrap();
        assert!(!telem.blocked, "the allowed telemetry POST is not blocked");
    }
}
