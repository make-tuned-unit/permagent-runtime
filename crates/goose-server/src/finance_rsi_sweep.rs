//! Overbought sell-signal sweep for open holdings.
//!
//! Not on the 12s/60s Finance GET. A GET paints the board; this loop is the
//! notify path. One pass every few hours, daily dedup per symbol via
//! `finance_rsi_alerts`. Copy is a sell *signal* — never an order.
//!
//! Holdings only. Watchlist names and Picker picks are not alerted here.
//! The Picker ranker is never consulted; this read is Yahoo daily closes.

use crate::state::AppState;
use permagent::finance_ledger::{self, RSI_THRESHOLD_KEY};
use permagent::overbought;
use std::sync::Arc;
use std::time::Duration;

const STARTUP_DELAY: Duration = Duration::from_secs(300);
/// Windows close on day boundaries; four passes a day is enough for a
/// daily-deduped alert without hammering Yahoo on the board poll.
const TICK: Duration = Duration::from_secs(6 * 3600);

pub fn spawn(state: Arc<AppState>) {
    tracing::info!(
        target: "permagentd::finance",
        "overbought sell-signal sweep armed — holdings only, daily dedup, RSI bar from {RSI_THRESHOLD_KEY}"
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
    let threshold = overbought::rsi_threshold();
    let lots = overbought::assess_open_lots(&pool, threshold).await?;
    let day = chrono::Utc::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    for lot in lots {
        if !lot.reading.signal {
            continue;
        }
        if finance_ledger::rsi_alert_seen_today(&pool, &lot.symbol, &day).await? {
            continue;
        }
        let rsi = lot.reading.rsi.unwrap_or(0.0);
        finance_ledger::record_rsi_alert(&pool, &lot.symbol, &day, rsi, threshold).await?;
        let message = lot.reading.summary(&lot.symbol);
        permagent::events::emit(permagent::events::proactive_nudge(
            "sell_signal",
            &lot.symbol,
            &message,
            1,
            &chrono::Utc::now().to_rfc3339(),
            None,
            None,
        ));
        tracing::info!(target: "permagentd::finance", "{message}");
    }
    Ok(())
}
