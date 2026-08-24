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
//! work. This test fails on `unknown == $0`, which is the whole point: a model
//! added to a routing table without a price must break the build, not silently
//! disable the ceiling.

use permagent::providers::base::Usage;
use permagent::providers::canonical::{cost_of, maybe_get_pricing};

/// Providers whose models run on this machine (or are otherwise not billed
/// per token), where a missing price is CORRECT rather than a hole.
/// `agents::reply_parts::is_local_provider` is the runtime counterpart.
const LOCAL_OR_FREE_PROVIDERS: &[&str] = &["ollama", "local", "lmstudio", "llama_swap"];

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

/// THE regression. Every selectable model prices above zero, or is local/free.
#[test]
fn every_selectable_model_is_priced_or_explicitly_free() {
    let usage = reference_usage();
    let mut unpriced = Vec::new();

    for (provider, model) in declarative_provider_models() {
        if is_local_or_free(&provider) {
            continue;
        }
        let cost = maybe_get_pricing(&provider, &model).and_then(|p| cost_of(&usage, &p));
        match cost {
            Some(c) if c > 0.0 => {}
            Some(c) => unpriced.push(format!("{provider}/{model} priced at ${c} (zero)")),
            None => unpriced.push(format!("{provider}/{model} has NO price (bills as $0.00)")),
        }
    }

    assert!(
        unpriced.is_empty(),
        "these models are selectable and chargeable but bill as $0.00, so the spend gate \
         cannot fire for them. Add a published rate to \
         providers::canonical::published_prices (with the source URL and the date you read \
         it), or list the provider in LOCAL_OR_FREE_PROVIDERS if it genuinely is not billed \
         per token:\n  {}",
        unpriced.join("\n  ")
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
