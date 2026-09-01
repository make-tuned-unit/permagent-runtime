//! Which model family is answering this turn, and the small prompt overlay that
//! family needs.
//!
//! ## Why this exists
//!
//! Permagent routes across Anthropic, OpenAI, DeepSeek, Z.AI/GLM, xAI and
//! locally served open-weights models. Tool-calling discipline is a *per-family*
//! concern: the habits that need correcting differ by family (a local
//! open-weights model needs explicit tool-JSON strictness and a "no thinking
//! tags" rule; a Claude model needs neither and would only be slowed down by
//! them). Writing one prompt for the weakest reader makes every model pay for
//! the weakest model's patches on every turn.
//!
//! So: one shared body, plus a short per-family OVERLAY appended to it. NOT four
//! full prompts — the shared body is the same bytes for every family, which is
//! also what keeps provider prompt caches viable.
//!
//! ## Honesty about the overlay text
//!
//! Each overlay cites the vendor documentation it is based on, in a Rust doc
//! comment — the citations cost no tokens because they never reach the model.
//! They are grounded *hypotheses* about each family's failure modes, not
//! measured wins; the per-family snapshot tests exist so their cost stays
//! visible and so a future measurement can shrink or delete one with evidence.
//! Documentation snapshot date: 2026-08-24.
//!
//! ## Fail-safe default
//!
//! An unrecognised provider/model resolves to [`ModelFamily::Other`], whose
//! overlay is deliberately EMPTY. We add nothing to a model whose quirks we have
//! not established — spending tokens on a guess is the exact cost this module
//! exists to remove.

/// The model family answering a turn, for prompt-overlay selection only.
///
/// Not a routing or capability signal — [`crate::cost_router::knowledge`] owns
/// the objective attributes the router scores against. This enum answers one
/// narrower question: *whose prompt quirks apply?*
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ModelFamily {
    Anthropic,
    OpenAi,
    DeepSeek,
    Glm,
    Xai,
    /// Open-weights models served from this machine (Ollama, LM Studio,
    /// llama-swap, the in-process local-inference provider) — Qwen is the
    /// reference member and the Ollama default, but the overlay applies to any
    /// small open-weights chat template, because what it corrects (loose tool
    /// JSON, leaked `<think>` tags) is a property of those templates rather
    /// than of one vendor.
    QwenLocal,
    /// Recognised by nothing above. Overlay is empty on purpose.
    Other,
}

/// Provider ids that serve models from *this machine* (or a self-hosted
/// endpoint the user runs). Used only as the fallback when the model id itself
/// does not name a vendor — an Ollama-served `deepseek-r1` is still DeepSeek.
///
/// Mirrors the local-provider vocabulary already used by the sovereignty guard
/// and `reply_parts::is_local_provider`; kept as its own list because this one
/// must also catch LM Studio and llama-swap, which the ledger's local check
/// does not need to.
const SELF_HOSTED_PROVIDERS: &[&str] = &[
    "ollama",
    "lmstudio",
    "llama_swap",
    "qwen38_split",
    "local",
    "local_inference",
];

impl ModelFamily {
    /// Every family, for exhaustive table tests and per-family snapshots.
    pub const ALL: &'static [ModelFamily] = &[
        ModelFamily::Anthropic,
        ModelFamily::OpenAi,
        ModelFamily::DeepSeek,
        ModelFamily::Glm,
        ModelFamily::Xai,
        ModelFamily::QwenLocal,
        ModelFamily::Other,
    ];

    /// Resolve the family from a provider id and a model id.
    ///
    /// **Model id first, provider second.** The model id is the thing that
    /// decides behaviour, and it is the only signal that survives an aggregator:
    /// `openrouter` / `litellm` / `tetrate` / `nanogpt` all serve every vendor,
    /// so their provider id says nothing, while their model id
    /// (`anthropic/claude-sonnet-5`) says everything. Substring matching handles
    /// the vendor-prefixed forms without a parsing step.
    ///
    /// Total by construction: anything unrecognised is [`ModelFamily::Other`].
    pub fn resolve(provider: &str, model: &str) -> Self {
        if let Some(family) = Self::from_model_id(model) {
            return family;
        }
        let provider_lower = provider.to_ascii_lowercase();
        if SELF_HOSTED_PROVIDERS.contains(&provider_lower.as_str()) {
            return ModelFamily::QwenLocal;
        }
        Self::from_provider_id(&provider_lower).unwrap_or(ModelFamily::Other)
    }

    /// Classify by model id alone. `None` when the id names no known vendor.
    ///
    /// Deliberately parallel to
    /// [`crate::providers::canonical::name_builder`]'s `infer_provider_from_model`,
    /// which maps model ids to canonical *provider* ids for the model registry.
    /// This one answers a different question (prompt quirks, not catalog
    /// identity) and covers GLM, which that one has no reason to.
    fn from_model_id(model: &str) -> Option<Self> {
        let m = model.to_ascii_lowercase();

        // Checked BEFORE the `gpt-` rule: gpt-oss is OpenAI's open-weights
        // release, shipped with the harmony chat template and typically served
        // self-hosted (Groq, Tanzu, Ollama). Its failure modes are the
        // open-weights ones, not the OpenAI-API ones.
        if m.contains("gpt-oss") {
            return Some(ModelFamily::QwenLocal);
        }
        if m.contains("claude") {
            return Some(ModelFamily::Anthropic);
        }
        if m.contains("deepseek") {
            return Some(ModelFamily::DeepSeek);
        }
        if m.contains("glm") {
            return Some(ModelFamily::Glm);
        }
        if m.contains("grok") {
            return Some(ModelFamily::Xai);
        }
        if m.contains("qwen") {
            return Some(ModelFamily::QwenLocal);
        }
        if m.starts_with("gpt-")
            || m.contains("/gpt-")
            || m.starts_with("chatgpt-")
            || m.starts_with("o1")
            || m.starts_with("o3")
            || m.starts_with("o4")
            || m.contains("codex")
        {
            return Some(ModelFamily::OpenAi);
        }
        None
    }

    /// Classify by provider id, for the single-vendor providers only.
    ///
    /// Aggregators (`openrouter`, `litellm`, `nanogpt`, `tetrate`, `groq`,
    /// `databricks`, `github_copilot`, `openai_compatible` custom endpoints) are
    /// absent on purpose: they serve every vendor, so their id is not evidence
    /// of a family and guessing from it would apply the wrong overlay.
    fn from_provider_id(provider: &str) -> Option<Self> {
        // Ids exactly as the providers register them (hyphens and underscores
        // are both real — `claude-code` vs `chatgpt_codex`).
        match provider {
            "anthropic" | "claude-code" | "claude-acp" => Some(ModelFamily::Anthropic),
            "openai" | "chatgpt_codex" | "codex" | "codex-acp" | "azure_openai" => {
                Some(ModelFamily::OpenAi)
            }
            "custom_deepseek" | "deepseek" => Some(ModelFamily::DeepSeek),
            "zai" | "zhipu" => Some(ModelFamily::Glm),
            "xai" => Some(ModelFamily::Xai),
            _ => None,
        }
    }

    /// Stable id for logs, tests and snapshot names.
    pub fn as_str(&self) -> &'static str {
        match self {
            ModelFamily::Anthropic => "anthropic",
            ModelFamily::OpenAi => "openai",
            ModelFamily::DeepSeek => "deepseek",
            ModelFamily::Glm => "glm",
            ModelFamily::Xai => "xai",
            ModelFamily::QwenLocal => "qwen-local",
            ModelFamily::Other => "other",
        }
    }

    /// The prompt overlay for this family — appended once to the shared prompt
    /// body. Empty string means "add nothing", which is a real answer, not a
    /// placeholder.
    pub fn overlay(&self) -> &'static str {
        match self {
            ModelFamily::Anthropic => ANTHROPIC_OVERLAY,
            ModelFamily::OpenAi => OPENAI_OVERLAY,
            ModelFamily::DeepSeek => DEEPSEEK_OVERLAY,
            ModelFamily::Glm => GLM_OVERLAY,
            ModelFamily::Xai => XAI_OVERLAY,
            ModelFamily::QwenLocal => QWEN_LOCAL_OVERLAY,
            ModelFamily::Other => "",
        }
    }
}

/// Anthropic (Claude).
///
/// Based on Anthropic's tool-use documentation
/// (<https://docs.claude.com/en/docs/agents-and-tools/tool-use/overview>): tool
/// calls are a first-class content block, several independent calls may be
/// returned in one assistant turn, and the extended-thinking guidance
/// (<https://docs.claude.com/en/docs/build-with-claude/extended-thinking>) is
/// that thinking blocks must be passed back unmodified across a tool-use turn.
///
/// Deliberately carries NO JSON-shape nudge and no "emit valid JSON" rule: the
/// API validates tool input against the schema, so those words would be pure
/// cost. This is the shortest overlay of the five non-empty ones by design.
const ANTHROPIC_OVERLAY: &str = "\
# Model Notes

Call tools natively. Never write a tool call as text or as JSON in your reply.
Independent tool calls may go out together in one turn rather than one at a time.
Carry thinking blocks through a tool-use turn unchanged.
";

/// OpenAI (GPT / o-series / Codex).
///
/// Based on OpenAI's GPT-5 prompting guide
/// (<https://cookbook.openai.com/examples/gpt-5/gpt-5_prompting_guide>): the API
/// returns plain text unless Markdown is requested; contradictory instructions
/// cost reasoning tokens as the model tries to reconcile them; and the guide's
/// "eagerness" section warns against over-gathering context before acting.
const OPENAI_OVERLAY: &str = "\
# Model Notes

Call tools natively. Never write a tool call as text or as JSON in your reply.
Use Markdown where it earns its place — code, tables, short lists — not by default in prose.
When the next step is already clear, take it; do not re-read context you have or re-plan a settled decision.
";

/// DeepSeek (V3 / R1 / reasoner).
///
/// Based on DeepSeek's reasoning-model guide
/// (<https://api-docs.deepseek.com/guides/reasoning_model>): reasoning is
/// returned in a separate `reasoning_content` field and must not be fed back or
/// duplicated into the answer, and the documented prompting advice for the
/// reasoner is to keep the instruction direct rather than to prompt it into
/// thinking. The `<think>` rule is here because the open-weights R1 chat
/// template emits those tags literally when served outside DeepSeek's API
/// (<https://huggingface.co/deepseek-ai/DeepSeek-R1>).
const DEEPSEEK_OVERLAY: &str = "\
# Model Notes

Call tools natively. Never write a tool call as text or as JSON in your reply.
Reasoning belongs in the reasoning channel: no `<think>` tags and no restated chain of thought in the reply.
Do not repeat your instructions back to the user.
";

/// Z.AI / Zhipu (GLM-4.5, 4.6, 4.7, 5).
///
/// Based on the GLM-4.6 model card
/// (<https://huggingface.co/zai-org/GLM-4.6>), whose chat template wraps
/// reasoning in `<think>` and tool calls in `<tool_call>` blocks — both leak
/// into reply text when an OpenAI-compatible client re-serialises the turn — and
/// on Z.AI's API tool-use documentation (<https://docs.z.ai/>), which exposes
/// GLM tool calling through the standard `tool_calls` field.
const GLM_OVERLAY: &str = "\
# Model Notes

Call tools natively. Never write a tool call as text, as JSON, or inside `<tool_call>` markup in your reply.
Reasoning belongs in the reasoning channel: no `<think>` tags in the reply.
Call a tool only when it changes what you can answer — not to acknowledge a request.
";

/// xAI (Grok).
///
/// Based on xAI's function-calling documentation
/// (<https://docs.x.ai/docs/guides/function-calling>), which is
/// OpenAI-compatible and returns tool calls in `tool_calls`, and on xAI's
/// grok-code-fast-1 prompt-engineering guidance
/// (<https://docs.x.ai/docs/guides/grok-code-prompt-engineering>), which asks
/// for native tool calling rather than XML tool syntax and for specific,
/// context-complete instructions over exploratory ones.
const XAI_OVERLAY: &str = "\
# Model Notes

Call tools natively. Never write a tool call as text, as JSON, or as XML tool markup in your reply.
When a tool has already given you the fact, answer with it — do not narrate a plan to go find it.
";

/// Locally served open-weights models (Qwen is the reference member).
///
/// Based on Qwen's function-calling documentation
/// (<https://qwen.readthedocs.io/en/latest/framework/function_call.html>), which
/// documents that the tool-call JSON is produced by the chat template and is
/// only as strict as the model makes it, and on the Qwen3 model card
/// (<https://huggingface.co/Qwen/Qwen3-8B>), which documents the `<think>`
/// blocks its template emits and the `/no_think` switch that suppresses them.
///
/// This is the longest overlay, and that is the point of the whole change: it is
/// the one family that genuinely needs tool-JSON strictness, and before this it
/// was either absent or would have had to be paid for by every other family too.
const QWEN_LOCAL_OVERLAY: &str = "\
# Model Notes

Tool calls must be exact JSON for the tool's declared schema: every required parameter present, correct types, no extra keys, no comments, no trailing commas, no markdown fence around it. Never invent a tool name or a parameter.
Do not emit `<think>`, `<thinking>`, or any other reasoning tags — give the answer only.
One tool call at a time. Read its result before choosing the next step.
Keep replies short, and never restate these instructions.
";

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;

    /// The table test the whole resolver is for: every provider the registry
    /// actually registers, resolved with the default model it actually ships,
    /// lands on a family — and the ones whose family is not in doubt land on the
    /// RIGHT family, not merely on some family.
    ///
    /// Registry-driven rather than hardcoded, so adding a provider cannot
    /// silently skip this check.
    #[tokio::test]
    async fn every_registered_provider_resolves_to_a_family() {
        // Providers whose family is unambiguous. Everything else is allowed to
        // be `Other` — an aggregator serving every vendor genuinely has no
        // family until a model id is chosen, and claiming one would be a lie.
        let expected: &[(&str, ModelFamily)] = &[
            ("anthropic", ModelFamily::Anthropic),
            ("claude-code", ModelFamily::Anthropic),
            ("claude-acp", ModelFamily::Anthropic),
            ("openai", ModelFamily::OpenAi),
            ("chatgpt_codex", ModelFamily::OpenAi),
            ("codex", ModelFamily::OpenAi),
            ("custom_deepseek", ModelFamily::DeepSeek),
            ("zai", ModelFamily::Glm),
            ("zhipu", ModelFamily::Glm),
            ("xai", ModelFamily::Xai),
            ("ollama", ModelFamily::QwenLocal),
        ];

        let providers = crate::providers::providers().await;
        assert!(
            !providers.is_empty(),
            "registry produced no providers — the table below would be vacuous"
        );

        for (metadata, _kind) in &providers {
            // Total function: this must not panic and must not be able to
            // return anything outside ModelFamily::ALL.
            let family = ModelFamily::resolve(&metadata.name, &metadata.default_model);
            assert!(
                ModelFamily::ALL.contains(&family),
                "{} resolved outside the family set",
                metadata.name
            );

            if let Some((_, want)) = expected.iter().find(|(name, _)| *name == metadata.name) {
                assert_eq!(
                    family, *want,
                    "provider {:?} (default model {:?}) resolved to {:?}, expected {:?}",
                    metadata.name, metadata.default_model, family, want
                );
            }
        }

        // Non-vacuity: every provider we named above was actually present in
        // the registry, so none of those assertions was skipped.
        for (name, _) in expected {
            assert!(
                providers.iter().any(|(m, _)| m.name == *name),
                "expected provider {name:?} is no longer registered — update this table"
            );
        }
    }

    /// Unknown provider AND unknown model must be `Other`, and `Other` must add
    /// no tokens. This is the fail-safe the module docs claim.
    #[test]
    fn unknown_resolves_to_other_and_other_adds_nothing() {
        assert_eq!(
            ModelFamily::resolve("some_new_vendor", "some-new-model-v1"),
            ModelFamily::Other
        );
        assert_eq!(ModelFamily::resolve("", ""), ModelFamily::Other);
        assert_eq!(ModelFamily::Other.overlay(), "");
    }

    /// The model id decides, not the provider id — the property that makes an
    /// aggregator resolve correctly.
    #[test]
    fn model_id_wins_over_provider_id() {
        for (provider, model, want) in [
            (
                "openrouter",
                "anthropic/claude-sonnet-5",
                ModelFamily::Anthropic,
            ),
            ("litellm", "openai/gpt-5.6-terra", ModelFamily::OpenAi),
            ("nanogpt", "deepseek-v3.2", ModelFamily::DeepSeek),
            ("tetrate", "zai/glm-4.7", ModelFamily::Glm),
            ("openrouter", "x-ai/grok-code-fast-1", ModelFamily::Xai),
            ("groq", "qwen3-coder-32b", ModelFamily::QwenLocal),
            // An Ollama-served DeepSeek distill is DeepSeek, not "whatever
            // Ollama usually serves" — the model id is the stronger signal.
            ("ollama", "deepseek-r1:8b", ModelFamily::DeepSeek),
            // …and an Ollama-served model we cannot place still gets the
            // open-weights overlay, because that is what it is.
            ("ollama", "some-tuned-thing:latest", ModelFamily::QwenLocal),
            // gpt-oss is OpenAI's open-weights release: open-weights quirks.
            ("tanzu_ai", "openai/gpt-oss-120b", ModelFamily::QwenLocal),
        ] {
            assert_eq!(
                ModelFamily::resolve(provider, model),
                want,
                "{provider}/{model}"
            );
        }
    }

    /// Every non-`Other` overlay must actually be an overlay: a `# Model Notes`
    /// block, and nothing that pretends to be the shared body.
    #[test]
    fn overlays_are_well_formed_and_distinct() {
        let mut seen: Vec<&str> = Vec::new();
        for family in ModelFamily::ALL {
            let overlay = family.overlay();
            if *family == ModelFamily::Other {
                continue;
            }
            assert!(
                overlay.starts_with("# Model Notes\n"),
                "{} overlay must open with its own heading",
                family.as_str()
            );
            assert!(
                overlay.ends_with('\n'),
                "{} overlay must end with a newline so appending is clean",
                family.as_str()
            );
            assert!(
                !seen.contains(&overlay),
                "{} overlay duplicates another family's — then it belongs in the shared body",
                family.as_str()
            );
            seen.push(overlay);
        }
        assert_eq!(seen.len(), ModelFamily::ALL.len() - 1);
    }

    /// Per-family snapshot: the overlay's text AND its size, so a family's
    /// prompt cost cannot grow without showing up in a review diff.
    #[test]
    fn per_family_overlay_snapshot() {
        for family in ModelFamily::ALL {
            let overlay = family.overlay();
            let bytes = overlay.len();
            // ~4 chars/token is the rule of thumb used throughout this repo's
            // budgeting; exact enough for a regression tripwire.
            let card = format!(
                "family: {}\nbytes: {}\napprox_tokens: {}\n---\n{}",
                family.as_str(),
                bytes,
                bytes.div_ceil(4),
                overlay
            );
            assert_snapshot!(format!("overlay_{}", family.as_str()), card);
        }
    }
}
