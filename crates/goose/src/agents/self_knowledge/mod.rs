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

/// A single teaching step — the Phase-2 extension point. Empty (`&[]`) in
/// Phase 1; Phase 2 will populate these to teach the agent how to drive a
/// feature, without changing the descriptor contract.
#[derive(Debug, Clone, Copy)]
pub struct TeachingStep {
    pub title: &'static str,
    pub body: &'static str,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every known worker id must have exactly one descriptor. Catches a worker
    /// added without a co-located descriptor wired into [`WORKER_DESCRIPTORS`].
    const KNOWN_WORKER_IDS: &[&str] = &["scheduler", "librarian"];
    /// Every known surface id must have exactly one descriptor.
    const KNOWN_SURFACE_IDS: &[&str] = &["reader", "world_view"];

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
