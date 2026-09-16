//! The capability directory — one card per model the user actually has,
//! carrying its objective KB-heuristic strengths AND any durable graduation
//! evidence, so dispatch can plan on the strongest model and work on the
//! cheapest one it can trust, from a single source both the planner and the
//! worker router read.
//!
//! ## The ruling this implements (the user, 2026-09-09, verbatim intent)
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

/// The DAG/tool-selection pass rate a graduation claim must clear WHEN
/// `dag_selection_pct` is present (`CODING_HARNESS_ROLE_GRADUATION_DAG.md`:
/// "≥ 95% required (Excellence gate) — correct roadmap/dispatch tool chosen
/// over an equivalent"). Only Orchestrate's evidence populates this field
/// today, so the check is naturally role-scoped by which evidence carries a
/// value at all — [`validate_graduation`] enforces it whenever `Some`,
/// skipping it (not requiring it) when `None`, so a non-Orchestrate role's
/// evidence record is never rejected for a field its role doesn't produce.
pub const DAG_SELECTION_PASS_FLOOR: f64 = 0.95;

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
/// nothing (the default, unrated state). `graduated=true` requires: a passed
/// smoke test; a `micro_suite_pass_pct` present AND at/above
/// [`MICRO_SUITE_PASS_FLOOR`]; a `dag_selection_pct`, when present, at/above
/// [`DAG_SELECTION_PASS_FLOOR`] (absent is skipped, not required — only
/// Orchestrate's evidence populates it); at least one evidence-run id; AND a
/// `snapshot_date` no older than [`KB_SNAPSHOT_STALE_AFTER_DAYS`] as of
/// `today` — a caller-supplied "today" so the rule is testable without the
/// wall clock (mirrors [`super::knowledge::kb_is_stale`]'s own pattern).
///
/// G7 finding F3: pre-fix, this checked only the evidence-id and
/// snapshot-freshness pair — a `graduated=true` record with, say,
/// `micro_suite_pass_pct: Some(0.10)` was accepted outright, letting a stale
/// or malformed evidence write claim graduation the pass-rate floor never
/// actually supported.
#[allow(clippy::too_many_arguments)]
pub fn validate_graduation(
    graduated: bool,
    smoke_passed: bool,
    micro_suite_pass_pct: Option<f64>,
    dag_selection_pct: Option<f64>,
    evidence_run_ids: &[String],
    snapshot_date: Option<NaiveDate>,
    today: NaiveDate,
) -> Result<(), String> {
    if !graduated {
        return Ok(());
    }
    if !smoke_passed {
        return Err("graduated=true requires smoke_passed".to_string());
    }
    match micro_suite_pass_pct {
        None => return Err("graduated=true requires a micro_suite_pass_pct".to_string()),
        Some(p) if p < MICRO_SUITE_PASS_FLOOR => {
            return Err(format!(
                "graduated=true requires micro_suite_pass_pct >= {MICRO_SUITE_PASS_FLOOR:.2} \
                 (got {p:.2})"
            ));
        }
        Some(_) => {}
    }
    if let Some(d) = dag_selection_pct {
        if d < DAG_SELECTION_PASS_FLOOR {
            return Err(format!(
                "graduated=true requires dag_selection_pct >= {DAG_SELECTION_PASS_FLOOR:.2} \
                 when present (got {d:.2})"
            ));
        }
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
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
        validate_graduation(
            graduated,
            smoke_passed,
            micro_suite_pass_pct,
            dag_selection_pct,
            &evidence_run_ids,
            snapshot_date,
            today,
        )?;
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
    /// The evidence run's observed cost, USD. Added for Lane Z4's evidence
    /// pipeline (`permagent-eval graduate`, G2): a run that reports no cost
    /// for a field must be able to preserve a PRIOR observation rather than
    /// silently zeroing it, which requires reading the existing value first.
    pub fn observed_cost_usd(&self) -> Option<f64> {
        self.observed_cost_usd
    }
    /// The evidence run's observed wall-clock latency, seconds. See
    /// [`Self::observed_cost_usd`]'s doc for why this getter exists.
    pub fn observed_latency_s(&self) -> Option<f64> {
        self.observed_latency_s
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
    ///
    /// G7 finding F3: a card is first filtered through
    /// [`evidence_excludes_role`] — real micro-suite evidence below
    /// [`MICRO_SUITE_PASS_FLOOR`] disqualifies it as a candidate at all,
    /// regardless of the KB heuristic OR a `graduated` flag (a stale/pre-fix
    /// `graduated=true` with a failing pass rate must never win the planner
    /// pick just because the flag says so).
    pub fn planner(&self) -> Option<(RoleModel, PlannerReason)> {
        let floor = WorkflowRole::Orchestrate
            .capability_floor()
            .orchestration_min;
        let candidates: Vec<&CapabilityCard> = self
            .cards
            .iter()
            .filter(|c| !evidence_excludes_role(c, WorkflowRole::Orchestrate))
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
        let strongest = *candidates.iter().max_by(|a, b| {
            a.strengths
                .orchestration
                .total_cmp(&b.strengths.orchestration)
                .then_with(|| a.provider.cmp(&b.provider))
                .then_with(|| a.model.cmp(&b.model))
        })?;
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

    /// G7 finding F2: the model dispatch actually uses for `role` (or `None`
    /// for the session model), plus a short label naming WHERE it came from
    /// (`pinned: ...` / `derived (...)` / a session-fallback reason). Pure —
    /// both pins are pre-read by the caller, same contract as
    /// [`resolve_planner`] — so this is testable without touching
    /// Config/IO. Reuses the EXACT precedence real dispatch uses per role:
    /// Orchestrate goes through [`resolve_planner`] (an
    /// `ORCHESTRATOR_DECOMPOSITION_*` pin, then a
    /// `PERMAGENT_ROLE_ORCHESTRATE_*` pin gated by
    /// [`pin_clears_orchestrate_floor`], then [`Self::planner`], then the
    /// session) — the same composition `resolve_planner_route` performs, only
    /// with the directory/pins handed in rather than freshly discovered.
    /// Every other role goes through the SAME precedence
    /// `role_model_or_derived` implements: a `PERMAGENT_ROLE_<ROLE>_*` pin,
    /// then the derived map, then the session.
    pub fn current_assignment(
        &self,
        role: WorkflowRole,
        derived: &DerivedRoleMap,
        decomposition_pin: Option<RoleModel>,
        role_pin: Option<RoleModel>,
    ) -> (Option<RoleModel>, String) {
        if role == WorkflowRole::Orchestrate {
            let (route, reason) = resolve_planner(decomposition_pin, role_pin, self);
            let Some(rm) = route else {
                return (None, reason.describe());
            };
            if matches!(reason, PlannerReason::Pinned { .. })
                && !pin_clears_orchestrate_floor(&rm.provider, &rm.model, self)
            {
                return (
                    None,
                    format!(
                        "pinned planner {}/{} is unrated/below the ORCHESTRATE floor — using \
                         the session coordinator",
                        rm.provider, rm.model
                    ),
                );
            }
            let label = if matches!(reason, PlannerReason::Pinned { .. }) {
                format!("pinned: {}", reason.describe())
            } else {
                format!("derived: {}", reason.describe())
            };
            return (Some(rm), label);
        }
        // Non-Orchestrate precedence matches `resolve_role_model_or_derived`
        // exactly, including the enforceability filter it applies to BOTH the
        // pin and the derived fallback (e.g. a `cursor-agent` pin cannot
        // truthfully back a permission-gated worker) — so this table can
        // never show a row real dispatch would have refused.
        if let Some(rm) = role_pin.filter(super::role_map::role_model_is_enforceable) {
            return (Some(rm), "pinned".to_string());
        }
        match derived
            .get(role)
            .filter(|(rm, _)| super::role_map::role_model_is_enforceable(rm))
        {
            Some((rm, prov)) => (Some(rm.clone()), format!("derived ({})", prov.describe())),
            None => (
                None,
                "nothing clears the floor and nothing is pinned".to_string(),
            ),
        }
    }

    /// The `explain()` table: model, billing class, cost/Mtok, plan/edit
    /// strength, per-role graduated/unrated, and — going through the SAME
    /// pin-aware precedence [`Self::current_assignment`] composes from
    /// `derived` plus the caller-supplied pins — the current assignment per
    /// role with its source labeled (`pinned` / `derived` / session
    /// fallback), so this can never show a table that disagrees with what
    /// dispatch actually does (G7 finding F2: it previously read `derived`
    /// alone, so a hand-pinned role printed the algorithmic pick instead of
    /// what actually dispatches).
    pub fn explain(
        &self,
        derived: &DerivedRoleMap,
        decomposition_pin: Option<RoleModel>,
        role_pin: impl Fn(WorkflowRole) -> Option<RoleModel>,
    ) -> String {
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
            let dpin = if role == WorkflowRole::Orchestrate {
                decomposition_pin.clone()
            } else {
                None
            };
            let (assignment, label) = self.current_assignment(role, derived, dpin, role_pin(role));
            match assignment {
                Some(rm) => out.push_str(&format!(
                    "  {:<11} -> {}/{}  ({})\n",
                    role.as_str().to_uppercase(),
                    rm.provider,
                    rm.model,
                    label
                )),
                None => out.push_str(&format!(
                    "  {:<11} -> session model ({})\n",
                    role.as_str().to_uppercase(),
                    label
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
///
/// G7 finding F3: a card is checked through [`evidence_excludes_role`]
/// FIRST — real evidence below [`MICRO_SUITE_PASS_FLOOR`] refuses the pin
/// outright, before either the `graduated` flag or the KB heuristic get a
/// vote, for the same "a stale flag can never win" reason as [`Self::planner`].
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
        if evidence_excludes_role(card, WorkflowRole::Orchestrate) {
            return false;
        }
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

// ── G4/G6: dispatch-visible graduation reporting ────────────────────────────
//
// `list_workers`/`check_worker` (`orchestrator.rs::handle_list_workers`/
// `handle_check_worker`) must show a role's graduation BEFORE dispatch, not
// only discover it by failure (`CODING_HARNESS_ROLE_GRADUATION_DAG.md` G4
// rule 1). The three functions below are the pure core those two call sites
// share, so the JSON they render can be unit-tested without a live agent
// context (a `WorkerPersona` needs a full `OrchestratorClient`/DB/probe
// harness to construct end-to-end).

/// One role's graduation status, as reported to a caller deciding whether to
/// trust a model for that role. Three states, never collapsed to a bool:
/// `Unrated` (the default — no evidence either way) is a genuinely different
/// claim from `ExcludedByEvidence` (evidence exists and actively disqualifies
/// the role), and both differ from `Graduated`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleGraduationStatus {
    Graduated,
    ExcludedByEvidence,
    Unrated,
}

impl RoleGraduationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RoleGraduationStatus::Graduated => "graduated",
            RoleGraduationStatus::ExcludedByEvidence => "excluded_by_evidence",
            RoleGraduationStatus::Unrated => "unrated",
        }
    }
}

/// Classify `card`'s evidence for `role`. `graduated()` wins outright (it can
/// only be `true` through [`validate_graduation`], so it already implies the
/// evidence cleared every floor); otherwise [`evidence_excludes_role`] decides
/// between an active exclusion and plain unrated.
pub fn role_graduation_status(card: &CapabilityCard, role: WorkflowRole) -> RoleGraduationStatus {
    match card.role(role) {
        Some(e) if e.graduated() => RoleGraduationStatus::Graduated,
        _ if evidence_excludes_role(card, role) => RoleGraduationStatus::ExcludedByEvidence,
        _ => RoleGraduationStatus::Unrated,
    }
}

/// The full per-role graduation object for one card: every [`WorkflowRole`],
/// its [`RoleGraduationStatus`], the evidence-run COUNT (never the raw ids —
/// this is a summary surface, not an audit log) and the evidence's
/// `snapshot_date` (`None` when unrated). This is what a `list_workers`/
/// `check_worker` row's `"graduation"` field is built from.
pub fn card_role_graduation_json(card: &CapabilityCard) -> serde_json::Value {
    let mut roles = serde_json::Map::new();
    for role in WorkflowRole::all() {
        let evidence = card.role(role);
        let status = role_graduation_status(card, role);
        roles.insert(
            role.as_str().to_string(),
            serde_json::json!({
                "status": status.as_str(),
                "evidence_run_count": evidence.map(|e| e.evidence_run_ids().len()).unwrap_or(0),
                "snapshot_date": evidence.and_then(|e| e.snapshot_date()).map(|d| d.to_string()),
            }),
        );
    }
    serde_json::Value::Object(roles)
}

/// The `"graduation"` field for one roster worker: `card_role_graduation_json`
/// for its `(provider, model)` card, `null` when the persona names no
/// explicit provider/model (an in-process subagent that clones the session
/// model — nothing in the directory to report against), or a short note when
/// the persona names a pair the directory does not currently carry (not
/// available/probed at the last `--refresh`) so the caller can tell "no
/// evidence" apart from "not in the directory at all".
pub fn worker_graduation_json(
    directory: &CapabilityDirectory,
    provider: Option<&str>,
    model: Option<&str>,
) -> serde_json::Value {
    let (Some(provider), Some(model)) = (provider, model) else {
        return serde_json::Value::Null;
    };
    match directory
        .cards
        .iter()
        .find(|c| c.provider == provider && c.model == model)
    {
        Some(card) => card_role_graduation_json(card),
        None => serde_json::json!({
            "note": "not in the capability directory (unavailable or not yet probed) — \
                     run `permagent packs directory --refresh`",
        }),
    }
}

/// G4 rule 1, worker-dispatch side: when a goal is dispatched with an
/// ORCHESTRATE workflow role and `derived_role_map` had nothing for it
/// (`resolved == false`), [`super::derived::model_routing_receipt`]'s generic
/// "not derivable from the models available" message is technically true but
/// reads like plain absence — it does not say an ACTIVE, evidence-based
/// refusal happened. This enriches that one receipt with the same specific
/// reason `resolve_planner_route` already gives the roadmap-creation Decision
/// Inbox payload (`orchestrator.rs::create_roadmap`), so a worker dispatched
/// under the Orchestrate role carries the identical honest refusal instead of
/// a vaguer one. No-op for every other role, and a no-op when `resolved` is
/// `true` (nothing to explain).
pub fn annotate_orchestrate_refusal(
    mut receipt: serde_json::Value,
    role: Option<WorkflowRole>,
    resolved: bool,
    reason: &str,
) -> serde_json::Value {
    if role != Some(WorkflowRole::Orchestrate) || resolved {
        return receipt;
    }
    if let Some(obj) = receipt.as_object_mut() {
        obj.insert("refused".to_string(), serde_json::json!(true));
        obj.insert("refusal_reason".to_string(), serde_json::json!(reason));
        obj.insert(
            "summary".to_string(),
            serde_json::json!(format!(
                "ORCHESTRATE role refused for dispatch: {reason} — using the session model"
            )),
        );
    }
    receipt
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

    /// G7 finding F3: `validate_graduation` must cross-check `graduated=true`
    /// against the pass-rate floor, not just the evidence-id/snapshot pair.
    /// Pre-fix, every one of the `expect_err` cases below constructed fine
    /// (`RoleEvidence::new` only checked `evidence_run_ids` and
    /// `snapshot_date`), so a `graduated=true, micro_suite_pass_pct: Some(0.10)`
    /// record — "graduated=true without evidence actually supporting it" —
    /// was legal.
    #[test]
    fn validate_graduation_cross_checks_the_pass_rate_floor() {
        let run_ids = || vec!["run-1".to_string()];

        let err = RoleEvidence::new(
            true,
            false, // smoke NOT passed
            Some(0.95),
            Some(0.98),
            run_ids(),
            None,
            None,
            Some(today()),
            today(),
        )
        .expect_err("graduated=true with smoke_passed=false must be rejected");
        assert!(err.contains("smoke_passed"), "{err}");

        let err = RoleEvidence::new(
            true,
            true,
            Some(0.10), // well below the 0.80 floor
            Some(0.98),
            run_ids(),
            None,
            None,
            Some(today()),
            today(),
        )
        .expect_err("graduated=true with a below-floor micro_suite_pass_pct must be rejected");
        assert!(err.contains("micro_suite_pass_pct"), "{err}");

        let err = RoleEvidence::new(
            true,
            true,
            None, // no pass-rate recorded at all — nothing to cross-check
            Some(0.98),
            run_ids(),
            None,
            None,
            Some(today()),
            today(),
        )
        .expect_err("graduated=true with no micro_suite_pass_pct must be rejected");
        assert!(err.contains("micro_suite_pass_pct"), "{err}");

        let err = RoleEvidence::new(
            true,
            true,
            Some(0.95),
            Some(0.50), // present but below the 0.95 DAG-selection floor
            run_ids(),
            None,
            None,
            Some(today()),
            today(),
        )
        .expect_err("graduated=true with a below-floor dag_selection_pct must be rejected");
        assert!(err.contains("dag_selection_pct"), "{err}");

        // Absent dag_selection_pct is fine — only Orchestrate's evidence
        // populates it, and "the role requires it" must not become "every
        // role's evidence must carry it".
        let ok = RoleEvidence::new(
            true,
            true,
            Some(0.95),
            None,
            run_ids(),
            None,
            None,
            Some(today()),
            today(),
        )
        .expect("graduated=true with no dag_selection_pct must still be accepted");
        assert!(ok.graduated());
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

    /// G7 finding F3, second half: `planner()` and `pin_clears_orchestrate_floor`
    /// must route through `evidence_excludes_role` so a card whose real
    /// micro-suite evidence is below the floor is never a candidate — even
    /// with `graduated: true`. `validate_graduation` (the test above) now
    /// makes that combination impossible to construct through the public
    /// API, so this simulates the one place it could still appear: a
    /// grandfathered/stale record on disk from before that guard existed,
    /// via a raw struct literal (this same module can see `RoleEvidence`'s
    /// private fields) — real dispatch must still refuse it, not trust the
    /// flag.
    #[test]
    fn planner_and_pin_never_trust_a_stale_graduated_flag_over_the_evidence() {
        let mut roles: BTreeMap<String, RoleEvidence> = WorkflowRole::all()
            .iter()
            .map(|r| (r.as_str().to_string(), RoleEvidence::default()))
            .collect();
        roles.insert(
            WorkflowRole::Orchestrate.as_str().to_string(),
            RoleEvidence {
                graduated: true,
                smoke_passed: true,
                micro_suite_pass_pct: Some(0.10), // well below the 0.80 floor
                dag_selection_pct: Some(0.98),
                evidence_run_ids: vec!["run-1".to_string()],
                observed_cost_usd: None,
                observed_latency_s: None,
                snapshot_date: Some(today()),
            },
        );
        let mut stale_card = card("anthropic", "claude-opus-4-8", 0.886, 5.0, 25.0, false);
        stale_card.roles = roles;
        let dir = directory_of(vec![stale_card]);

        assert!(
            dir.planner().is_none(),
            "a card the evidence excludes must never be the planner pick, stale flag or not"
        );
        assert!(
            !pin_clears_orchestrate_floor("anthropic", "claude-opus-4-8", &dir),
            "a pin naming that same card must be refused for the same reason"
        );
    }

    // ── G7 finding F2: "Current assignment" is pin-aware ─────────────────────

    fn derived_with(role: WorkflowRole, rm: RoleModel) -> DerivedRoleMap {
        let mut derived = DerivedRoleMap::empty();
        derived.by_role.insert(role, rm);
        derived.provenance.insert(
            role,
            super::super::derived::Provenance {
                floor_met: true,
                confidence: crate::cost_router::knowledge::LookupConfidence::Exact,
                cost_hint: 5.0,
            },
        );
        derived
    }

    /// Pure formatting path, unpinned: a worker role with no
    /// `PERMAGENT_ROLE_*` pin must show the derived pick, labeled `derived
    /// (...)` — matching `resolve_role_model_or_derived`'s fallback tier.
    #[test]
    fn current_assignment_unpinned_role_shows_the_derived_pick() {
        let dir = directory_of(vec![]);
        let derived_pick = RoleModel {
            provider: "anthropic".to_string(),
            model: "claude-opus-4-8".to_string(),
        };
        let derived = derived_with(WorkflowRole::Edit, derived_pick.clone());

        let (rm, label) = dir.current_assignment(WorkflowRole::Edit, &derived, None, None);
        assert_eq!(rm, Some(derived_pick));
        assert!(
            label.starts_with("derived"),
            "unpinned role must be labeled as derived, got: {label}"
        );
    }

    /// Pure formatting path, pinned: G7's exact defect. Pre-fix, `explain()`
    /// read `derived` alone and had no notion of a pin at all — a hand-pinned
    /// role's row would show the algorithmic derived pick (or nothing, for
    /// Orchestrate, since a role pin bypasses `planner()` entirely) instead
    /// of what dispatch actually uses. The pin must win here and be labeled
    /// `pinned`, for both a worker role (`role_model_or_derived`'s
    /// precedence) and Orchestrate (`resolve_planner`'s precedence).
    #[test]
    fn current_assignment_pinned_role_shows_the_pin_not_the_derived_pick() {
        let derived_pick = RoleModel {
            provider: "anthropic".to_string(),
            model: "claude-opus-4-8".to_string(),
        };
        let pin = RoleModel {
            provider: "openai".to_string(),
            model: "gpt-5.6".to_string(),
        };

        // Worker role: a PERMAGENT_ROLE_EDIT_* pin must win over the derived map.
        let dir = directory_of(vec![]);
        let derived = derived_with(WorkflowRole::Edit, derived_pick.clone());
        let (rm, label) =
            dir.current_assignment(WorkflowRole::Edit, &derived, None, Some(pin.clone()));
        assert_eq!(rm, Some(pin.clone()));
        assert_eq!(label, "pinned");

        // Orchestrate: a graduated card exists for the derived pick, but a
        // PERMAGENT_ROLE_ORCHESTRATE_* pin must still win and must be
        // labeled "pinned: ..." — mirrors `resolve_planner`'s own precedence
        // test (`planner_pin_wins`), through the display path this time.
        let orch_dir = directory_of(vec![card(
            "anthropic",
            "claude-opus-4-8",
            0.886,
            5.0,
            25.0,
            false,
        )]);
        let orch_pin = RoleModel {
            provider: "acme".to_string(),
            model: "no-such-model".to_string(),
        };
        let (rm, label) = orch_dir.current_assignment(
            WorkflowRole::Orchestrate,
            &DerivedRoleMap::empty(),
            None,
            Some(orch_pin.clone()),
        );
        assert_eq!(
            rm, None,
            "acme/no-such-model is neither in this directory nor the static KB — the pin \
             must be refused (unrated), never silently substituted"
        );
        assert!(label.contains("ORCHESTRATE floor"), "label: {label}");
    }

    /// `explain()`'s printed table must route each role through
    /// `current_assignment` — same pin, same result — never disagree with
    /// what the pure helper says.
    #[test]
    fn explain_table_reflects_the_pin() {
        let dir = directory_of(vec![]);
        let derived = derived_with(
            WorkflowRole::Edit,
            RoleModel {
                provider: "anthropic".to_string(),
                model: "claude-opus-4-8".to_string(),
            },
        );
        let pin = RoleModel {
            provider: "openai".to_string(),
            model: "gpt-5.6".to_string(),
        };
        let pin_for_edit = pin.clone();
        let out = dir.explain(&derived, None, move |role| {
            (role == WorkflowRole::Edit).then(|| pin_for_edit.clone())
        });
        assert!(
            out.contains("EDIT        -> openai/gpt-5.6  (pinned)"),
            "table must show the pin, not the derived pick: {out}"
        );
    }

    // ── G4/G6: list_workers/check_worker graduation reporting ────────────

    /// A card whose EDIT role is graduated, whose MECHANICAL role's evidence
    /// actively excludes it (below-floor pass rate), and whose other two
    /// roles carry no evidence at all — the three statuses must be reported
    /// distinctly, never collapsed to a bool.
    fn card_with_mixed_role_statuses() -> CapabilityCard {
        let mut c = card_with_role_evidence(None);
        c.roles.insert(
            WorkflowRole::Edit.as_str().to_string(),
            RoleEvidence::new(
                true,
                true,
                Some(0.95),
                None,
                vec![
                    "run-1".to_string(),
                    "run-2".to_string(),
                    "run-3".to_string(),
                ],
                None,
                None,
                Some(today()),
                today(),
            )
            .unwrap(),
        );
        c.roles.insert(
            WorkflowRole::Mechanical.as_str().to_string(),
            RoleEvidence::new(
                false,
                true,
                Some(0.40), // below MICRO_SUITE_PASS_FLOOR
                None,
                vec!["run-4".to_string()],
                None,
                None,
                Some(today()),
                today(),
            )
            .unwrap(),
        );
        c
    }

    #[test]
    fn role_graduation_status_distinguishes_all_three_states() {
        let card = card_with_mixed_role_statuses();
        assert_eq!(
            role_graduation_status(&card, WorkflowRole::Edit),
            RoleGraduationStatus::Graduated
        );
        assert_eq!(
            role_graduation_status(&card, WorkflowRole::Mechanical),
            RoleGraduationStatus::ExcludedByEvidence
        );
        assert_eq!(
            role_graduation_status(&card, WorkflowRole::Review),
            RoleGraduationStatus::Unrated
        );
    }

    #[test]
    fn card_role_graduation_json_reports_every_role_with_counts_and_dates() {
        let card = card_with_mixed_role_statuses();
        let json = card_role_graduation_json(&card);
        let edit = &json[WorkflowRole::Edit.as_str()];
        assert_eq!(edit["status"], "graduated");
        assert_eq!(edit["evidence_run_count"], 3);
        assert_eq!(edit["snapshot_date"], today().to_string());

        let mech = &json[WorkflowRole::Mechanical.as_str()];
        assert_eq!(mech["status"], "excluded_by_evidence");
        assert_eq!(mech["evidence_run_count"], 1);

        let review = &json[WorkflowRole::Review.as_str()];
        assert_eq!(review["status"], "unrated");
        assert_eq!(review["evidence_run_count"], 0);
        assert!(review["snapshot_date"].is_null());

        // Every role is present explicitly — never a role silently absent
        // from the JSON standing in for "unrated".
        for role in WorkflowRole::all() {
            assert!(json.get(role.as_str()).is_some(), "{role:?} missing");
        }
    }

    #[test]
    fn worker_graduation_json_is_null_with_no_explicit_provider_or_model() {
        let dir = directory_of(vec![card_with_mixed_role_statuses()]);
        assert!(worker_graduation_json(&dir, None, None).is_null());
        assert!(worker_graduation_json(&dir, Some("openai"), None).is_null());
    }

    #[test]
    fn worker_graduation_json_notes_a_pair_absent_from_the_directory() {
        let dir = directory_of(vec![]);
        let v = worker_graduation_json(&dir, Some("openai"), Some("gpt-5.6-mini"));
        assert!(
            v["note"].as_str().unwrap().contains("--refresh"),
            "must point the caller at how to populate it: {v}"
        );
    }

    #[test]
    fn worker_graduation_json_matches_the_cards_own_graduation_json() {
        let card = card_with_mixed_role_statuses();
        let dir = directory_of(vec![card.clone()]);
        let via_worker = worker_graduation_json(&dir, Some(&card.provider), Some(&card.model));
        assert_eq!(via_worker, card_role_graduation_json(&card));
    }

    // ── G4: Orchestrate-role dispatch refusal is explicit, not generic ────

    #[test]
    fn annotate_orchestrate_refusal_is_a_noop_for_every_non_orchestrate_role() {
        let receipt = serde_json::json!({"role": "edit", "source": "session"});
        let out = annotate_orchestrate_refusal(
            receipt.clone(),
            Some(WorkflowRole::Edit),
            false,
            "irrelevant",
        );
        assert_eq!(out, receipt, "a non-Orchestrate role must be untouched");

        let out_none = annotate_orchestrate_refusal(receipt.clone(), None, false, "irrelevant");
        assert_eq!(out_none, receipt, "role: None must be untouched");
    }

    #[test]
    fn annotate_orchestrate_refusal_is_a_noop_when_resolved() {
        let receipt = serde_json::json!({"role": "orchestrate", "source": "derived"});
        let out = annotate_orchestrate_refusal(
            receipt.clone(),
            Some(WorkflowRole::Orchestrate),
            true,
            "irrelevant — nothing to explain when it resolved",
        );
        assert_eq!(out, receipt);
    }

    #[test]
    fn annotate_orchestrate_refusal_stamps_the_specific_reason_when_unresolved() {
        let receipt = serde_json::json!({
            "role": "orchestrate",
            "source": "session",
            "summary": "session model (role not configured and not derivable from the models available)",
        });
        let reason = "no model in the directory clears the ORCHESTRATE floor (0.80) and none is \
                       pinned or graduated — using the session coordinator";
        let out =
            annotate_orchestrate_refusal(receipt, Some(WorkflowRole::Orchestrate), false, reason);
        assert_eq!(out["refused"], true);
        assert_eq!(out["refusal_reason"], reason);
        let summary = out["summary"].as_str().unwrap();
        assert!(summary.contains("refused"), "{summary}");
        assert!(summary.contains(reason), "{summary}");
    }
}
