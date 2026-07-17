use indoc::indoc;
use rmcp::model::{Tool, ToolAnnotations};
use rmcp::object;
pub const PLATFORM_MANAGE_SCHEDULE_TOOL_NAME: &str = "platform__manage_schedule";

pub const PLATFORM_LOAD_FEATURE_LESSON_TOOL_NAME: &str = "platform__load_feature_lesson";

/// Teaching tool: fetch one capability's lesson on demand (so the per-turn
/// prompt stays lean), open its real surface, and mark it taught. Use it to
/// onboard the user into anything they haven't tried — whether they ask
/// ("teach me something new", "what haven't I used?") or you offer. Marks the
/// tour engaged so the first-run offer stops. See the `tour` builtin skill for
/// how to run a multi-feature loop.
pub fn load_feature_lesson_tool() -> Tool {
    Tool::new(
        PLATFORM_LOAD_FEATURE_LESSON_TOOL_NAME.to_string(),
        indoc! {r#"
            Load the teaching lesson for one capability, as step-by-step
            instructions you deliver conversationally: explain what it does and
            why it matters, open its surface via navigate_app when the lesson
            says to, then confirm the user acted before moving on. Calling this
            marks the capability as taught, so it drops off the user's
            "haven't tried yet" list.

            Pass `feature_id` = a capability id you want to teach. The classic
            first-run set is "reader", "brain", "scheduler", "persona", but any
            capability from your self-knowledge inventory works (e.g. "projects",
            "build", "decision_inbox", "voice", "web_search", "devices",
            "run_roster", "world_view"). If the id isn't teachable, the tool
            returns the list of teachable capabilities.

            Pass `feature_id` = "decline" if the user does NOT want a tour — this
            stops future tour offers and returns no lesson.

            Calling this tool also marks the tour as engaged, so the one-time
            first-run offer will not appear again.
        "#}
        .to_string(),
        object!({
            "type": "object",
            "required": ["feature_id"],
            "properties": {
                "feature_id": {
                    "type": "string",
                    "description": "Capability id to teach (e.g. reader|brain|scheduler|persona|projects|build|decision_inbox|voice|web_search|devices|run_roster|world_view), or \"decline\" to stop tour offers."
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
