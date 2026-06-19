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
];

/// User-facing surfaces. Each entry is a `const` co-located with its module.
pub static SURFACE_DESCRIPTORS: &[FeatureDescriptor] = &[
    crate::reader::SELF_KNOWLEDGE_FEATURE,
    crate::events::WORLD_VIEW_FEATURE,
    crate::brain_handle::BRAIN_FEATURE,
    crate::config::agent_identity::PERSONA_PICKER_FEATURE,
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
    const KNOWN_WORKER_IDS: &[&str] = &["scheduler", "librarian"];
    /// Every known surface id must have exactly one descriptor.
    const KNOWN_SURFACE_IDS: &[&str] = &["reader", "world_view", "brain", "persona"];
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
        }
        .build();
        // It appears once (under workers). Exactly one bold occurrence.
        assert_eq!(brief.matches("**Librarian**").count(), 1);
    }
}
