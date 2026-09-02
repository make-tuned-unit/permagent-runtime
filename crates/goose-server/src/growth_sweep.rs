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

/// This loop is a real worker and said so nowhere: no event of any kind
/// reached the World, so a user had no way to see that measurement exists, let
/// alone that it is running (D18). Same honesty clamp as the Steward's and the
/// Guard's loops — `working` only while the pass genuinely is.
fn announce(state_label: &str) {
    permagent::events::emit(permagent::events::agent_state_changed(
        permagent::growth::GROWTH_MEASUREMENT_FEATURE.id,
        permagent::growth::GROWTH_MEASUREMENT_FEATURE.display_name,
        state_label,
    ));
}

/// Tell the open clients which projects have new verdicts.
///
/// `project_changed(_, "growth_actions")` is the frame the Grow lens already
/// refetches on — every other growth-actions writer emits it from the routes.
/// The nightly pass was the one writer that did not, so the Actions and
/// Results lenses fell back to a 120-second poll for a verdict that had
/// already landed (`GrowView.tsx`'s `VERDICT_POLL_MS` names this exact gap).
/// One frame per PROJECT, and none at all on a pass that judged nothing: a
/// tick-rate announcement would make every open client refetch four times a
/// day to be told nothing changed.
fn announce_measured(report: &sweep::SweepReport) {
    for project_id in &report.projects_judged {
        permagent::events::emit(permagent::events::project_changed(
            project_id,
            "growth_actions",
        ));
    }
}

async fn run_once(state: &AppState) {
    let pool = match state.session_manager().pool_clone().await {
        Ok(pool) => pool,
        Err(e) => {
            tracing::debug!(target: "permagentd::growth", "measurement pass skipped: {e}");
            return;
        }
    };
    announce("working");
    let outcome = sweep::run(&pool, chrono::Utc::now()).await;
    announce(if outcome.is_ok() {
        "available"
    } else {
        "error"
    });
    match outcome {
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
            announce_measured(&report);
        }
        Err(e) => tracing::warn!(target: "permagentd::growth", "measurement pass failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_judged_pass_announces_once_per_project_and_a_quiet_one_says_nothing() {
        let mut bus = permagent::events::subscribe();

        announce_measured(&sweep::SweepReport::default());
        assert!(
            bus.try_recv().is_err(),
            "a pass that judged nothing must not wake every open client"
        );

        announce_measured(&sweep::SweepReport {
            windows_judged: 4,
            projects_judged: vec!["p1".to_string(), "p2".to_string()],
            ..Default::default()
        });
        let mut announced: Vec<String> = Vec::new();
        while let Ok(evt) = bus.try_recv() {
            if evt.payload["change"] == "growth_actions" {
                announced.push(evt.payload["project_id"].as_str().unwrap_or("").to_string());
            }
        }
        assert_eq!(
            announced,
            vec!["p1".to_string(), "p2".to_string()],
            "four windows across two projects is two refetches, one per project"
        );
    }
}
