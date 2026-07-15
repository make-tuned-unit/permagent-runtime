//! The cheap-cloud tier's provider ladder — "the cheapest cloud model that can
//! do the work, and only escalate when it can't."
//!
//! The tier ladder in [`super::tier`] has a single [`Tier::CheapCloud`] rung
//! between local Ollama and the frontier. This module turns that one abstract
//! rung into a *ranked set* of concrete cheap-cloud providers — **MiniMax**,
//! **Kimi (Moonshot)**, and **Haiku 4.5** — ordered cheapest-first, and picks
//! the cheapest one the operator has actually configured (its API key is set).
//!
//! That is Jesse's "choose the cheapest but required path" applied *within* the
//! cheap tier: the router never pays for Haiku when a cheaper keyed provider
//! (MiniMax/Kimi) can do the same mechanical work, and never strands the task
//! when none is keyed — Haiku, which rides the operator's existing Anthropic
//! auth, is the always-available **anchor**. On a failed attempt the ladder
//! walks toward the anchor (a dearer, more capable cheap model) and only once it
//! is exhausted does the caller escalate the *tier* to the frontier via
//! [`super::tier::next_after`] — reusing the existing #249/#720 escalation
//! unchanged, one rung further down.
//!
//! ## The ladder, cheapest-first (the default; every field is retunable)
//!
//! | Rung       | Default provider / model        | API-key env      | Notes                          |
//! |------------|---------------------------------|------------------|--------------------------------|
//! | 0 cheapest | `minimax` / `MiniMax-M2.5`      | `MINIMAX_API_KEY`  | in $0.30 / out $1.20 per 1M    |
//! | 1          | `moonshot` / `kimi-k2.5` (Kimi) | `MOONSHOT_API_KEY` | in $0.60 / out $3.00 per 1M    |
//! | 2 anchor   | `anthropic` / Haiku 4.5         | — (main auth)    | in $1.00 / out $5.00 per 1M    |
//!
//! The ordering is grounded in the published cheap rates now carried by
//! [`crate::providers::canonical`] pricing, so `cost_of` agrees with the ladder
//! (see the `cheap_cloud_ladder_is_ordered_by_real_canonical_cost` test). The
//! anchor row is byte-identical to the existing mechanical pack default in
//! [`super::packs`], so with no cheap keys set the CheapCloud tier resolves to
//! exactly what it does today — no behavior change until a key is supplied.
//!
//! ## Configurable, like the rest of the router
//!
//! Every knob is overridable via `PERMAGENT_CHEAP_*` config/env keys (each
//! candidate's model, its key-env, and the anchor's provider+model), mirroring
//! the pure-core-plus-thin-env-wrapper convention in [`super::budget`] and
//! [`super::packs`]. The Kimi key-env defaults to `MOONSHOT_API_KEY` but an
//! operator who provisioned a `KIMI_API_KEY` can point it there without code.
//!
//! ## The mesh seam below Ollama
//!
//! This ladder is deliberately the *cloud* half of the cheapest-adequate ladder;
//! the tier BELOW local Ollama — the trusted mesh/pool ([`Tier::MeshFree`]) — is
//! the future "open-source LLM hosted across Permagent users" seam, gated and
//! wired in [`super::mesh`] (the #702/#717 seam). A mesh failure already
//! degrades straight to *this* cheap-cloud tier via
//! [`super::mesh::fallback_tier`], so the two halves compose without change here.
//!
//! Pure so it is unit-testable without env or network (env-mutating tests flake
//! under parallel `cargo test`): selection and escalation are pure functions
//! over an `is_available` predicate; only [`load_ladder`] and
//! [`is_key_configured`] touch config, and each is a thin wrapper over a pure
//! core.

use super::tier::Tier;

// ── Config keys (all optional; unset ⇒ the ladder default) ───────────────────

pub const KEY_CHEAP_MINIMAX_MODEL: &str = "PERMAGENT_CHEAP_MINIMAX_MODEL";
pub const KEY_CHEAP_MINIMAX_KEY_ENV: &str = "PERMAGENT_CHEAP_MINIMAX_KEY_ENV";
pub const KEY_CHEAP_KIMI_MODEL: &str = "PERMAGENT_CHEAP_KIMI_MODEL";
pub const KEY_CHEAP_KIMI_KEY_ENV: &str = "PERMAGENT_CHEAP_KIMI_KEY_ENV";
pub const KEY_CHEAP_ANCHOR_PROVIDER: &str = "PERMAGENT_CHEAP_ANCHOR_PROVIDER";
pub const KEY_CHEAP_ANCHOR_MODEL: &str = "PERMAGENT_CHEAP_ANCHOR_MODEL";

// ── Defaults (the cheap-cloud ladder, cheapest-first) ────────────────────────

/// The MiniMax declarative provider id (see `providers/declarative/minimax.json`).
pub const PROVIDER_MINIMAX: &str = "minimax";
/// The Moonshot/Kimi declarative provider id (see `providers/declarative/moonshot.json`).
pub const PROVIDER_MOONSHOT: &str = "moonshot";
/// The Anthropic provider id — the anchor rides the operator's main auth.
pub const PROVIDER_ANTHROPIC: &str = "anthropic";

/// MiniMax's default model — matches `minimax.json` and the priced canonical row
/// `minimax/MiniMax-M2.5`.
pub const DEFAULT_MINIMAX_MODEL: &str = "MiniMax-M2.5";
/// The env var whose presence activates the MiniMax declarative provider.
pub const DEFAULT_MINIMAX_KEY_ENV: &str = "MINIMAX_API_KEY";

/// Kimi's default model — matches `moonshot.json` and the priced canonical row
/// `moonshot/kimi-k2.5` (the cheapest flagship K2 line).
pub const DEFAULT_KIMI_MODEL: &str = "kimi-k2.5";
/// The env var whose presence activates the Moonshot/Kimi declarative provider.
/// An operator with a `KIMI_API_KEY` overrides via [`KEY_CHEAP_KIMI_KEY_ENV`].
pub const DEFAULT_KIMI_KEY_ENV: &str = "MOONSHOT_API_KEY";

/// The anchor provider — always available (rides the main Anthropic auth), so a
/// cheap-cloud unit of work never strands even with no cheap keys set.
pub const DEFAULT_ANCHOR_PROVIDER: &str = PROVIDER_ANTHROPIC;
/// The anchor model — byte-identical to `packs::DEFAULT_MECHANICAL_MODEL` so the
/// CheapCloud tier keeps its current behavior until a cheaper key is supplied.
pub const DEFAULT_ANCHOR_MODEL: &str = "claude-haiku-4-5-20251001";

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
    fn keyed(provider: &str, model: &str, key_env: &str) -> Self {
        Self {
            provider: provider.to_string(),
            model: model.to_string(),
            key_env: key_env.to_string(),
            anchor: false,
        }
    }

    fn anchor(provider: &str, model: &str) -> Self {
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

// ── The ladder ───────────────────────────────────────────────────────────────

/// The cheap-cloud provider ladder, cheapest-first. CONFIGURABLE: the defaults
/// are Jesse's spend/quality call and every field is overridable via the
/// `PERMAGENT_CHEAP_*` keys. Invariant: the last rung is the anchor, so
/// [`CheapLadder::select`] always returns something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheapLadder {
    /// Cheapest-first; the final entry is the anchor.
    pub candidates: Vec<CheapCandidate>,
}

impl Default for CheapLadder {
    /// Cheapest-first: MiniMax · Kimi (Moonshot) · Haiku 4.5 (anchor).
    fn default() -> Self {
        Self {
            candidates: vec![
                CheapCandidate::keyed(
                    PROVIDER_MINIMAX,
                    DEFAULT_MINIMAX_MODEL,
                    DEFAULT_MINIMAX_KEY_ENV,
                ),
                CheapCandidate::keyed(PROVIDER_MOONSHOT, DEFAULT_KIMI_MODEL, DEFAULT_KIMI_KEY_ENV),
                CheapCandidate::anchor(DEFAULT_ANCHOR_PROVIDER, DEFAULT_ANCHOR_MODEL),
            ],
        }
    }
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
    /// available candidate (walk toward the anchor). `None` means the cheap tier
    /// is exhausted — the caller then escalates the tier itself to the frontier
    /// via [`super::tier::next_after`] (`CheapCloud` → `Frontier`), reusing the
    /// existing #249/#720 escalation one rung further down.
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

/// Pure constructor: apply any provided overrides over the ladder defaults.
/// Split from [`load_ladder`] so it is unit-testable without the process-global
/// config (mirrors [`super::packs::packs_from`]).
#[allow(clippy::too_many_arguments)]
pub fn ladder_from(
    minimax_model: Option<String>,
    minimax_key_env: Option<String>,
    kimi_model: Option<String>,
    kimi_key_env: Option<String>,
    anchor_provider: Option<String>,
    anchor_model: Option<String>,
) -> CheapLadder {
    CheapLadder {
        candidates: vec![
            CheapCandidate::keyed(
                PROVIDER_MINIMAX,
                &minimax_model.unwrap_or_else(|| DEFAULT_MINIMAX_MODEL.to_string()),
                &minimax_key_env.unwrap_or_else(|| DEFAULT_MINIMAX_KEY_ENV.to_string()),
            ),
            CheapCandidate::keyed(
                PROVIDER_MOONSHOT,
                &kimi_model.unwrap_or_else(|| DEFAULT_KIMI_MODEL.to_string()),
                &kimi_key_env.unwrap_or_else(|| DEFAULT_KIMI_KEY_ENV.to_string()),
            ),
            CheapCandidate::anchor(
                &anchor_provider.unwrap_or_else(|| DEFAULT_ANCHOR_PROVIDER.to_string()),
                &anchor_model.unwrap_or_else(|| DEFAULT_ANCHOR_MODEL.to_string()),
            ),
        ],
    }
}

/// Load the cheap-cloud ladder from config/env, falling back to Jesse's defaults
/// for any unset key. Thin wrapper over [`ladder_from`].
pub fn load_ladder() -> CheapLadder {
    let cfg = crate::config::Config::global();
    let read = |key: &str| cfg.get_param::<String>(key).ok();
    ladder_from(
        read(KEY_CHEAP_MINIMAX_MODEL),
        read(KEY_CHEAP_MINIMAX_KEY_ENV),
        read(KEY_CHEAP_KIMI_MODEL),
        read(KEY_CHEAP_KIMI_KEY_ENV),
        read(KEY_CHEAP_ANCHOR_PROVIDER),
        read(KEY_CHEAP_ANCHOR_MODEL),
    )
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
    use crate::providers::base::Usage;
    use crate::providers::canonical::{cost_of, maybe_get_canonical_model};

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

    // ── The ladder shape ──────────────────────────────────────────────────

    #[test]
    fn default_ladder_is_minimax_then_kimi_then_haiku_anchor() {
        let l = CheapLadder::default();
        assert_eq!(l.candidates.len(), 3);

        assert_eq!(l.candidates[0].provider, "minimax");
        assert_eq!(l.candidates[0].model, "MiniMax-M2.5");
        assert_eq!(l.candidates[0].key_env, "MINIMAX_API_KEY");
        assert!(!l.candidates[0].anchor);

        assert_eq!(l.candidates[1].provider, "moonshot");
        assert_eq!(l.candidates[1].model, "kimi-k2.5");
        assert_eq!(l.candidates[1].key_env, "MOONSHOT_API_KEY");
        assert!(!l.candidates[1].anchor);

        assert_eq!(l.candidates[2].provider, "anthropic");
        assert_eq!(l.candidates[2].model, "claude-haiku-4-5-20251001");
        assert!(l.candidates[2].anchor);
        // The anchor rides the main auth — no key of its own.
        assert_eq!(l.candidates[2].key_env, "");
        assert_eq!(l.anchor(), &l.candidates[2]);
    }

    // ── Cheapest-adequate selection (test bar: the cheapest keyed provider) ──

    #[test]
    fn no_cheap_keys_selects_the_haiku_anchor() {
        // Preserves today's behavior: CheapCloud → Haiku when nothing else is set.
        let l = CheapLadder::default();
        let picked = l.select(&none());
        assert!(picked.anchor);
        assert_eq!(picked.provider, "anthropic");
        assert_eq!(picked.model, "claude-haiku-4-5-20251001");
    }

    #[test]
    fn only_minimax_keyed_selects_minimax() {
        let l = CheapLadder::default();
        assert_eq!(
            l.select(&available(&["MINIMAX_API_KEY"])).provider,
            "minimax"
        );
    }

    #[test]
    fn only_kimi_keyed_selects_kimi() {
        // MiniMax (cheaper) is unavailable, so the cheapest AVAILABLE is Kimi.
        let l = CheapLadder::default();
        let picked = l.select(&available(&["MOONSHOT_API_KEY"]));
        assert_eq!(picked.provider, "moonshot");
        assert_eq!(picked.model, "kimi-k2.5");
    }

    #[test]
    fn both_cheap_keyed_selects_the_cheaper_minimax() {
        let l = CheapLadder::default();
        let picked = l.select(&available(&["MINIMAX_API_KEY", "MOONSHOT_API_KEY"]));
        assert_eq!(
            picked.provider, "minimax",
            "cheapest-but-required = MiniMax"
        );
    }

    // ── Escalate-on-failure: cheapest cheap → next cheap → anchor → tier up ──

    #[test]
    fn escalation_walks_toward_the_anchor_then_exhausts() {
        let l = CheapLadder::default();
        let both = available(&["MINIMAX_API_KEY", "MOONSHOT_API_KEY"]);

        // MiniMax fails → Kimi (next dearer available).
        let minimax = &l.candidates[0];
        let after_minimax = l.escalate_after(minimax, &both).unwrap();
        assert_eq!(after_minimax.provider, "moonshot");

        // Kimi fails → Haiku anchor.
        let kimi = &l.candidates[1];
        let after_kimi = l.escalate_after(kimi, &both).unwrap();
        assert!(after_kimi.anchor);

        // Anchor fails → nothing left on the cheap tier (caller escalates tier).
        assert!(l.escalate_after(l.anchor(), &both).is_none());
    }

    #[test]
    fn escalation_skips_unavailable_rungs() {
        // Only MiniMax keyed: MiniMax fails → skip unkeyed Kimi → Haiku anchor.
        let l = CheapLadder::default();
        let minimax = &l.candidates[0];
        let after = l
            .escalate_after(minimax, &available(&["MINIMAX_API_KEY"]))
            .unwrap();
        assert!(after.anchor);
    }

    #[test]
    fn end_to_end_cheap_path_then_tier_escalation() {
        // The full "cheapest but required path, escalate on failure" story:
        // start cheapest keyed → walk the cheap ladder → then the TIER ladder.
        use crate::cost_router::tier::{next_after, Attempt, Next, Tier};

        let l = CheapLadder::default();
        let both = available(&["MINIMAX_API_KEY", "MOONSHOT_API_KEY"]);

        let start = l.select(&both);
        assert_eq!(start.provider, "minimax");

        let step2 = l.escalate_after(start, &both).unwrap();
        assert_eq!(step2.provider, "moonshot");

        let step3 = l.escalate_after(step2, &both).unwrap();
        assert!(step3.anchor); // Haiku

        // Cheap tier exhausted → reuse the existing tier escalation to Frontier.
        assert!(l.escalate_after(step3, &both).is_none());
        assert_eq!(
            next_after(CHEAP_TIER, Attempt::Failed),
            Next::Escalate(Tier::Frontier)
        );
    }

    // ── Config is retunable ────────────────────────────────────────────────

    #[test]
    fn empty_overrides_are_exactly_the_defaults() {
        assert_eq!(
            ladder_from(None, None, None, None, None, None),
            CheapLadder::default()
        );
    }

    #[test]
    fn overrides_apply_over_defaults() {
        let l = ladder_from(
            Some("MiniMax-M2.5-highspeed".to_string()),
            None,
            Some("kimi-k2.6".to_string()),
            Some("KIMI_API_KEY".to_string()), // operator provisioned KIMI_API_KEY
            None,
            Some("claude-haiku-4-5".to_string()),
        );
        assert_eq!(l.candidates[0].model, "MiniMax-M2.5-highspeed");
        assert_eq!(l.candidates[0].key_env, "MINIMAX_API_KEY"); // untouched default
        assert_eq!(l.candidates[1].model, "kimi-k2.6");
        assert_eq!(l.candidates[1].key_env, "KIMI_API_KEY");
        assert_eq!(l.candidates[2].model, "claude-haiku-4-5");
        assert!(l.candidates[2].anchor);
        // Retuning the Kimi key-env changes which key activates it.
        assert_eq!(l.select(&available(&["KIMI_API_KEY"])).provider, "moonshot");
    }

    // ── The ladder ordering is the REAL canonical cost order (test bar) ──────

    /// A fixed workload for cost comparison: pure fresh input+output, no cache,
    /// so the ordering reflects the headline rates.
    fn workload() -> Usage {
        Usage::new(Some(1_000_000), Some(1_000_000), None)
    }

    fn cost_for(provider: &str, model: &str) -> f64 {
        let m = maybe_get_canonical_model(provider, model)
            .unwrap_or_else(|| panic!("no canonical pricing for {provider}/{model}"));
        cost_of(&workload(), &m.cost)
            .unwrap_or_else(|| panic!("no computable cost for {provider}/{model}"))
    }

    #[test]
    fn kimi_and_minimax_pricing_makes_cost_of_compute_a_cost() {
        // The test bar: their pricing entries make `cost_of` compute a cost.
        // 1M in + 1M out. MiniMax M2.5: 0.30 + 1.20 = 1.50. Kimi k2.5: 0.60 + 3.00 = 3.60.
        assert!((cost_for("minimax", "MiniMax-M2.5") - 1.50).abs() < 1e-9);
        assert!((cost_for("moonshot", "kimi-k2.5") - 3.60).abs() < 1e-9);
        // The declarative provider names resolve too (moonshot.json / minimax.json).
        assert!(cost_for("moonshot", "kimi-k2.6") > 0.0);
    }

    #[test]
    fn cheap_cloud_ladder_is_ordered_by_real_canonical_cost() {
        // Ollama (local $0) < MiniMax < Kimi < Haiku (cheap cloud) < Sonnet (mid)
        // < Opus (frontier) — the exact ladder ordering the test bar names,
        // proven against the real canonical pricing `cost_of` uses.
        let ollama = 0.0; // local, not chargeable
        let minimax = cost_for("minimax", "MiniMax-M2.5");
        let kimi = cost_for("moonshot", "kimi-k2.5");
        let haiku = cost_for("anthropic", "claude-haiku-4-5-20251001");
        let sonnet = cost_for("anthropic", "claude-sonnet-5");
        let opus = cost_for("anthropic", "claude-opus-4-8");

        assert!(ollama < minimax, "local free is cheapest");
        assert!(minimax < kimi, "MiniMax < Kimi");
        assert!(kimi < haiku, "Kimi < Haiku");
        assert!(haiku < sonnet, "cheap cloud < Sonnet (mid)");
        assert!(sonnet < opus, "Sonnet (mid) < Opus (frontier)");
    }

    #[test]
    fn the_default_ladder_candidates_are_all_priced() {
        // Every default cheap-cloud rung must have a computable cost, or the
        // cost ledger would silently record it as an estimated $0.
        for c in &CheapLadder::default().candidates {
            assert!(
                cost_for(&c.provider, &c.model) > 0.0,
                "{}/{} has no canonical price",
                c.provider,
                c.model
            );
        }
    }
}
