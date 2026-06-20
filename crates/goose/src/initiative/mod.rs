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
//!   - the tick driver is the existing [`crate::scheduler`],
//!   - the timing/quality signal is [`crate::recognition`].
//!
//! Stage map (only `gate`/`command_counter`/`draft` are the novel origination
//! stage; `emit` is reuse):
//!   1. [`command_counter`] — deterministic pattern detection over activity.
//!   2. [`gate`]            — Tier 0 zero-token gate (the cost win).
//!   3. [`draft`]           — Tier 1 cheap-model proposal draft.
//!   4. [`emit`]            — surface as a goal card (Steward's contract).

pub mod command_counter;
pub mod draft;
pub mod emit;
pub mod gate;
pub mod tick;

pub use command_counter::{CommandCounter, CommandPattern};
pub use draft::{draft_with_provider, DraftedProposal};
pub use emit::{surface_initiative_proposal, InitiativeOutcome};
pub use gate::{evaluate, GateConfig, GateDecision, GateInputs, SkipReason};
pub use tick::{run_initiative_tick, TickOutcome};
