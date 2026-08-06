//! Sampling gate for the `Brain::turn` retrieval path.
//!
//! `turn` is the read-only retrieval that hands back a receipt and learns
//! nothing until outcomes are reported — the corpus Spectral needs in order to
//! answer whether wing-scoped retrieval, the constellation tier, and the
//! reranking levers actually earn their cost. Those verdicts were all measured
//! on benchmark corpora with no wing structure, which is exactly what this
//! brain has and they lack.
//!
//! It is NOT the default recall path. The daemon enables Spectral's deferred
//! delivery write, but keeps `turn` sampled and shadowed so each selected turn
//! can measure divergence from the user-facing cascade without changing the
//! memories delivered to the model.

use std::sync::atomic::{AtomicU64, Ordering};

/// Env var holding the sample rate, 0.0–1.0. Absent or unparseable ⇒ disabled.
pub const TURN_SAMPLE_RATE_ENV: &str = "PERMAGENT_TURN_SAMPLE_RATE";

/// Monotonic turn counter backing deterministic sampling.
static TURN_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Read the configured sample rate, clamped to 0.0–1.0. Defaults to 0.0 (off).
pub fn sample_rate() -> f64 {
    std::env::var(TURN_SAMPLE_RATE_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<f64>().ok())
        .filter(|rate| rate.is_finite())
        .map(|rate| rate.clamp(0.0, 1.0))
        .unwrap_or(0.0)
}

/// Whether this turn should take the `turn` path.
///
/// Deterministic counter-based sampling rather than a random draw: at rate
/// 0.1 this fires on exactly one turn in ten instead of approximately one, so
/// a short dogfood session yields a predictable number of samples rather than
/// possibly zero. It also makes the decision reproducible in tests, which a
/// PRNG would not be.
pub fn should_sample_turn() -> bool {
    decide(sample_rate(), TURN_COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// Pure sampling decision, extracted so the rule is testable without env vars
/// or global state.
pub fn decide(rate: f64, nth: u64) -> bool {
    if rate <= 0.0 {
        return false;
    }
    if rate >= 1.0 {
        return true;
    }
    // Fire when the counter crosses another 1/rate boundary. Integer maths on
    // the scaled counter avoids float drift accumulating over a long session.
    let scaled = (nth as f64 * rate).floor() as u64;
    let prev = ((nth.saturating_sub(1)) as f64 * rate).floor() as u64;
    nth == 0 || scaled > prev
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_by_default() {
        // The path regresses p95; it must never be on unless asked for.
        assert!(!decide(0.0, 0));
        for nth in 0..100 {
            assert!(!decide(0.0, nth));
        }
    }

    #[test]
    fn full_rate_takes_every_turn() {
        for nth in 0..50 {
            assert!(decide(1.0, nth));
        }
    }

    #[test]
    fn one_in_ten_fires_ten_times_per_hundred() {
        let fired = (0..100).filter(|n| decide(0.1, *n)).count();
        assert_eq!(fired, 10, "counter sampling must be exact, not approximate");
    }

    #[test]
    fn one_in_four_fires_twenty_five_times_per_hundred() {
        let fired = (0..100).filter(|n| decide(0.25, *n)).count();
        assert_eq!(fired, 25);
    }

    #[test]
    fn always_samples_the_very_first_turn_when_enabled() {
        // A dogfood session that never samples produces no data and looks like
        // a broken feature.
        assert!(decide(0.05, 0));
    }

    #[test]
    fn rate_is_clamped_and_junk_is_ignored() {
        assert!(decide(5.0, 7), "above 1.0 behaves as always-on");
        assert!(!decide(-1.0, 7), "below 0.0 behaves as off");
        assert!(!decide(f64::NAN, 7), "NaN must not enable sampling");
    }

    #[test]
    fn sampling_is_spread_not_bunched() {
        // 1-in-5 should fire every fifth turn, not five times then never.
        let hits: Vec<u64> = (0..20).filter(|n| decide(0.2, *n)).collect();
        assert_eq!(hits, vec![0, 5, 10, 15]);
    }
}
