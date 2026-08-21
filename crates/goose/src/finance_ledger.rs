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

pub async fn add_position(pool: &Pool<Sqlite>, p: NewPosition) -> Result<Position, String> {
    let symbol = normalize_symbol(&p.symbol)?;
    let company_name = p.company_name.trim();
    if company_name.is_empty() {
        return Err("company name is empty".into());
    }
    if p.entry_date.trim().is_empty() {
        return Err("entry date is empty".into());
    }
    if p.entry_price <= 0.0 {
        return Err("entry price must be positive".into());
    }
    if p.shares == 0 {
        return Err("shares cannot be zero".into());
    }
    let notes = p.notes.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let id = Uuid::now_v7().to_string();
    let ts = now_iso();
    sqlx::query(
        "INSERT INTO finance_positions
            (id, symbol, company_name, entry_date, entry_price, shares, exit_date, exit_price, notes, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&symbol)
    .bind(company_name)
    .bind(p.entry_date.trim())
    .bind(p.entry_price)
    .bind(p.shares)
    .bind(p.exit_date.as_deref().map(str::trim).filter(|s| !s.is_empty()))
    .bind(p.exit_price)
    .bind(notes)
    .bind(&ts)
    .bind(&ts)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    get_position(pool, &id)
        .await?
        .ok_or_else(|| "position vanished after insert".into())
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

    #[test]
    fn empty_symbol_is_refused() {
        assert!(normalize_symbol("  ").is_err());
    }
}
