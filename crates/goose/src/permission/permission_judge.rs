use crate::agents::reply_parts::AccountedFastCompletion;
use crate::conversation::message::{Message, MessageContent, ToolRequest};
use crate::conversation::Conversation;
use crate::prompt_template::render_template;
use crate::providers::base::Provider;
use crate::utils::sanitize_unicode_tags;
use chrono::Utc;
use indoc::indoc;
use rmcp::model::{Tool, ToolAnnotations};
use rmcp::object;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::Arc;

#[derive(Serialize)]
struct PermissionJudgeContext {
    // Empty struct for now since the current template doesn't need variables
}

/// Creates the tool definition for checking read-only permissions.
fn create_read_only_tool() -> Tool {
    Tool::new(
        "platform__tool_by_tool_permission".to_string(),
        indoc! {r#"
            Analyze the tool requests and determine which ones perform read-only operations.

            What constitutes a read-only operation:
            - A read-only operation retrieves information without modifying any data or state.
            - Examples include:
                - Reading a file without writing to it.
                - Querying a database without making updates.
                - Retrieving information from APIs without performing POST, PUT, or DELETE operations.

            Examples of read vs. write operations:
            - Read Operations:
                - `SELECT` query in SQL.
                - Reading file metadata or content.
                - Listing directory contents.
            - Write Operations:
                - `INSERT`, `UPDATE`, or `DELETE` in SQL.
                - Writing or appending to a file.
                - Modifying system configurations.
                - Sending messages to Slack channel.

            How to analyze tool requests:
            - Inspect each tool request to identify its purpose based on its name and arguments.
            - Categorize the operation as read-only if it does not involve any state or data modification.
            - Return a list of request IDs that are strictly read-only. If you cannot make the decision, then it is not read-only.

            Use this analysis to generate the list of request IDs performing read-only operations from the provided tool requests.
        "#}
        .to_string(),
        object!({
            "type": "object",
            "properties": {
                "read_only_tools": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    },
                    "description": "Optional list of request IDs which have read-only operations."
                }
            },
            "required": []
        })
    ).annotate(ToolAnnotations::with_title("Check tool operation".to_string()).read_only(true).destructive(false).idempotent(false).open_world(false))
}

/// Builds the message to be sent to the LLM for detecting read-only operations.
fn create_check_messages(tool_requests: Vec<&ToolRequest>) -> Option<Conversation> {
    let requests = tool_requests
        .into_iter()
        .map(|request| {
            let tool_call = request.tool_call.as_ref().ok()?;
            let arguments = sanitize_arguments(tool_call.arguments.as_ref()?)?;
            Some(serde_json::json!({
                "request_id": sanitize_unicode_tags(&request.id),
                "tool_name": sanitize_unicode_tags(tool_call.name.as_ref()),
                "arguments": arguments,
            }))
        })
        .collect::<Option<Vec<_>>>()?;
    let requests = serde_json::to_string_pretty(&requests).ok()?;
    let check_messages = vec![Message::new(
        rmcp::model::Role::User,
        Utc::now().timestamp(),
        vec![MessageContent::text(format!(
                "Here are the tool requests as JSON data:\n{requests}\n\nAnalyze each tool request and list the request IDs that perform read-only operations. \
                \n\nGuidelines for Read-Only Operations: \
                \n- Read-only operations do not modify any data or state. \
                \n- Examples include file reading, SELECT queries in SQL, and directory listing. \
                \n- Write operations include INSERT, UPDATE, DELETE, and file writing. \
                \n\nPlease provide a list of request IDs that qualify as read-only:"
            ))],
    )];
    Some(Conversation::new_unvalidated(check_messages))
}

fn sanitize_arguments(arguments: &Map<String, Value>) -> Option<Value> {
    fn sanitize_value(value: &Value) -> Option<Value> {
        match value {
            Value::String(value) => Some(Value::String(sanitize_unicode_tags(value))),
            Value::Array(values) => values
                .iter()
                .map(sanitize_value)
                .collect::<Option<Vec<_>>>()
                .map(Value::Array),
            Value::Object(values) => {
                let mut sanitized = Map::new();
                for (key, value) in values {
                    let key = sanitize_unicode_tags(key);
                    if sanitized.insert(key, sanitize_value(value)?).is_some() {
                        return None;
                    }
                }
                Some(Value::Object(sanitized))
            }
            value => Some(value.clone()),
        }
    }

    sanitize_value(&Value::Object(arguments.clone()))
}

/// Processes the response to extract the list of tools with read-only operations.
fn extract_read_only_tools(response: &Message) -> Option<Vec<String>> {
    for content in &response.content {
        if let MessageContent::ToolRequest(tool_request) = content {
            if let Ok(tool_call) = &tool_request.tool_call {
                if tool_call.name == "platform__tool_by_tool_permission" {
                    if let Some(arguments) = &tool_call.arguments {
                        if let Some(Value::Array(read_only_tools)) =
                            arguments.get("read_only_tools")
                        {
                            return Some(
                                read_only_tools
                                    .iter()
                                    .filter_map(|tool| tool.as_str().map(String::from))
                                    .collect(),
                            );
                        }
                    }
                }
            }
        }
    }
    None
}

/// Executes read-only detection and returns the IDs of read-only requests.
pub async fn detect_read_only_tools(
    provider: Arc<dyn Provider>,
    session_id: &str,
    tool_requests: Vec<&ToolRequest>,
) -> Vec<String> {
    if tool_requests.is_empty() {
        return vec![];
    }
    let tool = create_read_only_tool();
    let Some(check_messages) = create_check_messages(tool_requests) else {
        return vec![];
    };

    let context = PermissionJudgeContext {};
    let system_prompt = render_template("permission_judge.md", &context)
        .unwrap_or_else(|_| "You are a good analyst and can detect operations whether they have read-only operations.".to_string());

    let manager = Arc::new(crate::session::SessionManager::instance());
    let session = match manager.get_session(session_id, false).await {
        Ok(session) => session,
        Err(error) => {
            tracing::warn!(session_id, "permission judge session unavailable: {error}");
            return vec![];
        }
    };
    let res = AccountedFastCompletion::complete_accounted(
        manager,
        session,
        provider,
        &system_prompt,
        check_messages.messages(),
        std::slice::from_ref(&tool),
        false,
    )
    .await;

    // Process the response and return an empty vector if the response is invalid
    if let Ok((message, _usage)) = res {
        extract_read_only_tools(&message).unwrap_or_default()
    } else {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::CallToolRequestParams;

    #[test]
    fn permission_judge_cannot_bypass_shared_paid_dispatch_boundary() {
        let source = include_str!("permission_judge.rs");
        let direct_call = [".", "complete("].concat();
        assert!(source.contains("complete_accounted"));
        assert!(!source.contains(&direct_call));
    }

    fn request(id: &str, query: &str) -> ToolRequest {
        ToolRequest {
            id: id.to_string(),
            tool_call: Ok(
                CallToolRequestParams::new("database.query").with_arguments(object!({
                    "sql": query,
                })),
            ),
            metadata: None,
            tool_meta: None,
        }
    }

    #[test]
    fn judge_input_includes_request_ids_names_and_arguments() {
        let select = request("select-request", "SELECT * FROM users");
        let delete = request("delete-request", "DELETE FROM users");

        let messages = create_check_messages(vec![&select, &delete]).unwrap();
        let MessageContent::Text(input) = &messages.messages()[0].content[0] else {
            panic!("judge input must be text");
        };

        assert!(input.text.contains("\"request_id\": \"select-request\""));
        assert!(input.text.contains("\"request_id\": \"delete-request\""));
        assert!(input.text.contains("\"tool_name\": \"database.query\""));
        assert!(input.text.contains("\"sql\": \"SELECT * FROM users\""));
        assert!(input.text.contains("\"sql\": \"DELETE FROM users\""));
    }

    #[test]
    fn unsafe_argument_sanitization_fails_closed() {
        let request = ToolRequest {
            id: "request".to_string(),
            tool_call: Ok(CallToolRequestParams::new("tool").with_arguments(object!({
                "same": "first",
                "\u{e0001}s\u{e007f}ame": "second",
            }))),
            metadata: None,
            tool_meta: None,
        };

        assert!(create_check_messages(vec![&request]).is_none());
    }

    #[test]
    fn uncertain_or_invalid_judge_response_returns_no_approvals() {
        assert_eq!(extract_read_only_tools(&Message::assistant()), None);
    }
}

/// Result of permission checking for tool requests
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionCheckResult {
    pub approved: Vec<ToolRequest>,
    pub needs_approval: Vec<ToolRequest>,
    pub denied: Vec<ToolRequest>,
}
