use reqwest::StatusCode;
use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq)]
pub enum ProviderError {
    #[error("Authentication error: {0}")]
    Authentication(String),

    #[error("Context length exceeded: {0}")]
    ContextLengthExceeded(String),

    #[error("Rate limit exceeded: {details}")]
    RateLimitExceeded {
        details: String,
        retry_delay: Option<Duration>,
    },

    #[error("Server error: {0}")]
    ServerError(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Request failed: {0}")]
    RequestFailed(String),

    #[error("Execution error: {0}")]
    ExecutionError(String),

    #[error("Usage data error: {0}")]
    UsageError(String),

    #[error("Unsupported operation: {0}")]
    NotImplemented(String),

    #[error("Endpoint not found (404): {0}")]
    EndpointNotFound(String),

    #[error("Credits exhausted: {details}")]
    CreditsExhausted {
        details: String,
        top_up_url: Option<String>,
    },
}

/// Marker prefix for [`ProviderError::stream_budget_exceeded`]. It is a prefix,
/// not a separate variant, so the whole provider surface (telemetry class,
/// retry policy, every `match` over `ProviderError`) keeps working unchanged.
pub(crate) const STREAM_BUDGET_PREFIX: &str = "Provider stream budget exceeded: ";

/// Sub-marker nested inside [`STREAM_BUDGET_PREFIX`], for the specific case
/// of a stall before the provider sent even the first byte of a response
/// (Lane R19a). See [`ProviderError::first_byte_stall`].
pub(crate) const FIRST_BYTE_STALL_MARKER: &str = "first byte: ";

impl ProviderError {
    pub fn telemetry_type(&self) -> &'static str {
        match self {
            ProviderError::Authentication(_) => "auth",
            ProviderError::ContextLengthExceeded(_) => "context_length",
            ProviderError::RateLimitExceeded { .. } => "rate_limit",
            ProviderError::ServerError(_) => "server",
            ProviderError::NetworkError(_) => "network",
            ProviderError::RequestFailed(_) => "request",
            ProviderError::ExecutionError(_) => "execution",
            ProviderError::UsageError(_) => "usage",
            ProviderError::NotImplemented(_) => "not_implemented",
            ProviderError::EndpointNotFound(_) => "endpoint_not_found",
            ProviderError::CreditsExhausted { .. } => "credits_exhausted",
        }
    }

    pub fn is_endpoint_not_found(&self) -> bool {
        matches!(self, ProviderError::EndpointNotFound(_))
    }

    /// Is this a PERMANENT client-side failure — one that will fail identically
    /// on every retry, no matter how long we wait?
    ///
    /// Observed 2026-08-23 (session 20260823_4): Anthropic returned HTTP 400
    /// `invalid_request_error` — "Your credit balance is too low to access the
    /// Anthropic API" — on every request. The retry layer treated it exactly
    /// like a transient timeout: 3/3 retries with exponential backoff, ~8s of
    /// dead wall-clock per turn, three identical WARN lines, and then the raw
    /// API error read out to the user. A depleted balance does not refill
    /// because we slept 4 seconds.
    ///
    /// Permanent: auth (401/403), billing/credit (402 and the 400 variants
    /// providers use for it), a malformed request (400 `invalid_request_error`),
    /// a model that does not exist (404), and a payload over the limit (413 —
    /// surfaced as `ContextLengthExceeded`). Transient — and therefore NOT
    /// permanent: 408, 429, every 5xx, and remote network failures.
    pub fn is_permanent(&self) -> bool {
        match self {
            ProviderError::Authentication(_)
            | ProviderError::CreditsExhausted { .. }
            | ProviderError::ContextLengthExceeded(_)
            | ProviderError::EndpointNotFound(_)
            | ProviderError::NotImplemented(_) => true,
            ProviderError::RequestFailed(msg) => is_permanent_client_message(msg),
            // Y8a: a stream that blew its budget is permanent BY CONSTRUCTION.
            // The bytes we would resend are identical, and the failure mode is
            // the model repeating itself — a retry re-enters the same runaway
            // and burns the same wall-clock and the same unmetered tokens.
            //
            // Lane R19c (G24 LOW): EXCEPT a first-byte stall, which is
            // retryable by definition (Lane R19a/R19b) — a pre-first-byte
            // stall's bytes-we-would-resend are identical too, but the
            // failure mode is a provider that has not answered YET, not one
            // caught repeating itself; `agents::agent` retries it once for
            // exactly that reason. Leaving this `true` for first-byte stalls
            // contradicted that retry premise for every OTHER caller that
            // reads `is_permanent()` (`cost_router::fallback`, the
            // orchestrator, `goal_a2a`) even though it happened not to
            // matter for R19a's own agent.rs arms (inserted ahead of the
            // generic catch-all that reads this).
            ProviderError::ExecutionError(msg) => {
                msg.starts_with(STREAM_BUDGET_PREFIX) && !self.is_first_byte_provider_stall()
            }
            _ => false,
        }
    }

    /// Mid-stream body that could not be decoded. The HTTP request already
    /// succeeded; the socket died, a proxy truncated the SSE, or the codec
    /// saw garbage. Transient — not a 4xx rejection of the payload.
    ///
    /// Session 20260827_1 (2026-08-27): two `Stream decode error: error
    /// decoding response body` failures at 04:26 and 04:36 UTC were typed as
    /// `RequestFailed`, so the reply path told the user the provider
    /// "rejected this request as invalid" and invited a model switch.
    /// Resending would have worked.
    pub fn stream_decode(err: impl std::fmt::Display) -> Self {
        ProviderError::NetworkError(format!("Stream decode error: {err}"))
    }

    pub fn is_stream_decode(&self) -> bool {
        match self {
            ProviderError::NetworkError(msg) | ProviderError::RequestFailed(msg) => {
                is_stream_decode_message(msg)
            }
            _ => false,
        }
    }

    /// A provider response that ran past its own safety budget and was stopped
    /// by us, not by the provider (Y8a, harness loop 2026-09-10).
    ///
    /// M14 slice, fixture glm47/fx04: one z.ai `glm-4.7` response streamed
    /// **441 s, 12 960 delta events and 3.0 MB** of the same sentence
    /// (`"Good! Cart preview is now updated. Need to check admin and API
    /// next."`) and stopped only when the eval driver killed the process at its
    /// 600 s ceiling. `stream_openai_compat`'s `while let Some(..)` had no
    /// wall-clock and no output budget, so the only bound on a degenerate model
    /// was the process lifetime. The reservation for that call never settled —
    /// the tokens were streamed and billed, and no ledger row was written.
    ///
    /// Typed as `ExecutionError` deliberately: `should_retry` already returns
    /// `false` for that variant, and [`is_permanent`](Self::is_permanent) also
    /// reports it as permanent, so neither the retry layer nor a pre-commit
    /// failover can walk straight back into the same loop.
    pub fn stream_budget_exceeded(detail: impl std::fmt::Display) -> Self {
        ProviderError::ExecutionError(format!("{STREAM_BUDGET_PREFIX}{detail}"))
    }

    pub fn is_stream_budget_exceeded(&self) -> bool {
        matches!(self, ProviderError::ExecutionError(msg) if msg.starts_with(STREAM_BUDGET_PREFIX))
    }

    /// A stream that never delivered even one byte — not a real delta, not a
    /// usage-only chunk, nothing — within
    /// `PERMAGENT_PROVIDER_STREAM_FIRST_BYTE_SECS` (Lane R19a, harness loop
    /// 2026-09-14).
    ///
    /// Distinct from the general [`stream_budget_exceeded`] on purpose: a
    /// pre-first-byte stall is commonly the provider still queueing or
    /// prefilling a large context (M19 DeepSeek forensics: a ~39 KB
    /// coordinator system prompt during a ~20 load-average window stalled at
    /// 0 chars for exactly the 120 s stall window, three times in a row, on a
    /// request whose z.ai twin — same binary, same moment, same payload
    /// shape — answered in seconds). That is not "the model is repeating
    /// itself" (the general case this module exists to catch); it is worth
    /// one retry of the identical request before the turn is forced to end.
    ///
    /// Nested inside [`STREAM_BUDGET_PREFIX`] rather than a new top-level
    /// variant, so [`is_stream_budget_exceeded`](Self::is_stream_budget_exceeded)
    /// keeps matching every stream-budget failure exactly as before;
    /// [`is_first_byte_provider_stall`](Self::is_first_byte_provider_stall)
    /// narrows it to this one.
    pub fn first_byte_stall(detail: impl std::fmt::Display) -> Self {
        ProviderError::ExecutionError(format!(
            "{STREAM_BUDGET_PREFIX}{FIRST_BYTE_STALL_MARKER}{detail}"
        ))
    }

    pub fn is_first_byte_provider_stall(&self) -> bool {
        matches!(self, ProviderError::ExecutionError(msg) if msg
            .strip_prefix(STREAM_BUDGET_PREFIX)
            .is_some_and(|rest| rest.starts_with(FIRST_BYTE_STALL_MARKER)))
    }

    /// A short, human sentence safe to show a user or read aloud — never the
    /// raw provider payload. `raw` bodies carry request ids, model ids, JSON
    /// braces and support URLs; session 20260823_4 read one of those out loud.
    pub fn user_facing_summary(&self) -> String {
        if self.is_first_byte_provider_stall() {
            // Deliberately silent on whether a retry happened: this summary
            // is shared by every consumer of a streaming response, and only
            // the coding-harness coordinator turn (`agents::agent`) retries
            // a first-byte stall once before giving up (Lane R19a).
            return "the provider did not send anything back in time, so the request was stopped"
                .to_string();
        }
        if self.is_stream_budget_exceeded() {
            return "the model kept generating past its safety budget, so the reply was stopped"
                .to_string();
        }
        if self.is_stream_decode() {
            return "the model stream dropped before the reply finished".to_string();
        }
        match self {
            ProviderError::CreditsExhausted { .. } => {
                "the provider rejected the request for billing reasons (credit balance too low)"
                    .to_string()
            }
            ProviderError::Authentication(_) => {
                "the provider rejected the API key for this model".to_string()
            }
            ProviderError::ContextLengthExceeded(_) => {
                "the conversation is too long for this model's context window".to_string()
            }
            ProviderError::RateLimitExceeded { .. } => {
                "the provider is rate-limiting this model right now".to_string()
            }
            ProviderError::ServerError(_) => "the provider had a server error".to_string(),
            ProviderError::NetworkError(_) => "the provider could not be reached".to_string(),
            ProviderError::EndpointNotFound(_) => {
                "that model or endpoint does not exist on this provider".to_string()
            }
            _ => "the provider request failed".to_string(),
        }
    }
}

/// Providers disagree on which status code carries "you are out of money".
/// Anthropic sends HTTP 400 `invalid_request_error` with "credit balance is too
/// low"; OpenAI sends 429 `insufficient_quota`; others send a plain 402. Match
/// the *body*, not just the status, so the billing case is recognised wherever
/// it is sent from.
///
/// ## Why the markers are narrow
///
/// A bare "billing" substring is NOT enough. OpenAI's ordinary 429 rate-limit
/// body reads "…Please add a payment method to your account to increase your
/// rate limit. Visit https://platform.openai.com/account/billing…" — matching on
/// "billing" there would classify a genuine, retryable rate limit as a permanent
/// billing failure and stop the turn dead. Every marker below is a phrase that
/// only appears when the account actually cannot pay, and
/// [`looks_like_rate_limit`] vetoes the match if the body is also describing a
/// rate limit.
pub fn is_billing_message(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    let billing_marker = m.contains("credit balance")
        || m.contains("insufficient_quota")
        || m.contains("insufficient credits")
        || m.contains("insufficient balance")
        || m.contains("insufficient funds")
        || m.contains("billing_hard_limit_reached")
        || m.contains("payment required")
        || m.contains("purchase credits")
        || m.contains("plans & billing")
        || m.contains("exceeded your current quota");

    billing_marker && !looks_like_rate_limit(&m)
}

/// Does this body describe a THROTTLE rather than an empty account? A throttle
/// clears by waiting, so it must stay retryable even when the same body also
/// mentions payment.
fn looks_like_rate_limit(lowercased: &str) -> bool {
    // "exceeded your current quota" is OpenAI's out-of-money phrasing and never
    // appears in a throttle body, so it wins outright.
    if lowercased.contains("exceeded your current quota")
        || lowercased.contains("insufficient_quota")
    {
        return false;
    }
    lowercased.contains("rate limit")
        || lowercased.contains("rate_limit")
        || lowercased.contains("try again in")
        || lowercased.contains("requests per")
        || lowercased.contains("tokens per")
        || lowercased.contains("too many requests")
}

/// An auth rejection that a provider chose to send as something other than a
/// 401/403 (some send 400 `authentication_error`, some say "invalid x-api-key").
fn is_auth_message(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("authentication_error")
        || m.contains("permission_error")
        || m.contains("invalid api key")
        || m.contains("invalid x-api-key")
        || m.contains("invalid_api_key")
}

/// Lane R36a (session-scoped provider circuit breaker): classify a failed
/// provider call's error text into the reason class the breaker records, or
/// `None` when the failure is NOT "this specific provider is unavailable
/// right now" — a malformed request or an unknown model id would fail
/// identically against a DIFFERENT provider too, so those must never open a
/// breaker (matches Lane R28b's `REVIEWER_PROVIDER_UNAVAILABLE_MARKERS`
/// rationale, promoted here so the planner's `complete_fast` decomposition
/// path and the salvage reviewer path share one classifier instead of two
/// that could disagree).
pub fn classify_unavailable(msg: &str) -> Option<&'static str> {
    if is_billing_message(msg) {
        return Some("billing");
    }
    let m = msg.to_ascii_lowercase();
    // `is_auth_message` alone is narrower than Lane R28b's original marker
    // set (it looks for the API-key-rejection phrasing, not a bare 401) —
    // "unauthorized"/"authentication error" (space form) are kept here so a
    // plain "401 Unauthorized" still classifies, matching R28b's retry
    // behavior byte-for-byte.
    if is_auth_message(msg) || m.contains("unauthorized") || m.contains("authentication error") {
        return Some("auth");
    }
    if m.contains("rate limit") || m.contains("rate-limit") || m.contains("too many requests") {
        return Some("rate_limit");
    }
    if m.contains("server error")
        || m.contains("internal server error")
        || m.contains("bad gateway")
        || m.contains("service unavailable")
        || m.contains("gateway timeout")
    {
        return Some("server_error");
    }
    if m.contains("could not be reached")
        || m.contains("connection refused")
        || m.contains("connection reset")
        || m.contains("network error")
    {
        return Some("network");
    }
    if m.contains("timed out") || m.contains("did not respond within") {
        return Some("timeout");
    }
    None
}

/// Display-text prefix of [`ProviderError::ContextLengthExceeded`]'s own
/// `#[error(...)]` (`"Context length exceeded: {0}"`, above). A caller that
/// only holds the STRINGIFIED error — e.g. `Agent::take_context_length_
/// failure`, which a headless `delegate`/`delegate_many` subagent's caller
/// reads back to classify an otherwise-opaque `AgentRuntimeOutcome::Failed`
/// (harness loop, "A43 analysis" lever 3) — can still recognise "this was
/// specifically a context-length overflow" against this marker without
/// re-parsing a typed `ProviderError`.
pub const CONTEXT_LENGTH_EXCEEDED_MARKER: &str = "Context length exceeded";

/// Best-effort extraction of a local-inference server's own token-budget
/// numbers from a raw context-length-exceeded body. A llama.cpp-shaped 400
/// (the exact shape "A43 analysis" observed: MiniCPM5-2B's `delegate` calls
/// failed with `n_prompt_tokens: 34930` against `n_ctx: 32768`) embeds
/// `"n_prompt_tokens":<N>` and `"n_ctx":<M>` directly in the error text this
/// module's `map_http_error_with_retry_after` wraps verbatim into
/// [`ProviderError::ContextLengthExceeded`]. Returns `None` when the body
/// isn't in this shape (a different provider's 413/400, or free text) —
/// callers must have a fallback message that reads fine without numbers.
///
/// Char-based scanning only (`chars()`/`split_once`), never a byte-index
/// slice: the raw body is untrusted provider text, not guaranteed ASCII
/// past the JSON key names this looks for.
pub fn extract_context_length_token_counts(raw: &str) -> Option<(u64, u64)> {
    let prompt_tokens = extract_json_number_field(raw, "n_prompt_tokens")?;
    let limit = extract_json_number_field(raw, "n_ctx")?;
    Some((prompt_tokens, limit))
}

fn extract_json_number_field(raw: &str, field: &str) -> Option<u64> {
    let marker = format!("\"{field}\"");
    let (_, after) = raw.split_once(marker.as_str())?;
    let digits: String = after
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

/// Message-level classification for the `RequestFailed` catch-all, which is
/// where 4xx bodies land once the status code has been discarded.
fn is_permanent_client_message(msg: &str) -> bool {
    if is_billing_message(msg) || is_auth_message(msg) {
        return true;
    }
    let m = msg.to_ascii_lowercase();
    // A 400 the provider itself labelled "invalid request" is deterministic:
    // the same bytes produce the same rejection forever.
    m.contains("invalid_request_error")
        || m.contains("bad request (400)")
        || m.contains("status: 400")
        || m.contains("status 400")
        // A model/route that does not exist will not appear because we waited.
        || m.contains("resource not found (404)")
        || m.contains("model_not_found")
        || m.contains("status: 404")
        || m.contains("status 404")
        // Payload too large is a property of the request, not of the moment.
        || m.contains("(413)")
        || m.contains("status: 413")
        || m.contains("status 413")
}

fn is_stream_decode_message(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("stream decode error") || m.contains("error decoding response body")
}

fn is_network_error(err: &reqwest::Error) -> bool {
    err.is_connect()
        || err.is_timeout()
        || err.is_decode()
        || err.is_body()
        || (err.status().is_none() && err.is_request())
}

fn provider_error_from_reqwest(error: &reqwest::Error) -> ProviderError {
    if error.is_decode() || error.is_body() {
        return ProviderError::stream_decode(error);
    }
    if is_network_error(error) {
        let msg = if error.is_timeout() {
            "Request timed out — check your network connection and try again.".to_string()
        } else if error.is_connect() {
            if let Some(url) = error.url() {
                if let Some(host) = url.host_str() {
                    let port_info = url.port().map(|p| format!(":{}", p)).unwrap_or_default();
                    format!(
                        "Could not connect to {}{} — check your network connection and try again.",
                        host, port_info
                    )
                } else {
                    "Could not connect to the provider — check your network connection and try again.".to_string()
                }
            } else {
                "Could not connect to the provider — check your network connection and try again."
                    .to_string()
            }
        } else {
            "Network error — check your network connection and try again.".to_string()
        };
        return ProviderError::NetworkError(msg);
    }

    let mut details = vec![];
    if let Some(status) = error.status() {
        details.push(format!("status: {}", status));
    }
    let msg = if details.is_empty() {
        error.to_string()
    } else {
        format!("{} ({})", error, details.join(", "))
    };
    ProviderError::RequestFailed(msg)
}

impl From<anyhow::Error> for ProviderError {
    fn from(error: anyhow::Error) -> Self {
        if let Some(reqwest_err) = error.downcast_ref::<reqwest::Error>() {
            return provider_error_from_reqwest(reqwest_err);
        }
        ProviderError::ExecutionError(error.to_string())
    }
}

impl From<reqwest::Error> for ProviderError {
    fn from(error: reqwest::Error) -> Self {
        provider_error_from_reqwest(&error)
    }
}

#[derive(Debug)]
pub enum GoogleErrorCode {
    BadRequest = 400,
    Unauthorized = 401,
    Forbidden = 403,
    NotFound = 404,
    TooManyRequests = 429,
    InternalServerError = 500,
    ServiceUnavailable = 503,
}

impl GoogleErrorCode {
    pub fn to_status_code(&self) -> StatusCode {
        match self {
            Self::BadRequest => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            Self::InternalServerError => StatusCode::INTERNAL_SERVER_ERROR,
            Self::ServiceUnavailable => StatusCode::SERVICE_UNAVAILABLE,
        }
    }

    pub fn from_code(code: u64) -> Option<Self> {
        match code {
            400 => Some(Self::BadRequest),
            401 => Some(Self::Unauthorized),
            403 => Some(Self::Forbidden),
            404 => Some(Self::NotFound),
            429 => Some(Self::TooManyRequests),
            500 => Some(Self::InternalServerError),
            503 => Some(Self::ServiceUnavailable),
            _ => Some(Self::InternalServerError),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_decode_is_a_transient_network_error() {
        let err = ProviderError::stream_decode("error decoding response body");
        assert!(matches!(err, ProviderError::NetworkError(_)));
        assert!(err.is_stream_decode());
        assert!(!err.is_permanent());
        let summary = err.user_facing_summary();
        assert_eq!(
            summary,
            "the model stream dropped before the reply finished"
        );
        assert!(!summary.contains("decoding response body"));
    }

    #[test]
    fn leftover_request_failed_stream_decode_is_still_detected() {
        let err = ProviderError::RequestFailed(
            "Stream decode error: error decoding response body".into(),
        );
        assert!(err.is_stream_decode());
        assert!(!err.is_permanent());
        assert_eq!(
            err.user_facing_summary(),
            "the model stream dropped before the reply finished"
        );
    }

    /// Lane R19a. A first-byte stall is a NARROWER case of a stream-budget
    /// failure: both predicates must see it, so nothing that already matches
    /// on `is_stream_budget_exceeded` (telemetry, `is_permanent`) loses this
    /// error by accident.
    #[test]
    fn a_first_byte_stall_is_also_a_stream_budget_exceeded_error() {
        let err = ProviderError::first_byte_stall("0 chars in 120 s");
        assert!(err.is_first_byte_provider_stall(), "{err}");
        assert!(err.is_stream_budget_exceeded(), "{err}");
        assert!(err.to_string().contains("0 chars in 120 s"), "{err}");
    }

    /// The general (mid-stream / repetition / wall-clock) stream-budget error
    /// must NOT be mistaken for a first-byte stall — that would make Y8a's
    /// "never retry a runaway" case eligible for the R19a retry-once path.
    #[test]
    fn an_ordinary_stream_budget_error_is_not_a_first_byte_stall() {
        let err = ProviderError::stream_budget_exceeded("the model repeated itself");
        assert!(err.is_stream_budget_exceeded(), "{err}");
        assert!(!err.is_first_byte_provider_stall(), "{err}");
    }

    /// The user-facing text for a first-byte stall must describe what
    /// actually happened — the provider sent nothing, so "the model kept
    /// generating" (the general stream-budget summary) would be a lie.
    #[test]
    fn a_first_byte_stall_gets_its_own_user_facing_summary() {
        let err = ProviderError::first_byte_stall("0 chars in 300 s");
        let summary = err.user_facing_summary();
        assert!(summary.contains("did not send anything back"), "{summary}");
        assert!(!summary.contains("kept generating"), "{summary}");
    }

    /// Lane R19c (G24 LOW). A first-byte stall is retryable by definition —
    /// `agents::agent` retries it once (Lane R19a/R19b) — so it must not
    /// report itself as permanent the way a genuine mid-stream stall or a
    /// repeating model correctly does. Every OTHER caller of `is_permanent()`
    /// (`cost_router::fallback`, the orchestrator, `goal_a2a`) reads this
    /// flag to decide whether a retry/failover is worth trying at all.
    #[test]
    fn a_first_byte_stall_is_not_permanent() {
        let err = ProviderError::first_byte_stall("0 chars in 180 s");
        assert!(!err.is_permanent(), "{err}");
    }

    /// The general (mid-stream / repetition / wall-clock) stream-budget
    /// error is UNCHANGED by the R19c carve-out above: it must stay
    /// permanent, or a caller elsewhere could walk straight back into the
    /// exact runaway Y8a exists to stop.
    #[test]
    fn an_ordinary_stream_budget_error_is_still_permanent() {
        let err = ProviderError::stream_budget_exceeded("the model repeated itself");
        assert!(err.is_permanent(), "{err}");
    }

    /// Lane R36a: the classifier the session-scoped provider circuit breaker
    /// keys off must recognize every class Lane R28b already retried on.
    #[test]
    fn classify_unavailable_recognizes_every_breaker_worthy_class() {
        assert_eq!(
            classify_unavailable("Your credit balance is too low to access the API"),
            Some("billing")
        );
        assert_eq!(
            classify_unavailable("401 authentication_error: invalid x-api-key"),
            Some("auth")
        );
        assert_eq!(
            classify_unavailable("429 Too Many Requests, rate limit exceeded"),
            Some("rate_limit")
        );
        assert_eq!(
            classify_unavailable("502 Bad Gateway: server error"),
            Some("server_error")
        );
        assert_eq!(
            classify_unavailable("Could not be reached: connection refused"),
            Some("network")
        );
        assert_eq!(
            classify_unavailable("the independent reviewer did not respond within 120s"),
            Some("timeout")
        );
    }

    /// A request-shaped failure (bad model id, malformed body) would recur
    /// identically against ANY provider, so it must never open a breaker.
    #[test]
    fn classify_unavailable_does_not_classify_request_shaped_failures() {
        assert_eq!(
            classify_unavailable("400 invalid_request_error: unknown field 'foo'"),
            None
        );
        assert_eq!(classify_unavailable("model_not_found: no such model"), None);
    }

    /// R46a (harness loop, "A43 analysis" lever 3): a `ProviderError::
    /// ContextLengthExceeded`'s own Display text (what `Agent::
    /// take_context_length_failure` hands back, via `provider_err.
    /// to_string()`) must be recognisable against `CONTEXT_LENGTH_EXCEEDED_
    /// MARKER` and still yield the real numbers embedded in its wrapped
    /// body — the exact regression MiniCPM5-2B's `delegate` calls hit.
    #[test]
    fn extract_context_length_token_counts_reads_a_provider_error_display_string() {
        let err = ProviderError::ContextLengthExceeded(
            r#"{"error":{"type":"exceed_context_size_error","n_prompt_tokens":34930,"n_ctx":32768}}"#
                .to_string(),
        );
        let raw = err.to_string();
        assert!(raw.starts_with(CONTEXT_LENGTH_EXCEEDED_MARKER), "{raw}");
        assert_eq!(
            extract_context_length_token_counts(&raw),
            Some((34930, 32768))
        );
    }

    /// The exact JSON-object shape the real 400 body embeds (quoted keys,
    /// nested under an `"error"` object), matching the raw evidence in
    /// "A43 analysis" (`n_prompt_tokens: 34933, n_ctx: 32768`).
    #[test]
    fn extract_context_length_token_counts_reads_the_real_json_shape() {
        let raw = r#"Context length exceeded: {"error":{"type":"exceed_context_size_error","n_prompt_tokens":34933,"n_ctx":32768}}"#;
        assert_eq!(
            extract_context_length_token_counts(raw),
            Some((34933, 32768))
        );
    }

    /// A different provider's context-length rejection (free text, no
    /// `n_prompt_tokens`/`n_ctx` fields) must not fabricate numbers — callers
    /// need a text fallback for exactly this case.
    #[test]
    fn extract_context_length_token_counts_is_none_for_a_non_llama_cpp_body() {
        assert_eq!(
            extract_context_length_token_counts(
                "This model's maximum context length is 128000 tokens."
            ),
            None
        );
    }
}
