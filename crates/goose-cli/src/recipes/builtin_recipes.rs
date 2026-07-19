//! Recipes shipped inside the Permagent CLI binary.
//!
//! A built-in recipe resolves by a reserved name from
//! [`crate::recipes::search_recipe::load_recipe_file`] — after any local recipe
//! of the same name (so operators can override it) and before the GitHub
//! fallback. This lets a fully-configured coding session launch with
//! `permagent run --recipe permagent-coding --interactive` on any install,
//! with no filesystem setup and no GitHub recipe repo configured. The Build
//! tab's "Permagent" option runs exactly that command in a terminal, the same
//! way its "Claude" / "Codex" options run those CLIs.

use std::path::PathBuf;

use permagent::recipe::read_recipe_file_content::RecipeFile;

/// Reserved name for the Permagent coding-harness recipe.
pub const PERMAGENT_CODING_RECIPE_NAME: &str = "permagent-coding";

/// The `title` the built-in coding-harness recipe declares (see
/// `builtin/permagent-coding.yaml`). Session-build uses it to detect the harness
/// — so it, and only it, receives the auto-injected repo-map context. Kept in
/// sync with the YAML by `title_constant_matches_embedded_recipe`.
pub const PERMAGENT_CODING_RECIPE_TITLE: &str = "Permagent Coding Harness";

/// The coding-harness recipe, embedded at compile time so it ships in the
/// binary rather than depending on a file on disk.
const PERMAGENT_CODING_RECIPE: &str = include_str!("builtin/permagent-coding.yaml");

/// Whether `recipe` is the built-in Permagent coding harness. Used to gate the
/// repo-map context injection to the coding loop (rather than every CLI session).
pub fn is_coding_harness_recipe(recipe: &permagent::recipe::Recipe) -> bool {
    recipe.title == PERMAGENT_CODING_RECIPE_TITLE
}

/// Resolve a built-in recipe by name, accepting either the bare reserved name
/// (`permagent-coding`) or a `<name>.yaml` form. Returns `None` for any name
/// that is not built in, so callers can fall through to other resolvers.
pub fn builtin_recipe_file(recipe_name: &str) -> Option<RecipeFile> {
    let stem = PathBuf::from(recipe_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| recipe_name.to_string());

    let content = match stem.as_str() {
        PERMAGENT_CODING_RECIPE_NAME => PERMAGENT_CODING_RECIPE,
        _ => return None,
    };

    // Built-in recipes are self-contained (no relative sub-recipes or file
    // includes), so `parent_dir` only needs to be a sane cwd for templating.
    let parent_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    Some(RecipeFile {
        content: content.to_string(),
        parent_dir,
        file_path: PathBuf::from(format!("<builtin>/{PERMAGENT_CODING_RECIPE_NAME}.yaml")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_recipe_resolves_by_bare_and_yaml_name() {
        assert!(builtin_recipe_file(PERMAGENT_CODING_RECIPE_NAME).is_some());
        assert!(builtin_recipe_file("permagent-coding.yaml").is_some());
    }

    #[test]
    fn unknown_names_are_not_built_in() {
        assert!(builtin_recipe_file("definitely-not-a-builtin").is_none());
    }

    #[test]
    fn title_constant_matches_embedded_recipe() {
        // The gate constant must stay in sync with the recipe YAML's title, or
        // the coding harness would silently stop getting its repo-map injection.
        assert!(
            PERMAGENT_CODING_RECIPE.contains(PERMAGENT_CODING_RECIPE_TITLE),
            "PERMAGENT_CODING_RECIPE_TITLE ({PERMAGENT_CODING_RECIPE_TITLE:?}) not found in the embedded recipe"
        );
    }
}
