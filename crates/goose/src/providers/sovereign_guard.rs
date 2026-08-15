//! The sovereignty enforcement choke point.
//!
//! [`SovereignGuardProvider`] is a decorator that wraps **every** provider
//! minted by the factory (`crate::providers::create*` →
//! `ProviderEntry::create` / `create_with_default_model`). Because every
//! `Arc<dyn Provider>` in the daemon is produced there, wrapping at that single
//! point makes this the one place all inference egress passes through.
//!
//! It intercepts the two trait methods that actually send content off-provider:
//! [`Provider::stream`] and [`Provider::create_embeddings`]. The higher-level
//! `complete`, `complete_fast`, and `generate_session_name` are deliberately
//! **not** overridden: their trait defaults funnel into `self.stream` (verified
//! — no provider overrides them), so they are guarded transitively without
//! extra code. Every other trait method is faithfully delegated to the inner
//! provider so wrapping is behaviorally transparent.
//!
//! Enforcement is **fail-closed**: for a [`DataLocality::Cloud`] provider used
//! in a sovereign context, the call is refused before any bytes leave, and the
//! attempt is recorded in the egress audit log. A cloud call in a non-sovereign
//! context is allowed but still recorded — the audit is complete. If the audit
//! row itself cannot be written, an allowed cloud call is refused too — no
//! unlogged egress.

use std::sync::Arc;

use async_trait::async_trait;
use rmcp::model::Tool;

use super::base::{MessageStream, PermissionRouting, Provider};
use super::errors::ProviderError;
use super::retry::RetryConfig;
use crate::config::GooseMode;
use crate::conversation::message::Message;
use crate::cost_router::cache::SystemPromptParts;
use crate::model::ModelConfig;
use crate::permission::PermissionConfirmation;
use crate::sovereignty::{self, DataLocality, EgressKind, EgressRecord};

/// Provider decorator that enforces the sovereignty data boundary.
pub struct SovereignGuardProvider {
    inner: Arc<dyn Provider>,
    /// The inner provider's data locality, resolved once at wrap time.
    locality: DataLocality,
    #[cfg(test)]
    audit_result: Option<Result<(), String>>,
}

impl SovereignGuardProvider {
    /// Wrap a freshly-minted provider so every use passes through the boundary.
    /// Called at the single factory choke point.
    pub fn wrap(inner: Arc<dyn Provider>) -> Arc<dyn Provider> {
        let locality = inner.data_locality();
        Arc::new(Self {
            inner,
            locality,
            #[cfg(test)]
            audit_result: None,
        })
    }

    #[cfg(test)]
    fn wrap_with_audit_result(
        inner: Arc<dyn Provider>,
        audit_result: Result<(), String>,
    ) -> Arc<dyn Provider> {
        let locality = inner.data_locality();
        Arc::new(Self {
            inner,
            locality,
            audit_result: Some(audit_result),
        })
    }

    async fn record_egress(&self, record: EgressRecord) -> anyhow::Result<()> {
        #[cfg(test)]
        if let Some(result) = &self.audit_result {
            return result.clone().map_err(anyhow::Error::msg);
        }
        sovereignty::record_egress(record).await
    }

    /// The shared cloud-egress gate. For a **cloud** provider it records the
    /// egress event (blocked or allowed) and returns `Err` when the context is
    /// sovereign — the caller must propagate that error WITHOUT delegating to
    /// the inner provider, so no bytes leave. For a **local** provider it is a
    /// no-op (`Ok`), since nothing leaves the machine.
    async fn gate(
        &self,
        session_id: &str,
        model: &str,
        kind: EgressKind,
        content_hash: String,
        prompt: Option<String>,
    ) -> Result<(), ProviderError> {
        if self.locality != DataLocality::Cloud {
            return Ok(());
        }
        let blocked = sovereignty::is_context_sovereign(session_id);
        // Always-on audit: record every cloud call (blocked or allowed) before
        // it is allowed to proceed. An unlogged cloud call is a lying audit.
        let audit = self
            .record_egress(EgressRecord {
                provider: self.inner.get_name().to_string(),
                model: model.to_string(),
                session_id: session_id.to_string(),
                project_id: None,
                kind,
                blocked,
                content_hash,
                prompt,
            })
            .await;
        if blocked {
            // Unchanged fail-closed refusal — an audit failure cannot make a
            // sovereign context leak, and `record_egress` has already logged
            // the failure loudly with full context.
            return Err(ProviderError::ExecutionError(
                sovereignty::sovereign_block_message(self.inner.get_name(), model),
            ));
        }
        // Audit persistence is a non-disableable precondition for ALLOWED cloud
        // egress. Refuse before delegating rather than permit an unlogged send.
        if audit.is_err() {
            return Err(ProviderError::ExecutionError(
                sovereignty::audit_block_message(self.inner.get_name(), model),
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl Provider for SovereignGuardProvider {
    fn get_name(&self) -> &str {
        self.inner.get_name()
    }

    async fn stream(
        &self,
        model_config: &ModelConfig,
        session_id: &str,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        if self.locality == DataLocality::Cloud {
            let prompt = if sovereignty::capture_prompts_enabled() {
                Some(sovereignty::render_prompt(system, messages))
            } else {
                None
            };
            let content_hash = sovereignty::hash_prompt(system, messages);
            self.gate(
                session_id,
                &model_config.model_name,
                EgressKind::Inference,
                content_hash,
                prompt,
            )
            .await?;
        }
        self.inner
            .stream(model_config, session_id, system, messages, tools)
            .await
    }

    /// Forwards the split to the wrapped provider. Without this override the
    /// default would flatten the prompt before it ever reached the inner
    /// provider, so wrapping Anthropic in the sovereignty guard would silently
    /// cost every session its prompt cache — a decorator quietly disabling a
    /// property of the thing it decorates.
    ///
    /// The egress gate keys on the FULL prompt (`render()`), not the prefix:
    /// what crosses the wire is the whole thing, and hashing only the cached
    /// half would under-report what left the device.
    async fn stream_split(
        &self,
        model_config: &ModelConfig,
        session_id: &str,
        system: &SystemPromptParts,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        if self.locality == DataLocality::Cloud {
            let rendered = system.render();
            let prompt = if sovereignty::capture_prompts_enabled() {
                Some(sovereignty::render_prompt(&rendered, messages))
            } else {
                None
            };
            let content_hash = sovereignty::hash_prompt(&rendered, messages);
            self.gate(
                session_id,
                &model_config.model_name,
                EgressKind::Inference,
                content_hash,
                prompt,
            )
            .await?;
        }
        self.inner
            .stream_split(model_config, session_id, system, messages, tools)
            .await
    }

    async fn create_embeddings(
        &self,
        session_id: &str,
        texts: Vec<String>,
    ) -> Result<Vec<Vec<f32>>, ProviderError> {
        if self.locality == DataLocality::Cloud {
            let content_hash = sovereignty::hash_texts(&texts);
            self.gate(
                session_id,
                &self.inner.get_model_config().model_name,
                EgressKind::Embedding,
                content_hash,
                None,
            )
            .await?;
        }
        self.inner.create_embeddings(session_id, texts).await
    }

    // ── Faithful delegation of everything else ──────────────────────────────
    // (`complete` / `complete_fast` / `generate_session_name` are intentionally
    // left as trait defaults so they funnel through the guarded `stream`.)

    fn get_model_config(&self) -> ModelConfig {
        self.inner.get_model_config()
    }

    fn data_locality(&self) -> DataLocality {
        self.locality
    }

    fn retry_config(&self) -> RetryConfig {
        self.inner.retry_config()
    }

    async fn fetch_supported_models(&self) -> Result<Vec<String>, ProviderError> {
        self.inner.fetch_supported_models().await
    }

    fn skip_canonical_filtering(&self) -> bool {
        self.inner.skip_canonical_filtering()
    }

    async fn fetch_recommended_models(&self) -> Result<Vec<String>, ProviderError> {
        self.inner.fetch_recommended_models().await
    }

    async fn map_to_canonical_model(
        &self,
        provider_model: &str,
    ) -> Result<Option<String>, ProviderError> {
        self.inner.map_to_canonical_model(provider_model).await
    }

    fn supports_embeddings(&self) -> bool {
        self.inner.supports_embeddings()
    }

    fn manages_own_context(&self) -> bool {
        self.inner.manages_own_context()
    }

    async fn supports_cache_control(&self) -> bool {
        self.inner.supports_cache_control().await
    }

    async fn configure_oauth(&self) -> Result<(), ProviderError> {
        self.inner.configure_oauth().await
    }

    async fn refresh_credentials(&self) -> Result<(), ProviderError> {
        self.inner.refresh_credentials().await
    }

    async fn update_mode(&self, session_id: &str, mode: GooseMode) -> Result<(), ProviderError> {
        self.inner.update_mode(session_id, mode).await
    }

    fn permission_routing(&self) -> PermissionRouting {
        self.inner.permission_routing()
    }

    async fn handle_permission_confirmation(
        &self,
        request_id: &str,
        confirmation: &PermissionConfirmation,
    ) -> bool {
        self.inner
            .handle_permission_confirmation(request_id, confirmation)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::Message;
    use crate::providers::base::{stream_from_single_message, ProviderUsage, Usage};
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A minimal provider whose locality is configurable and that records
    /// whether its `stream` was actually reached (i.e. the guard let the egress
    /// through).
    struct MockProvider {
        locality: DataLocality,
        streamed: Arc<AtomicBool>,
    }

    #[async_trait]
    impl Provider for MockProvider {
        fn get_name(&self) -> &str {
            "mock"
        }

        fn data_locality(&self) -> DataLocality {
            self.locality
        }

        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new("mock-model").unwrap()
        }

        async fn stream(
            &self,
            _model_config: &ModelConfig,
            _session_id: &str,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<MessageStream, ProviderError> {
            self.streamed.store(true, Ordering::SeqCst);
            Ok(stream_from_single_message(
                Message::assistant().with_text("ok"),
                ProviderUsage::new("mock-model".to_string(), Usage::default()),
            ))
        }
    }

    fn mock_with_audit_result(
        locality: DataLocality,
        audit_result: Result<(), String>,
    ) -> (Arc<dyn Provider>, Arc<AtomicBool>) {
        let streamed = Arc::new(AtomicBool::new(false));
        let inner = Arc::new(MockProvider {
            locality,
            streamed: streamed.clone(),
        });
        (
            SovereignGuardProvider::wrap_with_audit_result(inner, audit_result),
            streamed,
        )
    }

    fn mock(locality: DataLocality) -> (Arc<dyn Provider>, Arc<AtomicBool>) {
        mock_with_audit_result(locality, Ok(()))
    }

    async fn call(provider: &Arc<dyn Provider>, session_id: &str) -> Result<(), ProviderError> {
        let cfg = ModelConfig::new("mock-model").unwrap();
        provider
            .stream(
                &cfg,
                session_id,
                "sys",
                &[Message::user().with_text("hi")],
                &[],
            )
            .await
            .map(|_| ())
    }

    #[tokio::test]
    async fn cloud_provider_blocked_in_sovereign_session_and_inner_never_called() {
        let (guarded, streamed) = mock(DataLocality::Cloud);
        let sid = "guard-test-cloud-sovereign";
        sovereignty::mark_session_sovereign(sid);

        let result = call(&guarded, sid).await;

        assert!(
            result.is_err(),
            "cloud must be refused in a sovereign context"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains(sovereignty::SOVEREIGN_BLOCK_PREFIX),
            "error must be the sovereign block, got: {msg}"
        );
        assert!(
            !streamed.load(Ordering::SeqCst),
            "fail-closed: the inner provider's stream must NOT be reached"
        );
        sovereignty::unmark_session_sovereign(sid);
    }

    #[tokio::test]
    async fn cloud_provider_allowed_in_non_sovereign_session() {
        let (guarded, streamed) = mock(DataLocality::Cloud);
        let sid = "guard-test-cloud-open";

        let result = call(&guarded, sid).await;

        assert!(
            result.is_ok(),
            "cloud is allowed when the context is not sovereign"
        );
        assert!(
            streamed.load(Ordering::SeqCst),
            "the inner provider's stream must be reached"
        );
    }

    #[tokio::test]
    async fn allowed_cloud_provider_refused_when_audit_write_fails() {
        let (guarded, streamed) =
            mock_with_audit_result(DataLocality::Cloud, Err("audit unavailable".to_string()));
        let sid = "guard-test-cloud-audit-failure";

        let result = call(&guarded, sid).await;

        let msg = result
            .expect_err("an unaudited cloud call must be refused")
            .to_string();
        assert!(
            msg.contains("cloud call refused: could not write the required egress audit record"),
            "error must explain the fail-closed audit requirement, got: {msg}"
        );
        assert!(
            !streamed.load(Ordering::SeqCst),
            "the inner provider must not be reached after an audit failure"
        );
    }

    #[tokio::test]
    async fn local_provider_allowed_even_in_sovereign_session() {
        let (guarded, streamed) = mock(DataLocality::Local);
        let sid = "guard-test-local-sovereign";
        sovereignty::mark_session_sovereign(sid);

        let result = call(&guarded, sid).await;

        assert!(result.is_ok(), "local inference is always allowed");
        assert!(
            streamed.load(Ordering::SeqCst),
            "local provider stream must be reached"
        );
        sovereignty::unmark_session_sovereign(sid);
    }

    #[test]
    fn wrap_preserves_identity_and_locality() {
        let (guarded, _) = mock(DataLocality::Cloud);
        assert_eq!(guarded.get_name(), "mock");
        assert_eq!(guarded.data_locality(), DataLocality::Cloud);
    }
}
