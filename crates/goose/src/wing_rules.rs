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
///
/// Public because the write-time corroboration check in
/// [`crate::session_wing`] must be able to ask "does this turn name *this one*
/// project?" for the display name and the slug SEPARATELY — the generated
/// rule set above fuses both variants into one alternation, which answers
/// "which project" but not "by which spelling". Two callers, one tokenizer:
/// a second implementation is how a corroboration check and the classifier it
/// is supposed to agree with quietly diverge.
pub fn tokens_to_pattern(raw: &str) -> Option<String> {
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

/// The `(slug, name)` project registry, in rule order.
///
/// Split out of [`load_project_wing_rules`] because the write-time
/// corroboration check and the backfill need the ROWS (they match a specific
/// project's name and slug separately), not the fused rule strings. Best-effort:
/// any error degrades to an empty vec, which every caller reads as "we know of
/// no projects", i.e. nothing can be corroborated — the safe direction.
pub async fn load_project_rows(pool: &Pool<Sqlite>) -> Vec<(String, String)> {
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
            Vec::new()
        }
    }
}

/// Load all projects (any status — archived projects' history still deserves
/// correct wings) and generate rules. Best-effort: any error degrades to an
/// empty vec, which callers treat as "pass None / keep defaults".
pub async fn load_project_wing_rules(pool: &Pool<Sqlite>) -> Vec<(String, String)> {
    let rows = load_project_rows(pool).await;
    let rules = project_wing_rules(&rows);
    debug!(
        target: "permagent::wing_rules",
        projects = rows.len(),
        rules = rules.len(),
        "generated per-project wing rules"
    );
    rules
}

/// [`tokens_to_pattern`], anchored to whole tokens.
///
/// Without anchoring, `permagent` matches inside `permagent-runtime` — and
/// those are two of the largest real wings (the marketing site and this
/// codebase). Anchoring alone does not settle it either, because `\b` sits
/// happily against the hyphen; the caller must ALSO take the longest match.
/// This function provides the first half. See
/// [`crate::session_wing::WingCorroborator`] for the second.
///
/// The boundary is added only where the adjacent character is a word
/// character. A project called `C++ (native)` ends in punctuation, and `\b`
/// after `+` would never match — an anchor that silently makes a project
/// unrecognisable is worse than no anchor for that project.
pub fn bounded_token_pattern(raw: &str) -> Option<String> {
    let body = tokens_to_pattern(raw)?;
    let lowered = raw.to_lowercase();
    let tokens: Vec<&str> = lowered
        .split(|c: char| c.is_whitespace() || c == '-' || c == '_' || c == '.')
        .filter(|t| t.len() >= MIN_TOKEN_LEN)
        .collect();
    let first = tokens.first()?.chars().next()?;
    let last = tokens.last()?.chars().last()?;

    let mut out = String::new();
    if first.is_alphanumeric() || first == '_' {
        out.push_str(r"\b");
    }
    out.push_str(&body);
    if last.is_alphanumeric() || last == '_' {
        out.push_str(r"\b");
    }
    Some(out)
}

/// Project wing rules compiled once, matched many times.
///
/// [`project_wing_rules`] returns patterns as strings because that is the shape
/// `BrainConfig::wing_rules` wants. Anything that evaluates them locally — the
/// write-time corroboration check, the backfill sweep over a thousand memories —
/// would otherwise recompile every regex for every row. Compile once here and
/// keep Spectral's semantics exactly: lowercased haystack, rules tried in the
/// order [`project_wing_rules`] produced (most specific first), FIRST MATCH
/// WINS, no match means no project was named.
pub struct CompiledWingRules {
    rules: Vec<(regex::Regex, String)>,
}

impl CompiledWingRules {
    /// Compile `(pattern, wing)` pairs, dropping any pattern that does not
    /// compile rather than failing the whole set — a single malformed project
    /// name must not blind the matcher to the other twenty-one. A dropped rule
    /// is warned about, not swallowed silently.
    pub fn compile(rules: &[(String, String)]) -> Self {
        let mut compiled = Vec::with_capacity(rules.len());
        for (pattern, wing) in rules {
            match regex::Regex::new(pattern) {
                Ok(re) => compiled.push((re, wing.clone())),
                Err(e) => warn!(
                    target: "permagent::wing_rules",
                    wing = %wing,
                    pattern = %pattern,
                    error = %e,
                    "wing rule failed to compile — that project cannot be recognised in text"
                ),
            }
        }
        Self { rules: compiled }
    }

    /// Build straight from the `(slug, name)` project registry.
    pub fn from_projects(projects: &[(String, String)]) -> Self {
        Self::compile(&project_wing_rules(projects))
    }

    /// The wing of the first project named in `text`, or `None` when the text
    /// names no known project. `text` is lowercased here so callers cannot
    /// forget to.
    pub fn first_match(&self, text: &str) -> Option<&str> {
        let text = text.to_lowercase();
        self.rules
            .iter()
            .find(|(re, _)| re.is_match(&text))
            .map(|(_, wing)| wing.as_str())
    }

    /// Number of compiled rules — the honest count after any drop above.
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
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

    #[test]
    fn compiled_rules_agree_with_the_string_rules_they_came_from() {
        let projects = [p("permagent", "Permagent"), p("plekk", "Plekk")];
        let compiled = CompiledWingRules::from_projects(&projects);
        assert_eq!(compiled.len(), 2);
        assert_eq!(
            compiled.first_match("the Permagent daemon"),
            Some("permagent")
        );
        assert_eq!(compiled.first_match("PLEKK onboarding"), Some("plekk"));
        assert_eq!(compiled.first_match("a grocery list"), None);
    }

    #[test]
    fn compiled_rules_keep_first_match_wins_ordering() {
        let projects = [
            p("permagent", "Permagent"),
            p("permagent-runtime", "Permagent Runtime"),
        ];
        let compiled = CompiledWingRules::from_projects(&projects);
        assert_eq!(
            compiled.first_match("bug in permagent-runtime"),
            Some("permagent-runtime")
        );
    }

    #[test]
    fn an_uncompilable_rule_is_dropped_not_fatal() {
        // A pattern that cannot compile must cost only its own project.
        let compiled = CompiledWingRules::compile(&[
            ("(unclosed".to_string(), "broken".to_string()),
            ("plekk".to_string(), "plekk".to_string()),
        ]);
        assert_eq!(compiled.len(), 1);
        assert_eq!(compiled.first_match("plekk onboarding"), Some("plekk"));
    }

    #[test]
    fn bounded_patterns_anchor_on_word_characters_only() {
        let p = bounded_token_pattern("Permagent").unwrap();
        assert!(p.starts_with(r"\b") && p.ends_with(r"\b"));
        let re = regex::Regex::new(&p).unwrap();
        assert!(re.is_match("the permagent daemon"));
        assert!(
            !re.is_match("superpermagentish"),
            "a whole-token match must not fire inside a longer word"
        );
    }

    #[test]
    fn a_name_ending_in_punctuation_is_not_anchored_into_uselessness() {
        // `\b` after `+` can never match, so anchoring there would make the
        // project unrecognisable rather than precise.
        let p = bounded_token_pattern("C++ (native)").unwrap();
        // Patterns are lowercased by the generator, so callers lowercase the
        // haystack — `CompiledWingRules::first_match` and
        // `session_wing::WingCorroborator::verdict` both do.
        assert!(regex::Regex::new(&p)
            .unwrap()
            .is_match(&"the C++ (native) build".to_lowercase()));
    }
}
