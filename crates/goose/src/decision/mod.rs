//! Typed judgments, independent of text providers. Call sites must use
//! `decide_guarded` or `decide_or_fallback`, declare state privacy, and route on
//! distributions. No state text is logged here. Fail awake means returning the
//! caller's deterministic answer with an explicit reason, never inventing one.

mod http;
mod journal;
mod role;
mod types;

pub use http::{endpoint_locality, HttpTimeouts, SystemOneHttpBackend};
pub use journal::{
    fallback_reason_code, DecisionJournal, DecisionJournalEntry, DecisionJournalRow,
    InvalidStateId, StateRef,
};
pub use role::{
    build_judge_backend, configured_judge, resolve_judge, resolve_judge_with_billing, JudgeBilling,
    JudgeConfigError, JudgeProvider, JudgeResolution, JudgeSettings,
};
pub use types::{
    Answer, ChoiceBuilder, DecisionError, DecisionState, Question, QuestionKind, StateClass,
};

use crate::cost_router::metered::{
    finish_metered_call, reserve_metered_call, unreserved_local_call, MeteredCallContext,
    MeteredCallError, MeteredCallFinish, MeteredCallKind, MeteredCallRequest,
};
use crate::session::{Session, SessionManager};
use crate::sovereignty::{DataLocality, EgressKind, EgressRecord};
use async_trait::async_trait;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::Mutex;
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
    fn model_name(&self) -> Option<&str> {
        None
    }
    fn billing(&self) -> JudgeBilling {
        JudgeBilling::PaidApi
    }
    fn data_locality(&self) -> DataLocality;
    /// Implementations must check locality before building requests, using the
    /// same guard as `decide_guarded`. Consumers must enter through that helper.
    async fn decide_with_usage(
        &self,
        state: &DecisionState,
        questions: &[Question],
    ) -> Result<DecisionResponse, DecisionError>;

    /// Paid backends must implement accounting at their physical send boundary.
    async fn decide_with_accounting(
        &self,
        state: &DecisionState,
        questions: &[Question],
        _accounting: Arc<dyn MeteredAccounting>,
    ) -> Result<DecisionResponse, DecisionError> {
        if self.billing() == JudgeBilling::PaidApi {
            return Err(DecisionError::Accounting);
        }
        self.decide_with_usage(state, questions).await
    }

    async fn decide(
        &self,
        state: &DecisionState,
        questions: &[Question],
    ) -> Result<Vec<Answer>, DecisionError> {
        Ok(self.decide_with_usage(state, questions).await?.answers)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecisionUsage {
    pub input_tokens: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecisionResponse {
    pub answers: Vec<Answer>,
    pub usage: DecisionUsage,
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
    Ok(decide_guarded_with_usage(backend, state, questions)
        .await?
        .answers)
}

async fn decide_guarded_with_usage(
    backend: &dyn DecisionBackend,
    state: &DecisionState,
    questions: &[Question],
) -> Result<DecisionResponse, DecisionError> {
    if state.text.trim().is_empty() {
        return Err(DecisionError::Validation("empty state".into()));
    }
    let locality = backend.data_locality();
    if let Err(error) = check_locality(state, locality) {
        if state.class == StateClass::Private && locality == DataLocality::Cloud {
            let audit = crate::sovereignty::record_egress(EgressRecord {
                provider: backend.name().to_string(),
                model: "decision_judge".to_string(),
                session_id: "-".to_string(),
                project_id: None,
                kind: EgressKind::Inference,
                blocked: true,
                content_hash: crate::sovereignty::hash_texts(std::slice::from_ref(&state.text)),
                prompt: None,
            })
            .await;
            if let Err(audit_error) = audit {
                tracing::warn!(
                    target: "sovereignty",
                    backend = backend.name(),
                    error = %format!("{audit_error:#}"),
                    "failed to write decision refusal egress audit row"
                );
            }
        }
        return Err(error);
    }
    validate_questions(questions)?;
    let response = tokio::time::timeout(
        DECISION_DEADLINE,
        backend.decide_with_usage(state, questions),
    )
    .await
    .map_err(|_| DecisionError::Timeout)??;
    if response.answers.len() != questions.len() {
        return Err(DecisionError::MalformedResponse);
    }
    for (answer, question) in response.answers.iter().zip(questions) {
        answer.validate(question)?;
    }
    Ok(response)
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
    fn billing(&self) -> JudgeBilling {
        JudgeBilling::LocalFree
    }
    fn name(&self) -> &str {
        "deterministic"
    }

    fn model_name(&self) -> Option<&str> {
        Some("rules")
    }

    fn data_locality(&self) -> DataLocality {
        DataLocality::Local
    }

    async fn decide_with_usage(
        &self,
        state: &DecisionState,
        questions: &[Question],
    ) -> Result<DecisionResponse, DecisionError> {
        check_locality(state, self.data_locality())?;
        validate_questions(questions)?;
        let answers: Vec<Answer> = questions
            .iter()
            .map(|question| {
                let answer = self
                    .rules
                    .get(question.id())
                    .ok_or(DecisionError::MissingRule)?;
                answer.validate(question)?;
                Ok(answer.clone())
            })
            .collect::<Result<Vec<Answer>, DecisionError>>()?;
        Ok(DecisionResponse {
            answers,
            usage: DecisionUsage { input_tokens: None },
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FallbackReason {
    PrimaryAbsent,
    BudgetRefused(BudgetRefusal),
    Auth,
    /// Carries the primary backend's redacted validation message.
    Validation(String),
    RateLimited,
    RateLimitedLocally,
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct DecisionTrace {
    backend_name: String,
    backend_model: String,
    billing: JudgeBilling,
    input_tokens: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
struct DecisionOutcomeWithTrace {
    outcome: DecisionOutcome,
    trace: DecisionTrace,
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
    Ok(
        decide_or_fallback_with_trace(primary, fallback, state, questions)
            .await?
            .outcome,
    )
}

async fn decide_or_fallback_with_trace(
    primary: Option<&dyn DecisionBackend>,
    fallback: &DeterministicBackend,
    state: &DecisionState,
    questions: &[Question],
) -> Result<DecisionOutcomeWithTrace, DecisionError> {
    let reason = match primary {
        None => FallbackReason::PrimaryAbsent,
        Some(backend) => match decide_guarded_with_usage(backend, state, questions).await {
            Ok(response) => {
                return Ok(DecisionOutcomeWithTrace {
                    outcome: DecisionOutcome {
                        answers: response.answers,
                        fallback_reason: None,
                    },
                    trace: DecisionTrace {
                        backend_name: backend.name().to_string(),
                        backend_model: backend.model_name().unwrap_or("decision_judge").to_string(),
                        billing: backend.billing(),
                        input_tokens: response.usage.input_tokens,
                    },
                })
            }
            Err(DecisionError::BudgetRefused(reason)) => FallbackReason::BudgetRefused(reason),
            Err(DecisionError::Auth) => FallbackReason::Auth,
            Err(DecisionError::Validation(message)) => FallbackReason::Validation(message),
            Err(DecisionError::RateLimited) => FallbackReason::RateLimited,
            Err(error) => FallbackReason::PrimaryFailed(error),
        },
    };
    let response = decide_guarded_with_usage(fallback, state, questions).await?;
    Ok(DecisionOutcomeWithTrace {
        outcome: DecisionOutcome {
            answers: response.answers,
            fallback_reason: Some(reason),
        },
        trace: DecisionTrace {
            backend_name: fallback.name().to_string(),
            backend_model: fallback.model_name().unwrap_or("rules").to_string(),
            billing: fallback.billing(),
            input_tokens: response.usage.input_tokens,
        },
    })
}

#[derive(Clone, Debug, PartialEq)]
pub enum BudgetRefusal {
    NoTaskIdentity,
    CeilingExhausted {
        scope: crate::cost_router::budget::BudgetScope,
        spent_usd: f64,
        held_usd: f64,
        requested_usd: f64,
        ceiling_usd: f64,
        approval_required: bool,
    },
}

impl From<MeteredCallError> for DecisionError {
    fn from(error: MeteredCallError) -> Self {
        match error {
            MeteredCallError::MissingTaskIdentity { .. }
            | MeteredCallError::TaskIdentityMismatch { .. } => {
                Self::BudgetRefused(BudgetRefusal::NoTaskIdentity)
            }
            MeteredCallError::CeilingExhausted {
                scope,
                spent_usd,
                held_usd,
                requested_usd,
                ceiling_usd,
                approval_required,
            } => Self::BudgetRefused(BudgetRefusal::CeilingExhausted {
                scope,
                spent_usd,
                held_usd,
                requested_usd,
                ceiling_usd,
                approval_required,
            }),
            // Storage/configuration details may contain sensitive data.
            MeteredCallError::InvalidBound { .. }
            | MeteredCallError::BudgetUnknown { .. }
            | MeteredCallError::InvocationNotFresh { .. }
            | MeteredCallError::Storage(_) => Self::Accounting,
        }
    }
}

#[async_trait]
pub trait MeteredAccounting: Send + Sync {
    async fn reserve(
        &self,
        request: MeteredCallRequest,
    ) -> Result<MeteredCallContext, MeteredCallError>;
    async fn finish(
        &self,
        call: &MeteredCallContext,
        finish: MeteredCallFinish,
    ) -> Result<(), MeteredCallError>;
}

pub struct SessionMeteredAccounting {
    manager: Arc<SessionManager>,
    session: Session,
}

impl SessionMeteredAccounting {
    pub fn new(manager: Arc<SessionManager>, session: Session) -> Self {
        Self { manager, session }
    }
}

#[async_trait]
impl MeteredAccounting for SessionMeteredAccounting {
    async fn reserve(
        &self,
        request: MeteredCallRequest,
    ) -> Result<MeteredCallContext, MeteredCallError> {
        reserve_metered_call(Arc::clone(&self.manager), &self.session, request).await
    }

    async fn finish(
        &self,
        call: &MeteredCallContext,
        finish: MeteredCallFinish,
    ) -> Result<(), MeteredCallError> {
        finish_metered_call(Arc::clone(&self.manager), call, finish).await
    }
}

struct AccountedBackend<'a> {
    backend: &'a dyn DecisionBackend,
    accounting: Arc<dyn MeteredAccounting>,
}

#[async_trait]
impl DecisionBackend for AccountedBackend<'_> {
    fn billing(&self) -> JudgeBilling {
        self.backend.billing()
    }
    fn name(&self) -> &str {
        self.backend.name()
    }
    fn model_name(&self) -> Option<&str> {
        self.backend.model_name()
    }
    fn data_locality(&self) -> DataLocality {
        self.backend.data_locality()
    }

    async fn decide_with_usage(
        &self,
        state: &DecisionState,
        questions: &[Question],
    ) -> Result<DecisionResponse, DecisionError> {
        self.backend
            .decide_with_accounting(state, questions, Arc::clone(&self.accounting))
            .await
    }
}

#[derive(Clone, Debug)]
pub struct DecisionContext {
    pub session_id: String,
    /// Caller hint retained for compatibility; accounting uses the persisted session type.
    pub is_headless: bool,
    pub acted: bool,
}

pub trait DecisionClock: Send + Sync {
    fn now_seconds(&self) -> f64;
}

#[derive(Debug)]
pub struct SystemDecisionClock {
    started: std::time::Instant,
}

impl Default for SystemDecisionClock {
    fn default() -> Self {
        Self {
            started: std::time::Instant::now(),
        }
    }
}

impl DecisionClock for SystemDecisionClock {
    fn now_seconds(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }
}

#[derive(Debug)]
struct TokenBucket {
    tokens: f64,
    last_refill_seconds: f64,
}

pub trait DecisionRateLimiter: Send + Sync {
    fn allow(&self, site: &str) -> bool;
}

/// Each site has its own bucket: N sites can consume N × capacity, with no
/// global envelope. TODO(Astra): decide whether to add a shared global limit.
pub struct LocalDecisionRateLimiter {
    capacity: f64,
    refill_per_minute: f64,
    clock: Arc<dyn DecisionClock>,
    buckets: Mutex<HashMap<String, TokenBucket>>,
}

impl LocalDecisionRateLimiter {
    pub fn from_env() -> Self {
        Self::new_with_clock(
            decision_rate_capacity() as f64,
            decision_rate_per_minute() as f64,
            Arc::new(SystemDecisionClock::default()),
        )
    }

    pub fn new_with_clock(
        capacity: f64,
        refill_per_minute: f64,
        clock: Arc<dyn DecisionClock>,
    ) -> Self {
        Self {
            capacity: capacity.max(1.0),
            refill_per_minute: refill_per_minute.max(1.0),
            clock,
            buckets: Mutex::new(HashMap::new()),
        }
    }
}

impl DecisionRateLimiter for LocalDecisionRateLimiter {
    fn allow(&self, site: &str) -> bool {
        let now = self.clock.now_seconds();
        let capacity = self.capacity;
        let refill_per_minute = self.refill_per_minute;
        let mut buckets = self.buckets.lock().expect("rate limiter mutex poisoned");
        let bucket = buckets.entry(site.to_string()).or_insert(TokenBucket {
            tokens: capacity,
            last_refill_seconds: now,
        });
        let elapsed = (now - bucket.last_refill_seconds).max(0.0);
        let refill = elapsed * (refill_per_minute / 60.0);
        bucket.tokens = (bucket.tokens + refill).min(capacity);
        bucket.last_refill_seconds = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Where a decision is being made: the call site, the version of its
/// questions, and a REFERENCE to the state (never the text).
pub struct DecisionSite<'a> {
    pub site: &'a str,
    pub question_version: &'a str,
    pub state_ref: &'a StateRef,
}

/// The machinery a journaled decision runs on.
pub struct JudgeRuntime<'a> {
    pub journal: &'a DecisionJournal,
    pub rate_limiter: &'a dyn DecisionRateLimiter,
    pub session_manager: &'a Arc<SessionManager>,
}

pub async fn decide_journaled(
    at: DecisionSite<'_>,
    primary: Option<&dyn DecisionBackend>,
    fallback: &DeterministicBackend,
    state: &DecisionState,
    questions: &[Question],
    runtime: JudgeRuntime<'_>,
    context: &DecisionContext,
) -> Result<DecisionOutcome, DecisionError> {
    let DecisionSite {
        site,
        question_version,
        state_ref,
    } = at;
    let JudgeRuntime {
        journal,
        rate_limiter,
        session_manager,
    } = runtime;
    let started = std::time::Instant::now();
    let session = session_manager
        .get_session(&context.session_id, false)
        .await
        .map_err(|_| DecisionError::Accounting)?;
    let accounted = primary.map(|backend| AccountedBackend {
        backend,
        accounting: Arc::new(SessionMeteredAccounting::new(
            Arc::clone(session_manager),
            session.clone(),
        )),
    });
    let primary = accounted
        .as_ref()
        .map(|backend| backend as &dyn DecisionBackend);
    let traced = if !rate_limiter.allow(site) {
        let response = decide_guarded_with_usage(fallback, state, questions).await?;
        DecisionOutcomeWithTrace {
            outcome: DecisionOutcome {
                answers: response.answers,
                fallback_reason: Some(FallbackReason::RateLimitedLocally),
            },
            trace: DecisionTrace {
                backend_name: fallback.name().to_string(),
                backend_model: fallback.model_name().unwrap_or("rules").to_string(),
                billing: fallback.billing(),
                input_tokens: response.usage.input_tokens,
            },
        }
    } else {
        decide_or_fallback_with_trace(primary, fallback, state, questions).await?
    };

    if traced.trace.billing == JudgeBilling::LocalFree {
        let estimated_tokens = estimate_input_tokens(state.text.chars().count());
        let (input_tokens, is_estimated) = match traced.trace.input_tokens {
            Some(tokens) => (tokens, false),
            None => (estimated_tokens, true),
        };
        let call = unreserved_local_call(
            &session,
            uuid::Uuid::now_v7().to_string(),
            MeteredCallKind::Decision,
            traced.trace.backend_name.clone(),
            Some(traced.trace.backend_model.clone()),
            estimated_tokens as u64,
        );
        finish_metered_call(
            Arc::clone(session_manager),
            &call,
            MeteredCallFinish::Usage {
                input_tokens: u64::try_from(input_tokens).map_err(|_| DecisionError::Accounting)?,
                is_estimated,
            },
        )
        .await?;
    }

    let backend = traced
        .outcome
        .fallback_reason
        .as_ref()
        .map(|reason| format!("fallback:{}", fallback_reason_code(reason)))
        .unwrap_or_else(|| traced.trace.backend_name.clone());
    let entry = DecisionJournalEntry {
        site: site.to_string(),
        question_version: question_version.to_string(),
        state_ref: state_ref.clone(),
        backend,
        distribution: traced.outcome.answers.clone(),
        acted: context.acted,
        fallback_reason: traced.outcome.fallback_reason.clone(),
        latency_ms: started.elapsed().as_millis() as i64,
    };
    journal.record(&entry).await;

    Ok(traced.outcome)
}

fn decision_rate_capacity() -> u32 {
    crate::config::Config::global()
        .get_param::<String>("PERMAGENT_DECISION_RATE_CAPACITY")
        .ok()
        .and_then(|raw| parse_positive_u32(&raw))
        .unwrap_or(60)
}

fn decision_rate_per_minute() -> u32 {
    crate::config::Config::global()
        .get_param::<String>("PERMAGENT_DECISION_RATE_PER_MINUTE")
        .ok()
        .and_then(|raw| parse_positive_u32(&raw))
        .unwrap_or(60)
}

fn parse_positive_u32(raw: &str) -> Option<u32> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let value = trimmed.parse::<u32>().ok()?;
    if value == 0 {
        None
    } else {
        Some(value)
    }
}

fn estimate_input_tokens(char_len: usize) -> i64 {
    let estimated = (char_len.saturating_add(3)) / 4;
    i64::try_from(estimated).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests;
