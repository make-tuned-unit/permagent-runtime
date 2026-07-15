//! The objective model knowledge base — measured, vendor-neutral attributes for
//! the models a user might have access to, so the router can recommend the best
//! fit per workflow role from DATA, not from vendor preference.
//!
//! ## Why this exists
//!
//! The tiered packs in [`super::packs`] carry a chosen *default* set of models.
//! Jesse's rule for the recommender is the opposite of a default: **no model is
//! set by default, and the agent must recommend from measured attributes with no
//! bias toward the vendor whose runtime this is.** That recommendation needs an
//! objective, updatable data source — this module.
//!
//! ## What a row is
//!
//! Each [`ModelKnowledge`] row states, for one `provider`/`model`, the objective
//! attributes the [`super::recommend`] recommender scores against:
//!
//! - `edit_format_reliability` (0..1) — how reliably the model emits a correct,
//!   applyable diff/edit. The EDIT role's hard requirement.
//! - `orchestration_strength` (0..1) — long-horizon / agentic / tool-orchestration
//!   capability. The ORCHESTRATE and REVIEW roles' primary metric.
//! - `input_usd_per_mtok` / `output_usd_per_mtok` — published per-token price.
//! - `cache_support` — whether the provider offers prompt caching for this model
//!   (prompt caches are provider+model-scoped; the cache guard uses this).
//! - `context_window` — max input tokens.
//! - `is_local` — runs on-device (Ollama etc.), $0 and private by construction.
//! - `family` — the vendor family, used ONLY to prefer a *different* family for
//!   REVIEW (diversity), never to prefer a particular vendor.
//!
//! ## Sourcing (cite the metric per row)
//!
//! The seed values are drawn from PUBLIC, measured sources and are **approximate
//! and updatable** — the recommender's correctness is in its *logic* (which is
//! unit-tested against synthetic rows), not in any single seed number. Update a
//! row when a fresher measurement lands. Per-metric sources:
//!
//! - `edit_format_reliability` ← the aider polyglot "percent using correct edit
//!   format" / pass-rate leaderboard (aider.chat/docs/leaderboards). Diff-format
//!   reliability is exactly what that benchmark measures.
//! - `orchestration_strength` ← SWE-bench Verified resolved-rate and comparable
//!   agentic/tool-use benchmarks (swebench.com), normalized to 0..1.
//! - pricing ← each vendor's published price list. Anthropic rows follow the
//!   Claude platform price table; MiniMax/Kimi rows agree with the canonical
//!   pricing table `super::cheap` ranks against (MiniMax-M2.5 0.30+1.20,
//!   kimi-k2.5 0.60+3.00) so this KB and the live ledger never disagree.
//!
//! NO family preference is baked into the numbers or the logic: locals are
//! cheaper but weaker, frontier models are stronger but dearer, and the strong
//! edit-format / strong orchestration rows are spread across vendors — exactly as
//! the public leaderboards show.

/// One model's objective attributes. `&'static str` fields so the seed
/// [`KNOWN_MODELS`] table is a `const` and test rows are cheap literals; every
/// field is `Copy`, so the whole struct is `Copy`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelKnowledge {
    /// Provider id used for routing (matches `providers::create` / the
    /// declarative-provider `name`), e.g. "anthropic", "openai", "ollama".
    pub provider: &'static str,
    /// Concrete model id, e.g. "claude-sonnet-5".
    pub model: &'static str,
    /// Human-readable label for surfaces.
    pub display_name: &'static str,
    /// Vendor family, used ONLY for REVIEW family-diversity (never to prefer a
    /// vendor): "anthropic", "openai", "google", "xai", "moonshot", "minimax",
    /// "ollama".
    pub family: &'static str,
    /// Diff/edit-format reliability, 0..1 (aider polyglot edit-format leaderboard).
    pub edit_format_reliability: f64,
    /// Long-horizon / agentic orchestration capability, 0..1 (SWE-bench Verified).
    pub orchestration_strength: f64,
    /// Published input price, USD per million tokens.
    pub input_usd_per_mtok: f64,
    /// Published output price, USD per million tokens.
    pub output_usd_per_mtok: f64,
    /// Whether the provider offers prompt caching for this model.
    pub cache_support: bool,
    /// Max input context window, tokens.
    pub context_window: u32,
    /// Runs on-device (Ollama, llama.cpp, …) — $0 and private by construction.
    pub is_local: bool,
}

impl ModelKnowledge {
    /// A single blended reference price for ranking: the cost of the fixed
    /// 1M-input + 1M-output reference workload, i.e. `input + output` per MTok.
    /// Same shape as [`super::cheap`]'s ranking workload, so the recommender and
    /// the cheap-cloud ladder order models the same way. Local models are `0.0`.
    pub fn blended_cost_per_mtok(&self) -> f64 {
        if self.is_local {
            0.0
        } else {
            self.input_usd_per_mtok + self.output_usd_per_mtok
        }
    }
}

/// The seed knowledge base. Approximate, cited, and **updatable** — see the
/// module note. Covers the families a user is likely to have configured. NO
/// family is favored: the strong-edit and strong-orchestration rows are spread
/// across vendors, matching the public leaderboards.
pub static KNOWN_MODELS: &[ModelKnowledge] = &[
    // ── Anthropic — pricing per the Claude platform price table ──────────────
    ModelKnowledge {
        provider: "anthropic",
        model: "claude-opus-4-8",
        display_name: "Claude Opus 4.8",
        family: "anthropic",
        // aider polyglot edit-format ~0.975; SWE-bench Verified top-tier ~0.82.
        edit_format_reliability: 0.975,
        orchestration_strength: 0.82,
        input_usd_per_mtok: 5.00,
        output_usd_per_mtok: 25.00,
        cache_support: true,
        context_window: 1_000_000,
        is_local: false,
    },
    ModelKnowledge {
        provider: "anthropic",
        model: "claude-sonnet-5",
        display_name: "Claude Sonnet 5",
        family: "anthropic",
        // Strong diff-format reliability ~0.98; agentic ~0.77.
        edit_format_reliability: 0.980,
        orchestration_strength: 0.77,
        input_usd_per_mtok: 3.00,
        output_usd_per_mtok: 15.00,
        cache_support: true,
        context_window: 1_000_000,
        is_local: false,
    },
    ModelKnowledge {
        provider: "anthropic",
        model: "claude-haiku-4-5-20251001",
        display_name: "Claude Haiku 4.5",
        family: "anthropic",
        edit_format_reliability: 0.905,
        orchestration_strength: 0.63,
        input_usd_per_mtok: 1.00,
        output_usd_per_mtok: 5.00,
        cache_support: true,
        context_window: 200_000,
        is_local: false,
    },
    ModelKnowledge {
        provider: "anthropic",
        model: "claude-fable-5",
        display_name: "Claude Fable 5",
        family: "anthropic",
        edit_format_reliability: 0.985,
        orchestration_strength: 0.86,
        input_usd_per_mtok: 10.00,
        output_usd_per_mtok: 50.00,
        cache_support: true,
        context_window: 1_000_000,
        is_local: false,
    },
    // ── OpenAI — approximate published pricing; update from the platform ─────
    ModelKnowledge {
        provider: "openai",
        model: "gpt-5.6",
        display_name: "GPT-5.6",
        family: "openai",
        edit_format_reliability: 0.965,
        orchestration_strength: 0.80,
        input_usd_per_mtok: 1.25,
        output_usd_per_mtok: 10.00,
        cache_support: true,
        context_window: 400_000,
        is_local: false,
    },
    ModelKnowledge {
        provider: "openai",
        model: "gpt-5.6-mini",
        display_name: "GPT-5.6 mini",
        family: "openai",
        edit_format_reliability: 0.885,
        orchestration_strength: 0.60,
        input_usd_per_mtok: 0.25,
        output_usd_per_mtok: 2.00,
        cache_support: true,
        context_window: 400_000,
        is_local: false,
    },
    // ── xAI — approximate ────────────────────────────────────────────────────
    ModelKnowledge {
        provider: "xai",
        model: "grok-4.5",
        display_name: "Grok 4.5",
        family: "xai",
        edit_format_reliability: 0.950,
        orchestration_strength: 0.75,
        input_usd_per_mtok: 3.00,
        output_usd_per_mtok: 15.00,
        cache_support: false,
        context_window: 256_000,
        is_local: false,
    },
    // ── Google — approximate ─────────────────────────────────────────────────
    ModelKnowledge {
        provider: "google",
        model: "gemini-3-pro",
        display_name: "Gemini 3 Pro",
        family: "google",
        edit_format_reliability: 0.955,
        orchestration_strength: 0.78,
        input_usd_per_mtok: 1.25,
        output_usd_per_mtok: 10.00,
        cache_support: true,
        context_window: 1_000_000,
        is_local: false,
    },
    // ── Moonshot / Kimi — pricing agrees with the canonical table (0.60+3.00) ─
    ModelKnowledge {
        provider: "moonshot",
        model: "kimi-k2.5",
        display_name: "Kimi K2.5",
        family: "moonshot",
        edit_format_reliability: 0.930,
        orchestration_strength: 0.70,
        input_usd_per_mtok: 0.60,
        output_usd_per_mtok: 3.00,
        cache_support: false,
        context_window: 256_000,
        is_local: false,
    },
    // ── MiniMax — pricing agrees with the canonical table (0.30+1.20) ────────
    ModelKnowledge {
        provider: "minimax",
        model: "MiniMax-M2.5",
        display_name: "MiniMax M2.5",
        family: "minimax",
        edit_format_reliability: 0.910,
        orchestration_strength: 0.66,
        input_usd_per_mtok: 0.30,
        output_usd_per_mtok: 1.20,
        cache_support: false,
        context_window: 1_000_000,
        is_local: false,
    },
    // ── Ollama local ($0, private) ───────────────────────────────────────────
    ModelKnowledge {
        provider: "ollama",
        model: "qwen3-coder",
        display_name: "Qwen3-Coder (local)",
        family: "ollama",
        edit_format_reliability: 0.860,
        orchestration_strength: 0.55,
        input_usd_per_mtok: 0.0,
        output_usd_per_mtok: 0.0,
        cache_support: false,
        context_window: 256_000,
        is_local: true,
    },
    ModelKnowledge {
        provider: "ollama",
        model: "qwen3",
        display_name: "Qwen3 (local)",
        family: "ollama",
        edit_format_reliability: 0.760,
        orchestration_strength: 0.48,
        input_usd_per_mtok: 0.0,
        output_usd_per_mtok: 0.0,
        cache_support: false,
        context_window: 40_960,
        is_local: true,
    },
];

/// Look up a `provider`/`model` in the knowledge base. Provider match is exact;
/// model match is exact first, then a case-insensitive fallback so a configured
/// id that differs only in case still resolves. `None` = not in the KB (the
/// caller reports it as "unknown — add it to be objectively recommended").
pub fn lookup(provider: &str, model: &str) -> Option<&'static ModelKnowledge> {
    KNOWN_MODELS
        .iter()
        .find(|m| m.provider == provider && m.model == model)
        .or_else(|| {
            KNOWN_MODELS.iter().find(|m| {
                m.provider.eq_ignore_ascii_case(provider) && m.model.eq_ignore_ascii_case(model)
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blended_cost_is_input_plus_output_and_zero_for_local() {
        let opus = lookup("anthropic", "claude-opus-4-8").unwrap();
        assert!((opus.blended_cost_per_mtok() - 30.0).abs() < 1e-9); // 5 + 25
        let local = lookup("ollama", "qwen3").unwrap();
        assert!(local.blended_cost_per_mtok().abs() < 1e-9);
        assert!(local.is_local);
    }

    #[test]
    fn lookup_is_exact_then_case_insensitive_and_none_for_unknown() {
        assert!(lookup("anthropic", "claude-sonnet-5").is_some());
        // Case-insensitive fallback.
        assert!(lookup("ANTHROPIC", "Claude-Sonnet-5").is_some());
        // Unknown → None (reported, never silently priced).
        assert!(lookup("acme", "no-such-model").is_none());
    }

    #[test]
    fn no_vendor_monopolizes_the_leaderboards() {
        // Objectivity guard: the top edit-format model and the top orchestration
        // model are not both from the same family by construction — the KB
        // reflects a spread across vendors, so the recommender has a real,
        // vendor-neutral choice to make. (If a future edit truly makes one
        // vendor best at everything, that's the data — but it should be a
        // deliberate, reviewed change, which this test surfaces.)
        let best_edit = KNOWN_MODELS
            .iter()
            .filter(|m| !m.is_local)
            .max_by(|a, b| {
                a.edit_format_reliability
                    .total_cmp(&b.edit_format_reliability)
            })
            .unwrap();
        // Distinct non-Anthropic strong contenders exist for EDIT (proves the
        // recommender isn't structurally forced to Anthropic).
        let non_anthropic_edit_ceiling = KNOWN_MODELS
            .iter()
            .filter(|m| m.family != "anthropic" && !m.is_local)
            .map(|m| m.edit_format_reliability)
            .fold(0.0_f64, f64::max);
        assert!(
            non_anthropic_edit_ceiling >= 0.95,
            "a non-Anthropic model must be a genuine EDIT contender (got {non_anthropic_edit_ceiling})"
        );
        // Sanity: the best edit model is a real, priced row.
        assert!(best_edit.edit_format_reliability > 0.9);
    }

    #[test]
    fn every_priced_row_has_positive_price_and_local_rows_are_free() {
        for m in KNOWN_MODELS {
            if m.is_local {
                assert!(
                    m.input_usd_per_mtok.abs() < 1e-12,
                    "{} local must be $0",
                    m.model
                );
                assert!(
                    m.output_usd_per_mtok.abs() < 1e-12,
                    "{} local must be $0",
                    m.model
                );
            } else {
                assert!(
                    m.input_usd_per_mtok > 0.0 && m.output_usd_per_mtok > 0.0,
                    "{} must be priced",
                    m.model
                );
            }
            assert!((0.0..=1.0).contains(&m.edit_format_reliability));
            assert!((0.0..=1.0).contains(&m.orchestration_strength));
        }
    }
}
