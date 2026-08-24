//! The live verifier-driven escalation policy — the ACTION half of the closed
//! verify loop (#739 shipped the DECISION core; this completes it).
//!
//! When a goal's `verify` keeps failing the SAME way across a bounded run of
//! self-fix rounds (the runaway-loop guard's S6 signal, `tool_monitor`), the
//! cheapest-adequate model it was dispatched on has demonstrated a capability
//! gap *here*. This module decides whether to auto-escalate the fix to a
//! STRONGER model and re-attempt — the DeepSWE 42→59% mechanism — and, if so,
//! which concrete model, honoring four hard guardrails so the documented
//! "silently escalate everything to the frontier" cascade failure cannot occur:
//!
//! 1. **No baked-in default.** The escalated tier's model is resolved ONLY from
//!    the user's CONFIGURED role→model map ([`super::role_map::role_model`],
//!    which returns `None` when unset) — NEVER the [`super::packs`] tier-pack
//!    defaults. A single-model user (no ladder) has nothing stronger to climb
//!    to → NO swap → park. (The standing rule.)
//! 2. **Bounded per-goal budget.** At most `max_escalations` climbs per goal,
//!    orthogonal to the goal's normal `attempt_cap` — a pathological goal cannot
//!    walk the whole ladder to the frontier.
//! 3. **Spend-cap honored.** Escalating costs more; if it would cross the spend
//!    gate/hard ceiling ([`super::budget`]) the goal parks to the Decision Inbox
//!    instead of overspending.
//! 4. **Park-first preserved.** The swap is a NEW option WITHIN the loop, tried
//!    BEFORE parking. Only when there is nothing stronger configured, the budget
//!    is spent, or the spend-cap/max-escalations ceiling is hit does the
//!    existing work-preserving park (human-gated) apply.
//!
//! The decision core ([`decide_escalation`]) is pure — no async/DB/IO — so the
//! guardrails are unit-tested exhaustively. The live re-dispatch that acts on a
//! [`EscalationOutcome::Swap`] (persist the per-goal tier, hand the prior diff +
//! failure forward as context, and spawn the fix on the stronger model) is wired
//! by [`crate::agents::platform_extensions::orchestrator::escalate_verify_fix_loop`].

use serde::{Deserialize, Serialize};

use super::budget::BudgetVerdict;
use super::recommend::WorkflowRole;
use super::role_map::RoleModel;
use super::tier::{next_after, Attempt, Next, Tier, VERIFY_ESCALATE_AT};

/// The metadata_json key under which a goal's live escalation state persists, so
/// the climb survives the kill-and-re-dispatch that a model swap requires (a new
/// worker/session is minted per attempt — in-memory state would be lost).
pub const ESCALATION_METADATA_KEY: &str = "verify_escalation";

/// Default per-goal escalation budget: at most this many stronger-model climbs
/// before the goal parks for a human, regardless of how many ladder rungs remain
/// (guardrail 2). Two climbs (e.g. local→cheap→frontier) covers the realistic
/// capability ladder while bounding the frontier cascade. CONFIGURABLE.
pub const MAX_ESCALATIONS_DEFAULT: u8 = 2;

/// Config key overriding [`MAX_ESCALATIONS_DEFAULT`].
pub const KEY_MAX_ESCALATIONS: &str = "PERMAGENT_VERIFY_MAX_ESCALATIONS";

// ── Tier ↔ workflow-role bridge ──────────────────────────────────────────────

/// The workflow role whose CONFIGURED model runs a given escalated tier's work —
/// the bridge from the abstract tier ladder ([`Tier`]) to the user-facing
/// role→model map ([`super::role_map`]). Aligns with [`super::packs::role_for_tier`]
/// under the user-facing role names: the frontier rung is the `Orchestrate`
/// (hard-reasoning) role, cheap cloud is `Mechanical`, and both free tiers reuse
/// the on-device `Local` model. This is how "escalate to `Frontier`" becomes
/// "run the model the user mapped to Orchestrate" — and `None` (unmapped) is the
/// no-default park.
pub fn workflow_role_for_tier(tier: Tier) -> WorkflowRole {
    match tier {
        Tier::Frontier => WorkflowRole::Orchestrate,
        Tier::CheapCloud => WorkflowRole::Mechanical,
        Tier::LocalFree | Tier::MeshFree => WorkflowRole::Local,
    }
}

/// The tier ladder position a worker's workflow role STARTS at — used to seed a
/// goal's escalation tier at first dispatch, so the first verify-loop climb
/// knows which rung it is leaving. The judgment-dense roles sit at the frontier,
/// the token-heavy editor and mechanical work at cheap cloud, and the on-device
/// role at the free tier. `Review` and `Orchestrate` start at the ceiling
/// (nothing stronger to climb to — an already-strong worker parks rather than
/// escalating), matching the no-loop-at-the-top rule in [`next_after`].
pub fn tier_for_workflow_role(role: WorkflowRole) -> Tier {
    match role {
        WorkflowRole::Orchestrate | WorkflowRole::Review => Tier::Frontier,
        WorkflowRole::Edit | WorkflowRole::Mechanical => Tier::CheapCloud,
        WorkflowRole::Local => Tier::LocalFree,
    }
}

// ── The pure decision ────────────────────────────────────────────────────────

/// Why a verify-loop escalation is parking instead of swapping — the ceiling
/// conditions from the guardrails, surfaced so telemetry and the park decision
/// can explain WHICH limit stopped the climb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkReason {
    /// The goal runs on a single model with no configured ladder position —
    /// there is nothing stronger to escalate to (guardrail 1, load-bearing).
    SingleModelNoLadder,
    /// The per-goal `max_escalations` budget is spent (guardrail 2).
    MaxEscalationsExhausted,
    /// Already at the top of the tier ladder — no stronger tier exists.
    TierCeiling,
    /// A stronger tier exists but the user configured no model for it — the
    /// no-default rule refuses to inject one (guardrail 1).
    NoConfiguredStrongerTier,
    /// Escalating would cross the spend gate/hard ceiling (guardrail 3).
    SpendCap,
}

impl ParkReason {
    /// Stable, human-readable label for the park decision detail + telemetry.
    pub fn label(self) -> &'static str {
        match self {
            ParkReason::SingleModelNoLadder => "no configured stronger model (single-model)",
            ParkReason::MaxEscalationsExhausted => "per-goal escalation budget spent",
            ParkReason::TierCeiling => "already on the strongest tier",
            ParkReason::NoConfiguredStrongerTier => "no model configured for the next tier",
            ParkReason::SpendCap => "spend cap would be exceeded",
        }
    }
}

/// What the live verify-fix loop should do next, given the observed run of
/// identical `verify` failures for one goal and the four guardrails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EscalationOutcome {
    /// Below the same-failure threshold — let the current model keep fixing.
    KeepFixing,
    /// Escalate: re-attempt the fix on `model` (the CONFIGURED model for the
    /// next tier up). `to_tier` and `new_escalations_used` are the per-goal state
    /// the caller must persist before re-dispatching.
    Swap {
        to_tier: Tier,
        model: RoleModel,
        new_escalations_used: u8,
    },
    /// A ceiling is reached — stop climbing and hand the goal to a human via the
    /// existing work-preserving park (never climb forever).
    Park(ParkReason),
}

/// THE verifier-driven escalation decision (pure). Given the goal's current tier
/// (`None` ⇒ single-model, no ladder), climbs spent, budget, the observed
/// consecutive-same-failure count, a resolver from tier→CONFIGURED model (the
/// no-default role map), and the current spend verdict, decide whether to swap to
/// a stronger model or park.
///
/// The guardrails are structural, checked in ceiling-precedence order so the park
/// reason names the true limiter:
/// 1. below threshold ⇒ [`EscalationOutcome::KeepFixing`] (never escalate varying
///    or transient failures — the count is reset upstream in `tool_monitor`);
/// 2. single-model / no ladder ⇒ park (no-default, load-bearing);
/// 3. per-goal budget spent ⇒ park;
/// 4. tier ceiling ⇒ park;
/// 5. next tier has no configured model ⇒ park (no-default);
/// 6. escalation would cross the spend cap ⇒ park;
/// 7. otherwise ⇒ swap to the next tier's configured model, recording the climb.
pub fn decide_escalation(
    current: Option<Tier>,
    escalations_used: u8,
    max_escalations: u8,
    consecutive: u32,
    resolve: impl Fn(Tier) -> Option<RoleModel>,
    budget: BudgetVerdict,
) -> EscalationOutcome {
    // 1. Only the Nth CONSECUTIVE identical failure warrants a climb.
    if consecutive < VERIFY_ESCALATE_AT {
        return EscalationOutcome::KeepFixing;
    }
    // 2. No ladder to climb (single-model) — the load-bearing no-default park.
    let Some(current) = current else {
        return EscalationOutcome::Park(ParkReason::SingleModelNoLadder);
    };
    // 3. Per-goal escalation budget spent — do NOT climb further.
    if escalations_used >= max_escalations {
        return EscalationOutcome::Park(ParkReason::MaxEscalationsExhausted);
    }
    // 4. Tier ceiling — nothing stronger exists.
    let next = match next_after(current, Attempt::VerifyFailed) {
        Next::Escalate(t) => t,
        Next::Accept | Next::GiveUp => {
            return EscalationOutcome::Park(ParkReason::TierCeiling);
        }
    };
    // 5. No-default: escalate ONLY to a model the user actually configured.
    let Some(model) = resolve(next) else {
        return EscalationOutcome::Park(ParkReason::NoConfiguredStrongerTier);
    };
    // 6. Spend-cap: escalating costs more — park rather than overspend.
    if budget.needs_gate() || budget.must_stop() {
        return EscalationOutcome::Park(ParkReason::SpendCap);
    }
    // 7. Climb: re-attempt on the stronger configured model.
    EscalationOutcome::Swap {
        to_tier: next,
        model,
        new_escalations_used: escalations_used.saturating_add(1),
    }
}

// ── Per-goal persisted state (survives the kill-and-re-dispatch) ─────────────

/// One goal's live escalation state, persisted on the card's `metadata_json`
/// under [`ESCALATION_METADATA_KEY`]. A model swap kills the current worker and
/// re-dispatches a fresh one, so this MUST be durable, not in-memory.
///
/// `current_tier == None` marks a single-model goal (no ladder position) — the
/// escalation immediately parks (no-default). `escalations_used == 0` marks a
/// FRESH (un-escalated) dispatch: the dispatch path routes it by the worker's
/// normal role→model; `>= 1` marks an escalated re-dispatch that must run the
/// `current_tier`'s configured model and carry `handoff` forward.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalEscalationState {
    /// The tier the goal is CURRENTLY attempting on. `None` ⇒ single-model.
    pub current_tier: Option<Tier>,
    /// Stronger-model climbs spent on this goal so far (the guardrail-2 budget).
    pub escalations_used: u8,
    /// Carry-forward context handed to the escalated attempt: the prior model's
    /// diff + the verify failure — a human-style "here's what was tried, here's
    /// why it failed" handoff so the stronger model never restarts cold (R2).
    pub handoff: Option<String>,
}

/// Wire form persisted in metadata_json (tier as its stable string label).
#[derive(Debug, Serialize, Deserialize)]
struct StateWire {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tier: Option<String>,
    #[serde(default)]
    escalations_used: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    handoff: Option<String>,
}

/// Parse a [`Tier`] from its [`Tier::as_str`] label (the persisted form).
fn tier_from_label(s: &str) -> Option<Tier> {
    match s {
        "mesh_free" => Some(Tier::MeshFree),
        "local_free" => Some(Tier::LocalFree),
        "cheap_cloud" => Some(Tier::CheapCloud),
        "frontier" => Some(Tier::Frontier),
        _ => None,
    }
}

impl GoalEscalationState {
    /// A fresh (un-escalated) state seeded at first dispatch from the worker's
    /// role tier (`None` ⇒ single-model).
    pub fn seed(current_tier: Option<Tier>) -> Self {
        Self {
            current_tier,
            escalations_used: 0,
            handoff: None,
        }
    }

    /// The state to persist when a [`EscalationOutcome::Swap`] is acted on: the
    /// climbed tier + count + the carry-forward handoff. `escalations_used >= 1`
    /// is what the dispatch path keys on to route the re-dispatch to the climbed
    /// tier's configured model (see [`Self::is_escalated`]).
    pub fn escalated_to(to_tier: Tier, new_escalations_used: u8, handoff: String) -> Self {
        Self {
            current_tier: Some(to_tier),
            escalations_used: new_escalations_used,
            handoff: Some(handoff),
        }
    }

    /// True once at least one climb has happened — the marker the dispatch path
    /// uses to route an escalated re-dispatch to `current_tier`'s configured
    /// model (rather than the worker's normal role→model).
    pub fn is_escalated(&self) -> bool {
        self.escalations_used >= 1
    }

    /// Read the escalation state from a card's `metadata_json` object, if present
    /// and well-formed. An absent or malformed entry ⇒ `None` (treated as no
    /// escalation, so dispatch falls through to the normal role→model path).
    pub fn from_metadata(meta: &serde_json::Map<String, serde_json::Value>) -> Option<Self> {
        let raw = meta.get(ESCALATION_METADATA_KEY)?;
        let wire: StateWire = serde_json::from_value(raw.clone()).ok()?;
        Some(Self {
            current_tier: wire.tier.as_deref().and_then(tier_from_label),
            escalations_used: wire.escalations_used,
            handoff: wire.handoff,
        })
    }

    /// Serialize to the value stored at `metadata_json[ESCALATION_METADATA_KEY]`.
    pub fn to_metadata_value(&self) -> serde_json::Value {
        let wire = StateWire {
            tier: self.current_tier.map(|t| t.as_str().to_string()),
            escalations_used: self.escalations_used,
            handoff: self.handoff.clone(),
        };
        serde_json::to_value(wire).unwrap_or(serde_json::Value::Null)
    }
}

// ── Carry-forward handoff (R2) ───────────────────────────────────────────────

/// Build the human-style handoff appended to the escalated attempt's
/// instructions: what the weaker model tried (its diff) and why it failed (the
/// verify output), so the stronger model continues from that context instead of
/// starting cold. Pure so its shape is testable; the caller supplies the diff and
/// failure captured from the worker.
pub fn build_handoff(prior_model: &str, verify_failure: &str, diff: &str) -> String {
    let failure = verify_failure.trim();
    let diff = diff.trim();
    let mut out = format!(
        "## Escalation handoff\n\nA previous attempt on model `{}` could not make the project's \
         `verify` check pass — it kept failing the same way. You are a stronger model picking up \
         where it left off. Do NOT restart from scratch; read what was tried and why it failed, \
         then fix the remaining problem.\n",
        prior_model
    );
    if !failure.is_empty() {
        out.push_str("\n### The verify failure that blocked the previous attempt\n```\n");
        out.push_str(failure);
        out.push_str("\n```\n");
    }
    if !diff.is_empty() {
        out.push_str("\n### The changes the previous attempt made (its diff)\n```diff\n");
        out.push_str(diff);
        out.push_str("\n```\n");
    }
    out
}

// ── Thin config wrapper ──────────────────────────────────────────────────────

/// Apply an optional override over [`MAX_ESCALATIONS_DEFAULT`]. Split for pure
/// testing (mirrors the repo's pure-core-plus-thin-env-wrapper convention).
pub fn max_escalations_from(override_value: Option<u8>) -> u8 {
    override_value.unwrap_or(MAX_ESCALATIONS_DEFAULT)
}

/// Load the per-goal escalation budget from config, falling back to the default.
pub fn load_max_escalations() -> u8 {
    let cfg = crate::config::Config::global();
    max_escalations_from(cfg.get_param::<u8>(KEY_MAX_ESCALATIONS).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cost_router::budget::{budget_verdict, BudgetConfig};

    fn rm(provider: &str, model: &str) -> RoleModel {
        RoleModel {
            provider: provider.to_string(),
            model: model.to_string(),
        }
    }

    /// A spend verdict comfortably under every ceiling (no gate/stop).
    fn budget_ok() -> BudgetVerdict {
        budget_verdict(0.0, 0.0, &BudgetConfig::default())
    }

    /// A resolver that maps EVERY tier to a distinct configured model — a fully
    /// laddered user. `frontier`→opus, `cheap_cloud`→haiku, free→qwen.
    fn full_ladder(tier: Tier) -> Option<RoleModel> {
        Some(match tier {
            Tier::Frontier => rm("anthropic", "claude-opus-4-8"),
            Tier::CheapCloud => rm("anthropic", "claude-haiku-4-5"),
            Tier::LocalFree | Tier::MeshFree => rm("ollama", "qwen3"),
        })
    }

    // ── Tier ↔ role bridge ──────────────────────────────────────────────────

    #[test]
    fn tier_maps_to_the_user_facing_role() {
        assert_eq!(
            workflow_role_for_tier(Tier::Frontier),
            WorkflowRole::Orchestrate
        );
        assert_eq!(
            workflow_role_for_tier(Tier::CheapCloud),
            WorkflowRole::Mechanical
        );
        assert_eq!(workflow_role_for_tier(Tier::LocalFree), WorkflowRole::Local);
        assert_eq!(workflow_role_for_tier(Tier::MeshFree), WorkflowRole::Local);
    }

    #[test]
    fn role_seeds_the_expected_starting_tier() {
        assert_eq!(
            tier_for_workflow_role(WorkflowRole::Orchestrate),
            Tier::Frontier
        );
        assert_eq!(tier_for_workflow_role(WorkflowRole::Review), Tier::Frontier);
        assert_eq!(tier_for_workflow_role(WorkflowRole::Edit), Tier::CheapCloud);
        assert_eq!(
            tier_for_workflow_role(WorkflowRole::Mechanical),
            Tier::CheapCloud
        );
        assert_eq!(tier_for_workflow_role(WorkflowRole::Local), Tier::LocalFree);
    }

    // ── The core: consecutive same-fail → escalate to next_after's model ─────

    #[test]
    fn consecutive_same_failure_escalates_to_the_next_tiers_configured_model() {
        // LocalFree goal, 3rd identical failure, full ladder, within budget →
        // climb ONE rung to CheapCloud and run the model the user mapped there.
        let out = decide_escalation(
            Some(Tier::LocalFree),
            0,
            2,
            VERIFY_ESCALATE_AT,
            full_ladder,
            budget_ok(),
        );
        assert_eq!(
            out,
            EscalationOutcome::Swap {
                to_tier: Tier::CheapCloud,
                model: rm("anthropic", "claude-haiku-4-5"),
                new_escalations_used: 1,
            }
        );
    }

    #[test]
    fn cheap_cloud_escalates_to_the_frontier_model() {
        let out = decide_escalation(
            Some(Tier::CheapCloud),
            0,
            2,
            VERIFY_ESCALATE_AT,
            full_ladder,
            budget_ok(),
        );
        assert_eq!(
            out,
            EscalationOutcome::Swap {
                to_tier: Tier::Frontier,
                model: rm("anthropic", "claude-opus-4-8"),
                new_escalations_used: 1,
            }
        );
    }

    #[test]
    fn below_the_threshold_keeps_fixing_never_escalates() {
        // Transient / varying failures never reach the consecutive-same threshold.
        for n in 0..VERIFY_ESCALATE_AT {
            assert_eq!(
                decide_escalation(Some(Tier::LocalFree), 0, 2, n, full_ladder, budget_ok()),
                EscalationOutcome::KeepFixing,
                "consecutive={n} below threshold must not escalate"
            );
        }
    }

    // ── Guardrail 1: no-default (LOAD-BEARING) ───────────────────────────────

    #[test]
    fn single_model_no_ladder_never_swaps_it_parks() {
        // current == None ⇒ the user runs one model, no configured ladder. The
        // decision must NEVER inject a stronger model — it parks. This is the
        // load-bearing no-default rule.
        let out = decide_escalation(None, 0, 2, VERIFY_ESCALATE_AT, full_ladder, budget_ok());
        assert_eq!(
            out,
            EscalationOutcome::Park(ParkReason::SingleModelNoLadder)
        );
    }

    #[test]
    fn next_tier_unconfigured_parks_never_a_baked_default() {
        // A laddered start (LocalFree) but the user configured NO model for the
        // next tier (resolver returns None) ⇒ park, never the packs.rs default.
        let no_next = |_tier: Tier| None;
        let out = decide_escalation(
            Some(Tier::LocalFree),
            0,
            2,
            VERIFY_ESCALATE_AT,
            no_next,
            budget_ok(),
        );
        assert_eq!(
            out,
            EscalationOutcome::Park(ParkReason::NoConfiguredStrongerTier)
        );
    }

    /// Sibling of the two no-default tests above: when the resolver is the
    /// DERIVED best-fit map (built only from models the user actually has —
    /// here two OpenAI models the recommender ranks apart) and it names a
    /// stronger model for the next tier, the decision SWAPS to it. The
    /// pure contract is unchanged: `decide_escalation` only ever sees a
    /// resolver; the derived map is a resolver whose picks are the user's own
    /// models, never the packs.rs defaults.
    #[test]
    fn next_tier_derived_from_available_models_swaps() {
        use crate::cost_router::derived::derive_role_map;
        use crate::cost_router::recommend::AvailableModel;
        let derived = derive_role_map(&[
            AvailableModel::new("openai", "gpt-5.6"),
            AvailableModel::new("openai", "gpt-5.6-mini"),
        ]);
        let resolve_derived = |tier: Tier| {
            derived
                .get(workflow_role_for_tier(tier))
                .map(|(rm, _)| rm.clone())
        };
        let frontier = resolve_derived(Tier::Frontier)
            .expect("two floor-clearing OpenAI models derive an ORCHESTRATE (frontier) pick");
        let out = decide_escalation(
            Some(Tier::CheapCloud),
            0,
            2,
            VERIFY_ESCALATE_AT,
            resolve_derived,
            budget_ok(),
        );
        assert_eq!(
            out,
            EscalationOutcome::Swap {
                to_tier: Tier::Frontier,
                model: frontier.clone(),
                new_escalations_used: 1,
            }
        );
        // Provenance, not a vendor name: the swapped-to model is one the user has.
        assert_eq!(frontier.provider, "openai");
        assert!(frontier.model.starts_with("gpt-5.6"));
        // And the pack default is NOT what got swapped to (the user does not have it).
        let pack_default = crate::cost_router::packs::ModelPacks::default().hard;
        assert!(
            !(frontier.provider == pack_default.provider && frontier.model == pack_default.model)
        );
    }

    // ── Guardrail 2: per-goal max-escalations cap ────────────────────────────

    #[test]
    fn max_escalations_cap_parks_rather_than_climbing_to_frontier() {
        // Budget = 1 climb spent already; the next same-failure is refused even
        // though CheapCloud→Frontier is available and configured — the per-goal
        // cap, not the ladder ceiling, stops it.
        let out = decide_escalation(
            Some(Tier::CheapCloud),
            1,
            1,
            VERIFY_ESCALATE_AT,
            full_ladder,
            budget_ok(),
        );
        assert_eq!(
            out,
            EscalationOutcome::Park(ParkReason::MaxEscalationsExhausted)
        );
    }

    #[test]
    fn tier_ceiling_parks_there_is_no_stronger_tier() {
        // Frontier is the top; a generous budget cannot climb past it.
        let out = decide_escalation(
            Some(Tier::Frontier),
            0,
            9,
            VERIFY_ESCALATE_AT,
            full_ladder,
            budget_ok(),
        );
        assert_eq!(out, EscalationOutcome::Park(ParkReason::TierCeiling));
    }

    // ── Guardrail 3: spend-cap → park at budget, never overspend ─────────────

    #[test]
    fn spend_gate_parks_instead_of_escalating() {
        // Session spend at the gate ceiling ($25 default) → the verdict needs a
        // gate → escalation parks rather than adding more spend.
        let at_gate = budget_verdict(0.0, 25.0, &BudgetConfig::default());
        assert!(at_gate.needs_gate());
        let out = decide_escalation(
            Some(Tier::LocalFree),
            0,
            2,
            VERIFY_ESCALATE_AT,
            full_ladder,
            at_gate,
        );
        assert_eq!(out, EscalationOutcome::Park(ParkReason::SpendCap));
    }

    #[test]
    fn spend_hard_stop_parks() {
        let hard = budget_verdict(0.0, 50.0, &BudgetConfig::default());
        assert!(hard.must_stop());
        let out = decide_escalation(
            Some(Tier::LocalFree),
            0,
            2,
            VERIFY_ESCALATE_AT,
            full_ladder,
            hard,
        );
        assert_eq!(out, EscalationOutcome::Park(ParkReason::SpendCap));
    }

    #[test]
    fn soft_band_still_escalates_only_gate_and_hard_park() {
        // A SOFT alert (under the gate) is non-blocking — escalation proceeds.
        let soft = budget_verdict(0.0, 10.0, &BudgetConfig::default());
        assert!(!soft.needs_gate() && !soft.must_stop());
        let out = decide_escalation(
            Some(Tier::LocalFree),
            0,
            2,
            VERIFY_ESCALATE_AT,
            full_ladder,
            soft,
        );
        assert!(matches!(out, EscalationOutcome::Swap { .. }));
    }

    // ── Per-goal state advances then resets across the ladder ────────────────

    #[test]
    fn escalations_used_advances_the_climb_and_a_new_goal_starts_fresh() {
        // Two climbs in sequence (local→cheap→frontier) advance escalations_used;
        // a fresh goal (used = 0) starts the ladder over — per-goal, no bleed.
        let first = decide_escalation(
            Some(Tier::LocalFree),
            0,
            3,
            VERIFY_ESCALATE_AT,
            full_ladder,
            budget_ok(),
        );
        let EscalationOutcome::Swap {
            to_tier,
            new_escalations_used,
            ..
        } = first
        else {
            panic!("expected a swap");
        };
        assert_eq!(to_tier, Tier::CheapCloud);
        assert_eq!(new_escalations_used, 1);

        let second = decide_escalation(
            Some(to_tier),
            new_escalations_used,
            3,
            VERIFY_ESCALATE_AT,
            full_ladder,
            budget_ok(),
        );
        assert_eq!(
            second,
            EscalationOutcome::Swap {
                to_tier: Tier::Frontier,
                model: rm("anthropic", "claude-opus-4-8"),
                new_escalations_used: 2,
            }
        );

        // A DIFFERENT goal, fresh state, climbs from the bottom again.
        let fresh = decide_escalation(
            Some(Tier::LocalFree),
            0,
            3,
            VERIFY_ESCALATE_AT,
            full_ladder,
            budget_ok(),
        );
        assert!(matches!(
            fresh,
            EscalationOutcome::Swap {
                to_tier: Tier::CheapCloud,
                new_escalations_used: 1,
                ..
            }
        ));
    }

    // ── Persistence round-trips (state survives the re-dispatch) ─────────────

    #[test]
    fn state_round_trips_through_metadata() {
        // `escalated_to` is exactly what the orchestrator persists on a swap; the
        // climbed tier + handoff must survive the requeue-and-re-dispatch (a new
        // worker/session is minted, so in-memory state would be lost).
        let state =
            GoalEscalationState::escalated_to(Tier::CheapCloud, 1, "prior diff + failure".into());
        let mut meta = serde_json::Map::new();
        meta.insert(
            ESCALATION_METADATA_KEY.to_string(),
            state.to_metadata_value(),
        );

        let read = GoalEscalationState::from_metadata(&meta).expect("state present");
        assert_eq!(read, state);
        assert!(
            read.is_escalated(),
            "escalated goal runs the climbed tier's model"
        );
        assert_eq!(read.current_tier, Some(Tier::CheapCloud));
        assert_eq!(read.handoff.as_deref(), Some("prior diff + failure"));
    }

    #[test]
    fn seed_is_fresh_and_single_model_seed_has_no_tier() {
        let laddered = GoalEscalationState::seed(Some(Tier::LocalFree));
        assert_eq!(laddered.escalations_used, 0);
        assert!(!laddered.is_escalated(), "a seed is not yet escalated");

        // A single-model seed persists with no tier and reads back as None.
        let single = GoalEscalationState::seed(None);
        let mut meta = serde_json::Map::new();
        meta.insert(
            ESCALATION_METADATA_KEY.to_string(),
            single.to_metadata_value(),
        );
        let read = GoalEscalationState::from_metadata(&meta).unwrap();
        assert_eq!(read.current_tier, None);
        assert_eq!(read.escalations_used, 0);
    }

    #[test]
    fn absent_or_malformed_metadata_reads_as_none() {
        let empty = serde_json::Map::new();
        assert_eq!(GoalEscalationState::from_metadata(&empty), None);

        let mut bad = serde_json::Map::new();
        bad.insert(
            ESCALATION_METADATA_KEY.to_string(),
            serde_json::json!("not-an-object"),
        );
        assert_eq!(GoalEscalationState::from_metadata(&bad), None);

        // An unknown tier label degrades to single-model (None), not a panic.
        let mut unknown = serde_json::Map::new();
        unknown.insert(
            ESCALATION_METADATA_KEY.to_string(),
            serde_json::json!({ "tier": "quantum", "escalations_used": 2 }),
        );
        let read = GoalEscalationState::from_metadata(&unknown).unwrap();
        assert_eq!(read.current_tier, None);
        assert_eq!(read.escalations_used, 2);
    }

    // ── Carry-forward handoff (R2) ───────────────────────────────────────────

    #[test]
    fn handoff_carries_prior_model_failure_and_diff() {
        let h = build_handoff(
            "qwen3",
            "test failed: assertion left != right",
            "--- a/x.rs\n+++ b/x.rs\n@@ -1 +1 @@\n-old\n+new",
        );
        assert!(h.contains("qwen3"), "names the prior model");
        assert!(h.contains("assertion left != right"), "carries the failure");
        assert!(h.contains("+new"), "carries the diff");
        assert!(
            h.contains("Do NOT restart"),
            "instructs continue-not-restart"
        );
    }

    #[test]
    fn handoff_omits_empty_sections() {
        let h = build_handoff("qwen3", "", "");
        assert!(h.contains("qwen3"));
        assert!(
            !h.contains("```"),
            "no empty code fences when nothing captured"
        );
    }

    // ── Config wrapper ───────────────────────────────────────────────────────

    #[test]
    fn max_escalations_default_and_override() {
        assert_eq!(max_escalations_from(None), MAX_ESCALATIONS_DEFAULT);
        assert_eq!(max_escalations_from(Some(5)), 5);
    }
}
