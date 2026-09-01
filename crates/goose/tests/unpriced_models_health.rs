//! Regression: every model the routing tables can select must have a real
//! price, or be explicitly local/free.
//!
//! The 2026-08-24 nightly health review reported that a DeepSeek model's calls
//! were billing at $0.00, so the spend ceilings in `cost_router::budget` could
//! never fire. The live cost ledger confirmed it — 128 `deepseek-v4-flash` calls
//! at `cost_usd = 0.0, is_estimated = 1`.
//!
//! The mechanism (`agents::reply_parts`): a chargeable call with no canonical
//! price records `cost_usd = 0.0` and sets `is_estimated`. Nothing downstream
//! reads that flag, so an unpriced model is indistinguishable from a free one
//! and the budget gate sums real spend as zero.
//!
//! A price is therefore not cosmetic metadata — it is what makes the spend cap
//! work.
//!
//! Running this over the real registry showed the problem is far wider than one
//! DeepSeek model: **34 selectable models across 8 providers** have no published
//! rate at all. That is why the fix is not "add 34 prices" (which would mean
//! inventing most of them) but the fail-closed path in `agents::reply_parts` —
//! an unpriced chargeable call is billed at its provider's worst known rate, so
//! it can only make the ceiling fire early, never late.
//!
//! These tests therefore assert the invariant that protects the cap — every
//! selectable model resolves to SOME rate, and the fallback rate is positive —
//! rather than demanding an upstream citation that may not exist.
//! `report_models_without_a_published_price` prints the remaining list for
//! whoever works it down.

use permagent::providers::base::Usage;
use permagent::providers::canonical::{cost_of, maybe_get_pricing, worst_case_pricing};

/// Providers whose models run on this machine (or are otherwise not billed
/// per token), where a missing price is CORRECT rather than a hole.
/// `agents::reply_parts::is_local_provider` is the runtime counterpart.
const LOCAL_OR_FREE_PROVIDERS: &[&str] =
    &["ollama", "local", "lmstudio", "llama_swap", "qwen38_split"];

fn is_local_or_free(provider: &str) -> bool {
    let p = provider.to_ascii_lowercase();
    LOCAL_OR_FREE_PROVIDERS
        .iter()
        .any(|f| p == *f || p.contains(f))
}

/// A reference workload big enough that any real rate produces a
/// clearly-non-zero figure.
fn reference_usage() -> Usage {
    Usage {
        input_tokens: Some(100_000),
        output_tokens: Some(10_000),
        total_tokens: Some(110_000),
        ..Default::default()
    }
}

/// Every `(provider, model)` a declarative provider definition can select.
/// These JSON files ARE the routing table for the openai-compatible providers
/// — `deepseek.json` is where `deepseek-v4-flash` becomes selectable.
fn declarative_provider_models() -> Vec<(String, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/providers/declarative");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("declarative provider dir readable") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("declarative json readable");
        let value: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let provider = value
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or_else(|| panic!("{}: missing provider name", path.display()))
            .to_string();
        let Some(models) = value.get("models").and_then(|m| m.as_array()) else {
            continue;
        };
        for model in models {
            if let Some(name) = model.get("name").and_then(|n| n.as_str()) {
                out.push((provider.clone(), name.to_string()));
            }
        }
    }
    assert!(
        !out.is_empty(),
        "found no declarative provider models — the test would pass vacuously"
    );
    out
}

/// THE regression: no selectable model can silently bill as $0.00.
///
/// ## Why this asserts the effective cost, not "has a published price"
///
/// The first version of this test demanded a published rate for every
/// selectable model. Run against the real registry it named **34 models across
/// 8 providers** (ovhcloud, moonshot, mistral, inception, tensorix, tanzu…)
/// with no price at all — a far bigger hole than the review found, and one that
/// cannot honestly be closed by citing rates that do not exist upstream.
/// Demanding them would leave exactly two options: block the work, or invent
/// numbers. An invented price is worse than a missing one, because it makes the
/// budget gate confidently wrong instead of visibly conservative.
///
/// So this asserts the invariant that actually protects the spend cap, and that
/// the fail-closed path in `agents::reply_parts` genuinely upholds: **every
/// chargeable call resolves to a rate**, either its own published one or its
/// provider's worst known one. A model with no rate anywhere would bill $0.00
/// and blind the ceiling — that, and not a missing upstream citation, is the
/// defect worth failing the build over.
///
/// An EXPLICIT zero (a registry row that really says free, e.g. zhipu's
/// `glm-4.5-flash`) is allowed through. An ABSENT price is not.
#[test]
fn no_selectable_model_can_bill_as_zero_with_no_rate() {
    let usage = reference_usage();
    let mut unrated = Vec::new();

    for (provider, model) in declarative_provider_models() {
        if is_local_or_free(&provider) {
            continue;
        }
        // Exactly what the ledger does: the model's own price, else the
        // provider's worst known rate.
        let effective = maybe_get_pricing(&provider, &model)
            .or_else(|| worst_case_pricing(&provider))
            .and_then(|p| cost_of(&usage, &p));

        match effective {
            // Priced (or explicitly free) — fine.
            Some(_) => {}
            None => unrated.push(format!("{provider}/{model}")),
        }
    }

    assert!(
        unrated.is_empty(),
        "these models would bill as $0.00 with no rate at all, so the spend gate is blind          to them. The fail-closed path in agents::reply_parts should have caught them — if          it cannot, providers::canonical::worst_case_pricing needs to widen:\n  {}",
        unrated.join("\n  ")
    );
}

/// The fail-closed path must produce a POSITIVE rate, not merely a defined one.
/// A worst case of $0 would be the original bug wearing a different hat.
#[test]
fn the_fail_closed_rate_is_always_positive() {
    let usage = reference_usage();
    let mut broken = Vec::new();

    for (provider, model) in declarative_provider_models() {
        if is_local_or_free(&provider) {
            continue;
        }
        if maybe_get_pricing(&provider, &model).is_some() {
            continue; // Has its own price; not the fail-closed path.
        }
        match worst_case_pricing(&provider).and_then(|p| cost_of(&usage, &p)) {
            Some(c) if c > 0.0 => {}
            other => broken.push(format!("{provider}/{model} → {other:?}")),
        }
    }

    assert!(
        broken.is_empty(),
        "the worst-case fallback produced a non-positive rate for:\n  {}",
        broken.join("\n  ")
    );
}

/// Visibility, not a gate: how many selectable models still lack a PUBLISHED
/// rate. These are covered by the fail-closed path, so spend is capped — but
/// they are billed at an upper bound rather than measured, and someone should
/// work the list down. Printed, never asserted, so it cannot rot into a number
/// people bump without thinking.
#[test]
fn report_models_without_a_published_price() {
    let usage = reference_usage();
    let mut unpublished: Vec<String> = declarative_provider_models()
        .into_iter()
        .filter(|(p, _)| !is_local_or_free(p))
        .filter(|(p, m)| {
            maybe_get_pricing(p, m)
                .and_then(|pr| cost_of(&usage, &pr))
                .is_none()
        })
        .map(|(p, m)| format!("{p}/{m}"))
        .collect();
    unpublished.sort();

    println!(
        "{} selectable models have no published rate and are billed at their provider's \
         worst known rate:\n  {}",
        unpublished.len(),
        unpublished.join("\n  ")
    );
}

/// The exact model from the 2026-08-24 ledger, resolved the way the ledger
/// resolves it: by the CONFIGURED provider id (`custom_deepseek`), not the
/// canonical one.
#[test]
fn the_deepseek_model_from_the_incident_now_has_a_price() {
    let usage = reference_usage();
    let pricing = maybe_get_pricing("custom_deepseek", "deepseek-v4-flash")
        .expect("deepseek-v4-flash must be priced — 128 calls billed as $0.00 on 2026-08-23");
    let cost = cost_of(&usage, &pricing).expect("a priced model must yield a cost");
    assert!(
        cost > 0.0,
        "deepseek-v4-flash must bill above zero, got ${cost}"
    );
}

/// A local model billing at zero is CORRECT, and must not be "fixed" by giving
/// it a price. This is the case the test above must never start flagging.
#[test]
fn local_providers_are_allowed_to_be_free() {
    assert!(is_local_or_free("ollama"));
    assert!(is_local_or_free("Ollama"));
    assert!(is_local_or_free("lmstudio"));
    assert!(!is_local_or_free("custom_deepseek"));
    assert!(!is_local_or_free("anthropic"));
}

// ── Fail-closed: an unpriced model must still count against the cap ──────────

use permagent::cost_router::budget::{budget_verdict_with_unpriced, BudgetBand, BudgetConfig};
use permagent::providers::canonical::provider_worst_case_pricing;

/// The downstream half of the 2026-08-24 hole. Adding a price per newly-selectable
/// model closes today's gap; the generated table always lags what is selectable,
/// so an unknown price must fail CLOSED rather than bill as a confident $0.00.
#[test]
fn an_unpriced_model_is_billed_at_the_provider_worst_case() {
    let pricing = provider_worst_case_pricing("anthropic")
        .expect("the registry knows priced anthropic models");
    let input = pricing.input.expect("worst case has an input rate");
    let output = pricing.output.expect("worst case has an output rate");
    assert!(input > 0.0 && output > 0.0);

    // It really is the worst case: no known anthropic model costs more per
    // output token than the rate we would bill an unknown one at.
    let usage = reference_usage();
    let worst = cost_of(&usage, &pricing).expect("worst case prices");
    for model in ["claude-haiku-4-5", "claude-3.5-haiku", "claude-sonnet-4"] {
        if let Some(known) = maybe_get_pricing("anthropic", model).and_then(|p| cost_of(&usage, &p))
        {
            assert!(
                worst >= known,
                "{model} costs ${known}, above the ${worst} worst case we would bill an \
                 unpriced model at — the cap could then fire late"
            );
        }
    }
}

/// A provider id the registry has never heard of still gets a rate, so a
/// wrapper provider (`custom_deepseek` was exactly this shape) cannot slip
/// spend past the ceiling.
#[test]
fn an_unknown_provider_still_gets_a_worst_case_rate() {
    let pricing = worst_case_pricing("some_wrapper_provider_we_have_never_seen")
        .expect("must widen to the whole registry rather than give up");
    assert!(pricing.input.unwrap_or(0.0) > 0.0);
    assert!(pricing.output.unwrap_or(0.0) > 0.0);
}

/// The ceiling now fires on estimated spend. Before the fix these dollars were
/// recorded as $0.00 and the band stayed `Ok` forever.
#[test]
fn estimated_spend_still_trips_the_ceiling() {
    let cfg = BudgetConfig::default();

    // $12 of estimated task spend is past the $10 task hard ceiling.
    let verdict = budget_verdict_with_unpriced(12.0, 12.0, 40, &cfg);
    assert_eq!(
        verdict.band,
        BudgetBand::Hard,
        "unpriced-but-estimated spend must be able to hard-stop"
    );
    assert_eq!(verdict.unpriced_calls, 40, "the estimate must stay flagged");

    // And a gate is reachable the same way.
    let verdict = budget_verdict_with_unpriced(6.0, 6.0, 12, &cfg);
    assert_eq!(verdict.band, BudgetBand::Gate);
    assert!(verdict.needs_gate());
}

/// Honesty, not arithmetic: below every ceiling, unpriced calls still lift the
/// band to Soft so the user is told part of the figure is an upper bound.
#[test]
fn unpriced_calls_are_flagged_even_when_cheap() {
    let cfg = BudgetConfig::default();
    assert_eq!(
        budget_verdict_with_unpriced(0.01, 0.01, 0, &cfg).band,
        BudgetBand::Ok
    );
    assert_eq!(
        budget_verdict_with_unpriced(0.01, 0.01, 3, &cfg).band,
        BudgetBand::Soft,
        "an estimated figure must never read as a confident measured one"
    );
}
