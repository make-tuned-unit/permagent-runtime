use super::types::QuestionKind;
use super::{
    check_locality, validate_questions, Answer, BudgetRefusal, DecisionBackend, DecisionError,
    DecisionResponse, DecisionState, DecisionUsage, MeteredAccounting, MeteredCallContext,
    MeteredCallFinish, MeteredCallKind, MeteredCallRequest, Question, MAX_QUESTIONS_PER_STATE,
};
use crate::sovereignty::DataLocality;
use async_trait::async_trait;
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};
use std::collections::BTreeMap;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use url::{Host, Url};

#[derive(Clone, Copy, Debug)]
pub struct HttpTimeouts {
    pub connect: Duration,
    pub total: Duration,
}

impl Default for HttpTimeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(2),
            total: Duration::from_secs(5),
        }
    }
}

pub fn endpoint_locality(endpoint: &str) -> DataLocality {
    let Ok(url) = Url::parse(endpoint) else {
        return DataLocality::Cloud;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return DataLocality::Cloud;
    }
    let local = match url.host() {
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        Some(Host::Domain(host)) => host == "localhost",
        None => false,
    };
    if local {
        DataLocality::Local
    } else {
        DataLocality::Cloud
    }
}

#[derive(Clone)]
pub struct SystemOneHttpBackend {
    endpoint: String,
    billing: super::JudgeBilling,
    model: String,
    api_key: String,
    client: reqwest::Client,
    accounting: Option<Arc<dyn MeteredAccounting>>,
    input_usd_per_mtok: f64,
}

impl fmt::Debug for SystemOneHttpBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Neither endpoint nor model is printed: misconfiguration can embed a key there.
        f.debug_struct("SystemOneHttpBackend")
            .field("api_key", &"[REDACTED]")
            .field("data_locality", &self.data_locality())
            .finish_non_exhaustive()
    }
}

impl SystemOneHttpBackend {
    pub fn new(
        endpoint: String,
        model: String,
        api_key: String,
        timeouts: HttpTimeouts,
    ) -> Result<Self, DecisionError> {
        if timeouts.connect.is_zero() || timeouts.total.is_zero() {
            return Err(DecisionError::Validation("timeouts must be nonzero".into()));
        }
        let client = reqwest::Client::builder()
            .connect_timeout(timeouts.connect)
            .timeout(timeouts.total)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .resolve(
                "localhost",
                SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            )
            .build()
            .map_err(|_| DecisionError::Unavailable)?;
        Ok(Self {
            endpoint,
            billing: super::JudgeBilling::PaidApi,
            model,
            api_key,
            client,
            accounting: None,
            input_usd_per_mtok: super::role::judge_input_usd_per_mtok(),
        })
    }

    pub fn with_billing(mut self, billing: super::JudgeBilling) -> Self {
        self.billing = billing;
        self
    }

    pub fn with_accounting(mut self, accounting: Arc<dyn MeteredAccounting>) -> Self {
        self.accounting = Some(accounting);
        self
    }

    #[cfg(test)]
    pub(super) fn hosted_test_backend(address: SocketAddr) -> Self {
        let mut backend = Self::new(
            format!("http://judge.test:{}", address.port()),
            "jev-latest".into(),
            String::new(),
            HttpTimeouts::default(),
        )
        .unwrap();
        backend.client = reqwest::Client::builder()
            .resolve("judge.test", address)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        backend
    }

    pub(super) fn request_url(&self) -> Result<Url, DecisionError> {
        let mut url = Url::parse(&self.endpoint).map_err(|_| DecisionError::Unavailable)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(DecisionError::Unavailable);
        }
        let trimmed = url.path().trim_end_matches('/');
        let path = if trimmed.ends_with("/v1/systemone") {
            trimmed.to_owned()
        } else {
            format!("{trimmed}/v1/systemone")
        };
        url.set_path(&path);
        Ok(url)
    }
}

#[async_trait]
impl DecisionBackend for SystemOneHttpBackend {
    fn execution_kind(&self) -> super::DecisionExecutionKind {
        super::DecisionExecutionKind::Model
    }

    fn name(&self) -> &str {
        "systemone"
    }

    fn model_name(&self) -> Option<&str> {
        Some(&self.model)
    }

    fn billing(&self) -> super::JudgeBilling {
        self.billing
    }

    fn data_locality(&self) -> DataLocality {
        endpoint_locality(&self.endpoint)
    }

    async fn decide_with_accounting(
        &self,
        state: &DecisionState,
        questions: &[Question],
        accounting: Arc<dyn MeteredAccounting>,
    ) -> Result<DecisionResponse, DecisionError> {
        // A journaled call always uses its own session, never a stale binding.
        self.clone()
            .with_accounting(accounting)
            .decide_with_usage(state, questions)
            .await
    }

    async fn decide_with_usage(
        &self,
        state: &DecisionState,
        questions: &[Question],
    ) -> Result<DecisionResponse, DecisionError> {
        check_locality(state, self.data_locality())?;
        validate_questions(questions)?;
        let mut answers = Vec::with_capacity(questions.len());
        let mut input_tokens: Option<i64> = None;
        for batch in questions.chunks(MAX_QUESTIONS_PER_STATE) {
            let mut request = self.client.post(self.request_url()?);
            if !self.api_key.trim().is_empty() {
                request = request.bearer_auth(&self.api_key);
            }
            let wire = WireRequest {
                state: &state.text,
                model: &self.model,
                questions: WireQuestions(batch),
            };
            let question_text = serde_json::to_string(&wire.questions)
                .map_err(|_| DecisionError::MalformedResponse)?;
            let chars = state
                .text
                .chars()
                .count()
                .saturating_add(question_text.chars().count());
            let estimate = (chars.saturating_add(3) / 4).max(1) as u64;
            // Build before reserving: serialization/URL errors cannot have sent bytes.
            let request = request.json(&wire).build().map_err(transport_error)?;
            let mut hold = match self.billing {
                super::JudgeBilling::PaidApi => {
                    let accounting = self
                        .accounting
                        .as_ref()
                        .ok_or(DecisionError::BudgetRefused(BudgetRefusal::NoTaskIdentity))?;
                    let call = accounting
                        .reserve(MeteredCallRequest {
                            call_id: uuid::Uuid::now_v7().to_string(),
                            kind: MeteredCallKind::Decision,
                            backend: self.name().to_string(),
                            model: Some(self.model.clone()),
                            estimated_input_tokens: estimate,
                            // One token per UTF-8 byte is a conservative bound for this text.
                            input_token_upper_bound: (state
                                .text
                                .len()
                                .saturating_add(question_text.len())
                                as u64)
                                .max(estimate),
                            input_usd_per_mtok: self.input_usd_per_mtok,
                        })
                        .await?;
                    Some(PendingCall {
                        accounting: Arc::clone(accounting),
                        call: Some(call),
                    })
                }
                super::JudgeBilling::LocalFree => {
                    if let Some(accounting) = self.accounting.as_ref() {
                        let call = accounting
                            .record_local_call(self.name(), &self.model, estimate)
                            .await
                            .map_err(DecisionError::from)?;
                        Some(PendingCall {
                            accounting: Arc::clone(accounting),
                            call: Some(call),
                        })
                    } else {
                        None
                    }
                }
            };
            if let Some(pending) = hold.as_ref() {
                if let Some(call) = pending.call.as_ref() {
                    pending
                        .accounting
                        .mark_dispatch(call)
                        .await
                        .map_err(DecisionError::from)?;
                }
            }
            let mut response = match self.client.execute(request).await {
                Ok(response) => response,
                Err(error) => {
                    // Only connection establishment failures prove no request bytes left.
                    let finish = if error.is_connect() {
                        MeteredCallFinish::NotSent
                    } else {
                        MeteredCallFinish::UsageUnknown
                    };
                    finish_pending(&mut hold, finish).await?;
                    return Err(transport_error(error));
                }
            };
            let status = response.status().as_u16();
            if !(200..300).contains(&status) {
                // A server response alone does not establish billable token usage.
                finish_pending(&mut hold, MeteredCallFinish::UsageUnknown).await?;
                return Err(status_error(status));
            }
            let body = match read_response_body(&mut response).await {
                Ok(body) => body,
                Err(error) => {
                    finish_pending(&mut hold, MeteredCallFinish::UsageUnknown).await?;
                    return Err(error);
                }
            };
            // Settle usage even when the answer distribution is malformed.
            let usage = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|value| value.get("usage")?.get("input_tokens")?.as_u64());
            finish_pending(
                &mut hold,
                MeteredCallFinish::Usage {
                    input_tokens: usage.unwrap_or(estimate),
                    is_estimated: usage.is_none(),
                },
            )
            .await?;
            let parsed = parse_wire_response(&body, batch)?;
            answers.extend(parsed.answers);
            if let Some(batch_tokens) = parsed.usage.input_tokens {
                input_tokens = Some(input_tokens.unwrap_or(0).saturating_add(batch_tokens));
            }
        }
        Ok(DecisionResponse {
            answers,
            usage: DecisionUsage { input_tokens },
        })
    }
}

struct PendingCall {
    accounting: Arc<dyn MeteredAccounting>,
    call: Option<MeteredCallContext>,
}

async fn finish_pending(
    pending: &mut Option<PendingCall>,
    finish: MeteredCallFinish,
) -> Result<(), DecisionError> {
    if let Some(pending) = pending {
        if let Some(call) = pending.call.take() {
            // Settlement failures belong to metered reconciliation, never a second finish.
            if let Err(error) = pending.accounting.finish(&call, finish).await {
                tracing::warn!("failed to finish decision accounting");
                return Err(error.into());
            }
        }
    }
    Ok(())
}

impl Drop for PendingCall {
    fn drop(&mut self) {
        // The overall guard can cancel a batch after sending. Preserve its hold.
        if let Some(call) = self.call.take() {
            let accounting = Arc::clone(&self.accounting);
            tokio::spawn(async move {
                if accounting
                    .finish(&call, MeteredCallFinish::UsageUnknown)
                    .await
                    .is_err()
                {
                    tracing::warn!("failed to mark cancelled decision accounting unknown");
                }
            });
        }
    }
}

fn transport_error(error: reqwest::Error) -> DecisionError {
    if error.is_timeout() {
        DecisionError::Timeout
    } else {
        DecisionError::Unavailable
    }
}

pub(super) const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

async fn read_response_body(response: &mut reqwest::Response) -> Result<String, DecisionError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(DecisionError::MalformedResponse);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(DecisionError::MalformedResponse);
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|_| DecisionError::MalformedResponse)
}

fn status_error(status: u16) -> DecisionError {
    match status {
        401 => DecisionError::Auth,
        422 => DecisionError::Validation("remote rejected the request (HTTP 422)".into()),
        429 => DecisionError::RateLimited,
        500..=599 => DecisionError::Unavailable,
        _ => DecisionError::MalformedResponse,
    }
}

#[derive(Serialize)]
pub(super) struct WireRequest<'a> {
    pub state: &'a str,
    pub model: &'a str,
    pub questions: WireQuestions<'a>,
}

pub(super) struct WireQuestions<'a>(pub &'a [Question]);

impl Serialize for WireQuestions<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for question in self.0 {
            map.serialize_entry(question.id(), &WireQuestion(question))?;
        }
        map.end()
    }
}

struct WireQuestion<'a>(&'a Question);

impl Serialize for WireQuestion<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(3))?;
        let kind = match &self.0.kind {
            QuestionKind::Choice { .. } => "choice",
            QuestionKind::Score { .. } => "score",
            QuestionKind::Noul { .. } => "noul",
        };
        map.serialize_entry("type", kind)?;
        map.serialize_entry("instructions", &self.0.instructions)?;
        match &self.0.kind {
            QuestionKind::Choice { options } => {
                map.serialize_entry("criteria", &OrderedOptions(options))?
            }
            QuestionKind::Score { levels } => map.serialize_entry("criteria", levels)?,
            QuestionKind::Noul {
                yes_means,
                no_means,
            } => {
                map.serialize_entry(
                    "criteria",
                    &BTreeMap::from([("true", yes_means), ("false", no_means)]),
                )?;
            }
        }
        map.end()
    }
}

struct OrderedOptions<'a>(&'a [(String, String)]);

impl Serialize for OrderedOptions<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (key, description) in self.0 {
            map.serialize_entry(key, description)?;
        }
        map.end()
    }
}

#[derive(Deserialize)]
struct WireResponse {
    #[serde(rename = "model")]
    _model: String,
    answers: BTreeMap<String, Answer>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct WireUsage {
    #[serde(rename = "input_tokens")]
    input_tokens: Option<u64>,
    #[serde(default, rename = "output_tokens")]
    _output_tokens: u64,
}

#[cfg(test)]
pub(super) fn parse_response(
    body: &str,
    questions: &[Question],
) -> Result<Vec<Answer>, DecisionError> {
    Ok(parse_wire_response(body, questions)?.answers)
}

fn parse_wire_response(
    body: &str,
    questions: &[Question],
) -> Result<DecisionResponse, DecisionError> {
    let mut response: WireResponse =
        serde_json::from_str(body).map_err(|_| DecisionError::MalformedResponse)?;
    let answers: Vec<Answer> = questions
        .iter()
        .map(|question| {
            let answer = response
                .answers
                .remove(question.id())
                .ok_or(DecisionError::MalformedResponse)?;
            answer.validate(question)?;
            Ok(answer)
        })
        .collect::<Result<Vec<Answer>, DecisionError>>()?;
    let input_tokens = response
        .usage
        .and_then(|usage| usage.input_tokens)
        .map(|tokens| i64::try_from(tokens).unwrap_or(i64::MAX));
    Ok(DecisionResponse {
        answers,
        usage: DecisionUsage { input_tokens },
    })
}
