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

// Note: prune_noise_memories and consolidate_clusters use Paths::brain_dir()
// which can't be overridden in unit tests. The SQL correctness is validated
// by the daemon at startup. The ingest filter tests in ingestion.rs cover
// the filtering logic independently.
