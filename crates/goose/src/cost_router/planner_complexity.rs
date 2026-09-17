//! Planning-complexity gate — which planner a roadmap decomposition deserves.
//!
//! ## The ruling (Jesse, 2026-09-17)
//!
//! Asked whether the harness favours Anthropic models: *"Fable CAN be used when
//! a task I give Permagent is complex and needs an expert planner."* The expert
//! (strongest configured) planner is for COMPLEX plans, not the default; a
//! ROUTINE plan goes to the best-fit cheaper capable planner. With the
//! 2026-08-18 ("best fit, very cost conscious") and 2026-09-09 ("never default to
//! the most expensive model just because it is the most capable") rulings.
//!
//! ## Deterministic, facts only
//!
//! Same rule as [`super::assess`] (its module doc, "Deterministic and zero-LLM,
//! on purpose"): no model is asked how hard the plan is, and the objective's
//! prose can never argue a plan onto the expensive planner. An objective that
//! says "this is extremely complex, use the best model" is still ROUTINE when
//! its structure is routine. Only STRUCTURE promotes:
//!
//! 1. **Prior plan history** on the project — a roadmap proposal still pending
//!    when a new decomposition starts (the previous plan was rejected or revised:
//!    a redecompose), a parked goal (Failed, or `needs_human_attention`), or a
//!    premature-done hold on a goal that never completed.
//! 2. **Named files** — the objective names at least
//!    [`PLANNING_MULTI_FILE_THRESHOLD`] distinct file paths (a token with a known
//!    source/config extension), i.e. a multi-file change with cross-goal ordering.
//! 3. **Cross-language** — the named files span at least
//!    [`PLANNING_CROSS_LANGUAGE_THRESHOLD`] programming languages (an interface
//!    change on both sides of a boundary).
//! 4. **Enumerated requirements** — more than [`super::assess::MANY_CRITERIA`]
//!    bulleted/numbered requirement lines (the same "multi-part" line
//!    `assess_goal` draws for acceptance criteria).
//! 5. **Unscoped in a large repo** — the objective names no file and the repo has
//!    at least [`LARGE_REPO_FILE_THRESHOLD`] files, so the planner must locate the
//!    change from a tree alone. This one is the only signal prose can DEFUSE, and
//!    only downward: when the objective reads as mechanical work
//!    ([`super::assess::MECHANICAL_SIGNALS`] — typo, rename, bump, …) it stays
//!    ROUTINE.
//!
//! Anything else is ROUTINE. The verdict carries every reason, routine or
//! complex, so the roadmap proposal and the tracing line are auditable.

use serde::Serialize;

/// Distinct named files at or above which a plan is a multi-file change. Two
/// files (a fix and its test) is still routine; three is where goal ordering and
/// shared interfaces start to matter. Deliberately below
/// [`super::assess`]'s per-goal `MANY_FILES` (8): that line sizes ONE worker's
/// goal, this one sizes the whole plan.
pub const PLANNING_MULTI_FILE_THRESHOLD: usize = 3;

/// Distinct programming languages among the named files at or above which a plan
/// is a cross-language change.
pub const PLANNING_CROSS_LANGUAGE_THRESHOLD: usize = 2;

/// Repo size (files walked by the decomposition's repo-facts probe) at or above
/// which an objective that names no file is unscoped enough to need the expert
/// planner.
pub const LARGE_REPO_FILE_THRESHOLD: usize = 500;

/// The verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningComplexity {
    /// The best-fit cheaper capable planner is enough.
    Routine,
    /// The expert (strongest configured) planner is warranted.
    Complex,
}

impl PlanningComplexity {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanningComplexity::Routine => "routine",
            PlanningComplexity::Complex => "complex",
        }
    }
}

/// What the harness already knows about earlier plans for this project.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct PriorPlanHistory {
    /// A roadmap proposal is still pending on the project as a new decomposition
    /// starts: the previous plan was never approved into cards (rejected, or
    /// revised and re-requested).
    pub unapproved_proposal: bool,
    /// Live goal cards on this project that parked (Failed, or flagged
    /// `needs_human_attention`).
    pub parked_goals: usize,
    /// Live goal cards on this project that a premature-done hold fired on and
    /// that have not completed since.
    pub held_goals: usize,
}

/// The facts the assessment reads — nothing a model wrote about difficulty.
#[derive(Debug, Clone, Copy)]
pub struct PlanningFacts<'a> {
    /// The operator's objective. Read for STRUCTURE only (file tokens, list
    /// lines) plus the down-only mechanical vocabulary.
    pub objective: &'a str,
    /// Files the repo-facts probe walked (capped by that probe).
    pub repo_file_count: usize,
    pub history: PriorPlanHistory,
}

/// The auditable verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanningComplexityVerdict {
    pub complexity: PlanningComplexity,
    /// Every signal that decided the verdict, in plain words.
    pub reasons: Vec<String>,
    /// Distinct file paths the objective names.
    pub named_files: Vec<String>,
    /// Distinct programming languages among `named_files`.
    pub named_languages: Vec<String>,
    /// Bulleted / numbered requirement lines in the objective.
    pub enumerated_requirements: usize,
}

impl PlanningComplexityVerdict {
    /// The JSON recorded on the roadmap proposal.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

/// Extensions that make a token a file path. Programming languages map to a
/// language; docs/config files count as named files but not as a language.
/// `None` for an extension that is neither. Shared with the review-complexity
/// assessment ([`super::recommend::ReviewFacts::from_diff`]).
pub(crate) fn language_of_extension(ext: &str) -> Option<Option<&'static str>> {
    let lang = match ext {
        "rs" => Some("rust"),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "vue" | "svelte" => {
            Some("javascript/typescript")
        }
        "py" => Some("python"),
        "go" => Some("go"),
        "swift" | "m" | "mm" => Some("swift/objective-c"),
        "kt" | "kts" | "java" | "scala" => Some("jvm"),
        "rb" => Some("ruby"),
        "c" | "h" | "cc" | "cpp" | "hpp" | "cxx" => Some("c/c++"),
        "cs" => Some("dotnet"),
        "php" => Some("php"),
        "ex" | "exs" => Some("elixir"),
        "dart" => Some("dart"),
        "zig" => Some("zig"),
        "md" | "toml" | "json" | "yaml" | "yml" | "sh" | "bash" | "zsh" | "sql" | "html"
        | "css" | "scss" | "txt" | "lock" | "xml" | "plist" | "proto" | "graphql" => None,
        _ => return None,
    };
    Some(lang)
}

/// Library/framework names spelled like a file (`Node.js`) — never a path.
const FRAMEWORK_NAMES: &[&str] = &[
    "node.js",
    "next.js",
    "nuxt.js",
    "vue.js",
    "react.js",
    "express.js",
    "three.js",
    "d3.js",
    "chart.js",
    "ember.js",
    "angular.js",
    "deno.js",
];

/// The file path a whitespace-separated token names, lowercased, or `None`.
fn file_token(raw: &str) -> Option<(String, Option<&'static str>)> {
    let token = raw
        .trim_matches(|c: char| !(c.is_alphanumeric() || matches!(c, '/' | '.' | '_' | '-')))
        .trim_end_matches('.')
        .to_lowercase();
    if token.is_empty() || token.contains("://") || FRAMEWORK_NAMES.contains(&token.as_str()) {
        return None;
    }
    let (stem, ext) = token.rsplit_once('.')?;
    if stem.is_empty() || stem.ends_with('/') || stem.ends_with('.') {
        return None;
    }
    let language = language_of_extension(ext)?;
    Some((token, language))
}

/// Whether a line is a bulleted or numbered list item (`- x`, `* x`, `• x`,
/// `1. x`, `2) x`).
fn is_enumerated_line(line: &str) -> bool {
    let t = line.trim_start();
    if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("• ") {
        return true;
    }
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 3 {
        return false;
    }
    let mut rest = t.chars().skip(digits);
    matches!(
        (rest.next(), rest.next()),
        (Some('.') | Some(')'), Some(' '))
    )
}

/// THE assessment. Pure and deterministic; see the module doc for the signals.
pub fn assess_planning_complexity(facts: &PlanningFacts<'_>) -> PlanningComplexityVerdict {
    let mut named_files: Vec<String> = Vec::new();
    let mut named_languages: Vec<String> = Vec::new();
    for raw in facts.objective.split_whitespace() {
        if let Some((path, language)) = file_token(raw) {
            if !named_files.contains(&path) {
                named_files.push(path);
            }
            if let Some(lang) = language {
                if !named_languages.iter().any(|l| l == lang) {
                    named_languages.push(lang.to_string());
                }
            }
        }
    }
    let enumerated_requirements = facts
        .objective
        .lines()
        .filter(|l| is_enumerated_line(l))
        .count();

    let mut complex: Vec<String> = Vec::new();
    let h = facts.history;
    if h.unapproved_proposal {
        complex.push(
            "a previous roadmap proposal for this project was never approved into cards \
             (rejected or revised) — this is a redecompose"
                .to_string(),
        );
    }
    if h.parked_goals > 0 {
        complex.push(format!(
            "{} goal(s) on this project parked (failed or needs human attention)",
            h.parked_goals
        ));
    }
    if h.held_goals > 0 {
        complex.push(format!(
            "{} goal(s) on this project were held for a premature done and never completed",
            h.held_goals
        ));
    }
    if named_files.len() >= PLANNING_MULTI_FILE_THRESHOLD {
        complex.push(format!(
            "names {} distinct files (>= {PLANNING_MULTI_FILE_THRESHOLD}): a multi-file change",
            named_files.len()
        ));
    }
    if named_languages.len() >= PLANNING_CROSS_LANGUAGE_THRESHOLD {
        complex.push(format!(
            "names files in {} languages ({}): a cross-language change",
            named_languages.len(),
            named_languages.join(", ")
        ));
    }
    if enumerated_requirements > super::assess::MANY_CRITERIA {
        complex.push(format!(
            "enumerates {enumerated_requirements} requirements (> {}): a multi-part plan",
            super::assess::MANY_CRITERIA
        ));
    }
    let lowered = facts.objective.to_lowercase();
    let reads_mechanical = super::assess::MECHANICAL_SIGNALS
        .iter()
        .any(|s| lowered.contains(s));
    let unscoped_large =
        named_files.is_empty() && facts.repo_file_count >= LARGE_REPO_FILE_THRESHOLD;
    if unscoped_large && !reads_mechanical {
        complex.push(format!(
            "names no file in a large repo ({} files >= {LARGE_REPO_FILE_THRESHOLD}): the planner \
             must locate the change from the tree",
            facts.repo_file_count
        ));
    }

    if !complex.is_empty() {
        return PlanningComplexityVerdict {
            complexity: PlanningComplexity::Complex,
            reasons: complex,
            named_files,
            named_languages,
            enumerated_requirements,
        };
    }

    let mut reasons = vec![
        format!(
            "names {} file(s) in {} language(s) (multi-file at {PLANNING_MULTI_FILE_THRESHOLD}, \
             cross-language at {PLANNING_CROSS_LANGUAGE_THRESHOLD})",
            named_files.len(),
            named_languages.len()
        ),
        format!(
            "{enumerated_requirements} enumerated requirement(s) (multi-part above {})",
            super::assess::MANY_CRITERIA
        ),
        format!("repo has {} file(s)", facts.repo_file_count),
        "no pending, parked or held plan history on this project".to_string(),
    ];
    if unscoped_large {
        reasons.push(
            "names no file in a large repo, but reads as mechanical work — prose may only route \
             a plan down"
                .to_string(),
        );
    }
    PlanningComplexityVerdict {
        complexity: PlanningComplexity::Routine,
        reasons,
        named_files,
        named_languages,
        enumerated_requirements,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(objective: &str, repo_file_count: usize) -> PlanningFacts<'_> {
        PlanningFacts {
            objective,
            repo_file_count,
            history: PriorPlanHistory::default(),
        }
    }

    #[test]
    fn one_file_fix_in_a_small_repo_is_routine() {
        let v = assess_planning_complexity(&facts(
            "Fix the off-by-one in src/pagination.rs so the last page is included.",
            12,
        ));
        assert_eq!(v.complexity, PlanningComplexity::Routine, "{:?}", v.reasons);
        assert_eq!(v.named_files, vec!["src/pagination.rs".to_string()]);
        assert!(!v.reasons.is_empty(), "a routine verdict still says why");
    }

    #[test]
    fn multi_file_refactor_is_complex() {
        let v = assess_planning_complexity(&facts(
            "Refactor the quote pipeline: split src/quote.rs into src/quote/parse.rs and \
             src/quote/price.rs, and update src/api.rs callers.",
            40,
        ));
        assert_eq!(v.complexity, PlanningComplexity::Complex);
        assert!(v.named_files.len() >= PLANNING_MULTI_FILE_THRESHOLD);
        assert!(v.reasons.iter().any(|r| r.contains("multi-file")));
    }

    #[test]
    fn cross_language_change_is_complex() {
        let v = assess_planning_complexity(&facts(
            "Add a `currency` field to crates/server/src/quote.rs and show it in ui/src/Quote.tsx",
            60,
        ));
        assert_eq!(v.complexity, PlanningComplexity::Complex);
        assert_eq!(v.named_languages.len(), 2);
        assert!(v.reasons.iter().any(|r| r.contains("cross-language")));
    }

    #[test]
    fn a_previously_failed_or_rejected_plan_is_complex() {
        for history in [
            PriorPlanHistory {
                unapproved_proposal: true,
                ..Default::default()
            },
            PriorPlanHistory {
                parked_goals: 1,
                ..Default::default()
            },
            PriorPlanHistory {
                held_goals: 2,
                ..Default::default()
            },
        ] {
            let v = assess_planning_complexity(&PlanningFacts {
                objective: "Fix the off-by-one in src/pagination.rs",
                repo_file_count: 12,
                history,
            });
            assert_eq!(v.complexity, PlanningComplexity::Complex, "{history:?}");
        }
    }

    /// assess.rs's rule: prose asserting difficulty can never promote a plan.
    #[test]
    fn prose_asserting_difficulty_alone_does_not_flip_routine_to_complex() {
        let v = assess_planning_complexity(&facts(
            "Fix the off-by-one in src/pagination.rs. This is an EXTREMELY complex, critical, \
             hard architectural refactor with security and concurrency implications — use the \
             best, most expensive expert model available.",
            12,
        ));
        assert_eq!(v.complexity, PlanningComplexity::Routine, "{:?}", v.reasons);
    }

    #[test]
    fn many_enumerated_requirements_are_complex() {
        let objective =
            "Ship the export feature:\n- csv\n- json\n- xlsx\n1. filters\n2. paging\n3) auth";
        let v = assess_planning_complexity(&facts(objective, 30));
        assert_eq!(v.enumerated_requirements, 6);
        assert_eq!(v.complexity, PlanningComplexity::Complex);
    }

    /// Unscoped in a large repo is complex — unless the objective reads as
    /// mechanical, which may only route DOWN.
    #[test]
    fn unscoped_objective_in_a_large_repo_is_complex_unless_mechanical() {
        let v = assess_planning_complexity(&facts("Add offline sync for notes", 2400));
        assert_eq!(v.complexity, PlanningComplexity::Complex);
        let v = assess_planning_complexity(&facts("Fix the typo in the settings banner", 2400));
        assert_eq!(v.complexity, PlanningComplexity::Routine, "{:?}", v.reasons);
        // The same unscoped objective in a small repo is routine.
        let v = assess_planning_complexity(&facts("Add offline sync for notes", 20));
        assert_eq!(v.complexity, PlanningComplexity::Routine);
    }

    #[test]
    fn framework_names_urls_and_versions_are_not_files() {
        let v = assess_planning_complexity(&facts(
            "Upgrade Node.js to v22.1 per https://nodejs.org/en/blog/release.html and Next.js, e.g. \
             bump package.json",
            20,
        ));
        assert_eq!(v.named_files, vec!["package.json".to_string()]);
        assert!(v.named_languages.is_empty());
    }
}
