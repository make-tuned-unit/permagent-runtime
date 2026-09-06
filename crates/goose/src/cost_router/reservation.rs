//! Conservative, pure bounds for authorizing provider invocations.
//!
//! The reservation is a worst-case *authorization* amount, not a prediction of
//! what a model will consume.  A paid call may start only when this bound is
//! finite and positive.  Local and subscription calls return `Ok(None)` since
//! they do not create a paid hold.
//!
//! The planner intentionally does not infer a proportional token bound from
//! raw request bytes. Providers may apply server-side chat templates and add
//! BOS/EOS/control/image tokens that are absent from those bytes. A smaller
//! authorization bound therefore requires the provider adapter at the Agent
//! seam to return an authoritative, certified input-token upper bound that
//! includes all such overhead; until that seam exists this module reserves the
//! full accepted context capacity.

use crate::model::ModelConfig;
use crate::providers::canonical::{maybe_get_pricing, worst_case_pricing, Pricing};
use crate::session::CostTier;
use thiserror::Error;

/// Where the rate used for a reservation came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PricingProvenance {
    /// A complete rate was found for the requested provider/model pair.
    CanonicalExact,
    /// No complete pair-specific rate was available; the provider/registry
    /// maximum was used so an unknown model cannot evade a spend cap.
    WorstCase,
}

/// A finite upper bound for one physical provider invocation (including its
/// allowed retries).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReservationBound {
    /// Maximum dollars that may be committed by this invocation family.
    pub amount_usd: f64,
    /// Input-token upper bound reserved for the request.
    pub input_tokens: usize,
    /// Maximum output tokens per physical attempt.
    pub output_tokens: usize,
    /// Maximum number of physical attempts covered by the bound.
    pub max_physical_attempts: u32,
    pub pricing_provenance: PricingProvenance,
}

/// Fail-closed errors.  Callers must not turn any of these into a zero-dollar
/// reservation or proceed with a paid invocation.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ReservationBoundError {
    #[error("context limit is required for a paid reservation")]
    MissingContextLimit,
    #[error("maximum output tokens are required for a paid reservation")]
    MissingMaxOutput,
    #[error("context limit must be positive")]
    InvalidContextLimit,
    #[error("maximum output tokens must be positive")]
    InvalidMaxOutput,
    #[error("maximum output tokens ({output}) exceed context limit ({context})")]
    OutputExceedsContext { output: usize, context: usize },
    #[error("maximum physical attempts must be positive")]
    InvalidAttempts,
    #[error(
        "no complete pricing is available for provider {provider:?} and no worst-case rate exists"
    )]
    MissingPricing { provider: String },
    #[error("{field} pricing rate must be finite and positive")]
    InvalidRate { field: &'static str },
    #[error("reservation bound arithmetic overflowed or was not finite")]
    ArithmeticOverflow,
}

/// Resolve pricing and calculate a conservative reservation bound.
///
/// `ModelConfig::context_limit` and `ModelConfig::max_tokens` are deliberately
/// read directly.  The convenience defaults (`context_limit()` and
/// `max_output_tokens()`) are estimates and therefore cannot authorize a paid
/// call.  Input capacity is the entire accepted context minus the maximum
/// output, and the input side uses the most expensive of fresh, cache-read, and
/// cache-write rates.  The resulting one-attempt amount is multiplied by the
/// maximum number of physical attempts.
pub fn plan_reservation_bound(
    provider: &str,
    model: &str,
    cost_tier: CostTier,
    model_config: &ModelConfig,
    max_physical_attempts: u32,
) -> Result<Option<ReservationBound>, ReservationBoundError> {
    validate_attempts(max_physical_attempts)?;

    if !cost_tier.is_chargeable() {
        return Ok(None);
    }

    let (pricing, provenance) = resolve_pricing(provider, model)?;

    calculate_bound(model_config, max_physical_attempts, pricing, provenance).map(Some)
}

fn resolve_pricing(
    provider: &str,
    model: &str,
) -> Result<(Pricing, PricingProvenance), ReservationBoundError> {
    match maybe_get_pricing(provider, model) {
        Some(pricing) => Ok((pricing, PricingProvenance::CanonicalExact)),
        None => Ok((
            worst_case_pricing(provider).ok_or_else(|| ReservationBoundError::MissingPricing {
                provider: provider.to_string(),
            })?,
            PricingProvenance::WorstCase,
        )),
    }
}

fn validate_attempts(max_physical_attempts: u32) -> Result<(), ReservationBoundError> {
    if max_physical_attempts == 0 {
        Err(ReservationBoundError::InvalidAttempts)
    } else {
        Ok(())
    }
}

fn calculate_bound(
    model_config: &ModelConfig,
    max_physical_attempts: u32,
    pricing: Pricing,
    pricing_provenance: PricingProvenance,
) -> Result<ReservationBound, ReservationBoundError> {
    validate_attempts(max_physical_attempts)?;
    let context = model_config
        .context_limit
        .ok_or(ReservationBoundError::MissingContextLimit)?;
    if context == 0 {
        return Err(ReservationBoundError::InvalidContextLimit);
    }
    let output_i32 = model_config
        .max_tokens
        .ok_or(ReservationBoundError::MissingMaxOutput)?;
    let output =
        usize::try_from(output_i32).map_err(|_| ReservationBoundError::InvalidMaxOutput)?;
    if output == 0 {
        return Err(ReservationBoundError::InvalidMaxOutput);
    }
    if output > context {
        return Err(ReservationBoundError::OutputExceedsContext { output, context });
    }
    let input_rate = positive_rate(pricing.input, "input")?;
    let output_rate = positive_rate(pricing.output, "output")?;
    let cache_read_rate = optional_positive_rate(pricing.cache_read, "cache_read")?;
    let cache_write_rate = optional_positive_rate(pricing.cache_write, "cache_write")?;
    let input_rate = [Some(input_rate), cache_read_rate, cache_write_rate]
        .into_iter()
        .flatten()
        .fold(0.0_f64, f64::max);

    // Rates are all positive, so input_rate cannot be zero here.  Keep the
    // explicit check as a defence against future changes to the validation.
    if !input_rate.is_finite() || input_rate <= 0.0 {
        return Err(ReservationBoundError::ArithmeticOverflow);
    }

    let input_tokens = context - output;
    let input_cost = (input_tokens as f64 / 1_000_000.0) * input_rate;
    let output_cost = (output as f64 / 1_000_000.0) * output_rate;
    let per_attempt = input_cost + output_cost;
    let amount_usd = per_attempt * max_physical_attempts as f64;
    if !per_attempt.is_finite() || !amount_usd.is_finite() || amount_usd <= 0.0 {
        return Err(ReservationBoundError::ArithmeticOverflow);
    }

    Ok(ReservationBound {
        amount_usd,
        input_tokens,
        output_tokens: output,
        max_physical_attempts,
        pricing_provenance,
    })
}

fn positive_rate(rate: Option<f64>, field: &'static str) -> Result<f64, ReservationBoundError> {
    match rate {
        Some(rate) if rate.is_finite() && rate > 0.0 => Ok(rate),
        _ => Err(ReservationBoundError::InvalidRate { field }),
    }
}

fn optional_positive_rate(
    rate: Option<f64>,
    field: &'static str,
) -> Result<Option<f64>, ReservationBoundError> {
    rate.map(|rate| positive_rate(Some(rate), field))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(context: Option<usize>, output: Option<i32>) -> ModelConfig {
        ModelConfig {
            model_name: "test-model".to_string(),
            context_limit: context,
            max_tokens: output,
            ..ModelConfig::default()
        }
    }

    fn pricing() -> Pricing {
        Pricing {
            input: Some(2.0),
            output: Some(8.0),
            cache_read: Some(0.5),
            cache_write: Some(3.0),
        }
    }

    #[test]
    fn known_canonical_pricing_is_exact_and_uses_context_capacity() {
        let cfg = config(Some(1_000_000), Some(100_000));
        let bound = plan_reservation_bound("openai", "gpt-4o", CostTier::PaidApi, &cfg, 2)
            .unwrap()
            .unwrap();
        assert_eq!(bound.pricing_provenance, PricingProvenance::CanonicalExact);
        assert_eq!(bound.input_tokens, 900_000);
        assert_eq!(bound.output_tokens, 100_000);
        assert_eq!(bound.max_physical_attempts, 2);
        assert!(bound.amount_usd.is_finite() && bound.amount_usd > 0.0);
    }

    #[test]
    fn unknown_model_uses_worst_case_pricing() {
        let cfg = config(Some(16_000), Some(4_000));
        let bound = plan_reservation_bound(
            "provider-that-is-not-in-registry",
            "brand-new-model",
            CostTier::PaidApi,
            &cfg,
            1,
        )
        .unwrap()
        .unwrap();
        assert_eq!(bound.pricing_provenance, PricingProvenance::WorstCase);
    }

    #[test]
    fn free_tiers_do_not_require_limits_or_pricing() {
        let cfg = config(None, None);
        assert_eq!(
            plan_reservation_bound("missing", "missing", CostTier::LocalFree, &cfg, 1).unwrap(),
            None
        );
        assert_eq!(
            plan_reservation_bound("missing", "missing", CostTier::Subscription, &cfg, 1).unwrap(),
            None
        );
    }

    #[test]
    fn missing_limits_fail_closed() {
        assert_eq!(
            calculate_bound(
                &config(None, Some(100)),
                1,
                pricing(),
                PricingProvenance::CanonicalExact
            ),
            Err(ReservationBoundError::MissingContextLimit)
        );
        assert_eq!(
            calculate_bound(
                &config(Some(100), None),
                1,
                pricing(),
                PricingProvenance::CanonicalExact
            ),
            Err(ReservationBoundError::MissingMaxOutput)
        );
        assert_eq!(
            calculate_bound(
                &config(Some(100), Some(101)),
                1,
                pricing(),
                PricingProvenance::CanonicalExact
            ),
            Err(ReservationBoundError::OutputExceedsContext {
                output: 101,
                context: 100
            })
        );
    }

    #[test]
    fn invalid_rates_fail_closed() {
        for bad in [Some(0.0), Some(-1.0), Some(f64::NAN), Some(f64::INFINITY)] {
            let mut p = pricing();
            p.input = bad;
            assert_eq!(
                calculate_bound(
                    &config(Some(100), Some(20)),
                    1,
                    p,
                    PricingProvenance::CanonicalExact
                ),
                Err(ReservationBoundError::InvalidRate { field: "input" })
            );
        }
        for bad in [Some(0.0), Some(-1.0), Some(f64::NAN), Some(f64::INFINITY)] {
            let mut p = pricing();
            p.cache_write = bad;
            assert_eq!(
                calculate_bound(
                    &config(Some(100), Some(20)),
                    1,
                    p,
                    PricingProvenance::CanonicalExact
                ),
                Err(ReservationBoundError::InvalidRate {
                    field: "cache_write"
                })
            );
        }
    }

    #[test]
    fn retries_multiply_the_one_attempt_bound() {
        let cfg = config(Some(100_000), Some(10_000));
        let one = calculate_bound(&cfg, 1, pricing(), PricingProvenance::CanonicalExact).unwrap();
        let three = calculate_bound(&cfg, 3, pricing(), PricingProvenance::CanonicalExact).unwrap();
        assert!((three.amount_usd - one.amount_usd * 3.0).abs() < 1e-12);
    }

    #[test]
    fn zero_attempts_fail_closed() {
        assert_eq!(
            plan_reservation_bound(
                "openai",
                "gpt-4o",
                CostTier::PaidApi,
                &config(None, None),
                0
            ),
            Err(ReservationBoundError::InvalidAttempts)
        );
    }
}
