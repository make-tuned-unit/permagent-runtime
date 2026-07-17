//! The cheap-cloud tier's provider ladder — "the cheapest cloud model that can
//! do the work, and only escalate when it can't", derived from **pricing**, not a
//! fixed provider list.
//!
//! The tier ladder in [`super::tier`] has a single [`Tier::CheapCloud`] rung
//! between local Ollama and the frontier. This module turns that one abstract
//! rung into a *ranked set* of concrete cheap-cloud providers ordered
//! cheapest-first, and picks the cheapest one the operator has actually
//! configured (its API key is set).
//!
//! ## Generic by construction — cheapest CONFIGURED + PRICED, any provider
//!
//! The ladder is **not** a hardcoded `[MiniMax < Kimi < Haiku]` list. It is
//! *derived*, at load time, from two data sources:
//!
//! 1. **The configured provider surface** — every declarative provider
//!    ([`crate::config::declarative_providers::all_declarative_provider_configs`],
//!    the bundled JSONs plus the operator's custom ones) whose `api_key_env` is
//!    set. Each config supplies the provider id, that key-env, and the models it
//!    serves.
//! 2. **The canonical pricing table** — [`crate::providers::canonical`], the
//!    single source of truth `cost_of` reads. Every candidate model is priced for
//!    a fixed reference workload; models with no computable price are dropped.
//!
//! The candidates are then **sorted by real canonical cost, cheapest-first**, and
//! [`CheapLadder::select`] returns the cheapest *available* one. So ANY provider
//! that is less expensive — MiniMax, Kimi, DeepSeek, Groq, or a provider that does
//! not exist yet — automatically becomes the preferred cheap tier the moment its
//! key is present and it is priced: **zero code change, just a JSON + a key**.
//! That is Jesse's "choose the cheapest but required path" made general: the
//! router never pays for a dearer model when a cheaper keyed+priced one can do the
//! same mechanical work, and never strands the task when none is keyed.
//!
//! ## The anchor — the always-available ceiling of the cheap tier
//!
//! Haiku 4.5, riding the operator's existing Anthropic auth, is the always-
//! available **anchor**. It is byte-identical to the mechanical pack default in
//! [`super::packs`], so with **no** cheap key set the CheapCloud tier resolves to
//! exactly what it does today — no behavior change until a cheaper priced provider
//! is configured. The anchor is also the tier's *ceiling*: [`build_ladder`] keeps
//! only discovered candidates **strictly cheaper** than the anchor (a model at or
//! above the anchor's price offers no saving over the reliable, free-to-reach
//! anchor), and appends the anchor last. On a failed attempt the ladder walks up
//! the price-ordered list toward the anchor via [`CheapLadder::escalate_after`];
//! only once it is exhausted does the caller escalate the *tier* to the frontier
//! via [`super::tier::next_after`] — reusing the #249/#720 escalation unchanged.
//!
//! ## Retunable, like the rest of the router
//!
//! The derived order is the default; the operator can still override:
//! - `PERMAGENT_CHEAP_ANCHOR_PROVIDER` / `PERMAGENT_CHEAP_ANCHOR_MODEL` — retune
//!   the always-available anchor (e.g. point it at a different reliable model).
//! - `PERMAGENT_CHEAP_PIN_PROVIDER` / `PERMAGENT_CHEAP_PIN_MODEL`
//!   (+ optional `PERMAGENT_CHEAP_PIN_KEY_ENV`) — **pin** the primary cheap rung
//!   to a specific provider+model, bypassing the cost-derived choice. The pin is
//!   honored only when its key is configured; otherwise the automatic derivation
//!   applies (never strand on a pin whose key is absent). The pinned provider's
//!   key-env is auto-resolved from its declarative config when not given.
//!
//! ## The mesh seam below Ollama
//!
//! This ladder is deliberately the *cloud* half of the cheapest-adequate ladder;
//! the tier BELOW local Ollama — the trusted mesh/pool ([`Tier::MeshFree`]) — is
//! the future "open-source LLM hosted across Permagent users" seam, gated and
//! wired in [`super::mesh`]. A mesh failure already degrades straight to *this*
//! cheap-cloud tier via [`super::mesh::fallback_tier`], so the two halves compose
//! without change here.
//!
//! ## Purity
//!
//! The selection is pure so it is unit-testable without env or network
//! (env-mutating tests flake under parallel `cargo test`): [`build_ladder`],
//! [`CheapLadder::select`], and [`CheapLadder::escalate_after`] are pure functions
//! over priced candidates and an `is_available` predicate. Only
//! [`load_ladder`]/[`discover_priced_candidates`] and [`is_key_configured`] touch
//! config/registry, and each is a thin wrapper over that pure core.

use super::tier::Tier;
use crate::providers::base::Usage;
use crate::providers::canonical::{cost_of, maybe_get_canonical_model};

// ── Config keys (all optional; unset ⇒ the derived default) ───────────────────

/// Override the always-available anchor's provider.
pub const KEY_CHEAP_ANCHOR_PROVIDER: &str = "PERMAGENT_CHEAP_ANCHOR_PROVIDER";
/// Override the always-available anchor's model.
pub const KEY_CHEAP_ANCHOR_MODEL: &str = "PERMAGENT_CHEAP_ANCHOR_MODEL";
/// Pin the primary cheap rung to a specific provider (bypass the cost-derived
/// choice). Requires [`KEY_CHEAP_PIN_MODEL`] too.
pub const KEY_CHEAP_PIN_PROVIDER: &str = "PERMAGENT_CHEAP_PIN_PROVIDER";
/// The model for a pinned cheap rung (see [`KEY_CHEAP_PIN_PROVIDER`]).
pub const KEY_CHEAP_PIN_MODEL: &str = "PERMAGENT_CHEAP_PIN_MODEL";
/// Optional: the env var that gates a pinned rung. Defaults to the pinned
/// provider's declarative `api_key_env`.
pub const KEY_CHEAP_PIN_KEY_ENV: &str = "PERMAGENT_CHEAP_PIN_KEY_ENV";

// ── Known shipped cheap providers (documentation / regression references) ─────
//
// These name the cheap-cloud providers Permagent ships pricing + a declarative
// JSON for. They are NOT used to build the ladder — the ladder is derived from
// the configured provider surface + pricing (see the module note). They remain
// as stable references so the declarative-config guard tests can assert each
// shipped JSON's id / key-env / default model still agree with the router's
// expectations, and so `shipped_cheap_providers_are_priced` can prove they price.

/// The MiniMax declarative provider id (see `providers/declarative/minimax.json`).
pub const PROVIDER_MINIMAX: &str = "minimax";
/// The Moonshot/Kimi declarative provider id (see `providers/declarative/moonshot.json`).
pub const PROVIDER_MOONSHOT: &str = "moonshot";
/// The Anthropic provider id — the anchor rides the operator's main auth.
pub const PROVIDER_ANTHROPIC: &str = "anthropic";

/// MiniMax's headline cheap model — matches `minimax.json` and the priced
/// canonical row `minimax/MiniMax-M2.5`.
pub const DEFAULT_MINIMAX_MODEL: &str = "MiniMax-M2.5";
/// The env var whose presence activates the MiniMax declarative provider.
pub const DEFAULT_MINIMAX_KEY_ENV: &str = "MINIMAX_API_KEY";

/// Kimi's headline cheap model — matches `moonshot.json` and the priced canonical
/// row `moonshot/kimi-k2.5` (the cheapest flagship K2 line).
pub const DEFAULT_KIMI_MODEL: &str = "kimi-k2.5";
/// The env var whose presence activates the Moonshot/Kimi declarative provider.
pub const DEFAULT_KIMI_KEY_ENV: &str = "MOONSHOT_API_KEY";

// ── Anchor defaults ───────────────────────────────────────────────────────────

/// The anchor provider — always available (rides the main Anthropic auth), so a
/// cheap-cloud unit of work never strands even with no cheap keys set.
pub const DEFAULT_ANCHOR_PROVIDER: &str = PROVIDER_ANTHROPIC;
/// The anchor model — byte-identical to `packs::DEFAULT_MECHANICAL_MODEL` so the
/// CheapCloud tier keeps its current behavior until a cheaper provider is keyed.
pub const DEFAULT_ANCHOR_MODEL: &str = "claude-haiku-4-5-20251001";

// ── The reference workload used to RANK candidates by cost ────────────────────

/// The fixed comparison workload: pure fresh input + output, no cache, so ranking
/// reflects the headline per-token rates and is independent of any real
/// conversation's cache state. 1M in + 1M out.
const REF_INPUT: i32 = 1_000_000;
const REF_OUTPUT: i32 = 1_000_000;

fn reference_workload() -> Usage {
    Usage::new(Some(REF_INPUT), Some(REF_OUTPUT), None)
}

/// The canonical cost of one `provider`/`model` for the reference workload, or
/// `None` when the pricing table has no computable price for it (unpriced models
/// are not eligible cheap-tier candidates — a `$0` estimate would corrupt both
/// the ranking and the cost ledger). This is the single ranking key; it reads the
/// same [`crate::providers::canonical`] table `cost_of` bills against.
pub fn reference_cost_for(provider: &str, model: &str) -> Option<f64> {
    let m = maybe_get_canonical_model(provider, model)?;
    cost_of(&reference_workload(), &m.cost)
}

// ── A cheap-cloud candidate ──────────────────────────────────────────────────

/// One rung of the cheap-cloud ladder: a concrete provider+model, the API-key
/// env var whose presence means "the operator configured this provider", and
/// whether it is the always-available anchor (which rides the main provider auth
/// and so needs no key of its own).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheapCandidate {
    pub provider: String,
    pub model: String,
    /// The env var that activates this provider (the declarative config's
    /// `api_key_env`). Empty for the anchor.
    pub key_env: String,
    /// The guaranteed fallback — chosen when no cheaper keyed candidate is
    /// available, and always reachable by escalation.
    pub anchor: bool,
}

impl CheapCandidate {
    pub fn keyed(provider: &str, model: &str, key_env: &str) -> Self {
        Self {
            provider: provider.to_string(),
            model: model.to_string(),
            key_env: key_env.to_string(),
            anchor: false,
        }
    }

    pub fn anchor(provider: &str, model: &str) -> Self {
        Self {
            provider: provider.to_string(),
            model: model.to_string(),
            key_env: String::new(),
            anchor: true,
        }
    }

    /// Whether this candidate can run given a key-availability check. The anchor
    /// is always available; a keyed candidate is available iff its key is set.
    pub fn is_available(&self, is_key_set: &impl Fn(&str) -> bool) -> bool {
        self.anchor || is_key_set(&self.key_env)
    }
}

/// A configured, priced cheap-cloud candidate discovered from the provider
/// surface: a concrete provider+model, the env var that activates it, and its
/// canonical cost for the reference workload (USD; lower = cheaper). This is the
/// pure input to [`build_ladder`] — it carries the cost so the builder never has
/// to touch the pricing table (keeping the ranking testable without IO).
#[derive(Debug, Clone, PartialEq)]
pub struct PricedCandidate {
    pub provider: String,
    pub model: String,
    pub key_env: String,
    pub cost: f64,
}

impl PricedCandidate {
    pub fn new(provider: &str, model: &str, key_env: &str, cost: f64) -> Self {
        Self {
            provider: provider.to_string(),
            model: model.to_string(),
            key_env: key_env.to_string(),
            cost,
        }
    }
}

// ── The ladder ───────────────────────────────────────────────────────────────

/// The cheap-cloud provider ladder, cheapest-first. DERIVED from pricing +
/// configured providers by [`build_ladder`] / [`load_ladder`] — not a fixed list.
/// Invariant: the last rung is the anchor, so [`CheapLadder::select`] always
/// returns something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheapLadder {
    /// Cheapest-first; the final entry is the anchor.
    pub candidates: Vec<CheapCandidate>,
}

impl Default for CheapLadder {
    /// The ladder with NO cheap providers configured: just the always-available
    /// anchor (Haiku). There is no hardcoded provider list — the full ladder is
    /// DERIVED from the configured, priced provider surface by [`build_ladder`] /
    /// [`load_ladder`]. With nothing configured, the CheapCloud tier is exactly
    /// the anchor, preserving today's behavior.
    fn default() -> Self {
        Self {
            candidates: vec![default_anchor()],
        }
    }
}

/// The default always-available anchor (Anthropic / Haiku 4.5).
pub fn default_anchor() -> CheapCandidate {
    CheapCandidate::anchor(DEFAULT_ANCHOR_PROVIDER, DEFAULT_ANCHOR_MODEL)
}

impl CheapLadder {
    /// The always-available anchor rung (the last candidate).
    pub fn anchor(&self) -> &CheapCandidate {
        self.candidates
            .last()
            .expect("cheap ladder always has an anchor")
    }

    /// The cheapest candidate the operator has configured. Walks cheapest-first
    /// and returns the first available rung; the anchor guarantees a result.
    /// This is the "cheapest but required path" pick for the CheapCloud tier.
    pub fn select(&self, is_key_set: &impl Fn(&str) -> bool) -> &CheapCandidate {
        self.candidates
            .iter()
            .find(|c| c.is_available(is_key_set))
            .unwrap_or_else(|| self.anchor())
    }

    /// The next rung to try after `current` fails/times out: the next *dearer*
    /// available candidate (walk up the price-ordered list toward the anchor).
    /// `None` means the cheap tier is exhausted — the caller then escalates the
    /// tier itself to the frontier via [`super::tier::next_after`]
    /// (`CheapCloud` → `Frontier`), reusing the existing #249/#720 escalation one
    /// rung further down.
    ///
    /// If `current` is not on this ladder (e.g. an operator hard-pinned a model
    /// out-of-band), escalation is exhausted immediately — fail up to the tier
    /// ladder rather than guess a lateral move.
    pub fn escalate_after(
        &self,
        current: &CheapCandidate,
        is_key_set: &impl Fn(&str) -> bool,
    ) -> Option<&CheapCandidate> {
        let idx = self.candidates.iter().position(|c| c == current)?;
        self.candidates[idx + 1..]
            .iter()
            .find(|c| c.is_available(is_key_set))
    }
}

/// Build the cheap-cloud ladder, data-driven, from discovered priced candidates
/// and the always-available anchor. **Pure** — all pricing is already folded into
/// each candidate's `cost`, so this is unit-testable without config or network.
///
/// - When `anchor_cost` is known, keep only candidates **strictly cheaper** than
///   it (the anchor is the ceiling of the cheap tier — a model at or above its
///   price offers no saving over the reliable, free-to-reach anchor, and would
///   break the "escalate toward the anchor" invariant). When `anchor_cost` is
///   `None` (the anchor is unpriced), all candidates are kept — the ordering
///   cannot be proven, so trust the operator's configured cheap providers.
/// - Sort ascending by cost; ties broken by `(provider, model)` for a
///   deterministic, reproducible order.
/// - Dedupe by `(provider, model)` (a custom JSON may restate a fixed one).
/// - Append the anchor last, preserving the invariant that the final rung is the
///   always-available anchor.
pub fn build_ladder(
    priced: Vec<PricedCandidate>,
    anchor: CheapCandidate,
    anchor_cost: Option<f64>,
) -> CheapLadder {
    let mut kept: Vec<PricedCandidate> = priced
        .into_iter()
        .filter(|c| match anchor_cost {
            Some(cap) => c.cost < cap,
            None => true,
        })
        .collect();

    kept.sort_by(|a, b| {
        a.cost
            .partial_cmp(&b.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.provider.cmp(&b.provider))
            .then_with(|| a.model.cmp(&b.model))
    });

    let mut seen: Vec<(String, String)> = Vec::new();
    let mut candidates: Vec<CheapCandidate> = Vec::with_capacity(kept.len() + 1);
    for c in kept {
        let key = (c.provider.clone(), c.model.clone());
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        candidates.push(CheapCandidate::keyed(&c.provider, &c.model, &c.key_env));
    }
    candidates.push(anchor);
    CheapLadder { candidates }
}

// ── IO wrappers over the pure core ────────────────────────────────────────────

/// Discover every configured, priced cheap-cloud candidate from the declarative
/// provider surface. A provider counts as *configured* only when it requires auth
/// (`api_key_env` non-empty) AND that key is set — a keyless/local provider's
/// reachability is not queryable here without a network probe, so it is not
/// treated as a cheap-cloud candidate (its $0 local role belongs to the LocalFree
/// tier). Each of a configured provider's models is priced via
/// [`reference_cost_for`]; unpriced models are dropped. Thin IO wrapper — the
/// ranking/selection it feeds is the pure [`build_ladder`].
pub fn discover_priced_candidates(is_key_set: &impl Fn(&str) -> bool) -> Vec<PricedCandidate> {
    use crate::config::declarative_providers::all_declarative_provider_configs;

    let mut out = Vec::new();
    for provider in all_declarative_provider_configs() {
        let key_env = provider.api_key_env.trim();
        if key_env.is_empty() || !is_key_set(key_env) {
            continue;
        }
        for model in &provider.models {
            if let Some(cost) = reference_cost_for(&provider.name, &model.name) {
                out.push(PricedCandidate::new(
                    &provider.name,
                    &model.name,
                    key_env,
                    cost,
                ));
            }
        }
    }
    out
}

fn read_param(key: &str) -> Option<String> {
    crate::config::Config::global()
        .get_param::<String>(key)
        .ok()
        .filter(|s| !s.trim().is_empty())
}

/// The always-available anchor, honoring the `PERMAGENT_CHEAP_ANCHOR_*` overrides.
pub fn load_anchor() -> CheapCandidate {
    let provider = read_param(KEY_CHEAP_ANCHOR_PROVIDER)
        .unwrap_or_else(|| DEFAULT_ANCHOR_PROVIDER.to_string());
    let model =
        read_param(KEY_CHEAP_ANCHOR_MODEL).unwrap_or_else(|| DEFAULT_ANCHOR_MODEL.to_string());
    CheapCandidate::anchor(&provider, &model)
}

/// The operator's manual pin, if set AND usable: `[pinned]` bypasses the
/// cost-derived choice. The pin's key-env is `PERMAGENT_CHEAP_PIN_KEY_ENV`, else
/// the pinned provider's declarative `api_key_env`. Honored only when that key is
/// configured — a pin whose key is absent returns `None`, so the caller falls
/// back to automatic derivation rather than stranding on an unreachable pin.
pub fn load_pin(is_key_set: &impl Fn(&str) -> bool) -> Option<CheapCandidate> {
    let provider = read_param(KEY_CHEAP_PIN_PROVIDER)?;
    let model = read_param(KEY_CHEAP_PIN_MODEL)?;
    let key_env = read_param(KEY_CHEAP_PIN_KEY_ENV)
        .or_else(|| declarative_key_env_for(&provider))
        .unwrap_or_default();
    if key_env.is_empty() || !is_key_set(&key_env) {
        return None;
    }
    Some(CheapCandidate::keyed(&provider, &model, &key_env))
}

/// The `api_key_env` of a declarative provider by id, if any.
fn declarative_key_env_for(provider: &str) -> Option<String> {
    use crate::config::declarative_providers::all_declarative_provider_configs;
    all_declarative_provider_configs()
        .into_iter()
        .find(|c| c.name == provider)
        .map(|c| c.api_key_env)
        .filter(|k| !k.trim().is_empty())
}

/// Load the cheap-cloud ladder from config + the configured provider surface,
/// cheapest-first. Thin IO wrapper over [`build_ladder`]:
/// 1. resolve the retunable anchor (`PERMAGENT_CHEAP_ANCHOR_*`);
/// 2. if the operator pinned a provider (`PERMAGENT_CHEAP_PIN_*`) and its key is
///    set, honor `[pin, anchor]`;
/// 3. otherwise discover every configured, priced candidate and rank it against
///    the anchor's price.
pub fn load_ladder() -> CheapLadder {
    let is_key_set = |k: &str| is_key_configured(k);
    let anchor = load_anchor();

    if let Some(pin) = load_pin(&is_key_set) {
        return CheapLadder {
            candidates: vec![pin, anchor],
        };
    }

    let anchor_cost = reference_cost_for(&anchor.provider, &anchor.model);
    let priced = discover_priced_candidates(&is_key_set);
    build_ladder(priced, anchor, anchor_cost)
}

/// Whether a provider's API key is configured — the availability signal for the
/// ladder's keyed rungs. A key counts as set if it is present as a secret or
/// param in the layered [`crate::config::Config`], or in the process env (so the
/// declarative-provider activation path and the router agree). Thin IO wrapper
/// over the pure selection core; pass `|k| is_key_configured(k)` to
/// [`CheapLadder::select`] / [`CheapLadder::escalate_after`].
pub fn is_key_configured(key_env: &str) -> bool {
    if key_env.is_empty() {
        return false;
    }
    let cfg = crate::config::Config::global();
    let non_empty = |v: String| (!v.trim().is_empty()).then_some(());
    cfg.get_secret::<String>(key_env)
        .ok()
        .and_then(non_empty)
        .or_else(|| cfg.get_param::<String>(key_env).ok().and_then(non_empty))
        .or_else(|| std::env::var(key_env).ok().and_then(non_empty))
        .is_some()
}

/// The tier a mesh/local failure degrades to and the tier this ladder concretizes
/// — restated here so callers wiring the ladder don't have to reach across
/// modules. Always [`Tier::CheapCloud`].
pub const CHEAP_TIER: Tier = Tier::CheapCloud;

#[cfg(test)]
mod tests {
    use super::*;

    /// A key-availability predicate backed by a fixed allow-set (no env, so the
    /// test is pure and parallel-safe). Owns its set so it can be bound and
    /// reused across escalation steps without borrowing a temporary.
    fn available(set: &[&str]) -> impl Fn(&str) -> bool {
        let owned: Vec<String> = set.iter().map(|s| s.to_string()).collect();
        move |k| owned.iter().any(|s| s == k)
    }

    fn none() -> impl Fn(&str) -> bool {
        |_| false
    }

    /// The default anchor's canonical cost (used as the ceiling in most tests).
    fn anchor_cost() -> f64 {
        reference_cost_for(DEFAULT_ANCHOR_PROVIDER, DEFAULT_ANCHOR_MODEL)
            .expect("the anchor model must be priced")
    }

    // A few synthetic priced candidates, well below the anchor, for the pure
    // ranking/selection tests (cost is what matters here, not real rows).
    fn cheap_a() -> PricedCandidate {
        PricedCandidate::new("prov_a", "model-a", "A_KEY", 1.0)
    }
    fn cheap_b() -> PricedCandidate {
        PricedCandidate::new("prov_b", "model-b", "B_KEY", 2.0)
    }
    fn cheap_c() -> PricedCandidate {
        PricedCandidate::new("prov_c", "model-c", "C_KEY", 3.0)
    }

    // ── build_ladder shape + invariants ───────────────────────────────────

    #[test]
    fn empty_candidate_set_is_just_the_anchor() {
        // No cheap provider configured/priced → the ladder is the anchor alone,
        // and selection returns it. This is today's behavior, preserved.
        let l = build_ladder(vec![], default_anchor(), Some(anchor_cost()));
        assert_eq!(l.candidates.len(), 1);
        assert!(l.anchor().anchor);
        assert_eq!(l.anchor().provider, DEFAULT_ANCHOR_PROVIDER);
        assert_eq!(l.anchor().model, DEFAULT_ANCHOR_MODEL);
        assert_eq!(l.select(&none()), l.anchor());
        // CheapLadder::default() agrees.
        assert_eq!(CheapLadder::default(), l);
    }

    #[test]
    fn the_last_rung_is_always_the_anchor() {
        let l = build_ladder(
            vec![cheap_c(), cheap_a(), cheap_b()],
            default_anchor(),
            Some(anchor_cost()),
        );
        assert!(l.candidates.last().unwrap().anchor);
        assert_eq!(l.candidates.last().unwrap(), l.anchor());
    }

    #[test]
    fn candidates_are_sorted_cheapest_first_regardless_of_input_order() {
        // Fed out of order; the derived ladder is cost-ascending, anchor last.
        let l = build_ladder(
            vec![cheap_c(), cheap_a(), cheap_b()],
            default_anchor(),
            Some(anchor_cost()),
        );
        let order: Vec<&str> = l.candidates.iter().map(|c| c.provider.as_str()).collect();
        assert_eq!(
            order,
            ["prov_a", "prov_b", "prov_c", DEFAULT_ANCHOR_PROVIDER]
        );
    }

    // ── Cheapest-adequate selection (the genuinely cheapest) ───────────────

    #[test]
    fn selects_the_genuinely_cheapest_configured_candidate() {
        let l = build_ladder(
            vec![cheap_a(), cheap_b(), cheap_c()],
            default_anchor(),
            Some(anchor_cost()),
        );
        // All three keyed → the cheapest (prov_a @ $1.0) wins.
        let picked = l.select(&available(&["A_KEY", "B_KEY", "C_KEY"]));
        assert_eq!(picked.provider, "prov_a");
    }

    #[test]
    fn adding_a_cheaper_configured_priced_model_changes_the_selection() {
        // THE POINT OF THE FEATURE — generality. Start with prov_b as the only
        // configured cheap provider; it is selected.
        let base = build_ladder(
            vec![cheap_b(), cheap_c()],
            default_anchor(),
            Some(anchor_cost()),
        );
        assert_eq!(
            base.select(&available(&["B_KEY", "C_KEY"])).provider,
            "prov_b"
        );

        // Now the operator adds a NEW, cheaper priced+configured provider
        // (prov_a @ $1.0 < prov_b @ $2.0). With ZERO change to the selection
        // logic, it becomes the preferred cheap tier.
        let grown = build_ladder(
            vec![cheap_a(), cheap_b(), cheap_c()],
            default_anchor(),
            Some(anchor_cost()),
        );
        assert_eq!(
            grown
                .select(&available(&["A_KEY", "B_KEY", "C_KEY"]))
                .provider,
            "prov_a",
            "a newly-added cheaper configured provider must auto-win the cheap tier"
        );
    }

    #[test]
    fn an_unconfigured_cheaper_model_is_skipped() {
        // prov_a is cheaper but its key is NOT set → not selectable; the cheapest
        // AVAILABLE (prov_b) is chosen instead. No key ⇒ never selected.
        let l = build_ladder(
            vec![cheap_a(), cheap_b(), cheap_c()],
            default_anchor(),
            Some(anchor_cost()),
        );
        let picked = l.select(&available(&["B_KEY", "C_KEY"])); // A_KEY absent
        assert_eq!(picked.provider, "prov_b");
    }

    #[test]
    fn no_cheap_keys_selects_the_anchor_even_when_cheaper_rungs_exist() {
        // Rungs exist on the ladder but none is keyed → the always-available
        // anchor is selected (today's behavior).
        let l = build_ladder(
            vec![cheap_a(), cheap_b()],
            default_anchor(),
            Some(anchor_cost()),
        );
        let picked = l.select(&none());
        assert!(picked.anchor);
        assert_eq!(picked.provider, DEFAULT_ANCHOR_PROVIDER);
        assert_eq!(picked.model, DEFAULT_ANCHOR_MODEL);
    }

    // ── The anchor is the ceiling of the cheap tier ────────────────────────

    #[test]
    fn a_candidate_dearer_than_the_anchor_is_dropped() {
        // A configured, priced provider that costs MORE than the anchor is not a
        // "cheap" rung — it must not displace or trail the anchor. Dropped.
        let cap = anchor_cost();
        let too_dear = PricedCandidate::new("prov_dear", "big", "DEAR_KEY", cap + 1.0);
        let l = build_ladder(vec![cheap_a(), too_dear], default_anchor(), Some(cap));
        // Only the genuinely-cheap rung + the anchor remain.
        assert_eq!(l.candidates.len(), 2);
        assert_eq!(l.candidates[0].provider, "prov_a");
        assert!(l.candidates[1].anchor);
        // Even with the dear provider keyed, the anchor caps the tier.
        assert_eq!(
            l.select(&available(&["DEAR_KEY"])).provider,
            DEFAULT_ANCHOR_PROVIDER
        );
    }

    #[test]
    fn unpriced_anchor_keeps_all_candidates() {
        // If the anchor has no known price, we cannot prove the ceiling, so every
        // discovered priced candidate is kept (best effort), anchor still last.
        let l = build_ladder(vec![cheap_a(), cheap_b()], default_anchor(), None);
        assert_eq!(l.candidates.len(), 3);
        assert_eq!(l.candidates[0].provider, "prov_a");
        assert!(l.candidates.last().unwrap().anchor);
    }

    // ── Ties are deterministic ─────────────────────────────────────────────

    #[test]
    fn ties_break_deterministically_by_provider_then_model() {
        // Two candidates at the SAME cost → stable order by (provider, model), so
        // the derived ladder is reproducible run to run.
        let t1 = PricedCandidate::new("zeta", "m", "Z_KEY", 1.5);
        let t2 = PricedCandidate::new("alpha", "m", "AL_KEY", 1.5);
        let t3 = PricedCandidate::new("alpha", "a", "AL_KEY", 1.5);
        let l = build_ladder(vec![t1, t2, t3], default_anchor(), Some(anchor_cost()));
        let order: Vec<(&str, &str)> = l
            .candidates
            .iter()
            .filter(|c| !c.anchor)
            .map(|c| (c.provider.as_str(), c.model.as_str()))
            .collect();
        assert_eq!(order, [("alpha", "a"), ("alpha", "m"), ("zeta", "m")]);
    }

    #[test]
    fn duplicate_provider_model_is_deduped() {
        // A custom JSON restating a fixed provider's row must not double the rung.
        let dup1 = PricedCandidate::new("prov_a", "model-a", "A_KEY", 1.0);
        let dup2 = PricedCandidate::new("prov_a", "model-a", "A_KEY", 1.0);
        let l = build_ladder(vec![dup1, dup2], default_anchor(), Some(anchor_cost()));
        assert_eq!(l.candidates.len(), 2); // one rung + anchor
    }

    // ── Price-ordered escalation toward the anchor, then tier-up ───────────

    #[test]
    fn escalation_walks_up_the_price_order_toward_the_anchor() {
        let l = build_ladder(
            vec![cheap_a(), cheap_b(), cheap_c()],
            default_anchor(),
            Some(anchor_cost()),
        );
        let all = available(&["A_KEY", "B_KEY", "C_KEY"]);

        // cheapest (a) fails → next dearer available (b) → then (c) → anchor.
        let a = &l.candidates[0];
        assert_eq!(a.provider, "prov_a");
        let after_a = l.escalate_after(a, &all).unwrap();
        assert_eq!(after_a.provider, "prov_b");
        let after_b = l.escalate_after(after_a, &all).unwrap();
        assert_eq!(after_b.provider, "prov_c");
        let after_c = l.escalate_after(after_b, &all).unwrap();
        assert!(after_c.anchor);
        // Anchor exhausted → tier escalation (handled by the caller).
        assert!(l.escalate_after(l.anchor(), &all).is_none());
    }

    #[test]
    fn escalation_skips_unavailable_rungs() {
        // Only A and C keyed: A fails → skip unkeyed B → C.
        let l = build_ladder(
            vec![cheap_a(), cheap_b(), cheap_c()],
            default_anchor(),
            Some(anchor_cost()),
        );
        let a = &l.candidates[0];
        let after = l
            .escalate_after(a, &available(&["A_KEY", "C_KEY"]))
            .unwrap();
        assert_eq!(after.provider, "prov_c");
    }

    #[test]
    fn end_to_end_cheap_path_then_tier_escalation() {
        // The full "cheapest but required path, escalate on failure" story:
        // start cheapest keyed → walk the cheap ladder → then the TIER ladder.
        use crate::cost_router::tier::{next_after, Attempt, Next, Tier};

        let l = build_ladder(
            vec![cheap_a(), cheap_b()],
            default_anchor(),
            Some(anchor_cost()),
        );
        let both = available(&["A_KEY", "B_KEY"]);

        let start = l.select(&both);
        assert_eq!(start.provider, "prov_a");
        let step2 = l.escalate_after(start, &both).unwrap();
        assert_eq!(step2.provider, "prov_b");
        let step3 = l.escalate_after(step2, &both).unwrap();
        assert!(step3.anchor);

        // Cheap tier exhausted → reuse the existing tier escalation to Frontier.
        assert!(l.escalate_after(step3, &both).is_none());
        assert_eq!(
            next_after(CHEAP_TIER, Attempt::Failed),
            Next::Escalate(Tier::Frontier)
        );
    }

    // ── A pinned rung is selected first (pin semantics, pure) ──────────────

    #[test]
    fn a_pinned_rung_is_preferred_over_the_anchor_when_keyed() {
        // load_pin builds `[pin, anchor]`; the pin wins when its key is set and
        // falls through to the anchor when it is not.
        let pinned = CheapCandidate::keyed("prov_pin", "pinned-model", "PIN_KEY");
        let l = CheapLadder {
            candidates: vec![pinned, default_anchor()],
        };
        assert_eq!(l.select(&available(&["PIN_KEY"])).provider, "prov_pin");
        assert!(l.select(&none()).anchor);
    }

    // ── The ranking is the REAL canonical cost order (generality on live data)

    #[test]
    fn shipped_cheap_providers_are_priced() {
        // The shipped cheap providers must have a computable cost, or the ledger
        // would silently record them as an estimated $0 and the ranking would be
        // wrong. 1M in + 1M out — MiniMax M2.5: 0.30+1.20=1.50; Kimi k2.5: 0.60+3.00=3.60.
        let minimax = reference_cost_for(PROVIDER_MINIMAX, DEFAULT_MINIMAX_MODEL)
            .expect("minimax must be priced");
        let kimi =
            reference_cost_for(PROVIDER_MOONSHOT, DEFAULT_KIMI_MODEL).expect("kimi must be priced");
        assert!((minimax - 1.50).abs() < 1e-9);
        assert!((kimi - 3.60).abs() < 1e-9);
    }

    #[test]
    fn derived_ladder_orders_shipped_providers_by_real_canonical_cost() {
        // Build the ladder from the REAL canonical costs of the shipped cheap
        // providers (the generic path, on live pricing): MiniMax < Kimi < Haiku
        // anchor. Proves the derivation ranks by the same cost `cost_of` bills.
        let minimax_cost = reference_cost_for(PROVIDER_MINIMAX, DEFAULT_MINIMAX_MODEL).unwrap();
        let kimi_cost = reference_cost_for(PROVIDER_MOONSHOT, DEFAULT_KIMI_MODEL).unwrap();
        let anchor = default_anchor();
        let anchor_c = anchor_cost();

        // Both shipped providers are strictly cheaper than the Haiku anchor.
        assert!(
            minimax_cost < anchor_c,
            "MiniMax must be cheaper than the anchor"
        );
        assert!(kimi_cost < anchor_c, "Kimi must be cheaper than the anchor");

        let l = build_ladder(
            vec![
                PricedCandidate::new(
                    PROVIDER_MOONSHOT,
                    DEFAULT_KIMI_MODEL,
                    DEFAULT_KIMI_KEY_ENV,
                    kimi_cost,
                ),
                PricedCandidate::new(
                    PROVIDER_MINIMAX,
                    DEFAULT_MINIMAX_MODEL,
                    DEFAULT_MINIMAX_KEY_ENV,
                    minimax_cost,
                ),
            ],
            anchor,
            Some(anchor_c),
        );
        let order: Vec<&str> = l.candidates.iter().map(|c| c.provider.as_str()).collect();
        assert_eq!(
            order,
            [PROVIDER_MINIMAX, PROVIDER_MOONSHOT, DEFAULT_ANCHOR_PROVIDER]
        );

        // Both keyed → MiniMax (cheapest) is the cheapest-but-required pick.
        let both = available(&[DEFAULT_MINIMAX_KEY_ENV, DEFAULT_KIMI_KEY_ENV]);
        assert_eq!(l.select(&both).provider, PROVIDER_MINIMAX);
    }

    #[test]
    fn full_tier_cost_order_holds_on_real_pricing() {
        // Ollama (local $0) < MiniMax < Kimi < Haiku (cheap cloud) < Sonnet (mid)
        // < Opus (frontier) — proven against the real canonical pricing.
        let ollama = 0.0;
        let minimax = reference_cost_for(PROVIDER_MINIMAX, DEFAULT_MINIMAX_MODEL).unwrap();
        let kimi = reference_cost_for(PROVIDER_MOONSHOT, DEFAULT_KIMI_MODEL).unwrap();
        let haiku = reference_cost_for("anthropic", "claude-haiku-4-5-20251001").unwrap();
        let sonnet = reference_cost_for("anthropic", "claude-sonnet-5").unwrap();
        let opus = reference_cost_for("anthropic", "claude-opus-4-8").unwrap();

        assert!(ollama < minimax, "local free is cheapest");
        assert!(minimax < kimi, "MiniMax < Kimi");
        assert!(kimi < haiku, "Kimi < Haiku");
        assert!(haiku < sonnet, "cheap cloud < Sonnet (mid)");
        assert!(sonnet < opus, "Sonnet (mid) < Opus (frontier)");
    }

    #[test]
    fn unpriced_model_has_no_reference_cost() {
        // A model with no canonical row is unpriced → not an eligible candidate.
        assert!(reference_cost_for("minimax", "no-such-model-xyz").is_none());
    }
}
