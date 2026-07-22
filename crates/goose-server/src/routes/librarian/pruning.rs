//! Navigation-memory pruning and stale-memory detection.
//!
//! Conservative noise removal. Only runs when pruning_enabled = true
//! in the LibrarianSchedule. ALL conditions must be true to prune a memory.

pub(super) fn run_pruning_pass() -> Result<usize, String> {
    let db_path = permagent::config::paths::Paths::brain_dir().join("memory.db");
    let conn = rusqlite::Connection::open(&db_path)
        .map_err(|e| format!("Failed to open brain DB: {e}"))?;

    // #276: enforce foreign keys on this raw write connection.
    //
    // `PRAGMA foreign_keys` is *per-connection* and defaults OFF. The child
    // tables of `memories` (memory_annotations, memory_spectrogram,
    // constellation_fingerprints) all carry `ON DELETE CASCADE` in the pinned
    // Spectral schema, but a bare `DELETE FROM memories` on a connection with
    // FKs OFF silently orphans them — the exact bug PR #260 had to clean up
    // (110,982 orphaned rows). With FKs ON the delete cascades for free, no
    // explicit child deletes required. Must be set before any statement.
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(|e| format!("Failed to enable foreign_keys: {e}"))?;

    prune_pass_on_conn(&conn)
}

/// Run the pruning pass on an arbitrary connection.
///
/// The connection MUST already have `PRAGMA foreign_keys = ON` set so that
/// deletes cascade to child tables (see [`run_pruning_pass`]). Exposed for
/// testing — the production entrypoint is [`run_pruning_pass`].
pub(super) fn prune_pass_on_conn(conn: &rusqlite::Connection) -> Result<usize, String> {
    let annotation_table_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='memory_annotations'",
            [],
            |r| r.get::<_, usize>(0),
        )
        .unwrap_or(0)
        > 0;

    // Find candidates: short content, activity source, never recalled, not consolidated
    let mut stmt = conn
        .prepare(
            "SELECT m.id, m.key, m.content FROM memories m \
             WHERE length(m.content) < 20 \
               AND m.source = 'permagent.activity' \
               AND m.last_reinforced_at IS NULL \
               AND m.key NOT IN (SELECT source_key FROM consolidation_edges)",
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

        // All conditions met — prune (hard delete). FK enforcement (set by the
        // caller) cascades the delete to child tables instead of orphaning them.
        match conn.execute("DELETE FROM memories WHERE id = ?1", [id]) {
            Ok(_) => {
                tracing::info!(
                    target: "permagentd::librarian",
                    key,
                    content,
                    reason = "short content, no entities, no recall, activity source",
                    "Pruned noise memory"
                );
                pruned += 1;
            }
            Err(e) => {
                // With FKs ON and the pinned CASCADE schema this should not
                // happen; if a legacy NO-ACTION table is ever encountered the
                // delete is refused (RESTRICT) rather than orphaning children —
                // safe, but log it so the stale schema is visible.
                tracing::warn!(
                    target: "permagentd::librarian",
                    key,
                    error = %e,
                    "Prune delete failed (FK enforcement refused it or DB busy) — skipping"
                );
            }
        }
    }

    Ok(pruned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// Build a minimal schema mirroring the pinned Spectral store: the
    /// `memories` parent plus the child tables that reference it with
    /// `ON DELETE CASCADE`. FTS/triggers are omitted (not exercised by pruning).
    fn schema(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE memories (
                 id                  TEXT PRIMARY KEY,
                 key                 TEXT NOT NULL UNIQUE,
                 content             TEXT NOT NULL,
                 source              TEXT DEFAULT NULL,
                 last_reinforced_at  TEXT DEFAULT NULL
             );
             CREATE TABLE constellation_fingerprints (
                 id                TEXT PRIMARY KEY,
                 fingerprint_hash  TEXT NOT NULL,
                 anchor_memory_id  TEXT NOT NULL,
                 target_memory_id  TEXT NOT NULL,
                 FOREIGN KEY (anchor_memory_id) REFERENCES memories(id) ON DELETE CASCADE,
                 FOREIGN KEY (target_memory_id) REFERENCES memories(id) ON DELETE CASCADE
             );
             CREATE TABLE memory_spectrogram (
                 memory_id  TEXT PRIMARY KEY,
                 novelty    REAL,
                 FOREIGN KEY (memory_id) REFERENCES memories(id) ON DELETE CASCADE
             );
             CREATE TABLE memory_annotations (
                 id          TEXT PRIMARY KEY,
                 memory_id   TEXT NOT NULL,
                 description TEXT NOT NULL,
                 FOREIGN KEY (memory_id) REFERENCES memories(id) ON DELETE CASCADE
             );
             CREATE TABLE consolidation_edges (
                 source_key      TEXT NOT NULL,
                 target_key      TEXT NOT NULL,
                 PRIMARY KEY (source_key, target_key)
             );",
        )
        .unwrap();
    }

    /// Insert a prunable noise memory (short content, activity source, never
    /// reinforced) plus fingerprint + spectrogram children pointing at it.
    fn insert_noise_with_children(conn: &Connection, id: &str, key: &str) {
        conn.execute(
            "INSERT INTO memories (id, key, content, source, last_reinforced_at) \
             VALUES (?1, ?2, 'x', 'permagent.activity', NULL)",
            rusqlite::params![id, key],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO constellation_fingerprints (id, fingerprint_hash, anchor_memory_id, target_memory_id) \
             VALUES (?1, 'h', ?2, ?2)",
            rusqlite::params![format!("fp-{id}"), id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memory_spectrogram (memory_id, novelty) VALUES (?1, 0.5)",
            rusqlite::params![id],
        )
        .unwrap();
    }

    fn count(conn: &Connection, table: &str) -> usize {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    /// #276 regression: with FK enforcement ON (as `run_pruning_pass` sets),
    /// deleting a pruned memory must cascade to its child rows — no orphans.
    #[test]
    fn prune_cascades_children_with_fk_enforcement() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        schema(&conn);
        insert_noise_with_children(&conn, "m1", "activity:noise:1");

        assert_eq!(count(&conn, "constellation_fingerprints"), 1);
        assert_eq!(count(&conn, "memory_spectrogram"), 1);

        let pruned = prune_pass_on_conn(&conn).unwrap();

        assert_eq!(pruned, 1, "the noise memory should be pruned");
        assert_eq!(count(&conn, "memories"), 0, "parent gone");
        assert_eq!(
            count(&conn, "constellation_fingerprints"),
            0,
            "fingerprint child must cascade — no orphan (#276)"
        );
        assert_eq!(
            count(&conn, "memory_spectrogram"),
            0,
            "spectrogram child must cascade — no orphan (#276)"
        );
    }

    /// Negative control proving the bug: the SAME prune on a connection with
    /// FK enforcement OFF deletes the parent but ORPHANS the children — the
    /// pre-fix behavior the pragma in `run_pruning_pass` prevents.
    #[test]
    fn prune_without_fk_enforcement_orphans_children() {
        let conn = Connection::open_in_memory().unwrap();
        // foreign_keys defaults OFF; do not enable it.
        schema(&conn);
        insert_noise_with_children(&conn, "m1", "activity:noise:1");

        let pruned = prune_pass_on_conn(&conn).unwrap();

        assert_eq!(pruned, 1);
        assert_eq!(count(&conn, "memories"), 0, "parent gone");
        assert_eq!(
            count(&conn, "constellation_fingerprints"),
            1,
            "without FK enforcement the fingerprint is orphaned — this is the #276 bug"
        );
        assert_eq!(
            count(&conn, "memory_spectrogram"),
            1,
            "spectrogram orphaned"
        );
    }

    /// Pruning must respect its conservative predicate: memories with an
    /// annotation, or that are consolidation sources, or non-activity source,
    /// or already reinforced, must be left alone.
    #[test]
    fn prune_leaves_protected_memories() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        schema(&conn);

        // Annotated → protected.
        conn.execute(
            "INSERT INTO memories (id, key, content, source) VALUES ('a', 'k-a', 'x', 'permagent.activity')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO memory_annotations (id, memory_id, description) VALUES ('ann', 'a', 'd')",
            [],
        )
        .unwrap();

        // Consolidation source → protected.
        conn.execute(
            "INSERT INTO memories (id, key, content, source) VALUES ('b', 'k-b', 'x', 'permagent.activity')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO consolidation_edges (source_key, target_key) VALUES ('k-b', 'k-t')",
            [],
        )
        .unwrap();

        // Non-activity source → protected.
        conn.execute(
            "INSERT INTO memories (id, key, content, source) VALUES ('c', 'k-c', 'x', 'permagent.reader')",
            [],
        ).unwrap();

        // Long content → protected.
        conn.execute(
            "INSERT INTO memories (id, key, content, source) VALUES ('d', 'k-d', 'this content is way longer than twenty chars', 'permagent.activity')",
            [],
        ).unwrap();

        let pruned = prune_pass_on_conn(&conn).unwrap();
        assert_eq!(pruned, 0, "no protected memory should be pruned");
        assert_eq!(count(&conn, "memories"), 4);
    }
}
