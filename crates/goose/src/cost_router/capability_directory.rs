//! The capability directory — one card per model the user actually has,
//! carrying its objective KB-heuristic strengths AND any durable graduation
//! evidence, so dispatch can plan on the strongest model and work on the
//! cheapest one it can trust, from a single source both the planner and the
//! worker router read.
//!
//! ## The ruling this implements (Jesse, 2026-09-09, verbatim intent)
//!
//! *"We should always use the most capable and advanced model to write the
//! plan... Use the best model we can to plan the work efficiently and with
//! costs in mind and then have it assign that work to cheaper worker models.
//! The harness should keep a directory of what models have what strengths
//! from my configured list so it can assign it appropriately for the best
//! outcome. It should never default to the most expensive model just because
//! it is the most capable."*
//!
//! ## Why this exists (root cause, Round C / Lane Z3 brief)
//!
//! `orchestrator.rs`'s roadmap decomposition (#249) shipped with a compiled-in
//! CHEAP default (`ollama/qwen2.5:7b`) for the PLANNING pass — the opposite of
//! the ruling. Its eligibility gate (`config::model_roles::assess_harness_
//! coordinator`) only trusted the one compiled-in `TrustedDefault`, so any
//! other candidate — cheap or strong — silently fell through to a
//! `tracing::warn` nobody sees. There was also no durable place to record
//! "this model graduated for this role on real harness evidence"; the static
//! knowledge base ([`super::knowledge`]) is an approximate, pre-graduation
//! heuristic, not a promotion record. This module is that place: a directory
//! built ONLY from models the user actually has, seeded from the KB and
//! mergeable with persisted evidence, that a planner-selection rule
//! ([`CapabilityDirectory::planner`]) and the worker router
//! ([`super::derived::derive_role_map`]) both read — so the directory and the
//! routing can never disagree.
//!
//! ## What a card is (G0 schema, `docs/orchestrator/CODING_HARNESS_ROLE_
//! GRADUATION_DAG.md`)
//!
//! One [`CapabilityCard`] per `(provider, model)` the user has configured or
//! pulled — never a vendor pack, never a model absent from
//! [`super::recommend::AvailableModel`] discovery. Its `strengths` are the KB
//! heuristic ([`super::knowledge::ModelKnowledge`]); its `roles` map carries a
//! [`RoleEvidence`] entry for every [`WorkflowRole`] — unrated is explicit,
//! never an absent key. `graduated=true` on a [`RoleEvidence`] cannot be
//! constructed (by [`RoleEvidence::new`] or by deserializing a persisted
//! file) without at least one evidence-run id and a snapshot date inside
//! [`super::knowledge::KB_SNAPSHOT_STALE_AFTER_DAYS`] — [`validate_graduation`]
//! is the single guarded gate both paths run through.
//!
//! ## What is NOT built here
//!
//! The full evidence PIPELINE (G2: parsing retained `permagent-eval` reports
//! into `RoleEvidence` rows automatically) and the tool/DAG-selection
//! micro-fixtures (G3) are out of this lane's scope — see the DAG doc. Today a
//! card's roles start `Unrated` (KB-seeded strengths only) unless a prior run
//! of this module persisted graduated evidence for it; nothing here invents a
//! pass rate.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::derived::DerivedRoleMap;
use super::knowledge::{lookup_with_confidence, KB_SNAPSHOT_STALE_AFTER_DAYS};
use super::recommend::{AvailableModel, WorkflowRole};
use super::role_map::RoleModel;
use crate::config::agent_identity::WorkerBillingClass;

/// How much orchestration strength the planner may trade away for a cheaper
/// model. Named and documented rather than an inline literal because it IS
/// the ruling's "with costs in mind" clause made concrete: two candidates
/// within this gap are, for planning purposes, indistinguishable in quality,
/// so cost decides between them; a gap wider than this is a real capability
/// difference the ruling says never to spend past ("never default to the
/// most expensive model just because it is the most capable" — the converse
/// also holds: never default to the *cheapest* when it is genuinely weaker).
/// 0.03 is one third of [`super::recommend::REVIEW_DIVERSITY_MAX_STRENGTH_GAP`]
/// (0.10) — tighter, because a wrong PLAN (this gate) costs every downstream
/// token, while REVIEW diversity is a secondary preference among already-
/// capable verifiers.
pub const PLANNER_STRENGTH_BAND: f64 = 0.03;

/// The micro-suite pass rate below which evidence EXCLUDES a role even when
/// the KB heuristic would otherwise admit it (Excellence DAG's ≥80% gate).
/// Evidence beats the KB heuristic in both directions: it can graduate a
/// model the KB alone wouldn't clear, and it can exclude one the KB alone
/// would have admitted.
pub const MICRO_SUITE_PASS_FLOOR: f64 = 0.80;

/// Where a card's data came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardSource {
    /// Strengths seeded from the static [`super::knowledge`] table.
    KnowledgeBase,
    /// The model has no [`super::knowledge`] row, but persisted evidence
    /// exists for it (a model that graduated on Permagent's own harness
    /// before ever earning a KB row).
    Evidence,
    /// Synthesized purely from an operator pin, with neither a KB row nor
    /// evidence yet. Reserved for a future pin-aware build path; not
    /// produced by [`CapabilityDirectory::build_with_persisted`] today.
    Pinned,
}

/// The KB-heuristic strength profile for one model. `mechanical`/`review` are
/// DERIVED booleans, not separate scored axes — this crate has no modeled
/// "tool support" signal beyond the two scored numbers, so "clears the role's
/// job" is defined exactly as [`WorkflowRole::capability_floor`] already
/// defines it for that role, never a new ad hoc heuristic.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StrengthProfile {
    /// [`super::knowledge::ModelKnowledge::orchestration_strength`] — the
    /// planning/orchestration axis.
    pub orchestration: f64,
    /// [`super::knowledge::ModelKnowledge::edit_format_reliability`].
    pub edit_format: f64,
    /// Whether this model clears [`WorkflowRole::Mechanical`]'s capability floor.
    pub mechanical: bool,
    /// Whether this model clears [`WorkflowRole::Review`]'s capability floor.
    pub review: bool,
}

/// Raw, unvalidated mirror of [`RoleEvidence`] used only as the serde
/// `try_from` source, so a corrupt or hand-edited JSON file cannot smuggle a
/// `graduated=true` claim past [`validate_graduation`].
#[derive(Debug, Clone, Deserialize)]
struct RoleEvidenceRaw {
    #[serde(default)]
    graduated: bool,
    #[serde(default)]
    smoke_passed: bool,
    #[serde(default)]
    micro_suite_pass_pct: Option<f64>,
    #[serde(default)]
    dag_selection_pct: Option<f64>,
    #[serde(default)]
    evidence_run_ids: Vec<String>,
    #[serde(default)]
    observed_cost_usd: Option<f64>,
    #[serde(default)]
    observed_latency_s: Option<f64>,
    #[serde(default)]
    snapshot_date: Option<NaiveDate>,
}

/// G0: whether a graduation claim is admissible. `graduated=false` needs
/// nothing (the default, unrated state). `graduated=true` requires at least
/// one evidence-run id AND a `snapshot_date` no older than
/// [`KB_SNAPSHOT_STALE_AFTER_DAYS`] as of `today` — a caller-supplied "today"
/// so the rule is testable without the wall clock (mirrors
/// [`super::knowledge::kb_is_stale`]'s own pattern).
pub fn validate_graduation(
    graduated: bool,
    evidence_run_ids: &[String],
    snapshot_date: Option<NaiveDate>,
    today: NaiveDate,
) -> Result<(), String> {
    if !graduated {
        return Ok(());
    }
    if evidence_run_ids.is_empty() {
        return Err("graduated=true requires at least one evidence_run_id".to_string());
    }
    match snapshot_date {
        None => Err("graduated=true requires a snapshot_date".to_string()),
        Some(d) => {
            if (today - d).num_days() > KB_SNAPSHOT_STALE_AFTER_DAYS {
                Err(format!(
                    "graduated=true snapshot_date {d} is stale (> {KB_SNAPSHOT_STALE_AFTER_DAYS} \
                     days old as of {today})"
                ))
            } else {
                Ok(())
            }
        }
    }
}

impl TryFrom<RoleEvidenceRaw> for RoleEvidence {
    type Error = String;
    fn try_from(raw: RoleEvidenceRaw) -> Result<Self, String> {
        RoleEvidence::new(
            raw.graduated,
            raw.smoke_passed,
            raw.micro_suite_pass_pct,
            raw.dag_selection_pct,
            raw.evidence_run_ids,
            raw.observed_cost_usd,
            raw.observed_latency_s,
            raw.snapshot_date,
            Utc::now().date_naive(),
        )
    }
}

/// One workflow role's graduation record for one model. Fields are private —
/// the ONLY way to construct a `graduated=true` instance is [`RoleEvidence::new`]
/// (direct calls) or deserializing a persisted file (via the guarded
/// `try_from`), both of which run [`validate_graduation`]. `Default` is the
/// unrated state every role starts in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RoleEvidenceRaw")]
pub struct RoleEvidence {
    graduated: bool,
    smoke_passed: bool,
    micro_suite_pass_pct: Option<f64>,
    dag_selection_pct: Option<f64>,
    evidence_run_ids: Vec<String>,
    observed_cost_usd: Option<f64>,
    observed_latency_s: Option<f64>,
    snapshot_date: Option<NaiveDate>,
}

impl Default for RoleEvidence {
    fn default() -> Self {
        Self {
            graduated: false,
            smoke_passed: false,
            micro_suite_pass_pct: None,
            dag_selection_pct: None,
            evidence_run_ids: Vec::new(),
            observed_cost_usd: None,
            observed_latency_s: None,
            snapshot_date: None,
        }
    }
}

impl RoleEvidence {
    /// Construct a validated evidence record. `today` is caller-supplied for
    /// testability; production callers pass `Utc::now().date_naive()`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        graduated: bool,
        smoke_passed: bool,
        micro_suite_pass_pct: Option<f64>,
        dag_selection_pct: Option<f64>,
        evidence_run_ids: Vec<String>,
        observed_cost_usd: Option<f64>,
        observed_latency_s: Option<f64>,
        snapshot_date: Option<NaiveDate>,
        today: NaiveDate,
    ) -> Result<Self, String> {
        validate_graduation(graduated, &evidence_run_ids, snapshot_date, today)?;
        Ok(Self {
            graduated,
            smoke_passed,
            micro_suite_pass_pct,
            dag_selection_pct,
            evidence_run_ids,
            observed_cost_usd,
            observed_latency_s,
            snapshot_date,
        })
    }

    pub fn graduated(&self) -> bool {
        self.graduated
    }
    pub fn smoke_passed(&self) -> bool {
        self.smoke_passed
    }
    pub fn micro_suite_pass_pct(&self) -> Option<f64> {
        self.micro_suite_pass_pct
    }
    pub fn dag_selection_pct(&self) -> Option<f64> {
        self.dag_selection_pct
    }
    pub fn evidence_run_ids(&self) -> &[String] {
        &self.evidence_run_ids
    }
    pub fn snapshot_date(&self) -> Option<NaiveDate> {
        self.snapshot_date
    }
}

/// One model's card: KB-heuristic strengths, billing class (never inferred
/// from the provider/model NAME — see [`billing_class_for`]), and a
/// [`RoleEvidence`] entry for every [`WorkflowRole`], keyed by
/// [`WorkflowRole::as_str`] so the persisted JSON is a plain string map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityCard {
    pub provider: String,
    pub model: String,
    pub display_name: String,
    pub family: String,
    pub billing_class: WorkerBillingClass,
    pub is_local: bool,
    pub context_window: u32,
    pub input_usd_per_mtok: f64,
    pub output_usd_per_mtok: f64,
    pub strengths: StrengthProfile,
    pub roles: BTreeMap<String, RoleEvidence>,
    pub source: CardSource,
}

impl CapabilityCard {
    /// This card's evidence for `role`. Always `Some` for a card built by
    /// [`CapabilityDirectory::build_with_persisted`] — every role is seeded
    /// explicitly, unrated is never an absent key.
    pub fn role(&self, role: WorkflowRole) -> Option<&RoleEvidence> {
        self.roles.get(role.as_str())
    }

    /// The blended reference price (input + output per MTok; `0.0` for
    /// local) — same shape as [`super::knowledge::ModelKnowledge::
    /// blended_cost_per_mtok`], so the planner and the KB rank models the
    /// same way.
    pub fn blended_cost_per_mtok(&self) -> f64 {
        if self.is_local {
            0.0
        } else {
            self.input_usd_per_mtok + self.output_usd_per_mtok
        }
    }
}

/// Classify a model's billing from the CONFIGURED worker roster
/// (`config::agent_identity::load_agent_config`), never from the provider or
/// model string. A local model is [`WorkerBillingClass::LocalFree`] by
/// construction (on-device, $0). Otherwise: if a roster persona names this
/// exact `(provider, model)` pair, its own declared `cost_tier` decides
/// (Subscription vs PaidApi); with no such persona, a keyed cloud API is
/// billed per token by default — [`WorkerBillingClass::PaidApi`], never
/// guessed as free or subscription from how the id looks.
pub fn billing_class_for(provider: &str, model: &str, is_local: bool) -> WorkerBillingClass {
    if is_local {
        return WorkerBillingClass::LocalFree;
    }
    let agent_cfg = crate::config::agent_identity::load_agent_config();
    for persona in agent_cfg.workers.values() {
        let provider_matches = persona.provider.as_deref() == Some(provider);
        let model_matches = persona.model.as_deref() == Some(model);
        if provider_matches && model_matches {
            if let Ok(bc) = persona.configured_billing_class() {
                return bc;
            }
        }
    }
    WorkerBillingClass::PaidApi
}

/// Schema version for the persisted directory file (G0: versioned, additive).
pub const SCHEMA_VERSION: u32 = 1;

/// One capability card per provider/model the user actually has, plus
/// whatever graduation evidence has been persisted for it. See the module
/// doc for the full contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityDirectory {
    pub schema: u32,
    pub generated_at: DateTime<Utc>,
    pub cards: Vec<CapabilityCard>,
}

impl CapabilityDirectory {
    /// The persisted directory file location: `<data dir>/models/
    /// capability_directory.json` — the same `models` directory
    /// `providers::local_inference::local_model_registry` already uses
    /// (`Paths::in_data_dir("models")`), not a hardcoded `~/Library` path.
    pub fn default_path() -> PathBuf {
        crate::config::paths::Paths::in_data_dir("models").join("capability_directory.json")
    }

    /// Pure core: build a directory from the models the user actually has
    /// ([`AvailableModel`] — configured/keyed providers + pulled Ollama
    /// tags), seeding strengths from the KB and merging in any evidence
    /// carried by `persisted` for the SAME `(provider, model)`. A KB model
    /// the user does not have never appears (never a vendor pack). A model
    /// with no KB row is dropped UNLESS `persisted` already carries real
    /// evidence for it (the "graduated before it ever earned a KB row" case,
    /// `source = Evidence`).
    pub fn build_with_persisted(
        available: &[AvailableModel],
        persisted: Option<&CapabilityDirectory>,
    ) -> CapabilityDirectory {
        let mut cards: Vec<CapabilityCard> = Vec::new();
        let mut seen: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        for a in available {
            let dedup_key = (a.provider.clone(), a.model.clone());
            if !seen.insert(dedup_key) {
                continue;
            }
            let persisted_card = persisted.and_then(|d| {
                d.cards
                    .iter()
                    .find(|c| c.provider == a.provider && c.model == a.model)
            });

            let Some((known, _confidence)) = lookup_with_confidence(&a.provider, &a.model) else {
                // Not in the static KB — carry forward ONLY when persisted
                // evidence is real (G6's "graduated before a KB row exists").
                if let Some(p) = persisted_card {
                    let has_evidence = p.roles.values().any(|e| !e.evidence_run_ids().is_empty());
                    if has_evidence {
                        cards.push(CapabilityCard {
                            source: CardSource::Evidence,
                            ..p.clone()
                        });
                    }
                }
                continue;
            };

            let mut roles: BTreeMap<String, RoleEvidence> = WorkflowRole::all()
                .iter()
                .map(|r| (r.as_str().to_string(), RoleEvidence::default()))
                .collect();
            if let Some(p) = persisted_card {
                for (role_key, evidence) in &p.roles {
                    roles.insert(role_key.clone(), evidence.clone());
                }
            }

            let strengths = StrengthProfile {
                orchestration: known.orchestration_strength,
                edit_format: known.edit_format_reliability,
                mechanical: WorkflowRole::Mechanical.capability_floor().clears(known),
                review: WorkflowRole::Review.capability_floor().clears(known),
            };

            cards.push(CapabilityCard {
                provider: a.provider.clone(),
                model: a.model.clone(),
                display_name: known.display_name.to_string(),
                family: known.family.to_string(),
                billing_class: billing_class_for(&a.provider, &a.model, known.is_local),
                is_local: known.is_local,
                context_window: known.context_window,
                input_usd_per_mtok: known.input_usd_per_mtok,
                output_usd_per_mtok: known.output_usd_per_mtok,
                strengths,
                roles,
                source: CardSource::KnowledgeBase,
            });
        }
        cards.sort_by(|x, y| x.provider.cmp(&y.provider).then(x.model.cmp(&y.model)));
        CapabilityDirectory {
            schema: SCHEMA_VERSION,
            generated_at: Utc::now(),
            cards,
        }
    }

    /// Thin IO wrapper: [`Self::build_with_persisted`] over the last
    /// persisted file at [`Self::default_path`] (tolerant of a missing file
    /// — a KB-only directory).
    pub fn build(available: &[AvailableModel]) -> CapabilityDirectory {
        let persisted = Self::load_persisted(&Self::default_path());
        Self::build_with_persisted(available, persisted.as_ref())
    }

    /// Load a persisted directory from `path`. Tolerant: a missing or
    /// corrupt file returns `None` rather than an error — the caller then
    /// builds a KB-only directory, never blocking on a bad cache file.
    pub fn load_persisted(path: &Path) -> Option<CapabilityDirectory> {
        let s = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&s).ok()
    }

    /// Atomic write: a sibling `.json.tmp` file, then rename over the
    /// target, mirroring the ledger pattern in
    /// `agents::platform_extensions::librarian_entities::save_ledger_to`.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)
    }

    pub fn save_default(&self) -> io::Result<()> {
        self.save(&Self::default_path())
    }

    /// IO entry point for `permagent packs directory --refresh`: probe live
    /// availability (config + Ollama), rebuild merging any persisted
    /// evidence forward, and write the result.
    pub async fn refresh() -> io::Result<CapabilityDirectory> {
        let available = super::recommend::discover_available_models_async().await;
        let persisted = Self::load_persisted(&Self::default_path());
        let dir = Self::build_with_persisted(&available, persisted.as_ref());
        dir.save_default()?;
        Ok(dir)
    }

    /// The planner rule (D2, the 2026-09-09 ruling): among cards that clear
    /// the ORCHESTRATE floor (or carry graduated evidence for it regardless
    /// of the KB heuristic), pick the STRONGEST by orchestration strength —
    /// unless a candidate within [`PLANNER_STRENGTH_BAND`] of the strongest
    /// is cheaper, in which case cost breaks the near-tie. `None` when no
    /// candidate clears the floor and none is graduated — callers must show
    /// this honestly (a visible reason), never substitute a default.
    ///
    /// This does NOT consult config pins — [`resolve_planner`] composes an
    /// explicit pin ahead of this, since a pin always outranks the
    /// automatic pick (`CODING_HARNESS_ROLE_GRADUATION_DAG.md` G4 rule 3).
    pub fn planner(&self) -> Option<(RoleModel, PlannerReason)> {
        let floor = WorkflowRole::Orchestrate
            .capability_floor()
            .orchestration_min;
        let candidates: Vec<&CapabilityCard> = self
            .cards
            .iter()
            .filter(|c| {
                c.strengths.orchestration >= floor
                    || c.role(WorkflowRole::Orchestrate)
                        .map(|e| e.graduated())
                        .unwrap_or(false)
            })
            .collect();
        if candidates.is_empty() {
            return None;
        }
        let ord = |a: &&CapabilityCard, b: &&CapabilityCard| {
            a.strengths
                .orchestration
                .total_cmp(&b.strengths.orchestration)
                .then_with(|| a.provider.cmp(&b.provider))
                .then_with(|| a.model.cmp(&b.model))
        };
        let strongest = *candidates.iter().max_by(ord)?;
        let band_floor = strongest.strengths.orchestration - PLANNER_STRENGTH_BAND;
        let pick = *candidates
            .iter()
            .filter(|c| c.strengths.orchestration >= band_floor)
            .min_by(|a, b| {
                a.blended_cost_per_mtok()
                    .total_cmp(&b.blended_cost_per_mtok())
                    .then_with(|| a.provider.cmp(&b.provider))
                    .then_with(|| a.model.cmp(&b.model))
            })?;
        let reason = if candidates.len() == 1 {
            PlannerReason::OnlyCandidate
        } else if pick.provider == strongest.provider && pick.model == strongest.model {
            PlannerReason::Strongest {
                orchestration: pick.strengths.orchestration,
            }
        } else {
            PlannerReason::CostTieBreak {
                orchestration: pick.strengths.orchestration,
                cost_per_mtok: pick.blended_cost_per_mtok(),
                lead_over_strongest: strongest.strengths.orchestration
                    - pick.strengths.orchestration,
            }
        };
        Some((
            RoleModel {
                provider: pick.provider.clone(),
                model: pick.model.clone(),
            },
            reason,
        ))
    }

    /// The `explain()` table: model, billing class, cost/Mtok, plan/edit
    /// strength, per-role graduated/unrated, and — reading `derived`, the
    /// SAME map dispatch actually uses — the current assignment per role
    /// with its reason, so this can never show a table that disagrees with
    /// routing.
    pub fn explain(&self, derived: &DerivedRoleMap) -> String {
        let mut out = String::new();
        out.push_str("Capability directory (models you have configured)\n");
        out.push_str(&format!(
            "{:<32} {:<12} {:>10} {:>6} {:>6}  roles\n",
            "model", "billing", "$/Mtok", "plan", "edit"
        ));
        for c in &self.cards {
            let cost = if c.is_local {
                "0.00 (local)".to_string()
            } else {
                format!("{:.2}", c.blended_cost_per_mtok())
            };
            let roles: Vec<String> = WorkflowRole::all()
                .iter()
                .map(|r| {
                    let graduated = c.role(*r).map(|e| e.graduated()).unwrap_or(false);
                    format!(
                        "{}={}",
                        r.as_str(),
                        if graduated { "graduated" } else { "unrated" }
                    )
                })
                .collect();
            out.push_str(&format!(
                "{:<32} {:<12} {:>10} {:>6.2} {:>6.2}  {}\n",
                format!("{}/{}", c.provider, c.model),
                c.billing_class.as_str(),
                cost,
                c.strengths.orchestration,
                c.strengths.edit_format,
                roles.join(", ")
            ));
        }
        out.push_str("\nCurrent assignment:\n");
        for role in WorkflowRole::all() {
            match derived.get(role) {
                Some((rm, prov)) => out.push_str(&format!(
                    "  {:<11} -> {}/{}  ({})\n",
                    role.as_str().to_uppercase(),
                    rm.provider,
                    rm.model,
                    prov.describe()
                )),
                None => out.push_str(&format!(
                    "  {:<11} -> session model (nothing clears the floor)\n",
                    role.as_str().to_uppercase()
                )),
            }
        }
        out
    }
}

/// Why the planner chose a route — carried into the roadmap receipt and the
/// `tracing::info` line so the choice is visible, never a silent
/// `tracing::warn` nobody reads.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlannerReason {
    /// Only one candidate cleared the floor / carried graduated evidence.
    OnlyCandidate,
    /// The strongest available orchestrator, with no cheaper contender
    /// inside [`PLANNER_STRENGTH_BAND`].
    Strongest { orchestration: f64 },
    /// A cheaper candidate within [`PLANNER_STRENGTH_BAND`] of the strongest
    /// was preferred on cost — "with costs in mind", never past the band.
    CostTieBreak {
        orchestration: f64,
        cost_per_mtok: f64,
        lead_over_strongest: f64,
    },
    /// An explicit config pin (`ORCHESTRATOR_DECOMPOSITION_*` or
    /// `PERMAGENT_ROLE_ORCHESTRATE_*`) named this route.
    Pinned { via: &'static str },
    /// Nothing clears the floor and nothing is pinned/graduated — the
    /// session model runs it, and this string is the visible reason.
    SessionFallback { reason: String },
}

impl PlannerReason {
    pub fn describe(&self) -> String {
        match self {
            PlannerReason::OnlyCandidate => {
                "only candidate clearing the ORCHESTRATE floor".to_string()
            }
            PlannerReason::Strongest { orchestration } => format!(
                "strongest available orchestrator (agentic {orchestration:.2}); no cheaper \
                 contender within the {PLANNER_STRENGTH_BAND:.2} strength band"
            ),
            PlannerReason::CostTieBreak {
                orchestration,
                cost_per_mtok,
                lead_over_strongest,
            } => format!(
                "within {PLANNER_STRENGTH_BAND:.2} of the strongest orchestrator (lead \
                 {lead_over_strongest:.3}) — picked the cheaper of the two (agentic \
                 {orchestration:.2}, ${cost_per_mtok:.2}/Mtok)"
            ),
            PlannerReason::Pinned { via } => format!("explicit pin ({via})"),
            PlannerReason::SessionFallback { reason } => reason.clone(),
        }
    }
}

/// Compose the decomposition-provider precedence (D2): an explicit
/// `ORCHESTRATOR_DECOMPOSITION_*` pin (kept for back-compat, treated as an
/// ORCHESTRATE pin) beats a `PERMAGENT_ROLE_ORCHESTRATE_*` pin, which beats
/// [`CapabilityDirectory::planner`], which beats `None` (the session model,
/// with a visible reason — never a silent `tracing::warn`). Pure: both pins
/// are pre-read by the caller so this stays testable without config/IO.
pub fn resolve_planner(
    decomposition_pin: Option<RoleModel>,
    role_pin: Option<RoleModel>,
    directory: &CapabilityDirectory,
) -> (Option<RoleModel>, PlannerReason) {
    if let Some(rm) = decomposition_pin {
        return (
            Some(rm),
            PlannerReason::Pinned {
                via: "ORCHESTRATOR_DECOMPOSITION_PROVIDER/MODEL",
            },
        );
    }
    if let Some(rm) = role_pin {
        return (
            Some(rm),
            PlannerReason::Pinned {
                via: "PERMAGENT_ROLE_ORCHESTRATE_PROVIDER/MODEL",
            },
        );
    }
    if let Some((rm, reason)) = directory.planner() {
        return (Some(rm), reason);
    }
    (
        None,
        PlannerReason::SessionFallback {
            reason: "no model in the directory clears the ORCHESTRATE floor (0.80) and none is \
                     pinned or graduated — using the session coordinator"
                .to_string(),
        },
    )
}

/// Whether a PINNED provider/model is eligible to plan: it clears the
/// ORCHESTRATE floor OR carries graduated evidence for that role. This is the
/// D2 replacement for the old "`TrustedDefault` only" gate on the
/// decomposition-pin path — see `config::model_roles::assess_harness_
/// coordinator`'s doc comment for the distinction from the session-
/// coordinator gate, which this function does NOT replace. Checks the
/// directory first (evidence beats the heuristic); falls back to a direct KB
/// lookup when the pin names a model with no card in `directory` (e.g. not
/// currently available) — `Unrated`/`BelowFloor` still refuse, matching the
/// prior gate's contract.
pub fn pin_clears_orchestrate_floor(
    provider: &str,
    model: &str,
    directory: &CapabilityDirectory,
) -> bool {
    if let Some(card) = directory
        .cards
        .iter()
        .find(|c| c.provider == provider && c.model == model)
    {
        if card
            .role(WorkflowRole::Orchestrate)
            .map(|e| e.graduated())
            .unwrap_or(false)
        {
            return true;
        }
        return card.strengths.orchestration
            >= WorkflowRole::Orchestrate
                .capability_floor()
                .orchestration_min;
    }
    match lookup_with_confidence(provider, model) {
        Some((k, _)) => {
            k.orchestration_strength
                >= WorkflowRole::Orchestrate
                    .capability_floor()
                    .orchestration_min
        }
        None => false,
    }
}

/// D2's evidence-beats-heuristic-in-both-directions rule: a card whose
/// evidence for `role` shows a micro-suite pass rate below
/// [`MICRO_SUITE_PASS_FLOOR`] is EXCLUDED from that role even though the KB
/// heuristic (capability floor) would otherwise admit it. A card with no
/// such evidence (the common case today — no evidence pipeline populates it
/// yet) is never excluded by this rule; it is a no-op until real evidence
/// lands, wired in now so the exclusion path exists and is tested.
pub fn evidence_excludes_role(card: &CapabilityCard, role: WorkflowRole) -> bool {
    card.role(role)
        .and_then(|e| e.micro_suite_pass_pct())
        .map(|p| p < MICRO_SUITE_PASS_FLOOR)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn am(provider: &str, model: &str) -> AvailableModel {
        AvailableModel::new(provider, model)
    }

    fn today() -> NaiveDate {
        chrono::Utc::now().date_naive()
    }

    // ── G0 schema: graduation is guarded, not merely advisory ───────────────

    #[test]
    fn graduated_requires_evidence_ids() {
        let err = RoleEvidence::new(
            true,
            true,
            Some(0.9),
            Some(0.95),
            Vec::new(),
            None,
            None,
            Some(today()),
            today(),
        )
        .expect_err("graduated=true with zero evidence_run_ids must be rejected");
        assert!(err.contains("evidence_run_id"), "{err}");

        let err = RoleEvidence::new(
            true,
            true,
            Some(0.9),
            Some(0.95),
            vec!["run-1".to_string()],
            None,
            None,
            None,
            today(),
        )
        .expect_err("graduated=true with no snapshot_date must be rejected");
        assert!(err.contains("snapshot_date"), "{err}");

        let stale = today() - chrono::Duration::days(KB_SNAPSHOT_STALE_AFTER_DAYS + 1);
        let err = RoleEvidence::new(
            true,
            true,
            Some(0.9),
            Some(0.95),
            vec!["run-1".to_string()],
            None,
            None,
            Some(stale),
            today(),
        )
        .expect_err("a stale snapshot_date must be rejected");
        assert!(err.contains("stale"), "{err}");

        // A valid graduation is accepted.
        let ok = RoleEvidence::new(
            true,
            true,
            Some(0.9),
            Some(0.95),
            vec!["run-1".to_string()],
            None,
            None,
            Some(today()),
            today(),
        )
        .expect("valid graduation must be accepted");
        assert!(ok.graduated());

        // The deserialize path (a hand-edited or corrupted file) runs the
        // SAME guard, via `#[serde(try_from = "RoleEvidenceRaw")]`.
        let bad_json = r#"{"graduated":true,"smoke_passed":true,"evidence_run_ids":[]}"#;
        assert!(
            serde_json::from_str::<RoleEvidence>(bad_json).is_err(),
            "deserializing a graduated=true record with no evidence must fail"
        );
        // Unrated (the default) always deserializes fine.
        let unrated_json = r#"{}"#;
        let unrated: RoleEvidence =
            serde_json::from_str(unrated_json).expect("unrated must deserialize with defaults");
        assert!(!unrated.graduated());
    }

    // ── Directory construction ───────────────────────────────────────────

    #[test]
    fn directory_built_only_from_available_models() {
        let available = vec![am("anthropic", "claude-sonnet-5")];
        let dir = CapabilityDirectory::build_with_persisted(&available, None);
        assert_eq!(dir.cards.len(), 1);
        assert_eq!(dir.cards[0].provider, "anthropic");
        assert_eq!(dir.cards[0].model, "claude-sonnet-5");
        // A KB model the user does NOT have (claude-opus-4-8) never appears,
        // even though it is a real, well-known KB row.
        assert!(!dir.cards.iter().any(|c| c.model == "claude-opus-4-8"));
        // Every role is explicit for the one card that IS present — never an
        // absent key standing in for "unrated".
        for role in WorkflowRole::all() {
            assert!(dir.cards[0].role(role).is_some());
            assert!(!dir.cards[0].role(role).unwrap().graduated());
        }
    }

    #[test]
    fn directory_round_trip() {
        let available = vec![
            am("anthropic", "claude-sonnet-5"),
            am("ollama", "qwen3-coder"),
        ];
        let dir = CapabilityDirectory::build_with_persisted(&available, None);
        let tmp = tempfile::NamedTempFile::new().unwrap();
        dir.save(tmp.path()).expect("save must succeed");
        let loaded =
            CapabilityDirectory::load_persisted(tmp.path()).expect("a just-written file must load");
        assert_eq!(loaded.schema, dir.schema);
        assert_eq!(loaded.cards.len(), dir.cards.len());
        for (a, b) in dir.cards.iter().zip(loaded.cards.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn missing_file_loads_as_none_kb_only_directory() {
        let path = std::path::Path::new("/tmp/permagent-z3-does-not-exist-capdir.json");
        assert!(CapabilityDirectory::load_persisted(path).is_none());
        // build() with no persisted file present tolerates it and still
        // produces a directory from the KB alone.
        let available = vec![am("anthropic", "claude-sonnet-5")];
        let dir = CapabilityDirectory::build_with_persisted(&available, None);
        assert_eq!(dir.cards.len(), 1);
    }

    // ── Billing class: never inferred from the name ──────────────────────

    #[test]
    fn billing_class_is_local_free_for_local_and_paid_api_by_default_for_cloud() {
        assert_eq!(
            billing_class_for("ollama", "qwen3-coder", true),
            WorkerBillingClass::LocalFree
        );
        // No roster persona names this provider/model → PaidApi default, not
        // guessed as Subscription from e.g. a "claude" substring.
        assert_eq!(
            billing_class_for("acme-cloud", "made-up-model", false),
            WorkerBillingClass::PaidApi
        );
    }

    // ── D2: evidence excludes a role even when the heuristic would admit it ─

    fn card_with_role_evidence(pass_pct: Option<f64>) -> CapabilityCard {
        let mut roles: BTreeMap<String, RoleEvidence> = WorkflowRole::all()
            .iter()
            .map(|r| (r.as_str().to_string(), RoleEvidence::default()))
            .collect();
        if let Some(pct) = pass_pct {
            roles.insert(
                WorkflowRole::Edit.as_str().to_string(),
                RoleEvidence::new(
                    false,
                    true,
                    Some(pct),
                    Some(0.9),
                    vec![],
                    None,
                    None,
                    None,
                    today(),
                )
                .unwrap(),
            );
        }
        CapabilityCard {
            provider: "openai".to_string(),
            model: "gpt-5.6-mini".to_string(),
            display_name: "GPT-5.6 mini".to_string(),
            family: "openai".to_string(),
            billing_class: WorkerBillingClass::PaidApi,
            is_local: false,
            context_window: 400_000,
            input_usd_per_mtok: 1.0,
            output_usd_per_mtok: 6.0,
            strengths: StrengthProfile {
                orchestration: 0.60,
                edit_format: 0.885,
                mechanical: true,
                review: false,
            },
            roles,
            source: CardSource::KnowledgeBase,
        }
    }

    #[test]
    fn evidence_below_80pct_excludes_role() {
        let excluded = card_with_role_evidence(Some(0.5));
        assert!(evidence_excludes_role(&excluded, WorkflowRole::Edit));

        let admitted = card_with_role_evidence(Some(0.95));
        assert!(!evidence_excludes_role(&admitted, WorkflowRole::Edit));

        let no_evidence = card_with_role_evidence(None);
        assert!(
            !evidence_excludes_role(&no_evidence, WorkflowRole::Edit),
            "no evidence must never exclude — this is a no-op until evidence exists"
        );
    }

    // ── The planner rule ──────────────────────────────────────────────────

    fn card(
        provider: &str,
        model: &str,
        orchestration: f64,
        input: f64,
        output: f64,
        is_local: bool,
    ) -> CapabilityCard {
        let roles: BTreeMap<String, RoleEvidence> = WorkflowRole::all()
            .iter()
            .map(|r| (r.as_str().to_string(), RoleEvidence::default()))
            .collect();
        CapabilityCard {
            provider: provider.to_string(),
            model: model.to_string(),
            display_name: model.to_string(),
            family: provider.to_string(),
            billing_class: if is_local {
                WorkerBillingClass::LocalFree
            } else {
                WorkerBillingClass::PaidApi
            },
            is_local,
            context_window: 128_000,
            input_usd_per_mtok: input,
            output_usd_per_mtok: output,
            strengths: StrengthProfile {
                orchestration,
                edit_format: 0.9,
                mechanical: true,
                review: true,
            },
            roles,
            source: CardSource::KnowledgeBase,
        }
    }

    fn directory_of(cards: Vec<CapabilityCard>) -> CapabilityDirectory {
        CapabilityDirectory {
            schema: SCHEMA_VERSION,
            generated_at: Utc::now(),
            cards,
        }
    }

    /// Real KB numbers: qwen2.5:7b-equivalent (0.30, $0), a cheap-but-below-
    /// floor deepseek-ish model (~0.62), and two real Anthropic rows
    /// (Sonnet 5 at 0.80, Opus 4.8 at 0.886). The planner must pick the
    /// STRONGEST above the floor (Opus), never the cheapest, and never qwen.
    #[test]
    fn planner_picks_strongest_not_cheapest() {
        let dir = directory_of(vec![
            card("ollama", "qwen2.5:7b", 0.30, 0.0, 0.0, true),
            card("custom_deepseek", "deepseek-chat", 0.62, 0.44, 1.32, false),
            card("anthropic", "claude-sonnet-5", 0.80, 3.0, 15.0, false),
            card("anthropic", "claude-opus-4-8", 0.886, 5.0, 25.0, false),
        ]);
        let (rm, reason) = dir.planner().expect("a floor-clearing candidate exists");
        assert_eq!(rm.provider, "anthropic");
        assert_eq!(rm.model, "claude-opus-4-8");
        assert_ne!(rm.model, "qwen2.5:7b");
        assert!(matches!(reason, PlannerReason::Strongest { .. }));
    }

    /// The tie-band, both directions: a 0.02 gap (inside the 0.03 band) lets
    /// cost decide; a 0.10 gap (outside it) does not — the pricier model
    /// wins because the strength gap is a real capability difference, not a
    /// rounding artifact.
    #[test]
    fn planner_never_most_expensive_by_default() {
        let inside_band = directory_of(vec![
            card("cheap_vendor", "cheap", 0.80, 1.0, 4.0, false), // $5/Mtok
            card("dear_vendor", "dear", 0.82, 5.0, 20.0, false),  // $25/Mtok
        ]);
        let (rm, reason) = inside_band.planner().unwrap();
        assert_eq!(
            rm.model, "cheap",
            "a 0.02 gap is inside the tie band — cost decides"
        );
        assert!(matches!(reason, PlannerReason::CostTieBreak { .. }));

        let outside_band = directory_of(vec![
            card("cheap_vendor", "cheap", 0.80, 1.0, 4.0, false),
            card("dear_vendor", "dear", 0.90, 5.0, 20.0, false),
        ]);
        let (rm, reason) = outside_band.planner().unwrap();
        assert_eq!(
            rm.model, "dear",
            "a 0.10 gap is outside the tie band — strength wins even though it costs more"
        );
        assert!(matches!(reason, PlannerReason::Strongest { .. }));
    }

    #[test]
    fn planner_pin_wins() {
        let dir = directory_of(vec![card(
            "anthropic",
            "claude-opus-4-8",
            0.886,
            5.0,
            25.0,
            false,
        )]);
        let pin = RoleModel {
            provider: "zai".to_string(),
            model: "glm-4.7".to_string(),
        };
        let (route, reason) = resolve_planner(Some(pin.clone()), None, &dir);
        assert_eq!(route, Some(pin));
        assert!(matches!(reason, PlannerReason::Pinned { .. }));

        // The ROLE pin wins over the directory's own pick, but loses to an
        // explicit decomposition pin.
        let role_pin = RoleModel {
            provider: "openai".to_string(),
            model: "gpt-5.6".to_string(),
        };
        let (route, reason) = resolve_planner(None, Some(role_pin.clone()), &dir);
        assert_eq!(route, Some(role_pin));
        assert!(matches!(reason, PlannerReason::Pinned { .. }));
    }

    #[test]
    fn planner_falls_back_to_session_with_reason() {
        let dir = directory_of(vec![card("ollama", "qwen3", 0.48, 0.0, 0.0, true)]); // below floor
        assert!(dir.planner().is_none());
        let (route, reason) = resolve_planner(None, None, &dir);
        assert_eq!(route, None);
        match reason {
            PlannerReason::SessionFallback { reason } => {
                assert!(
                    !reason.is_empty(),
                    "the fallback reason must be a visible string"
                );
                assert!(reason.contains("ORCHESTRATE"));
            }
            other => panic!("expected SessionFallback, got {other:?}"),
        }
    }

    #[test]
    fn pin_eligibility_refuses_unrated_and_below_floor() {
        let dir = directory_of(vec![card(
            "anthropic",
            "claude-sonnet-5",
            0.80,
            3.0,
            15.0,
            false,
        )]);
        assert!(pin_clears_orchestrate_floor(
            "anthropic",
            "claude-sonnet-5",
            &dir
        ));
        // Below floor, present in the directory.
        let below = directory_of(vec![card("ollama", "qwen3", 0.48, 0.0, 0.0, true)]);
        assert!(!pin_clears_orchestrate_floor("ollama", "qwen3", &below));
        // Unrated: not in the directory and not in the static KB either.
        assert!(!pin_clears_orchestrate_floor(
            "acme",
            "no-such-model",
            &below
        ));
        // Below-floor but a real KB row, absent from the (possibly stale)
        // directory: falls back to the KB lookup and still refuses.
        assert!(!pin_clears_orchestrate_floor(
            "ollama",
            "qwen3-coder",
            &below
        ));
    }
}
