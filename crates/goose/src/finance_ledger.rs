//! The Financier's ledger — the Finance tab's system of record.
//!
//! Watchlist symbols, research notes, and positions live here so the tab
//! works even when the user's optional Picker scanner is down. Quotes are
//! never stored; they are fetched at read time and travel with their
//! timestamp. Nothing here can place an order.

use serde::{Deserialize, Serialize};
use sqlx::{Pool, Row, Sqlite};
use uuid::Uuid;

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Config key for the holdings RSI-14 heat threshold (default 74).
pub const RSI_THRESHOLD_KEY: &str = "finance_rsi_threshold";
/// Default RSI-14 threshold. One of the overbought signs on open lots.
pub const DEFAULT_RSI_THRESHOLD: f64 = 74.0;

/// Uppercase ticker the ledger keys on. Empty / whitespace is refused.
pub fn normalize_symbol(raw: &str) -> Result<String, String> {
    let s = raw.trim().to_uppercase();
    if s.is_empty() {
        return Err("symbol is empty".into());
    }
    if s.len() > 24 {
        return Err("symbol is too long".into());
    }
    Ok(s)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WatchlistItem {
    pub id: String,
    pub symbol: String,
    pub label: Option<String>,
    pub notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FinanceNote {
    pub id: String,
    pub title: String,
    pub body: String,
    pub symbol: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub id: String,
    pub symbol: String,
    pub company_name: String,
    pub entry_date: String,
    pub entry_price: f64,
    pub shares: i64,
    pub exit_date: Option<String>,
    pub exit_price: Option<f64>,
    pub notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

fn watchlist_from_row(row: &sqlx::sqlite::SqliteRow) -> WatchlistItem {
    WatchlistItem {
        id: row.get("id"),
        symbol: row.get("symbol"),
        label: row.get("label"),
        notes: row.get("notes"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn note_from_row(row: &sqlx::sqlite::SqliteRow) -> FinanceNote {
    FinanceNote {
        id: row.get("id"),
        title: row.get("title"),
        body: row.get("body"),
        symbol: row.get("symbol"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn position_from_row(row: &sqlx::sqlite::SqliteRow) -> Position {
    Position {
        id: row.get("id"),
        symbol: row.get("symbol"),
        company_name: row.get("company_name"),
        entry_date: row.get("entry_date"),
        entry_price: row.get("entry_price"),
        shares: row.get("shares"),
        exit_date: row.get("exit_date"),
        exit_price: row.get("exit_price"),
        notes: row.get("notes"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

pub async fn list_watchlist(pool: &Pool<Sqlite>) -> Result<Vec<WatchlistItem>, String> {
    let rows = sqlx::query(
        "SELECT id, symbol, label, notes, created_at, updated_at
         FROM finance_watchlist ORDER BY sort_order ASC, created_at ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(watchlist_from_row).collect())
}

pub async fn add_watchlist(
    pool: &Pool<Sqlite>,
    symbol: &str,
    label: Option<&str>,
    notes: Option<&str>,
) -> Result<WatchlistItem, String> {
    let symbol = normalize_symbol(symbol)?;
    let label = label.map(str::trim).filter(|s| !s.is_empty());
    let notes = notes.map(str::trim).filter(|s| !s.is_empty());
    let existing: Option<String> =
        sqlx::query_scalar("SELECT id FROM finance_watchlist WHERE symbol = ?")
            .bind(&symbol)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;
    if let Some(id) = existing {
        return get_watchlist_item(pool, &id)
            .await?
            .ok_or_else(|| format!("{symbol} is already on the watchlist"));
    }
    let id = Uuid::now_v7().to_string();
    let ts = now_iso();
    sqlx::query(
        "INSERT INTO finance_watchlist (id, symbol, label, notes, sort_order, created_at, updated_at)
         VALUES (?, ?, ?, ?, (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM finance_watchlist), ?, ?)",
    )
    .bind(&id)
    .bind(&symbol)
    .bind(label)
    .bind(notes)
    .bind(&ts)
    .bind(&ts)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    get_watchlist_item(pool, &id)
        .await?
        .ok_or_else(|| "watchlist row vanished after insert".into())
}

pub async fn remove_watchlist(pool: &Pool<Sqlite>, symbol: &str) -> Result<bool, String> {
    let symbol = normalize_symbol(symbol)?;
    let result = sqlx::query("DELETE FROM finance_watchlist WHERE symbol = ?")
        .bind(&symbol)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(result.rows_affected() > 0)
}

async fn get_watchlist_item(
    pool: &Pool<Sqlite>,
    id: &str,
) -> Result<Option<WatchlistItem>, String> {
    let row = sqlx::query(
        "SELECT id, symbol, label, notes, created_at, updated_at FROM finance_watchlist WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.as_ref().map(watchlist_from_row))
}

pub async fn list_notes(pool: &Pool<Sqlite>) -> Result<Vec<FinanceNote>, String> {
    let rows = sqlx::query(
        "SELECT id, title, body, symbol, created_at, updated_at
         FROM finance_notes ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(note_from_row).collect())
}

pub async fn add_note(
    pool: &Pool<Sqlite>,
    title: &str,
    body: &str,
    symbol: Option<&str>,
) -> Result<FinanceNote, String> {
    let title = title.trim();
    let body = body.trim();
    if title.is_empty() {
        return Err("note title is empty".into());
    }
    if body.is_empty() {
        return Err("note body is empty".into());
    }
    let symbol = match symbol {
        Some(s) => Some(normalize_symbol(s)?),
        None => None,
    };
    let id = Uuid::now_v7().to_string();
    let ts = now_iso();
    sqlx::query(
        "INSERT INTO finance_notes (id, title, body, symbol, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(title)
    .bind(body)
    .bind(symbol.as_deref())
    .bind(&ts)
    .bind(&ts)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    get_note(pool, &id)
        .await?
        .ok_or_else(|| "note vanished after insert".into())
}

pub async fn update_note(
    pool: &Pool<Sqlite>,
    id: &str,
    title: Option<&str>,
    body: Option<&str>,
    symbol: Option<Option<&str>>,
) -> Result<FinanceNote, String> {
    let current = get_note(pool, id)
        .await?
        .ok_or_else(|| "no note with that id".to_string())?;
    let title = title
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(current.title.as_str());
    let body = body
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(current.body.as_str());
    let symbol = match symbol {
        Some(Some(s)) => Some(normalize_symbol(s)?),
        Some(None) => None,
        None => current.symbol.clone(),
    };
    let ts = now_iso();
    sqlx::query(
        "UPDATE finance_notes SET title = ?, body = ?, symbol = ?, updated_at = ? WHERE id = ?",
    )
    .bind(title)
    .bind(body)
    .bind(symbol.as_deref())
    .bind(&ts)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    get_note(pool, id)
        .await?
        .ok_or_else(|| "note vanished after update".into())
}

pub async fn delete_note(pool: &Pool<Sqlite>, id: &str) -> Result<bool, String> {
    let result = sqlx::query("DELETE FROM finance_notes WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(result.rows_affected() > 0)
}

async fn get_note(pool: &Pool<Sqlite>, id: &str) -> Result<Option<FinanceNote>, String> {
    let row = sqlx::query(
        "SELECT id, title, body, symbol, created_at, updated_at FROM finance_notes WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.as_ref().map(note_from_row))
}

pub async fn list_positions(pool: &Pool<Sqlite>) -> Result<Vec<Position>, String> {
    let rows = sqlx::query(
        "SELECT id, symbol, company_name, entry_date, entry_price, shares,
                exit_date, exit_price, notes, created_at, updated_at
         FROM finance_positions ORDER BY entry_date DESC, created_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(position_from_row).collect())
}

pub struct NewPosition {
    pub symbol: String,
    pub company_name: String,
    pub entry_date: String,
    pub entry_price: f64,
    pub shares: i64,
    pub exit_date: Option<String>,
    pub exit_price: Option<f64>,
    pub notes: Option<String>,
}

struct PreparedPosition {
    symbol: String,
    company_name: String,
    entry_date: String,
    entry_price: f64,
    shares: i64,
    exit_date: Option<String>,
    exit_price: Option<f64>,
    notes: Option<String>,
}

fn prepare_position(p: NewPosition) -> Result<PreparedPosition, String> {
    let symbol = normalize_symbol(&p.symbol)?;
    let company_name = {
        let c = p.company_name.trim();
        if c.is_empty() {
            symbol.clone()
        } else {
            c.to_string()
        }
    };
    let entry_date = p.entry_date.trim();
    if entry_date.is_empty() {
        return Err("entry date is empty".into());
    }
    if p.entry_price <= 0.0 {
        return Err("entry price must be positive".into());
    }
    if p.shares == 0 {
        return Err("shares cannot be zero".into());
    }
    let exit_date = p
        .exit_date
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if let Some(px) = p.exit_price {
        if px <= 0.0 {
            return Err("exit price must be positive".into());
        }
    }
    Ok(PreparedPosition {
        symbol,
        company_name,
        entry_date: entry_date.to_string(),
        entry_price: p.entry_price,
        shares: p.shares,
        exit_date,
        exit_price: p.exit_price.filter(|px| *px > 0.0),
        notes: p
            .notes
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    })
}

pub async fn add_position(pool: &Pool<Sqlite>, p: NewPosition) -> Result<Position, String> {
    let p = prepare_position(p)?;
    let id = Uuid::now_v7().to_string();
    let ts = now_iso();
    sqlx::query(
        "INSERT INTO finance_positions
            (id, symbol, company_name, entry_date, entry_price, shares, exit_date, exit_price, notes, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&p.symbol)
    .bind(&p.company_name)
    .bind(&p.entry_date)
    .bind(p.entry_price)
    .bind(p.shares)
    .bind(&p.exit_date)
    .bind(p.exit_price)
    .bind(&p.notes)
    .bind(&ts)
    .bind(&ts)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    get_position(pool, &id)
        .await?
        .ok_or_else(|| "position vanished after insert".into())
}

/// Rewrite a ledger lot. Used when the Finance tab edits a trade that never
/// made it into Picker (scanner down). Does not place an order.
pub async fn update_position(
    pool: &Pool<Sqlite>,
    id: &str,
    p: NewPosition,
) -> Result<Position, String> {
    let current = get_position(pool, id)
        .await?
        .ok_or_else(|| "no position with that id".to_string())?;
    let p = prepare_position(p)?;
    let ts = now_iso();
    sqlx::query(
        "UPDATE finance_positions
         SET symbol = ?, company_name = ?, entry_date = ?, entry_price = ?,
             shares = ?, exit_date = ?, exit_price = ?, notes = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(&p.symbol)
    .bind(&p.company_name)
    .bind(&p.entry_date)
    .bind(p.entry_price)
    .bind(p.shares)
    .bind(&p.exit_date)
    .bind(p.exit_price)
    .bind(&p.notes)
    .bind(&ts)
    .bind(&current.id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    get_position(pool, &current.id)
        .await?
        .ok_or_else(|| "position vanished after update".into())
}

pub async fn close_position(
    pool: &Pool<Sqlite>,
    id: &str,
    exit_date: &str,
    exit_price: f64,
) -> Result<Position, String> {
    if exit_date.trim().is_empty() {
        return Err("exit date is empty".into());
    }
    if exit_price <= 0.0 {
        return Err("exit price must be positive".into());
    }
    let current = get_position(pool, id)
        .await?
        .ok_or_else(|| "no position with that id".to_string())?;
    if current.exit_date.is_some() {
        return Err("that position is already closed".into());
    }
    let ts = now_iso();
    sqlx::query(
        "UPDATE finance_positions SET exit_date = ?, exit_price = ?, updated_at = ? WHERE id = ?",
    )
    .bind(exit_date.trim())
    .bind(exit_price)
    .bind(&ts)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    get_position(pool, id)
        .await?
        .ok_or_else(|| "position vanished after close".into())
}

pub async fn delete_position(pool: &Pool<Sqlite>, id: &str) -> Result<bool, String> {
    let result = sqlx::query("DELETE FROM finance_positions WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(result.rows_affected() > 0)
}

async fn get_position(pool: &Pool<Sqlite>, id: &str) -> Result<Option<Position>, String> {
    let row = sqlx::query(
        "SELECT id, symbol, company_name, entry_date, entry_price, shares,
                exit_date, exit_price, notes, created_at, updated_at
         FROM finance_positions WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.as_ref().map(position_from_row))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Transaction {
    pub id: String,
    pub date: String,
    pub amount: f64,
    pub payee: String,
    pub category: String,
    pub account: Option<String>,
    pub source_file: Option<String>,
    pub created_at: String,
}

fn txn_from_row(row: &sqlx::sqlite::SqliteRow) -> Transaction {
    Transaction {
        id: row.get("id"),
        date: row.get("date"),
        amount: row.get("amount"),
        payee: row.get("payee"),
        category: row.get("category"),
        account: row.get("account"),
        source_file: row.get("source_file"),
        created_at: row.get("created_at"),
    }
}

pub async fn list_transactions(
    pool: &Pool<Sqlite>,
    limit: i64,
) -> Result<Vec<Transaction>, String> {
    let rows = sqlx::query(
        "SELECT id, date, amount, payee, category, account, source_file, created_at
         FROM finance_transactions ORDER BY date DESC, created_at DESC LIMIT ?",
    )
    .bind(limit.clamp(1, 500))
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(txn_from_row).collect())
}

pub async fn insert_transactions(
    pool: &Pool<Sqlite>,
    rows: &[crate::finance_statements::ParsedTxn],
    account: Option<&str>,
    source_file: &str,
) -> Result<usize, String> {
    let account = account.map(str::trim).filter(|s| !s.is_empty());
    let mut inserted = 0usize;
    for r in rows {
        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM finance_transactions
             WHERE date = ? AND amount = ? AND payee = ? AND IFNULL(source_file,'') = ?",
        )
        .bind(&r.date)
        .bind(r.amount)
        .bind(&r.payee)
        .bind(source_file)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
        if existing.is_some() {
            continue;
        }
        let id = Uuid::now_v7().to_string();
        let ts = now_iso();
        sqlx::query(
            "INSERT INTO finance_transactions
                (id, date, amount, payee, category, account, source_file, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&r.date)
        .bind(r.amount)
        .bind(&r.payee)
        .bind(&r.category)
        .bind(account)
        .bind(source_file)
        .bind(&ts)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
        inserted += 1;
    }
    Ok(inserted)
}

pub async fn recategorize(pool: &Pool<Sqlite>, id: &str, category: &str) -> Result<bool, String> {
    let cat = category.trim().to_lowercase();
    if !crate::finance_statements::CATEGORIES.contains(&cat.as_str()) {
        return Err(format!("{cat} is not a known category"));
    }
    let result = sqlx::query("UPDATE finance_transactions SET category = ? WHERE id = ?")
        .bind(&cat)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(result.rows_affected() > 0)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CategorySpend {
    pub category: String,
    pub amount: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Recurring {
    pub payee: String,
    pub typical_amount: f64,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpendForecast {
    pub days_used: i64,
    pub spend_90d: f64,
    pub run_rate_30d: f64,
    pub run_rate_90d: f64,
    pub by_category: Vec<CategorySpend>,
    pub recurring: Vec<Recurring>,
    /// Honest label: this is a trailing average, not a model.
    pub method: String,
}

pub async fn spend_forecast(pool: &Pool<Sqlite>) -> Result<SpendForecast, String> {
    let since = (chrono::Utc::now() - chrono::Duration::days(90))
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    let rows = sqlx::query(
        "SELECT date, amount, payee, category FROM finance_transactions WHERE date >= ?",
    )
    .bind(&since)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut by_cat: std::collections::BTreeMap<String, f64> = std::collections::BTreeMap::new();
    let mut by_payee: std::collections::BTreeMap<String, (f64, i64)> =
        std::collections::BTreeMap::new();
    let mut spend = 0.0;
    let mut min_date: Option<String> = None;
    let mut max_date: Option<String> = None;
    for row in &rows {
        let amount: f64 = row.get("amount");
        let cat: String = row.get("category");
        let payee: String = row.get("payee");
        let date: String = row.get("date");
        min_date = Some(min_date.map_or(date.clone(), |m| m.min(date.clone())));
        max_date = Some(max_date.map_or(date.clone(), |m| m.max(date.clone())));
        if amount < 0.0 {
            spend += -amount;
            *by_cat.entry(cat).or_insert(0.0) += -amount;
            let e = by_payee.entry(payee).or_insert((0.0, 0));
            e.0 += -amount;
            e.1 += 1;
        }
    }
    let days = match (&min_date, &max_date) {
        (Some(a), Some(b)) => {
            let da = chrono::NaiveDate::parse_from_str(a, "%Y-%m-%d").ok();
            let db = chrono::NaiveDate::parse_from_str(b, "%Y-%m-%d").ok();
            match (da, db) {
                (Some(x), Some(y)) => (y - x).num_days().max(1),
                _ => 90,
            }
        }
        _ => 90,
    };
    let daily = if days > 0 { spend / days as f64 } else { 0.0 };
    let mut recurring: Vec<Recurring> = by_payee
        .into_iter()
        .filter(|(_, (_, c))| *c >= 2)
        .map(|(payee, (total, count))| Recurring {
            payee,
            typical_amount: (total / count as f64 * 100.0).round() / 100.0,
            count,
        })
        .collect();
    recurring.sort_by_key(|r| std::cmp::Reverse(r.count));
    recurring.truncate(12);

    Ok(SpendForecast {
        days_used: days,
        spend_90d: (spend * 100.0).round() / 100.0,
        run_rate_30d: (daily * 30.0 * 100.0).round() / 100.0,
        run_rate_90d: (daily * 90.0 * 100.0).round() / 100.0,
        by_category: by_cat
            .into_iter()
            .map(|(category, amount)| CategorySpend {
                category,
                amount: (amount * 100.0).round() / 100.0,
            })
            .collect(),
        recurring,
        method: "trailing run-rate, not a model".into(),
    })
}

/// RSI heat alerts, one row per symbol per civil day — the notify dedup.
pub async fn rsi_alert_seen_today(
    pool: &Pool<Sqlite>,
    symbol: &str,
    day: &str,
) -> Result<bool, String> {
    let n: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM finance_rsi_alerts WHERE symbol = ? AND day = ?")
            .bind(symbol)
            .bind(day)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;
    Ok(n.is_some())
}

pub async fn record_rsi_alert(
    pool: &Pool<Sqlite>,
    symbol: &str,
    day: &str,
    rsi: f64,
    threshold: f64,
) -> Result<(), String> {
    sqlx::query(
        "INSERT OR IGNORE INTO finance_rsi_alerts (symbol, day, rsi, threshold, created_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(symbol)
    .bind(day)
    .bind(rsi)
    .bind(threshold)
    .bind(now_iso())
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::spectral_schema::init_spectral_db;

    async fn pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        crate::session::spectral_schema::apply_finance_ledger_schema(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn watchlist_add_is_idempotent_on_symbol() {
        let pool = pool().await;
        let a = add_watchlist(&pool, "aapl", Some("Apple"), None)
            .await
            .unwrap();
        assert_eq!(a.symbol, "AAPL");
        let b = add_watchlist(&pool, "AAPL", None, None).await.unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(list_watchlist(&pool).await.unwrap().len(), 1);
        assert!(remove_watchlist(&pool, "aapl").await.unwrap());
        assert!(!remove_watchlist(&pool, "aapl").await.unwrap());
    }

    #[tokio::test]
    async fn notes_round_trip() {
        let pool = pool().await;
        let n = add_note(&pool, "SHOP", "watching the range", Some("shop.to"))
            .await
            .unwrap();
        assert_eq!(n.symbol.as_deref(), Some("SHOP.TO"));
        let u = update_note(&pool, &n.id, Some("Shopify"), None, None)
            .await
            .unwrap();
        assert_eq!(u.title, "Shopify");
        assert!(delete_note(&pool, &n.id).await.unwrap());
        assert!(list_notes(&pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn position_open_and_close() {
        let pool = pool().await;
        let p = add_position(
            &pool,
            NewPosition {
                symbol: "enb".into(),
                company_name: "Enbridge".into(),
                entry_date: "2026-01-15".into(),
                entry_price: 52.1,
                shares: 100,
                exit_date: None,
                exit_price: None,
                notes: Some("user said they bought it".into()),
            },
        )
        .await
        .unwrap();
        assert_eq!(p.symbol, "ENB");
        assert!(p.exit_date.is_none());
        let closed = close_position(&pool, &p.id, "2026-08-01", 55.0)
            .await
            .unwrap();
        assert_eq!(closed.exit_price, Some(55.0));
        assert!(close_position(&pool, &p.id, "2026-08-02", 56.0)
            .await
            .unwrap_err()
            .contains("already closed"));
    }

    #[tokio::test]
    async fn position_update_rewrites_the_lot() {
        let pool = pool().await;
        let p = add_position(
            &pool,
            NewPosition {
                symbol: "enb".into(),
                company_name: "".into(),
                entry_date: "2026-01-15".into(),
                entry_price: 52.1,
                shares: 100,
                exit_date: None,
                exit_price: None,
                notes: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(p.company_name, "ENB");
        let u = update_position(
            &pool,
            &p.id,
            NewPosition {
                symbol: "enb".into(),
                company_name: "Enbridge".into(),
                entry_date: "2026-01-16".into(),
                entry_price: 53.0,
                shares: 80,
                exit_date: Some("2026-08-01".into()),
                exit_price: Some(55.0),
                notes: Some("corrected".into()),
            },
        )
        .await
        .unwrap();
        assert_eq!(u.shares, 80);
        assert_eq!(u.entry_date, "2026-01-16");
        assert_eq!(u.exit_price, Some(55.0));
        assert_eq!(u.notes.as_deref(), Some("corrected"));
    }

    #[tokio::test]
    async fn statement_insert_is_idempotent_on_same_file_row() {
        let pool = pool().await;
        crate::session::spectral_schema::apply_finance_spend_schema(&pool)
            .await
            .unwrap();
        let recent = (chrono::Utc::now() - chrono::Duration::days(10))
            .date_naive()
            .format("%Y-%m-%d")
            .to_string();
        let rows = vec![crate::finance_statements::ParsedTxn {
            date: recent,
            amount: -86.4,
            payee: "Sobeys".into(),
            category: "groceries".into(),
        }];
        assert_eq!(
            insert_transactions(&pool, &rows, Some("visa"), "jan.csv")
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            insert_transactions(&pool, &rows, Some("visa"), "jan.csv")
                .await
                .unwrap(),
            0
        );
        let f = spend_forecast(&pool).await.unwrap();
        assert!(f.spend_90d > 0.0);
        assert!(f.method.contains("run-rate"));
    }

    #[tokio::test]
    async fn rsi_alert_is_once_per_symbol_per_day() {
        let pool = pool().await;
        crate::session::spectral_schema::apply_finance_spend_schema(&pool)
            .await
            .unwrap();
        assert!(!rsi_alert_seen_today(&pool, "SHOP", "2026-08-21")
            .await
            .unwrap());
        record_rsi_alert(&pool, "SHOP", "2026-08-21", 78.2, 74.0)
            .await
            .unwrap();
        assert!(rsi_alert_seen_today(&pool, "SHOP", "2026-08-21")
            .await
            .unwrap());
        record_rsi_alert(&pool, "SHOP", "2026-08-21", 80.0, 74.0)
            .await
            .unwrap();
        assert!(!rsi_alert_seen_today(&pool, "SHOP", "2026-08-22")
            .await
            .unwrap());
    }

    #[test]
    fn empty_symbol_is_refused() {
        assert!(normalize_symbol("  ").is_err());
    }
}
