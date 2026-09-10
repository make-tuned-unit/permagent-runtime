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
//! Now the hold covers one attempt by default, so the retry loop has to be
//! told what it may spend. The count travels as a task-local rather than as a
//! new argument threaded through `Provider::stream`/`Provider::complete` and
//! every provider implementation: the reserved dispatch seams
//! (`agents::agent`'s primary stream and `agents::reply_parts`'
//! `AccountedFastCompletion::complete_one`) wrap their own provider call in
//! [`with_authorized_attempts`], and the retry loop reads it back with
//! [`authorized_attempts`].
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

tokio::task_local! {
    static AUTHORIZED_ATTEMPTS: u32;
}

/// Run `future` with `attempts` physical provider attempts authorized by the
/// caller's reservation.
pub async fn with_authorized_attempts<F: Future>(attempts: u32, future: F) -> F::Output {
    AUTHORIZED_ATTEMPTS.scope(attempts, future).await
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
    AUTHORIZED_ATTEMPTS.try_with(|attempts| *attempts).ok()
}

/// Would a physical attempt numbered `next_attempt` (1-based) be covered by
/// the enclosing reservation?
///
/// `None` (no reservation scope) authorizes everything — see the module doc.
pub fn attempt_is_authorized(next_attempt: u32) -> bool {
    authorized_attempts().is_none_or(|authorized| next_attempt <= authorized)
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
