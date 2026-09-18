//! Cross-crate schema-drift guard for Lane Z4's `permagent-eval graduate`
//! evidence pipeline (D1, `docs/orchestrator/
//! CODING_HARNESS_ROLE_GRADUATION_DAG.md` G2).
//!
//! `permagent-eval` (`crates/permagent-eval/src/graduate.rs`) cannot depend
//! on this crate (`permagent`) to call `cost_router::capability_directory`'s
//! API directly — `crates/goose/Cargo.toml:76` already depends on
//! `permagent-eval` (for `ProgramDag`/`ProgramNode`), so the reverse
//! dependency would be a cyclic package dependency Cargo refuses to build.
//! `graduate.rs` therefore hand-mirrors this crate's capability-directory
//! schema (`RoleEvidence`, `validate_graduation`, `MICRO_SUITE_PASS_FLOOR`,
//! `DAG_SELECTION_PASS_FLOOR`, `KB_SNAPSHOT_STALE_AFTER_DAYS`,
//! `SCHEMA_VERSION`, `default_path`, the atomic-save shape) — real
//! duplication with a real drift risk.
//!
//! THIS crate, going the other direction, already depends on
//! `permagent-eval` as an ordinary (non-dev) dependency, so it is the one
//! side that CAN see both schemas at once. This module drives
//! `permagent_eval::graduate`'s real read/compute/write path against a REAL
//! [`CapabilityDirectory`], round-trips it through this crate's real
//! [`RoleEvidence`]/[`validate_graduation`], and asserts every duplicated
//! constant stays equal. If the two schemas ever drift, this test fails on
//! the goose side — `permagent-eval`'s own test suite has no way to know.

use chrono::NaiveDate;

use super::capability_directory::{
    validate_graduation, CapabilityDirectory, RoleEvidence, DAG_SELECTION_PASS_FLOOR,
    MICRO_SUITE_PASS_FLOOR, SCHEMA_VERSION,
};
use super::knowledge::KB_SNAPSHOT_STALE_AFTER_DAYS;
use super::recommend::{AvailableModel, WorkflowRole};

fn today() -> NaiveDate {
    chrono::Utc::now().date_naive()
}

/// A fixture run report in `report::render_json`'s shape (see
/// `graduate.rs`'s module doc), with the fizzbuzz smoke solved and a 90%
/// pass rate — a qualifying run for the ORCHESTRATE role when
/// `dag_selection_pct` is also present.
fn qualifying_run_json(day: u32) -> serde_json::Value {
    serde_json::json!({
        "generated_at": format!("2026-09-0{day}T00:00:00Z"),
        "dag_selection_pct": 0.97,
        "tiers": [{
            "provider": "anthropic",
            "model": "claude-sonnet-5",
            "summary": {"attempted": 10, "solved": 9},
            "tasks": [{"task_id": "fizzbuzz", "solved": true}],
        }],
    })
}

/// D1's own duplicated-constant list (see the module doc's table in the
/// lane report), checked against this crate's real values, including
/// `DAG_SELECTION_PASS_FLOOR` — added by Lane Z-fix's G7 finding F3 fix
/// (`2ebf7d8b`) after this lane's original constant table was written, so
/// this is exactly the kind of drift this test exists to catch.
#[test]
fn duplicated_constants_stay_equal() {
    assert_eq!(
        permagent_eval::graduate::MICRO_SUITE_PASS_FLOOR,
        MICRO_SUITE_PASS_FLOOR,
        "permagent-eval's mirror of MICRO_SUITE_PASS_FLOOR (graduate.rs) has drifted from \
         capability_directory.rs's real constant"
    );
    assert_eq!(
        permagent_eval::graduate::KB_SNAPSHOT_STALE_AFTER_DAYS,
        KB_SNAPSHOT_STALE_AFTER_DAYS,
        "permagent-eval's mirror of KB_SNAPSHOT_STALE_AFTER_DAYS (graduate.rs) has drifted from \
         knowledge.rs's real constant"
    );
    assert_eq!(
        permagent_eval::graduate::SCHEMA_VERSION,
        SCHEMA_VERSION,
        "permagent-eval's mirror of SCHEMA_VERSION (graduate.rs) has drifted from \
         capability_directory.rs's real constant"
    );
    assert_eq!(
        permagent_eval::graduate::DAG_SELECTION_PASS_FLOOR,
        DAG_SELECTION_PASS_FLOOR,
        "permagent-eval's mirror of DAG_SELECTION_PASS_FLOOR (graduate.rs) has drifted from \
         capability_directory.rs's real constant"
    );
}

/// (1) Build a directory with `CapabilityDirectory::build` (well, its pure
/// core, `build_with_persisted`, from a literal available-model list — the
/// same thing `permagent packs directory --refresh` does) and save it. (2)
/// Call `permagent_eval::graduate`'s library entry point on that file with a
/// fixture run report (three consecutive qualifying runs, so `graduated`
/// actually flips — not just the trivial `false` case). (3) Reload via the
/// REAL `CapabilityDirectory::load_persisted` and assert the `RoleEvidence`
/// fields, and that the REAL `validate_graduation` accepts what
/// `graduate.rs` wrote.
#[test]
fn graduate_runs_into_directory_writes_evidence_the_real_crate_accepts() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let directory_path = tmp.path().join("capability_directory.json");

    // (1)
    let available = vec![AvailableModel::new("anthropic", "claude-sonnet-5")];
    let dir = CapabilityDirectory::build_with_persisted(&available, None);
    dir.save(&directory_path).expect("save must succeed");

    // (2)
    let mut run_paths = Vec::new();
    for day in 1..=3 {
        let run_path = tmp.path().join(format!("run{day}.json"));
        std::fs::write(
            &run_path,
            serde_json::to_vec(&qualifying_run_json(day)).expect("serialize fixture"),
        )
        .expect("write fixture run file");
        run_paths.push(run_path);
    }
    let outcome = permagent_eval::graduate::graduate_runs_into_directory(
        &permagent_eval::graduate::GraduateRequest {
            runs: &run_paths,
            provider: None,
            model: None,
            role: permagent_eval::graduate::WorkflowRole::Orchestrate,
            snapshot_date: None,
            directory_path: &directory_path,
            dry_run: false,
        },
    )
    .expect("graduate_runs_into_directory must succeed");
    assert!(
        outcome.after.graduated,
        "three consecutive qualifying runs must graduate"
    );
    assert!(outcome.written);

    // (3)
    let reloaded = CapabilityDirectory::load_persisted(&directory_path).expect(
        "a file graduate.rs wrote must be loadable by the REAL crate — this is the \
                 schema-compatibility assertion itself: a bad field name/type here fails \
                 deserialization or `#[serde(try_from = \"RoleEvidenceRaw\")]`'s guard",
    );
    let card = reloaded
        .cards
        .iter()
        .find(|c| c.provider == "anthropic" && c.model == "claude-sonnet-5")
        .expect("the graded card must exist in the reloaded, REAL directory");
    let role_evidence = card
        .role(WorkflowRole::Orchestrate)
        .expect("every role is explicit on a real card");
    assert!(role_evidence.graduated());
    assert!(role_evidence.smoke_passed());
    assert_eq!(role_evidence.evidence_run_ids().len(), 3);
    assert!((role_evidence.micro_suite_pass_pct().unwrap() - 0.9).abs() < 1e-9);
    assert!((role_evidence.dag_selection_pct().unwrap() - 0.97).abs() < 1e-9);

    // The REAL crate's own gate, asserted explicitly against the exact
    // fields `graduate.rs` wrote (deserialization already ran this once via
    // `#[serde(try_from = "RoleEvidenceRaw")]`'s guard when `load_persisted`
    // succeeded above; this makes the assertion visible rather than relying
    // on that side effect alone).
    validate_graduation(
        role_evidence.graduated(),
        role_evidence.smoke_passed(),
        role_evidence.micro_suite_pass_pct(),
        role_evidence.dag_selection_pct(),
        role_evidence.evidence_run_ids(),
        role_evidence.snapshot_date(),
        today(),
    )
    .expect("the REAL validate_graduation must accept what graduate.rs wrote");
}

/// (4) The reverse: a card the goose side marks graduated (using the REAL
/// `RoleEvidence::new`/`validate_graduation`) round-trips through
/// `graduate.rs`'s reader (`load_directory_json` +
/// `existing_role_evidence`) unchanged, field for field.
#[test]
fn a_card_the_goose_side_marks_graduated_round_trips_through_graduate_rs_unchanged() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let directory_path = tmp.path().join("capability_directory.json");

    let available = vec![AvailableModel::new("anthropic", "claude-sonnet-5")];
    let mut dir = CapabilityDirectory::build_with_persisted(&available, None);
    let real_evidence = RoleEvidence::new(
        true,
        true,
        Some(0.95),
        Some(0.98),
        vec![
            "real-run-1".to_string(),
            "real-run-2".to_string(),
            "real-run-3".to_string(),
        ],
        Some(0.03),
        Some(5.5),
        Some(today()),
        today(),
    )
    .expect("a valid graduation must construct on the real crate's own gate");
    let card = dir
        .cards
        .iter_mut()
        .find(|c| c.provider == "anthropic" && c.model == "claude-sonnet-5")
        .expect("the seeded card must exist");
    card.roles.insert("edit".to_string(), real_evidence.clone());
    dir.save(&directory_path).expect("save must succeed");

    // permagent-eval's reader, loading a file it never wrote.
    let loaded_json = permagent_eval::graduate::load_directory_json(&directory_path)
        .expect("graduate.rs's reader must load a real capability_directory.json");
    let mirrored = permagent_eval::graduate::existing_role_evidence(
        &loaded_json,
        "anthropic",
        "claude-sonnet-5",
        permagent_eval::graduate::WorkflowRole::Edit,
    )
    .expect("graduate.rs's reader must find the role the goose side wrote");

    assert_eq!(mirrored.graduated, real_evidence.graduated());
    assert_eq!(mirrored.smoke_passed, real_evidence.smoke_passed());
    assert_eq!(
        mirrored.micro_suite_pass_pct,
        real_evidence.micro_suite_pass_pct()
    );
    assert_eq!(
        mirrored.dag_selection_pct,
        real_evidence.dag_selection_pct()
    );
    assert_eq!(
        mirrored.evidence_run_ids,
        real_evidence.evidence_run_ids().to_vec()
    );
    assert_eq!(
        mirrored.observed_cost_usd,
        real_evidence.observed_cost_usd()
    );
    assert_eq!(
        mirrored.observed_latency_s,
        real_evidence.observed_latency_s()
    );
    assert_eq!(mirrored.snapshot_date, real_evidence.snapshot_date());
}
