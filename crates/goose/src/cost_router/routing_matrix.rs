//! X-cost-3 — the E3 gate's "deterministic routing matrix" as a pure
//! function plus its own test table.
//!
//! [`docs/orchestrator/CODING_HARNESS_EXCELLENCE_DAG.md`]'s E3 contract:
//! "route by required capability first, then expected cost per solved
//! node — not nominal token price" and its gate: "a deterministic routing
//! matrix chooses the cheapest graduated worker for every fixture, denies
//! ungraduated coordinators, and reconciles estimated versus recorded
//! spend." This module is the FIRST half of that gate: for a fixture's
//! required capability (and difficulty), choose the cheapest candidate on a
//! caller-supplied roster that is GRADUATED for it — an ungraduated
//! candidate is denied outright, never chosen as a fallback, no matter how
//! cheap.
//!
//! ## Not yet wired to real dispatch — on purpose
//!
//! The capability-card / graduation registry now exists
//! ([`super::capability_directory::CapabilityCard`], Round C Lane Z3) and
//! [`RoutingCandidate`] has a [`From`] impl for it below — the TODO(X-grad)
//! that used to sit here is resolved at that ONE seam. `choose_worker` itself
//! is still NOT wired into a real dispatch path: no clean seam exists yet
//! (real dispatch routes by [`super::recommend::WorkflowRole`] via
//! [`super::derived::derive_role_map`] and
//! [`super::capability_directory::CapabilityDirectory::planner`], not by
//! [`FixtureCapability`]), and rewiring it would be exactly the scope
//! creep the lane briefs ask to avoid ("wire it to real selection only if a
//! clean seam exists"). This stays a pure decision function over
//! caller-supplied candidates, now buildable from real cards.

use serde::{Deserialize, Serialize};

use super::capability_directory::CapabilityCard;
use super::recommend::WorkflowRole;

/// A bundled eval fixture's routing-relevant capability need. Mirrors
/// `crates/permagent-eval/tasks/*/task.yaml`'s `category` field (free text
/// today), grouped by [`capability_for_category`] into one of these coarse
/// buckets — the granularity a routing decision actually needs, not the
/// fixture's marketing name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureCapability {
    /// Ordinary code edits: `add-function`, `bug-fix`, `classic`, `game-ui`.
    CodeEdit,
    /// The `model-defaults-{py,rs,ts}` categories: language-idiom fidelity
    /// across ecosystems.
    LanguageIdiom,
    /// The `harness-lever-*` categories: harness-mechanics stress tests
    /// (apply reliability, multi-file refactor, regression traps, spec edge
    /// cases, terminal environment, TDD discipline, "do not overreach").
    HarnessLever,
    /// The `selection` category (`docs/orchestrator/
    /// CODING_HARNESS_ROLE_GRADUATION_DAG.md` G3, `crates/permagent-eval/
    /// tasks/select-*`): tool/DAG-selection fixtures graded on which tool the
    /// harness calls and in what order — not the code it writes. This is
    /// the ONE new variant G3 adds; every use of `FixtureCapability` in this
    /// module matches on it via `==`/`match` with an explicit arm or a
    /// wildcard (checked: no exhaustive match elsewhere in the workspace
    /// depends on this enum staying 3-way — `grep -rn "FixtureCapability::"`
    /// finds no external match sites, only this file's own), so adding it is
    /// additive and does not require touching `capability_directory.rs` or
    /// any other file.
    Selection,
}

/// How hard a fixture is. `permagent-eval`'s task schema has no `difficulty`
/// field today (checked directly: zero `difficulty:` keys across
/// `crates/permagent-eval/tasks/*/task.yaml` as of this lane) —
/// **TODO(X-grad)**: add one to the task schema once the graduation card can
/// consume it. Every real fixture is [`RoutingDifficulty::Standard`] until then;
/// [`RoutingDifficulty::Hard`] exists so the matrix's "capability first, cost
/// second" rule is exercised against more than one axis now, ready for that
/// field when it lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingDifficulty {
    Standard,
    Hard,
}

/// The routing-relevant shape of one fixture: which capability it needs, and
/// how hard it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fixture {
    pub capability: FixtureCapability,
    pub difficulty: RoutingDifficulty,
}

/// One candidate worker on a fake (test) or, later, real capability roster.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutingCandidate {
    pub name: String,
    /// Whether this candidate is graduated AT ALL for `capability`. `false`
    /// means denied outright for a fixture needing `capability`, regardless
    /// of cost.
    pub graduated: bool,
    /// Which capability this candidate is graduated for. Ignored when
    /// `graduated` is `false`.
    pub capability: FixtureCapability,
    /// Expected cost per 1k tokens for this candidate on this class of work
    /// (E3's "expected cost per solved node — not nominal token price"; the
    /// per-node/per-solve normalization itself is the caller's job, this
    /// module only ranks the figure it is handed). `0.0` for a $0 local
    /// candidate.
    pub cost_per_1k_tokens: f64,
    /// Whether this candidate is ALSO graduated for [`RoutingDifficulty::Hard`]
    /// work, not just its base capability — a `Hard` fixture requires both.
    /// Ignored for a [`RoutingDifficulty::Standard`] fixture.
    pub graduated_for_hard: bool,
}

impl From<&CapabilityCard> for RoutingCandidate {
    /// TODO(X-grad) resolved at this seam: build a routing row from a real
    /// capability card. `capability` is fixed to [`FixtureCapability::CodeEdit`]
    /// and `graduated`/`graduated_for_hard` come from the card's EDIT-role
    /// evidence — a card carries evidence for five [`WorkflowRole`]s, not
    /// three [`FixtureCapability`] buckets, and EDIT ("can this model
    /// correctly edit code") is the evidence every fixture capability here
    /// ultimately measures. A caller that needs a `LanguageIdiom`/
    /// `HarnessLever` row clones the result with `.capability` overridden.
    /// Not wired into real dispatch — see the module doc.
    fn from(card: &CapabilityCard) -> Self {
        let edit = card.role(WorkflowRole::Edit);
        let graduated = edit.map(|e| e.graduated()).unwrap_or(false);
        RoutingCandidate {
            name: format!("{}/{}", card.provider, card.model),
            graduated,
            capability: FixtureCapability::CodeEdit,
            cost_per_1k_tokens: card.blended_cost_per_mtok() / 1000.0,
            graduated_for_hard: graduated
                && card.strengths.orchestration
                    >= WorkflowRole::Orchestrate
                        .capability_floor()
                        .orchestration_min,
        }
    }
}

/// Pure: choose the cheapest GRADUATED candidate for a fixture.
///
/// `None` when no candidate is graduated for the fixture's capability (and,
/// for a [`RoutingDifficulty::Hard`] fixture, also graduated for Hard) — an
/// ungraduated coordinator is DENIED, never chosen as a cheaper fallback.
/// Ties break on name so the choice is deterministic (matches E3's own
/// "deterministic routing matrix" wording).
pub fn choose_worker<'a>(
    fixture: &Fixture,
    roster: &'a [RoutingCandidate],
) -> Option<&'a RoutingCandidate> {
    roster
        .iter()
        .filter(|c| c.graduated && c.capability == fixture.capability)
        .filter(|c| fixture.difficulty != RoutingDifficulty::Hard || c.graduated_for_hard)
        .min_by(|a, b| {
            a.cost_per_1k_tokens
                .partial_cmp(&b.cost_per_1k_tokens)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.name.cmp(&b.name))
        })
}

/// Pure classification: which [`FixtureCapability`] a bundled eval fixture's
/// `task.yaml` `category` string names. An unrecognized category falls back
/// to [`FixtureCapability::CodeEdit`] — the largest, least-specialized
/// bucket — rather than panicking or denying a fixture whose category this
/// list has not caught up with yet.
pub fn capability_for_category(category: &str) -> FixtureCapability {
    match category {
        "model-defaults-py" | "model-defaults-rs" | "model-defaults-ts" => {
            FixtureCapability::LanguageIdiom
        }
        c if c.starts_with("harness-lever-") => FixtureCapability::HarnessLever,
        // G3: `crates/permagent-eval/tasks/select-*/task.yaml`'s
        // `category: selection` — see [`FixtureCapability::Selection`].
        "selection" => FixtureCapability::Selection,
        _ => FixtureCapability::CodeEdit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(
        name: &str,
        graduated: bool,
        capability: FixtureCapability,
        cost: f64,
    ) -> RoutingCandidate {
        RoutingCandidate {
            name: name.to_string(),
            graduated,
            capability,
            cost_per_1k_tokens: cost,
            graduated_for_hard: false,
        }
    }

    #[test]
    fn cheapest_graduated_candidate_wins() {
        let fixture = Fixture {
            capability: FixtureCapability::CodeEdit,
            difficulty: RoutingDifficulty::Standard,
        };
        let roster = vec![
            candidate("expensive", true, FixtureCapability::CodeEdit, 5.0),
            candidate("cheap", true, FixtureCapability::CodeEdit, 0.5),
            candidate("mid", true, FixtureCapability::CodeEdit, 2.0),
        ];
        let chosen = choose_worker(&fixture, &roster).expect("a graduated candidate exists");
        assert_eq!(chosen.name, "cheap");
    }

    /// The whole point: an ungraduated coordinator is refused even when it
    /// is the cheapest — sometimes $0 — option on the roster.
    #[test]
    fn ungraduated_coordinators_are_denied_even_when_cheapest() {
        let fixture = Fixture {
            capability: FixtureCapability::CodeEdit,
            difficulty: RoutingDifficulty::Standard,
        };
        let roster = vec![
            candidate(
                "free_but_ungraduated",
                false,
                FixtureCapability::CodeEdit,
                0.0,
            ),
            candidate("paid_graduated", true, FixtureCapability::CodeEdit, 3.0),
        ];
        let chosen = choose_worker(&fixture, &roster).expect("a graduated candidate exists");
        assert_eq!(chosen.name, "paid_graduated");
    }

    #[test]
    fn no_graduated_candidate_for_the_capability_denies_dispatch_entirely() {
        let fixture = Fixture {
            capability: FixtureCapability::HarnessLever,
            difficulty: RoutingDifficulty::Standard,
        };
        let roster = vec![candidate(
            "wrong_capability",
            true,
            FixtureCapability::CodeEdit,
            0.1,
        )];
        assert!(choose_worker(&fixture, &roster).is_none());
    }

    #[test]
    fn a_hard_fixture_requires_hard_graduation_too() {
        let fixture = Fixture {
            capability: FixtureCapability::LanguageIdiom,
            difficulty: RoutingDifficulty::Hard,
        };
        let cheap_not_hard = candidate(
            "cheap_not_hard_graduated",
            true,
            FixtureCapability::LanguageIdiom,
            0.1,
        );
        let mut strong_hard = candidate(
            "strong_hard_graduated",
            true,
            FixtureCapability::LanguageIdiom,
            4.0,
        );
        strong_hard.graduated_for_hard = true;
        let roster = vec![cheap_not_hard, strong_hard];
        let chosen = choose_worker(&fixture, &roster).expect("a hard-graduated candidate exists");
        assert_eq!(chosen.name, "strong_hard_graduated");
    }

    #[test]
    fn a_standard_fixture_does_not_require_hard_graduation() {
        let fixture = Fixture {
            capability: FixtureCapability::CodeEdit,
            difficulty: RoutingDifficulty::Standard,
        };
        let roster = vec![candidate(
            "cheap_not_hard_graduated",
            true,
            FixtureCapability::CodeEdit,
            0.1,
        )];
        let chosen =
            choose_worker(&fixture, &roster).expect("standard fixtures don't need hard graduation");
        assert_eq!(chosen.name, "cheap_not_hard_graduated");
    }

    #[test]
    fn ties_break_deterministically_on_name() {
        let fixture = Fixture {
            capability: FixtureCapability::CodeEdit,
            difficulty: RoutingDifficulty::Standard,
        };
        let roster = vec![
            candidate("zeta", true, FixtureCapability::CodeEdit, 1.0),
            candidate("alpha", true, FixtureCapability::CodeEdit, 1.0),
        ];
        let chosen = choose_worker(&fixture, &roster).unwrap();
        assert_eq!(chosen.name, "alpha");
    }

    // ── The bundled eval fixture categories, classified ─────────────────────

    /// The observed `category:` values across every
    /// `crates/permagent-eval/tasks/*/task.yaml` as of this lane
    /// (`grep -rh '^category:' crates/permagent-eval/tasks/*/task.yaml | sort -u`).
    /// A category added later that this list has not caught up with falls
    /// back to `CodeEdit` (see `an_unknown_category_falls_back_to_code_edit`),
    /// never panics and never silently denies a fixture a route.
    #[test]
    fn every_bundled_category_classifies_to_the_expected_capability() {
        let categories = [
            ("add-function", FixtureCapability::CodeEdit),
            ("bug-fix", FixtureCapability::CodeEdit),
            ("classic", FixtureCapability::CodeEdit),
            ("game-ui", FixtureCapability::CodeEdit),
            ("model-defaults-py", FixtureCapability::LanguageIdiom),
            ("model-defaults-rs", FixtureCapability::LanguageIdiom),
            ("model-defaults-ts", FixtureCapability::LanguageIdiom),
            (
                "harness-lever-do-not-overreach",
                FixtureCapability::HarnessLever,
            ),
            (
                "harness-lever-edit-apply-reliability",
                FixtureCapability::HarnessLever,
            ),
            (
                "harness-lever-localization",
                FixtureCapability::HarnessLever,
            ),
            (
                "harness-lever-multi-file-refactor",
                FixtureCapability::HarnessLever,
            ),
            (
                "harness-lever-regression-trap",
                FixtureCapability::HarnessLever,
            ),
            (
                "harness-lever-spec-following-edge-cases",
                FixtureCapability::HarnessLever,
            ),
            (
                "harness-lever-terminal-environment",
                FixtureCapability::HarnessLever,
            ),
            ("harness-lever-test-driven", FixtureCapability::HarnessLever),
            ("selection", FixtureCapability::Selection),
        ];
        for (cat, expected) in categories {
            assert_eq!(capability_for_category(cat), expected, "{cat}");
        }
    }

    #[test]
    fn an_unknown_category_falls_back_to_code_edit() {
        assert_eq!(
            capability_for_category("something-new-nobody-has-added-yet"),
            FixtureCapability::CodeEdit
        );
    }

    /// G3: `crates/permagent-eval/tasks/select-*/task.yaml`'s
    /// `category: "selection"` classifies to its own bucket, not the
    /// `CodeEdit` fallback — a selection fixture is graded on tool/DAG
    /// choice, not code correctness, so it must never be silently lumped in
    /// with ordinary edits.
    #[test]
    fn selection_category_maps_to_its_own_capability() {
        assert_eq!(
            capability_for_category("selection"),
            FixtureCapability::Selection
        );
    }

    // ── Full matrix: every bundled category × a fake capability roster ──────

    /// The E3 gate itself, as a table: for every bundled fixture category, a
    /// fake roster carrying a cheaper UNGRADUATED option alongside a pricier
    /// graduated one for that category's capability — proving graduation is
    /// checked BEFORE cost, not merely coincidentally aligned with it.
    #[test]
    fn routing_matrix_over_every_bundled_category_picks_the_cheapest_graduated_worker() {
        let roster = vec![
            candidate(
                "local_ungraduated_edit",
                false,
                FixtureCapability::CodeEdit,
                0.0,
            ),
            candidate(
                "cheap_cloud_graduated_edit",
                true,
                FixtureCapability::CodeEdit,
                0.3,
            ),
            candidate(
                "frontier_graduated_edit",
                true,
                FixtureCapability::CodeEdit,
                5.0,
            ),
            candidate(
                "local_ungraduated_idiom",
                false,
                FixtureCapability::LanguageIdiom,
                0.0,
            ),
            candidate(
                "idiom_graduated",
                true,
                FixtureCapability::LanguageIdiom,
                0.8,
            ),
            candidate(
                "local_ungraduated_lever",
                false,
                FixtureCapability::HarnessLever,
                0.0,
            ),
            candidate(
                "lever_graduated",
                true,
                FixtureCapability::HarnessLever,
                1.2,
            ),
        ];
        let categories = [
            "add-function",
            "bug-fix",
            "classic",
            "game-ui",
            "model-defaults-py",
            "model-defaults-rs",
            "model-defaults-ts",
            "harness-lever-do-not-overreach",
            "harness-lever-edit-apply-reliability",
            "harness-lever-localization",
            "harness-lever-multi-file-refactor",
            "harness-lever-regression-trap",
            "harness-lever-spec-following-edge-cases",
            "harness-lever-terminal-environment",
            "harness-lever-test-driven",
        ];
        for category in categories {
            let fixture = Fixture {
                capability: capability_for_category(category),
                difficulty: RoutingDifficulty::Standard,
            };
            let chosen = choose_worker(&fixture, &roster)
                .unwrap_or_else(|| panic!("{category} must have a graduated candidate"));
            assert!(chosen.graduated, "{category}");
            let cheapest_graduated_cost = roster
                .iter()
                .filter(|c| c.graduated && c.capability == fixture.capability)
                .map(|c| c.cost_per_1k_tokens)
                .fold(f64::INFINITY, f64::min);
            assert!(
                (chosen.cost_per_1k_tokens - cheapest_graduated_cost).abs() < 1e-9,
                "{category}: expected the cheapest graduated candidate"
            );
            assert!(
                !chosen.name.starts_with("local_ungraduated"),
                "{category}: must not pick the cheaper ungraduated option"
            );
        }
    }

    // ── From<&CapabilityCard> for RoutingCandidate (TODO(X-grad) seam) ──────

    fn card_with_edit_graduation(
        provider: &str,
        model: &str,
        orchestration: f64,
        graduated: bool,
    ) -> super::super::capability_directory::CapabilityCard {
        use super::super::capability_directory::{
            CapabilityCard, CardSource, RoleEvidence, StrengthProfile,
        };
        let mut roles = std::collections::BTreeMap::new();
        for r in WorkflowRole::all() {
            roles.insert(r.as_str().to_string(), RoleEvidence::default());
        }
        if graduated {
            roles.insert(
                WorkflowRole::Edit.as_str().to_string(),
                RoleEvidence::new(
                    true,
                    true,
                    Some(0.95),
                    Some(0.98),
                    vec!["run-1".to_string()],
                    None,
                    None,
                    Some(chrono::Utc::now().date_naive()),
                    chrono::Utc::now().date_naive(),
                )
                .unwrap(),
            );
        }
        CapabilityCard {
            provider: provider.to_string(),
            model: model.to_string(),
            display_name: model.to_string(),
            family: provider.to_string(),
            billing_class: crate::config::agent_identity::WorkerBillingClass::PaidApi,
            is_local: false,
            context_window: 128_000,
            input_usd_per_mtok: 1.0,
            output_usd_per_mtok: 4.0,
            strengths: StrengthProfile {
                orchestration,
                edit_format: 0.95,
                mechanical: true,
                review: true,
            },
            roles,
            source: CardSource::KnowledgeBase,
        }
    }

    #[test]
    fn routing_candidate_from_capability_card_reads_edit_graduation() {
        let ungraduated = card_with_edit_graduation("openai", "gpt-5.6-mini", 0.60, false);
        let rc: RoutingCandidate = (&ungraduated).into();
        assert_eq!(rc.name, "openai/gpt-5.6-mini");
        assert!(!rc.graduated);
        assert_eq!(rc.capability, FixtureCapability::CodeEdit);
        assert!(!rc.graduated_for_hard);

        let graduated_strong =
            card_with_edit_graduation("anthropic", "claude-opus-4-8", 0.886, true);
        let rc: RoutingCandidate = (&graduated_strong).into();
        assert!(rc.graduated);
        assert!(
            rc.graduated_for_hard,
            "graduated AND above the ORCHESTRATE floor must also clear Hard"
        );
        // input(1.0) + output(4.0) = 5.0 $/Mtok blended, / 1000 = $/1k tokens.
        assert!((rc.cost_per_1k_tokens - 0.005).abs() < 1e-9);

        // Graduated but below the ORCHESTRATE floor: eligible, not Hard-graduated.
        let graduated_weak = card_with_edit_graduation("zai", "glm-4.5-air", 0.62, true);
        let rc: RoutingCandidate = (&graduated_weak).into();
        assert!(rc.graduated);
        assert!(!rc.graduated_for_hard);
    }
}
