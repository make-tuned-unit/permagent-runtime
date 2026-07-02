//! Initiative layer (#360) — ambient goal-ORIGINATION from unscoped personal
//! observation.
//!
//! This is the one novel organ in the system: it mints a *new* goal from
//! passively-observed user activity, rather than executing intent a human or
//! recipe already supplied. The unscoped-perception input that makes it
//! possible is the always-on [`crate::activity`] layer — the signal no
//! cloud-hosted agent can have, because a user will not let a remote agent
//! watch everything.
//!
//! Everything DOWNSTREAM of origination is deliberately reused, not rebuilt:
//!   - the proposal sink is the Steward's card seam ([`emit`] mirrors
//!     `steward::surface_destructive_proposal` → `cards::create_card`),
//!   - the tick driver is a native interval loop ([`driver`], per the W6
//!     ruling — the scheduler's role is executing *approved* automations),
//!   - the timing/quality signal is [`crate::recognition`].
//!
//! Stage map (only `gate`/`command_counter`/`draft` are the novel origination
//! stage; `emit` is reuse):
//!   1. [`command_counter`] — deterministic pattern detection over activity.
//!   2. [`gate`]            — Tier 0 zero-token gate (the cost win).
//!   3. [`draft`]           — Tier 1 cheap-model proposal draft.
//!   4. [`emit`]            — surface as a goal card (Steward's contract).
//!   5. [`driver`]          — the native host loop (#360 wiring).

pub mod command_counter;
pub mod draft;
pub mod driver;
pub mod emit;
pub mod gate;
pub mod tick;

pub use command_counter::{CommandCounter, CommandPattern};
pub use draft::{draft_with_provider, DraftedProposal};
pub use emit::{surface_initiative_proposal, InitiativeOutcome};
pub use gate::{evaluate, GateConfig, GateDecision, GateInputs, SkipReason};
pub use tick::{run_initiative_tick, TickOutcome};

/// The Initiative layer as a background worker Henry must be able to describe.
/// Surfaced in the `<permagent_self>` brief under "Workers". Always listed;
/// the live-state arm reports the real on/off switch (state-label, not
/// present/absent), so a disabled driver is described honestly rather than
/// hidden or over-claimed.
pub const SELF_KNOWLEDGE_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "initiative",
        display_name: "Initiative",
        category: crate::agents::self_knowledge::FeatureCategory::Worker,
        what_it_does: "An ambient origination loop that passively watches activity for repeated \
             successful terminal commands. When the same command crosses a repeat threshold \
             and the user has been quiet for a few minutes, it drafts an automation proposal \
             (cheap local model with a deterministic template fallback) and surfaces it as a \
             goal card in the Triage column — it only ever proposes, never acts. A proposal \
             the user declined is remembered and never re-pitched",
        why_it_matters:
            "It is how the agent notices work worth automating without being asked — the one \
             capability that requires watching everything locally. Every proposal is a \
             visible card the user approves or archives; nothing fires silently and there is \
             a cooldown between proposals",
        state_source: crate::agents::self_knowledge::StateSource::Queryable,
        teaching: &[],
    };
