//! Guided-tour tool handler (`platform__load_feature_lesson`).
//!
//! Returns one feature's lesson on demand so the per-turn prompt stays lean,
//! and marks the tour as engaged (stopping the first-run offer). The
//! orchestration — offer → walk → navigate_app → confirm → adapt — lives in the
//! `tour` builtin skill; the lesson content lives in the co-located
//! `FeatureDescriptor::teaching` steps.

use rmcp::model::Content;

use super::Agent;
use crate::agents::self_knowledge::{lesson_for, TOUR_COMPLETED_KEY};
use crate::config::Config;
use crate::mcp_utils::ToolResult;

impl Agent {
    /// Handle a `load_feature_lesson` tool call. Sync — reads process-global
    /// descriptor data and the config flag; no async work.
    pub fn handle_load_feature_lesson(&self, feature_id: &str) -> ToolResult<Vec<Content>> {
        // Engaging the tour — or explicitly declining it — stops the one-time
        // first-run offer. Best-effort: a failed write only means the offer may
        // reappear, never a hard error.
        if let Err(e) = Config::global().set_param(TOUR_COMPLETED_KEY, true) {
            tracing::warn!("tour: failed to set {TOUR_COMPLETED_KEY}: {e}");
        }

        if feature_id == "decline" {
            return Ok(vec![Content::text(
                "Tour offer dismissed — I won't bring it up again unless you ask.",
            )]);
        }

        match lesson_for(feature_id) {
            Some(text) => Ok(vec![Content::text(text)]),
            None => Ok(vec![Content::text(format!(
                "No lesson for \"{feature_id}\". Available lessons: reader, brain, scheduler, persona."
            ))]),
        }
    }
}
