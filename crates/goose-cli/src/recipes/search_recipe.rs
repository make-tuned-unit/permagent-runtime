use anyhow::Result;
use permagent::config::Config;
use permagent::recipe::read_recipe_file_content::RecipeFile;

use super::github_recipe::{
    list_github_recipes, retrieve_recipe_from_github, RecipeInfo, RecipeSource,
    GOOSE_RECIPE_GITHUB_REPO_CONFIG_KEY,
};
use permagent::recipe::local_recipes::{discover_local_recipes, load_local_recipe_file};

use super::builtin_recipes::builtin_recipe_file;

pub fn load_recipe_file(recipe_name: &str) -> Result<RecipeFile> {
    // Resolution order: a local recipe of this name wins (operators can
    // override), then Permagent's built-in recipes (e.g. the coding harness),
    // then the configured GitHub recipe repo.
    load_local_recipe_file(recipe_name)
        .or_else(|local_err| builtin_recipe_file(recipe_name).ok_or(local_err))
        .or_else(|err| {
            if let Some(recipe_repo_full_name) = configured_github_recipe_repo() {
                retrieve_recipe_from_github(recipe_name, &recipe_repo_full_name)
            } else {
                Err(err)
            }
        })
}

fn configured_github_recipe_repo() -> Option<String> {
    let config = Config::global();
    match config.get_param(GOOSE_RECIPE_GITHUB_REPO_CONFIG_KEY) {
        Ok(Some(recipe_repo_full_name)) => Some(recipe_repo_full_name),
        _ => None,
    }
}

/// Lists all available recipes from local paths and GitHub repositories
pub fn list_available_recipes() -> Result<Vec<RecipeInfo>> {
    let mut recipes = Vec::new();

    // Search local recipes
    if let Ok(discovery) = discover_local_recipes() {
        recipes.extend(discovery.recipes.into_iter().map(|(path, recipe)| {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            RecipeInfo {
                name,
                source: RecipeSource::Local,
                path: path.display().to_string(),
                title: Some(recipe.title),
                description: Some(recipe.description),
            }
        }));
    }

    // Search GitHub recipes if configured
    if let Some(repo) = configured_github_recipe_repo() {
        if let Ok(github_recipes) = list_github_recipes(&repo) {
            recipes.extend(github_recipes);
        }
    }

    Ok(recipes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipes::builtin_recipes::PERMAGENT_CODING_RECIPE_NAME;
    use permagent::recipe::template_recipe::parse_recipe_content;

    /// The Build tab's third option runs
    /// `permagent run --recipe permagent-coding --interactive`. This proves that
    /// command resolves to a real coding configuration of the internal loop with
    /// no filesystem setup and no GitHub repo — i.e. the option is not a dead
    /// button but a mounted seam onto the internal agent loop.
    #[test]
    fn builtin_permagent_coding_recipe_resolves_and_parses() {
        let rf = load_recipe_file(PERMAGENT_CODING_RECIPE_NAME)
            .expect("built-in coding recipe should resolve by name");

        // Pins the coding toolset explicitly: developer = edit + search + shell
        // + tree; analyze = the code-structure map; summon = the subagent seam
        // for tiered cost-routing (delegate mechanical sub-work to a cheaper tier).
        assert!(
            rf.content.contains("name: developer"),
            "coding harness must enable the developer extension"
        );
        assert!(
            rf.content.contains("name: analyze"),
            "coding harness must enable the analyze extension"
        );
        assert!(
            rf.content.contains("name: summon"),
            "coding harness must enable the summon extension for tiered subagent routing"
        );

        // Parses into a valid recipe with a coding system prompt and three
        // pinned extensions.
        let (recipe, _) =
            parse_recipe_content(&rf.content, None).expect("built-in coding recipe should parse");
        assert_eq!(recipe.title, "Permagent Coding Harness");
        assert!(
            recipe.instructions.is_some(),
            "coding harness must set a system prompt"
        );
        assert_eq!(
            recipe.extensions.map(|e| e.len()).unwrap_or_default(),
            3,
            "coding harness pins developer + analyze + summon"
        );
    }
}
