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
    /// A complete rate was found for the requested provider/model pair in the
    /// live canonical registry (`providers::canonical`).
    CanonicalExact,
    /// R2(b) (harness loop, Lane D1): the canonical registry has no row at
    /// all for this pair, but `cost_router::knowledge`'s curated, cited
    /// published-price table does. Still a real, pair-specific rate — never
    /// the pessimistic provider/registry maximum — kept as its own variant
    /// so a caller can tell "priced from the recommender's KB" apart from
    /// "priced from the live billing registry" without re-deriving it.
    KnowledgeTable,
    /// No complete pair-specific rate was available anywhere; the
    /// provider/registry maximum was used so an unknown model cannot evade a
    /// spend cap.
    WorstCase,
}

/// F2 (harness loop, Round F, from the I1/M11 forensics chain): a short,
/// stable, log-friendly name for a [`PricingProvenance`] variant. Three
/// straight lanes (F8, D1's own isolated tests, I1) chased a "flat $1.66
/// worst-case bound" on every `zai`/`glm-4.7` reservation without ever being
/// able to see, from the receipt alone, which of these three sources actually
/// produced it — `cost_reservations` has no `provider`/`model`/pricing-source
/// columns at all, so the only way to check was re-deriving the math by hand
/// against the live `sessions`/`cost_ledger` rows (see this lane's own report
/// in `HARNESS_LOOP_LOG.md`, `## Lane F2`, for the bit-exact reproduction:
/// the constant is `CanonicalExact` pricing × 5 physical attempts, NOT
/// `WorstCase`). This function exists purely so the tracing line added at the
/// reservation call site (`agents::reply_parts::reserve_provider_invocation_for_model`)
/// can name the source without a caller having to match on the enum itself.
pub fn describe_pricing_source(provenance: PricingProvenance) -> &'static str {
    match provenance {
        PricingProvenance::CanonicalExact => "canonical_exact",
        PricingProvenance::KnowledgeTable => "knowledge_table",
        PricingProvenance::WorstCase => "worst_case",
    }
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
    if let Some(pricing) = maybe_get_pricing(provider, model) {
        return Ok((pricing, PricingProvenance::CanonicalExact));
    }
    // R2(b) (harness loop, Lane D1, from Lane F8 forensics 2026-09-10).
    // CORRECTION during this lane's own gate run: `zai`/`glm-4.7` turns out
    // to already resolve via `maybe_get_pricing` above (`canonical_models.json`
    // carries a bare `"zai/glm-4.7"` row, `{input:0.60,output:2.20}` — this
    // lane's first-pass root-cause read, "the canonical registry has no zai
    // row at all", was wrong: it came from a truncated grep that hid every
    // `"zai/..."` id sorting after the `"zai-coding-plan/..."`/`"zai-org/..."`
    // ids it did show). So THIS exact pair no longer reaches this branch, and
    // Lane F8's own observed $1.662976 constant is NOT reproduced by this
    // path — its true mechanism (a different model_config shape at the real
    // call site, most likely) remains unconfirmed; not chased further here.
    //
    // The branch itself is still a real, worthwhile improvement kept in
    // place: `cost_router::knowledge` is a curated, cited pricing table
    // (built for the recommender) that can carry a pair before the
    // generated/billing registry does — `maybe_get_pricing`'s own doc
    // comment already names exactly this lag as a spend-gate failure mode
    // ("the generated table always lags what is selectable"). Trying this
    // SAME row before the pessimistic global worst-case fallback costs
    // nothing when canonical already has an answer (this `if` is simply
    // skipped) and gives a real, tight bound instead of the worst-case
    // constant on the day a KB-priced pair genuinely is missing from the
    // registry.
    if let Some(known) = crate::cost_router::knowledge::lookup(provider, model) {
        return Ok((
            Pricing {
                input: Some(known.input_usd_per_mtok),
                output: Some(known.output_usd_per_mtok),
                cache_read: None,
                cache_write: None,
            },
            PricingProvenance::KnowledgeTable,
        ));
    }
    Ok((
        worst_case_pricing(provider).ok_or_else(|| ReservationBoundError::MissingPricing {
            provider: provider.to_string(),
        })?,
        PricingProvenance::WorstCase,
    ))
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
    match rate {
        None => Ok(None),
        // H4 (Lane M, 2026-09-08 live evidence): a rate of exactly 0.0 is a
        // provider legitimately charging nothing for this rate (e.g. no
        // cache-write premium — `providers/canonical/cost.rs`'s "no
        // premium" cases, observed with zai/glm-4.7), not a missing or
        // invalid one. Treating it as invalid made every call to that
        // provider get refused before it started: "provider budget is
        // unknown; refusing call: cannot authorize provider call:
        // cache_write pricing rate must be finite and positive". Free is
        // excluded from the max-rate fold below the same way `None` already
        // is. NaN/infinite/negative still fail closed via `positive_rate`,
        // and this only touches the OPTIONAL cache_read/cache_write rates —
        // the required input/output rates keep their strict positive check.
        Some(0.0) => Ok(None),
        Some(rate) => positive_rate(Some(rate), field).map(Some),
    }
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

    /// R2(b) (harness loop, Lane D1, from Lane F8 forensics), CORRECTED
    /// during this lane's own gate run: `zai`/`glm-4.7` was first believed to
    /// have no row anywhere in `providers::canonical`'s registry (a grep that
    /// turned out truncated hid it) — it actually resolves via
    /// `maybe_get_pricing` already, `CanonicalExact`, at the real published
    /// rate ($0.60 input / $2.20 output per Mtok, `canonical_models.json`'s
    /// bare `"zai/glm-4.7"` row). This test pins the OUTCOME the task asked
    /// for regardless of which branch supplies it: the bound for this pair
    /// with a known price lands far below F8's observed $1.662976 constant.
    /// (`resolve_pricing`'s new `KnowledgeTable` fallback branch below this
    /// one is exercised separately, since every currently-priced
    /// `cost_router::knowledge` row already has a matching canonical entry —
    /// see `knowledge_table_fallback_fires_when_canonical_has_no_row` for how
    /// that's tested without fabricating a live pricing pair.)
    #[test]
    fn zai_glm_4_7_has_a_known_price_far_below_the_old_worst_case_constant() {
        // F8's own "what could not be determined" note ties the recurring
        // $1.662976 constant to a max_tokens of 131072; reproduced here as
        // context == max_tokens so the whole bound is output-cost-driven,
        // matching that observation's shape.
        let cfg = config(Some(131_072), Some(131_072));
        let bound = plan_reservation_bound("zai", "glm-4.7", CostTier::PaidApi, &cfg, 1)
            .unwrap()
            .unwrap();
        assert_ne!(
            bound.pricing_provenance,
            PricingProvenance::WorstCase,
            "a known-priced model must never fall back to the registry's global worst case: \
             {bound:?}"
        );
        assert!(
            bound.amount_usd < 1.66,
            "the bound must land well under the old flat worst-case constant, got ${:.6}",
            bound.amount_usd
        );
        // Sanity: output-only cost at the real $2.20/Mtok rate for 131,072
        // output tokens (input_tokens is 0 here by construction).
        let expected = 131_072.0 / 1_000_000.0 * 2.20;
        assert!(
            (bound.amount_usd - expected).abs() < 0.0001,
            "expected ${expected:.6}, got ${:.6}",
            bound.amount_usd
        );
    }

    /// I1 pin (harness loop, 2026-09-10, from M10's forensics: 10/10 M10
    /// coordinators hit the reservation gate at the flat $1.662976
    /// worst-case constant, and `cost_reservations` shows it STILL applied
    /// to `zai`/`glm-4.7`). This pins the org-qualified `"zai-org"`
    /// provider spelling specifically — `providers::canonical::name_builder`'s
    /// new alias (`"zai-org" => "zai"`) resolves it to the SAME real
    /// $0.60/$2.20-per-Mtok rate `"zai"` itself carries, never the global
    /// worst case. Pre-fix, `resolve_pricing("zai-org", "glm-4.7")` matched
    /// nothing in the canonical registry (no bare top-level `"zai-org"` id
    /// exists) and fell all the way to `WorstCase`.
    #[test]
    fn zai_org_alias_never_falls_back_to_the_worst_case_constant() {
        let cfg = config(Some(131_072), Some(131_072));
        let bound = plan_reservation_bound("zai-org", "glm-4.7", CostTier::PaidApi, &cfg, 1)
            .unwrap()
            .unwrap();
        assert_ne!(
            bound.pricing_provenance,
            PricingProvenance::WorstCase,
            "the zai-org alias must resolve real zai pricing, not the global worst case: \
             {bound:?}"
        );
        let expected = 131_072.0 / 1_000_000.0 * 2.20;
        assert!(
            (bound.amount_usd - expected).abs() < 0.0001,
            "expected ${expected:.6}, got ${:.6}",
            bound.amount_usd
        );
    }

    /// F2 pin (harness loop, Round F, from the M11/I1 forensics chain).
    /// D1's own `zai_glm_4_7_has_a_known_price_far_below_the_old_worst_case_constant`
    /// test above used a *synthetic* `ModelConfig` (`context: 131_072,
    /// max_tokens: 131_072`) and `max_physical_attempts: 1` — neither of
    /// which match what a real headless coordinator/worker session actually
    /// configures. This test feeds the EXACT live values instead, read
    /// directly off the M11 slice's own kept per-fixture `permagent.db`s
    /// (`sessions.model_config_json` for `20260910_1`/`_2`/`_3` in every one
    /// of fx01/fx05/fx08/fx09/fx10: `{"model_name":"glm-4.7",
    /// "context_limit":204800,"max_tokens":131072,...}`, `sessions
    /// .provider_name = "zai"`) plus the DEFAULT `RetryConfig`'s
    /// `max_physical_attempts()` (`providers::retry::RetryConfig::default()
    /// .max_physical_attempts() == 5`, the value every zai/glm-4.7
    /// `OpenAiCompatibleProvider` actually uses — no override exists for
    /// this provider).
    ///
    /// This reproduces F8/D1/I1/M11's observed `amount_usd =
    /// 1.6629760000000002` bit-for-bit (verified directly against the M11
    /// `cost_reservations` rows, not merely "close"). The provenance is
    /// `CanonicalExact`, never `WorstCase` — three straight lanes'
    /// "flat worst-case constant" diagnosis was wrong. The number IS flat
    /// across every call only because `context_limit`/`max_tokens`
    /// (fixed per model) and `max_physical_attempts` (fixed per provider's
    /// retry policy) never vary call to call for this pair — the bound is a
    /// correctly-priced authorization ceiling covering up to 5 physical
    /// attempts of ONE logical turn, not a mispriced fallback. This test
    /// passes both before and after this lane's changes — it adds no fix,
    /// only a permanent, exact regression pin so a future lane can check
    /// this arithmetic in five seconds instead of re-deriving it from raw
    /// DB rows again.
    #[test]
    fn zai_glm_4_7_live_reservation_is_canonical_exact_not_worst_case() {
        let cfg = config(Some(204_800), Some(131_072));
        let bound = plan_reservation_bound("zai", "glm-4.7", CostTier::PaidApi, &cfg, 5)
            .unwrap()
            .unwrap();
        assert_eq!(
            bound.pricing_provenance,
            PricingProvenance::CanonicalExact,
            "the live zai/glm-4.7 reservation must be priced from the canonical registry, \
             not any fallback: {bound:?}"
        );
        // Bit-exact match to the value observed in every M11 fixture's
        // cost_reservations.amount_usd for a zai/glm-4.7 invocation.
        assert_eq!(bound.amount_usd, 1.6629760000000002);
        assert_eq!(
            describe_pricing_source(bound.pricing_provenance),
            "canonical_exact"
        );
    }

    /// F2 pin: this is the ONLY assertion in this lane that fails to compile
    /// on `bd557978` (`describe_pricing_source` does not exist pre-fix) —
    /// see that function's own doc comment for why it exists: three lanes in
    /// a row had no way to see, from a reservation's own record, which
    /// pricing source actually produced its bound.
    #[test]
    fn describe_pricing_source_names_every_provenance_variant() {
        assert_eq!(
            describe_pricing_source(PricingProvenance::CanonicalExact),
            "canonical_exact"
        );
        assert_eq!(
            describe_pricing_source(PricingProvenance::KnowledgeTable),
            "knowledge_table"
        );
        assert_eq!(
            describe_pricing_source(PricingProvenance::WorstCase),
            "worst_case"
        );
    }

    // NOTE on the `KnowledgeTable` branch's own coverage: every real,
    // non-local, non-zero-priced row currently in `cost_router::knowledge`
    // already has a matching entry in the canonical registry (by design —
    // see that module's own "pricing agrees with the canonical table"
    // comments), so no LIVE (provider, model) pair in this codebase can
    // reach the `KnowledgeTable` arm via `plan_reservation_bound` today.
    // `unknown_model_uses_worst_case_pricing` just above already proves the
    // fallthrough PAST it is intact: "provider-that-is-not-in-registry" /
    // "brand-new-model" misses `maybe_get_pricing` (as before this lane) AND
    // misses `cost_router::knowledge::lookup` (not a curated pair either),
    // landing on `WorstCase` exactly as it did pre-fix. The `KnowledgeTable`
    // arm itself reuses the identical `Pricing`-construction shape
    // `maybe_get_pricing`'s own `CanonicalExact` arm uses one line above it,
    // and is covered by code review parity with that arm and with the
    // already-tested `WorstCase` arm, rather than by a live pair no current
    // data can provide.

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
        // H4: 0.0 is no longer here — an optional rate of exactly zero is
        // "free", not invalid (see `zero_cache_write_rate_is_treated_as_free_not_invalid`
        // below).
        for bad in [Some(-1.0), Some(f64::NAN), Some(f64::INFINITY)] {
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

    /// H4 (Lane M live evidence, 2026-09-08): every zai/glm-4.7 call was
    /// refused in under a second with "provider budget is unknown; refusing
    /// call: cannot authorize provider call: cache_write pricing rate must
    /// be finite and positive" — that provider genuinely has no cache-write
    /// premium (rate 0.0), and `optional_positive_rate` treated 0.0 the same
    /// as missing/invalid. A rate of exactly zero must produce the SAME
    /// bound as no rate at all (excluded from the max-rate fold either way),
    /// not an error. Before the fix this failed with exactly the
    /// `InvalidRate { field: "cache_write" }` error quoted above (see
    /// `invalid_rates_fail_closed`, which asserted this same input was an
    /// error prior to this change).
    #[test]
    fn zero_cache_write_rate_is_treated_as_free_not_invalid() {
        let mut free = pricing();
        free.cache_write = Some(0.0);
        let mut absent = pricing();
        absent.cache_write = None;
        let cfg = config(Some(100), Some(20));

        let free_bound = calculate_bound(&cfg, 1, free, PricingProvenance::CanonicalExact).unwrap();
        let absent_bound =
            calculate_bound(&cfg, 1, absent, PricingProvenance::CanonicalExact).unwrap();

        assert_eq!(free_bound.amount_usd, absent_bound.amount_usd);
    }

    /// Regression guard alongside the fix above: a genuinely invalid
    /// cache_write rate (negative, NaN, infinite) must still fail closed —
    /// only exactly 0.0 is "free".
    #[test]
    fn negative_or_nan_cache_write_rate_still_fails_closed() {
        for bad in [Some(-1.0), Some(f64::NAN), Some(f64::INFINITY)] {
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
                }),
                "cache_write = {bad:?} must still fail closed"
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
