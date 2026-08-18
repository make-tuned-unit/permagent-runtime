//! The persisted role→model dispatch map — the seam that turns the objective
//! recommendation ([`super::recommend`]) into ACTUAL routing: each workflow role
//! a delegated unit of work plays is routed to the model the USER configured for
//! it. This is the wiring follow-up the `permagent packs recommend` CLI promised.
//!
//! ## The load-bearing guarantee (Jesse's hard rule)
//!
//! There is **NO baked-in default here**. When a role has no configured mapping,
//! [`resolve_role_model`] returns `None`, and the dispatch path
//! ([`crate::agents::platform_extensions::summon`]'s `resolve_provider` and the
//! goal engine) falls through to the CURRENT single-model behaviour — the
//! parent/session model. The tiered-pack defaults in [`super::packs`]
//! (`ModelPacks::default()` — Opus/Sonnet/Haiku/Ollama) are the cost-router's
//! internal escalation ladder (and a named preset the user may explicitly apply);
//! they are **never** consulted by the dispatch path. Set nothing, stay
//! single-model — no vendor default applies.
//!
//! ## Role reconciliation (4 vs 5)
//!
//! Two role sets exist and serve different layers:
//!
//! - [`super::packs::Role`] (4 — Edit/Hard/Mechanical/Local) is the cost-router's
//!   TIER ladder (`packs::role_for_tier`), internal to the cheapest-path
//!   escalation. Left untouched.
//! - [`super::recommend::WorkflowRole`] (5 — Orchestrate/Edit/Mechanical/Review/
//!   Local) is the USER-facing set the recommender emits and this map persists.
//!   It is the CANONICAL role for wiring.
//!
//! They align as: `packs::Role::Hard` == `WorkflowRole::Orchestrate` (the
//! frontier-reasoning tier, under the user-facing name), plus `Review`
//! (cross-family verification) which the tier ladder has no rung for. Dispatch
//! routes by `WorkflowRole`; the tier ladder is unchanged.
//!
//! `Local` is intentionally NOT persona-derived (see [`derive_role`]): it is the
//! on-device tier the cost-router prefers for mechanical work, not a role a
//! delegate carries — a delegate's mechanical work routes via `Mechanical`, whose
//! configured model may itself be a local one.
//!
//! Pure core (`resolve_role_model`, `derive_role`) plus a thin IO wrapper over
//! `Config` — mirroring [`super::budget`] and [`super::cheap`].

use super::recommend::{RoleRecommendation, WorkflowRole};

// ── Config-key namespace (distinct from PERMAGENT_PACK_* — the tier packs) ────

fn key(role: WorkflowRole, field: &str) -> String {
    format!("PERMAGENT_ROLE_{}_{}", role.as_str().to_uppercase(), field)
}

/// The config key holding a role's provider id (e.g. `PERMAGENT_ROLE_EDIT_PROVIDER`).
pub fn provider_key(role: WorkflowRole) -> String {
    key(role, "PROVIDER")
}

/// The config key holding a role's model id (e.g. `PERMAGENT_ROLE_EDIT_MODEL`).
pub fn model_key(role: WorkflowRole) -> String {
    key(role, "MODEL")
}

/// A resolved role→model mapping entry: the concrete provider+model a role routes
/// to. Directly usable by `summon`'s provider/model resolution and by
/// `providers::create`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleModel {
    pub provider: String,
    pub model: String,
}

/// Pure: resolve one role's mapping from a key reader. `Some` only when BOTH the
/// provider AND model keys are present and non-empty — a half-configured role is
/// treated as unset, so dispatch falls through cleanly rather than routing to a
/// provider with no model (or vice versa). Split from [`role_model`] so the
/// precedence logic is unit-testable without the process-global config.
pub fn resolve_role_model(
    role: WorkflowRole,
    read: impl Fn(&str) -> Option<String>,
) -> Option<RoleModel> {
    let non_empty = |k: String| {
        read(&k)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let provider = non_empty(provider_key(role))?;
    let model = non_empty(model_key(role))?;
    Some(RoleModel { provider, model })
}

/// Pure: derive the workflow role a unit of work plays from a worker's declared
/// tool kinds and optional explicit tag (`WorkerPersona::workflow_role`).
///
/// Returns `None` when nothing indicates a role — the caller then leaves routing
/// to the single-model fallback. Precedence: an explicit tag wins; otherwise the
/// tool-kind signal, checked review → edit → orchestrate/hard → mechanical so the
/// more judgment-dense role wins when a worker carries several kinds.
///
/// `Local` is never derived here (see the module note): a delegate does not carry
/// the on-device tier as a role.
pub fn derive_role(
    tool_kinds: &[impl AsRef<str>],
    explicit_tag: Option<&str>,
) -> Option<WorkflowRole> {
    if let Some(tag) = explicit_tag {
        if let Some(role) = WorkflowRole::from_tag(tag) {
            return Some(role);
        }
    }
    let has = |needles: &[&str]| {
        tool_kinds.iter().any(|k| {
            let k = k.as_ref().to_ascii_lowercase();
            needles.iter().any(|n| k.contains(n))
        })
    };
    if has(&["review", "verify", "critique"]) {
        Some(WorkflowRole::Review)
    } else if has(&["code_edit"]) {
        Some(WorkflowRole::Edit)
    } else if has(&["orchestrate", "plan", "hard", "reason"]) {
        Some(WorkflowRole::Orchestrate)
    } else if has(&["memory_ops", "search", "read", "grep", "summar", "fetch"]) {
        Some(WorkflowRole::Mechanical)
    } else {
        None
    }
}

/// Pure: the `(role, provider, model)` triples `permagent packs apply` should
/// persist from an objective recommendation — every role the recommender gave a
/// concrete model. Filters out roles with no model (e.g. `Local` when the user has
/// no on-device model), which are advisory-only and must not write an empty
/// mapping. Split out so `apply` is testable without touching config.
pub fn mappings_to_persist(recs: &[RoleRecommendation]) -> Vec<(WorkflowRole, String, String)> {
    recs.iter()
        .filter(|r| !r.provider.is_empty() && !r.model.is_empty())
        .map(|r| (r.role, r.provider.clone(), r.model.clone()))
        .collect()
}

/// Pure: whether the live dispatch cache guard should warn — a cache-heavy role
/// (`is_cache_heavy`) that the role map ACTUALLY selected the provider for
/// (`role_applied`), routed to a provider WITHOUT prompt caching. Shared by both
/// dispatch sites (`summon::resolve_provider` and the goal engine) so the warning
/// fires on one tested rule. Mirrors the recommender's own cache-guard warning.
pub fn cache_guard_should_warn(
    role: WorkflowRole,
    role_applied: bool,
    provider_supports_cache: bool,
) -> bool {
    role_applied && role.is_cache_heavy() && !provider_supports_cache
}

// ── Thin IO wrappers over Config ─────────────────────────────────────────────

/// Live lookup of a role's configured mapping. Thin wrapper over
/// [`resolve_role_model`] against the global config.
pub fn role_model(role: WorkflowRole) -> Option<RoleModel> {
    let cfg = crate::config::Config::global();
    resolve_role_model(role, |k| cfg.get_param::<String>(k).ok())
}

/// Persist a role's provider+model to config (`permagent packs set` / `apply`).
pub fn set_role_model(
    role: WorkflowRole,
    provider: &str,
    model: &str,
) -> Result<(), crate::config::ConfigError> {
    let cfg = crate::config::Config::global();
    cfg.set_param(&provider_key(role), provider.to_string())?;
    cfg.set_param(&model_key(role), model.to_string())?;
    Ok(())
}

/// Remove a role's mapping (back to single-model for that role). `Config::delete`
/// is idempotent, so clearing an unset role is a no-op, not an error.
pub fn clear_role_model(role: WorkflowRole) -> Result<(), crate::config::ConfigError> {
    let cfg = crate::config::Config::global();
    cfg.delete(&provider_key(role))?;
    cfg.delete(&model_key(role))?;
    Ok(())
}

/// Every configured role→model mapping, in [`WorkflowRole::all`] order. Empty
/// when nothing is configured — the single-model-fallback state. Backs
/// `permagent packs show`.
pub fn configured() -> Vec<(WorkflowRole, RoleModel)> {
    WorkflowRole::all()
        .into_iter()
        .filter_map(|role| role_model(role).map(|rm| (role, rm)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A pure in-memory key reader for the resolution tests (no config/IO).
    fn reader(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    // ── The load-bearing guarantee: unset ⇒ None (single-model fallback) ─────

    #[test]
    fn unset_role_resolves_to_none_no_baked_default() {
        // Nothing configured for ANY role → every role resolves to None, so the
        // dispatch path falls through to the single (parent/session) model. This
        // is the anti-baked-default guarantee: the Opus/Sonnet/Haiku tier-pack
        // defaults are NEVER reached from here.
        let empty = reader(&[]);
        for role in WorkflowRole::all() {
            assert_eq!(
                resolve_role_model(role, &empty),
                None,
                "{role:?} must be unset when nothing is configured"
            );
        }
    }

    #[test]
    fn a_configured_role_resolves_to_its_provider_and_model() {
        let read = reader(&[
            ("PERMAGENT_ROLE_EDIT_PROVIDER", "openai"),
            ("PERMAGENT_ROLE_EDIT_MODEL", "gpt-5.6"),
        ]);
        assert_eq!(
            resolve_role_model(WorkflowRole::Edit, &read),
            Some(RoleModel {
                provider: "openai".into(),
                model: "gpt-5.6".into(),
            })
        );
        // A DIFFERENT role stays unset — mappings are per-role, no bleed.
        assert_eq!(resolve_role_model(WorkflowRole::Mechanical, &read), None);
    }

    #[test]
    fn half_configured_role_is_treated_as_unset() {
        // Provider set but no model → unset (don't route to a provider with no
        // model). Same for model-only.
        let provider_only = reader(&[("PERMAGENT_ROLE_MECHANICAL_PROVIDER", "ollama")]);
        assert_eq!(
            resolve_role_model(WorkflowRole::Mechanical, &provider_only),
            None
        );
        let model_only = reader(&[("PERMAGENT_ROLE_MECHANICAL_MODEL", "qwen3-coder")]);
        assert_eq!(
            resolve_role_model(WorkflowRole::Mechanical, &model_only),
            None
        );
    }

    #[test]
    fn blank_values_are_treated_as_unset() {
        let blank = reader(&[
            ("PERMAGENT_ROLE_REVIEW_PROVIDER", "   "),
            ("PERMAGENT_ROLE_REVIEW_MODEL", ""),
        ]);
        assert_eq!(resolve_role_model(WorkflowRole::Review, &blank), None);
    }

    #[test]
    fn each_role_has_a_distinct_key_namespace() {
        // Keys are per-role and distinct from the PERMAGENT_PACK_* tier packs.
        for role in WorkflowRole::all() {
            assert!(provider_key(role).starts_with("PERMAGENT_ROLE_"));
            assert!(provider_key(role).ends_with("_PROVIDER"));
            assert!(model_key(role).ends_with("_MODEL"));
            assert!(!provider_key(role).contains("PACK"));
        }
        assert_eq!(
            provider_key(WorkflowRole::Edit),
            "PERMAGENT_ROLE_EDIT_PROVIDER"
        );
        assert_eq!(
            model_key(WorkflowRole::Orchestrate),
            "PERMAGENT_ROLE_ORCHESTRATE_MODEL"
        );
    }

    // ── Role derivation from persona/tool_kinds ──────────────────────────────

    #[test]
    fn derive_code_edit_is_edit() {
        assert_eq!(
            derive_role(&["code_edit", "shell", "git"], None),
            Some(WorkflowRole::Edit)
        );
    }

    #[test]
    fn derive_read_only_is_mechanical() {
        assert_eq!(
            derive_role(&["memory_ops"], None),
            Some(WorkflowRole::Mechanical)
        );
        assert_eq!(
            derive_role(&["web_search"], None),
            Some(WorkflowRole::Mechanical)
        );
    }

    #[test]
    fn derive_review_wins_over_edit() {
        // A worker that both edits and reviews routes to REVIEW (the judgment-dense
        // role wins), so review work reaches the cross-family verifier model.
        assert_eq!(
            derive_role(&["code_edit", "review"], None),
            Some(WorkflowRole::Review)
        );
    }

    #[test]
    fn derive_orchestrate_from_planning_kinds() {
        assert_eq!(
            derive_role(&["orchestrate"], None),
            Some(WorkflowRole::Orchestrate)
        );
        assert_eq!(
            derive_role(&["planning"], None),
            Some(WorkflowRole::Orchestrate)
        );
    }

    #[test]
    fn derive_none_when_no_signal() {
        // Empty / unrecognized tool kinds and no tag → no role → single-model.
        let no_kinds: &[&str] = &[];
        assert_eq!(derive_role(no_kinds, None), None);
        assert_eq!(derive_role(&["telepathy"], None), None);
    }

    #[test]
    fn explicit_tag_overrides_tool_kind_derivation() {
        // A worker whose kinds say edit, but explicitly tagged mechanical, routes
        // mechanical. The tag is the operator's deliberate override.
        assert_eq!(
            derive_role(&["code_edit"], Some("mechanical")),
            Some(WorkflowRole::Mechanical)
        );
        // "hard" is an accepted alias for the orchestrate role.
        let no_kinds: &[&str] = &[];
        assert_eq!(
            derive_role(no_kinds, Some("hard")),
            Some(WorkflowRole::Orchestrate)
        );
        // An unknown tag is ignored, falling back to tool-kind derivation.
        assert_eq!(
            derive_role(&["code_edit"], Some("nonsense")),
            Some(WorkflowRole::Edit)
        );
    }

    // ── Persistence round-trips through a real Config (temp files, not global) ─

    #[test]
    fn set_keys_persist_and_read_back_through_config() {
        use crate::config::Config;
        let cfg_file = tempfile::NamedTempFile::new().unwrap();
        let sec_file = tempfile::NamedTempFile::new().unwrap();
        let config = Config::new_with_file_secrets(cfg_file.path(), sec_file.path()).unwrap();

        // Persist EDIT via the exact keys the map uses.
        config
            .set_param(&provider_key(WorkflowRole::Edit), "openai".to_string())
            .unwrap();
        config
            .set_param(&model_key(WorkflowRole::Edit), "gpt-5.6".to_string())
            .unwrap();

        // Read back through the pure resolver over this config.
        let read = |k: &str| config.get_param::<String>(k).ok();
        assert_eq!(
            resolve_role_model(WorkflowRole::Edit, read),
            Some(RoleModel {
                provider: "openai".into(),
                model: "gpt-5.6".into(),
            }),
        );
        // An unset role stays single-model.
        assert_eq!(resolve_role_model(WorkflowRole::Review, read), None);

        // Clearing one key half-configures the role → treated as unset.
        config.delete(&model_key(WorkflowRole::Edit)).unwrap();
        assert_eq!(resolve_role_model(WorkflowRole::Edit, read), None);
    }

    // ── apply: which mappings a recommendation persists ──────────────────────

    #[test]
    fn mappings_to_persist_skips_roles_with_no_model() {
        // RoleRecommendation is in scope via `use super::*`.
        let rec = |role, provider: &str, model: &str| RoleRecommendation {
            role,
            provider: provider.to_string(),
            model: model.to_string(),
            display_name: model.to_string(),
            family: provider.to_string(),
            blended_cost_per_mtok: 0.0,
            reason: String::new(),
            warnings: Vec::new(),
            floor_met: true,
        };
        let recs = vec![
            rec(WorkflowRole::Edit, "openai", "gpt-5.6"),
            rec(WorkflowRole::Mechanical, "ollama", "qwen3-coder"),
            // No local model configured → empty provider/model → not persisted.
            rec(WorkflowRole::Local, "", ""),
        ];
        let persisted = mappings_to_persist(&recs);
        assert_eq!(persisted.len(), 2, "the empty Local rec must be skipped");
        assert!(persisted
            .iter()
            .any(|(r, p, m)| *r == WorkflowRole::Edit && p == "openai" && m == "gpt-5.6"));
        assert!(!persisted.iter().any(|(r, _, _)| *r == WorkflowRole::Local));
    }

    // ── Cache guard (dispatch) decision ──────────────────────────────────────

    #[test]
    fn cache_guard_warns_only_for_applied_cache_heavy_roles_on_non_caching_providers() {
        // Cache-heavy roles (orchestrate/edit) on a non-caching provider, when the
        // role map selected it → warn.
        assert!(cache_guard_should_warn(
            WorkflowRole::Orchestrate,
            true,
            false
        ));
        assert!(cache_guard_should_warn(WorkflowRole::Edit, true, false));
        // A caching provider → no warning.
        assert!(!cache_guard_should_warn(WorkflowRole::Edit, true, true));
        // Not cache-heavy (mechanical/review/local) → never warns.
        assert!(!cache_guard_should_warn(
            WorkflowRole::Mechanical,
            true,
            false
        ));
        assert!(!cache_guard_should_warn(WorkflowRole::Review, true, false));
        // Role map did NOT select the provider (explicit override) → don't second-guess.
        assert!(!cache_guard_should_warn(WorkflowRole::Edit, false, false));
    }

    #[test]
    fn local_is_never_persona_derived() {
        // No tool-kind derivation yields Local — it's the on-device tier, not a
        // dispatch role. (An explicit `local` tag is the only way to name it.)
        for kinds in [
            vec!["code_edit"],
            vec!["memory_ops"],
            vec!["orchestrate"],
            vec!["review"],
        ] {
            assert_ne!(derive_role(&kinds, None), Some(WorkflowRole::Local));
        }
    }
}
