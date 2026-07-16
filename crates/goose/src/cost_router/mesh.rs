//! The mesh cheap-tier: the health-gate and the auto-fallback.
//!
//! Per the mesh research, a trusted pooled/mesh endpoint is conceptually the
//! cheapest tier — but content-bearing work only ever routes to it under FOUR
//! conjunctive conditions, and it is NEVER on the interactive critical path.
//! This module is the pure gate + fallback LOGIC behind the `resolve_route`
//! seam (#702, merged). The **live inputs provider** is the mesh pool engine:
//! `crate::mesh::pool::live_gate_inputs` (or `PoolEngine::gate_inputs`) builds
//! [`MeshGateInputs`] from real peer health/trust state, and the engine's
//! dispatch consults [`gate`] before every pool rung.
//!
//! ## The four gate conditions (all required)
//! 1. the work is BATCH / mechanical (latency-tolerant) — never Interactive;
//! 2. a pool endpoint is configured;
//! 3. the pool is explicitly TRUSTED (mirrors `mesh::trusted_pool_enabled()`);
//! 4. the pool is HEALTHY (a recent successful health probe).
//!
//! ## The auto-fallback
//! On mesh timeout/failure the router degrades to a cheap CLOUD model — the
//! Reader's 60s-timeout-then-degrade pattern (`reader::ollama_summary`),
//! generalized: a dropped or slow mesh node must never hang or abort the task.
//! See `super::tier::next_after`, which routes a `MeshFree` failure to
//! `fallback_tier()`.
//!
//! ## Wiring to #702 (merged) and the pool engine
//! This module deliberately does NOT import `crate::mesh` — it mirrors the
//! seam's concepts (`Workload::Batch` vs `Interactive`, the trusted-pool
//! opt-in) as plain inputs so the gate stays a pure function. The bridge now
//! exists in production: `crate::mesh::pool` supplies live inputs and runs
//! this gate inside its dispatch ladder (pool → local → the
//! [`fallback_tier`] escalation below).
//!
//! Pure so it is unit-testable without env or network (env-mutating tests flake
//! under parallel `cargo test`, per the mesh scaffold's own note).

use super::tier::Tier;

/// The mesh health/response budget — the Reader's 60s Ollama timeout
/// (`reader::ollama_summary`), promoted into the router. Exceeding it is an
/// `Attempt::TimedOut` and triggers `fallback_tier()`.
pub const MESH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Whether the unit of work is latency-tolerant enough to leave the device.
/// Mirrors `mesh::Workload` without depending on the unmerged scaffold; at the
/// #702 bridge, `Batch` maps to `mesh::Workload::Batch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshWorkload {
    /// Latency-tolerant background work — eligible to offload.
    Batch,
    /// User-facing, latency-sensitive work — never offloaded (the RTT wall).
    Interactive,
}

/// The health of a configured pool endpoint, from a recent probe. `Unknown`
/// (never probed / probe stale) is treated as NOT healthy — fail closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolHealth {
    Healthy,
    Unhealthy,
    Unknown,
}

/// The four gate inputs, supplied by the caller/seam. Kept as plain data so the
/// gate is a pure function (no env, no network).
#[derive(Debug, Clone, Copy)]
pub struct MeshGateInputs {
    pub workload: MeshWorkload,
    /// A pool endpoint is configured (mirrors `mesh::configured_pool_endpoint`).
    pub pool_configured: bool,
    /// The pool is explicitly trusted (mirrors `mesh::trusted_pool_enabled`).
    pub trusted: bool,
    /// The latest health-probe result for the pool.
    pub health: PoolHealth,
}

/// The gate verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshRoute {
    /// Eligible: route this batch unit to the trusted, healthy pool.
    UseMesh,
    /// Not eligible: the caller uses its normal on-device / cloud tier. Carries
    /// the specific reason (actionable, and asserted in tests).
    Ineligible(MeshIneligible),
}

/// Why the mesh tier was not chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshIneligible {
    /// Interactive/critical-path work — never offloaded, whatever else holds.
    Interactive,
    /// No pool endpoint configured.
    NoPool,
    /// A pool is configured but the user has not marked it trusted.
    Untrusted,
    /// The pool failed (or never passed) its health probe.
    Unhealthy,
}

/// The mesh health-gate. Routes to the pool ONLY when all four conditions hold;
/// otherwise returns the specific reason.
///
/// Interactive work is rejected FIRST and unconditionally — the critical-path
/// wall the mesh vision refuses to cross. No configuration (a trusted, healthy
/// pool included) can admit interactive work here.
pub fn gate(inputs: MeshGateInputs) -> MeshRoute {
    // (1) Never offload interactive work — checked first so no later condition
    // can accidentally admit it.
    if inputs.workload == MeshWorkload::Interactive {
        return MeshRoute::Ineligible(MeshIneligible::Interactive);
    }
    // (2) A pool must be configured.
    if !inputs.pool_configured {
        return MeshRoute::Ineligible(MeshIneligible::NoPool);
    }
    // (3) It must be explicitly trusted (content-bearing work off-device).
    if !inputs.trusted {
        return MeshRoute::Ineligible(MeshIneligible::Untrusted);
    }
    // (4) It must be healthy right now.
    if inputs.health != PoolHealth::Healthy {
        return MeshRoute::Ineligible(MeshIneligible::Unhealthy);
    }
    MeshRoute::UseMesh
}

/// Convenience for the #702 bridge: `true` iff the gate admits this unit, so the
/// caller may consult `mesh::resolve_route`. Equivalent to
/// `matches!(gate(inputs), MeshRoute::UseMesh)`.
pub fn should_consult_mesh_seam(inputs: MeshGateInputs) -> bool {
    matches!(gate(inputs), MeshRoute::UseMesh)
}

/// The auto-fallback tier when a mesh attempt fails/times out: a reliable cheap
/// CLOUD model. NOT local (a lighter device may not run the batch model) and
/// NEVER an abort — the task always completes. The Reader's timeout-then-degrade,
/// promoted into the router.
pub fn fallback_tier() -> Tier {
    Tier::CheapCloud
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(
        workload: MeshWorkload,
        pool_configured: bool,
        trusted: bool,
        health: PoolHealth,
    ) -> MeshGateInputs {
        MeshGateInputs {
            workload,
            pool_configured,
            trusted,
            health,
        }
    }

    // ── The health-gate: mesh ONLY when Batch + configured + trusted + healthy ─

    #[test]
    fn routes_to_mesh_only_when_all_four_hold() {
        assert_eq!(
            gate(inputs(MeshWorkload::Batch, true, true, PoolHealth::Healthy)),
            MeshRoute::UseMesh
        );
    }

    #[test]
    fn interactive_is_never_offloaded_even_fully_configured() {
        // Interactive loses whatever else is true — the critical-path wall.
        assert_eq!(
            gate(inputs(
                MeshWorkload::Interactive,
                true,
                true,
                PoolHealth::Healthy
            )),
            MeshRoute::Ineligible(MeshIneligible::Interactive)
        );
    }

    #[test]
    fn each_missing_condition_names_its_reason() {
        assert_eq!(
            gate(inputs(
                MeshWorkload::Batch,
                false,
                true,
                PoolHealth::Healthy
            )),
            MeshRoute::Ineligible(MeshIneligible::NoPool)
        );
        assert_eq!(
            gate(inputs(
                MeshWorkload::Batch,
                true,
                false,
                PoolHealth::Healthy
            )),
            MeshRoute::Ineligible(MeshIneligible::Untrusted)
        );
        assert_eq!(
            gate(inputs(
                MeshWorkload::Batch,
                true,
                true,
                PoolHealth::Unhealthy
            )),
            MeshRoute::Ineligible(MeshIneligible::Unhealthy)
        );
    }

    #[test]
    fn unknown_health_fails_closed() {
        assert_eq!(
            gate(inputs(MeshWorkload::Batch, true, true, PoolHealth::Unknown)),
            MeshRoute::Ineligible(MeshIneligible::Unhealthy)
        );
    }

    #[test]
    fn untrusted_pool_is_never_used_even_if_healthy() {
        // The InferencePayload/is_trusted_peer invariant: content-bearing work
        // never goes to an untrusted peer, even a reachable healthy one.
        assert!(!should_consult_mesh_seam(inputs(
            MeshWorkload::Batch,
            true,
            false,
            PoolHealth::Healthy
        )));
    }

    #[test]
    fn should_consult_matches_gate() {
        let good = inputs(MeshWorkload::Batch, true, true, PoolHealth::Healthy);
        assert!(should_consult_mesh_seam(good));
        let bad = inputs(MeshWorkload::Batch, true, true, PoolHealth::Unhealthy);
        assert!(!should_consult_mesh_seam(bad));
    }

    // ── The fallback: mesh timeout/fail → cheap cloud, not abort ───────────

    #[test]
    fn fallback_is_cheap_cloud() {
        assert_eq!(fallback_tier(), Tier::CheapCloud);
    }

    #[test]
    fn mesh_timeout_budget_matches_readers_60s() {
        assert_eq!(MESH_TIMEOUT, std::time::Duration::from_secs(60));
    }
}
