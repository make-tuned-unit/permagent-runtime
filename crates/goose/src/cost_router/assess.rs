//! Upfront difficulty assessment: which rung of the ladder a goal should START on.
//!
//! The escalation ladder ([`super::escalation`]) is reactive — begin cheap,
//! climb when verification fails, carrying a handoff. That is the right shape,
//! because a failed verification is *measured* evidence a model was not up to
//! the job, where any upfront guess is a prediction. But starting every goal on
//! the cheapest rung wastes a full dispatch on work that was obviously hard,
//! and starting every goal high is the expense the ladder exists to avoid.
//!
//! This narrows the starting rung, and deliberately does no more than that. The
//! ladder still owns the outcome: a goal assessed Cheap that turns out hard
//! escalates on its own.
//!
//! ## Deterministic and zero-LLM, on purpose
//!
//! No model is asked how hard a goal is. A router that consults an LLM can be
//! argued into the expensive tier by the very prompt it is pricing — a goal
//! whose description says "this is extremely complex, use the best model" would
//! route itself, and prompt text is attacker-influenced in any shared project.
//! It is also unbudgetable: a call to decide whether to spend is a call.
//!
//! The same reasoning as [`crate::cards::title_similarity`]: a decision that a
//! model makes is a decision a model can be talked out of.
//!
//! ## What it reads
//!
//! Signals that correlate with difficulty and are cheap to observe, read with
//! one asymmetry: the goal's free-text title and body can only route it DOWN.
//! Raising the tier takes structure (breadth), an explicit tier pin, or an
//! explicit `tags` entry — never prose, which is written by whoever, or
//! whatever, filed the goal.

use super::recommend::WorkflowRole;
use super::tier::Tier;

/// Why a goal landed on its starting rung — carried into telemetry and the
/// dispatch snapshot so "why did this run on the expensive model?" is
/// answerable after the fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assessment {
    pub tier: Tier,
    pub reason: &'static str,
}

/// Words that mark work as reasoning-heavy rather than mechanical. Matched only
/// against explicit `tags` metadata — never title or description — so free text
/// cannot promote a goal. Chosen for naming a KIND of work, not a degree of it:
/// "complex", "hard" and "difficult" are deliberately absent.
const HARD_SIGNALS: &[&str] = &[
    "architect",
    "architecture",
    "refactor",
    "redesign",
    "migrate",
    "migration",
    "concurrency",
    "race condition",
    "deadlock",
    "security",
    "vulnerability",
    "cryptograph",
    "performance",
    "optimize",
    "debug",
    "root cause",
    "algorithm",
    "schema change",
    "backwards compatib",
    "protocol",
];

/// Work that is well-specified and largely typing. Same rule: these name a kind
/// of edit, not a claimed difficulty.
const MECHANICAL_SIGNALS: &[&str] = &[
    "typo",
    "rename",
    "readme",
    "changelog",
    "comment",
    "docstring",
    "formatting",
    "lint",
    "gitignore",
    "bump",
    "add a test",
    "documentation",
];

/// Acceptance criteria above this count read as a multi-part goal.
const MANY_CRITERIA: usize = 5;
/// Files above this count read as a broad change.
const MANY_FILES: usize = 8;

/// Assess a goal's starting tier from its title, body and metadata.
///
/// Precedence is explicit intent → breadth → vocabulary → default. The default
/// is [`Tier::CheapCloud`], NOT the cheapest rung: local models cannot reliably
/// drive an agentic coding loop, and the ladder's job is to climb from a rung
/// that can at least attempt the work.
pub fn assess_goal(title: &str, description: &str, metadata: &serde_json::Value) -> Assessment {
    // 1. An explicit user pin always wins. Someone who has said which tier they
    //    want is not asking for an opinion.
    if let Some(pinned) = metadata.get("tier").and_then(serde_json::Value::as_str) {
        if let Some(tier) = tier_from_str(pinned) {
            return Assessment {
                tier,
                reason: "explicitly pinned on the goal",
            };
        }
    }

    // 2. Breadth. A goal touching many files or carrying many acceptance
    //    criteria is a coordination problem regardless of its vocabulary —
    //    structure is harder to fake than words.
    let criteria = metadata
        .get("acceptance_criteria")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    if criteria > MANY_CRITERIA {
        return Assessment {
            tier: Tier::Frontier,
            reason: "many acceptance criteria — a multi-part goal",
        };
    }
    let files = metadata
        .get("paths")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    if files > MANY_FILES {
        return Assessment {
            tier: Tier::Frontier,
            reason: "touches many files — a broad change",
        };
    }

    // 3. Vocabulary, with the promotion path narrowed to explicit `tags`: the
    //    title and body are prose the goal's author writes, so matching them
    //    lets a goal name its way onto the expensive rung. Mechanical signals
    //    still read title and body — they can only route DOWN.
    let tagged_hard = metadata
        .get("tags")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|tags| {
            tags.iter().filter_map(serde_json::Value::as_str).any(|t| {
                let t = t.to_lowercase();
                HARD_SIGNALS.iter().any(|s| t.contains(s))
            })
        });
    if tagged_hard {
        return Assessment {
            tier: Tier::Frontier,
            reason: "tagged as reasoning-heavy work",
        };
    }
    let haystack = format!("{} {}", title, description).to_lowercase();
    if MECHANICAL_SIGNALS.iter().any(|s| haystack.contains(s)) && criteria <= 1 {
        return Assessment {
            tier: Tier::LocalFree,
            reason: "mechanical, single-criterion edit",
        };
    }

    Assessment {
        tier: Tier::CheapCloud,
        reason: "no strong signal — start cheap and let the ladder climb",
    }
}

/// File extensions the on-device tier is expected to handle comfortably —
/// small, well-trodden scripting languages, not the exhaustive set of
/// "languages a local model can technically write."
///
/// R55b (HARNESS_LOOP_LOG.md "A52 analysis" point 2): narrowed to
/// `{py, js, ts, sh}` — `jsx`/`tsx` dropped. A React component file is
/// disproportionately likely to be one piece of a larger, cross-file UI
/// change (the exact shape A52 found sweeping partial-local goals into a
/// bigger decomposition), so it now falls through to the cheap-cloud
/// `Mechanical` rung instead of `Local`.
const LOCAL_LANGUAGE_EXTENSIONS: &[&str] = &["py", "js", "ts", "sh"];

/// R55b: the file-count ceiling for the `Local` rung — above this, the
/// default rule never routes local regardless of language or vocabulary.
const LOCAL_MAX_FILES: usize = 2;

/// R55b (`## A52 analysis` point 2, `harness-refactor-shipping-quote-
/// signature`'s 3-of-6 Local split): free-text markers meaning a goal is
/// part of a larger cross-file change even when its OWN `paths` list is
/// short — a single call-site edit that names, in its own prose, that it
/// is one caller among many of a changed signature. Matched against the
/// goal's title, description AND acceptance criteria; per this module's
/// existing DOWN-only asymmetry (see the module doc comment), these can
/// only DENY the `Local` shortcut — never promote a goal to `Orchestrate`
/// on their own; an untagged goal that merely happens to mention one of
/// these words still lands on `Mechanical`, not `Orchestrate`.
const LOCAL_INELIGIBLE_TEXT_SIGNALS: &[&str] = &[
    // Build system.
    "makefile",
    "cmake",
    "webpack",
    "package.json",
    "cargo.toml",
    "tsconfig",
    "pyproject.toml",
    "setup.py",
    "build system",
    "build script",
    // Refactor/rename across files.
    "call site",
    "call sites",
    "every caller",
    "all callers",
    "each caller",
    "update the callers",
    "across the codebase",
    "signature change",
    "signature update",
    "rename",
    "refactor",
];

/// R55b: whether this goal's own free text (title, description, and
/// acceptance criteria — never `tags`, already checked separately as the
/// promotable vocabulary) names a build system or a cross-file refactor/
/// rename, denying it the `Local` shortcut in [`default_worker_tier_for_
/// goal`].
fn goal_text_names_build_system_or_cross_file_change(
    title: &str,
    description: &str,
    metadata: &serde_json::Value,
) -> bool {
    let criteria = metadata
        .get("acceptance_criteria")
        .and_then(serde_json::Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    let haystack = format!("{title} {description} {criteria}").to_lowercase();
    LOCAL_INELIGIBLE_TEXT_SIGNALS
        .iter()
        .any(|s| haystack.contains(s))
}

/// R48d (HARNESS_LOOP_LOG.md "A45 design", Addendum (b)): the default
/// `WorkflowRole` a dispatch routes to when `goal_advance{action: dispatch}`
/// carries neither an explicit `worker_tier` nor a worker persona
/// `workflow_role` tag — i.e. it sits exactly where [`assess_goal`]'s own
/// inference sits today, refining it into a concrete role rather than
/// replacing its signals. Reuses [`assess_goal`]'s breadth ([`MANY_FILES`])
/// and vocabulary ([`HARD_SIGNALS`], tags-only — the same no-self-promotion
/// rule: free text can only route DOWN, never up) checks verbatim, adding
/// structural signals: file extension and file count, read from
/// `metadata.paths` when present, plus (R55b) a free-text DENY-only check
/// ([`goal_text_names_build_system_or_cross_file_change`]) that keeps a
/// goal off the `Local` rung even when its own `paths` list is short.
/// Never asks a model, for the same reason `assess_goal` never does (see
/// this module's doc comment).
///
/// Precedence: breadth (many files) or tagged hard-signal vocabulary →
/// `Orchestrate` (the multi-file/refactor/build-system rung); a small
/// (`paths.len() <= LOCAL_MAX_FILES`) goal in a language the on-device
/// tier handles well, whose own text names no build system or cross-file
/// refactor → `Local`; otherwise `Mechanical` — the same cheap-cloud-shaped
/// rung an unsignalled goal starts on in [`assess_goal`]. Returns the
/// resolved role together with a static reason string for the dispatch
/// audit trail.
pub fn default_worker_tier_for_goal(
    title: &str,
    description: &str,
    metadata: &serde_json::Value,
) -> (WorkflowRole, &'static str) {
    let paths: Vec<&str> = metadata
        .get("paths")
        .and_then(serde_json::Value::as_array)
        .map(|arr| arr.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();

    // Breadth: many files is a coordination problem regardless of vocabulary
    // — the same structural signal `assess_goal` reads, reused verbatim.
    if paths.len() > MANY_FILES {
        return (
            WorkflowRole::Orchestrate,
            "touches many files — a broad change",
        );
    }

    // Vocabulary: explicit `tags` only, mirroring `assess_goal`'s
    // no-self-promotion rule — title/description prose cannot promote a
    // goal to the expensive tier.
    let tagged_hard = metadata
        .get("tags")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|tags| {
            tags.iter().filter_map(serde_json::Value::as_str).any(|t| {
                let t = t.to_lowercase();
                HARD_SIGNALS.iter().any(|s| t.contains(s))
            })
        });
    if tagged_hard {
        return (WorkflowRole::Orchestrate, "tagged as reasoning-heavy work");
    }

    // Language: a small goal in a language the local tier handles well is
    // the "give the free rung a fair shot" case.
    let is_local_language = !paths.is_empty()
        && paths.iter().all(|p| {
            p.rsplit('.').next().is_some_and(|ext| {
                let ext = ext.to_lowercase();
                LOCAL_LANGUAGE_EXTENSIONS.contains(&ext.as_str())
            })
        });
    // R55b: even a small, single/two-file, right-language goal is denied
    // the Local shortcut when its own text names a build system or a
    // cross-file refactor/rename — the A52 shape where a single call-site
    // edit's own `paths` list stayed short while its prose named it as one
    // caller among several being updated for a changed signature.
    if paths.len() <= LOCAL_MAX_FILES
        && is_local_language
        && !goal_text_names_build_system_or_cross_file_change(title, description, metadata)
    {
        return (
            WorkflowRole::Local,
            "small goal, right language, no build-system or cross-file-refactor signal",
        );
    }

    (
        WorkflowRole::Mechanical,
        "no strong signal — start on the cheap rung",
    )
}

fn tier_from_str(s: &str) -> Option<Tier> {
    match s.trim().to_lowercase().as_str() {
        "mesh_free" => Some(Tier::MeshFree),
        "local_free" | "local" => Some(Tier::LocalFree),
        "cheap_cloud" | "cheap" => Some(Tier::CheapCloud),
        "frontier" | "hard" => Some(Tier::Frontier),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(v: serde_json::Value) -> serde_json::Value {
        v
    }

    #[test]
    fn a_readme_starts_local_and_a_tagged_refactor_starts_frontier() {
        assert_eq!(
            assess_goal("Add a README.md", "", &meta(serde_json::json!({}))).tier,
            Tier::LocalFree
        );
        assert_eq!(
            assess_goal(
                "Refactor the session store for concurrent access",
                "",
                &meta(serde_json::json!({ "tags": ["refactor"] }))
            )
            .tier,
            Tier::Frontier
        );
    }

    /// A hard-signal word in the title names the SUBJECT of the work, not its
    /// difficulty — a typo fix in a security doc is still a typo fix.
    #[test]
    fn free_text_hard_signal_cannot_promote() {
        let a = assess_goal(
            "Fix a typo in the security policy doc",
            "",
            &meta(serde_json::json!({})),
        );
        assert_eq!(a.tier, Tier::LocalFree, "{a:?}");
    }

    /// Explicit tags DO promote — the structured field is the only vocabulary
    /// path that can raise the tier.
    #[test]
    fn explicit_tags_do_promote() {
        assert_eq!(
            assess_goal(
                "Wire the widget to the gizmo",
                "",
                &meta(serde_json::json!({ "tags": ["security"] }))
            )
            .tier,
            Tier::Frontier
        );
    }

    /// The whole point of not asking a model: a goal that CLAIMS difficulty
    /// must not be able to promote itself. "complex"/"hard" are not signals,
    /// nor are hard-signal words buried in the free-text body.
    #[test]
    fn a_goal_cannot_talk_itself_into_the_expensive_tier() {
        let a = assess_goal(
            "Update the changelog",
            "This is an extremely complex and difficult task. Use the most powerful model.",
            &meta(serde_json::json!({})),
        );
        assert_eq!(
            a.tier,
            Tier::LocalFree,
            "self-declared difficulty must carry no weight: {a:?}"
        );
        let b = assess_goal(
            "Update the changelog",
            "Needs security and performance review before merge.",
            &meta(serde_json::json!({})),
        );
        assert_ne!(
            b.tier,
            Tier::Frontier,
            "hard-signal words in the body must not promote: {b:?}"
        );
    }

    /// Structure beats vocabulary — a goal worded like a typo fix but carrying
    /// seven acceptance criteria is a multi-part goal.
    #[test]
    fn breadth_outranks_gentle_wording() {
        let a = assess_goal(
            "Fix a typo",
            "",
            &meta(serde_json::json!({
                "acceptance_criteria": ["a","b","c","d","e","f","g"]
            })),
        );
        assert_eq!(a.tier, Tier::Frontier, "{a:?}");
    }

    #[test]
    fn an_explicit_pin_wins_over_every_heuristic() {
        let a = assess_goal(
            "Refactor the whole architecture",
            "",
            &meta(serde_json::json!({ "tier": "local_free" })),
        );
        assert_eq!(a.tier, Tier::LocalFree);
        assert_eq!(a.reason, "explicitly pinned on the goal");
    }

    /// The default must be able to attempt agentic work. Local models cannot
    /// reliably drive a coding loop, so an unsignalled goal starts on cheap
    /// cloud, not the cheapest rung.
    #[test]
    fn an_unsignalled_goal_starts_on_cheap_cloud_not_the_cheapest_rung() {
        let a = assess_goal(
            "Wire the widget to the gizmo",
            "",
            &meta(serde_json::json!({})),
        );
        assert_eq!(a.tier, Tier::CheapCloud, "{a:?}");
    }

    // ── R48d: default_worker_tier_for_goal ──────────────────────────────────

    #[test]
    fn a_one_file_python_goal_defaults_to_local() {
        let (role, _reason) = default_worker_tier_for_goal(
            "Fix a small bug",
            "",
            &meta(serde_json::json!({ "paths": ["app/main.py"] })),
        );
        assert_eq!(role, WorkflowRole::Local);
    }

    #[test]
    fn a_one_file_js_or_ts_goal_also_defaults_to_local() {
        for ext in ["app.js", "app.ts"] {
            let (role, _) = default_worker_tier_for_goal(
                "Small fix",
                "",
                &meta(serde_json::json!({ "paths": [ext] })),
            );
            assert_eq!(role, WorkflowRole::Local, "{ext}");
        }
    }

    #[test]
    fn a_one_file_shell_script_goal_defaults_to_local() {
        let (role, _) = default_worker_tier_for_goal(
            "Fix the deploy script",
            "",
            &meta(serde_json::json!({ "paths": ["deploy.sh"] })),
        );
        assert_eq!(role, WorkflowRole::Local);
    }

    /// R55b: the local language allowlist narrowed to `{py, js, ts, sh}` —
    /// `jsx`/`tsx` dropped, so a React component file now falls through to
    /// the cheap-cloud `Mechanical` rung instead of `Local`.
    #[test]
    fn a_one_file_tsx_or_jsx_goal_no_longer_defaults_to_local() {
        for ext in ["app.tsx", "app.jsx"] {
            let (role, _) = default_worker_tier_for_goal(
                "Small fix",
                "",
                &meta(serde_json::json!({ "paths": [ext] })),
            );
            assert_eq!(role, WorkflowRole::Mechanical, "{ext}");
        }
    }

    #[test]
    fn a_two_file_python_goal_with_no_build_or_refactor_signal_still_defaults_to_local() {
        let (role, _) = default_worker_tier_for_goal(
            "Fix the discount rounding bug",
            "Correct the off-by-one in the discount calculation and its test.",
            &meta(serde_json::json!({ "paths": ["discount.py", "test_discount.py"] })),
        );
        assert_eq!(role, WorkflowRole::Local);
    }

    /// R55b: file count above `LOCAL_MAX_FILES` alone (no vocabulary, no
    /// tag) is enough to deny the Local shortcut — same "structure beats
    /// wording" principle `assess_goal`'s own breadth check already uses.
    #[test]
    fn a_three_file_goal_with_no_other_signal_no_longer_defaults_to_local() {
        let (role, _) = default_worker_tier_for_goal(
            "Fix three related bugs",
            "",
            &meta(serde_json::json!({ "paths": ["a.py", "b.py", "c.py"] })),
        );
        assert_eq!(role, WorkflowRole::Mechanical);
    }

    /// R55b (`## A52 analysis` point 2): the exact `harness-refactor-
    /// shipping-quote-signature` shape — one call-site edit's own `paths`
    /// list stays short (a single file), but its prose names it as one
    /// caller among several being updated for a changed signature. Must
    /// not default to Local even though the file-count/language checks
    /// alone would allow it.
    #[test]
    fn a_single_file_goal_naming_a_cross_file_refactor_in_prose_is_excluded_from_local() {
        let (role, reason) = default_worker_tier_for_goal(
            "Update the call site in manualQuote.ts",
            "Part of a signature update — every caller of computeQuote must pass the discount \
             rate.",
            &meta(serde_json::json!({ "paths": ["manualQuote.ts"] })),
        );
        assert_ne!(role, WorkflowRole::Local, "{reason}");
    }

    #[test]
    fn a_goal_mentioning_a_build_system_in_prose_is_excluded_from_local() {
        let (role, _) = default_worker_tier_for_goal(
            "Fix the requirements pin",
            "Update pyproject.toml's version constraint for requests.",
            &meta(serde_json::json!({ "paths": ["deps.py"] })),
        );
        assert_ne!(role, WorkflowRole::Local);
    }

    #[test]
    fn a_goal_whose_acceptance_criteria_name_a_cross_file_refactor_is_excluded_from_local() {
        let (role, _) = default_worker_tier_for_goal(
            "Update the discount call site",
            "",
            &meta(serde_json::json!({
                "paths": ["a.py"],
                "acceptance_criteria": ["Every caller of compute_discount passes the new rate"]
            })),
        );
        assert_ne!(role, WorkflowRole::Local);
    }

    /// R55b: the file-count ceiling AND the free-text refactor signal both
    /// deny Local here — either alone already would.
    #[test]
    fn a_three_file_ts_refactor_defaults_to_cloud_not_local() {
        let (role, reason) = default_worker_tier_for_goal(
            "Refactor the shipping quote signature",
            "Update every caller of computeQuote to pass the new discount parameter.",
            &meta(serde_json::json!({ "paths": ["a.ts", "b.ts", "c.ts"] })),
        );
        assert_ne!(role, WorkflowRole::Local, "{reason}");
    }

    #[test]
    fn a_five_file_tagged_refactor_defaults_to_orchestrate() {
        let (role, reason) = default_worker_tier_for_goal(
            "Refactor the payment module",
            "",
            &meta(serde_json::json!({
                "paths": ["a.py", "b.py", "c.py", "d.py", "e.py"],
                "tags": ["refactor"],
            })),
        );
        assert_eq!(role, WorkflowRole::Orchestrate, "{reason}");
    }

    #[test]
    fn breadth_alone_defaults_to_orchestrate_even_with_no_vocabulary() {
        let paths: Vec<&str> = (0..9).map(|_| "file.py").collect();
        let (role, _) = default_worker_tier_for_goal(
            "Touch a lot of files",
            "",
            &meta(serde_json::json!({ "paths": paths })),
        );
        assert_eq!(role, WorkflowRole::Orchestrate);
    }

    /// Free-text vocabulary must not promote — same no-self-promotion rule
    /// as `assess_goal`. A title naming "refactor" with no explicit tag stays
    /// on the cheap rung, not Orchestrate.
    #[test]
    fn free_text_refactor_wording_cannot_promote_the_default_rule() {
        let (role, _) = default_worker_tier_for_goal(
            "Refactor the payment module",
            "",
            &meta(serde_json::json!({ "paths": ["a.py", "b.py"] })),
        );
        assert_ne!(role, WorkflowRole::Orchestrate);
    }

    #[test]
    fn a_multi_language_or_pathless_goal_defaults_to_mechanical() {
        let (role, _) =
            default_worker_tier_for_goal("Wire the widget", "", &meta(serde_json::json!({})));
        assert_eq!(role, WorkflowRole::Mechanical);

        let (role, _) = default_worker_tier_for_goal(
            "Touch two files",
            "",
            &meta(serde_json::json!({ "paths": ["a.py", "b.rs"] })),
        );
        assert_eq!(role, WorkflowRole::Mechanical);
    }
}
