//! Proof that no memory can be captured by a fixture wing.
//!
//! **History.** Spectral's `Brain` resolves `config.wing_rules.unwrap_or_else(
//! default_wing_rule_strings)`, and those defaults used to be the
//! example-scenario fixtures (alice/apollo/acme/polaris/vega/infra/travel/
//! charity), whose patterns were broad enough to swallow ordinary text:
//! `apollo|polymarket|strategy|weather|prediction|wager|trade`. So declining to
//! set wing rules did not mean "classify nothing" — it meant classify with the
//! fixtures.
//!
//! That was not hypothetical. In the live brain the fixtures had captured 46
//! memories into `apollo`, 18 into `alice`, 17 into `acme` and 16 into
//! `polaris` by keyword collision, and Spectral caught a fresh production write
//! landing in `acme` on 2026-08-04. The cause on our side was `state.rs`
//! guarding the builder call with `if !rules.is_empty()`, combined with
//! `spectral-recognition` not being a default feature: in the shipping daemon
//! the rule set was ALWAYS empty, so the guard meant the live brain ran on
//! fixtures permanently. Both halves were closed here — the builder call is
//! unconditional, and the daemon enables `spectral-recognition` by default.
//!
//! **As of pin 7025328, the other half is closed upstream too:** Spectral's
//! `default_wing_rule_pairs` is now deliberately empty, on the reasoning that a
//! wing is consumer domain knowledge the library cannot invent. With no rules,
//! `classify_wing` returns `"general"`.
//!
//! So the first test below is inverted rather than deleted. It used to assert
//! the hazard EXISTS (justifying our guard); it now asserts the hazard is GONE
//! and stays gone. If Spectral ever reintroduces defaults that capture ordinary
//! prose, this fails — which is the same protection, the right way round.
//!
//! Wing labels are a double lever — recognition-validation ground truth AND
//! the TACT retrieval gate — so a wrong label is worse than an absent one.
//! These tests pin every half of that claim.
//!
//! Runs against a temp brain, never production data.

use spectral::{Brain, Visibility};

/// Text that the fixture rules capture and that a real user could plausibly
/// write. "strategy" alone is enough to land in `apollo`.
const FIXTURE_BAIT: &str = "Reviewed the pricing strategy for next quarter and \
                            decided to trade the weekly report for a dashboard.";

fn brain_with_rules(rules: Option<Vec<(String, String)>>) -> (tempfile::TempDir, Brain) {
    let dir = tempfile::tempdir().expect("tempdir");
    let ontology = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/ontology.toml");
    let mut builder = Brain::builder()
        .data_dir(dir.path())
        .ontology_path(&ontology);
    if let Some(rules) = rules {
        builder = builder.wing_rules(rules);
    }
    (dir, builder.build().expect("brain builds"))
}

fn wing_of(brain: &Brain, dir: &std::path::Path, body: &str) -> String {
    let key = "wing-fallthrough-probe";
    brain
        .remember(key, body, Visibility::Private)
        .expect("remember");
    let conn = rusqlite::Connection::open(dir.join("memory.db")).expect("open memory.db");
    conn.query_row(
        "SELECT wing FROM memories WHERE key = ?1 ORDER BY rowid DESC LIMIT 1",
        [key],
        |r| r.get::<_, String>(0),
    )
    .expect("memory row has a wing")
}

/// Passing NO rules must no longer hand the brain a fixture taxonomy.
///
/// This is the upstream half of the fix. `FIXTURE_BAIT` is text the old
/// defaults captured — "strategy" alone was enough to land it in `apollo` —
/// so if this ever returns anything but `general`, Spectral has reintroduced
/// defaults that swallow ordinary prose and the `state.rs` reasoning needs
/// revisiting.
#[test]
fn absent_rules_no_longer_fall_through_to_fixture_wings() {
    let (dir, brain) = brain_with_rules(None);
    let wing = wing_of(&brain, dir.path(), FIXTURE_BAIT);
    assert_eq!(
        wing, "general",
        "no rules must mean no taxonomy, not a fixture taxonomy; got {wing:?}"
    );
}

/// The library's own default list is empty, checked directly rather than only
/// through a classification. The behavioural test above would also pass if the
/// defaults were non-empty but simply stopped matching `FIXTURE_BAIT`; this
/// pins the actual contract.
#[test]
fn spectral_ships_no_default_wing_rules() {
    let defaults = spectral::ingest::default_wing_rule_strings();
    assert!(
        defaults.is_empty(),
        "Spectral should ship no wing taxonomy of its own; got {defaults:?}"
    );
}

/// The fix: an empty rule set is still a rule set. Nothing matches, so ordinary
/// content lands in `general` instead of being mislabelled `apollo`/`acme`.
#[test]
fn empty_rules_suppress_fixture_wings() {
    let (dir, brain) = brain_with_rules(Some(Vec::new()));
    let wing = wing_of(&brain, dir.path(), FIXTURE_BAIT);
    assert_eq!(
        wing, "general",
        "an empty wing-rule set must classify to general, not a fixture wing"
    );
}

/// And real project rules still win — the empty case is a floor, not a ceiling.
#[test]
fn project_rules_still_classify() {
    let rules = permagent::wing_rules::project_wing_rules(&[(
        "permagent".to_string(),
        "Permagent".to_string(),
    )]);
    assert!(!rules.is_empty(), "generator produced no rule");
    let (dir, brain) = brain_with_rules(Some(rules));
    let wing = wing_of(
        &brain,
        dir.path(),
        "Shipped the Permagent splash rework today.",
    );
    assert_eq!(wing, "permagent");
}
