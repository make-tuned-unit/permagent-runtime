pub mod adversary_inspector;
pub mod classification_client;
pub mod egress_inspector;
pub mod patterns;
pub mod scanner;
pub mod security_inspector;
pub mod untrusted_content;
pub mod write_jail;

use crate::config::Config;
use crate::conversation::message::{Message, ToolRequest};
use crate::permission::permission_judge::PermissionCheckResult;
use anyhow::Result;
use scanner::PromptInjectionScanner;
use std::sync::OnceLock;
use uuid::Uuid;

pub struct SecurityManager {
    scanner: OnceLock<PromptInjectionScanner>,
}

#[derive(Debug, Clone)]
pub struct SecurityResult {
    pub is_malicious: bool,
    pub confidence: f32,
    pub explanation: String,
    pub should_ask_user: bool,
    pub finding_id: String,
    pub tool_request_id: String,
}

impl SecurityManager {
    pub fn new() -> Self {
        Self {
            scanner: OnceLock::new(),
        }
    }

    /// Prompt-injection detection is ON by default (C3): the scanner gates the
    /// dangerous tools (shell + write/edit) and its above-threshold findings
    /// block by routing through the Decision Inbox as an answerable approval.
    /// Set `SECURITY_PROMPT_ENABLED: false` to disable scanning entirely, or
    /// `SECURITY_PROMPT_LOG_ONLY: true` to keep scanning but never block.
    pub fn is_prompt_injection_detection_enabled(&self) -> bool {
        prompt_detection_enabled(
            Config::global()
                .get_param::<bool>("SECURITY_PROMPT_ENABLED")
                .ok(),
        )
    }

    /// Log-only mode: findings are still scanned and logged, but never block
    /// (the pre-C3 posture). Explicit config over a hidden default.
    pub fn is_log_only(&self) -> bool {
        prompt_detection_log_only(
            Config::global()
                .get_param::<bool>("SECURITY_PROMPT_LOG_ONLY")
                .ok(),
        )
    }

    fn is_ml_scanning_enabled(&self) -> bool {
        let config = Config::global();

        let prompt_enabled = config
            .get_param::<bool>("SECURITY_PROMPT_CLASSIFIER_ENABLED")
            .unwrap_or(false);

        let command_enabled = config
            .get_param::<bool>("SECURITY_COMMAND_CLASSIFIER_ENABLED")
            .unwrap_or(false);

        prompt_enabled || command_enabled
    }

    pub async fn analyze_tool_requests(
        &self,
        tool_requests: &[ToolRequest],
        messages: &[Message],
    ) -> Result<Vec<SecurityResult>> {
        if !self.is_prompt_injection_detection_enabled() {
            tracing::debug!(
                monotonic_counter.goose.prompt_injection_scanner_disabled = 1,
                "Security scanning disabled"
            );
            return Ok(vec![]);
        }

        let scanner = self.scanner.get_or_init(|| {
            let config = Config::global();
            let command_classifier_enabled = config
                .get_param::<bool>("SECURITY_COMMAND_CLASSIFIER_ENABLED")
                .unwrap_or(false);
            let prompt_classifier_enabled = config
                .get_param::<bool>("SECURITY_PROMPT_CLASSIFIER_ENABLED")
                .unwrap_or(false);

            tracing::info!(
                monotonic_counter.goose.security_command_classifier_enabled = if command_classifier_enabled { 1 } else { 0 },
                monotonic_counter.goose.security_prompt_classifier_enabled = if prompt_classifier_enabled { 1 } else { 0 },
                "Security classifier configuration"
            );

            let ml_enabled = self.is_ml_scanning_enabled();

            let scanner = if ml_enabled {
                match PromptInjectionScanner::with_ml_detection() {
                    Ok(s) => {
                        tracing::info!(
                            monotonic_counter.goose.prompt_injection_scanner_enabled = 1,
                            "Security scanner initialized with ML-based detection"
                        );
                        s
                    }
                    Err(e) => {
                        let error_chain = format!("{:#}", e);
                        tracing::warn!(
                            "ML scanning requested but failed to initialize. Falling back to pattern-only scanning.\n\nError details:\n{}",
                            error_chain
                        );
                        PromptInjectionScanner::new()
                    }
                }
            } else {
                tracing::info!(
                    monotonic_counter.goose.prompt_injection_scanner_enabled = 1,
                    "Security scanner initialized with pattern-based detection only"
                );
                PromptInjectionScanner::new()
            };

            scanner
        });

        let mut results = Vec::new();

        tracing::debug!(
            "Starting security analysis - {} tool requests, {} messages",
            tool_requests.len(),
            messages.len()
        );

        for tool_request in tool_requests.iter() {
            if let Ok(tool_call) = &tool_request.tool_call {
                let analysis_result = scanner
                    .analyze_tool_call_with_context(tool_call, messages)
                    .await?;

                let config_threshold = scanner.get_threshold_from_config();
                let sanitized_explanation = analysis_result.explanation.replace('\n', " | ");

                if analysis_result.is_malicious {
                    let above_threshold = analysis_result.confidence > config_threshold;
                    let finding_id = format!("SEC-{}", Uuid::new_v4().simple());

                    let tool_call_json =
                        serde_json::to_string(&tool_call).unwrap_or_else(|_| "{}".to_string());

                    tracing::warn!(
                        monotonic_counter.goose.prompt_injection_finding = 1,
                        threat_type = "command_injection",
                        above_threshold = above_threshold,
                        tool_name = %tool_call.name,
                        tool_request_id = %tool_request.id,
                        tool_call_json = %tool_call_json,
                        confidence = analysis_result.confidence,
                        explanation = %sanitized_explanation,
                        finding_id = %finding_id,
                        threshold = config_threshold,
                        "{}",
                        if above_threshold {
                            "Prompt injection detection: Current tool call flagged as malicious after security analysis (above threshold)"
                        } else {
                            "Prompt injection detection: Security finding below threshold (logged but not blocking execution)"
                        }
                    );
                    if above_threshold {
                        results.push(SecurityResult {
                            is_malicious: analysis_result.is_malicious,
                            confidence: analysis_result.confidence,
                            explanation: analysis_result.explanation,
                            should_ask_user: true, // Always ask user for threats above threshold
                            finding_id,
                            tool_request_id: tool_request.id.clone(),
                        });
                    }
                } else if analysis_result.scanned {
                    let tool_call_json =
                        serde_json::to_string(&tool_call).unwrap_or_else(|_| "{}".to_string());

                    tracing::info!(
                        monotonic_counter.goose.prompt_injection_tool_call_passed = 1,
                        tool_name = %tool_call.name,
                        tool_request_id = %tool_request.id,
                        tool_call_json = %tool_call_json,
                        confidence = analysis_result.confidence,
                        explanation = %sanitized_explanation,
                        "Current tool call passed security analysis"
                    );
                }
            }
        }

        tracing::info!(
            monotonic_counter.goose.prompt_injection_analysis_performed = 1,
            security_issues_found = results.len(),
            "Prompt injection detection: Security analysis complete"
        );
        Ok(results)
    }

    pub async fn filter_malicious_tool_calls(
        &self,
        messages: &[Message],
        permission_check_result: &PermissionCheckResult,
        _system_prompt: Option<&str>,
    ) -> Result<Vec<SecurityResult>> {
        let tool_requests: Vec<_> = permission_check_result
            .approved
            .iter()
            .chain(permission_check_result.needs_approval.iter())
            .cloned()
            .collect();

        self.analyze_tool_requests(&tool_requests, messages).await
    }
}

impl Default for SecurityManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Pure default policies, split out so the defaults are testable without the
/// process-global config: scanning defaults ON, log-only defaults OFF.
pub(crate) fn prompt_detection_enabled(param: Option<bool>) -> bool {
    param.unwrap_or(true)
}

pub(crate) fn prompt_detection_log_only(param: Option<bool>) -> bool {
    param.unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// C3 default posture: scanning ON unless explicitly disabled; blocking
    /// (not log-only) unless explicitly reverted.
    #[test]
    fn default_posture_is_scan_and_block() {
        assert!(prompt_detection_enabled(None), "scanning defaults ON");
        assert!(prompt_detection_enabled(Some(true)));
        assert!(!prompt_detection_enabled(Some(false)), "explicit off wins");

        assert!(!prompt_detection_log_only(None), "blocking is the default");
        assert!(
            prompt_detection_log_only(Some(true)),
            "log-only revert flag"
        );
        assert!(!prompt_detection_log_only(Some(false)));
    }
}
