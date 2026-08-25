use super::errors::ProviderError;
use crate::providers::base::Provider;
use crate::providers::wait_status;
use async_trait::async_trait;
use std::future::Future;
use std::time::Duration;
use tokio::time::sleep;

pub const DEFAULT_MAX_RETRIES: usize = 3;
pub const DEFAULT_INITIAL_RETRY_INTERVAL_MS: u64 = 1000;
pub const DEFAULT_BACKOFF_MULTIPLIER: f64 = 2.0;
pub const DEFAULT_MAX_RETRY_INTERVAL_MS: u64 = 30_000;

/// Floor for a rate-limit backoff when the provider names no wait period.
///
/// Rate limits are quoted per minute, not per second. On 2026-08-25 a Z.AI
/// 429 (`code 1302`, "Rate limit reached for requests", no `Retry-After`) was
/// retried at 1 s, 2 s and 3 s — the whole budget spent inside five seconds,
/// well before the window it was waiting on could have rolled over — and then
/// handed to the user as a failure. Twenty seconds is a floor, not a guess at
/// the window; the provider's own `Retry-After` still wins when it sends one.
pub const RATE_LIMIT_MIN_RETRY_INTERVAL_MS: u64 = 20_000;

/// Attempts allowed for a rate limit specifically, so the floor above adds up
/// to more than a minute of patience (20 s + 25 s + 30 s + 30 s) rather than
/// giving up inside one quota window.
pub const RATE_LIMIT_MAX_RETRIES: usize = 4;

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
    /// Floor for a rate-limit wait when the provider names no period of its
    /// own. Defaults to [`RATE_LIMIT_MIN_RETRY_INTERVAL_MS`].
    pub(crate) rate_limit_min_interval_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_MAX_RETRIES,
            initial_interval_ms: DEFAULT_INITIAL_RETRY_INTERVAL_MS,
            backoff_multiplier: DEFAULT_BACKOFF_MULTIPLIER,
            max_interval_ms: DEFAULT_MAX_RETRY_INTERVAL_MS,
            transient_only: false,
            rate_limit_min_interval_ms: RATE_LIMIT_MIN_RETRY_INTERVAL_MS,
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
            rate_limit_min_interval_ms: RATE_LIMIT_MIN_RETRY_INTERVAL_MS,
        }
    }

    /// Lower the rate-limit floor. Exists for tests and for a deployment that
    /// genuinely knows its provider's window is shorter; the default is the one
    /// to trust otherwise.
    pub fn with_rate_limit_floor_ms(mut self, ms: u64) -> Self {
        self.rate_limit_min_interval_ms = ms;
        self
    }

    pub fn transient_only(mut self) -> Self {
        self.transient_only = true;
        self
    }

    pub fn max_retries(&self) -> usize {
        self.max_retries
    }

    /// Attempts allowed for one particular error.
    ///
    /// A rate limit is the one class where giving up early is strictly worse
    /// than waiting: the request is valid, the quota simply has not rolled over
    /// yet. Everything else keeps the configured budget.
    pub fn retries_for(&self, error: &ProviderError) -> usize {
        match error {
            ProviderError::RateLimitExceeded { .. } => {
                std::cmp::max(self.max_retries, RATE_LIMIT_MAX_RETRIES)
            }
            _ => self.max_retries,
        }
    }

    /// How long to wait before retrying this error.
    ///
    /// Order of authority: the provider's own `Retry-After` first, then the
    /// rate-limit floor, then ordinary exponential backoff.
    pub fn delay_for_error(&self, error: &ProviderError, attempt: usize) -> Duration {
        match error {
            // The provider told us. Nothing we compute beats that.
            ProviderError::RateLimitExceeded {
                retry_delay: Some(provider_delay),
                ..
            } => *provider_delay,
            ProviderError::RateLimitExceeded {
                retry_delay: None, ..
            } => std::cmp::max(
                self.delay_for_attempt(attempt),
                jittered(self.rate_limit_min_interval_ms),
            ),
            _ => self.delay_for_attempt(attempt),
        }
    }

    pub fn delay_for_attempt(&self, attempt: usize) -> Duration {
        if attempt == 0 {
            return Duration::from_millis(0);
        }

        let exponent = (attempt - 1) as u32;
        let base_delay_ms = (self.initial_interval_ms as f64
            * self.backoff_multiplier.powi(exponent as i32)) as u64;

        let capped_delay_ms = std::cmp::min(base_delay_ms, self.max_interval_ms);

        Duration::from_millis(jittered_ms(capped_delay_ms))
    }
}

/// ±20% so a fleet of clients released by the same rate-limit window does not
/// walk back into it in lockstep.
fn jittered_ms(base_ms: u64) -> u64 {
    let jitter_factor_to_avoid_thundering_herd = 0.8 + (rand::random::<f64>() * 0.4);
    (base_ms as f64 * jitter_factor_to_avoid_thundering_herd) as u64
}

fn jittered(base_ms: u64) -> Duration {
    Duration::from_millis(jittered_ms(base_ms))
}

/// The wait reason a UI should show for this error.
fn wait_reason(error: &ProviderError) -> wait_status::WaitReason {
    match error {
        ProviderError::RateLimitExceeded { .. } => wait_status::WaitReason::RateLimited,
        ProviderError::ServerError(_) => wait_status::WaitReason::ServerError,
        _ => wait_status::WaitReason::Transient,
    }
}

/// Announce a wait, sleep it out, then take the announcement down.
///
/// Clearing on the way out is the part that matters: a status line left behind
/// after the wait ended is a lie the user has no way to correct.
async fn announce_and_sleep(
    provider: &str,
    error: &ProviderError,
    delay: Duration,
    attempt: usize,
    max_attempts: usize,
) {
    wait_status::publish(wait_status::ProviderWait {
        provider: provider.to_string(),
        reason: wait_reason(error),
        started: std::time::Instant::now(),
        delay,
        attempt,
        max_attempts,
    });
    sleep(delay).await;
    wait_status::clear();
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
                let budget = config.retries_for(&error);
                if should_retry(&error, config) && attempts < budget {
                    attempts += 1;
                    tracing::warn!(
                        "Request failed, retrying ({}/{}): {:?}",
                        attempts,
                        budget,
                        error
                    );

                    let delay = config.delay_for_error(&error, attempts);
                    announce_and_sleep("", &error, delay, attempts, budget).await;
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

                    let budget = config.retries_for(&error);
                    if should_retry(&error, &config) && attempts < budget {
                        attempts += 1;
                        tracing::warn!(
                            "Request failed, retrying ({}/{}): {:?}",
                            attempts,
                            budget,
                            error
                        );

                        let delay = config.delay_for_error(&error, attempts);

                        let skip_backoff = std::env::var("GOOSE_PROVIDER_SKIP_BACKOFF")
                            .unwrap_or_default()
                            .parse::<bool>()
                            .unwrap_or(false);

                        if skip_backoff {
                            tracing::info!("Skipping backoff due to GOOSE_PROVIDER_SKIP_BACKOFF");
                        } else {
                            tracing::info!("Backing off for {:?} before retry", delay);
                            announce_and_sleep(self.get_name(), &error, delay, attempts, budget)
                                .await;
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

    fn rate_limit(retry_delay: Option<Duration>) -> ProviderError {
        ProviderError::RateLimitExceeded {
            details: "Rate limit reached for requests".into(),
            retry_delay,
        }
    }

    /// The exact failure from 2026-08-25: Z.AI 429 with no `Retry-After`,
    /// retried at roughly 1 s, 2 s and 3 s and then given up on — the whole
    /// budget spent inside five seconds against a per-minute quota.
    #[test]
    fn a_rate_limit_with_no_provider_delay_waits_at_least_the_floor() {
        let config = RetryConfig::default();
        for attempt in 1..=RATE_LIMIT_MAX_RETRIES {
            let delay = config.delay_for_error(&rate_limit(None), attempt);
            assert!(
                delay >= Duration::from_millis(
                    (RATE_LIMIT_MIN_RETRY_INTERVAL_MS as f64 * 0.8) as u64
                ),
                "attempt {attempt} waited only {delay:?}"
            );
        }
    }

    /// Total patience for a rate limit must exceed a minute, so a per-minute
    /// quota has a chance to roll over before we hand the user a failure.
    #[test]
    fn the_rate_limit_budget_covers_more_than_one_quota_window() {
        let config = RetryConfig::default();
        let error = rate_limit(None);
        let budget = config.retries_for(&error);
        assert!(budget >= RATE_LIMIT_MAX_RETRIES, "budget was {budget}");

        // Worst case, with jitter at its lowest.
        let total: Duration = (1..=budget)
            .map(|attempt| config.delay_for_error(&error, attempt))
            .sum();
        assert!(
            total >= Duration::from_secs(60),
            "a rate limit gets only {total:?} of patience"
        );
    }

    /// The provider's own number wins outright — floor included. If Z.AI says
    /// two seconds, we wait two seconds, not twenty.
    #[test]
    fn a_provider_supplied_retry_after_wins_over_the_floor() {
        let config = RetryConfig::default();
        assert_eq!(
            config.delay_for_error(&rate_limit(Some(Duration::from_secs(2))), 1),
            Duration::from_secs(2)
        );
        assert_eq!(
            config.delay_for_error(&rate_limit(Some(Duration::from_secs(300))), 3),
            Duration::from_secs(300)
        );
    }

    /// The floor is for rate limits only. A 5xx keeps the fast schedule — the
    /// point of the change is patience with quotas, not slowness everywhere.
    #[test]
    fn other_errors_keep_the_ordinary_backoff() {
        let config = RetryConfig::default();
        let server_error = ProviderError::ServerError("502".into());
        assert_eq!(config.retries_for(&server_error), config.max_retries());
        let delay = config.delay_for_error(&server_error, 1);
        assert!(delay < Duration::from_secs(5), "got {delay:?}");
    }

    /// A retry loop that sleeps in silence is the bug the status slot exists to
    /// fix. Drives `retry_operation` against a fake 429 and checks that the
    /// wait is visible *while it happens* and gone afterwards.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_fake_429_is_announced_while_we_wait_and_cleared_after() {
        wait_status::clear();

        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_for_op = calls.clone();

        // A real (short) wait rather than a paused clock: `tokio`'s time
        // pausing needs the `test-util` feature, which this crate does not
        // carry, and what is under test is the announcement, not its length.
        let config = RetryConfig::new(2, 20, 2.0, 100).with_rate_limit_floor_ms(400);

        let retrying = tokio::spawn(async move {
            retry_operation(&config, || {
                let calls = calls_for_op.clone();
                async move {
                    if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                        Err(rate_limit(None))
                    } else {
                        Ok::<&str, ProviderError>("second attempt succeeded")
                    }
                }
            })
            .await
        });

        // Catch the wait in flight.
        let mut announced = None;
        for _ in 0..200 {
            if let Some(wait) = wait_status::current() {
                announced = Some(wait);
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        let announced = announced.expect("the wait was never announced");
        assert_eq!(announced.reason, wait_status::WaitReason::RateLimited);
        assert!(
            announced.status_line().contains("rate limit"),
            "{}",
            announced.status_line()
        );

        assert_eq!(retrying.await.unwrap().unwrap(), "second attempt succeeded");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(
            wait_status::current().is_none(),
            "the status line outlived the wait"
        );
    }

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
