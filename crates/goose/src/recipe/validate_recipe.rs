use crate::recipe::read_recipe_file_content::RecipeFile;
use crate::recipe::template_recipe::parse_recipe_content;
use crate::recipe::{
    Recipe, RecipeParameter, RecipeParameterInputType, RecipeParameterRequirement,
    BUILT_IN_RECIPE_DIR_PARAM,
};
use anyhow::Result;
use std::collections::HashSet;

pub fn parse_and_validate_parameters(
    recipe_file_content: &str,
    recipe_dir_str: Option<String>,
) -> Result<Recipe> {
    let (recipe_template, template_variables) =
        parse_recipe_content(recipe_file_content, recipe_dir_str)?;
    let recipe_parameters = &recipe_template.parameters;
    validate_optional_parameters(recipe_parameters)?;
    validate_parameters_in_template(recipe_parameters, &template_variables)?;
    Ok(recipe_template)
}

/// Validate a recipe `response.json_schema` value.
///
/// Shared by recipe save/validation (authoring time) and
/// [`crate::agents::final_output_tool::FinalOutputTool::new`] (run time), so a
/// schema that would fail at run time is rejected in the recipe editor
/// instead. The schema must be a non-empty JSON object that passes JSON Schema
/// meta-validation and compiles. Note `true` and `{}` are valid JSON Schemas
/// but are rejected here: they don't describe a usable final-output shape and
/// previously crashed the daemon at tool-listing time.
pub fn validate_response_json_schema(schema: &serde_json::Value) -> Result<()> {
    let type_name = match schema {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    };
    let obj = schema.as_object().ok_or_else(|| {
        anyhow::anyhow!(
            "response.json_schema must be a JSON object, got {}",
            type_name
        )
    })?;
    if obj.is_empty() {
        return Err(anyhow::anyhow!(
            "response.json_schema must not be an empty object"
        ));
    }
    jsonschema::meta::validate(schema).map_err(|err| {
        anyhow::anyhow!("response.json_schema is not a valid JSON Schema: {}", err)
    })?;
    jsonschema::validator_for(schema)
        .map_err(|err| anyhow::anyhow!("response.json_schema failed to compile: {}", err))?;
    Ok(())
}

pub fn validate_recipe_template_from_file(recipe_file: &RecipeFile) -> Result<Recipe> {
    let recipe_dir = recipe_file
        .parent_dir
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Error getting recipe directory"))?
        .to_string();

    validate_recipe_template_from_content(&recipe_file.content, Some(recipe_dir))
}

pub fn validate_recipe_template_from_content(
    recipe_content: &str,
    recipe_dir: Option<String>,
) -> Result<Recipe> {
    parse_and_validate_parameters(recipe_content, recipe_dir.clone())?;
    let (recipe, _) = parse_recipe_content(recipe_content, recipe_dir)?;

    validate_prompt_or_instructions(&recipe)?;
    validate_retry_config(&recipe)?;
    if let Some(response) = &recipe.response {
        match &response.json_schema {
            Some(json_schema) => validate_response_json_schema(json_schema)?,
            // A `response` block without a schema used to slip through here
            // and panic at run time when the final-output tool was built.
            None => {
                return Err(anyhow::anyhow!(
                    "Recipe `response` block requires a `json_schema` (a non-empty JSON object)"
                ))
            }
        }
    }

    Ok(recipe)
}

fn validate_retry_config(recipe: &Recipe) -> Result<()> {
    if let Some(ref retry_config) = recipe.retry {
        if let Err(validation_error) = retry_config.validate() {
            return Err(anyhow::anyhow!(
                "Invalid retry configuration: {}",
                validation_error
            ));
        }
    }
    Ok(())
}

fn validate_prompt_or_instructions(recipe: &Recipe) -> Result<()> {
    let has_instructions = recipe
        .instructions
        .as_ref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    let has_prompt = recipe
        .prompt
        .as_ref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);

    if has_instructions || has_prompt {
        return Ok(());
    }

    Err(anyhow::anyhow!(
        "Recipe must specify at least one of `instructions` or `prompt`."
    ))
}

fn validate_parameters_in_template(
    recipe_parameters: &Option<Vec<RecipeParameter>>,
    template_variables: &HashSet<String>,
) -> Result<()> {
    let mut template_variables = template_variables.clone();
    template_variables.remove(BUILT_IN_RECIPE_DIR_PARAM);

    let param_keys: HashSet<String> = recipe_parameters
        .as_ref()
        .unwrap_or(&vec![])
        .iter()
        .map(|p| p.key.clone())
        .collect();

    let missing_keys = template_variables
        .difference(&param_keys)
        .collect::<Vec<_>>();

    let extra_keys = param_keys
        .difference(&template_variables)
        .collect::<Vec<_>>();

    if missing_keys.is_empty() && extra_keys.is_empty() {
        return Ok(());
    }

    let mut message = String::new();

    if !missing_keys.is_empty() {
        message.push_str(&format!(
            "Missing definitions for parameters in the recipe file: {}.",
            missing_keys
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if !extra_keys.is_empty() {
        message.push_str(&format!(
            "\nUnnecessary parameter definitions: {}.",
            extra_keys
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Err(anyhow::anyhow!("{}", message.trim_end()))
}

fn validate_optional_parameters(parameters: &Option<Vec<RecipeParameter>>) -> Result<()> {
    let empty_params = vec![];
    let params = parameters.as_ref().unwrap_or(&empty_params);

    let file_params_with_defaults: Vec<String> = params
        .iter()
        .filter(|p| matches!(p.input_type, RecipeParameterInputType::File) && p.default.is_some())
        .map(|p| p.key.clone())
        .collect();

    if !file_params_with_defaults.is_empty() {
        return Err(anyhow::anyhow!("File parameters cannot have default values to avoid importing sensitive user files: {}", file_params_with_defaults.join(", ")));
    }

    let optional_params_without_default_values: Vec<String> = params
        .iter()
        .filter(|p| {
            matches!(p.requirement, RecipeParameterRequirement::Optional) && p.default.is_none()
        })
        .map(|p| p.key.clone())
        .collect();

    if optional_params_without_default_values.is_empty() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("Optional parameters missing default values in the recipe: {}. Please provide defaults.", optional_params_without_default_values.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_recipe_template_from_content_success() {
        let recipe_content = r#"
version: 1.0.0
title: Test Recipe
description: A test recipe for validation
instructions: Test instructions with {{ user_role }}
prompt: |
  {% if user_role in ["Director, Account Management", "Senior Director, Account Management"] %}
  - Focus on strategic planning and organizational performance
  {% else %}
  - Provide foundational account management guidance
  {% endif %}
parameters:
  - key: user_role
    input_type: string
    requirement: required
    description: A test parameter
"#;

        let result = validate_recipe_template_from_content(recipe_content, None);
        if let Err(e) = &result {
            eprintln!("Validation error: {}", e);
            eprintln!("Error chain:");
            let mut source = e.source();
            while let Some(err) = source {
                eprintln!("  Caused by: {}", err);
                source = err.source();
            }
        }
        assert!(result.is_ok(), "Validation failed: {:?}", result.err());

        let recipe = result.unwrap();
        assert_eq!(recipe.title, "Test Recipe");
        assert_eq!(recipe.description, "A test recipe for validation");
        assert!(recipe.instructions.is_some());
        println!("Recipe: {:?}", recipe.prompt);
    }

    // ── response.json_schema validation (bug-sweep wave 1) ──────────────────
    //
    // Every rejected shape here previously panicked at RUN time when the
    // final-output tool was constructed (or, for `true`, on every turn) —
    // feeding the panic breaker and crash-cycling the daemon on a scheduled
    // recipe. Rejecting at authoring time puts the error in the recipe editor.

    #[test]
    fn response_schema_boolean_true_rejected() {
        let err = validate_response_json_schema(&serde_json::json!(true)).unwrap_err();
        assert!(err.to_string().contains("must be a JSON object"));
    }

    #[test]
    fn response_schema_empty_object_rejected() {
        let err = validate_response_json_schema(&serde_json::json!({})).unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn response_schema_garbage_type_rejected() {
        // `type: 42` violates the JSON Schema meta-schema.
        let result = validate_response_json_schema(&serde_json::json!({"type": 42}));
        assert!(result.is_err());
    }

    #[test]
    fn response_schema_non_object_shapes_rejected() {
        for bad in [
            serde_json::json!(false),
            serde_json::json!(null),
            serde_json::json!([1, 2]),
            serde_json::json!("schema"),
            serde_json::json!(7),
        ] {
            let err = validate_response_json_schema(&bad).unwrap_err();
            assert!(
                err.to_string().contains("must be a JSON object"),
                "{:?} → {}",
                bad,
                err
            );
        }
    }

    #[test]
    fn response_schema_valid_object_accepted() {
        validate_response_json_schema(&serde_json::json!({
            "type": "object",
            "properties": {"answer": {"type": "string"}},
            "required": ["answer"]
        }))
        .unwrap();
    }

    fn recipe_with_response(response_yaml: &str) -> String {
        format!(
            r#"
version: 1.0.0
title: Schema Test
description: response schema validation
instructions: do the thing
{}
"#,
            response_yaml
        )
    }

    #[test]
    fn recipe_with_missing_json_schema_in_response_rejected() {
        let content = recipe_with_response("response: {}");
        let err = validate_recipe_template_from_content(&content, None).unwrap_err();
        assert!(
            err.to_string().contains("requires a `json_schema`"),
            "{}",
            err
        );
    }

    #[test]
    fn recipe_with_boolean_json_schema_rejected() {
        let content = recipe_with_response("response:\n  json_schema: true");
        let err = validate_recipe_template_from_content(&content, None).unwrap_err();
        assert!(err.to_string().contains("must be a JSON object"), "{}", err);
    }

    #[test]
    fn recipe_with_empty_object_json_schema_rejected() {
        let content = recipe_with_response("response:\n  json_schema: {}");
        let err = validate_recipe_template_from_content(&content, None).unwrap_err();
        assert!(err.to_string().contains("empty"), "{}", err);
    }

    #[test]
    fn recipe_with_garbage_json_schema_rejected() {
        let content = recipe_with_response("response:\n  json_schema:\n    type: 42");
        assert!(validate_recipe_template_from_content(&content, None).is_err());
    }

    #[test]
    fn recipe_with_valid_json_schema_accepted() {
        let content = recipe_with_response(
            "response:\n  json_schema:\n    type: object\n    properties:\n      answer:\n        type: string",
        );
        let recipe = validate_recipe_template_from_content(&content, None).unwrap();
        assert!(recipe.response.is_some());
    }

    #[test]
    fn recipe_without_response_block_still_accepted() {
        let content = recipe_with_response("");
        assert!(validate_recipe_template_from_content(&content, None).is_ok());
    }
}
