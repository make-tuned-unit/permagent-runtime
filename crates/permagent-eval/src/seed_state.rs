//! Lane K8: fixture DB-state seeding.
//!
//! Some `category: "selection"` fixtures describe pre-existing coordinator
//! state in prose ("a paused roadmap with 2 goals exists", "a Ready goal
//! card already exists") that no tool the coordinator can call ever
//! actually surfaces — the driver used to seed every run's SQLite store with
//! nothing but the bare `personal` project (see `runner::run_task`), and a
//! workspace file like `roadmap-state.json` is inert (no tool reads it,
//! `HARNESS_LOOP_LOG.md` Lane R20d / A20 Q2-Q3). Both
//! `select-existing-roadmap-resumes-not-redecomposes` and
//! `select-checks-worker-before-dispatch` were therefore unsatisfiable: the
//! model could only escalate (correct given what it could see) or
//! re-decompose.
//!
//! A task may ship `seed/<id>` (via [`crate::task::Task::seed_state_path`])
//! pointing at `seed/harness_state.json`, describing a project (and
//! optionally a paused roadmap + goal cards) to insert into the run's own
//! isolated store — the same `PERMAGENT_PATH_ROOT/spectral/permagent.db`
//! [`crate::cost::LedgerCostReader`] already reads after the run.
//!
//! ## Why direct SQL, not a Rust call into the harness
//!
//! `permagent-eval` cannot depend on `crates/goose` (package `permagent`):
//! `goose` already depends on `permagent-eval` (`program_bridge.rs` /
//! `decisions_effects.rs` / `decision_inbox/policy.rs`), so the reverse edge
//! is a cyclic package dependency Cargo refuses to build (see this crate's
//! `Cargo.toml`). There is also no `permagent seed-state <file>` (or
//! equivalent) subcommand on the harness CLI to shell out to.
//!
//! ## Why a warm-up boot, not a hand-duplicated schema
//!
//! `init_spectral_db` (`crates/goose/src/session/spectral_schema.rs`)
//! creates ~30 tables in one ~780-line function, stamping
//! `schema_version = SPECTRAL_SCHEMA_VERSION` (currently 14 — the pinned
//! FRESH-INIT baseline, not "latest": `SessionStorage::pool`
//! (`crates/goose/src/session/session_manager.rs`) runs a long, frequently
//! extended `if version < N` migration ladder past it on every subsequent
//! boot). Hand-copying that DDL here would both duplicate ~780 lines this
//! crate cannot compile-check against the source of truth AND go stale the
//! next time a migration lands — exactly the "drift" this module exists to
//! avoid.
//!
//! Instead, [`apply`] shells out to the real `permagent` binary ONCE
//! (`permagent session list`, a purely local read with no provider/model
//! calls — the same "shell out to the real binary for one side effect"
//! seam `runner::SubprocessRecipeSource` already uses for
//! `--render-recipe`) against this run's own `PERMAGENT_PATH_ROOT`. Since
//! the store does not exist yet, this is a genuinely fresh boot:
//! `SessionStorage::pool`'s `is_schema_initialized` is false, so it takes
//! the `init_spectral_db` branch ONLY (the migration ladder only runs on a
//! database that is ALREADY initialized) — the store is left at exactly the
//! `SPECTRAL_SCHEMA_VERSION` baseline. [`seed_into_connection`] then inserts
//! rows into that known, pinned, documented baseline shape via `rusqlite`
//! (already a dependency, matching `cost::LedgerCostReader`'s read side).
//!
//! The harness's OWN graded invocation that follows (a separate process,
//! its own fresh `SessionStorage`/`OnceCell`) then boots against this
//! ALREADY-initialized store for real and runs the WHOLE migration ladder
//! itself, for real, using the harness's own real code — no migration is
//! ever re-implemented here. Every migration's own doc comments describe
//! itself as idempotent / additive / "card-data-safe", so running the real
//! ladder over these seed rows is exactly as safe as running it over any
//! other real fresh install that happened to get a project and some goal
//! cards before its second boot.
//!
//! [`seed_into_connection`] asserts the warmed store's shape before writing
//! anything (`assert_schema`): both the `schema_version` row AND, more
//! specifically, that every column this module's INSERTs touch is actually
//! present (`PRAGMA table_info`) — so a future schema change that resizes
//! or renames `projects`/`project_tags`/`board_columns`/`cards` fails this
//! seed step loudly (a clear `bail!` naming the missing column) instead of
//! silently inserting rows the coordinator's own queries can never find.

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// The `SPECTRAL_SCHEMA_VERSION` fresh-init baseline
/// (`crates/goose/src/session/spectral_schema.rs`) this module's INSERTs are
/// written against. A fresh [`apply`] boot always stamps exactly this value
/// (see module docs) — a mismatch means either this constant or the
/// harness's own `SPECTRAL_SCHEMA_VERSION` has moved without the other, and
/// [`assert_schema`] refuses to seed rather than guess which shape is live.
pub const EXPECTED_FRESH_INIT_SCHEMA_VERSION: i32 = 14;

/// The goal lifecycle column layout `cards::GOAL_COLUMNS`
/// (`crates/goose/src/cards.rs`) seeds for any project using goal cards —
/// mirrored here (state bindings + display names + positions only) because
/// the harness's own lazy seeding (`cards::seed_goal_columns`, called from
/// application code this crate cannot depend on — see module docs) is not
/// reachable from this crate. [`assert_schema`] checks the live
/// `board_columns` shape before this is used, so a rename upstream is
/// caught rather than silently producing columns the real
/// `cards::get_goal_column` lookup can never find.
const GOAL_COLUMNS: &[(&str, &str, i32)] = &[
    ("triage", "Triage", 100),
    ("ready", "Ready", 101),
    ("in_progress", "In Progress", 102),
    ("review", "Review", 103),
    ("complete", "Complete", 104),
    ("cancelled", "Cancelled", 105),
    ("failed", "Failed", 106),
];

/// One fixture's `seed/harness_state.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct HarnessStateSeed {
    /// The [`EXPECTED_FRESH_INIT_SCHEMA_VERSION`] this seed's column layout
    /// was written against. Compared for exact equality by
    /// [`assert_schema`] — a fresh-init boot always stamps exactly one
    /// value, so "close enough" would mask real drift.
    pub schema_version_expected: i32,
    pub project: SeedProject,
    /// Tags the project with `roadmap_paused` (the tag
    /// `crate::projects::add_tag`/`remove_tag` and `resolve_resume_project`
    /// actually read — see `HARNESS_LOOP_LOG.md` Lane R20d) when true.
    #[serde(default)]
    pub roadmap_paused: bool,
    /// Goal cards to create, each in the named lifecycle column.
    #[serde(default)]
    pub goals: Vec<SeedGoal>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeedProject {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeedGoal {
    pub title: String,
    /// One of `GOAL_COLUMNS`' state bindings: `triage`, `ready`,
    /// `in_progress`, `review`, `complete`, `cancelled`, `failed`.
    pub state: String,
    #[serde(default)]
    pub description: String,
}

/// Parse a fixture's `seed/harness_state.json`.
pub fn load(path: &Path) -> Result<HarnessStateSeed> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading seed state {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing seed state {}", path.display()))
}

/// Warm this run's store to the harness's own fresh-init baseline, then
/// insert `seed`'s rows into it. See module docs for the full design.
///
/// `permagent_bin` is `RunConfig::permagent_bin` (the SAME binary the
/// graded invocation uses); `data_root` is the run's own
/// `PERMAGENT_PATH_ROOT` (`run_dir/data` in `runner::run_task` — the store
/// lives at `data_root/spectral/permagent.db`, matching
/// `cost::LedgerCostReader::db_path`); `workdir` is the run's seeded
/// workspace directory (`run_dir/workspace`) — the seeded project's
/// `root_path` is set to this (canonicalized), so `resolve_resume_project`'s
/// / `resolve_decompose_project`'s cwd match finds it exactly as it would a
/// real user project.
pub fn apply(
    seed: &HarnessStateSeed,
    permagent_bin: &str,
    data_root: &Path,
    workdir: &Path,
) -> Result<()> {
    warm_schema(permagent_bin, data_root)?;

    let db_path = data_root.join("spectral").join("permagent.db");
    let conn = Connection::open(&db_path)
        .with_context(|| format!("opening {} to seed harness state", db_path.display()))?;

    let canonical_workdir = workdir
        .canonicalize()
        .unwrap_or_else(|_| workdir.to_path_buf());
    let root_path = canonical_workdir.to_string_lossy().into_owned();

    seed_into_connection(&conn, seed, &root_path)
}

/// Boot the real `permagent` binary once against `data_root` with a
/// provider/model-free local subcommand, purely to materialize the fresh
/// `SPECTRAL_SCHEMA_VERSION` baseline (see module docs). NOT itself unit
/// tested (it shells out to a real binary this crate cannot fake) — see
/// [`seed_into_connection`] for the part that is.
fn warm_schema(permagent_bin: &str, data_root: &Path) -> Result<()> {
    let output = Command::new(permagent_bin)
        .args(["session", "list"])
        .env("PERMAGENT_PATH_ROOT", data_root)
        .env("PERMAGENT_DISABLE_KEYRING", "1")
        .output()
        .with_context(|| format!("running `{permagent_bin} session list` to warm the schema"))?;
    if !output.status.success() {
        bail!(
            "`{permagent_bin} session list` (schema warm-up for fixture state seeding) failed \
             (exit {:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr),
        );
    }
    Ok(())
}

/// The pure, directly-testable core: assert the connection's schema matches
/// what this module's INSERTs assume, then write `seed`'s rows.
/// `project_root_path` is the already-resolved (canonicalized) path to set
/// on the seeded project's `root_path` column.
pub fn seed_into_connection(
    conn: &Connection,
    seed: &HarnessStateSeed,
    project_root_path: &str,
) -> Result<()> {
    assert_schema(conn, seed.schema_version_expected)?;

    for goal in &seed.goals {
        if !GOAL_COLUMNS
            .iter()
            .any(|&(binding, _, _)| binding == goal.state)
        {
            let valid: Vec<&str> = GOAL_COLUMNS.iter().map(|&(b, _, _)| b).collect();
            bail!(
                "seed goal '{}' has state '{}', not one of the harness's goal lifecycle \
                 bindings {valid:?}",
                goal.title,
                goal.state,
            );
        }
    }

    // Deterministic, collision-resistant-enough IDs (one seed applies once
    // per run, into a store this run alone owns) — a real UUID is not
    // needed, only uniqueness within this store.
    let project_id = format!("eval-seed-{}", seed.project.slug);

    conn.execute(
        "INSERT INTO projects (id, user_id, slug, name, description, status, root_path)
         VALUES (?1, 'default', ?2, ?3, ?4, 'active', ?5)",
        params![
            project_id,
            seed.project.slug,
            seed.project.name,
            seed.project.description,
            project_root_path,
        ],
    )
    .context("inserting seed project")?;

    if seed.roadmap_paused {
        conn.execute(
            "INSERT INTO project_tags (project_id, tag) VALUES (?1, 'roadmap_paused')",
            params![project_id],
        )
        .context("tagging seed project roadmap_paused")?;
    }

    let mut column_ids: HashMap<&str, String> = HashMap::new();
    for &(state_binding, name, position) in GOAL_COLUMNS {
        let id = format!("{project_id}-col-{state_binding}");
        conn.execute(
            "INSERT INTO board_columns (id, project_id, name, position, column_kind, state_binding)
             VALUES (?1, ?2, ?3, ?4, 'state', ?5)",
            params![id, project_id, name, position, state_binding],
        )
        .with_context(|| format!("inserting goal column '{state_binding}'"))?;
        column_ids.insert(state_binding, id);
    }

    for (i, goal) in seed.goals.iter().enumerate() {
        let column_id = column_ids
            .get(goal.state.as_str())
            .expect("goal state already validated against GOAL_COLUMNS above");
        let card_id = format!("{project_id}-goal-{i}");
        conn.execute(
            "INSERT INTO cards \
             (id, project_id, card_type, title, description, column_id, position, created_by, metadata_json)
             VALUES (?1, ?2, 'goal', ?3, ?4, ?5, ?6, 'user', '{}')",
            params![card_id, project_id, goal.title, goal.description, column_id, i as i32],
        )
        .with_context(|| format!("inserting seed goal card '{}'", goal.title))?;
    }

    Ok(())
}

/// Fail loudly, before writing anything, if the connection's schema does not
/// match what this module's INSERTs assume — a numeric `schema_version`
/// check (catches the fresh-init baseline itself moving) AND a structural
/// `PRAGMA table_info` check on every column this module actually touches
/// (catches a rename/drop even if the version number did not move, which is
/// the more likely real-world drift: `SPECTRAL_SCHEMA_VERSION`'s own doc
/// says it is deliberately NOT bumped for most schema changes).
fn assert_schema(conn: &Connection, expected_version: i32) -> Result<()> {
    let version: i32 = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .context("reading schema_version after the warm-up boot")?;
    if version != expected_version {
        bail!(
            "schema drift: fixture state seeding expects a fresh-init spectral schema_version \
             of {expected_version}, but the warmed store reports {version} -- \
             EXPECTED_FRESH_INIT_SCHEMA_VERSION in seed_state.rs and SPECTRAL_SCHEMA_VERSION in \
             crates/goose/src/session/spectral_schema.rs have gone out of sync",
        );
    }

    for (table, cols) in [
        (
            "projects",
            &[
                "id",
                "user_id",
                "slug",
                "name",
                "description",
                "status",
                "root_path",
            ][..],
        ),
        ("project_tags", &["project_id", "tag"][..]),
        (
            "board_columns",
            &[
                "id",
                "project_id",
                "name",
                "position",
                "column_kind",
                "state_binding",
            ][..],
        ),
        (
            "cards",
            &[
                "id",
                "project_id",
                "card_type",
                "title",
                "description",
                "column_id",
                "position",
                "created_by",
                "metadata_json",
            ][..],
        ),
    ] {
        let present = table_columns(conn, table)?;
        for col in cols {
            if !present.iter().any(|c| c == col) {
                bail!(
                    "schema drift: table `{table}` no longer has a `{col}` column -- \
                     seed_state.rs's hardcoded INSERT statements need updating to match",
                );
            }
        }
    }
    Ok(())
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .with_context(|| format!("preparing PRAGMA table_info({table})"))?;
    let cols: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .with_context(|| format!("querying PRAGMA table_info({table})"))?
        .collect::<rusqlite::Result<Vec<String>>>()
        .with_context(|| format!("reading `{table}` column names"))?;
    if cols.is_empty() {
        bail!("schema drift: table `{table}` does not exist in the warmed store");
    }
    Ok(cols)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors the columns `assert_schema`/`seed_into_connection` actually
    /// touch on `projects`/`project_tags`/`board_columns`/`cards`, plus the
    /// `schema_version` stamp `init_spectral_db` writes — a deliberately
    /// MINIMAL slice of the real ~30-table baseline (see module docs on why
    /// this crate does not hand-duplicate the whole thing), just enough to
    /// unit test this module's own SQL without a real `permagent` binary.
    fn fresh_init_like_db(schema_version: i32) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE schema_version (version INTEGER PRIMARY KEY);
             INSERT INTO schema_version (version) VALUES ({schema_version});

             CREATE TABLE projects (
                 id TEXT PRIMARY KEY,
                 user_id TEXT NOT NULL DEFAULT 'default',
                 slug TEXT NOT NULL,
                 name TEXT NOT NULL,
                 description TEXT NOT NULL DEFAULT '',
                 status TEXT NOT NULL DEFAULT 'active',
                 root_path TEXT
             );

             CREATE TABLE project_tags (
                 project_id TEXT NOT NULL,
                 tag TEXT NOT NULL,
                 PRIMARY KEY (project_id, tag)
             );

             CREATE TABLE board_columns (
                 id TEXT PRIMARY KEY,
                 project_id TEXT NOT NULL,
                 name TEXT NOT NULL,
                 position INTEGER NOT NULL,
                 column_kind TEXT NOT NULL DEFAULT 'manual',
                 state_binding TEXT,
                 wip_limit INTEGER
             );

             CREATE TABLE cards (
                 id TEXT PRIMARY KEY,
                 project_id TEXT NOT NULL,
                 card_type TEXT NOT NULL DEFAULT 'standard',
                 title TEXT NOT NULL,
                 description TEXT NOT NULL DEFAULT '',
                 column_id TEXT NOT NULL,
                 position INTEGER NOT NULL DEFAULT 0,
                 created_by TEXT NOT NULL DEFAULT 'user',
                 metadata_json TEXT NOT NULL DEFAULT '{{}}'
             );"
        ))
        .unwrap();
        conn
    }

    fn roadmap_seed() -> HarnessStateSeed {
        HarnessStateSeed {
            schema_version_expected: EXPECTED_FRESH_INIT_SCHEMA_VERSION,
            project: SeedProject {
                slug: "acme-api".to_string(),
                name: "Acme API".to_string(),
                description: "".to_string(),
            },
            roadmap_paused: true,
            goals: vec![
                SeedGoal {
                    title: "Add rate limiting to the API".to_string(),
                    state: "complete".to_string(),
                    description: "".to_string(),
                },
                SeedGoal {
                    title: "Add a retry policy to the client".to_string(),
                    state: "ready".to_string(),
                    description: "".to_string(),
                },
            ],
        }
    }

    #[test]
    fn seeds_project_tag_and_goal_cards_the_real_read_paths_find() {
        let conn = fresh_init_like_db(EXPECTED_FRESH_INIT_SCHEMA_VERSION);
        seed_into_connection(&conn, &roadmap_seed(), "/work/acme-api").unwrap();

        // Mirrors `resolve_resume_project`'s cwd match + `roadmap_paused` tag
        // check (orchestrator.rs).
        let (project_id, root_path): (String, Option<String>) = conn
            .query_row(
                "SELECT id, root_path FROM projects WHERE slug = 'acme-api'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(root_path.as_deref(), Some("/work/acme-api"));

        let tagged: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM project_tags WHERE project_id = ?1 AND tag = 'roadmap_paused')",
                params![project_id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(tagged, "project must carry the roadmap_paused tag");

        // Mirrors `eligible_ready_goals`'s join (cards JOIN board_columns).
        let ready_titles: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT c.title FROM cards c JOIN board_columns bc ON c.column_id = bc.id \
                     WHERE c.project_id = ?1 AND c.card_type = 'goal' AND bc.state_binding = 'ready'",
                )
                .unwrap();
            stmt.query_map(params![project_id], |r| r.get(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<String>>>()
                .unwrap()
        };
        assert_eq!(ready_titles, vec!["Add a retry policy to the client"]);

        let complete_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM cards c JOIN board_columns bc ON c.column_id = bc.id \
                 WHERE c.project_id = ?1 AND bc.state_binding = 'complete'",
                params![project_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(complete_count, 1);
    }

    #[test]
    fn no_roadmap_paused_tag_when_seed_does_not_ask_for_one() {
        let conn = fresh_init_like_db(EXPECTED_FRESH_INIT_SCHEMA_VERSION);
        let mut seed = roadmap_seed();
        seed.roadmap_paused = false;
        seed_into_connection(&conn, &seed, "/work/acme-api").unwrap();
        let tag_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM project_tags", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tag_count, 0);
    }

    #[test]
    fn schema_version_mismatch_fails_loudly_before_writing_anything() {
        let conn = fresh_init_like_db(EXPECTED_FRESH_INIT_SCHEMA_VERSION + 1);
        let err = seed_into_connection(&conn, &roadmap_seed(), "/work/acme-api").unwrap_err();
        assert!(
            err.to_string().contains("schema drift"),
            "unexpected error: {err}"
        );
        let project_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            project_count, 0,
            "must not write any row once the version check fails"
        );
    }

    #[test]
    fn missing_column_fails_loudly_instead_of_inserting_wrong_shape() {
        let conn = fresh_init_like_db(EXPECTED_FRESH_INIT_SCHEMA_VERSION);
        conn.execute_batch(
            "ALTER TABLE board_columns RENAME COLUMN state_binding TO lifecycle_state;",
        )
        .unwrap();
        let err = seed_into_connection(&conn, &roadmap_seed(), "/work/acme-api").unwrap_err();
        assert!(
            err.to_string().contains("state_binding"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn unknown_goal_state_is_rejected() {
        let conn = fresh_init_like_db(EXPECTED_FRESH_INIT_SCHEMA_VERSION);
        let mut seed = roadmap_seed();
        seed.goals[0].state = "blocked".to_string();
        let err = seed_into_connection(&conn, &seed, "/work/acme-api").unwrap_err();
        assert!(
            err.to_string().contains("blocked"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_parses_a_real_fixture_seed_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("harness_state.json");
        std::fs::write(
            &path,
            serde_json::to_string(&serde_json::json!({
                "schema_version_expected": EXPECTED_FRESH_INIT_SCHEMA_VERSION,
                "project": {"slug": "acme-api", "name": "Acme API"},
                "roadmap_paused": true,
                "goals": [
                    {"title": "Add rate limiting to the API", "state": "complete"},
                    {"title": "Add a retry policy to the client", "state": "ready"}
                ]
            }))
            .unwrap(),
        )
        .unwrap();
        let seed = load(&path).unwrap();
        assert_eq!(seed.project.slug, "acme-api");
        assert_eq!(seed.goals.len(), 2);
        assert!(seed.roadmap_paused);
    }
}
