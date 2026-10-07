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
use std::collections::BTreeSet;

use super::budget::{BudgetBand, BudgetVerdict};
use super::knowledge::{
    lookup as lookup_model, lookup_with_confidence, LookupConfidence, ModelKnowledge,
};
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
    /// Legacy author-family display. The contributor-set API records the sorted,
    /// comma-joined set here for compatibility. This display is never evidence
    /// from which contributor exclusions may be reconstructed.
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

/// Complete native contributor history, or history whose completeness/families
/// cannot be established. Callers must obtain this from protected physical-call
/// evidence, never model arguments or the latest worker route alone.
#[derive(Debug, Clone, Copy)]
pub enum ContributorFamilies<'a> {
    /// An empty set is valid only with native proof of no model editing
    /// contributors; absent history must be `Unknown` instead.
    Known(&'a BTreeSet<String>),
    Unknown,
}

/// A model family established by a knowledge row or an explicit KB alias.
/// Provider-id fallback, opaque runtime labels (including KB `ollama` rows),
/// and approximate local tag-prefix matches cannot prove the physical family
/// of an unknown model behind a proxy or alias.
pub fn known_physical_family_of(provider: &str, model: &str) -> Option<String> {
    let (row, confidence) = lookup_with_confidence(provider, model)?;
    let family = row.family.trim().to_ascii_lowercase();
    if confidence == LookupConfidence::FamilyEstimate || !physical_family_label(&family) {
        return None;
    }
    Some(family)
}

// These identify runtimes/transports or missing identity, not model producers.
fn physical_family_label(family: &str) -> bool {
    !matches!(
        family,
        "" | "ollama"
            | "local"
            | "openrouter"
            | "unknown"
            | "custom"
            | "proxy"
            | "openai-compatible"
            | "openai_compatible"
            | "vllm"
            | "llamacpp"
            | "llama.cpp"
            | "lmstudio"
            | "localai"
    )
}

/// Choose a reviewer independent of ALL contributors. Unlike the legacy API,
/// configured conflicts are not a fallback and unknown history fails closed.
/// This proves family exclusion only. Budget/runtime eligibility, empirical
/// local-model qualification, and rechecking the actual physical reviewer route
/// remain caller obligations before dispatch and before accepting a verdict.
pub fn select_reviewer_for_contributors(
    contributors: ContributorFamilies<'_>,
    configured: Option<&RoleModel>,
    derived: Option<&RoleModel>,
    available: &[AvailableModel],
    changed_lines: usize,
) -> ReviewerSelection {
    let ContributorFamilies::Known(families) = contributors else {
        return ReviewerSelection::Unavailable {
            reason: "complete trusted contributor family history is unavailable; an independent reviewer cannot be selected".into(),
        };
    };
    let families: BTreeSet<String> = families
        .iter()
        .map(|family| family.trim().to_ascii_lowercase())
        .collect();
    if families.iter().any(|family| !physical_family_label(family)) {
        return ReviewerSelection::Unavailable {
            reason: "trusted contributor history contains an unknown family; an independent reviewer cannot be selected".into(),
        };
    }
    let display = families.iter().cloned().collect::<Vec<_>>().join(",");
    let mut selection = select_reviewer_with_policy(
        FamilyPolicy {
            display: display.clone(),
            history_known: true,
            allow_configured_conflict: false,
            eligible: |provider: &str, model: &str| {
                !model_is_retired(provider, model)
                    && known_physical_family_of(provider, model)
                        .is_some_and(|family| !families.contains(&family))
            },
        },
        configured,
        derived,
        available,
        changed_lines,
    );
    match &mut selection {
        ReviewerSelection::Reviewer(pick) => {
            pick.cross_family = true; // Only the complete structured set proved this.
            pick.warning = None;
            pick.why = format!(
                "{}; known reviewer family {} is outside the complete contributor set [{}]",
                pick.why, pick.family, pick.worker_family
            );
        }
        ReviewerSelection::Unavailable { reason } => {
            *reason = format!(
                "no eligible known reviewer outside the complete contributor set [{display}] is available; the independent review could not run and the goal must remain parked"
            );
        }
    }
    selection
}

struct FamilyPolicy<F> {
    display: String,
    history_known: bool,
    allow_configured_conflict: bool,
    eligible: F,
}

/// The message recorded when nothing can review the work independently.
pub const NO_REVIEWER_AVAILABLE: &str =
    "no model from a family other than the worker's is available to review this work — \
     the independent review could not run, so the goal is not marked complete. Add a \
     model from another vendor (see `permagent packs recommend`), or turn the \
     independent review off for this project if you accept unreviewed completions.";

/// Anthropic API ids that 404 as of 2026-08. A configured REVIEW pin of one of
/// these was honouring the operator's mapping by calling a dead model, which
/// took the independent-review gate down (`claude-3-5-sonnet-20241022`).
/// Skipping them lets best-fit (or a live configured model) take over.
const RETIRED_ANTHROPIC_REVIEW_MODELS: &[&str] = &[
    "claude-3-5-sonnet-20241022",
    "claude-3-5-sonnet-20240620",
    "claude-3-5-haiku-20241022",
    "claude-3-opus-20240229",
    "claude-3-sonnet-20240229",
    "claude-3-haiku-20240307",
];

/// True when this (provider, model) will 404 if the reviewer actually calls it.
pub fn model_is_retired(provider: &str, model: &str) -> bool {
    provider.trim().eq_ignore_ascii_case("anthropic")
        && RETIRED_ANTHROPIC_REVIEW_MODELS
            .iter()
            .any(|id| id.eq_ignore_ascii_case(model.trim()))
}

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
    let (worker_family, worker_known) = match worker {
        Some((p, m)) => (family_of(p, m), true),
        None => (String::new(), false),
    };
    select_reviewer_with_policy(
        FamilyPolicy {
            display: worker_family.clone(),
            history_known: worker_known,
            allow_configured_conflict: true,
            eligible: |provider: &str, model: &str| {
                worker_known && family_of(provider, model) != worker_family
            },
        },
        configured,
        derived,
        available,
        changed_lines,
    )
}

fn select_reviewer_with_policy(
    policy: FamilyPolicy<impl Fn(&str, &str) -> bool>,
    configured: Option<&RoleModel>,
    derived: Option<&RoleModel>,
    available: &[AvailableModel],
    changed_lines: usize,
) -> ReviewerSelection {
    let worker_family = policy.display;
    let worker_known = policy.history_known;
    let diverse = policy.eligible;

    // 1. The operator's explicit mapping wins — even same-family, which is then
    //    recorded (never silently rerouted, and never silently called independent).
    //    A retired id is not honoured: calling it 404s the gate.
    if let Some(rm) = configured {
        if !model_is_retired(&rm.provider, &rm.model) && diverse(&rm.provider, &rm.model) {
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
        if !model_is_retired(&rm.provider, &rm.model) && diverse(&rm.provider, &rm.model) {
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
    //    Retired ids are still skipped: a 404 is not a second opinion.
    if let Some(rm) = configured.filter(|_| policy.allow_configured_conflict) {
        if !model_is_retired(&rm.provider, &rm.model) {
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

/// Lane R63c: the cross-family candidate a REVIEW DISPATCH (`delegate` with
/// `worker_persona: "reviewer"`) escalates to when no dedicated review pin
/// decided it — the cheapest different-family model the user actually has
/// that clears the size-scaled capability floor ([`select_reviewer`]'s
/// BestFit step, with the configured and derived steps deliberately skipped).
///
/// Why not the derived map's REVIEW entry: the recommender picks REVIEW as the
/// *strongest* different-family verifier (`recommend::review_rank`), not the
/// cheapest capable one, so with an `ANTHROPIC_API_KEY` present it names
/// `claude-fable-5` — the priciest row in the knowledge base. M47 r63 s5
/// (`dschat`, every pack pinned to DeepSeek): the coordinator's reviewer
/// delegate ran five turns on `anthropic/claude-fable-5` for $1.97, ~85% of
/// the record's spend, for a one-line diff that `claude-haiku-4-5` clears the
/// small-change floor for. Review keeps its cross-family requirement (the
/// verification gate counts `reviewer_not_independent`); it just stops buying
/// the frontier model to satisfy it.
///
/// `None` when nothing different-family clears the floor, or the author is
/// unknown (no family can be proven different) — the caller's same-family
/// fallback then applies, honestly labeled.
///
/// Lane N3 (M48 r64 s5, ruling 2026-09-17): the reviewer delegate and the
/// reviewer mandate no longer call this. Its size-only floor read `git diff
/// HEAD`, which is 0 after an auto-land, so a complex change got the cheapest
/// reviewer; they now use `orchestrator::assign_delegate_reviewer`
/// (`recommend::assign_reviewer` over the goal's trunk-relative diff).
pub fn cheapest_cross_family_reviewer(
    author: Option<(&str, &str)>,
    available: &[AvailableModel],
    changed_lines: usize,
) -> Option<RoleModel> {
    select_reviewer(author, None, None, available, changed_lines)
        .pick()
        .filter(|pick| pick.cross_family)
        .map(|pick| RoleModel {
            provider: pick.provider.clone(),
            model: pick.model.clone(),
        })
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

    fn contributor_set(families: &[&str]) -> BTreeSet<String> {
        families
            .iter()
            .map(|family| (*family).to_string())
            .collect()
    }

    #[test]
    fn contributor_set_excludes_both_authors_on_all_selection_paths() {
        let families = contributor_set(&["anthropic", "openai"]);
        let available: Vec<AvailableModel> = KNOWN_MODELS
            .iter()
            .map(|model| AvailableModel::new(model.provider, model.model))
            .collect();
        let selection = select_reviewer_for_contributors(
            ContributorFamilies::Known(&families),
            Some(&rm("anthropic", "claude-sonnet-5")),
            Some(&rm("openai", "gpt-5.4-mini")),
            &available,
            10,
        );
        let pick = selection
            .pick()
            .expect("an unrelated capable family exists");
        assert!(!families.contains(&pick.family));
        assert_eq!(pick.source, ReviewerSource::BestFit);
        assert_eq!(pick.worker_family, "anthropic,openai");
        assert!(pick.cross_family);
    }

    #[test]
    fn contributor_set_conflicting_explicit_pin_cannot_be_a_last_resort() {
        let families = contributor_set(&["openai"]);
        let selection = select_reviewer_for_contributors(
            ContributorFamilies::Known(&families),
            Some(&rm("openai", "gpt-5.4-mini")),
            None,
            &[],
            10,
        );
        assert!(matches!(selection, ReviewerSelection::Unavailable { .. }));
    }

    #[test]
    fn contributor_set_refuses_unknown_history_even_with_explicit_reviewer() {
        let selection = select_reviewer_for_contributors(
            ContributorFamilies::Unknown,
            Some(&rm("anthropic", "claude-sonnet-5")),
            None,
            &[],
            10,
        );
        assert!(matches!(selection, ReviewerSelection::Unavailable { .. }));
    }

    #[test]
    fn contributor_set_refuses_proxy_provider_fallback_and_approximate_alias_family() {
        assert_eq!(
            known_physical_family_of("openrouter", "mystery-model"),
            None
        );
        assert_eq!(
            known_physical_family_of("ollama", "qwen3:unproven-custom-tag"),
            None
        );
        let families = contributor_set(&["openai"]);
        let selection = select_reviewer_for_contributors(
            ContributorFamilies::Known(&families),
            Some(&rm("openrouter", "mystery-model")),
            Some(&rm("ollama", "qwen3:unproven-custom-tag")),
            &[AvailableModel::new("openrouter", "mystery-model")],
            10,
        );
        assert!(matches!(selection, ReviewerSelection::Unavailable { .. }));
    }

    #[test]
    fn contributor_set_no_eligible_available_family_parks() {
        let available: Vec<AvailableModel> = KNOWN_MODELS
            .iter()
            .map(|model| AvailableModel::new(model.provider, model.model))
            .collect();
        let families: BTreeSet<String> = KNOWN_MODELS
            .iter()
            .map(|model| model.family.to_ascii_lowercase())
            .collect();
        assert!(matches!(
            select_reviewer_for_contributors(
                ContributorFamilies::Known(&families),
                None,
                None,
                &available,
                10
            ),
            ReviewerSelection::Unavailable { .. }
        ));
    }

    #[test]
    fn contributor_set_current_known_local_below_capability_floor_is_refused() {
        let families = contributor_set(&["anthropic", "openai"]);
        let local = lookup_model("qwen38_split", "qwen3.8-27b")
            .expect("the current local split model has a KB row");
        assert!(local.is_local);
        assert!(local.orchestration_strength < REVIEWER_MIN_ORCHESTRATION);
        assert_eq!(
            known_physical_family_of(local.provider, local.model),
            Some("qwen".into())
        );
        assert!(matches!(
            select_reviewer_for_contributors(
                ContributorFamilies::Known(&families),
                None,
                None,
                &[AvailableModel::new(local.provider, local.model)],
                10,
            ),
            ReviewerSelection::Unavailable { .. }
        ));
    }

    #[test]
    fn contributor_set_opaque_runtime_kb_family_never_proves_independence() {
        assert!(lookup_model("ollama", "qwen3-coder").is_some());
        assert_eq!(known_physical_family_of("ollama", "qwen3-coder"), None);
        let families = contributor_set(&["openai"]);
        assert!(matches!(
            select_reviewer_for_contributors(
                ContributorFamilies::Known(&families),
                Some(&rm("ollama", "qwen3-coder")),
                None,
                &[],
                10,
            ),
            ReviewerSelection::Unavailable { .. }
        ));
        let opaque_contributor = contributor_set(&["ollama"]);
        assert!(matches!(
            select_reviewer_for_contributors(
                ContributorFamilies::Known(&opaque_contributor),
                Some(&rm("openai", "gpt-5.4-mini")),
                None,
                &[],
                10,
            ),
            ReviewerSelection::Unavailable { .. }
        ));
    }

    #[test]
    fn contributor_set_known_empty_is_distinct_from_unknown_or_malformed_history() {
        let families = BTreeSet::new();
        assert!(select_reviewer_for_contributors(
            ContributorFamilies::Known(&families),
            Some(&rm("openai", "gpt-5.4-mini")),
            None,
            &[],
            10,
        )
        .pick()
        .is_some());
        let malformed = contributor_set(&[""]);
        assert!(select_reviewer_for_contributors(
            ContributorFamilies::Known(&malformed),
            Some(&rm("openai", "gpt-5.4-mini")),
            None,
            &[],
            10,
        )
        .pick()
        .is_none());
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
    fn a_retired_configured_reviewer_is_skipped_so_best_fit_can_run() {
        let worker = lookup_model("openai", "gpt-5.4-mini").expect("gpt-5.4-mini is in the KB");
        let available: Vec<AvailableModel> = KNOWN_MODELS
            .iter()
            .map(|m| AvailableModel::new(m.provider, m.model))
            .collect();
        let sel = select_reviewer(
            Some((worker.provider, worker.model)),
            Some(&rm("anthropic", "claude-3-5-sonnet-20241022")),
            None,
            &available,
            10,
        );
        let pick = sel.pick().expect("best-fit must replace the dead pin");
        assert_ne!(pick.model, "claude-3-5-sonnet-20241022");
        assert_eq!(pick.source, ReviewerSource::BestFit);
        assert!(pick.cross_family);
    }

    #[test]
    fn a_retired_configured_reviewer_with_no_alternative_is_unavailable() {
        let worker = lookup_model("openai", "gpt-5.4-mini").expect("gpt-5.4-mini is in the KB");
        let sel = select_reviewer(
            Some((worker.provider, worker.model)),
            Some(&rm("anthropic", "claude-3-5-sonnet-20241022")),
            None,
            &[],
            10,
        );
        assert!(
            matches!(sel, ReviewerSelection::Unavailable { .. }),
            "a 404 pin must not be called as a last resort, got {sel:?}"
        );
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

    /// Lane R63c (M47 r63 s5): a DeepSeek author with the Anthropic line
    /// available. The derived map's REVIEW entry is the strongest
    /// different-family verifier (`claude-fable-5`); the review DISPATCH
    /// candidate must instead be the cheapest different-family model that
    /// clears the size-scaled floor — never the frontier row for a one-line
    /// diff, and still never the author's own family.
    #[test]
    fn the_review_dispatch_candidate_is_the_cheapest_capable_cross_family_model_not_the_strongest()
    {
        let author = ("custom_deepseek", "deepseek-chat");
        let available = vec![
            AvailableModel::new("custom_deepseek", "deepseek-chat"),
            AvailableModel::new("anthropic", "claude-fable-5"),
            AvailableModel::new("anthropic", "claude-sonnet-5"),
            AvailableModel::new("anthropic", "claude-haiku-4-5-20251001"),
        ];
        let cheapest_price = |floor: f64| {
            available
                .iter()
                .filter_map(|a| lookup_model(&a.provider, &a.model))
                .filter(|k| k.family != "deepseek" && k.orchestration_strength >= floor)
                .map(|k| k.blended_cost_per_mtok())
                .fold(f64::INFINITY, f64::min)
        };

        let small = cheapest_cross_family_reviewer(Some(author), &available, 1)
            .expect("a cross-family model clears the small-change floor");
        assert_ne!(
            small.model, "claude-fable-5",
            "never the frontier row by default"
        );
        assert_ne!(family_of(&small.provider, &small.model), "deepseek");
        let small_kb = lookup_model(&small.provider, &small.model).expect("known");
        assert!(
            (small_kb.blended_cost_per_mtok() - cheapest_price(REVIEWER_MIN_ORCHESTRATION)).abs()
                < 1e-9
        );

        let large = cheapest_cross_family_reviewer(Some(author), &available, SMALL_DIFF_LINES + 1)
            .expect("a cross-family model clears the large-change floor");
        let large_kb = lookup_model(&large.provider, &large.model).expect("known");
        assert!(large_kb.orchestration_strength >= REVIEWER_STRONG_ORCHESTRATION);
        assert!(
            (large_kb.blended_cost_per_mtok() - cheapest_price(REVIEWER_STRONG_ORCHESTRATION))
                .abs()
                < 1e-9
        );
        assert_ne!(large.model, "claude-fable-5");

        // Only the author's own family available ⇒ no cross-family candidate
        // (the caller's honestly-labeled same-family fallback applies).
        assert_eq!(
            cheapest_cross_family_reviewer(
                Some(author),
                &[AvailableModel::new("custom_deepseek", "deepseek-chat")],
                1
            ),
            None
        );
        // Unknown author ⇒ nothing can be proven cross-family.
        assert_eq!(cheapest_cross_family_reviewer(None, &available, 1), None);
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
