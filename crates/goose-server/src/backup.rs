//! Automated backup snapshots for memory.db and permagent.db.
//!
//! Uses SQLite's online backup API via a dedicated read-only rusqlite
//! connection. Both databases run in WAL mode, and the backup API copies page
//! by page under a read transaction, so the snapshot is consistent and includes
//! rows still sitting in the WAL — without conflicting with live writers.
//!
//! Snapshots are NOT compacted. This used VACUUM INTO until 2026-08-14, which
//! also rebuilt every page: ~15% smaller files for 10x the time (12.5 s vs
//! 1.3 s on a 209 MB memory.db, measured). That mattered because the snapshot
//! runs on the startup path as a pre-migration safety net, and the cost was
//! pushing daemon startup past the desktop shell's health-wait budget. A safety
//! net is for fidelity, not for saving disk.
//!
//! Layout:
//!   ~/.permagent/backups/
//!     brain/    memory-20260609T080000Z-daily.db
//!     spectral/ permagent-20260609T080000Z-daily.db
//!
//! Rotation: 7 daily + 4 weekly per database. A weekly is the most recent
//! daily promoted once per ISO week. Prune oldest beyond limits after each
//! successful snapshot.

use chrono::{DateTime, Datelike, Utc};
use rusqlite::OpenFlags;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

// ── Snapshot tiers ──────────────────────────────────────────────────────────

/// Backup tier — daily snapshots, with the most recent daily per ISO week
/// promoted to weekly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Daily,
    Weekly,
}

/// A parsed snapshot filename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotEntry {
    pub filename: String,
    pub timestamp: DateTime<Utc>,
    pub tier: Tier,
    pub db_name: String,
}

/// Info returned by the list API.
#[derive(Debug, Clone, Serialize)]
pub struct SnapshotInfo {
    pub db: String,
    pub filename: String,
    pub timestamp: String,
    pub tier: Tier,
    pub size_bytes: u64,
    pub integrity_ok: bool,
}

// ── Constants ───────────────────────────────────────────────────────────────

const STALENESS_THRESHOLD: Duration = Duration::from_secs(20 * 3600); // 20 hours
const MAX_DAILY: usize = 7;
const MAX_WEEKLY: usize = 4;

// ── Public API ──────────────────────────────────────────────────────────────

/// Which database to back up.
#[derive(Debug, Clone, Copy)]
pub enum DbTarget {
    Brain,
    Spectral,
}

impl DbTarget {
    fn subdir(&self) -> &str {
        match self {
            DbTarget::Brain => "brain",
            DbTarget::Spectral => "spectral",
        }
    }

    fn prefix(&self) -> &str {
        match self {
            DbTarget::Brain => "memory",
            DbTarget::Spectral => "permagent",
        }
    }

    fn label(&self) -> &str {
        match self {
            DbTarget::Brain => "brain/memory.db",
            DbTarget::Spectral => "spectral/permagent.db",
        }
    }
}

/// Take a snapshot of the given database if the most recent one is stale
/// (older than 20 hours or absent). Returns `Ok(true)` if a snapshot was
/// taken, `Ok(false)` if skipped (fresh enough), `Err` on failure.
///
/// Failures are always non-fatal to the caller — this function logs errors
/// internally but the caller should catch and continue on Err.
pub fn snapshot_if_stale(
    source_db: &Path,
    backup_root: &Path,
    target: DbTarget,
) -> Result<bool, BackupError> {
    let dest_dir = backup_root.join(target.subdir());

    if !source_db.exists() {
        tracing::debug!(
            target: "permagentd::backup",
            db = target.label(),
            "Source DB does not exist — skipping backup"
        );
        return Ok(false);
    }

    let existing = list_snapshots_in_dir(&dest_dir, target.prefix());
    if !is_stale(&existing, Utc::now()) {
        tracing::info!(
            target: "permagentd::backup",
            db = target.label(),
            newest = existing.first().map(|e| e.timestamp.to_rfc3339()).unwrap_or_default(),
            "Recent backup exists — skipping (within 20h)"
        );
        return Ok(false);
    }

    take_snapshot(source_db, &dest_dir, target)?;
    Ok(true)
}

/// Take an unconditional snapshot (for the POST /api/backups/run endpoint).
pub fn force_snapshot(
    source_db: &Path,
    backup_root: &Path,
    target: DbTarget,
) -> Result<SnapshotInfo, BackupError> {
    if !source_db.exists() {
        return Err(BackupError::SourceMissing(source_db.to_path_buf()));
    }
    let dest_dir = backup_root.join(target.subdir());
    take_snapshot(source_db, &dest_dir, target)
}

/// List all snapshots for a given database target.
pub fn list_snapshot_info(backup_root: &Path, target: DbTarget) -> Vec<SnapshotInfo> {
    let dest_dir = backup_root.join(target.subdir());
    let entries = list_snapshots_in_dir(&dest_dir, target.prefix());
    let promoted = promote_and_prune(entries);

    promoted
        .into_iter()
        .map(|e| {
            let path = dest_dir.join(&e.filename);
            let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let integrity_ok = check_integrity(&path);
            SnapshotInfo {
                db: target.subdir().to_string(),
                filename: e.filename,
                timestamp: e.timestamp.to_rfc3339(),
                tier: e.tier,
                size_bytes,
                integrity_ok,
            }
        })
        .collect()
}

// ── Error type ──────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("source database missing: {0}")]
    SourceMissing(PathBuf),
    #[error("insufficient disk space: need {need_bytes} bytes, have {free_bytes}")]
    InsufficientSpace { need_bytes: u64, free_bytes: u64 },
    #[error("snapshot copy failed: {0}")]
    SnapshotFailed(String),
    #[error("integrity check failed on snapshot: {0}")]
    IntegrityFailed(PathBuf),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

// ── Internal helpers ────────────────────────────────────────────────────────

fn take_snapshot(
    source_db: &Path,
    dest_dir: &Path,
    target: DbTarget,
) -> Result<SnapshotInfo, BackupError> {
    std::fs::create_dir_all(dest_dir)?;

    // Check disk headroom: require 1.5x source size free on the destination volume.
    let source_size = std::fs::metadata(source_db)?.len();
    let need = (source_size as f64 * 1.5) as u64;
    let free = free_space_bytes(dest_dir)?;
    if free < need {
        tracing::warn!(
            target: "permagentd::backup",
            db = target.label(),
            need_bytes = need,
            free_bytes = free,
            "Insufficient disk space for backup — skipping"
        );
        return Err(BackupError::InsufficientSpace {
            need_bytes: need,
            free_bytes: free,
        });
    }

    let now = Utc::now();
    let ts = now.format("%Y%m%dT%H%M%SZ");
    let filename = format!("{}-{}-daily.db", target.prefix(), ts);
    let tmp_path = dest_dir.join(format!("{}.tmp", filename));
    let final_path = dest_dir.join(&filename);

    // SQLite's online backup API via a read-only connection. Safe under WAL.
    //
    // This was `VACUUM INTO` until 2026-08-14. Both produce a consistent
    // snapshot of a live database; the difference is that VACUUM also COMPACTS,
    // rebuilding every page. On a 209 MB brain/memory.db that cost 12.5 s of a
    // 15.4 s snapshot — and this runs on the STARTUP path, because the snapshot
    // is a pre-migration safety net and has to precede schema migration.
    //
    // Measured on the user's machine: process start to `listening on :3001` was
    // 21.4 s, against the desktop shell's 20 s health-wait budget. The app
    // reported "backend slow to start" once a day, on the one launch where the
    // 20-hour staleness check fired, for work that was proceeding correctly.
    //
    // Compaction is not what a safety net is for. It buys ~15% on disk (209 MB
    // -> 178 MB per snapshot) and costs an order of magnitude in the one place
    // where latency is visible to the user. Fidelity is unchanged: the backup
    // API copies page by page under a read transaction, and the integrity check
    // below still gates every snapshot.
    let t0 = std::time::Instant::now();

    let source_conn = rusqlite::Connection::open_with_flags(
        source_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| BackupError::SnapshotFailed(format!("open source: {e}")))?;

    {
        let mut dest_conn = rusqlite::Connection::open(&tmp_path)
            .map_err(|e| BackupError::SnapshotFailed(format!("open destination: {e}")))?;
        let backup = rusqlite::backup::Backup::new(&source_conn, &mut dest_conn)
            .map_err(|e| BackupError::SnapshotFailed(format!("start backup: {e}")))?;

        // `step(-1)` is SQLite's documented "copy all remaining pages" form.
        // Stepping in chunks exists to yield to concurrent writers mid-copy;
        // this call blocks startup, so finishing promptly matters more than
        // interleaving. The loop is belt-and-braces — one step should reach
        // Done — and it must not spin: Busy and Locked are returned as errors
        // rather than retried forever on the startup path.
        loop {
            match backup
                .step(-1)
                .map_err(|e| BackupError::SnapshotFailed(format!("{e}")))?
            {
                rusqlite::backup::StepResult::Done => break,
                rusqlite::backup::StepResult::More => continue,
                rusqlite::backup::StepResult::Busy => {
                    return Err(BackupError::SnapshotFailed(
                        "source database busy during snapshot".into(),
                    ))
                }
                rusqlite::backup::StepResult::Locked => {
                    return Err(BackupError::SnapshotFailed(
                        "source database locked during snapshot".into(),
                    ))
                }
                // StepResult is #[non_exhaustive]. An outcome this code does
                // not recognise must not be read as success — a backup that
                // silently isn't one is worse than a loud failure, and the
                // caller already treats Err as non-fatal.
                other => {
                    return Err(BackupError::SnapshotFailed(format!(
                        "unrecognised backup step result: {other:?}"
                    )))
                }
            }
        }
    }

    drop(source_conn);

    let copy_ms = t0.elapsed().as_millis();

    // Integrity check on the snapshot.
    if !check_integrity(&tmp_path) {
        let _ = std::fs::remove_file(&tmp_path);
        tracing::error!(
            target: "permagentd::backup",
            db = target.label(),
            path = %tmp_path.display(),
            "Snapshot failed integrity check — deleted"
        );
        return Err(BackupError::IntegrityFailed(tmp_path));
    }

    // Atomic rename.
    std::fs::rename(&tmp_path, &final_path)?;

    let size_bytes = std::fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);
    let total_ms = t0.elapsed().as_millis();

    tracing::info!(
        target: "permagentd::backup",
        db = target.label(),
        path = %final_path.display(),
        source_bytes = source_size,
        size_bytes,
        copy_ms,
        total_ms,
        "Backup snapshot created"
    );

    // Rotate: promote weeklies and prune old files.
    let entries = list_snapshots_in_dir(dest_dir, target.prefix());
    let keep = promote_and_prune(entries);
    prune_files(dest_dir, target.prefix(), &keep);

    Ok(SnapshotInfo {
        db: target.subdir().to_string(),
        filename,
        timestamp: now.to_rfc3339(),
        tier: Tier::Daily,
        size_bytes,
        integrity_ok: true,
    })
}

fn check_integrity(db_path: &Path) -> bool {
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return false;
    };
    let result: Result<String, _> = conn.query_row("PRAGMA integrity_check;", [], |row| row.get(0));
    matches!(result, Ok(ref s) if s == "ok")
}

/// Get free space on the volume containing `path` using statvfs.
/// On macOS, queries the actual data volume (not the sealed system volume).
fn free_space_bytes(path: &Path) -> Result<u64, std::io::Error> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        // f_bavail is u64 on Linux, u32 on macOS — cast required cross-platform
        #[allow(clippy::unnecessary_cast)]
        let free_bytes = stat.f_bavail as u64 * stat.f_frsize;
        Ok(free_bytes)
    }
}

// ── Filename parsing ────────────────────────────────────────────────────────

/// Parse "memory-20260609T080000Z-daily.db" → SnapshotEntry.
fn parse_snapshot_filename(filename: &str, expected_prefix: &str) -> Option<SnapshotEntry> {
    // Format: {prefix}-{YYYYMMDDTHHMMSSZ}-{tier}.db
    let stem = filename.strip_suffix(".db")?;
    let rest = stem.strip_prefix(expected_prefix)?.strip_prefix('-')?;

    let (ts_str, tier_str) = rest.rsplit_once('-')?;

    let tier = match tier_str {
        "daily" => Tier::Daily,
        "weekly" => Tier::Weekly,
        _ => return None,
    };

    let timestamp = chrono::NaiveDateTime::parse_from_str(ts_str, "%Y%m%dT%H%M%SZ")
        .ok()?
        .and_utc();

    Some(SnapshotEntry {
        filename: filename.to_string(),
        timestamp,
        tier,
        db_name: expected_prefix.to_string(),
    })
}

fn list_snapshots_in_dir(dir: &Path, prefix: &str) -> Vec<SnapshotEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut snapshots: Vec<SnapshotEntry> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            parse_snapshot_filename(&name, prefix)
        })
        .collect();

    // Newest first.
    snapshots.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    snapshots
}

// ── Staleness check ─────────────────────────────────────────────────────────

fn is_stale(snapshots: &[SnapshotEntry], now: DateTime<Utc>) -> bool {
    match snapshots.first() {
        None => true,
        Some(newest) => {
            let age = now.signed_duration_since(newest.timestamp);
            age.to_std().unwrap_or(Duration::ZERO) > STALENESS_THRESHOLD
        }
    }
}

// ── Rotation policy ─────────────────────────────────────────────────────────

/// Pure function: given a list of snapshot entries (newest-first), promote
/// the most recent daily per ISO week to weekly and prune beyond limits.
/// Returns the entries to keep.
pub fn promote_and_prune(mut entries: Vec<SnapshotEntry>) -> Vec<SnapshotEntry> {
    // Sort newest first.
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    // Reset all tiers to daily for re-evaluation.
    for e in &mut entries {
        e.tier = Tier::Daily;
    }

    // Promote: the newest daily in each ISO week becomes weekly.
    let mut seen_weeks: std::collections::HashSet<(i32, u32)> = std::collections::HashSet::new();
    for e in &mut entries {
        let iso = e.timestamp.iso_week();
        let key = (iso.year(), iso.week());
        if seen_weeks.insert(key) {
            e.tier = Tier::Weekly;
        }
    }

    // Partition.
    let (weeklies, dailies): (Vec<_>, Vec<_>) =
        entries.into_iter().partition(|e| e.tier == Tier::Weekly);

    // Keep newest N of each.
    let mut keep: Vec<SnapshotEntry> = Vec::new();
    keep.extend(dailies.into_iter().take(MAX_DAILY));
    keep.extend(weeklies.into_iter().take(MAX_WEEKLY));

    keep.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    keep
}

/// Delete snapshot files not in the keep list.
fn prune_files(dir: &Path, prefix: &str, keep: &[SnapshotEntry]) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let keep_names: std::collections::HashSet<&str> =
        keep.iter().map(|e| e.filename.as_str()).collect();

    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().to_string();
        if parse_snapshot_filename(&name, prefix).is_some() && !keep_names.contains(name.as_str()) {
            if let Err(e) = std::fs::remove_file(entry.path()) {
                tracing::warn!(
                    target: "permagentd::backup",
                    file = %name,
                    error = %e,
                    "Failed to prune old snapshot"
                );
            } else {
                tracing::info!(
                    target: "permagentd::backup",
                    file = %name,
                    "Pruned old snapshot"
                );
            }
        }
    }
}

// ── Background scheduler ────────────────────────────────────────────────────

/// Background loop: ticks once per hour, runs daily backups if stale.
/// Same pattern as the librarian scheduler loop.
pub async fn backup_scheduler_loop() {
    let mut interval = tokio::time::interval(Duration::from_secs(3600));
    loop {
        interval.tick().await;

        let base = permagent::config::paths::Paths::data_dir();
        let backup_root = base.join("backups");

        for (source, target) in [
            (
                permagent::config::paths::Paths::brain_dir().join("memory.db"),
                DbTarget::Brain,
            ),
            (
                permagent::config::paths::Paths::spectral_db(),
                DbTarget::Spectral,
            ),
        ] {
            match snapshot_if_stale(&source, &backup_root, target) {
                Ok(true) => tracing::info!(
                    target: "permagentd::backup",
                    db = target.label(),
                    "Scheduled backup completed"
                ),
                Ok(false) => {}
                Err(e) => tracing::error!(
                    target: "permagentd::backup",
                    db = target.label(),
                    error = %e,
                    "Scheduled backup failed"
                ),
            }
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(name: &str, ts: &str, tier: Tier) -> SnapshotEntry {
        SnapshotEntry {
            filename: name.to_string(),
            timestamp: chrono::NaiveDateTime::parse_from_str(ts, "%Y%m%dT%H%M%SZ")
                .unwrap()
                .and_utc(),
            tier,
            db_name: "memory".to_string(),
        }
    }

    #[test]
    fn test_parse_snapshot_filename_daily() {
        let e = parse_snapshot_filename("memory-20260609T080000Z-daily.db", "memory").unwrap();
        assert_eq!(e.tier, Tier::Daily);
        assert_eq!(e.db_name, "memory");
        assert_eq!(e.timestamp.year(), 2026);
    }

    #[test]
    fn test_parse_snapshot_filename_weekly() {
        let e =
            parse_snapshot_filename("permagent-20260601T120000Z-weekly.db", "permagent").unwrap();
        assert_eq!(e.tier, Tier::Weekly);
        assert_eq!(e.db_name, "permagent");
    }

    #[test]
    fn test_parse_snapshot_filename_rejects_malformed() {
        assert!(parse_snapshot_filename("memory.db", "memory").is_none());
        assert!(parse_snapshot_filename("memory-baddate-daily.db", "memory").is_none());
        assert!(parse_snapshot_filename("other-20260609T080000Z-daily.db", "memory").is_none());
        assert!(parse_snapshot_filename("memory-20260609T080000Z-hourly.db", "memory").is_none());
    }

    #[test]
    fn test_staleness_empty_is_stale() {
        assert!(is_stale(&[], Utc::now()));
    }

    #[test]
    fn test_staleness_recent_not_stale() {
        let now = Utc::now();
        let recent = SnapshotEntry {
            filename: "memory-test-daily.db".to_string(),
            timestamp: now - chrono::Duration::hours(10),
            tier: Tier::Daily,
            db_name: "memory".to_string(),
        };
        assert!(!is_stale(&[recent], now));
    }

    #[test]
    fn test_staleness_old_is_stale() {
        let now = Utc::now();
        let old = SnapshotEntry {
            filename: "memory-test-daily.db".to_string(),
            timestamp: now - chrono::Duration::hours(21),
            tier: Tier::Daily,
            db_name: "memory".to_string(),
        };
        assert!(is_stale(&[old], now));
    }

    #[test]
    fn test_rotation_promotes_weekly() {
        // 10 dailies across 2 ISO weeks → should keep 7 daily + 2 weekly
        let entries: Vec<SnapshotEntry> = (0..10)
            .map(|i| {
                let day = 1 + i; // June 1..10, 2026
                let ts = format!("202606{:02}T080000Z", day);
                // June 1 = Mon W23, June 8 = Mon W24
                make_entry(
                    &format!("memory-202606{:02}T080000Z-daily.db", day),
                    &ts,
                    Tier::Daily,
                )
            })
            .collect();

        let kept = promote_and_prune(entries);
        let daily_count = kept.iter().filter(|e| e.tier == Tier::Daily).count();
        let weekly_count = kept.iter().filter(|e| e.tier == Tier::Weekly).count();

        // At most 7 daily + 4 weekly
        assert!(daily_count <= MAX_DAILY);
        assert!(weekly_count <= MAX_WEEKLY);
        // At least 1 weekly (multiple ISO weeks present)
        assert!(weekly_count >= 1);
    }

    #[test]
    fn test_rotation_empty_dir() {
        let kept = promote_and_prune(vec![]);
        assert!(kept.is_empty());
    }

    #[test]
    fn test_rotation_single_entry() {
        let entry = make_entry(
            "memory-20260609T080000Z-daily.db",
            "20260609T080000Z",
            Tier::Daily,
        );
        let kept = promote_and_prune(vec![entry]);
        assert_eq!(kept.len(), 1);
        // Single entry gets promoted to weekly (first in its ISO week)
        assert_eq!(kept[0].tier, Tier::Weekly);
    }

    #[test]
    fn test_rotation_prunes_excess_dailies() {
        // 15 dailies in the same ISO week → 1 promoted to weekly, 7 daily kept
        let entries: Vec<SnapshotEntry> = (0..15)
            .map(|i| {
                let hour = i;
                let ts_str = format!("20260609T{:02}0000Z", hour);
                let fname = format!("memory-20260609T{:02}0000Z-daily.db", hour);
                make_entry(&fname, &ts_str, Tier::Daily)
            })
            .collect();

        let kept = promote_and_prune(entries);
        // All same ISO week → 1 weekly + 7 daily = 8 max, but the weekly
        // replaces one of the dailies, so total = 1 weekly + 7 daily = 8
        assert!(kept.len() <= MAX_DAILY + MAX_WEEKLY);
        let weekly_count = kept.iter().filter(|e| e.tier == Tier::Weekly).count();
        assert_eq!(weekly_count, 1);
    }

    #[test]
    fn test_disk_headroom_calculation() {
        // This just tests the math, not actual statvfs
        let source_size: u64 = 100_000_000; // 100 MB
        let need = (source_size as f64 * 1.5) as u64;
        assert_eq!(need, 150_000_000);
    }

    // ── Integration tests (tempdir-based) ───────────────────────────────────

    #[test]
    fn test_snapshot_creates_valid_db() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("test.db");
        let backup_root = tmp.path().join("backups");

        // Create a source DB with some data.
        {
            let conn = rusqlite::Connection::open(&source).unwrap();
            conn.execute_batch(
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT);
                 INSERT INTO items VALUES (1, 'alpha');
                 INSERT INTO items VALUES (2, 'beta');",
            )
            .unwrap();
        }

        let info = force_snapshot(&source, &backup_root, DbTarget::Brain).unwrap();
        assert!(info.integrity_ok);
        assert!(info.size_bytes > 0);
        assert_eq!(info.db, "brain");

        // Verify the snapshot has the same data.
        let snap_path = backup_root.join("brain").join(&info.filename);
        let conn = rusqlite::Connection::open(&snap_path).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM items", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    /// The snapshot must be faithful while the source is OPEN and in WAL mode —
    /// which is how brain/memory.db always looks in production. `VACUUM INTO`
    /// gave this for free; the online backup API has to be shown to give it
    /// too, including rows sitting in the WAL rather than the main file.
    #[test]
    fn a_snapshot_of_a_live_wal_database_captures_uncheckpointed_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("live.db");
        let backup_root = tmp.path().join("backups");

        // Held open across the snapshot, exactly like the running daemon.
        let live = rusqlite::Connection::open(&source).unwrap();
        live.pragma_update(None, "journal_mode", "WAL").unwrap();
        live.execute_batch(
            "CREATE TABLE memories (id INTEGER PRIMARY KEY, body TEXT);
             INSERT INTO memories VALUES (1, 'checkpointed');",
        )
        .unwrap();
        live.pragma_update(None, "wal_checkpoint", "FULL").unwrap();
        // Written AFTER the checkpoint, so it lives in the WAL, not the main
        // database file. A naive file copy would lose exactly this row.
        live.execute("INSERT INTO memories VALUES (2, ?1)", ["in-wal"])
            .unwrap();

        let info = force_snapshot(&source, &backup_root, DbTarget::Brain).unwrap();
        assert!(info.integrity_ok, "snapshot failed its integrity check");

        let snap =
            rusqlite::Connection::open(backup_root.join("brain").join(&info.filename)).unwrap();
        let bodies: Vec<String> = snap
            .prepare("SELECT body FROM memories ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            bodies,
            vec!["checkpointed".to_string(), "in-wal".to_string()],
            "the snapshot dropped a row that was still in the WAL"
        );
    }

    #[test]
    fn test_snapshot_if_stale_skips_recent() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("test.db");
        let backup_root = tmp.path().join("backups");

        // Create source.
        {
            let conn = rusqlite::Connection::open(&source).unwrap();
            conn.execute_batch("CREATE TABLE t (id INTEGER);").unwrap();
        }

        // First snapshot should succeed.
        assert!(snapshot_if_stale(&source, &backup_root, DbTarget::Brain).unwrap());
        // Second should skip (within 20h).
        assert!(!snapshot_if_stale(&source, &backup_root, DbTarget::Brain).unwrap());
    }

    #[test]
    fn test_snapshot_source_missing_returns_ok_false() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("nonexistent.db");
        let backup_root = tmp.path().join("backups");

        let result = snapshot_if_stale(&missing, &backup_root, DbTarget::Brain).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_corrupt_snapshot_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let backup_dir = tmp.path().join("backups").join("brain");
        std::fs::create_dir_all(&backup_dir).unwrap();

        // Write a corrupt "snapshot" directly.
        let corrupt_path = backup_dir.join("memory-20260609T080000Z-daily.db");
        std::fs::write(&corrupt_path, b"this is not a sqlite database").unwrap();

        assert!(!check_integrity(&corrupt_path));
    }

    #[test]
    fn test_no_tmp_litter_on_integrity_failure() {
        // We can't easily simulate VACUUM INTO producing a corrupt file,
        // but we verify that the tmp cleanup path works by checking that
        // no .tmp files remain after a normal snapshot.
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("test.db");
        let backup_root = tmp.path().join("backups");

        {
            let conn = rusqlite::Connection::open(&source).unwrap();
            conn.execute_batch("CREATE TABLE t (id INTEGER);").unwrap();
        }

        force_snapshot(&source, &backup_root, DbTarget::Brain).unwrap();

        // No .tmp files should remain.
        let brain_dir = backup_root.join("brain");
        let tmp_files: Vec<_> = std::fs::read_dir(&brain_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(
            tmp_files.is_empty(),
            "Found leftover .tmp files: {:?}",
            tmp_files
        );
    }

    #[test]
    fn test_list_snapshots_returns_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("test.db");
        let backup_root = tmp.path().join("backups");

        {
            let conn = rusqlite::Connection::open(&source).unwrap();
            conn.execute_batch("CREATE TABLE t (id INTEGER);").unwrap();
        }

        // Create two snapshots (we need to manipulate time, so just create files directly).
        let brain_dir = backup_root.join("brain");
        std::fs::create_dir_all(&brain_dir).unwrap();

        // Copy source as two differently-named snapshots.
        std::fs::copy(&source, brain_dir.join("memory-20260608T080000Z-daily.db")).unwrap();
        std::fs::copy(&source, brain_dir.join("memory-20260609T080000Z-daily.db")).unwrap();

        let infos = list_snapshot_info(&backup_root, DbTarget::Brain);
        assert_eq!(infos.len(), 2);
        // Newest first.
        assert!(infos[0].timestamp > infos[1].timestamp);
    }
}
