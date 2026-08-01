//! Retrospect — the agent reviews where it struggled and asks for better tools.
//!
//! Two tools:
//!
//! - `review_struggles` reads the RECORD of a session's tool calls (not the
//!   prose transcript) and reports where work went sideways: calls that failed,
//!   the same call shape retried over and over, long runs of failure on one
//!   tool. It is a read of `tasks`, which only became truthful once failed calls
//!   stopped being logged as completed.
//! - `request_capability` files a `capability_gap` proposal into the Decision
//!   Inbox when the honest conclusion is "I did not have a tool for this."
//!
//! ## Why a request and not a lesson
//!
//! The motivating case: asked for the weather, the agent tried a search API,
//! four government URLs, weather.com, DuckDuckGo and `curl` over several
//! minutes, while the answer sat on the user's own dashboard. No amount of
//! self-instruction fixes that — the tool did not exist. A lesson would have
//! taught it to flail more confidently.
//!
//! This module therefore only ever PROPOSES, matching the Steward's constraint
//! (`crate::steward`): the agent surfaces the gap, the user decides whether to
//! close it. Nothing here writes a lesson into the agent's own context, so it
//! carries none of the −9.2pp risk that authoritative distilled hints do (see
//! `librarian_atoms`, `playbook`).
//!
//! ## Evidence, not vibes
//!
//! `request_capability` requires concrete failed attempts. An agent that can
//! request capabilities on a hunch is one that can fabricate a problem to look
//! diligent about solving it — the documented phantom-guardrail failure mode.
//! `review_struggles` reads persisted rows, so the evidence exists independent
//! of anything the model asserts.

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use async_trait::async_trait;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool,
};
use sqlx::{Pool, Sqlite};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "retrospect";

/// A tool that failed at least this many times in one session is a struggle
/// worth reporting rather than an ordinary miss. Two is the smallest number
/// that distinguishes "it failed" from "it kept failing".
const REPEAT_FAILURE_FLOOR: i64 = 2;

/// One tool's failure record inside a session.
#[derive(Debug, Clone, PartialEq)]
pub struct StruggleRow {
    pub tool: String,
    pub failures: i64,
    /// Distinct argument shapes among the failures. One shape repeated means
    /// the same call was retried unchanged; many shapes means it was casting
    /// around for a formulation that worked. Different problems.
    pub distinct_shapes: i64,
    pub last_error: Option<String>,
}

/// Classify a struggle from its counts. Kept pure and separate from the query so
/// the judgement is unit-testable without a database.
pub fn classify_struggle(row: &StruggleRow) -> &'static str {
    if row.failures < REPEAT_FAILURE_FLOOR {
        "isolated failure"
    } else if row.distinct_shapes <= 1 {
        // Same call, same arguments, again. Retrying an unchanged call is not
        // progress; something outside the arguments is wrong.
        "repeated identical call — retrying unchanged"
    } else if row.distinct_shapes >= row.failures {
        // Every attempt was a different shape: no formulation worked, which is
        // what a missing capability looks like from the inside.
        "flailing across formulations — possible missing capability"
    } else {
        "repeated failure with variation"
    }
}

/// Read a session's failed tool calls, grouped by tool.
///
/// Reads `tasks`, which is only trustworthy because failed calls are now
/// recorded as `failed` — before that fix every row said `completed` and this
/// query would have returned nothing, forever.
pub async fn struggles_for_session(
    pool: &Pool<Sqlite>,
    session_id: &str,
) -> Result<Vec<StruggleRow>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String, i64, i64, Option<String>)>(
        "SELECT COALESCE(tool_used, 'unknown') AS tool,
                COUNT(*) AS failures,
                COUNT(DISTINCT COALESCE(argument_shape_hash, '')) AS distinct_shapes,
                MAX(error_message) AS last_error
           FROM tasks
          WHERE session_id = ? AND status = 'failed'
          GROUP BY tool
          ORDER BY failures DESC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(tool, failures, distinct_shapes, last_error)| StruggleRow {
                tool,
                failures,
                distinct_shapes,
                last_error,
            },
        )
        .collect())
}

/// Render the struggle report the agent reads back.
pub fn format_struggles(session_id: &str, rows: &[StruggleRow]) -> String {
    if rows.is_empty() {
        return format!(
            "No failed tool calls recorded for session {session_id}. Either the work went \
             cleanly, or the struggle was not one the tool layer can see — a wrong answer \
             confidently given leaves no failed call behind. Ask the user what went wrong \
             rather than concluding nothing did."
        );
    }

    let mut out = format!("Where session {session_id} struggled:\n");
    for r in rows {
        out.push_str(&format!(
            "\n- {} — {} failure(s), {} distinct call shape(s): {}",
            r.tool,
            r.failures,
            r.distinct_shapes,
            classify_struggle(r)
        ));
        if let Some(err) = r.last_error.as_deref() {
            let brief: String = err
                .lines()
                .next()
                .unwrap_or(err)
                .chars()
                .take(160)
                .collect();
            out.push_str(&format!("\n  last error: {brief}"));
        }
    }
    out.push_str(
        "\n\nIf one of these is a tool you did not have rather than a tool you used badly, \
         call request_capability with the attempts as evidence.",
    );
    out
}

pub struct RetrospectClient {
    info: InitializeResult,
}

impl RetrospectClient {
    pub fn new(_context: PlatformExtensionContext) -> Result<Self, anyhow::Error> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME.to_string(), "1.0.0".to_string())
                    .with_title("Retrospect"),
            );
        Ok(Self { info })
    }

    pub(crate) fn get_tools() -> Vec<Tool> {
        let review_schema: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Session to review. Omit to review the current one."
                }
            },
            "required": []
        }))
        .expect("static schema");

        let request_schema: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "user_goal": {
                    "type": "string",
                    "description": "What the user was trying to get done, in their words."
                },
                "missing_capability": {
                    "type": "string",
                    "description": "The tool that would have made this one step instead of many. Be concrete: 'read the weather card on the Home dashboard', not 'better web access'."
                },
                "attempts": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "What you actually tried, in order, and how each failed. REQUIRED — a request with no failed attempts is a guess, and will be refused."
                },
                "nearest_existing_tool": {
                    "type": "string",
                    "description": "An existing tool that nearly covers this, if any. Often the answer is to extend one rather than add another."
                }
            },
            "required": ["user_goal", "missing_capability", "attempts"]
        }))
        .expect("static schema");

        vec![
            Tool::new(
                "review_struggles".to_string(),
                "Review where you struggled in a session — which tool calls failed, which you \
                 retried unchanged, and which you kept reformulating. Reads the recorded outcome \
                 of every call, so it reflects what actually happened rather than what you \
                 remember. Use it when the user says something went badly, or to check your own \
                 work after a long or frustrating session."
                    .to_string(),
                review_schema,
            ),
            Tool::new(
                "request_capability".to_string(),
                "Ask the user for a tool you do not have. Use this when review_struggles (or \
                 plain experience) shows you were missing a capability rather than using one \
                 badly — you tried several routes and none of them was the right shape for the \
                 job. Files a proposal in the Decision Inbox for the user to accept or decline; \
                 it does NOT build anything. Requires concrete failed attempts as evidence."
                    .to_string(),
                request_schema,
            ),
        ]
    }

    async fn handle_review(&self, session_id: &str) -> Result<Vec<Content>, String> {
        if session_id.trim().is_empty() {
            return Err(
                "No session id available to review. Pass session_id explicitly.".to_string(),
            );
        }
        let pool = crate::tasks::global()
            .map(|logger| logger.pool().clone())
            .ok_or_else(|| "Task history is unavailable in this process.".to_string())?;

        let rows = struggles_for_session(&pool, session_id)
            .await
            .map_err(|e| format!("Could not read task history: {e}"))?;

        Ok(vec![Content::text(format_struggles(session_id, &rows))])
    }

    async fn handle_request(
        &self,
        payload: crate::decisions::CapabilityGapPayload,
    ) -> Result<Vec<Content>, String> {
        let pool = crate::tasks::global()
            .map(|logger| logger.pool().clone())
            .ok_or_else(|| "Decision storage is unavailable in this process.".to_string())?;

        let headline = format!(
            "Tool request: {}",
            payload
                .missing_capability
                .chars()
                .take(60)
                .collect::<String>()
        );
        let detail = format!(
            "While trying to: {}\n\nMissing capability: {}\n\nWhat was tried:\n{}\n{}",
            payload.user_goal,
            payload.missing_capability,
            payload
                .attempts
                .iter()
                .filter(|a| !a.trim().is_empty())
                .map(|a| format!("  - {a}"))
                .collect::<Vec<_>>()
                .join("\n"),
            payload
                .nearest_existing_tool
                .as_deref()
                .map(|t| format!("\nNearest existing tool: {t}"))
                .unwrap_or_default(),
        );

        let payload_json = serde_json::to_value(&payload)
            .map_err(|e| format!("Could not serialize the request: {e}"))?;

        // Tier 2 by construction: adding a capability is the user's call, and an
        // unseeded action class fails closed to them anyway.
        let req = crate::decisions::NewDecision {
            kind: "capability_gap".to_string(),
            headline: Some(headline),
            detail: Some(detail),
            payload: payload_json,
            ..Default::default()
        };

        let decision = crate::decisions::create_decision(&pool, req)
            .await
            .map_err(|e| format!("Could not file the request: {e}"))?;

        Ok(vec![Content::text(format!(
            "Filed a tool request in the Decision Inbox ({}). It is a proposal — nothing was \
             built and nothing changed. Tell the user plainly what you were missing and that you \
             have asked for it, rather than implying the gap is now closed.",
            decision.id
        ))])
    }
}

#[async_trait]
impl McpClientTrait for RetrospectClient {
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
        ctx: &ToolCallContext,
        name: &str,
        arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        let args = arguments.unwrap_or_default();
        match name {
            "review_struggles" => {
                let session_id = args
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| ctx.session_id.clone());
                match self.handle_review(&session_id).await {
                    Ok(content) => Ok(CallToolResult::success(content)),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e)])),
                }
            }
            "request_capability" => {
                let mut payload: crate::decisions::CapabilityGapPayload =
                    match serde_json::from_value(serde_json::Value::Object(args)) {
                        Ok(p) => p,
                        Err(e) => {
                            return Ok(CallToolResult::error(vec![Content::text(format!(
                                "request_capability arguments were not valid: {e}"
                            ))]))
                        }
                    };
                if payload.session_id.is_none() {
                    payload.session_id = Some(ctx.session_id.clone());
                }
                match self.handle_request(payload).await {
                    Ok(content) => Ok(CallToolResult::success(content)),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e)])),
                }
            }
            _ => Ok(CallToolResult::error(vec![Content::text(format!(
                "Unknown tool: {name}"
            ))])),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(tool: &str, failures: i64, shapes: i64) -> StruggleRow {
        StruggleRow {
            tool: tool.to_string(),
            failures,
            distinct_shapes: shapes,
            last_error: None,
        }
    }

    #[test]
    fn one_failure_is_not_a_struggle() {
        assert_eq!(classify_struggle(&row("shell", 1, 1)), "isolated failure");
    }

    #[test]
    fn the_same_call_retried_unchanged_is_named_as_such() {
        assert_eq!(
            classify_struggle(&row("shell", 4, 1)),
            "repeated identical call — retrying unchanged"
        );
    }

    #[test]
    fn a_different_shape_every_time_reads_as_a_missing_capability() {
        // The weather case: six attempts, six different formulations, none right.
        assert_eq!(
            classify_struggle(&row("shell", 6, 6)),
            "flailing across formulations — possible missing capability"
        );
    }

    #[test]
    fn repeated_failure_with_some_variation_is_its_own_case() {
        assert_eq!(
            classify_struggle(&row("shell", 5, 2)),
            "repeated failure with variation"
        );
    }

    #[test]
    fn an_empty_report_refuses_to_claim_nothing_went_wrong() {
        let out = format_struggles("s1", &[]);
        // A confidently wrong answer leaves no failed call behind, so silence
        // here must not be reported as success.
        assert!(out.contains("Ask the user what went wrong"));
    }

    #[test]
    fn the_report_names_the_tool_the_counts_and_the_verdict() {
        let out = format_struggles("s1", &[row("shell", 6, 6)]);
        assert!(out.contains("shell"));
        assert!(out.contains("6 failure(s)"));
        assert!(out.contains("possible missing capability"));
        assert!(out.contains("request_capability"));
    }
}
