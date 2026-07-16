//! Single-source-of-truth cost computation for one provider response.
//!
//! [`cost_of`] is the ONE function that turns a [`Usage`] + [`Pricing`] into a
//! USD figure, folding ALL FOUR billable components — fresh input, output,
//! cache read, cache write. It exists because cost used to be computed in
//! several places that disagreed: the CLI estimate folded only input+output and
//! silently ignored cache, while the verification digest applied a single
//! blended `$/1k-token` rate to the grand token total. Both now route through
//! here so there is exactly one formula.
//!
//! ## The cache-inclusive gotcha (why this is not a naive four-term sum)
//! Both provider parsers store [`Usage::input_tokens`] as the FULL prompt
//! surface — it ALREADY INCLUDES the cache-read tokens, and for Anthropic the
//! cache-write (creation) tokens too (see `formats/anthropic.rs::get_usage`,
//! which sets `input = input + cache_creation + cache_read`, and
//! `formats/openai.rs::get_usage`, where `input = prompt_tokens` and the cached
//! tokens are a subset of it). `cache_read_input_tokens` /
//! `cache_write_input_tokens` are therefore a *breakdown* of that same input,
//! not tokens billed on top of it. Summing `input*in_rate +
//! cache_read*cr_rate + cache_write*cw_rate` would bill the cached tokens twice.
//!
//! So `cost_of` carves a cache category out of the input bucket ONLY when that
//! category has its own rate; tokens whose category rate is absent stay in the
//! input bucket and are billed at the plain input rate. A missing `cache_write`
//! rate (OpenAI / Google fold writes into input) means "no write premium" — it
//! must NEVER be read as "these tokens are free". This keeps the *total* correct
//! for every provider while still yielding a faithful per-component breakdown.

use super::model::Pricing;
use crate::providers::base::Usage;

/// Per-component USD breakdown of a single provider response. `total_cost` is
/// the authoritative number; the components exist so the per-call ledger can
/// attribute spend (input vs output vs cache) without recomputing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CostBreakdown {
    pub input_cost: f64,
    pub output_cost: f64,
    pub cache_read_cost: f64,
    pub cache_write_cost: f64,
    pub total_cost: f64,
}

/// `(tokens / 1e6) * rate` — the per-category formula, applied uniformly.
#[inline]
fn rate_cost(tokens: i64, rate_per_million: f64) -> f64 {
    (tokens as f64 / 1_000_000.0) * rate_per_million
}

#[inline]
fn non_negative(t: Option<i32>) -> i64 {
    t.unwrap_or(0).max(0) as i64
}

/// Full per-component breakdown, or `None` when the model has no input **or**
/// output price. `None` means "cost is unknowable for this model" — callers must
/// treat it as unknown/estimated, never silently as zero.
pub fn cost_breakdown(usage: &Usage, pricing: &Pricing) -> Option<CostBreakdown> {
    let input_rate = pricing.input?;
    let output_rate = pricing.output?;

    let input_tokens = non_negative(usage.input_tokens);
    let output_tokens = non_negative(usage.output_tokens);
    let cache_read = non_negative(usage.cache_read_input_tokens);
    let cache_write = non_negative(usage.cache_write_input_tokens);

    // Start from the full (cache-inclusive) input surface and carve out a cache
    // category ONLY if it is separately rated. See the module gotcha note.
    let mut fresh_input = input_tokens;

    let cache_read_cost = match pricing.cache_read {
        Some(rate) => {
            fresh_input -= cache_read;
            rate_cost(cache_read, rate)
        }
        // Folded into input: leave the tokens in the input bucket, add no line.
        None => 0.0,
    };

    let cache_write_cost = match pricing.cache_write {
        Some(rate) => {
            fresh_input -= cache_write;
            rate_cost(cache_write, rate)
        }
        // No write premium exists (never assume one): billed at the input rate.
        None => 0.0,
    };

    // Guard against a malformed row where the cache subsets exceed the reported
    // input — cost must never go negative.
    let fresh_input = fresh_input.max(0);

    let input_cost = rate_cost(fresh_input, input_rate);
    let output_cost = rate_cost(output_tokens, output_rate);
    let total_cost = input_cost + output_cost + cache_read_cost + cache_write_cost;

    Some(CostBreakdown {
        input_cost,
        output_cost,
        cache_read_cost,
        cache_write_cost,
        total_cost,
    })
}

/// THE canonical cost function: total USD for one provider response, folding
/// fresh input + output + cache read + cache write. `None` when the model has no
/// input/output price. Every cost figure in the system should trace back here.
pub fn cost_of(usage: &Usage, pricing: &Pricing) -> Option<f64> {
    cost_breakdown(usage, pricing).map(|b| b.total_cost)
}

/// Dollars saved by cache *reads* versus paying the full input rate for those
/// same tokens — the visible "cache saved $X" trust signal. Only reads yield a
/// saving (cache reads are cheaper than fresh input); cache writes are a premium,
/// not a saving, so they never contribute here. Returns `0.0` when there is no
/// input rate or no distinct (cheaper) cache-read rate.
pub fn cache_savings_of(usage: &Usage, pricing: &Pricing) -> f64 {
    let (Some(input_rate), Some(cache_read_rate)) = (pricing.input, pricing.cache_read) else {
        return 0.0;
    };
    let cache_read = non_negative(usage.cache_read_input_tokens);
    let saving_per_million = (input_rate - cache_read_rate).max(0.0);
    rate_cost(cache_read, saving_per_million)
}

/// Prompt-cache **hit rate** for one response: the fraction of *cacheable* input
/// tokens that were served from cache (reads) rather than freshly written
/// (creation) — `reads / (reads + creation)`. It measures whether the warm
/// prefix is actually being reused: 1.0 = every cacheable token was a cheap read,
/// 0.0 = every cacheable token was a fresh write. Fresh (never-cacheable) input
/// is deliberately excluded so the figure reflects cache effectiveness, not
/// prompt shape.
///
/// Returns `None` when nothing went through the cache this call (no reads and no
/// writes) — the rate is undefined, not zero. Computable only because [`Usage`]
/// now carries the read/creation split (populated by
/// `formats/anthropic.rs::get_usage`); this is the day-one measurable surface for
/// the prompt-cache discipline (#717/#727).
pub fn cache_hit_rate_of(usage: &Usage) -> Option<f64> {
    let reads = non_negative(usage.cache_read_input_tokens);
    let writes = non_negative(usage.cache_write_input_tokens);
    let cacheable = reads + writes;
    if cacheable == 0 {
        return None;
    }
    Some(reads as f64 / cacheable as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Representative real rates (USD per 1M tokens).
    fn anthropic_pricing() -> Pricing {
        // Claude Sonnet-class: input 3, output 15, cache read 0.30 (0.1x),
        // cache write 3.75 (1.25x).
        Pricing {
            input: Some(3.0),
            output: Some(15.0),
            cache_read: Some(0.30),
            cache_write: Some(3.75),
        }
    }

    fn openai_pricing() -> Pricing {
        // GPT-class: input 2.50, output 10, cache read 1.25 (0.5x), and NO
        // separate cache-write rate (writes fold into input).
        Pricing {
            input: Some(2.50),
            output: Some(10.0),
            cache_read: Some(1.25),
            cache_write: None,
        }
    }

    fn usage(input: i32, output: i32, cache_read: i32, cache_write: i32) -> Usage {
        Usage::new(Some(input), Some(output), None)
            .with_cache_tokens(Some(cache_read), Some(cache_write))
    }

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-12, "expected {b}, got {a}");
    }

    #[test]
    fn folds_all_four_components() {
        // input_tokens is cache-INCLUSIVE: 1000 total = 500 fresh + 200 read + 300 write.
        let u = usage(1000, 500, 200, 300);
        let b = cost_breakdown(&u, &anthropic_pricing()).unwrap();
        approx(b.input_cost, 500.0 / 1e6 * 3.0); // fresh input only
        approx(b.output_cost, 500.0 / 1e6 * 15.0);
        approx(b.cache_read_cost, 200.0 / 1e6 * 0.30);
        approx(b.cache_write_cost, 300.0 / 1e6 * 3.75);
        approx(
            b.total_cost,
            b.input_cost + b.output_cost + b.cache_read_cost + b.cache_write_cost,
        );
        // Sanity: the fresh-input carve-out means we do NOT bill 1000 at input rate.
        assert!(b.input_cost < 1000.0 / 1e6 * 3.0);
    }

    #[test]
    fn cache_write_none_is_no_premium_not_free() {
        // OpenAI-style: no cache_write rate. The 100 write tokens must NOT be
        // billed at a premium (component cost 0) but must NOT vanish either —
        // they stay in the input bucket at the input rate.
        let u = usage(1000, 500, 200, 100);
        let b = cost_breakdown(&u, &openai_pricing()).unwrap();
        approx(b.cache_write_cost, 0.0); // no premium
                                         // fresh_input = 1000 - 200 (read carved) = 800; write NOT carved.
        approx(b.input_cost, 800.0 / 1e6 * 2.50);
        approx(b.cache_read_cost, 200.0 / 1e6 * 1.25);
        approx(b.output_cost, 500.0 / 1e6 * 10.0);
        // Total still accounts for the write tokens (via the input bucket).
        approx(b.total_cost, 0.002 + 0.005 + 0.00025);
    }

    #[test]
    fn cache_write_some_costs_more_than_none() {
        // Same tokens, a write rate present vs absent: present must cost strictly
        // more (the write premium is real when Anthropic charges it).
        let u = usage(1000, 500, 0, 300);
        let mut with_rate = openai_pricing();
        with_rate.cache_write = Some(5.0); // premium over the 2.50 input rate
        let without = cost_of(&u, &openai_pricing()).unwrap();
        let with = cost_of(&u, &with_rate).unwrap();
        assert!(
            with > without,
            "write rate should add cost: {with} vs {without}"
        );
    }

    #[test]
    fn zero_tokens_is_zero_not_none() {
        let u = usage(0, 0, 0, 0);
        approx(cost_of(&u, &anthropic_pricing()).unwrap(), 0.0);
    }

    #[test]
    fn missing_price_is_none() {
        let u = usage(1000, 500, 0, 0);
        let no_input = Pricing {
            input: None,
            output: Some(15.0),
            cache_read: None,
            cache_write: None,
        };
        let no_output = Pricing {
            input: Some(3.0),
            output: None,
            cache_read: None,
            cache_write: None,
        };
        assert_eq!(cost_of(&u, &no_input), None);
        assert_eq!(cost_of(&u, &no_output), None);
    }

    #[test]
    fn anthropic_vs_openai_rows_differ_correctly() {
        // Identical token shape, different provider pricing → different totals,
        // each computed by its own rate card.
        let u = usage(2000, 1000, 500, 400);
        let a = cost_of(&u, &anthropic_pricing()).unwrap();

        // Anthropic: fresh = 2000 - 500 - 400 = 1100.
        let a_expected = 1100.0 / 1e6 * 3.0     // input
            + 1000.0 / 1e6 * 15.0               // output
            + 500.0 / 1e6 * 0.30                // cache read
            + 400.0 / 1e6 * 3.75; // cache write
        approx(a, a_expected);

        let o = cost_of(&u, &openai_pricing()).unwrap();
        // OpenAI: no write rate → fresh = 2000 - 500 = 1500, write stays in input.
        let o_expected = 1500.0 / 1e6 * 2.50    // input (incl. the 400 write tokens)
            + 1000.0 / 1e6 * 10.0               // output
            + 500.0 / 1e6 * 1.25; // cache read
        approx(o, o_expected);

        assert!(a != o);
    }

    #[test]
    fn cache_savings_reflects_read_discount() {
        // 500 cache-read tokens at Anthropic rates: saving = 500 * (3.0 - 0.30)/1e6.
        let u = usage(1000, 0, 500, 0);
        approx(
            cache_savings_of(&u, &anthropic_pricing()),
            500.0 / 1e6 * (3.0 - 0.30),
        );
        // No cache-read rate → no computable saving.
        let no_cr = Pricing {
            input: Some(3.0),
            output: Some(15.0),
            cache_read: None,
            cache_write: None,
        };
        approx(cache_savings_of(&u, &no_cr), 0.0);
    }

    #[test]
    fn cache_hit_rate_is_reads_over_reads_plus_writes() {
        // Warm prefix fully reused: all cacheable tokens were reads → 1.0.
        approx(cache_hit_rate_of(&usage(1000, 0, 500, 0)).unwrap(), 1.0);
        // Cold prefix: all cacheable tokens were fresh writes → 0.0.
        approx(cache_hit_rate_of(&usage(1000, 0, 0, 500)).unwrap(), 0.0);
        // Mixed (the realistic long-session case): 5000 read of 15000 cacheable.
        approx(
            cache_hit_rate_of(&usage(20000, 0, 5000, 10000)).unwrap(),
            5000.0 / 15000.0,
        );
        // Fresh input alone is excluded from the denominator — the rate reflects
        // cache effectiveness, not prompt shape.
        approx(cache_hit_rate_of(&usage(9999, 0, 3, 1)).unwrap(), 0.75);
    }

    #[test]
    fn cache_hit_rate_is_none_without_cache_activity() {
        // Nothing went through the cache this call → undefined, not zero.
        assert_eq!(cache_hit_rate_of(&usage(1000, 50, 0, 0)), None);
        // A Usage with no cache fields populated at all → None (never a divide-by-zero).
        assert_eq!(
            cache_hit_rate_of(&Usage::new(Some(1000), Some(50), None)),
            None
        );
    }
}
