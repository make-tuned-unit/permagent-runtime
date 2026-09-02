//! Exit-signal sweep for open holdings, between close scans.
//!
//! The 15:30 close scan is the primary cadence: it runs the same exit check on
//! the same bars, once per trading day, right after the scanner finishes. This
//! six-hour loop is the safety net for everything outside that window — a
//! daemon that was asleep at 15:30, a weekend restart, a holiday.
//!
//! **It defers rather than duplicates.** Both cadences call the same
//! [`overbought::file_sell_notices`], which refuses to file a second notice for
//! a ticker+rule that already has one open. So a sweep following a close scan
//! files nothing, without either side knowing about the other. The old failure
//! mode this replaces was two surfaces disagreeing: a toast from here and a
//! card from the close scan, on the same holding, computed from different
//! indicators.
//!
//! Holdings only. Watchlist names and Picker picks are not alerted here.

use crate::state::AppState;
use permagent::overbought;
use std::sync::Arc;
use std::time::Duration;

const STARTUP_DELAY: Duration = Duration::from_secs(300);
const TICK: Duration = Duration::from_secs(6 * 3600);

pub fn spawn(state: Arc<AppState>) {
    tracing::info!(
        target: "permagentd::finance",
        "exit-signal sweep armed — files decision-inbox proposals, defers to the close scan"
    );
    tokio::spawn(async move {
        tokio::time::sleep(STARTUP_DELAY).await;
        loop {
            if let Err(e) = sweep_once(&state).await {
                tracing::debug!(target: "permagentd::finance", "sell-signal sweep skipped: {e}");
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

async fn sweep_once(state: &Arc<AppState>) -> Result<(), String> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| e.to_string())?;
    for message in overbought::file_sell_notices(&pool).await? {
        tracing::info!(target: "permagentd::finance", "sell notice filed: {message}");
    }
    Ok(())
}
