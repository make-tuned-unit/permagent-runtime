//! Per-project wing rules — generated from the project registry.
//!
//! Wing labels are a DOUBLE LEVER: they are the recognition-validation ground
//! truth (verdicts are judged against them) AND the gate on Spectral's TACT
//! fast path. Today the Brain is opened with `wing_rules: None`, which falls
//! back to Spectral's 8 demo regexes (alice/apollo/acme/…) — on the live
//! brain that dumps ~45% of memories into the "general" wing.
//!
//! This module derives `(regex_pattern, wing_slug)` pairs from the `projects`
//! table instead: we KNOW the project names, so content mentioning a project
//! classifies into that project's wing. The wing value is the project slug —
//! the same label `activity::ingestion::derive_wing_slug` stamps as a wing
//! override at ambient-write time, so classifier-derived and override-derived
//! wings agree.
//!
//! Spectral semantics (pinned rev): rules are tried in order against the
//! lowercased `key + content + category`, first match wins, no match →
//! "general". Passing `Some(rules)` REPLACES the demo defaults entirely —
//! deliberate: those defaults misclassify real content on generic words
//! ("strategy", "trade", "cook").
//!
//! The generator is pure and always compiled (unit-testable under default
//! features); the production wiring in `goose-server/src/state.rs` is gated
//! behind the `spectral-recognition` feature, so a default build still passes
//! `None`.

use sqlx::{Pool, Sqlite};
use tracing::{debug, warn};

/// Minimum token length kept in a pattern. Single characters ("a", "x") match
/// far too much; two-letter tokens are kept because real project names have
/// them ("Go", "AI").
const MIN_TOKEN_LEN: usize = 2;

/// Separator class between name tokens: matches "atlas atlantic",
/// "atlas-atlantic", "atlas_atlantic", "atlas.atlantic" AND the collapsed
/// "atlasatlantic" (zero-or-more).
const SEP: &str = r"[\s._-]*";

/// Build `(regex_pattern, wing_slug)` rules from `(slug, name)` project rows.
///
/// * the implicit "personal" project is skipped — its name would swallow any
///   content containing the word "personal"; unscoped content should keep
///   falling through to the classifier default instead;
/// * each project contributes one rule whose pattern is an alternation of the
///   slug variant and the display-name variant (deduped);
/// * rules are ordered most-specific-first (token count, then pattern length)
///   so "permagent-runtime" outranks "permagent" under first-match-wins.
pub fn project_wing_rules(projects: &[(String, String)]) -> Vec<(String, String)> {
    let mut rules: Vec<(String, String)> = Vec::new();

    for (slug, name) in projects {
        if slug == "personal" {
            continue;
        }
        let mut variants: Vec<String> = Vec::new();
        for candidate in [slug.as_str(), name.as_str()] {
            if let Some(p) = tokens_to_pattern(candidate) {
                if !variants.contains(&p) {
                    variants.push(p);
                }
            }
        }
        if variants.is_empty() {
            warn!(
                target: "permagent::wing_rules",
                slug = %slug,
                "project produced no usable wing pattern — skipped"
            );
            continue;
        }
        let pattern = if variants.len() == 1 {
            variants.remove(0)
        } else {
            format!("(?:{})", variants.join("|"))
        };
        rules.push((pattern, slug.clone()));
    }

    // Most-specific-first: more tokens (approximated by separator count in the
    // slug), then longer pattern. Stable for equal keys.
    rules.sort_by(|a, b| {
        let key = |r: &(String, String)| (r.1.matches(['-', '_']).count(), r.0.len());
        key(b).cmp(&key(a))
    });
    rules
}

/// Lowercase, split on whitespace and separator punctuation, escape each
/// token, and rejoin with the flexible separator class. Returns `None` when
/// nothing survives (e.g. a name that is all punctuation).
fn tokens_to_pattern(raw: &str) -> Option<String> {
    let tokens: Vec<String> = raw
        .to_lowercase()
        .split(|c: char| c.is_whitespace() || c == '-' || c == '_' || c == '.')
        .filter(|t| t.len() >= MIN_TOKEN_LEN)
        .map(regex::escape)
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(SEP))
    }
}

/// Load all projects (any status — archived projects' history still deserves
/// correct wings) and generate rules. Best-effort: any error degrades to an
/// empty vec, which callers treat as "pass None / keep defaults".
pub async fn load_project_wing_rules(pool: &Pool<Sqlite>) -> Vec<(String, String)> {
    let rows: Vec<(String, String)> =
        match sqlx::query_as("SELECT slug, name FROM projects ORDER BY slug")
            .fetch_all(pool)
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                warn!(
                    target: "permagent::wing_rules",
                    error = %e,
                    "failed to load project registry — falling back to default wing rules"
                );
                return Vec::new();
            }
        };

    let rules = project_wing_rules(&rows);
    debug!(
        target: "permagent::wing_rules",
        projects = rows.len(),
        rules = rules.len(),
        "generated per-project wing rules"
    );
    rules
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(slug: &str, name: &str) -> (String, String) {
        (slug.to_string(), name.to_string())
    }

    fn rules_for(projects: &[(String, String)]) -> Vec<(String, String)> {
        project_wing_rules(projects)
    }

    fn classify<'a>(rules: &'a [(String, String)], text: &str) -> Option<&'a str> {
        let text = text.to_lowercase();
        for (pattern, wing) in rules {
            if regex::Regex::new(pattern).unwrap().is_match(&text) {
                return Some(wing);
            }
        }
        None
    }

    #[test]
    fn patterns_compile_and_match_name_and_slug_forms() {
        let rules = rules_for(&[p("atlas-atlantic", "Atlas Atlantic")]);
        assert_eq!(rules.len(), 1);
        for text in [
            "meeting notes for Atlas Atlantic",
            "deployed atlas-atlantic today",
            "the atlasatlantic listener wedged",
            "atlas_atlantic config",
        ] {
            assert_eq!(classify(&rules, text), Some("atlas-atlantic"), "{text}");
        }
        assert_eq!(classify(&rules, "unrelated grocery list"), None);
    }

    #[test]
    fn slug_and_divergent_name_both_match() {
        // Real case: slug and display name share no tokens beyond the first.
        let rules = rules_for(&[p("grocery-savings-planner", "Grocery Savers")]);
        assert_eq!(
            classify(&rules, "opened grocery-savings-planner"),
            Some("grocery-savings-planner")
        );
        assert_eq!(
            classify(&rules, "Grocery Savers signup flow"),
            Some("grocery-savings-planner")
        );
    }

    #[test]
    fn more_specific_project_wins_first_match() {
        let rules = rules_for(&[
            p("permagent", "Permagent"),
            p("permagent-runtime", "Permagent Runtime"),
        ]);
        // permagent-runtime must sort before its prefix project.
        assert_eq!(
            classify(&rules, "fixed a bug in permagent-runtime"),
            Some("permagent-runtime")
        );
        assert_eq!(
            classify(&rules, "the permagent daemon restarted"),
            Some("permagent")
        );
    }

    #[test]
    fn personal_project_is_skipped() {
        let rules = rules_for(&[p("personal", "Personal"), p("plekk", "Plekk")]);
        assert_eq!(rules.len(), 1);
        assert_eq!(classify(&rules, "a personal note"), None);
        assert_eq!(classify(&rules, "plekk onboarding"), Some("plekk"));
    }

    #[test]
    fn regex_metacharacters_in_names_are_escaped() {
        let rules = rules_for(&[p(
            "harborview-residence-association",
            "Harbourview Residents' Association",
        )]);
        // Must compile and match despite the apostrophe.
        assert_eq!(
            classify(&rules, "Harbourview Residents' Association AGM"),
            Some("harborview-residence-association")
        );
        // A name with actual metacharacters must not panic the generator and
        // must match literally (escaped), via either variant.
        let weird = rules_for(&[p("cpp", "C++ (native)")]);
        assert_eq!(classify(&weird, "cpp toolchain broke"), Some("cpp"));
        assert_eq!(classify(&weird, "the C++ (native) build"), Some("cpp"));
    }

    #[test]
    fn empty_registry_yields_no_rules() {
        assert!(rules_for(&[]).is_empty());
        // Callers treat empty as "pass None" — asserted at the state.rs seam.
    }
}
