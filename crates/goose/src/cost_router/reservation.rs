//! Conservative, pure bounds for authorizing provider invocations.
//!
//! The reservation is a worst-case *authorization* amount, not a prediction of
//! what a model will consume.  A paid call may start only when this bound is
//! finite and positive.  Local and subscription calls return `Ok(None)` since
//! they do not create a paid hold.
//!
//! The planner intentionally does not infer a proportional token bound from
//! raw request bytes. Providers may apply server-side chat templates and add
//! BOS/EOS/control/image tokens that are absent from those bytes. When the
//! caller has counted the prompt it is about to send
//! ([`plan_reservation_bound_for_prompt`]), the input side is bounded by that
//! count plus [`prompt_token_margin`], which is sized to cover that overhead
//! and a provider tokenizer that counts the same text higher than the local
//! one. When the prompt size is unknown this module still reserves the full
//! accepted context capacity: an uncounted call fails closed onto the worst
//! case, never onto a guess.

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

/// Y7 (harness loop, from Lane A12's Q6 measurement): how many *physical*
/// attempts one logical provider call reserves money for up front.
///
/// A reservation is a hold, not a prediction, so the historical choice was to
/// pre-reserve the provider's entire retry envelope
/// ([`RetryConfig::max_physical_attempts`](crate::providers::retry::RetryConfig::max_physical_attempts),
/// 5 under every default config). A12 measured what that costs in practice on
/// the M12 slice: every `zai`/`glm-4.7` hold was `$1.662976` while the largest
/// *settled* single call in the whole slice was `$0.0234` and the typical one
/// `$0.005` — an over-reservation of ~70–330×. With a `$5.00` session ceiling
/// two live holds ($3.325952) leave no room for a third, so the run stalls on
/// hold arithmetic long before it approaches its actual `$2.50` spend cap
/// (every gate message in that slice: "holds $3.33, requested bound $1.66,
/// ceiling $5.00"). The ceiling was never the binding constraint; this
/// multiplier was.
///
/// Default 2 (orchestrator ruling, 2026-09-10): reserve what one attempt can
/// actually cost, plus one real retry. The retry envelope is then enforced
/// against the reservation at the seam where a physical retry would happen
/// (`cost_router::authorized_attempts`), so a retry that the hold does not
/// cover is refused with an honest error rather than silently spending past
/// the authorization.
///
/// Two rather than one because a default of one would mean a paid call that
/// hits a 429 or a 5xx never retries at all — the exact failure
/// `RATE_LIMIT_MIN_RETRY_INTERVAL_MS` / `RATE_LIMIT_MAX_RETRIES` were added
/// for on 2026-08-25 (Z.AI 1302). Two holds `$0.665` on the measured
/// `zai`/`glm-4.7` inputs: still 7 concurrent holds under a `$5.00` ceiling
/// instead of the pre-Y7 2, so the spend cap goes back to being the binding
/// constraint, and one rate-limit retry still survives.
///
/// Still two after 2026-09-18, deliberately. A one-attempt default was
/// proposed because the retry loop can extend the SAME hold when it retries
/// (`providers::retry` -> `authorize_attempt` -> `extend_reservation`), and a
/// one-attempt hold made the harness's first call fit under the old $1.00
/// task gate. It was not adopted: calls granted through
/// `grant_reservation_after_gate_approval` carry no extension evidence, so
/// under a one-attempt default they could never retry a 429 or 5xx — a
/// reliability regression on exactly the calls the user has just approved.
/// It also reversed the ruling above without the user's sign-off. It is no
/// longer needed either: the input side is now sized to the counted prompt
/// (`plan_reservation_bound_for_prompt`), which brings gpt-5.4-mini's
/// two-attempt bound to ~$1.23, comfortably under the current $3.00 task gate.
pub const RESERVE_ATTEMPTS_ENV: &str = "PERMAGENT_RESERVE_ATTEMPTS";

/// Physical attempts reserved when [`RESERVE_ATTEMPTS_ENV`] says nothing:
/// one attempt plus one retry (see [`RESERVE_ATTEMPTS_ENV`] for why two).
pub const DEFAULT_RESERVED_ATTEMPTS: u32 = 2;

/// How many physical attempts to reserve, given the retry policy's own
/// envelope.
///
/// Clamped into `1..=max_physical_attempts`: the knob can never authorize more
/// attempts than the retry policy would ever make (that would over-reserve
/// again), and never fewer than one (a zero-attempt reservation cannot
/// authorize the call at all). `max_physical_attempts == 0` is passed straight
/// through so [`plan_reservation_bound`] still fails closed with
/// [`ReservationBoundError::InvalidAttempts`] instead of this function
/// inventing an attempt the policy does not allow.
pub fn reserved_physical_attempts(max_physical_attempts: u32) -> u32 {
    reserved_physical_attempts_with(max_physical_attempts, |key| std::env::var(key).ok())
}

/// Testable core of [`reserved_physical_attempts`]; `read` stands in for the
/// process environment.
pub fn reserved_physical_attempts_with(
    max_physical_attempts: u32,
    read: impl Fn(&str) -> Option<String>,
) -> u32 {
    if max_physical_attempts == 0 {
        return 0;
    }
    read(RESERVE_ATTEMPTS_ENV)
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_RESERVED_ATTEMPTS)
        .clamp(1, max_physical_attempts)
}

/// A finite upper bound for one physical provider invocation (including its
/// allowed retries).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReservationBound {
    /// Maximum dollars that may be committed by this invocation family.
    pub amount_usd: f64,
    /// Input-token upper bound reserved for the request.
    pub input_tokens: usize,
    /// Maximum output tokens per physical attempt: [`output_reservation_cap`]
    /// ([`RESERVED_OUTPUT_TOKENS`], or [`REASONING_RESERVED_OUTPUT_TOKENS`] for
    /// a reasoning model), clamped down to the model's own `max_tokens` when
    /// that is smaller.
    /// This is also the number the caller must send the provider as its
    /// request-level output cap, or the reservation prices a number the call
    /// was never actually limited to.
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

/// Whether this exact `provider`/`model` pair runs on local hardware at no
/// marginal cost, according to the one curated, cited source of that fact in
/// the runtime: [`crate::cost_router::knowledge`]'s `is_local` rows.
///
/// A provider object states its own billing class through
/// [`Provider::cost_tier`](crate::providers::base::Provider::cost_tier), and
/// that default is deliberately fail-closed
/// ([`CostTier::PaidApi`], `providers/base.rs`). A *declarative* provider
/// (`providers/declarative/*.json`) is built on a shared engine — the local
/// split `qwen38_split` is served over an OpenAI-compatible socket and is
/// therefore an `OpenAiProvider`, which never overrides `cost_tier` — so a
/// local runtime inherits the paid default and the spend gate then demands a
/// dollar bound for a call that costs nothing.
///
/// 2026-09-18, `permagent run --recipe permagent-coding`: OpenAI's stream
/// dropped pre-commit, `agents::agent`'s failover switched the session to
/// `qwen38_split`/`qwen3.8-27b`, and the very next turn died with "provider
/// budget is unknown; refusing call: cannot authorize provider call: maximum
/// output tokens are required for a paid reservation" — a *local* model
/// refused for want of a *paid* reservation. That call could never have been
/// authorized on the paid path at all: the fallback's `ModelConfig` carries no
/// `max_tokens` (a declarative local model has no canonical row for
/// `ModelConfig::with_canonical_limits` to read one from), and even with one,
/// this pair's published rate is $0.00/$0.00, which `positive_rate` rejects as
/// an invalid rate. Every branch of the paid path fails closed on a free
/// model, which is exactly why the free model must not enter it.
///
/// Deliberately pair-scoped and exact, NOT a provider-name rule: the sibling
/// helper `cost_router::is_local_provider` matches `"ollama"` as a substring,
/// which also matches the metered `ollama_cloud` declarative provider. Reusing
/// that here would hand a real paid endpoint a free pass; a curated row states
/// a price for one named pair and cannot.
pub fn is_free_local_pair(provider: &str, model: &str) -> bool {
    crate::cost_router::knowledge::lookup(provider, model).is_some_and(|row| {
        row.is_local && row.input_usd_per_mtok == 0.0 && row.output_usd_per_mtok == 0.0
    })
}

/// The billing class a call is actually authorized and settled under: the tier
/// the provider object declared, downgraded to [`CostTier::LocalFree`] when
/// [`is_free_local_pair`] knows the pair costs nothing.
///
/// Only ever downgrades. A pair the knowledge base does not vouch for keeps
/// whatever the provider declared, so an unknown or genuinely metered endpoint
/// still reserves money and is still refused when the budget is unknown.
///
/// Callers must use the SAME answer for authorization and for settlement. The
/// reservation seam (`agents::reply_parts::reserve_provider_invocation_for_model`)
/// records this tier on the invocation context, because settlement refuses a
/// chargeable invocation that carries no reservation id ("paid provider
/// invocation has no reservation; refusing to settle silently") — so a call
/// authorized as free but remembered as paid would simply fail one step later.
pub fn effective_cost_tier(provider: &str, model: &str, declared: CostTier) -> CostTier {
    if declared.is_chargeable() && is_free_local_pair(provider, model) {
        CostTier::LocalFree
    } else {
        declared
    }
}

/// Resolve pricing and calculate a conservative reservation bound.
///
/// `ModelConfig::context_limit` and `ModelConfig::max_tokens` are deliberately
/// read directly.  The convenience defaults (`context_limit()` and
/// `max_output_tokens()`) are estimates and therefore cannot authorize a paid
/// call.  Input capacity is the entire accepted context minus the maximum
/// output (this entry point does not know the prompt size; see
/// [`plan_reservation_bound_for_prompt`] for a caller that does), and the
/// input side uses the most expensive of fresh, cache-read, and cache-write
/// rates.  The resulting one-attempt amount is multiplied by the
/// maximum number of physical attempts.
pub fn plan_reservation_bound(
    provider: &str,
    model: &str,
    cost_tier: CostTier,
    model_config: &ModelConfig,
    max_physical_attempts: u32,
) -> Result<Option<ReservationBound>, ReservationBoundError> {
    plan_reservation_bound_for_prompt(
        provider,
        model,
        cost_tier,
        model_config,
        max_physical_attempts,
        None,
    )
}

/// Fixed allowance, in tokens, added to every counted prompt before it bounds
/// a reservation. It covers what a local count of the serialized request
/// cannot see: the provider's chat template (role/turn/BOS/EOS markers per
/// message), the wrapper text a provider renders tool schemas into, and any
/// short instructions an adapter prepends on its own.
pub const PROMPT_OVERHEAD_TOKENS: usize = 4_096;

/// The output side of a reservation bound, in tokens — an ENFORCED request
/// cap, not a prediction of what the model will produce.
///
/// Measured on the live cost ledger: 63 reservations held $81.35 in total and
/// settled at $1.23 — every call held roughly $1.50 and settled around $0.02,
/// 1.1-1.6% of its hold. That gap is `model_config.max_tokens` doing the
/// pricing: a call was reserved against the model's theoretical maximum
/// output (e.g. 131,072 for `zai`/`glm-4.7`) when the actual reply was tiny.
/// Real outputs across 1,329 recorded calls: median 163 tokens, p95 1,858,
/// p99 5,779, max 14,618. The task budget gate then refused real work
/// ("spent $0.95, holds $1.24, bound $1.56, ceiling $3.00") while actual
/// spend was pennies — the user's "the harness fails on simple tasks"
/// complaint.
///
/// 4,096 is a deliberately fixed authorization ceiling: comfortably above the
/// p95 (1,858) and the overwhelming majority of real replies, while staying
/// far below the multi-hundred-thousand-token `max_tokens` values that drove
/// the over-reservation (p99 at 5,779 and the 14,618 max do exceed it — this
/// is a chosen cap, not a percentile fit, and a reply that would have run
/// longer is stopped at the cap rather than left unbounded).
///
/// A reservation is only honest if the number priced here is the number the
/// provider is actually told it may spend: this same value is threaded
/// through to become the request's own `max_output_tokens`
/// (`agents::reply_parts::stream_response_from_provider`,
/// `ProviderInvocationContext::reserved_output_tokens`), so the model is
/// asked to stop at the cap instead of being merely priced as if it would.
pub const RESERVED_OUTPUT_TOKENS: usize = 4_096;

/// [`RESERVED_OUTPUT_TOKENS`] for a reasoning model
/// (`ModelConfig::is_reasoning_model`).
///
/// A reasoning model's hidden reasoning is billed and capped as output, so
/// the 4,096 cap above is not an answer budget for it: on 2026-09-25 two
/// gpt-5.4-mini goal workers each spent the whole 4,096 reasoning, got back
/// an incomplete response holding only a reasoning item, and ended their run
/// with no tool call. The measured reply sizes behind the 4,096 cap were
/// visible output only. OpenAI's reasoning guide asks for at least 25,000
/// tokens of room for reasoning plus output; this matches the unreserved
/// default (`model::REASONING_DEFAULT_MAX_OUTPUT_TOKENS`). The hold is still
/// released at settlement, so a larger cap costs headroom, not money.
pub const REASONING_RESERVED_OUTPUT_TOKENS: usize =
    crate::model::REASONING_DEFAULT_MAX_OUTPUT_TOKENS as usize;

/// The enforced output cap a reservation prices for `model_config`, before
/// it is clamped to the model's own `max_tokens`.
pub fn output_reservation_cap(model_config: &ModelConfig) -> usize {
    if model_config.is_reasoning_model() {
        REASONING_RESERVED_OUTPUT_TOKENS
    } else {
        RESERVED_OUTPUT_TOKENS
    }
}

/// The safety margin a counted prompt is padded by: half again the count, plus
/// [`PROMPT_OVERHEAD_TOKENS`].
///
/// The count comes from the local o200k tokenizer
/// (`agents::reply_parts::counted_prompt_tokens`), which is not the tokenizer
/// every provider bills with; another vendor's vocabulary can split the same
/// text into materially more tokens, most of all for code. Half again is
/// deliberately generous: this bound is an authorization hold that is
/// released at settlement, so padding it costs nothing but headroom, while an
/// under-stated hold would let a call spend money nobody authorized.
pub fn prompt_token_margin(counted_prompt_tokens: usize) -> usize {
    (counted_prompt_tokens / 2).saturating_add(PROMPT_OVERHEAD_TOKENS)
}

/// [`plan_reservation_bound`], bounded by the size of the prompt actually
/// being sent when the caller knows it.
///
/// `counted_prompt_tokens` is the caller's count of everything it is about to
/// send (system prompt, messages and tool schemas). `Some(n)` reserves
/// `n + prompt_token_margin(n)` input tokens, never more than the context can
/// accept; `None` reserves the whole window exactly as before, so a caller that
/// cannot count its request can never obtain a smaller hold than the worst
/// case.
///
/// Why this exists: the harness refused its very first call with "task budget
/// gate required before provider call (spent $0.12, authorization holds
/// $0.00, requested bound $1.56, ceiling $1.00)". That call sent ~30k tokens,
/// 7% of a 400k window, and cost $0.12 — but the bound priced all
/// `context - output` input tokens regardless, so every call reserved the
/// worst case and any sane budget cap deadlocked (the caps in
/// `cost_router::budget` had to be raised to $1/$3/$6 because of it).
pub fn plan_reservation_bound_for_prompt(
    provider: &str,
    model: &str,
    cost_tier: CostTier,
    model_config: &ModelConfig,
    max_physical_attempts: u32,
    counted_prompt_tokens: Option<usize>,
) -> Result<Option<ReservationBound>, ReservationBoundError> {
    validate_attempts(max_physical_attempts)?;

    // A free local pair holds no dollars, so it never needs a priced bound —
    // see [`is_free_local_pair`] for the failure this guard is the fix for.
    // The declared tier is still honoured on its own: this only ever turns a
    // chargeable claim into a free one, never the reverse.
    if !effective_cost_tier(provider, model, cost_tier).is_chargeable() {
        return Ok(None);
    }

    let (pricing, provenance) = resolve_pricing(provider, model)?;

    calculate_bound_for_prompt(
        model_config,
        max_physical_attempts,
        pricing,
        provenance,
        counted_prompt_tokens,
    )
    .map(Some)
}

pub(crate) fn resolve_pricing(
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

#[cfg(test)]
fn calculate_bound(
    model_config: &ModelConfig,
    max_physical_attempts: u32,
    pricing: Pricing,
    pricing_provenance: PricingProvenance,
) -> Result<ReservationBound, ReservationBoundError> {
    calculate_bound_for_prompt(
        model_config,
        max_physical_attempts,
        pricing,
        pricing_provenance,
        None,
    )
}

fn calculate_bound_for_prompt(
    model_config: &ModelConfig,
    max_physical_attempts: u32,
    pricing: Pricing,
    pricing_provenance: PricingProvenance,
    counted_prompt_tokens: Option<usize>,
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
    let model_max_output =
        usize::try_from(output_i32).map_err(|_| ReservationBoundError::InvalidMaxOutput)?;
    if model_max_output == 0 {
        return Err(ReservationBoundError::InvalidMaxOutput);
    }
    if model_max_output > context {
        return Err(ReservationBoundError::OutputExceedsContext {
            output: model_max_output,
            context,
        });
    }
    // The reservation prices [`RESERVED_OUTPUT_TOKENS`] — an enforced request
    // cap — rather than the model's own theoretical maximum, clamped down
    // further when that maximum is itself smaller (a model that can never
    // produce more than its own `max_tokens` needs no larger a hold than
    // that). See `RESERVED_OUTPUT_TOKENS` for the measured evidence this
    // bound is based on.
    // A reasoning model gets [`REASONING_RESERVED_OUTPUT_TOKENS`] instead:
    // its reasoning counts against the same cap.
    let output = model_max_output.min(output_reservation_cap(model_config));
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

    // The context can never accept more input than `context - output` next
    // to a full-length reply, so that stays the ceiling either way. A counted
    // prompt narrows it to what is actually being sent plus the margin; an
    // uncounted one keeps the whole window (fail closed). A count larger than
    // the window is clamped rather than refused: the provider will reject an
    // over-long prompt, and it cannot bill more input than the window holds.
    let window_input = context - output;
    let input_tokens = match counted_prompt_tokens {
        Some(counted) => counted
            .saturating_add(prompt_token_margin(counted))
            .min(window_input),
        None => window_input,
    };
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
        // The reservation reserves RESERVED_OUTPUT_TOKENS (4,096), not this
        // model's 100,000-token max_tokens — see that constant's doc comment
        // for the measured evidence. The input side still gets the rest of
        // the context capacity next to that smaller reserved output.
        assert_eq!(bound.input_tokens, 1_000_000 - RESERVED_OUTPUT_TOKENS);
        assert_eq!(bound.output_tokens, RESERVED_OUTPUT_TOKENS);
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
        // context == max_tokens so the shape matches that observation.
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
        // Sanity, post-RESERVED_OUTPUT_TOKENS: output is capped at 4,096 (far
        // below this model's raw 131,072 max_tokens), so the window opens up
        // to 126,976 input tokens next to it at the real $0.60/$2.20-per-Mtok
        // rates — no longer the output-only, zero-input shape this test
        // pinned before the cap existed.
        let expected = (126_976.0 / 1_000_000.0 * 0.60) + (4_096.0 / 1_000_000.0 * 2.20);
        assert_eq!(bound.output_tokens, RESERVED_OUTPUT_TOKENS);
        assert_eq!(bound.input_tokens, 126_976);
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
        // Post-RESERVED_OUTPUT_TOKENS shape: see the sibling `"zai"` test
        // above for why this is no longer output-only.
        let expected = (126_976.0 / 1_000_000.0 * 0.60) + (4_096.0 / 1_000_000.0 * 2.20);
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
    /// F8/D1/I1/M11 observed `amount_usd = 1.6629760000000002` bit-for-bit on
    /// this exact live shape (verified directly against the M11
    /// `cost_reservations` rows, not merely "close"). The provenance is
    /// `CanonicalExact`, never `WorstCase` — three straight lanes'
    /// "flat worst-case constant" diagnosis was wrong; the number was flat
    /// only because `context_limit`/`max_tokens`/`max_physical_attempts`
    /// never varied call to call for this pair, not because it was a
    /// mispriced fallback.
    ///
    /// The $1.66 figure itself is now historical, not reproduced here: this
    /// is exactly the over-reservation `RESERVED_OUTPUT_TOKENS` exists to
    /// fix (measured on the live ledger — 63 reservations of roughly this
    /// shape held $81.35 and settled at $1.23; see that constant's doc
    /// comment). Post-fix, the bound reserves 4,096 output tokens instead of
    /// this model's raw 131,072 `max_tokens`, so the number pinned below is
    /// the NEW correctly-capped bound, not the old one.
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
        assert_eq!(bound.output_tokens, RESERVED_OUTPUT_TOKENS);
        assert_eq!(bound.input_tokens, 204_800 - RESERVED_OUTPUT_TOKENS);
        // Bit-exact regression pin for the NEW, output-capped bound: 200,704
        // input tokens at $0.60/Mtok plus 4,096 output tokens at $2.20/Mtok,
        // times the 5-attempt envelope.
        assert_eq!(bound.amount_usd, 0.647168);
        assert!(
            bound.amount_usd < 1.6629760000000002,
            "the new output-capped bound (${:.6}) must land well under the old \
             max_tokens-based bound ($1.662976)",
            bound.amount_usd
        );
        assert_eq!(
            describe_pricing_source(bound.pricing_provenance),
            "canonical_exact"
        );
    }

    /// Y7 pin (harness loop, from Lane A12's Q6 measurement): the hold scales
    /// linearly with the reserved attempt count, for the identical live inputs
    /// pinned by the test above. One attempt holds exactly one fifth of the
    /// output-capped five-attempt bound, and the SHIPPED default of two holds
    /// exactly two fifths of it. Nothing about pricing changes here: the only
    /// difference from the test above is how many attempts' worth of money is
    /// held up front — see `RESERVED_OUTPUT_TOKENS` for why the underlying
    /// per-attempt number itself is now far smaller than the historical
    /// $0.332595 this test used to pin (that figure priced 131,072 raw output
    /// tokens; this reserves 4,096).
    #[test]
    fn the_reserved_attempt_count_scales_the_hold_off_the_five_attempt_bound() {
        let cfg = config(Some(204_800), Some(131_072));
        let five = plan_reservation_bound("zai", "glm-4.7", CostTier::PaidApi, &cfg, 5)
            .unwrap()
            .unwrap();
        let one = plan_reservation_bound("zai", "glm-4.7", CostTier::PaidApi, &cfg, 1)
            .unwrap()
            .unwrap();
        assert_eq!(one.max_physical_attempts, 1);
        assert_eq!(one.amount_usd, 0.12943359999999998);
        assert_eq!(one.amount_usd * 5.0, five.amount_usd);
        // Everything else about the bound is untouched.
        assert_eq!(one.input_tokens, five.input_tokens);
        assert_eq!(one.output_tokens, five.output_tokens);
        assert_eq!(one.pricing_provenance, five.pricing_provenance);

        // The shipped default: two attempts, two fifths of the old hold (see
        // `DEFAULT_RESERVED_ATTEMPTS` for why not one).
        let shipped = plan_reservation_bound(
            "zai",
            "glm-4.7",
            CostTier::PaidApi,
            &cfg,
            DEFAULT_RESERVED_ATTEMPTS,
        )
        .unwrap()
        .unwrap();
        assert_eq!(shipped.max_physical_attempts, 2);
        assert!((shipped.amount_usd - 2.0 * one.amount_usd).abs() < 1e-9);
        assert!((shipped.amount_usd * 2.5 - five.amount_usd).abs() < 1e-9);
    }

    /// Y7: the knob can raise or lower the hold but never authorize more
    /// attempts than the retry policy would ever physically make, and never
    /// zero. The shipped default is two (see `DEFAULT_RESERVED_ATTEMPTS`).
    #[test]
    fn reserved_attempts_default_to_two_and_clamp_to_the_retry_envelope() {
        let env = |value: Option<&str>| {
            let value = value.map(str::to_string);
            move |key: &str| {
                assert_eq!(key, RESERVE_ATTEMPTS_ENV);
                value.clone()
            }
        };
        assert_eq!(DEFAULT_RESERVED_ATTEMPTS, 2);
        assert_eq!(reserved_physical_attempts_with(5, env(None)), 2);
        assert_eq!(reserved_physical_attempts_with(5, env(Some("1"))), 1);
        assert_eq!(reserved_physical_attempts_with(5, env(Some("2"))), 2);
        assert_eq!(reserved_physical_attempts_with(5, env(Some("3"))), 3);
        assert_eq!(reserved_physical_attempts_with(5, env(Some(" 5 "))), 5);
        // Above the envelope: clamped down, never over-reserved again.
        assert_eq!(reserved_physical_attempts_with(5, env(Some("9"))), 5);
        // Meaningless values fall back to the default rather than to zero.
        assert_eq!(reserved_physical_attempts_with(5, env(Some("0"))), 2);
        assert_eq!(reserved_physical_attempts_with(5, env(Some("many"))), 2);
        assert_eq!(reserved_physical_attempts_with(5, env(Some(""))), 2);
        // A one-attempt envelope still gets its one attempt.
        assert_eq!(reserved_physical_attempts_with(1, env(None)), 1);
        // A zero envelope stays zero so `plan_reservation_bound` still fails
        // closed instead of this function inventing an attempt.
        assert_eq!(reserved_physical_attempts_with(0, env(Some("4"))), 0);
        assert_eq!(
            plan_reservation_bound(
                "zai",
                "glm-4.7",
                CostTier::PaidApi,
                &config(Some(204_800), Some(131_072)),
                reserved_physical_attempts_with(0, env(None)),
            ),
            Err(ReservationBoundError::InvalidAttempts)
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
    fn arbitrary_model_name_on_a_local_free_provider_is_free() {
        let cfg = config(None, None);
        assert_eq!(
            plan_reservation_bound(
                "lmstudio",
                "runtime-discovered-model",
                CostTier::LocalFree,
                &cfg,
                DEFAULT_RESERVED_ATTEMPTS
            )
            .unwrap(),
            None
        );
    }

    /// 2026-09-18 user-reported blocker, `permagent run --recipe
    /// permagent-coding`: OpenAI's stream dropped before any reply started,
    /// the pre-commit failover switched the session to the LOCAL split
    /// `qwen38_split`/`qwen3.8-27b`, and the next call died with "provider
    /// budget is unknown; refusing call: cannot authorize provider call:
    /// maximum output tokens are required for a paid reservation".
    ///
    /// The shape below is the exact shape of that fallback: the declarative
    /// provider is an `OpenAiProvider`, so it reports `Provider::cost_tier`'s
    /// fail-closed `PaidApi` default, and
    /// `Agent::switch_to_permanent_failure_fallback` builds its `ModelConfig`
    /// with `ModelConfig::with_canonical_limits`, which finds no canonical row
    /// for a declarative local model and therefore leaves `max_tokens` unset.
    /// Nothing about a free model can satisfy the paid path — the $0.00/$0.00
    /// published rate would be rejected by `positive_rate` even with a
    /// `max_tokens` present — so it must not enter it.
    #[test]
    fn a_local_split_fallback_is_authorized_without_a_paid_reservation() {
        let cfg = config(Some(4_096), None);
        assert_eq!(
            plan_reservation_bound(
                "qwen38_split",
                "qwen3.8-27b",
                CostTier::PaidApi,
                &cfg,
                DEFAULT_RESERVED_ATTEMPTS
            )
            .unwrap(),
            None,
            "a local model costs nothing and must never need a paid reservation"
        );
        assert_eq!(
            effective_cost_tier("qwen38_split", "qwen3.8-27b", CostTier::PaidApi),
            CostTier::LocalFree,
            "settlement must agree with authorization, or the call fails one step later \
             on \"paid provider invocation has no reservation\""
        );
    }

    /// The safety property the fix must not spend: a genuinely metered call
    /// whose budget cannot be established is still refused before any bytes
    /// leave. `openai`/`gpt-5.4` is priced and not local, so the missing
    /// `max_tokens` still fails closed exactly as it did before.
    #[test]
    fn a_paid_provider_with_an_unknowable_bound_is_still_refused() {
        let cfg = config(Some(400_000), None);
        assert_eq!(
            plan_reservation_bound(
                "openai",
                "gpt-5.4",
                CostTier::PaidApi,
                &cfg,
                DEFAULT_RESERVED_ATTEMPTS
            ),
            Err(ReservationBoundError::MissingMaxOutput)
        );
        assert_eq!(
            effective_cost_tier("openai", "gpt-5.4", CostTier::PaidApi),
            CostTier::PaidApi
        );
    }

    /// The reason this gate reads a curated per-pair row instead of the
    /// sibling `cost_router::is_local_provider` name rule: that rule matches
    /// `"ollama"` as a SUBSTRING, so `ollama_cloud` — a metered hosted API
    /// declared in `providers/declarative/ollama_cloud.json` — would have been
    /// handed a free pass by it. A curated row prices one named pair and
    /// cannot leak across to a lookalike id.
    #[test]
    fn a_cloud_provider_with_a_local_sounding_name_is_not_freed() {
        assert!(crate::cost_router::is_local_provider("ollama_cloud"));
        assert!(
            !is_free_local_pair("ollama_cloud", "qwen3-coder"),
            "a hosted, metered endpoint must not inherit a local row's $0 price"
        );
        let cfg = config(Some(256_000), None);
        assert_eq!(
            plan_reservation_bound("ollama_cloud", "qwen3-coder", CostTier::PaidApi, &cfg, 1),
            Err(ReservationBoundError::MissingMaxOutput)
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

    /// `openai`/`gpt-5.4-mini`'s published rates ($0.75 in, $0.075 cache
    /// read, $4.50 out per Mtok), written out so the arithmetic below does not
    /// move if the generated registry is refreshed.
    fn gpt_5_4_mini_pricing() -> Pricing {
        Pricing {
            input: Some(0.75),
            output: Some(4.5),
            cache_read: Some(0.075),
            cache_write: None,
        }
    }

    /// The user-reported deadlock: "task budget gate required before provider
    /// call (spent $0.12, authorization holds $0.00, requested bound $1.56,
    /// ceiling $1.00)". A 400k-context model with a 128k reply cap, two
    /// reserved attempts (the default then), and no prompt size reproduced
    /// that $1.56 by pricing all 272k of the window's input tokens against
    /// the model's raw 128,000-token `max_tokens`.
    ///
    /// Post-`RESERVED_OUTPUT_TOKENS`, even an UNCOUNTED prompt (this test
    /// passes none) no longer prices that raw `max_tokens` on the output
    /// side: output is capped at 4,096, which both shrinks the output cost
    /// and — since input capacity is `context - output` — opens up the
    /// window's input side to 395,904 tokens instead of 272,000. The bound
    /// drops from the reported $1.56 to about $0.63, well under a $1.00
    /// ceiling that used to be unreachable for this shape.
    #[test]
    fn the_reported_1_56_bound_is_the_full_window_priced_for_an_unknown_prompt() {
        let cfg = config(Some(400_000), Some(128_000));
        let full = plan_reservation_bound("openai", "gpt-5.4-mini", CostTier::PaidApi, &cfg, 2)
            .unwrap()
            .unwrap();
        assert_eq!(full.output_tokens, RESERVED_OUTPUT_TOKENS);
        assert_eq!(full.input_tokens, 400_000 - RESERVED_OUTPUT_TOKENS);
        assert!(
            (full.amount_usd - 0.63072).abs() < 1e-9,
            "expected ~$0.63072 (down from the reported $1.56), got ${:.6}",
            full.amount_usd
        );
    }

    /// (a) A counted 30k-token prompt reserves input for that prompt plus the
    /// margin — 49,096 tokens, not the 395,904-token window — so the input
    /// side of the hold stays under $0.04 per attempt.
    ///
    /// Unlike before `RESERVED_OUTPUT_TOKENS` existed, the OUTPUT side no
    /// longer prices this model's raw 128,000-token `max_tokens`: it is
    /// capped at 4,096 (this model's own max is well above the cap, so the
    /// cap wins), which is also the number now threaded through to become
    /// the request's own `max_output_tokens`
    /// (`agents::reply_parts::stream_response_from_provider`). The
    /// assertions pin both sides: input bounded by what is actually sent,
    /// output bounded by the enforced cap rather than the model's ceiling.
    #[test]
    fn a_counted_prompt_bounds_input_by_what_is_sent_not_the_window() {
        let cfg = config(Some(400_000), Some(128_000));
        let counted = calculate_bound_for_prompt(
            &cfg,
            2,
            gpt_5_4_mini_pricing(),
            PricingProvenance::CanonicalExact,
            Some(30_000),
        )
        .unwrap();
        let full = calculate_bound_for_prompt(
            &cfg,
            2,
            gpt_5_4_mini_pricing(),
            PricingProvenance::CanonicalExact,
            None,
        )
        .unwrap();

        assert_eq!(
            counted.input_tokens,
            30_000 + 15_000 + PROMPT_OVERHEAD_TOKENS
        );
        assert_eq!(counted.output_tokens, RESERVED_OUTPUT_TOKENS);
        assert_eq!(counted.max_physical_attempts, 2);
        // Never below what is actually sent: the margin only ever adds.
        assert!(counted.input_tokens >= 30_000);
        // Nowhere near the whole window.
        assert!(counted.input_tokens * 5 < full.input_tokens);

        let per_attempt_input = counted.input_tokens as f64 / 1_000_000.0 * 0.75;
        let per_attempt_output = RESERVED_OUTPUT_TOKENS as f64 / 1_000_000.0 * 4.5;
        let expected = (per_attempt_input + per_attempt_output) * 2.0;
        assert!(
            (counted.amount_usd - expected).abs() < 1e-9,
            "expected ${expected:.6}, got ${:.6}",
            counted.amount_usd
        );
        assert!(per_attempt_input < 0.04);
        // The saving is exactly the input the window reserved and the prompt
        // does not need (output is identical — RESERVED_OUTPUT_TOKENS — on
        // both sides of this comparison).
        let saved = (full.input_tokens - counted.input_tokens) as f64 / 1_000_000.0 * 0.75 * 2.0;
        assert!((full.amount_usd - counted.amount_usd - saved).abs() < 1e-9);
    }

    /// The same, through the public entry point and the live registry row.
    #[test]
    fn plan_for_prompt_uses_the_counted_prompt_for_a_priced_pair() {
        let cfg = config(Some(400_000), Some(128_000));
        let bound = plan_reservation_bound_for_prompt(
            "openai",
            "gpt-5.4-mini",
            CostTier::PaidApi,
            &cfg,
            2,
            Some(30_000),
        )
        .unwrap()
        .unwrap();
        assert_eq!(bound.input_tokens, 49_096);
        assert!(bound.amount_usd < 1.56);
    }

    /// (b) An unknown prompt size keeps the full-window bound: a caller that
    /// cannot count its request never gets a smaller hold than the worst case.
    #[test]
    fn an_unknown_prompt_size_still_reserves_the_full_window() {
        let cfg = config(Some(400_000), Some(128_000));
        let unknown = calculate_bound_for_prompt(
            &cfg,
            2,
            gpt_5_4_mini_pricing(),
            PricingProvenance::CanonicalExact,
            None,
        )
        .unwrap();
        assert_eq!(unknown.input_tokens, 400_000 - RESERVED_OUTPUT_TOKENS);
        assert_eq!(
            unknown,
            calculate_bound(
                &cfg,
                2,
                gpt_5_4_mini_pricing(),
                PricingProvenance::CanonicalExact
            )
            .unwrap()
        );
        assert_eq!(
            plan_reservation_bound_for_prompt(
                "openai",
                "gpt-5.4-mini",
                CostTier::PaidApi,
                &cfg,
                2,
                None
            ),
            plan_reservation_bound("openai", "gpt-5.4-mini", CostTier::PaidApi, &cfg, 2)
        );
    }

    /// The reported first call ("spent $0.12 ... requested bound $1.56,
    /// ceiling $1.00") against what ships NOW: the two-attempt default and the
    /// current task gate, read from `BudgetConfig::default()` rather than
    /// written as a literal, so this cannot drift when a ceiling moves.
    ///
    /// This is the requirement `RESERVED_OUTPUT_TOKENS` exists to satisfy: a
    /// bound for this exact reported shape that leaves real room under the
    /// gate, not merely a smaller number that still crowds it. Counting the
    /// ~30k-token prompt (30,040 tokens under o200k for the reply_parts
    /// fixture) AND capping the output side at 4,096 instead of this model's
    /// raw 128,000-token `max_tokens` takes the two-attempt hold from the
    /// reported $1.56 down to about $0.11 — nowhere near the $1.225734 a
    /// counted-prompt-only fix (no output cap) would still have produced.
    #[test]
    fn the_reported_first_call_fits_the_shipped_gate_at_the_shipped_default() {
        let gate = crate::cost_router::budget::BudgetConfig::default()
            .task
            .gate;
        let headroom = gate - 0.12;
        let cfg = config(Some(400_000), Some(128_000));
        let bound = |attempts, counted| {
            calculate_bound_for_prompt(
                &cfg,
                attempts,
                gpt_5_4_mini_pricing(),
                PricingProvenance::CanonicalExact,
                counted,
            )
            .unwrap()
            .amount_usd
        };
        let shipped = reserved_physical_attempts_with(5, |_| None);
        assert_eq!(shipped, 2);

        let counted = bound(shipped, Some(30_040));
        assert!(
            (counted - 0.110_598).abs() < 1e-6,
            "expected $0.110598, got ${counted:.6}"
        );
        assert!(
            counted < headroom,
            "the reported call must fit the shipped ${gate:.2} gate: ${counted:.4}"
        );
        // Counting the prompt AND capping the output both matter: even the
        // uncounted full-window bound is now $0.63072 (down from the
        // reported $1.56), because RESERVED_OUTPUT_TOKENS shrinks the output
        // side regardless of whether the prompt was counted at all.
        assert!((bound(2, None) - 0.63072).abs() < 1e-9);
        assert!(counted < bound(2, None));
    }

    /// (c) A counted prompt at or beyond the window is clamped to the input
    /// the window can accept next to a full reply — never more (the provider
    /// cannot bill it) and never an arithmetic overflow.
    ///
    /// The window itself is now bigger than before `RESERVED_OUTPUT_TOKENS`
    /// existed (395,904 rather than 272,000): capping output at 4,096
    /// instead of this model's raw 128,000-token `max_tokens` frees up the
    /// difference for input capacity, since input capacity is always
    /// `context - output`.
    #[test]
    fn a_prompt_larger_than_the_context_is_clamped_to_the_window() {
        let cfg = config(Some(400_000), Some(128_000));
        let window_input = 400_000 - RESERVED_OUTPUT_TOKENS;
        for counted in [280_000, 300_000, 400_000, 1_000_000, usize::MAX] {
            let bound = calculate_bound_for_prompt(
                &cfg,
                2,
                gpt_5_4_mini_pricing(),
                PricingProvenance::CanonicalExact,
                Some(counted),
            )
            .unwrap();
            assert_eq!(bound.input_tokens, window_input, "counted = {counted}");
        }
        // The margin itself never wraps: half of the largest count plus the
        // fixed overhead still fits, and adding it to that count saturates.
        assert_eq!(
            prompt_token_margin(usize::MAX),
            usize::MAX / 2 + PROMPT_OVERHEAD_TOKENS
        );
        assert_eq!(
            usize::MAX.saturating_add(prompt_token_margin(usize::MAX)),
            usize::MAX
        );
    }

    /// A counted prompt changes only the size of a paid hold: a free local
    /// pair still holds nothing, and a paid pair whose limits are unknowable
    /// is still refused.
    #[test]
    fn a_counted_prompt_keeps_the_free_exit_and_the_fail_closed_errors() {
        assert_eq!(
            plan_reservation_bound_for_prompt(
                "qwen38_split",
                "qwen3.8-27b",
                CostTier::PaidApi,
                &config(Some(4_096), None),
                DEFAULT_RESERVED_ATTEMPTS,
                Some(1_000),
            )
            .unwrap(),
            None
        );
        assert_eq!(
            plan_reservation_bound_for_prompt(
                "openai",
                "gpt-5.4",
                CostTier::PaidApi,
                &config(Some(400_000), None),
                DEFAULT_RESERVED_ATTEMPTS,
                Some(1_000),
            ),
            Err(ReservationBoundError::MissingMaxOutput)
        );
        assert_eq!(
            plan_reservation_bound_for_prompt(
                "openai",
                "gpt-5.4",
                CostTier::PaidApi,
                &config(Some(400_000), Some(128_000)),
                0,
                Some(1_000),
            ),
            Err(ReservationBoundError::InvalidAttempts)
        );
    }

    /// 2026-09-25: a reasoning model's reasoning is capped as output, so the
    /// reservation (and through it the wire `max_output_tokens`) must leave
    /// room for it. The catalog's `reasoning` flag decides, not the name.
    #[test]
    fn a_reasoning_model_reserves_room_for_its_reasoning() {
        let reasoning = ModelConfig {
            reasoning: Some(true),
            ..config(Some(400_000), Some(128_000))
        };
        let bound = plan_reservation_bound("openai", "gpt-4o", CostTier::PaidApi, &reasoning, 2)
            .unwrap()
            .unwrap();
        assert_eq!(bound.output_tokens, REASONING_RESERVED_OUTPUT_TOKENS);
        assert_eq!(
            bound.input_tokens,
            400_000 - REASONING_RESERVED_OUTPUT_TOKENS
        );

        let plain = ModelConfig {
            reasoning: Some(false),
            ..config(Some(400_000), Some(128_000))
        };
        let bound = plan_reservation_bound("openai", "gpt-4o", CostTier::PaidApi, &plain, 2)
            .unwrap()
            .unwrap();
        assert_eq!(bound.output_tokens, RESERVED_OUTPUT_TOKENS);

        // Still a ceiling, never a floor: a reasoning model whose own
        // max_tokens is smaller keeps its own number.
        let small = ModelConfig {
            reasoning: Some(true),
            ..config(Some(400_000), Some(8_000))
        };
        let bound = plan_reservation_bound("openai", "gpt-4o", CostTier::PaidApi, &small, 1)
            .unwrap()
            .unwrap();
        assert_eq!(bound.output_tokens, 8_000);
    }

    /// Requirement 3 of the `RESERVED_OUTPUT_TOKENS` fix: a model whose own
    /// `max_tokens` is smaller than the cap must reserve (and be sent) its
    /// own smaller number, never inflated up to 4,096. `RESERVED_OUTPUT_TOKENS`
    /// is a ceiling, not a floor.
    #[test]
    fn a_model_with_max_tokens_below_the_cap_reserves_its_own_smaller_number() {
        let cfg = config(Some(16_000), Some(2_000));
        let bound = plan_reservation_bound("openai", "gpt-4o", CostTier::PaidApi, &cfg, 1)
            .unwrap()
            .unwrap();
        assert_eq!(bound.output_tokens, 2_000);
        assert_ne!(bound.output_tokens, RESERVED_OUTPUT_TOKENS);
        assert_eq!(bound.input_tokens, 16_000 - 2_000);
    }

    /// The exact pair and shape the task report asks for: `zai`/`glm-5.2`
    /// (1,000,000-token context, 128,000-token `max_tokens`,
    /// $1.40/$4.40-per-Mtok published rates) with a ~30k-token prompt at the
    /// shipped two-attempt default, spent $0.95 and holds $1.24 already
    /// outstanding against the $3.00 task gate (both drawn from the report's
    /// own reproduction: "spent $0.95, holds $1.24, bound $1.56 [sic, for a
    /// different pair], ceiling $3.00" refuses real work).
    ///
    /// Pre-fix, pricing the full 128,000-token `max_tokens` on the output
    /// side produced a bound of ~$1.264 — added to the $2.19 already spent
    /// and held, that overruns the $3.00 gate by more than 45 cents and the
    /// call is refused. Post-fix, the same call reserves 4,096 output tokens
    /// instead and the bound drops to ~$0.174, comfortably inside the
    /// remaining $0.81 of headroom.
    #[test]
    fn the_reported_call_now_fits_for_zai_glm_5_2() {
        let cfg = config(Some(1_000_000), Some(128_000));
        let pricing = Pricing {
            input: Some(1.4),
            output: Some(4.4),
            cache_read: Some(0.26),
            cache_write: None,
        };
        let counted = Some(30_000);

        let old_style_output = 128_000usize; // this model's raw max_tokens
        let margin = 30_000 / 2 + PROMPT_OVERHEAD_TOKENS;
        let input_tokens = 30_000 + margin;
        let old_bound = (input_tokens as f64 / 1_000_000.0 * 1.4
            + old_style_output as f64 / 1_000_000.0 * 4.4)
            * 2.0;

        let new_bound = calculate_bound_for_prompt(
            &cfg,
            DEFAULT_RESERVED_ATTEMPTS,
            pricing,
            PricingProvenance::CanonicalExact,
            counted,
        )
        .unwrap();

        let gate = crate::cost_router::budget::BudgetConfig::default()
            .task
            .gate;
        let already_committed = 0.95 + 1.24; // spent + held, from the report
        let headroom = gate - already_committed;

        assert_eq!(new_bound.output_tokens, RESERVED_OUTPUT_TOKENS);
        assert!(
            old_bound > headroom,
            "the OLD max_tokens-based bound (${old_bound:.6}) must exceed the \
             remaining ${headroom:.2} headroom, reproducing the reported refusal"
        );
        assert!(
            new_bound.amount_usd < headroom,
            "the NEW output-capped bound (${:.6}) must fit the remaining ${headroom:.2} \
             headroom",
            new_bound.amount_usd
        );
        assert!(
            new_bound.amount_usd < old_bound,
            "the fix must strictly reduce the bound for this reported shape"
        );
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
