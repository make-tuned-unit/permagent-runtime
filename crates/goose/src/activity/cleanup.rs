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

        let summary_key = format!(
            "activity:consolidated:browser_navigated:{}",
            cluster.domain
        );

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

/// One-shot cleanup for buggy domain cluster consolidation.
///
/// A substring offset bug in `find_domain_clusters` caused all https URLs to be
/// grouped under "tps:" and all http URLs under "ttp:" instead of actual domains.
/// This un-consolidates the victims and deletes the catchall cluster memories.
///
/// Idempotent: uses a marker file at `~/.permagent/brain/.domain-cluster-cleanup-applied`.
pub fn cleanup_buggy_domain_clusters() -> Result<(usize, usize)> {
    let brain_dir = crate::config::paths::Paths::brain_dir();
    let marker = brain_dir.join(".domain-cluster-cleanup-applied");

    if marker.exists() {
        debug!(target: "permagent::cleanup", "domain-cluster cleanup: already applied, skipping");
        return Ok((0, 0));
    }

    let db_path = brain_dir.join("memory.db");
    if !db_path.exists() {
        debug!(target: "permagent::cleanup", "domain-cluster cleanup: no memory.db, skipping");
        return Ok((0, 0));
    }

    let conn = rusqlite::Connection::open(&db_path)?;
    let (un_consolidated, deleted) = run_domain_cluster_cleanup_sql(&conn)?;

    // Write marker
    if let Some(parent) = marker.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&marker, "applied")?;

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

// Note: prune_noise_memories and consolidate_clusters use Paths::brain_dir()
// which can't be overridden in unit tests. The SQL correctness is validated
// by the daemon at startup. The ingest filter tests in ingestion.rs cover
// the filtering logic independently.
