//! The DERIVED role→model map — the router's default when the user has not
//! hand-configured a role ([`super::role_map`]).
//!
//! ## The ruling (2026-08-18)
//!
//! *"The router should route to the model that is best for the job, whether
//! local or cloud, while being very cost conscious."* When no per-role model is
//! hand-configured, dispatch uses the recommender-derived best-fit map — for
//! each role the cheapest model the user ACTUALLY HAS (keyed cloud provider or
//! installed local model) that clears the role's capability floor
//! ([`WorkflowRole::capability_floor`]) — by DEFAULT. Not cheapest-first-then-
//! climb; best fit under the floor. A hand-configured role always wins
//! ([`super::role_map::resolve_role_model_or_derived`]).
//!
//! ## What can and cannot be in the map
//!
//! - Only models the recommender saw as AVAILABLE ([`AvailableModel`]) —
//!   configured/keyed providers plus the models a local Ollama actually reports
//!   pulled. Never a hardcoded vendor pack: `cheap::default_anchor()` and
//!   `packs::ModelPacks::default()` are not consulted here.
//! - The recommender scores by knowledge-base ROW; an installed Ollama tag such
//!   as `qwen3-coder:30b` resolves to the `qwen3-coder` row as a family estimate.
//!   The map is keyed back to the id the user actually has ([`realize`]) — it
//!   routes to `ollama/qwen3-coder:30b`, never to an untagged id that may not be
//!   pulled — and the provenance records the estimate.
//! - A role whose best available model does NOT clear its floor
//!   ([`RoleRecommendation::floor_met`] == `false`) is NOT derived: dispatch
//!   falls through to the session model, exactly as if nothing were configured.
//!   The router does not knowingly under-fit; the recommender's warning is the
//!   user's cue to add a stronger model or pin the role by hand.
//!
//! ## Provenance
//!
//! Every derived entry carries a [`Provenance`] — floor met, lookup confidence
//! (exact / alias / family estimate), and the blended cost hint — so the goal
//! card's routing receipt can say "derived (floor met, family estimate)" versus
//! "configured", and a reader can see WHY a model was picked.
//!
//! ## Guardrails left untouched
//!
//! Spend caps ([`super::budget`]), `MAX_ESCALATIONS_DEFAULT`,
//! `VERIFY_ESCALATE_AT`, marginal-cost worker ranking, and the interactive main
//! loop's single stable model are not consulted or changed here — the derived
//! map only answers "which model for THIS delegated role"; every other gate is
//! applied by its existing owner.
//!
//! ## Cache
//!
//! Derivation probes the local Ollama daemon (bounded, [`super::recommend::OLLAMA_PROBE_TIMEOUT`])
//! and reads config, so the live map is cached process-wide with a short TTL
//! ([`DERIVED_ROLE_MAP_TTL`]) and invalidated ([`invalidate_derived_role_map`])
//! from the config write paths that change what is available or configured
//! ([`config_key_affects_derived_map`]).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::knowledge::{lookup_with_confidence, LookupConfidence};
use super::recommend::{
    discover_available_models_async, recommend_from_available, AvailableModel, WorkflowRole,
};
use super::role_map::RoleModel;

/// Why a derived entry was chosen — carried into the routing receipt.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Provenance {
    /// Whether the model clears the role's capability floor. Always `true` for an
    /// entry in the map (an under-floor role is not derived) — kept explicit so
    /// the receipt states it rather than implies it.
    pub floor_met: bool,
    /// How the model resolved to its knowledge-base row: exact / alias (same
    /// weights) / family estimate (an Ollama-tagged variant scored by its family).
    pub confidence: LookupConfidence,
    /// Blended reference price (input + output per MTok; 0 for local).
    pub cost_hint: f64,
}

impl Provenance {
    /// Short human phrase for the receipt: `floor met, family estimate`.
    pub fn describe(&self) -> String {
        let floor = if self.floor_met {
            "floor met"
        } else {
            "below floor"
        };
        let conf = match self.confidence {
            LookupConfidence::Exact => "exact match",
            LookupConfidence::Alias => "alias match",
            LookupConfidence::FamilyEstimate => "family estimate",
        };
        format!("{floor}, {conf}")
    }
}

/// The derived best-fit map: role → the model the user actually has that best
/// fits it, plus per-role provenance. Empty when the user has no scorable model
/// (dispatch then stays on the session model everywhere).
#[derive(Debug, Clone)]
pub struct DerivedRoleMap {
    pub by_role: HashMap<WorkflowRole, RoleModel>,
    pub provenance: HashMap<WorkflowRole, Provenance>,
    pub derived_at: chrono::DateTime<chrono::Utc>,
}

impl DerivedRoleMap {
    /// The no-model map: every role falls through to the session model.
    pub fn empty() -> Self {
        Self {
            by_role: HashMap::new(),
            provenance: HashMap::new(),
            derived_at: chrono::Utc::now(),
        }
    }

    /// The derived model and provenance for `role`, if it was derived.
    pub fn get(&self, role: WorkflowRole) -> Option<(&RoleModel, &Provenance)> {
        let rm = self.by_role.get(&role)?;
        let prov = self.provenance.get(&role)?;
        Some((rm, prov))
    }

    pub fn is_empty(&self) -> bool {
        self.by_role.is_empty()
    }
}

/// Map a recommended knowledge-base row (`provider`/`model`) back to an id the
/// user ACTUALLY has. Prefers the exact id, then an alias (same weights), then
/// the first (sorted) family variant — so an installed `qwen3-coder:30b` is what
/// gets routed to, never a bare `qwen3-coder` that may not be pulled. `None`
/// when no available id resolves to that row (defensive: the recommender only
/// recommends from `available`, so this should not happen).
fn realize(
    available: &[AvailableModel],
    provider: &str,
    model: &str,
) -> Option<(String, LookupConfidence)> {
    if available
        .iter()
        .any(|a| a.provider == provider && a.model == model)
    {
        return Some((model.to_string(), LookupConfidence::Exact));
    }
    let rank = |c: LookupConfidence| match c {
        LookupConfidence::Exact => 0u8,
        LookupConfidence::Alias => 1,
        LookupConfidence::FamilyEstimate => 2,
    };
    available
        .iter()
        .filter(|a| a.provider.eq_ignore_ascii_case(provider))
        .filter_map(|a| {
            let (row, conf) = lookup_with_confidence(&a.provider, &a.model)?;
            (row.provider == provider && row.model == model).then(|| (a.model.clone(), conf))
        })
        .min_by(|(am, ac), (bm, bc)| rank(*ac).cmp(&rank(*bc)).then(am.cmp(bm)))
}

/// Pure: derive the best-fit map from the models the user has. Only roles whose
/// recommendation names a concrete model that clears the role's floor are
/// entered; each is keyed to an id actually in `available`.
///
/// `WorkflowRole::Orchestrate` is a special case (the 2026-09-09 "plan on the
/// strongest, work on the cheapest graduated" ruling, `capability_directory`
/// D2): it is NOT picked by [`recommend_from_available`]'s cheapest-that-
/// clears-the-floor family of ranks — it is picked by
/// [`super::capability_directory::CapabilityDirectory::planner`], which is
/// built from the SAME `available` models so the directory and this map can
/// never disagree. Every other role keeps the existing cheapest-that-clears
/// rule, with one addition: a role whose evidence shows a sub-[`super::
/// capability_directory::MICRO_SUITE_PASS_FLOOR`] micro-suite pass rate is
/// EXCLUDED from that role even though the KB heuristic would otherwise admit
/// it — evidence beats the heuristic in both directions.
pub fn derive_role_map(available: &[AvailableModel]) -> DerivedRoleMap {
    let directory =
        super::capability_directory::CapabilityDirectory::build_with_persisted(available, None);
    derive_role_map_from_directory(available, &directory)
}

/// The pure core of [`derive_role_map`], parameterized on an already-built
/// [`super::capability_directory::CapabilityDirectory`] — so a caller that HAS
/// persisted graduation evidence (the IO wrapper, [`derived_role_map`], via
/// [`super::capability_directory::CapabilityDirectory::build`]) can have that
/// evidence actually reach routing, while [`derive_role_map`] itself stays a
/// pure, no-IO, KB-only convenience for tests and any caller with no
/// evidence to merge. Both entry points share this ONE selection logic — the
/// directory and the routing can never disagree.
pub fn derive_role_map_from_directory(
    available: &[AvailableModel],
    directory: &super::capability_directory::CapabilityDirectory,
) -> DerivedRoleMap {
    let rec = recommend_from_available(available);
    let mut map = DerivedRoleMap::empty();

    if let Some((rm, _reason)) = directory.planner() {
        // `_reason` (Strongest / CostTieBreak / OnlyCandidate) is surfaced by
        // the roadmap-creation receipt in `orchestrator.rs`, not by the
        // generic per-goal `Provenance` this map carries for every role.
        if let Some((model, confidence)) = realize(available, &rm.provider, &rm.model) {
            let picked_card = directory
                .cards
                .iter()
                .find(|c| c.provider == rm.provider && c.model == rm.model);
            let cost_hint = picked_card
                .map(|c| c.blended_cost_per_mtok())
                .unwrap_or(0.0);
            // Ordinarily the planner only ever picks a floor-clearer, so this
            // is `true` in every case the current (no-evidence-pipeline) build
            // can produce; computed rather than hardcoded so a future
            // graduated-but-below-floor pick (real evidence beating the KB
            // heuristic) still reports its provenance honestly.
            let floor_met = picked_card
                .map(|c| {
                    c.strengths.orchestration
                        >= WorkflowRole::Orchestrate
                            .capability_floor()
                            .orchestration_min
                })
                .unwrap_or(true);
            map.by_role.insert(
                WorkflowRole::Orchestrate,
                RoleModel {
                    provider: rm.provider,
                    model,
                },
            );
            map.provenance.insert(
                WorkflowRole::Orchestrate,
                Provenance {
                    floor_met,
                    confidence,
                    cost_hint,
                },
            );
        }
    }

    for r in rec.recommendations {
        if r.role == WorkflowRole::Orchestrate {
            continue; // handled above by the planner
        }
        if r.provider.is_empty() || r.model.is_empty() || !r.floor_met {
            continue;
        }
        if let Some(card) = directory
            .cards
            .iter()
            .find(|c| c.provider == r.provider && c.model == r.model)
        {
            if super::capability_directory::evidence_excludes_role(card, r.role) {
                continue;
            }
        }
        let Some((model, confidence)) = realize(available, &r.provider, &r.model) else {
            continue;
        };
        map.by_role.insert(
            r.role,
            RoleModel {
                provider: r.provider.clone(),
                model,
            },
        );
        map.provenance.insert(
            r.role,
            Provenance {
                floor_met: r.floor_met,
                confidence,
                cost_hint: r.blended_cost_per_mtok,
            },
        );
    }
    map
}

// ── Process-wide cache ───────────────────────────────────────────────────────

/// How long a derived map is reused before re-discovery. Short enough that a
/// newly pulled Ollama model or a newly added key shows up without a restart;
/// long enough that a burst of dispatches costs one probe, not one each.
pub const DERIVED_ROLE_MAP_TTL: Duration = Duration::from_secs(300);

static CACHE: RwLock<Option<(Instant, Arc<DerivedRoleMap>)>> = RwLock::new(None);

fn cached() -> Option<Arc<DerivedRoleMap>> {
    let guard = CACHE.read().ok()?;
    let (at, map) = guard.as_ref()?;
    (at.elapsed() < DERIVED_ROLE_MAP_TTL).then(|| map.clone())
}

/// The live derived map — discovery (keyed providers + installed local models,
/// bounded Ollama probe) → the capability directory (KB seeded, merged with
/// any PERSISTED graduation evidence — [`super::capability_directory::
/// CapabilityDirectory::build`]) → [`derive_role_map_from_directory`] — cached
/// for [`DERIVED_ROLE_MAP_TTL`]. This is the ONLY entry point that sees
/// persisted evidence; [`derive_role_map`] stays pure/KB-only. Concurrent
/// cache misses may derive twice; harmless.
pub async fn derived_role_map() -> Arc<DerivedRoleMap> {
    if let Some(map) = cached() {
        return map;
    }
    let available = discover_available_models_async().await;
    let directory = super::capability_directory::CapabilityDirectory::build(&available);
    let map = Arc::new(derive_role_map_from_directory(&available, &directory));
    if let Ok(mut guard) = CACHE.write() {
        *guard = Some((Instant::now(), map.clone()));
    }
    map
}

/// Drop the cached map so the next dispatch re-derives. Called from the config
/// write paths that change availability or the hand-configured roles; the TTL
/// covers writes that bypass them (e.g. a key exported into the environment).
pub fn invalidate_derived_role_map() {
    if let Ok(mut guard) = CACHE.write() {
        *guard = None;
    }
}

/// Pure: whether a config write to `key` can change the derived map — a
/// hand-configured role (`PERMAGENT_ROLE_*`), the session provider/model, the
/// Ollama host, or any provider credential (`*_API_KEY` / `*_KEY`, and every
/// secret write, since provider keys are stored as secrets). Used by the daemon's
/// `/config/upsert` and `/config/remove` handlers to invalidate minimally.
pub fn config_key_affects_derived_map(key: &str, is_secret: bool) -> bool {
    if is_secret {
        return true;
    }
    let k = key.trim().to_ascii_uppercase();
    k.starts_with("PERMAGENT_ROLE_")
        || k == "GOOSE_PROVIDER"
        || k == "GOOSE_MODEL"
        || k == "OLLAMA_HOST"
        || k.ends_with("_API_KEY")
        || k.ends_with("_KEY")
        || k.ends_with("_TOKEN")
}

// ── Routing receipt ──────────────────────────────────────────────────────────

/// Where a role's model came from — recorded on the goal card's routing receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleSource {
    /// The user's hand-configured mapping (`PERMAGENT_ROLE_*`) — always wins.
    Configured,
    /// The recommender-derived best-fit map (this module).
    Derived,
    /// An explicit `PERMAGENT_PACK_*` pin, consulted only by a HEADLESS
    /// dispatch's `resolve_dispatch_role_model` (X-cost-1) — a daemon
    /// dispatch's `role_model_or_derived` never produces this variant, since
    /// it does not consult the pack pins at all.
    PackPin,
}

impl RoleSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            RoleSource::Configured => "configured",
            RoleSource::Derived => "derived",
            RoleSource::PackPin => "pack_pin",
        }
    }
}

/// Pure: the `model_routing` receipt for a dispatch — which role, which model,
/// and whether it was hand-configured, derived (with provenance), or fell
/// through to the session model. Rendered into the goal card's routing snapshot
/// so the card says how the model was chosen, not just which worker ran.
pub fn model_routing_receipt(
    role: Option<WorkflowRole>,
    resolved: Option<&(RoleModel, RoleSource)>,
    derived: &DerivedRoleMap,
) -> serde_json::Value {
    let Some(role) = role else {
        return serde_json::json!({
            "role": serde_json::Value::Null,
            "source": "session",
            "summary": "session model (no workflow role)",
        });
    };
    match resolved {
        Some((rm, RoleSource::Configured)) => serde_json::json!({
            "role": role.as_str(),
            "provider": rm.provider,
            "model": rm.model,
            "source": RoleSource::Configured.as_str(),
            "summary": "configured",
        }),
        Some((rm, RoleSource::Derived)) => {
            let prov = derived.provenance.get(&role);
            let summary = match prov {
                Some(p) => format!("derived ({})", p.describe()),
                None => "derived".to_string(),
            };
            serde_json::json!({
                "role": role.as_str(),
                "provider": rm.provider,
                "model": rm.model,
                "source": RoleSource::Derived.as_str(),
                "floor_met": prov.map(|p| p.floor_met),
                "confidence": prov.map(|p| p.confidence),
                "cost_hint_per_mtok": prov.map(|p| p.cost_hint),
                "derived_at": derived.derived_at.to_rfc3339(),
                "summary": summary,
            })
        }
        Some((rm, RoleSource::PackPin)) => serde_json::json!({
            "role": role.as_str(),
            "provider": rm.provider,
            "model": rm.model,
            "source": RoleSource::PackPin.as_str(),
            "summary": "pack pin (PERMAGENT_PACK_*, headless dispatch — X-cost-1)",
        }),
        None => serde_json::json!({
            "role": role.as_str(),
            "source": "session",
            "summary": "session model (role not configured and not derivable from the models available)",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cost_router::knowledge::KNOWN_MODELS;
    use crate::cost_router::role_map::{resolve_role_model_or_derived, RoleModel};

    fn am(provider: &str, model: &str) -> AvailableModel {
        AvailableModel::new(provider, model)
    }

    /// Every entry names a model the user actually has — provider AND id are
    /// drawn from `available`, never from the knowledge base at large and never
    /// from the tier-pack defaults.
    #[test]
    fn derived_map_only_contains_available_models() {
        let available = vec![
            am("openai", "gpt-5.6"),
            am("openai", "gpt-5.6-mini"),
            am("ollama", "qwen3-coder"),
        ];
        let map = derive_role_map(&available);
        assert!(!map.is_empty(), "three known models must derive something");
        for (role, rm) in &map.by_role {
            assert!(
                available.contains(&am(&rm.provider, &rm.model)),
                "{role:?} → {}/{} is not one of the user's available models",
                rm.provider,
                rm.model
            );
            let prov = map
                .provenance
                .get(role)
                .expect("every entry has provenance");
            assert!(
                prov.floor_met,
                "{role:?}: only floor-clearing picks are derived"
            );
        }
        // The pack default anchor is NOT reachable: it is a KB row, but the user
        // does not have it, so it cannot appear.
        let pack_default = crate::cost_router::packs::ModelPacks::default().hard;
        assert!(
            !map.by_role
                .values()
                .any(|rm| rm.provider == pack_default.provider && rm.model == pack_default.model),
            "the tier-pack default must not leak into a derived map the user cannot reach"
        );
    }

    /// No available (scorable) models ⇒ empty map ⇒ every role is `None` ⇒ the
    /// session model. This is the no-baked-default guarantee for the derived path.
    #[test]
    fn no_available_models_derives_nothing() {
        let map = derive_role_map(&[]);
        assert!(map.is_empty());
        for role in WorkflowRole::all() {
            assert!(map.get(role).is_none());
        }
        // Unknown-to-the-KB models are not scorable → likewise nothing.
        let map = derive_role_map(&[am("acme", "mystery-1")]);
        assert!(map.is_empty());
    }

    /// A role whose best available model is BELOW its floor is not derived — the
    /// router does not knowingly under-fit; the role falls through to the
    /// session model. Uses the weakest local row in the KB as the sole model.
    #[test]
    fn under_floor_role_is_not_derived() {
        // Pick the KB row with the lowest orchestration strength: it cannot clear
        // ORCHESTRATE's 0.80 floor.
        let weakest = KNOWN_MODELS
            .iter()
            .min_by(|a, b| {
                a.orchestration_strength
                    .total_cmp(&b.orchestration_strength)
            })
            .unwrap();
        assert!(
            !WorkflowRole::Orchestrate.capability_floor().clears(weakest),
            "test premise: the weakest KB row must not clear the ORCHESTRATE floor"
        );
        let map = derive_role_map(&[am(weakest.provider, weakest.model)]);
        assert!(
            map.get(WorkflowRole::Orchestrate).is_none(),
            "ORCHESTRATE must fall through when nothing clears its floor"
        );
        // A strong-but-under-the-EDIT-floor model (gpt-5.6: 95% diff-format,
        // floor 97%) derives ORCHESTRATE/MECHANICAL/REVIEW but NOT EDIT.
        let map = derive_role_map(&[am("openai", "gpt-5.6")]);
        assert!(map.get(WorkflowRole::Orchestrate).is_some());
        assert!(map.get(WorkflowRole::Mechanical).is_some());
        assert!(
            map.get(WorkflowRole::Edit).is_none(),
            "EDIT must fall through to the session model when the best available is under floor"
        );
    }

    /// An installed Ollama tag (`qwen3-coder:30b`) is scored by its family row
    /// but ROUTED by the id the user actually has, with the estimate recorded.
    #[test]
    fn ollama_tagged_variant_is_routed_by_its_installed_id_and_flagged_estimate() {
        let available = vec![am("ollama", "qwen3-coder:30b")];
        let map = derive_role_map(&available);
        let (rm, prov) = map
            .get(WorkflowRole::Local)
            .expect("a floor-clearing local model derives LOCAL");
        assert_eq!(rm.provider, "ollama");
        assert_eq!(
            rm.model, "qwen3-coder:30b",
            "route to the pulled tag, not the bare id"
        );
        assert_eq!(prov.confidence, LookupConfidence::FamilyEstimate);
        assert_eq!(prov.describe(), "floor met, family estimate");
        // The exact id, when available, is preferred over a family variant.
        let both = vec![am("ollama", "qwen3-coder"), am("ollama", "qwen3-coder:30b")];
        let map = derive_role_map(&both);
        let (rm, prov) = map.get(WorkflowRole::Local).unwrap();
        assert_eq!(rm.model, "qwen3-coder");
        assert_eq!(prov.confidence, LookupConfidence::Exact);
    }

    /// Configured wins; otherwise derived; otherwise None (session model).
    #[test]
    fn configured_wins_over_derived_and_derived_over_nothing() {
        let derived =
            derive_role_map(&[am("google", "gemini-3-pro"), am("openai", "gpt-5.6-mini")]);
        let derived_edit = derived.get(WorkflowRole::Edit).unwrap().0.clone();
        let read_configured = |k: &str| match k {
            "PERMAGENT_ROLE_EDIT_PROVIDER" => Some("anthropic".to_string()),
            "PERMAGENT_ROLE_EDIT_MODEL" => Some("my-pinned-model".to_string()),
            _ => None,
        };
        // Configured wins even though the derived map has an EDIT pick.
        assert_eq!(
            resolve_role_model_or_derived(WorkflowRole::Edit, read_configured, &derived),
            Some((
                RoleModel {
                    provider: "anthropic".into(),
                    model: "my-pinned-model".into()
                },
                RoleSource::Configured
            ))
        );
        // Unconfigured → derived.
        let read_none = |_: &str| None;
        assert_eq!(
            resolve_role_model_or_derived(WorkflowRole::Edit, read_none, &derived),
            Some((derived_edit, RoleSource::Derived))
        );
        // Unconfigured and not derivable → None → session model.
        assert_eq!(
            resolve_role_model_or_derived(WorkflowRole::Edit, read_none, &DerivedRoleMap::empty()),
            None
        );
    }

    #[test]
    fn routing_receipt_says_how_the_model_was_chosen() {
        let derived = derive_role_map(&[am("ollama", "qwen3-coder:30b")]);
        let (rm, _) = derived.get(WorkflowRole::Local).unwrap();
        let v = model_routing_receipt(
            Some(WorkflowRole::Local),
            Some(&(rm.clone(), RoleSource::Derived)),
            &derived,
        );
        assert_eq!(v["source"], "derived");
        assert_eq!(v["model"], "qwen3-coder:30b");
        assert_eq!(v["summary"], "derived (floor met, family estimate)");
        assert_eq!(v["floor_met"], true);
        assert_eq!(v["confidence"], "family_estimate");

        let pinned = RoleModel {
            provider: "openai".into(),
            model: "gpt-5.6".into(),
        };
        let v = model_routing_receipt(
            Some(WorkflowRole::Edit),
            Some(&(pinned, RoleSource::Configured)),
            &derived,
        );
        assert_eq!(v["source"], "configured");
        assert_eq!(v["summary"], "configured");

        let v = model_routing_receipt(Some(WorkflowRole::Edit), None, &DerivedRoleMap::empty());
        assert_eq!(v["source"], "session");
        let v = model_routing_receipt(None, None, &DerivedRoleMap::empty());
        assert_eq!(v["source"], "session");
        assert!(v["role"].is_null());
    }

    #[test]
    fn invalidation_keys_cover_roles_session_model_host_and_credentials() {
        assert!(config_key_affects_derived_map(
            "PERMAGENT_ROLE_EDIT_MODEL",
            false
        ));
        assert!(config_key_affects_derived_map("GOOSE_PROVIDER", false));
        assert!(config_key_affects_derived_map("goose_model", false));
        assert!(config_key_affects_derived_map("OLLAMA_HOST", false));
        assert!(config_key_affects_derived_map("OPENAI_API_KEY", false));
        assert!(config_key_affects_derived_map("anything", true));
        assert!(!config_key_affects_derived_map("wizard_complete", false));
        assert!(!config_key_affects_derived_map("GOOSE_MODE", false));
    }

    #[test]
    fn cache_invalidation_drops_the_map() {
        // Seed the cache directly (no discovery), then invalidate.
        if let Ok(mut g) = CACHE.write() {
            *g = Some((Instant::now(), Arc::new(DerivedRoleMap::empty())));
        }
        assert!(cached().is_some());
        invalidate_derived_role_map();
        assert!(cached().is_none());
    }

    /// The interactive main loop's model path never consults the derived map:
    /// only the three delegated-work seams (goal dispatch, escalation, summon)
    /// reference it. Grep-based over the sources so a future edit that reaches
    /// for the map from the main-loop provider path fails here.
    #[test]
    fn main_loop_model_path_never_consults_the_derived_map() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace = root.parent().unwrap().parent().unwrap();
        let needles = [
            "derived_role_map",
            "resolve_role_model_or_derived",
            "role_model_or_derived",
        ];
        let main_loop_paths = [
            root.join("src/agents/agent.rs"),
            workspace.join("crates/goose-cli/src/session/builder.rs"),
            workspace.join("crates/goose-server/src/routes/agent.rs"),
        ];
        for p in &main_loop_paths {
            let src = std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            for n in needles {
                assert!(
                    !src.contains(n),
                    "{} must not consult the derived role map (found `{n}`) — the main loop \
                     stays on its single model",
                    p.display()
                );
            }
        }
        // Positive control: the delegated-work seams DO reach the map, so the
        // assertion above is not vacuous. Each is named with the symbol that
        // carries it there — `summon` consults the map through
        // `cost_router::delegate`, which gates the pick on the operator's pins and
        // the escalation knob rather than taking it raw (see that module), so the
        // literal moved one level down and the control follows it.
        for (p, needle) in [
            (
                root.join("src/agents/platform_extensions/orchestrator.rs"),
                "derived_role_map",
            ),
            (
                root.join("src/agents/platform_extensions/summon.rs"),
                "delegate_routing_live",
            ),
            (root.join("src/cost_router/delegate.rs"), "derived_role_map"),
        ] {
            let src = std::fs::read_to_string(&p).unwrap();
            assert!(
                src.contains(needle),
                "{} is a delegated-work seam and must route through the derived map \
                 (expected `{needle}`)",
                p.display()
            );
        }
    }

    // ── D2: workers keep the existing cheapest-clearing-floor rule ──────────

    /// Only ORCHESTRATE changed (planner rule). Every other role still picks
    /// the cheapest model that clears ITS floor, exactly as before — a
    /// stronger, dearer model must NOT win MECHANICAL just because the
    /// directory now exists.
    #[test]
    fn workers_stay_cheapest_clearing_floor() {
        let available = vec![
            am("anthropic", "claude-opus-4-8"), // strongest, dearest
            am("ollama", "qwen3-coder"),        // $0, clears the mechanical floor
        ];
        let map = derive_role_map(&available);
        let (mech, _) = map
            .get(WorkflowRole::Mechanical)
            .expect("mechanical must derive something");
        assert_eq!(
            mech.provider, "ollama",
            "MECHANICAL must still be the cheapest floor-clearer, not the strongest model"
        );
        // ORCHESTRATE, meanwhile, is the planner's pick (strongest above the
        // floor) — qwen3-coder (0.55) never clears the 0.80 floor, so Opus
        // wins there, proving the two roles now use genuinely different rules.
        let (orch, _) = map
            .get(WorkflowRole::Orchestrate)
            .expect("orchestrate must derive something");
        assert_eq!(orch.provider, "anthropic");
        assert_eq!(orch.model, "claude-opus-4-8");
    }

    /// D2's evidence-excludes-a-role rule, wired end to end: a role whose
    /// persisted evidence shows a sub-80% micro-suite pass rate is excluded
    /// from the derived map even though the KB heuristic alone would admit
    /// it. `derive_role_map` itself stays pure (no persisted-file IO), so
    /// this drives it through `CapabilityDirectory::build_with_persisted`
    /// directly to prove the filter is really wired, not just unit-tested in
    /// isolation.
    #[test]
    fn evidence_below_80pct_excludes_role_end_to_end() {
        use super::super::capability_directory::{CapabilityDirectory, RoleEvidence};
        // gpt-5.6 clears the MECHANICAL floor on the KB heuristic alone.
        let available = vec![am("openai", "gpt-5.6")];
        let baseline = derive_role_map(&available);
        assert!(
            baseline.get(WorkflowRole::Mechanical).is_some(),
            "test premise: gpt-5.6 clears the mechanical floor without evidence"
        );

        // Build a directory carrying evidence that gpt-5.6 FAILS the
        // mechanical micro-suite (< 80%), then drive it through the SAME
        // selection core `derived_role_map()` (the IO wrapper) uses —
        // `derive_role_map_from_directory` — to prove the exclusion is really
        // wired into routing, not just true of the pure filter function.
        let today = chrono::Utc::now().date_naive();
        let mut seeded = CapabilityDirectory::build_with_persisted(&available, None);
        for card in seeded.cards.iter_mut() {
            if card.provider == "openai" && card.model == "gpt-5.6" {
                card.roles.insert(
                    WorkflowRole::Mechanical.as_str().to_string(),
                    RoleEvidence::new(
                        false,
                        true,
                        Some(0.40),
                        Some(0.90),
                        vec![],
                        None,
                        None,
                        None,
                        today,
                    )
                    .unwrap(),
                );
            }
        }
        let with_evidence = derive_role_map_from_directory(&available, &seeded);
        assert!(
            with_evidence.get(WorkflowRole::Mechanical).is_none(),
            "sub-80% mechanical evidence must EXCLUDE the role from the derived map, \
             even though the KB heuristic alone would have admitted it"
        );
    }
}
