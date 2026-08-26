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
//! - **Execution-grounded best-of-N** turns spare cheap-tier compute into
//!   accuracy: on a task the difficulty router judges hard, sample N candidate
//!   solutions and select the one that PASSES `verify` (CodeT dual-execution
//!   agreement), never a neural reranker. Cheap-tier only, spend-capped. See
//!   `best_of_n`.
//!
//! The whole module is pure/near-pure by design (the test bar demands testable
//! decisions, and CI is the build gate): every routing, escalation, gate, and
//! budget decision is a pure function; only `budget::raise_budget_gate` and
//! `budget::load_budget_config` touch IO, and each is a thin wrapper over a pure
//! core.

pub mod assess;
pub mod best_of_n;
pub mod budget;
pub mod cache;
pub mod cheap;
pub mod delegate;
pub mod derived;
pub mod escalation;
pub mod fallback;
pub mod hold_done;
pub mod knowledge;
pub mod mesh;
pub mod packs;
pub mod recommend;
pub mod review_gate;
pub mod reviewer_pick;
pub mod role_map;
pub mod snapshot;
pub mod tier;
pub mod tool_signals;

pub use assess::{assess_goal, Assessment};
pub use best_of_n::{
    best_of_n_enabled, best_of_n_from, difficulty, load_best_of_n, plan_candidates, run_best_of_n,
    select as select_candidate, BestOfNOutcome, BestOfNPlan, CandidateSource, CandidateVerdict,
    Difficulty, DifficultySignals, Selection, Verdict, BEST_OF_N_DEFAULT, MAX_BEST_OF_N,
};
pub use budget::{
    budget_verdict, budget_verdict_with_unpriced, BudgetBand, BudgetCeilings, BudgetConfig,
    BudgetScope, BudgetVerdict,
};
pub use cache::{
    may_swap_main_loop_model, model_change_breaks_cache, prefix_is_cache_stable, ModelKey,
    PrefixSegment, CANONICAL_PREFIX, HARNESS_PREFIX,
};
pub use cheap::{
    build_ladder, default_anchor, discover_priced_candidates, is_key_configured, load_ladder,
    reference_cost_for, CheapCandidate, CheapLadder, PricedCandidate,
};
pub use delegate::{
    decide_delegate_model, delegate_routing, delegate_routing_live,
    escalation_allowed as delegate_escalation_allowed, DelegateRouting, DelegateSource,
    EscalationRefusal, KEY_ALLOW_ESCALATION,
};
pub use derived::{
    config_key_affects_derived_map, derive_role_map, derived_role_map, invalidate_derived_role_map,
    model_routing_receipt, DerivedRoleMap, Provenance, RoleSource, DERIVED_ROLE_MAP_TTL,
};
pub use escalation::{
    build_handoff, decide_escalation, load_max_escalations, max_escalations_from,
    tier_for_workflow_role, workflow_role_for_tier, EscalationOutcome, GoalEscalationState,
    ParkReason, ESCALATION_METADATA_KEY, MAX_ESCALATIONS_DEFAULT,
};
pub use hold_done::{decide_hold, HoldOutcome, HoldState, HOLD_METADATA_KEY, MAX_HOLDS};
pub use knowledge::{
    kb_is_stale, kb_snapshot_date, lookup as lookup_model_knowledge, lookup_with_confidence,
    LookupConfidence, ModelKnowledge, KB_SNAPSHOT_DATE, KB_SNAPSHOT_STALE_AFTER_DAYS, KNOWN_MODELS,
};
pub use mesh::{
    gate as mesh_gate, MeshGateInputs, MeshIneligible, MeshRoute, MeshWorkload, PoolHealth,
};
pub use packs::{
    configured_pack_pin, load_packs, pack_pin, pack_role_for_workflow_role, packs_from,
    resolve as resolve_model, role_for_tier, ModelPack, ModelPacks, Role,
};
pub use recommend::{
    available_from, discover_available_models, discover_available_models_async,
    discover_ollama_models_async, is_provider_configured, provider_key_env, recommend,
    recommend_configured, recommend_configured_async, recommend_from_available, resolve_known,
    AvailableModel, CapabilityFloor, ProviderModels, Recommendation, RoleRecommendation,
    WorkflowRole, EDIT_RELIABILITY_FLOOR,
};
pub use review_gate::{
    build_review_prompt, classify_path, gate_decision, parse_review, review_required,
    reviewer_dispatch, reviewer_routing, FileClass, GateDecision, ReviewFinding, ReviewLens,
    ReviewOutcome, ReviewTrigger, ReviewerDispatch, ReviewerRouting, Verdict as ReviewVerdict,
    REVIEWER_DIVERSITY_WARNING, REVIEWER_EXTENSIONS, REVIEW_ESCALATE_AT, REVIEW_SYSTEM_PROMPT,
};
pub use review_gate::{build_rubric_prompt, REVIEW_RUBRIC_SYSTEM_PROMPT};
pub use reviewer_pick::{
    family_of, model_is_retired, reviewer_spend_gate, select_reviewer, ReviewerPick,
    ReviewerSelection, ReviewerSource, SpendDecision, NO_REVIEWER_AVAILABLE,
    REVIEWER_MIN_ORCHESTRATION, REVIEWER_STRONG_ORCHESTRATION, SMALL_DIFF_LINES,
};
pub use role_map::{
    cache_guard_should_warn, clear_role_model, configured as configured_role_models, derive_role,
    mappings_to_persist, resolve_role_model, resolve_role_model_or_derived, role_model,
    role_model_or_derived, set_role_model, RoleModel,
};
pub use snapshot::{RoutingSnapshot, ROUTING_SNAPSHOT_KEY};
pub use tier::{
    classify, minimum_tier, next_after, Attempt, Next, TaskClass, TaskSignals, Tier,
    VerifyEscalation, VerifyEscalationAction, VERIFY_ESCALATE_AT,
};
pub use tool_signals::{
    corroborates_verify_climb, corroborating_consecutive, extract as extract_tool_signals,
    extract_from_messages as extract_tool_signals_from_messages, ToolTranscriptSignals, ToolTurn,
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
            "The cost-governance system behind the coding harness. It has two halves. The \
             RECOMMENDER is objective and vendor-neutral: from measured diff-format reliability, \
             orchestration strength and price — including the models actually pulled into a \
             local Ollama — it names the best fit for each workflow role (planning/orchestration, \
             editing, mechanical search-and-summarize, review, and a free on-device tier). Each \
             role carries a capability floor; the recommender ranks only among models that clear \
             it, so MECHANICAL is the cheapest model that can do the job rather than the cheapest \
             model, and it says plainly when nothing clears a floor. Its per-role suggestion is \
             shown by `permagent packs recommend` and persisted by `permagent packs apply`. The \
             ROUTER then dispatches DELEGATED work — subagents and goal workers — by role: a \
             hand-configured mapping wins; otherwise it derives a best-fit map from the models \
             you actually have (keyed providers and installed local models) — for each role the \
             cheapest, local or cloud, that clears the role's bar — and routes on that by \
             default; a role nothing clears stays on the single session model. There is no \
             baked-in vendor default it silently falls back to, and the goal card's routing \
             receipt says which source picked the model (configured, derived with its floor and \
             confidence, or session). The interactive main loop always stays on one model to keep its prompt cache \
             warm; latency-tolerant sub-work goes to separate subagents, and a cache-heavy role \
             routed to a non-caching provider is flagged at dispatch. Every dispatched GOAL is \
             assessed to a starting tier before any model runs — deterministic, zero-LLM: the \
             tier is raised only by structure (acceptance-criteria count, breadth), an explicit \
             tag, or an explicit pin — never by the wording of the goal's own title and \
             description, which can only route it cheaper. Simple work starts cheap; a verify \
             failure climbs the configured escalation ladder carrying the prior attempt's diff \
             — harness tool transcripts (severity, spinning) corroborate a climb but never swap \
             the interactive main-loop model on their own — and the user can pin a tier \
             explicitly with metadata.tier on the goal. The routing snapshot on each goal card \
             and the Build cost meter is the receipt. Worker \
             selection ranks by real marginal cost (local free, then flat-rate subscription CLIs, \
             then metered APIs) and goal_advance's worker parameter pins a named worker outright \
             — a pin is honoured or refused loudly, never silently rerouted. A live cost meter is \
             always on — a cache-aware, single-source running total with a per-call ledger — and \
             spend caps route any overage to the Decision Inbox for approval",
        why_it_matters:
            "It is how running Permagent's own harness stays cheap per outcome, with no \
             surprise bills and no vendor lock-in: the recommender picks, per role, the cheapest \
             model that clears that role's capability floor, whether local or cloud, with no bias \
             toward the vendor whose runtime this is; delegated work then runs on the mapping the \
             user hand-configured, else on that derived best fit, under the same spend caps — and \
             nothing routes to a model the user does not have. When the user asks what a build \
             will cost, worries about spend, or asks which models to use where, point them at the \
             live meter and the objective per-role recommendation (`permagent packs recommend`), \
             and explain that a hand-set mapping pins a role while an unset one is derived from \
             the models they have. When you dispatch work of \
             ANY kind — a blog post, a lookup, a refactor, a build from scratch — you can state \
             with confidence HOW the path was chosen: which worker won and why (cost rank or an \
             explicit pin), which tier the goal was assessed to and the recorded reason, and that \
             escalation is earned by a measured verify failure rather than guessed up front. Say \
             so plainly; the routing snapshot on each goal card is the receipt",
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
                       the model they configured for it — or, with nothing configured, on the \
                       best fit derived from the models they actually have (local or cloud, the \
                       cheapest that clears the role's bar), and on their single session model \
                       when nothing clears it — never a built-in vendor default. The pick comes \
                       from an objective recommender using measured reliability and price, not \
                       vendor preference, and each goal card's routing receipt says whether the \
                       model was configured or derived. Point them at `permagent packs recommend` \
                       to see the best-fit-per-role suggestion, and `permagent packs set` / \
                       `apply` to pin a role by hand.",
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
