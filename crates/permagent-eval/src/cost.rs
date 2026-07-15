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
}

impl CostReading {
    /// No ledger data — cost cannot be attributed.
    pub fn unknown() -> Self {
        Self {
            usd: None,
            estimated: false,
            ledger_rows: 0,
        }
    }

    /// A concrete reading (test/helper constructor).
    pub fn known(usd: f64, estimated: bool, ledger_rows: u64) -> Self {
        Self {
            usd: Some(usd),
            estimated,
            ledger_rows,
        }
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

    let (rows, total, est_max): (i64, f64, i64) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(cost_usd), 0.0), COALESCE(MAX(is_estimated), 0) \
             FROM cost_ledger",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .context("summing cost_ledger")?;

    if rows <= 0 {
        return Ok(CostReading::unknown());
    }
    Ok(CostReading {
        usd: Some(total),
        estimated: est_max != 0,
        ledger_rows: rows as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    /// Create a ledger DB mirroring the real schema's relevant columns.
    fn seed_db(path: &Path, rows: &[(f64, i64)]) {
        let conn = Connection::open(path).unwrap();
        conn.execute(
            "CREATE TABLE cost_ledger (\
                call_id TEXT PRIMARY KEY, \
                cost_usd REAL NOT NULL DEFAULT 0, \
                is_estimated INTEGER NOT NULL DEFAULT 0)",
            [],
        )
        .unwrap();
        for (i, (cost, est)) in rows.iter().enumerate() {
            conn.execute(
                "INSERT INTO cost_ledger (call_id, cost_usd, is_estimated) VALUES (?1, ?2, ?3)",
                params![format!("call-{i}"), cost, est],
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
}
