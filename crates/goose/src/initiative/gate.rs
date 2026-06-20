//! W2 — Tier 0 deterministic, zero-token gate.
//!
//! This is the structural cost win: origination spends a token ONLY when a free
//! signal says it's worth it. The gate is a pure function over values the tick
//! assembles (from [`crate::activity`]'s live snapshot and persisted cursors) —
//! it performs no IO and no model call, so it is fully deterministic and
//! testable. The tick (W6) owns loading/storing the cursors as Config flags.
//!
//! All four guards must pass to reach the (token-spending) Tier 1 draft:
//!   1. activity-delta — observation changed since the last tick. Kills the
//!      common idle tick for free.
//!   2. not-engaged    — the user is not mid-interaction. This is also the
//!      anti-creepy mechanism: never originate while the user is actively
//!      working.
//!   3. cooldown       — no proposal fired within the cooldown window (anti-spam).
//!   4. pattern        — a concrete deterministic pattern crossed threshold
//!      (the only guard that can carry a candidate forward).

use crate::initiative::command_counter::CommandPattern;
use chrono::{DateTime, Duration, Utc};

/// Default quiet window before the user is considered "not engaged".
pub const DEFAULT_QUIET_SECS: i64 = 5 * 60; // 5 min
/// Default cooldown between surfaced proposals.
pub const DEFAULT_COOLDOWN_SECS: i64 = 6 * 60 * 60; // 6 hours

/// Inputs the tick gathers before calling [`evaluate`]. Pure data — no handles.
#[derive(Debug, Clone)]
pub struct GateInputs {
    /// Current tick time.
    pub now: DateTime<Utc>,
    /// Timestamp of the most recent observed activity (live snapshot). `None`
    /// when nothing has ever been observed.
    pub last_activity_at: Option<DateTime<Utc>>,
    /// `last_activity_at` as seen at the previous tick (the delta cursor).
    pub prev_tick_activity_at: Option<DateTime<Utc>>,
    /// Most recent direct user engagement (chat turn / active session). `None`
    /// if never engaged.
    pub last_engagement_at: Option<DateTime<Utc>>,
    /// When the last initiative proposal was surfaced (cooldown cursor).
    pub last_proposal_at: Option<DateTime<Utc>>,
    /// The threshold-crossed pattern from W3, if any.
    pub pattern: Option<CommandPattern>,
}

/// Tunable windows for the gate.
#[derive(Debug, Clone, Copy)]
pub struct GateConfig {
    pub quiet_secs: i64,
    pub cooldown_secs: i64,
}

impl Default for GateConfig {
    fn default() -> Self {
        Self {
            quiet_secs: DEFAULT_QUIET_SECS,
            cooldown_secs: DEFAULT_COOLDOWN_SECS,
        }
    }
}

/// Why a tick was stopped before spending a token (telemetry + tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// No new activity since the last tick.
    NoActivityDelta,
    /// The user is actively engaged (also the anti-creepy guard).
    UserEngaged,
    /// A proposal fired too recently.
    Cooldown,
    /// No deterministic pattern crossed threshold.
    NoPattern,
}

/// Outcome of the Tier 0 gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    /// Proceed to Tier 1 with the carried pattern (spends a token).
    Proceed(CommandPattern),
    /// Stop for free.
    Skip(SkipReason),
}

/// Evaluate the four guards in cheap-first order. The first failing guard wins.
pub fn evaluate(input: &GateInputs, cfg: &GateConfig) -> GateDecision {
    // 1. activity-delta: did observation advance since last tick?
    let advanced = match (input.last_activity_at, input.prev_tick_activity_at) {
        (Some(latest), Some(prev)) => latest > prev,
        (Some(_), None) => true, // first-ever activity counts as a delta
        (None, _) => false,      // nothing observed yet
    };
    if !advanced {
        return GateDecision::Skip(SkipReason::NoActivityDelta);
    }

    // 2. not-engaged: respect an active user (anti-creepy).
    if let Some(eng) = input.last_engagement_at {
        if input.now - eng < Duration::seconds(cfg.quiet_secs.max(0)) {
            return GateDecision::Skip(SkipReason::UserEngaged);
        }
    }

    // 3. cooldown: anti-spam.
    if let Some(last) = input.last_proposal_at {
        if input.now - last < Duration::seconds(cfg.cooldown_secs.max(0)) {
            return GateDecision::Skip(SkipReason::Cooldown);
        }
    }

    // 4. pattern: the only guard that can carry a candidate.
    match &input.pattern {
        Some(p) => GateDecision::Proceed(p.clone()),
        None => GateDecision::Skip(SkipReason::NoPattern),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    fn pattern() -> CommandPattern {
        CommandPattern {
            normalized: "npm run build".into(),
            count: 3,
            exemplars: vec!["npm run build".into()],
        }
    }

    /// All guards green → Proceed with the pattern.
    fn passing_inputs() -> GateInputs {
        GateInputs {
            now: t(100_000),
            last_activity_at: Some(t(99_000)),
            prev_tick_activity_at: Some(t(98_000)),
            last_engagement_at: Some(t(0)), // long ago → not engaged
            last_proposal_at: None,
            pattern: Some(pattern()),
        }
    }

    #[test]
    fn all_guards_pass_proceeds() {
        let d = evaluate(&passing_inputs(), &GateConfig::default());
        assert_eq!(d, GateDecision::Proceed(pattern()));
    }

    #[test]
    fn no_activity_delta_skips() {
        let mut i = passing_inputs();
        i.last_activity_at = Some(t(98_000));
        i.prev_tick_activity_at = Some(t(98_000)); // unchanged
        assert_eq!(
            evaluate(&i, &GateConfig::default()),
            GateDecision::Skip(SkipReason::NoActivityDelta)
        );
    }

    #[test]
    fn no_activity_ever_skips() {
        let mut i = passing_inputs();
        i.last_activity_at = None;
        assert_eq!(
            evaluate(&i, &GateConfig::default()),
            GateDecision::Skip(SkipReason::NoActivityDelta)
        );
    }

    #[test]
    fn engaged_user_skips_even_with_pattern() {
        let mut i = passing_inputs();
        i.last_engagement_at = Some(t(99_900)); // 100s ago < 5min quiet window
        assert_eq!(
            evaluate(&i, &GateConfig::default()),
            GateDecision::Skip(SkipReason::UserEngaged)
        );
    }

    #[test]
    fn cooldown_skips() {
        let mut i = passing_inputs();
        i.last_proposal_at = Some(t(99_000)); // 1000s ago < 6h cooldown
        assert_eq!(
            evaluate(&i, &GateConfig::default()),
            GateDecision::Skip(SkipReason::Cooldown)
        );
    }

    #[test]
    fn no_pattern_skips() {
        let mut i = passing_inputs();
        i.pattern = None;
        assert_eq!(
            evaluate(&i, &GateConfig::default()),
            GateDecision::Skip(SkipReason::NoPattern)
        );
    }

    #[test]
    fn first_ever_activity_counts_as_delta() {
        let mut i = passing_inputs();
        i.prev_tick_activity_at = None;
        assert_eq!(
            evaluate(&i, &GateConfig::default()),
            GateDecision::Proceed(pattern())
        );
    }
}
