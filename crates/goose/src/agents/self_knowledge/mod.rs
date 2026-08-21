//! Self-knowledge — the `permagent_self` brief.
//!
//! Phase 1 of #353. Gives the agent an authoritative, always-fresh description
//! of its own capabilities, assembled each turn and injected into the system
//! prompt. The goal is to close the "core gap": the agent under-reports or
//! mis-states what the Permagent runtime can actually do.
//!
//! ## The descriptor contract
//!
//! Every user-facing capability is described by a [`FeatureDescriptor`]. There
//! are three categories ([`FeatureCategory`]):
//!
//! - **Tool** — an agent-callable platform extension. Descriptors are *derived*
//!   from [`PlatformExtensionDef`] via [`PlatformExtensionDef::descriptor`]; the
//!   new descriptor fields are non-`Option`, so a missing one is a **compile
//!   error** at every registry `map.insert(...)`. That is the robustness
//!   guarantee for tools.
//! - **Worker** — a background loop (Scheduler, Librarian). Descriptors are
//!   co-located with the worker module and aggregated in [`WORKER_DESCRIPTORS`].
//! - **Surface** — a user-facing view (Reader, World View). Co-located and
//!   aggregated in [`SURFACE_DESCRIPTORS`].
//!
//! For workers/surfaces there is no central struct to enforce coverage, so a
//! completeness test ([`tests`]) asserts every known id has a descriptor.
//!
//! ## Live state vs editorial
//!
//! [`StateSource::Queryable`] features merge real runtime state into the brief
//! (e.g. scheduler job count, librarian phase). [`StateSource::Static`] features
//! render editorial-only — we describe what they are without claiming a live
//! status we cannot cheaply observe. This avoids over-claiming.
//!
//! ## Onboarding (agent-led teaching)
//!
//! Two submodules turn this inventory into an active teaching loop:
//! [`usage`] tracks which capabilities the user has actually engaged (fed by the
//! existing activity bus), and [`teachable`] is the curated set of features the
//! agent can walk the user through — each mapped to a real navigable surface.
//! `inventory − used` is the "learn next" list.

pub mod teachable;
pub mod usage;

use std::fmt::Write as _;

use crate::agents::platform_extensions::{
    librarian_state, PlatformExtensionDef, PLATFORM_EXTENSIONS,
};

/// What kind of capability a [`FeatureDescriptor`] describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureCategory {
    /// An agent-callable tool (platform extension).
    Tool,
    /// A background worker loop that runs on its own.
    Worker,
    /// A user-facing surface / view.
    Surface,
    /// A deterministic guardrail the agent is *subject to* (not one it calls).
    Guard,
}

/// Whether a feature's runtime state can be cheaply queried for the brief.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateSource {
    /// Live state is merged into the brief each turn.
    Queryable,
    /// Always-on; rendered editorially with no live status claim.
    Static,
}

/// A static reference to an app surface the agent can open during a lesson,
/// via the existing `navigate_app` tool (NOT a new nav wire). Static-friendly —
/// `&'static str` only, no `serde_json::Value` (it must live in a `const`). A
/// deep-link `state` (e.g. a project id) is intentionally omitted in v1; add a
/// `state_json: Option<&'static str>` parsed at runtime if a lesson ever needs
/// one.
#[derive(Debug, Clone, Copy)]
pub struct SurfaceRef {
    /// app_catalog tab name, e.g. "Brain", "Automate", "Settings".
    pub tab: &'static str,
    /// Optional sub-section within the tab, e.g. Some("identity").
    pub section: Option<&'static str>,
}

/// A read-back check that confirms the user actually did a lesson step. Every
/// variant maps to a **queryable** predicate — the agent verifies it by reading
/// the live state already rendered in the `permagent_self` brief (job count,
/// librarian progress, persona name, tool `[active]` flags) or, for
/// [`Self::MemoryRecallable`], via the `search_memory` tool. This is the Phase-1
/// `StateSource` guard in action: a confirm step is only expressible through a
/// queryable signal, so a `Static` surface (Reader) confirms *by proxy*
/// (`MemoryRecallable`), never by claiming live status it cannot observe.
#[derive(Debug, Clone, Copy)]
pub enum ConfirmCheck {
    /// A platform extension is enabled — shows `[active]` in the brief's Tools list.
    ExtensionEnabled(&'static str),
    /// At least one scheduled job exists — the brief's Scheduler line goes `0 → 1`.
    HasScheduledJob,
    /// The Librarian has described ≥1 memory — visible in the brief's Librarian line.
    LibrarianDescribedAtLeastOne,
    /// A memory matching this phrase is recallable — verify with `search_memory`.
    /// The proxy confirm for Static surfaces that write to the Brain (Reader).
    MemoryRecallable(&'static str),
    /// The persona has been personalized (a name other than the default, or a
    /// voice set) — the brief's "You are <name>" line reflects it next turn.
    PersonaConfigured,
}

/// A single teaching step — the Phase-2 lesson unit hung off a
/// [`FeatureDescriptor`]. `&[]` for features without a lesson yet.
#[derive(Debug, Clone, Copy)]
pub struct TeachingStep {
    /// Short step label.
    pub title: &'static str,
    /// What the agent says/does this step (persona-neutral — no name literal).
    pub body: &'static str,
    /// An app surface to open this step, via `navigate_app`. `None` = no surface
    /// (e.g. a drag-and-drop demo).
    pub open_surface: Option<SurfaceRef>,
    /// An optional read-back to confirm the user acted. `None` = no confirmation.
    pub confirm: Option<ConfirmCheck>,
}

/// Authoritative description of one capability. The unit the `permagent_self`
/// brief is assembled from.
#[derive(Debug, Clone, Copy)]
pub struct FeatureDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub category: FeatureCategory,
    /// Plain-language statement of what the feature does.
    pub what_it_does: &'static str,
    /// Why the agent should care — when/why to reach for it.
    pub why_it_matters: &'static str,
    pub state_source: StateSource,
    /// Phase-2 hook. `&[]` in Phase 1.
    pub teaching: &'static [TeachingStep],
}

// ── Worker / surface registries (co-located descriptors, aggregated here) ──

/// Background workers. Each entry is a `const` co-located with its module.
pub static WORKER_DESCRIPTORS: &[FeatureDescriptor] = &[
    crate::scheduler::SELF_KNOWLEDGE_FEATURE,
    crate::agents::platform_extensions::librarian::SELF_KNOWLEDGE_FEATURE,
    crate::steward::SELF_KNOWLEDGE_FEATURE,
    crate::initiative::SELF_KNOWLEDGE_FEATURE,
    crate::echo::SELF_KNOWLEDGE_FEATURE,
    usage::ONBOARDING_COACH_FEATURE,
    crate::growth::GROWTH_MEASUREMENT_FEATURE,
    // Render-gated (see `worker_descriptor_visible`): hidden from the brief
    // while `playbook_enabled` is off (Settings → Features), so this experimental, unproven
    // capability does not enter every user's Henry until deliberately enabled.
    crate::playbook::PLAYBOOK_SYNTHESIS_FEATURE,
    // Render-gated on `concierge_enabled` (Settings → Features, default OFF): the Concierge
    // inbox-triage character (#640) is hidden from the brief until deliberately
    // enabled, so the canonical prompt snapshots stay byte-for-byte identical.
    crate::concierge::SELF_KNOWLEDGE_FEATURE,
    // Render-gated on `strix_enabled` (default OFF): a security agent that
    // runs live scan tooling is switched on deliberately, and until then the
    // brief stays byte-for-byte identical.
    crate::strix::SELF_KNOWLEDGE_FEATURE,
    crate::agents::platform_extensions::finance::SELF_KNOWLEDGE_FEATURE,
];

/// The Git Steward's worker-descriptor id. The descriptor itself spells the id
/// as a literal (`steward::SELF_KNOWLEDGE_FEATURE`), so there is no const to
/// import; `gate_ids_match_the_descriptors_that_own_them` pins this one against
/// it, because a renamed descriptor id would otherwise turn its gate into a
/// silent no-op rather than a compile error.
pub const GIT_STEWARD_FEATURE_ID: &str = "git_steward";

/// The Initiative driver's worker-descriptor id. Same reason as
/// [`GIT_STEWARD_FEATURE_ID`]: the descriptor names it as a literal.
pub const INITIATIVE_FEATURE_ID: &str = "initiative";

/// The config key that switches the Steward's git-health sweep on.
///
/// The loop that actually reads it lives in the daemon crate
/// (`crates/goose-server/src/steward_sweep.rs`), which this crate cannot depend
/// on, so the key is named HERE — where the gate table needs it — and pinned by
/// `steward_gate_key_is_the_key_the_descriptor_names` against the Steward's own
/// descriptor prose. That is the strongest in-crate pin available; a rename in
/// the daemon crate alone would still drift.
pub const STEWARD_SCAN_ENABLED_KEY: &str = "steward_scan_enabled";

/// The single boolean config key that switches one worker on.
///
/// Two consumers read this table and they deliberately differ:
///
/// * The `permagent_self` brief renders only what the agent can actually DO, so
///   a gated-off worker is ABSENT from it — which is also what keeps the
///   canonical prompt snapshots byte-for-byte identical.
/// * Settings → Agents must LIST a gated-off worker, because the switch lives on
///   its page. Hiding the thing the user came to switch on is a dead end, and it
///   is the one that sent a product owner hunting through five panes.
///
/// `hides_from_brief` is that difference, and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerGate {
    pub key: &'static str,
    pub hides_from_brief: bool,
}

impl WorkerGate {
    /// Whether this gate is currently on, read from an already-loaded
    /// [`FeatureFlags`] rather than from live config, so a renderer stays a pure
    /// function of its inputs.
    ///
    /// The `_ => false` arm exists only because the key is a string; a gate key
    /// that was never wired to a `FeatureFlags` field would read as permanently
    /// off, which is why `every_gate_key_resolves_from_feature_flags` asserts
    /// every gate reads TRUE under all-flags-on.
    pub fn is_on(&self, flags: FeatureFlags) -> bool {
        match self.key {
            "playbook_enabled" => flags.playbook_enabled,
            "concierge_enabled" => flags.concierge_enabled,
            "strix_enabled" => flags.strix_enabled,
            "initiative_enabled" => flags.initiative_enabled,
            STEWARD_SCAN_ENABLED_KEY => flags.steward_scan_enabled,
            _ => false,
        }
    }
}

/// The gate table — the one source of truth for "which key switches this worker
/// on". `None` means the worker has no boolean switch at all, which a surface
/// must not confuse with a switch that is off.
pub fn worker_gate(descriptor_id: &str) -> Option<WorkerGate> {
    let gate = |key, hides_from_brief| {
        Some(WorkerGate {
            key,
            hides_from_brief,
        })
    };
    match descriptor_id {
        // Experimental or cost-bearing workers stay out of the brief until they
        // are deliberately enabled, so the prompt is byte-identical to one built
        // before the descriptor existed.
        id if id == crate::playbook::PLAYBOOK_FEATURE_ID => gate("playbook_enabled", true),
        id if id == crate::concierge::CONCIERGE_FEATURE_ID => gate("concierge_enabled", true),
        id if id == crate::strix::STRIX_FEATURE_ID => gate("strix_enabled", true),
        // These two are ALWAYS described: their descriptors report the real
        // on/off switch as a state label, which is honest without hiding them.
        INITIATIVE_FEATURE_ID => gate("initiative_enabled", false),
        GIT_STEWARD_FEATURE_ID => gate(STEWARD_SCAN_ENABLED_KEY, false),
        _ => None,
    }
}

/// Whether a worker descriptor should be rendered into the `permagent_self`
/// brief. Almost all are always visible; a flag-gated, experimental worker is
/// hidden until its flag is on, so the capability the agent can DESCRIBE is
/// exactly the one it can DO — and, with the flag off, the brief is byte-for-
/// byte identical to before the descriptor existed (the canonical snapshots
/// stay unchanged; a dedicated test covers the enabled rendering).
///
/// DERIVED from [`worker_gate`] so the brief and the Agents surface cannot
/// disagree about which key gates which worker. This is a brief-only predicate:
/// Settings → Agents deliberately does NOT filter on it, because the switch
/// lives on the page a hidden worker would be missing from.
pub fn worker_descriptor_visible(d: &FeatureDescriptor, flags: FeatureFlags) -> bool {
    match worker_gate(d.id) {
        Some(gate) if gate.hides_from_brief => gate.is_on(flags),
        _ => true,
    }
}

/// Live state line for a worker descriptor, or `None` when the worker
/// is `Static` or exposes no queryable signal. Shared by the
/// `permagent_self` brief and the Agents API so the two can never
/// disagree about what a worker is doing.
pub fn worker_live_state_for(
    d: &FeatureDescriptor,
    scheduled_job_count: Option<usize>,
    flags: FeatureFlags,
) -> Option<String> {
    if d.state_source != StateSource::Queryable {
        return None;
    }
    match d.id {
        "scheduler" => scheduled_job_count.map(|n| format!("{n} job(s) scheduled")),
        id if id == librarian_state_id() => {
            let s = librarian_state::get_state();
            // The "awaiting your context" clause (#387 v2 ask-seam) renders
            // only when non-zero: the zero state stays byte-identical to the
            // pre-v2 brief (snapshot-stable), and the agent is only prompted
            // to ask when there is actually something to ask about.
            Some(if s.entities_awaiting_context > 0 {
                format!(
                    "{} described, {} pending, {} awaiting your context",
                    s.lifetime_stats.described,
                    s.lifetime_stats.pending,
                    s.entities_awaiting_context
                )
            } else {
                format!(
                    "{} described, {} pending",
                    s.lifetime_stats.described, s.lifetime_stats.pending
                )
            })
        }
        "initiative" => Some(if flags.initiative_enabled {
            "on — watching for repeated commands".to_string()
        } else {
            "off (initiative_enabled=false)".to_string()
        }),
        "strix" => Some(if flags.strix_enabled {
            "on — security sweeps every 24h; a missed Docker/strix preflight is a skip, not a clean scan".to_string()
        } else {
            "off (strix_enabled=false)".to_string()
        }),
        _ => None,
    }
}

/// Deterministic guardrails the agent operates under. Co-located with the
/// safety-core module that enforces each one.
pub static GUARD_DESCRIPTORS: &[FeatureDescriptor] = &[
    crate::steward::secret_scan::SELF_KNOWLEDGE_FEATURE,
    crate::session::crash_capture::DURABILITY_FEATURE,
    crate::tool_monitor::SELF_KNOWLEDGE_FEATURE,
    crate::agents::schema_validation::SELF_KNOWLEDGE_FEATURE,
    crate::sovereignty::SELF_KNOWLEDGE_FEATURE,
    crate::agents::platform_extensions::goal_engine::GOAL_LANDING_FEATURE,
];

/// User-facing surfaces. Each entry is a `const` co-located with its module.
pub static SURFACE_DESCRIPTORS: &[FeatureDescriptor] = &[
    crate::reader::SELF_KNOWLEDGE_FEATURE,
    crate::events::WORLD_VIEW_FEATURE,
    crate::brain_handle::BRAIN_FEATURE,
    crate::config::agent_identity::PERSONA_PICKER_FEATURE,
    crate::config::agent_identity::VOICE_FEATURE,
    crate::config::agent_identity::WEB_SEARCH_FEATURE,
    crate::agents::platform_extensions::project_manager::BUILD_TAB_FEATURE,
    crate::agents::platform_extensions::project_manager::PROJECT_WORKSPACE_FEATURE,
    crate::echo::PROJECT_INSIGHTS_FEATURE,
    crate::agents::platform_extensions::project_manager::DEVICES_FEATURE,
    crate::agents::platform_extensions::project_manager::IOS_COMPANION_FEATURE,
    crate::gateway::TELEGRAM_GATEWAY_FEATURE,
    crate::decision_inbox::DECISION_INBOX_FEATURE,
    crate::inbox::INBOX_FEATURE,
    crate::activity_journal::TIMELINE_FEATURE,
    crate::scheduler::RUN_ROSTER_FEATURE,
    crate::agents::platform_extensions::project_manager::GROW_FEATURE,
    crate::agents::platform_extensions::finance::FINANCE_TAB_FEATURE,
    crate::agents::platform_extensions::project_manager::FIRST_PARTY_ANALYTICS_FEATURE,
    crate::agents::platform_extensions::analyze::CODEBASE_INDEX_FEATURE,
    crate::agents::platform_extensions::project_manager::CODING_HARNESS_FEATURE,
    crate::cost_router::COST_OPTIMIZER_FEATURE,
    crate::mesh::MESH_FEATURE,
    crate::skills::SKILLS_FEATURE,
    crate::session::SESSIONS_FEATURE,
    crate::events::TRACE_FEATURE,
    crate::dictation::MEETING_DICTATION_FEATURE,
    crate::sovereignty::GOVERNANCE_SURFACE_FEATURE,
    crate::agents::platform_extensions::app_perception::SELF_KNOWLEDGE_FEATURE,
    crate::config::agent_identity::AGENTS_SURFACE_FEATURE,
];

/// Tool ids that are described under another category and therefore skipped in
/// the Tools section (the librarian is a platform extension *and* a background
/// worker — we describe it once, as a worker, to avoid double-listing).
const TOOL_IDS_RENDERED_ELSEWHERE: &[&str] = &[librarian_state_id()];

const fn librarian_state_id() -> &'static str {
    crate::agents::platform_extensions::librarian::EXTENSION_NAME
}

// ── Builder ────────────────────────────────────────────────────────────

/// The flags that gate what the brief renders. Read once by the caller and
/// passed in, so the rendered brief is a pure function of its inputs: a test can
/// render either state without the process-global config deciding for it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FeatureFlags {
    pub playbook_enabled: bool,
    pub concierge_enabled: bool,
    pub strix_enabled: bool,
    pub initiative_enabled: bool,
    /// The Steward's git-health sweep. Read here so the gate table can answer
    /// for every worker, and DELIBERATELY not rendered anywhere in the brief:
    /// the Steward descriptor already states the flag in its prose, and a new
    /// live-state line would move the line counts the canonical snapshot tests
    /// pin.
    pub steward_scan_enabled: bool,
}

impl FeatureFlags {
    /// The one place the renderer's flags are read from live process state.
    pub fn from_live_config() -> Self {
        Self {
            playbook_enabled: crate::playbook::is_enabled(),
            concierge_enabled: crate::concierge::is_enabled(),
            strix_enabled: crate::strix::is_enabled(),
            initiative_enabled: crate::initiative::driver::is_enabled(),
            // The loop that acts on this lives in the daemon crate
            // (`steward_sweep.rs`), which cannot be imported here, so the value
            // is read straight from config rather than through an `is_enabled`
            // helper like its four siblings.
            steward_scan_enabled: crate::config::Config::global()
                .get_param::<bool>(STEWARD_SCAN_ENABLED_KEY)
                .unwrap_or(false),
        }
    }
}

/// Assembles the `permagent_self` brief. Live state is fetched at the call site
/// and passed in, keeping rendering deterministic for the supplied inputs.
pub struct SelfKnowledgeBuilder {
    /// The agent's display name (persona-resolved; default "Aria"). Never
    /// hardcoded — interpolated from the resolved persona.
    pub agent_display_name: String,
    /// Live scheduled-job count (Queryable). `None` when the scheduler is not
    /// wired (e.g. tests) → rendered editorially.
    pub scheduled_job_count: Option<usize>,
    /// Feature gates that determine which workers and live states are rendered.
    pub flags: FeatureFlags,
    /// Workers the orchestrator can dispatch goals to, with live status.
    /// Pre-computed by the (async) caller so this builder stays pure and
    /// snapshot-stable. Empty → the section is omitted (e.g. tests, or when
    /// orchestration is not active).
    pub dispatchable_workers: Vec<DispatchableWorker>,
    /// Unread reports from the worker agents (see [`crate::briefings`]).
    /// Pre-computed by the async caller.
    ///
    /// `None` means the briefings could not be read at all (no pool — tests, a
    /// degraded boot). `Some(vec![])` means they WERE read and nothing is
    /// pending. The distinction is load-bearing: with a plain `Vec`, a failed
    /// read is indistinguishable from a clean slate, and Henry would tell the
    /// user "nothing to report" on the strength of a query that never ran.
    pub agent_briefings: Option<Vec<BriefingLine>>,
}

/// One unread briefing, flattened for rendering. Deliberately a display-only
/// struct rather than `briefings::Briefing` — the builder must stay pure, and
/// the brief has no business carrying ids, timestamps or ref links into every
/// prompt. Henry gets the headline; the detail is a query away.
#[derive(Debug, Clone)]
pub struct BriefingLine {
    /// Reporting agent's display name ("Steward"), not its roster key.
    pub from: String,
    /// Rendered severity ("action required", "attention", "info").
    pub severity: String,
    pub summary: String,
}

/// A worker the orchestrator can dispatch a goal to, plus its live status,
/// for the `<permagent_self>` brief. Populated dynamically — see
/// [`dispatchable_workers_from_config`].
#[derive(Debug, Clone)]
pub struct DispatchableWorker {
    pub display_name: String,
    /// Human-readable status: "available", "unavailable: <reason>", or
    /// "engine pending".
    pub status: String,
}

/// Build the dispatchable-worker list from the live registry + availability
/// probe. `Pending`-engine workers are reported as "engine pending" without
/// probing; runnable workers report their live probe result. Deterministically
/// sorted by display name. The probe may block (`model_loaded:` does HTTP) —
/// call from a blocking context (e.g. `spawn_blocking`).
pub fn dispatchable_workers_from_config(
    config: &crate::config::agent_identity::AgentConfig,
) -> Vec<DispatchableWorker> {
    use crate::config::agent_identity::WorkerEngineKind;
    let mut workers: Vec<DispatchableWorker> = config
        .workers
        .values()
        .map(|w| {
            let status = match &w.engine {
                WorkerEngineKind::Pending => "engine pending".to_string(),
                _ => {
                    let (ok, reason) =
                        crate::config::worker_probe::probe_worker(&w.availability_check);
                    if ok {
                        "available".to_string()
                    } else {
                        format!(
                            "unavailable: {}",
                            reason.unwrap_or_else(|| "not available".to_string())
                        )
                    }
                }
            };
            DispatchableWorker {
                display_name: w.display_name(),
                status,
            }
        })
        .collect();
    workers.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    workers
}

/// Whether the orchestrator's goal-dispatch capability is active, using the
/// same predicate the tools list uses for the orchestrator entry. The
/// dispatchable-worker brief line is gated on this so Henry only claims to
/// dispatch when the orchestrator tool is active.
pub fn orchestrator_dispatch_active() -> bool {
    PLATFORM_EXTENSIONS
        .get("orchestrator")
        .map(|d| crate::config::extensions::is_extension_enabled(d.name) || d.default_enabled)
        .unwrap_or(false)
}

impl SelfKnowledgeBuilder {
    /// The whole brief as one string — the stable inventory followed by the live
    /// status block. Kept for callers with no cache breakpoint to place; the
    /// prompt path uses [`build_parts`](Self::build_parts).
    pub fn build(&self) -> String {
        let (stable, volatile) = self.build_parts();
        format!("{stable}{volatile}")
    }

    /// Build the brief split into its byte-stable half and its turn-volatile
    /// half.
    ///
    /// The **stable** half is the capability inventory: who the agent is, the
    /// tools/workers/surfaces/guardrails it has. It changes only when the user
    /// changes something (renames the persona, enables an extension) — and then
    /// it *should* bust the cache, because the agent's description of itself
    /// genuinely changed.
    ///
    /// The **volatile** half is everything that moves on its own: unread
    /// briefings (Info-severity ones are acknowledged the moment they are
    /// rendered, so turn N+1 differs from turn N *by construction*), live worker
    /// counters, and worker availability probes that do live HTTP. None of it is
    /// dropped — dropping it would blind the agent to what its workers just
    /// reported — it just rides after the cache breakpoint, where changing it
    /// costs its own tokens instead of the whole prompt.
    pub fn build_parts(&self) -> (String, String) {
        let mut out = String::new();
        let name = &self.agent_display_name;

        writeln!(out, "# Who You Are — Your Capabilities").ok();
        writeln!(out).ok();
        writeln!(
            out,
            "You are {name}. The following is an authoritative, live inventory of \
             what you (the Permagent runtime) can actually do. Trust it over any \
             assumption about your own abilities."
        )
        .ok();

        // ── Tools ──
        writeln!(out, "\n## Tools you can call").ok();
        let mut tools: Vec<&PlatformExtensionDef> = PLATFORM_EXTENSIONS
            .values()
            .filter(|d| !d.hidden && !TOOL_IDS_RENDERED_ELSEWHERE.contains(&d.name))
            .collect();
        tools.sort_by(|a, b| a.name.cmp(b.name));
        for def in tools {
            let desc = def.descriptor();
            // Queryable live state: enabled if explicitly enabled in config, or
            // on by default and not explicitly disabled.
            let active =
                crate::config::extensions::is_extension_enabled(def.name) || def.default_enabled;
            let state = if active {
                "active"
            } else {
                "available — enable to use"
            };
            writeln!(
                out,
                "- **{}** [{}] — {}. {}",
                desc.display_name, state, desc.what_it_does, desc.why_it_matters
            )
            .ok();
        }

        // ── Workers ──
        //
        // The bullet is the worker's *description*, which does not move. Its
        // live counters ("3 described, 7 pending") do move — every idle sweep
        // rewrites them — so they render in the volatile block below instead of
        // inline here, where they would have invalidated the whole cached prompt
        // on a background job the user never touched.
        writeln!(
            out,
            "\n## Background workers (run on their own, even when you are idle)"
        )
        .ok();
        for d in WORKER_DESCRIPTORS {
            // Flag-gated workers are omitted from the brief while disabled — off
            // is a byte-for-byte no-op (see `worker_descriptor_visible`).
            if !worker_descriptor_visible(d, self.flags) {
                continue;
            }
            writeln!(
                out,
                "- **{}** — {}. {}",
                d.display_name, d.what_it_does, d.why_it_matters
            )
            .ok();
        }

        // ── Surfaces ──
        writeln!(out, "\n## Surfaces the user can see").ok();
        for d in SURFACE_DESCRIPTORS {
            // Static surfaces render editorial-only — no live status claim.
            writeln!(
                out,
                "- **{}** — {}. {}",
                d.display_name, d.what_it_does, d.why_it_matters
            )
            .ok();
        }

        // ── Guardrails ──
        // Deterministic constraints the agent is subject to. Surfacing them is a
        // correctness requirement: a guard the agent can't describe is a bug.
        writeln!(
            out,
            "\n## Guardrails you operate under (deterministic — not your judgment)"
        )
        .ok();
        for d in GUARD_DESCRIPTORS {
            writeln!(
                out,
                "- **{}** — {}. {}",
                d.display_name, d.what_it_does, d.why_it_matters
            )
            .ok();
        }

        (out, self.live_status_block())
    }

    /// The turn-volatile half of the brief.
    ///
    /// The rule is deliberately blunt — **anything `worker_live_state` reports
    /// goes here**, plus briefings and dispatch probes — rather than a per-field
    /// judgement about which counters "really" move. A carve-out list is exactly
    /// the thing a later contributor extends without noticing they have put a
    /// moving value back inside the cached block. The cost of the blunt rule is
    /// ~60 uncached tokens a turn against a system prompt in the thousands.
    ///
    /// Empty string when there is nothing live at all, so the section never
    /// renders as an empty heading.
    fn live_status_block(&self) -> String {
        let mut worker_states: Vec<(&'static str, String)> = Vec::new();
        for d in WORKER_DESCRIPTORS {
            // Same gate as the inventory half: a worker hidden from its
            // description must not reappear here as a live-status line.
            if !worker_descriptor_visible(d, self.flags) {
                continue;
            }
            if let Some(state) = self.worker_live_state(d) {
                worker_states.push((d.display_name, state));
            }
        }

        let has_briefings = self.agent_briefings.is_some();
        if !has_briefings && self.dispatchable_workers.is_empty() && worker_states.is_empty() {
            return String::new();
        }

        let mut out = String::new();
        writeln!(out, "\n# Live Status (this turn)").ok();
        writeln!(
            out,
            "\nThis section is re-read every turn and changes on its own. Everything \
             above it is fixed for the session."
        )
        .ok();

        // ── Briefings from the worker agents ──
        // Placed FIRST among the live sections on purpose: this is the only
        // part of the brief that can be waiting on you. The agents report to
        // you; you answer to the user.
        //
        // `None` (could not read) omits the section entirely rather than
        // rendering an empty one — claiming "nothing to report" off a query
        // that never ran is worse than saying nothing.
        if let Some(briefings) = &self.agent_briefings {
            writeln!(out, "\n## Briefings from your agents").ok();
            writeln!(
                out,
                "Your worker agents — the Steward, the Watcher — report their work \
                 to you. They post to boards and panels for the user, but they \
                 also brief you directly, so you can raise what matters unprompted \
                 instead of waiting to be asked. Seeing a briefing is not approving \
                 it: destructive work still waits on a human."
            )
            .ok();
            if briefings.is_empty() {
                writeln!(out, "\nNothing unread right now.").ok();
            } else {
                writeln!(out, "\nUnread:").ok();
                for b in briefings {
                    writeln!(out, "- **{}** [{}] — {}", b.from, b.severity, b.summary).ok();
                }
            }
        }

        // ── Workers you can dispatch goals to (dynamic; orchestrator) ──
        // Volatile because each entry's status is a live availability probe —
        // `model_loaded:` does HTTP, and a model that unloads flips the line.
        if !self.dispatchable_workers.is_empty() {
            writeln!(out, "\n## Workers you can dispatch goals to").ok();
            for w in &self.dispatchable_workers {
                writeln!(out, "- **{}** — {}", w.display_name, w.status).ok();
            }
        }

        if !worker_states.is_empty() {
            writeln!(out, "\n## Background worker status").ok();
            for (display_name, state) in worker_states {
                writeln!(out, "- **{display_name}** — {state}").ok();
            }
        }

        out
    }

    /// Live state for a Queryable worker, or `None` for Static / unavailable.
    fn worker_live_state(&self, d: &FeatureDescriptor) -> Option<String> {
        worker_live_state_for(d, self.scheduled_job_count, self.flags)
    }
}

// ── Lessons (Phase 2) ───────────────────────────────────────────────────

/// Config flag: set once the user has engaged with (or declined) the guided
/// tour. Mirrors `onboarding_memories_seeded` idempotency — a plain config
/// bool, no DB schema/migration. Gates the one-time first-run tour offer.
pub const TOUR_COMPLETED_KEY: &str = "tour_completed";

/// The Phase-2-v1 lesson set, in tour order.
pub const V1_TOUR_LESSONS: &[&str] = &["reader", "brain", "scheduler", "persona"];

/// Find a feature's descriptor by id across tools, workers, and surfaces.
pub fn find_descriptor(id: &str) -> Option<FeatureDescriptor> {
    if let Some(d) = WORKER_DESCRIPTORS.iter().find(|d| d.id == id) {
        return Some(*d);
    }
    if let Some(d) = SURFACE_DESCRIPTORS.iter().find(|d| d.id == id) {
        return Some(*d);
    }
    if let Some(d) = GUARD_DESCRIPTORS.iter().find(|d| d.id == id) {
        return Some(*d);
    }
    PLATFORM_EXTENSIONS
        .values()
        .find(|def| def.name == id)
        .map(|def| def.descriptor())
}

/// Render a feature's teaching steps into agent-facing lesson text. Returns
/// `None` for an unknown feature, or a "no lesson yet" note for a known feature
/// whose `teaching` is still empty. Used by the `load_feature_lesson` tool so
/// the per-turn prompt stays lean — lessons are fetched only when teaching.
pub fn lesson_for(id: &str) -> Option<String> {
    let d = find_descriptor(id)?;
    let mut out = String::new();
    writeln!(out, "# Lesson: {}", d.display_name).ok();
    writeln!(out, "\n{}. {}\n", d.what_it_does, d.why_it_matters).ok();

    if d.teaching.is_empty() {
        writeln!(
            out,
            "(No step-by-step lesson authored yet — explain it in your own words \
             using the description above.)"
        )
        .ok();
        return Some(out);
    }

    for (i, step) in d.teaching.iter().enumerate() {
        writeln!(out, "**Step {} — {}**", i + 1, step.title).ok();
        writeln!(out, "{}", step.body).ok();
        if let Some(s) = step.open_surface {
            match s.section {
                Some(sec) => writeln!(
                    out,
                    "→ Open it for them: call `navigate_app` with tab \"{}\", section \"{}\".",
                    s.tab, sec
                ),
                None => writeln!(
                    out,
                    "→ Open it for them: call `navigate_app` with tab \"{}\".",
                    s.tab
                ),
            }
            .ok();
        }
        if let Some(c) = step.confirm {
            writeln!(out, "✓ Confirm before moving on: {}", confirm_hint(&c)).ok();
        }
        writeln!(out).ok();
    }
    Some(out)
}

/// Human-facing guidance for verifying a [`ConfirmCheck`]. The read-back reuses
/// the live state already in the brief (no new endpoint), except
/// `MemoryRecallable` which uses the `search_memory` tool.
fn confirm_hint(c: &ConfirmCheck) -> String {
    match c {
        ConfirmCheck::ExtensionEnabled(id) => {
            format!("check your capabilities brief — the \"{id}\" tool should now read [active].")
        }
        ConfirmCheck::HasScheduledJob => "re-read your capabilities brief — the Scheduler line \
             should now show 1 (or more) job(s) scheduled, up from 0."
            .to_string(),
        ConfirmCheck::LibrarianDescribedAtLeastOne => {
            "re-read your capabilities brief — the Librarian line should show at least one \
             memory described."
                .to_string()
        }
        ConfirmCheck::MemoryRecallable(phrase) => format!(
            "call `search_memory` for \"{phrase}\" — it should now return a result, proving \
             the content was ingested into the Brain."
        ),
        ConfirmCheck::PersonaConfigured => "re-read your capabilities brief — the opening \
             \"You are …\" line should now show the name they chose, not the default."
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All five gates, on. Written once so a test asserting the ON half cannot
    /// silently stop covering a newly added gate.
    fn all_flags_on() -> FeatureFlags {
        FeatureFlags {
            playbook_enabled: true,
            concierge_enabled: true,
            strix_enabled: true,
            initiative_enabled: true,
            steward_scan_enabled: true,
        }
    }

    /// The LITERAL set of workers the brief withholds while their flag is off.
    ///
    /// Named, not re-derived: `worker_descriptor_visible` IS
    /// `worker_gate(..).hides_from_brief`, so asserting one against the other
    /// compares the function to a copy of itself and passes whatever the table
    /// says. Spelling the three ids out is what makes flipping a
    /// `hides_from_brief` — which silently adds or removes a paragraph from
    /// every brief the agent renders — a test failure instead of a diff nobody
    /// reads. The Steward and Initiative are deliberately absent: they are
    /// always described, with their switch reported as a state label.
    #[test]
    fn brief_withholds_exactly_the_experimental_workers() {
        let mut hidden: Vec<&str> = WORKER_DESCRIPTORS
            .iter()
            .filter(|d| !worker_descriptor_visible(d, FeatureFlags::default()))
            .map(|d| d.id)
            .collect();
        hidden.sort_unstable();
        assert_eq!(hidden, vec!["concierge", "playbook", "strix"]);

        // With every flag on nothing is withheld — which is also what makes the
        // `_ => false` arm of `is_on` safe to have.
        for d in WORKER_DESCRIPTORS {
            assert!(
                worker_descriptor_visible(d, all_flags_on()),
                "{} stayed hidden with every flag on",
                d.id
            );
        }
    }

    /// `WorkerGate::is_on` matches on a STRING key, so an unwired key would
    /// compile and read as permanently off. Asserting the all-on half is what
    /// makes the `_ => false` arm safe: a gate whose key reaches no
    /// `FeatureFlags` field fails here instead of quietly never switching on.
    #[test]
    fn every_gate_key_resolves_from_feature_flags() {
        let gates: Vec<WorkerGate> = WORKER_DESCRIPTORS
            .iter()
            .filter_map(|d| worker_gate(d.id))
            .collect();
        assert_eq!(
            gates.len(),
            5,
            "expected five gated workers, found {gates:?}"
        );
        for gate in gates {
            assert!(
                gate.is_on(all_flags_on()),
                "{} is not wired to any FeatureFlags field",
                gate.key
            );
            assert!(
                !gate.is_on(FeatureFlags::default()),
                "{} reads as on with every flag off",
                gate.key
            );
        }
    }

    /// The Steward's and Initiative's descriptor ids are literals in their own
    /// modules, so the gate table repeats them. A rename there would turn the
    /// gate into a silent no-op — the switch would write a key and the roster
    /// would show no gate — rather than failing to compile.
    #[test]
    fn gate_ids_match_the_descriptors_that_own_them() {
        assert_eq!(
            GIT_STEWARD_FEATURE_ID,
            crate::steward::SELF_KNOWLEDGE_FEATURE.id
        );
        assert_eq!(
            INITIATIVE_FEATURE_ID,
            crate::initiative::SELF_KNOWLEDGE_FEATURE.id
        );
        for id in [
            GIT_STEWARD_FEATURE_ID,
            INITIATIVE_FEATURE_ID,
            crate::strix::STRIX_FEATURE_ID,
            crate::playbook::PLAYBOOK_FEATURE_ID,
            crate::concierge::CONCIERGE_FEATURE_ID,
        ] {
            assert!(worker_gate(id).is_some(), "{id} lost its gate");
            assert!(
                WORKER_DESCRIPTORS.iter().any(|d| d.id == id),
                "{id} names no worker descriptor"
            );
        }
    }

    /// The only in-crate pin against the daemon-side literal at
    /// `crates/goose-server/src/steward_sweep.rs`, which this crate cannot
    /// import. The Steward descriptor's own prose names the key so the agent can
    /// tell the user how to switch the sweep on; if the table and the prose
    /// disagree, one of them is lying to the user.
    #[test]
    fn steward_gate_key_is_the_key_the_descriptor_names() {
        assert_eq!(
            worker_gate(GIT_STEWARD_FEATURE_ID).unwrap().key,
            STEWARD_SCAN_ENABLED_KEY
        );
        assert!(crate::steward::SELF_KNOWLEDGE_FEATURE
            .what_it_does
            .contains(STEWARD_SCAN_ENABLED_KEY));
    }

    /// The gate table IS the set of switches Settings → Features renders. When a
    /// worker gains or loses a gate here,
    /// `ui/command-center/src/components/settings/features/features.ts` must
    /// move with it, or the pane will be missing a switch the daemon honours —
    /// which is exactly how the Guard came to have no toggle outside the Models
    /// pane.
    #[test]
    fn gate_set_is_exactly_the_features_pane_set() {
        let mut keys: Vec<&str> = WORKER_DESCRIPTORS
            .iter()
            .filter_map(|d| worker_gate(d.id).map(|g| g.key))
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "concierge_enabled",
                "initiative_enabled",
                "playbook_enabled",
                "steward_scan_enabled",
                "strix_enabled",
            ]
        );
    }

    /// Rerunnable raw evidence for descriptor audits. Kept ignored because it
    /// prints the full registries and MCP tool copy rather than asserting.
    #[test]
    #[ignore = "audit dump, not an assertion"]
    fn descriptor_audit_dump() {
        fn one_line(text: &str) -> String {
            text.replace(['\r', '\n'], " ")
        }

        fn print_descriptor(registry: &str, d: FeatureDescriptor) {
            println!(
                "{}|{}|{}|{:?}|{}|{}",
                registry,
                d.id,
                one_line(d.display_name),
                d.category,
                one_line(d.what_it_does),
                one_line(d.why_it_matters)
            );
        }

        let mut platform: Vec<&PlatformExtensionDef> = PLATFORM_EXTENSIONS.values().collect();
        platform.sort_by_key(|def| def.name);
        for def in platform {
            let registry = if def.hidden {
                "platform_extension(hidden)"
            } else {
                "platform_extension"
            };
            print_descriptor(registry, def.descriptor());
        }
        for d in WORKER_DESCRIPTORS {
            print_descriptor("worker", *d);
        }
        for d in GUARD_DESCRIPTORS {
            print_descriptor("guard", *d);
        }
        for d in SURFACE_DESCRIPTORS {
            print_descriptor("surface", *d);
        }

        for (ext_name, tools) in extension_tool_inventories() {
            for tool in tools {
                println!(
                    "mcp_tool|{}|{}|{}",
                    ext_name,
                    tool.name,
                    one_line(tool.description.as_deref().unwrap_or_default())
                );
            }
        }
    }

    /// Every known worker id must have exactly one descriptor. Catches a worker
    /// added without a co-located descriptor wired into [`WORKER_DESCRIPTORS`].
    const KNOWN_WORKER_IDS: &[&str] = &[
        "scheduler",
        "librarian",
        "git_steward",
        "initiative",
        "watcher",
        "onboarding_coach",
        "growth_measurement",
        // In the registry always (so `find_descriptor` resolves it); its render
        // into the brief is flag-gated (see `worker_descriptor_visible`).
        "playbook",
        // Same render-gated contract as the playbook (`concierge_enabled`).
        "concierge",
        // Same render-gated contract, on the `strix_enabled` config key.
        "strix",
        "financier",
    ];
    /// Every known surface id must have exactly one descriptor.
    const KNOWN_SURFACE_IDS: &[&str] = &[
        "reader",
        "world_view",
        "brain",
        "persona",
        "voice",
        "web_search",
        "build",
        "projects",
        "project_insights",
        "devices",
        "ios_companion",
        "telegram_gateway",
        "decision_inbox",
        "downloads_inbox",
        "timeline",
        "run_roster",
        "grow",
        "finance_tab",
        "first_party_analytics",
        "codebase",
        "coding_harness",
        "cost_optimizer",
        "mesh",
        "skills_library",
        "sessions",
        "trace",
        "meeting_dictation",
        "governance",
        "app_awareness",
        "agents",
    ];
    /// The Phase-2-v1 lesson set — each must resolve to a descriptor with steps.
    const V1_LESSON_IDS: &[&str] = &["reader", "brain", "scheduler", "persona"];

    /// No descriptor id may occur twice, either inside one registry or across
    /// registries. Historical DEFECT A let the Downloads inbox surface shadow
    /// the Decision Inbox extension at id `inbox`; DEFECT B likewise let the
    /// Skills Library surface shadow the Skills extension at id `skills`.
    #[test]
    fn descriptor_ids_are_unique_across_registries() {
        // Librarian deliberately has both a platform extension and a worker
        // descriptor. The platform copy is omitted from the brief by
        // TOOL_IDS_RENDERED_ELSEWHERE, so the worker is the one rendered.
        const ALLOWLIST: &[(&str, &[&str])] = &[("librarian", &["platform", "worker"])];

        let mut by_id: std::collections::BTreeMap<&'static str, Vec<&'static str>> =
            std::collections::BTreeMap::new();
        {
            let mut add = |registry: &'static str, id: &'static str| {
                let registries = by_id.entry(id).or_default();
                assert!(
                    !registries.contains(&registry),
                    "descriptor id {id:?} appears twice in the {registry} registry; \
                     find_descriptor resolves worker > surface > guard > platform, so a collision \
                     silently shadows a descriptor and serves the wrong lesson"
                );
                registries.push(registry);
            };

            for def in PLATFORM_EXTENSIONS.values() {
                add("platform", def.name);
            }
            for d in WORKER_DESCRIPTORS {
                add("worker", d.id);
            }
            for d in GUARD_DESCRIPTORS {
                add("guard", d.id);
            }
            for d in SURFACE_DESCRIPTORS {
                add("surface", d.id);
            }
        }

        for (id, registries) in by_id {
            if registries.len() == 1 {
                continue;
            }
            let allowlisted = ALLOWLIST.iter().any(|(allowed_id, allowed_registries)| {
                id == *allowed_id && registries.as_slice() == *allowed_registries
            });
            assert!(
                allowlisted,
                "descriptor id {id:?} occurs across registries {registries:?}; \
                 find_descriptor resolves worker > surface > guard > platform, so a collision \
                 silently shadows a descriptor and serves the wrong lesson"
            );
        }
    }

    /// Every descriptor rendered in the brief needs a distinct bold label.
    #[test]
    fn descriptor_display_names_are_unique_in_the_brief() {
        // These two bullets are intentionally complementary: the extension
        // exposes tools that settle decisions from chat, while the surface is
        // the dashboard queue where those decisions land.
        type RegistryEntry = (&'static str, &'static str);
        type DisplayNameAllowance = (&'static str, RegistryEntry, RegistryEntry);
        const ALLOWLIST: &[DisplayNameAllowance] = &[(
            "Decision Inbox",
            ("platform", "inbox"),
            ("surface", "decision_inbox"),
        )];

        let mut by_name: std::collections::BTreeMap<
            &'static str,
            Vec<(&'static str, &'static str)>,
        > = std::collections::BTreeMap::new();
        {
            let mut add = |registry: &'static str, id: &'static str, display_name: &'static str| {
                by_name
                    .entry(display_name)
                    .or_default()
                    .push((registry, id));
            };

            for def in PLATFORM_EXTENSIONS
                .values()
                .filter(|def| !def.hidden && !TOOL_IDS_RENDERED_ELSEWHERE.contains(&def.name))
            {
                add("platform", def.name, def.display_name);
            }
            for d in WORKER_DESCRIPTORS
                .iter()
                .filter(|d| worker_descriptor_visible(d, FeatureFlags::default()))
            {
                add("worker", d.id, d.display_name);
            }
            for d in GUARD_DESCRIPTORS {
                add("guard", d.id, d.display_name);
            }
            for d in SURFACE_DESCRIPTORS {
                add("surface", d.id, d.display_name);
            }
        }

        for (display_name, entries) in by_name {
            if entries.len() == 1 {
                continue;
            }
            let allowlisted = ALLOWLIST.iter().any(|(allowed_name, left, right)| {
                display_name == *allowed_name
                    && entries.len() == 2
                    && entries.contains(left)
                    && entries.contains(right)
            });
            assert!(
                allowlisted,
                "brief display name {display_name:?} is shared by {entries:?}; the brief renders \
                 each descriptor as `- **{{display_name}}** — ...`, so duplicate bold labels hand \
                 the agent two definitions of one name"
            );
        }
    }

    /// Paired extension and non-tool descriptors must retain the same defining
    /// claim even though each copy serves a different audience.
    #[test]
    fn paired_descriptors_do_not_contradict() {
        // The Decision Inbox surface intentionally does not name callable tools;
        // "decisions" is the honest shared claim instead of an invented tool
        // reference. The other pairs share their concrete implementation handle.
        const PAIRS: &[(&str, &str, &str, &str)] = &[
            (
                "inbox",
                "decision_inbox",
                "decisions",
                "both copies must identify decisions as the queue's subject",
            ),
            (
                "app_perception",
                "app_awareness",
                "observe_app",
                "both copies must name the read-only app-perception tool",
            ),
            (
                "skills",
                "skills_library",
                "SKILL.md folder",
                "both copies must preserve the portable on-disk skills format",
            ),
            (
                "analyze",
                "codebase",
                "tree-sitter",
                "both copies must preserve how the code map is produced",
            ),
            (
                "projectmanager",
                "build",
                "project_launch",
                "both copies must name the tool that opens the project-rooted terminal",
            ),
        ];

        for &(extension_name, descriptor_id, claim, reason) in PAIRS {
            let extension = PLATFORM_EXTENSIONS
                .values()
                .find(|def| def.name == extension_name)
                .unwrap_or_else(|| panic!("paired extension {extension_name:?} is not registered"));
            let other = find_descriptor(descriptor_id)
                .unwrap_or_else(|| panic!("paired descriptor {descriptor_id:?} is not registered"));
            let extension_prose = format!("{} {}", extension.description, extension.why_it_matters);
            let other_prose = format!("{} {}", other.what_it_does, other.why_it_matters);

            assert!(
                extension_prose.contains(claim),
                "extension {extension_name:?} lost shared claim {claim:?}: {reason}"
            );
            assert!(
                other_prose.contains(claim),
                "descriptor {descriptor_id:?} lost shared claim {claim:?}: {reason}"
            );
        }
    }

    #[test]
    fn every_known_worker_has_a_descriptor() {
        for id in KNOWN_WORKER_IDS {
            let n = WORKER_DESCRIPTORS.iter().filter(|d| d.id == *id).count();
            assert_eq!(n, 1, "worker id {id:?} must have exactly one descriptor");
        }
        assert_eq!(
            WORKER_DESCRIPTORS.len(),
            KNOWN_WORKER_IDS.len(),
            "WORKER_DESCRIPTORS has an entry not in KNOWN_WORKER_IDS"
        );
    }

    #[test]
    fn every_known_surface_has_a_descriptor() {
        for id in KNOWN_SURFACE_IDS {
            let n = SURFACE_DESCRIPTORS.iter().filter(|d| d.id == *id).count();
            assert_eq!(n, 1, "surface id {id:?} must have exactly one descriptor");
        }
        assert_eq!(
            SURFACE_DESCRIPTORS.len(),
            KNOWN_SURFACE_IDS.len(),
            "SURFACE_DESCRIPTORS has an entry not in KNOWN_SURFACE_IDS"
        );
    }

    /// Every known guardrail must have exactly one descriptor, be in the Guard
    /// category, and render into the brief — Henry must be able to describe a
    /// constraint he is subject to.
    #[test]
    fn guardrails_have_descriptors_and_render() {
        const KNOWN_GUARD_IDS: &[&str] = &[
            "credential_commit_guard",
            "durability_supervision",
            "runaway_loop_guard",
            "tool_argument_validation",
            "sovereignty_guard",
            "goal_landing",
        ];
        for id in KNOWN_GUARD_IDS {
            let n = GUARD_DESCRIPTORS.iter().filter(|d| d.id == *id).count();
            assert_eq!(n, 1, "guard id {id:?} must have exactly one descriptor");
        }
        assert_eq!(
            GUARD_DESCRIPTORS.len(),
            KNOWN_GUARD_IDS.len(),
            "GUARD_DESCRIPTORS has an entry not in KNOWN_GUARD_IDS"
        );
        for d in GUARD_DESCRIPTORS {
            assert_eq!(d.category, FeatureCategory::Guard);
        }

        let brief = SelfKnowledgeBuilder {
            agent_display_name: "Aria".to_string(),
            scheduled_job_count: None,
            flags: FeatureFlags::default(),
            dispatchable_workers: Vec::new(),
            agent_briefings: None,
        }
        .build();
        assert!(brief.contains("## Guardrails you operate under"));
        assert!(brief.contains("**Credential commit guard**"));
        assert!(brief.contains("**Tool argument validation**"));
        // The sovereignty guard renders unconditionally (always-visible, like
        // every guard): the agent must be able to describe the boundary — and
        // recognize a `[sovereign]` refusal — even when the mode is off, and
        // sovereignty is per-context, so there is no single bit to gate on.
        assert!(brief.contains("**Sovereignty guard**"));
        assert!(brief.contains("[sovereign]"));
    }

    #[test]
    fn workers_are_queryable_surfaces_are_static() {
        for d in WORKER_DESCRIPTORS {
            assert_eq!(d.category, FeatureCategory::Worker);
            assert_eq!(
                d.state_source,
                StateSource::Queryable,
                "{} is a worker and must be Queryable",
                d.id
            );
        }
        for d in SURFACE_DESCRIPTORS {
            assert_eq!(d.category, FeatureCategory::Surface);
            assert_eq!(
                d.state_source,
                StateSource::Static,
                "{} is a surface and must be Static",
                d.id
            );
        }
    }

    #[test]
    fn v1_lessons_have_authored_steps() {
        for id in V1_LESSON_IDS {
            let d = find_descriptor(id)
                .unwrap_or_else(|| panic!("lesson feature {id:?} has no descriptor"));
            assert!(
                !d.teaching.is_empty(),
                "lesson feature {id:?} must have authored teaching steps"
            );
            let lesson = lesson_for(id).expect("lesson_for must render a known feature");
            assert!(lesson.contains("# Lesson:"));
            assert!(lesson.contains("Step 1"));
        }
    }

    #[test]
    fn unknown_feature_has_no_lesson() {
        assert!(lesson_for("not_a_real_feature").is_none());
    }

    /// Every authored confirm must be readable back from something the brief
    /// renders for that feature, or from `search_memory`.
    ///
    /// `HasScheduledJob` and `LibrarianDescribedAtLeastOne` are read back from
    /// the feature's own live status line, which [`worker_live_state_for`]
    /// returns `None` for unless `state_source` is `Queryable`. So a `Static`
    /// feature authoring either of those asks the agent to confirm against a
    /// line that will not be there. The other variants read signals the brief
    /// renders unconditionally — the tool `[active]` flag, the "You are <name>"
    /// line — or go through the `search_memory` proxy, and are fine either way.
    ///
    /// The earlier form of this test stated a rule in a comment and checked
    /// something else: it matched `if let Some(MemoryRecallable(p))` and
    /// asserted only that the phrase was non-empty, so every non-matching
    /// variant — including the two that are genuinely unconfirmable — fell
    /// through the `if` and passed. The match below is exhaustive, so a new
    /// `ConfirmCheck` cannot be added without deciding here whether a Static
    /// feature can read it back.
    #[test]
    fn a_static_feature_never_confirms_against_a_live_status_line() {
        let mut confirms_seen = 0;
        let mut static_confirms_seen = 0;

        for d in WORKER_DESCRIPTORS
            .iter()
            .chain(SURFACE_DESCRIPTORS)
            .chain(GUARD_DESCRIPTORS)
            .copied()
            .chain(PLATFORM_EXTENSIONS.values().map(|def| def.descriptor()))
        {
            for step in d.teaching {
                let Some(confirm) = step.confirm else {
                    continue;
                };
                confirms_seen += 1;
                let is_static = d.state_source == StateSource::Static;
                if is_static {
                    static_confirms_seen += 1;
                }
                let (needs_live_status, phrase) = match confirm {
                    // Read back via search_memory — no brief line involved.
                    ConfirmCheck::MemoryRecallable(p) => (false, Some(p)),
                    // Rendered unconditionally, not via worker_live_state_for.
                    ConfirmCheck::ExtensionEnabled(_) | ConfirmCheck::PersonaConfigured => {
                        (false, None)
                    }
                    // Merged only for Queryable features.
                    ConfirmCheck::HasScheduledJob | ConfirmCheck::LibrarianDescribedAtLeastOne => {
                        (true, None)
                    }
                };
                assert!(
                    !(needs_live_status && is_static),
                    "{}: a Static feature confirms with {confirm:?}, which reads back from a \
                     live status line that worker_live_state_for suppresses for Static \
                     features — the step can never be confirmed. Use MemoryRecallable, or \
                     make the feature Queryable and merge the signal into the brief.",
                    d.id
                );
                if let Some(p) = phrase {
                    assert!(
                        !p.is_empty(),
                        "{}: MemoryRecallable phrase must be non-empty — the agent searches \
                         memory for it verbatim",
                        d.id
                    );
                }
            }
        }

        // Floors. Without these the loop above passes on a registry where no
        // step authors a confirm at all, which is what the previous version of
        // this test was actually doing whenever the `if let` missed.
        assert!(
            confirms_seen > 0,
            "no teaching step authors a confirm, so the rule went untested"
        );
        assert!(
            static_confirms_seen > 0,
            "no Static feature authors a confirm, so the case this test exists \
             for was never exercised — Reader is expected to be one"
        );
    }

    #[test]
    fn brief_renders_every_category() {
        let brief = SelfKnowledgeBuilder {
            agent_display_name: "Aria".to_string(),
            scheduled_job_count: Some(3),
            flags: FeatureFlags::default(),
            dispatchable_workers: Vec::new(),
            agent_briefings: None,
        }
        .build();

        // Header + the agent name (never hardcoded — passed in here).
        assert!(brief.contains("# Who You Are"));
        assert!(brief.contains("You are Aria"));
        // Each category section present.
        assert!(brief.contains("## Tools you can call"));
        assert!(brief.contains("## Background workers"));
        assert!(brief.contains("## Surfaces the user can see"));
        // A representative tool, both workers, both surfaces are visible.
        assert!(brief.contains("**Analyze**"));
        assert!(brief.contains("**Scheduler**"));
        assert!(brief.contains("**Librarian**"));
        assert!(brief.contains("**Reader**"));
        assert!(brief.contains("**World View**"));
        // Phase-2 surfaces added to the registry.
        assert!(brief.contains("**Brain**"));
        assert!(brief.contains("**Persona"));
        // Queryable scheduler state merged in.
        assert!(brief.contains("3 job(s) scheduled"));
    }

    /// The two config-backed flags must reach the brief only through
    /// [`FeatureFlags`]. Before this, the renderer read `Config::global()` itself,
    /// so the brief — and the four canonical prompt snapshots taken from it —
    /// encoded whether the developer running the suite had the Guard switched on.
    #[test]
    fn config_backed_flags_reach_the_brief_only_through_the_builder() {
        let parts = |flags| {
            SelfKnowledgeBuilder {
                agent_display_name: "Aria".to_string(),
                scheduled_job_count: None,
                flags,
                dispatchable_workers: Vec::new(),
                agent_briefings: None,
            }
            .build_parts()
        };
        let build = |flags| {
            let (stable, volatile) = parts(flags);
            format!("{stable}{volatile}")
        };
        let off = build(FeatureFlags::default());
        let guard_on = build(FeatureFlags {
            strix_enabled: true,
            ..FeatureFlags::default()
        });
        let initiative_on = build(FeatureFlags {
            initiative_enabled: true,
            ..FeatureFlags::default()
        });

        // Off, the Guard is render-gated out entirely (not rendered as "off"),
        // in BOTH halves — a worker hidden from the inventory must not reappear
        // as a live-status line behind the cache breakpoint.
        assert!(!off.contains("The Guard"));
        assert!(off.contains("off (initiative_enabled=false)"));

        assert!(guard_on.contains("**The Guard**"));
        assert!(guard_on.contains("on — security sweeps every 24h"));

        // Enabling the flag must not disturb anything else in the brief. Since
        // the prefix/suffix split, the Guard costs exactly TWO lines, one in
        // each half, and the halves are checked separately so a line that moved
        // across the breakpoint cannot hide inside a single total.
        let (off_stable, off_volatile) = parts(FeatureFlags::default());
        let (on_stable, on_volatile) = parts(FeatureFlags {
            strix_enabled: true,
            ..FeatureFlags::default()
        });
        // The inventory half gains the Guard's description…
        assert_eq!(off_stable.lines().count() + 1, on_stable.lines().count());
        assert_eq!(
            on_stable
                .lines()
                .filter(|l| l.contains("The Guard"))
                .count(),
            1
        );
        // …and the live half gains its current state, and nothing more.
        assert_eq!(
            off_volatile.lines().count() + 1,
            on_volatile.lines().count()
        );
        assert_eq!(
            on_volatile
                .lines()
                .filter(|l| l.contains("The Guard"))
                .count(),
            1
        );

        assert!(initiative_on.contains("on — watching for repeated commands"));
    }

    /// B1 render-gate: the flag-gated Decision Playbook worker is HIDDEN from the
    /// brief when `playbook_enabled` is off. This is why the canonical
    /// prompt_manager snapshots stay byte-for-byte unchanged — off is a no-op.
    #[test]
    fn playbook_descriptor_hidden_when_flag_off() {
        let brief = SelfKnowledgeBuilder {
            agent_display_name: "Aria".to_string(),
            scheduled_job_count: None,
            flags: FeatureFlags::default(),
            dispatchable_workers: Vec::new(),
            agent_briefings: None,
        }
        .build();

        assert!(
            !brief.contains("Decision Playbook"),
            "playbook descriptor must be hidden from the brief when the flag is off"
        );
    }

    /// B1 enabled rendering: when `playbook_enabled` is on, the Decision
    /// Playbook worker renders in the brief with its hints-with-provenance
    /// framing — so the capability the agent can DO is exactly the one it can
    /// DESCRIBE. The dedicated guard the coordinator asked for.
    #[test]
    fn playbook_descriptor_shown_when_flag_on() {
        let brief = SelfKnowledgeBuilder {
            agent_display_name: "Aria".to_string(),
            scheduled_job_count: None,
            flags: FeatureFlags {
                playbook_enabled: true,
                ..FeatureFlags::default()
            },
            dispatchable_workers: Vec::new(),
            agent_briefings: None,
        }
        .build();

        assert!(
            brief.contains("**Decision Playbook**"),
            "playbook descriptor must render in the brief when the flag is on"
        );
        // The rendered copy must carry the non-authoritative, provenance framing.
        assert!(
            brief.contains("provenance"),
            "the rendered playbook descriptor must convey hints-with-provenance"
        );
    }

    /// Concierge render-gate (#640): the flag-gated Concierge inbox-triage
    /// character is HIDDEN from the brief when `concierge_enabled` is off. This
    /// is why the canonical prompt_manager snapshots stay byte-for-byte
    /// unchanged by this PR — off is a no-op.
    #[test]
    fn concierge_descriptor_hidden_when_flag_off() {
        let brief = SelfKnowledgeBuilder {
            agent_display_name: "Aria".to_string(),
            scheduled_job_count: None,
            flags: FeatureFlags::default(),
            dispatchable_workers: Vec::new(),
            agent_briefings: None,
        }
        .build();

        assert!(
            !brief.contains("The Concierge"),
            "concierge descriptor must be hidden from the brief when the flag is off"
        );
    }

    /// Concierge enabled rendering: when `concierge_enabled` is on, the
    /// character renders in the brief carrying its safe-by-construction framing
    /// (draft-only, read-only, local-tier) — so the capability the agent can DO is
    /// exactly the one it can DESCRIBE.
    #[test]
    fn concierge_descriptor_shown_when_flag_on() {
        let brief = SelfKnowledgeBuilder {
            agent_display_name: "Aria".to_string(),
            scheduled_job_count: None,
            flags: FeatureFlags {
                concierge_enabled: true,
                ..FeatureFlags::default()
            },
            dispatchable_workers: Vec::new(),
            agent_briefings: None,
        }
        .build();

        assert!(
            brief.contains("**The Concierge**"),
            "concierge descriptor must render in the brief when the flag is on"
        );
        // The rendered copy must convey the load-bearing safety properties.
        assert!(
            brief.contains("draft") && brief.contains("read-only") && brief.contains("local"),
            "the rendered concierge descriptor must convey draft-only, read-only, local-tier"
        );
    }

    #[test]
    fn librarian_is_not_double_listed_as_a_tool() {
        let (inventory, live) = SelfKnowledgeBuilder {
            agent_display_name: "Aria".to_string(),
            scheduled_job_count: None,
            flags: FeatureFlags::default(),
            dispatchable_workers: Vec::new(),
            agent_briefings: None,
        }
        .build_parts();
        // It appears once in the inventory (under workers, not also under
        // tools). Asserted against the inventory half specifically: the live
        // half legitimately names it again to report its counters, and folding
        // the two together would make this guard unable to tell a genuine
        // double-listing from a status line.
        assert_eq!(inventory.matches("**Librarian**").count(), 1);
        assert!(
            live.contains("**Librarian**"),
            "the librarian's live counters belong in the volatile half"
        );
    }

    /// The orchestrator self-describes its real capability — goal orchestration
    /// and the supervised Decision Inbox — not just bare session management. The
    /// text is auto-rendered from its `PlatformExtensionDef` (no hand-listed
    /// descriptor); this locks the enriched copy in place so a future trim of
    /// the registry strings can't silently strip Henry's self-knowledge of it.
    #[test]
    fn orchestrator_self_describes_goal_orchestration_and_decision_inbox() {
        let brief = SelfKnowledgeBuilder {
            agent_display_name: "Aria".to_string(),
            scheduled_job_count: None,
            flags: FeatureFlags::default(),
            dispatchable_workers: Vec::new(),
            agent_briefings: None,
        }
        .build();
        // Rendered once, under Tools (it is a platform extension, not hidden).
        assert_eq!(brief.matches("**Orchestrator**").count(), 1);
        // The capability narrative the audited #390 work cares about.
        assert!(
            brief.contains("dispatch roadmap goals"),
            "orchestrator brief must mention goal dispatch"
        );
        assert!(
            brief.contains("Decision Inbox"),
            "orchestrator brief must mention the Decision Inbox"
        );
        assert!(
            brief.contains("supervised approval"),
            "orchestrator brief must mention supervised approval"
        );
    }

    #[test]
    fn dispatchable_workers_render_when_present_and_omit_when_empty() {
        // Empty → no section (snapshot-stable default).
        let empty = SelfKnowledgeBuilder {
            agent_display_name: "Aria".to_string(),
            scheduled_job_count: None,
            flags: FeatureFlags::default(),
            dispatchable_workers: Vec::new(),
            agent_briefings: None,
        }
        .build();
        assert!(!empty.contains("Workers you can dispatch goals to"));

        // Present → a section listing each worker + its status.
        let brief = SelfKnowledgeBuilder {
            agent_display_name: "Aria".to_string(),
            scheduled_job_count: None,
            flags: FeatureFlags::default(),
            dispatchable_workers: vec![
                DispatchableWorker {
                    display_name: "Claude Code".to_string(),
                    status: "available".to_string(),
                },
                DispatchableWorker {
                    display_name: "Librarian".to_string(),
                    status: "engine pending".to_string(),
                },
            ],
            agent_briefings: None,
        }
        .build();
        assert!(brief.contains("## Workers you can dispatch goals to"));
        assert!(brief.contains("**Claude Code** — available"));
        assert!(brief.contains("**Librarian** — engine pending"));
    }

    /// The three states must stay distinguishable. `None` (could not read) and
    /// `Some(vec![])` (read, nothing pending) are NOT the same claim, and
    /// collapsing them would have Henry telling the user his agents have
    /// nothing to report on the strength of a query that never ran.
    #[test]
    fn briefings_distinguish_unreadable_from_empty_from_present() {
        let base = |briefings| {
            SelfKnowledgeBuilder {
                agent_display_name: "Henry".to_string(),
                scheduled_job_count: None,
                flags: FeatureFlags::default(),
                dispatchable_workers: Vec::new(),
                agent_briefings: briefings,
            }
            .build()
        };

        // Could not read → the section is omitted entirely. Henry says nothing
        // about briefings rather than something false.
        let unreadable = base(None);
        assert!(
            !unreadable.contains("Briefings from your agents"),
            "an unreadable briefing store must omit the section, not render it empty"
        );

        // Read, nothing pending → the section renders and says so.
        let empty = base(Some(Vec::new()));
        assert!(empty.contains("## Briefings from your agents"));
        assert!(empty.contains("Nothing unread right now."));

        // Present → each briefing is listed with its reporter and severity.
        let present = base(Some(vec![BriefingLine {
            from: "Steward".to_string(),
            severity: "action required".to_string(),
            summary: "Proposed branch delete on `feat/x` — awaiting approval.".to_string(),
        }]));
        assert!(present.contains("**Steward** [action required]"));
        assert!(present.contains("awaiting approval"));
        assert!(
            !present.contains("Nothing unread right now."),
            "a populated list must not also claim nothing is unread"
        );

        // The standing relationship is stated whenever the section renders, so
        // Henry knows his agents report to him even on a quiet day.
        for brief in [&empty, &present] {
            assert!(
                brief.contains("report their work"),
                "the brief must state the reporting relationship, not just the backlog"
            );
        }
    }

    #[test]
    fn dispatchable_workers_from_config_reports_engine_pending_without_probing() {
        use crate::config::agent_identity::{default_roster, AgentConfig, PrimaryPersona};
        let config = AgentConfig {
            primary: PrimaryPersona::default(),
            workers: default_roster(),
        };
        let workers = dispatchable_workers_from_config(&config);
        // The Pending librarian is reported as engine-pending (no probe).
        let lib = workers
            .iter()
            .find(|w| w.display_name == "Librarian")
            .expect("librarian present");
        assert_eq!(lib.status, "engine pending");
        // Sorted deterministically by display name.
        let names: Vec<&str> = workers.iter().map(|w| w.display_name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    /// The coding harness and its cost optimizer are discoverable in
    /// self-knowledge: findable by id, rendered into the brief, and carrying
    /// authored teaching steps so the agent can describe, launch, and GUIDE the
    /// user through them. This is the capstone of the coding-harness workstream
    /// (#719/#720) — the agent must KNOW the harness exists and how to use it,
    /// not just that a Build tab exists.
    #[test]
    fn coding_harness_capabilities_are_discoverable_and_teachable() {
        for (id, display) in [
            ("coding_harness", "Permagent coding harness"),
            ("cost_optimizer", "Cost optimizer"),
        ] {
            let d = find_descriptor(id)
                .unwrap_or_else(|| panic!("{id:?} must be discoverable via find_descriptor"));
            assert_eq!(d.display_name, display);
            assert_eq!(d.category, FeatureCategory::Surface);
            assert!(
                !d.teaching.is_empty(),
                "{id:?} must carry teaching steps so the agent can guide the user"
            );
            let lesson = lesson_for(id).expect("lesson_for must render a known feature");
            assert!(lesson.contains("Step 1"), "{id:?} lesson must have steps");
        }

        let brief = SelfKnowledgeBuilder {
            agent_display_name: "Aria".to_string(),
            scheduled_job_count: None,
            flags: FeatureFlags::default(),
            dispatchable_workers: Vec::new(),
            agent_briefings: None,
        }
        .build();
        // The harness + cost optimizer render under Surfaces and self-describe
        // their headline properties, so the agent can answer "build this with
        // the Permagent harness" knowing what it is and how to launch it.
        assert!(brief.contains("**Permagent coding harness**"));
        assert!(brief.contains("**Cost optimizer**"));
        assert!(brief.contains("provider-agnostic"));
        assert!(brief.contains("launched from the Build tab"));
        // The agent can now describe the independent adversarial reviewer gate:
        // after its own tests pass, a different-model reviewer checks the diff
        // before it calls the work done.
        assert!(
            brief.contains("different-model reviewer adversarially checks the diff"),
            "the coding-harness self-knowledge must describe the independent reviewer gate"
        );
    }

    /// The Grow still loop and per-project publisher must be in the
    /// `permagent_self` brief so the agent can run them.
    #[test]
    fn grow_post_flow_is_in_self_knowledge() {
        let d = find_descriptor("grow").expect("grow must be discoverable");
        assert!(
            d.teaching.len() >= 3,
            "Grow teaching must cover open, draft, and connect"
        );
        let lesson = lesson_for("grow").expect("grow lesson");
        assert!(
            lesson.contains("retry_social_media"),
            "the Grow lesson must name retry_social_media"
        );
        assert!(
            lesson.contains("approve_social_post"),
            "the Grow lesson must name approve_social_post"
        );
        assert!(
            lesson.contains("connect_project_channel"),
            "the Grow lesson must name connect_project_channel"
        );

        let brief = SelfKnowledgeBuilder {
            agent_display_name: "Aria".to_string(),
            scheduled_job_count: None,
            flags: FeatureFlags::default(),
            dispatchable_workers: Vec::new(),
            agent_briefings: None,
        }
        .build();
        assert!(brief.contains("**Grow tab**"));
        assert!(brief.contains("retry_social_media"));
        assert!(brief.contains("approve_social_post"));
        assert!(brief.contains("social_content_brief"));
        assert!(brief.contains("connect_project_channel"));
        assert!(brief.contains("publisher_status"));
        assert!(
            brief.contains("title and body stay")
                || brief.contains("without rewriting the copy")
                || brief.contains("never throw away the copy"),
            "the brief must say regenerating a still keeps the post copy"
        );
        assert!(
            brief.contains("connected account")
                || brief.contains("schedules the post on the connected")
                || brief.contains("via Postiz"),
            "the brief must say Approve posts when this project has connected the channel"
        );
    }

    /// Tokenize a capability description into whole identifier tokens (split on
    /// any non-`[A-Za-z0-9_]` char, lowercased). A tool counts as "named" only
    /// if its name is one of these tokens — so `search` is not satisfied by
    /// "research" nor `tree` by "street".
    fn description_tokens(desc: &str) -> std::collections::HashSet<String> {
        desc.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|t| !t.is_empty())
            .map(|t| t.to_ascii_lowercase())
            .collect()
    }

    /// Every tool in `tools` that `desc` fails to name (empty == complete). The
    /// single checker BOTH the guard and its meta-test run through, so the
    /// meta-test genuinely exercises the guard's logic.
    fn tools_not_named_in(desc: &str, tools: &[String]) -> Vec<String> {
        let toks = description_tokens(desc);
        tools
            .iter()
            .filter(|t| !toks.contains(&t.to_ascii_lowercase()))
            .cloned()
            .collect()
    }

    /// The registry `description` (== `what_it_does`) for a platform extension,
    /// looked up by its `def.name`.
    fn platform_extension_description(ext_name: &str) -> &'static str {
        PLATFORM_EXTENSIONS
            .values()
            .find(|d| d.name == ext_name)
            .unwrap_or_else(|| panic!("extension {ext_name:?} missing from PLATFORM_EXTENSIONS"))
            .description
    }

    /// The statically-derivable tool inventory for every brief-visible
    /// extension, keyed by `PlatformExtensionDef.name`. Each inventory comes
    /// from the extension's REAL tool constructor — the same code `list_tools`
    /// serves — so adding a tool anywhere makes the completeness guard fail
    /// until the registry `description` names it:
    ///
    /// - Most extensions expose a static `get_tools()` (`list_tools` returns it
    ///   verbatim) — fully drift-proof.
    /// - **Extension Manager** gates tools at runtime (resources support, Brain
    ///   loaded) but `get_tools` *selects from* `all_possible_tools()` by name,
    ///   so a tool absent there cannot ship at all; the superset is the
    ///   inventory. `dynamic_tool_inventories_match_constructed_clients` below
    ///   pins the gate split against a real constructed run.
    /// - **Summon** hides `delegate` from subagent sessions; the superset
    ///   (`all_possible_tools()` = the main-session view, where the brief
    ///   renders) is the inventory, pinned by the same constructed-client test.
    /// - **Code Mode** varies by disclosure mode; `all_possible_tools()` is the
    ///   union of every mode's `tools_for_disclosure`, so a tool added to any
    ///   branch lands here automatically.
    fn extension_tool_inventories() -> Vec<(&'static str, Vec<rmcp::model::Tool>)> {
        use crate::agents::platform_extensions::{
            analyze, app_conductor, app_perception, apps, browser, chatrecall, dashboard, desktop,
            developer, ext_manager, file_to_project, finance, inbox_tools, listen, model_manager,
            orchestrator, people, project_manager, pronunciation, recipe_author, retrospect,
            skills, storage_health, summarize, summon, todo,
        };

        let mut project_manager_tools = project_manager::ProjectManagerClient::get_tools();
        // Keep these review-gated research tools explicit in the self-knowledge
        // inventory: prompt-manager snapshots are regenerated by the reviewer.
        let mut review_gated_tools = Vec::new();
        project_manager_tools.retain(|tool| {
            if matches!(
                tool.name.as_ref(),
                "research_project_intel" | "propose_project_intel"
            ) {
                review_gated_tools.push(tool.clone());
                false
            } else {
                true
            }
        });
        project_manager_tools.extend(review_gated_tools);

        let mut inventories = vec![
            (analyze::EXTENSION_NAME, analyze::AnalyzeClient::get_tools()),
            (
                inbox_tools::EXTENSION_NAME,
                inbox_tools::InboxClient::get_tools(),
            ),
            (
                app_conductor::EXTENSION_NAME,
                app_conductor::AppConductorClient::get_tools(),
            ),
            (
                app_perception::EXTENSION_NAME,
                app_perception::AppPerceptionClient::get_tools(),
            ),
            (apps::EXTENSION_NAME, apps::AppsManagerClient::get_tools()),
            (browser::EXTENSION_NAME, browser::BrowserClient::get_tools()),
            (
                chatrecall::EXTENSION_NAME,
                chatrecall::ChatRecallClient::get_tools(),
            ),
            (
                dashboard::EXTENSION_NAME,
                dashboard::DashboardClient::get_tools(),
            ),
            (
                retrospect::EXTENSION_NAME,
                retrospect::RetrospectClient::get_tools(),
            ),
            (
                // Flag-gated at runtime (DESKTOP_CONTROL_ENABLED): `list_tools`
                // selects from this superset, so the superset is the inventory
                // (the Extension Manager precedent).
                desktop::EXTENSION_NAME,
                desktop::DesktopClient::get_tools(),
            ),
            (
                developer::EXTENSION_NAME,
                developer::DeveloperClient::get_tools(),
            ),
            (
                ext_manager::EXTENSION_NAME,
                ext_manager::ExtensionManagerClient::all_possible_tools(),
            ),
            (
                file_to_project::EXTENSION_NAME,
                file_to_project::FileToProjectClient::get_tools(),
            ),
            (finance::EXTENSION_NAME, finance::FinanceClient::get_tools()),
            (listen::EXTENSION_NAME, listen::ListenClient::get_tools()),
            (
                model_manager::EXTENSION_NAME,
                model_manager::ModelManagerClient::get_tools(),
            ),
            (
                orchestrator::EXTENSION_NAME,
                orchestrator::OrchestratorClient::get_tools(),
            ),
            (people::EXTENSION_NAME, people::PeopleClient::get_tools()),
            (project_manager::EXTENSION_NAME, project_manager_tools),
            (
                pronunciation::EXTENSION_NAME,
                pronunciation::PronunciationClient::get_tools(),
            ),
            (
                recipe_author::EXTENSION_NAME,
                recipe_author::RecipeAuthorClient::get_tools(),
            ),
            (skills::EXTENSION_NAME, skills::SkillsClient::get_tools()),
            (
                storage_health::EXTENSION_NAME,
                storage_health::StorageHealthClient::get_tools(),
            ),
            (
                summarize::EXTENSION_NAME,
                summarize::SummarizeClient::get_tools(),
            ),
            (
                summon::EXTENSION_NAME,
                summon::SummonClient::all_possible_tools(),
            ),
            (todo::EXTENSION_NAME, todo::TodoClient::get_tools()),
        ];

        #[cfg(feature = "code-mode")]
        inventories.push((
            crate::agents::platform_extensions::code_execution::EXTENSION_NAME,
            crate::agents::platform_extensions::code_execution::CodeExecutionClient::all_possible_tools(),
        ));

        inventories
    }

    /// Brief-visible extensions with NO callable tools — exempt from the
    /// tool-naming contract because there is nothing to name. Each entry
    /// carries its reason here; `dynamic_tool_inventories_match_constructed_clients`
    /// asserts the claim against a real constructed client.
    ///
    /// - `tom` (Top Of Mind) injects context via `get_moim`; its `list_tools`
    ///   returns an empty vec.
    const NO_TOOL_EXTENSIONS: &[&str] = &[crate::agents::platform_extensions::tom::EXTENSION_NAME];

    /// **Self-knowledge completeness (structural guard).** Every tool an
    /// extension can actually expose must be *named* in that extension's
    /// registry `description` — otherwise the tool renders into no named line
    /// of the `permagent_self` brief and the agent cannot know it exists. This
    /// closes the gap the descriptor contract left open:
    /// `PlatformExtensionDef::descriptor` copies `description` verbatim into
    /// `what_it_does`, so coverage was guaranteed at *extension* granularity
    /// but never at *tool* granularity — an extension could ship a tool its
    /// blurb never mentions and every test stayed green. Shipping an
    /// undescribed tool now fails the build.
    ///
    /// **Generality is enforced, not hoped for:** the case table is
    /// [`extension_tool_inventories`], and a completeness meta-check asserts it
    /// covers every brief-visible extension in `PLATFORM_EXTENSIONS` — so
    /// registering a new extension (or un-hiding one) without adding its
    /// inventory here is itself a red build, in both directions.
    #[test]
    fn tool_descriptions_name_every_callable_tool() {
        let inventories = extension_tool_inventories();

        // ── The naming contract itself ──
        let mut gaps = Vec::new();
        for (ext_name, tools) in &inventories {
            assert!(
                !tools.is_empty(),
                "{ext_name}: empty tool inventory — test wiring is broken"
            );
            let desc = platform_extension_description(ext_name);
            let tool_names: Vec<String> = tools.iter().map(|tool| tool.name.to_string()).collect();
            for missing in tools_not_named_in(desc, &tool_names) {
                gaps.push(format!(
                    "{ext_name}: tool `{missing}` is callable but its description never names it"
                ));
            }
        }
        assert!(
            gaps.is_empty(),
            "self-knowledge completeness gap — an extension ships a tool its \
             description never names, so the agent cannot know the tool exists. \
             Name each missing tool in its `PlatformExtensionDef.description`:\n{}",
            gaps.join("\n")
        );

        // ── Meta-check: the guard covers EVERY brief-visible extension ──
        // The brief's Tools section renders every non-hidden registry entry
        // except those described under another category, so exactly that set
        // must have an inventory (or a documented no-tools exemption).
        let covered: std::collections::HashSet<&str> =
            inventories.iter().map(|(name, _)| *name).collect();
        for def in PLATFORM_EXTENSIONS.values() {
            if def.hidden
                || TOOL_IDS_RENDERED_ELSEWHERE.contains(&def.name)
                || NO_TOOL_EXTENSIONS.contains(&def.name)
            {
                continue;
            }
            assert!(
                covered.contains(def.name),
                "extension {:?} renders into the brief's Tools section but has no tool \
                 inventory registered in extension_tool_inventories() — add its \
                 get_tools()-derived case so its blurb is held to the naming contract",
                def.name
            );
        }
        // …and inversely: every case must be a real, registered extension, so a
        // rename/removal cannot leave a stale case silently asserting nothing.
        for (ext_name, _) in &inventories {
            assert!(
                PLATFORM_EXTENSIONS.values().any(|d| d.name == *ext_name),
                "extension_tool_inventories() lists {ext_name:?} which is not in \
                 PLATFORM_EXTENSIONS — remove or fix the stale case"
            );
        }
    }

    /// **Test-of-the-test.** Proves the completeness guard above actually has
    /// teeth: it must REPORT a gap when a description omits a tool, not silently
    /// pass. Pinned on the highest-priority case — Extension Manager owns
    /// `search_memory` (Brain recall), the exact tool whose omission the r1
    /// guard was built to catch. If a future refactor neutered
    /// `tool_descriptions_name_every_callable_tool` into a no-op, this fails.
    #[test]
    fn completeness_guard_catches_a_dropped_search_memory() {
        use crate::agents::platform_extensions::ext_manager;

        let inventory: Vec<String> = ext_manager::ExtensionManagerClient::all_possible_tools()
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        assert!(
            inventory.iter().any(|t| t == "search_memory"),
            "ext_manager::all_possible_tools() must include search_memory (the tool this guard enforces)"
        );

        // The REAL, shipped Extension Manager description names every tool.
        let real = platform_extension_description(ext_manager::EXTENSION_NAME);
        assert!(
            tools_not_named_in(real, &inventory).is_empty(),
            "the real Extension Manager description must name every tool in all_possible_tools()"
        );

        // The pre-fix blurb (which never named search_memory) MUST be flagged —
        // the exact regression the guard exists to prevent. This is the
        // "verify it RED-builds if search_memory is dropped" check, run in CI.
        let pre_fix_blurb =
            "Enable extension management tools for discovering, enabling, and disabling extensions";
        let missing = tools_not_named_in(pre_fix_blurb, &inventory);
        assert!(
            missing.iter().any(|t| t == "search_memory"),
            "guard is a no-op: a description dropping search_memory was not flagged (missing: {missing:?})"
        );
    }

    /// **RED-build proof for round 2's headline case.** The r1 guard held its
    /// "no undescribed tool ships" property for only 3 extensions — proven when
    /// App Conductor's `open_item` shipped undescribed with everything green.
    /// This pins that exact regression: an App Conductor blurb that stops
    /// naming `open_item` (the pre-r2 registry text, verbatim) must be flagged.
    #[test]
    fn completeness_guard_catches_a_dropped_open_item() {
        use crate::agents::platform_extensions::app_conductor;

        let inventory: Vec<String> = app_conductor::AppConductorClient::get_tools()
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        assert_eq!(
            inventory,
            vec![
                "navigate_app",
                "app_action",
                "open_item",
                "copy_to_clipboard"
            ],
            "app_conductor's real tool list changed — update this pin deliberately"
        );

        let real = platform_extension_description(app_conductor::EXTENSION_NAME);
        assert!(
            tools_not_named_in(real, &inventory).is_empty(),
            "the real App Conductor description must name every tool"
        );

        // The pre-r2 blurb: describes all three capabilities, names zero tools.
        let pre_fix_blurb = "Navigate the user to tabs and views, act within them — \
             open/close/detach the chat dock, show/hide the Build tab's browser and terminal \
             panes — and carry them the last mile past a tab to a specific item: a goal's \
             detail or a project's Grow planner, in the Permagent app";
        let missing = tools_not_named_in(pre_fix_blurb, &inventory);
        assert!(
            missing.iter().any(|t| t == "open_item"),
            "guard is a no-op: a description dropping open_item was not flagged (missing: {missing:?})"
        );
    }

    /// **Dynamic-inventory ground truth.** For the extensions whose `list_tools`
    /// is genuinely dynamic, the static superset the guard uses must match an
    /// actually-constructed client run — otherwise the superset could drift
    /// from reality and the guard would assert against fiction.
    ///
    /// - **Summon**: a non-subagent session sees exactly `all_possible_tools()`
    ///   (`load` + `delegate`). Residual: a tool exposed ONLY to subagent
    ///   sessions would escape this check — none exists today.
    /// - **Extension Manager**: with no extension manager and no Brain, a real
    ///   run returns exactly the ungated prefix of `all_possible_tools()`, and
    ///   the gated remainder is exactly `GATED_TOOL_NAMES`. Residual: the
    ///   resources/Brain gates are not flipped on here (that needs a live
    ///   resource-capable ExtensionManager / the process-global Brain), but
    ///   `get_tools` *selects from* `all_possible_tools()` by name, so even a
    ///   gated tool cannot ship without being in the guarded superset.
    /// - **Top Of Mind**: backs the `NO_TOOL_EXTENSIONS` exemption — a real
    ///   run returns no tools.
    #[tokio::test]
    async fn dynamic_tool_inventories_match_constructed_clients() {
        use crate::agents::mcp_client::McpClientTrait;
        use crate::agents::platform_extensions::{
            ext_manager, summon, tom, PlatformExtensionContext,
        };

        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().display().to_string();
        let _guard = env_lock::lock_env([
            ("HOME", Some(root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(root.as_str())),
        ]);
        let context = PlatformExtensionContext {
            extension_manager: None,
            session_manager: std::sync::Arc::new(crate::session::SessionManager::new(
                tmp.path().to_path_buf(),
            )),
            session: None,
        };
        let cancel = tokio_util::sync::CancellationToken::new();

        // Summon: an unknown session id is not a subagent → the full superset.
        let summon_client = summon::SummonClient::new(context.clone()).expect("summon constructs");
        let listed: Vec<String> = summon_client
            .list_tools("not-a-real-session", None, cancel.clone())
            .await
            .expect("summon list_tools")
            .tools
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        let superset: Vec<String> = summon::SummonClient::all_possible_tools()
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        assert_eq!(
            listed, superset,
            "summon's main-session list_tools must equal all_possible_tools() — \
             a tool was added to one but not the other"
        );

        // Extension Manager: ungated prefix + gated tail == the full superset.
        let ext_client = ext_manager::ExtensionManagerClient::new(context.clone())
            .expect("ext_manager constructs");
        let listed: Vec<String> = ext_client
            .list_tools("not-a-real-session", None, cancel.clone())
            .await
            .expect("ext_manager list_tools")
            .tools
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        let all: Vec<String> = ext_manager::ExtensionManagerClient::all_possible_tools()
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        let gated: Vec<String> = ext_manager::ExtensionManagerClient::GATED_TOOL_NAMES
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            listed.as_slice(),
            &all[..all.len() - gated.len()],
            "with no gates open, a real run must return exactly the ungated prefix \
             of all_possible_tools()"
        );
        assert_eq!(
            &all[listed.len()..],
            gated.as_slice(),
            "the gated tail of all_possible_tools() must be exactly GATED_TOOL_NAMES — \
             a gated tool was added to one but not the other"
        );

        // Top Of Mind: genuinely tool-less (the NO_TOOL_EXTENSIONS exemption).
        let tom_client = tom::TomClient::new(context).expect("tom constructs");
        let listed = tom_client
            .list_tools("not-a-real-session", None, cancel)
            .await
            .expect("tom list_tools");
        assert!(
            listed.tools.is_empty(),
            "tom is exempted as tool-less but a real run returned tools: {:?} — \
             remove the exemption and name them in its description",
            listed
                .tools
                .iter()
                .map(|t| t.name.to_string())
                .collect::<Vec<_>>()
        );
    }

    /// Maximal `[A-Za-z0-9_]` runs in `text` that look like tool names:
    /// all-lowercase, at least one underscore, starting with a letter. This is
    /// deliberately narrow — single-word tool names (`load`, `verify`, …) are
    /// indistinguishable from prose and are NOT validated (documented residual);
    /// uppercase tokens (env vars like `PERMAGENT_MOIM_MESSAGE_TEXT`) are
    /// excluded.
    fn tool_shaped_tokens(text: &str) -> Vec<String> {
        text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|t| {
                t.contains('_')
                    && t.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                    && t.chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            })
            .map(|t| t.to_string())
            .collect()
    }

    /// Real, callable tools that exist OUTSIDE the statically-enumerable
    /// platform-extension universe, so descriptor prose may legitimately name
    /// them. Every entry needs a justification:
    ///
    /// - `web_search`: registered by the connected search provider (Brave or
    ///   Tavily) when one is configured — the Audience Listening fall-through
    ///   and the Web search surface descriptor reference it, and the surface
    ///   copy already states its conditionality ("when one is connected its
    ///   search tools appear in your tool list").
    const KNOWN_DYNAMIC_TOOLS: &[&str] = &["web_search"];

    /// Snake_case prose tokens that are NOT tools (parameter names, config
    /// keys, …). Kept minimal — every entry needs a justification, so that a new
    /// snake_case token must be either a real tool or an explicit choice here.
    ///
    /// - `sub_recipes`, `worker_persona`: `create_recipe` (recipe_author)
    ///   authoring FIELDS on the `Recipe`/`ScheduledJob`, named in the
    ///   descriptor prose so the agent knows the richer recipe surface exists.
    ///   They are recipe schema keys, not callable tools.
    // `strix_llm` is a config.yaml key (the Guard's scanner model), named in
    // the Guard's cost teaching step — a setting, not a tool.
    // `strix_docker_ssh` is the Guard's remote scanner host (`user@host`) in
    // ~/.permagent/config.yaml, named in the Guard's setup teaching steps so
    // the agent checks Docker on that machine instead of this Mac.
    // `steward_scan_enabled` is the git-health sweep's config flag
    // (~/.permagent/config.yaml), named in the git_steward descriptor so the
    // agent can tell the user how to turn the sweep on — a setting, not a tool.
    // `recipe_author` is an EXTENSION name (a bundle of tools), not a tool. It
    // is named in the Scheduler descriptor because it is one of the two entries
    // in `scheduler::HEADLESS_DENYLIST` — a headless run withholds exactly
    // `orchestrator` and `recipe_author` unless the recipe declares them, and
    // the agent can only state that guardrail correctly by naming them. Its
    // sibling `orchestrator` needs no entry only because it happens to be a
    // single word, so the tool-shaped-token scan never sees it.
    // `decision_inbox` is an ASPECT of `observe_app` (a surface it can read),
    // not a tool. It is named in the app_awareness descriptor because `inbox`
    // and `decision_inbox` are two different surfaces — Downloads intake versus
    // what is waiting on the user's approval — and the agent picks the wrong one
    // unless the prose distinguishes them. Its sibling aspects (`grow`, `trace`,
    // `brain`, …) need no entry only because they are single words the
    // tool-shaped-token scan never sees.
    const NON_TOOL_PROSE_TOKENS: &[&str] = &[
        "sub_recipes",
        "worker_persona",
        "strix_llm",
        "strix_docker_ssh",
        "steward_scan_enabled",
        "recipe_author",
        "decision_inbox",
    ];

    /// Every tool name that exists in the runtime: the statically-derived
    /// per-extension inventories, hidden-but-real extensions (Git Steward),
    /// the two platform tools registered directly in `agent.rs`, and the
    /// documented dynamic extras.
    fn real_tool_inventory() -> std::collections::HashSet<String> {
        let mut inv: std::collections::HashSet<String> = extension_tool_inventories()
            .into_iter()
            .flat_map(|(_, tools)| tools)
            .map(|tool| tool.name.to_string())
            .collect();
        inv.extend(
            crate::agents::platform_extensions::steward::StewardClient::get_tools()
                .iter()
                .map(|t| t.name.to_string()),
        );
        inv.insert(
            crate::agents::platform_tools::manage_schedule_tool()
                .name
                .to_string(),
        );
        inv.insert(
            crate::agents::platform_tools::load_feature_lesson_tool()
                .name
                .to_string(),
        );
        inv.extend(KNOWN_DYNAMIC_TOOLS.iter().map(|s| s.to_string()));
        inv
    }

    /// **Phantom-tool guard.** Rendered descriptor prose — `what_it_does`,
    /// `why_it_matters`, and teaching steps, across ALL registries including
    /// surfaces — must never name a tool that does not exist. This is the class
    /// the `list_projects` phantom escaped through: the Projects-workspace
    /// surface told the agent to reach for a tool whose real name is
    /// `project_list`, and no guard scanned surface prose at all. Now every
    /// tool-shaped token (see [`tool_shaped_tokens`] — backticked or bare) must
    /// be a real tool from [`real_tool_inventory`], an entry in
    /// [`KNOWN_DYNAMIC_TOOLS`], or an explicitly-classified non-tool in
    /// [`NON_TOOL_PROSE_TOKENS`].
    ///
    /// Out of scope (documented residuals): single-word tool names cannot be
    /// told apart from prose, and tool *descriptions* themselves are not
    /// scanned (the model reads those alongside the real tool list, which
    /// self-corrects a phantom there; descriptor prose renders into the brief
    /// with no such correction).
    #[test]
    fn descriptor_prose_names_only_real_tools() {
        let inventory = real_tool_inventory();

        let platform: Vec<FeatureDescriptor> = PLATFORM_EXTENSIONS
            .values()
            .map(|d| d.descriptor())
            .collect();
        let sources: [(&str, &[FeatureDescriptor]); 4] = [
            ("platform_extension", platform.as_slice()),
            ("worker", WORKER_DESCRIPTORS),
            ("guard", GUARD_DESCRIPTORS),
            ("surface", SURFACE_DESCRIPTORS),
        ];

        let mut phantoms = Vec::new();
        for (kind, descriptors) in sources {
            for d in descriptors {
                let mut fields: Vec<(String, &str)> = vec![
                    ("what_it_does".to_string(), d.what_it_does),
                    ("why_it_matters".to_string(), d.why_it_matters),
                ];
                for (i, step) in d.teaching.iter().enumerate() {
                    fields.push((format!("teaching[{i}].title"), step.title));
                    fields.push((format!("teaching[{i}].body"), step.body));
                }
                for (field, text) in fields {
                    for token in tool_shaped_tokens(text) {
                        if !inventory.contains(&token)
                            && !NON_TOOL_PROSE_TOKENS.contains(&token.as_str())
                        {
                            phantoms.push(format!(
                                "{kind} descriptor {:?} field `{field}` names `{token}`, \
                                 which is not a real tool",
                                d.id
                            ));
                        }
                    }
                }
            }
        }

        assert!(
            phantoms.is_empty(),
            "descriptor prose names nonexistent tool(s) — fix the name to the real \
             tool, or (if it is a real dynamic tool) add it to KNOWN_DYNAMIC_TOOLS \
             with justification, or (if it is not a tool) classify it in \
             NON_TOOL_PROSE_TOKENS:\n{}",
            phantoms.join("\n")
        );
    }

    /// **Test-of-the-test for the phantom guard.** The exact historical
    /// phantom — `list_projects` in the Projects-workspace surface prose —
    /// must be flagged by the scanner, and the real neighboring tool name
    /// (`board_summary`) must not. If the tokenizer or inventory ever loosens
    /// into a no-op, this fails.
    #[test]
    fn phantom_guard_catches_the_list_projects_phantom() {
        let inventory = real_tool_inventory();
        // The pre-fix PROJECT_WORKSPACE_FEATURE prose, verbatim.
        let pre_fix = "Reach for the project tools (list_projects, board_summary) \
             to read or change what this surface shows";
        let flagged: Vec<String> = tool_shaped_tokens(pre_fix)
            .into_iter()
            .filter(|t| !inventory.contains(t) && !NON_TOOL_PROSE_TOKENS.contains(&t.as_str()))
            .collect();
        assert_eq!(
            flagged,
            vec!["list_projects".to_string()],
            "the scanner must flag exactly the phantom (list_projects) and accept \
             the real tool (board_summary)"
        );
    }

    /// **Branding guard (systems fix).** No user-facing capability string may
    /// leak the upstream `goose` fork name. Every field that renders into the
    /// self-knowledge brief and the capability cards — `display_name`,
    /// `what_it_does`, `why_it_matters` — is scanned case-insensitively across
    /// all descriptor registries (platform extensions, workers, guards,
    /// surfaces). A re-introduced "goose" in card copy or the brief fails the
    /// build instead of shipping. The scan also covers the two out-of-registry
    /// platform tools (`manage_schedule`, `load_feature_lesson`) registered
    /// directly in `agent.rs` rather than via PLATFORM_EXTENSIONS — their
    /// user-facing name/description is exactly where a leak previously hid.
    ///
    /// In scope: rendered card/brief copy only. OUT of scope (never reaches
    /// these strings): the internal crate name `goose`, directory paths, and
    /// type identifiers — renaming those is a separate refactor. If a rendered
    /// field must ever legitimately reference the crate, add an explicit
    /// `(descriptor_id, field_name)` pair to `ALLOWLIST` with justification.
    #[test]
    fn no_user_facing_goose_branding_leak() {
        // Internal-OK allowlist. Empty by design — nothing user-facing
        // legitimately says "goose". Each entry is (descriptor id, field name).
        const ALLOWLIST: &[(&str, &str)] = &[];

        // Platform extensions render via their derived descriptor; the worker /
        // guard / surface registries are already `FeatureDescriptor`s.
        let platform: Vec<FeatureDescriptor> = PLATFORM_EXTENSIONS
            .values()
            .map(|d| d.descriptor())
            .collect();
        let sources: [(&str, &[FeatureDescriptor]); 4] = [
            ("platform_extension", platform.as_slice()),
            ("worker", WORKER_DESCRIPTORS),
            ("guard", GUARD_DESCRIPTORS),
            ("surface", SURFACE_DESCRIPTORS),
        ];

        let mut leaks = Vec::new();
        for (kind, descriptors) in sources {
            for d in descriptors {
                for (field, text) in [
                    ("display_name", d.display_name),
                    ("what_it_does", d.what_it_does),
                    ("why_it_matters", d.why_it_matters),
                ] {
                    if text.to_lowercase().contains("goose") && !ALLOWLIST.contains(&(d.id, field))
                    {
                        leaks.push(format!(
                            "{kind} descriptor {:?} field `{field}` leaks 'goose': {text:?}",
                            d.id
                        ));
                    }
                }
            }
        }

        // Out-of-registry platform tools: `manage_schedule` and
        // `load_feature_lesson` are pushed straight into the tool list in
        // `agent.rs`, not via PLATFORM_EXTENSIONS, so the descriptor scan above
        // never reaches them. Their user-facing name + description must not leak
        // the fork name either — this is where the goose-riddled schedule blurb
        // used to hide, escaping the descriptor-only guard.
        for tool in [
            crate::agents::platform_tools::manage_schedule_tool(),
            crate::agents::platform_tools::load_feature_lesson_tool(),
        ] {
            let desc = tool.description.as_deref().unwrap_or_default();
            for (field, text) in [("name", &*tool.name), ("description", desc)] {
                if text.to_lowercase().contains("goose") {
                    leaks.push(format!(
                        "platform_tool {:?} field `{field}` leaks 'goose': {text:?}",
                        tool.name
                    ));
                }
            }
        }

        assert!(
            leaks.is_empty(),
            "user-facing 'goose' branding leak(s) found — rebrand to Permagent \
             (or allowlist if genuinely internal):\n{}",
            leaks.join("\n")
        );
    }
}
