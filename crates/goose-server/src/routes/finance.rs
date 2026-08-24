//! Finance tab — the money board.
//!
//! `GET /api/finance` is Polybot status, holdings with live marks, Picker
//! picks run through a Yahoo + financialdatasets + loop-engineering gate,
//! household spend, and research notes. Quotes are never persisted.
//! Polybot start/pause/scan drive the user's bot; that bot can place orders.
//! Keys stay in the keychain.

use crate::state::AppState;
use axum::{
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post},
    Json, Router,
};
use permagent::finance_ledger::{self, NewPosition};
use permagent::finance_statements;
use permagent::financier_close::{self, DailyPick};
use permagent::market_data::{self, FundamentalsError, Quote};
use permagent::overbought::{self, OverboughtReading};
use permagent::pick_loop::{self, LoopGate};
use permagent::picker::{self, TradeEntry, TradeRow};
use permagent::polybot;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Plain-text `(StatusCode, String)` became "Unknown error" in the tab —
/// `apiFetch` only reads JSON `{ message }`. Same shape as growth_actions.
struct ApiError(StatusCode, String);

impl From<(StatusCode, String)> for ApiError {
    fn from((status, message): (StatusCode, String)) -> Self {
        ApiError(status, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "message": self.1 }))).into_response()
    }
}

const MAX_QUOTES: usize = 20;
const MAX_PICKS: usize = 15;
const MAX_LOOP: usize = 8;
const MAX_FILE_SIZE: usize = 20 * 1024 * 1024;
pub use permagent::finance_ledger::{DEFAULT_RSI_THRESHOLD, RSI_THRESHOLD_KEY};

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/finance", get(get_board))
        .route("/api/finance/watchlist", post(add_watchlist))
        .route("/api/finance/watchlist/{symbol}", delete(remove_watchlist))
        .route("/api/finance/notes", post(add_note))
        .route(
            "/api/finance/notes/{id}",
            patch(update_note).delete(delete_note),
        )
        .route("/api/finance/positions", post(add_position))
        .route("/api/finance/positions/{id}/close", post(close_position))
        .route(
            "/api/finance/positions/{id}",
            patch(update_local_position).delete(delete_position),
        )
        .route("/api/finance/picker/start", post(start_picker))
        .route("/api/finance/picker/scan", post(scan_picker))
        .route("/api/finance/polybot/start", post(start_polybot))
        .route("/api/finance/polybot/pause", post(pause_polybot))
        .route("/api/finance/polybot/scan", post(scan_polybot))
        .route("/api/finance/picker/trades", post(record_picker_trade))
        .route(
            "/api/finance/picker/trades/{id}/close",
            post(close_picker_trade),
        )
        .route(
            "/api/finance/picker/trades/{id}",
            patch(update_picker_trade).delete(delete_picker_trade),
        )
        .route(
            "/api/finance/statements",
            post(ingest_statement).layer(DefaultBodyLimit::max(MAX_FILE_SIZE * 2)),
        )
        .route("/api/finance/transactions/{id}", patch(recategorize))
        .with_state(state)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WatchlistRow {
    #[serde(flatten)]
    item: finance_ledger::WatchlistItem,
    quote: Option<QuoteView>,
    quote_error: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct QuoteView {
    symbol: String,
    name: Option<String>,
    currency: Option<String>,
    exchange: Option<String>,
    price: Option<f64>,
    previous_close: Option<f64>,
    change: Option<f64>,
    change_percent: Option<f64>,
    day_high: Option<f64>,
    day_low: Option<f64>,
    fifty_two_week_high: Option<f64>,
    fifty_two_week_low: Option<f64>,
    volume: Option<u64>,
    quoted_at: Option<String>,
    market_closed: bool,
}

impl From<Quote> for QuoteView {
    fn from(q: Quote) -> Self {
        Self {
            symbol: q.symbol,
            name: q.name,
            currency: q.currency,
            exchange: q.exchange,
            price: q.price,
            previous_close: q.previous_close,
            change: q.change,
            change_percent: q.change_percent,
            day_high: q.day_high,
            day_low: q.day_low,
            fifty_two_week_high: q.fifty_two_week_high,
            fifty_two_week_low: q.fifty_two_week_low,
            volume: q.volume,
            quoted_at: q.quoted_at,
            market_closed: q.market_closed,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PickerView {
    reachable: bool,
    base_url: String,
    scan_in_progress: bool,
    scan_date: Option<String>,
    results: Option<u64>,
    detail: Option<String>,
}

impl From<picker::PickerStatus> for PickerView {
    fn from(p: picker::PickerStatus) -> Self {
        Self {
            reachable: p.reachable,
            base_url: p.base_url,
            scan_in_progress: p.scan_in_progress,
            scan_date: p.scan_date,
            results: p.results,
            detail: p.detail,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HoldingRow {
    id: String,
    symbol: String,
    company_name: String,
    entry_date: String,
    entry_price: f64,
    shares: i64,
    exit_date: Option<String>,
    exit_price: Option<f64>,
    notes: Option<String>,
    source: String,
    quote: Option<QuoteView>,
    quote_error: Option<String>,
    last: Option<f64>,
    unrealized: Option<f64>,
    unrealized_pct: Option<f64>,
    realized: Option<f64>,
    rsi: Option<f64>,
    sell_signal: bool,
    overbought_signs: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HoldingsView {
    source: String,
    open_count: usize,
    net_unrealized: f64,
    net_realized: f64,
    net_pnl: f64,
    rows: Vec<HoldingRow>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidatedPick {
    ticker: String,
    company_name: Option<String>,
    rank: Option<i64>,
    score: Option<f64>,
    tier: Option<String>,
    picker_rsi: Option<f64>,
    picker_price: Option<f64>,
    confidence: Option<f64>,
    buy_window: Option<String>,
    reason: Option<String>,
    quote: Option<QuoteView>,
    quote_error: Option<String>,
    price_mismatch: bool,
    fundamentals: FundamentalsView,
    #[serde(rename = "loop")]
    loop_gate: Option<LoopGate>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FundamentalsView {
    available: bool,
    summary: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SellSignal {
    symbol: String,
    rsi: Option<f64>,
    rsi_threshold: f64,
    signs: Vec<String>,
    summary: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HouseholdView {
    recent: Vec<finance_ledger::Transaction>,
    forecast: finance_ledger::SpendForecast,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FinanceBoard {
    polybot: permagent::polybot::PolybotStatus,
    holdings: HoldingsView,
    watchlist: Vec<WatchlistRow>,
    notes: Vec<finance_ledger::FinanceNote>,
    positions: Vec<finance_ledger::Position>,
    picker: PickerView,
    picks: Vec<ValidatedPick>,
    sell_signals: Vec<SellSignal>,
    rsi_threshold: f64,
    daily_pick: Option<DailyPick>,
    household: HouseholdView,
}

fn rsi_threshold() -> f64 {
    permagent::config::Config::global()
        .get_param::<f64>(RSI_THRESHOLD_KEY)
        .unwrap_or(DEFAULT_RSI_THRESHOLD)
}

async fn pool(state: &AppState) -> Result<sqlx::Pool<sqlx::Sqlite>, ApiError> {
    state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into())
}

async fn quote_map(symbols: &[String]) -> HashMap<String, Result<QuoteView, String>> {
    let mut out = HashMap::new();
    for (i, sym) in symbols.iter().enumerate() {
        if i >= MAX_QUOTES {
            out.insert(
                sym.clone(),
                Err("not quoted — quote cap (20) reached".into()),
            );
            continue;
        }
        match market_data::quote(sym).await {
            Ok(q) => {
                out.insert(sym.clone(), Ok(QuoteView::from(q)));
            }
            Err(e) => {
                out.insert(sym.clone(), Err(e));
            }
        }
    }
    out
}

async fn get_board(State(state): State<Arc<AppState>>) -> Result<Json<FinanceBoard>, ApiError> {
    let pool = pool(&state).await?;
    let items = finance_ledger::list_watchlist(&pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let watch_syms: Vec<String> = items.iter().map(|i| i.symbol.clone()).collect();
    let watch_quotes = quote_map(&watch_syms).await;
    let mut watchlist = Vec::with_capacity(items.len());
    for item in items {
        let (quote, quote_error) = match watch_quotes.get(&item.symbol) {
            Some(Ok(q)) => (Some(q.clone()), None),
            Some(Err(e)) => (None, Some(e.clone())),
            None => (None, None),
        };
        watchlist.push(WatchlistRow {
            item,
            quote,
            quote_error,
        });
    }

    let notes = finance_ledger::list_notes(&pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let positions = finance_ledger::list_positions(&pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let picker = PickerView::from(picker::status().await);
    let polybot = permagent::polybot::status();

    let (holdings, sell_signals) = assemble_holdings(&positions, rsi_threshold()).await;
    let picks = assemble_picks(&picker).await;
    let daily_pick = financier_close::latest(&pool).await.ok().flatten();

    let recent = finance_ledger::list_transactions(&pool, 80)
        .await
        .unwrap_or_default();
    let forecast =
        finance_ledger::spend_forecast(&pool)
            .await
            .unwrap_or(finance_ledger::SpendForecast {
                days_used: 0,
                spend_90d: 0.0,
                run_rate_30d: 0.0,
                run_rate_90d: 0.0,
                by_category: vec![],
                recurring: vec![],
                method: "trailing run-rate, not a model".into(),
            });

    Ok(Json(FinanceBoard {
        polybot,
        holdings,
        watchlist,
        notes,
        positions,
        picker,
        picks,
        sell_signals,
        rsi_threshold: rsi_threshold(),
        daily_pick,
        household: HouseholdView { recent, forecast },
    }))
}

async fn assemble_holdings(
    local: &[finance_ledger::Position],
    threshold: f64,
) -> (HoldingsView, Vec<SellSignal>) {
    let picker_trades = picker::trades().await.ok().map(|raw| {
        raw.iter()
            .filter_map(picker::parse_trade_row)
            .collect::<Vec<_>>()
    });
    let (source, rows_src): (String, Vec<HoldingSeed>) = match picker_trades {
        Some(trades) if !trades.is_empty() => (
            "picker".into(),
            trades.into_iter().map(HoldingSeed::from_picker).collect(),
        ),
        _ => (
            "ledger".into(),
            local.iter().map(HoldingSeed::from_local).collect(),
        ),
    };

    let mut unique = Vec::new();
    for r in &rows_src {
        if r.exit_date.is_none() && !unique.iter().any(|s: &String| s == &r.symbol) {
            unique.push(r.symbol.clone());
        }
    }
    let quotes = quote_map(&unique).await;

    let mut readings: HashMap<String, OverboughtReading> = HashMap::new();
    for (i, sym) in unique.iter().enumerate() {
        if i >= 10 {
            break;
        }
        let high = quotes
            .get(sym)
            .and_then(|q| q.as_ref().ok())
            .and_then(|q| q.fifty_two_week_high);
        if let Ok(closes) = market_data::daily_closes(sym, "6mo").await {
            readings.insert(sym.clone(), overbought::assess(&closes, high, threshold));
        }
    }

    let mut rows = Vec::new();
    let mut net_unrealized = 0.0;
    let mut net_realized = 0.0;
    let mut open_count = 0usize;
    let mut sell_signals = Vec::new();

    for seed in rows_src {
        let (quote, quote_error) = match quotes.get(&seed.symbol) {
            Some(Ok(q)) => (Some(q.clone()), None),
            Some(Err(e)) => (None, Some(e.clone())),
            None => (None, None),
        };
        let last = quote.as_ref().and_then(|q| q.price);
        let (unrealized, unrealized_pct, realized) =
            if let (Some(exit), Some(px)) = (seed.exit_date.as_ref(), seed.exit_price) {
                let _ = exit;
                let r = (px - seed.entry_price) * seed.shares as f64;
                net_realized += r;
                (None, None, Some(r))
            } else {
                open_count += 1;
                if let Some(px) = last {
                    let u = (px - seed.entry_price) * seed.shares as f64;
                    net_unrealized += u;
                    let pct = if seed.entry_price != 0.0 {
                        Some((px / seed.entry_price - 1.0) * 100.0)
                    } else {
                        None
                    };
                    (Some(u), pct, None)
                } else {
                    (None, None, None)
                }
            };
        let reading = readings.get(&seed.symbol);
        let rsi = reading.and_then(|r| r.rsi);
        let sell_signal = reading.map(|r| r.signal).unwrap_or(false);
        let overbought_signs = reading.map(|r| r.signs.clone()).unwrap_or_default();
        if sell_signal
            && !sell_signals
                .iter()
                .any(|a: &SellSignal| a.symbol == seed.symbol)
        {
            if let Some(r) = reading {
                sell_signals.push(SellSignal {
                    symbol: seed.symbol.clone(),
                    rsi: r.rsi,
                    rsi_threshold: r.rsi_threshold,
                    signs: r.signs.clone(),
                    summary: r.summary(&seed.symbol),
                });
            }
        }
        rows.push(HoldingRow {
            id: seed.id,
            symbol: seed.symbol,
            company_name: seed.company_name,
            entry_date: seed.entry_date,
            entry_price: seed.entry_price,
            shares: seed.shares,
            exit_date: seed.exit_date,
            exit_price: seed.exit_price,
            notes: seed.notes,
            source: source.clone(),
            quote,
            quote_error,
            last,
            unrealized,
            unrealized_pct,
            realized,
            rsi,
            sell_signal,
            overbought_signs,
        });
    }

    let net_pnl = net_unrealized + net_realized;
    (
        HoldingsView {
            source,
            open_count,
            net_unrealized,
            net_realized,
            net_pnl,
            rows,
        },
        sell_signals,
    )
}

struct HoldingSeed {
    id: String,
    symbol: String,
    company_name: String,
    entry_date: String,
    entry_price: f64,
    shares: i64,
    exit_date: Option<String>,
    exit_price: Option<f64>,
    notes: Option<String>,
}

impl HoldingSeed {
    fn from_picker(t: TradeRow) -> Self {
        Self {
            id: t.id,
            symbol: t.ticker,
            company_name: t.company_name,
            entry_date: t.entry_date,
            entry_price: t.entry_price,
            shares: t.shares,
            exit_date: t.exit_date,
            exit_price: t.exit_price,
            notes: t.notes,
        }
    }
    fn from_local(p: &finance_ledger::Position) -> Self {
        Self {
            id: p.id.clone(),
            symbol: p.symbol.clone(),
            company_name: p.company_name.clone(),
            entry_date: p.entry_date.clone(),
            entry_price: p.entry_price,
            shares: p.shares,
            exit_date: p.exit_date.clone(),
            exit_price: p.exit_price,
            notes: p.notes.clone(),
        }
    }
}

async fn assemble_picks(picker: &PickerView) -> Vec<ValidatedPick> {
    if !picker.reachable || picker.scan_in_progress {
        return Vec::new();
    }
    let Ok(raw) = picker::top_picks().await else {
        return Vec::new();
    };
    let batch = raw.len().min(MAX_PICKS);
    let mut out = Vec::new();
    for (i, v) in raw.into_iter().take(MAX_PICKS).enumerate() {
        let Some(ticker) = v
            .get("ticker")
            .or_else(|| v.get("symbol"))
            .and_then(|s| s.as_str())
            .map(|s| s.trim().to_uppercase())
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let picker_price = json_num(&v, &["price", "close"]);
        let quote = market_data::quote(&ticker).await;
        let (quote, quote_error) = match quote {
            Ok(q) => (Some(QuoteView::from(q)), None),
            Err(e) => (None, Some(e)),
        };
        let yahoo_last = quote.as_ref().and_then(|q| q.price);
        let price_mismatch = match (picker_price, yahoo_last) {
            (Some(a), Some(b)) if a != 0.0 => ((a - b) / a).abs() > 0.02,
            _ => false,
        };
        let loop_gate = if i < MAX_LOOP {
            match market_data::daily_closes(&ticker, "1y").await {
                Ok(closes) => Some(pick_loop::validate_closes(&closes, batch)),
                Err(_) => None,
            }
        } else {
            None
        };
        let fundamentals = if i < MAX_LOOP {
            fundamentals_snapshot(&ticker).await
        } else {
            FundamentalsView {
                available: false,
                summary: None,
                error: None,
            }
        };
        out.push(ValidatedPick {
            ticker,
            company_name: v
                .get("company_name")
                .or_else(|| v.get("companyName"))
                .or_else(|| v.get("name"))
                .and_then(|s| s.as_str())
                .map(str::to_string),
            rank: v.get("rank").and_then(|n| n.as_i64()),
            score: json_num(&v, &["total_score", "score"]),
            tier: v.get("tier").and_then(|s| s.as_str()).map(str::to_string),
            picker_rsi: json_num(&v, &["rsi"]),
            picker_price,
            confidence: json_num(&v, &["confidence", "conv"]),
            buy_window: json_str(
                &v,
                &["buy_window", "buyWindow", "suggested_buy_window", "window"],
            ),
            reason: json_str(&v, &["reason", "thesis", "rationale", "summary", "note"]),
            quote,
            quote_error,
            price_mismatch,
            fundamentals,
            loop_gate,
        });
    }
    out
}

fn json_str(v: &serde_json::Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = v.get(*k).and_then(|x| x.as_str()).map(str::trim) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn json_num(v: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    for k in keys {
        if let Some(n) = v.get(*k) {
            if let Some(f) = n.as_f64() {
                return Some(f);
            }
            if let Some(s) = n.as_str() {
                if let Some(f) = picker::parse_money(s) {
                    return Some(f);
                }
            }
        }
    }
    None
}

async fn fundamentals_snapshot(ticker: &str) -> FundamentalsView {
    match market_data::fundamentals(ticker, "annual", 1).await {
        Ok(f) => FundamentalsView {
            available: true,
            summary: Some(market_data::describe_fundamentals(&f)),
            error: None,
        },
        Err(FundamentalsError::NotConfigured) => FundamentalsView {
            available: false,
            summary: None,
            error: Some("no financialdatasets.ai key configured".into()),
        },
        Err(FundamentalsError::Failed(e)) => FundamentalsView {
            available: true,
            summary: None,
            error: Some(e),
        },
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WatchlistBody {
    symbol: String,
    label: Option<String>,
    notes: Option<String>,
}

async fn add_watchlist(
    State(state): State<Arc<AppState>>,
    Json(body): Json<WatchlistBody>,
) -> Result<Json<finance_ledger::WatchlistItem>, ApiError> {
    let pool = pool(&state).await?;
    finance_ledger::add_watchlist(
        &pool,
        &body.symbol,
        body.label.as_deref(),
        body.notes.as_deref(),
    )
    .await
    .map(Json)
    .map_err(|e| (StatusCode::BAD_REQUEST, e).into())
}

async fn remove_watchlist(
    State(state): State<Arc<AppState>>,
    Path(symbol): Path<String>,
) -> Result<StatusCode, ApiError> {
    let pool = pool(&state).await?;
    let gone = finance_ledger::remove_watchlist(&pool, &symbol)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    if gone {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((
            StatusCode::NOT_FOUND,
            format!("{symbol} is not on the watchlist"),
        )
            .into())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NoteBody {
    title: Option<String>,
    body: Option<String>,
    symbol: Option<String>,
}

async fn add_note(
    State(state): State<Arc<AppState>>,
    Json(body): Json<NoteBody>,
) -> Result<Json<finance_ledger::FinanceNote>, ApiError> {
    let pool = pool(&state).await?;
    let title = body.title.as_deref().unwrap_or("");
    let text = body.body.as_deref().unwrap_or("");
    finance_ledger::add_note(&pool, title, text, body.symbol.as_deref())
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e).into())
}

async fn update_note(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<NoteBody>,
) -> Result<Json<finance_ledger::FinanceNote>, ApiError> {
    let pool = pool(&state).await?;
    let symbol = if body.symbol.is_some() {
        Some(body.symbol.as_deref())
    } else {
        None
    };
    finance_ledger::update_note(
        &pool,
        &id,
        body.title.as_deref(),
        body.body.as_deref(),
        symbol,
    )
    .await
    .map(Json)
    .map_err(|e| (StatusCode::BAD_REQUEST, e).into())
}

async fn delete_note(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let pool = pool(&state).await?;
    let gone = finance_ledger::delete_note(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if gone {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, "no note with that id".into()).into())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PositionBody {
    symbol: String,
    company_name: String,
    entry_date: String,
    entry_price: f64,
    shares: i64,
    exit_date: Option<String>,
    exit_price: Option<f64>,
    notes: Option<String>,
}

async fn add_position(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PositionBody>,
) -> Result<Json<finance_ledger::Position>, ApiError> {
    let pool = pool(&state).await?;
    finance_ledger::add_position(
        &pool,
        NewPosition {
            symbol: body.symbol,
            company_name: body.company_name,
            entry_date: body.entry_date,
            entry_price: body.entry_price,
            shares: body.shares,
            exit_date: body.exit_date,
            exit_price: body.exit_price,
            notes: body.notes,
        },
    )
    .await
    .map(Json)
    .map_err(|e| (StatusCode::BAD_REQUEST, e).into())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloseBody {
    exit_date: String,
    exit_price: f64,
}

async fn close_position(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<CloseBody>,
) -> Result<Json<finance_ledger::Position>, ApiError> {
    let pool = pool(&state).await?;
    finance_ledger::close_position(&pool, &id, &body.exit_date, body.exit_price)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e).into())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PickerAction {
    detail: String,
}

async fn start_picker() -> Result<Json<PickerAction>, ApiError> {
    picker::ensure_running()
        .await
        .map(|detail| Json(PickerAction { detail }))
        .map_err(|e| (StatusCode::BAD_GATEWAY, e).into())
}

async fn scan_picker() -> Result<Json<PickerAction>, ApiError> {
    let s = picker::status().await;
    if !s.reachable {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!(
                "the scanner is not running at {} — start it from this tab first",
                s.base_url
            ),
        )
            .into());
    }
    if s.scan_in_progress {
        return Ok(Json(PickerAction {
            detail: "a scan is already running".into(),
        }));
    }
    picker::start_scan()
        .await
        .map(|detail| Json(PickerAction { detail }))
        .map_err(|e| (StatusCode::BAD_GATEWAY, e).into())
}

async fn start_polybot() -> Result<Json<PickerAction>, ApiError> {
    polybot::start()
        .await
        .map(|detail| Json(PickerAction { detail }))
        .map_err(|e| (StatusCode::BAD_GATEWAY, e).into())
}

async fn pause_polybot() -> Result<Json<PickerAction>, ApiError> {
    polybot::pause()
        .map(|detail| Json(PickerAction { detail }))
        .map_err(|e| (StatusCode::BAD_GATEWAY, e).into())
}

async fn scan_polybot() -> Result<Json<PickerAction>, ApiError> {
    polybot::request_scan()
        .await
        .map(|detail| Json(PickerAction { detail }))
        .map_err(|e| (StatusCode::BAD_GATEWAY, e).into())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TradeBody {
    #[serde(alias = "ticker")]
    symbol: String,
    #[serde(default)]
    company_name: Option<String>,
    entry_date: String,
    entry_price: f64,
    shares: i64,
    exit_date: Option<String>,
    exit_price: Option<f64>,
    notes: Option<String>,
}

impl TradeBody {
    fn entry(&self) -> picker::TradeEntry {
        let ticker = self.symbol.trim().to_uppercase();
        let company = self
            .company_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(ticker.as_str())
            .to_string();
        TradeEntry {
            entry_date: self.entry_date.clone(),
            ticker,
            company_name: company,
            entry_price: self.entry_price,
            shares: self.shares,
            exit_date: self.exit_date.clone(),
            exit_price: self.exit_price,
            notes: self.notes.clone(),
        }
    }

    fn as_new_position(trade: &TradeEntry) -> NewPosition {
        NewPosition {
            symbol: trade.ticker.clone(),
            company_name: trade.company_name.clone(),
            entry_date: trade.entry_date.clone(),
            entry_price: trade.entry_price,
            shares: trade.shares,
            exit_date: trade.exit_date.clone(),
            exit_price: trade.exit_price,
            notes: trade.notes.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordedTrade {
    local: Option<finance_ledger::Position>,
    picker: Option<serde_json::Value>,
    picker_error: Option<String>,
}

fn recorded(
    local: Option<finance_ledger::Position>,
    picker: Result<serde_json::Value, String>,
) -> Result<Json<RecordedTrade>, ApiError> {
    match picker {
        Ok(v) => Ok(Json(RecordedTrade {
            local,
            picker: Some(v),
            picker_error: None,
        })),
        Err(e) if local.is_some() => Ok(Json(RecordedTrade {
            local,
            picker: None,
            picker_error: Some(e),
        })),
        Err(e) => Err((StatusCode::BAD_GATEWAY, e).into()),
    }
}

async fn existing_trade(pool: &sqlx::Pool<sqlx::Sqlite>, id: &str) -> Option<TradeEntry> {
    if let Ok(raw) = picker::trades().await {
        if let Some(t) = raw
            .iter()
            .filter_map(picker::parse_trade_row)
            .find(|t| t.id == id)
        {
            return Some(TradeEntry::from(&t));
        }
    }
    finance_ledger::list_positions(pool)
        .await
        .ok()?
        .into_iter()
        .find(|p| p.id == id)
        .map(|p| TradeEntry {
            entry_date: p.entry_date,
            ticker: p.symbol,
            company_name: p.company_name,
            entry_price: p.entry_price,
            shares: p.shares,
            exit_date: p.exit_date,
            exit_price: p.exit_price,
            notes: p.notes,
        })
}

/// Picker history first. If the scanner is down, the tab ledger is the
/// fallback so the user never has to open Picker to enter a lot they already
/// took. Dual-write is avoided: when Picker accepts the row, holdings already
/// read from Picker.
async fn record_picker_trade(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TradeBody>,
) -> Result<Json<RecordedTrade>, ApiError> {
    let trade = body.entry();
    if trade.ticker.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "ticker is required".into()).into());
    }
    match picker::record_trade(&trade).await {
        Ok(v) => Ok(Json(RecordedTrade {
            local: None,
            picker: Some(v),
            picker_error: None,
        })),
        Err(e) => {
            let pool = pool(&state).await?;
            let local = finance_ledger::add_position(&pool, TradeBody::as_new_position(&trade))
                .await
                .ok();
            recorded(local, Err(e))
        }
    }
}

async fn update_picker_trade(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<TradeBody>,
) -> Result<Json<RecordedTrade>, ApiError> {
    let trade = body.entry();
    if trade.ticker.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "ticker is required".into()).into());
    }
    let picker_saved = picker::update_trade(&id, &trade).await;
    let pool = pool(&state).await.ok();
    let local = if let Some(pool) = pool.as_ref() {
        finance_ledger::update_position(pool, &id, TradeBody::as_new_position(&trade))
            .await
            .ok()
    } else {
        None
    };
    recorded(local, picker_saved)
}

async fn close_picker_trade(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<CloseBody>,
) -> Result<Json<RecordedTrade>, ApiError> {
    let pool = pool(&state).await.ok();
    let existing = if let Some(pool) = pool.as_ref() {
        existing_trade(pool, &id).await
    } else {
        None
    };
    let picker_saved =
        picker::close_trade(&id, &body.exit_date, body.exit_price, existing.as_ref()).await;
    let local = if let Some(pool) = pool.as_ref() {
        finance_ledger::close_position(pool, &id, &body.exit_date, body.exit_price)
            .await
            .ok()
    } else {
        None
    };
    recorded(local, picker_saved)
}

async fn update_local_position(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<PositionBody>,
) -> Result<Json<finance_ledger::Position>, ApiError> {
    let pool = pool(&state).await?;
    finance_ledger::update_position(
        &pool,
        &id,
        NewPosition {
            symbol: body.symbol,
            company_name: body.company_name,
            entry_date: body.entry_date,
            entry_price: body.entry_price,
            shares: body.shares,
            exit_date: body.exit_date,
            exit_price: body.exit_price,
            notes: body.notes,
        },
    )
    .await
    .map(Json)
    .map_err(|e| (StatusCode::BAD_REQUEST, e).into())
}

async fn delete_picker_trade(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let picker_err = picker::delete_trade(&id).await.err();
    let pool = pool(&state).await?;
    let local_gone = finance_ledger::delete_position(&pool, &id)
        .await
        .unwrap_or(false);
    if picker_err.is_none() || local_gone {
        return Ok(StatusCode::NO_CONTENT);
    }
    Err((
        StatusCode::BAD_GATEWAY,
        picker_err.unwrap_or_else(|| "no trade with that id".into()),
    )
        .into())
}

async fn delete_position(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let pool = pool(&state).await?;
    let gone = finance_ledger::delete_position(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if gone {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, "no position with that id".into()).into())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct IngestResult {
    inserted: usize,
    parsed: usize,
    source_file: String,
    ocr_used: bool,
}

async fn ingest_statement(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<IngestResult>, ApiError> {
    let pool = pool(&state).await?;
    let Ok(Some(field)) = multipart.next_field().await else {
        return Err((StatusCode::BAD_REQUEST, "no file".into()).into());
    };
    let filename = field
        .file_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "statement".into());
    let mime = field
        .content_type()
        .map(|s| s.to_string())
        .unwrap_or_default();
    let data = field
        .bytes()
        .await
        .map_err(|_| (StatusCode::BAD_REQUEST, "could not read the file".into()))?;
    if data.len() > MAX_FILE_SIZE {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "file too large".into()).into());
    }

    let lower = filename.to_lowercase();
    let ocr_text = if lower.ends_with(".csv")
        || lower.ends_with(".ofx")
        || lower.ends_with(".qfx")
        || mime.contains("csv")
        || mime.contains("ofx")
    {
        None
    } else if mime.starts_with("image/") {
        let digest = permagent::reader::ingest_image(&data, &filename)
            .await
            .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
        Some(digest.summary)
    } else {
        let digest = permagent::reader::ingest_document(&data, &filename, &mime)
            .await
            .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
        Some(digest.summary)
    };

    let parsed = finance_statements::parse_statement(&filename, &mime, &data, ocr_text.as_deref())
        .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e))?;
    let parsed_n = parsed.len();
    let inserted = finance_ledger::insert_transactions(&pool, &parsed, None, &filename)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(IngestResult {
        inserted,
        parsed: parsed_n,
        source_file: filename,
        ocr_used: ocr_text.is_some(),
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecatBody {
    category: String,
}

async fn recategorize(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<RecatBody>,
) -> Result<StatusCode, ApiError> {
    let pool = pool(&state).await?;
    let ok = finance_ledger::recategorize(&pool, &id, &body.category)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    if ok {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, "no transaction with that id".into()).into())
    }
}
