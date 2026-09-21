use super::types::QuestionKind;
use super::{
    check_locality, validate_questions, Answer, DecisionBackend, DecisionError, DecisionState,
    Question, MAX_QUESTIONS_PER_STATE,
};
use crate::sovereignty::DataLocality;
use async_trait::async_trait;
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};
use std::collections::BTreeMap;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
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

pub struct SystemOneHttpBackend {
    endpoint: String,
    model: String,
    api_key: String,
    client: reqwest::Client,
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
            model,
            api_key,
            client,
        })
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
    fn name(&self) -> &str {
        "systemone"
    }

    fn data_locality(&self) -> DataLocality {
        endpoint_locality(&self.endpoint)
    }

    async fn decide(
        &self,
        state: &DecisionState,
        questions: &[Question],
    ) -> Result<Vec<Answer>, DecisionError> {
        check_locality(state, self.data_locality())?;
        validate_questions(questions)?;
        let mut answers = Vec::with_capacity(questions.len());
        for batch in questions.chunks(MAX_QUESTIONS_PER_STATE) {
            let mut request = self.client.post(self.request_url()?);
            if !self.api_key.trim().is_empty() {
                request = request.bearer_auth(&self.api_key);
            }
            let mut response = request
                .json(&WireRequest {
                    state: &state.text,
                    model: &self.model,
                    questions: WireQuestions(batch),
                })
                .send()
                .await
                .map_err(transport_error)?;
            let status = response.status().as_u16();
            if status == 422 {
                return Err(DecisionError::Validation(
                    "remote rejected the request (HTTP 422)".into(),
                ));
            }
            if !(200..300).contains(&status) {
                return Err(status_error(status));
            }
            let body = read_response_body(&mut response).await?;
            answers.extend(parse_response(&body, batch)?);
        }
        Ok(answers)
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
    #[serde(rename = "usage")]
    _usage: WireUsage,
}

#[derive(Deserialize)]
struct WireUsage {
    #[serde(rename = "input_tokens")]
    _input_tokens: u64,
    #[serde(rename = "output_tokens")]
    _output_tokens: u64,
}

pub(super) fn parse_response(
    body: &str,
    questions: &[Question],
) -> Result<Vec<Answer>, DecisionError> {
    let mut response: WireResponse =
        serde_json::from_str(body).map_err(|_| DecisionError::MalformedResponse)?;
    questions
        .iter()
        .map(|question| {
            let answer = response
                .answers
                .remove(question.id())
                .ok_or(DecisionError::MalformedResponse)?;
            answer.validate(question)?;
            Ok(answer)
        })
        .collect()
}
