//! Per-role model defaults — which model answers a CHAT turn, and which one
//! runs the CODING HARNESS.
//!
//! Permagent had one model knob (`GOOSE_PROVIDER` / `GOOSE_MODEL`, "the session
//! model") and three jobs with genuinely different shapes:
//!
//! - **Voice** wants the first spoken syllable fast. Measured in
//!   `docs/research/VOICE_MODEL_BENCH_2026-08-25.md`; configured by
//!   [`crate::config::voice_model`] (`voice_provider` / `voice_model`).
//! - **Chat** wants the first *written* word fast without giving up answer
//!   quality — a person is watching a cursor blink, not waiting out loud.
//! - **The coding harness** wants the highest pass rate per dollar. Latency
//!   barely matters across a ten-minute agent loop; a wrong answer costs a
//!   re-run and a right one is worth paying for.
//!
//! `docs/research/MODEL_DEFAULTS_BENCH_2026-08-25.md` measured chat and harness
//! the way the voice bench measured voice. This module is where those two
//! answers are configured, and it is deliberately ONE module with a role enum
//! rather than three near-identical files: Chat, Harness — and Voice, once
//! `voice_model.rs` is folded in here — are the same concept seen three times,
//! and the Settings UI shows them as three rows of one table.
//!
//! ## Precedence, and why it differs from voice
//!
//! ```text
//! CLI --provider/--model        (one run, wins outright)
//!   > the resumed session's own saved model
//!   > the recipe's `settings:` block
//!   > <role>_provider + <role>_model          <- this module
//!   > GOOSE_PROVIDER + GOOSE_MODEL            <- the session model
//!   > the measured default for the role       <- this module
//! ```
//!
//! Note where the measured default sits: **below** the session model, not above
//! it. [`crate::config::voice_model`] puts the voice default *above*
//! `GOOSE_MODEL`, and that is right for voice — a spoken turn on a reasoning
//! model is a 10-second silence, so the bench winner should apply even to
//! someone who has set a session model and never thought about voice.
//!
//! Chat and the harness are not like that. The session model IS the chat model
//! today; someone who set `GOOSE_MODEL` chose the model that answers them, and
//! silently outranking that choice with a benchmark result would be changing a
//! setting the user made without asking. So:
//!
//! - **Nothing configured at all** (a fresh install) → the default is chosen
//!   from the providers the user has ACTUALLY configured, reported as
//!   [`RoleModelSource::Default`]. The measured bench winners are only the
//!   preference order among those: the first entry of [`ModelRole::preferences`]
//!   whose provider has a key (or, for `ollama`, answers on its port) wins; the
//!   local model ([`crate::config::local_default_model`]) is the last rung.
//!   When no provider is usable the role has no route and the source is
//!   [`RoleModelSource::Unconfigured`] ([`NO_PROVIDER_REASON`] says what to do).
//!   A default that needs a key the user lacks is never returned.
//! - **A session model is set, no role keys** → the session model, reported as
//!   [`RoleModelSource::SessionModel`]. An existing choice is left alone for
//!   ordinary chat. The coding harness applies one additional evidence gate at
//!   session build: an unrated or below-floor model cannot own the top-level
//!   coding DAG and is routed to its graduated harness route instead.
//! - **Both role keys set** → that route, [`RoleModelSource::Configured`].
//! - **Exactly one role key set** → [`RoleModelSource::HalfConfigured`]: the
//!   caller WARNs and resolution continues as if neither were set. Half a pair
//!   is a typo, not an intention, and routing to a provider with no model fails
//!   in the middle of the work.
//! - **`session` / `off` / `none`** in either key → [`RoleModelSource::Disabled`]
//!   with no route: run the role on the session model, which is the pre-bench
//!   behaviour and the way back to one model for everything.
//!
//! Pure core plus a thin IO wrapper, so the precedence logic is unit-testable
//! without the process-global config.

use serde::{Deserialize, Serialize};

/// Heuristic orchestration threshold used to classify candidates. This is
/// deliberately lower than the delegated ORCHESTRATE role floor (0.80), but
/// it is NOT graduation evidence: the coding-suite pass/tool/DAG rates needed
/// to promote a model are not yet recorded. Mechanical workers have their own
/// cheaper, lower contract and are unaffected by this gate.
pub const HARNESS_COORDINATOR_MIN_ORCHESTRATION: f64 = 0.60;

/// Evidence status for a candidate coding-harness coordinator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinatorEligibility {
    /// A route listed in the `trusted_default_models` config list (its
    /// built-in default list holds the measured harness routes). Trusted
    /// coordinators are the only ones that graduate until a benchmark-backed
    /// graduation record is added.
    TrustedDefault,
    /// A known model cleared the knowledge-base heuristic, but has not yet
    /// graduated on Permagent's own coding harness evidence.
    HeuristicCandidate,
    /// The model has no measured knowledge-base row, so it cannot graduate by
    /// guesswork (for example `ollama/qwen25-16k:latest`).
    Unrated,
    /// A measured model is below the coordinator floor; it may still run
    /// bounded mechanical work.
    BelowFloor,
}

impl CoordinatorEligibility {
    pub fn is_eligible(self) -> bool {
        matches!(self, Self::TrustedDefault)
    }

    pub fn reason(self) -> &'static str {
        match self {
            Self::TrustedDefault => "listed in `trusted_default_models` (measured harness route)",
            Self::HeuristicCandidate => {
                "knowledge-base heuristic clears the floor, but harness graduation evidence is missing"
            }
            Self::Unrated => "no measured orchestration evidence for this model",
            Self::BelowFloor => "knowledge-base orchestration heuristic is below the coordinator floor",
        }
    }
}

/// Classify whether a provider/model is trusted for the top-level coding
/// coordinator role. The knowledge row is a heuristic only; unknown models
/// fail closed and a known non-default model still needs harness graduation
/// evidence before it may coordinate. All models remain available to the
/// mechanical routing lane.
///
/// ## Distinct from the roadmap-decomposition planner gate (2026-09-09)
///
/// This function is deliberately conservative — it grants `TrustedDefault`
/// to exactly the ONE compiled-in route ([`DEFAULT_HARNESS_PROVIDER_ID`]/
/// [`DEFAULT_HARNESS_MODEL_ID`]) and treats every other model, however
/// strong, as only a `HeuristicCandidate` — because it gates the SESSION
/// coordinator: the model that owns the top-level coding DAG for an entire
/// run with no further human check. Since Round C (`HARNESS_LOOP_LOG.md`
/// Lane Z3), the roadmap-DECOMPOSITION planner uses a different, narrower
/// gate instead: `cost_router::capability_directory::pin_clears_orchestrate_
/// floor` — "clears the ORCHESTRATE floor (0.80) or carries graduated
/// evidence" — for a single PLANNING pass whose output (goals + acceptance
/// criteria) is itself reviewed at approval before anything executes. Do not
/// widen THIS function's gate to match that one; they answer different
/// questions with different blast radii.
pub fn assess_harness_coordinator(provider: &str, model: &str) -> CoordinatorEligibility {
    assess_harness_coordinator_in(provider, model, &trusted_default_models())
}

/// Pure core of [`assess_harness_coordinator`] over an explicit trusted list.
pub fn assess_harness_coordinator_in(
    provider: &str,
    model: &str,
    trusted: &[RoleModel],
) -> CoordinatorEligibility {
    let is_trusted = trusted
        .iter()
        .any(|t| t.provider.eq_ignore_ascii_case(provider) && t.model.eq_ignore_ascii_case(model));
    let Some((knowledge, _confidence)) =
        crate::cost_router::lookup_with_confidence(provider, model)
    else {
        // A trusted route with no knowledge row is still trusted: the operator
        // (or the shipped list) vouched for it explicitly.
        return if is_trusted {
            CoordinatorEligibility::TrustedDefault
        } else {
            CoordinatorEligibility::Unrated
        };
    };
    if is_trusted {
        CoordinatorEligibility::TrustedDefault
    } else if knowledge.orchestration_strength >= HARNESS_COORDINATOR_MIN_ORCHESTRATION {
        CoordinatorEligibility::HeuristicCandidate
    } else {
        CoordinatorEligibility::BelowFloor
    }
}

/// What the coordinator gate did with the route it was handed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessGateOutcome {
    /// The requested route is graduated and runs as asked.
    Kept,
    /// Not graduated, but explicitly chosen (CLI flag or a complete
    /// `harness_provider`/`harness_model` pair): kept, with a spend warning,
    /// rather than silently replaced by a paid fallback.
    KeptExplicit,
    /// Not graduated and no graduated fallback exists: kept, with the missing
    /// evidence made visible.
    KeptNoFallback,
    /// Not graduated: the top-level session is routed to a graduated fallback.
    Rerouted,
}

/// The coordinator gate's decision for one requested route. The ONE place the
/// "which model actually coordinates the coding harness" rule lives: the CLI
/// session builder applies it at launch and the daemon's `GET /config`
/// projects it for Settings, so the two cannot disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessGateDecision {
    /// The route the precedence chain produced, before the gate.
    pub requested: RoleModel,
    /// The route that will actually coordinate.
    pub route: RoleModel,
    /// The requested route's evidence status.
    pub eligibility: CoordinatorEligibility,
    pub outcome: HarnessGateOutcome,
}

impl HarnessGateDecision {
    /// One plain-words sentence for a person reading Settings, or `None` when
    /// the requested route runs as asked. The CLI keeps its own longer
    /// warnings (spend and cost-meter detail); this is the short version.
    pub fn plain_notice(&self) -> Option<String> {
        let requested = format!("{}/{}", self.requested.provider, self.requested.model);
        match self.outcome {
            HarnessGateOutcome::Kept => None,
            HarnessGateOutcome::Rerouted => Some(format!(
                "{requested} is not graduated as a coordinator; using {}/{}",
                self.route.provider, self.route.model
            )),
            HarnessGateOutcome::KeptExplicit => Some(format!(
                "{requested} is not graduated as a coordinator, but it was chosen explicitly, so it stays in control"
            )),
            HarnessGateOutcome::KeptNoFallback => Some(format!(
                "{requested} is not graduated as a coordinator, and no graduated fallback is available; keeping it"
            )),
        }
    }
}

/// Pure: apply the coordinator gate to a requested route.
///
/// A graduated route is kept. An ungraduated one is kept when
/// `explicit_override` (the operator chose it; replacing it would be
/// unexpected spend), otherwise routed to the first graduated candidate of
/// `configured_fallback` then [`ModelRole::Harness`]'s measured default. If
/// neither is graduated the requested route is kept and says so.
pub fn gate_harness_coordinator(
    requested: RoleModel,
    configured_fallback: Option<RoleModel>,
    explicit_override: bool,
) -> HarnessGateDecision {
    gate_harness_coordinator_in(
        requested,
        configured_fallback,
        explicit_override,
        &RoleContext::live(),
    )
}

/// Pure core of [`gate_harness_coordinator`]. The last-resort graduated route
/// is the first trusted default whose provider the user can actually use.
pub fn gate_harness_coordinator_in(
    requested: RoleModel,
    configured_fallback: Option<RoleModel>,
    explicit_override: bool,
    ctx: &RoleContext,
) -> HarnessGateDecision {
    let eligibility =
        assess_harness_coordinator_in(&requested.provider, &requested.model, &ctx.trusted);
    let keep = |outcome| HarnessGateDecision {
        route: requested.clone(),
        requested: requested.clone(),
        eligibility,
        outcome,
    };
    if eligibility.is_eligible() {
        return keep(HarnessGateOutcome::Kept);
    }
    if explicit_override {
        return keep(HarnessGateOutcome::KeptExplicit);
    }
    let trusted_usable = ctx
        .trusted
        .iter()
        .find(|t| ctx.availability.is_usable(&t.provider))
        .cloned();
    let target = [configured_fallback, trusted_usable]
        .into_iter()
        .flatten()
        .find(|candidate| {
            assess_harness_coordinator_in(&candidate.provider, &candidate.model, &ctx.trusted)
                .is_eligible()
        });
    match target {
        Some(target) => HarnessGateDecision {
            requested: requested.clone(),
            route: target,
            eligibility,
            outcome: HarnessGateOutcome::Rerouted,
        },
        None => keep(HarnessGateOutcome::KeptNoFallback),
    }
}

/// Config key holding the session provider — the model everything falls back to.
pub const SESSION_PROVIDER_KEY: &str = "GOOSE_PROVIDER";
/// Config key holding the session model.
pub const SESSION_MODEL_KEY: &str = "GOOSE_MODEL";

/// Values that mean "no separate model for this role — use the session model".
const DISABLE_VALUES: &[&str] = &["session", "off", "none"];

/// A job with its own model default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRole {
    /// The daemon's chat reply path — Aria answering a typed turn.
    Chat,
    /// The coding harness (`permagent run --recipe permagent-coding`).
    Harness,
}

impl ModelRole {
    /// The config key holding this role's provider id.
    pub fn provider_key(self) -> &'static str {
        match self {
            ModelRole::Chat => "chat_provider",
            ModelRole::Harness => "harness_provider",
        }
    }

    /// The config key holding this role's model id.
    pub fn model_key(self) -> &'static str {
        match self {
            ModelRole::Chat => "chat_model",
            ModelRole::Harness => "harness_model",
        }
    }

    /// Human label used in logs and in Settings.
    pub fn label(self) -> &'static str {
        match self {
            ModelRole::Chat => "chat",
            ModelRole::Harness => "harness",
        }
    }

    /// The most-preferred measured winner for this role, IGNORING which
    /// providers the user has. It is the head of [`Self::preferences`]; the
    /// route actually used on a fresh install comes from
    /// [`RoleContext::default_for`], which skips providers without credentials.
    ///
    /// Both numbers below come from
    /// `docs/research/MODEL_DEFAULTS_BENCH_2026-08-25.md`; if you change a value
    /// here, change it there too, with the measurement that justifies it.
    pub fn measured_default(self) -> RoleModel {
        match self {
            ModelRole::Chat => RoleModel::new(DEFAULT_CHAT_PROVIDER_ID, DEFAULT_CHAT_MODEL_ID),
            ModelRole::Harness => {
                RoleModel::new(DEFAULT_HARNESS_PROVIDER_ID, DEFAULT_HARNESS_MODEL_ID)
            }
        }
    }

    /// The measured preference order for this role. A default is the first
    /// entry whose provider the user has configured; the list spans several
    /// providers so no single vendor is required.
    pub fn preferences(self) -> Vec<RoleModel> {
        let pair = |p: &str, m: &str| RoleModel::new(p, m);
        match self {
            ModelRole::Chat => vec![
                pair(DEFAULT_CHAT_PROVIDER_ID, DEFAULT_CHAT_MODEL_ID),
                pair("anthropic", "claude-haiku-4-5-20251001"),
                pair("openai", "gpt-5.4-mini"),
                pair("google", "gemini-3-pro"),
            ],
            ModelRole::Harness => vec![
                pair(DEFAULT_HARNESS_PROVIDER_ID, DEFAULT_HARNESS_MODEL_ID),
                pair("custom_deepseek", "deepseek-chat"),
                pair("anthropic", "claude-haiku-4-5-20251001"),
                pair("google", "gemini-3-pro"),
            ],
        }
    }

    /// Every role, for callers that render all of them (Settings, `permagent info`).
    pub fn all() -> [ModelRole; 2] {
        [ModelRole::Chat, ModelRole::Harness]
    }
}

/// Provider id of the chat default. See
/// `docs/research/MODEL_DEFAULTS_BENCH_2026-08-25.md`.
pub const DEFAULT_CHAT_PROVIDER_ID: &str = "custom_deepseek";
/// Model id of the chat default: DeepSeek Chat (`deepseek-chat`, billed as
/// `deepseek-v4-flash` on 2026-08-25).
///
/// The original chat bar was p90 time-to-first-token under 2.5 s. DeepSeek is
/// the only candidate that cleared it (median 1.83 s, p90 2.06 s). Haiku was
/// first on quality-with-silence-as-zero and never went wordless, but its p90
/// was 7.5 s — the code comment that said it met the bar was false on the same
/// table. GPT-5.4-mini picked the right tool on 1 of 10 tool turns, so it is
/// not a chat default. Voice uses this same model so typed and spoken turns
/// land on one family.
pub const DEFAULT_CHAT_MODEL_ID: &str = "deepseek-chat";

/// Provider id of the measured coding-harness default. See
/// `docs/research/MODEL_DEFAULTS_BENCH_2026-08-25.md`.
pub const DEFAULT_HARNESS_PROVIDER_ID: &str = "openai";
/// Model id of the coding-harness default: GPT-5.4-mini.
///
/// Six candidates solved every task they ran (n ≤ 3), so pass rate ranked
/// nothing. Raw $/solved favoured Haiku only because it was the sole model
/// whose prompt cache worked (79% vs 0% on every OpenAI-format candidate).
/// That prefix-stability bug is fixed in #1122. Cache-corrected, DeepSeek
/// ($0.17) and GPT-5.4-mini ($0.18) beat Haiku ($0.44). Mini is the coding
/// default: near-cheapest after the fix, a different family from chat/voice,
/// and the id the harness already knows how to pin.
pub const DEFAULT_HARNESS_MODEL_ID: &str = "gpt-5.4-mini";

/// Config key holding the list of `provider/model` routes trusted as the
/// coding harness's top-level coordinator.
pub const TRUSTED_DEFAULT_MODELS_KEY: &str = "trusted_default_models";

/// The built-in trusted list: the routes the 2026-08 harness bench measured as
/// cost-effective coordinators. It spans two providers so no single vendor is
/// required. Override with `trusted_default_models:` in `config.yaml` (a list
/// of `provider/model` strings; an explicit empty list trusts nothing).
pub const DEFAULT_TRUSTED_DEFAULT_MODELS: &[&str] =
    &["openai/gpt-5.4-mini", "custom_deepseek/deepseek-chat"];

/// Why a role has no route when no provider is usable.
pub const NO_PROVIDER_REASON: &str = "No model provider is configured. Add an API key or start a local Ollama in Settings > Models (or set GOOSE_PROVIDER and GOOSE_MODEL), then retry.";

fn parse_route(raw: &str) -> Option<RoleModel> {
    let (provider, model) = raw.trim().split_once('/')?;
    let (provider, model) = (provider.trim(), model.trim());
    (!provider.is_empty() && !model.is_empty()).then(|| RoleModel::new(provider, model))
}

/// Pure: the trusted list from a configured value (`None` = key unset).
pub fn parse_trusted_default_models(configured: Option<Vec<String>>) -> Vec<RoleModel> {
    match configured {
        Some(list) => list.iter().filter_map(|r| parse_route(r)).collect(),
        None => DEFAULT_TRUSTED_DEFAULT_MODELS
            .iter()
            .filter_map(|r| parse_route(r))
            .collect(),
    }
}

/// The trusted coordinator list, read from the process-global config.
pub fn trusted_default_models() -> Vec<RoleModel> {
    parse_trusted_default_models(
        crate::config::Config::global()
            .get_param::<Vec<String>>(TRUSTED_DEFAULT_MODELS_KEY)
            .ok(),
    )
}

/// Which providers the user can actually use right now.
#[derive(Debug, Clone)]
pub struct ProviderAvailability {
    /// `None` means "treat every provider as usable" (pure tests).
    usable: Option<std::collections::BTreeSet<String>>,
}

impl ProviderAvailability {
    /// Every provider counts as usable.
    pub fn all() -> Self {
        Self { usable: None }
    }

    /// Exactly these providers are usable (lowercase ids).
    pub fn only(ids: &[&str]) -> Self {
        Self {
            usable: Some(ids.iter().map(|i| i.to_ascii_lowercase()).collect()),
        }
    }

    /// No provider is usable.
    pub fn none() -> Self {
        Self::only(&[])
    }

    /// Detect from the machine: keyed providers with a configured key, plus
    /// `ollama` when its port answers.
    pub fn live() -> Self {
        let mut usable = std::collections::BTreeSet::new();
        for (provider, env) in crate::cost_router::recommend::BUILTIN_PROVIDERS {
            if !env.is_empty() && crate::cost_router::cheap::is_key_configured(env) {
                usable.insert(provider.to_string());
            }
        }
        for c in crate::config::declarative_providers::all_declarative_provider_configs() {
            if !c.api_key_env.trim().is_empty()
                && crate::cost_router::cheap::is_key_configured(&c.api_key_env)
            {
                usable.insert(c.name.to_ascii_lowercase());
            }
        }
        if ollama_reachable_cached() {
            usable.insert("ollama".to_string());
        }
        Self {
            usable: Some(usable),
        }
    }

    pub fn is_usable(&self, provider: &str) -> bool {
        match &self.usable {
            None => true,
            Some(set) => set.contains(&provider.to_ascii_lowercase()),
        }
    }
}

/// Ollama counts as usable only if something answers on its port. Cached for a
/// few seconds because a role is resolved on every chat turn.
fn ollama_reachable_cached() -> bool {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    if let Ok(guard) = CACHE.lock() {
        if let Some((at, up)) = *guard {
            if at.elapsed() < Duration::from_secs(15) {
                return up;
            }
        }
    }
    let up = ollama_reachable();
    if let Ok(mut guard) = CACHE.lock() {
        *guard = Some((Instant::now(), up));
    }
    up
}

fn ollama_reachable() -> bool {
    use std::net::ToSocketAddrs;
    use std::time::Duration;
    let raw = crate::config::Config::global()
        .get_param::<String>("OLLAMA_HOST")
        .ok();
    let base = crate::cost_router::recommend::ollama_base_url(raw);
    let Ok(url) = reqwest::Url::parse(&base) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let port = url.port_or_known_default().unwrap_or(11434);
    (host, port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut addrs| addrs.next())
        .is_some_and(|addr| {
            std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok()
        })
}

/// Everything the default-selection rules need beyond the config reader.
#[derive(Debug, Clone)]
pub struct RoleContext {
    pub availability: ProviderAvailability,
    pub trusted: Vec<RoleModel>,
    pub local_model: String,
}

impl RoleContext {
    /// Detect availability from the machine and read the trusted list and
    /// local model from the process-global config.
    pub fn live() -> Self {
        Self {
            availability: ProviderAvailability::live(),
            trusted: trusted_default_models(),
            local_model: crate::config::local_default_model(),
        }
    }

    /// Every provider usable, the built-in trusted list, the default local
    /// model. For pure tests of the precedence rules.
    pub fn permissive() -> Self {
        Self::with_availability(ProviderAvailability::all())
    }

    pub fn with_availability(availability: ProviderAvailability) -> Self {
        Self {
            availability,
            trusted: parse_trusted_default_models(None),
            local_model: crate::config::DEFAULT_LOCAL_MODEL.to_string(),
        }
    }

    /// The default route for a role given what the user has: the first
    /// preference whose provider is usable, then the local model when Ollama is
    /// up. `None` when nothing is usable — never a route that needs a missing key.
    pub fn default_for(&self, role: ModelRole) -> Option<RoleModel> {
        role.preferences()
            .into_iter()
            .find(|p| self.availability.is_usable(&p.provider))
            .or_else(|| {
                self.availability
                    .is_usable("ollama")
                    .then(|| RoleModel::new("ollama", self.local_model.clone()))
            })
    }
}

/// A resolved route: the concrete provider+model a role's turn goes to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleModel {
    pub provider: String,
    pub model: String,
}

impl RoleModel {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
        }
    }
}

/// Where a resolved route came from — carried so the caller can log it rather
/// than leaving the operator to guess which model answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleModelSource {
    /// Both role keys were set by the operator.
    Configured,
    /// No role keys; the session model (`GOOSE_*`) applies.
    SessionModel,
    /// Nothing configured anywhere; the measured default applies.
    Default,
    /// Exactly one role key was set. Resolution continued as if neither were,
    /// and the caller should WARN — this is a typo or an unfinished edit.
    HalfConfigured,
    /// A role key held `session` / `off` / `none`: run on the session model.
    Disabled,
    /// Nothing is configured and no provider is usable (no API key, no
    /// reachable Ollama), so there is no default to offer. See
    /// [`NO_PROVIDER_REASON`].
    Unconfigured,
}

impl RoleModelSource {
    /// Whether the caller should emit a warning about a misconfiguration.
    pub fn should_warn(self) -> bool {
        matches!(self, RoleModelSource::HalfConfigured)
    }

    /// Plain-words explanation for the one source that has no route at all.
    pub fn reason(self) -> Option<&'static str> {
        matches!(self, RoleModelSource::Unconfigured).then_some(NO_PROVIDER_REASON)
    }
}

/// The outcome of resolving one role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleResolution {
    /// The route to use, or `None` when the caller should leave the session's
    /// existing model alone (nothing role-specific applies).
    pub route: Option<RoleModel>,
    /// How [`Self::route`] was arrived at.
    pub source: RoleModelSource,
}

/// Pure: resolve one role's route from a key reader, with provider
/// availability detected live (see [`RoleContext::live`]).
///
/// `read` is given raw config keys — the role's two keys and, when they do not
/// decide it, [`SESSION_PROVIDER_KEY`] / [`SESSION_MODEL_KEY`].
pub fn resolve_role_model(
    role: ModelRole,
    read: impl Fn(&str) -> Option<String>,
) -> RoleResolution {
    resolve_role_model_in(role, read, &RoleContext::live())
}

/// [`resolve_role_model`] over an explicit [`RoleContext`] (tests, and callers
/// that already hold one).
pub fn resolve_role_model_in(
    role: ModelRole,
    read: impl Fn(&str) -> Option<String>,
    ctx: &RoleContext,
) -> RoleResolution {
    let value = |key: &str| {
        read(key)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let is_disable = |v: &Option<String>| v.as_ref().is_some_and(|v| is_disable_value(v.as_str()));

    let provider = value(role.provider_key());
    let model = value(role.model_key());

    if is_disable(&provider) || is_disable(&model) {
        return RoleResolution {
            route: None,
            source: RoleModelSource::Disabled,
        };
    }

    let half_configured = match (&provider, &model) {
        (Some(provider), Some(model)) => {
            return RoleResolution {
                route: Some(RoleModel::new(provider.clone(), model.clone())),
                source: RoleModelSource::Configured,
            }
        }
        (None, None) => false,
        _ => true,
    };

    // No usable role keys. The session model is an explicit user choice and
    // outranks the bench; only a machine with nothing configured at all falls
    // through to a default chosen from the providers the user has.
    let session_provider = value(SESSION_PROVIDER_KEY);
    let session_model = value(SESSION_MODEL_KEY);
    let session_is_set = session_provider.is_some() && session_model.is_some();

    if half_configured {
        return RoleResolution {
            route: if session_is_set {
                None
            } else {
                ctx.default_for(role)
            },
            source: RoleModelSource::HalfConfigured,
        };
    }
    if session_is_set {
        return RoleResolution {
            route: None,
            source: RoleModelSource::SessionModel,
        };
    }
    match ctx.default_for(role) {
        Some(route) => RoleResolution {
            route: Some(route),
            source: RoleModelSource::Default,
        },
        None => RoleResolution {
            route: None,
            source: RoleModelSource::Unconfigured,
        },
    }
}

/// The coding harness's coordinator as a launch from config alone would run
/// it: role keys, then the session model, then the measured default — and
/// then the coordinator gate. What Settings must show, because the bare
/// config keys are not what runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveHarnessRoute {
    /// The route that coordinates. Empty strings only when nothing at all is
    /// configured to run on (`session` disabled role keys and no session
    /// model), in which case a launch fails with "No provider configured".
    pub route: RoleModel,
    /// Where the requested route came from, before the gate.
    pub source: RoleModelSource,
    /// What the gate did; `None` only in the nothing-to-run-on case above.
    pub outcome: Option<HarnessGateOutcome>,
    /// A plain-words sentence when the route shown is not simply the one
    /// configured (see [`HarnessGateDecision::plain_notice`]).
    pub notice: Option<String>,
}

/// Pure: resolve the coding harness's effective coordinator from a key reader.
///
/// Mirrors `goose-cli`'s session builder for a launch with no CLI
/// `--provider/--model`, no resumed session and a recipe that does not pin a
/// model (the built-in `permagent-coding` recipe pins none). Those per-run
/// layers cannot be known from config and are not represented here.
pub fn effective_harness_route(read: impl Fn(&str) -> Option<String>) -> EffectiveHarnessRoute {
    effective_harness_route_in(read, &RoleContext::live())
}

/// [`effective_harness_route`] over an explicit [`RoleContext`].
pub fn effective_harness_route_in(
    read: impl Fn(&str) -> Option<String>,
    ctx: &RoleContext,
) -> EffectiveHarnessRoute {
    let value = |key: &str| {
        read(key)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let resolution = resolve_role_model_in(ModelRole::Harness, &read, ctx);
    // `resolve_role_model` returns `None` when the session model applies.
    let requested = resolution.route.clone().or_else(|| {
        match (value(SESSION_PROVIDER_KEY), value(SESSION_MODEL_KEY)) {
            (Some(provider), Some(model)) => Some(RoleModel::new(provider, model)),
            _ => None,
        }
    });
    let Some(requested) = requested else {
        let provider = value(SESSION_PROVIDER_KEY).unwrap_or_default();
        let model = value(SESSION_MODEL_KEY).unwrap_or_default();
        return EffectiveHarnessRoute {
            route: RoleModel::new(provider, model),
            source: resolution.source,
            outcome: None,
            notice: Some(match resolution.source.reason() {
                Some(reason) => reason.to_string(),
                None => {
                    "No session model is set, so the coding harness has no model to run on".into()
                }
            }),
        };
    };
    let decision = gate_harness_coordinator_in(
        requested,
        resolution.route,
        resolution.source == RoleModelSource::Configured,
        ctx,
    );
    EffectiveHarnessRoute {
        notice: decision.plain_notice(),
        outcome: Some(decision.outcome),
        route: decision.route,
        source: resolution.source,
    }
}

/// [`effective_harness_route`] read from the process-global config.
pub fn effective_harness_route_from_config() -> EffectiveHarnessRoute {
    let config = crate::config::Config::global();
    effective_harness_route(|key| config.get_param::<String>(key).ok())
}

fn is_disable_value(value: &str) -> bool {
    DISABLE_VALUES.contains(&value.trim().to_lowercase().as_str())
}

/// One role's route, read from the process-global config
/// (`~/.permagent/config.yaml`, or the matching environment variables).
pub fn role_model_from_config(role: ModelRole) -> RoleResolution {
    let config = crate::config::Config::global();
    resolve_role_model(role, |key| config.get_param::<String>(key).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // The precedence tests below are about ordering, not about what the host
    // machine has configured, so they run with every provider usable. The
    // provider-aware tests at the bottom pin availability explicitly.
    fn resolve_role_model(
        role: ModelRole,
        read: impl Fn(&str) -> Option<String>,
    ) -> RoleResolution {
        resolve_role_model_in(role, read, &RoleContext::permissive())
    }

    fn effective_harness_route(read: impl Fn(&str) -> Option<String>) -> EffectiveHarnessRoute {
        effective_harness_route_in(read, &RoleContext::permissive())
    }

    fn assess_harness_coordinator(provider: &str, model: &str) -> CoordinatorEligibility {
        assess_harness_coordinator_in(provider, model, &RoleContext::permissive().trusted)
    }

    fn reader(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    #[test]
    fn a_fresh_install_gets_the_measured_default() {
        for role in ModelRole::all() {
            let resolved = resolve_role_model(role, reader(&[]));
            assert_eq!(resolved.source, RoleModelSource::Default);
            assert_eq!(resolved.route, Some(role.measured_default()));
        }
    }

    #[test]
    fn an_explicit_session_model_outranks_the_measured_default() {
        // The whole point of the precedence choice: a user who set GOOSE_MODEL
        // chose their model, and a benchmark does not get to overrule that.
        for role in ModelRole::all() {
            let resolved = resolve_role_model(
                role,
                reader(&[
                    (SESSION_PROVIDER_KEY, "zai"),
                    (SESSION_MODEL_KEY, "glm-5.3"),
                ]),
            );
            assert_eq!(resolved.source, RoleModelSource::SessionModel);
            assert_eq!(
                resolved.route,
                None,
                "{}: the caller must be left to use the session model",
                role.label()
            );
        }
    }

    #[test]
    fn both_role_keys_set_outrank_the_session_model() {
        let resolved = resolve_role_model(
            ModelRole::Harness,
            reader(&[
                ("harness_provider", "anthropic"),
                ("harness_model", "claude-sonnet-5"),
                (SESSION_PROVIDER_KEY, "zai"),
                (SESSION_MODEL_KEY, "glm-5.3"),
            ]),
        );
        assert_eq!(resolved.source, RoleModelSource::Configured);
        assert_eq!(
            resolved.route,
            Some(RoleModel::new("anthropic", "claude-sonnet-5"))
        );
    }

    #[test]
    fn the_roles_do_not_read_each_others_keys() {
        let resolved = resolve_role_model(
            ModelRole::Chat,
            reader(&[
                ("harness_provider", "anthropic"),
                ("harness_model", "claude-sonnet-5"),
            ]),
        );
        assert_eq!(resolved.source, RoleModelSource::Default);
        assert_eq!(resolved.route, Some(ModelRole::Chat.measured_default()));
    }

    #[test]
    fn a_half_configured_pair_warns_and_resolves_as_if_unset() {
        for role in ModelRole::all() {
            for key in [role.provider_key(), role.model_key()] {
                let resolved = resolve_role_model(role, reader(&[(key, "anthropic")]));
                assert_eq!(
                    resolved.source,
                    RoleModelSource::HalfConfigured,
                    "{}: {key} alone",
                    role.label()
                );
                assert!(resolved.source.should_warn());
                assert_eq!(resolved.route, Some(role.measured_default()));

                // …and with a session model set, half a pair falls back to it
                // rather than to the bench winner.
                let with_session = resolve_role_model(
                    role,
                    reader(&[
                        (key, "anthropic"),
                        (SESSION_PROVIDER_KEY, "zai"),
                        (SESSION_MODEL_KEY, "glm-5.3"),
                    ]),
                );
                assert_eq!(with_session.source, RoleModelSource::HalfConfigured);
                assert_eq!(with_session.route, None);
            }
        }
    }

    #[test]
    fn session_off_and_none_pin_the_role_to_the_session_model() {
        for role in ModelRole::all() {
            for value in ["session", "off", "none", "SESSION", " Off "] {
                for key in [role.provider_key(), role.model_key()] {
                    let resolved = resolve_role_model(role, reader(&[(key, value)]));
                    assert_eq!(
                        resolved,
                        RoleResolution {
                            route: None,
                            source: RoleModelSource::Disabled,
                        },
                        "{}: {key}={value:?} should disable the role override",
                        role.label()
                    );
                }
            }
        }
    }

    #[test]
    fn a_disable_value_beats_a_fully_configured_partner_key() {
        // `harness_provider: session` with a stale model id left behind means
        // "back to one model for everything", not "route to the stale id".
        let resolved = resolve_role_model(
            ModelRole::Harness,
            reader(&[
                ("harness_provider", "session"),
                ("harness_model", "claude-sonnet-5"),
            ]),
        );
        assert_eq!(resolved.source, RoleModelSource::Disabled);
        assert_eq!(resolved.route, None);
    }

    #[test]
    fn empty_and_whitespace_values_are_unset_not_a_provider_named_space() {
        let resolved = resolve_role_model(
            ModelRole::Chat,
            reader(&[("chat_provider", "   "), ("chat_model", "")]),
        );
        assert_eq!(resolved.source, RoleModelSource::Default);
        assert_eq!(resolved.route, Some(ModelRole::Chat.measured_default()));
    }

    #[test]
    fn a_half_set_session_model_is_not_a_session_model() {
        // GOOSE_PROVIDER without GOOSE_MODEL cannot route anything, so it must
        // not suppress the measured default.
        let resolved =
            resolve_role_model(ModelRole::Chat, reader(&[(SESSION_PROVIDER_KEY, "zai")]));
        assert_eq!(resolved.source, RoleModelSource::Default);
        assert_eq!(resolved.route, Some(ModelRole::Chat.measured_default()));
    }

    #[test]
    fn role_keys_are_distinct_and_stable() {
        let keys: Vec<&str> = ModelRole::all()
            .iter()
            .flat_map(|r| [r.provider_key(), r.model_key()])
            .collect();
        assert_eq!(
            keys,
            vec![
                "chat_provider",
                "chat_model",
                "harness_provider",
                "harness_model"
            ]
        );
    }

    #[test]
    fn only_measured_coordinators_graduate() {
        assert_eq!(
            assess_harness_coordinator("openai", "gpt-5.4-mini"),
            CoordinatorEligibility::TrustedDefault
        );
        assert_eq!(
            assess_harness_coordinator("ollama", "qwen3-coder:30b"),
            CoordinatorEligibility::BelowFloor
        );
        // Rated since 2026-09-18 (an installed Ollama pull with a knowledge
        // row), and its estimated orchestration score is below the floor.
        assert_eq!(
            assess_harness_coordinator("ollama", "qwen25-16k:latest"),
            CoordinatorEligibility::BelowFloor
        );
        // A model with no knowledge row at all stays unrated.
        assert_eq!(
            assess_harness_coordinator("ollama", "llama3.1:8b"),
            CoordinatorEligibility::Unrated
        );
    }

    #[test]
    fn coordinator_gate_is_vendor_neutral_and_keeps_mechanical_models_available() {
        // A measured non-OpenAI model can graduate on the same score rule.
        assert_eq!(
            assess_harness_coordinator("minimax", "MiniMax-M2.5"),
            CoordinatorEligibility::HeuristicCandidate
        );
        // The gate only describes the top-level role; it does not alter the
        // separate local/mechanical capability contract.
        assert!(!CoordinatorEligibility::HeuristicCandidate.is_eligible());
        assert!(!CoordinatorEligibility::BelowFloor.is_eligible());
    }

    #[test]
    fn effective_harness_route_keeps_a_configured_pair() {
        // A complete harness pair is an operator choice: it wins over the
        // session model, and even ungraduated it is kept (with a notice)
        // rather than replaced by a paid fallback.
        let effective = effective_harness_route(reader(&[
            ("harness_provider", "anthropic"),
            ("harness_model", "claude-haiku-4-5"),
            (SESSION_PROVIDER_KEY, "zai"),
            (SESSION_MODEL_KEY, "glm-5.3"),
        ]));
        assert_eq!(effective.source, RoleModelSource::Configured);
        assert_eq!(
            effective.route,
            RoleModel::new("anthropic", "claude-haiku-4-5")
        );
        assert_eq!(effective.outcome, Some(HarnessGateOutcome::KeptExplicit));
        let notice = effective.notice.expect("an ungraduated pair must say so");
        assert!(notice.contains("anthropic/claude-haiku-4-5"), "{notice}");
        assert!(notice.contains("chosen explicitly"), "{notice}");

        // A configured pair that IS graduated runs silently.
        let graduated = effective_harness_route(reader(&[
            ("harness_provider", DEFAULT_HARNESS_PROVIDER_ID),
            ("harness_model", DEFAULT_HARNESS_MODEL_ID),
            (SESSION_PROVIDER_KEY, "zai"),
            (SESSION_MODEL_KEY, "glm-5.3"),
        ]));
        assert_eq!(graduated.source, RoleModelSource::Configured);
        assert_eq!(graduated.route, ModelRole::Harness.measured_default());
        assert_eq!(graduated.outcome, Some(HarnessGateOutcome::Kept));
        assert_eq!(graduated.notice, None);
    }

    #[test]
    fn effective_harness_route_reroutes_an_ungraduated_session_model_with_a_notice() {
        // The Settings bug: GOOSE_MODEL=claude-haiku-4-5 was shown as the
        // harness model, but a launch replaced it with the measured default.
        let effective = effective_harness_route(reader(&[
            (SESSION_PROVIDER_KEY, "anthropic"),
            (SESSION_MODEL_KEY, "claude-haiku-4-5"),
        ]));
        assert_eq!(effective.source, RoleModelSource::SessionModel);
        assert_eq!(effective.route, ModelRole::Harness.measured_default());
        assert_eq!(effective.outcome, Some(HarnessGateOutcome::Rerouted));
        let expected = format!(
            "anthropic/claude-haiku-4-5 is not graduated as a coordinator; using {}/{}",
            DEFAULT_HARNESS_PROVIDER_ID, DEFAULT_HARNESS_MODEL_ID
        );
        assert_eq!(effective.notice.as_deref(), Some(expected.as_str()));

        // `session` in a role key means the same session model, and the same gate.
        let disabled = effective_harness_route(reader(&[
            ("harness_model", "session"),
            (SESSION_PROVIDER_KEY, "anthropic"),
            (SESSION_MODEL_KEY, "claude-haiku-4-5"),
        ]));
        assert_eq!(disabled.source, RoleModelSource::Disabled);
        assert_eq!(disabled.route, ModelRole::Harness.measured_default());
        assert_eq!(disabled.outcome, Some(HarnessGateOutcome::Rerouted));
    }

    #[test]
    fn effective_harness_route_passes_the_trusted_default_without_a_notice() {
        let fresh = effective_harness_route(reader(&[]));
        assert_eq!(fresh.source, RoleModelSource::Default);
        assert_eq!(fresh.route, ModelRole::Harness.measured_default());
        assert_eq!(fresh.outcome, Some(HarnessGateOutcome::Kept));
        assert_eq!(fresh.notice, None);

        let session = effective_harness_route(reader(&[
            (SESSION_PROVIDER_KEY, DEFAULT_HARNESS_PROVIDER_ID),
            (SESSION_MODEL_KEY, DEFAULT_HARNESS_MODEL_ID),
        ]));
        assert_eq!(session.source, RoleModelSource::SessionModel);
        assert_eq!(session.route, ModelRole::Harness.measured_default());
        assert_eq!(session.notice, None);
    }

    #[test]
    fn effective_harness_route_with_nothing_to_run_on_says_so() {
        let effective = effective_harness_route(reader(&[("harness_model", "off")]));
        assert_eq!(effective.source, RoleModelSource::Disabled);
        assert_eq!(effective.outcome, None);
        assert_eq!(effective.route, RoleModel::new("", ""));
        assert!(effective.notice.is_some());
    }

    fn ctx(providers: &[&str]) -> RoleContext {
        RoleContext::with_availability(ProviderAvailability::only(providers))
    }

    #[test]
    fn no_providers_means_no_route_and_a_configure_a_provider_reason() {
        for role in ModelRole::all() {
            let resolved = resolve_role_model_in(role, reader(&[]), &ctx(&[]));
            assert_eq!(resolved.route, None, "{}", role.label());
            assert_eq!(resolved.source, RoleModelSource::Unconfigured);
            let reason = resolved.source.reason().expect("a reason is given");
            assert!(
                reason.contains("No model provider is configured"),
                "{reason}"
            );
        }
        let effective = effective_harness_route_in(reader(&[]), &ctx(&[]));
        assert_eq!(effective.outcome, None);
        assert!(effective
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("No model provider is configured")));
    }

    #[test]
    fn only_an_anthropic_key_defaults_to_anthropic_never_a_provider_without_a_key() {
        for role in ModelRole::all() {
            let resolved = resolve_role_model_in(role, reader(&[]), &ctx(&["anthropic"]));
            assert_eq!(resolved.source, RoleModelSource::Default);
            let route = resolved.route.expect("anthropic is usable");
            assert_eq!(route.provider, "anthropic", "{}", role.label());
        }
    }

    #[test]
    fn only_ollama_defaults_to_the_configured_local_model() {
        let mut context = ctx(&["ollama"]);
        context.local_model = "llama3.2:3b".to_string();
        for role in ModelRole::all() {
            let resolved = resolve_role_model_in(role, reader(&[]), &context);
            assert_eq!(resolved.source, RoleModelSource::Default);
            assert_eq!(
                resolved.route,
                Some(RoleModel::new("ollama", "llama3.2:3b"))
            );
        }
    }

    #[test]
    fn a_keyed_provider_outranks_the_local_rung_and_preference_order_holds() {
        let both = resolve_role_model_in(
            ModelRole::Harness,
            reader(&[]),
            &ctx(&["openai", "anthropic", "ollama"]),
        );
        assert_eq!(both.route, Some(ModelRole::Harness.measured_default()));
        let deepseek = resolve_role_model_in(
            ModelRole::Chat,
            reader(&[]),
            &ctx(&["custom_deepseek", "openai"]),
        );
        assert_eq!(deepseek.route, Some(ModelRole::Chat.measured_default()));
    }

    #[test]
    fn explicit_config_stays_authoritative_even_when_the_provider_looks_unusable() {
        let resolved = resolve_role_model_in(
            ModelRole::Chat,
            reader(&[("chat_provider", "zai"), ("chat_model", "glm-5.3")]),
            &ctx(&[]),
        );
        assert_eq!(resolved.source, RoleModelSource::Configured);
        assert_eq!(resolved.route, Some(RoleModel::new("zai", "glm-5.3")));
        let session = resolve_role_model_in(
            ModelRole::Chat,
            reader(&[
                (SESSION_PROVIDER_KEY, "zai"),
                (SESSION_MODEL_KEY, "glm-5.3"),
            ]),
            &ctx(&[]),
        );
        assert_eq!(session.source, RoleModelSource::SessionModel);
    }

    #[test]
    fn the_trusted_list_is_config_driven_and_spans_providers() {
        let defaults = parse_trusted_default_models(None);
        let providers: std::collections::BTreeSet<_> =
            defaults.iter().map(|t| t.provider.as_str()).collect();
        assert!(providers.len() >= 2, "{providers:?}");

        let custom = parse_trusted_default_models(Some(vec![
            "anthropic/claude-haiku-4-5-20251001".into(),
            "garbage".into(),
            "/x".into(),
        ]));
        assert_eq!(
            custom,
            vec![RoleModel::new("anthropic", "claude-haiku-4-5-20251001")]
        );
        assert_eq!(
            assess_harness_coordinator_in("anthropic", "claude-haiku-4-5-20251001", &custom),
            CoordinatorEligibility::TrustedDefault
        );
        assert_ne!(
            assess_harness_coordinator_in("openai", "gpt-5.4-mini", &custom),
            CoordinatorEligibility::TrustedDefault
        );
        assert!(parse_trusted_default_models(Some(vec![])).is_empty());
    }

    #[test]
    fn the_gate_reroutes_to_a_trusted_route_the_user_can_actually_use() {
        let context = ctx(&["custom_deepseek"]);
        let decision = gate_harness_coordinator_in(
            RoleModel::new("anthropic", "claude-haiku-4-5"),
            None,
            false,
            &context,
        );
        assert_eq!(decision.outcome, HarnessGateOutcome::Rerouted);
        assert_eq!(
            decision.route,
            RoleModel::new("custom_deepseek", "deepseek-chat")
        );

        let nothing = gate_harness_coordinator_in(
            RoleModel::new("anthropic", "claude-haiku-4-5"),
            None,
            false,
            &ctx(&["anthropic"]),
        );
        assert_eq!(nothing.outcome, HarnessGateOutcome::KeptNoFallback);
    }
}
