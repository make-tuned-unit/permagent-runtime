//! Memory consolidation: duplicate detection, domain clustering, and merge.
//!
//! Detects redundant memory clusters and merges them into representative
//! summaries. Uses direct SQL against the Brain's memory.db — Spectral
//! has no delete/archive API (append-only by design).
//!
//! FORWARD-LOOKING NOTE: _pm_consolidated_into will be removed in an
//! upcoming PR (PR 4) once Spectral's consolidate_into API at pin a18041e
//! is in use. Keep the column and ALTER for now.

/// Find groups of memories with identical content that haven't been consolidated yet.
/// Returns Vec<(content, count)> — each entry is a cluster of exact duplicates.
pub fn find_exact_duplicate_clusters(
    conn: &rusqlite::Connection,
) -> Result<Vec<(String, usize)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT content, COUNT(*) as cnt FROM memories \
             WHERE _pm_consolidated_into IS NULL \
             GROUP BY content HAVING cnt > 1 ORDER BY cnt DESC LIMIT 50",
        )
        .map_err(|e| format!("Prepare failed: {e}"))?;
    let clusters: Vec<(String, usize)> = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?)))
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
               AND _pm_consolidated_into IS NULL \
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

/// Build the SQL UPDATE that marks a duplicate as consolidated into the keeper.
/// Returns the statement string and params. Does NOT execute.
pub fn build_consolidation_update_sql() -> &'static str {
    "UPDATE memories SET _pm_consolidated_into = ?1 WHERE id = ?2"
}

pub(super) fn run_consolidation_scan(brain: &spectral::Brain) -> Result<(usize, usize), String> {
    let db_path = permagent::config::paths::Paths::brain_dir().join("memory.db");
    let conn = rusqlite::Connection::open(&db_path)
        .map_err(|e| format!("Failed to open brain DB: {e}"))?;

    // Ensure Permagent-side column exists (idempotent)
    conn.execute_batch(
        "ALTER TABLE memories ADD COLUMN _pm_consolidated_into TEXT DEFAULT NULL;"
    ).ok(); // Ignore "duplicate column" error

    let mut total_clusters = 0usize;
    let mut total_originals = 0usize;

    // ── Strategy 1: Exact content duplicates ──
    let dup_clusters = find_exact_duplicate_clusters(&conn)?;

    for (content, count) in &dup_clusters {
        // Keep the oldest, mark the rest
        let ids: Vec<(String, String)> = conn
            .prepare(
                "SELECT id, key FROM memories \
                 WHERE content = ?1 AND _pm_consolidated_into IS NULL \
                 ORDER BY created_at ASC",
            )
            .and_then(|mut s| {
                s.query_map([content], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default();

        if ids.len() < 2 {
            continue;
        }
        let keeper_id = &ids[0].0;
        for (id, key) in &ids[1..] {
            conn.execute(
                build_consolidation_update_sql(),
                rusqlite::params![keeper_id, id],
            )
            .ok();
            tracing::debug!(
                target: "permagentd::librarian",
                key,
                keeper = keeper_id,
                "Consolidated duplicate"
            );
        }
        total_clusters += 1;
        total_originals += count - 1;
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

        if let Ok(r) = result {
            // Mark originals as consolidated
            conn.execute(
                "UPDATE memories SET _pm_consolidated_into = ?1 \
                 WHERE source = 'permagent.activity' \
                   AND _pm_consolidated_into IS NULL \
                   AND content LIKE ?2",
                rusqlite::params![r.memory_id, format!("Navigated to%{domain}%")],
            )
            .ok();

            total_clusters += 1;
            total_originals += count;

            tracing::debug!(
                target: "permagentd::librarian",
                domain,
                count,
                summary_id = r.memory_id,
                "Consolidated browser navigation cluster"
            );
        }
    }

    Ok((total_clusters, total_originals))
}
