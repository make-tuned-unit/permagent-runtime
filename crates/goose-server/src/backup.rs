//! Automated backup snapshots for memory.db and permagent.db.
//!
//! Two write modes, chosen by the caller's latency budget (see [`SnapshotMode`]):
//! the startup pre-migration snapshot uses SQLite's online backup API (fast,
//! uncompacted); the hourly background scheduler uses VACUUM INTO (compacted,
//! ~10x slower, invisible where it runs). Both produce a consistent copy of a
//! live WAL database including rows not yet checkpointed.
//!
//! Layout:
//!   ~/.permagent/backups/
//!     brain/    memory-20260609T080000Z-daily.db
//!     spectral/ permagent-20260609T080000Z-daily.db
//!
//! Retention is a ladder of age bands, not a count — one snapshot per band, the
//! newest that is at least that old. See [`RETENTION_BANDS`] for why span beats
//! count, and why keeping exactly one snapshot is the worst option available.

use chrono::{DateTime, Utc};
use rusqlite::OpenFlags;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

// ── Snapshot tiers ──────────────────────────────────────────────────────────

/// Which retention band a snapshot occupies. Descriptive, not a property of the
/// file: tier is recomputed from age on every prune, and every snapshot is
/// written with the same filename shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Daily,
    Weekly,
    Monthly,
}

impl Tier {
    /// Map a [`RETENTION_BANDS`] index to its label. Out-of-range indices
    /// clamp to Monthly rather than panicking — adding a fourth band should
    /// widen the ladder, never crash the prune that keeps disk in check.
    fn from_band(index: usize) -> Self {
        match index {
            0 => Tier::Daily,
            1 => Tier::Weekly,
            _ => Tier::Monthly,
        }
    }
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

/// Retention ladder: minimum age of the snapshot kept in each band, newest
/// first. One snapshot survives per band — the newest one at least that old.
///
/// # Why bands instead of "keep the last N"
///
/// A backup is only useful if it predates the damage, so what matters is the
/// SPAN of history covered, not the count of files. Different failures surface
/// on very different timescales:
///
///   * a bad schema migration — seconds
///   * SQLite or disk corruption — hours to days
///   * bad data (a wrong consolidation, garbage memories) — days to weeks
///
/// The previous policy kept 7 dailies + 4 weeklies: eleven files, ~2 GB for
/// brain/memory.db, most of them clustered in the last week. The opposite
/// instinct — keep exactly one — is worse than it looks: with a single slot,
/// tonight's backup faithfully overwrites the last good copy with the corrupted
/// state, so the system dutifully propagates the damage. One snapshot only
/// protects against failures you notice within a day.
///
/// Three bands cover the three timescales in three files (~534 MB compacted,
/// less than the 685 MB the eleven-file policy was already using here).
///
/// The deliberate gap: between "newest" and "7 days" there is no intermediate
/// copy, so damage noticed on day 3 rewinds to day 7 rather than day 2. That is
/// the price of the size target. If finer recent granularity is wanted later,
/// add a band — the ladder is data, and `select_retained` needs no change.
const RETENTION_BANDS: [chrono::TimeDelta; 3] = [
    chrono::TimeDelta::zero(),
    chrono::TimeDelta::days(7),
    chrono::TimeDelta::days(30),
];

// ── Public API ──────────────────────────────────────────────────────────────

/// How to write the snapshot.
///
/// Both modes produce a consistent copy of a live WAL database. The difference
/// is compaction, and therefore an order of magnitude in time — measured on a
/// 209 MB brain/memory.db: 1.3 s uncompacted, 12.5 s compacted (178 MB).
///
/// The split exists because the two callers have opposite constraints. The
/// startup snapshot is a pre-migration safety net on the path between launching
/// the app and serving requests, where 12.5 s pushed daemon startup past the
/// desktop shell's health-wait budget and produced a daily "backend slow to
/// start". The scheduled snapshot runs hourly in the background, where the same
/// 12.5 s is invisible and the 15% saved is worth having.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotMode {
    /// SQLite online backup API. Fast, uncompacted. For latency-sensitive paths.
    Fast,
    /// VACUUM INTO. Compacts, and costs roughly 10x. For background work.
    Compacted,
}

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
    mode: SnapshotMode,
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

    take_snapshot(source_db, &dest_dir, target, mode)?;
    Ok(true)
}

/// Take an unconditional snapshot (for the POST /api/backups/run endpoint).
pub fn force_snapshot(
    source_db: &Path,
    backup_root: &Path,
    target: DbTarget,
    mode: SnapshotMode,
) -> Result<SnapshotInfo, BackupError> {
    if !source_db.exists() {
        return Err(BackupError::SourceMissing(source_db.to_path_buf()));
    }
    let dest_dir = backup_root.join(target.subdir());
    take_snapshot(source_db, &dest_dir, target, mode)
}

/// List all snapshots for a given database target.
pub fn list_snapshot_info(backup_root: &Path, target: DbTarget) -> Vec<SnapshotInfo> {
    let dest_dir = backup_root.join(target.subdir());
    let entries = list_snapshots_in_dir(&dest_dir, target.prefix());
    let promoted = select_retained(entries, Utc::now());

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
    mode: SnapshotMode,
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

    if mode == SnapshotMode::Compacted {
        // VACUUM INTO: consistent AND compacted, at roughly 10x the cost. Only
        // reached from the background scheduler, where the wait is invisible.
        source_conn
            .execute_batch(&format!(
                "VACUUM INTO '{}';",
                tmp_path.to_string_lossy().replace('\'', "''")
            ))
            .map_err(|e| BackupError::SnapshotFailed(format!("{e}")))?;
    } else {
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
    let keep = select_retained(entries, now);
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

/// Pure function: choose which snapshots to retain, one per [`RETENTION_BANDS`]
/// entry — the newest snapshot at least that old.
///
/// `now` is a parameter rather than `Utc::now()` so retention is testable at
/// specific ages instead of only at whatever time the suite happens to run.
///
/// A band with no qualifying snapshot simply contributes nothing; a young
/// backup directory keeps everything it has. Bands never delete a snapshot they
/// could not replace.
pub fn select_retained(mut entries: Vec<SnapshotEntry>, now: DateTime<Utc>) -> Vec<SnapshotEntry> {
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    let mut keep: Vec<SnapshotEntry> = Vec::new();
    for (band_index, min_age) in RETENTION_BANDS.iter().enumerate() {
        // Newest entry old enough for this band, that a nearer band has not
        // already claimed. Without the `already claimed` check a sparse history
        // would fill every band with the same file and silently retain one
        // snapshot while reporting three.
        let candidate = entries
            .iter()
            .find(|e| {
                now.signed_duration_since(e.timestamp) >= *min_age
                    && !keep.iter().any(|k| k.filename == e.filename)
            })
            .cloned();

        if let Some(mut entry) = candidate {
            entry.tier = Tier::from_band(band_index);
            keep.push(entry);
        }
    }

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
            // Compacted here on purpose: this loop runs hourly in the
            // background, where the ~10x cost is invisible and the ~15% saved
            // is worth having. The startup path uses Fast for the opposite
            // reason.
            match snapshot_if_stale(&source, &backup_root, target, SnapshotMode::Compacted) {
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

    // ── Retention ladder ────────────────────────────────────────────────
    //
    // These replaced the "7 daily + 4 weekly" rotation tests on 2026-08-14.
    // The policy is now age bands (see RETENTION_BANDS), so the old assertions
    // (count caps, ISO-week promotion) no longer describe anything real.

    /// Build an entry `days_ago` before `now`, timestamp and filename agreeing.
    fn aged(now: DateTime<Utc>, days_ago: i64) -> SnapshotEntry {
        let ts = now - chrono::Duration::days(days_ago);
        SnapshotEntry {
            filename: format!("memory-{}-daily.db", ts.format("%Y%m%dT%H%M%SZ")),
            timestamp: ts,
            tier: Tier::Daily,
            db_name: "memory".to_string(),
        }
    }

    /// The point of the ladder: a long dense history collapses to one snapshot
    /// per band, and the survivors SPAN the history rather than clustering at
    /// the recent end. Under the old policy these 40 days kept eleven files,
    /// nine of them from the last two weeks.
    #[test]
    fn a_dense_history_collapses_to_one_snapshot_per_band() {
        let now = Utc::now();
        let entries: Vec<SnapshotEntry> = (0..40).map(|d| aged(now, d)).collect();

        let kept = select_retained(entries, now);

        assert_eq!(kept.len(), 3, "one per band: {kept:?}");
        let ages: Vec<i64> = kept
            .iter()
            .map(|e| now.signed_duration_since(e.timestamp).num_days())
            .collect();
        assert_eq!(ages, vec![0, 7, 30], "bands should land on their minimums");
    }

    /// Bands must never delete something they cannot replace. A directory with
    /// only recent snapshots keeps its newest rather than dropping to nothing
    /// because the 7- and 30-day bands are unsatisfiable.
    #[test]
    fn a_young_history_keeps_what_it_has() {
        let now = Utc::now();
        let kept = select_retained(vec![aged(now, 0), aged(now, 1)], now);

        assert_eq!(kept.len(), 1, "only the newest band is satisfiable");
        assert_eq!(now.signed_duration_since(kept[0].timestamp).num_days(), 0);
    }

    /// One snapshot must not be counted three times. Without the
    /// already-claimed check every band resolves to the same file and the
    /// caller is told three are retained while one exists — a retention policy
    /// that reports coverage it does not have.
    #[test]
    fn one_old_snapshot_fills_exactly_one_band() {
        let now = Utc::now();
        let kept = select_retained(vec![aged(now, 45)], now);

        assert_eq!(
            kept.len(),
            1,
            "one file cannot satisfy three bands: {kept:?}"
        );
    }

    /// A snapshot older than every band still counts as the oldest band, so
    /// history beyond 30 days is not silently discarded.
    #[test]
    fn snapshots_older_than_the_last_band_are_still_retained() {
        let now = Utc::now();
        let kept = select_retained(vec![aged(now, 0), aged(now, 9), aged(now, 400)], now);

        let ages: Vec<i64> = kept
            .iter()
            .map(|e| now.signed_duration_since(e.timestamp).num_days())
            .collect();
        assert_eq!(ages, vec![0, 9, 400]);
    }

    #[test]
    fn an_empty_directory_retains_nothing() {
        assert!(select_retained(vec![], Utc::now()).is_empty());
    }

    /// Tier is a label derived from the band, not a property of the file — so
    /// the same snapshot set must report Daily/Weekly/Monthly in age order.
    #[test]
    fn tiers_are_labelled_by_band_not_by_filename() {
        let now = Utc::now();
        let kept = select_retained(vec![aged(now, 0), aged(now, 8), aged(now, 31)], now);

        let tiers: Vec<Tier> = kept.iter().map(|e| e.tier).collect();
        assert_eq!(tiers, vec![Tier::Daily, Tier::Weekly, Tier::Monthly]);
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
