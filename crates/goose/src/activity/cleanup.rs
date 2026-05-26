//! One-time Brain cleanup routines: noise pruning and cluster consolidation.
//!
//! Both functions operate directly on the Brain's SQLite database via rusqlite.
//! They are designed to run once at daemon startup (spawn_blocking) after the
//! Brain is mounted.

use anyhow::Result;
use tracing::{debug, info, warn};

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
    let conn = rusqlite::Connection::open(&db_path)?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;

    // First, count how many will be pruned (for logging)
    let noise_count: usize = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE \
         content LIKE '%about:blank%' \
         OR content LIKE '%doubleclick%' \
         OR content LIKE '%crwdcntrl%' \
         OR content LIKE '%recaptcha%' \
         OR content LIKE '%adtrafficquality%' \
         OR content LIKE '%ogs.google.com%' \
         OR content LIKE '%googleads%' \
         OR content LIKE 'Chat turn completed%' \
         OR (key LIKE 'activity:%' AND content LIKE 'Navigated to%' AND LENGTH(content) < 100)",
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

    // Delete annotations for noise memories first (belt-and-suspenders with CASCADE)
    let annotations_deleted: usize = conn.execute(
        "DELETE FROM memory_annotations WHERE memory_id IN ( \
         SELECT id FROM memories WHERE \
         content LIKE '%about:blank%' \
         OR content LIKE '%doubleclick%' \
         OR content LIKE '%crwdcntrl%' \
         OR content LIKE '%recaptcha%' \
         OR content LIKE '%adtrafficquality%' \
         OR content LIKE '%ogs.google.com%' \
         OR content LIKE '%googleads%' \
         OR content LIKE 'Chat turn completed%' \
         OR (key LIKE 'activity:%' AND content LIKE 'Navigated to%' AND LENGTH(content) < 100) \
         )",
        [],
    )?;

    // Delete the noise memories
    let deleted: usize = conn.execute(
        "DELETE FROM memories WHERE \
         content LIKE '%about:blank%' \
         OR content LIKE '%doubleclick%' \
         OR content LIKE '%crwdcntrl%' \
         OR content LIKE '%recaptcha%' \
         OR content LIKE '%adtrafficquality%' \
         OR content LIKE '%ogs.google.com%' \
         OR content LIKE '%googleads%' \
         OR content LIKE 'Chat turn completed%' \
         OR (key LIKE 'activity:%' AND content LIKE 'Navigated to%' AND LENGTH(content) < 100)",
        [],
    )?;

    info!(
        target: "permagent::cleanup",
        deleted = deleted,
        annotations_deleted = annotations_deleted,
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
pub fn consolidate_clusters() -> Result<usize> {
    let db_path = crate::config::paths::Paths::brain_dir().join("memory.db");
    let conn = rusqlite::Connection::open(&db_path)?;
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
    let brain = match crate::agents::platform_extensions::get_global_brain() {
        Some(b) => b,
        None => {
            warn!(target: "permagent::cleanup", "No global brain — skipping cluster consolidation");
            return Ok(0);
        }
    };

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
                ..Default::default()
            },
        );

        match result {
            Ok(_) => {
                // Delete annotations for the originals
                let _ = conn.execute(
                    "DELETE FROM memory_annotations WHERE memory_id IN ( \
                     SELECT id FROM memories \
                     WHERE key LIKE 'activity:%browser_navigated%' \
                       AND content LIKE '%' || ?1 || '%' \
                       AND key != ?2 \
                     )",
                    rusqlite::params![cluster.domain, summary_key],
                );

                // Delete the original cluster members (but not the new summary)
                let originals_deleted: usize = conn.execute(
                    "DELETE FROM memories \
                     WHERE key LIKE 'activity:%browser_navigated%' \
                       AND content LIKE '%' || ?1 || '%' \
                       AND key != ?2",
                    rusqlite::params![cluster.domain, summary_key],
                )?;

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

    let conn = rusqlite::Connection::open(&db_path)?;

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

// Note: prune_noise_memories and consolidate_clusters use Paths::brain_dir()
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
pub fn migrate_consolidated_into_to_spectral(
    brain: &spectral::Brain,
) -> Result<ConsolidateMigrationStats> {
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

    let conn = rusqlite::Connection::open(&db_path)?;
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
