use std::collections::HashMap;
use std::fs;
use std::hash::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;

use crate::routes::errors::ErrorResponse;
use crate::state::AppState;
use anyhow::Result;
use axum::http::StatusCode;
use permagent::agents::Agent;
use permagent::recipe::build_recipe::{
    build_recipe_from_template, resolve_sub_recipe_path, RecipeError,
};
use permagent::recipe::local_recipes::{
    discover_local_recipes, get_recipe_library_dir, LocalRecipeDiscovery, UntrustedRecipeLocation,
};
use permagent::recipe::validate_recipe::validate_recipe_template_from_content;
use permagent::recipe::Recipe;
use serde::Serialize;
use tracing::error;
use utoipa::ToSchema;

pub struct RecipeValidationError {
    pub status: StatusCode,
    pub message: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecipeManifest {
    pub id: String,
    pub recipe: Recipe,
    #[schema(value_type = String)]
    pub file_path: PathBuf,
    pub last_modified: String,
    pub schedule_cron: Option<String>,
    pub slash_command: Option<String>,
}

pub fn short_id_from_path(path: &str) -> String {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    let h = hasher.finish();
    format!("{:016x}", h)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UntrustedRecipeDir {
    #[schema(value_type = String)]
    pub dir: PathBuf,
    pub recipe_count: usize,
    pub message: String,
}

pub fn get_all_recipes_manifests() -> Result<Vec<RecipeManifest>> {
    Ok(discover_recipe_manifests()?.0)
}

pub fn discover_recipe_manifests() -> Result<(Vec<RecipeManifest>, Vec<UntrustedRecipeDir>)> {
    let LocalRecipeDiscovery { recipes, untrusted } = discover_local_recipes()?;
    let mut recipe_manifests_with_path = Vec::new();
    for (file_path, mut recipe) in recipes {
        // `modified()` can fail independently of `metadata()` (platform /
        // filesystem dependent) — chain it instead of unwrapping inside the
        // map, which escaped the surrounding `let Ok … else` and panicked.
        let Ok(last_modified) = fs::metadata(&file_path)
            .and_then(|m| m.modified())
            .map(|m| chrono::DateTime::<chrono::Utc>::from(m).to_rfc3339())
        else {
            continue;
        };

        if let Some(recipe_dir) = file_path.parent() {
            if let Some(ref mut sub_recipes) = recipe.sub_recipes {
                for sr in sub_recipes.iter_mut() {
                    if let Ok(resolved) = resolve_sub_recipe_path(&sr.path, recipe_dir) {
                        sr.path = resolved;
                    }
                }
            }
        }

        let manifest_with_path = RecipeManifest {
            id: short_id_from_path(file_path.to_string_lossy().as_ref()),
            recipe,
            file_path,
            last_modified,
            schedule_cron: None,
            slash_command: None,
        };
        recipe_manifests_with_path.push(manifest_with_path);
    }
    recipe_manifests_with_path.sort_by(|a, b| b.last_modified.cmp(&a.last_modified));

    let untrusted_dirs = untrusted
        .into_iter()
        .map(
            |UntrustedRecipeLocation { dir, recipe_count }| UntrustedRecipeDir {
                message: format!(
                    "Found {recipe_count} recipe(s) in {} but that directory is not trusted. \
                     Cloning a repository is not consent to run them. Trust this workspace to load them.",
                    dir.display()
                ),
                dir,
                recipe_count,
            },
        )
        .collect();

    Ok((recipe_manifests_with_path, untrusted_dirs))
}

pub fn validate_recipe(recipe: &Recipe) -> Result<(), RecipeValidationError> {
    let recipe_yaml = recipe.to_yaml().map_err(|err| {
        let message = err.to_string();
        error!("Failed to serialize recipe for validation: {}", message);
        RecipeValidationError {
            status: StatusCode::BAD_REQUEST,
            message,
        }
    })?;

    validate_recipe_template_from_content(&recipe_yaml, None).map_err(|err| {
        let message = err.to_string();
        error!("Recipe validation failed: {}", message);
        RecipeValidationError {
            status: StatusCode::BAD_REQUEST,
            message,
        }
    })?;

    Ok(())
}

pub async fn get_recipe_file_path_by_id(
    state: &AppState,
    id: &str,
) -> Result<PathBuf, ErrorResponse> {
    let cached_path = {
        let map = state.recipe_file_hash_map.lock().await;
        map.get(id).cloned()
    };

    if let Some(path) = cached_path {
        return Ok(path);
    }

    let recipe_manifest_with_paths = get_all_recipes_manifests().unwrap_or_default();
    let mut recipe_file_hash_map = HashMap::new();
    let mut resolved_path: Option<PathBuf> = None;

    for recipe_manifest_with_path in &recipe_manifest_with_paths {
        if recipe_manifest_with_path.id == id {
            resolved_path = Some(recipe_manifest_with_path.file_path.clone());
        }
        recipe_file_hash_map.insert(
            recipe_manifest_with_path.id.clone(),
            recipe_manifest_with_path.file_path.clone(),
        );
    }

    state.set_recipe_file_hash_map(recipe_file_hash_map).await;

    resolved_path.ok_or_else(|| ErrorResponse {
        message: format!("Recipe not found: {}", id),
        status: StatusCode::NOT_FOUND,
    })
}

pub async fn load_recipe_by_id(state: &AppState, id: &str) -> Result<Recipe, ErrorResponse> {
    let path = get_recipe_file_path_by_id(state, id).await?;

    if let Some(parent) = path.parent() {
        if !permagent::workspace_trust::WorkspaceTrustStore::default_store()
            .allows_active_config(parent)
        {
            return Err(ErrorResponse {
                message: format!(
                    "Recipe at {} is in an untrusted directory. Cloning a repository is not consent to run its recipes. Trust this workspace first.",
                    parent.display()
                ),
                status: StatusCode::FORBIDDEN,
            });
        }
    }

    let mut recipe = Recipe::from_file_path(&path).map_err(|err| ErrorResponse {
        message: format!("Failed to load recipe: {}", err),
        status: StatusCode::INTERNAL_SERVER_ERROR,
    })?;

    if let Some(recipe_dir) = path.parent() {
        if let Some(ref mut sub_recipes) = recipe.sub_recipes {
            for sr in sub_recipes.iter_mut() {
                if let Ok(resolved) = resolve_sub_recipe_path(&sr.path, recipe_dir) {
                    sr.path = resolved;
                }
            }
        }
    }

    Ok(recipe)
}

pub async fn build_recipe_with_parameter_values(
    original_recipe: &Recipe,
    user_recipe_values: HashMap<String, String>,
) -> Result<Option<Recipe>> {
    let recipe_content = original_recipe.to_yaml()?;

    let recipe_dir = get_recipe_library_dir(true);
    let params = user_recipe_values.into_iter().collect();

    let recipe = match build_recipe_from_template(
        recipe_content,
        &recipe_dir,
        params,
        None::<fn(&str, &str) -> Result<String, anyhow::Error>>,
    ) {
        Ok(recipe) => Some(recipe),
        Err(RecipeError::MissingParams { .. }) => None,
        Err(e) => return Err(anyhow::anyhow!(e)),
    };

    Ok(recipe)
}

/// Apply recipe components (final-output tool) to the agent and return the
/// recipe instructions. A recipe whose `response.json_schema` is invalid is a
/// 400 — previously this panicked the daemon at run time.
pub async fn apply_recipe_to_agent(
    agent: &Arc<Agent>,
    recipe: &Recipe,
    include_final_output_tool: bool,
) -> Result<Option<String>, ErrorResponse> {
    agent
        .apply_recipe_components(recipe.response.clone(), include_final_output_tool)
        .await
        .map_err(|err| {
            error!("Recipe response schema rejected: {}", err);
            ErrorResponse {
                message: err.to_string(),
                status: StatusCode::BAD_REQUEST,
            }
        })?;

    Ok(recipe.instructions.as_ref().cloned())
}
