use crate::agents::schema_validation::{compile_schema, schema_error_message, validation_errors};
use crate::agents::tool_execution::ToolCallResult;
use crate::recipe::validate_recipe::validate_response_json_schema;
use crate::recipe::Response;
use indoc::formatdoc;
use rmcp::model::{CallToolRequestParams, Content, ErrorCode, ErrorData, Tool, ToolAnnotations};
use serde_json::Value;
use std::borrow::Cow;

pub const FINAL_OUTPUT_TOOL_NAME: &str = "recipe__final_output";
pub const FINAL_OUTPUT_CONTINUATION_MESSAGE: &str =
    "You MUST call the `final_output` tool NOW with the final output for the user.";

pub struct FinalOutputTool {
    pub response: Response,
    /// The final output collected for the user. It will be a single line string for easy script extraction from output.
    pub final_output: Option<String>,
}

impl FinalOutputTool {
    /// Build the final-output tool from a recipe `response` block.
    ///
    /// This must NEVER panic on any input: a bad recipe schema used to panic
    /// here (and per-turn in [`Self::tool`]), which fed the crash breaker and
    /// could crash-cycle the daemon on a scheduled recipe. The schema must be
    /// present, a non-empty JSON object, and meta-valid; anything else is an
    /// error the caller surfaces to the user.
    pub fn new(response: Response) -> anyhow::Result<Self> {
        let schema = response.json_schema.as_ref().ok_or_else(|| {
            anyhow::anyhow!("recipe response.json_schema invalid: json_schema is required")
        })?;
        validate_response_json_schema(schema)
            .map_err(|e| anyhow::anyhow!("recipe response.json_schema invalid: {}", e))?;
        Ok(Self {
            response,
            final_output: None,
        })
    }

    /// Pretty-printed schema for prompts. `new` guarantees the schema exists;
    /// the fallback is purely defensive (the `response` field is public) and
    /// can never panic.
    fn schema_pretty(&self) -> String {
        self.response
            .json_schema
            .as_ref()
            .and_then(|s| serde_json::to_string_pretty(s).ok())
            .unwrap_or_else(|| "{}".to_string())
    }

    /// The schema as a JSON object for the tool definition. `new` guarantees a
    /// non-empty object; the fallback is defensive and can never panic.
    fn schema_object(&self) -> serde_json::Map<String, Value> {
        self.response
            .json_schema
            .as_ref()
            .and_then(|s| s.as_object())
            .cloned()
            .unwrap_or_default()
    }

    pub fn tool(&self) -> Tool {
        let instructions = formatdoc! {r#"
            The final_output tool collects the final output for the user and provides validation for structured JSON final output against a predefined schema.

            This final_output tool MUST be called with the final output for the user.
            
            Purpose:
            - Collects the final output for the user
            - Ensures that final outputs conform to the expected JSON structure
            - Provides clear validation feedback when outputs don't match the schema
            
            Usage:
            - Call the `final_output` tool with your JSON final output passed as the argument.
            
            The expected JSON schema format is:

            {}
            
            When validation fails, you'll receive:
            - Specific validation errors
            - The expected format
        "#, self.schema_pretty()};

        Tool::new(
            FINAL_OUTPUT_TOOL_NAME.to_string(),
            instructions,
            self.schema_object(),
        )
        .annotate(
            ToolAnnotations::with_title("Final Output".to_string())
                .read_only(false)
                .destructive(false)
                .idempotent(true)
                .open_world(false),
        )
    }

    pub fn system_prompt(&self) -> String {
        formatdoc! {r#"
            # Final Output Instructions

            You MUST use the `final_output` tool to collect the final output for the user rather than providing the output directly in your response.
            The final output MUST be a valid JSON object that is provided to the `final_output` tool when called and it must match the following schema:

            {}

            ----
        "#, self.schema_pretty()}
    }

    async fn validate_json_output(&self, output: &Value) -> Result<Value, String> {
        let Some(schema) = self.response.json_schema.as_ref() else {
            return Err("Internal error: final output schema is missing".to_string());
        };
        let validator = compile_schema(schema)
            .map_err(|e| format!("Internal error: Failed to compile schema: {}", e))?;
        let errors = validation_errors(&validator, output);
        if errors.is_empty() {
            Ok(output.clone())
        } else {
            Err(schema_error_message(schema, &errors))
        }
    }

    pub async fn execute_tool_call(&mut self, tool_call: CallToolRequestParams) -> ToolCallResult {
        match tool_call.name.to_string().as_str() {
            FINAL_OUTPUT_TOOL_NAME => {
                let result = self.validate_json_output(&tool_call.arguments.into()).await;
                match result {
                    Ok(parsed_value) => {
                        self.final_output = Some(Self::parsed_final_output_string(parsed_value));
                        ToolCallResult::from(Ok(rmcp::model::CallToolResult::success(vec![
                            Content::text("Final output successfully collected.".to_string()),
                        ])))
                    }
                    Err(error) => ToolCallResult::from(Err(ErrorData {
                        code: ErrorCode::INVALID_PARAMS,
                        message: Cow::from(error),
                        data: None,
                    })),
                }
            }
            _ => ToolCallResult::from(Err(ErrorData {
                code: ErrorCode::INVALID_REQUEST,
                message: Cow::from(format!("Unknown tool: {}", tool_call.name)),
                data: None,
            })),
        }
    }

    // Formats the parsed JSON as a single line string so its easy to extract
    // from the output. `Value`'s `Display` is infallible compact JSON.
    fn parsed_final_output_string(parsed_json: Value) -> String {
        parsed_json.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::Response;
    use rmcp::model::CallToolRequestParams;
    use rmcp::object;
    use serde_json::json;

    fn create_complex_test_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "user": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "age": {"type": "number"}
                    },
                    "required": ["name", "age"]
                },
                "tags": {
                    "type": "array",
                    "items": {"type": "string"}
                }
            },
            "required": ["user", "tags"]
        })
    }

    fn expect_invalid(json_schema: Option<Value>, expected_fragment: &str) {
        let err = FinalOutputTool::new(Response { json_schema })
            .err()
            .expect("constructor must reject the schema, not panic");
        let msg = err.to_string();
        assert!(
            msg.contains("recipe response.json_schema invalid"),
            "error must be recognizable as a schema problem: {}",
            msg
        );
        assert!(
            msg.contains(expected_fragment),
            "error {:?} must mention {:?}",
            msg,
            expected_fragment
        );
    }

    #[test]
    fn test_new_with_missing_schema_errors() {
        expect_invalid(None, "json_schema is required");
    }

    #[test]
    fn test_new_with_empty_schema_errors() {
        expect_invalid(Some(json!({})), "empty");
    }

    #[test]
    fn test_new_with_boolean_schema_errors() {
        // `true` IS a valid JSON Schema (meta-validation passes) but is not an
        // object — this used to panic on EVERY TURN in `tool()`, taking the
        // daemon down via the panic breaker.
        expect_invalid(Some(json!(true)), "JSON object");
        expect_invalid(Some(json!(false)), "JSON object");
    }

    #[test]
    fn test_new_with_non_object_schema_errors() {
        expect_invalid(Some(json!([1, 2])), "JSON object");
        expect_invalid(Some(json!("string")), "JSON object");
        expect_invalid(Some(json!(42)), "JSON object");
    }

    #[test]
    fn test_new_with_garbage_schema_errors() {
        // `type` must be a string/array per the meta-schema.
        expect_invalid(Some(json!({"type": 42})), "");
    }

    #[test]
    fn test_new_with_invalid_type_names_errors() {
        expect_invalid(
            Some(json!({
                "type": "invalid_type",
                "properties": {
                    "message": {
                        "type": "unknown_type"
                    }
                }
            })),
            "",
        );
    }

    #[test]
    fn test_new_with_valid_schema_succeeds_and_tool_is_panic_free() {
        let tool = FinalOutputTool::new(Response {
            json_schema: Some(create_complex_test_schema()),
        })
        .expect("valid schema must construct");
        // `tool()` used to unwrap per turn; it must be panic-free now.
        let t = tool.tool();
        assert_eq!(t.name, FINAL_OUTPUT_TOOL_NAME);
        assert!(tool.system_prompt().contains("final_output"));
    }

    #[tokio::test]
    async fn test_execute_tool_call_schema_validation_failure() {
        let response = Response {
            json_schema: Some(json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string"
                    },
                    "count": {
                        "type": "number"
                    }
                },
                "required": ["message", "count"]
            })),
        };

        let mut tool = FinalOutputTool::new(response).unwrap();
        let tool_call =
            CallToolRequestParams::new(FINAL_OUTPUT_TOOL_NAME).with_arguments(object!({
                "message": "Hello"  // Missing required "count" field
            }));

        let result = tool.execute_tool_call(tool_call).await;
        let tool_result = result.result.await;
        assert!(tool_result.is_err());
        if let Err(error) = tool_result {
            assert!(error.to_string().contains("Validation failed"));
        }
    }

    #[tokio::test]
    async fn test_execute_tool_call_complex_valid_json() {
        let response = Response {
            json_schema: Some(create_complex_test_schema()),
        };

        let mut tool = FinalOutputTool::new(response).unwrap();
        let tool_call =
            CallToolRequestParams::new(FINAL_OUTPUT_TOOL_NAME).with_arguments(object!({
                "user": {
                    "name": "John",
                    "age": 30
                },
                "tags": ["developer", "rust"]
            }));

        let result = tool.execute_tool_call(tool_call).await;
        let tool_result = result.result.await;
        assert!(tool_result.is_ok());
        assert!(tool.final_output.is_some());

        let final_output = tool.final_output.unwrap();
        assert!(serde_json::from_str::<Value>(&final_output).is_ok());
        assert!(!final_output.contains('\n'));
    }
}
