//! Memory consolidation: duplicate detection, domain clustering, and merge.
//!
//! Detects redundant memory clusters and merges them into representative
//! summaries. Uses Spectral's consolidate_into() API for recording
//! consolidation edges; recall automatically filters consolidated sources.

use crate::brain_ops::read_only_brain_conn;

/// Find groups of memories with identical content that haven't been consolidated yet.
/// Returns Vec<(content, count)> — each entry is a cluster of exact duplicates.
pub fn find_exact_duplicate_clusters(
    conn: &rusqlite::Connection,
) -> Result<Vec<(String, usize)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT content, COUNT(*) as cnt FROM memories \
             WHERE key NOT IN (SELECT source_key FROM consolidation_edges) \
             GROUP BY content HAVING cnt > 1 ORDER BY cnt DESC LIMIT 50",
        )
        .map_err(|e| format!("Prepare failed: {e}"))?;
    let clusters: Vec<(String, usize)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
        })
        .map_err(|e| format!("Query failed: {e}"))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(clusters)
}

/// Find clusters of browser navigation memories grouped by domain.
/// Returns Vec<(domain, count, first_visit, last_visit)> for domains with 3+ entries.
pub fn find_domain_clusters(
    conn: &rusqlite::Connection,
) -> Result<Vec<(String, usize, String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT \
               CASE \
                 WHEN content LIKE 'Navigated to https://%' THEN \
                   substr(content, 22, instr(substr(content, 22), '/') - 1) \
                 WHEN content LIKE 'Navigated to http://%' THEN \
                   substr(content, 21, instr(substr(content, 21), '/') - 1) \
                 ELSE NULL \
               END as domain, \
               COUNT(*) as cnt, \
               MIN(created_at) as first_visit, \
               MAX(created_at) as last_visit \
             FROM memories \
             WHERE source = 'permagent.activity' \
               AND key NOT IN (SELECT source_key FROM consolidation_edges) \
               AND content LIKE 'Navigated to http%' \
             GROUP BY domain \
             HAVING cnt >= 3 AND domain IS NOT NULL \
             ORDER BY cnt DESC \
             LIMIT 30",
        )
        .map_err(|e| format!("Domain query failed: {e}"))?;

    let clusters: Vec<(String, usize, String, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, usize>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| format!("Domain query map failed: {e}"))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(clusters)
}

/// Caller MUST be inside spawn_blocking — uses raw_blocking_handle() internally.
pub(super) fn run_consolidation_scan(
    brain: &permagent::brain_handle::SafeBrain,
) -> Result<(usize, usize), String> {
    let brain = brain.raw_blocking_handle();
    let conn = read_only_brain_conn().map_err(|e| format!("Failed to open brain DB: {e}"))?;

    let mut total_clusters = 0usize;
    let mut total_originals = 0usize;

    // ── Strategy 1: Exact content duplicates ──
    let dup_clusters = find_exact_duplicate_clusters(&conn)?;

    for (content, _count) in &dup_clusters {
        // Keep the oldest, mark the rest
        let keys: Vec<String> = conn
            .prepare(
                "SELECT key FROM memories \
                 WHERE content = ?1 \
                   AND key NOT IN (SELECT source_key FROM consolidation_edges) \
                 ORDER BY created_at ASC",
            )
            .and_then(|mut s| {
                s.query_map([content], |row| row.get::<_, String>(0))
                    .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default();

        if keys.len() < 2 {
            continue;
        }
        let keeper_key = &keys[0];
        let source_keys: Vec<String> = keys[1..].to_vec();

        match brain.consolidate_into(
            &source_keys,
            keeper_key,
            &spectral::ingest::ConsolidateOpts::default(),
        ) {
            Ok(result) => {
                for key in &result.consolidated {
                    tracing::debug!(
                        target: "permagentd::librarian",
                        key,
                        keeper = keeper_key,
                        "Consolidated duplicate"
                    );
                }
                total_clusters += 1;
                total_originals += result.consolidated.len();
            }
            Err(e) => {
                tracing::warn!(
                    target: "permagentd::librarian",
                    error = %e,
                    keeper = keeper_key,
                    "Failed to consolidate duplicate cluster"
                );
            }
        }
    }

    // ── Strategy 2: Same-domain browser navigations with 3+ entries ──
    let domain_clusters = find_domain_clusters(&conn)?;

    for (domain, count, first_visit, last_visit) in &domain_clusters {
        let summary = format!(
            "{domain} — visited {count} times between {first} and {last}",
            first = first_visit.split('T').next().unwrap_or(first_visit),
            last = last_visit.split('T').next().unwrap_or(last_visit),
        );
        let key = format!("consolidated:browser:{domain}");

        // Create the summary memory via Brain API
        let result = brain.remember_with(
            &key,
            &summary,
            spectral::RememberOpts {
                source: Some("librarian.consolidation".into()),
                visibility: spectral::Visibility::Private,
                ..Default::default()
            },
        );

        if let Ok(_r) = result {
            // Collect the keys of the originals to consolidate
            let source_keys: Vec<String> = conn
                .prepare(
                    "SELECT key FROM memories \
                     WHERE source = 'permagent.activity' \
                       AND key NOT IN (SELECT source_key FROM consolidation_edges) \
                       AND content LIKE ?1",
                )
                .and_then(|mut s| {
                    s.query_map(
                        rusqlite::params![format!("Navigated to%{domain}%")],
                        |row| row.get::<_, String>(0),
                    )
                    .map(|rows| rows.filter_map(|r| r.ok()).collect())
                })
                .unwrap_or_default();

            if !source_keys.is_empty() {
                match brain.consolidate_into(
                    &source_keys,
                    &key,
                    &spectral::ingest::ConsolidateOpts::default(),
                ) {
                    Ok(result) => {
                        total_clusters += 1;
                        total_originals += result.consolidated.len();

                        tracing::debug!(
                            target: "permagentd::librarian",
                            domain,
                            consolidated = result.consolidated.len(),
                            summary_key = key,
                            "Consolidated browser navigation cluster"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "permagentd::librarian",
                            error = %e,
                            domain,
                            "Failed to consolidate browser navigation cluster"
                        );
                    }
                }
            }
        }
    }

    Ok((total_clusters, total_originals))
}
