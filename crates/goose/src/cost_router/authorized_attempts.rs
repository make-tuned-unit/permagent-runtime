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
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::session::{CostReservationOutcome, SessionManager};

struct AttemptAuthorization {
    granted: AtomicU32,
    extension: Option<(Arc<SessionManager>, String)>,
    extension_lock: tokio::sync::Mutex<()>,
    #[cfg(test)]
    extension_calls: AtomicU32,
}

impl AttemptAuthorization {
    fn new(attempts: u32, extension: Option<(Arc<SessionManager>, String)>) -> Arc<Self> {
        Arc::new(Self {
            granted: AtomicU32::new(attempts),
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
    authorization.extension_calls.fetch_add(1, Ordering::Relaxed);
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
