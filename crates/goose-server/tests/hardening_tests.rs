//! PR 1 hardening tests — four pure-logic surfaces.
//!
//! Surfaces:
//! 1. Annotation parser (parse_structured_description)
//! 2. Consolidation cluster detection (SQL helpers)
//! 3. Auto-skill hash (compute_argument_shape_hash)
//! 4. Recall filter (filter_recall_hits)

// ═══════════════════════════════════════════════════════════════════
// Surface 1: Annotation parser
// ═══════════════════════════════════════════════════════════════════

mod annotation_parser {
    use permagent::agents::platform_extensions::librarian::parse_structured_description;

    fn fixture(name: &str) -> String {
        let path = format!(
            "{}/tests/fixtures/librarian/{}.txt",
            env!("CARGO_MANIFEST_DIR"),
            name
        );
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Failed to read fixture {}: {}", path, e))
    }

    // ── Well-formed inputs ──

    #[test]
    fn well_formed_slack() {
        let raw = fixture("well_formed_slack");
        let result = parse_structured_description(&raw).unwrap();
        assert!(result.starts_with("Jesse asked Henry"), "got: {result}");
        assert!(result.contains("Related terms:"));
        assert!(result.contains("joke, jokes"));
        assert!(result.contains("Categories:"));
        assert!(result.contains("conversation, chat"));
    }

    #[test]
    fn well_formed_browser() {
        let raw = fixture("well_formed_browser");
        let result = parse_structured_description(&raw).unwrap();
        assert!(result.contains("Gmail"));
        assert!(result.contains("navigate, navigation"));
        assert!(result.contains("web browsing, email"));
    }

    #[test]
    fn well_formed_task() {
        let raw = fixture("well_formed_task");
        let result = parse_structured_description(&raw).unwrap();
        assert!(result.contains("Phase 2 documentation"));
        assert!(result.contains("task, tasks"));
        assert!(result.contains("software development"));
    }

    #[test]
    fn well_formed_unicode() {
        let raw = fixture("well_formed_unicode");
        let result = parse_structured_description(&raw).unwrap();
        assert!(result.contains("café"));
        assert!(result.contains("résumé"));
        assert!(result.contains("internationalization"));
    }

    #[test]
    fn well_formed_quotes() {
        let raw = fixture("well_formed_quotes");
        let result = parse_structured_description(&raw).unwrap();
        assert!(result.contains("don't forget the API key"));
        assert!(result.contains("API, key"));
    }

    #[test]
    fn well_formed_extra_whitespace() {
        let raw = fixture("well_formed_extra_whitespace");
        let result = parse_structured_description(&raw).unwrap();
        assert!(result.starts_with("Dr. Patel"), "got: {result}");
        assert!(result.contains("doctor, doctors"));
        assert!(result.contains("healthcare, medical"));
    }

    // ── Malformed inputs (retry path triggers) ──

    #[test]
    fn malformed_missing_categories_returns_none() {
        let raw = fixture("malformed_missing_categories");
        assert!(parse_structured_description(&raw).is_none());
    }

    #[test]
    fn malformed_too_few_terms_returns_none() {
        let raw = fixture("malformed_too_few_terms");
        assert!(parse_structured_description(&raw).is_none());
    }

    #[test]
    fn malformed_empty_sections_returns_none() {
        let raw = fixture("malformed_empty_sections");
        assert!(parse_structured_description(&raw).is_none());
    }

    // ── Raw fallback path ──

    #[test]
    fn raw_fallback_has_no_structure() {
        let raw = fixture("raw_fallback");
        // Raw fallback text has no FACTS/TERMS/CATEGORIES — parser returns None,
        // and describe_one stores the raw string as-is with Fallback quality.
        assert!(parse_structured_description(&raw).is_none());
    }

    // ── Edge cases ──

    #[test]
    fn empty_input_returns_none() {
        assert!(parse_structured_description("").is_none());
    }

    #[test]
    fn only_whitespace_returns_none() {
        assert!(parse_structured_description("   \n\n  \t  ").is_none());
    }

    // These boundary cases used `a, b, c, d` / `x, y` as placeholders. Index
    // cleaning now drops single-character items — one letter cannot
    // discriminate between memories — so those inputs produced ZERO usable
    // terms and the count boundary was never actually being tested. Two of
    // them failed outright; `one_category_rejected` kept passing for the wrong
    // reason (rejected as noise, not as too-few). Real words restore the
    // stated intent, and the noise rules get their own tests below.

    #[test]
    fn missing_facts_returns_none() {
        let raw = "TERMS: alpha, beta, gamma, delta\nCATEGORIES: work, notes";
        assert!(parse_structured_description(raw).is_none());
    }

    #[test]
    fn exactly_four_terms_accepted() {
        let raw =
            "FACTS: Something happened.\nTERMS: alpha, beta, gamma, delta\nCATEGORIES: work, notes";
        assert!(parse_structured_description(raw).is_some());
    }

    #[test]
    fn exactly_two_categories_accepted() {
        let raw =
            "FACTS: Something happened.\nTERMS: alpha, beta, gamma, delta\nCATEGORIES: work, notes";
        let result = parse_structured_description(raw).unwrap();
        assert!(result.contains("Categories: work, notes."));
    }

    #[test]
    fn one_category_rejected() {
        let raw = "FACTS: Something happened.\nTERMS: alpha, beta, gamma, delta\nCATEGORIES: work";
        assert!(parse_structured_description(raw).is_none());
    }

    // ── Index-quality rules (2026-08-13: a quantised 30B model padded these
    //    lists with digits, single letters, and duplicates) ──

    #[test]
    fn single_character_terms_do_not_count_toward_the_minimum() {
        // Four "terms", none of which can discriminate anything.
        let raw = "FACTS: Something happened.\nTERMS: a, b, c, d\nCATEGORIES: work, notes";
        assert!(parse_structured_description(raw).is_none());
    }

    #[test]
    fn bare_numbers_do_not_count_toward_the_minimum() {
        let raw =
            "FACTS: A migration ran.\nTERMS: migration, 2733, 19, 41\nCATEGORIES: work, notes";
        assert!(
            parse_structured_description(raw).is_none(),
            "one real term plus three numbers is not four terms"
        );
    }

    #[test]
    fn duplicates_do_not_pad_a_list_to_the_minimum() {
        let raw =
            "FACTS: Something happened.\nTERMS: alpha, Alpha, ALPHA, beta\nCATEGORIES: work, notes";
        assert!(
            parse_structured_description(raw).is_none(),
            "case-insensitive duplicates collapse to two distinct terms"
        );
    }

    #[test]
    fn a_long_list_is_capped_without_being_rejected() {
        let terms = (1..=20)
            .map(|i| format!("term{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let raw = format!("FACTS: Something happened.\nTERMS: {terms}\nCATEGORIES: work, notes");
        let result =
            parse_structured_description(&raw).expect("over-long lists are trimmed, not refused");
        // Leading terms are the model's most salient, so the cap keeps the head.
        assert!(result.contains("term1, term2"), "got: {result}");
        assert!(
            !result.contains("term11"),
            "cap of 10 not applied: {result}"
        );
    }

    #[test]
    fn output_format_is_correct() {
        let raw = "FACTS: The sky is blue.\nTERMS: sky, blue, color, colours\nCATEGORIES: nature, weather";
        let result = parse_structured_description(raw).unwrap();
        assert_eq!(
            result,
            "The sky is blue. Related terms: sky, blue, color, colours. Categories: nature, weather."
        );
    }
}

// ═══════════════════════════════════════════════════════════════════
// Surface 2: Consolidation cluster detection
// ═══════════════════════════════════════════════════════════════════

mod consolidation_clusters {
    use permagent_daemon::routes::librarian::consolidation::{
        find_domain_clusters, find_exact_duplicate_clusters,
    };

    /// Create an in-memory SQLite DB with the memories schema + consolidation_edges.
    fn setup_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE memories (
                id TEXT PRIMARY KEY,
                key TEXT NOT NULL,
                content TEXT NOT NULL,
                source TEXT,
                created_at TEXT,
                last_reinforced_at TEXT
            );
            CREATE TABLE consolidation_edges (
                source_key TEXT PRIMARY KEY,
                target_key TEXT NOT NULL,
                consolidated_at TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(source_key, target_key)
            );
            CREATE INDEX idx_consolidation_target ON consolidation_edges(target_key);",
        )
        .unwrap();
        conn
    }

    /// Create an in-memory SQLite DB with the OLD schema (includes _pm_consolidated_into).
    /// Used for testing the domain cluster cleanup migration which operates on the old column.
    fn setup_legacy_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE memories (
                id TEXT PRIMARY KEY,
                key TEXT NOT NULL,
                content TEXT NOT NULL,
                source TEXT,
                created_at TEXT,
                last_reinforced_at TEXT,
                _pm_consolidated_into TEXT DEFAULT NULL
            );
            CREATE TABLE consolidation_edges (
                source_key TEXT PRIMARY KEY,
                target_key TEXT NOT NULL,
                consolidated_at TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(source_key, target_key)
            );
            CREATE INDEX idx_consolidation_target ON consolidation_edges(target_key);",
        )
        .unwrap();
        conn
    }

    fn insert_memory(
        conn: &rusqlite::Connection,
        id: &str,
        key: &str,
        content: &str,
        source: &str,
        created_at: &str,
    ) {
        conn.execute(
            "INSERT INTO memories (id, key, content, source, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, key, content, source, created_at],
        ).unwrap();
    }

    // ── Exact duplicate clusters ──

    #[test]
    fn identical_content_forms_cluster() {
        let conn = setup_db();
        insert_memory(
            &conn,
            "m1",
            "k1",
            "Hello world",
            "test",
            "2026-01-01T00:00:00Z",
        );
        insert_memory(
            &conn,
            "m2",
            "k2",
            "Hello world",
            "test",
            "2026-01-02T00:00:00Z",
        );
        insert_memory(
            &conn,
            "m3",
            "k3",
            "Hello world",
            "test",
            "2026-01-03T00:00:00Z",
        );

        let clusters = find_exact_duplicate_clusters(&conn).unwrap();
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].0, "Hello world");
        assert_eq!(clusters[0].1, 3);
    }

    #[test]
    fn distinct_content_no_clusters() {
        let conn = setup_db();
        insert_memory(&conn, "m1", "k1", "Alpha", "test", "2026-01-01T00:00:00Z");
        insert_memory(&conn, "m2", "k2", "Beta", "test", "2026-01-02T00:00:00Z");
        insert_memory(&conn, "m3", "k3", "Gamma", "test", "2026-01-03T00:00:00Z");

        let clusters = find_exact_duplicate_clusters(&conn).unwrap();
        assert!(clusters.is_empty());
    }

    #[test]
    fn single_memory_no_cluster() {
        let conn = setup_db();
        insert_memory(
            &conn,
            "m1",
            "k1",
            "Only one",
            "test",
            "2026-01-01T00:00:00Z",
        );

        let clusters = find_exact_duplicate_clusters(&conn).unwrap();
        assert!(clusters.is_empty());
    }

    #[test]
    fn already_consolidated_excluded() {
        let conn = setup_db();
        insert_memory(
            &conn,
            "m1",
            "k1",
            "Duplicate",
            "test",
            "2026-01-01T00:00:00Z",
        );
        insert_memory(
            &conn,
            "m2",
            "k2",
            "Duplicate",
            "test",
            "2026-01-02T00:00:00Z",
        );
        // Mark k2 as consolidated into k1 via consolidation_edges
        conn.execute(
            "INSERT INTO consolidation_edges (source_key, target_key) VALUES ('k2', 'k1')",
            [],
        )
        .unwrap();

        let clusters = find_exact_duplicate_clusters(&conn).unwrap();
        assert!(
            clusters.is_empty(),
            "Consolidated memories should be excluded"
        );
    }

    #[test]
    fn multiple_distinct_clusters() {
        let conn = setup_db();
        insert_memory(&conn, "m1", "k1", "AAA", "test", "2026-01-01T00:00:00Z");
        insert_memory(&conn, "m2", "k2", "AAA", "test", "2026-01-02T00:00:00Z");
        insert_memory(&conn, "m3", "k3", "BBB", "test", "2026-01-03T00:00:00Z");
        insert_memory(&conn, "m4", "k4", "BBB", "test", "2026-01-04T00:00:00Z");
        insert_memory(&conn, "m5", "k5", "CCC", "test", "2026-01-05T00:00:00Z"); // singleton

        let clusters = find_exact_duplicate_clusters(&conn).unwrap();
        assert_eq!(clusters.len(), 2);
    }

    // ── Domain clusters ──

    #[test]
    fn browser_domain_cluster_with_three_entries() {
        let conn = setup_db();
        insert_memory(
            &conn,
            "m1",
            "k1",
            "Navigated to https://github.com/repo1",
            "permagent.activity",
            "2026-01-01T00:00:00Z",
        );
        insert_memory(
            &conn,
            "m2",
            "k2",
            "Navigated to https://github.com/repo2",
            "permagent.activity",
            "2026-01-02T00:00:00Z",
        );
        insert_memory(
            &conn,
            "m3",
            "k3",
            "Navigated to https://github.com/repo3",
            "permagent.activity",
            "2026-01-03T00:00:00Z",
        );

        let clusters = find_domain_clusters(&conn).unwrap();
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].0, "github.com");
        assert_eq!(clusters[0].1, 3);
    }

    #[test]
    fn browser_domain_two_entries_not_enough() {
        let conn = setup_db();
        insert_memory(
            &conn,
            "m1",
            "k1",
            "Navigated to https://example.com/page1",
            "permagent.activity",
            "2026-01-01T00:00:00Z",
        );
        insert_memory(
            &conn,
            "m2",
            "k2",
            "Navigated to https://example.com/page2",
            "permagent.activity",
            "2026-01-02T00:00:00Z",
        );

        let clusters = find_domain_clusters(&conn).unwrap();
        assert!(
            clusters.is_empty(),
            "Need 3+ entries to form a domain cluster"
        );
    }

    #[test]
    fn non_activity_source_excluded() {
        let conn = setup_db();
        insert_memory(
            &conn,
            "m1",
            "k1",
            "Navigated to https://test.com/a",
            "other.source",
            "2026-01-01T00:00:00Z",
        );
        insert_memory(
            &conn,
            "m2",
            "k2",
            "Navigated to https://test.com/b",
            "other.source",
            "2026-01-02T00:00:00Z",
        );
        insert_memory(
            &conn,
            "m3",
            "k3",
            "Navigated to https://test.com/c",
            "other.source",
            "2026-01-03T00:00:00Z",
        );

        let clusters = find_domain_clusters(&conn).unwrap();
        assert!(
            clusters.is_empty(),
            "Only permagent.activity source should be grouped"
        );
    }

    #[test]
    fn http_and_https_extract_different_domains() {
        let conn = setup_db();
        insert_memory(
            &conn,
            "m1",
            "k1",
            "Navigated to http://local.dev/api",
            "permagent.activity",
            "2026-01-01T00:00:00Z",
        );
        insert_memory(
            &conn,
            "m2",
            "k2",
            "Navigated to http://local.dev/dashboard",
            "permagent.activity",
            "2026-01-02T00:00:00Z",
        );
        insert_memory(
            &conn,
            "m3",
            "k3",
            "Navigated to http://local.dev/settings",
            "permagent.activity",
            "2026-01-03T00:00:00Z",
        );

        let clusters = find_domain_clusters(&conn).unwrap();
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].0, "local.dev");
    }

    // ── Domain cluster cleanup migration (operates on legacy _pm_consolidated_into column) ──

    #[test]
    fn cleanup_removes_buggy_clusters_and_unconsolidates_pointers() {
        use permagent::activity::cleanup::{
            ensure_and_check_migration, mark_migration, run_domain_cluster_cleanup_sql,
        };

        let conn = setup_legacy_db();

        // Insert the two buggy catchall cluster memories
        insert_memory(
            &conn,
            "tps_cluster",
            "consolidated:browser:tps:",
            "tps: — visited 50 times",
            "librarian.consolidation",
            "2026-01-10T00:00:00Z",
        );
        insert_memory(
            &conn,
            "ttp_cluster",
            "consolidated:browser:ttp:",
            "ttp: — visited 20 times",
            "librarian.consolidation",
            "2026-01-10T00:00:00Z",
        );

        // 3 memories pointing to the https catchall
        insert_memory(
            &conn,
            "m1",
            "k1",
            "Navigated to https://github.com/repo1",
            "permagent.activity",
            "2026-01-01T00:00:00Z",
        );
        insert_memory(
            &conn,
            "m2",
            "k2",
            "Navigated to https://github.com/repo2",
            "permagent.activity",
            "2026-01-02T00:00:00Z",
        );
        insert_memory(
            &conn,
            "m3",
            "k3",
            "Navigated to https://github.com/repo3",
            "permagent.activity",
            "2026-01-03T00:00:00Z",
        );
        conn.execute("UPDATE memories SET _pm_consolidated_into = 'tps_cluster' WHERE id IN ('m1','m2','m3')", []).unwrap();

        // 2 memories pointing to the http catchall
        insert_memory(
            &conn,
            "m4",
            "k4",
            "Navigated to http://local.dev/api",
            "permagent.activity",
            "2026-01-04T00:00:00Z",
        );
        insert_memory(
            &conn,
            "m5",
            "k5",
            "Navigated to http://local.dev/dash",
            "permagent.activity",
            "2026-01-05T00:00:00Z",
        );
        conn.execute(
            "UPDATE memories SET _pm_consolidated_into = 'ttp_cluster' WHERE id IN ('m4','m5')",
            [],
        )
        .unwrap();

        // 1 control memory — should not be touched
        insert_memory(
            &conn,
            "ctrl",
            "k_ctrl",
            "Unrelated memory",
            "test",
            "2026-01-06T00:00:00Z",
        );

        // Migration not yet applied
        assert!(!ensure_and_check_migration(&conn, "domain_cluster_cleanup_v1").unwrap());

        // Run cleanup
        let (un_consolidated, deleted) = run_domain_cluster_cleanup_sql(&conn).unwrap();
        mark_migration(&conn, "domain_cluster_cleanup_v1").unwrap();

        // 5 memories un-consolidated
        assert_eq!(un_consolidated, 5);
        // 2 catchall cluster memories deleted
        assert_eq!(deleted, 2);

        // Verify: all 5 now have _pm_consolidated_into = NULL
        let still_consolidated: usize = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE _pm_consolidated_into IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(still_consolidated, 0);

        // Verify: catchall clusters are gone
        let catchall_count: usize = conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE key LIKE 'consolidated:browser:tps:%' OR key LIKE 'consolidated:browser:ttp:%'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(catchall_count, 0);

        // Verify: control memory unchanged
        let ctrl_content: String = conn
            .query_row("SELECT content FROM memories WHERE id = 'ctrl'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(ctrl_content, "Unrelated memory");

        // Migration is now marked
        assert!(ensure_and_check_migration(&conn, "domain_cluster_cleanup_v1").unwrap());

        // Idempotency: run again — no changes
        let (un2, del2) = run_domain_cluster_cleanup_sql(&conn).unwrap();
        assert_eq!(un2, 0);
        assert_eq!(del2, 0);
    }

    // ── Consolidation migration: _pm_consolidated_into → consolidation_edges ──

    #[test]
    fn consolidate_into_migration_migrates_and_drops_column() {
        use permagent::activity::cleanup::{
            ensure_and_check_migration, run_consolidate_into_migration_sql,
        };

        let conn = setup_legacy_db();

        // Cluster 1: 3 sources → 1 target
        insert_memory(
            &conn,
            "t1",
            "target1",
            "Summary A",
            "test",
            "2026-01-01T00:00:00Z",
        );
        insert_memory(
            &conn,
            "s1",
            "src1",
            "Detail A1",
            "test",
            "2026-01-02T00:00:00Z",
        );
        insert_memory(
            &conn,
            "s2",
            "src2",
            "Detail A2",
            "test",
            "2026-01-03T00:00:00Z",
        );
        insert_memory(
            &conn,
            "s3",
            "src3",
            "Detail A3",
            "test",
            "2026-01-04T00:00:00Z",
        );
        conn.execute(
            "UPDATE memories SET _pm_consolidated_into = 't1' WHERE id IN ('s1','s2','s3')",
            [],
        )
        .unwrap();

        // Cluster 2: 2 sources → 1 target
        insert_memory(
            &conn,
            "t2",
            "target2",
            "Summary B",
            "test",
            "2026-01-05T00:00:00Z",
        );
        insert_memory(
            &conn,
            "s4",
            "src4",
            "Detail B1",
            "test",
            "2026-01-06T00:00:00Z",
        );
        insert_memory(
            &conn,
            "s5",
            "src5",
            "Detail B2",
            "test",
            "2026-01-07T00:00:00Z",
        );
        conn.execute(
            "UPDATE memories SET _pm_consolidated_into = 't2' WHERE id IN ('s4','s5')",
            [],
        )
        .unwrap();

        // Control: not consolidated
        insert_memory(
            &conn,
            "ctrl",
            "ctrl_key",
            "Not consolidated",
            "test",
            "2026-01-08T00:00:00Z",
        );

        // Dangling reference: source points to non-existent target id
        insert_memory(
            &conn,
            "orphan",
            "orphan_key",
            "Orphaned",
            "test",
            "2026-01-09T00:00:00Z",
        );
        conn.execute(
            "UPDATE memories SET _pm_consolidated_into = 'nonexistent_id' WHERE id = 'orphan'",
            [],
        )
        .unwrap();

        // Run migration
        let stats = run_consolidate_into_migration_sql(&conn).unwrap();

        // 5 rows migrated (3 in cluster 1 + 2 in cluster 2), orphan skipped
        assert_eq!(stats.rows_migrated, 5);
        assert_eq!(stats.distinct_targets, 2);
        assert_eq!(stats.orphans_skipped, 1);
        assert!(
            stats.column_dropped,
            "Column should be dropped after successful cross-check"
        );

        // Verify consolidation_edges has 5 rows
        let edge_count: usize = conn
            .query_row("SELECT COUNT(*) FROM consolidation_edges", [], |r| r.get(0))
            .unwrap();
        assert_eq!(edge_count, 5);

        // Verify correct source→target mappings
        let t1_sources: usize = conn
            .query_row(
                "SELECT COUNT(*) FROM consolidation_edges WHERE target_key = 'target1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(t1_sources, 3);

        let t2_sources: usize = conn
            .query_row(
                "SELECT COUNT(*) FROM consolidation_edges WHERE target_key = 'target2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(t2_sources, 2);

        // Verify column is dropped (querying it should fail)
        assert!(
            conn.prepare("SELECT _pm_consolidated_into FROM memories LIMIT 0")
                .is_err(),
            "_pm_consolidated_into column should be dropped"
        );

        // Verify control memory still exists and is unaffected
        let ctrl_content: String = conn
            .query_row("SELECT content FROM memories WHERE id = 'ctrl'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(ctrl_content, "Not consolidated");

        // Migration is marked
        assert!(ensure_and_check_migration(&conn, "consolidate_into_migration_v1").unwrap());

        // Idempotency: re-run is a no-op
        let stats2 = run_consolidate_into_migration_sql(&conn).unwrap();
        assert_eq!(stats2.rows_migrated, 0);
    }

    #[test]
    fn consolidate_into_migration_noop_without_column() {
        use permagent::activity::cleanup::run_consolidate_into_migration_sql;

        // DB without _pm_consolidated_into column (post-migration state)
        let conn = setup_db();

        let stats = run_consolidate_into_migration_sql(&conn).unwrap();
        assert_eq!(stats.rows_migrated, 0);
        assert!(!stats.column_dropped);
    }
}

// ═══════════════════════════════════════════════════════════════════
// Surface 3: Auto-skill hash
// ═══════════════════════════════════════════════════════════════════

mod auto_skill_hash {
    use permagent::tasks::compute_argument_shape_hash;

    #[test]
    fn reordered_keys_produce_same_hash() {
        let args1 = serde_json::json!({"alpha": "x", "beta": 1, "gamma": true});
        let args2 = serde_json::json!({"gamma": true, "alpha": "y", "beta": 2});
        let h1 = compute_argument_shape_hash(Some("tool"), Some(&args1));
        let h2 = compute_argument_shape_hash(Some("tool"), Some(&args2));
        assert_eq!(h1, h2, "Key order should not affect hash");
    }

    #[test]
    fn different_types_same_key_different_hash() {
        let args_string = serde_json::json!({"field": "text"});
        let args_number = serde_json::json!({"field": 42});
        let h1 = compute_argument_shape_hash(Some("tool"), Some(&args_string));
        let h2 = compute_argument_shape_hash(Some("tool"), Some(&args_number));
        assert_ne!(
            h1, h2,
            "Different value types should produce different hashes"
        );
    }

    #[test]
    fn array_type_in_hash() {
        let args = serde_json::json!({"items": [1, 2, 3]});
        let h = compute_argument_shape_hash(Some("tool"), Some(&args)).unwrap();
        assert_eq!(h.len(), 16);
        // Array type should be categorized as "array" — verify by comparing with object
        let args_obj = serde_json::json!({"items": {"a": 1}});
        let h2 = compute_argument_shape_hash(Some("tool"), Some(&args_obj)).unwrap();
        assert_ne!(h, h2, "Array and object types should hash differently");
    }

    #[test]
    fn object_type_in_hash() {
        let args = serde_json::json!({"config": {"nested": true}});
        let h = compute_argument_shape_hash(Some("tool"), Some(&args)).unwrap();
        assert_eq!(h.len(), 16);
    }

    #[test]
    fn null_type_in_hash() {
        let args = serde_json::json!({"field": null});
        let h = compute_argument_shape_hash(Some("tool"), Some(&args)).unwrap();
        assert_eq!(h.len(), 16);
        // Null should differ from string
        let args_str = serde_json::json!({"field": ""});
        let h2 = compute_argument_shape_hash(Some("tool"), Some(&args_str)).unwrap();
        assert_ne!(h, h2, "Null and string types should hash differently");
    }

    #[test]
    fn truncation_exactly_16_hex() {
        let args = serde_json::json!({"a": 1, "b": "hello", "c": true, "d": [1,2], "e": {"x": 1}});
        let h = compute_argument_shape_hash(Some("complex_tool_name"), Some(&args)).unwrap();
        assert_eq!(h.len(), 16, "Hash should be exactly 16 hex characters");
        assert!(
            h.chars().all(|c| c.is_ascii_hexdigit()),
            "Should be valid hex"
        );
    }

    #[test]
    fn pinned_vector_test() {
        // Lock the algorithm: this exact input MUST produce this exact hash.
        // If this test breaks, the hash algorithm has changed and auto-skill
        // detection will lose continuity with existing task data.
        let args = serde_json::json!({"query": "is:unread", "max_results": 10});
        let h = compute_argument_shape_hash(Some("gmail__search"), Some(&args)).unwrap();
        assert_eq!(
            h, "cc604cbb3886e776",
            "Pinned hash — algorithm must not change"
        );
    }

    #[test]
    fn empty_object_produces_hash() {
        let args = serde_json::json!({});
        let h = compute_argument_shape_hash(Some("tool"), Some(&args)).unwrap();
        assert_eq!(h.len(), 16);
    }

    #[test]
    fn non_object_value_returns_none() {
        let args = serde_json::json!("just a string");
        assert!(compute_argument_shape_hash(Some("tool"), Some(&args)).is_none());

        let args = serde_json::json!(42);
        assert!(compute_argument_shape_hash(Some("tool"), Some(&args)).is_none());
    }
}

// ═══════════════════════════════════════════════════════════════════
// Surface 4: Recall filter
// ═══════════════════════════════════════════════════════════════════

mod recall_filter {
    use permagent_daemon::brain_ops::{filter_recall_hits, RECALL_SCORE_FLOOR, RECALL_TOP_K};

    fn make_hit(id: &str, content: &str, score: f64) -> spectral::ingest::MemoryHit {
        spectral::ingest::MemoryHit {
            id: id.to_string(),
            key: format!("key:{id}"),
            content: content.to_string(),
            wing: None,
            hall: None,
            signal_score: score,
            visibility: "private".to_string(),
            hits: 1,
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
            episode_id: None,
            declarative_density: None,
            description: None,
            source_brain_id: None,
            signature: None,
        }
    }

    #[test]
    fn constants_match_spec() {
        assert!((RECALL_SCORE_FLOOR - 0.7).abs() < f64::EPSILON);
        assert_eq!(RECALL_TOP_K, 3);
    }

    #[test]
    fn score_boundary_0_69_excluded() {
        let hits = vec![make_hit("m1", "below threshold", 0.69)];
        let filtered = filter_recall_hits(&hits);
        assert!(filtered.is_empty());
    }

    #[test]
    fn score_boundary_0_70_included() {
        let hits = vec![make_hit("m1", "at threshold", 0.70)];
        let filtered = filter_recall_hits(&hits);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].content, "at threshold");
    }

    #[test]
    fn top_k_cap_respected() {
        let hits: Vec<_> = (0..10)
            .map(|i| {
                make_hit(
                    &format!("m{i}"),
                    &format!("hit {i}"),
                    0.8 + (i as f64) * 0.01,
                )
            })
            .collect();
        let filtered = filter_recall_hits(&hits);
        assert_eq!(filtered.len(), 3, "Should cap at RECALL_TOP_K=3");
    }

    #[test]
    fn empty_input_empty_output() {
        let hits: Vec<spectral::ingest::MemoryHit> = vec![];
        let filtered = filter_recall_hits(&hits);
        assert!(filtered.is_empty());
    }

    #[test]
    fn mixed_scores_filters_correctly() {
        let hits = vec![
            make_hit("m1", "high", 0.95),
            make_hit("m2", "low", 0.5),
            make_hit("m3", "medium", 0.75),
            make_hit("m4", "just below", 0.69),
            make_hit("m5", "exact", 0.70),
        ];
        let filtered = filter_recall_hits(&hits);
        assert_eq!(filtered.len(), 3, "3 of 5 are >= 0.7, capped at top-K=3");
        // Should take the first 3 that pass the filter (preserves input order)
        assert_eq!(filtered[0].content, "high");
        assert_eq!(filtered[1].content, "medium");
        assert_eq!(filtered[2].content, "exact");
    }

    #[test]
    fn all_below_threshold_returns_empty() {
        let hits = vec![
            make_hit("m1", "a", 0.1),
            make_hit("m2", "b", 0.3),
            make_hit("m3", "c", 0.69),
        ];
        let filtered = filter_recall_hits(&hits);
        assert!(filtered.is_empty());
    }

    #[test]
    fn exactly_three_above_threshold() {
        let hits = vec![
            make_hit("m1", "a", 0.8),
            make_hit("m2", "b", 0.9),
            make_hit("m3", "c", 0.7),
        ];
        let filtered = filter_recall_hits(&hits);
        assert_eq!(filtered.len(), 3);
    }

    // Verify that brain_ops is the single source of truth for recall filtering
    // (both reply.rs and session_events.rs now delegate to brain_ops::inject_recall)
    #[test]
    fn filter_is_deterministic() {
        let hits = vec![
            make_hit("m1", "a", 0.8),
            make_hit("m2", "b", 0.5),
            make_hit("m3", "c", 0.9),
        ];

        let r1 = filter_recall_hits(&hits);
        let r2 = filter_recall_hits(&hits);
        assert_eq!(r1.len(), r2.len());
        for (a, b) in r1.iter().zip(r2.iter()) {
            assert_eq!(a.id, b.id);
        }
    }
}
