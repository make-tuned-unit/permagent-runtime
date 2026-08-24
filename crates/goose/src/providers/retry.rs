use super::errors::ProviderError;
use crate::providers::base::Provider;
use async_trait::async_trait;
use std::future::Future;
use std::time::Duration;
use tokio::time::sleep;

pub const DEFAULT_MAX_RETRIES: usize = 3;
pub const DEFAULT_INITIAL_RETRY_INTERVAL_MS: u64 = 1000;
pub const DEFAULT_BACKOFF_MULTIPLIER: f64 = 2.0;
pub const DEFAULT_MAX_RETRY_INTERVAL_MS: u64 = 30_000;

#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub(crate) max_retries: usize,
    /// Initial interval between retries in milliseconds
    pub(crate) initial_interval_ms: u64,
    /// Multiplier for backoff (exponential)
    pub(crate) backoff_multiplier: f64,
    /// Maximum interval between retries in milliseconds
    pub(crate) max_interval_ms: u64,
    /// When true, only retry on transient errors (ServerError, NetworkError,
    /// RateLimitExceeded). RequestFailed (4xx client errors) will not be retried.
    pub(crate) transient_only: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_MAX_RETRIES,
            initial_interval_ms: DEFAULT_INITIAL_RETRY_INTERVAL_MS,
            backoff_multiplier: DEFAULT_BACKOFF_MULTIPLIER,
            max_interval_ms: DEFAULT_MAX_RETRY_INTERVAL_MS,
            transient_only: false,
        }
    }
}

impl RetryConfig {
    pub fn new(
        max_retries: usize,
        initial_interval_ms: u64,
        backoff_multiplier: f64,
        max_interval_ms: u64,
    ) -> Self {
        Self {
            max_retries,
            initial_interval_ms,
            backoff_multiplier,
            max_interval_ms,
            transient_only: false,
        }
    }

    pub fn transient_only(mut self) -> Self {
        self.transient_only = true;
        self
    }

    pub fn max_retries(&self) -> usize {
        self.max_retries
    }

    pub fn delay_for_attempt(&self, attempt: usize) -> Duration {
        if attempt == 0 {
            return Duration::from_millis(0);
        }

        let exponent = (attempt - 1) as u32;
        let base_delay_ms = (self.initial_interval_ms as f64
            * self.backoff_multiplier.powi(exponent as i32)) as u64;

        let capped_delay_ms = std::cmp::min(base_delay_ms, self.max_interval_ms);

        let jitter_factor_to_avoid_thundering_herd = 0.8 + (rand::random::<f64>() * 0.4);
        let jitter_delay_ms =
            (capped_delay_ms as f64 * jitter_factor_to_avoid_thundering_herd) as u64;

        Duration::from_millis(jitter_delay_ms)
    }
}

/// Is this network error a refused connection to a loopback endpoint?
///
/// Observed 2026-08-11: with no Ollama installed, a single background request
/// burned all ten retries against `localhost:11434` on exponential backoff —
/// minutes of wall-clock spent re-dialling a port that could not answer, and
/// ten identical WARN lines that drowned out the real ones.
///
/// A remote host can be down transiently and is worth retrying. "Connection
/// refused" from loopback means nothing is *listening on this machine*: no
/// amount of waiting starts a service that was never installed. Fail fast and
/// let the caller surface the real problem.
fn is_local_connection_refused(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    let looks_like_connect_failure = m.contains("could not connect")
        || m.contains("connection refused")
        || m.contains("tcp connect error");
    looks_like_connect_failure
        && (m.contains("localhost") || m.contains("127.0.0.1") || m.contains("[::1]"))
}

/// Should this error be retried at all?
///
/// The FIRST question is error class, not config: a permanent client error
/// (auth, billing, malformed request, missing model, oversized payload) fails
/// identically on every attempt, so retrying only buys dead wall-clock and
/// duplicate WARN lines. See [`ProviderError::is_permanent`] for the 2026-08-23
/// incident that made this explicit.
pub fn should_retry(error: &ProviderError, config: &RetryConfig) -> bool {
    if error.is_permanent() {
        return false;
    }
    match error {
        ProviderError::RateLimitExceeded { .. } | ProviderError::ServerError(_) => true,
        // Deterministic locally: retrying cannot make a missing local service appear.
        ProviderError::NetworkError(msg) => !is_local_connection_refused(msg),
        ProviderError::RequestFailed(_) => !config.transient_only,
        _ => false,
    }
}

pub async fn retry_operation<F, Fut, T>(
    config: &RetryConfig,
    operation: F,
) -> Result<T, ProviderError>
where
    F: Fn() -> Fut + Send,
    Fut: Future<Output = Result<T, ProviderError>> + Send,
    T: Send,
{
    let mut attempts = 0;

    loop {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(error) => {
                if should_retry(&error, config) && attempts < config.max_retries {
                    attempts += 1;
                    tracing::warn!(
                        "Request failed, retrying ({}/{}): {:?}",
                        attempts,
                        config.max_retries,
                        error
                    );

                    let delay = match &error {
                        ProviderError::RateLimitExceeded {
                            retry_delay: Some(d),
                            ..
                        } => *d,
                        _ => config.delay_for_attempt(attempts),
                    };

                    sleep(delay).await;
                    continue;
                }
                return Err(error);
            }
        }
    }
}

/// Trait for retry functionality to keep Provider dyn-compatible.
///
/// All `Provider` implementors get this via the blanket impl below.
#[async_trait]
pub trait ProviderRetry {
    fn retry_config(&self) -> RetryConfig {
        RetryConfig::default()
    }

    async fn with_retry<F, Fut, T>(&self, operation: F) -> Result<T, ProviderError>
    where
        F: Fn() -> Fut + Send,
        Fut: Future<Output = Result<T, ProviderError>> + Send,
        T: Send,
    {
        self.with_retry_config(operation, self.retry_config()).await
    }

    async fn with_retry_config<F, Fut, T>(
        &self,
        operation: F,
        config: RetryConfig,
    ) -> Result<T, ProviderError>
    where
        F: Fn() -> Fut + Send,
        Fut: Future<Output = Result<T, ProviderError>> + Send,
        T: Send;
}

#[async_trait]
impl<P: Provider> ProviderRetry for P {
    fn retry_config(&self) -> RetryConfig {
        Provider::retry_config(self)
    }

    async fn with_retry_config<F, Fut, T>(
        &self,
        operation: F,
        config: RetryConfig,
    ) -> Result<T, ProviderError>
    where
        F: Fn() -> Fut + Send,
        Fut: Future<Output = Result<T, ProviderError>> + Send,
        T: Send,
    {
        let mut attempts = 0;
        let mut auth_retried = false;

        loop {
            return match operation().await {
                Ok(result) => Ok(result),
                Err(error) => {
                    // Auth retry is separate from transient-error retries: we get
                    // at most 1 credential refresh, independent of max_retries.
                    if matches!(error, ProviderError::Authentication(_)) && !auth_retried {
                        auth_retried = true;
                        match self.refresh_credentials().await {
                            Ok(()) => {
                                tracing::warn!(
                                    "Credentials refreshed after auth error, retrying: {:?}",
                                    error
                                );
                                continue;
                            }
                            Err(refresh_err) => {
                                tracing::warn!(
                                    "Credential refresh failed, returning original auth error: {:?}",
                                    refresh_err
                                );
                            }
                        }
                    }

                    if should_retry(&error, &config) && attempts < config.max_retries {
                        attempts += 1;
                        tracing::warn!(
                            "Request failed, retrying ({}/{}): {:?}",
                            attempts,
                            config.max_retries,
                            error
                        );

                        let delay = match &error {
                            ProviderError::RateLimitExceeded {
                                retry_delay: Some(provider_delay),
                                ..
                            } => *provider_delay,
                            _ => config.delay_for_attempt(attempts),
                        };

                        let skip_backoff = std::env::var("GOOSE_PROVIDER_SKIP_BACKOFF")
                            .unwrap_or_default()
                            .parse::<bool>()
                            .unwrap_or(false);

                        if skip_backoff {
                            tracing::info!("Skipping backoff due to GOOSE_PROVIDER_SKIP_BACKOFF");
                        } else {
                            tracing::info!("Backing off for {:?} before retry", delay);
                            sleep(delay).await;
                        }
                        continue;
                    }

                    Err(error)
                }
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `RequestFailed` with no recognisable permanent marker stays retryable
    /// under the default config — that is what `transient_only` exists to turn
    /// off. (Before 2026-08-24 this test used a "Bad request (400)" message;
    /// that case is now classified permanent, which is the point of the fix.)
    #[test]
    fn default_config_retries_unclassified_request_failed() {
        let config = RetryConfig::default();
        let error = ProviderError::RequestFailed("Request failed with status 502".into());
        assert!(should_retry(&error, &config));
    }

    #[test]
    fn transient_only_skips_request_failed() {
        let config = RetryConfig::default().transient_only();
        let error = ProviderError::RequestFailed("Request failed with status 502".into());
        assert!(!should_retry(&error, &config));
    }

    #[test]
    fn transient_only_still_retries_server_error() {
        let config = RetryConfig::default().transient_only();
        assert!(should_retry(
            &ProviderError::ServerError("500 internal".into()),
            &config
        ));
    }

    #[test]
    fn transient_only_still_retries_network_error() {
        let config = RetryConfig::default().transient_only();
        assert!(should_retry(
            &ProviderError::NetworkError("connection refused".into()),
            &config
        ));
    }

    #[test]
    fn transient_only_still_retries_rate_limit() {
        let config = RetryConfig::default().transient_only();
        assert!(should_retry(
            &ProviderError::RateLimitExceeded {
                details: "too many requests".into(),
                retry_delay: None,
            },
            &config
        ));
    }

    #[test]
    fn never_retries_auth_errors() {
        let config = RetryConfig::default();
        assert!(!should_retry(
            &ProviderError::Authentication("invalid key".into()),
            &config
        ));
    }

    /// Regression (2026-08-11): with no Ollama installed, one background
    /// request burned all ten retries on exponential backoff against a port
    /// nothing was listening on. This is the exact message that did it.
    #[test]
    fn local_connection_refused_is_not_retried() {
        let config = RetryConfig::default();
        for msg in [
            "Could not connect to localhost:11434 — check your network connection and try again.",
            "Connection refused (os error 61) to 127.0.0.1:11434",
            "tcp connect error: [::1]:8080",
        ] {
            assert!(
                !should_retry(&ProviderError::NetworkError(msg.into()), &config),
                "must fail fast: {msg}"
            );
        }
    }

    /// The 2026-08-23 incident, as a table. Every row is a real status/error-type
    /// pair a provider sends; the boolean is whether the retry layer may try again.
    /// PERMANENT rows cost ~8s of dead wall-clock per turn before this fix.
    #[test]
    fn retry_decision_table() {
        let config = RetryConfig::default();
        let cases: Vec<(&str, ProviderError, bool)> = vec![
            // ── PERMANENT — must never retry ──────────────────────────────
            (
                "anthropic 400 invalid_request_error: credit balance too low",
                ProviderError::CreditsExhausted {
                    details: "Your credit balance is too low to access the Anthropic API. \
                              Please go to Plans & Billing to upgrade or purchase credits."
                        .into(),
                    top_up_url: None,
                },
                false,
            ),
            (
                "400 invalid_request_error, untyped",
                ProviderError::RequestFailed(
                    "Bad request (400): {\"type\":\"invalid_request_error\"}".into(),
                ),
                false,
            ),
            (
                "401 unauthorized",
                ProviderError::Authentication("Authentication failed. Status: 401".into()),
                false,
            ),
            (
                "403 forbidden",
                ProviderError::Authentication("Authentication failed. Status: 403".into()),
                false,
            ),
            (
                "402 payment required",
                ProviderError::CreditsExhausted {
                    details: "payment required".into(),
                    top_up_url: None,
                },
                false,
            ),
            (
                "404 model not found",
                ProviderError::RequestFailed("Resource not found (404): no such model".into()),
                false,
            ),
            (
                "404 endpoint not found",
                ProviderError::EndpointNotFound("models endpoint not found".into()),
                false,
            ),
            (
                "413 payload too large",
                ProviderError::ContextLengthExceeded("input length exceeds limit".into()),
                false,
            ),
            (
                "429 insufficient_quota is billing, not a rate limit",
                ProviderError::CreditsExhausted {
                    details: "insufficient_quota".into(),
                    top_up_url: None,
                },
                false,
            ),
            // ── TRANSIENT — must still retry ──────────────────────────────
            (
                "408 request timeout",
                ProviderError::RequestFailed("Request failed with status 408".into()),
                true,
            ),
            (
                "429 genuine rate limit",
                ProviderError::RateLimitExceeded {
                    details: "too many requests".into(),
                    retry_delay: None,
                },
                true,
            ),
            (
                "500 internal server error",
                ProviderError::ServerError("Server error (500)".into()),
                true,
            ),
            (
                "502 bad gateway",
                ProviderError::ServerError("Server error (502)".into()),
                true,
            ),
            (
                "503 service unavailable",
                ProviderError::ServerError("Server error (503)".into()),
                true,
            ),
            (
                "remote network timeout",
                ProviderError::NetworkError("Request timed out".into()),
                true,
            ),
        ];

        for (label, error, expected) in cases {
            assert_eq!(
                should_retry(&error, &config),
                expected,
                "retry decision wrong for: {label}"
            );
        }
    }

    /// The trap in classifying by message: OpenAI's ordinary 429 rate-limit body
    /// mentions payment and links to `/account/billing`. Matching on "billing"
    /// would turn a throttle — the single most retryable error there is — into a
    /// permanent failure that kills the turn.
    #[test]
    fn a_rate_limit_that_mentions_billing_is_still_retryable() {
        use crate::providers::errors::is_billing_message;
        let openai_429 = "Rate limit reached for gpt-4 in organization org-abc123 on requests \
                          per min (RPM): Limit 3, Used 3. Please try again in 20s. Please add a \
                          payment method to your account to increase your rate limit. Visit \
                          https://platform.openai.com/account/billing to add a payment method.";
        assert!(
            !is_billing_message(openai_429),
            "a throttle body that links to /account/billing must not read as out-of-credit"
        );
        assert!(should_retry(
            &ProviderError::RateLimitExceeded {
                details: openai_429.into(),
                retry_delay: None,
            },
            &RetryConfig::default()
        ));
    }

    /// The genuinely-terminal bodies, from three providers that each say it
    /// differently.
    #[test]
    fn real_billing_bodies_are_recognised() {
        use crate::providers::errors::is_billing_message;
        for body in [
            "Your credit balance is too low to access the Anthropic API. Please go to Plans & \
             Billing to upgrade or purchase credits.",
            "You exceeded your current quota, please check your plan and billing details.",
            "{\"code\":\"insufficient_quota\",\"message\":\"You have run out of credits\"}",
            "402 Payment Required",
            "Insufficient balance on your account.",
        ] {
            assert!(is_billing_message(body), "must read as billing: {body}");
        }
    }

    /// The exact production shape: the whole retry loop must make ZERO extra
    /// attempts on a billing rejection. Before the fix this ran 4 attempts
    /// (1 + 3 retries) with exponential backoff on every single turn.
    #[tokio::test]
    async fn billing_error_costs_zero_retries() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let attempts = AtomicUsize::new(0);
        let config = RetryConfig::default();

        let result: Result<(), ProviderError> = retry_operation(&config, || {
            attempts.fetch_add(1, Ordering::SeqCst);
            async {
                Err(ProviderError::CreditsExhausted {
                    details: "Your credit balance is too low to access the Anthropic API.".into(),
                    top_up_url: None,
                })
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "a permanent billing error must be attempted exactly once"
        );
    }

    /// The user-facing sentence must never carry the provider payload: no JSON
    /// braces, no error-type token, no support URL. Session 20260823_4 read the
    /// raw body out loud over TTS.
    #[test]
    fn user_facing_summary_never_leaks_raw_api_error() {
        let raw = "{\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\
                   \"message\":\"Your credit balance is too low to access the Anthropic API. \
                   Please go to Plans & Billing to upgrade or purchase credits.\"}}";
        let err = ProviderError::CreditsExhausted {
            details: raw.to_string(),
            top_up_url: Some("https://console.anthropic.com/settings/billing".into()),
        };
        let summary = err.user_facing_summary();
        for leaked in [
            "invalid_request_error",
            "{",
            "}",
            "https://",
            "console.anthropic.com",
        ] {
            assert!(
                !summary.contains(leaked),
                "user-facing summary leaked {leaked:?}: {summary}"
            );
        }
        assert!(summary.len() < 160, "summary should be one short sentence");
    }

    /// A REMOTE host can be transiently down — that is what retries are for.
    /// This is the case that must not regress when fixing the local one.
    #[test]
    fn remote_network_errors_are_still_retried() {
        let config = RetryConfig::default();
        for msg in [
            "Could not connect to api.example.com — check your network connection and try again.",
            "Connection refused (os error 61) to 100.74.232.95:11434",
            "dns error: failed to lookup address",
        ] {
            assert!(
                should_retry(&ProviderError::NetworkError(msg.into()), &config),
                "must still retry: {msg}"
            );
        }
    }
}
