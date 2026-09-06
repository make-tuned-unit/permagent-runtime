//! Apple on-device Foundation Models — inference that runs on this Mac.
//!
//! # Why this exists
//!
//! Bulk, low-complexity work was going to a cloud API and being billed per
//! call. Over the measured period the ledger showed 1,635 cloud calls costing
//! $43.51 against 15 local calls costing nothing, with 1,197 of the cloud calls
//! ($36.96 of the total) spent on a small fast model doing high-volume,
//! low-complexity jobs — the shape of work an on-device model can do. Every
//! call this provider serves is one that is not billed and one whose prompt
//! never leaves the machine.
//!
//! # What "local" means here, precisely
//!
//! **Inference is local.** [`DataLocality::Local`] is claimed for that reason
//! and no other: the prompt and the response stay on this Mac.
//!
//! **Provisioning is not.** The model weights are downloaded by macOS, not by
//! this code and not at our request. Until the OS has finished, availability
//! reports [`UnavailableReason::ModelNotReady`]. Any user-facing copy must say
//! that inference runs on this machine, not that nothing was ever downloaded.
//!
//! **Apple commits to no egress of the prompt; that is not the same as "Apple
//! logs nothing."** Apple does not make a no-local-telemetry commitment, so
//! this codebase must not make one on its behalf. What is verifiable and what
//! may be claimed: inference happens on-device and the prompt is not sent to a
//! server.
//!
//! `PrivateCloudComputeLanguageModel` is a *cloud* backend. It is deliberately
//! not wired up here. If it ever is, it is [`DataLocality::Cloud`] and must
//! never be described as local, however strong its privacy properties are.
//!
//! # Licence constraint — ADPLA §3.2(h)(2)
//!
//! The Apple Developer Program Licence Agreement, §3.2(h)(2), forbids using
//! output from Apple's model to train or improve any other model.
//!
//! This is written down here because it constrains a use that is otherwise
//! obvious and tempting: the Librarian's descriptions are a large, clean,
//! domain-specific corpus, and this provider will produce a growing share of
//! them. **They must not be used as training or fine-tuning data for another
//! model.** The per-call provenance logging exists partly so that this is
//! enforceable — a later corpus build can tell which descriptions came from
//! Apple's model and must exclude them, rather than having to assume the worst
//! about all of them.
//!
//! # Availability is a runtime property, not a build-time one
//!
//! Nothing here assumes the framework, the hardware, the user's settings, or
//! the assets. Availability is probed against the running system, and a probe
//! that said "available" a minute ago is not evidence about now.
//!
//! Availability alone is also not sufficient. Observed on 2026-08-19 on a
//! memory-pressured machine: `SystemLanguageModel.availability` reported
//! `.available` while the safety subsystem could not load its assets, so every
//! generation failed. Callers must therefore fall back on *generation* errors
//! as well as on unavailability — which is why [`AppleFmError`] separates the
//! two but treats both as ordinary, loggable outcomes.
//!
//! **Falling back is a first-class path, never an error path.** A user with
//! Apple Intelligence switched off must see exactly the behaviour they saw
//! before this provider existed, with the reason recorded in the log and
//! nothing surfaced as a failure.

mod sidecar;

pub use sidecar::{
    availability, context_size, generate, last_context_size, sidecar_path, AppleFmError,
    Availability, UnavailableReason,
};

use anyhow::Result;
use async_trait::async_trait;
use futures::future::BoxFuture;
use rmcp::model::Tool;

use crate::config::ExtensionConfig;
use crate::conversation::message::Message;
use crate::model::ModelConfig;
use crate::providers::base::{
    ConfigKey, MessageStream, Provider, ProviderDef, ProviderMetadata, ProviderUsage, Usage,
};
use crate::providers::errors::ProviderError;
use crate::sovereignty::DataLocality;

/// Registry name. Also the value that appears in provenance logs, so a later
/// measurement can separate this engine's output from Ollama's and the cloud's.
pub const PROVIDER_NAME: &str = "apple_foundation_models";

/// There is exactly one on-device system model; it has no version string the
/// framework will tell us, so this is a label rather than a selector.
pub const DEFAULT_MODEL: &str = "apple-on-device";

/// Cap on generated tokens when the caller does not specify one.
const DEFAULT_MAX_TOKENS: u32 = 1024;

/// Matches the temperature the Librarian's other backends use, so a change of
/// engine is not silently also a change of sampling.
const DEFAULT_TEMPERATURE: f32 = 0.2;

pub struct AppleFoundationModelsProvider {
    model_config: ModelConfig,
}

impl AppleFoundationModelsProvider {
    pub async fn from_env(model: ModelConfig, _extensions: Vec<ExtensionConfig>) -> Result<Self> {
        // Deliberately does not fail when the model is unavailable. Construction
        // succeeding says "this provider exists"; whether it can serve a given
        // call is decided per call, against the system as it is then.
        Ok(Self {
            model_config: model,
        })
    }

    /// The context window of the running model, probed now.
    ///
    /// Returns `None` when the model is unavailable — there is no window to
    /// report and no default worth inventing.
    pub async fn probed_context_limit(&self) -> Option<usize> {
        context_size().await
    }
}

impl ProviderDef for AppleFoundationModelsProvider {
    type Provider = Self;

    fn metadata() -> ProviderMetadata {
        ProviderMetadata::new(
            PROVIDER_NAME,
            "Apple Foundation Models",
            "Apple's on-device model. Inference runs on this Mac and prompts are not sent to a \
             server; the model assets themselves are provisioned by macOS. Requires Apple \
             Intelligence to be enabled.",
            DEFAULT_MODEL,
            vec![DEFAULT_MODEL],
            "https://developer.apple.com/documentation/foundationmodels",
            vec![ConfigKey::new(
                "PERMAGENT_APPLE_FM_ENABLED",
                false,
                false,
                Some("true"),
                false,
            )],
        )
    }

    fn from_env(
        model: ModelConfig,
        extensions: Vec<ExtensionConfig>,
    ) -> BoxFuture<'static, Result<Self::Provider>> {
        Box::pin(Self::from_env(model, extensions))
    }
}

#[async_trait]
impl Provider for AppleFoundationModelsProvider {
    fn get_name(&self) -> &str {
        PROVIDER_NAME
    }

    fn cost_tier(&self) -> crate::session::CostTier {
        crate::session::CostTier::LocalFree
    }

    fn get_model_config(&self) -> ModelConfig {
        self.model_config.clone()
    }

    /// On-device inference. The prompt and the response never leave this Mac.
    ///
    /// Note the trait default is the fail-closed [`DataLocality::Cloud`]; this
    /// override is a positive claim about where the computation happens, and it
    /// is true of inference only — the weights are provisioned by macOS.
    fn data_locality(&self) -> DataLocality {
        DataLocality::Local
    }

    async fn fetch_supported_models(&self) -> Result<Vec<String>, ProviderError> {
        // Only offer the model when the system can actually serve it, so a
        // machine with Apple Intelligence off shows nothing to select rather
        // than an option that cannot work.
        Ok(match availability().await {
            Availability::Available { .. } => vec![DEFAULT_MODEL.to_string()],
            Availability::Unavailable(_) => vec![],
        })
    }

    fn skip_canonical_filtering(&self) -> bool {
        // Not in the canonical cross-provider registry: there is no public
        // pricing or capability row for the on-device model.
        true
    }

    async fn stream(
        &self,
        model_config: &ModelConfig,
        _session_id: &str,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        if !tools.is_empty() {
            // FoundationModels has its own `Tool` protocol, which is a Swift
            // conformance rather than a JSON schema, so the tool surface cannot
            // be bridged by passing definitions across this wire. Refusing is
            // honest; pretending to accept tools and ignoring them is not.
            return Err(ProviderError::NotImplemented(
                "the on-device model does not accept this provider's tool definitions".to_string(),
            ));
        }

        let prompt = flatten_conversation(messages);
        if prompt.trim().is_empty() {
            return Err(ProviderError::ExecutionError(
                "no user content to send to the on-device model".to_string(),
            ));
        }

        let max_tokens = model_config.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS as i32);
        let temperature = model_config.temperature.unwrap_or(DEFAULT_TEMPERATURE);

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let system = system.to_string();
        let handle = tokio::spawn(async move {
            generate(
                &system,
                &prompt,
                max_tokens.max(1) as u32,
                temperature,
                |d| {
                    let _ = tx.send(d.to_string());
                },
            )
            .await
        });

        let model_name = model_config.model_name.clone();
        Ok(Box::pin(async_stream::try_stream! {
            while let Some(delta) = rx.recv().await {
                yield (Some(Message::assistant().with_text(delta)), None);
            }

            let outcome = handle
                .await
                .map_err(|e| ProviderError::ExecutionError(format!("on-device task failed: {}", e)))?;

            match outcome {
                Ok(_) => {
                    yield (None, Some(ProviderUsage::new(model_name.clone(), Usage::default())));
                }
                Err(err) => {
                    // Surfaced as an error here because a `Provider` has no
                    // other channel. Callers that can fall back — the Librarian
                    // does — should use `apple_fm::generate` directly, where
                    // unavailability is a typed outcome rather than a failure.
                    Err(map_error(err))?;
                }
            }
        }))
    }
}

/// Flatten a conversation into a single prompt.
///
/// The on-device session carries its own transcript across turns, but this
/// provider creates a fresh session per call so that independent jobs cannot
/// contaminate each other. The history therefore has to travel in the prompt.
fn flatten_conversation(messages: &[Message]) -> String {
    use crate::conversation::message::MessageContent;
    use rmcp::model::Role;

    let mut out = String::new();
    for message in messages {
        let mut text = String::new();
        for content in &message.content {
            if let MessageContent::Text(t) = content {
                text.push_str(&t.text);
            }
        }
        if text.trim().is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        match message.role {
            Role::User => out.push_str("User: "),
            Role::Assistant => out.push_str("Assistant: "),
        }
        out.push_str(text.trim());
    }
    out
}

fn map_error(err: AppleFmError) -> ProviderError {
    match err {
        AppleFmError::Generation { ref kind, .. } if kind == "context_window_exceeded" => {
            ProviderError::ContextLengthExceeded(err.to_string())
        }
        other => ProviderError::ExecutionError(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> AppleFoundationModelsProvider {
        AppleFoundationModelsProvider {
            model_config: ModelConfig::new_or_fail(DEFAULT_MODEL),
        }
    }

    #[test]
    fn inference_on_this_machine_is_reported_as_local() {
        // The trait default is the fail-closed `Cloud`. This override is the
        // whole sovereignty claim of the provider, so it is asserted directly
        // rather than left to be inferred.
        assert_eq!(provider().data_locality(), DataLocality::Local);
    }

    #[test]
    fn the_provider_is_named_distinctly_enough_to_attribute_output_to_it() {
        // Provenance depends on this string being unlike the other engines'.
        assert_eq!(provider().get_name(), "apple_foundation_models");
        assert_ne!(provider().get_name(), "ollama");
        assert_ne!(provider().get_name(), "local");
    }

    #[test]
    fn a_conversation_is_flattened_with_speaker_labels_and_blank_turns_dropped() {
        let messages = vec![
            Message::user().with_text("first"),
            Message::assistant().with_text("   "),
            Message::assistant().with_text("second"),
        ];
        assert_eq!(
            flatten_conversation(&messages),
            "User: first\n\nAssistant: second"
        );
    }

    #[test]
    fn an_over_long_prompt_is_reported_as_a_context_error_not_a_generic_one() {
        let mapped = map_error(AppleFmError::Generation {
            kind: "context_window_exceeded".into(),
            message: "too long".into(),
        });
        assert!(matches!(mapped, ProviderError::ContextLengthExceeded(_)));
    }

    /// The context window must come from the running model.
    ///
    /// Asserts the *relationship* — whatever the probe reports is what gets
    /// used and cached — rather than a literal, deliberately. The literal
    /// changes with the OS: it is 4096 on macOS 26.2, where `contextSize` is
    /// back-deployed and its shim returns that, and read from the installed
    /// model on 26.4 and later. There is no context-window constant anywhere in
    /// this module for a test to compare against, which is the point.
    #[tokio::test]
    async fn the_context_window_is_whatever_the_running_model_reports() {
        let probed = context_size().await;
        match availability().await {
            Availability::Available { context_size } => {
                assert_eq!(probed, Some(context_size));
                assert_eq!(last_context_size(), Some(context_size));
                assert!(context_size > 0);
            }
            Availability::Unavailable(reason) => {
                // No model, therefore no window to report. Reporting a default
                // here would be inventing a number.
                assert_eq!(probed, None, "unavailable must not yield a context size");
                assert!(!reason.as_str().is_empty(), "a reason is always loggable");
            }
        }
    }

    /// Unavailability must be an ordinary, described outcome — never a panic,
    /// never an untyped failure. This runs on every platform: off macOS it
    /// exercises the `unsupported_platform` short-circuit, which is exactly the
    /// non-macOS no-op path.
    #[tokio::test]
    async fn unavailability_is_always_reported_with_a_reason_the_caller_can_log() {
        let availability = availability().await;
        assert!(!availability.reason().is_empty());
        if !cfg!(target_os = "macos") {
            assert_eq!(availability.reason(), "unsupported_platform");
        }
    }

    /// A real round trip through the on-device model. Ignored by default:
    /// CI has no Apple Intelligence, and a machine that does may still have the
    /// assets unprovisioned. Run with
    /// `cargo test -p permagent --lib apple_fm -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "requires macOS 26+, Apple Intelligence enabled, and the built sidecar"]
    async fn a_real_on_device_round_trip_returns_text() {
        let availability = availability().await;
        let Availability::Available { context_size } = availability else {
            panic!("on-device model unavailable: {}", availability.reason());
        };
        println!("context_size (probed, not hardcoded) = {}", context_size);

        let started = std::time::Instant::now();
        let mut streamed = String::new();
        let text = generate(
            "You write one short factual sentence. No preamble.",
            "Summarise: the scheduling service moved to a new queue backend on Tuesday.",
            150,
            0.2,
            |delta| streamed.push_str(delta),
        )
        .await
        .expect("on-device generation");

        println!("latency = {}ms", started.elapsed().as_millis());
        println!("response = {}", text);
        assert!(!text.trim().is_empty());
        // The deltas must reconstruct the final text, or streaming consumers
        // see something different from what non-streaming ones do.
        assert_eq!(streamed, text);
    }
}
