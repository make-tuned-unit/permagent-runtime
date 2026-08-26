//! The objective model knowledge base — measured, vendor-neutral attributes for
//! the models a user might have access to, so the router can recommend the best
//! fit per workflow role from DATA, not from vendor preference.
//!
//! ## Why this exists
//!
//! The tiered packs in [`super::packs`] carry a chosen *default* set of models.
//! The standing rule for the recommender is the opposite of a default: **no model is
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
//! row when a fresher measurement lands — and bump [`KB_SNAPSHOT_DATE`], which
//! [`kb_is_stale`] measures against so surfaces can say how old the numbers are.
//! Per-metric sources:
//!
//! - `edit_format_reliability` ← the aider polyglot "percent using correct edit
//!   format" / pass-rate leaderboard (aider.chat/docs/leaderboards). Diff-format
//!   reliability is exactly what that benchmark measures. Leaderboard snapshot
//!   read 2026-07-15.
//! - `orchestration_strength` ← SWE-bench Verified resolved-rate and comparable
//!   agentic/tool-use benchmarks (swebench.com / vals.ai), normalized to 0..1.
//!   Snapshot read 2026-07-15.
//! - pricing ← each vendor's published price list. Anthropic rows follow the
//!   Claude platform price table; MiniMax/Kimi rows agree with the canonical
//!   pricing table `super::cheap` ranks against (MiniMax-M2.5 0.30+1.20,
//!   kimi-k2.5 0.60+3.00) so this KB and the live ledger never disagree.
//!
//! NO family preference is baked into the numbers or the logic: locals are
//! cheaper but weaker, frontier models are stronger but dearer, and the strong
//! edit-format / strong orchestration rows are spread across vendors — exactly as
//! the public leaderboards show (2026-07-15 snapshot: Google/Gemini 3 Pro tops
//! the aider edit-format column, Anthropic/Fable 5 tops SWE-bench Verified —
//! different vendors lead different metrics, so no single family is best at all).

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
        // edit: aider well-formed-edit ~0.973–0.987 (Opus-4 proxy, LB 2026-07-15).
        // orch: SWE-bench Verified 88.6% (vals.ai LB 2026-07-15).
        edit_format_reliability: 0.975,
        orchestration_strength: 0.886,
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
        // edit: aider well-formed-edit ~0.973–0.982 (Sonnet-4 proxy, LB 2026-07-15).
        // orch ~0.80: no single SWE-bench Verified row; sources span 0.727 (Anthropic
        // launch) to 0.821 (scaffolded), placed just below Opus 4.8.
        edit_format_reliability: 0.980,
        orchestration_strength: 0.80,
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
        // edit ~0.905 (Haiku-3.5 proxy 0.911, aider LB 2026-07-15).
        // orch 0.63: no public SWE-bench Verified row for Haiku 4.5 — small-model estimate.
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
        // edit ~0.985 (Opus-4 no-thinking proxy 0.987, aider LB 2026-07-15).
        // orch: SWE-bench Verified 95.0% — current LB leader (vals.ai 2026-07-15).
        edit_format_reliability: 0.985,
        orchestration_strength: 0.95,
        input_usd_per_mtok: 10.00,
        output_usd_per_mtok: 50.00,
        cache_support: true,
        context_window: 1_000_000,
        is_local: false,
    },
    // ── OpenAI — approximate published pricing; update from the platform ─────
    ModelKnowledge {
        provider: "openai",
        model: "gpt-5.6-sol",
        display_name: "GPT-5.6 Sol",
        family: "openai",
        edit_format_reliability: 0.950,
        orchestration_strength: 0.83,
        input_usd_per_mtok: 5.00,
        output_usd_per_mtok: 30.00,
        cache_support: true,
        context_window: 1_000_000,
        is_local: false,
    },
    ModelKnowledge {
        provider: "openai",
        model: "gpt-5.6-terra",
        display_name: "GPT-5.6 Terra",
        family: "openai",
        edit_format_reliability: 0.940,
        orchestration_strength: 0.80,
        input_usd_per_mtok: 2.50,
        output_usd_per_mtok: 15.00,
        cache_support: true,
        context_window: 1_000_000,
        is_local: false,
    },
    ModelKnowledge {
        provider: "openai",
        model: "gpt-5.6-luna",
        display_name: "GPT-5.6 Luna",
        family: "openai",
        edit_format_reliability: 0.885,
        orchestration_strength: 0.60,
        input_usd_per_mtok: 1.00,
        output_usd_per_mtok: 6.00,
        cache_support: true,
        context_window: 1_000_000,
        is_local: false,
    },
    ModelKnowledge {
        provider: "openai",
        model: "gpt-5.6",
        display_name: "GPT-5.6",
        family: "openai",
        // Alias / flagship row — Sol pricing when the bare id is used.
        edit_format_reliability: 0.950,
        orchestration_strength: 0.83,
        input_usd_per_mtok: 5.00,
        output_usd_per_mtok: 30.00,
        cache_support: true,
        context_window: 1_000_000,
        is_local: false,
    },
    ModelKnowledge {
        provider: "openai",
        model: "gpt-5.6-mini",
        display_name: "GPT-5.6 mini",
        family: "openai",
        edit_format_reliability: 0.885,
        orchestration_strength: 0.60,
        input_usd_per_mtok: 1.00,
        output_usd_per_mtok: 6.00,
        cache_support: true,
        context_window: 1_000_000,
        is_local: false,
    },
    ModelKnowledge {
        provider: "openai",
        model: "gpt-5.4-mini",
        display_name: "GPT-5.4 mini",
        family: "openai",
        // pricing: canonical table $0.75/$4.50 (cache_read $0.075), 2026-03-17.
        // orch 0.62: mini / coding-harness estimate — clears the small-review
        // floor, below the large-diff strong floor. No SWE-bench Verified row.
        edit_format_reliability: 0.885,
        orchestration_strength: 0.62,
        input_usd_per_mtok: 0.75,
        output_usd_per_mtok: 4.50,
        cache_support: true,
        context_window: 400_000,
        is_local: false,
    },
    // ── DeepSeek — peak list price (api-docs.deepseek.com, 2026-08-24) ───────
    // Provider id is the configured declarative name (`custom_deepseek`).
    // `deepseek-chat` aliases to `deepseek-v4-flash` on the live API.
    ModelKnowledge {
        provider: "custom_deepseek",
        model: "deepseek-chat",
        display_name: "DeepSeek Chat",
        family: "deepseek",
        // edit 0.905: V3-class proxy (aider). orch 0.62: flash / cheap-rung
        // estimate — clears the small-review floor. Peak rate; off-peak is half.
        edit_format_reliability: 0.905,
        orchestration_strength: 0.62,
        input_usd_per_mtok: 0.44,
        output_usd_per_mtok: 1.32,
        cache_support: true,
        context_window: 128_000,
        is_local: false,
    },
    ModelKnowledge {
        provider: "custom_deepseek",
        model: "deepseek-v4-flash",
        display_name: "DeepSeek V4 Flash",
        family: "deepseek",
        // Same billed model as `deepseek-chat` on 2026-08-25. Peak $0.44/$1.32
        // from published_prices.rs (the 2026-08-24 $0.00-ledger hole).
        edit_format_reliability: 0.905,
        orchestration_strength: 0.62,
        input_usd_per_mtok: 0.44,
        output_usd_per_mtok: 1.32,
        cache_support: true,
        context_window: 128_000,
        is_local: false,
    },
    // ── xAI — approximate ────────────────────────────────────────────────────
    ModelKnowledge {
        provider: "xai",
        model: "grok-4.5",
        display_name: "Grok 4.5",
        family: "xai",
        // pricing: xAI list $2/$6 (≤200K ctx; higher tier >200K), 2026-07-15.
        // edit 0.970 (Grok-4 proxy 0.973, aider LB 2026-07-15).
        // orch: SWE-bench Verified 86.6% (vals.ai LB 2026-07-15).
        edit_format_reliability: 0.970,
        orchestration_strength: 0.866,
        input_usd_per_mtok: 2.00,
        output_usd_per_mtok: 6.00,
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
        // pricing: Google list $2/$12 (≤200K ctx; $4/$18 >200K), 2026-07-15.
        // edit 0.990: Gemini 2.5 Pro tops the aider well-formed-edit column at
        // 0.996–1.000 (LB 2026-07-15) — Gemini leads edit-format, at/above Claude;
        // corrected UP from 0.955 (removes the artificial Anthropic-on-top ordering).
        // orch ~0.80: SWE-bench Verified (Gemini 3.1 Pro 80.6%, vals.ai/llm-stats 2026-07-15).
        edit_format_reliability: 0.990,
        orchestration_strength: 0.80,
        input_usd_per_mtok: 2.00,
        output_usd_per_mtok: 12.00,
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
        // edit 0.930 (Kimi-K2 proxy 0.929, aider LB 2026-07-15).
        // orch: SWE-bench Verified 76.8% (llm-stats LB 2026-07-15).
        // price 0.60/3.00 = Moonshot list price, matches canonical super::cheap table.
        edit_format_reliability: 0.930,
        orchestration_strength: 0.77,
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
        // edit 0.910: no aider row; mid open-weight proxy.
        // orch: SWE-bench Verified 80.2% (llm-stats LB 2026-07-15) — corrected up from 0.66.
        // price 0.30/1.20 = MiniMax list price, matches canonical super::cheap table.
        edit_format_reliability: 0.910,
        orchestration_strength: 0.80,
        input_usd_per_mtok: 0.30,
        output_usd_per_mtok: 1.20,
        cache_support: false,
        // 204_800 = the canonical MiniMax-M2.5 window (providers/declarative/minimax.json);
        // corrected from an earlier 1_000_000 that contradicted the shipped provider data.
        context_window: 204_800,
        is_local: false,
    },
    // ── Z.AI / Zhipu GLM — pricing from the Z.AI list price table ───────────
    // All three rows priced from https://docs.z.ai/guides/overview/pricing
    // (fetched 2026-08-24); context/output limits from the per-model guides
    // under https://docs.z.ai/guides/llm/ (same date). Z.AI publishes no
    // SWE-bench Verified row, so `orchestration_strength` below is derived from
    // the nearest agentic benchmark Z.AI does publish and is APPROXIMATE — it is
    // not on the same measured scale as the SWE-bench-derived rows above.
    ModelKnowledge {
        provider: "zai",
        model: "glm-5.2",
        display_name: "GLM-5.2",
        family: "glm",
        // edit 0.940: no aider row; top-of-open-weight coding proxy (Z.AI reports
        // the highest open-source scores across several coding benchmarks).
        // orch 0.81: Terminal-Bench 2.1 81.0 vs Claude Opus 4.8 85.0 (docs.z.ai
        // glm-5.2 guide, 2026-08-24) — agentic proxy, not SWE-bench Verified.
        edit_format_reliability: 0.940,
        orchestration_strength: 0.81,
        input_usd_per_mtok: 1.40,
        output_usd_per_mtok: 4.40,
        cache_support: true,
        context_window: 1_000_000,
        is_local: false,
    },
    ModelKnowledge {
        provider: "zai",
        model: "glm-4.7",
        display_name: "GLM-4.7",
        family: "glm",
        // edit 0.930: no aider row; mid/high open-weight proxy, level with Kimi.
        // orch 0.79: LiveCodeBench V6 84.9 and tau^2-Bench 84.7 for tool
        // invocation (docs.z.ai glm-4.7 guide, 2026-08-24) — agentic proxy.
        edit_format_reliability: 0.930,
        orchestration_strength: 0.79,
        input_usd_per_mtok: 0.60,
        output_usd_per_mtok: 2.20,
        cache_support: true,
        context_window: 204_800,
        is_local: false,
    },
    ModelKnowledge {
        provider: "zai",
        model: "glm-4.5-air",
        display_name: "GLM-4.5 Air",
        family: "glm",
        // edit 0.880 / orch 0.62: ESTIMATES. The 106B Air variant has no public
        // aider or SWE-bench row; scored below the full GLM-4.5 line on the
        // strength of its parameter count alone. Cheapest paid Z.AI rung.
        edit_format_reliability: 0.880,
        orchestration_strength: 0.62,
        input_usd_per_mtok: 0.20,
        output_usd_per_mtok: 1.10,
        cache_support: true,
        context_window: 131_072,
        is_local: false,
    },
    // ── Ollama local ($0, private) ───────────────────────────────────────────
    ModelKnowledge {
        provider: "ollama",
        model: "qwen3-coder",
        display_name: "Qwen3-Coder (local)",
        family: "ollama",
        // edit 0.860 (aider LB 2026-07-15: Qwen3-235B 0.929 / 32B 0.836; local-coder
        // estimate between). orch 0.55: local-variant estimate, no public SWE-bench row.
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
        // edit 0.760: small local Qwen3 (Qwen3-32B aider 0.836 is the upper bound,
        // LB 2026-07-15). orch 0.48: small-local estimate, no public SWE-bench row.
        edit_format_reliability: 0.760,
        orchestration_strength: 0.48,
        input_usd_per_mtok: 0.0,
        output_usd_per_mtok: 0.0,
        cache_support: false,
        context_window: 40_960,
        is_local: true,
    },
];

/// Strip a trailing `-YYYYMMDD` 8-digit date suffix from a model id — e.g. the
/// dated Anthropic form `claude-haiku-4-5-20251001` → `claude-haiku-4-5`. Returns
/// `s` unchanged when there is no such suffix. Used only for the alias fallback in
/// [`lookup`], so a row keyed by the canonical dated id and a query using the
/// undated alias (the id the runtime actually surfaces via
/// `ANTHROPIC_DEFAULT_FAST_MODEL`) resolve to the same row.
fn strip_date_suffix(s: &str) -> &str {
    if let Some((base, tail)) = s.rsplit_once('-') {
        if tail.len() == 8 && tail.bytes().all(|b| b.is_ascii_digit()) {
            return base;
        }
    }
    s
}

/// The date the seed numbers were last read from the public leaderboards / price
/// lists (see the module note). The numeric scores are NOT refreshed by code —
/// a scored refresh is a reviewed, cited edit to [`KNOWN_MODELS`]. Exposed so
/// surfaces can show how old the snapshot is ([`kb_is_stale`]).
pub const KB_SNAPSHOT_DATE: &str = "2026-07-15";

/// After this many days the snapshot is considered stale: model releases and
/// price changes land roughly quarterly, so a recommendation built on a snapshot
/// more than 90 days old should carry a "refresh the knowledge base" note rather
/// than be presented as current. Advisory only — nothing stops working.
pub const KB_SNAPSHOT_STALE_AFTER_DAYS: i64 = 90;

/// The snapshot date as a `NaiveDate`. Panics only if the const above is not a
/// valid `YYYY-MM-DD` — guarded by a test.
pub fn kb_snapshot_date() -> chrono::NaiveDate {
    chrono::NaiveDate::parse_from_str(KB_SNAPSHOT_DATE, "%Y-%m-%d")
        .expect("KB_SNAPSHOT_DATE must be a valid YYYY-MM-DD date")
}

/// Whether the seed snapshot is older than [`KB_SNAPSHOT_STALE_AFTER_DAYS`] as of
/// `today`. Pure (the caller passes today's date) so the UI/CLI can show a
/// "knowledge base snapshot is N days old — scores are estimates" note and tests
/// don't depend on the wall clock.
pub fn kb_is_stale(today: chrono::NaiveDate) -> bool {
    (today - kb_snapshot_date()).num_days() > KB_SNAPSHOT_STALE_AFTER_DAYS
}

/// How a [`lookup_with_confidence`] hit was resolved — so a caller can say
/// honestly whether the row's numbers describe the exact model or the nearest
/// known member of its family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LookupConfidence {
    /// Exact `provider`/`model` id (or a case-only difference).
    Exact,
    /// Dated/undated alias of the same model (`claude-haiku-4-5` ↔
    /// `claude-haiku-4-5-20251001`) — the same weights, so the row is exact.
    Alias,
    /// Resolved by family prefix: the query carried an Ollama-style `:tag`
    /// (`qwen3-coder:30b`, `qwen3:latest`) and matched the row for the untagged
    /// base id. The row is an ESTIMATE for that variant — a 30B and a 480B pull
    /// share a family, not a score. Surfaces should label it as such.
    FamilyEstimate,
}

/// Strip an Ollama-style `:tag` (`qwen3-coder:30b` → `qwen3-coder`,
/// `qwen3:latest` → `qwen3`). Returns `s` unchanged when there is no tag.
fn strip_ollama_tag(s: &str) -> &str {
    s.split_once(':').map(|(base, _)| base).unwrap_or(s)
}

/// Look up a `provider`/`model` in the knowledge base. Provider match is exact;
/// model match is exact first, then a case-insensitive fallback so a configured
/// id that differs only in case still resolves, then a dated/undated-alias
/// fallback ([`strip_date_suffix`]) so an id configured as `claude-haiku-4-5`
/// resolves to the row keyed by the canonical `claude-haiku-4-5-20251001` (and
/// vice-versa) instead of falling into `unknown_models`. `None` = not in the KB
/// (the caller reports it as "unknown — add it to be objectively recommended").
///
/// Finally a **family-prefix** fallback strips an Ollama-style `:tag` and matches
/// the untagged base id, so the models an Ollama install actually reports
/// (`qwen3-coder:30b`, `qwen3:latest`) resolve to the nearest known row instead
/// of being unrecommendable. That hit is an estimate — use
/// [`lookup_with_confidence`] to tell the cases apart.
pub fn lookup(provider: &str, model: &str) -> Option<&'static ModelKnowledge> {
    lookup_with_confidence(provider, model).map(|(m, _)| m)
}

/// [`lookup`] plus HOW the row was matched ([`LookupConfidence`]).
pub fn lookup_with_confidence(
    provider: &str,
    model: &str,
) -> Option<(&'static ModelKnowledge, LookupConfidence)> {
    let exact = KNOWN_MODELS
        .iter()
        .find(|m| m.provider == provider && m.model == model)
        .or_else(|| {
            KNOWN_MODELS.iter().find(|m| {
                m.provider.eq_ignore_ascii_case(provider) && m.model.eq_ignore_ascii_case(model)
            })
        });
    if let Some(m) = exact {
        return Some((m, LookupConfidence::Exact));
    }
    // Alias fallback: compare with any trailing date suffix removed on both
    // sides, provider-scoped so it can never collide across vendors.
    let q = strip_date_suffix(model);
    if let Some(m) = KNOWN_MODELS.iter().find(|m| {
        m.provider.eq_ignore_ascii_case(provider)
            && strip_date_suffix(m.model).eq_ignore_ascii_case(q)
    }) {
        return Some((m, LookupConfidence::Alias));
    }
    // Family-prefix fallback: an Ollama-style `:tag` stripped, matched on the
    // untagged base id. Provider-scoped; only fires when there WAS a tag, so an
    // unrelated untagged id still misses.
    let base = strip_ollama_tag(model);
    if base != model && !base.is_empty() {
        if let Some(m) = KNOWN_MODELS.iter().find(|m| {
            m.provider.eq_ignore_ascii_case(provider) && m.model.eq_ignore_ascii_case(base)
        }) {
            return Some((m, LookupConfidence::FamilyEstimate));
        }
    }
    None
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

    /// Z.AI is selectable by the router: its rows resolve, carry the published
    /// list price, and sit between the cheap open-weight tier and the frontier.
    /// This asserts availability only — nothing here changes default routing.
    #[test]
    fn zai_glm_rows_resolve_with_published_prices() {
        let glm47 = lookup("zai", "glm-4.7").expect("glm-4.7 must be in the KB");
        assert_eq!(glm47.input_usd_per_mtok, 0.60);
        assert_eq!(glm47.output_usd_per_mtok, 2.20);
        assert!(glm47.cache_support, "Z.AI bills a cached-input rate");
        assert!(!glm47.is_local);

        let air = lookup("zai", "glm-4.5-air").expect("glm-4.5-air must be in the KB");
        let flagship = lookup("zai", "glm-5.2").expect("glm-5.2 must be in the KB");
        assert_eq!(flagship.context_window, 1_000_000);

        // Cost-conscious ordering holds within the provider: Air < 4.7 < 5.2.
        assert!(air.blended_cost_per_mtok() < glm47.blended_cost_per_mtok());
        assert!(glm47.blended_cost_per_mtok() < flagship.blended_cost_per_mtok());

        // And the cheapest Z.AI rung undercuts the frontier it would stand in for.
        let opus = lookup("anthropic", "claude-opus-4-8").unwrap();
        assert!(air.blended_cost_per_mtok() < opus.blended_cost_per_mtok());
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
    fn deepseek_and_gpt54_mini_resolve_with_published_prices() {
        let chat = lookup("custom_deepseek", "deepseek-chat").expect("deepseek-chat must resolve");
        let flash =
            lookup("custom_deepseek", "deepseek-v4-flash").expect("deepseek-v4-flash must resolve");
        assert_eq!(chat.family, "deepseek");
        assert_eq!(flash.family, "deepseek");
        assert!((chat.input_usd_per_mtok - 0.44).abs() < 1e-9);
        let mini = lookup("openai", "gpt-5.4-mini").expect("gpt-5.4-mini must resolve");
        assert_eq!(mini.family, "openai");
        assert!(mini.orchestration_strength >= 0.60);
    }

    #[test]
    fn no_vendor_monopolizes_the_leaderboards() {
        // Objectivity guard. The STATED property, now actually asserted (F4): the
        // edit-format leader and the orchestration leader are NOT the same family,
        // so no single vendor tops both leaderboards and the recommender has a
        // real, vendor-neutral choice to make. (If a future edit truly makes one
        // vendor best at everything, that's the data — but it should be a
        // deliberate, reviewed change, which this failing test surfaces.)
        let non_local: Vec<&ModelKnowledge> = KNOWN_MODELS.iter().filter(|m| !m.is_local).collect();
        let top_edit = non_local
            .iter()
            .max_by(|a, b| {
                a.edit_format_reliability
                    .total_cmp(&b.edit_format_reliability)
            })
            .unwrap();
        let top_orch = non_local
            .iter()
            .max_by(|a, b| {
                a.orchestration_strength
                    .total_cmp(&b.orchestration_strength)
            })
            .unwrap();
        assert_ne!(
            top_edit.family, top_orch.family,
            "one family ({}) tops BOTH edit-format ({}/{}) and orchestration ({}/{}) — the KB \
             must reflect the real cross-vendor spread the module doc claims",
            top_edit.family, top_edit.provider, top_edit.model, top_orch.provider, top_orch.model
        );
        // And each metric has a genuine non-Anthropic contender — proves the
        // recommender is not structurally forced to Anthropic on either axis.
        let non_anthropic_edit_ceiling = non_local
            .iter()
            .filter(|m| m.family != "anthropic")
            .map(|m| m.edit_format_reliability)
            .fold(0.0_f64, f64::max);
        let non_anthropic_orch_ceiling = non_local
            .iter()
            .filter(|m| m.family != "anthropic")
            .map(|m| m.orchestration_strength)
            .fold(0.0_f64, f64::max);
        assert!(
            non_anthropic_edit_ceiling >= 0.95,
            "a non-Anthropic model must be a genuine EDIT contender (got {non_anthropic_edit_ceiling})"
        );
        assert!(
            non_anthropic_orch_ceiling >= 0.80,
            "a non-Anthropic model must be a genuine ORCHESTRATE/REVIEW contender (got {non_anthropic_orch_ceiling})"
        );
    }

    /// F6: an alias-configured Haiku (`claude-haiku-4-5`, the id the runtime
    /// surfaces) resolves to the same row as the canonical dated id
    /// (`claude-haiku-4-5-20251001`, keyed to match the pricing/pack tables) — so
    /// it is objectively recommended, not dropped into `unknown_models`.
    #[test]
    fn haiku_resolves_by_dated_id_and_undated_alias() {
        let dated = lookup("anthropic", "claude-haiku-4-5-20251001")
            .expect("the canonical dated Haiku id must resolve");
        let alias = lookup("anthropic", "claude-haiku-4-5")
            .expect("the undated Haiku alias must resolve, not fall into unknown_models");
        assert_eq!(
            (dated.provider, dated.model),
            (alias.provider, alias.model),
            "both ids must resolve to the same Haiku row"
        );
        assert_eq!(alias.display_name, "Claude Haiku 4.5");
        // The alias fallback is provider-scoped and only strips an 8-digit date —
        // an unrelated id still misses.
        assert!(lookup("anthropic", "claude-haiku-4-5-notadate").is_none());
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

    /// Installed Ollama models carry a `:tag` (`qwen3-coder:30b`, `qwen3:latest`);
    /// they resolve to the untagged family row — flagged as an ESTIMATE, not an
    /// exact score — instead of falling into `unknown_models`.
    #[test]
    fn ollama_tagged_ids_resolve_to_the_family_row_as_an_estimate() {
        let (row, conf) = lookup_with_confidence("ollama", "qwen3-coder:30b")
            .expect("a tagged Ollama id must resolve to its family row");
        assert_eq!(row.model, "qwen3-coder");
        assert_eq!(conf, LookupConfidence::FamilyEstimate);
        let (row, conf) = lookup_with_confidence("ollama", "qwen3:latest").unwrap();
        assert_eq!(row.model, "qwen3");
        assert_eq!(conf, LookupConfidence::FamilyEstimate);
        // Exact and alias hits are reported as such.
        assert_eq!(
            lookup_with_confidence("ollama", "qwen3-coder").unwrap().1,
            LookupConfidence::Exact
        );
        assert_eq!(
            lookup_with_confidence("anthropic", "claude-haiku-4-5")
                .unwrap()
                .1,
            LookupConfidence::Alias
        );
        // Provider-scoped and only for a tagged query: no cross-vendor or
        // untagged-unknown collision.
        assert!(lookup("openai", "qwen3-coder:30b").is_none());
        assert!(lookup("ollama", "qwen2.5-coder:7b").is_none());
        assert!(lookup("ollama", "qwen3-coderx").is_none());
    }

    #[test]
    fn kb_staleness_is_measured_from_the_snapshot_date() {
        let snap = kb_snapshot_date();
        assert_eq!(snap.to_string(), KB_SNAPSHOT_DATE);
        assert!(!kb_is_stale(snap));
        assert!(!kb_is_stale(
            snap + chrono::Duration::days(KB_SNAPSHOT_STALE_AFTER_DAYS)
        ));
        assert!(kb_is_stale(
            snap + chrono::Duration::days(KB_SNAPSHOT_STALE_AFTER_DAYS + 1)
        ));
    }
}
