mod chat_history_search;
pub mod crash_capture;
mod diagnostics;
pub mod extension_data;
pub mod session_manager;
pub mod spectral_schema;
pub mod thread_manager;

pub use diagnostics::{
    config_path, generate_diagnostics, get_system_info, latest_llm_log_path,
    latest_server_log_path, read_capped, read_tail, SystemInfo,
};
pub use extension_data::{EnabledExtensionsState, ExtensionData, ExtensionState, TodoState};
pub use session_manager::{
    CostLedgerRow, CostTier, Session, SessionInsights, SessionManager, SessionSummary, SessionType,
    SessionUpdateBuilder,
};
pub use thread_manager::{Thread, ThreadManager, ThreadMetadata};

/// Self-knowledge descriptor for the Session history surface. Lets the agent
/// tell the user where past conversations live and how to return to one. Static:
/// editorial, no live status claim. Reachable as an overlay (no seeded workspace
/// hosts it) — the agent opens it via `navigate_app("Sessions")`.
pub const SESSIONS_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "sessions",
        display_name: "Session history",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "The list of the user's past conversations with you — each session shown with its title and when it was last active, so the user can browse their history, reopen an earlier conversation to pick up where they left off (it loads back into the chat dock), or rename and delete old ones",
        why_it_matters:
            "It is how the user returns to and manages earlier conversations; when they ask to continue something you discussed before, find a past chat, or clear out old sessions, bring them here",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[],
    };
