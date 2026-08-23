//! Picker close scan — 15:30 America/New_York on cash-equity trading days.
//!
//! Start the user's scanner half an hour before the NYSE close. When the
//! scan finishes, The Financier asks Opus to name at most one surviving
//! pick for tomorrow — or none. Invented tickers are refused.

use crate::state::AppState;
use permagent::financier_close::{self, DailyPick};
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

    permagent::events::emit(permagent::events::agent_state_changed(
        "financier",
        "The Financier",
        "working",
    ));

    let started = start_and_wait().await;
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
    permagent::events::emit(permagent::events::agent_state_changed(
        "financier",
        "The Financier",
        "available",
    ));
    Ok(())
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
