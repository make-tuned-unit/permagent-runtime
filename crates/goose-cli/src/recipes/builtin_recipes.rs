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

/// Every built-in recipe, as `RecipeInfo` rows for `recipe list`. Without
/// these, `permagent recipe list` printed "No recipes found" on a clean
/// install even though `--recipe permagent-coding` works — the one command
/// someone runs to discover the harness name hid it.
pub fn builtin_recipe_infos() -> Vec<crate::recipes::github_recipe::RecipeInfo> {
    use crate::recipes::github_recipe::{RecipeInfo, RecipeSource};
    vec![RecipeInfo {
        name: PERMAGENT_CODING_RECIPE_NAME.to_string(),
        source: RecipeSource::Builtin,
        path: format!("<builtin>/{PERMAGENT_CODING_RECIPE_NAME}.yaml"),
        title: Some(PERMAGENT_CODING_RECIPE_TITLE.to_string()),
        description: Some(
            "The Permagent coding harness — a fully configured coding session.".to_string(),
        ),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_infos_are_listed_and_resolvable() {
        let infos = builtin_recipe_infos();
        assert!(!infos.is_empty(), "built-ins must be discoverable via list");
        for info in infos {
            assert!(
                builtin_recipe_file(&info.name).is_some(),
                "listed built-in {} must resolve",
                info.name
            );
        }
    }

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

    /// Observed 2026-09-06: `permagent run --recipe permagent-coding` logged
    /// `Failed to start extension 'council'` and continued without it. The
    /// recipe asked for a builtin named `council`, but the Council's registry
    /// key is deliberately `deliberate` (see
    /// `crates/goose/src/agents/platform_extensions/council.rs`'s module
    /// docs: it does not collide with the `council` worker id in the
    /// self-knowledge namespace) — so the lookup by `council` always misses.
    /// This pins every builtin the shipped coding-harness recipe requests
    /// against the actual registry so a future name drift fails the build
    /// instead of only showing up as a silently-skipped extension at runtime.
    #[test]
    fn every_builtin_extension_the_coding_harness_recipe_requests_resolves() {
        use permagent::agents::extension::ExtensionConfig;
        use permagent::agents::platform_extensions::PLATFORM_EXTENSIONS;
        use permagent::config::extensions::name_to_key;
        use permagent::recipe::Recipe;

        let recipe = Recipe::from_content(PERMAGENT_CODING_RECIPE)
            .expect("the embedded coding-harness recipe must parse");
        let extensions = recipe
            .extensions
            .expect("the coding-harness recipe declares extensions");
        let builtin_names: Vec<&str> = extensions
            .iter()
            .filter_map(|ext| match ext {
                ExtensionConfig::Builtin { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            !builtin_names.is_empty(),
            "the recipe must declare at least one builtin extension"
        );
        for name in builtin_names {
            assert!(
                PLATFORM_EXTENSIONS.contains_key(name_to_key(name).as_str()),
                "recipe requests builtin extension '{name}', which is not registered — \
                 it will silently fail to start (\"Failed to start extension '{name}'\") \
                 and the recipe's promised tools will be missing"
            );
        }
    }
}
