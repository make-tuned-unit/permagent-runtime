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
//! - The **mesh cheap tier** wires to the `resolve_route` seam of the (unmerged)
//!   mesh scaffold (PR #702): batch-only, trust-gated, health-gated, with an
//!   auto-fallback to cheap cloud — the Reader's 60s-timeout-then-degrade
//!   (`reader::ollama_summary`), promoted into the router. See `mesh`.
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
pub mod mesh;
pub mod packs;
pub mod tier;

pub use budget::{
    budget_verdict, BudgetBand, BudgetCeilings, BudgetConfig, BudgetScope, BudgetVerdict,
};
pub use cache::{
    may_swap_main_loop_model, model_change_breaks_cache, prefix_is_cache_stable, PrefixSegment,
    CANONICAL_PREFIX,
};
pub use cheap::{is_key_configured, ladder_from, load_ladder, CheapCandidate, CheapLadder};
pub use mesh::{
    gate as mesh_gate, MeshGateInputs, MeshIneligible, MeshRoute, MeshWorkload, PoolHealth,
};
pub use packs::{
    load_packs, packs_from, resolve as resolve_model, role_for_tier, ModelPack, ModelPacks, Role,
};
pub use tier::{classify, minimum_tier, next_after, Attempt, Next, TaskClass, TaskSignals, Tier};

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
/// cheapest cheap-cloud provider the operator has keyed (MiniMax, then Kimi),
/// else the Haiku anchor. This is the seam that puts Kimi + MiniMax on the
/// router's cheapest-adequate ladder — because `route` starts mechanical work at
/// `LocalFree` and only reaches `CheapCloud` on escalation
/// (`next_after(LocalFree, …) → CheapCloud`), the cheap-cloud provider choice
/// lives here rather than in the starting decision. With no cheap key set this
/// returns the Haiku anchor, which is byte-identical to the mechanical pack, so
/// behavior is unchanged until a key arrives.
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

    // ── Ladder-aware tier → concrete pack (the Kimi/MiniMax wiring) ──────────

    fn keyed(set: &[&str]) -> impl Fn(&str) -> bool {
        let owned: Vec<String> = set.iter().map(|s| s.to_string()).collect();
        move |k| owned.iter().any(|s| s == k)
    }

    #[test]
    fn no_cheap_keys_keeps_cheap_cloud_on_the_haiku_mechanical_pack() {
        // Regression guard: with no cheap key set, the CheapCloud tier resolves
        // to EXACTLY today's mechanical pack (Haiku) — Kimi/MiniMax change nothing
        // until a key arrives.
        let packs = ModelPacks::default();
        let ladder = CheapLadder::default();
        let pack = concrete_pack_for_tier(Tier::CheapCloud, &packs, &ladder, |_| false);
        assert_eq!(&pack, packs.pack_for(Role::Mechanical));
        assert_eq!(pack.provider, "anthropic");
        assert_eq!(pack.model, "claude-haiku-4-5-20251001");
    }

    #[test]
    fn escalated_cheap_cloud_prefers_the_cheapest_configured_provider() {
        // This is the point of the feature: once a cheap key is set, mechanical
        // cloud work (reached via LocalFree → CheapCloud escalation) routes to the
        // cheapest configured cheap-cloud model instead of always Haiku.
        let packs = ModelPacks::default();
        let ladder = CheapLadder::default();

        // MiniMax keyed → MiniMax (cheapest).
        let p = concrete_pack_for_tier(
            Tier::CheapCloud,
            &packs,
            &ladder,
            keyed(&["MINIMAX_API_KEY"]),
        );
        assert_eq!(p.provider, "minimax");
        assert_eq!(p.model, "MiniMax-M2.5");

        // Only Kimi keyed → Kimi.
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
