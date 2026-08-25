//! The Forecaster — where the market around each project is going.
//!
//! Not the Seer. The internal series Permagent owns (goal velocity, spend,
//! analytics) are 10–120 spiky points and no model recovers signal from that;
//! `UNLAZY_TIMESFM_DEEPAGENTS_2026-08-24.md` §3 works through the arithmetic
//! and correctly refuses. The Forecaster looks *outward* instead — at other
//! people's numbers, the one class of series that is already long, already
//! public, and already free to backfill.
//!
//! Three rules hold this module together:
//!
//! 1. **No second subject list.** A series hangs off a `project_intel` row that
//!    a human already approved. The Ecosystem panel and the Market card are one
//!    concept, so they cannot disagree about who the competition is.
//! 2. **Backfill or nothing.** A source that only starts accumulating today is
//!    useless for six months. [`series::SourceKind::backfills`] is the
//!    membership test, and the one source that fails it is labelled
//!    snapshot-only rather than quietly presented as a trend.
//! 3. **Every forecast carries the method that produced it.** Not an
//!    `Option<String>`, not a default — the same discipline as
//!    `SpendForecast.method`, which is literally labelled *"this is a trailing
//!    average, not a model"*.
//!
//! Storing history is a deliberate departure from the Financier, which never
//! stores a quote. Scoped, not broken: the Financier answers *what is it now*,
//! the Forecaster *where is it going*, and only the second needs a past.

pub mod backtest;
pub mod baseline;
pub mod collect;
pub mod forecast;
pub mod series;
pub mod store;

pub use forecast::{Forecast, Method, Refusal};
pub use series::{Cadence, Series, SeriesStatus, SourceKind};

/// The Forecaster's tunable behaviour, all of it read from the same config
/// store the RSI threshold uses (`Config::global().get_param`).
///
/// Each field is an open question from the design with a decided default. They
/// are knobs rather than constants because the honest answer to several of them
/// is "we will know in three months of collected data".
#[derive(Debug, Clone, PartialEq)]
pub struct Knobs {
    /// How a bound series becomes `active`.
    ///
    /// Default `ReviewGate`: reuse `project_intel`'s existing propose/approve
    /// surface rather than inventing a second one. `SelfBindApprovedIntel` lets
    /// the Forecaster activate a series whose `intel_id` points at a
    /// `kind='competitor'` row a human already approved — the subject was
    /// reviewed, only the metric is new.
    pub approval: Approval,
    /// How often collectors run. Default weekly (Sunday night): direction does
    /// not move on a one-day scale, and weekly halves the row count. Daily
    /// costs nothing extra in dollars and makes the 180-point minimum reachable
    /// sooner for snapshot-only sources.
    pub cadence: CollectionCadence,
    /// What the Market card shows for a project with no real market signal —
    /// the five nonprofit/community projects. Default: say so.
    pub no_signal: NoSignalDisplay,
    /// Apply the per-source subject alias table on bind. Default on.
    pub normalize_subjects: bool,
    /// Persist Yahoo daily closes into `forecaster_points`.
    ///
    /// Default **off**. `market_data.rs` already documents that endpoint as
    /// "not a supported API"; reading it at request time is a tolerated
    /// dependency, and writing months of it into our own database would make it
    /// a durable one. Equity series are served from the Financier's read-time
    /// fetch instead.
    pub persist_equity_closes: bool,
}

/// Config keys. Named so `permagent config` shows them together.
pub const APPROVAL_KEY: &str = "forecaster_approval";
pub const CADENCE_KEY: &str = "forecaster_collection_cadence";
pub const NO_SIGNAL_KEY: &str = "forecaster_no_signal_display";
pub const NORMALIZE_SUBJECTS_KEY: &str = "forecaster_normalize_subjects";
pub const PERSIST_EQUITY_KEY: &str = "forecaster_persist_equity_closes";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    ReviewGate,
    SelfBindApprovedIntel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionCadence {
    Daily,
    Weekly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoSignalDisplay {
    /// Render "no market series bound" — the honest state.
    HonestState,
    /// Omit the card entirely for that project.
    Hidden,
}

impl Default for Knobs {
    fn default() -> Self {
        Self {
            approval: Approval::ReviewGate,
            cadence: CollectionCadence::Weekly,
            no_signal: NoSignalDisplay::HonestState,
            normalize_subjects: true,
            persist_equity_closes: false,
        }
    }
}

impl Knobs {
    /// Read the knobs from config, falling back to the decided default for any
    /// key that is unset or unparseable. An unparseable value is a typo, and a
    /// typo must not silently change behaviour in the permissive direction.
    pub fn load() -> Self {
        let cfg = crate::config::Config::global();
        let mut k = Self::default();
        if let Ok(v) = cfg.get_param::<String>(APPROVAL_KEY) {
            match v.trim().to_ascii_lowercase().as_str() {
                "review_gate" => k.approval = Approval::ReviewGate,
                "self_bind_approved_intel" => k.approval = Approval::SelfBindApprovedIntel,
                other => tracing::warn!(
                    target: "permagent::forecaster",
                    "{APPROVAL_KEY}=\"{other}\" is not a setting; keeping review_gate"
                ),
            }
        }
        if let Ok(v) = cfg.get_param::<String>(CADENCE_KEY) {
            match v.trim().to_ascii_lowercase().as_str() {
                "daily" => k.cadence = CollectionCadence::Daily,
                "weekly" => k.cadence = CollectionCadence::Weekly,
                other => tracing::warn!(
                    target: "permagent::forecaster",
                    "{CADENCE_KEY}=\"{other}\" is not a cadence; keeping weekly"
                ),
            }
        }
        if let Ok(v) = cfg.get_param::<String>(NO_SIGNAL_KEY) {
            match v.trim().to_ascii_lowercase().as_str() {
                "honest_state" | "honest" => k.no_signal = NoSignalDisplay::HonestState,
                "hidden" | "hide" => k.no_signal = NoSignalDisplay::Hidden,
                other => tracing::warn!(
                    target: "permagent::forecaster",
                    "{NO_SIGNAL_KEY}=\"{other}\" is not a setting; keeping honest_state"
                ),
            }
        }
        if let Ok(v) = cfg.get_param::<bool>(NORMALIZE_SUBJECTS_KEY) {
            k.normalize_subjects = v;
        }
        if let Ok(v) = cfg.get_param::<bool>(PERSIST_EQUITY_KEY) {
            k.persist_equity_closes = v;
        }
        k
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_decided_defaults_are_the_conservative_ones() {
        let k = Knobs::default();
        // Approval stays with the human surface that already exists.
        assert_eq!(k.approval, Approval::ReviewGate);
        // Weekly: direction does not move on a one-day scale.
        assert_eq!(k.cadence, CollectionCadence::Weekly);
        // A project with no market signal says so.
        assert_eq!(k.no_signal, NoSignalDisplay::HonestState);
        assert!(k.normalize_subjects);
        // The unofficial endpoint stays a read-time dependency, not a stored one.
        assert!(!k.persist_equity_closes);
    }
}
