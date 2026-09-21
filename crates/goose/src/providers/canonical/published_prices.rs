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
    // Anthropic, read from https://platform.claude.com/docs/en/about-claude/pricing
    // on 2026-09-21. The generated registry carries no row for any of these,
    // so before this they billed at `worst_case_pricing` — o1-pro's
    // $150/$600 per Mtok — which both overstates the ledger and inflates
    // every reservation bound computed from it. One Fable call on
    // 2026-09-20 was recorded that way.
    //
    // `cache_write` is the 5-minute write (1.25x input); the 1-hour write
    // (2x) is a different product and is not what this pipeline reserves.
    (
        "anthropic",
        "claude-fable-5",
        Pricing {
            input: Some(10.0),
            output: Some(50.0),
            // Fable 5 reads at the standard 0.1x. Fable 5.1 is 0.025x — a
            // different number for a different model, not a typo.
            cache_read: Some(1.0),
            cache_write: Some(12.50),
        },
    ),
    (
        "anthropic",
        "claude-fable-5.1",
        Pricing {
            input: Some(10.0),
            output: Some(50.0),
            cache_read: Some(0.25),
            cache_write: Some(12.50),
        },
    ),
    (
        "anthropic",
        "claude-opus-5",
        Pricing {
            input: Some(5.0),
            output: Some(25.0),
            cache_read: Some(0.50),
            cache_write: Some(6.25),
        },
    ),
];

/// Prices the generated registry has WRONG, not merely missing.
///
/// [`PUBLISHED`] only backfills a row with no price, so it cannot repair one
/// that is present and wrong — and a wrong row is worse than a missing one,
/// because nothing flags it. Entries here therefore OVERRIDE the registry.
///
/// Each entry must cite where the number came from and when it was read, and
/// must be deleted the moment a regenerated registry agrees with it: an
/// override that outlives the defect it fixes is how a stale price gets
/// pinned forever. `corrections_are_still_needed` in this module's tests
/// fails when the registry catches up, so the deletion cannot be forgotten.
const CORRECTIONS: &[(&str, &str, Pricing)] = &[
    // Read from https://platform.claude.com/docs/en/about-claude/pricing on
    // 2026-09-21. The registry carries the OLD $3/$15 rate. That page states
    // the $2/$10 launch price became standard and the increase to $3/$15
    // scheduled for 2026-09-01 did not happen. Sonnet 5 is this install's
    // single largest line — 400 calls and a recorded $41.15 in the first
    // three weeks of September — so a 50% overstatement here distorts the
    // spend figures the user reads AND every bound derived from them.
    (
        "anthropic",
        "claude-sonnet-5",
        Pricing {
            input: Some(2.0),
            output: Some(10.0),
            cache_read: Some(0.20),
            cache_write: Some(2.50),
        },
    ),
];

/// A correction for this canonical `provider`/`model`, if one is recorded.
pub fn corrected_pricing(provider: &str, model: &str) -> Option<Pricing> {
    CORRECTIONS
        .iter()
        .find(|(p, m, _)| p.eq_ignore_ascii_case(provider) && m.eq_ignore_ascii_case(model))
        .map(|(_, _, pricing)| *pricing)
}

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

    /// Every correction must still be needed. When a regenerated registry
    /// carries the right number, the override has outlived its defect and
    /// must be deleted — otherwise it silently pins a price that upstream
    /// has since changed. This test is what makes that deletion happen.
    #[test]
    fn corrections_are_still_needed() {
        let registry = super::super::CanonicalModelRegistry::bundled().unwrap();
        for (provider, model, corrected) in CORRECTIONS {
            let row = registry.get(provider, model).unwrap_or_else(|| {
                panic!("{provider}/{model} left the registry — drop the correction")
            });
            assert_ne!(
                (row.cost.input, row.cost.output),
                (corrected.input, corrected.output),
                "the registry now agrees with the correction for {provider}/{model} — delete it",
            );
        }
    }

    /// The published Anthropic rates this install bills against, as read from
    /// platform.claude.com on 2026-09-21. Spelled out here so a change to the
    /// table is a deliberate edit with a source, never a silent drift.
    #[test]
    fn anthropic_rates_match_the_published_table() {
        for (model, input, output, cache_read, cache_write) in [
            ("claude-fable-5", 10.0, 50.0, 1.0, 12.50),
            ("claude-fable-5.1", 10.0, 50.0, 0.25, 12.50),
            ("claude-opus-5", 5.0, 25.0, 0.50, 6.25),
        ] {
            let p = published_pricing("anthropic", model)
                .unwrap_or_else(|| panic!("{model} must carry a published price"));
            assert_eq!(
                (p.input, p.output, p.cache_read, p.cache_write),
                (
                    Some(input),
                    Some(output),
                    Some(cache_read),
                    Some(cache_write)
                ),
                "{model}",
            );
        }
        let sonnet =
            corrected_pricing("anthropic", "claude-sonnet-5").expect("sonnet 5 correction");
        assert_eq!(
            (
                sonnet.input,
                sonnet.output,
                sonnet.cache_read,
                sonnet.cache_write
            ),
            (Some(2.0), Some(10.0), Some(0.20), Some(2.50)),
        );
    }

    /// The end of the road for both tables: what the pricing pipeline hands
    /// back for a model that used to fall through to the worst case.
    ///
    /// Through `maybe_get_pricing`, deliberately — that is the entry point the
    /// ledger uses, and it is the ONLY one that reaches a model with no
    /// registry row at all (`maybe_get_canonical_model` needs a row to exist
    /// before it can backfill a missing price into it, so Fable never gets
    /// that far).
    #[test]
    fn fable_and_sonnet_price_through_the_public_entry_point() {
        let fable = super::super::maybe_get_pricing("anthropic", "claude-fable-5")
            .expect("fable 5 must price, not fall to the worst case");
        assert_eq!((fable.input, fable.output), (Some(10.0), Some(50.0)));
        let sonnet = super::super::maybe_get_pricing("anthropic", "claude-sonnet-5")
            .expect("sonnet 5 must price");
        assert_eq!(
            (sonnet.input, sonnet.output),
            (Some(2.0), Some(10.0)),
            "the correction must win over the generated row",
        );
    }

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
