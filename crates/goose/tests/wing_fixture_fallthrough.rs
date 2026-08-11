//! Proof that an EMPTY wing-rule set suppresses Spectral's fixture wings.
//!
//! Spectral's `Brain` resolves `config.wing_rules.unwrap_or_else(
//! default_wing_rule_strings)`. So declining to set wing rules does not mean
//! "classify nothing" — it means classify with the FIXTURE rules
//! (alice/apollo/acme/polaris/vega/infra/travel/charity), whose patterns are
//! broad enough to swallow ordinary text: `apollo|polymarket|strategy|weather|
//! prediction|wager|trade`.
//!
//! That is not hypothetical. 118 memories in the live brain sit in fixture
//! wings, and Spectral caught a fresh production write landing in `acme` on
//! 2026-08-04 — at the rev this repo pins. The cause on our side was
//! `state.rs` guarding the builder call with `if !rules.is_empty()`, combined
//! with `spectral-recognition` not being a default feature: in the shipping
//! daemon the rule set is ALWAYS empty, so the guard meant the live brain ran
//! on fixtures permanently.
//!
//! Wing labels are a double lever — recognition-validation ground truth AND
//! the TACT retrieval gate — so a wrong label is worse than an absent one.
//! These tests pin both halves of that claim.
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

/// The failure being guarded: passing NO rules hands the brain the fixtures.
/// If this ever stops holding, Spectral changed the default and the guard in
/// `state.rs` can be revisited — until then it is the reason that guard exists.
#[test]
fn absent_rules_fall_through_to_spectral_fixture_wings() {
    let (dir, brain) = brain_with_rules(None);
    let wing = wing_of(&brain, dir.path(), FIXTURE_BAIT);
    assert_ne!(
        wing, "general",
        "expected fixture capture with no rules set; got {wing:?}. If Spectral \
         removed the fixture defaults this test should be retired, not relaxed."
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
