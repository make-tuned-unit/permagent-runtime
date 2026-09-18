//! Picker close scan — 15:30 America/New_York on cash-equity trading days.
//!
//! Start the user's scanner half an hour before the NYSE close. When the
//! scan finishes, The Financier asks Opus to name at most one surviving
//! pick for tomorrow — or none. Invented tickers are refused.
//!
//! The same pass also runs the exit check on open holdings. Both halves want
//! the same thing — the day's closing bars, once, on a trading day — and this
//! is where that already happens. The exit check ran on an unrelated six-hour
//! ticker before, which meant a notice tied to "the close" could be computed
//! at 04:00 on a Sunday. It still runs there as a safety net, but this is now
//! the primary cadence, and the shared ticker+rule dedupe keeps the two from
//! ever filing the same card twice.

use crate::state::AppState;
use permagent::financier_close::{self, DailyPick};
use permagent::overbought;
use permagent::picker;
use permagent::trading_calendar;
use std::sync::Arc;
use std::time::Duration;

const STARTUP_DELAY: Duration = Duration::from_secs(90);
const SCAN_POLL: Duration = Duration::from_secs(20);
const SCAN_BUDGET: Duration = Duration::from_secs(45 * 60);

pub fn spawn(state: Arc<AppState>) {
    tracing::info!(
        target: "permagentd::finance",
        "Picker close scan armed — 15:30 ET trading days, Opus judges, no invented picks"
    );
    tokio::spawn(async move {
        tokio::time::sleep(STARTUP_DELAY).await;
        loop {
            let now = chrono::Utc::now();
            if trading_calendar::should_scan(now) {
                if let Err(e) = run_once(&state).await {
                    tracing::warn!(target: "permagentd::finance", "close scan skipped: {e}");
                }
            }
            let wait = trading_calendar::sleep_until_next_window(chrono::Utc::now());
            let secs = wait.num_seconds().clamp(15, 30 * 60) as u64;
            tokio::time::sleep(Duration::from_secs(secs)).await;
        }
    });
}

async fn run_once(state: &Arc<AppState>) -> Result<(), String> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| e.to_string())?;
    financier_close::ensure_schema(&pool).await?;
    let day = trading_calendar::session_day(chrono::Utc::now());
    if financier_close::load_for_day(&pool, &day).await?.is_some() {
        return Ok(());
    }

    // The scan is the PICKER's work — its scanner, its universe, its 45-minute
    // budget — and it announced under `financier`, so the Financier's orb lit
    // for a desk it was only waiting on and the Picker's stayed dark through
    // the one thing it does all day (agent-QA D22, misattribution half).
    announce_scan("working");
    let started = start_and_wait().await;
    announce_scan(if started.is_ok() {
        "available"
    } else {
        "error"
    });

    // From here it IS the Financier: Opus reads the survivors, one pick or
    // none, then the exit check on open holdings.
    announce_judgement("working");
    let pick = match started {
        Err(e) => financier_close::none_pick(&day, format!("{e} No pick invented."), 0),
        Ok(()) => {
            let candidates = financier_close::surviving_candidates()
                .await
                .unwrap_or_default();
            if candidates.is_empty() {
                financier_close::none_pick(
                    &day,
                    "The scanner finished and no name cleared the loop gate. No pick for tomorrow.",
                    0,
                )
            } else {
                match financier_close::judge_with_opus(&day, &candidates).await {
                    Ok(p) => p,
                    Err(e) => financier_close::none_pick(&day, e, candidates.len()),
                }
            }
        }
    };

    financier_close::save(&pool, &pick).await?;
    emit_pick(&pick);

    // The sell side, on the same closing bars. A failure here must not lose
    // the day's pick, which is already saved: log it and finish.
    match overbought::file_sell_notices(&pool).await {
        Ok(filed) => {
            for message in &filed {
                tracing::info!(target: "permagentd::finance", "sell notice filed: {message}");
            }
        }
        Err(e) => {
            tracing::warn!(target: "permagentd::finance", "exit check skipped: {e}");
        }
    }

    announce_judgement("available");
    Ok(())
}

/// The Picker — the close-scan desk. Its own seat in the World keys off this
/// exact id; the display name matches the roster entry.
fn announce_scan(state: &str) {
    permagent::events::emit(permagent::events::agent_state_changed(
        "picker",
        "The Picker",
        state,
    ));
}

/// The Financier — reads the survivors, names at most one pick, files the
/// exit notices. Separate from the scan so neither desk takes credit for the
/// other's minutes.
fn announce_judgement(state: &str) {
    permagent::events::emit(permagent::events::agent_state_changed(
        "financier",
        "The Financier",
        state,
    ));
}

async fn start_and_wait() -> Result<(), String> {
    picker::ensure_running().await?;
    let status = picker::status().await;
    if !status.reachable {
        return Err(status
            .detail
            .unwrap_or_else(|| "the stock scanner is not running".into()));
    }
    if !status.scan_in_progress {
        picker::start_scan().await?;
    }
    let deadline = tokio::time::Instant::now() + SCAN_BUDGET;
    loop {
        let s = picker::status().await;
        if s.reachable && !s.scan_in_progress {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("the scanner did not finish before the close window ended.".into());
        }
        tokio::time::sleep(SCAN_POLL).await;
    }
}

fn emit_pick(pick: &DailyPick) {
    let (title, body) = financier_close::notify_copy(pick);
    tracing::info!(target: "permagentd::finance", "{title}: {body}");
    permagent::events::emit(permagent::events::proactive_nudge(
        "daily_pick",
        pick.ticker.as_deref().unwrap_or("none"),
        &body,
        1,
        &pick.as_of,
        None,
        None,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D22: one pass, two desks. The scan belongs to the Picker and the
    /// judgement to the Financier, and before this every frame in the pass
    /// carried the Financier's id — so Picker work lit the wrong orb and the
    /// Picker's own seat never moved.
    #[tokio::test]
    async fn the_scan_and_the_judgement_announce_as_different_desks() {
        let mut bus = permagent::events::subscribe();
        announce_scan("working");
        announce_judgement("working");

        let mut events = Vec::new();
        for _ in 0..64 {
            match bus.try_recv() {
                Ok(event) => events.push(event),
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                    panic!("the event bus closed before the announcements were read")
                }
            }
        }

        for (agent_id, name) in [("picker", "The Picker"), ("financier", "The Financier")] {
            let announced: Vec<_> = events
                .iter()
                .filter(|event| {
                    event.event_type == permagent::events::PermagentEventType::AgentStateChanged
                        && event.payload["agent_id"] == agent_id
                })
                .collect();
            assert_eq!(
                announced.len(),
                1,
                "{name} must announce exactly once; saw {events:?}"
            );
            assert_eq!(announced[0].payload["name"], name);
            assert_eq!(announced[0].payload["state"], "working");
        }
    }
}
