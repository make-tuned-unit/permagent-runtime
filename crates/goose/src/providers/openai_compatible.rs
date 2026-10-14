use anyhow::Error;
use async_stream::try_stream;
use futures::{Stream, TryStreamExt};
use reqwest::header::HeaderMap;
use reqwest::{Response, StatusCode};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::pin;
use tokio_stream::StreamExt;
use tokio_util::codec::{FramedRead, LinesCodec};
use tokio_util::io::StreamReader;

use super::api_client::ApiClient;
use super::base::{MessageStream, Provider};
use super::errors::{is_billing_message, ProviderError};
use super::retry::ProviderRetry;
use super::stream_budget::StreamBudget;
use super::utils::{ImageFormat, RequestLog};
use crate::conversation::message::Message;
use crate::cost_router::cache::SystemPromptParts;
use crate::model::ModelConfig;
use crate::providers::formats::openai::{create_request_split, response_to_streaming_message};
use rmcp::model::Tool;

pub struct OpenAiCompatibleProvider {
    name: String,
    /// Client targeted at the base URL (e.g. `https://api.x.ai/v1`)
    api_client: ApiClient,
    model: ModelConfig,
    /// Path prefix prepended to `chat/completions` (e.g. `"deployments/{name}/"` for Azure).
    completions_prefix: String,
}

impl OpenAiCompatibleProvider {
    pub fn new(
        name: String,
        api_client: ApiClient,
        model: ModelConfig,
        completions_prefix: String,
    ) -> Self {
        Self {
            name,
            api_client,
            model,
            completions_prefix,
        }
    }

    fn build_request(
        &self,
        model_config: &ModelConfig,
        system: &SystemPromptParts,
        messages: &[Message],
        tools: &[Tool],
        for_streaming: bool,
    ) -> Result<Value, ProviderError> {
        create_request_split(
            model_config,
            system,
            messages,
            tools,
            &ImageFormat::OpenAi,
            for_streaming,
        )
        .map_err(|e| ProviderError::RequestFailed(format!("Failed to create request: {}", e)))
    }

    /// The one body both `stream` and `stream_split` run; they differ only in
    /// whether the caller had a split prompt to hand.
    async fn stream_parts(
        &self,
        model_config: &ModelConfig,
        session_id: &str,
        system: &SystemPromptParts,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        let payload = self.build_request(model_config, system, messages, tools, true)?;
        let mut log = RequestLog::start(model_config, &payload)?;

        let completions_path = format!("{}chat/completions", self.completions_prefix);
        let response = self
            .with_retry(|| async {
                let resp = self
                    .api_client
                    .response_post(Some(session_id), &completions_path, &payload)
                    .await?;
                handle_status_openai_compat(resp).await
            })
            .await
            .inspect_err(|e| {
                let _ = log.error(e);
            })?;

        stream_openai_compat(response, log)
    }
}

#[async_trait::async_trait]
impl Provider for OpenAiCompatibleProvider {
    // Request/retry operations are direct awaits; no spawned request transport.
    fn supports_native_pretransport_gate(&self) -> bool {
        true
    }

    fn get_name(&self) -> &str {
        &self.name
    }

    fn get_model_config(&self) -> ModelConfig {
        self.model.clone()
    }

    async fn fetch_supported_models(&self) -> Result<Vec<String>, ProviderError> {
        let response = self
            .api_client
            .response_get(None, "models")
            .await
            .map_err(|e| ProviderError::RequestFailed(e.to_string()))?;
        let json = handle_response_openai_compat(response).await?;

        if let Some(err_obj) = json.get("error") {
            let msg = err_obj
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(ProviderError::Authentication(msg.to_string()));
        }

        let arr = json.get("data").and_then(|v| v.as_array()).ok_or_else(|| {
            ProviderError::RequestFailed("Missing 'data' array in models response".to_string())
        })?;
        let mut models: Vec<String> = arr
            .iter()
            .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        models.sort();
        Ok(models)
    }

    async fn stream(
        &self,
        model_config: &ModelConfig,
        session_id: &str,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        self.stream_parts(
            model_config,
            session_id,
            &SystemPromptParts::all_stable(system.to_string()),
            messages,
            tools,
        )
        .await
    }

    /// Overridden so the turn-volatile tail lands AFTER the conversation rather
    /// than inside the leading system message. Every provider behind this struct
    /// (DeepSeek, Z.AI, xAI, Azure, …) caches automatically on an exact prompt
    /// prefix, and the default flattening puts the one block that changes every
    /// turn upstream of the tool schemas and the entire history. See
    /// `formats::openai::create_request_split`.
    async fn stream_split(
        &self,
        model_config: &ModelConfig,
        session_id: &str,
        system: &SystemPromptParts,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        self.stream_parts(model_config, session_id, system, messages, tools)
            .await
    }
}

fn check_context_length_exceeded(text: &str) -> bool {
    let check_phrases = [
        "too long",
        "context length",
        "context_length_exceeded",
        "reduce the length",
        "token count",
        "exceeds",
        "exceed context limit",
        "input length",
        "max_tokens",
        "decrease input length",
        "context limit",
        "maximum prompt length",
    ];
    let text_lower = text.to_lowercase();
    check_phrases
        .iter()
        .any(|phrase| text_lower.contains(phrase))
}

/// How long the provider asked us to wait, from the standard `Retry-After`
/// header.
///
/// Two documented forms (RFC 9110 §10.2.3): whole seconds, or an HTTP-date.
/// Both appear in the wild; OpenAI also sends the non-standard
/// `x-ratelimit-reset-requests` in `1s` / `6m0s` form, which is read as a
/// fallback because it is the only hint some gateways give.
///
/// Returns `None` rather than a zero wait when nothing is parseable, so the
/// caller can tell "the provider said nothing" from "the provider said now".
pub fn parse_retry_after(headers: &HeaderMap) -> Option<std::time::Duration> {
    fn seconds(value: &str) -> Option<u64> {
        value.trim().parse::<u64>().ok()
    }

    fn http_date(value: &str) -> Option<u64> {
        let when = chrono::DateTime::parse_from_rfc2822(value.trim()).ok()?;
        let seconds_away = (when.timestamp() - chrono::Utc::now().timestamp()).max(0);
        Some(seconds_away as u64)
    }

    /// `6m0s`, `1s`, `1m30s` — the shape OpenAI's reset headers use.
    fn go_duration(value: &str) -> Option<u64> {
        let value = value.trim();
        if value.is_empty() || !value.ends_with('s') && !value.ends_with('m') {
            return None;
        }
        let mut total = 0u64;
        let mut digits = String::new();
        let mut saw_unit = false;
        for c in value.chars() {
            if c.is_ascii_digit() {
                digits.push(c);
                continue;
            }
            let n: u64 = digits.parse().ok()?;
            digits.clear();
            match c {
                'm' => total += n * 60,
                's' => total += n,
                'h' => total += n * 3600,
                _ => return None,
            }
            saw_unit = true;
        }
        saw_unit.then_some(total)
    }

    let candidates = [
        "retry-after",
        "x-ratelimit-reset-requests",
        "x-ratelimit-reset-tokens",
    ];
    for name in candidates {
        let Some(raw) = headers.get(name).and_then(|v| v.to_str().ok()) else {
            continue;
        };
        if let Some(secs) = seconds(raw)
            .or_else(|| http_date(raw))
            .or_else(|| go_duration(raw))
        {
            // A provider that says "wait 0" is saying "go now"; keep it as a
            // real answer rather than falling through to our own floor.
            return Some(std::time::Duration::from_secs(secs));
        }
    }
    None
}

pub fn map_http_error_to_provider_error(
    status: StatusCode,
    payload: Option<Value>,
) -> ProviderError {
    map_http_error_with_retry_after(status, payload, None)
}

pub fn map_http_error_with_retry_after(
    status: StatusCode,
    payload: Option<Value>,
    retry_after: Option<std::time::Duration>,
) -> ProviderError {
    let extract_message = || -> String {
        payload
            .as_ref()
            .and_then(|p| {
                p.get("error")
                    .and_then(|e| e.get("message"))
                    .or_else(|| p.get("message"))
                    .and_then(|m| m.as_str())
                    .map(String::from)
            })
            .unwrap_or_else(|| payload.as_ref().map(|p| p.to_string()).unwrap_or_default())
    };

    let error = match status {
        StatusCode::OK => unreachable!("Should not call this function with OK status"),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProviderError::Authentication(format!(
            "Authentication failed. Status: {}. Response: {}",
            status,
            extract_message()
        )),
        StatusCode::NOT_FOUND => {
            ProviderError::RequestFailed(format!("Resource not found (404): {}", extract_message()))
        }
        StatusCode::PAYMENT_REQUIRED => ProviderError::CreditsExhausted {
            details: extract_message(),
            top_up_url: None,
        },
        StatusCode::PAYLOAD_TOO_LARGE => ProviderError::ContextLengthExceeded(extract_message()),
        StatusCode::BAD_REQUEST => {
            let payload_str = extract_message();
            if check_context_length_exceeded(&payload_str) {
                ProviderError::ContextLengthExceeded(payload_str)
            } else if is_billing_message(&payload_str) {
                // Anthropic sends "Your credit balance is too low to access the
                // Anthropic API" as a 400 `invalid_request_error`, not a 402.
                // Typing it here is what keeps it out of the retry loop and out
                // of the user-facing reply (2026-08-23, session 20260823_4).
                ProviderError::CreditsExhausted {
                    details: payload_str,
                    top_up_url: None,
                }
            } else {
                ProviderError::RequestFailed(format!("Bad request (400): {}", payload_str))
            }
        }
        StatusCode::TOO_MANY_REQUESTS => {
            let details = extract_message();
            // OpenAI reports a depleted balance as 429 `insufficient_quota`.
            // Waiting does not top up an account, so this must not be retried
            // as an ordinary rate limit.
            if is_billing_message(&details) {
                ProviderError::CreditsExhausted {
                    details,
                    top_up_url: None,
                }
            } else {
                // Z.AI's `1302` and OpenAI's plain 429 both arrive here. What
                // separates a useful backoff from a guess is `retry_after`,
                // which until 2026-08-25 was never read off the response.
                ProviderError::RateLimitExceeded {
                    details,
                    retry_delay: retry_after,
                }
            }
        }
        _ if status.is_server_error() => {
            ProviderError::ServerError(format!("Server error ({}): {}", status, extract_message()))
        }
        _ => ProviderError::RequestFailed(format!(
            "Request failed with status {}: {}",
            status,
            extract_message()
        )),
    };

    if !status.is_success() {
        tracing::warn!(
            "Provider request failed with status: {}. Payload: {:?}. Returning error: {:?}",
            status,
            payload,
            error
        );
    }

    error
}

pub async fn handle_status_openai_compat(response: Response) -> Result<Response, ProviderError> {
    let status = response.status();
    if !status.is_success() {
        // Read the headers before the body: `text()` consumes the response.
        let retry_after = parse_retry_after(response.headers());
        let body = response.text().await.unwrap_or_default();
        let payload = serde_json::from_str::<Value>(&body).ok();
        return Err(map_http_error_with_retry_after(
            status,
            payload,
            retry_after,
        ));
    }
    Ok(response)
}

pub async fn handle_response_openai_compat(response: Response) -> Result<Value, ProviderError> {
    let response = handle_status_openai_compat(response).await?;

    response.json::<Value>().await.map_err(|e| {
        ProviderError::RequestFailed(format!("Response body is not valid JSON: {}", e))
    })
}

/// Lane R19b (harness loop 2026-09-14, follow-up to R19a's M19 DeepSeek
/// forensics). Wrap the RAW SSE line stream so EVERY line — a genuine
/// `data: …` chunk, an SSE comment line (`: keep-alive`), or a blank
/// keep-alive line — resets the liveness clock, BEFORE
/// `response_to_streaming_message` gets a chance to see it.
///
/// Why this has to sit here and not around the parsed stream: DeepSeek's
/// documented behavior under load is to hold the connection open and send
/// SSE comment/blank keep-alive lines while queued, rather than return 429.
/// `response_to_streaming_message`'s own inner consumption loop
/// (`formats/openai.rs:921-931`, `strip_data_prefix` returns `None` for a
/// line that isn't `data: …`, and `if line.is_none() || … { continue }`)
/// silently discards exactly those lines — no `yield`, so it can absorb an
/// unbounded run of keep-alives inside ONE call to its own stream's
/// `.next()`. Y8a's/R19a's budget was wrapped around THAT `.next()` (the
/// semantic, already-filtered stream), so it could not tell "the provider
/// went silent" from "the provider is alive and keep-aliving while queued" —
/// both looked like one `.next()` call that took too long. This wraps the
/// stream one layer down, at the raw line level, where every keep-alive is
/// still visible, mirroring the existing `ollama::with_line_timeout`'s
/// "raw wire level, not the parsed level" shape for the same class of
/// problem.
///
/// Two phases, matching `StreamBudget`: `first_byte_window` bounds the wait
/// for the FIRST raw line ever seen on the connection; `stall_window` bounds
/// the gap between every line after that — including a run of `:
/// keep-alive` lines, each of which resets it. `content_chars` is
/// `StreamBudget`'s own running content-char counter (updated by the caller
/// after every successful `budget.record`); it is read only to quote it in
/// the error text, so a keep-alived-but-otherwise-silent connection reports
/// two numbers — raw lines received and content chars received — instead of
/// one that conflates them.
pub(crate) fn with_raw_line_liveness(
    stream: impl Stream<Item = anyhow::Result<String>> + Unpin + Send + 'static,
    first_byte_window: Option<Duration>,
    stall_window: Option<Duration>,
    content_chars: Arc<AtomicU64>,
) -> impl Stream<Item = anyhow::Result<String>> + Unpin + Send + 'static {
    // Boxed and pinned so the result is `Unpin` — `response_to_streaming_message`
    // requires its input stream to be (formats/openai.rs:912), and a bare
    // `try_stream!` block (a generator) is not, the same reason
    // `stream_openai_compat` below boxes+pins its own final output.
    Box::pin(try_stream! {
        let mut stream = stream;
        let mut window = first_byte_window.or(stall_window);
        let mut seen_first = false;
        let mut raw_lines: u64 = 0;

        loop {
            let next = match window {
                Some(w) => tokio::time::timeout(w, stream.next()).await.map_err(|_| {
                    let chars = content_chars.load(Ordering::Relaxed);
                    let err: ProviderError = if seen_first {
                        ProviderError::stream_budget_exceeded(format!(
                            "the connection sent nothing — not even an SSE comment or blank \
                             keep-alive line — for longer than {} allows ({}s); {raw_lines} raw \
                             line(s) were received on the connection and {chars} content char(s) \
                             were streamed before the stall. Raise or disable {} (0 = unlimited) \
                             if this model legitimately pauses for longer.",
                            crate::providers::stream_budget::STALL_SECS_ENV,
                            w.as_secs(),
                            crate::providers::stream_budget::STALL_SECS_ENV,
                        ))
                    } else {
                        ProviderError::first_byte_stall(format!(
                            "the connection sent nothing at all — not even a raw SSE line, a \
                             comment, or a blank keep-alive — for longer than {} allows ({}s); \
                             {raw_lines} raw line(s) and {chars} content char(s) were received \
                             before the stream was stopped. Raise or disable {} (0 = unlimited) \
                             if this model or provider legitimately needs longer to start \
                             responding.",
                            crate::providers::stream_budget::FIRST_BYTE_SECS_ENV,
                            w.as_secs(),
                            crate::providers::stream_budget::FIRST_BYTE_SECS_ENV,
                        ))
                    };
                    anyhow::Error::from(err)
                })?,
                None => stream.next().await,
            };

            let Some(item) = next else { break };
            raw_lines += 1;
            seen_first = true;
            window = stall_window;
            yield item?;
        }
    })
}

/// The ONE SSE consumer eight providers share — `openai`, `openai_compatible`
/// (z.ai, x.ai, Azure, …), `openrouter`, `nanogpt`, `githubcopilot`,
/// `databricks` and `tetrate` all route their streams through here. The Y8a
/// budget therefore lives in exactly this function rather than in each caller;
/// `anthropic.rs` keeps its own loop (different framing) and carries the same
/// budget separately.
pub fn stream_openai_compat(
    response: Response,
    mut log: RequestLog,
) -> Result<MessageStream, ProviderError> {
    let stream = response.bytes_stream().map_err(std::io::Error::other);

    Ok(Box::pin(try_stream! {
        let stream_reader = StreamReader::new(stream);
        let framed = FramedRead::new(stream_reader, LinesCodec::new())
            .map_err(Error::from);

        // Y8a: bound the response. See `providers::stream_budget` for the M14
        // runaway (441 s / 12 960 deltas / 3.0 MB of one repeated sentence)
        // this exists to stop.
        let budget_source = StreamBudget::from_env();

        // Lane R19b: liveness ("is the provider still there at all") is now
        // enforced at the RAW LINE level, via `with_raw_line_liveness` — see
        // its doc comment for why a wrap around the semantic (yielded-item)
        // stream below cannot see SSE keep-alives. `content_chars` is shared
        // with the loop below so the raw-line layer's error text can quote
        // content chars received alongside raw lines received.
        let content_chars = Arc::new(AtomicU64::new(0));
        let framed = with_raw_line_liveness(
            framed,
            budget_source.first_byte_window(),
            budget_source.stall_window(),
            Arc::clone(&content_chars),
        );

        let message_stream = response_to_streaming_message(framed);
        pin!(message_stream);
        let mut budget = budget_source;
        loop {
            // Liveness is enforced inside `with_raw_line_liveness` now (Lane
            // R19b) — a stall, including a truly dead connection hiding
            // behind what LOOKS like a long wait for the next semantic item,
            // fires there with the right classification. What is left to
            // bound here is ONLY the total wall clock (Y8a's backstop): a
            // connection that stays alive forever — real content or an
            // endless run of keep-alives — but never produces a full,
            // USABLE reply must still end eventually.
            let next = match budget.remaining() {
                Some(remaining) if !remaining.is_zero() => {
                    tokio::time::timeout(remaining, message_stream.next())
                        .await
                        .map_err(|_| budget.wall_clock_error())?
                }
                Some(_) => Err(budget.wall_clock_error())?,
                None => message_stream.next().await,
            };
            let Some(message) = next else { break };
            let (message, usage) = message.map_err(|e|
                e.downcast::<ProviderError>()
                    .unwrap_or_else(ProviderError::stream_decode)
            )?;
            budget.record(message.as_ref())?;
            content_chars.store(budget.chars_so_far(), Ordering::Relaxed);
            log.write(&message, usage.as_ref().map(|f| f.usage).as_ref())?;
            yield (message, usage);
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Responses-API consumer reuses this wrapper; a connection that never
    /// sends a line must end in a typed first-byte stall, not wait forever.
    #[tokio::test]
    async fn raw_line_liveness_reports_a_silent_connection() {
        let silent = futures::stream::pending::<anyhow::Result<String>>();
        let mut guarded = with_raw_line_liveness(
            silent,
            Some(Duration::from_millis(30)),
            Some(Duration::from_millis(30)),
            Arc::new(AtomicU64::new(0)),
        );
        let err = StreamExt::next(&mut guarded)
            .await
            .expect("an item")
            .expect_err("a stall error");
        assert!(err.downcast_ref::<ProviderError>().is_some(), "{err}");
    }
    use serde_json::json;
    use test_case::test_case;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                reqwest::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        map
    }

    /// The live Z.AI 429 that started this: code 1302, no `Retry-After` at all.
    /// It must still be typed as a rate limit, and it must be honest that the
    /// provider named no wait — the floor is the retry layer's job, not a
    /// number invented here.
    #[test]
    fn zai_1302_is_a_rate_limit_with_no_provider_delay() {
        let error = map_http_error_with_retry_after(
            StatusCode::TOO_MANY_REQUESTS,
            Some(json!({"error": {"code": "1302", "message": "Rate limit reached for requests"}})),
            parse_retry_after(&headers(&[])),
        );
        match error {
            ProviderError::RateLimitExceeded {
                details,
                retry_delay,
            } => {
                assert_eq!(details, "Rate limit reached for requests");
                assert_eq!(retry_delay, None);
            }
            other => panic!("expected RateLimitExceeded, got {other:?}"),
        }
    }

    #[test]
    fn a_retry_after_header_becomes_the_provider_delay() {
        let error = map_http_error_with_retry_after(
            StatusCode::TOO_MANY_REQUESTS,
            Some(json!({"error": {"message": "slow down"}})),
            parse_retry_after(&headers(&[("retry-after", "42")])),
        );
        assert!(matches!(
            error,
            ProviderError::RateLimitExceeded {
                retry_delay: Some(d),
                ..
            } if d == std::time::Duration::from_secs(42)
        ));
    }

    #[test]
    fn retry_after_accepts_the_http_date_form() {
        let when = chrono::Utc::now() + chrono::Duration::seconds(90);
        let parsed = parse_retry_after(&headers(&[("retry-after", &when.to_rfc2822())]));
        let secs = parsed
            .expect("an HTTP-date Retry-After is parseable")
            .as_secs();
        assert!((85..=90).contains(&secs), "got {secs}s");
    }

    /// A date already in the past means "now", not a wildly negative wait.
    #[test]
    fn a_past_retry_after_date_is_zero_not_negative() {
        let when = chrono::Utc::now() - chrono::Duration::seconds(600);
        assert_eq!(
            parse_retry_after(&headers(&[("retry-after", &when.to_rfc2822())])),
            Some(std::time::Duration::from_secs(0))
        );
    }

    #[test]
    fn openai_reset_headers_are_read_when_retry_after_is_absent() {
        assert_eq!(
            parse_retry_after(&headers(&[("x-ratelimit-reset-requests", "6m0s")])),
            Some(std::time::Duration::from_secs(360))
        );
        assert_eq!(
            parse_retry_after(&headers(&[("x-ratelimit-reset-requests", "1s")])),
            Some(std::time::Duration::from_secs(1))
        );
    }

    #[test]
    fn an_unparseable_retry_after_is_none_not_zero() {
        assert_eq!(
            parse_retry_after(&headers(&[("retry-after", "soon")])),
            None
        );
        assert_eq!(parse_retry_after(&headers(&[])), None);
    }

    /// A depleted balance arriving as a 429 must not be retried, `Retry-After`
    /// or not — waiting does not top up an account.
    #[test]
    fn a_billing_429_stays_credits_exhausted_even_with_retry_after() {
        let error = map_http_error_with_retry_after(
            StatusCode::TOO_MANY_REQUESTS,
            Some(
                json!({"error": {"message": "You exceeded your current quota, insufficient_quota"}}),
            ),
            Some(std::time::Duration::from_secs(30)),
        );
        assert!(matches!(error, ProviderError::CreditsExhausted { .. }));
    }

    #[test_case(
        StatusCode::PAYMENT_REQUIRED,
        Some(json!({"error": {"message": "Insufficient credits to complete this request"}})),
        "CreditsExhausted"
        ; "402 with payload"
    )]
    #[test_case(
        StatusCode::PAYMENT_REQUIRED,
        None,
        "CreditsExhausted"
        ; "402 without payload"
    )]
    #[test_case(
        StatusCode::TOO_MANY_REQUESTS,
        Some(json!({"error": {"message": "Rate limit exceeded"}})),
        "RateLimitExceeded"
        ; "429 rate limit"
    )]
    #[test_case(
        StatusCode::UNAUTHORIZED,
        None,
        "Authentication"
        ; "401 unauthorized"
    )]
    #[test_case(
        StatusCode::BAD_REQUEST,
        Some(json!({"error": {"message": "This request exceeds the maximum context length"}})),
        "ContextLengthExceeded"
        ; "400 context length"
    )]
    // ── 2026-08-23, session 20260823_4 ────────────────────────────────────
    // Anthropic sends "out of credit" as a 400 `invalid_request_error`, not a
    // 402. Typed as a generic RequestFailed it was retried 3/3 times and its raw
    // payload was read aloud to the user.
    #[test_case(
        StatusCode::BAD_REQUEST,
        Some(json!({"error": {"type": "invalid_request_error", "message": "Your credit balance is too low to access the Anthropic API. Please go to Plans & Billing to upgrade or purchase credits."}})),
        "CreditsExhausted"
        ; "400 anthropic credit balance too low"
    )]
    // OpenAI sends a depleted balance as a 429. Waiting does not top up an account.
    #[test_case(
        StatusCode::TOO_MANY_REQUESTS,
        Some(json!({"error": {"code": "insufficient_quota", "message": "You exceeded your current quota, please check your plan and billing details."}})),
        "CreditsExhausted"
        ; "429 openai insufficient quota is billing"
    )]
    // …but an ordinary OpenAI throttle links to /account/billing in its own body,
    // and must stay a retryable rate limit.
    #[test_case(
        StatusCode::TOO_MANY_REQUESTS,
        Some(json!({"error": {"message": "Rate limit reached for gpt-4 on requests per min. Please try again in 20s. Please add a payment method to your account to increase your rate limit. Visit https://platform.openai.com/account/billing to add a payment method."}})),
        "RateLimitExceeded"
        ; "429 throttle that mentions billing stays a rate limit"
    )]
    // A 400 that is genuinely malformed still maps to RequestFailed.
    #[test_case(
        StatusCode::BAD_REQUEST,
        Some(json!({"error": {"message": "unknown parameter: 'temperture'"}})),
        "RequestFailed"
        ; "400 malformed request"
    )]
    #[test_case(
        StatusCode::INTERNAL_SERVER_ERROR,
        None,
        "ServerError"
        ; "500 server error"
    )]
    #[test_case(
        StatusCode::NOT_FOUND,
        None,
        "RequestFailed"
        ; "404 not found"
    )]
    #[test_case(
        StatusCode::NOT_FOUND,
        Some(json!({"error": {"message": "model not available"}})),
        "RequestFailed"
        ; "404 with error payload"
    )]
    fn http_status_maps_to_expected_error(
        status: StatusCode,
        payload: Option<Value>,
        expected_variant: &str,
    ) {
        let err = map_http_error_to_provider_error(status, payload);
        let actual = err.telemetry_type();
        let expected_telemetry = match expected_variant {
            "CreditsExhausted" => "credits_exhausted",
            "RateLimitExceeded" => "rate_limit",
            "Authentication" => "auth",
            "ContextLengthExceeded" => "context_length",
            "ServerError" => "server",
            "RequestFailed" => "request",
            other => panic!("Unknown variant: {other}"),
        };
        assert_eq!(
            actual, expected_telemetry,
            "Expected {expected_variant}, got error: {err:?}"
        );
    }

    // ── Y8a: the shared SSE loop is bounded ──────────────────────────────
    //
    // Lane A14's M14 forensics: one z.ai `glm-4.7` response streamed 441 s /
    // 12 960 deltas / 3.0 MB of the same sentence and stopped only when the
    // eval driver killed the process. Both tests below drive the REAL
    // `stream_openai_compat` loop over a body that never ends, and both fail
    // against the pre-Y8a `while let Some(..)` — it has no exit at all.

    fn sse_line(content: &str) -> String {
        format!(
            "data: {}\n",
            json!({
                "choices": [{
                    "index": 0,
                    "delta": {"role": "assistant", "content": content},
                    "finish_reason": serde_json::Value::Null,
                }]
            })
        )
    }

    fn fake_response(body: reqwest::Body) -> Response {
        Response::from(http::Response::new(body))
    }

    fn throwaway_log() -> RequestLog {
        // `RequestLog::start` writes under `Paths::in_state_dir("logs")`; the
        // env guard in each test points that at a tempdir.
        RequestLog::start(&ModelConfig::new_or_fail("y8a-test-model"), &json!({}))
            .expect("the request log opens under the test path root")
    }

    /// The fx04 shape: an endless body repeating one substantial delta. The
    /// repetition detector must end the stream with a permanent, non-retried
    /// stream-budget error.
    #[tokio::test]
    async fn an_endless_repeating_sse_body_is_stopped_by_the_stream_budget() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = env_lock::lock_env([
            ("PERMAGENT_PATH_ROOT", Some(tmp.path().to_str().unwrap())),
            (crate::providers::stream_budget::MAX_SECS_ENV, Some("60")),
            // Unlimited, so there is no ambiguity about which bound fired.
            (
                crate::providers::stream_budget::MAX_OUTPUT_CHARS_ENV,
                Some("0"),
            ),
            (crate::providers::stream_budget::STALL_SECS_ENV, Some("0")),
        ]);

        let chunk =
            sse_line("Good! Cart preview is now updated. Need to check admin and API next.\n\n");
        // `stream::repeat` needs a `Clone` item and `io::Error` is not one, so
        // the endless body is an endless ITERATOR of freshly cloned chunks.
        let body =
            reqwest::Body::wrap_stream(futures::stream::iter(std::iter::repeat_with(move || {
                Ok::<_, std::io::Error>(bytes::Bytes::from(chunk.clone()))
            })));
        let log = throwaway_log();
        let mut stream = stream_openai_compat(fake_response(body), log).expect("stream starts");

        let mut deltas = 0usize;
        let err = loop {
            match StreamExt::next(&mut stream).await {
                Some(Ok(_)) => {
                    deltas += 1;
                    assert!(
                        deltas < 10_000,
                        "the repetition detector must fire long before this"
                    );
                }
                Some(Err(e)) => break e,
                None => panic!("an endless body must never end cleanly"),
            }
        };

        assert!(err.is_stream_budget_exceeded(), "{err}");
        assert!(
            err.is_permanent(),
            "a looping stream must not be retried into the same loop: {err}"
        );
        assert!(err.to_string().contains("looping"), "{err}");
    }

    /// The other half: a body that goes silent forever after one delta. The
    /// INACTIVITY window must end it (G13 H2) — with the total unlimited, so
    /// this pins the stall window on its own and proves a stalled stream is
    /// still bounded when `PERMAGENT_PROVIDER_STREAM_MAX_SECS=0`.
    #[tokio::test]
    async fn a_stalled_sse_body_is_stopped_by_the_inactivity_window() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = env_lock::lock_env([
            ("PERMAGENT_PATH_ROOT", Some(tmp.path().to_str().unwrap())),
            (crate::providers::stream_budget::MAX_SECS_ENV, Some("0")),
            (crate::providers::stream_budget::STALL_SECS_ENV, Some("1")),
            (
                crate::providers::stream_budget::MAX_OUTPUT_CHARS_ENV,
                Some("0"),
            ),
        ]);

        let first = futures::stream::once(async {
            Ok::<_, std::io::Error>(bytes::Bytes::from(sse_line("thinking...")))
        });
        let body = reqwest::Body::wrap_stream(futures::StreamExt::chain(
            first,
            futures::stream::pending::<Result<bytes::Bytes, std::io::Error>>(),
        ));
        let log = throwaway_log();
        let mut stream = stream_openai_compat(fake_response(body), log).expect("stream starts");

        let started = std::time::Instant::now();
        let err = loop {
            match StreamExt::next(&mut stream).await {
                Some(Ok(_)) => {}
                Some(Err(e)) => break e,
                None => panic!("a stalled body must not end cleanly"),
            }
        };
        let elapsed = started.elapsed();

        assert!(err.is_stream_budget_exceeded(), "{err}");
        assert!(err.is_permanent(), "{err}");
        assert!(
            err.to_string()
                .contains(crate::providers::stream_budget::STALL_SECS_ENV),
            "the error must name the budget it broke: {err}"
        );
        // Lane R19a: this stream already delivered its one delta before
        // going silent, so this must NOT be classified as a first-byte
        // stall — it is the ordinary mid-stream case, unchanged by R19a.
        assert!(!err.is_first_byte_provider_stall(), "{err}");
        assert!(
            elapsed < std::time::Duration::from_secs(20),
            "the 1s budget must bite promptly, took {elapsed:?}"
        );
    }

    /// Lane R19a (M19 DeepSeek forensics, harness loop 2026-09-14): a body
    /// that sends NOTHING at all — not even one delta — must be stopped by
    /// the first-byte window, not the (here, disabled) mid-stream stall
    /// window, and must be classified as a first-byte stall so the caller
    /// (`agents::agent`) knows to retry it once.
    #[tokio::test]
    async fn a_body_that_never_sends_anything_is_stopped_by_the_first_byte_window() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = env_lock::lock_env([
            ("PERMAGENT_PATH_ROOT", Some(tmp.path().to_str().unwrap())),
            (crate::providers::stream_budget::MAX_SECS_ENV, Some("0")),
            // Disabled, so a mis-set phase (i.e. this test wrongly using the
            // mid-stream window from the start) would hang instead of
            // silently passing.
            (crate::providers::stream_budget::STALL_SECS_ENV, Some("0")),
            (
                crate::providers::stream_budget::FIRST_BYTE_SECS_ENV,
                Some("1"),
            ),
            (
                crate::providers::stream_budget::MAX_OUTPUT_CHARS_ENV,
                Some("0"),
            ),
        ]);

        let body = reqwest::Body::wrap_stream(futures::stream::pending::<
            Result<bytes::Bytes, std::io::Error>,
        >());
        let log = throwaway_log();
        let mut stream = stream_openai_compat(fake_response(body), log).expect("stream starts");

        let started = std::time::Instant::now();
        let err = match StreamExt::next(&mut stream).await {
            Some(Err(e)) => e,
            Some(Ok(_)) => panic!("a body that never sends anything must never deliver an item"),
            None => panic!("a body that never sends anything must not end cleanly"),
        };
        let elapsed = started.elapsed();

        assert!(err.is_stream_budget_exceeded(), "{err}");
        assert!(err.is_first_byte_provider_stall(), "{err}");
        assert!(
            err.to_string()
                .contains(crate::providers::stream_budget::FIRST_BYTE_SECS_ENV),
            "the error must name the budget it broke: {err}"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(20),
            "the 1s first-byte budget must bite promptly, took {elapsed:?}"
        );
    }

    /// Lane R19b: DeepSeek's documented behavior under load is to hold the
    /// connection open and send SSE comment/blank keep-alive lines rather
    /// than content while queued. Six `: keep-alive` lines, 400ms apart
    /// (2.4s total — more than double the 1s stall window below, so this
    /// proves EVERY keep-alive resets the clock, not just the first one),
    /// then the connection ends cleanly with no content ever sent. None of
    /// that may trip the stall — pre-R19b it would have, because
    /// `response_to_streaming_message` silently discards every one of these
    /// lines (they are not `data: …`) without ever yielding, so a budget
    /// wrapped around the yielded-item stream never saw them at all.
    #[tokio::test]
    async fn keep_alive_comment_lines_reset_the_liveness_clock_and_never_trip_the_stall() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = env_lock::lock_env([
            ("PERMAGENT_PATH_ROOT", Some(tmp.path().to_str().unwrap())),
            (crate::providers::stream_budget::MAX_SECS_ENV, Some("0")),
            (crate::providers::stream_budget::STALL_SECS_ENV, Some("1")),
            (
                crate::providers::stream_budget::FIRST_BYTE_SECS_ENV,
                Some("1"),
            ),
            (
                crate::providers::stream_budget::MAX_OUTPUT_CHARS_ENV,
                Some("0"),
            ),
        ]);

        let interval = std::time::Duration::from_millis(400);
        let body_stream = futures::stream::unfold(0u32, move |i| async move {
            if i >= 6 {
                return None;
            }
            tokio::time::sleep(interval).await;
            Some((
                Ok::<_, std::io::Error>(bytes::Bytes::from(": keep-alive\n")),
                i + 1,
            ))
        });
        let body = reqwest::Body::wrap_stream(body_stream);
        let log = throwaway_log();
        let mut stream = stream_openai_compat(fake_response(body), log).expect("stream starts");

        let started = std::time::Instant::now();
        match StreamExt::next(&mut stream).await {
            Some(Ok(_)) => panic!("no content was ever sent — there is no real delta to yield"),
            Some(Err(e)) => panic!("a run of live keep-alives must never trip the stall: {e}"),
            None => {}
        }
        let elapsed = started.elapsed();
        assert!(
            elapsed >= std::time::Duration::from_secs(2),
            "the keep-alives must have actually been consumed one at a time \
             (proving the raw-line level really saw each one), took {elapsed:?}"
        );
    }
}
