//! Reading a run's dollar cost from the per-call cost ledger (#714).
//!
//! Each provider response appends one row to the `cost_ledger` table in the
//! session SQLite database, whose `cost_usd` column is the single canonical,
//! cache-aware cost figure (local/Ollama calls record `0`; a chargeable call
//! with no price in the model registry records `0` with `is_estimated = 1`).
//!
//! Because the eval gives every run its own isolated `PERMAGENT_PATH_ROOT`, the
//! entire ledger in that data root is exactly this one run — parent session plus
//! any sub-agents it summoned — so the run's total cost is simply
//! `SUM(cost_usd)` over the whole `cost_ledger` table. No session-id bookkeeping
//! and no cross-run contamination.
//!
//! The read is isolated behind [`CostReader`] so orchestration can be tested with
//! a mock, while [`LedgerCostReader`] (and its testable core [`read_ledger_db`])
//! is exercised against a real temporary SQLite database.

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};

/// The cost of a run as read from its ledger.
#[derive(Debug, Clone, PartialEq)]
pub struct CostReading {
    /// Total USD, or `None` when unknown (no ledger written at all — e.g. the run
    /// crashed before its first model call). `Some(0.0)` means genuinely free
    /// (all-local), which is different from unknown and must not be conflated.
    pub usd: Option<f64>,
    /// True if any ledger row is an under-count (`is_estimated = 1`): a chargeable
    /// call whose model had no price, so the true cost is higher than `usd`.
    pub estimated: bool,
    /// Number of ledger rows (provider responses) seen.
    pub ledger_rows: u64,
    /// Sum of `input_tokens` across ledger rows. On this ledger `input_tokens`
    /// is INCLUSIVE of the cached share (it is not "fresh tokens only") — see
    /// [`Self::cache_hit_rate`]. `None` when unknown (no ledger / no rows),
    /// never conflated with a genuine `Some(0)`.
    pub input_tokens: Option<i64>,
    /// Sum of `output_tokens` across ledger rows. `None` when unknown.
    pub output_tokens: Option<i64>,
    /// Sum of `cache_read_tokens` across ledger rows. `None` when unknown.
    pub cache_read_tokens: Option<i64>,
    /// Sum of `cache_write_tokens` across ledger rows. `None` when unknown.
    pub cache_write_tokens: Option<i64>,
}

impl CostReading {
    /// No ledger data — cost cannot be attributed.
    pub fn unknown() -> Self {
        Self {
            usd: None,
            estimated: false,
            ledger_rows: 0,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        }
    }

    /// A concrete reading with no token/cache detail (test/helper constructor).
    pub fn known(usd: f64, estimated: bool, ledger_rows: u64) -> Self {
        Self {
            usd: Some(usd),
            estimated,
            ledger_rows,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        }
    }

    /// A concrete reading including token/cache detail (test/helper
    /// constructor).
    #[allow(clippy::too_many_arguments)]
    pub fn known_with_tokens(
        usd: f64,
        estimated: bool,
        ledger_rows: u64,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        cache_read_tokens: Option<i64>,
        cache_write_tokens: Option<i64>,
    ) -> Self {
        Self {
            usd: Some(usd),
            estimated,
            ledger_rows,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
        }
    }

    /// Cache-read share of prompt input tokens: `cache_read_tokens /
    /// input_tokens`. `input_tokens` is inclusive of the cached share on this
    /// ledger, so this is literally "what fraction of the prompt was served
    /// from cache" — not "cache reads vs fresh-only tokens". Returns `None`
    /// (never `0.0`) when `input_tokens` is unknown or `Some(0)`, so an
    /// undefined ratio is never misread as "no cache hits".
    pub fn cache_hit_rate(&self) -> Option<f64> {
        let input = self.input_tokens?;
        if input <= 0 {
            return None;
        }
        let cache_read = self.cache_read_tokens.unwrap_or(0);
        Some(cache_read as f64 / input as f64)
    }
}

/// Reads the total cost of a completed run from its isolated data root.
pub trait CostReader {
    /// Total cost for the run whose session DB lives under `data_root`.
    fn read_total(&self, data_root: &Path) -> Result<CostReading>;
}

/// The production reader: opens the ledger SQLite DB under a data root.
#[derive(Debug, Default, Clone, Copy)]
pub struct LedgerCostReader;

impl LedgerCostReader {
    /// The ledger database path for a given `PERMAGENT_PATH_ROOT`.
    pub fn db_path(data_root: &Path) -> PathBuf {
        data_root.join("spectral").join("permagent.db")
    }
}

impl CostReader for LedgerCostReader {
    fn read_total(&self, data_root: &Path) -> Result<CostReading> {
        let db = Self::db_path(data_root);
        if !db.exists() {
            return Ok(CostReading::unknown());
        }
        read_ledger_db(&db)
    }
}

/// Sum the `cost_ledger` of an existing SQLite database. A database without the
/// `cost_ledger` table, or with an empty one, reads as [`CostReading::unknown`].
pub fn read_ledger_db(db_path: &Path) -> Result<CostReading> {
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("opening cost-ledger db {}", db_path.display()))?;

    let table_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='cost_ledger'",
            [],
            |row| row.get(0),
        )
        .context("probing for cost_ledger table")?;
    if table_exists == 0 {
        return Ok(CostReading::unknown());
    }

    #[allow(clippy::type_complexity)]
    let (rows, total, est_max, sum_input, sum_output, sum_cache_read, sum_cache_write): (
        i64,
        f64,
        i64,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    ) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(cost_usd), 0.0), COALESCE(MAX(is_estimated), 0), \
             SUM(input_tokens), SUM(output_tokens), SUM(cache_read_tokens), SUM(cache_write_tokens) \
             FROM cost_ledger",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .context("summing cost_ledger")?;

    if rows <= 0 {
        return Ok(CostReading::unknown());
    }
    Ok(CostReading {
        usd: Some(total),
        estimated: est_max != 0,
        ledger_rows: rows as u64,
        // Plain (non-COALESCEd) SUMs: a genuine SQL NULL — no rows carrying
        // that column, or the column not present — stays `None` here, never
        // silently becomes `Some(0)`.
        input_tokens: sum_input,
        output_tokens: sum_output,
        cache_read_tokens: sum_cache_read,
        cache_write_tokens: sum_cache_write,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    /// Create a ledger DB mirroring the real schema's relevant columns.
    fn seed_db(path: &Path, rows: &[(f64, i64)]) {
        seed_db_with_tokens(
            path,
            &rows
                .iter()
                .map(|(cost, est)| (*cost, *est, 0, 0, 0, 0))
                .collect::<Vec<_>>(),
        );
    }

    /// Like [`seed_db`] but including the token/cache columns, mirroring
    /// `apply_cost_ledger_schema` in `crates/goose/src/session/spectral_schema.rs`
    /// (`input_tokens`/`output_tokens`/`cache_read_tokens`/`cache_write_tokens`,
    /// all `INTEGER NOT NULL DEFAULT 0`).
    fn seed_db_with_tokens(path: &Path, rows: &[(f64, i64, i64, i64, i64, i64)]) {
        let conn = Connection::open(path).unwrap();
        conn.execute(
            "CREATE TABLE cost_ledger (\
                call_id TEXT PRIMARY KEY, \
                cost_usd REAL NOT NULL DEFAULT 0, \
                is_estimated INTEGER NOT NULL DEFAULT 0, \
                input_tokens INTEGER NOT NULL DEFAULT 0, \
                output_tokens INTEGER NOT NULL DEFAULT 0, \
                cache_read_tokens INTEGER NOT NULL DEFAULT 0, \
                cache_write_tokens INTEGER NOT NULL DEFAULT 0)",
            [],
        )
        .unwrap();
        for (i, (cost, est, input, output, cache_read, cache_write)) in rows.iter().enumerate() {
            conn.execute(
                "INSERT INTO cost_ledger \
                 (call_id, cost_usd, is_estimated, input_tokens, output_tokens, \
                  cache_read_tokens, cache_write_tokens) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    format!("call-{i}"),
                    cost,
                    est,
                    input,
                    output,
                    cache_read,
                    cache_write
                ],
            )
            .unwrap();
        }
    }

    #[test]
    fn sums_costs_across_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("permagent.db");
        seed_db(&db, &[(0.0123, 0), (0.0077, 0), (0.05, 0)]);
        let r = read_ledger_db(&db).unwrap();
        assert_eq!(r.ledger_rows, 3);
        assert!((r.usd.unwrap() - 0.07).abs() < 1e-9);
        assert!(!r.estimated);
    }

    #[test]
    fn all_local_reads_as_free_not_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("permagent.db");
        seed_db(&db, &[(0.0, 0), (0.0, 0)]);
        let r = read_ledger_db(&db).unwrap();
        assert_eq!(r.usd, Some(0.0));
        assert_eq!(r.ledger_rows, 2);
    }

    #[test]
    fn flags_estimated_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("permagent.db");
        seed_db(&db, &[(0.01, 0), (0.0, 1)]);
        let r = read_ledger_db(&db).unwrap();
        assert!(r.estimated);
    }

    #[test]
    fn empty_table_is_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("permagent.db");
        seed_db(&db, &[]);
        assert_eq!(read_ledger_db(&db).unwrap(), CostReading::unknown());
    }

    #[test]
    fn missing_table_is_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("empty.db");
        // Create a DB with no cost_ledger table.
        Connection::open(&db)
            .unwrap()
            .execute("CREATE TABLE other (x INTEGER)", [])
            .unwrap();
        assert_eq!(read_ledger_db(&db).unwrap(), CostReading::unknown());
    }

    #[test]
    fn reader_returns_unknown_when_db_missing() {
        let tmp = tempfile::tempdir().unwrap();
        // No spectral/permagent.db under this root.
        let r = LedgerCostReader.read_total(tmp.path()).unwrap();
        assert_eq!(r, CostReading::unknown());
    }

    #[test]
    fn reader_reads_db_under_data_root() {
        let tmp = tempfile::tempdir().unwrap();
        let db = LedgerCostReader::db_path(tmp.path());
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        seed_db(&db, &[(0.25, 0)]);
        let r = LedgerCostReader.read_total(tmp.path()).unwrap();
        assert_eq!(r.usd, Some(0.25));
    }

    #[test]
    fn sums_token_and_cache_columns_across_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("permagent.db");
        // (cost, est, input, output, cache_read, cache_write)
        seed_db_with_tokens(
            &db,
            &[(0.01, 0, 1000, 200, 400, 50), (0.02, 0, 500, 100, 100, 0)],
        );
        let r = read_ledger_db(&db).unwrap();
        assert_eq!(r.input_tokens, Some(1500));
        assert_eq!(r.output_tokens, Some(300));
        assert_eq!(r.cache_read_tokens, Some(500));
        assert_eq!(r.cache_write_tokens, Some(50));
    }

    #[test]
    fn empty_ledger_has_unknown_not_zero_tokens() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("permagent.db");
        seed_db_with_tokens(&db, &[]);
        let r = read_ledger_db(&db).unwrap();
        assert_eq!(r, CostReading::unknown());
        assert_eq!(r.input_tokens, None);
    }

    #[test]
    fn cache_hit_rate_divides_read_by_input() {
        let r = CostReading::known_with_tokens(
            0.03,
            false,
            2,
            Some(1500),
            Some(300),
            Some(500),
            Some(50),
        );
        assert!((r.cache_hit_rate().unwrap() - 500.0 / 1500.0).abs() < 1e-12);
    }

    #[test]
    fn cache_hit_rate_is_none_when_input_unknown_or_zero() {
        assert_eq!(CostReading::known(0.01, false, 1).cache_hit_rate(), None);
        let zero_input =
            CostReading::known_with_tokens(0.0, false, 1, Some(0), Some(0), Some(0), Some(0));
        assert_eq!(zero_input.cache_hit_rate(), None);
    }

    #[test]
    fn cache_hit_rate_treats_missing_cache_read_as_zero_not_unknown() {
        // input known, cache_read unknown => 0 cache reads, not an undefined ratio.
        let r = CostReading::known_with_tokens(0.01, false, 1, Some(100), Some(10), None, None);
        assert_eq!(r.cache_hit_rate(), Some(0.0));
    }
}
