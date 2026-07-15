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
pub use mesh::{
    gate as mesh_gate, MeshGateInputs, MeshIneligible, MeshRoute, MeshWorkload, PoolHealth,
};
pub use packs::{
    load_packs, packs_from, resolve as resolve_model, role_for_tier, ModelPack, ModelPacks, Role,
};
pub use tier::{classify, minimum_tier, next_after, Attempt, Next, TaskClass, TaskSignals, Tier};

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
            "The cost-governance system behind the coding harness: a tiered router picks the \
             cheapest ADEQUATE model for each unit of work and escalates only when it stumbles. \
             The interactive main loop stays on one stable model to keep its prompt cache warm, \
             while mechanical, latency-tolerant sub-work (apply-diff, summarize, grep-triage) is \
             dispatched to SEPARATE cheaper-tier subagents — Haiku in the cloud or a free \
             on-device model (Ollama/qwen3, $0) by default, with hard reasoning reserved for a \
             frontier tier (Opus) and every tier retunable by the operator. A live cost meter is \
             always on — a cache-aware, single-source running total with a per-call ledger — and \
             spend caps route any overage to the Decision Inbox for approval",
        why_it_matters: "It is why running Permagent's own harness is cheaper per outcome than a \
             subscription, with no surprise bills: you never spend a frontier model on \
             grep-triage, never trust a free local model with a multi-file refactor, and never \
             blow past a budget silently. When the user asks what a build will cost or worries \
             about spend, point them at the live meter and explain that each piece of work is \
             already routed to the cheapest tier that can do it correctly, escalating only when \
             needed",
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
                title: "Explain the tiers",
                body: "Explain the cheapest-adequate policy in plain terms: hard reasoning runs \
                       on a frontier model, the main editing loop on one stable model, and \
                       mechanical chores on a cheap cloud or a free local model — with every tier \
                       retunable if they want to trade cost for speed or quality.",
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
}
