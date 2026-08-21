//! RSI-14 heat sweep for open holdings.
//!
//! Not on the 12s/60s Finance GET. A GET paints the board; this loop is the
//! notify path. One pass every few hours, daily dedup per symbol via
//! `finance_rsi_alerts`. Copy names the threshold — never a sell.
//!
//! Holdings only. Watchlist names and Picker picks are not alerted here.
//! The Picker ranker is never consulted; this read is Yahoo daily closes.

use crate::state::AppState;
use permagent::finance_ledger::{self, DEFAULT_RSI_THRESHOLD, RSI_THRESHOLD_KEY};
use permagent::market_data;
use permagent::pick_loop;
use permagent::picker;
use std::sync::Arc;
use std::time::Duration;

const STARTUP_DELAY: Duration = Duration::from_secs(300);
/// Windows close on day boundaries; four passes a day is enough for a
/// daily-deduped alert without hammering Yahoo on the board poll.
const TICK: Duration = Duration::from_secs(6 * 3600);
const MAX_SYMBOLS: usize = 10;

fn threshold() -> f64 {
    permagent::config::Config::global()
        .get_param::<f64>(RSI_THRESHOLD_KEY)
        .unwrap_or(DEFAULT_RSI_THRESHOLD)
}

pub fn spawn(state: Arc<AppState>) {
    tracing::info!(
        target: "permagentd::finance",
        "RSI heat sweep armed — holdings only, daily dedup, threshold from {RSI_THRESHOLD_KEY}"
    );
    tokio::spawn(async move {
        tokio::time::sleep(STARTUP_DELAY).await;
        loop {
            if let Err(e) = sweep_once(&state).await {
                tracing::debug!(target: "permagentd::finance", "RSI sweep skipped: {e}");
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
    let threshold = threshold();
    let mut symbols = open_holding_symbols(&pool).await?;
    symbols.sort();
    symbols.dedup();
    symbols.truncate(MAX_SYMBOLS);

    let day = chrono::Utc::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    for symbol in symbols {
        if finance_ledger::rsi_alert_seen_today(&pool, &symbol, &day).await? {
            continue;
        }
        let Ok(closes) = market_data::daily_closes(&symbol, "6mo").await else {
            continue;
        };
        let Some(rsi) = pick_loop::rsi_14(&closes) else {
            continue;
        };
        if rsi < threshold {
            continue;
        }
        finance_ledger::record_rsi_alert(&pool, &symbol, &day, rsi, threshold).await?;
        // "RSI 78 on SHOP — above your 74 threshold" — never "you should sell".
        let message = format!(
            "RSI {:.0} on {symbol} — above your {:.0} threshold",
            rsi, threshold
        );
        permagent::events::emit(permagent::events::proactive_nudge(
            "rsi_heat",
            &symbol,
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

async fn open_holding_symbols(pool: &sqlx::Pool<sqlx::Sqlite>) -> Result<Vec<String>, String> {
    let picker_trades = picker::trades().await.ok().map(|raw| {
        raw.iter()
            .filter_map(picker::parse_trade_row)
            .collect::<Vec<_>>()
    });
    let mut out = Vec::new();
    match picker_trades {
        Some(trades) if !trades.is_empty() => {
            for t in trades {
                if t.exit_date.is_none() && !out.iter().any(|s| s == &t.ticker) {
                    out.push(t.ticker);
                }
            }
        }
        _ => {
            let positions = finance_ledger::list_positions(pool).await?;
            for p in positions {
                if p.exit_date.is_none() && !out.iter().any(|s| s == &p.symbol) {
                    out.push(p.symbol);
                }
            }
        }
    }
    Ok(out)
}
