//! One-shot, idempotent repair for approval records misfiled into the `event`
//! hall.
//!
//! # Why this exists
//!
//! [`crate::hall_rules`] puts approval-shaped memories — "the user was asked X
//! and answered Y" — into the `fact` hall, because they are durable knowledge
//! of what was decided, not a thing that merely happened at a moment. That rule
//! only applies at **write** time. Memories written before the rule existed kept
//! whatever hall the Spectral defaults gave them, which for this shape is
//! `event`.
//!
//! A misfiled hall is not cosmetic: the hall is TACT's tier-1 routing axis, so
//! an approval sitting in `event` is not reachable the way an approval should
//! be. This module walks those rows once and moves them, using
//! [`SafeBrain::set_hall`] so Spectral re-hashes the constellation fingerprints
//! the memory participates in — a raw `UPDATE memories SET hall` would move the
//! column and leave the routing index behind.
//!
//! # Safety
//!
//! This is the only mutating maintenance op that rewrites existing rows in
//! place, so it **snapshots all three brain databases before touching
//! anything** (the same [`crate::backup::force_snapshot`] path the backup route
//! uses). If any snapshot fails, nothing is written and the error is returned.
//!
//! # Idempotence
//!
//! The selection predicate and the repair are complementary: once a row's hall
//! is `fact` it no longer matches, so a second run selects zero rows and is a
//! no-op. Re-running after a partial failure resumes exactly where it stopped.
//!
//! # Honest counts
//!
//! `before` and `after` are the same `COUNT(*)` over the same predicate, read
//! from the database on either side of the repair. A non-zero `after` means
//! `set_hall` did not reach some rows; those ids are reported verbatim in
//! [`HallBackfillReport::missed`] and [`HallBackfillReport::errors`]. Nothing
//! here rounds a remainder away or infers success from the absence of an error.

use crate::backup::{self, DbTarget, SnapshotMode};
use crate::brain_handle::SafeBrain;
use crate::hall_rules::APPROVAL_HALL;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// The three brain databases, snapshotted before any write.
const SNAPSHOT_TARGETS: [(DbTarget, &str); 3] = [
    (DbTarget::Brain, "memory.db"),
    (DbTarget::BrainGraph, "graph.sqlite"),
    (DbTarget::BrainRecognition, "recognition.db"),
];

/// The `WHERE` clause that defines "an approval record misfiled as an event".
///
/// Kept as one constant because the count and the selection MUST agree — a
/// count over a different predicate than the repair is how a backfill reports a
/// clean zero over rows it never looked at.
pub const MISFILED_APPROVAL_WHERE: &str =
    "content LIKE '%was asked:%' AND content LIKE '%answered:%' AND hall = 'event'";

/// One row the repair could not apply.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowFailure {
    pub id: String,
    pub error: String,
}

/// What one snapshot did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotOutcome {
    pub db: String,
    pub filename: String,
    pub size_bytes: u64,
    pub integrity_ok: bool,
}

/// The result of a backfill run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HallBackfillReport {
    /// Snapshots taken before any write. Always three on success.
    pub snapshots: Vec<SnapshotOutcome>,
    /// `COUNT(*)` over [`MISFILED_APPROVAL_WHERE`] before the repair.
    pub before: i64,
    /// The same `COUNT(*)` after. Expected 0; a remainder is a failure to
    /// report, not to round away.
    pub after: i64,
    /// Rows selected for repair.
    pub attempted: usize,
    /// Rows Spectral confirmed it moved (`set_hall` returned `true`).
    pub repaired: usize,
    /// Ids `set_hall` returned `false` for — the memory was not found. A miss,
    /// not an error, and still a row that did not move.
    pub missed: Vec<String>,
    /// Ids `set_hall` returned an error for, with the error verbatim.
    pub errors: Vec<RowFailure>,
}

impl HallBackfillReport {
    /// Whether the run left the brain in the intended state: nothing remaining,
    /// nothing missed, nothing errored.
    pub fn is_clean(&self) -> bool {
        self.after == 0 && self.missed.is_empty() && self.errors.is_empty()
    }

    /// A one-line summary for an agent to narrate or a log line to carry.
    pub fn summary(&self) -> String {
        if self.is_clean() {
            format!(
                "Hall backfill complete: {} approval records moved from `event` to `{}` ({} -> 0 remaining). {} snapshots taken first.",
                self.repaired,
                APPROVAL_HALL,
                self.before,
                self.snapshots.len(),
            )
        } else {
            format!(
                "Hall backfill INCOMPLETE: {} of {} moved, {} remaining (was {}), {} not found, {} errored. {} snapshots taken first.",
                self.repaired,
                self.attempted,
                self.after,
                self.before,
                self.missed.len(),
                self.errors.len(),
                self.snapshots.len(),
            )
        }
    }
}

/// Open the brain's `memory.db` read-only.
fn open_memory_db_read_only(brain_dir: &Path) -> Result<rusqlite::Connection, String> {
    let db_path = brain_dir.join("memory.db");
    rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("open {}: {e}", db_path.display()))
}

/// `COUNT(*)` of approval records still misfiled as events.
fn count_misfiled(conn: &rusqlite::Connection) -> Result<i64, String> {
    conn.query_row(
        &format!("SELECT COUNT(*) FROM memories WHERE {MISFILED_APPROVAL_WHERE}"),
        [],
        |row| row.get::<_, i64>(0),
    )
    .map_err(|e| format!("count misfiled approvals: {e}"))
}

/// The ids of approval records still misfiled as events.
fn select_misfiled(conn: &rusqlite::Connection) -> Result<Vec<String>, String> {
    let sql = format!("SELECT id FROM memories WHERE {MISFILED_APPROVAL_WHERE}");
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("prepare misfiled select: {e}"))?;
    let ids = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("select misfiled approvals: {e}"))?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|e| format!("read misfiled approval ids: {e}"))?;
    Ok(ids)
}

/// Snapshot all three brain databases. Returns `Err` on the first failure so
/// that no write happens without a full set of snapshots behind it.
async fn snapshot_all(
    brain_dir: &Path,
    backup_root: &Path,
) -> Result<Vec<SnapshotOutcome>, String> {
    let mut out = Vec::with_capacity(SNAPSHOT_TARGETS.len());
    for (target, filename) in SNAPSHOT_TARGETS {
        let source: PathBuf = brain_dir.join(filename);
        let root = backup_root.to_path_buf();
        let info = tokio::task::spawn_blocking(move || {
            backup::force_snapshot(&source, &root, target, SnapshotMode::Compacted)
        })
        .await
        .map_err(|e| format!("snapshot task panicked for {filename}: {e}"))?
        .map_err(|e| format!("snapshot {filename}: {e}"))?;

        out.push(SnapshotOutcome {
            db: info.db,
            filename: info.filename,
            size_bytes: info.size_bytes,
            integrity_ok: info.integrity_ok,
        });
    }
    Ok(out)
}

/// Move every approval record misfiled as an `event` into the `fact` hall.
///
/// Snapshots all three brain databases first; if any snapshot fails, nothing is
/// written. Safe to re-run — see the module docs.
///
/// `brain_dir` and `backup_root` are parameters rather than reads of
/// [`crate::config::paths::Paths`] so that tests can run this against a fixture
/// brain. Production callers pass the real paths via [`run_on_default_paths`].
pub async fn run(
    brain: &SafeBrain,
    brain_dir: &Path,
    backup_root: &Path,
) -> Result<HallBackfillReport, String> {
    // 1. Snapshot before anything is written. A failure here aborts.
    let snapshots = snapshot_all(brain_dir, backup_root).await?;

    // 2. Read the before-count and the work list from the same connection, so
    //    they cannot disagree about what "misfiled" means.
    let (before, ids) = {
        let conn = open_memory_db_read_only(brain_dir)?;
        let before = count_misfiled(&conn)?;
        let ids = select_misfiled(&conn)?;
        (before, ids)
    };

    tracing::info!(
        target: "permagent::hall_backfill",
        before,
        selected = ids.len(),
        snapshots = snapshots.len(),
        "Hall backfill starting"
    );

    // 3. Repair each row through Spectral, so the routing index moves with the
    //    column.
    let attempted = ids.len();
    let mut repaired = 0usize;
    let mut missed = Vec::new();
    let mut errors = Vec::new();

    for id in ids {
        match brain.set_hall(&id, APPROVAL_HALL).await {
            Ok(true) => repaired += 1,
            Ok(false) => {
                tracing::warn!(
                    target: "permagent::hall_backfill",
                    memory_id = %id,
                    "set_hall reported the memory does not exist — not repaired"
                );
                missed.push(id);
            }
            Err(e) => {
                tracing::warn!(
                    target: "permagent::hall_backfill",
                    memory_id = %id,
                    error = %e,
                    "set_hall failed — not repaired"
                );
                errors.push(RowFailure {
                    id,
                    error: e.to_string(),
                });
            }
        }
    }

    // 4. Re-read the count from disk. This is the only number that proves the
    //    repair landed; `repaired` alone is what Spectral claimed, not what the
    //    database says.
    let after = {
        let conn = open_memory_db_read_only(brain_dir)?;
        count_misfiled(&conn)?
    };

    let report = HallBackfillReport {
        snapshots,
        before,
        after,
        attempted,
        repaired,
        missed,
        errors,
    };

    if report.is_clean() {
        tracing::info!(target: "permagent::hall_backfill", summary = %report.summary(), "Hall backfill complete");
    } else {
        tracing::warn!(target: "permagent::hall_backfill", summary = %report.summary(), "Hall backfill incomplete");
    }

    Ok(report)
}

/// [`run`] against the live brain and backup roots.
pub async fn run_on_default_paths(brain: &SafeBrain) -> Result<HallBackfillReport, String> {
    let brain_dir = crate::config::paths::Paths::brain_dir();
    let backup_root = crate::config::paths::Paths::data_dir().join("backups");
    run(brain, &brain_dir, &backup_root).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The count and the selection must read the same predicate. If someone
    /// edits one and not the other, the op reports a clean zero over rows it
    /// never looked at — the exact failure this constant exists to prevent.
    #[test]
    fn count_and_select_share_one_predicate() {
        let count_sql = format!("SELECT COUNT(*) FROM memories WHERE {MISFILED_APPROVAL_WHERE}");
        let select_sql = format!("SELECT id FROM memories WHERE {MISFILED_APPROVAL_WHERE}");

        let count_where = count_sql.split_once(" WHERE ").unwrap().1;
        let select_where = select_sql.split_once(" WHERE ").unwrap().1;
        assert_eq!(count_where, select_where);
    }

    /// The predicate must be anchored on both halves of the approval shape and
    /// on the wrong hall — matching `was asked:` alone would sweep prose that
    /// merely narrates having asked something.
    #[test]
    fn predicate_is_anchored_on_both_halves_and_the_wrong_hall() {
        assert!(MISFILED_APPROVAL_WHERE.contains("'%was asked:%'"));
        assert!(MISFILED_APPROVAL_WHERE.contains("'%answered:%'"));
        assert!(MISFILED_APPROVAL_WHERE.contains("hall = 'event'"));
    }

    /// The repair target is the shared constant, not a second copy of "fact".
    #[test]
    fn repair_target_is_the_shared_approval_hall() {
        assert_eq!(APPROVAL_HALL, "fact");
    }

    #[test]
    fn a_remainder_is_never_reported_as_clean() {
        let mut report = HallBackfillReport {
            snapshots: vec![],
            before: 104,
            after: 0,
            attempted: 104,
            repaired: 104,
            missed: vec![],
            errors: vec![],
        };
        assert!(report.is_clean());
        assert!(report.summary().contains("104"));

        // A leftover row is not clean, however many were repaired.
        report.after = 3;
        assert!(!report.is_clean());
        assert!(report.summary().contains("INCOMPLETE"));
        assert!(report.summary().contains("3 remaining"));

        // A miss is not clean either, even with the count at zero — the count
        // and the per-row outcome can disagree if something else wrote
        // concurrently.
        report.after = 0;
        report.missed = vec!["missing-id".into()];
        assert!(!report.is_clean());
    }
}
