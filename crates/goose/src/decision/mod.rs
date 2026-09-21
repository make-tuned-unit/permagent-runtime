//! Typed judgments, independent of text providers. Call sites must use
//! `decide_guarded` or `decide_or_fallback`, declare state privacy, and route on
//! distributions. No state text is logged here. Fail awake means returning the
//! caller's deterministic answer with an explicit reason, never inventing one.

mod http;
mod types;

pub use http::{endpoint_locality, HttpTimeouts, SystemOneHttpBackend};
pub use types::{
    Answer, ChoiceBuilder, DecisionError, DecisionState, Question, QuestionKind, StateClass,
};

use crate::sovereignty::DataLocality;
use async_trait::async_trait;
use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

/// Measured for ITEMS batched into one state on hosted Jev: realZachi/pg-jev
/// reported 100% at 1-20, 92-98% at 40, 77-94% at 80; reproduced by
/// yodablocks/jev-orderby-bench. For questions about one item on a local backend,
/// this is an ASSUMPTION to measure later, not a measured API limit.
pub const MAX_QUESTIONS_PER_STATE: usize = 20;

/// Overall guard deadline, including all sequential batches. Backend futures
/// must yield; synchronous blocking work is not permitted by this contract.
pub const DECISION_DEADLINE: Duration = Duration::from_secs(30);

#[async_trait]
pub trait DecisionBackend: Send + Sync {
    fn name(&self) -> &str;
    fn data_locality(&self) -> DataLocality;
    /// Implementations must check locality before building requests, using the
    /// same guard as `decide_guarded`. Consumers must enter through that helper.
    async fn decide(
        &self,
        state: &DecisionState,
        questions: &[Question],
    ) -> Result<Vec<Answer>, DecisionError>;
}

fn check_locality(state: &DecisionState, locality: DataLocality) -> Result<(), DecisionError> {
    if state.class == StateClass::Private && locality == DataLocality::Cloud {
        return Err(DecisionError::RefusedByLocality);
    }
    Ok(())
}

fn validate_questions(questions: &[Question]) -> Result<(), DecisionError> {
    let mut ids = HashSet::new();
    for question in questions {
        if !ids.insert(question.id()) {
            return Err(DecisionError::Validation("duplicate question id".into()));
        }
    }
    Ok(())
}

pub async fn decide_guarded(
    backend: &dyn DecisionBackend,
    state: &DecisionState,
    questions: &[Question],
) -> Result<Vec<Answer>, DecisionError> {
    if state.text.trim().is_empty() {
        return Err(DecisionError::Validation("empty state".into()));
    }
    check_locality(state, backend.data_locality())?;
    validate_questions(questions)?;
    let answers = tokio::time::timeout(DECISION_DEADLINE, backend.decide(state, questions))
        .await
        .map_err(|_| DecisionError::Timeout)??;
    if answers.len() != questions.len() {
        return Err(DecisionError::MalformedResponse);
    }
    for (answer, question) in answers.iter().zip(questions) {
        answer.validate(question)?;
    }
    Ok(answers)
}

/// A local, I/O-free table of caller-supplied answers. Missing rules are errors;
/// there is deliberately no default answer or model-selected fallback.
#[derive(Clone, Debug)]
pub struct DeterministicBackend {
    rules: BTreeMap<String, Answer>,
}

impl DeterministicBackend {
    pub fn new(rules: BTreeMap<String, Answer>) -> Self {
        Self { rules }
    }
}

#[async_trait]
impl DecisionBackend for DeterministicBackend {
    fn name(&self) -> &str {
        "deterministic"
    }

    fn data_locality(&self) -> DataLocality {
        DataLocality::Local
    }

    async fn decide(
        &self,
        state: &DecisionState,
        questions: &[Question],
    ) -> Result<Vec<Answer>, DecisionError> {
        check_locality(state, self.data_locality())?;
        validate_questions(questions)?;
        questions
            .iter()
            .map(|question| {
                let answer = self
                    .rules
                    .get(question.id())
                    .ok_or(DecisionError::MissingRule)?;
                answer.validate(question)?;
                Ok(answer.clone())
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FallbackReason {
    PrimaryAbsent,
    Auth,
    /// Carries the primary backend's redacted validation message.
    Validation(String),
    RateLimited,
    PrimaryFailed(DecisionError),
}

impl FallbackReason {
    /// Configuration failures should alert callers; rate limits are transient.
    pub fn is_configuration_error(&self) -> bool {
        matches!(
            self,
            Self::Auth
                | Self::Validation(_)
                | Self::PrimaryFailed(
                    DecisionError::Auth
                        | DecisionError::Validation(_)
                        | DecisionError::RefusedByLocality
                )
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecisionOutcome {
    pub answers: Vec<Answer>,
    /// Callers should record this reason without copying the state text.
    pub fallback_reason: Option<FallbackReason>,
}

/// Every primary failure fails awake with an explicit reason for the caller.
/// Returns the fallback backend's original error if it cannot supply valid
/// answers (for example, `MissingRule` or validation failures).
pub async fn decide_or_fallback(
    primary: Option<&dyn DecisionBackend>,
    fallback: &DeterministicBackend,
    state: &DecisionState,
    questions: &[Question],
) -> Result<DecisionOutcome, DecisionError> {
    let reason = match primary {
        None => FallbackReason::PrimaryAbsent,
        Some(backend) => match decide_guarded(backend, state, questions).await {
            Ok(answers) => {
                return Ok(DecisionOutcome {
                    answers,
                    fallback_reason: None,
                })
            }
            Err(DecisionError::Auth) => FallbackReason::Auth,
            Err(DecisionError::Validation(message)) => FallbackReason::Validation(message),
            Err(DecisionError::RateLimited) => FallbackReason::RateLimited,
            Err(error) => FallbackReason::PrimaryFailed(error),
        },
    };
    Ok(DecisionOutcome {
        answers: decide_guarded(fallback, state, questions).await?,
        fallback_reason: Some(reason),
    })
}

#[cfg(test)]
mod tests;
