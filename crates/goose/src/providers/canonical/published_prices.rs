//! Hand-maintained published prices for models the generated canonical table
//! misses or has gone stale on.
//!
//! [`super::registry`] is built by `build_canonical_models` from an upstream
//! scrape. That scrape lags: a model can be selectable, billable, and running
//! for weeks before a row exists for it. When no row exists, `cost_breakdown`
//! returns `None` and the per-call ledger writes **$0.00** with
//! `is_estimated = 1` (see `agents::reply_parts`). Nothing downstream reads that
//! flag, so the budget gate in [`crate::cost_router::budget`] sums real spend as
//! zero and **never fires**.
//!
//! Measured in the live cost ledger on 2026-08-24:
//!
//! ```text
//! provider         model              calls   sum(cost_usd)  is_estimated
//! custom_deepseek  deepseek-v4-flash    128        0.00           128
//! openai           gpt-5.6-terra         60        0.00            60
//! openai           gpt-5.6-sol           23        0.00            23
//! ```
//!
//! 211 billable calls that the spend cap could not see. The 2026-08-24 health
//! review reported this as "DeepSeek's deepseek-chat model has no published
//! price"; the ledger shows the model actually in use is `deepseek-v4-flash`
//! (`deepseek-chat` IS priced upstream, which is why the report's own
//! reproduction did not find the hole).
//!
//! Rows here are consulted ONLY when the generated table has no price, so a
//! regenerated registry always wins and this file cannot silently pin a stale
//! rate.
//!
//! ## Peak vs off-peak
//!
//! DeepSeek bills half-rate outside 01:00-04:00 and 06:00-10:00 UTC Mon-Fri.
//! We record the **peak** rate. A spend gate that under-reads is the exact
//! defect being fixed here; over-estimating by at most 2x makes the ceiling
//! fire early, which is the safe direction.

use super::model::Pricing;

/// Published rates, USD per 1M tokens, as `(provider, model, pricing)`.
///
/// DeepSeek: <https://api-docs.deepseek.com/quick_start/pricing>, read
/// 2026-08-24. Peak rates; cache-hit input is the `cache_read` column and
/// cache-miss input is `input`. DeepSeek does not charge a cache-write premium,
/// so `cache_write` is `None` — which `cost_of` reads as "billed at the input
/// rate", never as "free".
const PUBLISHED: &[(&str, &str, Pricing)] = &[
    (
        "deepseek",
        "deepseek-v4-flash",
        Pricing {
            input: Some(0.44),
            output: Some(1.32),
            cache_read: Some(0.014),
            cache_write: None,
        },
    ),
    (
        "deepseek",
        "deepseek-v4-flash-vision-exp",
        Pricing {
            input: Some(0.44),
            output: Some(1.32),
            cache_read: Some(0.014),
            cache_write: None,
        },
    ),
    (
        "deepseek",
        "deepseek-v4-pro",
        Pricing {
            input: Some(1.32),
            output: Some(3.96),
            cache_read: Some(0.044),
            cache_write: None,
        },
    ),
];

/// A published price for this canonical `provider`/`model`, if one is recorded.
pub fn published_pricing(provider: &str, model: &str) -> Option<Pricing> {
    PUBLISHED
        .iter()
        .find(|(p, m, _)| p.eq_ignore_ascii_case(provider) && m.eq_ignore_ascii_case(model))
        .map(|(_, _, pricing)| *pricing)
}

/// Every model id this file prices, as `provider/model`. Used by the
/// unpriced-model regression test.
pub fn published_ids() -> Vec<String> {
    PUBLISHED
        .iter()
        .map(|(p, m, _)| format!("{p}/{m}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The row the ledger proved was missing on 2026-08-24.
    #[test]
    fn deepseek_v4_flash_is_priced() {
        let pricing =
            published_pricing("deepseek", "deepseek-v4-flash").expect("deepseek-v4-flash priced");
        assert_eq!(pricing.input, Some(0.44));
        assert_eq!(pricing.output, Some(1.32));
        assert_eq!(pricing.cache_read, Some(0.014));
    }

    /// Lookup is case-insensitive on both halves: provider ids arrive from
    /// config in whatever case the user typed.
    #[test]
    fn lookup_is_case_insensitive() {
        assert!(published_pricing("DeepSeek", "DeepSeek-V4-Flash").is_some());
    }

    #[test]
    fn unknown_models_have_no_published_price() {
        assert!(published_pricing("deepseek", "deepseek-v9-imaginary").is_none());
        assert!(published_pricing("anthropic", "deepseek-v4-flash").is_none());
    }

    /// Every recorded rate must be positive. A zero here would reintroduce the
    /// exact bug — an unpriced model that LOOKS priced.
    #[test]
    fn every_published_rate_is_positive() {
        for (provider, model, pricing) in PUBLISHED {
            let input = pricing.input.unwrap_or(0.0);
            let output = pricing.output.unwrap_or(0.0);
            assert!(input > 0.0, "{provider}/{model} has no input rate");
            assert!(output > 0.0, "{provider}/{model} has no output rate");
            assert!(
                output >= input,
                "{provider}/{model}: output should not be cheaper than input"
            );
            if let Some(cr) = pricing.cache_read {
                assert!(
                    cr > 0.0 && cr < input,
                    "{provider}/{model}: a cache read must be positive and cheaper than fresh input"
                );
            }
        }
    }
}
