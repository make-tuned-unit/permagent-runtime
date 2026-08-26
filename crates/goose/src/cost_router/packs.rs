//! Tiered model-packs — the tier→concrete-model seam.
//!
//! The router in [`super::tier`] decides an *abstract* [`Tier`] (cheapest
//! adequate rung, escalating on failure). This module turns that decision into
//! a *concrete* provider+model — the piece the router was missing, so a caller
//! can go from "this is mechanical batch work" all the way to "spawn a subagent
//! on `ollama`/`qwen3`".
//!
//! It carries the owner's chosen tiered packs as CONFIGURABLE defaults:
//!
//! | Role         | Default model                 | Where it runs                         |
//! |--------------|-------------------------------|---------------------------------------|
//! | `Edit`       | `claude-sonnet-5`             | the interactive main coding loop      |
//! | `Hard`       | `claude-opus-4-8`             | planning / hard reasoning (Frontier)  |
//! | `Mechanical` | `claude-haiku-4-5-20251001`   | apply-diff/summarize/grep (CheapCloud)|
//! | `Local`      | `qwen3` (ollama, $0)          | on-device mechanical (Local/Mesh)     |
//!
//! Every field is overridable via `PERMAGENT_PACK_*` config/env keys (the
//! defaults are the owner's spend/quality call, surfaced for tuning), mirroring the
//! pure-core-plus-thin-env-wrapper convention in [`super::budget`].
//!
//! Cache discipline (see [`super::cache`]) is why `Edit` is a distinct role
//! rather than a `Tier`: the main loop stays on ONE stable model (the operator's
//! configured strong model, or the `Edit` pack) so its prompt cache stays warm,
//! and cheaper-tier work is dispatched to SEPARATE subagent contexts — never by
//! swapping the live loop's model. So `role_for_tier` never returns `Edit`: the
//! escalation ladder (`Frontier`/`CheapCloud`/`Local`/`Mesh`) is for routed
//! sub-work, while `Edit` names the main loop the router deliberately never
//! offloads.

use super::mesh::MeshGateInputs;
use super::tier::{TaskSignals, Tier};

// ── Config keys (all optional; unset ⇒ the pack default) ─────────────────────

pub const KEY_EDIT_PROVIDER: &str = "PERMAGENT_PACK_EDIT_PROVIDER";
pub const KEY_EDIT_MODEL: &str = "PERMAGENT_PACK_EDIT_MODEL";
pub const KEY_HARD_PROVIDER: &str = "PERMAGENT_PACK_HARD_PROVIDER";
pub const KEY_HARD_MODEL: &str = "PERMAGENT_PACK_HARD_MODEL";
pub const KEY_MECHANICAL_PROVIDER: &str = "PERMAGENT_PACK_MECHANICAL_PROVIDER";
pub const KEY_MECHANICAL_MODEL: &str = "PERMAGENT_PACK_MECHANICAL_MODEL";
pub const KEY_LOCAL_PROVIDER: &str = "PERMAGENT_PACK_LOCAL_PROVIDER";
pub const KEY_LOCAL_MODEL: &str = "PERMAGENT_PACK_LOCAL_MODEL";

// ── Default models (the shipped tiered packs) ────────────────────────────────────

/// The Anthropic provider id (see `providers::anthropic`).
const PROVIDER_ANTHROPIC: &str = "anthropic";
/// The local Ollama provider id (see `providers::ollama`).
const PROVIDER_OLLAMA: &str = "ollama";

/// Planner / hard reasoning — the frontier escalation target.
const DEFAULT_HARD_MODEL: &str = "claude-opus-4-8";
/// Editor — the interactive main coding loop (stable, cache-warm).
const DEFAULT_EDIT_MODEL: &str = "claude-sonnet-5";
/// Apply/mechanical cloud work — apply-diff, summarize, grep-triage.
const DEFAULT_MECHANICAL_MODEL: &str = "claude-haiku-4-5-20251001";
/// Local $0 mechanical — matches `providers::ollama::OLLAMA_DEFAULT_MODEL`.
const DEFAULT_LOCAL_MODEL: &str = "qwen3";

// ── Roles ────────────────────────────────────────────────────────────────────

/// The role a unit of work plays in the coding harness. Fixes which pack (hence
/// which concrete model) runs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// The interactive main coding loop — the "editor". Stable for the loop's
    /// lifetime so its prompt cache stays warm; never reached via `route`.
    Edit,
    /// Planning / hard reasoning / multi-file change — the frontier tier.
    Hard,
    /// Mechanical cloud work a local model can't finish: apply-diff, summarize,
    /// grep-triage. The cheap-cloud escalation target.
    Mechanical,
    /// Local, on-device, $0 mechanical work — the cheapest starting tier.
    Local,
}

impl Role {
    /// Stable label for logs/telemetry (not used to make decisions).
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Edit => "edit",
            Role::Hard => "hard",
            Role::Mechanical => "mechanical",
            Role::Local => "local",
        }
    }
}

/// Which role runs a given tier's work. The mesh tier reuses the local model
/// (same batch model, a pooled endpoint instead of this device). `Edit` is
/// intentionally absent — it is the main loop, not a routed sub-tier.
pub fn role_for_tier(tier: Tier) -> Role {
    match tier {
        Tier::Frontier => Role::Hard,
        Tier::CheapCloud => Role::Mechanical,
        Tier::LocalFree | Tier::MeshFree => Role::Local,
    }
}

// ── A pack: one role's provider+model ────────────────────────────────────────

/// The concrete provider+model that runs one role's work. Directly usable by the
/// subagent seam (`summon`'s `provider`/`model` params) and by
/// `providers::create_with_named_model`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelPack {
    pub provider: String,
    pub model: String,
}

impl ModelPack {
    fn new(provider: &str, model: &str) -> Self {
        Self {
            provider: provider.to_string(),
            model: model.to_string(),
        }
    }

    /// The prompt-cache identity this pack runs under. Caches are provider+model
    /// scoped, so cache-stability questions must be asked with both halves.
    pub fn cache_key(&self) -> crate::cost_router::cache::ModelKey<'_> {
        crate::cost_router::cache::ModelKey::new(&self.provider, &self.model)
    }
}

/// The full set of tiered packs — one per role. CONFIGURABLE: the defaults are
/// The owner's call and every field is overridable via the `PERMAGENT_PACK_*` keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelPacks {
    pub edit: ModelPack,
    pub hard: ModelPack,
    pub mechanical: ModelPack,
    pub local: ModelPack,
}

impl Default for ModelPacks {
    /// The shipped tiered packs: Opus 4.8 (hard) · Sonnet 5 (edit) · Haiku 4.5
    /// (mechanical) · Ollama/qwen3 (local $0).
    fn default() -> Self {
        Self {
            edit: ModelPack::new(PROVIDER_ANTHROPIC, DEFAULT_EDIT_MODEL),
            hard: ModelPack::new(PROVIDER_ANTHROPIC, DEFAULT_HARD_MODEL),
            mechanical: ModelPack::new(PROVIDER_ANTHROPIC, DEFAULT_MECHANICAL_MODEL),
            local: ModelPack::new(PROVIDER_OLLAMA, DEFAULT_LOCAL_MODEL),
        }
    }
}

impl ModelPacks {
    /// The pack for a role.
    pub fn pack_for(&self, role: Role) -> &ModelPack {
        match role {
            Role::Edit => &self.edit,
            Role::Hard => &self.hard,
            Role::Mechanical => &self.mechanical,
            Role::Local => &self.local,
        }
    }

    /// The pack for a tier's work (via `role_for_tier`).
    pub fn pack_for_tier(&self, tier: Tier) -> &ModelPack {
        self.pack_for(role_for_tier(tier))
    }

    /// The concrete model name for a role.
    pub fn model_for(&self, role: Role) -> &str {
        &self.pack_for(role).model
    }

    /// The concrete model name for a tier's work.
    pub fn model_for_tier(&self, tier: Tier) -> &str {
        &self.pack_for_tier(tier).model
    }

    /// The stable main-loop pack (the "editor"). Kept on one model for the loop's
    /// lifetime so the prompt cache stays warm.
    pub fn main_loop(&self) -> &ModelPack {
        &self.edit
    }
}

/// Pure constructor: apply any provided overrides over the pack defaults. Split
/// from [`load_packs`] so it is unit-testable without the process-global config
/// (mirrors [`super::budget::budget_config_from`]).
#[allow(clippy::too_many_arguments)]
pub fn packs_from(
    edit_provider: Option<String>,
    edit_model: Option<String>,
    hard_provider: Option<String>,
    hard_model: Option<String>,
    mechanical_provider: Option<String>,
    mechanical_model: Option<String>,
    local_provider: Option<String>,
    local_model: Option<String>,
) -> ModelPacks {
    let d = ModelPacks::default();
    ModelPacks {
        edit: ModelPack {
            provider: edit_provider.unwrap_or(d.edit.provider),
            model: edit_model.unwrap_or(d.edit.model),
        },
        hard: ModelPack {
            provider: hard_provider.unwrap_or(d.hard.provider),
            model: hard_model.unwrap_or(d.hard.model),
        },
        mechanical: ModelPack {
            provider: mechanical_provider.unwrap_or(d.mechanical.provider),
            model: mechanical_model.unwrap_or(d.mechanical.model),
        },
        local: ModelPack {
            provider: local_provider.unwrap_or(d.local.provider),
            model: local_model.unwrap_or(d.local.model),
        },
    }
}

/// Load the tiered packs from config/env, falling back to the shipped defaults for
/// any unset key. Thin wrapper over [`packs_from`].
pub fn load_packs() -> ModelPacks {
    let cfg = crate::config::Config::global();
    let read = |key: &str| cfg.get_param::<String>(key).ok();
    packs_from(
        read(KEY_EDIT_PROVIDER),
        read(KEY_EDIT_MODEL),
        read(KEY_HARD_PROVIDER),
        read(KEY_HARD_MODEL),
        read(KEY_MECHANICAL_PROVIDER),
        read(KEY_MECHANICAL_MODEL),
        read(KEY_LOCAL_PROVIDER),
        read(KEY_LOCAL_MODEL),
    )
}

// ── Explicit pins (the delegate seam) ────────────────────────────────────────

/// The `packs::Role` a user-facing [`super::recommend::WorkflowRole`] pins
/// through. Every role maps, so "I pinned all four packs" leaves none of them
/// free to route elsewhere; the `Option` is kept for a future role the ladder
/// genuinely has no rung for.
///
/// `Orchestrate` is the tier ladder's `Hard` under its user-facing name (see the
/// note in [`super::role_map`]).
///
/// `Review` pins through `Hard` as well. The ladder has no review rung — it is a
/// cheapest-path escalation ladder and cross-family verification is not a rung on
/// it — but leaving `Review` unmapped would mean an operator who pinned all four
/// packs still had review delegates free to route elsewhere, which is not what
/// "I pinned the models" means to the person who typed it. Both roles are the
/// frontier-judgment tier, so both read the `HARD` pack. An operator who wants
/// review somewhere else sets `PERMAGENT_ROLE_REVIEW_*`, which outranks this.
///
/// The review gate's cross-family diversity preference still runs and still warns
/// when the reviewer shares the author's family; it does not override the pin —
/// an operator's explicit choice is not second-guessed.
pub fn pack_role_for_workflow_role(role: super::recommend::WorkflowRole) -> Option<Role> {
    use super::recommend::WorkflowRole;
    match role {
        WorkflowRole::Orchestrate | WorkflowRole::Review => Some(Role::Hard),
        WorkflowRole::Edit => Some(Role::Edit),
        WorkflowRole::Mechanical => Some(Role::Mechanical),
        WorkflowRole::Local => Some(Role::Local),
    }
}

/// The `(provider_key, model_key)` pair for a role.
pub fn keys_for(role: Role) -> (&'static str, &'static str) {
    match role {
        Role::Edit => (KEY_EDIT_PROVIDER, KEY_EDIT_MODEL),
        Role::Hard => (KEY_HARD_PROVIDER, KEY_HARD_MODEL),
        Role::Mechanical => (KEY_MECHANICAL_PROVIDER, KEY_MECHANICAL_MODEL),
        Role::Local => (KEY_LOCAL_PROVIDER, KEY_LOCAL_MODEL),
    }
}

/// Pure: the pack a role was EXPLICITLY pinned to, or `None`.
///
/// This is deliberately NOT [`load_packs`]: it never falls back to
/// [`ModelPacks::default`], so an unset role yields `None` and the caller falls
/// through instead of silently dispatching to a shipped vendor default. Both the
/// provider AND the model key must be present and non-empty — a half-configured
/// pin is treated as unset, matching [`super::role_map::resolve_role_model`].
///
/// Added 2026-08-25: the `PERMAGENT_PACK_*` keys existed but nothing on the
/// delegate dispatch path read them, so an operator who pinned every pack to a
/// cheaper model still had subagents routed elsewhere. See
/// [`super::delegate`].
pub fn configured_pack_pin(role: Role, read: impl Fn(&str) -> Option<String>) -> Option<ModelPack> {
    let (pk, mk) = keys_for(role);
    let non_empty = |k: &str| {
        read(k)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    Some(ModelPack {
        provider: non_empty(pk)?,
        model: non_empty(mk)?,
    })
}

/// Live [`configured_pack_pin`] against the global config.
pub fn pack_pin(role: Role) -> Option<ModelPack> {
    let cfg = crate::config::Config::global();
    configured_pack_pin(role, |k| cfg.get_param::<String>(k).ok())
}

// ── Compose: route → tier → pack ─────────────────────────────────────────────

/// The full "which model runs this sub-task" decision: classify + gate the work
/// into a [`Tier`] (via [`super::route`]), then resolve that tier to a concrete
/// pack. Returns both so the caller can log the tier and dispatch the model.
///
/// This is for ROUTED SUB-WORK only (mechanical/reasoning sub-tasks the coding
/// loop delegates to a subagent). The main loop itself stays on
/// [`ModelPacks::main_loop`] — see the module note on cache discipline.
pub fn resolve<'a>(
    signals: &TaskSignals,
    mesh: MeshGateInputs,
    packs: &'a ModelPacks,
) -> (Tier, &'a ModelPack) {
    let tier = super::route(signals, mesh);
    (tier, packs.pack_for_tier(tier))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cost_router::cache::may_swap_main_loop_model;
    use crate::cost_router::mesh::{MeshWorkload, PoolHealth};

    fn healthy_batch_pool() -> MeshGateInputs {
        MeshGateInputs {
            workload: MeshWorkload::Batch,
            pool_configured: true,
            trusted: true,
            health: PoolHealth::Healthy,
        }
    }

    fn no_pool() -> MeshGateInputs {
        MeshGateInputs {
            workload: MeshWorkload::Batch,
            pool_configured: false,
            trusted: false,
            health: PoolHealth::Unknown,
        }
    }

    // ── Explicit pins (the delegate seam) ──────────────────────────────────

    fn reader<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k: &str| {
            pairs
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.to_string())
        }
    }

    /// The whole point: an unset pack yields `None`, NOT the shipped default.
    /// If this ever returns Opus/Sonnet/Haiku, a delegate can be dispatched to a
    /// vendor the operator never chose.
    #[test]
    fn an_unset_pack_is_not_a_pin() {
        for role in [Role::Edit, Role::Hard, Role::Mechanical, Role::Local] {
            assert_eq!(configured_pack_pin(role, reader(&[])), None, "{role:?}");
        }
    }

    #[test]
    fn a_fully_set_pack_is_a_pin() {
        let env = [
            (KEY_EDIT_PROVIDER, "openai"),
            (KEY_EDIT_MODEL, "gpt-5.4-mini"),
        ];
        assert_eq!(
            configured_pack_pin(Role::Edit, reader(&env)),
            Some(ModelPack::new("openai", "gpt-5.4-mini"))
        );
        // …and it pins only its own role.
        assert_eq!(configured_pack_pin(Role::Hard, reader(&env)), None);
    }

    /// Half a pin is not a pin — the same rule as `role_map::resolve_role_model`.
    /// Half-applying it would pair a provider with a foreign model and 404.
    #[test]
    fn a_half_configured_pack_is_unset() {
        for env in [
            vec![(KEY_HARD_PROVIDER, "openai")],
            vec![(KEY_HARD_MODEL, "gpt-5.6-sol")],
            vec![(KEY_HARD_PROVIDER, "  "), (KEY_HARD_MODEL, "gpt-5.6-sol")],
            vec![(KEY_HARD_PROVIDER, "openai"), (KEY_HARD_MODEL, "")],
        ] {
            assert_eq!(
                configured_pack_pin(Role::Hard, reader(&env)),
                None,
                "{env:?}"
            );
        }
    }

    #[test]
    fn each_role_reads_its_own_keys() {
        assert_eq!(keys_for(Role::Edit), (KEY_EDIT_PROVIDER, KEY_EDIT_MODEL));
        assert_eq!(keys_for(Role::Hard), (KEY_HARD_PROVIDER, KEY_HARD_MODEL));
        assert_eq!(
            keys_for(Role::Mechanical),
            (KEY_MECHANICAL_PROVIDER, KEY_MECHANICAL_MODEL)
        );
        assert_eq!(keys_for(Role::Local), (KEY_LOCAL_PROVIDER, KEY_LOCAL_MODEL));
    }

    // ── tier → role → model (the test bar's tier→model mapping) ────────────

    #[test]
    fn tiers_map_to_the_expected_roles() {
        assert_eq!(role_for_tier(Tier::Frontier), Role::Hard);
        assert_eq!(role_for_tier(Tier::CheapCloud), Role::Mechanical);
        assert_eq!(role_for_tier(Tier::LocalFree), Role::Local);
        // The mesh tier reuses the local model (pooled endpoint, same model).
        assert_eq!(role_for_tier(Tier::MeshFree), Role::Local);
    }

    // ── The routing decision: hard→Opus, edit→Sonnet, mechanical→Haiku,
    //    local→Ollama (the test bar's exact mapping) ─────────────────────────

    #[test]
    fn hard_maps_to_opus() {
        let p = ModelPacks::default();
        assert_eq!(p.model_for(Role::Hard), "claude-opus-4-8");
        assert_eq!(p.model_for_tier(Tier::Frontier), "claude-opus-4-8");
        assert_eq!(p.pack_for(Role::Hard).provider, "anthropic");
    }

    #[test]
    fn edit_maps_to_sonnet() {
        let p = ModelPacks::default();
        assert_eq!(p.model_for(Role::Edit), "claude-sonnet-5");
        // The editor is the stable main-loop model.
        assert_eq!(p.main_loop().model, "claude-sonnet-5");
        assert_eq!(p.main_loop().provider, "anthropic");
    }

    #[test]
    fn mechanical_maps_to_haiku() {
        let p = ModelPacks::default();
        assert_eq!(p.model_for(Role::Mechanical), "claude-haiku-4-5-20251001");
        assert_eq!(
            p.model_for_tier(Tier::CheapCloud),
            "claude-haiku-4-5-20251001"
        );
        assert_eq!(p.pack_for(Role::Mechanical).provider, "anthropic");
    }

    #[test]
    fn local_maps_to_ollama() {
        let p = ModelPacks::default();
        assert_eq!(p.model_for(Role::Local), "qwen3");
        assert_eq!(p.model_for_tier(Tier::LocalFree), "qwen3");
        // Local runs on-device via the free Ollama provider.
        assert_eq!(p.pack_for(Role::Local).provider, "ollama");
    }

    // ── resolve: task class → tier → correct pack model ────────────────────

    #[test]
    fn hard_reasoning_resolves_to_the_opus_pack() {
        let p = ModelPacks::default();
        // A multi-file change is reasoning → Frontier → the hard pack.
        let s = TaskSignals {
            multi_file: true,
            ..Default::default()
        };
        let (tier, pack) = resolve(&s, no_pool(), &p);
        assert_eq!(tier, Tier::Frontier);
        assert_eq!(pack.model, "claude-opus-4-8");
    }

    #[test]
    fn mechanical_work_resolves_to_local_then_haiku_on_escalation() {
        let p = ModelPacks::default();
        // Interactive mechanical with no pool starts LOCAL (Ollama $0).
        let s = TaskSignals {
            mechanical: true,
            ..Default::default()
        };
        let (tier, pack) = resolve(&s, no_pool(), &p);
        assert_eq!(tier, Tier::LocalFree);
        assert_eq!(pack.model, "qwen3");

        // The cloud escalation target for mechanical work is Haiku.
        assert_eq!(
            p.model_for_tier(Tier::CheapCloud),
            "claude-haiku-4-5-20251001"
        );
    }

    #[test]
    fn mechanical_batch_on_a_trusted_pool_resolves_to_the_local_model() {
        let p = ModelPacks::default();
        let s = TaskSignals {
            mechanical: true,
            batch: true,
            ..Default::default()
        };
        let (tier, pack) = resolve(&s, healthy_batch_pool(), &p);
        assert_eq!(tier, Tier::MeshFree);
        // Pooled batch work runs the local model on a mesh endpoint.
        assert_eq!(pack.model, "qwen3");
        assert_eq!(pack.provider, "ollama");
    }

    // ── Config is retunable ────────────────────────────────────────────────

    #[test]
    fn overrides_apply_over_defaults() {
        let p = packs_from(
            None,
            Some("claude-opus-4-8".to_string()), // retune the editor to Opus
            None,
            None,
            Some("openai".to_string()), // retune mechanical to a different provider
            Some("gpt-4o-mini".to_string()),
            None,
            Some("qwen3-coder:30b".to_string()), // a coding-tuned local model
        );
        // Overridden fields take the new value…
        assert_eq!(p.edit.model, "claude-opus-4-8");
        assert_eq!(p.mechanical.provider, "openai");
        assert_eq!(p.mechanical.model, "gpt-4o-mini");
        assert_eq!(p.local.model, "qwen3-coder:30b");
        // …and untouched fields keep the shipped defaults.
        assert_eq!(p.edit.provider, "anthropic");
        assert_eq!(p.hard.model, "claude-opus-4-8");
        assert_eq!(p.local.provider, "ollama");
    }

    #[test]
    fn empty_overrides_are_exactly_the_defaults() {
        assert_eq!(
            packs_from(None, None, None, None, None, None, None, None),
            ModelPacks::default()
        );
    }

    // ── Cache-stability rule: main model stable, cheap tiers via subagents ──

    #[test]
    fn cheaper_tiers_cannot_ride_the_main_loop_they_need_a_subagent() {
        let p = ModelPacks::default();
        let main = p.main_loop().cache_key(); // the editor holds the warm cache

        // Routing any cheaper tier by swapping the LIVE main-loop model is
        // refused — it would bust the provider+model-scoped cache. Each must go
        // to a separate subagent context instead.
        assert!(!may_swap_main_loop_model(
            main,
            p.pack_for(Role::Hard).cache_key()
        ));
        assert!(!may_swap_main_loop_model(
            main,
            p.pack_for(Role::Mechanical).cache_key()
        ));
        assert!(!may_swap_main_loop_model(
            main,
            p.pack_for(Role::Local).cache_key()
        ));

        // The one safe "swap" is the no-op: the loop keeps running its own pack.
        assert!(may_swap_main_loop_model(main, main));
    }

    #[test]
    fn every_role_has_a_distinct_model_so_a_swap_is_always_a_real_swap() {
        let p = ModelPacks::default();
        let models = [
            p.model_for(Role::Edit),
            p.model_for(Role::Hard),
            p.model_for(Role::Mechanical),
            p.model_for(Role::Local),
        ];
        // No two roles share a model — every cross-role dispatch is a genuine
        // model change (so the cache guard above is never vacuously true).
        for i in 0..models.len() {
            for j in (i + 1)..models.len() {
                assert_ne!(models[i], models[j], "roles {i} and {j} share a model");
            }
        }
    }
}
