//! The objective recommender — given the models a user actually has, recommend
//! the best fit for each workflow role, purely from the measured
//! [`super::knowledge`] attributes and cost, with **zero vendor bias**.
//!
//! This is the heart of the standing rule: *no model is set by default; the agent
//! asks which models the user has access to and recommends which fit best where,
//! with no bias toward the vendor whose runtime this is — pure objectivity and
//! cost awareness.* Every pick here is a pure function of the data; the code has
//! no "prefer Anthropic" (or any vendor) branch, and the no-bias property test
//! ([`tests::non_anthropic_wins_edit_when_it_is_objectively_better`]) proves it.
//!
//! ## Roles (token-mass × judgment-density + cost)
//!
//! - **ORCHESTRATE** — the judgment-dense main loop; strongest available at
//!   orchestration / long-horizon (few tokens, high stakes), cost deciding only
//!   inside [`super::capability_directory::PLANNER_STRENGTH_BAND`] — the same
//!   rule as [`super::capability_directory::CapabilityDirectory::planner`].
//! - **EDIT** — the highest token-mass role; needs diff-format reliability.
//!   The cheapest model within [`EDIT_RELIABILITY_BAND`] of the most reliable
//!   editor ([`cheapest_within_band`]); flagged if the best available is below
//!   the ~97% diff-format floor.
//! - **MECHANICAL** — search/read/summarize; the cheapest model that clears the
//!   role's capability floor, preferring local at equal cost.
//! - **REVIEW** — verify; strong, and a *different family* than EDIT for
//!   diversity when the user has more than one; cost decides inside
//!   [`REVIEW_STRENGTH_BAND`].
//!
//! ## Proportionality (N1 finding 1, rulings 2026-08-18 / 2026-09-09)
//!
//! The strength-ranked roles (ORCHESTRATE, EDIT, REVIEW) never buy a price
//! multiple for a rounding-sized capability edge: among the floor-clearers,
//! every candidate within the role's named band of the strongest is treated as
//! equally fit and the cheapest of them wins ([`cheapest_within_band`], the one
//! helper the directory's `planner()` also uses). Outside the band strength
//! still wins, whatever it costs. Before this, EDIT sorted strictly by
//! diff-format reliability, so a 0.5-point edge bought a 3.3x price.
//! - **LOCAL** — on-device ($0, private); the best local model, if any.
//!
//! ## Capability floors ("best fit", not "cheapest-then-climb")
//!
//! Every role carries a [`CapabilityFloor`] ([`WorkflowRole::capability_floor`]).
//! The recommender first partitions the available models into those that clear
//! the floor and those below it, then applies the role's ranking among the
//! floor-clearers — so MECHANICAL is "the cheapest model that can do the job",
//! not "the cheapest model". When nothing clears, it falls back to the best
//! available by the same rank, sets [`RoleRecommendation::floor_met`] to `false`
//! and says so in the warnings; a caller never gets a silent under-fit.
//!
//! ## Cache discipline
//!
//! Prompt caches are provider+model-scoped, so the cache-heavy roles
//! (ORCHESTRATE, EDIT) carry a warning when the recommended model's provider has
//! no prompt caching — routing a token-heavy loop to a non-caching provider
//! throws away the warm-cache saving.
//!
//! ## Purity
//!
//! [`recommend`] and [`available_from`] are pure. Only [`discover_available_models`]
//! / [`discover_available_models_async`] and [`recommend_configured`] /
//! [`recommend_configured_async`] touch config/registry/network, each a thin
//! wrapper over the pure core (mirroring [`super::cheap`] and [`super::budget`]).
//! The async variants additionally probe the local Ollama daemon for the models
//! actually pulled ([`discover_ollama_models_async`]); the sync ones stay
//! network-free for pure tests and sync callers.

use std::cmp::Ordering;

use serde::Serialize;

use super::knowledge::{lookup, lookup_with_confidence, LookupConfidence, ModelKnowledge};

/// The diff-format reliability an EDIT model should clear. Below this, edits are
/// more likely to fail to apply and need retries — the recommender still picks
/// the best available but flags it.
pub const EDIT_RELIABILITY_FLOOR: f64 = 0.97;

/// How much orchestration strength REVIEW may trade away for family diversity. A
/// different-family model earns the "cross-family reviewer" preference only when
/// its orchestration strength is within this gap of the strongest available
/// orchestrator; a materially-weaker different-family model (e.g. a small local)
/// does NOT displace a much stronger same-family verifier just for diversity (F2).
pub const REVIEW_DIVERSITY_MAX_STRENGTH_GAP: f64 = 0.10;

/// How much diff-format reliability EDIT may trade away for a cheaper model
/// (N1 finding 1). Reliability is a success rate, so this band is at most one
/// extra failed edit per hundred — a retry tax far below any real price gap
/// between editors. Tighter than [`super::capability_directory::
/// PLANNER_STRENGTH_BAND`] (0.03) on purpose: the EDIT floor
/// ([`EDIT_RELIABILITY_FLOOR`]) sits only three points below perfect, so a
/// 0.03 band would make every floor-clearer "equal" and discard the measured
/// metric, turning EDIT into plain cheapest-that-clears.
pub const EDIT_RELIABILITY_BAND: f64 = 0.01;

/// How much orchestration strength REVIEW may trade away for a cheaper
/// verifier, inside the partition the diversity rule prefers. The same value
/// as the planner band: reviewing is orchestration-shaped work a notch below
/// planning, so it tolerates at least the planner's near-tie.
pub const REVIEW_STRENGTH_BAND: f64 = super::capability_directory::PLANNER_STRENGTH_BAND;

/// Float slack for band comparisons, so `0.985 - 0.01` still admits `0.975`.
const BAND_EPSILON: f64 = 1e-9;

/// The result of [`cheapest_within_band`]: the chosen candidate and the
/// strongest one it was measured against (equal when no cheaper candidate sat
/// inside the band).
#[derive(Debug, Clone, Copy)]
pub struct BandedPick<'a, T> {
    pub pick: &'a T,
    pub strongest: &'a T,
}

/// The proportionality rule shared by every strength-ranked pick (EDIT,
/// ORCHESTRATE and REVIEW here, and
/// [`super::capability_directory::CapabilityDirectory::planner`]): find the
/// strongest candidate, then among every candidate whose strength is within
/// `band` of it pick the CHEAPEST — stronger, then `id_order`, breaking an
/// exact cost tie. A local $0 model competes like any other once it is in the
/// candidate list. `None` only for an empty list. Candidates must already have
/// cleared the role's floor. This is [`cheapest_clearing_floor`] with the floor
/// anchored at `strongest - band` and no near-floor zone — ONE selection rule
/// for the bands and the review assignment.
pub fn cheapest_within_band<'a, T>(
    candidates: &[&'a T],
    band: f64,
    strength: impl Fn(&T) -> f64,
    cost: impl Fn(&T) -> f64,
    id_order: impl Fn(&T, &T) -> Ordering,
) -> Option<BandedPick<'a, T>> {
    let strongest: &'a T = *candidates.iter().max_by(|a, b| {
        strength(**a)
            .total_cmp(&strength(**b))
            .then_with(|| id_order(**a, **b))
    })?;
    let floor = strength(strongest) - band;
    let chosen = cheapest_clearing_floor(candidates, floor, 0.0, strength, cost, id_order)?;
    Some(BandedPick {
        pick: chosen.pick,
        strongest,
    })
}

// ── Proportional floor selection (ruling 2026-09-17, reviewer selection) ─────
//
// The user's ruling, 2026-09-17: "it is fine to use it if it is objectively true that its
// review will be magnitudes better than a cheaper model, but that has to be
// proven at the time of selection by assessing the complexity of the review and
// doing an objective assignment to the right model. Being better than half a
// percent is not enough to warrant a 3x cost."

/// How far below a capability floor a candidate may sit and still compete as
/// "at the floor". Smaller than [`super::capability_directory::
/// PLANNER_STRENGTH_BAND`] (0.03, the scale's near-tie): a gap this small is
/// inside the noise of the benchmark-normalized scores the floors are drawn
/// on, so it is not evidence the cheaper model is less able. A candidate MORE
/// than this below the floor falls MATERIALLY short and is not eligible.
pub const MATERIAL_FLOOR_SHORTFALL: f64 = 0.02;

/// `k` in the proportionality rule: a pricier candidate's strength gain over a
/// cheaper one must be at least `k * ln(price_ratio)`
/// ([`upgrade_is_proportionate`]). Logarithmic because each doubling of price
/// should demand the same further gain, not an ever-larger one (a linear rule
/// would forbid the frontier outright, a flat one would let any edge buy any
/// price). `0.05` sets a 2x price at a gain of 0.035 — just past the 0.03
/// near-tie band, so doubling the price needs a real difference — 3x at 0.055,
/// 10x at 0.115, 30x at 0.17 (roughly the whole spread between the ORCHESTRATE
/// floor and today's strongest model). A 0.005 edge at 3x fails by 11x.
pub const PROPORTIONALITY_K: f64 = 0.05;

/// The $/Mtok a price ratio treats as its minimum, so a $0 local model yields a
/// finite (very large) ratio instead of dividing by zero.
pub const PRICE_RATIO_MIN_COST_PER_MTOK: f64 = 0.10;

/// Whether a `strength_gain` justifies paying `pricier_cost` over
/// `cheaper_cost` ($/Mtok): `gain >= PROPORTIONALITY_K * ln(price_ratio)`. A
/// candidate that is not actually pricier needs only a non-negative gain.
pub fn upgrade_is_proportionate(strength_gain: f64, cheaper_cost: f64, pricier_cost: f64) -> bool {
    strength_gain + BAND_EPSILON >= required_gain_for_price(cheaper_cost, pricier_cost)
}

/// The strength gain [`upgrade_is_proportionate`] requires for this price step.
pub fn required_gain_for_price(cheaper_cost: f64, pricier_cost: f64) -> f64 {
    let ratio = pricier_cost.max(PRICE_RATIO_MIN_COST_PER_MTOK)
        / cheaper_cost.max(PRICE_RATIO_MIN_COST_PER_MTOK);
    if ratio <= 1.0 {
        0.0
    } else {
        PROPORTIONALITY_K * ratio.ln()
    }
}

/// The result of [`cheapest_clearing_floor`].
#[derive(Debug, Clone, Copy)]
pub struct FloorPick<'a, T> {
    pub pick: &'a T,
    /// Whether `pick` clears the floor outright (false = within
    /// [`MATERIAL_FLOOR_SHORTFALL`] below it and no pricier candidate's
    /// advantage was proportionate).
    pub clears_floor: bool,
    /// The cheaper near-floor candidate `pick` displaced on a proportionate
    /// advantage, if any.
    pub displaced: Option<&'a T>,
}

/// THE selection rule (bands, planner, review): among candidates within
/// `material_shortfall` of `floor`, walk cheapest first (ties: stronger, then
/// `id_order`) and take the first that CLEARS the floor. A pricier candidate
/// wins only when every cheaper one falls short: one that falls materially short
/// is not eligible at all, and one that falls short only within the margin is
/// displaced by a pricier floor-clearer only when that candidate's advantage is
/// proportionate to its price ([`upgrade_is_proportionate`]). `None` when no
/// candidate is eligible.
pub fn cheapest_clearing_floor<'a, T>(
    candidates: &[&'a T],
    floor: f64,
    material_shortfall: f64,
    strength: impl Fn(&T) -> f64,
    cost: impl Fn(&T) -> f64,
    id_order: impl Fn(&T, &T) -> Ordering,
) -> Option<FloorPick<'a, T>> {
    let clears = |c: &T| strength(c) >= floor - BAND_EPSILON;
    let mut eligible: Vec<&'a T> = candidates
        .iter()
        .copied()
        .filter(|c| strength(*c) >= floor - material_shortfall - BAND_EPSILON)
        .collect();
    eligible.sort_by(|a, b| {
        cost(*a)
            .total_cmp(&cost(*b))
            .then_with(|| strength(*b).total_cmp(&strength(*a)))
            .then_with(|| id_order(*a, *b))
    });
    let mut pick: &'a T = *eligible.first()?;
    let mut displaced = None;
    for candidate in eligible.iter().copied().skip(1) {
        if clears(pick) {
            break;
        }
        if clears(candidate)
            && upgrade_is_proportionate(
                strength(candidate) - strength(pick),
                cost(pick),
                cost(candidate),
            )
        {
            displaced = Some(pick);
            pick = candidate;
        }
    }
    Some(FloorPick {
        pick,
        clears_floor: clears(pick),
        displaced,
    })
}

/// A workflow role in the coding harness. The recommender maps each to a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRole {
    /// The judgment-dense main loop.
    Orchestrate,
    /// The token-heavy editing role — needs diff-format reliability.
    Edit,
    /// Cheap mechanical work: search / read / summarize.
    Mechanical,
    /// Verification; prefer a different family than EDIT for diversity.
    Review,
    /// On-device ($0, private).
    Local,
}

impl WorkflowRole {
    /// Every role, in workflow order.
    pub fn all() -> [WorkflowRole; 5] {
        [
            WorkflowRole::Orchestrate,
            WorkflowRole::Edit,
            WorkflowRole::Mechanical,
            WorkflowRole::Review,
            WorkflowRole::Local,
        ]
    }

    /// Stable label for logs / config keys / surfaces.
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkflowRole::Orchestrate => "orchestrate",
            WorkflowRole::Edit => "edit",
            WorkflowRole::Mechanical => "mechanical",
            WorkflowRole::Review => "review",
            WorkflowRole::Local => "local",
        }
    }

    /// Parse a role from an operator-supplied tag / config value, case-insensitively
    /// and accepting the common alias `hard` for [`WorkflowRole::Orchestrate`] (the
    /// frontier-reasoning role's name in the tier ladder, [`super::packs::Role::Hard`]).
    /// `None` for an unrecognized tag. Used by `WorkerPersona::workflow_role`
    /// overrides and `permagent packs set --role`.
    pub fn from_tag(tag: &str) -> Option<WorkflowRole> {
        match tag.trim().to_ascii_lowercase().as_str() {
            // R48f (G33): `cloud` is the roster's own word for the non-local
            // worker rung, and `default_worker_tier_for_goal` returns
            // `Orchestrate` for multi-file work — so a roster value of
            // `local_or_cloud` round-trips back through this parser.
            "orchestrate" | "hard" | "plan" | "planning" | "cloud" => {
                Some(WorkflowRole::Orchestrate)
            }
            "edit" | "code_edit" => Some(WorkflowRole::Edit),
            "mechanical" | "read_only" => Some(WorkflowRole::Mechanical),
            "review" | "verify" => Some(WorkflowRole::Review),
            "local" => Some(WorkflowRole::Local),
            _ => None,
        }
    }

    /// One-line description of what the role is for.
    pub fn description(&self) -> &'static str {
        match self {
            WorkflowRole::Orchestrate => "judgment-dense main loop / planning",
            WorkflowRole::Edit => "token-heavy code editing (diff-format reliability)",
            WorkflowRole::Mechanical => "cheap search / read / summarize",
            WorkflowRole::Review => "verification (prefer a different family)",
            WorkflowRole::Local => "on-device, $0, private",
        }
    }

    /// Whether this role runs a cache-heavy loop (so a non-caching provider is
    /// a warning). ORCHESTRATE and EDIT hold long, warm prefixes; MECHANICAL /
    /// REVIEW / LOCAL are short, isolated sub-work. Public so the live dispatch
    /// cache guard (`summon` / goal engine) fires on the same rule the recommender
    /// warns on.
    pub fn is_cache_heavy(&self) -> bool {
        matches!(self, WorkflowRole::Orchestrate | WorkflowRole::Edit)
    }

    /// The minimum measured capability a model must clear to be recommended for
    /// this role without a warning. The recommender ranks ONLY among the models
    /// that clear the floor (cheapest-that-clears for MECHANICAL, strongest for
    /// ORCHESTRATE, …) and falls back — flagged — to the best available when
    /// nothing does. Values are on the [`super::knowledge`] 0..1 scales.
    pub fn capability_floor(&self) -> CapabilityFloor {
        match self {
            // EDIT is the highest token-mass role and its failure mode is a diff
            // that doesn't apply: below ~97% edit-format reliability the retry
            // tax dominates any per-token saving. No orchestration requirement —
            // the editor is handed a decided plan.
            WorkflowRole::Edit => CapabilityFloor {
                edit_format_reliability_min: EDIT_RELIABILITY_FLOOR,
                orchestration_min: 0.0,
            },
            // ORCHESTRATE is few tokens, high stakes: a wrong decomposition costs
            // every downstream token. 0.80 is the SWE-bench-Verified-normalized
            // band where the current frontier agentic models sit; below it, plans
            // need human rescue often enough that the role should be flagged.
            WorkflowRole::Orchestrate => CapabilityFloor {
                edit_format_reliability_min: 0.0,
                orchestration_min: 0.80,
            },
            // MECHANICAL (search / read / summarize) needs a model that follows
            // the tool-call protocol and returns a coherent summary — the small
            // local-coder tier, not frontier. Modest floors so a $0 on-device
            // model can clear it, but a model that can't reliably emit a
            // well-formed tool call or edit is excluded (it would still cost a
            // round trip and a retry).
            WorkflowRole::Mechanical => CapabilityFloor {
                edit_format_reliability_min: 0.75,
                orchestration_min: 0.45,
            },
            // REVIEW must read a diff and judge it — orchestration-shaped work a
            // notch below planning; 0.75 admits strong-but-cheaper verifiers
            // (cross-family diversity matters more here than the last few points).
            WorkflowRole::Review => CapabilityFloor {
                edit_format_reliability_min: 0.0,
                orchestration_min: 0.75,
            },
            // LOCAL is the free/private tier: same modest bar as MECHANICAL — it
            // is what that tier is for.
            WorkflowRole::Local => CapabilityFloor {
                edit_format_reliability_min: 0.75,
                orchestration_min: 0.45,
            },
        }
    }
}

/// The per-role minimum capability (see [`WorkflowRole::capability_floor`]).
/// Both thresholds are on the knowledge-base 0..1 scales; `0.0` = no requirement
/// on that axis.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct CapabilityFloor {
    /// Minimum `edit_format_reliability` (aider-style diff-format reliability).
    pub edit_format_reliability_min: f64,
    /// Minimum `orchestration_strength` (agentic / long-horizon score).
    pub orchestration_min: f64,
}

impl CapabilityFloor {
    /// Whether `m` clears both thresholds.
    pub fn clears(&self, m: &ModelKnowledge) -> bool {
        m.edit_format_reliability >= self.edit_format_reliability_min
            && m.orchestration_strength >= self.orchestration_min
    }

    /// Human-readable summary for warnings / surfaces.
    pub fn describe(&self) -> String {
        format!(
            "diff-format ≥ {:.0}%, agentic ≥ {:.2}",
            self.edit_format_reliability_min * 100.0,
            self.orchestration_min
        )
    }
}

/// A model the user has available (from discovery, or declared by the user).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AvailableModel {
    pub provider: String,
    pub model: String,
}

impl AvailableModel {
    pub fn new(provider: &str, model: &str) -> Self {
        Self {
            provider: provider.to_string(),
            model: model.to_string(),
        }
    }

    /// "provider/model" for display.
    pub fn label(&self) -> String {
        format!("{}/{}", self.provider, self.model)
    }
}

/// The recommendation for one role: which model, why, at what relative cost.
/// Serializable so a UI can render it later without re-deriving the reasoning.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RoleRecommendation {
    pub role: WorkflowRole,
    /// Empty when no configured model fits this role (e.g. no local model).
    pub provider: String,
    pub model: String,
    pub display_name: String,
    pub family: String,
    /// Blended reference price (input + output per MTok; 0 for local).
    pub blended_cost_per_mtok: f64,
    /// Transparent reasoning: the measured metric and the cost that drove it.
    pub reason: String,
    /// Non-fatal flags: below the capability floor, non-caching cache-heavy
    /// role, single-family (no REVIEW diversity), no local model, …
    pub warnings: Vec<String>,
    /// Whether the chosen model clears the role's [`CapabilityFloor`]. `false`
    /// means the recommender fell back to the best available and the warnings
    /// say so — the UI/CLI can render an under-fit honestly instead of a green tick.
    pub floor_met: bool,
}

/// The full objective recommendation over the user's configured models.
#[derive(Debug, Clone, Serialize)]
pub struct Recommendation {
    /// One entry per role, in [`WorkflowRole::all`] order.
    pub recommendations: Vec<RoleRecommendation>,
    /// Configured models scored (in the knowledge base), as "provider/model".
    pub considered: Vec<String>,
    /// Configured models NOT in the knowledge base — cannot be objectively
    /// recommended until a row is added. Reported, never silently dropped.
    pub unknown_models: Vec<String>,
    /// Considered models that resolved to their knowledge row only by FAMILY
    /// (e.g. `ollama/qwen3-coder:30b` → the `qwen3-coder` row,
    /// [`LookupConfidence::FamilyEstimate`]) — their scores are estimates for that
    /// variant. Reported so a surface can label them, never silently exact.
    pub estimated_models: Vec<String>,
}

// ── Ranking comparators (best-first: `Less` == "a ranks before b") ───────────

fn id_order(a: &ModelKnowledge, b: &ModelKnowledge) -> Ordering {
    a.provider.cmp(b.provider).then(a.model.cmp(b.model))
}

/// EDIT, strict order: highest diff-format reliability, then lowest cost, then
/// stable id. Used only for the below-floor fallback — among floor-clearers
/// EDIT is picked by [`cheapest_within_band`] with [`EDIT_RELIABILITY_BAND`].
fn edit_rank(a: &ModelKnowledge, b: &ModelKnowledge) -> Ordering {
    b.edit_format_reliability
        .total_cmp(&a.edit_format_reliability)
        .then(
            a.blended_cost_per_mtok()
                .total_cmp(&b.blended_cost_per_mtok()),
        )
        .then(id_order(a, b))
}

/// ORCHESTRATE, strict order: strongest orchestration, then lowest cost, then
/// stable id. Used only for the below-floor fallback — among floor-clearers
/// ORCHESTRATE is picked by [`cheapest_within_band`] with the planner band.
fn orchestrate_rank(a: &ModelKnowledge, b: &ModelKnowledge) -> Ordering {
    b.orchestration_strength
        .total_cmp(&a.orchestration_strength)
        .then(
            a.blended_cost_per_mtok()
                .total_cmp(&b.blended_cost_per_mtok()),
        )
        .then(id_order(a, b))
}

/// MECHANICAL: cheapest, preferring local on a cost tie, then — at still-equal
/// cost — the MORE CAPABLE model (F3: agentic then edit-format), and only then a
/// stable id. Without the capability tiebreak two $0 locals fell to alphabetical
/// order, picking e.g. `qwen3` over the stronger `qwen3-coder`. Applied among the
/// models that clear [`WorkflowRole::Mechanical`]'s floor ([`pick`]) — i.e.
/// "cheapest that can do it", never a below-floor bargain.
fn mechanical_rank(a: &ModelKnowledge, b: &ModelKnowledge) -> Ordering {
    a.blended_cost_per_mtok()
        .total_cmp(&b.blended_cost_per_mtok())
        .then(b.is_local.cmp(&a.is_local)) // local (true) ranks before non-local
        .then(
            b.orchestration_strength
                .total_cmp(&a.orchestration_strength),
        )
        .then(
            b.edit_format_reliability
                .total_cmp(&a.edit_format_reliability),
        )
        .then(id_order(a, b))
}

/// LOCAL (among local models only): the most capable local editor.
fn local_rank(a: &ModelKnowledge, b: &ModelKnowledge) -> Ordering {
    b.edit_format_reliability
        .total_cmp(&a.edit_format_reliability)
        .then(
            b.orchestration_strength
                .total_cmp(&a.orchestration_strength),
        )
        .then(id_order(a, b))
}

/// REVIEW: prefer a family different from EDIT's — but ONLY when that model is a
/// genuinely capable verifier (orchestration strength at or above `diversity_floor`,
/// i.e. within [`REVIEW_DIVERSITY_MAX_STRENGTH_GAP`] of the strongest available).
/// A materially-weaker different-family model does not earn the preference (F2);
/// then strongest orchestration, then lowest cost, then stable id. This strict
/// order serves the below-floor fallback; among floor-clearers
/// [`recommend`] applies the same diversity preference and then
/// [`cheapest_within_band`] with [`REVIEW_STRENGTH_BAND`].
fn review_rank(
    a: &ModelKnowledge,
    b: &ModelKnowledge,
    edit_family: &str,
    diversity_floor: f64,
) -> Ordering {
    let a_diverse = a.family != edit_family && a.orchestration_strength >= diversity_floor;
    let b_diverse = b.family != edit_family && b.orchestration_strength >= diversity_floor;
    b_diverse
        .cmp(&a_diverse) // capable different-family (true) ranks before the rest
        .then(
            b.orchestration_strength
                .total_cmp(&a.orchestration_strength),
        )
        .then(
            a.blended_cost_per_mtok()
                .total_cmp(&b.blended_cost_per_mtok()),
        )
        .then(id_order(a, b))
}

fn base_rec(
    role: WorkflowRole,
    m: &ModelKnowledge,
    floor_met: bool,
    reason: String,
) -> RoleRecommendation {
    let mut rec = RoleRecommendation {
        role,
        provider: m.provider.to_string(),
        model: m.model.to_string(),
        display_name: m.display_name.to_string(),
        family: m.family.to_string(),
        blended_cost_per_mtok: m.blended_cost_per_mtok(),
        reason,
        warnings: Vec::new(),
        floor_met,
    };
    if !floor_met {
        let floor = role.capability_floor();
        let consequence = match role {
            WorkflowRole::Edit => "expect more failed-edit retries",
            WorkflowRole::Orchestrate => "expect plans that need more human rescue",
            WorkflowRole::Review => "expect weaker verification",
            WorkflowRole::Mechanical | WorkflowRole::Local => {
                "expect more malformed tool calls and retries"
            }
        };
        rec.warnings.push(format!(
            "no available model clears the {} capability floor ({}) — falling back to the best \
             available, {} ({:.1}% diff-format, agentic {:.2}); {}; add a stronger model for this \
             role if you can",
            role.as_str().to_uppercase(),
            floor.describe(),
            m.display_name,
            m.edit_format_reliability * 100.0,
            m.orchestration_strength,
            consequence
        ));
    }
    // Cache guard: a cache-heavy role on a non-caching provider forfeits the
    // warm-prefix saving that makes the loop cheap.
    if role.is_cache_heavy() && !m.cache_support {
        rec.warnings.push(format!(
            "{} provider has no prompt caching — the {} loop can't keep a warm cache; \
             prefer a caching provider for this role",
            m.family,
            role.as_str()
        ));
    }
    rec
}

/// Choose a role's model: the best by `rank` among the candidates that clear the
/// role's [`CapabilityFloor`]; when none does, the best by the same `rank` among
/// all candidates with `floor_met == false` (the caller's [`base_rec`] then
/// warns). `None` only when `candidates` is empty.
fn pick<'a>(
    role: WorkflowRole,
    candidates: &[&'a ModelKnowledge],
    mut rank: impl FnMut(&ModelKnowledge, &ModelKnowledge) -> Ordering,
) -> Option<(&'a ModelKnowledge, bool)> {
    let floor = role.capability_floor();
    let clears: Vec<&'a ModelKnowledge> = candidates
        .iter()
        .copied()
        .filter(|m| floor.clears(m))
        .collect();
    if let Some(m) = clears.iter().copied().min_by(|a, b| rank(a, b)) {
        return Some((m, true));
    }
    candidates
        .iter()
        .copied()
        .min_by(|a, b| rank(a, b))
        .map(|m| (m, false))
}

/// The floor-clearers for `role` among `candidates`.
fn floor_clearers<'a>(
    role: WorkflowRole,
    candidates: &[&'a ModelKnowledge],
) -> Vec<&'a ModelKnowledge> {
    let floor = role.capability_floor();
    candidates
        .iter()
        .copied()
        .filter(|m| floor.clears(m))
        .collect()
}

/// A strength-ranked role's pick: [`cheapest_within_band`] among the
/// floor-clearers; when nothing clears, the strict `fallback_rank` over every
/// candidate (the weakest options get no cost discount). Returns
/// `(pick, strongest, floor_met)`.
fn pick_banded<'a>(
    role: WorkflowRole,
    candidates: &[&'a ModelKnowledge],
    band: f64,
    strength: impl Fn(&ModelKnowledge) -> f64,
    fallback_rank: impl FnMut(&ModelKnowledge, &ModelKnowledge) -> Ordering,
) -> Option<(&'a ModelKnowledge, &'a ModelKnowledge, bool)> {
    let clears = floor_clearers(role, candidates);
    if let Some(b) = cheapest_within_band(
        &clears,
        band,
        strength,
        ModelKnowledge::blended_cost_per_mtok,
        id_order,
    ) {
        return Some((b.pick, b.strongest, true));
    }
    let (m, _) = pick(role, candidates, fallback_rank)?;
    Some((m, m, false))
}

fn same_model(a: &ModelKnowledge, b: &ModelKnowledge) -> bool {
    a.provider == b.provider && a.model == b.model
}

/// The objective recommendation for each role over the AVAILABLE models. Pure —
/// no config, no IO, no vendor branch. `available` is the knowledge rows for the
/// models the user has; empty in → empty out. Per role: rank among the models
/// that clear the role's [`CapabilityFloor`]; fall back (flagged) to the best
/// available when nothing does.
pub fn recommend(available: &[ModelKnowledge]) -> Vec<RoleRecommendation> {
    if available.is_empty() {
        return Vec::new();
    }
    let all: Vec<&ModelKnowledge> = available.iter().collect();
    let mut out = Vec::with_capacity(5);

    // ORCHESTRATE — strongest orchestration among floor-clearers, cost deciding
    // inside the planner band (the directory's `planner()` rule).
    let (orch, orch_top, orch_ok) = pick_banded(
        WorkflowRole::Orchestrate,
        &all,
        super::capability_directory::PLANNER_STRENGTH_BAND,
        |m| m.orchestration_strength,
        orchestrate_rank,
    )
    .unwrap();
    out.push(base_rec(
        WorkflowRole::Orchestrate,
        orch,
        orch_ok,
        if same_model(orch, orch_top) {
            format!(
                "strongest orchestration among your models (agentic score {:.2}, ${:.2}/Mtok \
                 blended)",
                orch.orchestration_strength,
                orch.blended_cost_per_mtok()
            )
        } else {
            format!(
                "within {:.2} of the strongest orchestrator ({}) at lower cost (agentic score \
                 {:.2}, ${:.2}/Mtok blended vs ${:.2})",
                super::capability_directory::PLANNER_STRENGTH_BAND,
                orch_top.display_name,
                orch.orchestration_strength,
                orch.blended_cost_per_mtok(),
                orch_top.blended_cost_per_mtok()
            )
        },
    ));

    // EDIT — the cheapest floor-clearer within EDIT_RELIABILITY_BAND of the most
    // reliable editor. Below the floor the fallback warning (base_rec) says so.
    let (edit, edit_top, edit_ok) = pick_banded(
        WorkflowRole::Edit,
        &all,
        EDIT_RELIABILITY_BAND,
        |m| m.edit_format_reliability,
        edit_rank,
    )
    .unwrap();
    let edit_rec = base_rec(
        WorkflowRole::Edit,
        edit,
        edit_ok,
        if same_model(edit, edit_top) {
            format!(
                "highest edit-format reliability at lowest cost ({:.1}% diff-format, \
                 ${:.2}/Mtok blended)",
                edit.edit_format_reliability * 100.0,
                edit.blended_cost_per_mtok()
            )
        } else {
            format!(
                "within {:.0} point of the most reliable editor ({}, {:.1}%) at lower cost \
                 ({:.1}% diff-format, ${:.2}/Mtok blended vs ${:.2})",
                EDIT_RELIABILITY_BAND * 100.0,
                edit_top.display_name,
                edit_top.edit_format_reliability * 100.0,
                edit.edit_format_reliability * 100.0,
                edit.blended_cost_per_mtok(),
                edit_top.blended_cost_per_mtok()
            )
        },
    );
    let edit_family = edit.family;
    out.push(edit_rec);

    // MECHANICAL — the CHEAPEST model that clears the floor, local at equal cost.
    let (mech, mech_ok) = pick(WorkflowRole::Mechanical, &all, mechanical_rank).unwrap();
    out.push(base_rec(
        WorkflowRole::Mechanical,
        mech,
        mech_ok,
        if mech.is_local {
            format!(
                "cheapest model that clears the mechanical floor — on-device {} at $0, no cloud \
                 spend for mechanical work",
                mech.display_name
            )
        } else {
            format!(
                "cheapest model that clears the mechanical floor among your configured providers \
                 (${:.2}/Mtok blended)",
                mech.blended_cost_per_mtok()
            )
        },
    ));

    // REVIEW — strong, prefer a different family than EDIT, but never trade away
    // material capability for diversity (F2). A different-family model must be
    // within REVIEW_DIVERSITY_MAX_STRENGTH_GAP of the strongest available
    // orchestrator to earn the cross-family preference.
    let best_orch = available
        .iter()
        .map(|m| m.orchestration_strength)
        .fold(f64::NEG_INFINITY, f64::max);
    let diversity_floor = best_orch - REVIEW_DIVERSITY_MAX_STRENGTH_GAP;
    // Among floor-clearers: the capable different-family partition when it is
    // non-empty (the cross-family rule, unchanged), else every floor-clearer;
    // then the cheapest within REVIEW_STRENGTH_BAND of that partition's
    // strongest. Nothing clears → the strict `review_rank` fallback.
    let review_clears = floor_clearers(WorkflowRole::Review, &all);
    let diverse: Vec<&ModelKnowledge> = review_clears
        .iter()
        .copied()
        .filter(|m| m.family != edit_family && m.orchestration_strength >= diversity_floor)
        .collect();
    let review_pool = if diverse.is_empty() {
        &review_clears
    } else {
        &diverse
    };
    let (review, review_ok) = match cheapest_within_band(
        review_pool,
        REVIEW_STRENGTH_BAND,
        |m: &ModelKnowledge| m.orchestration_strength,
        ModelKnowledge::blended_cost_per_mtok,
        id_order,
    ) {
        Some(b) => (b.pick, true),
        None => pick(WorkflowRole::Review, &all, |a, b| {
            review_rank(a, b, edit_family, diversity_floor)
        })
        .unwrap(),
    };
    let has_other_family = available.iter().any(|m| m.family != edit_family);
    let mut review_rec = base_rec(
        WorkflowRole::Review,
        review,
        review_ok,
        if review.family != edit_family {
            format!(
                "strong verifier from a DIFFERENT family than EDIT ({} vs {}) — cross-family \
                 review catches mistakes a model won't catch in its own output",
                review.family, edit_family
            )
        } else {
            format!(
                "strongest available verifier (agentic score {:.2})",
                review.orchestration_strength
            )
        },
    );
    if review.family == edit_family {
        if has_other_family {
            // A different family exists but is too weak to review well — we kept
            // the stronger same-family model rather than downgrade for diversity.
            review_rec.warnings.push(format!(
                "the only different-family model(s) are materially weaker verifiers (>{:.0}% \
                 agentic-score gap) — REVIEW stays with the stronger {} model for capability; \
                 add a stronger different-vendor model for independent cross-family review",
                REVIEW_DIVERSITY_MAX_STRENGTH_GAP * 100.0,
                review.family
            ));
        } else {
            review_rec.warnings.push(
                "only one model family available — REVIEW shares EDIT's family, so there is no \
                 cross-family diversity; add a model from another vendor for independent review"
                    .to_string(),
            );
        }
    }
    out.push(review_rec);

    // LOCAL — the best on-device model, if any.
    let locals: Vec<&ModelKnowledge> = all.iter().copied().filter(|m| m.is_local).collect();
    match pick(WorkflowRole::Local, &locals, local_rank) {
        Some((local, local_ok)) => out.push(base_rec(
            WorkflowRole::Local,
            local,
            local_ok,
            format!(
                "best on-device model ($0, private; {:.1}% diff-format)",
                local.edit_format_reliability * 100.0
            ),
        )),
        None => {
            let mut none_rec = RoleRecommendation {
                role: WorkflowRole::Local,
                provider: String::new(),
                model: String::new(),
                display_name: String::new(),
                family: String::new(),
                blended_cost_per_mtok: 0.0,
                reason: "no on-device model configured".to_string(),
                warnings: Vec::new(),
                floor_met: false,
            };
            none_rec.warnings.push(
                "no local model configured — the free/private on-device tier is unavailable, so \
                 mechanical work falls back to the cheapest cloud model. Install an Ollama coding \
                 model (e.g. qwen3-coder) to get a $0 tier."
                    .to_string(),
            );
            out.push(none_rec);
        }
    }

    // Reorder to WorkflowRole::all() order for a stable, readable presentation.
    let mut ordered = Vec::with_capacity(out.len());
    for role in WorkflowRole::all() {
        if let Some(pos) = out.iter().position(|r| r.role == role) {
            ordered.push(out.remove(pos));
        }
    }
    ordered
}

// ── Discovery ────────────────────────────────────────────────────────────────

/// The slim view of a configured provider the discovery core needs: the
/// provider id, the env var that activates it, and the model ids it serves. The
/// IO wrapper projects declarative-provider configs into this so the pure core
/// never touches the config type (keeping discovery unit-testable).
#[derive(Debug, Clone)]
pub struct ProviderModels {
    pub provider: String,
    pub api_key_env: String,
    pub models: Vec<String>,
}

/// The BUILTIN providers — those served by `providers/*.rs`, NOT by a declarative
/// `providers/declarative/*.json` — paired with the env var that activates each.
/// `all_declarative_provider_configs` does not enumerate these, so discovery must
/// add them explicitly or the canonical setup (an Anthropic key + a local Ollama)
/// discovers almost nothing (F1). Ollama is keyless (empty env) and is discovered
/// via [`discover_ollama_models`] rather than by enumerating the KB, since its
/// models are "available" only when actually pulled locally.
pub const BUILTIN_PROVIDERS: &[(&str, &str)] = &[
    ("anthropic", "ANTHROPIC_API_KEY"),
    ("openai", "OPENAI_API_KEY"),
    ("google", "GOOGLE_API_KEY"),
    ("xai", "XAI_API_KEY"),
    ("zai", "ZAI_API_KEY"),
    ("ollama", ""), // keyless local; models come from discovery, not the KB
];

/// The keyed builtin providers as [`ProviderModels`], their model lists sourced
/// from the objective knowledge base ([`super::knowledge::KNOWN_MODELS`]) so there
/// is a single source of truth for which builtin models exist. Pure. Ollama is
/// excluded (keyless, handled by [`discover_ollama_models`]).
fn builtin_provider_models() -> Vec<ProviderModels> {
    BUILTIN_PROVIDERS
        .iter()
        .filter(|(_, env)| !env.is_empty())
        .map(|(provider, env)| ProviderModels {
            provider: provider.to_string(),
            api_key_env: env.to_string(),
            models: super::knowledge::KNOWN_MODELS
                .iter()
                .filter(|m| m.provider == *provider && !m.is_local)
                .map(|m| m.model.to_string())
                .collect(),
        })
        .filter(|p| !p.models.is_empty())
        .collect()
}

/// Pure assembly of the full discoverable provider surface: the declarative
/// providers + the keyed builtins ([`builtin_provider_models`]) + a keyless local
/// Ollama entry when models were detected. Split from [`discover_available_models`]
/// so the builtin/ollama discovery logic is unit-testable without config or a
/// network probe.
fn discoverable_providers(
    mut declarative: Vec<ProviderModels>,
    ollama_models: Vec<String>,
) -> Vec<ProviderModels> {
    declarative.extend(builtin_provider_models());
    if !ollama_models.is_empty() {
        declarative.push(ProviderModels {
            provider: "ollama".to_string(),
            api_key_env: String::new(), // keyless — available_from includes it unconditionally
            models: ollama_models,
        });
    }
    declarative
}

/// The env var that activates a provider — builtin table first, then the
/// declarative configs. `Some("")` for a keyless provider (e.g. Ollama). `None`
/// for a provider not found in either surface (an unknown/custom id). Thin IO
/// wrapper (reads the declarative configs).
pub fn provider_key_env(provider: &str) -> Option<String> {
    if let Some((_, env)) = BUILTIN_PROVIDERS.iter().find(|(p, _)| *p == provider) {
        return Some((*env).to_string());
    }
    crate::config::declarative_providers::all_declarative_provider_configs()
        .into_iter()
        .find(|c| c.name == provider)
        .map(|c| c.api_key_env)
}

/// Whether a provider can be dispatched to right now. Keyless providers (empty
/// key env, e.g. Ollama) are always available; keyed providers need their key
/// configured ([`super::cheap::is_key_configured`]). An unknown/custom provider is
/// assumed available (don't block a setup we can't see). Used by the summon
/// consistency guard to avoid dispatching to a keyless-but-unconfigured provider.
pub fn is_provider_configured(provider: &str) -> bool {
    match provider_key_env(provider) {
        Some(env) if env.trim().is_empty() => true,
        Some(env) => super::cheap::is_key_configured(&env),
        None => true,
    }
}

/// Enumerate the user's AVAILABLE models — pure over the configured provider
/// surface. A provider counts as available when its `api_key_env` is set
/// (`is_key_set`); a keyless provider (empty `api_key_env`, e.g. a local one) is
/// included unconditionally since it needs no key. `main` is the session /
/// `GOOSE_*` default provider+model, always included. Deduped and sorted for a
/// deterministic result.
pub fn available_from(
    providers: &[ProviderModels],
    is_key_set: &impl Fn(&str) -> bool,
    main: Option<(String, String)>,
) -> Vec<AvailableModel> {
    let mut out: Vec<AvailableModel> = Vec::new();
    let mut push = |provider: &str, model: &str| {
        let am = AvailableModel::new(provider, model);
        if !out.contains(&am) {
            out.push(am);
        }
    };

    if let Some((p, m)) = &main {
        if !p.is_empty() && !m.is_empty() {
            push(p, m);
        }
    }

    for p in providers {
        let key = p.api_key_env.trim();
        let available = key.is_empty() || is_key_set(key);
        if !available {
            continue;
        }
        for model in &p.models {
            push(&p.provider, model);
        }
    }

    out.sort_by(|a, b| a.provider.cmp(&b.provider).then(a.model.cmp(&b.model)));
    out
}

/// Detect locally-available Ollama models WITHOUT a network call: it surfaces the
/// configured `GOOSE_MODEL` when the default provider is Ollama. This is the
/// sync floor ("at least the configured model when it's ollama") used by
/// [`discover_available_models`] on sync paths and by pure tests; the async
/// [`discover_ollama_models_async`] adds the live `/api/tags` probe on top.
fn discover_ollama_models() -> Vec<String> {
    let cfg = crate::config::Config::global();
    let mut models = Vec::new();
    if let Ok(p) = cfg.get_param::<String>("GOOSE_PROVIDER") {
        if p.eq_ignore_ascii_case("ollama") {
            if let Ok(m) = cfg.get_param::<String>("GOOSE_MODEL") {
                if !m.trim().is_empty() {
                    models.push(m);
                }
            }
        }
    }
    models
}

/// How long the local Ollama probe waits before deciding "not running". Short:
/// this runs on `permagent packs recommend` and a Settings load; a daemon that
/// is up answers `/api/tags` in milliseconds.
pub const OLLAMA_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// The base URL the Ollama probe hits, resolved from the same `OLLAMA_HOST`
/// setting the Ollama provider itself uses (`providers/ollama.rs`
/// `OllamaProvider::from_env`): a bare host gets `http://` and — for localhost —
/// the default port; a full URL is used as-is (trailing `/` stripped). Pure over
/// the raw setting so it is testable without touching config. Note the mesh /
/// Librarian pool uses a DIFFERENT setting (`PERMAGENT_OLLAMA_HOST`,
/// [`crate::config::ollama_host`]) — the recommender follows the provider,
/// because those are the models the `ollama` provider will actually serve.
pub fn ollama_base_url(raw: Option<String>) -> String {
    let host = raw
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| crate::providers::ollama::OLLAMA_HOST.to_string());
    let mut base = if host.starts_with("http://") || host.starts_with("https://") {
        host.clone()
    } else {
        format!("http://{host}")
    };
    let is_localhost = host == "localhost" || host == "127.0.0.1" || host == "::1";
    if is_localhost {
        base.push_str(&format!(
            ":{}",
            crate::providers::ollama::OLLAMA_DEFAULT_PORT
        ));
    }
    base.trim_end_matches('/').to_string()
}

/// Parse the model names out of an Ollama `/api/tags` body
/// (`{"models":[{"name":"qwen3-coder:30b",…},…]}`). Pure; malformed → empty.
/// Names are kept AS REPORTED (with their `:tag`) because that is the id the
/// `ollama` provider must be given; the knowledge base resolves the family via
/// [`super::knowledge::lookup`]'s prefix fallback.
pub fn parse_ollama_tags(body: &str) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct Tags {
        #[serde(default)]
        models: Vec<Tag>,
    }
    #[derive(serde::Deserialize)]
    struct Tag {
        #[serde(default)]
        name: String,
    }
    serde_json::from_str::<Tags>(body)
        .map(|t| {
            t.models
                .into_iter()
                .map(|m| m.name.trim().to_string())
                .filter(|n| !n.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Detect the models the local Ollama daemon actually has pulled: a best-effort
/// GET of `<OLLAMA_HOST>/api/tags` with a short timeout, `[]` when Ollama is not
/// running / unreachable / answers garbage (never an error — "no local models"
/// is a valid discovery result). The sync `GOOSE_MODEL` floor
/// ([`discover_ollama_models`]) is merged in so the configured model appears
/// even when the probe fails. Deduped, sorted.
///
/// The on-device GGUF registry (`providers/local_inference/local_model_registry`)
/// is deliberately NOT folded in: its ids are `<hf-repo>:<quant>` under the
/// `local` provider, and the knowledge base has no `local/*` rows, so
/// `lookup` could never resolve them — they would only swell `unknown_models`.
/// Add KB rows for that provider first, then extend this.
pub async fn discover_ollama_models_async() -> Vec<String> {
    let mut models = discover_ollama_models();
    let raw_host = crate::config::Config::global()
        .get_param::<String>("OLLAMA_HOST")
        .ok();
    let url = format!("{}/api/tags", ollama_base_url(raw_host));
    let probed = match reqwest::Client::builder()
        .timeout(OLLAMA_PROBE_TIMEOUT)
        .build()
    {
        Ok(client) => match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => resp
                .text()
                .await
                .map(|b| parse_ollama_tags(&b))
                .unwrap_or_default(),
            _ => Vec::new(),
        },
        Err(_) => Vec::new(),
    };
    for m in probed {
        if !models.contains(&m) {
            models.push(m);
        }
    }
    models.sort();
    models
}

/// Shared IO core of [`discover_available_models`] /
/// [`discover_available_models_async`]: the declarative providers + the keyed
/// builtins + the given local Ollama models, filtered by configured keys, plus
/// the `GOOSE_PROVIDER`/`GOOSE_MODEL` default.
fn discover_available_models_with(ollama_models: Vec<String>) -> Vec<AvailableModel> {
    use crate::config::declarative_providers::all_declarative_provider_configs;

    let declarative: Vec<ProviderModels> = all_declarative_provider_configs()
        .into_iter()
        .map(|c| ProviderModels {
            provider: c.name,
            api_key_env: c.api_key_env,
            models: c.models.into_iter().map(|m| m.name).collect(),
        })
        .collect();

    let providers = discoverable_providers(declarative, ollama_models);

    let is_key_set = |k: &str| super::cheap::is_key_configured(k);

    let cfg = crate::config::Config::global();
    let main = match (
        cfg.get_param::<String>("GOOSE_PROVIDER").ok(),
        cfg.get_param::<String>("GOOSE_MODEL").ok(),
    ) {
        (Some(p), Some(m)) => Some((p, m)),
        _ => None,
    };

    available_from(&providers, &is_key_set, main)
}

/// Discover the user's available models from the live configured-provider surface
/// — declarative providers + the builtin `providers/*.rs` providers (Anthropic /
/// OpenAI / Google / xAI, keyed; Ollama, local) — plus the
/// `GOOSE_PROVIDER`/`GOOSE_MODEL` default. SYNC and network-free: local Ollama
/// models are only the configured-default floor. Prefer
/// [`discover_available_models_async`] from an async context.
pub fn discover_available_models() -> Vec<AvailableModel> {
    discover_available_models_with(discover_ollama_models())
}

/// [`discover_available_models`] plus a live probe of the local Ollama daemon
/// for the models actually pulled ([`discover_ollama_models_async`]). This is
/// what the CLI (`permagent packs recommend`) and any Settings surface should
/// call — otherwise a user with five Ollama models and an Anthropic key sees only
/// the Anthropic side.
pub async fn discover_available_models_async() -> Vec<AvailableModel> {
    discover_available_models_with(discover_ollama_models_async().await)
}

/// Resolve available `(provider, model)` pairs to their knowledge rows,
/// deduped. The pure input to [`recommend`].
pub fn resolve_known(available: &[AvailableModel]) -> Vec<ModelKnowledge> {
    let mut known: Vec<ModelKnowledge> = Vec::new();
    for a in available {
        if let Some(m) = lookup(&a.provider, &a.model) {
            if !known
                .iter()
                .any(|k| k.provider == m.provider && k.model == m.model)
            {
                known.push(*m);
            }
        }
    }
    known
}

/// The full objective recommendation over the user's configured models. Sync,
/// network-free IO entry point (no live Ollama probe) — see
/// [`recommend_configured_async`] for the one the CLI / UI should use.
pub fn recommend_configured() -> Recommendation {
    let available = discover_available_models();
    recommend_from_available(&available)
}

/// [`recommend_configured`] over the async discovery (live Ollama probe). IO
/// entry point for `permagent packs recommend` and the Settings surface.
pub async fn recommend_configured_async() -> Recommendation {
    let available = discover_available_models_async().await;
    recommend_from_available(&available)
}

/// Pure assembly of a [`Recommendation`] from a discovered list — split from
/// [`recommend_configured`] so the discover→resolve→recommend→report pipeline is
/// testable without config.
pub fn recommend_from_available(available: &[AvailableModel]) -> Recommendation {
    let known = resolve_known(available);
    let considered: Vec<String> = known
        .iter()
        .map(|m| format!("{}/{}", m.provider, m.model))
        .collect();
    let unknown_models: Vec<String> = available
        .iter()
        .filter(|a| lookup(&a.provider, &a.model).is_none())
        .map(|a| a.label())
        .collect();
    let estimated_models: Vec<String> = available
        .iter()
        .filter(|a| {
            matches!(
                lookup_with_confidence(&a.provider, &a.model),
                Some((_, LookupConfidence::FamilyEstimate))
            )
        })
        .map(|a| a.label())
        .collect();
    Recommendation {
        recommendations: recommend(&known),
        considered,
        unknown_models,
        estimated_models,
    }
}

// ── Review complexity and reviewer assignment (ruling 2026-09-17) ────────────
//
// The complexity of a REVIEW is assessed from the diff at selection time —
// facts only, never a model's claim — and sets the capability floor; the
// reviewer is then assigned by the one selection rule
// ([`cheapest_clearing_floor`]) among cross-family candidates. The decision
// ([`ReviewerAssignment`]) carries every candidate with its strength and price,
// so "why this reviewer" is auditable after the fact.

/// The review verdict. Ordered ROUTINE < MODERATE < COMPLEX, so a caller can
/// hold a verdict to a minimum ([`assign_reviewer_at_least`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewComplexity {
    Routine,
    Moderate,
    Complex,
}

impl ReviewComplexity {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReviewComplexity::Routine => "routine",
            ReviewComplexity::Moderate => "moderate",
            ReviewComplexity::Complex => "complex",
        }
    }

    /// The orchestration-strength floor a reviewer must clear for this
    /// complexity. ROUTINE and MODERATE are exactly the small/large floors
    /// [`super::reviewer_pick::select_reviewer`] already applies, so both
    /// review paths agree; COMPLEX is the ORCHESTRATE floor (the band where
    /// long-horizon agentic models sit), because a cross-language or
    /// signature-with-callers diff needs a reviewer that can hold the whole
    /// change.
    pub fn floor(&self) -> f64 {
        match self {
            ReviewComplexity::Routine => REVIEW_FLOOR_ROUTINE,
            ReviewComplexity::Moderate => REVIEW_FLOOR_MODERATE,
            ReviewComplexity::Complex => REVIEW_FLOOR_COMPLEX,
        }
    }
}

/// ROUTINE review floor — `reviewer_pick`'s small-change floor.
pub const REVIEW_FLOOR_ROUTINE: f64 = super::reviewer_pick::REVIEWER_MIN_ORCHESTRATION;
/// MODERATE review floor — `reviewer_pick`'s large-change floor.
pub const REVIEW_FLOOR_MODERATE: f64 = super::reviewer_pick::REVIEWER_STRONG_ORCHESTRATION;
/// COMPLEX review floor — the ORCHESTRATE role's agentic floor
/// ([`WorkflowRole::capability_floor`]).
pub const REVIEW_FLOOR_COMPLEX: f64 = 0.80;

/// Changed lines above which a review is COMPLEX on size alone (three times the
/// small/large line `reviewer_pick::SMALL_DIFF_LINES` draws).
pub const REVIEW_COMPLEX_CHANGED_LINES: usize = 600;
/// Non-test code files at or above which a review is MODERATE.
pub const REVIEW_MODERATE_CODE_FILES: usize = 3;
/// Non-test code files at or above which a review is COMPLEX — the same
/// breadth line `assess::MANY_FILES` draws for a goal.
pub const REVIEW_COMPLEX_CODE_FILES: usize = super::assess::MANY_FILES;

/// The facts a review assessment reads, measured on the diff the reviewer will
/// see — the goal's trunk-relative (fork-point) diff, not `HEAD`, so work that
/// already auto-landed still measures.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ReviewFacts {
    /// `+`/`-` lines, excluding the `+++`/`---` headers.
    pub changed_lines: usize,
    /// Every file the diff touches.
    pub changed_files: Vec<String>,
    /// Changed files in a programming language that are not test files.
    pub code_files: usize,
    /// Changed files `tool_monitor::is_test_path` classifies as tests.
    pub test_files: usize,
    /// Distinct programming languages among the code files.
    pub languages: Vec<String>,
    /// Distinct parent directories of the code files.
    pub modules: Vec<String>,
    /// Symbols whose signature the diff changes
    /// (`call_site_completeness::detect_signature_changes`).
    pub signature_changes: Vec<String>,
    /// Call sites of those symbols the harness enumerated (Lane R62e), when the
    /// caller gathered them; 0 otherwise.
    pub call_sites: usize,
}

impl ReviewFacts {
    /// Measure a unified diff. `call_sites` is the enumerated call-site count
    /// for the diff's signature changes (IO the caller owns).
    pub fn from_diff(diff: &str, call_sites: usize) -> Self {
        let mut changed_lines = 0usize;
        let mut changed_files: Vec<String> = Vec::new();
        let mut last_old_path: Option<String> = None;
        for line in diff.lines() {
            if let Some(rest) = line.strip_prefix("--- ") {
                let path = rest.trim();
                last_old_path = (path != "/dev/null")
                    .then(|| path.strip_prefix("a/").unwrap_or(path).to_string());
                continue;
            }
            if let Some(rest) = line.strip_prefix("+++ ") {
                let path = rest.trim();
                let file = if path == "/dev/null" {
                    last_old_path.take()
                } else {
                    Some(path.strip_prefix("b/").unwrap_or(path).to_string())
                };
                if let Some(file) = file {
                    if !changed_files.contains(&file) {
                        changed_files.push(file);
                    }
                }
                continue;
            }
            if line.starts_with('+') || line.starts_with('-') {
                changed_lines += 1;
            }
        }

        let mut code_files = 0usize;
        let mut test_files = 0usize;
        let mut languages: Vec<String> = Vec::new();
        let mut modules: Vec<String> = Vec::new();
        for file in &changed_files {
            if crate::tool_monitor::is_test_path(file) {
                test_files += 1;
                continue;
            }
            let ext = file
                .rsplit_once('.')
                .map(|(_, e)| e.to_ascii_lowercase())
                .unwrap_or_default();
            let Some(Some(lang)) = super::planner_complexity::language_of_extension(&ext) else {
                continue;
            };
            code_files += 1;
            if !languages.iter().any(|l| l == lang) {
                languages.push(lang.to_string());
            }
            let module = file
                .rsplit_once('/')
                .map(|(dir, _)| dir.to_string())
                .unwrap_or_else(|| ".".to_string());
            if !modules.contains(&module) {
                modules.push(module);
            }
        }

        let mut signature_changes: Vec<String> = Vec::new();
        for change in super::call_site_completeness::detect_signature_changes(diff) {
            if !signature_changes.contains(&change.symbol) {
                signature_changes.push(change.symbol);
            }
        }

        ReviewFacts {
            changed_lines,
            changed_files,
            code_files,
            test_files,
            languages,
            modules,
            signature_changes,
            call_sites,
        }
    }
}

/// The review-complexity verdict and the floor it implies.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReviewComplexityVerdict {
    pub complexity: ReviewComplexity,
    pub floor: f64,
    pub reasons: Vec<String>,
}

/// THE review assessment. Deterministic, facts only (same no-prose rule as
/// [`super::assess`]):
///
/// - COMPLEX when the diff spans >= 2 languages, changes a signature that has
///   enumerated call sites, changes more than [`REVIEW_COMPLEX_CHANGED_LINES`]
///   lines, or touches >= [`REVIEW_COMPLEX_CODE_FILES`] code files;
/// - MODERATE when it changes more than `reviewer_pick::SMALL_DIFF_LINES` lines,
///   touches >= [`REVIEW_MODERATE_CODE_FILES`] code files or >= 2 modules,
///   changes a signature (no call sites found), or changes code with no test
///   file alongside beyond `review_gate::TRIVIAL_MAX_LINES` lines;
/// - ROUTINE otherwise.
///
/// No risk-area vocabulary (security, money, …) is read: the repo has no
/// objective classifier for those, and a keyword list would be prose.
pub fn assess_review_complexity(facts: &ReviewFacts) -> ReviewComplexityVerdict {
    let mut complex: Vec<String> = Vec::new();
    if facts.languages.len() >= 2 {
        complex.push(format!(
            "touches {} languages ({})",
            facts.languages.len(),
            facts.languages.join(", ")
        ));
    }
    if !facts.signature_changes.is_empty() && facts.call_sites > 0 {
        complex.push(format!(
            "changes the signature of {} with {} enumerated call site(s)",
            facts.signature_changes.join(", "),
            facts.call_sites
        ));
    }
    if facts.changed_lines > REVIEW_COMPLEX_CHANGED_LINES {
        complex.push(format!(
            "{} changed lines (> {REVIEW_COMPLEX_CHANGED_LINES})",
            facts.changed_lines
        ));
    }
    if facts.code_files >= REVIEW_COMPLEX_CODE_FILES {
        complex.push(format!(
            "{} code files (>= {REVIEW_COMPLEX_CODE_FILES})",
            facts.code_files
        ));
    }
    if !complex.is_empty() {
        return ReviewComplexityVerdict {
            complexity: ReviewComplexity::Complex,
            floor: ReviewComplexity::Complex.floor(),
            reasons: complex,
        };
    }

    let mut moderate: Vec<String> = Vec::new();
    if facts.changed_lines > super::reviewer_pick::SMALL_DIFF_LINES {
        moderate.push(format!(
            "{} changed lines (> {})",
            facts.changed_lines,
            super::reviewer_pick::SMALL_DIFF_LINES
        ));
    }
    if facts.code_files >= REVIEW_MODERATE_CODE_FILES {
        moderate.push(format!(
            "{} code files (>= {REVIEW_MODERATE_CODE_FILES})",
            facts.code_files
        ));
    }
    if facts.modules.len() >= 2 {
        moderate.push(format!("touches {} modules", facts.modules.len()));
    }
    if !facts.signature_changes.is_empty() {
        moderate.push(format!(
            "changes the signature of {} (no call sites enumerated)",
            facts.signature_changes.join(", ")
        ));
    }
    if facts.code_files > 0
        && facts.test_files == 0
        && facts.changed_lines > super::review_gate::TRIVIAL_MAX_LINES
    {
        moderate.push("changes code with no test file changed alongside".to_string());
    }
    if !moderate.is_empty() {
        return ReviewComplexityVerdict {
            complexity: ReviewComplexity::Moderate,
            floor: ReviewComplexity::Moderate.floor(),
            reasons: moderate,
        };
    }

    ReviewComplexityVerdict {
        complexity: ReviewComplexity::Routine,
        floor: ReviewComplexity::Routine.floor(),
        reasons: vec![format!(
            "{} changed line(s) in {} code file(s) ({} test file(s)), {} language(s), no \
             signature change",
            facts.changed_lines,
            facts.code_files,
            facts.test_files,
            facts.languages.len()
        )],
    }
}

/// One model considered for a review, as recorded on the decision.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReviewerCandidate {
    pub provider: String,
    pub model: String,
    pub family: String,
    /// Knowledge-base orchestration strength (the review floors' scale).
    pub orchestration: f64,
    /// Blended $/Mtok (0 for local).
    pub cost_per_mtok: f64,
    /// `picked`, `clears_floor`, `near_floor`, `materially_short`,
    /// `same_family`, `family_unproven` or `retired`.
    pub status: String,
}

/// The auditable reviewer assignment: verdict, floor, every candidate with its
/// strength and price, the pick, and why.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReviewerAssignment {
    pub complexity: ReviewComplexity,
    pub floor: f64,
    pub complexity_reasons: Vec<String>,
    pub facts: ReviewFacts,
    /// The author's family, when the author is known.
    pub worker_family: Option<String>,
    pub candidates: Vec<ReviewerCandidate>,
    pub pick_provider: Option<String>,
    pub pick_model: Option<String>,
    /// Whether the pick clears the floor outright (false = within
    /// [`MATERIAL_FLOOR_SHORTFALL`] below it).
    pub pick_clears_floor: bool,
    pub why: String,
}

impl ReviewerAssignment {
    /// `provider/model` of the pick.
    pub fn pick_label(&self) -> Option<String> {
        match (&self.pick_provider, &self.pick_model) {
            (Some(p), Some(m)) => Some(format!("{p}/{m}")),
            _ => None,
        }
    }

    /// The pick as a [`super::role_map::RoleModel`].
    pub fn pick_role_model(&self) -> Option<super::role_map::RoleModel> {
        match (&self.pick_provider, &self.pick_model) {
            (Some(p), Some(m)) => Some(super::role_map::RoleModel {
                provider: p.clone(),
                model: m.clone(),
            }),
            _ => None,
        }
    }

    /// The JSON recorded on the goal card.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

/// Assess the review from `facts` and assign the reviewer among the models the
/// user has (ruling 2026-09-17): candidates are the knowledge-base-rated,
/// non-retired models from a DIFFERENT family than the author (`worker`);
/// among them [`cheapest_clearing_floor`] with the verdict's floor and
/// [`MATERIAL_FLOOR_SHORTFALL`] — the cheapest that clears wins, and a pricier
/// one only when every cheaper one falls short and its advantage is
/// proportionate to its price. No pick when the author is unknown (no family
/// can be proven different) or nothing cross-family is eligible. Pure over the
/// knowledge base.
pub fn assign_reviewer(
    facts: ReviewFacts,
    worker: Option<(&str, &str)>,
    available: &[AvailableModel],
) -> ReviewerAssignment {
    assign_reviewer_at_least(facts, ReviewComplexity::Routine, None, worker, available)
}

/// [`assign_reviewer`] with the verdict held to at least `minimum`. The
/// delegate reviewer path uses it when no diff could be measured for the
/// review (no goal range and a clean working tree): an unmeasured review fails
/// toward a capable reviewer, never toward the cheapest (ruling 2026-09-17).
/// `minimum_reason` is recorded among the verdict's reasons when it raises it.
pub fn assign_reviewer_at_least(
    facts: ReviewFacts,
    minimum: ReviewComplexity,
    minimum_reason: Option<&str>,
    worker: Option<(&str, &str)>,
    available: &[AvailableModel],
) -> ReviewerAssignment {
    let mut verdict = assess_review_complexity(&facts);
    if verdict.complexity < minimum {
        verdict.complexity = minimum;
        verdict.floor = minimum.floor();
        verdict.reasons.push(
            minimum_reason
                .map(str::to_string)
                .unwrap_or_else(|| format!("held to at least a {} review", minimum.as_str())),
        );
    }
    let floor = verdict.floor;
    let worker_family = worker.map(|(p, m)| super::reviewer_pick::family_of(p, m));

    struct Rated {
        provider: String,
        model: String,
        family: String,
        k: &'static ModelKnowledge,
    }
    let mut rated: Vec<Rated> = Vec::new();
    for a in available {
        let Some(k) = lookup(&a.provider, &a.model) else {
            continue;
        };
        if rated
            .iter()
            .any(|r| r.provider == a.provider && r.model == a.model)
        {
            continue;
        }
        rated.push(Rated {
            provider: a.provider.clone(),
            model: a.model.clone(),
            family: super::reviewer_pick::family_of(&a.provider, &a.model),
            k,
        });
    }
    let is_eligible_family = |r: &Rated| match &worker_family {
        Some(wf) => &r.family != wf,
        None => false,
    };
    let is_retired = |r: &Rated| super::reviewer_pick::model_is_retired(&r.provider, &r.model);
    let pool: Vec<&Rated> = rated
        .iter()
        .filter(|r| is_eligible_family(r) && !is_retired(r))
        .collect();
    let chosen = cheapest_clearing_floor(
        &pool,
        floor,
        MATERIAL_FLOOR_SHORTFALL,
        |r: &Rated| r.k.orchestration_strength,
        |r: &Rated| r.k.blended_cost_per_mtok(),
        |a: &Rated, b: &Rated| a.provider.cmp(&b.provider).then(a.model.cmp(&b.model)),
    );

    let candidates: Vec<ReviewerCandidate> = rated
        .iter()
        .map(|r| {
            let s = r.k.orchestration_strength;
            let picked = chosen
                .as_ref()
                .is_some_and(|c| c.pick.provider == r.provider && c.pick.model == r.model);
            let status = if picked {
                "picked"
            } else if worker_family.is_none() {
                "family_unproven"
            } else if !is_eligible_family(r) {
                "same_family"
            } else if is_retired(r) {
                "retired"
            } else if s >= floor - BAND_EPSILON {
                "clears_floor"
            } else if s >= floor - MATERIAL_FLOOR_SHORTFALL - BAND_EPSILON {
                "near_floor"
            } else {
                "materially_short"
            };
            ReviewerCandidate {
                provider: r.provider.clone(),
                model: r.model.clone(),
                family: r.family.clone(),
                orchestration: s,
                cost_per_mtok: r.k.blended_cost_per_mtok(),
                status: status.to_string(),
            }
        })
        .collect();

    let complexity = verdict.complexity.as_str();
    let (pick_provider, pick_model, pick_clears_floor, why) = match (&chosen, &worker_family) {
        (_, None) => (
            None,
            None,
            false,
            "the author's model was not recorded, so no reviewer can be proven to come from a \
             different family"
                .to_string(),
        ),
        (None, Some(_)) => (
            None,
            None,
            false,
            format!(
                "no cross-family model you have reaches within {MATERIAL_FLOOR_SHORTFALL:.2} of \
                 the {complexity} review floor ({floor:.2})"
            ),
        ),
        (Some(c), Some(_)) => {
            let p = c.pick;
            let s = p.k.orchestration_strength;
            let cost = p.k.blended_cost_per_mtok();
            let why =
                if let Some(d) = c.displaced {
                    let need = required_gain_for_price(d.k.blended_cost_per_mtok(), cost);
                    format!(
                    "{}/{} clears the {complexity} review floor ({floor:.2}) at agentic {s:.2}, \
                     ${cost:.2}/Mtok; the cheaper {}/{} is {:.3} below the floor and this \
                     model's +{:.3} advantage is proportionate to the {:.1}x price (needs >= \
                     {need:.3})",
                    p.provider,
                    p.model,
                    d.provider,
                    d.model,
                    floor - d.k.orchestration_strength,
                    s - d.k.orchestration_strength,
                    cost.max(PRICE_RATIO_MIN_COST_PER_MTOK)
                        / d.k.blended_cost_per_mtok().max(PRICE_RATIO_MIN_COST_PER_MTOK),
                )
                } else if c.clears_floor {
                    format!(
                        "cheapest cross-family model clearing the {complexity} review floor \
                     ({floor:.2}): {}/{} at agentic {s:.2}, ${cost:.2}/Mtok — a pricier model \
                     is used only when every cheaper one falls short of the floor",
                        p.provider, p.model
                    )
                } else {
                    format!(
                    "cheapest cross-family candidate {}/{} is {:.3} below the {complexity} review \
                     floor ({floor:.2}), within the {MATERIAL_FLOOR_SHORTFALL:.2} margin, and no \
                     pricier floor-clearing model's advantage is proportionate to its price \
                     (agentic {s:.2}, ${cost:.2}/Mtok)",
                    p.provider,
                    p.model,
                    floor - s
                )
                };
            (
                Some(p.provider.clone()),
                Some(p.model.clone()),
                c.clears_floor,
                why,
            )
        }
    };

    ReviewerAssignment {
        complexity: verdict.complexity,
        floor,
        complexity_reasons: verdict.reasons,
        facts,
        worker_family,
        candidates,
        pick_provider,
        pick_model,
        pick_clears_floor,
        why,
    }
}

/// The next complexity tier up, or `None` at COMPLEX.
fn next_review_tier(complexity: ReviewComplexity) -> Option<ReviewComplexity> {
    match complexity {
        ReviewComplexity::Routine => Some(ReviewComplexity::Moderate),
        ReviewComplexity::Moderate => Some(ReviewComplexity::Complex),
        ReviewComplexity::Complex => None,
    }
}

/// The ONE escalation after an UNCERTAIN review (ruling 2026-09-17: a pricier
/// model only when the cheaper one objectively fell short). `first` is the
/// assignment that picked the first reviewer; `answered` is the reviewer that
/// actually returned UNCERTAIN (it can differ from `first`'s pick after a
/// provider fallback). Only models that rank ABOVE `answered` on the same
/// knowledge-base strength scale are considered, and neither `answered` nor
/// `first`'s pick is ever picked again.
///
/// - Below COMPLEX: the same proportional rule as [`assign_reviewer`], with the
///   verdict held to the next floor tier — the cheapest stronger cross-family
///   model clearing that floor.
/// - At COMPLEX, or when nothing stronger clears the next floor: the strongest
///   cross-family model that ranks above `answered` (cost breaks exact ties).
/// - Nothing ranks above `answered`: no pick, and `why` says so.
pub fn escalate_reviewer_after_uncertain(
    first: &ReviewerAssignment,
    answered: (&str, &str),
    worker: Option<(&str, &str)>,
    available: &[AvailableModel],
) -> ReviewerAssignment {
    let answered_label = format!("{}/{}", answered.0, answered.1);
    let answered_strength = lookup(answered.0, answered.1)
        .map(|k| k.orchestration_strength)
        .unwrap_or(0.0);
    let is_excluded = |a: &AvailableModel| {
        (a.provider == answered.0 && a.model == answered.1)
            || (first.pick_provider.as_deref() == Some(a.provider.as_str())
                && first.pick_model.as_deref() == Some(a.model.as_str()))
    };
    let stronger: Vec<AvailableModel> = available
        .iter()
        .filter(|a| !is_excluded(*a))
        .filter(|a| {
            lookup(&a.provider, &a.model)
                .is_some_and(|k| k.orchestration_strength > answered_strength + BAND_EPSILON)
        })
        .cloned()
        .collect();
    let escalation_note = format!(
        "escalated once after {answered_label} (agentic {answered_strength:.2}) returned \
         UNCERTAIN on a {} review",
        first.complexity.as_str()
    );

    if let Some(tier) = next_review_tier(first.complexity) {
        let mut next = assign_reviewer_at_least(
            first.facts.clone(),
            tier,
            Some(&escalation_note),
            worker,
            &stronger,
        );
        if next.pick_provider.is_some() {
            next.why = format!("{escalation_note}: {}", next.why);
            return next;
        }
    }

    // At COMPLEX (or nothing stronger clears the next floor): the strongest
    // cross-family model above the reviewer that was unsure.
    let mut top = assign_reviewer_at_least(
        first.facts.clone(),
        ReviewComplexity::Complex,
        Some(&escalation_note),
        worker,
        &stronger,
    );
    let eligible = |c: &ReviewerCandidate| {
        matches!(
            c.status.as_str(),
            "picked" | "clears_floor" | "near_floor" | "materially_short"
        )
    };
    let strongest = top
        .candidates
        .iter()
        .filter(|c| eligible(*c))
        .max_by(|a, b| {
            a.orchestration
                .total_cmp(&b.orchestration)
                .then_with(|| b.cost_per_mtok.total_cmp(&a.cost_per_mtok))
                .then_with(|| b.provider.cmp(&a.provider).then(b.model.cmp(&a.model)))
        })
        .map(|c| {
            (
                c.provider.clone(),
                c.model.clone(),
                c.orchestration,
                c.cost_per_mtok,
            )
        });
    match strongest {
        Some((provider, model, strength, cost)) => {
            for c in top.candidates.iter_mut() {
                if c.status == "picked" {
                    c.status = if c.orchestration >= top.floor - BAND_EPSILON {
                        "clears_floor".to_string()
                    } else {
                        "near_floor".to_string()
                    };
                }
                if c.provider == provider && c.model == model {
                    c.status = "picked".to_string();
                }
            }
            top.pick_clears_floor = strength >= top.floor - BAND_EPSILON;
            top.why = format!(
                "{escalation_note}: the strongest cross-family model ranking above it is \
                 {provider}/{model} (agentic {strength:.2}, ${cost:.2}/Mtok)"
            );
            top.pick_provider = Some(provider);
            top.pick_model = Some(model);
        }
        None => {
            top.pick_provider = None;
            top.pick_model = None;
            top.pick_clears_floor = false;
            top.why = format!(
                "{escalation_note}, but no cross-family model you have ranks above it, so there \
                 is no stronger reviewer to escalate to"
            );
        }
    }
    top
}

#[cfg(test)]
mod tests {
    use super::*;

    // A terse synthetic-row builder for the pure recommender tests.
    #[allow(clippy::too_many_arguments)]
    fn m(
        provider: &'static str,
        model: &'static str,
        family: &'static str,
        edit: f64,
        orch: f64,
        input: f64,
        output: f64,
        cache: bool,
        local: bool,
    ) -> ModelKnowledge {
        ModelKnowledge {
            provider,
            model,
            display_name: model,
            family,
            edit_format_reliability: edit,
            orchestration_strength: orch,
            input_usd_per_mtok: input,
            output_usd_per_mtok: output,
            cache_support: cache,
            context_window: 200_000,
            is_local: local,
        }
    }

    fn rec_for(recs: &[RoleRecommendation], role: WorkflowRole) -> &RoleRecommendation {
        recs.iter().find(|r| r.role == role).unwrap()
    }

    /// THE no-bias property test. A non-Anthropic model with higher edit-format
    /// reliability at lower cost MUST win EDIT over a Claude model. The logic has
    /// no vendor branch, so this holds by construction.
    #[test]
    fn non_anthropic_wins_edit_when_it_is_objectively_better() {
        let models = [
            // Claude: strong but dearer, slightly lower diff-format.
            m(
                "anthropic",
                "claude-sonnet-5",
                "anthropic",
                0.97,
                0.80,
                3.0,
                15.0,
                true,
                false,
            ),
            // OpenAI: higher diff-format AND cheaper.
            m(
                "openai", "gpt-x", "openai", 0.99, 0.70, 0.5, 1.5, true, false,
            ),
        ];
        let recs = recommend(&models);
        let edit = rec_for(&recs, WorkflowRole::Edit);
        assert_eq!(
            edit.provider, "openai",
            "objectively better non-Anthropic model must win EDIT"
        );
        assert_eq!(edit.model, "gpt-x");
    }

    /// The mirror: a Claude model that is objectively better wins on merit too —
    /// the recommender is neutral, not anti-Anthropic either.
    #[test]
    fn anthropic_wins_edit_when_it_is_objectively_better() {
        let models = [
            m(
                "anthropic",
                "claude-sonnet-5",
                "anthropic",
                0.99,
                0.80,
                3.0,
                15.0,
                true,
                false,
            ),
            m(
                "openai", "gpt-x", "openai", 0.95, 0.70, 0.5, 1.5, true, false,
            ),
        ];
        let recs = recommend(&models);
        assert_eq!(rec_for(&recs, WorkflowRole::Edit).provider, "anthropic");
    }

    /// Cost is the tie-break for EDIT: equal diff-format reliability → cheaper wins.
    #[test]
    fn edit_cost_tie_break_picks_the_cheaper_model() {
        let models = [
            m("vendor_a", "dear", "a", 0.98, 0.70, 5.0, 25.0, true, false),
            m("vendor_b", "cheap", "b", 0.98, 0.70, 0.5, 1.5, true, false),
        ];
        let recs = recommend(&models);
        assert_eq!(rec_for(&recs, WorkflowRole::Edit).model, "cheap");
    }

    // ── Proportionality bands (N1 finding 1) ─────────────────────────────────

    /// THE N1 finding 1 regression: a 0.5-point diff-format edge (Fable 5 at
    /// 98.5% vs Sonnet 5 at 98.0%, the knowledge-base numbers) is inside
    /// EDIT_RELIABILITY_BAND, so it must NOT buy a 3.3x price.
    #[test]
    fn edit_band_picks_the_cheaper_model_within_the_band() {
        let models = [
            m(
                "anthropic",
                "fable",
                "anthropic",
                0.985,
                0.95,
                10.0,
                50.0,
                true,
                false,
            ),
            m(
                "anthropic",
                "sonnet",
                "anthropic",
                0.980,
                0.80,
                3.0,
                15.0,
                true,
                false,
            ),
        ];
        let recs = recommend(&models);
        let edit = rec_for(&recs, WorkflowRole::Edit);
        assert_eq!(
            edit.model, "sonnet",
            "a 0.5-point reliability edge inside the band must not buy a 3.3x price"
        );
        assert!(edit.floor_met);
        assert!(edit.reason.contains("within"), "{}", edit.reason);

        // The same rule over the real knowledge-base rows.
        let known = resolve_known(&[
            AvailableModel::new("anthropic", "claude-fable-5"),
            AvailableModel::new("anthropic", "claude-sonnet-5"),
            AvailableModel::new("anthropic", "claude-opus-4-8"),
        ]);
        let recs = recommend(&known);
        assert_eq!(rec_for(&recs, WorkflowRole::Edit).model, "claude-sonnet-5");
    }

    /// Outside the band strength still wins, whatever it costs — the band is a
    /// near-tie rule, never "cheapest that clears".
    #[test]
    fn edit_band_keeps_the_more_reliable_model_outside_the_band() {
        let models = [
            m(
                "dear", "reliable", "dear", 0.995, 0.80, 10.0, 50.0, true, false,
            ),
            m(
                "cheap", "shaky", "cheap", 0.975, 0.80, 1.0, 4.0, true, false,
            ),
        ];
        let recs = recommend(&models);
        assert_eq!(
            rec_for(&recs, WorkflowRole::Edit).model,
            "reliable",
            "a 2-point reliability gap is outside the 1-point band — reliability wins"
        );
    }

    /// A $0 local model competes inside the band like any other, once it clears
    /// the EDIT floor.
    #[test]
    fn edit_band_admits_a_free_local_model_that_meets_the_band_and_floor() {
        let models = [
            m("cloud", "top", "cloud", 0.990, 0.85, 3.0, 15.0, true, false),
            m(
                "ollama",
                "local-editor",
                "ollama",
                0.982,
                0.60,
                0.0,
                0.0,
                false,
                true,
            ),
        ];
        let recs = recommend(&models);
        let edit = rec_for(&recs, WorkflowRole::Edit);
        assert_eq!(edit.provider, "ollama");
        assert!(edit.blended_cost_per_mtok.abs() < 1e-9);
    }

    /// REVIEW had the same defect: strength-first inside the preferred
    /// partition. Two capable different-family verifiers 0.02 apart → the
    /// cheaper one; 0.06 apart → the stronger one.
    #[test]
    fn review_band_picks_the_cheaper_verifier_within_the_band() {
        let edit_owner = m(
            "anthropic",
            "sonnet",
            "anthropic",
            0.99,
            0.80,
            3.0,
            15.0,
            true,
            false,
        );
        let inside = [
            edit_owner,
            m(
                "openai", "dear", "openai", 0.95, 0.83, 5.0, 30.0, true, false,
            ),
            m("zai", "cheap", "zai", 0.94, 0.81, 1.4, 4.4, true, false),
        ];
        let recs = recommend(&inside);
        assert_eq!(rec_for(&recs, WorkflowRole::Edit).family, "anthropic");
        let review = rec_for(&recs, WorkflowRole::Review);
        assert_eq!(
            review.model, "cheap",
            "0.02 apart is inside the review band"
        );
        assert!(review.floor_met);

        let outside = [
            edit_owner,
            m(
                "openai", "dear", "openai", 0.95, 0.87, 5.0, 30.0, true, false,
            ),
            m("zai", "cheap", "zai", 0.94, 0.81, 1.4, 4.4, true, false),
        ];
        let recs = recommend(&outside);
        assert_eq!(
            rec_for(&recs, WorkflowRole::Review).model,
            "dear",
            "0.06 apart is a real capability gap — the stronger verifier wins"
        );
    }

    /// The band never overrides the cross-family rule: a cheaper SAME-family
    /// model inside the band does not displace a capable different-family one.
    #[test]
    fn review_band_keeps_the_cross_family_rule() {
        let models = [
            m(
                "anthropic",
                "sonnet",
                "anthropic",
                0.99,
                0.82,
                3.0,
                15.0,
                true,
                false,
            ),
            m(
                "anthropic",
                "haiku-ish",
                "anthropic",
                0.90,
                0.81,
                0.5,
                2.0,
                true,
                false,
            ),
            m("xai", "grok", "xai", 0.95, 0.80, 2.0, 6.0, false, false),
        ];
        let recs = recommend(&models);
        assert_eq!(rec_for(&recs, WorkflowRole::Edit).family, "anthropic");
        assert_eq!(rec_for(&recs, WorkflowRole::Review).family, "xai");
    }

    /// ORCHESTRATE in the recommender follows the directory's planner band, and
    /// the below-floor fallback stays strict (no cost discount for weak picks).
    #[test]
    fn orchestrate_band_matches_the_planner_rule() {
        let models = [
            m("dear", "dear", "dear", 0.90, 0.82, 5.0, 20.0, true, false),
            m("cheap", "cheap", "cheap", 0.90, 0.80, 1.0, 4.0, true, false),
        ];
        let recs = recommend(&models);
        assert_eq!(rec_for(&recs, WorkflowRole::Orchestrate).model, "cheap");
    }

    #[test]
    fn orchestrate_picks_the_strongest_orchestrator() {
        let models = [
            m("a", "planner", "a", 0.90, 0.90, 8.0, 40.0, true, false),
            m("b", "editor", "b", 0.99, 0.60, 0.5, 1.5, true, false),
        ];
        let recs = recommend(&models);
        // ORCHESTRATE follows orchestration strength, not diff-format or cost.
        assert_eq!(rec_for(&recs, WorkflowRole::Orchestrate).model, "planner");
        // EDIT still follows diff-format.
        assert_eq!(rec_for(&recs, WorkflowRole::Edit).model, "editor");
    }

    #[test]
    fn mechanical_prefers_a_free_local_model() {
        let models = [
            m(
                "anthropic",
                "haiku",
                "anthropic",
                0.90,
                0.60,
                1.0,
                5.0,
                true,
                false,
            ),
            m(
                "ollama",
                "qwen3-coder",
                "ollama",
                0.86,
                0.55,
                0.0,
                0.0,
                false,
                true,
            ),
        ];
        let recs = recommend(&models);
        let mech = rec_for(&recs, WorkflowRole::Mechanical);
        assert_eq!(mech.provider, "ollama");
        assert!(mech.blended_cost_per_mtok.abs() < 1e-9);
    }

    #[test]
    fn mechanical_falls_back_to_cheapest_cloud_without_a_local() {
        let models = [
            m(
                "anthropic",
                "haiku",
                "anthropic",
                0.90,
                0.60,
                1.0,
                5.0,
                true,
                false,
            ),
            m(
                "minimax", "m2.5", "minimax", 0.91, 0.66, 0.30, 1.20, false, false,
            ),
        ];
        let recs = recommend(&models);
        // MiniMax blended 1.50 < Haiku blended 6.00 → cheapest cloud wins.
        assert_eq!(rec_for(&recs, WorkflowRole::Mechanical).model, "m2.5");
    }

    /// F3: two $0 local models — the equal-cost tiebreak must pick the more
    /// CAPABLE one, not the alphabetically-first id. Alphabetically `qwen3` sorts
    /// before `qwen3-coder`, so an id-only tiebreak wrongly picked the weaker one.
    #[test]
    fn mechanical_equal_cost_tiebreaks_on_capability_not_alphabetical() {
        let models = [
            m(
                "ollama", "qwen3", "ollama", 0.76, 0.48, 0.0, 0.0, false, true,
            ),
            m(
                "ollama",
                "qwen3-coder",
                "ollama",
                0.86,
                0.55,
                0.0,
                0.0,
                false,
                true,
            ),
        ];
        let recs = recommend(&models);
        assert_eq!(
            rec_for(&recs, WorkflowRole::Mechanical).model,
            "qwen3-coder",
            "at equal $0 cost MECHANICAL must tiebreak on capability, not alphabetical id"
        );
    }

    // ── Capability floors ("cheapest that clears the floor") ─────────────────

    /// (a) MECHANICAL picks the cheapest model that CLEARS the floor over a
    /// cheaper one below it — best fit, not cheapest-then-climb.
    #[test]
    fn mechanical_picks_cheapest_floor_clearer_over_a_cheaper_below_floor_model() {
        let models = [
            // Cheapest, but below the mechanical floor (edit 0.60 < 0.75).
            m(
                "bargain", "tiny", "bargain", 0.60, 0.30, 0.05, 0.20, false, false,
            ),
            // Dearer, clears the floor.
            m(
                "mid", "capable", "mid", 0.90, 0.60, 0.30, 1.20, false, false,
            ),
            // Dearest, clears the floor.
            m("top", "frontier", "top", 0.98, 0.85, 3.0, 15.0, true, false),
        ];
        let recs = recommend(&models);
        let mech = rec_for(&recs, WorkflowRole::Mechanical);
        assert_eq!(
            mech.model, "capable",
            "MECHANICAL must be the cheapest model that clears the floor, not the cheapest overall"
        );
        assert!(mech.floor_met);
        assert!(!mech.warnings.iter().any(|w| w.contains("capability floor")));
    }

    /// (b) A $0 local model that clears the floor wins MECHANICAL over any cloud
    /// model — the floor admits the small local-coder tier by design.
    #[test]
    fn local_wins_mechanical_when_it_clears_the_floor() {
        let models = [
            m("top", "frontier", "top", 0.98, 0.85, 3.0, 15.0, true, false),
            m(
                "ollama",
                "qwen3-coder",
                "ollama",
                0.86,
                0.55,
                0.0,
                0.0,
                false,
                true,
            ),
        ];
        let recs = recommend(&models);
        let mech = rec_for(&recs, WorkflowRole::Mechanical);
        assert_eq!(mech.provider, "ollama");
        assert!(mech.floor_met);
        assert!(mech.reason.contains("clears the mechanical floor"));
        // …and the same local model does NOT win when it is below the floor:
        // MECHANICAL goes to the cheapest floor-clearer instead.
        let weak = [
            m("top", "frontier", "top", 0.98, 0.85, 3.0, 15.0, true, false),
            m(
                "ollama", "tiny", "ollama", 0.50, 0.20, 0.0, 0.0, false, true,
            ),
        ];
        let recs = recommend(&weak);
        let mech = rec_for(&recs, WorkflowRole::Mechanical);
        assert_eq!(mech.model, "frontier");
        assert!(mech.floor_met);
        // The LOCAL role still reports the only local — but flagged below floor.
        let local = rec_for(&recs, WorkflowRole::Local);
        assert_eq!(local.model, "tiny");
        assert!(!local.floor_met);
        assert!(local
            .warnings
            .iter()
            .any(|w| w.contains("capability floor")));
    }

    /// (c) When NO model clears a role's floor the recommender falls back to the
    /// best available by the role's rank, sets `floor_met = false`, and warns —
    /// never a silent under-fit, never an empty slot.
    #[test]
    fn no_floor_clearer_falls_back_to_best_available_with_a_warning() {
        let models = [
            m("a", "weak-a", "a", 0.70, 0.40, 0.10, 0.40, false, false),
            m("b", "weak-b", "b", 0.72, 0.42, 0.20, 0.80, false, false),
        ];
        let recs = recommend(&models);
        for role in WorkflowRole::all() {
            let r = rec_for(&recs, role);
            if role == WorkflowRole::Local {
                assert!(r.model.is_empty(), "no local model in this set");
                continue;
            }
            assert!(!r.model.is_empty(), "{role:?} must still get a model");
            assert!(!r.floor_met, "{role:?} must report the floor was not met");
            assert!(
                r.warnings
                    .iter()
                    .any(|w| w.contains("capability floor") && w.contains("falling back")),
                "{role:?} must warn that the floor was not met: {:?}",
                r.warnings
            );
        }
        // The fallback still follows each role's own rank: MECHANICAL = cheapest,
        // ORCHESTRATE = strongest, EDIT = highest diff-format.
        assert_eq!(rec_for(&recs, WorkflowRole::Mechanical).model, "weak-a");
        assert_eq!(rec_for(&recs, WorkflowRole::Orchestrate).model, "weak-b");
        assert_eq!(rec_for(&recs, WorkflowRole::Edit).model, "weak-b");
    }

    /// Every role's floor is internally consistent and the KB's own local rows
    /// clear the modest MECHANICAL/LOCAL bar (that tier exists for them).
    #[test]
    fn capability_floors_are_sane_and_admit_the_local_coder_tier() {
        for role in WorkflowRole::all() {
            let f = role.capability_floor();
            assert!((0.0..=1.0).contains(&f.edit_format_reliability_min));
            assert!((0.0..=1.0).contains(&f.orchestration_min));
        }
        assert_eq!(
            WorkflowRole::Edit
                .capability_floor()
                .edit_format_reliability_min,
            EDIT_RELIABILITY_FLOOR
        );
        let qwen = lookup("ollama", "qwen3-coder").unwrap();
        assert!(WorkflowRole::Mechanical.capability_floor().clears(qwen));
        assert!(WorkflowRole::Local.capability_floor().clears(qwen));
        assert!(!WorkflowRole::Edit.capability_floor().clears(qwen));
    }

    #[test]
    fn review_prefers_a_different_family_than_edit() {
        let models = [
            // EDIT will pick the Anthropic model (highest diff-format).
            m(
                "anthropic",
                "sonnet",
                "anthropic",
                0.98,
                0.80,
                3.0,
                15.0,
                true,
                false,
            ),
            // A different family, strong enough to be the reviewer.
            m(
                "openai", "gpt-x", "openai", 0.96, 0.79, 1.25, 10.0, true, false,
            ),
        ];
        let recs = recommend(&models);
        assert_eq!(rec_for(&recs, WorkflowRole::Edit).family, "anthropic");
        let review = rec_for(&recs, WorkflowRole::Review);
        assert_eq!(
            review.family, "openai",
            "REVIEW must prefer a different family than EDIT"
        );
        assert!(review.warnings.is_empty());
    }

    #[test]
    fn single_family_flags_review_lacks_diversity() {
        let models = [
            m(
                "anthropic",
                "opus",
                "anthropic",
                0.975,
                0.82,
                5.0,
                25.0,
                true,
                false,
            ),
            m(
                "anthropic",
                "sonnet",
                "anthropic",
                0.98,
                0.77,
                3.0,
                15.0,
                true,
                false,
            ),
        ];
        let recs = recommend(&models);
        let review = rec_for(&recs, WorkflowRole::Review);
        assert_eq!(review.family, "anthropic");
        assert!(
            review
                .warnings
                .iter()
                .any(|w| w.contains("one model family")),
            "single-family review must be flagged"
        );
    }

    /// F2: REVIEW must NOT be handed to a materially-weaker different-family model
    /// purely for diversity. Given {Opus, Sonnet, local qwen3-coder}, the only
    /// different family (ollama, orch 0.55) is far below the best orchestrator
    /// (Opus 0.886) — REVIEW must stay with the stronger Anthropic model, not the
    /// weak local, and must NOT claim a "different family" verifier.
    #[test]
    fn review_does_not_pick_a_materially_weaker_different_family_model() {
        let models = [
            m(
                "anthropic",
                "claude-opus-4-8",
                "anthropic",
                0.975,
                0.886,
                5.0,
                25.0,
                true,
                false,
            ),
            m(
                "anthropic",
                "claude-sonnet-5",
                "anthropic",
                0.980,
                0.80,
                3.0,
                15.0,
                true,
                false,
            ),
            m(
                "ollama",
                "qwen3-coder",
                "ollama",
                0.860,
                0.55,
                0.0,
                0.0,
                false,
                true,
            ),
        ];
        let recs = recommend(&models);
        let review = rec_for(&recs, WorkflowRole::Review);
        assert_ne!(
            review.provider, "ollama",
            "REVIEW must not be the weak local model just because it is a different family"
        );
        assert_eq!(review.family, "anthropic");
        assert!(
            !review.reason.contains("DIFFERENT family"),
            "the reason must not falsely claim a different-family verifier"
        );
        // Honest warning: a different family exists but is too weak to review.
        assert!(
            review
                .warnings
                .iter()
                .any(|w| w.contains("materially weaker")),
            "must flag that the only different-family option was too weak"
        );
    }

    /// F2 guard: a genuinely capable different-family model IS still preferred —
    /// the strength floor only blocks materially-weaker ones. Grok (orch 0.866) is
    /// within the gap of Fable (0.90) and a different family, so it wins REVIEW.
    #[test]
    fn review_still_prefers_a_capable_different_family_model() {
        let models = [
            m(
                "anthropic",
                "fable",
                "anthropic",
                0.985,
                0.90,
                10.0,
                50.0,
                true,
                false,
            ),
            m("xai", "grok", "xai", 0.970, 0.866, 2.0, 6.0, false, false),
        ];
        let recs = recommend(&models);
        assert_eq!(rec_for(&recs, WorkflowRole::Edit).family, "anthropic");
        let review = rec_for(&recs, WorkflowRole::Review);
        assert_eq!(
            review.family, "xai",
            "a capable different-family model must still win REVIEW"
        );
        assert!(review.warnings.is_empty());
    }

    /// The cache guard fires when a cache-heavy role (ORCHESTRATE/EDIT) is routed
    /// to a non-caching provider.
    #[test]
    fn cache_guard_fires_for_a_cache_heavy_role_on_a_non_caching_provider() {
        // Only one non-caching model available → both cache-heavy roles land on
        // it and must warn.
        let models = [m("xai", "grok", "xai", 0.99, 0.90, 3.0, 15.0, false, false)];
        let recs = recommend(&models);
        for role in [WorkflowRole::Orchestrate, WorkflowRole::Edit] {
            let r = rec_for(&recs, role);
            assert!(
                r.warnings.iter().any(|w| w.contains("no prompt caching")),
                "{:?} on a non-caching provider must warn",
                role
            );
        }
        // MECHANICAL is not cache-heavy → no cache warning even on the same model.
        let mech = rec_for(&recs, WorkflowRole::Mechanical);
        assert!(!mech
            .warnings
            .iter()
            .any(|w| w.contains("no prompt caching")));
    }

    #[test]
    fn below_floor_edit_is_flagged() {
        // Best available diff-format is 0.90, below the 0.97 floor.
        let models = [m(
            "ollama",
            "qwen3-coder",
            "ollama",
            0.90,
            0.60,
            0.0,
            0.0,
            false,
            true,
        )];
        let recs = recommend(&models);
        let edit = rec_for(&recs, WorkflowRole::Edit);
        assert!(
            edit.warnings.iter().any(|w| w.contains("floor")),
            "an EDIT pick below the diff-format floor must be flagged"
        );
        assert!(!edit.floor_met);
        assert!(edit
            .warnings
            .iter()
            .any(|w| w.contains("failed-edit retries")));
    }

    #[test]
    fn no_local_model_yields_a_flagged_local_note() {
        let models = [m(
            "anthropic",
            "sonnet",
            "anthropic",
            0.98,
            0.77,
            3.0,
            15.0,
            true,
            false,
        )];
        let recs = recommend(&models);
        let local = rec_for(&recs, WorkflowRole::Local);
        assert!(local.model.is_empty());
        assert!(local.warnings.iter().any(|w| w.contains("no local model")));
    }

    #[test]
    fn empty_available_yields_no_recommendations() {
        assert!(recommend(&[]).is_empty());
    }

    #[test]
    fn from_tag_parses_roles_and_the_hard_alias() {
        assert_eq!(WorkflowRole::from_tag("edit"), Some(WorkflowRole::Edit));
        assert_eq!(
            WorkflowRole::from_tag("MECHANICAL"),
            Some(WorkflowRole::Mechanical)
        );
        assert_eq!(WorkflowRole::from_tag("review"), Some(WorkflowRole::Review));
        assert_eq!(WorkflowRole::from_tag("local"), Some(WorkflowRole::Local));
        // `hard` is the frontier-reasoning alias for orchestrate.
        assert_eq!(
            WorkflowRole::from_tag(" hard "),
            Some(WorkflowRole::Orchestrate)
        );
        assert_eq!(
            WorkflowRole::from_tag("orchestrate"),
            Some(WorkflowRole::Orchestrate)
        );
        // Round-trips with as_str for every role (config key stability).
        for role in WorkflowRole::all() {
            assert_eq!(WorkflowRole::from_tag(role.as_str()), Some(role));
        }
        assert_eq!(WorkflowRole::from_tag("nonsense"), None);
    }

    #[test]
    fn recommendations_are_in_role_order() {
        let models = [
            m(
                "anthropic",
                "sonnet",
                "anthropic",
                0.98,
                0.77,
                3.0,
                15.0,
                true,
                false,
            ),
            m(
                "ollama",
                "qwen3-coder",
                "ollama",
                0.86,
                0.55,
                0.0,
                0.0,
                false,
                true,
            ),
        ];
        let recs = recommend(&models);
        let roles: Vec<WorkflowRole> = recs.iter().map(|r| r.role).collect();
        assert_eq!(roles, WorkflowRole::all().to_vec());
    }

    // ── Proportional floor selection + reviewer assignment (ruling 2026-09-17) ──

    #[derive(Debug)]
    struct Cand {
        id: &'static str,
        strength: f64,
        cost: f64,
    }

    fn floor_pick(cands: &[Cand], floor: f64) -> Option<(&'static str, bool)> {
        let refs: Vec<&Cand> = cands.iter().collect();
        cheapest_clearing_floor(
            &refs,
            floor,
            MATERIAL_FLOOR_SHORTFALL,
            |c: &Cand| c.strength,
            |c: &Cand| c.cost,
            |a: &Cand, b: &Cand| a.id.cmp(b.id),
        )
        .map(|p| (p.pick.id, p.clears_floor))
    }

    /// THE ruling case: "Being better than half a percent is not enough to
    /// warrant a 3x cost." A 0.005 edge at 3x never wins — whether the cheaper
    /// model clears the floor or sits just under it.
    #[test]
    fn half_a_percent_edge_at_three_times_the_price_never_wins() {
        assert!(!upgrade_is_proportionate(0.005, 5.0, 15.0));
        let both_clear = [
            Cand {
                id: "cheap",
                strength: 0.800,
                cost: 5.0,
            },
            Cand {
                id: "dear",
                strength: 0.805,
                cost: 15.0,
            },
        ];
        assert_eq!(floor_pick(&both_clear, 0.80), Some(("cheap", true)));
        let cheap_just_under = [
            Cand {
                id: "cheap",
                strength: 0.795,
                cost: 5.0,
            },
            Cand {
                id: "dear",
                strength: 0.800,
                cost: 15.0,
            },
        ];
        assert_eq!(
            floor_pick(&cheap_just_under, 0.80),
            Some(("cheap", false)),
            "0.005 short of the floor is not material, and +0.005 at 3x is not proportionate"
        );
        // Also the EDIT row that started it: Fable 5 over Sonnet 5 on the
        // diff-format axis, 0.5 points at 3.3x.
        assert!(!upgrade_is_proportionate(0.005, 18.0, 60.0));
    }

    /// A pricier model wins only when every cheaper one falls short: materially
    /// (not eligible), or within the margin but with a proportionate upgrade.
    #[test]
    fn a_pricier_model_wins_only_when_cheaper_ones_fall_short_proportionately() {
        let materially_short = [
            Cand {
                id: "cheap",
                strength: 0.70,
                cost: 1.0,
            },
            Cand {
                id: "dear",
                strength: 0.81,
                cost: 30.0,
            },
        ];
        assert_eq!(floor_pick(&materially_short, 0.80), Some(("dear", true)));

        let near_but_far_weaker = [
            Cand {
                id: "cheap",
                strength: 0.785,
                cost: 5.0,
            },
            Cand {
                id: "dear",
                strength: 0.90,
                cost: 15.0,
            },
        ];
        // +0.115 at 3x clears the 0.055 bar.
        assert_eq!(floor_pick(&near_but_far_weaker, 0.80), Some(("dear", true)));

        assert_eq!(
            floor_pick(
                &[Cand {
                    id: "weak",
                    strength: 0.50,
                    cost: 0.0,
                }],
                0.80
            ),
            None,
            "nothing within the margin → no pick"
        );
        // The proportionality curve itself.
        assert!((required_gain_for_price(5.0, 10.0) - 0.05 * 2f64.ln()).abs() < 1e-12);
        assert_eq!(required_gain_for_price(10.0, 5.0), 0.0);
        assert!(required_gain_for_price(0.0, 60.0).is_finite());
    }

    const ROUTINE_DIFF: &str = "diff --git a/src/pager.rs b/src/pager.rs\n\
--- a/src/pager.rs\n\
+++ b/src/pager.rs\n\
@@ -10,3 +10,3 @@\n\
-    let last = total / per_page;\n\
+    let last = (total + per_page - 1) / per_page;\n\
diff --git a/tests/pager_test.rs b/tests/pager_test.rs\n\
--- a/tests/pager_test.rs\n\
+++ b/tests/pager_test.rs\n\
@@ -1,1 +1,2 @@\n\
+    assert_eq!(last_page(11, 10), 2);\n";

    const SIGNATURE_DIFF: &str = "diff --git a/src/quote.rs b/src/quote.rs\n\
--- a/src/quote.rs\n\
+++ b/src/quote.rs\n\
@@ -1,3 +1,3 @@\n\
-pub fn shipping_quote(order: &Order) -> Quote {\n\
+pub fn shipping_quote(order: &Order, opts: &QuoteOpts) -> Quote {\n\
diff --git a/src/api.rs b/src/api.rs\n\
--- a/src/api.rs\n\
+++ b/src/api.rs\n\
@@ -5,1 +5,1 @@\n\
-    let q = shipping_quote(&order);\n\
+    let q = shipping_quote(&order, &opts);\n";

    fn avail(pairs: &[(&str, &str)]) -> Vec<AvailableModel> {
        pairs
            .iter()
            .map(|(p, m)| AvailableModel::new(p, m))
            .collect()
    }

    #[test]
    fn review_facts_measure_the_diff() {
        let f = ReviewFacts::from_diff(ROUTINE_DIFF, 0);
        assert_eq!(f.changed_lines, 3);
        assert_eq!(
            f.changed_files,
            vec![
                "src/pager.rs".to_string(),
                "tests/pager_test.rs".to_string()
            ]
        );
        assert_eq!((f.code_files, f.test_files), (1, 1));
        assert_eq!(f.languages, vec!["rust".to_string()]);
        assert!(f.signature_changes.is_empty());

        let f = ReviewFacts::from_diff(SIGNATURE_DIFF, 4);
        assert_eq!(f.signature_changes, vec!["shipping_quote".to_string()]);
        assert_eq!(f.call_sites, 4);

        let deleted = "diff --git a/old.py b/old.py\n--- a/old.py\n+++ /dev/null\n-print(1)\n";
        let f = ReviewFacts::from_diff(deleted, 0);
        assert_eq!(f.changed_files, vec!["old.py".to_string()]);
        assert_eq!(f.languages, vec!["python".to_string()]);
    }

    /// Routine small diff → the cheapest cross-family model clearing the routine
    /// floor, never the frontier row.
    #[test]
    fn routine_small_diff_gets_the_cheapest_clearing_reviewer() {
        let available = avail(&[
            ("anthropic", "claude-fable-5"),
            ("anthropic", "claude-sonnet-5"),
            ("openai", "gpt-5.6-mini"),
            ("zai", "glm-4.5-air"),
            ("custom_deepseek", "deepseek-chat"),
            ("google", "gemini-3-pro"),
        ]);
        let a = assign_reviewer(
            ReviewFacts::from_diff(ROUTINE_DIFF, 0),
            Some(("anthropic", "claude-sonnet-5")),
            &available,
        );
        assert_eq!(
            a.complexity,
            ReviewComplexity::Routine,
            "{:?}",
            a.complexity_reasons
        );
        assert_eq!(a.floor, REVIEW_FLOOR_ROUTINE);
        assert_eq!(a.pick_label().as_deref(), Some("zai/glm-4.5-air"));
        assert!(a.pick_clears_floor);
    }

    /// Complex (signature change with call sites) → a stronger model, but only
    /// because the cheaper ones miss the raised floor; a cheaper model that
    /// clears it still wins.
    #[test]
    fn complex_signature_change_uses_a_stronger_model_only_when_cheaper_ones_miss_the_floor() {
        let author = Some(("anthropic", "claude-sonnet-5"));
        let without_mid = avail(&[
            ("anthropic", "claude-fable-5"),
            ("zai", "glm-4.5-air"),
            ("custom_deepseek", "deepseek-chat"),
            ("google", "gemini-3-pro"),
            ("openai", "gpt-5.6"),
        ]);
        let a = assign_reviewer(
            ReviewFacts::from_diff(SIGNATURE_DIFF, 3),
            author,
            &without_mid,
        );
        assert_eq!(
            a.complexity,
            ReviewComplexity::Complex,
            "{:?}",
            a.complexity_reasons
        );
        assert_eq!(a.floor, REVIEW_FLOOR_COMPLEX);
        assert_eq!(
            a.pick_label().as_deref(),
            Some("google/gemini-3-pro"),
            "the cheap models are materially short of 0.80; gemini is the cheapest that clears"
        );

        let mut with_mid = without_mid.clone();
        with_mid.push(AvailableModel::new("zai", "glm-5.2"));
        let a = assign_reviewer(ReviewFacts::from_diff(SIGNATURE_DIFF, 3), author, &with_mid);
        assert_eq!(a.pick_label().as_deref(), Some("zai/glm-5.2"));
    }

    /// The decision carries its evidence: verdict, floor, reasons, the facts,
    /// every candidate with strength and price and status, the pick and why.
    #[test]
    fn the_recorded_reviewer_decision_contains_the_evidence() {
        let available = avail(&[
            ("anthropic", "claude-fable-5"),
            ("zai", "glm-4.5-air"),
            ("google", "gemini-3-pro"),
        ]);
        let a = assign_reviewer(
            ReviewFacts::from_diff(SIGNATURE_DIFF, 3),
            Some(("anthropic", "claude-sonnet-5")),
            &available,
        );
        let json = a.to_json();
        assert_eq!(json["complexity"], "complex");
        assert_eq!(json["floor"], REVIEW_FLOOR_COMPLEX);
        assert!(json["complexity_reasons"][0]
            .as_str()
            .unwrap()
            .contains("shipping_quote"));
        assert_eq!(json["facts"]["call_sites"], 3);
        assert_eq!(json["pick_provider"], "google");
        assert_eq!(json["pick_model"], "gemini-3-pro");
        assert!(!json["why"].as_str().unwrap().is_empty());
        let cands = json["candidates"].as_array().unwrap();
        assert_eq!(cands.len(), 3);
        let status_of = |model: &str| {
            cands
                .iter()
                .find(|c| c["model"] == model)
                .map(|c| c["status"].as_str().unwrap().to_string())
                .unwrap()
        };
        assert_eq!(status_of("claude-fable-5"), "same_family");
        assert_eq!(status_of("glm-4.5-air"), "materially_short");
        assert_eq!(status_of("gemini-3-pro"), "picked");
        for c in cands {
            assert!(c["orchestration"].is_number() && c["cost_per_mtok"].is_number());
        }
    }

    /// Lane R65a (r65 s1): an UNCERTAIN from the cheapest floor-clearing
    /// reviewer on a MODERATE review escalates to the next floor tier — the
    /// cheapest STRONGER cross-family model clearing the COMPLEX floor, not the
    /// frontier row.
    #[test]
    fn uncertain_moderate_review_escalates_to_the_cheapest_stronger_reviewer_at_the_next_floor() {
        let author = Some(("custom_deepseek", "deepseek-chat"));
        let available = avail(&[
            ("minimax", "MiniMax-M2.5"),
            ("zai", "glm-4.5-air"),
            ("zai", "glm-5.2"),
            ("google", "gemini-3-pro"),
            ("xai", "grok-4.5"),
            ("anthropic", "claude-fable-5"),
            ("custom_deepseek", "deepseek-chat"),
        ]);
        let untested = "--- a/src/a.rs\n+++ b/src/a.rs\n+a\n+b\n+c\n+d\n";
        let first = assign_reviewer(ReviewFacts::from_diff(untested, 0), author, &available);
        assert_eq!(first.complexity, ReviewComplexity::Moderate);
        assert_eq!(first.pick_label().as_deref(), Some("minimax/MiniMax-M2.5"));

        let next = escalate_reviewer_after_uncertain(
            &first,
            ("minimax", "MiniMax-M2.5"),
            author,
            &available,
        );
        assert_eq!(next.complexity, ReviewComplexity::Complex);
        assert_eq!(
            next.pick_label().as_deref(),
            Some("zai/glm-5.2"),
            "cheapest model above MiniMax's 0.80 that clears the complex floor: {}",
            next.why
        );
        assert!(
            next.why
                .contains("escalated once after minimax/MiniMax-M2.5"),
            "{}",
            next.why
        );
        assert!(
            next.candidates.iter().all(|c| c.orchestration > 0.80),
            "only reviewers ranking above the unsure one are considered"
        );
        assert!(next
            .candidates
            .iter()
            .all(|c| c.model != "MiniMax-M2.5" && c.family != "deepseek"));
    }

    /// At COMPLEX there is no higher floor: escalate to the strongest
    /// cross-family model ranking above the unsure reviewer; with nothing
    /// above it, there is no escalation pick.
    #[test]
    fn uncertain_complex_review_escalates_to_the_strongest_reviewer_above_it_or_none() {
        let author = Some(("anthropic", "claude-sonnet-5"));
        let available = avail(&[
            ("google", "gemini-3-pro"),
            ("zai", "glm-5.2"),
            ("xai", "grok-4.5"),
            ("openai", "gpt-5.6"),
        ]);
        let first = assign_reviewer(
            ReviewFacts::from_diff(SIGNATURE_DIFF, 3),
            author,
            &available,
        );
        assert_eq!(first.complexity, ReviewComplexity::Complex);
        assert_eq!(first.pick_label().as_deref(), Some("zai/glm-5.2"));

        let next =
            escalate_reviewer_after_uncertain(&first, ("zai", "glm-5.2"), author, &available);
        assert_eq!(
            next.pick_label().as_deref(),
            Some("xai/grok-4.5"),
            "{}",
            next.why
        );
        assert!(
            next.why.contains("strongest cross-family model"),
            "{}",
            next.why
        );
        assert_eq!(
            next.candidates
                .iter()
                .filter(|c| c.status == "picked")
                .count(),
            1
        );

        let top = assign_reviewer(
            ReviewFacts::from_diff(SIGNATURE_DIFF, 3),
            Some(("openai", "gpt-5.6")),
            &avail(&[("anthropic", "claude-fable-5"), ("zai", "glm-4.5-air")]),
        );
        assert_eq!(
            top.pick_label().as_deref(),
            Some("anthropic/claude-fable-5")
        );
        let none = escalate_reviewer_after_uncertain(
            &top,
            ("anthropic", "claude-fable-5"),
            Some(("openai", "gpt-5.6")),
            &avail(&[("anthropic", "claude-fable-5"), ("zai", "glm-4.5-air")]),
        );
        assert_eq!(none.pick_label(), None);
        assert!(none.why.contains("no cross-family model"), "{}", none.why);
    }

    #[test]
    fn unknown_author_gets_no_reviewer_pick() {
        let a = assign_reviewer(
            ReviewFacts::from_diff(ROUTINE_DIFF, 0),
            None,
            &avail(&[("google", "gemini-3-pro")]),
        );
        assert_eq!(a.pick_label(), None);
        assert!(a.why.contains("not recorded"));
    }

    /// Cross-language and many-file diffs are complex; multi-module or untested
    /// code is moderate — structure only.
    #[test]
    fn review_complexity_ladder() {
        let cross = "--- a/src/a.rs\n+++ b/src/a.rs\n+x\n--- a/ui/b.ts\n+++ b/ui/b.ts\n+y\n";
        let v = assess_review_complexity(&ReviewFacts::from_diff(cross, 0));
        assert_eq!(v.complexity, ReviewComplexity::Complex);

        let untested = "--- a/src/a.rs\n+++ b/src/a.rs\n+a\n+b\n+c\n+d\n";
        let v = assess_review_complexity(&ReviewFacts::from_diff(untested, 0));
        assert_eq!(v.complexity, ReviewComplexity::Moderate, "{:?}", v.reasons);
        assert!(v.reasons.iter().any(|r| r.contains("no test file")));
        assert_eq!(v.floor, REVIEW_FLOOR_MODERATE);
    }

    // ── Discovery ────────────────────────────────────────────────────────────

    fn provider(name: &str, key_env: &str, models: &[&str]) -> ProviderModels {
        ProviderModels {
            provider: name.to_string(),
            api_key_env: key_env.to_string(),
            models: models.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn keyed(set: &[&str]) -> impl Fn(&str) -> bool {
        let owned: Vec<String> = set.iter().map(|s| s.to_string()).collect();
        move |k| owned.iter().any(|s| s == k)
    }

    #[test]
    fn discovery_enumerates_only_configured_providers() {
        let providers = [
            provider("openai", "OPENAI_API_KEY", &["gpt-5.6", "gpt-5.6-mini"]),
            provider("minimax", "MINIMAX_API_KEY", &["MiniMax-M2.5"]),
            // Keyless local provider — always available, no key needed.
            provider("ollama", "", &["qwen3-coder"]),
        ];
        // Only OpenAI keyed + the keyless local provider are configured.
        let available = available_from(&providers, &keyed(&["OPENAI_API_KEY"]), None);
        let labels: Vec<String> = available.iter().map(|a| a.label()).collect();
        assert!(labels.contains(&"openai/gpt-5.6".to_string()));
        assert!(labels.contains(&"openai/gpt-5.6-mini".to_string()));
        assert!(labels.contains(&"ollama/qwen3-coder".to_string()));
        // MiniMax is unkeyed → NOT enumerated.
        assert!(!labels.iter().any(|l| l.starts_with("minimax/")));
    }

    #[test]
    fn discovery_includes_the_main_default_and_dedupes() {
        let providers = [provider("openai", "OPENAI_API_KEY", &["gpt-5.6"])];
        let main = Some(("anthropic".to_string(), "claude-sonnet-5".to_string()));
        let available = available_from(&providers, &keyed(&["OPENAI_API_KEY"]), main);
        let labels: Vec<String> = available.iter().map(|a| a.label()).collect();
        assert!(labels.contains(&"anthropic/claude-sonnet-5".to_string()));
        assert!(labels.contains(&"openai/gpt-5.6".to_string()));

        // A main default that duplicates a discovered model is not double-listed.
        let main_dup = Some(("openai".to_string(), "gpt-5.6".to_string()));
        let deduped = available_from(&providers, &keyed(&["OPENAI_API_KEY"]), main_dup);
        assert_eq!(
            deduped
                .iter()
                .filter(|a| a.label() == "openai/gpt-5.6")
                .count(),
            1
        );
    }

    /// F1: the canonical setup — an Anthropic key set + a local Ollama model
    /// present — must discover BOTH a builtin Anthropic entry AND the local Ollama
    /// entry. Before the fix, discovery enumerated only declarative providers and
    /// so missed the builtin `providers/*.rs` families entirely (finding just ~1
    /// model, then wrongly advising "install ollama").
    #[test]
    fn discovery_includes_keyed_builtins_and_a_detected_local_ollama() {
        // No declarative providers configured; a local ollama model was detected.
        let providers = discoverable_providers(Vec::new(), vec!["qwen3-coder".to_string()]);
        // Only the Anthropic key is set.
        let available = available_from(&providers, &keyed(&["ANTHROPIC_API_KEY"]), None);
        let labels: Vec<String> = available.iter().map(|a| a.label()).collect();

        assert!(
            labels.iter().any(|l| l.starts_with("anthropic/")),
            "a builtin Anthropic model must be discovered when ANTHROPIC_API_KEY is set: {labels:?}"
        );
        assert!(
            labels.contains(&"ollama/qwen3-coder".to_string()),
            "the detected local Ollama model must be discovered (keyless): {labels:?}"
        );
        // A builtin whose key is NOT set stays out.
        assert!(
            !labels.iter().any(|l| l.starts_with("openai/")),
            "OpenAI must not be discovered without OPENAI_API_KEY: {labels:?}"
        );
    }

    /// F1: the keyed builtin table sources its model ids from the knowledge base
    /// (single source of truth) and excludes keyless Ollama (discovered instead).
    #[test]
    fn builtin_provider_models_are_keyed_and_kb_sourced() {
        let builtins = builtin_provider_models();
        // Anthropic is present, keyed, and carries KB model ids.
        let anthropic = builtins
            .iter()
            .find(|p| p.provider == "anthropic")
            .expect("anthropic must be a keyed builtin");
        assert_eq!(anthropic.api_key_env, "ANTHROPIC_API_KEY");
        assert!(anthropic.models.iter().any(|m| m == "claude-sonnet-5"));
        // Ollama is keyless → not enumerated from the KB here.
        assert!(
            !builtins.iter().any(|p| p.provider == "ollama"),
            "keyless ollama must not be enumerated as a keyed builtin"
        );
    }

    /// Z.AI is discoverable the moment ZAI_API_KEY is set — the "just make them
    /// selectable" half of the change. Adding it to BUILTIN_PROVIDERS is what
    /// lets discovery enumerate its KB models at all.
    #[test]
    fn zai_is_a_keyed_builtin_with_kb_models() {
        let builtins = builtin_provider_models();
        let zai = builtins
            .iter()
            .find(|p| p.provider == "zai")
            .expect("zai must be a keyed builtin");
        assert_eq!(zai.api_key_env, "ZAI_API_KEY");
        assert!(
            zai.models.iter().any(|m| m == "glm-4.7"),
            "zai builtin must carry its KB model ids, got {:?}",
            zai.models
        );
        assert_eq!(provider_key_env("zai").as_deref(), Some("ZAI_API_KEY"));
    }

    /// F1 support: provider availability powers the summon consistency guard —
    /// keyless providers are always available, keyed ones need their key.
    #[test]
    fn is_provider_configured_handles_keyless_keyed_and_unknown() {
        // Ollama is keyless → always available regardless of env.
        assert!(is_provider_configured("ollama"));
        // An unknown/custom provider is assumed available (we can't see its setup).
        assert!(is_provider_configured("some-custom-provider-xyz"));
        // A keyed builtin resolves to its activation env var.
        assert_eq!(
            provider_key_env("anthropic").as_deref(),
            Some("ANTHROPIC_API_KEY")
        );
        assert_eq!(provider_key_env("ollama").as_deref(), Some(""));
    }

    #[test]
    fn ollama_base_url_mirrors_the_provider_host_resolution() {
        assert_eq!(ollama_base_url(None), "http://localhost:11434");
        assert_eq!(
            ollama_base_url(Some("  ".to_string())),
            "http://localhost:11434"
        );
        assert_eq!(
            ollama_base_url(Some("127.0.0.1".to_string())),
            "http://127.0.0.1:11434"
        );
        // A bare remote host: scheme added, no port assumed (matches the provider).
        assert_eq!(
            ollama_base_url(Some("mini.local:11434".to_string())),
            "http://mini.local:11434"
        );
        // A full URL is used as-is, trailing slash stripped.
        assert_eq!(
            ollama_base_url(Some("http://10.0.0.5:11434/".to_string())),
            "http://10.0.0.5:11434"
        );
    }

    #[test]
    fn parse_ollama_tags_keeps_reported_names_and_tolerates_garbage() {
        let body = r#"{"models":[{"name":"qwen3-coder:30b","size":1},{"name":"qwen3:latest"},{"name":"  "}]}"#;
        assert_eq!(
            parse_ollama_tags(body),
            vec!["qwen3-coder:30b".to_string(), "qwen3:latest".to_string()]
        );
        assert!(parse_ollama_tags("not json").is_empty());
        assert!(parse_ollama_tags("{}").is_empty());
    }

    /// The end-to-end shape for an Ollama install: a tagged pull resolves (as a
    /// family estimate) and is recommended, and is reported under
    /// `estimated_models` so the surface can label the score as approximate.
    #[test]
    fn tagged_ollama_models_are_recommended_and_reported_as_estimates() {
        let available = [
            AvailableModel::new("anthropic", "claude-sonnet-5"),
            AvailableModel::new("ollama", "qwen3-coder:30b"),
        ];
        let out = recommend_from_available(&available);
        assert!(out.considered.contains(&"ollama/qwen3-coder".to_string()));
        assert!(out.unknown_models.is_empty());
        assert_eq!(
            out.estimated_models,
            vec!["ollama/qwen3-coder:30b".to_string()]
        );
        let mech = out
            .recommendations
            .iter()
            .find(|r| r.role == WorkflowRole::Mechanical)
            .unwrap();
        assert_eq!(mech.provider, "ollama");
    }

    #[test]
    fn recommend_from_available_reports_known_and_unknown() {
        let available = [
            AvailableModel::new("anthropic", "claude-sonnet-5"), // known
            AvailableModel::new("acme", "mystery-model"),        // unknown
        ];
        let out = recommend_from_available(&available);
        assert!(out
            .considered
            .contains(&"anthropic/claude-sonnet-5".to_string()));
        assert!(out
            .unknown_models
            .contains(&"acme/mystery-model".to_string()));
        assert!(!out.recommendations.is_empty());
    }
}
