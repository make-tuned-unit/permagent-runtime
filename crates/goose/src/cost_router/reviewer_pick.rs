//! WHICH model reviews the work — the cross-family half of the review gate.
//!
//! [`super::review_gate`] owns the *decision* (lenses, parsing, default-to-reject).
//! This module owns the *choice of reviewer*, and it exists because the gate's
//! whole claim rests on one property: the model that judges the work is not from
//! the family that produced it. A same-family "review" is a model agreeing with
//! itself, and the card must never call that an independent review.
//!
//! Precedence, in order (best-fit, cost-conscious — the standing rule):
//!
//! 1. the operator's hand-configured REVIEW role ([`super::role_map`]) — an
//!    explicit mapping is honoured even when it shares the author's family, but
//!    it is then recorded as **not** cross-family and carries the warning;
//! 2. the recommender-derived REVIEW entry ([`super::derived`]);
//! 3. otherwise the cheapest model the user actually has that is from a
//!    different family AND clears a capability floor scaled to the size of the
//!    change — a local model is $0 and wins outright when it qualifies;
//! 4. nothing different-family at all ⇒ [`ReviewerSelection::Unavailable`], which
//!    the gate turns into "Uncertain + parked", never a Pass.
//!
//! Spend is failed CLOSED: a model with no published price is refused rather
//! than billed as $0.00, and a Gate/Hard budget band refuses too. A refusal is
//! never an approval — see [`reviewer_spend_gate`].

use serde::{Deserialize, Serialize};

use super::budget::{BudgetBand, BudgetVerdict};
use super::knowledge::{lookup as lookup_model, ModelKnowledge};
use super::recommend::AvailableModel;
use super::role_map::RoleModel;

/// The orchestration-strength floor a different-family model must clear to be
/// picked as the reviewer for an ordinary change. Below this a model is not a
/// useful adversary — it produces vapor findings the grounding filter drops,
/// which costs a round and proves nothing.
pub const REVIEWER_MIN_ORCHESTRATION: f64 = 0.60;

/// The floor for a LARGE change. A big diff needs a reviewer that can hold it;
/// cheapness stops being the deciding axis once the thing under review is big.
pub const REVIEWER_STRONG_ORCHESTRATION: f64 = 0.75;

/// A diff at or below this many changed lines is "small" — the cheap/local end
/// of the ladder is enough, so the gate spends the least it can.
pub const SMALL_DIFF_LINES: usize = 200;

/// Where the reviewer came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewerSource {
    /// The operator's hand-configured REVIEW role (`PERMAGENT_ROLE_REVIEW_*`).
    Configured,
    /// The recommender-derived best-fit REVIEW entry.
    Derived,
    /// Chosen here: the cheapest capable different-family model available.
    BestFit,
}

impl ReviewerSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReviewerSource::Configured => "configured",
            ReviewerSource::Derived => "derived",
            ReviewerSource::BestFit => "best-fit",
        }
    }
}

/// The chosen reviewer, with everything the goal card needs to say WHO reviewed
/// and why it was allowed to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewerPick {
    pub provider: String,
    pub model: String,
    /// Vendor family, or the provider id when the knowledge base has no row.
    pub family: String,
    /// The author's family, as resolved the same way — recorded so the card can
    /// show the comparison rather than assert its conclusion.
    pub worker_family: String,
    pub source: ReviewerSource,
    /// THE property the gate rests on. False ⇒ the review still runs, but it is
    /// recorded as a same-family review and `warning` says so.
    pub cross_family: bool,
    /// Blended reference price (input + output per MTok); 0.0 for local.
    pub cost_hint_per_mtok: f64,
    /// Published input/output prices, when known — used to estimate one review.
    pub input_usd_per_mtok: f64,
    pub output_usd_per_mtok: f64,
    /// Whether the knowledge base had a price at all. False ⇒ the spend gate
    /// fails CLOSED (an unpriced model is not a free one).
    pub priced: bool,
    pub is_local: bool,
    /// One sentence for the card: how this reviewer was chosen.
    pub why: String,
    /// The diversity warning, when this review is not cross-family.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

impl ReviewerPick {
    /// `provider/model` for logs and surfaces.
    pub fn label(&self) -> String {
        format!("{}/{}", self.provider, self.model)
    }

    /// Estimated USD for ONE review at the given token counts. `None` when the
    /// model is unpriced (the honest answer — not $0.00). Local is `Some(0.0)`.
    pub fn estimate_cost_usd(&self, input_tokens: u64, output_tokens: u64) -> Option<f64> {
        if self.is_local {
            return Some(0.0);
        }
        if !self.priced {
            return None;
        }
        Some(
            (input_tokens as f64 / 1_000_000.0) * self.input_usd_per_mtok
                + (output_tokens as f64 / 1_000_000.0) * self.output_usd_per_mtok,
        )
    }
}

/// The outcome of choosing a reviewer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum ReviewerSelection {
    Reviewer(Box<ReviewerPick>),
    /// No model available can review this independently. The gate turns this
    /// into "Uncertain + parked" — never a Pass.
    Unavailable {
        reason: String,
    },
}

impl ReviewerSelection {
    pub fn pick(&self) -> Option<&ReviewerPick> {
        match self {
            ReviewerSelection::Reviewer(p) => Some(p),
            ReviewerSelection::Unavailable { .. } => None,
        }
    }
}

/// The family a (provider, model) pair belongs to. The knowledge base is the
/// authority; when it has no row the PROVIDER id stands in, because the vendor
/// is the coarse family and an unknown model from a different vendor is still
/// independent. Lowercased so the comparison is case-insensitive.
pub fn family_of(provider: &str, model: &str) -> String {
    match lookup_model(provider, model) {
        Some(k) => k.family.to_ascii_lowercase(),
        None => provider.trim().to_ascii_lowercase(),
    }
}

/// The message recorded when nothing can review the work independently.
pub const NO_REVIEWER_AVAILABLE: &str =
    "no model from a family other than the worker's is available to review this work — \
     the independent review could not run, so the goal is not marked complete. Add a \
     model from another vendor (see `permagent packs recommend`), or turn the \
     independent review off for this project if you accept unreviewed completions.";

/// Build a pick from a concrete (provider, model), resolving its family and
/// price from the knowledge base.
fn pick_from(
    provider: &str,
    model: &str,
    worker_family: &str,
    source: ReviewerSource,
    why: String,
) -> ReviewerPick {
    let kb: Option<&'static ModelKnowledge> = lookup_model(provider, model);
    let family = family_of(provider, model);
    let cross_family = !family.is_empty() && family != worker_family;
    let warning = (!cross_family).then(|| {
        format!(
            "the reviewer ({provider}/{model}) is from the SAME model family as the worker \
             ({worker_family}) — this is not an independent cross-family review, and it is \
             recorded as such. Add a model from another vendor so the review is genuinely \
             independent."
        )
    });
    ReviewerPick {
        provider: provider.to_string(),
        model: model.to_string(),
        family,
        worker_family: worker_family.to_string(),
        source,
        cross_family,
        cost_hint_per_mtok: kb.map(|k| k.blended_cost_per_mtok()).unwrap_or(0.0),
        input_usd_per_mtok: kb.map(|k| k.input_usd_per_mtok).unwrap_or(0.0),
        output_usd_per_mtok: kb.map(|k| k.output_usd_per_mtok).unwrap_or(0.0),
        priced: kb.is_some(),
        is_local: kb.map(|k| k.is_local).unwrap_or(false),
        why,
        warning,
    }
}

/// Pure: choose the reviewer for one completed goal.
///
/// `worker` is the (provider, model) that produced the work — `None` when the
/// dispatch record did not capture it, in which case NO family can be proven
/// different and every candidate is treated as same-family (honest, and it makes
/// the card say so rather than claim independence it cannot show).
///
/// `changed_lines` scales the capability floor: a small change may be reviewed
/// by the cheap/local end of the ladder; a large one may not.
pub fn select_reviewer(
    worker: Option<(&str, &str)>,
    configured: Option<&RoleModel>,
    derived: Option<&RoleModel>,
    available: &[AvailableModel],
    changed_lines: usize,
) -> ReviewerSelection {
    // No known author ⇒ no provable diversity. Use a sentinel family that
    // nothing matches so a candidate is never *claimed* cross-family, and say so.
    let (worker_family, worker_known) = match worker {
        Some((p, m)) => (family_of(p, m), true),
        None => (String::new(), false),
    };

    let diverse = |provider: &str, model: &str| -> bool {
        worker_known && family_of(provider, model) != worker_family
    };

    // 1. The operator's explicit mapping wins — even same-family, which is then
    //    recorded (never silently rerouted, and never silently called independent).
    if let Some(rm) = configured {
        if diverse(&rm.provider, &rm.model) {
            return ReviewerSelection::Reviewer(Box::new(pick_from(
                &rm.provider,
                &rm.model,
                &worker_family,
                ReviewerSource::Configured,
                "the model you mapped to the REVIEW role, from a different family than the \
                 worker"
                    .to_string(),
            )));
        }
    }

    // 2. The recommender-derived REVIEW entry, when it is genuinely diverse.
    if let Some(rm) = derived {
        if diverse(&rm.provider, &rm.model) {
            return ReviewerSelection::Reviewer(Box::new(pick_from(
                &rm.provider,
                &rm.model,
                &worker_family,
                ReviewerSource::Derived,
                "the recommender's best-fit REVIEW model among the models you have, from a \
                 different family than the worker"
                    .to_string(),
            )));
        }
    }

    // 3. Cheapest capable different-family model the user actually has.
    let floor = if changed_lines <= SMALL_DIFF_LINES {
        REVIEWER_MIN_ORCHESTRATION
    } else {
        REVIEWER_STRONG_ORCHESTRATION
    };
    let mut candidates: Vec<(&AvailableModel, &'static ModelKnowledge)> = available
        .iter()
        .filter_map(|a| lookup_model(&a.provider, &a.model).map(|k| (a, k)))
        .filter(|(a, k)| diverse(&a.provider, &a.model) && k.orchestration_strength >= floor)
        .collect();
    // Cheapest first (local is 0.0 and wins outright); ties break to the stronger
    // model, then to the label so the choice is deterministic.
    candidates.sort_by(|(a, ka), (b, kb)| {
        ka.blended_cost_per_mtok()
            .partial_cmp(&kb.blended_cost_per_mtok())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                kb.orchestration_strength
                    .partial_cmp(&ka.orchestration_strength)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.label().cmp(&b.label()))
    });
    if let Some((a, k)) = candidates.first() {
        let size = if changed_lines <= SMALL_DIFF_LINES {
            "small"
        } else {
            "large"
        };
        let price = if k.is_local {
            "on-device, $0".to_string()
        } else {
            format!("${:.2}/MTok blended", k.blended_cost_per_mtok())
        };
        return ReviewerSelection::Reviewer(Box::new(pick_from(
            &a.provider,
            &a.model,
            &worker_family,
            ReviewerSource::BestFit,
            format!(
                "the cheapest different-family model you have that clears the \
                 {size}-change capability floor ({price})"
            ),
        )));
    }

    // 4. Last resort: an explicitly-configured reviewer that happens to share the
    //    author's family is still a second opinion, and honouring the operator's
    //    mapping beats refusing outright — but it is recorded as same-family.
    if let Some(rm) = configured {
        return ReviewerSelection::Reviewer(Box::new(pick_from(
            &rm.provider,
            &rm.model,
            &worker_family,
            ReviewerSource::Configured,
            "the model you mapped to the REVIEW role — no different-family model is \
             available, so this review is not cross-family"
                .to_string(),
        )));
    }

    ReviewerSelection::Unavailable {
        reason: if worker_known {
            NO_REVIEWER_AVAILABLE.to_string()
        } else {
            format!(
                "the model that produced this work was not recorded, so no reviewer can be \
                 proven to come from a different family. {NO_REVIEWER_AVAILABLE}"
            )
        },
    }
}

// ── Spend, failed closed ─────────────────────────────────────────────────────

/// Whether the review may be paid for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum SpendDecision {
    Allow,
    /// The review must NOT run. The gate records Uncertain and parks — a spend
    /// refusal is never an approval.
    Refuse {
        reason: String,
    },
}

impl SpendDecision {
    pub fn refused(&self) -> Option<&str> {
        match self {
            SpendDecision::Allow => None,
            SpendDecision::Refuse { reason } => Some(reason),
        }
    }
}

/// Pure: may this review be paid for?
///
/// - a local model is $0 and always allowed (it cannot move a spend cap);
/// - an UNPRICED cloud model is REFUSED — billing it as $0.00 would make the cap
///   unenforceable, so the gate fails closed rather than spending blind;
/// - a Gate or Hard budget band refuses — the caps already say stop.
///
/// A Soft band proceeds: it is an alert, not a stop.
pub fn reviewer_spend_gate(pick: &ReviewerPick, verdict: &BudgetVerdict) -> SpendDecision {
    if pick.is_local {
        return SpendDecision::Allow;
    }
    if !pick.priced {
        return SpendDecision::Refuse {
            reason: format!(
                "the reviewer ({}) has no published price, so its spend cannot be counted \
                 against the cap — the review is refused rather than billed as $0.00. Add \
                 the model to the pricing knowledge base, or map the REVIEW role to a \
                 priced or on-device model.",
                pick.label()
            ),
        };
    }
    match verdict.band {
        BudgetBand::Gate => SpendDecision::Refuse {
            reason: format!(
                "the {} budget has reached its gate at ${:.2} (spent ${:.2}) — the \
                 independent review is not started until you raise it.",
                verdict.scope.word(),
                verdict.crossed,
                verdict.spent
            ),
        },
        BudgetBand::Hard => SpendDecision::Refuse {
            reason: format!(
                "the {} budget hard stop at ${:.2} has been reached (spent ${:.2}) — the \
                 independent review is not started.",
                verdict.scope.word(),
                verdict.crossed,
                verdict.spent
            ),
        },
        BudgetBand::Ok | BudgetBand::Soft => SpendDecision::Allow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cost_router::budget::{BudgetConfig, BudgetScope};
    use crate::cost_router::knowledge::KNOWN_MODELS;

    fn rm(provider: &str, model: &str) -> RoleModel {
        RoleModel {
            provider: provider.to_string(),
            model: model.to_string(),
        }
    }

    /// Two KB rows from DIFFERENT families, so the tests never hard-code a
    /// vendor: whatever the knowledge base actually ships, pick one row and the
    /// first row whose family differs.
    fn two_families() -> (&'static ModelKnowledge, &'static ModelKnowledge) {
        let a = KNOWN_MODELS.first().expect("knowledge base is not empty");
        let b = KNOWN_MODELS
            .iter()
            .find(|m| m.family != a.family)
            .expect("knowledge base has at least two families");
        (a, b)
    }

    fn a_local() -> Option<&'static ModelKnowledge> {
        KNOWN_MODELS.iter().find(|m| m.is_local)
    }

    #[test]
    fn family_falls_back_to_the_provider_when_the_model_is_unknown() {
        assert_eq!(family_of("Someco", "who-knows-1"), "someco");
    }

    #[test]
    fn configured_cross_family_reviewer_wins() {
        let (worker, reviewer) = two_families();
        let sel = select_reviewer(
            Some((worker.provider, worker.model)),
            Some(&rm(reviewer.provider, reviewer.model)),
            None,
            &[],
            50,
        );
        let pick = sel.pick().expect("a reviewer");
        assert_eq!(pick.source, ReviewerSource::Configured);
        assert!(pick.cross_family, "must be recorded as cross-family");
        assert_ne!(pick.family, pick.worker_family);
        assert!(pick.warning.is_none());
    }

    #[test]
    fn derived_review_entry_is_used_when_nothing_is_configured() {
        let (worker, reviewer) = two_families();
        let sel = select_reviewer(
            Some((worker.provider, worker.model)),
            None,
            Some(&rm(reviewer.provider, reviewer.model)),
            &[],
            50,
        );
        let pick = sel.pick().expect("a reviewer");
        assert_eq!(pick.source, ReviewerSource::Derived);
        assert!(pick.cross_family);
    }

    #[test]
    fn same_family_configured_reviewer_is_honoured_but_never_called_independent() {
        let (worker, _) = two_families();
        let sel = select_reviewer(
            Some((worker.provider, worker.model)),
            Some(&rm(worker.provider, worker.model)),
            None,
            &[],
            50,
        );
        let pick = sel.pick().expect("a reviewer");
        assert!(
            !pick.cross_family,
            "same family must not read as independent"
        );
        assert!(pick.warning.is_some(), "and it must say so");
    }

    #[test]
    fn best_fit_picks_the_cheapest_capable_different_family_model() {
        let (worker, _) = two_families();
        let worker_family = family_of(worker.provider, worker.model);
        let available: Vec<AvailableModel> = KNOWN_MODELS
            .iter()
            .map(|m| AvailableModel::new(m.provider, m.model))
            .collect();
        let sel = select_reviewer(
            Some((worker.provider, worker.model)),
            None,
            None,
            &available,
            10,
        );
        let pick = sel.pick().expect("a reviewer");
        assert_eq!(pick.source, ReviewerSource::BestFit);
        assert!(pick.cross_family);
        assert_ne!(pick.family, worker_family);

        // Nothing eligible is cheaper than what was chosen.
        let cheapest = KNOWN_MODELS
            .iter()
            .filter(|m| {
                family_of(m.provider, m.model) != worker_family
                    && m.orchestration_strength >= REVIEWER_MIN_ORCHESTRATION
            })
            .map(|m| m.blended_cost_per_mtok())
            .fold(f64::INFINITY, f64::min);
        assert!((pick.cost_hint_per_mtok - cheapest).abs() < 1e-9);
    }

    #[test]
    fn a_large_change_raises_the_capability_floor() {
        let (worker, _) = two_families();
        let worker_family = family_of(worker.provider, worker.model);
        let available: Vec<AvailableModel> = KNOWN_MODELS
            .iter()
            .map(|m| AvailableModel::new(m.provider, m.model))
            .collect();
        let sel = select_reviewer(
            Some((worker.provider, worker.model)),
            None,
            None,
            &available,
            SMALL_DIFF_LINES + 1,
        );
        let pick = sel.pick().expect("a reviewer");
        let kb = lookup_model(&pick.provider, &pick.model).expect("known");
        assert!(
            kb.orchestration_strength >= REVIEWER_STRONG_ORCHESTRATION,
            "a large change must not be reviewed below the strong floor"
        );
        assert_ne!(pick.family, worker_family);
    }

    #[test]
    fn no_different_family_model_is_unavailable_not_a_pass() {
        let (worker, _) = two_families();
        let available = vec![AvailableModel::new(worker.provider, worker.model)];
        let sel = select_reviewer(
            Some((worker.provider, worker.model)),
            None,
            None,
            &available,
            50,
        );
        assert!(matches!(sel, ReviewerSelection::Unavailable { .. }));
    }

    #[test]
    fn an_unrecorded_worker_model_can_never_be_proven_cross_family() {
        let available: Vec<AvailableModel> = KNOWN_MODELS
            .iter()
            .map(|m| AvailableModel::new(m.provider, m.model))
            .collect();
        let sel = select_reviewer(None, None, None, &available, 50);
        match sel {
            ReviewerSelection::Unavailable { reason } => {
                assert!(reason.contains("not recorded"));
            }
            ReviewerSelection::Reviewer(p) => {
                panic!("must not claim a reviewer: {}", p.label())
            }
        }
    }

    // ── Spend, failed closed ────────────────────────────────────────────────

    fn verdict(band: BudgetBand) -> BudgetVerdict {
        BudgetVerdict {
            band,
            scope: BudgetScope::Task,
            spent: 9.0,
            crossed: 5.0,
            unpriced_calls: 0,
        }
    }

    fn priced_pick(priced: bool, is_local: bool) -> ReviewerPick {
        ReviewerPick {
            provider: "p".into(),
            model: "m".into(),
            family: "f".into(),
            worker_family: "g".into(),
            source: ReviewerSource::BestFit,
            cross_family: true,
            cost_hint_per_mtok: 1.0,
            input_usd_per_mtok: 1.0,
            output_usd_per_mtok: 3.0,
            priced,
            is_local,
            why: String::new(),
            warning: None,
        }
    }

    #[test]
    fn an_unpriced_cloud_reviewer_is_refused_not_billed_as_zero() {
        let d = reviewer_spend_gate(&priced_pick(false, false), &verdict(BudgetBand::Ok));
        assert!(d.refused().expect("refused").contains("no published price"));
    }

    #[test]
    fn a_local_reviewer_runs_even_at_a_hard_stop() {
        assert_eq!(
            reviewer_spend_gate(&priced_pick(false, true), &verdict(BudgetBand::Hard)),
            SpendDecision::Allow
        );
    }

    #[test]
    fn a_gated_or_stopped_budget_refuses_the_review() {
        for band in [BudgetBand::Gate, BudgetBand::Hard] {
            assert!(
                reviewer_spend_gate(&priced_pick(true, false), &verdict(band))
                    .refused()
                    .is_some(),
                "{band:?} must refuse"
            );
        }
    }

    #[test]
    fn a_soft_alert_still_lets_the_review_run() {
        for band in [BudgetBand::Ok, BudgetBand::Soft] {
            assert_eq!(
                reviewer_spend_gate(&priced_pick(true, false), &verdict(band)),
                SpendDecision::Allow
            );
        }
    }

    #[test]
    fn one_review_is_estimated_in_dollars_or_honestly_not_at_all() {
        let priced = priced_pick(true, false);
        let est = priced.estimate_cost_usd(20_000, 500).expect("priced");
        // 20k in at $1/MTok + 500 out at $3/MTok.
        assert!((est - (0.02 + 0.0015)).abs() < 1e-9, "got {est}");
        assert_eq!(
            priced_pick(false, false).estimate_cost_usd(20_000, 500),
            None
        );
        assert_eq!(
            priced_pick(false, true).estimate_cost_usd(20_000, 500),
            Some(0.0)
        );
    }

    #[test]
    fn the_default_budget_config_is_not_needed_to_allow_a_local_review() {
        let cfg = BudgetConfig::default();
        let v = crate::cost_router::budget::budget_verdict(0.0, 0.0, &cfg);
        assert_eq!(
            reviewer_spend_gate(&priced_pick(false, true), &v),
            SpendDecision::Allow
        );
    }

    #[test]
    fn a_local_model_is_preferred_when_one_can_review_a_small_change() {
        let Some(local) = a_local() else { return };
        let worker = KNOWN_MODELS
            .iter()
            .find(|m| m.family != local.family)
            .expect("a non-local family");
        if local.orchestration_strength < REVIEWER_MIN_ORCHESTRATION {
            return; // no local model in the KB is a capable enough reviewer
        }
        let available = vec![
            AvailableModel::new(worker.provider, worker.model),
            AvailableModel::new(local.provider, local.model),
        ];
        let sel = select_reviewer(
            Some((worker.provider, worker.model)),
            None,
            None,
            &available,
            10,
        );
        let pick = sel.pick().expect("a reviewer");
        assert!(
            pick.is_local,
            "a $0 capable reviewer should win a small diff"
        );
    }
}
