use indoc::indoc;
use rmcp::model::{Tool, ToolAnnotations};
use rmcp::object;
pub const PLATFORM_MANAGE_SCHEDULE_TOOL_NAME: &str = "platform__manage_schedule";

pub const PLATFORM_LOAD_FEATURE_LESSON_TOOL_NAME: &str = "platform__load_feature_lesson";

/// Tour teaching tool: fetch one feature's step-by-step lesson on demand (so the
/// per-turn prompt stays lean). Also marks the tour as engaged so the first-run
/// offer stops. See the `tour` builtin skill for how to run the loop.
pub fn load_feature_lesson_tool() -> Tool {
    Tool::new(
        PLATFORM_LOAD_FEATURE_LESSON_TOOL_NAME.to_string(),
        indoc! {r#"
            Load the guided-tour lesson for one capability, as step-by-step
            instructions you deliver conversationally (open the surface via
            navigate_app, then confirm the user acted before moving on).

            Pass `feature_id` = one of: "reader", "brain", "scheduler", "persona".
            Pass `feature_id` = "decline" if the user does NOT want a tour — this
            stops future tour offers and returns no lesson.

            Calling this tool marks the tour as engaged, so the one-time first-run
            offer will not appear again.
        "#}
        .to_string(),
        object!({
            "type": "object",
            "required": ["feature_id"],
            "properties": {
                "feature_id": {
                    "type": "string",
                    "description": "Feature to teach (reader|brain|scheduler|persona), or \"decline\" to stop tour offers."
                }
            }
        }),
    )
    .annotate(
        ToolAnnotations::with_title("Load tour lesson".to_string())
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
    )
}

pub fn manage_schedule_tool() -> Tool {
    Tool::new(
        PLATFORM_MANAGE_SCHEDULE_TOOL_NAME.to_string(),
        indoc! {r#"
            Manage goose's internal scheduled recipe execution.

            Actions:
            - "list": List all goose scheduled jobs
            - "create": Create a new goose scheduled job from a recipe file
            - "run_now": Execute a goose scheduled job immediately
            - "pause": Pause a goose scheduled job
            - "unpause": Resume a paused goose scheduled job
            - "delete": Remove a goose scheduled job
            - "kill": Terminate a currently running goose scheduled job
            - "inspect": Get details about a running goose scheduled job
            - "sessions": List execution history for a goose scheduled job
            - "session_content": Get the full content (messages) of a specific session
        "#}
        .to_string(),
        object!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "create", "run_now", "pause", "unpause", "delete", "kill", "inspect", "sessions", "session_content"]
                },
                "job_id": {"type": "string", "description": "Job identifier for operations on existing jobs"},
                "recipe_path": {"type": "string", "description": "Path to recipe file for create action"},
                "cron_expression": {"type": "string", "description": "A cron expression for create action. Supports both 5-field (minute hour day month weekday) and 6-field (second minute hour day month weekday) formats. 5-field expressions are automatically converted to 6-field by prepending '0' for seconds."},
                "limit": {"type": "integer", "description": "Limit for sessions list", "default": 50},
                "session_id": {"type": "string", "description": "Session identifier for session_content action"}
            }
        }),
    ).annotate(ToolAnnotations::with_title("Manage scheduled recipes".to_string()).read_only(false).destructive(true).idempotent(false).open_world(false))
}
