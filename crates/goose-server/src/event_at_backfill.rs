//! One-time backfill of `event_at` for imported Brain memories (#92).
//!
//! Bulk imports (`import-core`, `import-task`, `import-conversation`,
//! `import-openbird`, `import-daily`, …) stamp `created_at` with the moment the
//! batch import ran, not when the underlying event actually happened. The Brain
//! view's today/all-time slider then clusters (say) March Slack history at the
//! import date instead of the conversation date.
//!
//! Original timestamps usually live *inside* the memory content (Slack messages
//! carry a date, browser history carries a navigation time). This module adds an
//! additive, nullable `event_at` column and does a one-time pass over imported
//! rows, extracting an event timestamp from content and recording it. The
//! timeline query then orders by `COALESCE(event_at, created_at)`, so rows we
//! couldn't date fall back to their old behavior.
//!
//! Runs at daemon boot, right after the Brain (Spectral) opens and migrates its
//! own schema — see `state.rs`. Idempotent and non-fatal: rows already carrying
//! an `event_at` are skipped, rows we can't date stay NULL and are retried
//! cheaply on the next boot (which also picks up any newly imported rows).

use std::path::Path;
use std::time::Duration;

use chrono::{NaiveDate, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};

/// Outcome of a backfill pass, for logging.
#[derive(Debug, Default, Clone, Copy)]
pub struct BackfillStats {
    /// The `event_at` column did not exist and was added this run.
    pub column_added: bool,
    /// Imported rows examined this run (event_at was NULL).
    pub rows_examined: usize,
    /// Rows we successfully dated and updated this run.
    pub rows_backfilled: usize,
}

/// Add `event_at` (if missing) and backfill it for imported memories.
///
/// `db_path` is the Brain's `memory.db`. Errors are returned so the caller can
/// log-and-continue; nothing here is fatal to startup.
pub fn backfill_event_at(db_path: &Path) -> Result<BackfillStats, String> {
    let mut conn = Connection::open(db_path).map_err(|e| format!("open memory.db: {e}"))?;
    // The Brain (Spectral) holds this DB open in WAL mode; give writes a moment
    // to acquire the lock instead of failing fast on SQLITE_BUSY.
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;

    // If the memories table isn't present yet (fresh brain, not migrated), there
    // is nothing to do — the next boot after the first import will catch it.
    if !table_exists(&conn, "memories")? {
        return Ok(BackfillStats::default());
    }
    // A `source` column is expected; if the schema predates it, only key-based
    // matching applies, which is still valid.
    let has_source = column_exists(&conn, "memories", "source")?;

    // Additive, idempotent column add.
    let column_added = if column_exists(&conn, "memories", "event_at")? {
        false
    } else {
        conn.execute("ALTER TABLE memories ADD COLUMN event_at TEXT", [])
            .map_err(|e| format!("add event_at column: {e}"))?;
        true
    };

    // Only imported rows are in scope, and only those not yet dated. Match on
    // both `source` and `key` so the import tag is caught wherever it lives.
    let where_import = if has_source {
        "event_at IS NULL AND (source LIKE 'import%' OR key LIKE 'import%')"
    } else {
        "event_at IS NULL AND key LIKE 'import%'"
    };
    let select_sql = format!("SELECT id, content FROM memories WHERE {where_import}");

    let candidates: Vec<(String, String)> = {
        let mut stmt = conn.prepare(&select_sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        rows.filter_map(|r| r.ok()).collect()
    };

    let rows_examined = candidates.len();
    let mut rows_backfilled = 0usize;

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for (id, content) in candidates {
        if let Some(event_at) = extract_event_at(&content) {
            tx.execute(
                "UPDATE memories SET event_at = ?1 WHERE id = ?2",
                params![event_at, id],
            )
            .map_err(|e| e.to_string())?;
            rows_backfilled += 1;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;

    Ok(BackfillStats {
        column_added,
        rows_examined,
        rows_backfilled,
    })
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
        params![name],
        |_| Ok(true),
    )
    .optional()
    .map(|o| o.unwrap_or(false))
    .map_err(|e| e.to_string())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool, String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let name: String = row.get(1).map_err(|e| e.to_string())?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Extract an event timestamp embedded in memory content, normalized to an
/// RFC 3339 UTC string comparable with `created_at`.
///
/// Recognizes the first plausible ISO-8601-ish date or datetime — the shape
/// imported content actually carries (`2026-03-14T09:30:00Z`,
/// `[2026-03-14 09:30:00]`, a bare `2026-03-14`). A time without a zone is read
/// as UTC; a date without a time is midnight UTC. Years are constrained to
/// 2000–2100 so stray digit runs aren't misread as dates. Returns `None` when
/// no plausible timestamp is present, leaving the row on its `created_at`
/// fallback.
pub fn extract_event_at(content: &str) -> Option<String> {
    // date (y-m-d), optional [ T]time (HH:MM[:SS]), optional fractional secs,
    // optional zone (Z or ±HH[:]MM).
    let re = regex::Regex::new(
        r"(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}:\d{2}(?::\d{2})?)(?:\.\d+)?\s*(Z|[+-]\d{2}:?\d{2})?)?",
    )
    .ok()?;
    extract_with(&re, content)
}

fn extract_with(re: &regex::Regex, content: &str) -> Option<String> {
    let caps = re.captures(content)?;
    // YYYY-MM-DD from the three date groups (avoids slicing the whole match).
    let date = format!("{}-{}-{}", &caps[1], &caps[2], &caps[3]);
    let nd = NaiveDate::parse_from_str(&date, "%Y-%m-%d").ok()?;
    if !(2000..=2100).contains(&chrono::Datelike::year(&nd)) {
        return None;
    }

    let time = caps.get(4).map(|m| m.as_str());
    let Some(time) = time else {
        // Date only → midnight UTC.
        let ndt = nd.and_hms_opt(0, 0, 0)?;
        return Some(Utc.from_utc_datetime(&ndt).to_rfc3339());
    };

    // Normalize to seconds precision and a concrete zone, then parse.
    let time = if time.len() == 5 {
        format!("{time}:00")
    } else {
        time.to_string()
    };
    let zone = match caps.get(5).map(|m| m.as_str()) {
        None | Some("Z") => "Z".to_string(),
        Some(off) if off.contains(':') => off.to_string(),
        // "+0100" → "+01:00": split before the final two minute digits.
        Some(off) => {
            let (hours, minutes) = off.split_at(off.len() - 2);
            format!("{hours}:{minutes}")
        }
    };
    let rfc = format!("{date}T{time}{zone}");
    chrono::DateTime::parse_from_rfc3339(&rfc)
        .ok()
        .map(|dt| dt.with_timezone(&Utc).to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn re() -> regex::Regex {
        regex::Regex::new(
            r"(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}:\d{2}(?::\d{2})?)(?:\.\d+)?\s*(Z|[+-]\d{2}:?\d{2})?)?",
        )
        .unwrap()
    }

    #[test]
    fn extracts_iso_datetime_with_zulu() {
        let out = extract_event_at("Slack message at 2026-03-14T09:30:00Z about the shed").unwrap();
        assert_eq!(out, "2026-03-14T09:30:00+00:00");
    }

    #[test]
    fn extracts_bracketed_space_datetime() {
        let out = extract_event_at("[2026-03-14 09:30:00] jesse: ping").unwrap();
        assert_eq!(out, "2026-03-14T09:30:00+00:00");
    }

    #[test]
    fn extracts_date_only_as_midnight() {
        let out = extract_event_at("daily note for 2026-03-14 — groceries").unwrap();
        assert_eq!(out, "2026-03-14T00:00:00+00:00");
    }

    #[test]
    fn honors_explicit_offset() {
        let out = extract_event_at("navigated at 2026-03-14T09:30:00+02:00").unwrap();
        // 09:30 +02:00 == 07:30 UTC
        assert_eq!(out, "2026-03-14T07:30:00+00:00");
    }

    #[test]
    fn normalizes_compact_offset() {
        let out = extract_with(&re(), "2026-03-14T09:30:00+0200").unwrap();
        assert_eq!(out, "2026-03-14T07:30:00+00:00");
    }

    #[test]
    fn takes_first_timestamp() {
        let out = extract_event_at("edited 2026-05-01 (orig 2026-03-14T09:30:00Z)").unwrap();
        assert_eq!(out, "2026-05-01T00:00:00+00:00");
    }

    #[test]
    fn rejects_content_without_timestamp() {
        assert!(extract_event_at("just some prose, no dates here").is_none());
        // A long opaque id must not be misread as a date.
        assert!(extract_event_at("task_0e7a5d3f-4b21-9c88 done").is_none());
    }

    #[test]
    fn rejects_implausible_year() {
        assert!(extract_event_at("version 1999-99 build").is_none());
        assert!(extract_event_at("ref 1899-01-01").is_none());
    }

    // ── Full backfill against an in-memory-ish sqlite file ────────────────

    fn seed_db(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE memories (
                 id TEXT PRIMARY KEY,
                 key TEXT,
                 content TEXT,
                 source TEXT,
                 created_at TEXT
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories VALUES ('m1','import-core','Slack at 2026-03-14T09:30:00Z','import-core','2026-07-01T00:00:00Z')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO memories VALUES ('m2','import-daily','note 2026-02-02','import-daily','2026-07-01T00:00:00Z')",
            [],
        ).unwrap();
        // Imported but undateable — stays NULL.
        conn.execute(
            "INSERT INTO memories VALUES ('m3','import-task','no date in here','import-task','2026-07-01T00:00:00Z')",
            [],
        ).unwrap();
        // Not imported — must be left untouched even though it has a date.
        conn.execute(
            "INSERT INTO memories VALUES ('m4','session:chat','said 2026-01-01','permagent.chat','2026-07-01T00:00:00Z')",
            [],
        ).unwrap();
    }

    #[test]
    fn backfill_adds_column_and_dates_imported_rows() {
        let dir = std::env::temp_dir().join(format!("evbf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("memory.db");
        let _ = std::fs::remove_file(&path);
        seed_db(&path);

        let stats = backfill_event_at(&path).unwrap();
        assert!(stats.column_added);
        assert_eq!(stats.rows_examined, 3); // m1, m2, m3 (imported, NULL)
        assert_eq!(stats.rows_backfilled, 2); // m1, m2 dated; m3 undateable

        let conn = Connection::open(&path).unwrap();
        let ev1: Option<String> = conn
            .query_row("SELECT event_at FROM memories WHERE id='m1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(ev1.as_deref(), Some("2026-03-14T09:30:00+00:00"));
        let ev3: Option<String> = conn
            .query_row("SELECT event_at FROM memories WHERE id='m3'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(ev3, None);
        let ev4: Option<String> = conn
            .query_row("SELECT event_at FROM memories WHERE id='m4'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(ev4, None, "non-imported rows must not be dated");

        // Second run is idempotent: no column re-add, m1/m2 already set so only
        // m3 is re-examined and still can't be dated.
        let stats2 = backfill_event_at(&path).unwrap();
        assert!(!stats2.column_added);
        assert_eq!(stats2.rows_backfilled, 0);
        assert_eq!(stats2.rows_examined, 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn backfill_no_memories_table_is_noop() {
        let dir = std::env::temp_dir().join(format!("evbf-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("memory.db");
        let _ = std::fs::remove_file(&path);
        Connection::open(&path).unwrap(); // empty db, no tables
        let stats = backfill_event_at(&path).unwrap();
        assert!(!stats.column_added);
        assert_eq!(stats.rows_examined, 0);
        let _ = std::fs::remove_file(&path);
    }
}
