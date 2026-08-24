//! Overbought sell-signal sweep for open holdings.
//!
//! The Financier computes the signs (Yahoo daily closes, RSI vs the user's
//! threshold). The Watcher delivers the nudge. This loop is the dedicated
//! notify path so a hot holding is not lost behind the Watcher's once-a-day
//! taste budget. The Watcher loop calls the same [`overbought::notify_open_lots`]
//! so either worker can fire; `finance_rsi_alerts` dedups per symbol per day.
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
        "overbought sell-signal sweep armed — Watcher delivers, Financier scores, daily dedup"
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
    let sent = overbought::notify_open_lots(&pool).await?;
    for message in sent {
        tracing::info!(target: "permagentd::finance", "{message}");
    }
    Ok(())
}
