//! The nightly re-evaluation of open measurement windows.
//!
//! Read-only over `analytics_events`, writing only `growth_action_outcomes`
//! rows and the owning action's status. It never fetches, never calls a model,
//! and never mutates a repo — the arithmetic is entirely local
//! (`permagent::growth::sweep`).
//!
//! The pass also measures an ARCHIVED action that still owes a window, and
//! never moves an archived action's status. Those two together are what make
//! filing a card away safe: the remaining windows are still written, so the
//! data point the archive exists to keep survives, and the card the user
//! cleared off the board is not pushed back onto it on the next tick.
//!
//! Deliberately a plain loop rather than an `automation` starter recipe. The
//! starters run an LLM against a prompt; a verdict produced that way would be
//! self-assessed prose, and the proposal is explicit that grading "must never be
//! self-assessed prose. It is computed from `growth_action_outcomes`, or it is
//! not a grade."

use crate::state::AppState;
use permagent::growth::sweep;
use std::sync::Arc;
use std::time::Duration;

/// Let boot settle. The analytics drain runs first and may backfill events a
/// window is waiting on, and there is no hurry: nothing here is due more often
/// than once a week per action.
const STARTUP_DELAY: Duration = Duration::from_secs(600);

/// Windows close on day boundaries, so anything faster than this re-reads the
/// same numbers. Four passes a day means a window that closed at 00:00 UTC is
/// judged within six hours.
const TICK: Duration = Duration::from_secs(6 * 3600);

pub fn spawn(state: Arc<AppState>) {
    tokio::spawn(async move {
        tokio::time::sleep(STARTUP_DELAY).await;
        loop {
            run_once(&state).await;
            tokio::time::sleep(TICK).await;
        }
    });
}

async fn run_once(state: &AppState) {
    let pool = match state.session_manager().pool_clone().await {
        Ok(pool) => pool,
        Err(e) => {
            tracing::debug!(target: "permagentd::growth", "measurement pass skipped: {e}");
            return;
        }
    };
    match sweep::run(&pool, chrono::Utc::now()).await {
        // Silence when there was nothing to judge. Most passes find every
        // window still open, and a log line per tick would bury the ones that
        // actually measured something.
        Ok(report) if report.windows_judged == 0 && report.errors.is_empty() => {}
        Ok(report) => {
            tracing::info!(
                target: "permagentd::growth",
                considered = report.actions_considered,
                judged = report.windows_judged,
                completed = report.actions_completed,
                "growth measurement pass"
            );
            for error in &report.errors {
                tracing::warn!(target: "permagentd::growth", "measurement error: {error}");
            }
        }
        Err(e) => tracing::warn!(target: "permagentd::growth", "measurement pass failed: {e}"),
    }
}
