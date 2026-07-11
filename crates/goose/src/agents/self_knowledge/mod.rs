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
];

/// Deterministic guardrails the agent operates under. Co-located with the
/// safety-core module that enforces each one.
pub static GUARD_DESCRIPTORS: &[FeatureDescriptor] = &[
    crate::steward::secret_scan::SELF_KNOWLEDGE_FEATURE,
    crate::session::crash_capture::DURABILITY_FEATURE,
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
                Some(format!(
                    "{} described, {} pending",
                    s.lifetime_stats.described, s.lifetime_stats.pending
                ))
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
    const KNOWN_WORKER_IDS: &[&str] = &["scheduler", "librarian", "git_steward", "initiative"];
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
        const KNOWN_GUARD_IDS: &[&str] = &["credential_commit_guard", "durability_supervision"];
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

    /// **Branding guard (systems fix).** No user-facing capability string may
    /// leak the upstream `goose` fork name. Every field that renders into the
    /// self-knowledge brief and the capability cards — `display_name`,
    /// `what_it_does`, `why_it_matters` — is scanned case-insensitively across
    /// all descriptor registries (platform extensions, workers, guards,
    /// surfaces). A re-introduced "goose" in card copy or the brief fails the
    /// build instead of shipping.
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
        assert!(
            leaks.is_empty(),
            "user-facing 'goose' branding leak(s) found — rebrand to Permagent \
             (or allowlist if genuinely internal):\n{}",
            leaks.join("\n")
        );
    }
}
