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

    /// A short, human sentence safe to show a user or read aloud — never the
    /// raw provider payload. `raw` bodies carry request ids, model ids, JSON
    /// braces and support URLs; session 20260823_4 read one of those out loud.
    pub fn user_facing_summary(&self) -> String {
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
}
