//! Harness Watch: chat tools that live-query coding-harness runs in the
//! user's terminals, so the user can ask for updates on that work from chat.
//!
//! Three read-only tools over [`crate::harness_runs`]:
//! - `harness_runs` lists live, lost and just-finished runs, one line each.
//! - `harness_run_status` drills into one run's goals and recent activity.
//! - `harness_attention` gathers what across those runs is waiting on the user.
//!
//! Every tool reads the session pool of THIS process — the database under
//! its own `Paths` root — and nothing else: no network, no model calls, no
//! message text, no file contents. Nothing here requeues, dispatches or
//! reclaims a goal, so calling it while a terminal is mid-run is safe.
//!
//! Registered default-on for chat. The coding-harness recipe declares its
//! extensions explicitly and does not list this one, so harness sessions do
//! not carry it.

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use crate::harness_runs::{
    self, human_duration, AttentionKind, AttentionReport, GoalStatusRow, RunChoice, RunSelection,
    RunStatus, RunSummary,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool,
};
use sqlx::{Pool, Sqlite};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "harness_watch";

pub const RUNS_TOOL: &str = "harness_runs";
pub const RUN_STATUS_TOOL: &str = "harness_run_status";
pub const ATTENTION_TOOL: &str = "harness_attention";

pub struct HarnessWatchClient {
    info: InitializeResult,
    context: PlatformExtensionContext,
}

impl HarnessWatchClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self, anyhow::Error> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME.to_string(), "1.0.0".to_string())
                    .with_title("Harness Watch"),
            );
        Ok(Self { info, context })
    }

    pub(crate) fn get_tools() -> Vec<Tool> {
        let empty: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object", "properties": {}, "required": []
        }))
        .expect("static schema");
        let run_schema: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "run": {
                    "type": "string",
                    "description": "Which run: its run handle or lease id, the coordinator session id, the session name, or a fragment of the repo or working directory (e.g. 'alpha' or 'my-worktree'). Omit when exactly one run is live."
                }
            },
            "required": []
        }))
        .expect("static schema");

        vec![
            Tool::new(
                RUNS_TOOL.to_string(),
                "List the coding-harness runs in the user's terminals: every live run, runs whose \
                 terminal stopped heartbeating (LOST — probably crashed or was killed), and runs \
                 that finished in the last 30 minutes. One line each: repo and worktree, phase, \
                 age, goals done/total, how many goals are waiting on the user, and spend so far \
                 including workers. Use this first when the user asks how the work in their \
                 terminals is going. Read-only; never changes a goal."
                    .to_string(),
                empty.clone(),
            ),
            Tool::new(
                RUN_STATUS_TOOL.to_string(),
                "Detailed status of one terminal harness run: each goal's state, attempt, worker, \
                 worker heartbeat age, last tool the worker called (name only), last error, \
                 verification verdict, pending gate or review, and landed commit — plus the run's \
                 recent activity. Pass `run` (handle, lease id, session id, session name, or a \
                 repo/cwd fragment); omit it when exactly one run is live. When the choice is \
                 ambiguous it returns the candidate runs instead. Read-only; never requeues or \
                 re-dispatches a goal, so it is safe while the terminal is mid-run."
                    .to_string(),
                run_schema,
            ),
            Tool::new(
                ATTENTION_TOOL.to_string(),
                "What the user's terminal harness runs are waiting on the user for: open \
                 decisions on the runs' goals and projects, goals parked for a human, pending \
                 approval gates, and terminals that exited unexpectedly while goals were still in \
                 progress. Use when the user asks what needs them or whether anything is stuck. \
                 Read-only."
                    .to_string(),
                empty,
            ),
        ]
    }

    async fn pool(&self) -> Result<Pool<Sqlite>, String> {
        self.context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| format!("Could not open the local session database: {e}"))
    }
}

// ── Handlers (pool-in, result-out, so tests run against a seeded pool) ──

fn structured(text: String, payload: serde_json::Value) -> CallToolResult {
    let mut result = CallToolResult::success(vec![Content::text(text)]);
    result.structured_content = Some(payload);
    result
}

fn money(usd: f64) -> String {
    format!("${usd:.2}")
}

fn ago_text(secs: Option<i64>) -> String {
    secs.map(|s| format!("{} ago", human_duration(s)))
        .unwrap_or_else(|| "at an unknown time".to_string())
}

fn run_line(run: &RunSummary) -> String {
    let mut parts = vec![
        format!("[{}] {}", run.liveness.label(), run.repo),
        run.phase.clone(),
    ];
    if let Some(age) = run.age_secs {
        parts.push(format!("up {}", human_duration(age)));
    }
    parts.push(format!("heartbeat {}", ago_text(run.heartbeat_age_secs)));
    parts.push(format!("goals {}/{} done", run.goals.done, run.goals.total));
    if run.goals.in_progress > 0 {
        parts.push(format!("{} in progress", run.goals.in_progress));
    }
    if run.goals.waiting_on_you > 0 {
        parts.push(format!("{} waiting on you", run.goals.waiting_on_you));
    }
    parts.push(money(run.spend_usd));
    if let Some(status) = &run.exit_status {
        parts.push(format!("exit {status}"));
    }
    parts.push(format!("run {}", run.run));
    let mut line = format!("- {}", parts.join(" · "));
    if let Some(title) = &run.title {
        line.push_str(&format!(" · \"{title}\""));
    }
    line
}

/// Render the listing.
pub fn render_runs(runs: &[RunSummary]) -> String {
    if runs.is_empty() {
        return "No coding-harness runs are live in the user's terminals, and none finished in \
                the last 30 minutes."
            .to_string();
    }
    let count = |l: harness_runs::RunLiveness| runs.iter().filter(|r| r.liveness == l).count();
    let mut lines = vec![format!(
        "Terminal harness runs: {} live, {} lost, {} recently ended.",
        count(harness_runs::RunLiveness::Live),
        count(harness_runs::RunLiveness::Lost),
        count(harness_runs::RunLiveness::Ended),
    )];
    lines.extend(runs.iter().map(run_line));
    lines.push(format!(
        "Call {RUN_STATUS_TOOL} with a run handle for per-goal detail, or {ATTENTION_TOOL} for \
         what is waiting on the user."
    ));
    lines.join("\n")
}

fn goal_line(goal: &GoalStatusRow) -> String {
    let mut parts = vec![format!("[{}] {}", goal.state, goal.title)];
    if goal.attempt_count > 0 {
        parts.push(format!("attempt {}", goal.attempt_count));
    }
    if let Some(worker) = goal
        .worker_key
        .as_ref()
        .or(goal.worker_session_name.as_ref())
    {
        parts.push(format!("worker {worker}"));
    }
    if let Some(state) = &goal.receipt_state {
        parts.push(format!(
            "receipt {state}, beat {}",
            ago_text(goal.receipt_heartbeat_age_secs)
        ));
    }
    if let Some(tool) = &goal.last_tool {
        parts.push(format!(
            "last tool {tool} {}",
            ago_text(goal.last_tool_age_secs)
        ));
    }
    if let Some(verdict) = &goal.verify_verdict {
        parts.push(format!("verify {verdict}"));
    }
    if let Some(review) = &goal.review_state {
        parts.push(match review.as_str() {
            "awaiting_approval" => "awaiting your approval".to_string(),
            other => other.replace('_', " "),
        });
    }
    if let Some(sha) = &goal.landed_commit {
        parts.push(format!("landed {sha}"));
    } else if let Some(outcome) = &goal.landing_outcome {
        parts.push(format!("landing {outcome}"));
    }
    let mut line = format!("- {}", parts.join(" · "));
    if goal.needs_human_attention {
        line.push_str(&format!(
            "\n    ! parked for you{}",
            goal.attention_reason
                .as_deref()
                .map(|r| format!(": {r}"))
                .unwrap_or_default()
        ));
    }
    if let Some(gate) = &goal.pending_gate {
        line.push_str(&format!("\n    ! gate {gate}"));
    }
    if let Some(error) = &goal.last_error {
        line.push_str(&format!("\n    last error: {error}"));
    }
    if goal.abandoned_work {
        line.push_str("\n    ! work was abandoned on this goal");
    }
    line
}

/// Render one run's status.
pub fn render_status(status: &RunStatus) -> String {
    let run = &status.run;
    let mut lines = vec![
        format!(
            "Run {} · {} · {} · phase {} · pid {} · {} mode · heartbeat {}",
            run.run,
            run.repo,
            run.liveness.label(),
            run.phase,
            run.pid,
            run.mode,
            ago_text(run.heartbeat_age_secs)
        ),
        format!(
            "cwd {} · session {}{} · spend {} · {} open decision{}",
            run.working_dir,
            run.session_id,
            run.title
                .as_deref()
                .map(|t| format!(" (\"{t}\")"))
                .unwrap_or_default(),
            money(run.spend_usd),
            run.open_decisions,
            if run.open_decisions == 1 { "" } else { "s" }
        ),
    ];
    if let Some(activity) = &run.last_activity {
        lines.push(format!("Coordinator last activity: {activity}"));
    }
    let by_state: Vec<String> = run
        .goals
        .by_state
        .iter()
        .map(|(state, n)| format!("{state} {n}"))
        .collect();
    lines.push(format!(
        "Goals ({}{}){}:",
        run.goals.total,
        if by_state.is_empty() {
            String::new()
        } else {
            format!("; {}", by_state.join(", "))
        },
        if status.goals_truncated {
            format!(", newest {} shown", status.goals.len())
        } else {
            String::new()
        }
    ));
    if status.goals.is_empty() {
        lines.push("- none yet".to_string());
    }
    lines.extend(status.goals.iter().map(goal_line));
    if let Some(t) = &status.telemetry {
        let mut parts = Vec::new();
        if let Some(calls) = t.tool_calls {
            parts.push(format!("{calls} tool calls"));
        }
        if let Some(verdict) = &t.verification_verdict {
            parts.push(format!("verification {verdict}"));
        }
        if let Some(gate) = &t.pending_gate_tool {
            parts.push(format!("pending gate: {gate}"));
        }
        if !parts.is_empty() {
            lines.push(format!("Coordinator telemetry: {}", parts.join(" · ")));
        }
    }
    if !status.journal.is_empty() {
        lines.push("Recent activity:".to_string());
        for row in &status.journal {
            lines.push(format!(
                "- {} · {} · {} ({})",
                ago_text(row.age_secs),
                row.kind,
                row.title,
                row.actor
            ));
        }
    }
    lines.join("\n")
}

fn choice_line(choice: &RunChoice) -> String {
    format!(
        "- run {} · [{}] {} · {} · heartbeat {}{}",
        choice.run,
        choice.liveness.label(),
        choice.repo,
        choice.phase,
        ago_text(choice.heartbeat_age_secs),
        choice
            .title
            .as_deref()
            .map(|t| format!(" · \"{t}\""))
            .unwrap_or_default()
    )
}

/// Render a selection outcome.
pub fn render_selection(selection: &RunSelection) -> String {
    match selection {
        RunSelection::Resolved(status) => render_status(status),
        RunSelection::NoRuns => {
            "No coding-harness runs are live or recent in the user's terminals.".to_string()
        }
        RunSelection::Choose { reason, choices } => {
            let mut lines = vec![reason.clone()];
            lines.extend(choices.iter().map(choice_line));
            lines.push(format!(
                "Call {RUN_STATUS_TOOL} again with `run` set to one of these handles."
            ));
            lines.join("\n")
        }
    }
}

/// Render the attention report.
pub fn render_attention(report: &AttentionReport) -> String {
    if report.runs_considered == 0 {
        return "No coding-harness runs are live or recent, so nothing from the terminals is \
                waiting on the user."
            .to_string();
    }
    if report.items.is_empty() {
        return format!(
            "Nothing from the {} terminal harness run{} is waiting on the user.",
            report.runs_considered,
            if report.runs_considered == 1 { "" } else { "s" }
        );
    }
    let mut lines = vec![format!(
        "{} item{} from terminal harness runs need the user{}:",
        report.items.len(),
        if report.items.len() == 1 { "" } else { "s" },
        if report.truncated {
            " (list capped)"
        } else {
            ""
        }
    )];
    for item in &report.items {
        let label = match item.kind {
            AttentionKind::Decision => format!(
                "decision {}",
                item.decision_kind.as_deref().unwrap_or("open")
            ),
            AttentionKind::PendingGate => "gate".to_string(),
            AttentionKind::ParkedGoal => "parked goal".to_string(),
            AttentionKind::ProjectDecision => format!(
                "project decision {}",
                item.decision_kind.as_deref().unwrap_or("open")
            ),
            AttentionKind::LostRun => "lost terminal".to_string(),
        };
        let goal = item
            .goal_title
            .as_deref()
            .map(|t| format!(" · goal \"{t}\""))
            .unwrap_or_default();
        lines.push(format!(
            "- [{label}] {}{goal} · run {} ({}, {})",
            item.summary,
            item.run,
            item.repo,
            item.liveness.label()
        ));
    }
    lines.join("\n")
}

fn to_payload<T: serde::Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
}

pub async fn handle_runs(
    pool: &Pool<Sqlite>,
    now: DateTime<Utc>,
) -> Result<CallToolResult, String> {
    let runs = harness_runs::list_runs(pool, now, harness_runs::DEFAULT_RECENT_ENDED).await?;
    Ok(structured(
        render_runs(&runs),
        serde_json::json!({ "runs": to_payload(&runs) }),
    ))
}

pub async fn handle_run_status(
    pool: &Pool<Sqlite>,
    run: Option<&str>,
    now: DateTime<Utc>,
) -> Result<CallToolResult, String> {
    let selection = harness_runs::run_status(pool, run, now).await?;
    Ok(structured(
        render_selection(&selection),
        to_payload(&selection),
    ))
}

pub async fn handle_attention(
    pool: &Pool<Sqlite>,
    now: DateTime<Utc>,
) -> Result<CallToolResult, String> {
    let report = harness_runs::attention(pool, now).await?;
    Ok(structured(render_attention(&report), to_payload(&report)))
}

#[async_trait]
impl McpClientTrait for HarnessWatchClient {
    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        Ok(ListToolsResult {
            tools: Self::get_tools(),
            next_cursor: None,
            meta: None,
        })
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }

    async fn call_tool(
        &self,
        _ctx: &ToolCallContext,
        name: &str,
        arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        let now = Utc::now();
        let result = match self.pool().await {
            Err(e) => Err(e),
            Ok(pool) => match name {
                RUNS_TOOL => handle_runs(&pool, now).await,
                RUN_STATUS_TOOL => {
                    let run = arguments
                        .as_ref()
                        .and_then(|a| a.get("run"))
                        .and_then(|v| v.as_str());
                    handle_run_status(&pool, run, now).await
                }
                ATTENTION_TOOL => handle_attention(&pool, now).await,
                _ => Err(format!("Unknown tool: {name}")),
            },
        };
        Ok(result.unwrap_or_else(|e| CallToolResult::error(vec![Content::text(e)])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness_runs::test_support::*;

    fn text_of(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn assert_private(result: &CallToolResult) {
        let text = text_of(result);
        assert!(!text.contains(SENTINEL), "text leaked: {text}");
        let payload = serde_json::to_string(&result.structured_content).unwrap();
        assert!(
            !payload.contains(SENTINEL),
            "structured content leaked: {payload}"
        );
    }

    #[test]
    fn the_extension_ships_exactly_the_three_named_tools() {
        let names: Vec<String> = HarnessWatchClient::get_tools()
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        assert_eq!(names, vec![RUNS_TOOL, RUN_STATUS_TOOL, ATTENTION_TOOL]);
    }

    /// The self-knowledge naming contract, checked locally too: the registry
    /// description must name every callable tool or the agent never learns
    /// they exist.
    #[test]
    fn the_registry_description_names_every_tool() {
        let def = crate::agents::platform_extensions::PLATFORM_EXTENSIONS
            .get(EXTENSION_NAME)
            .expect("harness_watch is registered");
        assert!(def.default_enabled, "on by default for chat");
        assert!(!def.hidden);
        let tokens: std::collections::HashSet<&str> = def
            .description
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .collect();
        for tool in HarnessWatchClient::get_tools() {
            assert!(
                tokens.contains(&*tool.name),
                "description never names {}",
                tool.name
            );
        }
    }

    #[test]
    fn the_coding_recipe_does_not_carry_the_extension() {
        let recipe =
            include_str!("../../../../goose-cli/src/recipes/builtin/permagent-coding.yaml");
        assert!(
            !recipe.contains(EXTENSION_NAME),
            "harness sessions must not load the chat-side watch tools"
        );
    }

    #[tokio::test]
    async fn harness_runs_lists_both_runs_without_leaking_message_text() {
        let pool = seeded_pool().await;
        let result = handle_runs(&pool, now()).await.unwrap();
        let text = text_of(&result);
        assert!(text.contains("1 live, 1 lost, 0 recently ended"), "{text}");
        assert!(text.contains("[live] alpha (alpha-wt)"), "{text}");
        assert!(text.contains("[LOST] beta"), "{text}");
        assert!(text.contains("goals 1/4 done"), "{text}");
        assert!(text.contains("2 waiting on you"), "{text}");
        assert!(text.contains("$0.80"), "{text}");
        let runs = result.structured_content.as_ref().unwrap()["runs"]
            .as_array()
            .unwrap()
            .len();
        assert_eq!(runs, 2);
        assert_private(&result);
    }

    #[tokio::test]
    async fn harness_run_status_resolves_zero_one_and_many_live_runs() {
        // One live run: resolved without an argument.
        let pool = seeded_pool().await;
        let result = handle_run_status(&pool, None, now()).await.unwrap();
        let payload = result.structured_content.clone().unwrap();
        assert_eq!(payload["resolution"], "resolved");
        let text = text_of(&result);
        assert!(text.contains("last tool developer__shell"), "{text}");
        assert!(text.contains("awaiting your approval"), "{text}");
        assert!(text.contains("landed 0123456789ab"), "{text}");
        assert!(text.contains("parked for you: budget exhausted"), "{text}");
        assert!(text.contains("pending gate: Write"), "{text}");
        assert_private(&result);

        // Many live: the choices come back, not a guess.
        insert_lease(
            &pool,
            "hpl-live-0003-cccc",
            "life-3",
            "sess-3",
            "/work/gamma",
            None,
            "idle",
            &ago(3),
            None,
        )
        .await;
        let result = handle_run_status(&pool, None, now()).await.unwrap();
        assert_eq!(
            result.structured_content.as_ref().unwrap()["resolution"],
            "choose"
        );
        assert!(text_of(&result).contains("run hpl-live-000"));
        assert_private(&result);

        // A selector disambiguates.
        let result = handle_run_status(&pool, Some("gamma"), now())
            .await
            .unwrap();
        assert_eq!(
            result.structured_content.as_ref().unwrap()["resolution"],
            "resolved"
        );

        // Zero runs.
        let empty = empty_pool().await;
        let result = handle_run_status(&empty, None, now()).await.unwrap();
        assert_eq!(
            result.structured_content.as_ref().unwrap()["resolution"],
            "no_runs"
        );
    }

    #[tokio::test]
    async fn harness_attention_names_the_lost_terminal_and_what_waits() {
        let pool = seeded_pool().await;
        let result = handle_attention(&pool, now()).await.unwrap();
        let text = text_of(&result);
        assert!(
            text.contains("[lost terminal] terminal exited unexpectedly"),
            "{text}"
        );
        assert!(text.contains("[decision approve_review]"), "{text}");
        assert!(text.contains("[parked goal]"), "{text}");
        assert!(text.contains("[gate]"), "{text}");
        assert_private(&result);

        let empty = empty_pool().await;
        let text = text_of(&handle_attention(&empty, now()).await.unwrap());
        assert!(text.contains("No coding-harness runs are live or recent"));
    }

    #[tokio::test]
    async fn a_heartbeat_one_second_past_the_window_reads_as_lost() {
        let pool = empty_pool().await;
        insert_lease(
            &pool,
            "hpl-edge-45",
            "l45",
            "s45",
            "/work/edge45",
            None,
            "turn",
            &ago(45),
            None,
        )
        .await;
        insert_lease(
            &pool,
            "hpl-edge-46",
            "l46",
            "s46",
            "/work/edge46",
            None,
            "turn",
            &ago(46),
            None,
        )
        .await;
        let text = text_of(&handle_runs(&pool, now()).await.unwrap());
        assert!(text.contains("[live] edge45"), "{text}");
        assert!(text.contains("[LOST] edge46"), "{text}");
    }

    #[tokio::test]
    async fn the_tools_read_only_the_pool_they_are_handed() {
        // The client hands these handlers its own session pool — the database
        // under this process's Paths root. An eval run rooted elsewhere writes
        // its lease into a different database, which these reads never see.
        let eval_root = seeded_pool().await;
        let own_root = empty_pool().await;
        assert!(text_of(&handle_runs(&eval_root, now()).await.unwrap()).contains("1 live"));
        assert!(text_of(&handle_runs(&own_root, now()).await.unwrap())
            .contains("No coding-harness runs are live"));
    }
}
