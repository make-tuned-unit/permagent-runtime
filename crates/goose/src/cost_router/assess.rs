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
//! Signals that correlate with difficulty and are cheap to observe. It reads
//! only STRUCTURE and explicit user intent, never a self-assessment of
//! difficulty written into the goal body.

use super::tier::Tier;

/// Why a goal landed on its starting rung — carried into telemetry and the
/// dispatch snapshot so "why did this run on the expensive model?" is
/// answerable after the fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assessment {
    pub tier: Tier,
    pub reason: &'static str,
}

/// Words that mark work as reasoning-heavy rather than mechanical. Chosen for
/// naming a KIND of work, not a degree of it: "complex", "hard" and "difficult"
/// are deliberately absent — they are self-assessment, which is the thing this
/// must not trust.
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

    // 3. Vocabulary, over title AND body. Hard signals win ties: the cost of
    //    under-tiering a hard goal is a wasted dispatch plus an escalation,
    //    while over-tiering a simple one is a single cheap-ish call.
    let haystack = format!("{} {}", title, description).to_lowercase();
    if HARD_SIGNALS.iter().any(|s| haystack.contains(s)) {
        return Assessment {
            tier: Tier::Frontier,
            reason: "names reasoning-heavy work",
        };
    }
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
    fn a_readme_starts_local_and_a_refactor_starts_frontier() {
        assert_eq!(
            assess_goal("Add a README.md", "", &meta(serde_json::json!({}))).tier,
            Tier::LocalFree
        );
        assert_eq!(
            assess_goal(
                "Refactor the session store for concurrent access",
                "",
                &meta(serde_json::json!({}))
            )
            .tier,
            Tier::Frontier
        );
    }

    /// The whole point of not asking a model: a goal that CLAIMS difficulty
    /// must not be able to promote itself. "complex"/"hard" are not signals.
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
}
