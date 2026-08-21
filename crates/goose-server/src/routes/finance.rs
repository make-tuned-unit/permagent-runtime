//! Finance tab — the Financier's ledger.
//!
//! `GET /api/finance` is the board: watchlist (with live quotes fetched at
//! read time), research notes, positions, and optional Picker scanner status.
//! Mutations write the local ledger; quotes are never persisted.

use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, patch, post},
    Json, Router,
};
use permagent::finance_ledger::{self, NewPosition};
use permagent::market_data::{self, Quote};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
        .route("/api/finance/positions/{id}", delete(delete_position))
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

#[derive(Debug, Serialize)]
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

impl From<permagent::picker::PickerStatus> for PickerView {
    fn from(p: permagent::picker::PickerStatus) -> Self {
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
struct FinanceBoard {
    watchlist: Vec<WatchlistRow>,
    notes: Vec<finance_ledger::FinanceNote>,
    positions: Vec<finance_ledger::Position>,
    picker: PickerView,
}

async fn pool(state: &AppState) -> Result<sqlx::Pool<sqlx::Sqlite>, (StatusCode, String)> {
    state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn get_board(
    State(state): State<Arc<AppState>>,
) -> Result<Json<FinanceBoard>, (StatusCode, String)> {
    let pool = pool(&state).await?;
    let items = finance_ledger::list_watchlist(&pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let mut watchlist = Vec::with_capacity(items.len());
    // Bound the fan-out so a long watchlist cannot become a scrape.
    const MAX_QUOTES: usize = 20;
    for (i, item) in items.into_iter().enumerate() {
        let (quote, quote_error) = if i < MAX_QUOTES {
            match market_data::quote(&item.symbol).await {
                Ok(q) => (Some(QuoteView::from(q)), None),
                Err(e) => (None, Some(e)),
            }
        } else {
            (
                None,
                Some("not quoted — watchlist quote cap (20) reached".into()),
            )
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
    let picker = PickerView::from(permagent::picker::status().await);
    Ok(Json(FinanceBoard {
        watchlist,
        notes,
        positions,
        picker,
    }))
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
) -> Result<Json<finance_ledger::WatchlistItem>, (StatusCode, String)> {
    let pool = pool(&state).await?;
    finance_ledger::add_watchlist(
        &pool,
        &body.symbol,
        body.label.as_deref(),
        body.notes.as_deref(),
    )
    .await
    .map(Json)
    .map_err(|e| (StatusCode::BAD_REQUEST, e))
}

async fn remove_watchlist(
    State(state): State<Arc<AppState>>,
    Path(symbol): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
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
        ))
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
) -> Result<Json<finance_ledger::FinanceNote>, (StatusCode, String)> {
    let pool = pool(&state).await?;
    let title = body.title.as_deref().unwrap_or("");
    let text = body.body.as_deref().unwrap_or("");
    finance_ledger::add_note(&pool, title, text, body.symbol.as_deref())
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))
}

async fn update_note(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<NoteBody>,
) -> Result<Json<finance_ledger::FinanceNote>, (StatusCode, String)> {
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
    .map_err(|e| (StatusCode::BAD_REQUEST, e))
}

async fn delete_note(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let pool = pool(&state).await?;
    let gone = finance_ledger::delete_note(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if gone {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, "no note with that id".into()))
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
) -> Result<Json<finance_ledger::Position>, (StatusCode, String)> {
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
    .map_err(|e| (StatusCode::BAD_REQUEST, e))
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
) -> Result<Json<finance_ledger::Position>, (StatusCode, String)> {
    let pool = pool(&state).await?;
    finance_ledger::close_position(&pool, &id, &body.exit_date, body.exit_price)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))
}

async fn delete_position(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let pool = pool(&state).await?;
    let gone = finance_ledger::delete_position(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if gone {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, "no position with that id".into()))
    }
}
