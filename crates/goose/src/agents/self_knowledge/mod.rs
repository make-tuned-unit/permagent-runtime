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
    // Render-gated (see `worker_descriptor_visible`): hidden from the brief
    // while `PERMAGENT_PLAYBOOK_ENABLED` is off, so this experimental, unproven
    // capability does not enter every user's Henry until deliberately enabled.
    crate::playbook::PLAYBOOK_SYNTHESIS_FEATURE,
    // Render-gated on `PERMAGENT_CONCIERGE_ENABLED` (default OFF): the Concierge
    // inbox-triage character (#640) is hidden from the brief until deliberately
    // enabled, so the canonical prompt snapshots stay byte-for-byte identical.
    crate::concierge::SELF_KNOWLEDGE_FEATURE,
];

/// Whether a worker descriptor should be rendered into the `permagent_self`
/// brief. Almost all are always visible; a flag-gated, experimental worker is
/// hidden until its flag is on, so the capability the agent can DESCRIBE is
/// exactly the one it can DO — and, with the flag off, the brief is byte-for-
/// byte identical to before the descriptor existed (the canonical snapshots
/// stay unchanged; a dedicated test covers the enabled rendering).
fn worker_descriptor_visible(d: &FeatureDescriptor) -> bool {
    if d.id == crate::playbook::PLAYBOOK_FEATURE_ID {
        return crate::playbook::is_enabled();
    }
    if d.id == crate::concierge::CONCIERGE_FEATURE_ID {
        return crate::concierge::is_enabled();
    }
    true
}

/// Deterministic guardrails the agent operates under. Co-located with the
/// safety-core module that enforces each one.
pub static GUARD_DESCRIPTORS: &[FeatureDescriptor] = &[
    crate::steward::secret_scan::SELF_KNOWLEDGE_FEATURE,
    crate::session::crash_capture::DURABILITY_FEATURE,
    crate::tool_monitor::SELF_KNOWLEDGE_FEATURE,
    crate::sovereignty::SELF_KNOWLEDGE_FEATURE,
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
    crate::agents::platform_extensions::project_manager::DEVICES_FEATURE,
    crate::decision_inbox::DECISION_INBOX_FEATURE,
    crate::inbox::INBOX_FEATURE,
    crate::activity_journal::TIMELINE_FEATURE,
    crate::scheduler::RUN_ROSTER_FEATURE,
    crate::agents::platform_extensions::project_manager::GROW_FEATURE,
    crate::agents::platform_extensions::analyze::CODEBASE_INDEX_FEATURE,
    crate::agents::platform_extensions::project_manager::CODING_HARNESS_FEATURE,
    crate::cost_router::COST_OPTIMIZER_FEATURE,
    crate::mesh::MESH_FEATURE,
    crate::skills::SKILLS_FEATURE,
    crate::session::SESSIONS_FEATURE,
    crate::events::TRACE_FEATURE,
    crate::dictation::MEETING_DICTATION_FEATURE,
    crate::sovereignty::GOVERNANCE_SURFACE_FEATURE,
];

/// Tool ids that are described under another category and therefore skipped in
/// the Tools section (the librarian is a platform extension *and* a background
/// worker — we describe it once, as a worker, to avoid double-listing).
const TOOL_IDS_RENDERED_ELSEWHERE: &[&str] = &[librarian_state_id()];

const fn librarian_state_id() -> &'static str {
    crate::agents::platform_extensions::librarian::EXTENSION_NAME
}

// ── Builder ────────────────────────────────────────────────────────────

/// Assembles the `permagent_self` brief. Live state that requires async access
/// (the scheduler) is fetched at the call site and passed in; everything else
/// is read from process-global state inside [`build`](Self::build).
pub struct SelfKnowledgeBuilder {
    /// The agent's display name (persona-resolved; default "Aria"). Never
    /// hardcoded — interpolated from the resolved persona.
    pub agent_display_name: String,
    /// Live scheduled-job count (Queryable). `None` when the scheduler is not
    /// wired (e.g. tests) → rendered editorially.
    pub scheduled_job_count: Option<usize>,
    /// Workers the orchestrator can dispatch goals to, with live status.
    /// Pre-computed by the (async) caller so this builder stays pure and
    /// snapshot-stable. Empty → the section is omitted (e.g. tests, or when
    /// orchestration is not active).
    pub dispatchable_workers: Vec<DispatchableWorker>,
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
    /// Build the full brief, grouped by category. Deterministic output (tools
    /// sorted by id) so it is prompt-cache- and snapshot-stable.
    pub fn build(&self) -> String {
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

        // ── Workers you can dispatch goals to (dynamic; orchestrator) ──
        // Populated only when orchestration is active; empty → omitted, keeping
        // the brief snapshot-stable.
        if !self.dispatchable_workers.is_empty() {
            writeln!(out, "\n## Workers you can dispatch goals to").ok();
            for w in &self.dispatchable_workers {
                writeln!(out, "- **{}** — {}", w.display_name, w.status).ok();
            }
        }

        // ── Workers ──
        writeln!(
            out,
            "\n## Background workers (run on their own, even when you are idle)"
        )
        .ok();
        for d in WORKER_DESCRIPTORS {
            // Flag-gated workers are omitted from the brief while disabled — off
            // is a byte-for-byte no-op (see `worker_descriptor_visible`).
            if !worker_descriptor_visible(d) {
                continue;
            }
            let live = self.worker_live_state(d);
            match live {
                Some(state) => writeln!(
                    out,
                    "- **{}** — {}. {} _(now: {})_",
                    d.display_name, d.what_it_does, d.why_it_matters, state
                ),
                None => writeln!(
                    out,
                    "- **{}** — {}. {}",
                    d.display_name, d.what_it_does, d.why_it_matters
                ),
            }
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

        out
    }

    /// Live state for a Queryable worker, or `None` for Static / unavailable.
    fn worker_live_state(&self, d: &FeatureDescriptor) -> Option<String> {
        if d.state_source != StateSource::Queryable {
            return None;
        }
        match d.id {
            "scheduler" => self
                .scheduled_job_count
                .map(|n| format!("{n} job(s) scheduled")),
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
            "initiative" => Some(if crate::initiative::driver::is_enabled() {
                "on — watching for repeated commands".to_string()
            } else {
                "off (initiative_enabled=false)".to_string()
            }),
            _ => None,
        }
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

    /// Every known worker id must have exactly one descriptor. Catches a worker
    /// added without a co-located descriptor wired into [`WORKER_DESCRIPTORS`].
    const KNOWN_WORKER_IDS: &[&str] = &[
        "scheduler",
        "librarian",
        "git_steward",
        "initiative",
        "watcher",
        "onboarding_coach",
        // In the registry always (so `find_descriptor` resolves it); its render
        // into the brief is flag-gated (see `worker_descriptor_visible`).
        "playbook",
        // Same render-gated contract as the playbook (PERMAGENT_CONCIERGE_ENABLED).
        "concierge",
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
        "devices",
        "decision_inbox",
        "inbox",
        "timeline",
        "run_roster",
        "grow",
        "codebase",
        "coding_harness",
        "cost_optimizer",
        "mesh",
        "skills",
        "sessions",
        "trace",
        "meeting_dictation",
        "governance",
    ];
    /// The Phase-2-v1 lesson set — each must resolve to a descriptor with steps.
    const V1_LESSON_IDS: &[&str] = &["reader", "brain", "scheduler", "persona"];

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
            "sovereignty_guard",
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
            dispatchable_workers: Vec::new(),
        }
        .build();
        assert!(brief.contains("## Guardrails you operate under"));
        assert!(brief.contains("**Credential commit guard**"));
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

    #[test]
    fn confirm_checks_only_on_queryable_signals() {
        // Every authored confirm maps to a queryable read-back. Static surfaces
        // (Reader) are allowed a confirm only via the MemoryRecallable proxy.
        for id in V1_LESSON_IDS {
            let d = find_descriptor(id).unwrap();
            for step in d.teaching {
                if let Some(ConfirmCheck::MemoryRecallable(p)) = step.confirm {
                    assert!(
                        !p.is_empty(),
                        "{id}: MemoryRecallable phrase must be non-empty"
                    );
                }
            }
        }
    }

    #[test]
    fn brief_renders_every_category() {
        let brief = SelfKnowledgeBuilder {
            agent_display_name: "Aria".to_string(),
            scheduled_job_count: Some(3),
            dispatchable_workers: Vec::new(),
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

    /// B1 render-gate: the flag-gated Decision Playbook worker is HIDDEN from the
    /// brief when `PERMAGENT_PLAYBOOK_ENABLED` is off. This is why the canonical
    /// prompt_manager snapshots stay byte-for-byte unchanged — off is a no-op.
    /// (Pins config to an empty temp root, like the snapshot tests, so the flag
    /// is the only variable.)
    #[test]
    fn playbook_descriptor_hidden_when_flag_off() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().display().to_string();
        let _guard = env_lock::lock_env([
            ("HOME", Some(root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(root.as_str())),
            ("PERMAGENT_PLAYBOOK_ENABLED", None::<&str>),
        ]);

        let brief = SelfKnowledgeBuilder {
            agent_display_name: "Aria".to_string(),
            scheduled_job_count: None,
            dispatchable_workers: Vec::new(),
        }
        .build();

        assert!(
            !brief.contains("Decision Playbook"),
            "playbook descriptor must be hidden from the brief when the flag is off"
        );
    }

    /// B1 enabled rendering: when `PERMAGENT_PLAYBOOK_ENABLED` is on, the Decision
    /// Playbook worker renders in the brief with its hints-with-provenance
    /// framing — so the capability the agent can DO is exactly the one it can
    /// DESCRIBE. The dedicated guard the coordinator asked for.
    #[test]
    fn playbook_descriptor_shown_when_flag_on() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().display().to_string();
        let _guard = env_lock::lock_env([
            ("HOME", Some(root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(root.as_str())),
            ("PERMAGENT_PLAYBOOK_ENABLED", Some("1")),
        ]);

        let brief = SelfKnowledgeBuilder {
            agent_display_name: "Aria".to_string(),
            scheduled_job_count: None,
            dispatchable_workers: Vec::new(),
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
    /// character is HIDDEN from the brief when `PERMAGENT_CONCIERGE_ENABLED` is
    /// off. This is why the canonical prompt_manager snapshots stay byte-for-byte
    /// unchanged by this PR — off is a no-op.
    #[test]
    fn concierge_descriptor_hidden_when_flag_off() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().display().to_string();
        let _guard = env_lock::lock_env([
            ("HOME", Some(root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(root.as_str())),
            ("PERMAGENT_CONCIERGE_ENABLED", None::<&str>),
        ]);

        let brief = SelfKnowledgeBuilder {
            agent_display_name: "Aria".to_string(),
            scheduled_job_count: None,
            dispatchable_workers: Vec::new(),
        }
        .build();

        assert!(
            !brief.contains("The Concierge"),
            "concierge descriptor must be hidden from the brief when the flag is off"
        );
    }

    /// Concierge enabled rendering: when `PERMAGENT_CONCIERGE_ENABLED` is on, the
    /// character renders in the brief carrying its safe-by-construction framing
    /// (draft-only, read-only, local-tier) — so the capability the agent can DO is
    /// exactly the one it can DESCRIBE.
    #[test]
    fn concierge_descriptor_shown_when_flag_on() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().display().to_string();
        let _guard = env_lock::lock_env([
            ("HOME", Some(root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(root.as_str())),
            ("PERMAGENT_CONCIERGE_ENABLED", Some("1")),
        ]);

        let brief = SelfKnowledgeBuilder {
            agent_display_name: "Aria".to_string(),
            scheduled_job_count: None,
            dispatchable_workers: Vec::new(),
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
        let brief = SelfKnowledgeBuilder {
            agent_display_name: "Aria".to_string(),
            scheduled_job_count: None,
            dispatchable_workers: Vec::new(),
        }
        .build();
        // It appears once (under workers). Exactly one bold occurrence.
        assert_eq!(brief.matches("**Librarian**").count(), 1);
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
            dispatchable_workers: Vec::new(),
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
            dispatchable_workers: Vec::new(),
        }
        .build();
        assert!(!empty.contains("Workers you can dispatch goals to"));

        // Present → a section listing each worker + its status.
        let brief = SelfKnowledgeBuilder {
            agent_display_name: "Aria".to_string(),
            scheduled_job_count: None,
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
        }
        .build();
        assert!(brief.contains("## Workers you can dispatch goals to"));
        assert!(brief.contains("**Claude Code** — available"));
        assert!(brief.contains("**Librarian** — engine pending"));
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
            dispatchable_workers: Vec::new(),
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
    fn extension_tool_inventories() -> Vec<(&'static str, Vec<String>)> {
        use crate::agents::platform_extensions::{
            analyze, app_conductor, apps, browser, chatrecall, desktop, developer, ext_manager,
            file_to_project, listen, model_manager, orchestrator, people, project_manager,
            pronunciation, recipe_author, skills, storage_health, summarize, summon, todo,
        };

        fn names(tools: Vec<rmcp::model::Tool>) -> Vec<String> {
            tools.iter().map(|t| t.name.to_string()).collect()
        }

        let mut project_manager_tools = names(project_manager::ProjectManagerClient::get_tools());
        // Keep these review-gated research tools explicit in the self-knowledge
        // inventory: prompt-manager snapshots are regenerated by the reviewer.
        project_manager_tools.retain(|name| {
            !matches!(
                name.as_str(),
                "research_project_intel" | "propose_project_intel"
            )
        });
        project_manager_tools.extend([
            "research_project_intel".to_string(),
            "propose_project_intel".to_string(),
        ]);

        let mut inventories = vec![
            (
                analyze::EXTENSION_NAME,
                names(analyze::AnalyzeClient::get_tools()),
            ),
            (
                app_conductor::EXTENSION_NAME,
                names(app_conductor::AppConductorClient::get_tools()),
            ),
            (
                apps::EXTENSION_NAME,
                names(apps::AppsManagerClient::get_tools()),
            ),
            (
                browser::EXTENSION_NAME,
                names(browser::BrowserClient::get_tools()),
            ),
            (
                chatrecall::EXTENSION_NAME,
                names(chatrecall::ChatRecallClient::get_tools()),
            ),
            (
                // Flag-gated at runtime (DESKTOP_CONTROL_ENABLED): `list_tools`
                // selects from this superset, so the superset is the inventory
                // (the Extension Manager precedent).
                desktop::EXTENSION_NAME,
                names(desktop::DesktopClient::get_tools()),
            ),
            (
                developer::EXTENSION_NAME,
                names(developer::DeveloperClient::get_tools()),
            ),
            (
                ext_manager::EXTENSION_NAME,
                names(ext_manager::ExtensionManagerClient::all_possible_tools()),
            ),
            (
                file_to_project::EXTENSION_NAME,
                names(file_to_project::FileToProjectClient::get_tools()),
            ),
            (
                listen::EXTENSION_NAME,
                names(listen::ListenClient::get_tools()),
            ),
            (
                model_manager::EXTENSION_NAME,
                names(model_manager::ModelManagerClient::get_tools()),
            ),
            (
                orchestrator::EXTENSION_NAME,
                names(orchestrator::OrchestratorClient::get_tools()),
            ),
            (
                people::EXTENSION_NAME,
                names(people::PeopleClient::get_tools()),
            ),
            (project_manager::EXTENSION_NAME, project_manager_tools),
            (
                pronunciation::EXTENSION_NAME,
                names(pronunciation::PronunciationClient::get_tools()),
            ),
            (
                recipe_author::EXTENSION_NAME,
                names(recipe_author::RecipeAuthorClient::get_tools()),
            ),
            (
                skills::EXTENSION_NAME,
                names(skills::SkillsClient::get_tools()),
            ),
            (
                storage_health::EXTENSION_NAME,
                names(storage_health::StorageHealthClient::get_tools()),
            ),
            (
                summarize::EXTENSION_NAME,
                names(summarize::SummarizeClient::get_tools()),
            ),
            (
                summon::EXTENSION_NAME,
                names(summon::SummonClient::all_possible_tools()),
            ),
            (todo::EXTENSION_NAME, names(todo::TodoClient::get_tools())),
        ];

        #[cfg(feature = "code-mode")]
        inventories.push((
            crate::agents::platform_extensions::code_execution::EXTENSION_NAME,
            names(crate::agents::platform_extensions::code_execution::CodeExecutionClient::all_possible_tools()),
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
            for missing in tools_not_named_in(desc, tools) {
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
            vec!["navigate_app", "app_action", "open_item"],
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
    const NON_TOOL_PROSE_TOKENS: &[&str] = &["sub_recipes", "worker_persona"];

    /// Every tool name that exists in the runtime: the statically-derived
    /// per-extension inventories, hidden-but-real extensions (Git Steward),
    /// the two platform tools registered directly in `agent.rs`, and the
    /// documented dynamic extras.
    fn real_tool_inventory() -> std::collections::HashSet<String> {
        let mut inv: std::collections::HashSet<String> = extension_tool_inventories()
            .into_iter()
            .flat_map(|(_, tools)| tools)
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
