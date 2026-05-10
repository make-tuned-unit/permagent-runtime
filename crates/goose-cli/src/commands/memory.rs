use anyhow::{Context, Result};
use comfy_table::{presets, ContentArrangement, Table};
use permagent::config::paths::Paths;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;
use std::str::FromStr;

fn preview(content: &str, max_len: usize) -> String {
    let flat = content.replace('\n', " ");
    if flat.len() <= max_len {
        flat
    } else {
        let truncated: String = flat.chars().take(max_len).collect();
        format!("{}...", truncated)
    }
}

async fn open_spectral_db() -> Result<sqlx::SqlitePool> {
    let db_path = Paths::spectral_db();
    if !db_path.exists() {
        anyhow::bail!("Spectral database not found. Run permagent setup first.");
    }

    let connect_opts = SqliteConnectOptions::from_str(&format!("sqlite:{}", db_path.display()))?
        .create_if_missing(false)
        .read_only(false);

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(connect_opts)
        .await
        .context("Failed to open Spectral database")?;

    Ok(pool)
}

pub async fn handle_memory_search(query: &str, limit: usize) -> Result<()> {
    let pool = open_spectral_db().await?;

    let rows = sqlx::query(
        "SELECT m.key, m.content, m.category, m.wing, m.hall, m.confidence, m.created_at
         FROM memories_fts fts
         JOIN memories m ON m.rowid = fts.rowid
         WHERE memories_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    )
    .bind(query)
    .bind(limit as i64)
    .fetch_all(&pool)
    .await
    .context("FTS search failed")?;

    if rows.is_empty() {
        println!("No memories found matching '{}'.", query);
        return Ok(());
    }

    let mut table = Table::new();
    table
        .load_preset(presets::UTF8_FULL_CONDENSED)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            "Key",
            "Content",
            "Category",
            "Wing",
            "Hall",
            "Confidence",
            "Created",
        ]);

    for row in &rows {
        let key: &str = row.get("key");
        let content: &str = row.get("content");
        let category: &str = row.get("category");
        let wing: Option<&str> = row.get("wing");
        let hall: Option<&str> = row.get("hall");
        let confidence: f64 = row.get("confidence");
        let created_at: &str = row.get("created_at");

        table.add_row(vec![
            key.to_string(),
            preview(content, 100),
            category.to_string(),
            wing.unwrap_or("-").to_string(),
            hall.unwrap_or("-").to_string(),
            format!("{:.2}", confidence),
            created_at.get(..10).unwrap_or(created_at).to_string(),
        ]);
    }

    println!("{}", table);
    println!("{} result(s)", rows.len());
    Ok(())
}

pub async fn handle_memory_list(
    wing: Option<&str>,
    hall: Option<&str>,
    category: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<()> {
    let pool = open_spectral_db().await?;

    let mut sql = String::from(
        "SELECT key, category, wing, hall, content, created_at
         FROM memories WHERE 1=1",
    );
    let mut binds: Vec<String> = Vec::new();

    if let Some(w) = wing {
        sql.push_str(&format!(" AND wing = ?{}", binds.len() + 1));
        binds.push(w.to_string());
    }
    if let Some(h) = hall {
        sql.push_str(&format!(" AND hall = ?{}", binds.len() + 1));
        binds.push(h.to_string());
    }
    if let Some(c) = category {
        sql.push_str(&format!(" AND category = ?{}", binds.len() + 1));
        binds.push(c.to_string());
    }

    sql.push_str(&format!(
        " ORDER BY created_at DESC LIMIT ?{} OFFSET ?{}",
        binds.len() + 1,
        binds.len() + 2
    ));

    let mut q = sqlx::query(&sql);
    for b in &binds {
        q = q.bind(b);
    }
    q = q.bind(limit as i64).bind(offset as i64);

    let rows = q
        .fetch_all(&pool)
        .await
        .context("Failed to list memories")?;

    if rows.is_empty() {
        println!("No memories found.");
        return Ok(());
    }

    let mut table = Table::new();
    table
        .load_preset(presets::UTF8_FULL_CONDENSED)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            "Key", "Category", "Wing", "Hall", "Content", "Created",
        ]);

    for row in &rows {
        let key: &str = row.get("key");
        let category: &str = row.get("category");
        let wing: Option<&str> = row.get("wing");
        let hall: Option<&str> = row.get("hall");
        let content: &str = row.get("content");
        let created_at: &str = row.get("created_at");

        table.add_row(vec![
            key.to_string(),
            category.to_string(),
            wing.unwrap_or("-").to_string(),
            hall.unwrap_or("-").to_string(),
            preview(content, 100),
            created_at.get(..10).unwrap_or(created_at).to_string(),
        ]);
    }

    println!("{}", table);
    println!("{} result(s) (offset {})", rows.len(), offset);
    Ok(())
}

pub async fn handle_memory_add(
    key: &str,
    content: &str,
    category: Option<&str>,
    wing: Option<&str>,
    hall: Option<&str>,
) -> Result<()> {
    let pool = open_spectral_db().await?;

    let id = uuid::Uuid::new_v4().to_string();
    let cat = category.unwrap_or("manual");

    sqlx::query(
        "INSERT INTO memories (id, user_id, key, content, category, wing, hall, confidence)
         VALUES (?1, 'default', ?2, ?3, ?4, ?5, ?6, 1.0)",
    )
    .bind(&id)
    .bind(key)
    .bind(content)
    .bind(cat)
    .bind(wing)
    .bind(hall)
    .execute(&pool)
    .await
    .context("Failed to insert memory")?;

    println!("Memory created: {}", id);
    println!("  key:      {}", key);
    println!("  category: {}", cat);
    if let Some(w) = wing {
        println!("  wing:     {}", w);
    }
    if let Some(h) = hall {
        println!("  hall:     {}", h);
    }

    Ok(())
}
