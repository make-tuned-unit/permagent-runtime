//! Navigation-memory pruning and stale-memory detection.
//!
//! Conservative noise removal. Only runs when pruning_enabled = true
//! in the LibrarianSchedule. ALL conditions must be true to prune a memory.

/// One memory the pruning predicate has cleared for deletion.
pub(super) struct PruneCandidate {
    pub key: String,
    pub content: String,
}

/// How long one Librarian pruning pass may spend deleting.
///
/// Shorter than the startup budget: this runs on a daily schedule alongside the
/// consolidation scan, and anything it does not reach is picked up tomorrow.
/// See `permagent::activity::cleanup::forget_memories` for why deletes are
/// bounded by wall clock at all.
const PRUNE_BUDGET: std::time::Duration = std::time::Duration::from_secs(30);

pub(super) fn run_pruning_pass() -> Result<usize, String> {
    let db_path = permagent::config::paths::Paths::brain_dir().join("memory.db");
    let conn = rusqlite::Connection::open(&db_path)
        .map_err(|e| format!("Failed to open brain DB: {e}"))?;

    let candidates = prune_candidates(&conn)?;
    // Release the read connection before the Brain writes: SQLite serializes
    // writers, and holding a second handle over the delete loop only invites
    // SQLITE_BUSY.
    drop(conn);

    if candidates.is_empty() {
        return Ok(0);
    }

    let contents: std::collections::HashMap<&str, &str> = candidates
        .iter()
        .map(|c| (c.key.as_str(), c.content.as_str()))
        .collect();
    let keys: Vec<String> = candidates.iter().map(|c| c.key.clone()).collect();

    // Called from spawn_blocking (scheduling.rs), which is what the Brain
    // handle inside `forget_memories` requires.
    let report = permagent::activity::cleanup::forget_memories(
        &keys,
        "librarian prune",
        std::time::Instant::now() + PRUNE_BUDGET,
    );

    for key in &report.forgotten {
        tracing::info!(
            target: "permagentd::librarian",
            key,
            content = contents.get(key.as_str()).copied().unwrap_or(""),
            reason = "short content, no entities, no recall, activity source",
            "Pruned noise memory"
        );
    }

    tracing::info!(
        target: "permagentd::librarian",
        pruned = report.forgotten.len(),
        candidates = candidates.len(),
        remaining = report.remaining,
        residual = report.residual,
        elapsed_ms = report.elapsed.as_millis() as u64,
        "Pruning pass complete"
    );

    Ok(report.forgotten.len())
}

/// Select the memories this pass is allowed to delete.
///
/// Conservative by construction: short content, written by the activity
/// ingester, never recalled, not a consolidation source, and carrying no
/// entity annotations. Read-only — the caller deletes.
///
/// Deletion is deliberately NOT done on this connection. `PRAGMA
/// foreign_keys = ON` (#276) cascades to the child tables of `memories`, but a
/// foreign key cannot cross database files and recognition state lives in a
/// separate one (`recognition.db`). A raw `DELETE FROM memories` therefore left
/// the whole recognition footprint behind: on this machine's brain, 5 pruned
/// memories had orphaned 154 recognition rows (5 enrolments, 96 pairs, 5 grams,
/// 5 minhash signatures, 43 minhash bands — ~31 rows each), measured
/// 2026-08-19. `Brain::forget` reaches every substrate including that sidecar,
/// and verifies afterwards, so the delete goes through it.
pub(super) fn prune_candidates(conn: &rusqlite::Connection) -> Result<Vec<PruneCandidate>, String> {
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

    let rows: Vec<(String, String, String)> = stmt
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

    let mut candidates = Vec::new();
    for (id, key, content) in rows {
        // Check: zero entity connections
        if annotation_table_exists {
            let ann_count: usize = conn
                .query_row(
                    "SELECT COUNT(*) FROM memory_annotations WHERE memory_id = ?1",
                    [&id],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if ann_count > 0 {
                continue;
            }
        }
        candidates.push(PruneCandidate { key, content });
    }

    Ok(candidates)
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

    /// The pass must select exactly the memories its predicate clears, and
    /// hand back the KEY — `Brain::forget` is keyed, and the key is what makes
    /// the delete reach the recognition sidecar in the other database file.
    #[test]
    fn prune_selects_noise_by_key() {
        let conn = Connection::open_in_memory().unwrap();
        schema(&conn);
        insert_noise_with_children(&conn, "m1", "activity:noise:1");

        let candidates = prune_candidates(&conn).unwrap();

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].key, "activity:noise:1");
        assert_eq!(candidates[0].content, "x");
        assert_eq!(
            count(&conn, "memories"),
            1,
            "selection must not delete anything — the Brain owns the delete"
        );
    }

    /// #276 was fixed by turning FK enforcement on so a raw delete cascaded to
    /// the child tables of `memories`. That cascade cannot cross a database
    /// file, and recognition state lives in `recognition.db`, so a raw delete
    /// still orphaned it. Deleting through `Brain::forget` subsumes the #276
    /// cascade AND reaches the sidecar — this test pins that the pass no longer
    /// has a raw delete to get wrong.
    #[test]
    fn prune_selection_never_writes() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        schema(&conn);
        insert_noise_with_children(&conn, "m1", "activity:noise:1");

        let _ = prune_candidates(&conn).unwrap();

        assert_eq!(count(&conn, "memories"), 1);
        assert_eq!(count(&conn, "constellation_fingerprints"), 1);
        assert_eq!(count(&conn, "memory_spectrogram"), 1);
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

        let candidates = prune_candidates(&conn).unwrap();
        assert!(
            candidates.is_empty(),
            "no protected memory should be selected for pruning"
        );
        assert_eq!(count(&conn, "memories"), 4);
    }
}
