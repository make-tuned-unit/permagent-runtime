use super::api_client::{ApiClient, AuthMethod};
use super::base::{ConfigKey, ProviderDef, ProviderMetadata};
use super::openai_compatible::OpenAiCompatibleProvider;
use crate::model::ModelConfig;
use anyhow::Result;
use futures::future::BoxFuture;

const ZAI_PROVIDER_NAME: &str = "zai";

/// Z.AI (Zhipu AI) OpenAI-compatible chat completions base URL.
/// Source: https://docs.z.ai/api-reference/llm/chat-completion (fetched 2026-08-24)
pub const ZAI_API_HOST: &str = "https://api.z.ai/api/paas/v4";

/// The GLM Coding Plan's OpenAI-protocol base URL. The Coding Plan is a
/// flat-rate subscription rather than per-token billing, so for heavy coding use
/// it is far cheaper than the pay-as-you-go rates above — set `ZAI_HOST` to this
/// to spend against the plan instead of the API balance. Same model ids either
/// way, which is why it needs no separate provider.
/// Source: https://docs.z.ai/devpack/quick-start (fetched 2026-08-24)
pub const ZAI_CODING_PLAN_HOST: &str = "https://api.z.ai/api/coding/paas/v4";

/// Z.AI also publishes an Anthropic-protocol base URL for the GLM Coding Plan
/// (https://api.z.ai/api/anthropic). We speak the OpenAI-compatible protocol
/// here, so this is recorded for reference only — it is NOT a valid `ZAI_HOST`
/// for this provider, which would send OpenAI-shaped requests to it.
/// Source: https://docs.z.ai/devpack/quick-start (fetched 2026-08-24)
pub const ZAI_ANTHROPIC_HOST: &str = "https://api.z.ai/api/anthropic";

/// Best-fit, cost-conscious default: GLM-4.7 is Z.AI's strongest cheap coding
/// model ($0.60/$2.20 per 1M tokens) rather than the pricier GLM-5.x flagships.
pub const ZAI_DEFAULT_MODEL: &str = "glm-4.7";

/// Model ids taken verbatim from the Z.AI docs model pages, not guessed.
/// Source: https://docs.z.ai/api-reference/llm/chat-completion and the
/// per-model guides under https://docs.z.ai/guides/ (fetched 2026-08-24).
pub const ZAI_KNOWN_MODELS: &[&str] = &[
    // GLM-5 flagship line
    "glm-5.3",
    "glm-5.2",
    "glm-5.1",
    "glm-5",
    "glm-5-turbo",
    // GLM-4.x line
    "glm-4.7",
    "glm-4.7-flashx",
    "glm-4.7-flash",
    "glm-4.6",
    "glm-4.5",
    "glm-4.5-x",
    "glm-4.5-air",
    "glm-4.5-airx",
    "glm-4.5-flash",
    "glm-4-32b-0414-128k",
    // Vision / multimodal line
    "glm-5v-turbo",
    "glm-4.6v",
    "glm-4.6v-flashx",
    "glm-4.6v-flash",
    "glm-4.5v",
];

pub const ZAI_DOC_URL: &str = "https://docs.z.ai/guides/overview/pricing";

// Z.AI reports cached prompt tokens as `usage.prompt_tokens_details.cached_tokens`
// — the OpenAI shape, a subset of `prompt_tokens` rather than an addition to it
// (<https://docs.z.ai/api-reference/llm/chat-completion>). The shared parser in
// `formats/openai.rs::get_usage` reads that field, so cache hits are visible in
// the ledger. It changes no total: `canonical::cost` only carves cache-read
// tokens out of the input bucket when the model publishes its own cache-read
// rate, and leaves them billed as plain input otherwise.

pub struct ZaiProvider;

impl ProviderDef for ZaiProvider {
    type Provider = OpenAiCompatibleProvider;

    fn metadata() -> ProviderMetadata {
        ProviderMetadata::new(
            ZAI_PROVIDER_NAME,
            "Z.AI",
            "GLM models from Z.AI (Zhipu AI), including the GLM-5 and GLM-4.x coding and vision families",
            ZAI_DEFAULT_MODEL,
            ZAI_KNOWN_MODELS.to_vec(),
            ZAI_DOC_URL,
            vec![
                ConfigKey::new("ZAI_API_KEY", true, true, None, true),
                ConfigKey::new("ZAI_HOST", false, false, Some(ZAI_API_HOST), false),
            ],
        )
    }

    fn from_env(
        model: ModelConfig,
        _extensions: Vec<crate::config::ExtensionConfig>,
    ) -> BoxFuture<'static, Result<OpenAiCompatibleProvider>> {
        Box::pin(async move {
            let config = crate::config::Config::global();
            let api_key: String = config.get_secret("ZAI_API_KEY")?;
            let host: String = config
                .get_param("ZAI_HOST")
                .unwrap_or_else(|_| ZAI_API_HOST.to_string());

            let api_client = ApiClient::new(host, AuthMethod::BearerToken(api_key))?;

            Ok(OpenAiCompatibleProvider::new(
                ZAI_PROVIDER_NAME.to_string(),
                api_client,
                model,
                String::new(),
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::{Message, MessageContent};
    use crate::providers::base::{Provider, Usage};
    use crate::providers::canonical::{cost_of, maybe_get_canonical_model};
    use futures::StreamExt;
    use rmcp::model::Tool;
    use rmcp::object;
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ── Metadata ─────────────────────────────────────────────────────────────

    #[test]
    fn metadata_exposes_the_documented_key_host_and_default() {
        let meta = ZaiProvider::metadata();
        assert_eq!(meta.name, "zai");
        assert_eq!(meta.display_name, "Z.AI");
        assert_eq!(meta.default_model, "glm-4.7");

        let key = meta
            .config_keys
            .iter()
            .find(|k| k.name == "ZAI_API_KEY")
            .expect("ZAI_API_KEY must be a config key");
        assert!(key.required, "the API key is required");
        assert!(key.secret, "the API key must be stored as a secret");
        assert!(key.primary, "the API key is the primary field in Settings");

        let host = meta
            .config_keys
            .iter()
            .find(|k| k.name == "ZAI_HOST")
            .expect("ZAI_HOST must be an overridable config key");
        assert!(!host.required);
        assert!(!host.secret);
        assert_eq!(
            host.default.as_deref(),
            Some("https://api.z.ai/api/paas/v4"),
            "default host must be the documented OpenAI-compatible base URL"
        );
    }

    /// The Coding Plan is reachable by pointing ZAI_HOST at it — same protocol,
    /// same model ids, flat-rate billing instead of per-token.
    #[test]
    fn coding_plan_host_is_the_openai_protocol_one() {
        assert_eq!(ZAI_CODING_PLAN_HOST, "https://api.z.ai/api/coding/paas/v4");
        assert!(
            ZAI_CODING_PLAN_HOST.starts_with("https://api.z.ai/"),
            "the Coding Plan must stay on the Z.AI host"
        );
        // The Anthropic-protocol URL must never be mistaken for a ZAI_HOST value.
        assert_ne!(ZAI_ANTHROPIC_HOST, ZAI_CODING_PLAN_HOST);
    }

    #[test]
    fn model_list_carries_the_current_glm_families() {
        for expected in [
            "glm-5.3",
            "glm-5.2",
            "glm-5",
            "glm-4.7",
            "glm-4.6",
            "glm-4.5-air",
            "glm-4.6v",
        ] {
            assert!(
                ZAI_KNOWN_MODELS.contains(&expected),
                "{expected} missing from ZAI_KNOWN_MODELS"
            );
        }
        assert!(
            ZAI_KNOWN_MODELS.contains(&ZAI_DEFAULT_MODEL),
            "the default model must be offered in the picker"
        );
    }

    // ── Canonical coverage: limits + pricing ─────────────────────────────────

    /// Every id we advertise must resolve in the canonical registry. Without a
    /// row a model gets no context limit and `cost_of` returns `None`, so the
    /// spend ledger would record the turn as unknown — this test is what catches
    /// a typo'd or retired model id before it reaches the picker.
    #[test]
    fn every_offered_model_has_canonical_limits_and_pricing() {
        for model in ZAI_KNOWN_MODELS {
            let canonical = maybe_get_canonical_model("zai", model)
                .unwrap_or_else(|| panic!("no canonical row for zai/{model}"));
            assert!(
                canonical.limit.context > 0,
                "zai/{model} has no context limit"
            );
            assert!(
                canonical.cost.input.is_some() && canonical.cost.output.is_some(),
                "zai/{model} has no input/output price, so its cost is unknowable"
            );
        }
    }

    /// Prices are the Z.AI published list, per million tokens.
    /// Source: https://docs.z.ai/guides/overview/pricing (fetched 2026-08-24).
    #[test]
    fn published_prices_and_limits_match_the_zai_list() {
        let cases = [
            // (model, input $/Mtok, output $/Mtok, context window)
            ("glm-5.3", 1.4, 4.4, 1_000_000),
            ("glm-5.2", 1.4, 4.4, 1_000_000),
            ("glm-5", 1.0, 3.2, 204_800),
            ("glm-4.7", 0.6, 2.2, 204_800),
            ("glm-4.7-flashx", 0.07, 0.4, 200_000),
            ("glm-4.5-air", 0.2, 1.1, 131_072),
            ("glm-4.5-x", 2.2, 8.9, 131_072),
            ("glm-4.6v", 0.3, 0.9, 128_000),
        ];
        for (model, input, output, context) in cases {
            let c = maybe_get_canonical_model("zai", model)
                .unwrap_or_else(|| panic!("no canonical row for zai/{model}"));
            assert_eq!(c.cost.input, Some(input), "{model} input price");
            assert_eq!(c.cost.output, Some(output), "{model} output price");
            assert_eq!(c.limit.context, context, "{model} context window");
        }
    }

    /// The Flash tiers are genuinely $0 on Z.AI's list — priced-at-zero, not
    /// unpriced. The distinction matters: `cost_of` returns `Some(0.0)` here,
    /// which the ledger records as a real zero, whereas a missing price returns
    /// `None` ("unknowable") and is surfaced as an estimate instead.
    #[test]
    fn free_flash_tiers_are_priced_at_zero_not_unpriced() {
        for model in ["glm-4.7-flash", "glm-4.5-flash", "glm-4.6v-flash"] {
            let c = maybe_get_canonical_model("zai", model)
                .unwrap_or_else(|| panic!("no canonical row for zai/{model}"));
            assert_eq!(c.cost.input, Some(0.0), "{model} input");
            assert_eq!(c.cost.output, Some(0.0), "{model} output");
        }
    }

    #[test]
    fn cost_of_a_glm_4_7_turn_uses_the_published_rate() {
        let c = maybe_get_canonical_model("zai", "glm-4.7").expect("glm-4.7 canonical row");
        let usage = Usage {
            input_tokens: Some(1_000_000),
            output_tokens: Some(1_000_000),
            total_tokens: Some(2_000_000),
            cache_read_input_tokens: None,
            cache_write_input_tokens: None,
        };
        let cost = cost_of(&usage, &c.cost).expect("glm-4.7 must be priced");
        // 1M in @ $0.60 + 1M out @ $2.20
        assert!(
            (cost - 2.80).abs() < 1e-9,
            "expected $2.80 for 1M+1M, got {cost}"
        );
    }

    // ── Wire round-trip against a recorded Z.AI response ─────────────────────

    fn weather_tool() -> Tool {
        Tool::new(
            "get_weather",
            "Get the weather for a city",
            object!({
                "type": "object",
                "required": ["city"],
                "properties": { "city": { "type": "string" } }
            }),
        )
    }

    /// Streams a recorded Z.AI SSE response (no network). Asserts three things at
    /// once: the request lands on the documented completions path with a bearer
    /// token, the tool call round-trips out of the stream intact, and the usage
    /// block becomes token counts the spend gate can read.
    ///
    /// The SSE body below is the shape documented at
    /// https://docs.z.ai/api-reference/llm/chat-completion (fetched 2026-08-24).
    #[tokio::test]
    async fn streaming_tool_call_and_usage_round_trip() {
        let server = MockServer::start().await;

        let sse = concat!(
            "data: {\"id\":\"2026082400\",\"created\":1756000000,\"model\":\"glm-4.7\",",
            "\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Checking.\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"2026082400\",\"created\":1756000000,\"model\":\"glm-4.7\",",
            "\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_zai_1\",\"type\":\"function\",",
            "\"function\":{\"name\":\"get_weather\",\"arguments\":\"{\\\"city\\\":\\\"Shanghai\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"2026082400\",\"created\":1756000000,\"model\":\"glm-4.7\",",
            "\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}],",
            "\"usage\":{\"prompt_tokens\":1200,\"completion_tokens\":64,\"total_tokens\":1264}}\n\n",
            "data: [DONE]\n\n",
        );

        Mock::given(method("POST"))
            // The documented endpoint is <host>/chat/completions where the host
            // already carries /api/paas/v4 — this asserts the URL join is right.
            .and(path("/api/paas/v4/chat/completions"))
            .and(header("authorization", "Bearer test-zai-key"))
            .and(body_string_contains("\"model\":\"glm-4.7\""))
            .and(body_string_contains("\"stream\":true"))
            .and(body_string_contains("get_weather"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse),
            )
            .expect(1)
            .mount(&server)
            .await;

        let host = format!("{}/api/paas/v4", server.uri());
        let api_client = ApiClient::new(host, AuthMethod::BearerToken("test-zai-key".to_string()))
            .expect("api client");
        let model = ModelConfig::new(ZAI_DEFAULT_MODEL)
            .expect("model config")
            .with_canonical_limits("zai");
        let provider = OpenAiCompatibleProvider::new(
            ZAI_PROVIDER_NAME.to_string(),
            api_client,
            model.clone(),
            String::new(),
        );

        let messages = vec![Message::user().with_text("Weather in Shanghai?")];
        let mut stream = provider
            .stream(
                &model,
                "zai-test-session",
                "You are terse.",
                &messages,
                &[weather_tool()],
            )
            .await
            .expect("stream should start");

        let mut text = String::new();
        let mut tool_names = Vec::new();
        let mut final_usage = None;
        while let Some(chunk) = stream.next().await {
            let (message, usage) = chunk.expect("chunk should parse");
            if let Some(msg) = message {
                for content in &msg.content {
                    match content {
                        MessageContent::Text(t) => text.push_str(&t.text),
                        MessageContent::ToolRequest(req) => {
                            if let Ok(call) = &req.tool_call {
                                tool_names.push(call.name.to_string());
                            }
                        }
                        _ => {}
                    }
                }
            }
            if let Some(u) = usage {
                final_usage = Some(u);
            }
        }

        assert!(
            text.contains("Checking."),
            "assistant text should stream out"
        );
        assert_eq!(
            tool_names,
            vec!["get_weather".to_string()],
            "the tool call must survive the round-trip"
        );

        let usage = final_usage
            .expect("Z.AI reports usage in the final chunk")
            .usage;
        assert_eq!(usage.input_tokens, Some(1200));
        assert_eq!(usage.output_tokens, Some(64));
        assert_eq!(usage.total_tokens, Some(1264));

        // And that usage is what the spend gate prices.
        let canonical = maybe_get_canonical_model("zai", "glm-4.7").expect("canonical row");
        let cost = cost_of(&usage, &canonical.cost).expect("priced");
        assert!(cost > 0.0, "a billed Z.AI turn must not cost $0");
    }

    #[tokio::test]
    async fn http_error_is_surfaced_not_swallowed() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/paas/v4/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": {"code": "401", "message": "Invalid API key"}
            })))
            .mount(&server)
            .await;

        let host = format!("{}/api/paas/v4", server.uri());
        let api_client =
            ApiClient::new(host, AuthMethod::BearerToken("bad".to_string())).expect("api client");
        let model = ModelConfig::new(ZAI_DEFAULT_MODEL).expect("model config");
        let provider = OpenAiCompatibleProvider::new(
            ZAI_PROVIDER_NAME.to_string(),
            api_client,
            model.clone(),
            String::new(),
        );

        // `expect_err` is unavailable here: MessageStream is not Debug.
        let err = match provider
            .stream(
                &model,
                "zai-test-session",
                "sys",
                &[Message::user().with_text("hi")],
                &[],
            )
            .await
        {
            Ok(_) => panic!("a 401 must surface as an error, not a stream"),
            Err(e) => e,
        };
        assert!(
            matches!(
                err,
                crate::providers::errors::ProviderError::Authentication(_)
            ),
            "expected an authentication error, got {err:?}"
        );
    }
}
