//! Opt-in, process-local quota for native OpenAI physical generation requests.
//! This is not an account-wide TPM guarantee: other processes and provider
//! framing are outside this window. Dollar reservations are never changed here.
//! Set PERMAGENT_OPENAI_TPM_BUDGET to enable (e.g.180000), optionally
//! PERMAGENT_OPENAI_MIN_INTERVAL_MS. Use an explicit wire output cap, e.g.
//! GOOSE_MAX_TOKENS=8192 in a fresh CLI. Other providers/processes are excluded.
use super::base::{MessageStream, Usage};
use super::errors::ProviderError;
use futures::StreamExt;
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::sync::{Mutex, Notify};
use tokio::time::Instant;

const WINDOW: Duration = Duration::from_secs(60);
const BUDGET_KEY: &str = "PERMAGENT_OPENAI_TPM_BUDGET";
const INTERVAL_KEY: &str = "PERMAGENT_OPENAI_MIN_INTERVAL_MS";
static PACER: LazyLock<Arc<Pacer>> = LazyLock::new(|| Arc::new(Pacer::default()));

#[derive(Clone, Copy)]
struct Config {
    budget: u64,
    interval: Duration,
}

fn configured() -> Result<Option<Config>, ProviderError> {
    let raw = std::env::var(BUDGET_KEY).map_err(|error| match error {
        std::env::VarError::NotPresent => None,
        _ => Some(refusal("minute budget is not valid Unicode")),
    });
    let raw = match raw {
        Ok(raw) => raw,
        Err(None) => return Ok(None),
        Err(Some(error)) => return Err(error),
    };
    let interval = match std::env::var(INTERVAL_KEY) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(_) => return Err(refusal("minimum interval is not valid Unicode")),
    };
    parse_config(Some(&raw), interval.as_deref())
}

fn parse_config(
    budget: Option<&str>,
    interval: Option<&str>,
) -> Result<Option<Config>, ProviderError> {
    let Some(raw) = budget else {
        return Ok(None);
    };
    let budget = raw
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| refusal("PERMAGENT_OPENAI_TPM_BUDGET must be a positive integer"))?;
    let interval = interval
        .unwrap_or("0")
        .parse::<u64>()
        .map_err(|_| refusal("PERMAGENT_OPENAI_MIN_INTERVAL_MS must be a nonnegative integer"))?;
    Ok(Some(Config {
        budget,
        interval: Duration::from_millis(interval),
    }))
}

fn refusal(message: &str) -> ProviderError {
    ProviderError::RequestFailed(format!("OpenAI local request quota: {message}"))
}

/// Count serialized bytes, not a guessed tokens/character ratio, plus the
/// explicit wire output bound. Hidden server-side context and media are refused.
fn request_charge(payload: &Value) -> Result<u64, ProviderError> {
    fn present(value: &Value, key: &str) -> bool {
        value.get(key).is_some_and(|value| !value.is_null())
    }
    fn content_is_text(value: &Value) -> bool {
        match value {
            Value::Null | Value::String(_) => true,
            Value::Array(parts) => parts.iter().all(|part| {
                matches!(
                    part.get("type").and_then(Value::as_str),
                    Some("text" | "input_text" | "output_text" | "refusal")
                )
            }),
            _ => false,
        }
    }
    fn input_is_text(value: &Value) -> bool {
        match value {
            Value::String(_) => true,
            Value::Array(items) => items.iter().all(|item| {
                if item.is_string() {
                    return true;
                }
                match item.get("type").and_then(Value::as_str) {
                    None | Some("message") => {
                        item.get("role").is_some()
                            && item.get("content").is_some_and(content_is_text)
                    }
                    Some("function_call") => true,
                    Some("function_call_output") => item.get("output").is_some_and(content_is_text),
                    Some("reasoning") => false, // May refer to opaque server-side reasoning state.
                    _ => false,
                }
            }),
            _ => false,
        }
    }
    // Validate protocol fields, never function-schema property names. A tool
    // schema containing e.g. conversation/file_id is ordinary counted text.
    let hidden = present(payload, "previous_response_id") || present(payload, "conversation");
    let media_output = present(payload, "audio")
        || payload
            .get("modalities")
            .and_then(Value::as_array)
            .is_some_and(|modes| modes.iter().any(|mode| mode.as_str() != Some("text")));
    let input_supported = payload
        .get("input")
        .or_else(|| payload.get("messages"))
        .is_some_and(input_is_text);
    let tools_supported = payload
        .get("tools")
        .filter(|value| !value.is_null())
        .is_none_or(|tools| {
            tools.as_array().is_some_and(|tools| {
                tools
                    .iter()
                    .all(|tool| tool.get("type").and_then(Value::as_str) == Some("function"))
            })
        });
    if hidden || media_output || !input_supported || !tools_supported {
        return Err(refusal("only explicit text/function payloads are supported; media or hidden context cannot be bounded"));
    }
    let output = ["max_output_tokens", "max_completion_tokens", "max_tokens"]
        .iter()
        .filter_map(|key| payload.get(key).and_then(Value::as_u64))
        .max()
        .filter(|n| *n > 0)
        .ok_or_else(|| refusal("an explicit positive output-token bound is required"))?;
    let bytes = serde_json::to_vec(payload)
        .map_err(|_| refusal("cannot size request"))?
        .len() as u64;
    bytes
        .checked_add(output)
        .ok_or_else(|| refusal("request bound overflow"))
}

#[derive(Default)]
struct State {
    next_id: u64,
    entries: VecDeque<Entry>,
    last: Option<Instant>,
}
struct Entry {
    id: u64,
    at: Instant,
    charge: u64,
}
enum Admission {
    Granted(u64),
    WaitUntil(Instant),
}

impl State {
    /// Pure scheduling transition: production supplies the clock; tests can
    /// advance explicit instants without changing Tokio's feature graph.
    fn admission(&mut self, config: Config, charge: u64, now: Instant) -> Admission {
        while self
            .entries
            .front()
            .is_some_and(|entry| now.duration_since(entry.at) >= WINDOW)
        {
            self.entries.pop_front();
        }
        let mut used = self.entries.iter().map(|entry| entry.charge).sum::<u64>();
        let interval_at = self.last.map(|last| last + config.interval).unwrap_or(now);
        if used <= config.budget - charge && interval_at <= now {
            self.next_id += 1;
            let id = self.next_id;
            self.entries.push_back(Entry {
                id,
                at: now,
                charge,
            });
            self.last = Some(now);
            return Admission::Granted(id);
        }
        let mut quota_at = now;
        for entry in &self.entries {
            if used <= config.budget - charge {
                break;
            }
            used -= entry.charge;
            quota_at = entry.at + WINDOW;
        }
        Admission::WaitUntil(interval_at.max(quota_at))
    }
}

#[derive(Default)]
struct Pacer {
    state: Mutex<State>,
    changed: Notify,
}

pub(super) struct Ticket {
    pacer: Arc<Pacer>,
    id: u64,
}
impl Pacer {
    async fn admit(self: &Arc<Self>, config: Config, charge: u64) -> Result<Ticket, ProviderError> {
        if charge > config.budget {
            return Err(refusal("single request bound exceeds configured minute budget; narrow context or increase the explicit local quota"));
        }
        loop {
            // Register before inspecting state: reconciliation between the check
            // and the sleep must not lose its wakeup.
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let admission = self
                .state
                .lock()
                .await
                .admission(config, charge, Instant::now());
            let deadline = match admission {
                Admission::Granted(id) => {
                    tracing::debug!(
                        charge,
                        budget = config.budget,
                        "OpenAI local physical request admitted"
                    );
                    return Ok(Ticket {
                        pacer: self.clone(),
                        id,
                    });
                }
                Admission::WaitUntil(deadline) => deadline,
            };
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => {},
                _ = &mut changed => {},
            }
        }
    }
}

impl Ticket {
    async fn reconcile(&self, actual: u64) {
        let mut state = self.pacer.state.lock().await;
        if let Some(entry) = state.entries.iter_mut().find(|entry| entry.id == self.id) {
            // Only lower a pessimistic local quota charge; never refund a
            // financial reservation, nor erase a failed physical attempt.
            entry.charge = entry.charge.min(actual);
        }
        drop(state);
        self.pacer.changed.notify_waiters();
    }
}

pub(super) async fn admit(
    provider: &str,
    payload: &Value,
) -> Result<Option<Ticket>, ProviderError> {
    if provider != "openai" {
        return Ok(None);
    }
    let Some(config) = configured()? else {
        return Ok(None);
    };
    PACER
        .admit(config, request_charge(payload)?)
        .await
        .map(Some)
}

fn complete_usage(usage: Usage) -> Option<u64> {
    let input = u64::try_from(usage.input_tokens?).ok()?;
    let output = u64::try_from(usage.output_tokens?).ok()?;
    let total = u64::try_from(usage.total_tokens?).ok()?;
    (input.checked_add(output)? == total).then_some(total)
}

/// Called inside the adapter, BEFORE Agent usage estimation. No authoritative
/// final counts, stream failure, early drop, or incomplete response => retain
/// the full local quota charge until its normal 60-second expiry.
pub(super) fn reconcile_stream(ticket: Option<Ticket>, mut stream: MessageStream) -> MessageStream {
    let Some(ticket) = ticket else {
        return stream;
    };
    Box::pin(async_stream::try_stream! {
        let mut actual = None;
        let mut incomplete = false;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if let Some(usage) = &chunk.1 {
                incomplete |= usage.incomplete_reason.is_some();
                actual = complete_usage(usage.usage);
            }
            yield chunk;
        }
        if !incomplete {
            if let Some(actual) = actual { ticket.reconcile(actual).await; }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::super::base::ProviderUsage;
    use super::*;
    use serde_json::json;

    fn config(budget: u64) -> Config {
        Config {
            budget,
            interval: Duration::ZERO,
        }
    }
    fn usage(input: i32, output: i32) -> ProviderUsage {
        ProviderUsage::new(
            "test".into(),
            Usage::new(Some(input), Some(output), Some(input + output)),
        )
    }
    async fn charge(pacer: &Pacer) -> u64 {
        pacer
            .state
            .lock()
            .await
            .entries
            .iter()
            .map(|entry| entry.charge)
            .sum()
    }

    #[test]
    fn disabled_and_invalid_config_and_unbounded_payloads() {
        assert!(parse_config(None, Some("invalid")).unwrap().is_none());
        assert!(parse_config(Some("invalid"), None).is_err());
        assert!(parse_config(Some("0"), None).is_err());
        assert!(parse_config(Some("180000"), Some("1000")).is_ok());
        assert!(request_charge(&json!({"input":"text"})).is_err());
        assert!(request_charge(&json!({"max_output_tokens":8192,"input":[{"type":"input_image","image_url":"https://example.com/image"}]})).is_err());
        assert!(
            request_charge(&json!({"max_output_tokens":8192,"previous_response_id":"hidden"}))
                .is_err()
        );
        let schema = json!({"input":"text","max_output_tokens":8192,"previous_response_id":null,"conversation":null,"tools":[{"type":"function","name":"read_metadata","parameters":{"type":"object","properties":{"file_id":{"type":"string"},"conversation":{"type":"string"},"image_url":{"type":"string"}}}}]});
        assert!(request_charge(&schema).is_ok());
        assert!(request_charge(&json!({"messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.com"}}]}],"max_tokens":8192})).is_err());
        let payload = json!({"input":"héllo","max_output_tokens":8192});
        assert_eq!(
            request_charge(&payload).unwrap(),
            serde_json::to_vec(&payload).unwrap().len() as u64 + 8192
        );
    }

    #[test]
    fn concurrent_physical_attempts_share_window_and_expire() {
        let mut state = State::default();
        let start = Instant::now();
        assert!(matches!(
            state.admission(config(100), 70, start),
            Admission::Granted(1)
        ));
        // Each physical retry asks again; a failed prior POST remains charged.
        for now in [start, start + Duration::from_secs(59)] {
            assert!(
                matches!(state.admission(config(100), 70, now), Admission::WaitUntil(at) if at == start + WINDOW)
            );
        }
        assert!(matches!(
            state.admission(config(100), 70, start + WINDOW),
            Admission::Granted(2)
        ));
        assert_eq!(state.entries.len(), 1);
        let mut interval_state = State::default();
        let cfg = Config {
            budget: 100,
            interval: Duration::from_secs(10),
        };
        assert!(matches!(
            interval_state.admission(cfg, 20, start),
            Admission::Granted(1)
        ));
        assert!(
            matches!(interval_state.admission(cfg, 20, start), Admission::WaitUntil(at) if at == start + Duration::from_secs(10))
        );
        assert!(matches!(
            interval_state.admission(cfg, 20, start + Duration::from_secs(10)),
            Admission::Granted(2)
        ));
    }

    #[tokio::test]
    async fn cancelled_waiter_does_not_reserve_future_quota_or_interval() {
        let pacer = Arc::new(Pacer::default());
        let ticket = pacer.admit(config(100), 90).await.unwrap();
        let waiter = {
            let pacer = pacer.clone();
            tokio::spawn(async move { pacer.admit(config(100), 20).await })
        };
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        waiter.abort();
        assert!(matches!(waiter.await, Err(error) if error.is_cancelled()));
        assert_eq!(charge(&pacer).await, 90);
        ticket.reconcile(10).await;
        tokio::time::timeout(Duration::from_secs(5), pacer.admit(config(100), 80))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(charge(&pacer).await, 90);
        assert!(pacer.admit(config(100), 101).await.is_err());
    }

    #[tokio::test]
    async fn complete_raw_usage_reconciles_and_wakes_waiting_request() {
        let pacer = Arc::new(Pacer::default());
        let ticket = pacer.admit(config(100), 90).await.unwrap();
        let waiter = {
            let pacer = pacer.clone();
            tokio::spawn(async move { pacer.admit(config(100), 70).await })
        };
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        let stream: MessageStream =
            Box::pin(futures::stream::iter(vec![Ok((None, Some(usage(10, 5))))]));
        let mut stream = reconcile_stream(Some(ticket), stream);
        assert!(stream.next().await.is_some());
        assert_eq!(
            charge(&pacer).await,
            90,
            "usage before full exhaustion is insufficient"
        );
        assert!(stream.next().await.is_none());
        tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(charge(&pacer).await, 85);
    }

    #[tokio::test]
    async fn error_drop_incomplete_or_missing_usage_keeps_full_charge() {
        for mode in 0..4 {
            let pacer = Arc::new(Pacer::default());
            let ticket = pacer.admit(config(100), 90).await.unwrap();
            let reported = match mode {
                2 => usage(10, 5).with_incomplete_reason(Some("max_output_tokens".into())),
                3 => ProviderUsage::new("test".into(), Usage::default()),
                _ => usage(10, 5),
            };
            let mut chunks = vec![Ok((None, Some(reported)))];
            if mode == 0 {
                chunks.push(Err(refusal("interrupted stream")));
            }
            let mut stream =
                reconcile_stream(Some(ticket), Box::pin(futures::stream::iter(chunks)));
            stream.next().await.unwrap().unwrap();
            if mode != 1 {
                while stream.next().await.is_some() {}
            }
            drop(stream);
            assert_eq!(charge(&pacer).await, 90, "mode {mode}");
        }
        assert!(complete_usage(Usage::new(Some(1), Some(2), Some(4))).is_none());
        assert!(complete_usage(Usage::new(Some(-1), Some(2), Some(1))).is_none());
    }

    #[tokio::test]
    async fn registered_notification_survives_reconcile_before_sleep() {
        let pacer = Arc::new(Pacer::default());
        let ticket = pacer.admit(config(100), 90).await.unwrap();
        let notification = pacer.changed.notified();
        tokio::pin!(notification);
        notification.as_mut().enable();
        ticket.reconcile(10).await;
        tokio::time::timeout(Duration::from_secs(5), notification)
            .await
            .unwrap();
        pacer.admit(config(100), 80).await.unwrap();
    }
}
