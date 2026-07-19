//! The objective recommender — given the models a user actually has, recommend
//! the best fit for each workflow role, purely from the measured
//! [`super::knowledge`] attributes and cost, with **zero vendor bias**.
//!
//! This is the heart of Jesse's rule: *no model is set by default; the agent
//! asks which models the user has access to and recommends which fit best where,
//! with no bias toward the vendor whose runtime this is — pure objectivity and
//! cost awareness.* Every pick here is a pure function of the data; the code has
//! no "prefer Anthropic" (or any vendor) branch, and the no-bias property test
//! ([`tests::non_anthropic_wins_edit_when_it_is_objectively_better`]) proves it.
//!
//! ## Roles (token-mass × judgment-density + cost)
//!
//! - **ORCHESTRATE** — the judgment-dense main loop; strongest available at
//!   orchestration / long-horizon (few tokens, high stakes).
//! - **EDIT** — the highest token-mass role; needs diff-format reliability.
//!   Highest `edit_format_reliability`, cheaper breaking ties; flagged if the
//!   best available is below the ~97% diff-format floor.
//! - **MECHANICAL** — search/read/summarize; cheapest capable, prefer local.
//! - **REVIEW** — verify; strong, and a *different family* than EDIT for
//!   diversity when the user has more than one.
//! - **LOCAL** — on-device ($0, private); the best local model, if any.
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
//! and [`recommend_configured`] touch config/registry, each a thin wrapper over
//! the pure core (mirroring [`super::cheap`] and [`super::budget`]).

use std::cmp::Ordering;

use serde::Serialize;

use super::knowledge::{lookup, ModelKnowledge};

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
            "orchestrate" | "hard" | "plan" | "planning" => Some(WorkflowRole::Orchestrate),
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
    /// Non-fatal flags: below the diff-format floor, non-caching cache-heavy
    /// role, single-family (no REVIEW diversity), no local model, …
    pub warnings: Vec<String>,
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
}

// ── Ranking comparators (best-first: `Less` == "a ranks before b") ───────────

fn id_order(a: &ModelKnowledge, b: &ModelKnowledge) -> Ordering {
    a.provider.cmp(b.provider).then(a.model.cmp(b.model))
}

/// EDIT: highest diff-format reliability, then lowest cost, then stable id.
fn edit_rank(a: &ModelKnowledge, b: &ModelKnowledge) -> Ordering {
    b.edit_format_reliability
        .total_cmp(&a.edit_format_reliability)
        .then(
            a.blended_cost_per_mtok()
                .total_cmp(&b.blended_cost_per_mtok()),
        )
        .then(id_order(a, b))
}

/// ORCHESTRATE: strongest orchestration, then lowest cost, then stable id.
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
/// order, picking e.g. `qwen3` over the stronger `qwen3-coder`.
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
/// then strongest orchestration, then lowest cost, then stable id.
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

fn base_rec(role: WorkflowRole, m: &ModelKnowledge, reason: String) -> RoleRecommendation {
    let mut rec = RoleRecommendation {
        role,
        provider: m.provider.to_string(),
        model: m.model.to_string(),
        display_name: m.display_name.to_string(),
        family: m.family.to_string(),
        blended_cost_per_mtok: m.blended_cost_per_mtok(),
        reason,
        warnings: Vec::new(),
    };
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

/// The objective recommendation for each role over the AVAILABLE models. Pure —
/// no config, no IO, no vendor branch. `available` is the knowledge rows for the
/// models the user has; empty in → empty out.
pub fn recommend(available: &[ModelKnowledge]) -> Vec<RoleRecommendation> {
    if available.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(5);

    // ORCHESTRATE — strongest orchestration. (`min_by` hands the comparator
    // `&&ModelKnowledge`, auto-deref-coerced to the rank fns' `&ModelKnowledge`.)
    let orch = available
        .iter()
        .min_by(|a, b| orchestrate_rank(a, b))
        .unwrap();
    out.push(base_rec(
        WorkflowRole::Orchestrate,
        orch,
        format!(
            "strongest orchestration among your models (agentic score {:.2}, ${:.2}/Mtok blended)",
            orch.orchestration_strength,
            orch.blended_cost_per_mtok()
        ),
    ));

    // EDIT — highest diff-format reliability, cheaper breaks ties.
    let edit = available.iter().min_by(|a, b| edit_rank(a, b)).unwrap();
    let mut edit_rec = base_rec(
        WorkflowRole::Edit,
        edit,
        format!(
            "highest edit-format reliability at lowest cost ({:.1}% diff-format, ${:.2}/Mtok blended)",
            edit.edit_format_reliability * 100.0,
            edit.blended_cost_per_mtok()
        ),
    );
    if edit.edit_format_reliability < EDIT_RELIABILITY_FLOOR {
        edit_rec.warnings.push(format!(
            "best available diff-format reliability is {:.1}%, below the ~{:.0}% floor — \
             expect more failed-edit retries; add a stronger editor if you can",
            edit.edit_format_reliability * 100.0,
            EDIT_RELIABILITY_FLOOR * 100.0
        ));
    }
    let edit_family = edit.family;
    out.push(edit_rec);

    // MECHANICAL — cheapest, prefer local.
    let mech = available
        .iter()
        .min_by(|a, b| mechanical_rank(a, b))
        .unwrap();
    out.push(base_rec(
        WorkflowRole::Mechanical,
        mech,
        if mech.is_local {
            format!(
                "cheapest capable tier — on-device {} at $0, no cloud spend for mechanical work",
                mech.display_name
            )
        } else {
            format!(
                "cheapest capable model among your configured providers (${:.2}/Mtok blended)",
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
    let review = available
        .iter()
        .min_by(|a, b| review_rank(a, b, edit_family, diversity_floor))
        .unwrap();
    let has_other_family = available.iter().any(|m| m.family != edit_family);
    let mut review_rec = base_rec(
        WorkflowRole::Review,
        review,
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
    match available
        .iter()
        .filter(|m| m.is_local)
        .min_by(|a, b| local_rank(a, b))
    {
        Some(local) => out.push(base_rec(
            WorkflowRole::Local,
            local,
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

/// Detect locally-available Ollama models. Best-effort and SYNCHRONOUS with no
/// network call: it surfaces the configured `GOOSE_MODEL` when the default
/// provider is Ollama. A live `/api/tags` probe is intentionally omitted here
/// because `discover_available_models` runs on the sync path of an async CLI
/// command, where a blocking HTTP client would panic inside the Tokio runtime —
/// this covers the "at least the configured GOOSE model when it's ollama" floor
/// the recommender needs; a richer probe can slot into the same seam later.
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

/// Discover the user's available models from the live configured-provider surface
/// — declarative providers + the builtin `providers/*.rs` providers (Anthropic /
/// OpenAI / Google / xAI, keyed; Ollama, local) — plus the
/// `GOOSE_PROVIDER`/`GOOSE_MODEL` default. Thin IO wrapper over the pure
/// [`discoverable_providers`] + [`available_from`].
pub fn discover_available_models() -> Vec<AvailableModel> {
    use crate::config::declarative_providers::all_declarative_provider_configs;

    let declarative: Vec<ProviderModels> = all_declarative_provider_configs()
        .into_iter()
        .map(|c| ProviderModels {
            provider: c.name,
            api_key_env: c.api_key_env,
            models: c.models.into_iter().map(|m| m.name).collect(),
        })
        .collect();

    let providers = discoverable_providers(declarative, discover_ollama_models());

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

/// The full objective recommendation over the user's configured models. IO
/// entry point for `permagent packs recommend` and the future Build-tab UI.
pub fn recommend_configured() -> Recommendation {
    let available = discover_available_models();
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
    Recommendation {
        recommendations: recommend(&known),
        considered,
        unknown_models,
    }
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
