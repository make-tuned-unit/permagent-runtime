//! W7 — the native driver that hosts the tick loop (the #360 wiring).
//!
//! This is the consumer the module was built for: a plain Rust interval loop
//! (per the W6 ruling — NOT a scheduler recipe, which would spend agent tokens
//! per run and defeat the zero-token Tier 0 gate). It does exactly three jobs:
//!
//!   1. OBSERVE — subscribe to the global event bus (the same seam the
//!      `ActivityIngester` uses) and feed every activity event through the
//!      in-memory [`CommandCounter`]; track the live `last_activity_at` /
//!      `last_engagement_at` timestamps the gate needs.
//!   2. TICK — on a fixed interval, assemble [`GateInputs`] and call
//!      [`run_initiative_tick`]. The gate makes idle ticks free.
//!   3. PERSIST — own the two cross-restart cursors (`prev_tick_activity_at`,
//!      `last_proposal_at`) as Config params, per the W2 contract ("the tick
//!      owns loading/storing the cursors as Config flags"). The counter itself
//!      is deliberately in-memory: a cold start re-accumulates (W3 contract).
//!
//! GATING — explicit config, default OFF. A proactive-origination organ must be
//! turned on deliberately (`initiative_enabled: true`), mirroring the posture
//! of the other autonomous workers (orchestrator, steward). The off state is
//! logged loudly at startup so it is never silently absent.
//!
//! A threshold-crossed pattern is HELD across skipped ticks (engaged user,
//! cooldown, no fresh activity) and consumed by the first tick that emits or
//! prunes it — the counter fires at the moment of the Nth repeat, which is
//! almost always while the user is still typing, i.e. exactly when the
//! anti-creepy guard forbids surfacing.

use crate::config::Config;
use crate::events::activity::{ActivityEvent, ActivityEventType};
use crate::events::{self, PermagentEventType};
use crate::initiative::command_counter::{CommandCounter, CommandPattern};
use crate::initiative::gate::{evaluate, GateConfig, GateDecision, GateInputs};
use crate::initiative::tick::{run_initiative_tick, TickOutcome};
use crate::projects::PERSONAL_PROJECT_ID;
use crate::providers::base::Provider;
use chrono::{DateTime, Utc};
use sqlx::{Pool, Sqlite};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;

/// Master switch. Explicit opt-in — the driver does not start without it.
pub const INITIATIVE_ENABLED_KEY: &str = "initiative_enabled";
/// Seconds between ticks. Idle ticks exit at Tier 0 for free.
pub const INITIATIVE_TICK_SECS_KEY: &str = "initiative_tick_secs";
/// Gate windows (fall back to the gate's own defaults when unset).
pub const INITIATIVE_QUIET_SECS_KEY: &str = "initiative_quiet_secs";
pub const INITIATIVE_COOLDOWN_SECS_KEY: &str = "initiative_cooldown_secs";
/// Cheap-draft model routing (#249 pattern). Empty string = explicit opt-out
/// (template drafts only). Mirrors `ORCHESTRATOR_DECOMPOSITION_*`.
pub const INITIATIVE_DRAFT_PROVIDER_KEY: &str = "INITIATIVE_DRAFT_PROVIDER";
pub const INITIATIVE_DRAFT_MODEL_KEY: &str = "INITIATIVE_DRAFT_MODEL";

const DEFAULT_TICK_SECS: u64 = 60;
const DEFAULT_DRAFT_PROVIDER: &str = "ollama";
const DEFAULT_DRAFT_MODEL: &str = "qwen2.5:7b";

/// Persisted cursors (RFC 3339 strings in config).
const PREV_TICK_ACTIVITY_KEY: &str = "initiative_prev_tick_activity_at";
const LAST_PROPOSAL_KEY: &str = "initiative_last_proposal_at";

/// Whether the driver is switched on. Also read by the self-knowledge brief so
/// Henry describes the worker's real state instead of over-claiming.
pub fn is_enabled() -> bool {
    Config::global()
        .get_param::<bool>(INITIATIVE_ENABLED_KEY)
        .unwrap_or(false)
}

/// Spawn the driver if enabled. Called once at daemon startup with the app DB
/// pool (cards + recognition live there). Never silent: logs the on/off state
/// either way.
pub fn spawn(pool: Pool<Sqlite>) {
    if !is_enabled() {
        tracing::info!(
            target: "initiative",
            "initiative driver OFF ({INITIATIVE_ENABLED_KEY}=false) — set it to true to enable ambient goal origination"
        );
        return;
    }
    let tick_secs = Config::global()
        .get_param::<u64>(INITIATIVE_TICK_SECS_KEY)
        .unwrap_or(DEFAULT_TICK_SECS)
        .max(5);
    let cfg = gate_config();
    tracing::info!(
        target: "initiative",
        tick_secs,
        quiet_secs = cfg.quiet_secs,
        cooldown_secs = cfg.cooldown_secs,
        "initiative driver ON — observing activity for repeated-command patterns"
    );
    tokio::spawn(run_driver(pool, tick_secs, cfg));
}

fn gate_config() -> GateConfig {
    let defaults = GateConfig::default();
    GateConfig {
        quiet_secs: Config::global()
            .get_param::<i64>(INITIATIVE_QUIET_SECS_KEY)
            .unwrap_or(defaults.quiet_secs),
        cooldown_secs: Config::global()
            .get_param::<i64>(INITIATIVE_COOLDOWN_SECS_KEY)
            .unwrap_or(defaults.cooldown_secs),
    }
}

fn load_cursor(key: &str) -> Option<DateTime<Utc>> {
    Config::global()
        .get_param::<String>(key)
        .ok()
        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
        .map(|dt| dt.with_timezone(&Utc))
}

fn store_cursor(key: &str, value: DateTime<Utc>) {
    if let Err(e) = Config::global().set_param(key, value.to_rfc3339()) {
        tracing::warn!(target: "initiative", "failed to persist cursor {key}: {e}");
    }
}

/// Resolve the cheap draft provider (#249 routing). Called only after the pure
/// gate has decided a token may be spent — never on idle ticks.
async fn resolve_draft_provider() -> Option<Arc<dyn Provider>> {
    let config = Config::global();
    let provider_name = config
        .get_param::<String>(INITIATIVE_DRAFT_PROVIDER_KEY)
        .unwrap_or_else(|_| DEFAULT_DRAFT_PROVIDER.to_string());
    let model_name = config
        .get_param::<String>(INITIATIVE_DRAFT_MODEL_KEY)
        .unwrap_or_else(|_| DEFAULT_DRAFT_MODEL.to_string());

    // Explicit opt-out: empty provider/model → deterministic template drafts.
    if provider_name.trim().is_empty() || model_name.trim().is_empty() {
        return None;
    }

    match crate::providers::create_with_named_model(&provider_name, &model_name, Vec::new()).await {
        Ok(provider) => Some(provider),
        Err(e) => {
            tracing::debug!(
                target: "initiative",
                "cheap draft provider {provider_name}/{model_name} unavailable ({e}); using template draft"
            );
            None
        }
    }
}

async fn run_driver(pool: Pool<Sqlite>, tick_secs: u64, cfg: GateConfig) {
    let mut rx = events::subscribe();
    let mut interval = tokio::time::interval(Duration::from_secs(tick_secs));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let mut counter = CommandCounter::default();
    let mut pending_pattern: Option<CommandPattern> = None;
    let mut last_activity_at: Option<DateTime<Utc>> = None;
    let mut last_engagement_at: Option<DateTime<Utc>> = None;
    let mut prev_tick_activity_at = load_cursor(PREV_TICK_ACTIVITY_KEY);
    let mut last_proposal_at = load_cursor(LAST_PROPOSAL_KEY);

    loop {
        tokio::select! {
            recv = rx.recv() => match recv {
                Ok(event) => {
                    if event.event_type != PermagentEventType::Activity {
                        continue;
                    }
                    let Some(inner) = event.payload.get("event") else { continue };
                    let Ok(activity) = serde_json::from_value::<ActivityEvent>(inner.clone())
                    else {
                        continue;
                    };
                    last_activity_at = last_activity_at.max(Some(activity.timestamp));
                    if matches!(
                        activity.event_type,
                        ActivityEventType::ChatTurnStarted | ActivityEventType::ChatTurnCompleted
                    ) {
                        last_engagement_at = last_engagement_at.max(Some(activity.timestamp));
                    }
                    if let Some(pattern) = counter.record(&activity) {
                        tracing::info!(
                            target: "initiative",
                            normalized = %pattern.normalized,
                            count = pattern.count,
                            "repeated-command pattern crossed threshold — held for next quiet tick"
                        );
                        pending_pattern = Some(pattern);
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!(target: "initiative", "event bus lagged, missed {n} events");
                }
                Err(RecvError::Closed) => break,
            },
            _ = interval.tick() => {
                let inputs = GateInputs {
                    now: Utc::now(),
                    last_activity_at,
                    prev_tick_activity_at,
                    last_engagement_at,
                    last_proposal_at,
                    pattern: pending_pattern.clone(),
                };
                // Pre-run the (pure, deterministic) gate so a provider handle is
                // only resolved when this tick will actually spend a token; the
                // tick re-evaluates the same inputs to the same decision.
                let provider = match evaluate(&inputs, &cfg) {
                    GateDecision::Proceed(_) => resolve_draft_provider().await,
                    GateDecision::Skip(_) => None,
                };
                match run_initiative_tick(&pool, PERSONAL_PROJECT_ID, &inputs, &cfg, provider.as_ref()).await {
                    Ok(TickOutcome::Skipped(reason)) => {
                        tracing::trace!(target: "initiative", ?reason, "tick skipped at Tier 0");
                    }
                    Ok(TickOutcome::Pruned) => {
                        tracing::info!(
                            target: "initiative",
                            "candidate pruned — previously proposed and declined; will not re-pitch"
                        );
                        pending_pattern = None;
                    }
                    Ok(TickOutcome::Emitted { card_id }) => {
                        tracing::info!(
                            target: "initiative",
                            %card_id,
                            "initiative surfaced — automation proposal card in Triage"
                        );
                        pending_pattern = None;
                        last_proposal_at = Some(inputs.now);
                        store_cursor(LAST_PROPOSAL_KEY, inputs.now);
                    }
                    Err(e) => {
                        tracing::warn!(target: "initiative", "tick failed: {e}");
                    }
                }
                // Advance the delta cursor to what this tick observed (W2
                // contract); persist only on change to avoid idle config writes.
                if prev_tick_activity_at != last_activity_at {
                    prev_tick_activity_at = last_activity_at;
                    if let Some(ts) = last_activity_at {
                        store_cursor(PREV_TICK_ACTIVITY_KEY, ts);
                    }
                }
            }
        }
    }
    tracing::info!(target: "initiative", "initiative driver stopped (event bus closed)");
}
