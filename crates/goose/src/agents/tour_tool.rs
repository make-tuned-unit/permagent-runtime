//! Guided-tour / teaching tool handler (`platform__load_feature_lesson`).
//!
//! Returns one capability's lesson on demand so the per-turn prompt stays lean,
//! marks the tour as engaged (stopping the first-run offer), and marks the
//! capability *taught* so it drops off the learn-next list. This is the core
//! agent-led-onboarding action: the agent teaches a real feature from the
//! authoritative self-knowledge inventory, opens its real surface via
//! `navigate_app`, and records that the user has now seen it.
//!
//! The lesson *content* is the co-located `FeatureDescriptor` (authored
//! `teaching` steps, or its `what_it_does`/`why_it_matters`); the navigable
//! *surface* comes from the [`teachable`](crate::agents::self_knowledge::teachable)
//! curriculum. Both are real by construction — see the tests there.

use rmcp::model::Content;

use super::Agent;
use crate::agents::self_knowledge::teachable::{find_teachable, teachable_ids};
use crate::agents::self_knowledge::usage::mark_taught;
use crate::agents::self_knowledge::{find_descriptor, lesson_for, TOUR_COMPLETED_KEY};
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

        // Only teach real capabilities. An unknown id lists the teachable set
        // rather than inventing a lesson.
        let Some(descriptor) = find_descriptor(feature_id) else {
            return Ok(vec![Content::text(format!(
                "No lesson for \"{feature_id}\". Teachable capabilities: {}. \
                 Pass \"decline\" to stop tour offers.",
                teachable_ids().join(", ")
            ))]);
        };

        // Real lesson text from the descriptor (authored steps, or a "explain it
        // in your own words" note built from its description).
        let mut text = lesson_for(feature_id).unwrap_or_default();

        // For a navigable capability whose descriptor has no authored
        // open_surface step, append the real navigate target so the agent still
        // opens a mounted surface. (Authored lessons already embed their own
        // navigate step, so we don't duplicate it.)
        if let Some(t) = find_teachable(feature_id) {
            let has_authored_surface = descriptor.teaching.iter().any(|s| s.open_surface.is_some());
            if !has_authored_surface {
                match t.surface.section {
                    Some(sec) => text.push_str(&format!(
                        "\n→ Open it for them: call `navigate_app` with tab \"{}\", section \
                         \"{}\".\n",
                        t.surface.tab, sec
                    )),
                    None => text.push_str(&format!(
                        "\n→ Open it for them: call `navigate_app` with tab \"{}\".\n",
                        t.surface.tab
                    )),
                }
            }
        }

        // Record that this capability has now been taught, so the learn-next
        // computation stops recommending it.
        mark_taught(feature_id);

        Ok(vec![Content::text(text)])
    }
}
