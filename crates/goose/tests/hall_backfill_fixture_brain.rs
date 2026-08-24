//! The approval-hall backfill, exercised against a real fixture Brain.
//!
//! This reproduces the historical condition rather than faking it: the brain is
//! opened WITHOUT Permagent's approval hall rule, which is exactly the state
//! every memory written before [`permagent::hall_rules`] existed was written
//! in. Spectral's defaults file approval-shaped content under `event`. The
//! backfill then moves it to `fact`, and the assertions read the same
//! `COUNT(*)` the op reports.
//!
//! It never touches `~/.permagent` — every path is inside a `tempdir`.
//!
//! Sanctioned raw `spectral::Brain` usage: the test owns its runtime and wraps
//! the Brain in a `SafeBrain` the way `state.rs` does in production.

use permagent::brain_handle::SafeBrain;
use permagent::hall_backfill;
use spectral::{Brain, DeviceId, RememberOpts, Visibility};
use std::path::Path;
use std::sync::Arc;
use tempfile::tempdir;

fn ontology_toml() -> &'static str {
    include_str!("../assets/ontology.toml")
}

/// Content in the approval shape the hall rule keys on: a question the agent
/// asked and the answer it got back.
fn approval_content(n: usize) -> String {
    format!(
        "The user was asked: may I delete the merged worktree number {n}? They answered: yes, go ahead."
    )
}

fn count_misfiled(brain_dir: &Path) -> i64 {
    let conn = rusqlite::Connection::open_with_flags(
        brain_dir.join("memory.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .expect("open fixture memory.db");
    conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM memories WHERE {}",
            hall_backfill::MISFILED_APPROVAL_WHERE
        ),
        [],
        |row| row.get::<_, i64>(0),
    )
    .expect("count misfiled")
}

fn hall_of(brain_dir: &Path, key: &str) -> String {
    let conn = rusqlite::Connection::open_with_flags(
        brain_dir.join("memory.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .expect("open fixture memory.db");
    conn.query_row("SELECT hall FROM memories WHERE key = ?1", [key], |row| {
        row.get::<_, String>(0)
    })
    .expect("read hall")
}

/// Build a fixture brain seeded with `approvals` approval-shaped memories and
/// `decoys` memories that must NOT move, opened WITHOUT the approval hall rule.
fn seed_fixture(brain_dir: &Path, ontology_path: &Path, approvals: usize) -> Arc<Brain> {
    std::fs::write(ontology_path, ontology_toml()).expect("write ontology");

    // No `.hall_rules(...)`: Spectral's defaults apply, which is the pre-rule
    // state that produced the misfiled rows on the live brain.
    let brain = Brain::builder()
        .data_dir(brain_dir)
        .ontology_path(ontology_path)
        .device_id(DeviceId::from_descriptor("permagent-hall-backfill-test"))
        .build()
        .expect("brain open");

    for n in 0..approvals {
        brain
            .remember_with(
                &format!("approval-{n}"),
                &approval_content(n),
                RememberOpts {
                    source: Some("decision".into()),
                    device_id: Some(*brain.device_id()),
                    confidence: Some(1.0),
                    visibility: Visibility::Private,
                    ..Default::default()
                },
            )
            .expect("remember approval");
    }

    // A decoy that narrates asking without carrying an answer. The predicate is
    // anchored on both halves precisely so this stays put.
    brain
        .remember_with(
            "decoy-narration",
            "I was asked: to look into the flaky test, and I spent the afternoon on it.",
            RememberOpts {
                source: Some("chat".into()),
                device_id: Some(*brain.device_id()),
                confidence: Some(1.0),
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .expect("remember decoy");

    Arc::new(brain)
}

#[tokio::test(flavor = "multi_thread")]
async fn backfill_moves_misfiled_approvals_and_reports_honest_counts() {
    let temp = tempdir().expect("tempdir");
    let brain_dir = temp.path().join("brain");
    let backup_root = temp.path().join("backups");
    let ontology_path = temp.path().join("ontology.toml");

    let brain = seed_fixture(&brain_dir, &ontology_path, 7);
    let safe = SafeBrain::from_arc(brain);

    // Precondition: the fixture really is in the broken state. If Spectral's
    // defaults ever stop filing approvals under `event`, this fails loudly
    // rather than letting the test pass over zero rows.
    let before_observed = count_misfiled(&brain_dir);
    assert_eq!(
        before_observed, 7,
        "fixture precondition: all 7 approvals should be misfiled as `event` before the backfill"
    );

    let report = hall_backfill::run(&safe, &brain_dir, &backup_root)
        .await
        .expect("backfill run");

    assert_eq!(
        report.before, 7,
        "before-count must match what the DB showed"
    );
    assert_eq!(report.attempted, 7);
    assert_eq!(report.repaired, 7);
    assert_eq!(report.after, 0, "every misfiled approval should have moved");
    assert!(report.missed.is_empty(), "missed: {:?}", report.missed);
    assert!(report.errors.is_empty(), "errors: {:?}", report.errors);
    assert!(report.is_clean());

    // The op's own after-count must agree with an independent read.
    assert_eq!(count_misfiled(&brain_dir), 0);

    // The rows moved to `fact`, they were not merely removed from the predicate.
    assert_eq!(hall_of(&brain_dir, "approval-0"), "fact");
    assert_eq!(hall_of(&brain_dir, "approval-6"), "fact");

    // The decoy did not move.
    assert_ne!(
        hall_of(&brain_dir, "decoy-narration"),
        "fact",
        "prose that merely narrates being asked must not be swept into `fact`"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn backfill_snapshots_all_three_databases_before_writing() {
    let temp = tempdir().expect("tempdir");
    let brain_dir = temp.path().join("brain");
    let backup_root = temp.path().join("backups");
    let ontology_path = temp.path().join("ontology.toml");

    let brain = seed_fixture(&brain_dir, &ontology_path, 2);
    let safe = SafeBrain::from_arc(brain);

    let report = hall_backfill::run(&safe, &brain_dir, &backup_root)
        .await
        .expect("backfill run");

    assert_eq!(
        report.snapshots.len(),
        3,
        "all three brain databases must be snapshotted before any write"
    );
    let dbs: Vec<&str> = report.snapshots.iter().map(|s| s.db.as_str()).collect();
    assert_eq!(
        dbs,
        vec![
            "brain/memory.db",
            "brain/graph.sqlite",
            "brain/recognition.db"
        ],
        "all three brain databases, in order"
    );
    for snap in &report.snapshots {
        assert!(
            snap.integrity_ok,
            "snapshot {} failed its integrity check",
            snap.filename
        );
        assert!(snap.size_bytes > 0, "snapshot {} is empty", snap.filename);
    }

    // The snapshot files are really on disk under the backup root.
    let brain_backups = backup_root.join("brain");
    let count = std::fs::read_dir(&brain_backups)
        .expect("backup dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".db"))
        .count();
    assert_eq!(count, 3, "three snapshot files under {brain_backups:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn backfill_is_idempotent() {
    let temp = tempdir().expect("tempdir");
    let brain_dir = temp.path().join("brain");
    let backup_root = temp.path().join("backups");
    let ontology_path = temp.path().join("ontology.toml");

    let brain = seed_fixture(&brain_dir, &ontology_path, 3);
    let safe = SafeBrain::from_arc(brain);

    let first = hall_backfill::run(&safe, &brain_dir, &backup_root)
        .await
        .expect("first run");
    assert_eq!(first.before, 3);
    assert_eq!(first.repaired, 3);
    assert_eq!(first.after, 0);

    let second = hall_backfill::run(&safe, &brain_dir, &backup_root)
        .await
        .expect("second run");
    assert_eq!(second.before, 0, "a second run must find nothing to do");
    assert_eq!(second.attempted, 0);
    assert_eq!(second.repaired, 0);
    assert_eq!(second.after, 0);
    assert!(second.is_clean());

    // Still snapshots — the safety net is unconditional, not contingent on
    // there being work.
    assert_eq!(second.snapshots.len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn backfill_aborts_without_writing_when_a_snapshot_cannot_be_taken() {
    let temp = tempdir().expect("tempdir");
    let brain_dir = temp.path().join("brain");
    let backup_root = temp.path().join("backups");
    let ontology_path = temp.path().join("ontology.toml");

    let brain = seed_fixture(&brain_dir, &ontology_path, 4);
    let safe = SafeBrain::from_arc(brain);

    // Point the op at a brain directory whose databases do not exist. The
    // snapshot step must fail and nothing must be written.
    let missing = temp.path().join("no-such-brain");
    std::fs::create_dir_all(&missing).expect("mkdir");

    let err = hall_backfill::run(&safe, &missing, &backup_root)
        .await
        .expect_err("a missing source database must abort the run");
    assert!(
        err.contains("snapshot"),
        "the error should name the snapshot step, got: {err}"
    );

    // The real brain is untouched: the rows are still misfiled.
    assert_eq!(
        count_misfiled(&brain_dir),
        4,
        "no row may move when the pre-write snapshot failed"
    );
}
