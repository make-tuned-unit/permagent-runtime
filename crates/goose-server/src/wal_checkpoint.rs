//! Periodic WAL checkpoint (durability F4).
//!
//! Both SQLite databases — the Brain's `memory.db` and Spectral's `permagent.db`
//! — run in WAL mode. SQLite's passive autocheckpoint (default 1000 pages) only
//! fires opportunistically and, crucially, a long-lived / leaked **read snapshot
//! pins the WAL** so it can never be truncated. Over weeks on a chronically
//! near-full disk that lets the WAL grow unbounded and can fill the volume in
//! days (live evidence: Brain WAL already 30 MB ≈ 7× the default threshold).
//!
//! This timer runs `PRAGMA wal_checkpoint(TRUNCATE)` against both DBs on a fixed
//! cadence to bound the WAL. TRUNCATE briefly takes the checkpoint lock; if a
//! reader is pinning the WAL it returns `busy=1` (or `SQLITE_BUSY` after the
//! busy_timeout) — we log that and retry next tick. It is never fatal: a failed
//! checkpoint just leaves the WAL as-is, exactly as today.
//!
//! Observability: every tick logs the outcome for both DBs under the
//! `durability` target — a silent checkpoint is impossible by design.

use std::time::Duration;

use sqlx::{Pool, Sqlite};

/// Default cadence between checkpoint sweeps. Overridable via
/// `PERMAGENT_WAL_CHECKPOINT_SECS` for ops tuning; explicit config over a hidden
/// constant.
const DEFAULT_INTERVAL_SECS: u64 = 900; // 15 minutes

/// How long a checkpoint connection waits for the write lock before giving up.
const BUSY_TIMEOUT_MS: u64 = 30_000;

fn interval_secs() -> u64 {
    std::env::var("PERMAGENT_WAL_CHECKPOINT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_INTERVAL_SECS)
}

/// Run the WAL-checkpoint timer forever. Spawned once at daemon startup with a
/// clone of the Spectral pool; resolves the Brain `memory.db` path itself.
pub async fn wal_checkpoint_loop(spectral_pool: Pool<Sqlite>) {
    let secs = interval_secs();
    let mut ticker = tokio::time::interval(Duration::from_secs(secs));
    // interval() fires immediately at t=0; skip that tick so we don't checkpoint
    // in the middle of startup migrations.
    ticker.tick().await;
    tracing::info!(
        target: "durability",
        interval_secs = secs,
        "WAL checkpoint timer started (Brain memory.db + Spectral permagent.db)"
    );
    loop {
        ticker.tick().await;
        checkpoint_brain().await;
        checkpoint_spectral(&spectral_pool).await;
    }
}

/// Checkpoint the Brain `memory.db` on a blocking thread (rusqlite is sync).
async fn checkpoint_brain() {
    let result = tokio::task::spawn_blocking(checkpoint_brain_blocking).await;
    match result {
        Ok(Ok(Some((busy, log, ckpt)))) => tracing::info!(
            target: "durability",
            db = "memory.db",
            busy,
            wal_frames = log,
            checkpointed = ckpt,
            "WAL checkpoint(TRUNCATE) ok"
        ),
        Ok(Ok(None)) => {
            tracing::debug!(target: "durability", db = "memory.db", "no memory.db present; skipping checkpoint")
        }
        Ok(Err(e)) => tracing::warn!(
            target: "durability",
            db = "memory.db",
            "WAL checkpoint failed (a reader may be pinning the WAL); will retry next tick: {e}"
        ),
        Err(e) => tracing::warn!(
            target: "durability",
            db = "memory.db",
            "WAL checkpoint task panicked: {e}"
        ),
    }
}

/// Returns `Ok(None)` if the DB file is absent (running without long-term
/// memory), else `Ok(Some((busy, wal_frames, checkpointed_frames)))`.
fn checkpoint_brain_blocking() -> anyhow::Result<Option<(i64, i64, i64)>> {
    let db_path = permagent::config::paths::Paths::brain_dir().join("memory.db");
    if !db_path.exists() {
        return Ok(None);
    }
    let conn = rusqlite::Connection::open(&db_path)?;
    conn.busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))?;
    // wal_checkpoint(TRUNCATE) yields one row: (busy, log_frames, checkpointed).
    // busy=1 means it couldn't fully checkpoint (a reader held the WAL).
    let row = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    Ok(Some(row))
}

/// Checkpoint the Spectral `permagent.db` directly on its sqlx pool.
async fn checkpoint_spectral(pool: &Pool<Sqlite>) {
    match sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(pool)
        .await
    {
        Ok(_) => tracing::info!(
            target: "durability",
            db = "permagent.db",
            "WAL checkpoint(TRUNCATE) ok"
        ),
        Err(e) => tracing::warn!(
            target: "durability",
            db = "permagent.db",
            "WAL checkpoint failed (a reader may be pinning the WAL); will retry next tick: {e}"
        ),
    }
}
