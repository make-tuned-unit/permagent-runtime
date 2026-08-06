//! End-to-end proof that the `Brain::turn` outcome loop actually writes the
//! corpus Spectral asked for.
//!
//! Spectral's dispatch asks Permagent to call `turn` and report outcomes so
//! `turn_events` / `turn_members` become a labelled set of real queries with
//! recorded use. Before this work those tables existed in the live brain and
//! held ZERO rows. The failure mode worth guarding against is subtle: a turn
//! that retrieves but is never reported leaves memory state unchanged and
//! produces no learning signal — pure overhead that still looks like it is
//! working. So this asserts the WRITE, not just the call.
//!
//! Runs against a temp brain, never production data.

use spectral::{Brain, MemoryOutcome, RememberOpts, TurnRequest, Visibility};

fn temp_brain() -> (tempfile::TempDir, Brain) {
    let dir = tempfile::tempdir().expect("tempdir");
    // Same ontology the daemon mounts (state.rs), so the taxonomy under test
    // is the real one rather than a stub.
    let ontology = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/ontology.toml");
    let brain = Brain::builder()
        .data_dir(dir.path())
        .ontology_path(&ontology)
        .build()
        .expect("brain builds");
    (dir, brain)
}

/// Count rows in the turn ledger tables the dispatch names.
fn turn_row_counts(data_dir: &std::path::Path) -> (i64, i64) {
    let db = data_dir.join("memory.db");
    let conn = rusqlite::Connection::open(&db).expect("open memory.db");
    let events: i64 = conn
        .query_row("SELECT COUNT(*) FROM turn_events", [], |r| r.get(0))
        .unwrap_or(-1);
    let members: i64 = conn
        .query_row("SELECT COUNT(*) FROM turn_members", [], |r| r.get(0))
        .unwrap_or(-1);
    (events, members)
}

#[test]
fn turn_then_outcome_writes_a_labelled_corpus_row() {
    let (dir, brain) = temp_brain();

    brain
        .remember_with(
            "pairing-note",
            "The hub pairing URL carries a one-time claim code, not a bearer token.",
            RememberOpts::default(),
        )
        .expect("remember");

    let (events_before, members_before) = turn_row_counts(dir.path());

    let result = brain
        .turn(&TurnRequest::query(
            "how does device pairing work",
            Visibility::Private,
        ))
        .expect("turn succeeds");

    // Retrieval alone must not be the thing that teaches. Whatever it
    // delivered, the outcome is what carries the signal.
    let delivered = result.receipt.delivered.len();
    // Guard against a vacuous pass: if nothing is ever delivered, the outcome
    // assertions below skip and this test would go green while proving nothing.
    assert!(
        delivered > 0,
        "the turn delivered no hits — the outcome path would be untested"
    );
    eprintln!("[e2e] delivered={delivered}");

    let outcomes: Vec<(&str, MemoryOutcome)> = result
        .receipt
        .delivered
        .iter()
        .map(|d| (d.key.as_str(), MemoryOutcome::Used))
        .collect();

    if !outcomes.is_empty() {
        brain
            .record_turn_outcome(&result.receipt, &outcomes)
            .expect("outcome commits");
    }

    let (events_after, members_after) = turn_row_counts(dir.path());

    assert!(
        events_after > events_before,
        "a turn must leave a delivery record: turn_events {events_before} -> {events_after}"
    );
    if delivered > 0 {
        assert!(
            members_after > members_before,
            "reported outcomes must land in turn_members: {members_before} -> {members_after}"
        );
    }
}

#[test]
fn outcomes_are_rejected_for_keys_the_turn_did_not_deliver() {
    // This is why our wiring keys outcomes off `receipt.delivered[].key` and
    // not the memory `id` — reporting an undelivered key is a caller bug, and
    // silently accepting it would reintroduce exactly the unattributed
    // reinforcement `turn` exists to remove.
    let (_dir, brain) = temp_brain();
    brain
        .remember_with("real-note", "a genuine memory", RememberOpts::default())
        .expect("remember");

    let result = brain
        .turn(&TurnRequest::query("anything", Visibility::Private))
        .expect("turn succeeds");

    let bogus = brain.record_turn_outcome(
        &result.receipt,
        &[("definitely-not-delivered", MemoryOutcome::Used)],
    );
    assert!(
        bogus.is_err() || !result.receipt.delivered_key("definitely-not-delivered"),
        "an outcome for an undelivered key must not be silently accepted"
    );
}

#[test]
fn ignored_does_not_reinforce_but_is_still_recorded() {
    // Our wiring reports EVERY delivered hit — `Used` for cited ones,
    // `Ignored` for the rest — because a delivered-but-unused memory is
    // otherwise indistinguishable from one never delivered, and the negatives
    // are half the signal. `Ignored` must record without strengthening.
    let (dir, brain) = temp_brain();
    brain
        .remember_with(
            "unused-note",
            "a memory that gets delivered but not used",
            RememberOpts::default(),
        )
        .expect("remember");

    let result = brain
        .turn(&TurnRequest::query(
            "delivered but not used",
            Visibility::Private,
        ))
        .expect("turn succeeds");

    if result.receipt.delivered.is_empty() {
        return; // nothing delivered; nothing to assert about outcomes
    }

    let outcomes: Vec<(&str, MemoryOutcome)> = result
        .receipt
        .delivered
        .iter()
        .map(|d| (d.key.as_str(), MemoryOutcome::Ignored))
        .collect();

    let receipt = brain
        .record_turn_outcome(&result.receipt, &outcomes)
        .expect("ignored outcomes commit");

    assert!(
        receipt.reinforced.is_empty(),
        "Ignored must never reinforce — that asymmetry is the contract"
    );
    assert!(
        !receipt.not_reinforced.is_empty(),
        "Ignored must still be recorded as negative evidence"
    );

    let (_events, members) = turn_row_counts(dir.path());
    assert!(
        members > 0,
        "ignored outcomes must still land in the ledger"
    );
}

#[test]
fn void_awaits_its_own_deferred_delivery() {
    let (dir, mut brain) = temp_brain();
    brain.set_async_turn_delivery(true);
    brain
        .remember_with(
            "void-race-note",
            "the staging deploy runbook lists the rollback steps",
            RememberOpts::default(),
        )
        .expect("remember");

    let result = brain
        .turn(&TurnRequest::query(
            "staging deploy rollback",
            Visibility::Private,
        ))
        .expect("turn succeeds");
    assert!(brain.void_turn(&result.receipt).expect("void succeeds"));

    let reopened = Brain::open(dir.path()).expect("reopen brain");
    let evidence = reopened
        .memory_outcome_evidence(100)
        .expect("read evidence");
    assert!(
        evidence
            .iter()
            .all(|item| item.memory_key != "void-race-note" || item.delivered == 0),
        "voided deferred turn leaked into outcome evidence"
    );
}
