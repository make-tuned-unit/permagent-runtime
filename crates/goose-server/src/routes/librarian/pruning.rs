//! Navigation-memory pruning and stale-memory detection.
//!
//! Conservative noise removal. Only runs when pruning_enabled = true
//! in the LibrarianSchedule. ALL conditions must be true to prune a memory.

pub(super) fn run_pruning_pass() -> Result<usize, String> {
    let db_path = permagent::config::paths::Paths::brain_dir().join("memory.db");
    let conn = rusqlite::Connection::open(&db_path)
        .map_err(|e| format!("Failed to open brain DB: {e}"))?;

    // Ensure columns exist
    conn.execute_batch(
        "ALTER TABLE memories ADD COLUMN _pm_consolidated_into TEXT DEFAULT NULL;"
    ).ok();

    let annotation_table_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='memory_annotations'",
            [],
            |r| r.get::<_, usize>(0),
        )
        .unwrap_or(0) > 0;

    // Find candidates: short content, activity source, never recalled, not consolidated
    let mut stmt = conn
        .prepare(
            "SELECT m.id, m.key, m.content FROM memories m \
             WHERE length(m.content) < 20 \
               AND m.source = 'permagent.activity' \
               AND m.last_reinforced_at IS NULL \
               AND m._pm_consolidated_into IS NULL",
        )
        .map_err(|e| format!("Prune query failed: {e}"))?;

    let candidates: Vec<(String, String, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| format!("Prune query map failed: {e}"))?
        .filter_map(|r| r.ok())
        .collect();

    let mut pruned = 0usize;
    for (id, key, content) in &candidates {
        // Check: zero entity connections
        if annotation_table_exists {
            let ann_count: usize = conn
                .query_row(
                    "SELECT COUNT(*) FROM memory_annotations WHERE memory_id = ?1",
                    [id],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if ann_count > 0 {
                continue;
            }
        }

        // All conditions met — prune (hard delete)
        if conn
            .execute("DELETE FROM memories WHERE id = ?1", [id])
            .is_ok()
        {
            tracing::info!(
                target: "permagentd::librarian",
                key,
                content,
                reason = "short content, no entities, no recall, activity source",
                "Pruned noise memory"
            );
            pruned += 1;
        }
    }

    Ok(pruned)
}
