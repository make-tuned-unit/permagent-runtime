//! Wing-rule behavior at the Spectral boundary.

use spectral::{Brain, Visibility};

const FIXTURE_BAIT: &str = "Reviewed the pricing strategy for next quarter and \
                            decided to trade the weekly report for a dashboard.";

fn brain_with_rules(rules: Vec<(String, String)>) -> (tempfile::TempDir, Brain) {
    let dir = tempfile::tempdir().expect("tempdir");
    let ontology = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/ontology.toml");
    let brain = Brain::builder()
        .data_dir(dir.path())
        .ontology_path(&ontology)
        .wing_rules(rules)
        .build()
        .expect("brain builds");
    (dir, brain)
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
        |row| row.get::<_, String>(0),
    )
    .expect("memory row has a wing")
}

#[test]
fn empty_rules_suppress_fixture_wings() {
    let (dir, brain) = brain_with_rules(Vec::new());
    let wing = wing_of(&brain, dir.path(), FIXTURE_BAIT);
    assert_eq!(wing, "general");
}

#[test]
fn project_rules_still_classify() {
    let rules = permagent::wing_rules::project_wing_rules(&[(
        "permagent".to_string(),
        "Permagent".to_string(),
    )]);
    assert!(!rules.is_empty(), "generator produced no rule");
    let (dir, brain) = brain_with_rules(rules);
    let wing = wing_of(
        &brain,
        dir.path(),
        "Shipped the Permagent splash rework today.",
    );
    assert_eq!(wing, "permagent");
}
