pub mod budget_projection;
pub mod chat_history_search;
pub mod crash_capture;
mod diagnostics;
pub mod extension_data;
pub mod session_manager;
pub mod spectral_schema;
pub mod thread_manager;

pub use budget_projection::{
    BillingEvidence, BudgetProjection, BudgetScopeProjection, CapTriplet, ProjectionBand,
    ProjectionCompleteness, ProjectionError, ProjectionProvenance, BUDGET_PROJECTION_VERSION,
};
pub use diagnostics::{
    config_path, generate_diagnostics, get_system_info, latest_llm_log_path,
    latest_server_log_path, read_capped, read_tail, SystemInfo,
};
pub use extension_data::{EnabledExtensionsState, ExtensionData, ExtensionState, TodoState};
pub use session_manager::{
    budget_task_id, goal_id, ChildSessionCost, CostLedgerRow, CostReservationOutcome, CostTier,
    LastCall, ParentSessionCost, ScheduleSessionSummary, Session, SessionInsights, SessionManager,
    SessionSummary, SessionType, SessionUpdateBuilder, BUDGET_TASK_EXTENSION_NAME,
    BUDGET_TASK_EXTENSION_VERSION, GOAL_ID_EXTENSION_NAME, GOAL_ID_EXTENSION_VERSION,
};
pub use thread_manager::{Thread, ThreadManager, ThreadMetadata};

/// Inherited only by a child CLI process launched for an existing session.
/// The value is always an existing durable session id; callers must leave it
/// unset for top-level runs rather than minting a synthetic parent.
pub const PARENT_SESSION_ID_ENV: &str = "PERMAGENT_PARENT_SESSION_ID";

/// Inherited by an external worker CLI launched for a goal. The value is a
/// real, already-persisted worker session id created by the daemon; child
/// Permagent invocations reuse it instead of creating an untracked session.
pub const WORKER_SESSION_ID_ENV: &str = "PERMAGENT_WORKER_SESSION_ID";

/// The card/goal identity associated with an external worker invocation. This
/// is explicit dispatch metadata, not something a child should infer by
/// parsing its prompt.
pub const GOAL_ID_ENV: &str = "PERMAGENT_GOAL_ID";

/// Budget identity inherited by a worker. The child must reuse this value;
/// it must not mint a second task boundary for the same parent turn.
pub const BUDGET_TASK_ID_ENV: &str = "PERMAGENT_BUDGET_TASK_ID";

/// Self-knowledge descriptor for the Session history surface. Lets the agent
/// tell the user where past conversations live and how to return to one. Static:
/// editorial, no live status claim. Lives as the Sessions page inside Settings
/// (2026-08 Console consolidation) — the agent opens it via
/// `navigate_app("Sessions")`, which deep-links to Settings → Sessions.
pub const SESSIONS_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "sessions",
        display_name: "Session history",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does: "The list of the user's past conversations with you — the Sessions page in Settings, each session shown with its title and when it was last active, so the user can browse their history, reopen an earlier conversation to pick up where they left off (it loads back into the chat dock), or rename and delete old ones",
        why_it_matters: "It is how the user returns to and manages earlier conversations; when they ask to continue something you discussed before, find a past chat, or clear out old sessions, bring them here",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[],
    };
