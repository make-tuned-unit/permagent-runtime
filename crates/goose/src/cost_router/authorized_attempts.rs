//! How many physical provider attempts the *enclosing reservation* paid for.
//!
//! Y7 (harness loop, from Lane A12's Q6 measurement). A reservation is a
//! dollar hold taken before dispatch; the provider's retry loop
//! (`providers::retry::ProviderRetry::with_retry_config`) is the only place
//! that can spend more of it, because each physical retry is another billable
//! request. Before Y7 the two agreed by over-reserving: the hold covered the
//! provider's whole retry envelope (5 attempts), so the loop could never
//! exceed it — at the cost of holding ~70–330× the money any call actually
//! settled for (see `cost_router::reservation::RESERVE_ATTEMPTS_ENV`).
//!
//! Now the hold covers two attempts by default, so the retry loop has to be
//! told what it may spend. The count travels as a task-local rather than as a
//! new argument threaded through `Provider::stream`/`Provider::complete` and
//! every provider implementation: the reserved dispatch seams
//! (`agents::agent`'s primary stream and `agents::reply_parts`'
//! `AccountedFastCompletion::complete_one`) wrap their own provider call in
//! an async reservation authorizer. The retry loop reads the granted count
//! and requests a one-attempt extension only when that count is exhausted.
//!
//! `None` — no scope installed — means *no reservation governs this call*:
//! local/free work, a provider's own auxiliary requests (Databricks
//! embeddings, credential refreshes), and any future call site that dispatches
//! outside the reservation seam. Those keep the retry policy's own budget
//! unchanged; they hold no dollars to overspend. This is deliberately
//! fail-open on scope and fail-closed on money: a paid call that goes through
//! the reservation seam always installs a scope, and the seam is the same
//! function that took the hold.

use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use crate::session::{CostReservationOutcome, SessionManager};

struct AttemptAuthorization {
    granted: AtomicU32,
    transport_attempts: AtomicU32,
    ambiguous_transport_failure: AtomicBool,
    extension: Option<(Arc<SessionManager>, String)>,
    extension_lock: tokio::sync::Mutex<()>,
    #[cfg(test)]
    extension_calls: AtomicU32,
}

impl AttemptAuthorization {
    fn new(attempts: u32, extension: Option<(Arc<SessionManager>, String)>) -> Arc<Self> {
        Arc::new(Self {
            granted: AtomicU32::new(attempts),
            transport_attempts: AtomicU32::new(0),
            ambiguous_transport_failure: AtomicBool::new(false),
            extension,
            extension_lock: tokio::sync::Mutex::new(()),
            #[cfg(test)]
            extension_calls: AtomicU32::new(0),
        })
    }
}

tokio::task_local! {
    static AUTHORIZED_ATTEMPTS: Arc<AttemptAuthorization>;
}

/// Run `future` with `attempts` physical provider attempts authorized by the
/// caller's reservation.
pub async fn with_authorized_attempts<F: Future>(attempts: u32, future: F) -> F::Output {
    AUTHORIZED_ATTEMPTS
        .scope(AttemptAuthorization::new(attempts, None), future)
        .await
}

/// [`with_authorized_attempts`] for a caller that may have no paid hold at
/// all: `None` runs `future` untouched, so an unreserved (local/subscription)
/// dispatch keeps the retry policy's own budget.
pub async fn with_authorized_attempts_opt<F: Future>(
    attempts: Option<u32>,
    future: F,
) -> F::Output {
    match attempts {
        Some(attempts) => with_authorized_attempts(attempts, future).await,
        None => future.await,
    }
}

/// How many physical attempts the enclosing reservation authorized, or `None`
/// when no reservation governs this call.
pub fn authorized_attempts() -> Option<u32> {
    AUTHORIZED_ATTEMPTS
        .try_with(|authorization| authorization.granted.load(Ordering::Acquire))
        .ok()
}

/// Would a physical attempt numbered `next_attempt` (1-based) be covered by
/// the enclosing reservation?
///
/// `None` (no reservation scope) authorizes everything — see the module doc.
pub fn attempt_is_authorized(next_attempt: u32) -> bool {
    authorized_attempts().is_none_or(|authorized| next_attempt <= authorized)
}

/// Install an extensible authorization at a hold-taking dispatch seam.
/// No paid hold leaves the enclosing task-local scope untouched.
pub(crate) async fn with_reservation_attempts<F: Future>(
    manager: Arc<SessionManager>,
    invocation_id: String,
    attempts: Option<u32>,
    future: F,
) -> F::Output {
    match attempts {
        Some(attempts) => {
            let authorization = AttemptAuthorization::new(attempts, Some((manager, invocation_id)));
            AUTHORIZED_ATTEMPTS.scope(authorization, future).await
        }
        None => future.await,
    }
}

/// Preserve transport evidence after the authorization scope ends. A later
/// explicit pre-reply rejection can release the hold only if no earlier
/// physical attempt in that hold might have been billed.
pub(crate) async fn with_reservation_attempts_evidence<F: Future>(
    manager: Arc<SessionManager>,
    invocation_id: String,
    attempts: Option<u32>,
    future: F,
) -> (F::Output, bool) {
    let Some(attempts) = attempts else {
        return (future.await, false);
    };
    let authorization = AttemptAuthorization::new(attempts, Some((manager, invocation_id)));
    let output = AUTHORIZED_ATTEMPTS
        .scope(Arc::clone(&authorization), future)
        .await;
    let known_transport = authorization.transport_attempts.load(Ordering::Acquire) > 0
        && !authorization
            .ambiguous_transport_failure
            .load(Ordering::Acquire);
    (output, known_transport)
}

/// Release only a rejected invocation with no output or ambiguous stream retry.
pub(crate) fn is_unbilled_rate_rejection(
    error: &crate::providers::errors::ProviderError,
    transport_rejection_safe: bool,
    stream_committed: bool,
    first_byte_stall_retried: bool,
) -> bool {
    matches!(
        error,
        crate::providers::errors::ProviderError::RateLimitExceeded { .. }
    ) && transport_rejection_safe
        && !stream_committed
        && !first_byte_stall_retried
}

pub(crate) fn record_transport_success() {
    // An accepted HTTP request can have generated billable work even when a
    // later stream frame says rate-limited. Only transport-level rejections
    // prove that dispatch was rejected before generation.
    let _ = AUTHORIZED_ATTEMPTS
        .try_with(|a| a.ambiguous_transport_failure.store(true, Ordering::Release));
}

pub(crate) async fn finalize_missing_usage(
    manager: &SessionManager,
    reservation_id: &str,
    rejected_without_usage: bool,
) -> anyhow::Result<()> {
    if rejected_without_usage {
        manager.release_provider_invocation(reservation_id).await?;
    } else {
        manager
            .mark_provider_invocation_unknown(reservation_id)
            .await?;
    }
    Ok(())
}

pub(crate) fn record_transport_attempt() {
    let _ = AUTHORIZED_ATTEMPTS.try_with(|a| a.transport_attempts.fetch_add(1, Ordering::Relaxed));
}

pub(crate) fn record_transport_failure(error: &crate::providers::errors::ProviderError) {
    use crate::providers::errors::ProviderError;
    // Do not infer non-billing from an absent usage row, a timeout, or a 5xx.
    let rejected = matches!(
        error,
        ProviderError::RateLimitExceeded { .. }
            | ProviderError::Authentication(_)
            | ProviderError::ContextLengthExceeded(_)
            | ProviderError::EndpointNotFound(_)
            | ProviderError::CreditsExhausted { .. }
    );
    if !rejected {
        let _ = AUTHORIZED_ATTEMPTS
            .try_with(|a| a.ambiguous_transport_failure.store(true, Ordering::Release));
    }
}

/// Extend by one only when needed. Serialize the database grant and count
/// update, rechecking after the lock so concurrent checks cannot double-extend.
pub(crate) async fn authorize_attempt(next_attempt: u32) -> Result<(), String> {
    if attempt_is_authorized(next_attempt) {
        return Ok(());
    }
    let authorization = AUTHORIZED_ATTEMPTS.with(Arc::clone);
    let _guard = authorization.extension_lock.lock().await;
    if next_attempt <= authorization.granted.load(Ordering::Acquire) {
        return Ok(());
    }
    let Some((manager, invocation_id)) = &authorization.extension else {
        return Err("no reservation extension authorizer installed".into());
    };
    #[cfg(test)]
    authorization
        .extension_calls
        .fetch_add(1, Ordering::Relaxed);
    match manager.extend_reservation(invocation_id, 1).await {
        Ok(CostReservationOutcome::Granted { .. }) => {
            let granted = authorization.granted.fetch_add(1, Ordering::Release) + 1;
            if next_attempt <= granted {
                Ok(())
            } else {
                Err("one-attempt extension does not cover the requested attempt".into())
            }
        }
        Ok(CostReservationOutcome::Unknown { reason }) => Err(reason),
        Ok(outcome) => Err(format!("{outcome:?}")),
        Err(error) => Err(format!("{error:#}")),
    }
}

#[cfg(test)]
pub(crate) fn extension_calls() -> u32 {
    AUTHORIZED_ATTEMPTS.with(|authorization| authorization.extension_calls.load(Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TransportProvider;

    #[async_trait::async_trait]
    impl crate::providers::base::Provider for TransportProvider {
        fn get_name(&self) -> &str {
            "test"
        }
        fn get_model_config(&self) -> crate::model::ModelConfig {
            crate::model::ModelConfig::new("test").unwrap()
        }
        async fn stream(
            &self,
            _: &crate::model::ModelConfig,
            _: &str,
            _: &str,
            _: &[crate::conversation::message::Message],
            _: &[rmcp::model::Tool],
        ) -> Result<crate::providers::base::MessageStream, crate::providers::errors::ProviderError>
        {
            unreachable!("transport closure is injected into the real retry loop")
        }
    }

    #[tokio::test]
    async fn transport_rejections_release_durable_holds_but_timeouts_and_accepted_requests_do_not()
    {
        use crate::providers::errors::ProviderError;
        use crate::providers::retry::{ProviderRetry, RetryConfig};
        for scenario in [
            "rejected",
            "timeout_then_rejected",
            "accepted_then_stream_rejected",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let manager = Arc::new(SessionManager::new(temp.path().to_path_buf()));
            let session = manager
                .create_session(
                    temp.path().to_path_buf(),
                    "transport test".into(),
                    crate::session::SessionType::User,
                    crate::config::GooseMode::Auto,
                )
                .await
                .unwrap();
            let task = manager.begin_budget_task(&session.id).await.unwrap();
            let reserve = manager
                .reserve_provider_invocation(
                    "first",
                    &session.id,
                    Some(&task),
                    0.5,
                    "2099-01-01T00:00:00Z",
                    &crate::cost_router::budget::BudgetConfig::default(),
                )
                .await
                .unwrap();
            let CostReservationOutcome::Granted { reservation_id } = reserve else {
                panic!("{reserve:?}")
            };
            let calls = AtomicU32::new(0);
            let (result, evidence) = with_reservation_attempts_evidence(
                Arc::clone(&manager),
                "first".into(),
                Some(2),
                TransportProvider.with_retry_config(
                    || async {
                        let attempt = calls.fetch_add(1, Ordering::SeqCst);
                        if scenario == "accepted_then_stream_rejected" {
                            return Ok(());
                        }
                        if scenario == "timeout_then_rejected" && attempt == 0 {
                            return Err(ProviderError::NetworkError(
                                "timeout after dispatch".into(),
                            ));
                        }
                        Err(ProviderError::RateLimitExceeded {
                            details: "HTTP 429".into(),
                            retry_delay: None,
                        })
                    },
                    RetryConfig::new(1, 0, 1.0, 0).with_rate_limit_floor_ms(0),
                ),
            )
            .await;
            let error = result
                .err()
                .unwrap_or_else(|| ProviderError::RateLimitExceeded {
                    details: "in-band rejection after HTTP acceptance".into(),
                    retry_delay: None,
                });
            let release = is_unbilled_rate_rejection(&error, evidence, false, false);
            finalize_missing_usage(&manager, &reservation_id, release)
                .await
                .unwrap();
            let pool = manager.pool_clone().await.unwrap();
            let state: String = sqlx::query_scalar(
                "SELECT state FROM cost_reservations WHERE invocation_id = 'first'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(
                state,
                if scenario == "rejected" {
                    "released"
                } else {
                    "unknown"
                },
                "{scenario}"
            );
            assert!(calls.load(Ordering::SeqCst) <= 2);
            // A retry uses a fresh invocation; release must not consume the
            // budget or synthesize a ledger charge for the rejected request.
            if scenario == "rejected" {
                let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cost_ledger")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
                assert_eq!(count, 0);
                let next = manager
                    .reserve_provider_invocation(
                        "next",
                        &session.id,
                        Some(&task),
                        0.5,
                        "2099-01-01T00:00:00Z",
                        &crate::cost_router::budget::BudgetConfig::default(),
                    )
                    .await
                    .unwrap();
                let CostReservationOutcome::Granted {
                    reservation_id: next_id,
                } = next
                else {
                    panic!("{next:?}")
                };
                let (success, _) = with_reservation_attempts_evidence(
                    Arc::clone(&manager),
                    "next".into(),
                    Some(1),
                    TransportProvider.with_retry_config(
                        || async { Ok::<_, ProviderError>(()) },
                        RetryConfig::new(0, 0, 1.0, 0),
                    ),
                )
                .await;
                success.unwrap();
                let row = crate::session::CostLedgerRow {
                    call_id: "next".into(),
                    ts: "2026-09-26T00:00:00Z".into(),
                    session_id: session.id.clone(),
                    parent_session_id: None,
                    task_id: Some(task.clone()),
                    goal_id: None,
                    subagent_id: None,
                    provider: Some("test".into()),
                    model: Some("test".into()),
                    cost_tier: crate::session::CostTier::PaidApi,
                    is_headless: true,
                    input_tokens: 10,
                    output_tokens: 5,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                    input_cost: 0.02,
                    output_cost: 0.0,
                    cache_read_cost: 0.0,
                    cache_write_cost: 0.0,
                    cost_usd: 0.02,
                    cache_savings_usd: 0.0,
                    is_estimated: false,
                    provenance: Default::default(),
                    outcome: Default::default(),
                };
                manager
                    .settle_provider_invocation(
                        &next_id,
                        &row,
                        None,
                        Some(15),
                        Some(10),
                        Some(5),
                        15,
                        10,
                        5,
                    )
                    .await
                    .unwrap();
                let (state, cost): (String, f64) = sqlx::query_as("SELECT state, settled_cost_usd FROM cost_reservations WHERE invocation_id = 'next'")
                    .fetch_one(&pool).await.unwrap();
                assert_eq!(state, "settled");
                assert_eq!(cost, 0.02);
                let charged: f64 = sqlx::query_scalar("SELECT SUM(cost_usd) FROM cost_ledger")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
                assert_eq!(charged, 0.02);
            }
        }
    }

    async fn rejection_evidence(errors: Vec<crate::providers::errors::ProviderError>) -> bool {
        let authorization = AttemptAuthorization::new(2, None);
        AUTHORIZED_ATTEMPTS
            .scope(Arc::clone(&authorization), async {
                for error in errors {
                    record_transport_attempt();
                    record_transport_failure(&error);
                }
            })
            .await;
        authorization.transport_attempts.load(Ordering::Acquire) > 0
            && !authorization
                .ambiguous_transport_failure
                .load(Ordering::Acquire)
    }

    #[tokio::test]
    async fn explicit_rate_rejections_provide_unambiguous_evidence() {
        use crate::providers::errors::ProviderError;
        assert!(
            rejection_evidence(vec![
                ProviderError::RateLimitExceeded {
                    details: "TPM limit".into(),
                    retry_delay: None,
                },
                ProviderError::RateLimitExceeded {
                    details: "TPM limit".into(),
                    retry_delay: None,
                }
            ])
            .await
        );
    }

    #[tokio::test]
    async fn a_timeout_before_rate_rejection_keeps_the_hold_unknown() {
        use crate::providers::errors::ProviderError;
        assert!(
            !rejection_evidence(vec![
                ProviderError::NetworkError("timeout after dispatch".into()),
                ProviderError::RateLimitExceeded {
                    details: "TPM limit".into(),
                    retry_delay: None
                },
            ])
            .await
        );
        assert!(!rejection_evidence(vec![]).await);
    }

    #[test]
    fn output_or_stalled_stream_prevents_rejection_release() {
        use crate::providers::errors::ProviderError;
        let rate = ProviderError::RateLimitExceeded {
            details: "TPM".into(),
            retry_delay: None,
        };
        assert!(is_unbilled_rate_rejection(&rate, true, false, false));
        assert!(!is_unbilled_rate_rejection(&rate, false, false, false));
        assert!(!is_unbilled_rate_rejection(&rate, true, true, false));
        assert!(!is_unbilled_rate_rejection(&rate, true, false, true));
        assert!(!is_unbilled_rate_rejection(
            &ProviderError::NetworkError("timeout".into()),
            true,
            false,
            false
        ));
    }

    #[tokio::test]
    async fn no_scope_means_no_reservation_governs_the_call() {
        assert_eq!(authorized_attempts(), None);
        assert!(attempt_is_authorized(1));
        assert!(attempt_is_authorized(9));
    }

    #[tokio::test]
    async fn a_scope_authorizes_exactly_the_reserved_attempts() {
        with_authorized_attempts(1, async {
            assert_eq!(authorized_attempts(), Some(1));
            assert!(attempt_is_authorized(1));
            assert!(!attempt_is_authorized(2));
        })
        .await;
        with_authorized_attempts(3, async {
            assert!(attempt_is_authorized(3));
            assert!(!attempt_is_authorized(4));
        })
        .await;
        // The scope ends with the future it wrapped.
        assert_eq!(authorized_attempts(), None);
    }

    #[tokio::test]
    async fn an_absent_count_runs_the_future_unscoped() {
        let seen = with_authorized_attempts_opt(None, async { authorized_attempts() }).await;
        assert_eq!(seen, None);
        let seen = with_authorized_attempts_opt(Some(2), async { authorized_attempts() }).await;
        assert_eq!(seen, Some(2));
    }
}
