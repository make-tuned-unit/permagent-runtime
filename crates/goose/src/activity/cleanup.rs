//! One-time Brain cleanup routines: noise pruning and cluster consolidation.
//!
//! Both functions operate directly on the Brain's SQLite database via rusqlite.
//! They are designed to run once at daemon startup (spawn_blocking) after the
//! Brain is mounted.

use anyhow::Result;
use tracing::{debug, info, warn};

/// Episode for every consolidated browser-navigation summary (R45). Stable
/// forever, because these summaries are keyed upserts refreshed on each
/// startup sweep — see the write site for why a per-run id would be wrong.
const BROWSER_ROLLUP_EPISODE_ID: &str = "activity:rollup:browser_navigated";

/// How long a cleanup connection will wait to acquire SQLite's write lock
/// before giving up with SQLITE_BUSY.
const CLEANUP_BUSY_TIMEOUT_MS: u64 = 30_000;

/// Open a connection to the Brain's `memory.db` configured for write-lock
/// contention at daemon startup.
///
/// The DB is already in WAL mode (Spectral's `SqliteStore` sets it), but WAL
/// still serializes writers: a second connection issuing `DELETE`/`UPDATE`
/// while the Brain's own connection holds the write lock (auto-migration,
/// health-check recall, annotation backfill, all racing at startup) gets an
/// immediate `SQLITE_BUSY` and the cleanup silently fails (#122).
///
/// Setting `busy_timeout` makes SQLite retry-with-backoff internally for up to
/// [`CLEANUP_BUSY_TIMEOUT_MS`], so the cleanup waits out the startup write
/// window instead of bailing on first contention. This is the minimal robust
/// fix: Spectral exposes no shared-connection accessor, and there is no
/// init-complete signal to await.
fn open_cleanup_conn(db_path: &std::path::Path) -> Result<rusqlite::Connection> {
    let conn = rusqlite::Connection::open(db_path)?;
    conn.busy_timeout(std::time::Duration::from_millis(CLEANUP_BUSY_TIMEOUT_MS))?;
    Ok(conn)
}

// SQL WHERE clause matching noise memories (used by prune + FK child cleanup).
const NOISE_FILTER: &str = "\
    content LIKE '%about:blank%' \
    OR content LIKE '%doubleclick%' \
    OR content LIKE '%crwdcntrl%' \
    OR content LIKE '%recaptcha%' \
    OR content LIKE '%adtrafficquality%' \
    OR content LIKE '%ogs.google.com%' \
    OR content LIKE '%googleads%' \
    OR content LIKE 'Chat turn completed%' \
    OR (key LIKE 'activity:%' AND content LIKE 'Navigated to%' AND LENGTH(content) < 100)";

/// Delete pure-noise memories from the Brain corpus.
///
/// Targets:
/// - about:blank navigations
/// - ad/tracking URLs (doubleclick, crwdcntrl, etc.)
/// - chat_turn_completed token-count entries
/// - very short browser_navigated entries (< 100 chars)
///
/// Returns the number of memories deleted.
pub fn prune_noise_memories() -> Result<usize> {
    let db_path = crate::config::paths::Paths::brain_dir().join("memory.db");
    let mut conn = open_cleanup_conn(&db_path)?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;

    // First, count how many will be pruned (for logging)
    let noise_count: usize = conn.query_row(
        &format!("SELECT COUNT(*) FROM memories WHERE {f}", f = NOISE_FILTER),
        [],
        |r| r.get(0),
    )?;

    if noise_count == 0 {
        info!(target: "permagent::cleanup", "No noise memories to prune");
        return Ok(0);
    }

    info!(
        target: "permagent::cleanup",
        count = noise_count,
        "Pruning noise memories"
    );

    let mut deleted = 0;
    let mut fingerprints_deleted = 0;
    let mut spectrograms_deleted = 0;
    let mut annotations_deleted = 0;
    loop {
        // Keep write transactions bounded while selecting each batch under the
        // same lock that protects its child + parent deletes.
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let ids = {
            let mut stmt = tx.prepare(&format!(
                "SELECT id FROM memories WHERE {f} LIMIT 100",
                f = NOISE_FILTER
            ))?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if ids.is_empty() {
            tx.rollback()?;
            break;
        }
        for id in &ids {
            let counts = delete_memory_with_children(&tx, id)?;
            fingerprints_deleted += counts.fingerprints;
            spectrograms_deleted += counts.spectrograms;
            annotations_deleted += counts.annotations;
            deleted += counts.memories;
        }
        tx.commit()?;
    }

    info!(
        target: "permagent::cleanup",
        deleted,
        fingerprints_deleted,
        spectrograms_deleted,
        annotations_deleted,
        "Noise prune complete"
    );

    Ok(deleted)
}

/// Find clusters of redundant browser_navigated memories and consolidate them.
///
/// A cluster is defined as 3+ memories with the same domain in browser_navigated
/// activity events. For each cluster, one summary memory is created and the
/// originals are deleted.
///
/// Returns the number of clusters consolidated.
pub fn consolidate_clusters_blocking() -> Result<usize> {
    let db_path = crate::config::paths::Paths::brain_dir().join("memory.db");
    let mut conn = open_cleanup_conn(&db_path)?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;

    // Find domain clusters from browser_navigated activity events.
    // Content format: "Navigated to <title> (<url>) in tab <id>."
    // We extract the domain from the URL portion between '(' and ')'.
    // GROUP BY domain HAVING COUNT > 2 to find clusters.
    let mut stmt = conn.prepare(
        "SELECT \
           SUBSTR( \
             content, \
             INSTR(content, '://') + 3, \
             INSTR(SUBSTR(content, INSTR(content, '://') + 3), '/') - 1 \
           ) AS domain, \
           COUNT(*) AS cnt, \
           MIN(created_at) AS first_seen, \
           MAX(created_at) AS last_seen \
         FROM memories \
         WHERE key LIKE 'activity:%browser_navigated%' \
           AND content LIKE 'Navigated to%' \
           AND content LIKE '%://%' \
         GROUP BY domain \
         HAVING cnt > 2 \
         ORDER BY cnt DESC",
    )?;

    struct Cluster {
        domain: String,
        count: usize,
        first_seen: String,
        last_seen: String,
    }

    let clusters: Vec<Cluster> = stmt
        .query_map([], |row| {
            Ok(Cluster {
                domain: row.get(0)?,
                count: row.get(1)?,
                first_seen: row.get(2)?,
                last_seen: row.get(3)?,
            })
        })?
        .filter_map(|r| r.ok())
        .filter(|c| !c.domain.is_empty())
        .collect();

    drop(stmt);

    if clusters.is_empty() {
        info!(target: "permagent::cleanup", "No browser navigation clusters to consolidate");
        return Ok(0);
    }

    info!(
        target: "permagent::cleanup",
        clusters = clusters.len(),
        "Consolidating browser navigation clusters"
    );

    // We need the Brain to create consolidated memories via remember_with.
    // Called from spawn_blocking context (state.rs startup cleanup).
    let safe_brain = match crate::agents::platform_extensions::get_global_brain() {
        Some(b) => b,
        None => {
            warn!(target: "permagent::cleanup", "No global brain — skipping cluster consolidation");
            return Ok(0);
        }
    };
    let brain = safe_brain.raw_blocking_handle();

    let mut consolidated = 0;

    for cluster in &clusters {
        let summary_content = format!(
            "{} — visited {} times between {} and {}. Regular usage pattern.",
            cluster.domain, cluster.count, cluster.first_seen, cluster.last_seen
        );

        let summary_key = format!("activity:consolidated:browser_navigated:{}", cluster.domain);

        // Create the consolidated memory
        let result = brain.remember_with(
            &summary_key,
            &summary_content,
            spectral::RememberOpts {
                source: Some("permagent.cleanup".to_string()),
                visibility: spectral::Visibility::Private,
                compaction_tier: Some(spectral::ingest::CompactionTier::Raw),
                // The browser-navigation rollup is one episode (R45): every
                // domain summary this sweep writes belongs to it. Deliberately
                // a constant and not a per-sweep id — these summaries are
                // keyed upserts (`activity:consolidated:browser_navigated:*`),
                // so a fresh id each startup would keep migrating the SAME
                // memory into a new episode, which is worse than no episode.
                episode_id: Some(BROWSER_ROLLUP_EPISODE_ID.to_string()),
                ..Default::default()
            },
        );

        match result {
            Ok(_) => {
                let tx =
                    conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let ids = {
                    let mut stmt = tx.prepare(
                        "SELECT id FROM memories \
                         WHERE key LIKE 'activity:%browser_navigated%' \
                           AND content LIKE '%' || ?1 || '%' \
                           AND key != ?2",
                    )?;
                    let rows = stmt
                        .query_map(rusqlite::params![cluster.domain, summary_key], |row| {
                            row.get::<_, String>(0)
                        })?;
                    rows.collect::<rusqlite::Result<Vec<_>>>()?
                };
                let mut originals_deleted = 0;
                for id in &ids {
                    originals_deleted += delete_memory_with_children(&tx, id)?.memories;
                }
                tx.commit()?;

                debug!(
                    target: "permagent::cleanup",
                    domain = %cluster.domain,
                    originals_deleted = originals_deleted,
                    "Consolidated cluster"
                );
                consolidated += 1;
            }
            Err(e) => {
                warn!(
                    target: "permagent::cleanup",
                    domain = %cluster.domain,
                    error = %e,
                    "Failed to create consolidated memory — skipping cluster"
                );
            }
        }
    }

    info!(
        target: "permagent::cleanup",
        consolidated = consolidated,
        "Cluster consolidation complete"
    );

    Ok(consolidated)
}

#[derive(Default)]
struct DeleteCounts {
    fingerprints: usize,
    spectrograms: usize,
    annotations: usize,
    memories: usize,
}

/// Delete one memory and all of its explicit FK children as an atomic unit.
///
/// The caller owns the transaction so candidate selection can share the same
/// write lock. Any child or parent failure aborts the caller before commit.
fn delete_memory_with_children(
    tx: &rusqlite::Transaction<'_>,
    memory_id: &str,
) -> rusqlite::Result<DeleteCounts> {
    let fingerprints = tx.execute(
        "DELETE FROM constellation_fingerprints \
         WHERE anchor_memory_id = ?1 OR target_memory_id = ?1",
        [memory_id],
    )?;
    let spectrograms = tx.execute(
        "DELETE FROM memory_spectrogram WHERE memory_id = ?1",
        [memory_id],
    )?;
    let annotations = tx.execute(
        "DELETE FROM memory_annotations WHERE memory_id = ?1",
        [memory_id],
    )?;
    let memories = tx.execute("DELETE FROM memories WHERE id = ?1", [memory_id])?;
    Ok(DeleteCounts {
        fingerprints,
        spectrograms,
        annotations,
        memories,
    })
}

// ── Part A: one-shot FK orphan cleanup ───────────────────────────

const MIGRATION_FK_ORPHAN_CLEANUP: &str = "fk_orphan_cleanup_v1";

/// One-shot cleanup of orphaned FK children in the Brain DB.
///
/// Deletes `constellation_fingerprints` and `memory_spectrogram` rows whose
/// parent memory no longer exists. These orphans accumulated while Spectral's
/// FK constraints were defined but never enforced (`PRAGMA foreign_keys` was
/// off). Now that cleanup.rs enforces FKs, these stale rows block legitimate
/// `DELETE FROM memories` operations.
///
/// Backs up memory.db before any data change. Gated by `_pm_migrations_applied`.
pub fn cleanup_orphaned_fk_children() -> Result<FkOrphanCleanupStats> {
    let db_path = crate::config::paths::Paths::brain_dir().join("memory.db");
    if !db_path.exists() {
        debug!(target: "permagent::cleanup", "FK orphan cleanup: no memory.db, skipping");
        return Ok(FkOrphanCleanupStats::default());
    }

    let conn = open_cleanup_conn(&db_path)?;
    ensure_migrations_table(&conn)?;

    if is_migration_applied(&conn, MIGRATION_FK_ORPHAN_CLEANUP)? {
        debug!(target: "permagent::cleanup", "FK orphan cleanup: already applied, skipping");
        return Ok(FkOrphanCleanupStats::default());
    }

    // Back up memory.db before any destructive operation.
    let backup_name = format!(
        "memory.db.pre-fk-cleanup-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
    );
    let backup_path = db_path.with_file_name(&backup_name);
    std::fs::copy(&db_path, &backup_path)?;

    // Verify the backup exists and is non-zero.
    let backup_meta = std::fs::metadata(&backup_path)?;
    anyhow::ensure!(
        backup_meta.len() > 0,
        "FK orphan cleanup: backup file is empty — aborting"
    );
    info!(
        target: "permagent::cleanup",
        backup = %backup_path.display(),
        size_bytes = backup_meta.len(),
        "Brain DB backed up before FK orphan cleanup"
    );

    // Count orphans before cleanup (for logging).
    let fp_anchor_orphans: usize = conn.query_row(
        "SELECT COUNT(*) FROM constellation_fingerprints \
         WHERE anchor_memory_id NOT IN (SELECT id FROM memories)",
        [],
        |r| r.get(0),
    )?;
    let fp_target_orphans: usize = conn.query_row(
        "SELECT COUNT(*) FROM constellation_fingerprints \
         WHERE target_memory_id NOT IN (SELECT id FROM memories)",
        [],
        |r| r.get(0),
    )?;
    let spec_orphans: usize = conn.query_row(
        "SELECT COUNT(*) FROM memory_spectrogram \
         WHERE memory_id NOT IN (SELECT id FROM memories)",
        [],
        |r| r.get(0),
    )?;

    if fp_anchor_orphans == 0 && fp_target_orphans == 0 && spec_orphans == 0 {
        info!(target: "permagent::cleanup", "FK orphan cleanup: no orphans found");
        mark_migration_applied(&conn, MIGRATION_FK_ORPHAN_CLEANUP)?;
        return Ok(FkOrphanCleanupStats::default());
    }

    info!(
        target: "permagent::cleanup",
        fp_anchor_orphans,
        fp_target_orphans,
        spec_orphans,
        "Cleaning up orphaned FK children"
    );

    // Delete orphaned constellation_fingerprints (anchor or target missing).
    let fp_deleted_anchor: usize = conn.execute(
        "DELETE FROM constellation_fingerprints \
         WHERE anchor_memory_id NOT IN (SELECT id FROM memories)",
        [],
    )?;
    let fp_deleted_target: usize = conn.execute(
        "DELETE FROM constellation_fingerprints \
         WHERE target_memory_id NOT IN (SELECT id FROM memories)",
        [],
    )?;

    // Delete orphaned memory_spectrogram rows.
    let spec_deleted: usize = conn.execute(
        "DELETE FROM memory_spectrogram \
         WHERE memory_id NOT IN (SELECT id FROM memories)",
        [],
    )?;

    mark_migration_applied(&conn, MIGRATION_FK_ORPHAN_CLEANUP)?;

    let stats = FkOrphanCleanupStats {
        fingerprints_deleted: fp_deleted_anchor + fp_deleted_target,
        spectrograms_deleted: spec_deleted,
        backup_path: backup_path.to_string_lossy().into_owned(),
    };

    info!(
        target: "permagent::cleanup",
        fingerprints_deleted = stats.fingerprints_deleted,
        spectrograms_deleted = stats.spectrograms_deleted,
        "FK orphan cleanup complete"
    );

    Ok(stats)
}

#[derive(Debug, Default)]
pub struct FkOrphanCleanupStats {
    pub fingerprints_deleted: usize,
    pub spectrograms_deleted: usize,
    pub backup_path: String,
}

const MIGRATION_DOMAIN_CLUSTER_CLEANUP: &str = "domain_cluster_cleanup_v1";

/// One-shot cleanup for buggy domain cluster consolidation.
///
/// A substring offset bug in `find_domain_clusters` caused all https URLs to be
/// grouped under "tps:" and all http URLs under "ttp:" instead of actual domains.
/// This un-consolidates the victims and deletes the catchall cluster memories.
///
/// Idempotent: tracked via `_pm_migrations_applied` table in the brain DB.
pub fn cleanup_buggy_domain_clusters() -> Result<(usize, usize)> {
    let db_path = crate::config::paths::Paths::brain_dir().join("memory.db");
    if !db_path.exists() {
        debug!(target: "permagent::cleanup", "domain-cluster cleanup: no memory.db, skipping");
        return Ok((0, 0));
    }

    let conn = open_cleanup_conn(&db_path)?;

    ensure_migrations_table(&conn)?;

    if is_migration_applied(&conn, MIGRATION_DOMAIN_CLUSTER_CLEANUP)? {
        debug!(target: "permagent::cleanup", "domain-cluster cleanup: already applied, skipping");
        return Ok((0, 0));
    }

    let (un_consolidated, deleted) = run_domain_cluster_cleanup_sql(&conn)?;
    mark_migration_applied(&conn, MIGRATION_DOMAIN_CLUSTER_CLEANUP)?;

    info!(
        target: "permagent::cleanup",
        un_consolidated,
        deleted,
        "domain-cluster cleanup: un-consolidated {} memories, deleted {} catchall clusters",
        un_consolidated,
        deleted
    );

    Ok((un_consolidated, deleted))
}

/// Run the buggy domain cluster cleanup SQL on an arbitrary connection.
/// Exposed for testing — the production entrypoint is `cleanup_buggy_domain_clusters`.
pub fn run_domain_cluster_cleanup_sql(conn: &rusqlite::Connection) -> Result<(usize, usize)> {
    let un_consolidated: usize = conn.execute(
        "UPDATE memories SET _pm_consolidated_into = NULL \
         WHERE _pm_consolidated_into IN ( \
           SELECT id FROM memories WHERE key LIKE 'consolidated:browser:tps:%' \
              OR key LIKE 'consolidated:browser:ttp:%' \
         )",
        [],
    )?;

    let deleted: usize = conn.execute(
        "DELETE FROM memories \
         WHERE key LIKE 'consolidated:browser:tps:%' \
            OR key LIKE 'consolidated:browser:ttp:%'",
        [],
    )?;

    Ok((un_consolidated, deleted))
}

// ── Permagent-side migration tracking ────────────────────────────
// Lightweight table in the Brain DB for one-shot Permagent migrations.
// Prefixed `_pm_` to signal it's Permagent-owned, not Spectral-owned.

fn ensure_migrations_table(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _pm_migrations_applied ( \
           name TEXT PRIMARY KEY, \
           applied_at TEXT NOT NULL DEFAULT (datetime('now')) \
         );",
    )?;
    Ok(())
}

fn is_migration_applied(conn: &rusqlite::Connection, name: &str) -> Result<bool> {
    let count: usize = conn.query_row(
        "SELECT COUNT(*) FROM _pm_migrations_applied WHERE name = ?1",
        [name],
        |r| r.get(0),
    )?;
    Ok(count > 0)
}

fn mark_migration_applied(conn: &rusqlite::Connection, name: &str) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO _pm_migrations_applied (name) VALUES (?1)",
        [name],
    )?;
    Ok(())
}

/// Check and mark a migration in the `_pm_migrations_applied` table.
/// Exposed for testing.
pub fn ensure_and_check_migration(conn: &rusqlite::Connection, name: &str) -> Result<bool> {
    ensure_migrations_table(conn)?;
    is_migration_applied(conn, name)
}

/// Mark a migration as applied. Exposed for testing.
pub fn mark_migration(conn: &rusqlite::Connection, name: &str) -> Result<()> {
    ensure_migrations_table(conn)?;
    mark_migration_applied(conn, name)
}

// Note: prune_noise_memories and consolidate_clusters_blocking use Paths::brain_dir()
// which can't be overridden in unit tests. The SQL correctness is validated
// by the daemon at startup. The ingest filter tests in ingestion.rs cover
// the filtering logic independently.

// ── Consolidation migration ─────────────────────────────────────

const MIGRATION_CONSOLIDATE_INTO: &str = "consolidate_into_migration_v1";

/// Stats returned by the consolidation migration.
#[derive(Debug)]
pub struct ConsolidateMigrationStats {
    pub rows_migrated: usize,
    pub distinct_targets: usize,
    pub orphans_skipped: usize,
    pub column_dropped: bool,
}

/// Migrate `_pm_consolidated_into` column data to Spectral's `consolidation_edges` table.
///
/// One-shot, idempotent (gated by `_pm_migrations_applied`).
/// Direct SQL INSERT (Option B) — no consolidate_into() calls, no score merging.
///
/// Steps:
/// 1. Check if already applied
/// 2. Resolve id→key via JOIN, INSERT OR IGNORE into consolidation_edges
/// 3. Cross-check: row count + API verification via list_consolidated()
/// 4. DROP COLUMN _pm_consolidated_into (gated on cross-check)
/// 5. Mark migration applied
///
/// Migrate _pm_consolidated_into column to Spectral's consolidation_edges.
/// Caller MUST be inside spawn_blocking — uses raw_blocking_handle() internally.
pub fn migrate_consolidated_into_to_spectral_blocking(
    brain: &crate::brain_handle::SafeBrain,
) -> Result<ConsolidateMigrationStats> {
    let brain = brain.raw_blocking_handle();
    let db_path = crate::config::paths::Paths::brain_dir().join("memory.db");
    if !db_path.exists() {
        debug!(target: "permagent::cleanup", "consolidate_into migration: no memory.db, skipping");
        return Ok(ConsolidateMigrationStats {
            rows_migrated: 0,
            distinct_targets: 0,
            orphans_skipped: 0,
            column_dropped: false,
        });
    }

    let conn = open_cleanup_conn(&db_path)?;
    ensure_migrations_table(&conn)?;

    if is_migration_applied(&conn, MIGRATION_CONSOLIDATE_INTO)? {
        debug!(target: "permagent::cleanup", "consolidate_into migration: already applied, skipping");
        return Ok(ConsolidateMigrationStats {
            rows_migrated: 0,
            distinct_targets: 0,
            orphans_skipped: 0,
            column_dropped: false,
        });
    }

    // Check if _pm_consolidated_into column exists
    let has_column: bool = conn
        .prepare("SELECT _pm_consolidated_into FROM memories LIMIT 0")
        .is_ok();

    if !has_column {
        // Column already gone (maybe a partial previous run) — just mark and return
        info!(target: "permagent::cleanup", "consolidate_into migration: column already absent, marking applied");
        mark_migration_applied(&conn, MIGRATION_CONSOLIDATE_INTO)?;
        return Ok(ConsolidateMigrationStats {
            rows_migrated: 0,
            distinct_targets: 0,
            orphans_skipped: 0,
            column_dropped: false,
        });
    }

    // Step 3a: Read mapping with id→key JOIN
    let mut stmt = conn.prepare(
        "SELECT src.key AS source_key, tgt.key AS target_key \
         FROM memories src \
         JOIN memories tgt ON tgt.id = src._pm_consolidated_into \
         WHERE src._pm_consolidated_into IS NOT NULL",
    )?;
    let mappings: Vec<(String, String)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);

    // Step 3b: Count orphans (dangling references)
    let orphans: usize = conn.query_row(
        "SELECT COUNT(*) FROM memories \
         WHERE _pm_consolidated_into IS NOT NULL \
           AND _pm_consolidated_into NOT IN (SELECT id FROM memories)",
        [],
        |r| r.get(0),
    )?;
    if orphans > 0 {
        warn!(
            target: "permagent::cleanup",
            orphans,
            "consolidate_into migration: {} orphan references skipped (target id not found)",
            orphans
        );
    }

    // Distinct targets
    let distinct_targets = {
        let mut targets = std::collections::HashSet::new();
        for (_, t) in &mappings {
            targets.insert(t.clone());
        }
        targets.len()
    };

    // Step 3c: INSERT OR IGNORE into consolidation_edges (single transaction)
    conn.execute_batch("BEGIN TRANSACTION;")?;

    // Ensure consolidation_edges table exists (Spectral's init_schema creates it,
    // but be defensive for the migration path)
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS consolidation_edges ( \
             source_key TEXT PRIMARY KEY, \
             target_key TEXT NOT NULL, \
             consolidated_at TEXT NOT NULL DEFAULT (datetime('now')), \
             UNIQUE(source_key, target_key) \
         ); \
         CREATE INDEX IF NOT EXISTS idx_consolidation_target \
             ON consolidation_edges(target_key);",
    )?;

    for (source_key, target_key) in &mappings {
        conn.execute(
            "INSERT OR IGNORE INTO consolidation_edges (source_key, target_key) VALUES (?1, ?2)",
            rusqlite::params![source_key, target_key],
        )?;
    }

    conn.execute_batch("COMMIT;")?;

    // Step 3d: Cross-check via direct SQL
    let edge_count: usize =
        conn.query_row("SELECT COUNT(*) FROM consolidation_edges", [], |r| r.get(0))?;

    // We expect edge_count >= mappings.len() (could be > if some edges were already
    // present from a partial previous run or from Spectral's own consolidation calls).
    // The critical check: every mapping we inserted should be present.
    let missing: usize = conn.query_row(
        "SELECT COUNT(*) FROM memories src \
         JOIN memories tgt ON tgt.id = src._pm_consolidated_into \
         WHERE src._pm_consolidated_into IS NOT NULL \
           AND NOT EXISTS ( \
               SELECT 1 FROM consolidation_edges ce \
               WHERE ce.source_key = src.key AND ce.target_key = tgt.key \
           )",
        [],
        |r| r.get(0),
    )?;

    if missing > 0 {
        warn!(
            target: "permagent::cleanup",
            missing,
            edge_count,
            expected = mappings.len(),
            "consolidate_into migration: cross-check FAILED — {} mappings missing from consolidation_edges. \
             Aborting column drop. Column left in place for triage.",
            missing
        );
        return Ok(ConsolidateMigrationStats {
            rows_migrated: mappings.len(),
            distinct_targets,
            orphans_skipped: orphans,
            column_dropped: false,
        });
    }

    // Cross-check via Spectral API: list_consolidated(None) should see the edges
    let api_edges = brain.list_consolidated(None);
    match api_edges {
        Ok(edges) => {
            if edges.len() < mappings.len() {
                warn!(
                    target: "permagent::cleanup",
                    api_count = edges.len(),
                    sql_count = mappings.len(),
                    "consolidate_into migration: API cross-check mismatch — \
                     list_consolidated() returned fewer edges than expected. \
                     Aborting column drop.",
                );
                return Ok(ConsolidateMigrationStats {
                    rows_migrated: mappings.len(),
                    distinct_targets,
                    orphans_skipped: orphans,
                    column_dropped: false,
                });
            }
            info!(
                target: "permagent::cleanup",
                api_count = edges.len(),
                "consolidate_into migration: API cross-check passed"
            );
        }
        Err(e) => {
            warn!(
                target: "permagent::cleanup",
                error = %e,
                "consolidate_into migration: API cross-check failed — \
                 list_consolidated() returned error. Aborting column drop.",
            );
            return Ok(ConsolidateMigrationStats {
                rows_migrated: mappings.len(),
                distinct_targets,
                orphans_skipped: orphans,
                column_dropped: false,
            });
        }
    }

    // Step 3e: Drop the column
    let column_dropped = match conn
        .execute_batch("ALTER TABLE memories DROP COLUMN _pm_consolidated_into;")
    {
        Ok(()) => {
            info!(target: "permagent::cleanup", "consolidate_into migration: _pm_consolidated_into column dropped");
            true
        }
        Err(e) => {
            warn!(
                target: "permagent::cleanup",
                error = %e,
                "consolidate_into migration: DROP COLUMN failed (vestigial column remains)"
            );
            false
        }
    };

    mark_migration_applied(&conn, MIGRATION_CONSOLIDATE_INTO)?;

    info!(
        target: "permagent::cleanup",
        rows_migrated = mappings.len(),
        distinct_targets,
        orphans_skipped = orphans,
        column_dropped,
        "consolidate_into migration complete"
    );

    Ok(ConsolidateMigrationStats {
        rows_migrated: mappings.len(),
        distinct_targets,
        orphans_skipped: orphans,
        column_dropped,
    })
}

/// Run the consolidation migration on an arbitrary connection (for testing).
/// Does not use Paths::brain_dir() — operates on the provided connection.
pub fn run_consolidate_into_migration_sql(
    conn: &rusqlite::Connection,
) -> Result<ConsolidateMigrationStats> {
    ensure_migrations_table(conn)?;

    if is_migration_applied(conn, MIGRATION_CONSOLIDATE_INTO)? {
        return Ok(ConsolidateMigrationStats {
            rows_migrated: 0,
            distinct_targets: 0,
            orphans_skipped: 0,
            column_dropped: false,
        });
    }

    // Check if column exists
    let has_column: bool = conn
        .prepare("SELECT _pm_consolidated_into FROM memories LIMIT 0")
        .is_ok();

    if !has_column {
        mark_migration_applied(conn, MIGRATION_CONSOLIDATE_INTO)?;
        return Ok(ConsolidateMigrationStats {
            rows_migrated: 0,
            distinct_targets: 0,
            orphans_skipped: 0,
            column_dropped: false,
        });
    }

    // Read mappings with id→key JOIN
    let mut stmt = conn.prepare(
        "SELECT src.key AS source_key, tgt.key AS target_key \
         FROM memories src \
         JOIN memories tgt ON tgt.id = src._pm_consolidated_into \
         WHERE src._pm_consolidated_into IS NOT NULL",
    )?;
    let mappings: Vec<(String, String)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);

    let orphans: usize = conn.query_row(
        "SELECT COUNT(*) FROM memories \
         WHERE _pm_consolidated_into IS NOT NULL \
           AND _pm_consolidated_into NOT IN (SELECT id FROM memories)",
        [],
        |r| r.get(0),
    )?;

    let distinct_targets = {
        let mut targets = std::collections::HashSet::new();
        for (_, t) in &mappings {
            targets.insert(t.clone());
        }
        targets.len()
    };

    // Ensure consolidation_edges table
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS consolidation_edges ( \
             source_key TEXT PRIMARY KEY, \
             target_key TEXT NOT NULL, \
             consolidated_at TEXT NOT NULL DEFAULT (datetime('now')), \
             UNIQUE(source_key, target_key) \
         ); \
         CREATE INDEX IF NOT EXISTS idx_consolidation_target \
             ON consolidation_edges(target_key);",
    )?;

    for (source_key, target_key) in &mappings {
        conn.execute(
            "INSERT OR IGNORE INTO consolidation_edges (source_key, target_key) VALUES (?1, ?2)",
            rusqlite::params![source_key, target_key],
        )?;
    }

    // Cross-check
    let missing: usize = conn.query_row(
        "SELECT COUNT(*) FROM memories src \
         JOIN memories tgt ON tgt.id = src._pm_consolidated_into \
         WHERE src._pm_consolidated_into IS NOT NULL \
           AND NOT EXISTS ( \
               SELECT 1 FROM consolidation_edges ce \
               WHERE ce.source_key = src.key AND ce.target_key = tgt.key \
           )",
        [],
        |r| r.get(0),
    )?;

    if missing > 0 {
        return Ok(ConsolidateMigrationStats {
            rows_migrated: mappings.len(),
            distinct_targets,
            orphans_skipped: orphans,
            column_dropped: false,
        });
    }

    // Drop column
    let column_dropped = conn
        .execute_batch("ALTER TABLE memories DROP COLUMN _pm_consolidated_into;")
        .is_ok();

    mark_migration_applied(conn, MIGRATION_CONSOLIDATE_INTO)?;

    Ok(ConsolidateMigrationStats {
        rows_migrated: mappings.len(),
        distinct_targets,
        orphans_skipped: orphans,
        column_dropped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cleanup_test_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE memories (id TEXT PRIMARY KEY);
             CREATE TABLE constellation_fingerprints (
                 id TEXT PRIMARY KEY,
                 anchor_memory_id TEXT NOT NULL,
                 target_memory_id TEXT NOT NULL
             );
             CREATE TABLE memory_spectrogram (
                 memory_id TEXT PRIMARY KEY
             );
             CREATE TABLE memory_annotations (
                 memory_id TEXT PRIMARY KEY
             );
             INSERT INTO memories (id) VALUES ('m1');
             INSERT INTO constellation_fingerprints
                 (id, anchor_memory_id, target_memory_id)
                 VALUES ('fp1', 'm1', 'm1');
             INSERT INTO memory_spectrogram (memory_id) VALUES ('m1');
             INSERT INTO memory_annotations (memory_id) VALUES ('m1');",
        )
        .unwrap();
        conn
    }

    fn cleanup_row_counts(conn: &rusqlite::Connection) -> (usize, usize, usize, usize) {
        let count = |table: &str| {
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
        };
        (
            count("memories"),
            count("constellation_fingerprints"),
            count("memory_spectrogram"),
            count("memory_annotations"),
        )
    }

    #[test]
    fn parent_delete_failure_rolls_back_child_deletes() {
        let mut conn = cleanup_test_conn();
        conn.execute_batch(
            "CREATE TRIGGER reject_memory_delete
             BEFORE DELETE ON memories
             BEGIN
                 SELECT RAISE(ABORT, 'injected parent delete failure');
             END;",
        )
        .unwrap();

        let result = {
            let tx = conn.transaction().unwrap();
            delete_memory_with_children(&tx, "m1")
        };

        assert!(result.is_err(), "injected parent failure must propagate");
        assert_eq!(
            cleanup_row_counts(&conn),
            (1, 1, 1, 1),
            "the memory and every child must survive the rolled-back unit"
        );
    }

    #[test]
    fn normal_memory_cleanup_deletes_parent_and_children() {
        let mut conn = cleanup_test_conn();
        let tx = conn.transaction().unwrap();
        let counts = delete_memory_with_children(&tx, "m1").unwrap();
        tx.commit().unwrap();

        assert_eq!(counts.memories, 1);
        assert_eq!(
            cleanup_row_counts(&conn),
            (0, 0, 0, 0),
            "the committed unit deletes the memory and every child"
        );
    }

    /// #122 regression: a cleanup connection must wait out the write lock held
    /// by another connection (the Brain at startup) instead of failing
    /// instantly with SQLITE_BUSY. `open_cleanup_conn` sets `busy_timeout`,
    /// so a write that begins while a competing write lock is held briefly
    /// must still succeed once that lock releases.
    #[test]
    fn busy_timeout_lets_writer_wait_out_lock() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("memory.db");

        // Initialize the DB in WAL mode (mirrors Spectral's store) and a table.
        let setup = rusqlite::Connection::open(&db_path).unwrap();
        setup
            .execute_batch("PRAGMA journal_mode = WAL; CREATE TABLE t (id INTEGER);")
            .unwrap();
        drop(setup);

        // Connection A holds an EXCLUSIVE write lock for a short window.
        let holder = rusqlite::Connection::open(&db_path).unwrap();
        holder.execute_batch("BEGIN IMMEDIATE;").unwrap();

        // Release the lock after ~300ms from a background thread.
        let handle = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            holder.execute_batch("COMMIT;").unwrap();
        });

        // Connection B (via the cleanup opener) must wait, not fail instantly.
        let conn = open_cleanup_conn(&db_path).unwrap();
        let start = std::time::Instant::now();
        let res = conn.execute("INSERT INTO t (id) VALUES (1)", []);
        let waited = start.elapsed();

        handle.join().unwrap();

        assert!(
            res.is_ok(),
            "write should succeed after the competing lock releases, got: {res:?}"
        );
        // It must have actually waited for the lock (proves busy_timeout engaged),
        // and far less than the full 30s timeout (proves it didn't just spin).
        assert!(
            waited >= std::time::Duration::from_millis(150),
            "expected the writer to block on the lock, waited only {waited:?}"
        );
    }

    /// #230 root cause: a bare `DELETE FROM memories` on a connection with
    /// `PRAGMA foreign_keys = ON` fails with "FOREIGN KEY constraint failed"
    /// when a child row references the victim through a **legacy NO ACTION** FK
    /// (RESTRICT semantics). This is exactly the consolidation-time abort the
    /// issue reported.
    #[test]
    fn no_action_fk_makes_delete_fail_under_enforcement() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn.execute_batch(
            "CREATE TABLE memories (id TEXT PRIMARY KEY, content TEXT);
             CREATE TABLE constellation_fingerprints (
                 id TEXT PRIMARY KEY,
                 anchor_memory_id TEXT NOT NULL,
                 FOREIGN KEY (anchor_memory_id) REFERENCES memories(id)
             );
             INSERT INTO memories (id, content) VALUES ('m1', 'x');
             INSERT INTO constellation_fingerprints (id, anchor_memory_id) VALUES ('fp1', 'm1');",
        )
        .unwrap();

        let res = conn.execute("DELETE FROM memories WHERE id = 'm1'", []);
        assert!(
            res.is_err(),
            "NO ACTION FK + enforcement must reject the delete (reproduces #230)"
        );
    }

    /// #230 fix (via the pinned Spectral CASCADE schema / migration): the same
    /// delete succeeds and cascades to the child when the FK is
    /// `ON DELETE CASCADE`. No explicit child cleanup, no FK violation.
    #[test]
    fn cascade_fk_makes_delete_succeed_and_clean_children() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn.execute_batch(
            "CREATE TABLE memories (id TEXT PRIMARY KEY, content TEXT);
             CREATE TABLE constellation_fingerprints (
                 id TEXT PRIMARY KEY,
                 anchor_memory_id TEXT NOT NULL,
                 FOREIGN KEY (anchor_memory_id) REFERENCES memories(id) ON DELETE CASCADE
             );
             INSERT INTO memories (id, content) VALUES ('m1', 'x');
             INSERT INTO constellation_fingerprints (id, anchor_memory_id) VALUES ('fp1', 'm1');",
        )
        .unwrap();

        conn.execute("DELETE FROM memories WHERE id = 'm1'", [])
            .expect("CASCADE FK allows the delete (fixes #230)");

        let children: usize = conn
            .query_row("SELECT COUNT(*) FROM constellation_fingerprints", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(children, 0, "child row cascaded — no orphan, no FK failure");
    }

    /// Control: a plain connection with no busy_timeout fails immediately under
    /// the same contention — demonstrating the bug `open_cleanup_conn` fixes.
    #[test]
    fn plain_conn_fails_fast_under_contention() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("memory.db");

        let setup = rusqlite::Connection::open(&db_path).unwrap();
        setup
            .execute_batch("PRAGMA journal_mode = WAL; CREATE TABLE t (id INTEGER);")
            .unwrap();
        drop(setup);

        let holder = rusqlite::Connection::open(&db_path).unwrap();
        holder.execute_batch("BEGIN IMMEDIATE;").unwrap();

        // No busy_timeout: write while the lock is held must error immediately.
        let plain = rusqlite::Connection::open(&db_path).unwrap();
        let res = plain.execute("INSERT INTO t (id) VALUES (1)", []);
        assert!(
            res.is_err(),
            "expected SQLITE_BUSY without busy_timeout, but write succeeded"
        );

        holder.execute_batch("COMMIT;").unwrap();
    }
}
