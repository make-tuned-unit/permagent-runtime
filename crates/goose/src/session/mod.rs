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
