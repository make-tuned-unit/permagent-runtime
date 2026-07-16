//! The cost-optimizing router — pick the cheapest ADEQUATE way to run each unit
//! of work, escalate only when it stumbles, and never spend past the caps
//! without asking. This is the harness's cost-governance core: cheaper per
//! outcome than a subscription, with no surprises.
//!
//! It generalizes routing patterns already proven in the codebase rather than
//! reinventing them:
//!
//! - The **cheap-first-then-escalate** policy is lifted from the orchestrator's
//!   roadmap decomposition (#249): a cheap/local model runs the structured pass
//!   first and only escalates to the strong session provider on
//!   failure/unparseable. `tier::next_after` makes that policy reusable by any
//!   caller. See `agents::platform_extensions::orchestrator`.
//! - The **tier ladder** extends `goal_state::cost_tier_rank`
//!   (`local_free` < `subscription` < `paid_api`) with a pooled tier below local.
//! - The **mesh cheap tier** wires to the `resolve_route` seam (#702, merged)
//!   and is fed live by the mesh pool engine (`crate::mesh::pool`): batch-only,
//!   trust-gated, health-gated, with an auto-fallback to cheap cloud — the
//!   Reader's 60s-timeout-then-degrade, promoted into the router. See `mesh`.
//! - The **spend caps** check the per-call cost ledger (#714,
//!   `providers::canonical::cost`) and gate through the Decision Inbox
//!   (`decisions`). See `budget`.
//! - The **prompt-cache discipline** keeps a conversation's model stable and its
//!   prefix ordered, so cheaper tiers never silently discard a warm cache. See
//!   `cache`.
//!
//! The whole module is pure/near-pure by design (the test bar demands testable
//! decisions, and CI is the build gate): every routing, escalation, gate, and
//! budget decision is a pure function; only `budget::raise_budget_gate` and
//! `budget::load_budget_config` touch IO, and each is a thin wrapper over a pure
//! core.

pub mod budget;
pub mod cache;
pub mod cheap;
pub mod knowledge;
pub mod mesh;
pub mod packs;
pub mod recommend;
pub mod role_map;
pub mod tier;

pub use budget::{
    budget_verdict, BudgetBand, BudgetCeilings, BudgetConfig, BudgetScope, BudgetVerdict,
};
pub use cache::{
    may_swap_main_loop_model, model_change_breaks_cache, prefix_is_cache_stable, PrefixSegment,
    CANONICAL_PREFIX, HARNESS_PREFIX,
};
pub use cheap::{
    build_ladder, default_anchor, discover_priced_candidates, is_key_configured, load_ladder,
    reference_cost_for, CheapCandidate, CheapLadder, PricedCandidate,
};
pub use knowledge::{lookup as lookup_model_knowledge, ModelKnowledge, KNOWN_MODELS};
pub use mesh::{
    gate as mesh_gate, MeshGateInputs, MeshIneligible, MeshRoute, MeshWorkload, PoolHealth,
};
pub use packs::{
    load_packs, packs_from, resolve as resolve_model, role_for_tier, ModelPack, ModelPacks, Role,
};
pub use recommend::{
    available_from, discover_available_models, is_provider_configured, provider_key_env, recommend,
    recommend_configured, recommend_from_available, resolve_known, AvailableModel, ProviderModels,
    Recommendation, RoleRecommendation, WorkflowRole, EDIT_RELIABILITY_FLOOR,
};
pub use role_map::{
    cache_guard_should_warn, clear_role_model, configured as configured_role_models, derive_role,
    mappings_to_persist, resolve_role_model, role_model, set_role_model, RoleModel,
};
pub use tier::{
    classify, minimum_tier, next_after, Attempt, Next, TaskClass, TaskSignals, Tier,
    VerifyEscalation, VerifyEscalationAction, VERIFY_ESCALATE_AT,
};

/// Self-knowledge descriptor for the **cost optimizer** (#714/#717/#720) — the
/// tiered router plus the always-on live cost meter that make the coding harness
/// cost-governed. Co-located with the router core it describes; aggregated by
/// `crate::agents::self_knowledge::SURFACE_DESCRIPTORS`. Static — the capability
/// is described without claiming a live spend figure (the meter renders that in
/// the UI).
pub const COST_OPTIMIZER_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "cost_optimizer",
        display_name: "Cost optimizer",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "The cost-governance system behind the coding harness: it routes each workflow role \
             — planning/orchestration, editing, mechanical search-and-summarize, and review — to \
             the model YOU configured for it, chosen by an objective, vendor-neutral recommender \
             from measured diff-format reliability, orchestration strength, and price. There is \
             no baked-in vendor default: configure a per-role mapping and each role runs on your \
             chosen model; configure none and the harness stays on your single session model — it \
             never silently falls back to a built-in Opus/Sonnet/Haiku pack. The interactive main \
             loop stays on one stable model to keep its prompt cache warm, mechanical \
             latency-tolerant sub-work is dispatched to SEPARATE cheaper-tier subagents, and a \
             cache-heavy role routed to a non-caching provider is flagged at dispatch. A live \
             cost meter is always on — a cache-aware, single-source running total with a per-call \
             ledger — and spend caps route any overage to the Decision Inbox for approval",
        why_it_matters: "It is why running Permagent's own harness is cheaper per outcome than a \
             subscription, with no surprise bills and no vendor lock-in: each piece of work runs \
             on the cheapest model that can do it correctly, the recommender carries no bias \
             toward the vendor whose runtime this is, and nothing routes to a model the user did \
             not choose. When the user asks what a build will cost, worries about spend, or asks \
             which models to use where, point them at the live meter and the objective per-role \
             recommendation (`permagent packs recommend`), and explain that setting no mapping \
             keeps everything on their one model",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[
            crate::agents::self_knowledge::TeachingStep {
                title: "Show the live cost meter",
                body: "When the user is in or launching the coding harness, point out the live \
                       cost meter — a running, cache-aware total with a per-call ledger. Reassure \
                       them there are no hidden bills: they can watch spend accrue in real time \
                       and it is capped.",
                open_surface: Some(crate::agents::self_knowledge::SurfaceRef {
                    tab: "Build",
                    section: None,
                }),
                confirm: None,
            },
            crate::agents::self_knowledge::TeachingStep {
                title: "Explain per-role routing (no vendor default)",
                body: "Explain the per-role routing in plain terms: each workflow role runs on \
                       the model they configured for it — or, with nothing configured, on their \
                       single session model, never a built-in vendor default — picked by an \
                       objective recommender from measured reliability and price, not vendor \
                       preference. Point them at `permagent packs recommend` to see the \
                       best-fit-per-role suggestion for the models they already have, and \
                       `permagent packs apply` to route each role to it.",
                open_surface: None,
                confirm: None,
            },
        ],
    };

/// The full starting-tier decision for one unit of work, honoring BOTH the task
/// classification (cheapest adequate tier) and the mesh gate (batch, configured,
/// trusted, and healthy make the pooled tier eligible). This is the single entry
/// point a caller uses to pick where to START; escalation is then driven by
/// `tier::next_after` as attempts report back.
///
/// Only mechanical work is ever eligible to prefer the mesh tier, and only when
/// the gate admits it. Reasoning work ignores the pool entirely — it starts
/// strong and never offloads (the interactive/critical-path wall).
pub fn route(signals: &TaskSignals, mesh_inputs: MeshGateInputs) -> Tier {
    let class = classify(signals);
    let base = minimum_tier(class);
    if class == TaskClass::Mechanical && matches!(mesh_gate(mesh_inputs), MeshRoute::UseMesh) {
        return Tier::MeshFree;
    }
    base
}

/// Resolve a [`Tier`] — a starting tier from [`route`] or an escalated one from
/// [`tier::next_after`] — to the concrete provider+model that runs it.
///
/// Identical to [`ModelPacks::pack_for_tier`] for every tier EXCEPT
/// [`Tier::CheapCloud`], which is resolved through the [`cheap`] ladder: the
/// cheapest configured + priced cheap-cloud provider — DERIVED from the pricing
/// table over the operator's configured providers, not a fixed list — else the
/// Haiku anchor. This is the seam that makes ANY cheaper configured+priced
/// provider the preferred cheap tier automatically. Because `route` starts
/// mechanical work at `LocalFree` and only reaches `CheapCloud` on escalation
/// (`next_after(LocalFree, …) → CheapCloud`), the cheap-cloud provider choice
/// lives here rather than in the starting decision. With no cheap key set this
/// returns the Haiku anchor, which is byte-identical to the mechanical pack, so
/// behavior is unchanged until a cheaper priced provider is keyed.
///
/// `is_key_set` is the availability predicate — pass
/// `|k| cheap::is_key_configured(k)` in production, a pure set in tests. Returns
/// an owned [`ModelPack`] because the cheap-cloud pick is computed, not borrowed.
pub fn concrete_pack_for_tier(
    tier: Tier,
    packs: &ModelPacks,
    ladder: &CheapLadder,
    is_key_set: impl Fn(&str) -> bool,
) -> ModelPack {
    if tier == Tier::CheapCloud {
        let c = ladder.select(&is_key_set);
        ModelPack {
            provider: c.provider.clone(),
            model: c.model.clone(),
        }
    } else {
        packs.pack_for_tier(tier).clone()
    }
}

/// The full concrete starting decision for one routed sub-task: [`route`] to a
/// [`Tier`], then [`concrete_pack_for_tier`] to a provider+model (ladder-aware
/// for the cheap-cloud tier). Escalation after the returned pack fails is driven
/// by [`cheap::CheapLadder::escalate_after`] within the cheap tier and
/// [`tier::next_after`] across tiers — the existing #249/#720 policy, unchanged.
pub fn route_pack(
    signals: &TaskSignals,
    mesh_inputs: MeshGateInputs,
    packs: &ModelPacks,
    ladder: &CheapLadder,
    is_key_set: impl Fn(&str) -> bool,
) -> (Tier, ModelPack) {
    let tier = route(signals, mesh_inputs);
    let pack = concrete_pack_for_tier(tier, packs, ladder, is_key_set);
    (tier, pack)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn mechanical_batch_prefers_a_healthy_trusted_pool() {
        let s = TaskSignals {
            mechanical: true,
            batch: true,
            ..Default::default()
        };
        assert_eq!(route(&s, healthy_batch_pool()), Tier::MeshFree);
    }

    #[test]
    fn mechanical_work_without_a_pool_starts_local() {
        let s = TaskSignals {
            mechanical: true,
            batch: true,
            ..Default::default()
        };
        assert_eq!(route(&s, no_pool()), Tier::LocalFree);
    }

    #[test]
    fn reasoning_never_offloads_even_with_a_healthy_pool() {
        // A multi-file change must start on the frontier model and ignore the
        // pool entirely — no reasoning work on the mesh/critical path.
        let s = TaskSignals {
            multi_file: true,
            batch: true,
            ..Default::default()
        };
        assert_eq!(route(&s, healthy_batch_pool()), Tier::Frontier);
    }

    #[test]
    fn interactive_mechanical_stays_local_not_mesh() {
        // Mechanical but interactive (not batch) → the gate rejects mesh, so it
        // stays local rather than crossing the RTT wall.
        let s = TaskSignals {
            mechanical: true,
            ..Default::default()
        };
        let interactive_pool = MeshGateInputs {
            workload: MeshWorkload::Interactive,
            ..healthy_batch_pool()
        };
        assert_eq!(route(&s, interactive_pool), Tier::LocalFree);
    }

    #[test]
    fn end_to_end_escalation_from_mesh_to_cloud_to_giveup() {
        // A batch unit starts on mesh; the node drops → cheap cloud; cloud
        // 500s → frontier; frontier fails → give up (never an infinite retry).
        let s = TaskSignals {
            mechanical: true,
            batch: true,
            ..Default::default()
        };
        let start = route(&s, healthy_batch_pool());
        assert_eq!(start, Tier::MeshFree);

        let Next::Escalate(t1) = next_after(start, Attempt::TimedOut) else {
            panic!("mesh timeout should escalate");
        };
        assert_eq!(t1, Tier::CheapCloud);

        let Next::Escalate(t2) = next_after(t1, Attempt::Failed) else {
            panic!("cloud failure should escalate");
        };
        assert_eq!(t2, Tier::Frontier);

        assert_eq!(next_after(t2, Attempt::Failed), Next::GiveUp);
    }

    // ── Ladder-aware tier → concrete pack (the generic cheapest-path wiring) ─

    fn keyed(set: &[&str]) -> impl Fn(&str) -> bool {
        let owned: Vec<String> = set.iter().map(|s| s.to_string()).collect();
        move |k| owned.iter().any(|s| s == k)
    }

    /// A ladder derived from the REAL canonical costs of two shipped cheap
    /// providers (MiniMax, Kimi) plus the default Haiku anchor — the same shape
    /// `load_ladder` produces when both keys are configured, built here without
    /// touching env/config so the wiring test stays pure.
    fn derived_two_provider_ladder() -> CheapLadder {
        let anchor = default_anchor();
        let anchor_cost = reference_cost_for(&anchor.provider, &anchor.model);
        let priced = vec![
            PricedCandidate::new(
                "moonshot",
                "kimi-k2.5",
                "MOONSHOT_API_KEY",
                reference_cost_for("moonshot", "kimi-k2.5").unwrap(),
            ),
            PricedCandidate::new(
                "minimax",
                "MiniMax-M2.5",
                "MINIMAX_API_KEY",
                reference_cost_for("minimax", "MiniMax-M2.5").unwrap(),
            ),
        ];
        build_ladder(priced, anchor, anchor_cost)
    }

    #[test]
    fn no_cheap_keys_keeps_cheap_cloud_on_the_haiku_mechanical_pack() {
        // Regression guard: with no cheap key set, the CheapCloud tier resolves
        // to EXACTLY today's mechanical pack (Haiku) — a derived cheap ladder
        // changes nothing until a key arrives. Even the default (anchor-only)
        // ladder must resolve to Haiku.
        let packs = ModelPacks::default();
        let ladder = CheapLadder::default();
        let pack = concrete_pack_for_tier(Tier::CheapCloud, &packs, &ladder, |_| false);
        assert_eq!(&pack, packs.pack_for(Role::Mechanical));
        assert_eq!(pack.provider, "anthropic");
        assert_eq!(pack.model, "claude-haiku-4-5-20251001");

        // A ladder that HAS cheap rungs but with no keys set also stays on Haiku.
        let full = derived_two_provider_ladder();
        let pack = concrete_pack_for_tier(Tier::CheapCloud, &packs, &full, |_| false);
        assert_eq!(pack.provider, "anthropic");
        assert_eq!(pack.model, "claude-haiku-4-5-20251001");
    }

    #[test]
    fn escalated_cheap_cloud_prefers_the_cheapest_configured_provider() {
        // This is the point of the feature: once a cheap key is set, mechanical
        // cloud work (reached via LocalFree → CheapCloud escalation) routes to the
        // cheapest configured cheap-cloud model instead of always Haiku — chosen
        // by DERIVED canonical cost, not a hardcoded ladder.
        let packs = ModelPacks::default();
        let ladder = derived_two_provider_ladder();

        // MiniMax keyed → MiniMax (cheapest by real pricing: 1.50 < Kimi 3.60).
        let p = concrete_pack_for_tier(
            Tier::CheapCloud,
            &packs,
            &ladder,
            keyed(&["MINIMAX_API_KEY"]),
        );
        assert_eq!(p.provider, "minimax");
        assert_eq!(p.model, "MiniMax-M2.5");

        // Only Kimi keyed → Kimi (the cheapest AVAILABLE).
        let p = concrete_pack_for_tier(
            Tier::CheapCloud,
            &packs,
            &ladder,
            keyed(&["MOONSHOT_API_KEY"]),
        );
        assert_eq!(p.provider, "moonshot");
        assert_eq!(p.model, "kimi-k2.5");
    }

    #[test]
    fn non_cheap_tiers_resolve_from_packs_unchanged() {
        // Only the CheapCloud tier consults the ladder; every other tier keeps
        // resolving from the tiered packs exactly as before.
        let packs = ModelPacks::default();
        let ladder = CheapLadder::default();
        // A fresh owned "both cheap keys set" predicate per call.
        let cheap = || keyed(&["MINIMAX_API_KEY", "MOONSHOT_API_KEY"]);

        // Frontier → the hard (Opus) pack, regardless of cheap keys.
        let f = concrete_pack_for_tier(Tier::Frontier, &packs, &ladder, cheap());
        assert_eq!(&f, packs.pack_for(Role::Hard));
        assert_eq!(f.model, "claude-opus-4-8");

        // Local + Mesh → the local (Ollama) pack.
        let l = concrete_pack_for_tier(Tier::LocalFree, &packs, &ladder, cheap());
        assert_eq!(l.provider, "ollama");
        assert_eq!(l.model, "qwen3");
        let m = concrete_pack_for_tier(Tier::MeshFree, &packs, &ladder, cheap());
        assert_eq!(&m, packs.pack_for(Role::Local));
    }

    #[test]
    fn route_pack_composes_route_and_the_ladder_aware_resolution() {
        let packs = ModelPacks::default();
        let ladder = CheapLadder::default();

        // Mechanical, no pool → starts LocalFree on Ollama (cheap keys irrelevant
        // to the STARTING tier — the ladder only concretizes the escalated
        // CheapCloud rung).
        let s = TaskSignals {
            mechanical: true,
            ..Default::default()
        };
        let (tier, pack) = route_pack(&s, no_pool(), &packs, &ladder, keyed(&["MINIMAX_API_KEY"]));
        assert_eq!(tier, Tier::LocalFree);
        assert_eq!(pack.model, "qwen3");

        // Reasoning → Frontier on Opus.
        let s = TaskSignals {
            multi_file: true,
            ..Default::default()
        };
        let (tier, pack) = route_pack(&s, no_pool(), &packs, &ladder, |_| false);
        assert_eq!(tier, Tier::Frontier);
        assert_eq!(pack.model, "claude-opus-4-8");
    }
}
