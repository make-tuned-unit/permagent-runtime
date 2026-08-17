use crate::agents::self_knowledge::{FeatureCategory, FeatureDescriptor, StateSource};
use crate::config::Config;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};

const CONFIG_KEY: &str = "GOOSE_TOOL_ARG_VALIDATION";
static WARNED_UNKNOWN_MODE: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolArgValidationMode {
    Off,
    Warn,
    Enforce,
}

impl ToolArgValidationMode {
    pub fn from_config() -> Self {
        let value = Config::global()
            .get_param::<String>(CONFIG_KEY)
            .unwrap_or_else(|_| "warn".to_string());
        match value.as_str() {
            "off" => Self::Off,
            "warn" => Self::Warn,
            "enforce" => Self::Enforce,
            unknown => {
                if !WARNED_UNKNOWN_MODE.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        value = unknown,
                        "unknown tool argument validation mode; using warn"
                    );
                }
                Self::Warn
            }
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Warn => "warn",
            Self::Enforce => "enforce",
        }
    }
}

/// Compile a declared schema once. Callers that need both the verdict and the
/// error list hold onto the validator, so nothing is compiled twice on the
/// failure path.
pub fn compile_schema(schema: &Value) -> Result<jsonschema::Validator, String> {
    jsonschema::validator_for(schema).map_err(|error| error.to_string())
}

/// One `- {instance path}: {problem}` line per violation.
pub fn validation_errors(validator: &jsonschema::Validator, value: &Value) -> Vec<String> {
    validator
        .iter_errors(value)
        .map(|error| format!("- {}: {}", error.instance_path, error))
        .collect()
}

/// The single copy of the repair message in the tree. `final_output` has
/// returned this shape since it shipped and models demonstrably act on it, so
/// the dispatch gate returns the identical text rather than inventing a second
/// dialect for the same failure.
pub fn schema_error_message(schema: &Value, errors: &[String]) -> String {
    format!(
        "Validation failed:\n{}\n\nExpected format:\n{}\n\nPlease correct your output to match the expected JSON schema and try again.",
        errors.join("\n"),
        serde_json::to_string_pretty(schema).unwrap_or_else(|_| "{}".to_string())
    )
}

pub fn validate_against_schema(schema: &Value, value: &Value) -> Result<(), String> {
    let validator = compile_schema(schema)?;
    let errors = validation_errors(&validator, value);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(schema_error_message(schema, &errors))
    }
}

/// True when validating would be pure cost: the schema constrains nothing that
/// a caller could get wrong. Plenty of MCP servers declare `{}` or a bare
/// `{"type": "object"}`, and compiling those on every dispatch buys nothing.
pub fn is_permissive_schema(schema: &Value) -> bool {
    let Some(object) = schema.as_object() else {
        return true;
    };
    let has_properties = object
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|properties| !properties.is_empty());
    let has_required = object
        .get("required")
        .and_then(Value::as_array)
        .is_some_and(|required| !required.is_empty());
    !has_properties && !has_required
}

pub const SELF_KNOWLEDGE_FEATURE: FeatureDescriptor = FeatureDescriptor {
    id: "tool_argument_validation",
    display_name: "Tool argument validation",
    category: FeatureCategory::Guard,
    what_it_does: "Checks model-emitted tool arguments against each tool's declared schema. It is warn-only by default: it logs and records a mismatch but still runs the tool. An operator can turn on enforcing mode, which returns concrete per-field errors and the expected schema so the agent can correct the arguments and retry. The retry budget is capped so it cannot loop",
    why_it_matters: "Malformed model output and a tool that ran and failed need different fixes; this guard makes that distinction observable while remaining compatible with servers whose declared schemas are stricter than their implementations",
    state_source: StateSource::Static,
    teaching: &[],
};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn permissive_schemas_are_identified() {
        assert!(is_permissive_schema(&json!({})));
        assert!(is_permissive_schema(
            &json!({"type": "object", "additionalProperties": true})
        ));
        assert!(!is_permissive_schema(
            &json!({"type": "object", "required": ["name"]})
        ));
    }

    #[test]
    fn violation_uses_shared_message() {
        let schema = json!({"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]});
        let validator = compile_schema(&schema).unwrap();

        // A conforming value passes.
        assert_eq!(
            validate_against_schema(&schema, &json!({"name": "ok"})),
            Ok(())
        );

        // A violating one produces the per-path errors, the pretty schema, and
        // the repair instruction — and `schema_error_message` is the only
        // producer of that text, so the gate and `final_output` cannot drift.
        let missing = json!({});
        let errors = validation_errors(&validator, &missing);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("- : "));
        assert_eq!(
            validate_against_schema(&schema, &missing),
            Err(schema_error_message(&schema, &errors))
        );

        let message = schema_error_message(&schema, &errors);
        assert!(message.starts_with("Validation failed:\n- : "));
        assert!(message.contains(&serde_json::to_string_pretty(&schema).unwrap()));
        assert!(message.ends_with(
            "Please correct your output to match the expected JSON schema and try again."
        ));
    }

    #[test]
    fn wrong_typed_property_is_reported_with_its_path() {
        let schema = json!({"type": "object", "properties": {"count": {"type": "integer"}}});
        let validator = compile_schema(&schema).unwrap();
        let errors = validation_errors(&validator, &json!({"count": "three"}));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("- /count: "), "{}", errors[0]);
    }

    #[test]
    #[serial_test::serial]
    fn mode_parsing_defaults_and_falls_back_to_warn() {
        std::env::remove_var(CONFIG_KEY);
        assert_eq!(
            ToolArgValidationMode::from_config(),
            ToolArgValidationMode::Warn
        );
        std::env::set_var(CONFIG_KEY, "off");
        assert_eq!(
            ToolArgValidationMode::from_config(),
            ToolArgValidationMode::Off
        );
        std::env::set_var(CONFIG_KEY, "enforce");
        assert_eq!(
            ToolArgValidationMode::from_config(),
            ToolArgValidationMode::Enforce
        );
        std::env::set_var(CONFIG_KEY, "unknown");
        assert_eq!(
            ToolArgValidationMode::from_config(),
            ToolArgValidationMode::Warn
        );
        std::env::remove_var(CONFIG_KEY);
    }
}
