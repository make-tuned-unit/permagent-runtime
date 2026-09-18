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
    /// K18 (`## A45 design` Q3): sum of `cost_usd` across ledger rows whose
    /// `parent_session_id IS NULL` — the coordinator's own spend, as
    /// distinct from any dispatched worker. `None` when the ledger carries
    /// no role/provider columns (an old-schema DB) or has no rows at all;
    /// never conflated with a genuine `Some(0.0)`.
    pub coordinator_usd: Option<f64>,
    /// K18: sum of `cost_usd` across ledger rows whose `parent_session_id`
    /// IS set — every dispatched goal worker is a distinct child session
    /// (`InternalSubagentEngine::spawn` via `create_session_with_parent`).
    /// `None` under the same conditions as [`Self::coordinator_usd`].
    pub worker_usd: Option<f64>,
    /// K18: sum of `input_tokens + output_tokens` across worker rows whose
    /// `provider = "ollama"` (a local, `$0` call — module doc above).
    /// `None` when no such rows exist, never conflated with `Some(0)`.
    pub worker_local_tokens: Option<i64>,
    /// K18: the USD "offset" [`Self::worker_local_tokens`] represents —
    /// what those tokens would have cost had they run on the MECHANICAL
    /// pack's cloud rung instead of locally (a shadow cost against the
    /// SAME pricing source the cost ledger and `cheapest_capable_worker`
    /// already use, not a new price table). See [`worker_offset_usd`].
    /// `None` exactly when [`Self::worker_local_tokens`] is `None`.
    pub worker_offset_usd: Option<f64>,
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
            coordinator_usd: None,
            worker_usd: None,
            worker_local_tokens: None,
            worker_offset_usd: None,
        }
    }

    /// A concrete reading with no token/cache/role detail (test/helper
    /// constructor).
    pub fn known(usd: f64, estimated: bool, ledger_rows: u64) -> Self {
        Self {
            usd: Some(usd),
            estimated,
            ledger_rows,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            coordinator_usd: None,
            worker_usd: None,
            worker_local_tokens: None,
            worker_offset_usd: None,
        }
    }

    /// A concrete reading including token/cache detail but no role detail
    /// (test/helper constructor).
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
            coordinator_usd: None,
            worker_usd: None,
            worker_local_tokens: None,
            worker_offset_usd: None,
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

/// K18: Anthropic's published per-Mtok list price for `cost_router::packs`'s
/// `DEFAULT_MECHANICAL_MODEL` (`claude-haiku-4-5-20251001`) — mirrored BY
/// HAND from `crates/goose/src/cost_router/knowledge.rs`'s
/// `claude-haiku-4-5-20251001` row (`input_usd_per_mtok: 1.00`,
/// `output_usd_per_mtok: 5.00`). This crate cannot import that value: `crates/
/// goose` (package `permagent`) already depends on `permagent-eval`
/// (`graduate.rs`'s module doc explains the resulting cyclic-package-
/// dependency constraint in full), and `cost_router::reservation::
/// resolve_pricing`/`knowledge::lookup` are `pub(crate)` inside that crate
/// besides. Real duplication with a real drift risk, same honest cost
/// `graduate.rs` already pays for its own mirrored constants.
const MECHANICAL_PACK_INPUT_USD_PER_MTOK: f64 = 1.00;
const MECHANICAL_PACK_OUTPUT_USD_PER_MTOK: f64 = 5.00;

/// A blended USD-per-token rate for the MECHANICAL pack's cloud rung,
/// derived the same way `cost_router::routing_matrix::nominal_call_cost`
/// derives its own per-call estimate (a nominal 8,000 input + 1,000 output
/// token call), collapsed to a single per-token rate so a raw token count
/// can be priced without a new pricing table. See [`worker_offset_usd`].
fn mechanical_pack_per_token_rate_usd() -> f64 {
    const NOMINAL_INPUT_TOKENS: f64 = 8_000.0;
    const NOMINAL_OUTPUT_TOKENS: f64 = 1_000.0;
    let nominal_call_cost_usd = (NOMINAL_INPUT_TOKENS * MECHANICAL_PACK_INPUT_USD_PER_MTOK
        + NOMINAL_OUTPUT_TOKENS * MECHANICAL_PACK_OUTPUT_USD_PER_MTOK)
        / 1_000_000.0;
    nominal_call_cost_usd / (NOMINAL_INPUT_TOKENS + NOMINAL_OUTPUT_TOKENS)
}

/// K18 (`## A45 design` Q3): the USD "offset" `worker_local_tokens`
/// represents — what those tokens would have cost had they run on the
/// MECHANICAL pack's cloud rung instead of locally on `$0` Ollama, i.e.
/// "what the worker's tokens would have cost on the cloud rung it was
/// spared." A shadow cost against the SAME pricing source the cost ledger
/// already uses, not a new pricing table. `None` exactly when
/// `worker_local_tokens` is `None` (no local-worker ledger rows at all),
/// never fabricated as `Some(0.0)`.
pub fn worker_offset_usd(worker_local_tokens: Option<i64>) -> Option<f64> {
    worker_local_tokens.map(|tokens| tokens as f64 * mechanical_pack_per_token_rate_usd())
}

/// Whether `table` has a column named `column`, via `PRAGMA table_info`.
/// Used to stay backward-compatible with a `cost_ledger` seeded before the
/// role/provider columns existed (or, in tests, a deliberately reduced
/// fixture schema) — such a DB simply reads role fields as `None` rather
/// than erroring.
fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .with_context(|| format!("preparing table_info({table})"))?;
    let mut rows = stmt
        .query([])
        .with_context(|| format!("querying table_info({table})"))?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// K18: per-role (coordinator vs. worker) cost/token totals from an open
/// `cost_ledger` connection. `(coordinator_usd, worker_usd,
/// worker_local_tokens)` — each `None` when the connection's `cost_ledger`
/// lacks the `provider`/`parent_session_id` columns this needs (see
/// [`has_column`]), or when no matching rows exist.
fn read_role_cost(conn: &Connection) -> Result<(Option<f64>, Option<f64>, Option<i64>)> {
    if !has_column(conn, "cost_ledger", "parent_session_id")?
        || !has_column(conn, "cost_ledger", "provider")?
    {
        return Ok((None, None, None));
    }

    let mut coordinator_usd = None;
    let mut worker_usd = None;
    {
        let mut stmt = conn
            .prepare(
                "SELECT CASE WHEN parent_session_id IS NULL THEN 'coordinator' ELSE 'worker' END \
                 AS role, SUM(cost_usd) FROM cost_ledger GROUP BY role",
            )
            .context("preparing role cost_usd grouping")?;
        let mut rows = stmt.query([]).context("querying role cost_usd grouping")?;
        while let Some(row) = rows.next()? {
            let role: String = row.get(0)?;
            let sum: f64 = row.get(1)?;
            match role.as_str() {
                "coordinator" => coordinator_usd = Some(sum),
                "worker" => worker_usd = Some(sum),
                _ => {}
            }
        }
    }

    // Local Ollama worker rows already record `cost_usd = 0` (module doc
    // above) — this is deliberately the raw token sum for that slice, not
    // a dollar figure; `worker_offset_usd` turns it into one.
    let worker_local_tokens: Option<i64> = conn
        .query_row(
            "SELECT SUM(COALESCE(input_tokens, 0) + COALESCE(output_tokens, 0)) \
             FROM cost_ledger WHERE parent_session_id IS NOT NULL AND provider = 'ollama'",
            [],
            |row| row.get(0),
        )
        .context("summing worker-local tokens")?;

    Ok((coordinator_usd, worker_usd, worker_local_tokens))
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
    let (coordinator_usd, worker_usd, worker_local_tokens) = read_role_cost(&conn)?;
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
        coordinator_usd,
        worker_usd,
        worker_offset_usd: worker_offset_usd(worker_local_tokens),
        worker_local_tokens,
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

    // ── K18: per-role, per-provider cost accounting ─────────────────────────

    /// Seed a ledger DB with the role/provider columns K18 reads
    /// (`provider`, `parent_session_id`), mirroring the real schema's
    /// relevant columns the same way [`seed_db`]/[`seed_db_with_tokens`] do
    /// for the columns they cover.
    fn seed_db_with_roles(
        path: &Path,
        // (cost_usd, provider, parent_session_id, input_tokens, output_tokens)
        rows: &[(f64, &str, Option<&str>, i64, i64)],
    ) {
        let conn = Connection::open(path).unwrap();
        conn.execute(
            "CREATE TABLE cost_ledger (\
                call_id TEXT PRIMARY KEY, \
                cost_usd REAL NOT NULL DEFAULT 0, \
                is_estimated INTEGER NOT NULL DEFAULT 0, \
                provider TEXT, \
                parent_session_id TEXT, \
                input_tokens INTEGER NOT NULL DEFAULT 0, \
                output_tokens INTEGER NOT NULL DEFAULT 0, \
                cache_read_tokens INTEGER NOT NULL DEFAULT 0, \
                cache_write_tokens INTEGER NOT NULL DEFAULT 0)",
            [],
        )
        .unwrap();
        for (i, (cost, provider, parent, input, output)) in rows.iter().enumerate() {
            conn.execute(
                "INSERT INTO cost_ledger \
                 (call_id, cost_usd, is_estimated, provider, parent_session_id, \
                  input_tokens, output_tokens, cache_read_tokens, cache_write_tokens) \
                 VALUES (?1, ?2, 0, ?3, ?4, ?5, ?6, 0, 0)",
                params![format!("call-{i}"), cost, provider, parent, input, output],
            )
            .unwrap();
        }
    }

    #[test]
    fn role_cost_groups_coordinator_vs_worker_across_two_providers() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("permagent.db");
        seed_db_with_roles(
            &db,
            &[
                // Coordinator: two anthropic calls, no parent session.
                (0.05, "anthropic", None, 1000, 200),
                (0.03, "anthropic", None, 500, 100),
                // Worker (child session "w1"): one paid anthropic call...
                (0.02, "anthropic", Some("w1"), 400, 80),
                // ...and one $0 local ollama call.
                (0.0, "ollama", Some("w1"), 900, 300),
                // Worker (child session "w2"): another $0 local ollama call.
                (0.0, "ollama", Some("w2"), 600, 150),
            ],
        );
        let r = read_ledger_db(&db).unwrap();
        // Hand computation: coordinator = 0.05 + 0.03; worker = 0.02 + 0 + 0.
        assert!((r.coordinator_usd.unwrap() - 0.08).abs() < 1e-9);
        assert!((r.worker_usd.unwrap() - 0.02).abs() < 1e-9);
        // worker_local_tokens: only the two ollama worker rows.
        // (900+300) + (600+150) = 1200 + 750 = 1950.
        assert_eq!(r.worker_local_tokens, Some(1950));
        assert_eq!(
            r.worker_offset_usd,
            worker_offset_usd(Some(1950)),
            "worker_offset_usd must be derived from worker_local_tokens via the same fn"
        );
        // Total run cost is unaffected by role accounting: sanity check.
        assert!((r.usd.unwrap() - 0.10).abs() < 1e-9);
    }

    #[test]
    fn role_cost_is_none_when_only_coordinator_rows_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("permagent.db");
        seed_db_with_roles(&db, &[(0.01, "anthropic", None, 100, 10)]);
        let r = read_ledger_db(&db).unwrap();
        assert_eq!(r.coordinator_usd, Some(0.01));
        assert_eq!(
            r.worker_usd, None,
            "no worker rows at all => None, not Some(0.0)"
        );
        assert_eq!(r.worker_local_tokens, None);
        assert_eq!(r.worker_offset_usd, None);
    }

    #[test]
    fn role_cost_is_none_when_worker_rows_are_all_paid_not_local() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("permagent.db");
        seed_db_with_roles(
            &db,
            &[
                (0.01, "anthropic", None, 100, 10),
                (0.02, "anthropic", Some("w1"), 200, 20),
            ],
        );
        let r = read_ledger_db(&db).unwrap();
        assert_eq!(r.worker_usd, Some(0.02));
        assert_eq!(
            r.worker_local_tokens, None,
            "worker ran on paid anthropic, not local ollama => no local tokens to offset"
        );
        assert_eq!(r.worker_offset_usd, None);
    }

    #[test]
    fn role_cost_is_none_on_a_schema_without_role_columns() {
        // The pre-K18 reduced fixture schema (no `provider`/`parent_session_id`
        // columns) must still read cleanly — role fields degrade to `None`,
        // never error.
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("permagent.db");
        seed_db(&db, &[(0.05, 0)]);
        let r = read_ledger_db(&db).unwrap();
        assert_eq!(r.usd, Some(0.05));
        assert_eq!(r.coordinator_usd, None);
        assert_eq!(r.worker_usd, None);
        assert_eq!(r.worker_local_tokens, None);
        assert_eq!(r.worker_offset_usd, None);
    }

    #[test]
    fn worker_offset_usd_is_none_without_local_tokens() {
        assert_eq!(worker_offset_usd(None), None);
    }

    #[test]
    fn worker_offset_usd_matches_the_blended_mechanical_pack_rate() {
        // Hand computation against the cited MECHANICAL pack fixture
        // (`claude-haiku-4-5-20251001`: $1.00/Mtok in, $5.00/Mtok out),
        // collapsed via the same 8,000-in/1,000-out nominal-call shape
        // `cost_router::routing_matrix::nominal_call_cost` uses:
        // rate = (8000*1.00 + 1000*5.00) / 1e6 / 9000 = 0.013 / 9000.
        let rate = 0.013 / 9000.0;
        let got = worker_offset_usd(Some(9_000)).unwrap();
        assert!((got - 9_000.0 * rate).abs() < 1e-12);
        assert!((got - 0.013).abs() < 1e-9);
    }

    #[test]
    fn worker_offset_usd_scales_linearly_with_tokens() {
        let one = worker_offset_usd(Some(1_000)).unwrap();
        let ten = worker_offset_usd(Some(10_000)).unwrap();
        assert!((ten - one * 10.0).abs() < 1e-12);
    }
}
