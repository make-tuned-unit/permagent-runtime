use anyhow::{anyhow, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::paths::Paths;
use crate::recipe::read_recipe_file_content::{read_recipe_file, RecipeFile};
use crate::recipe::Recipe;
use crate::recipe::RECIPE_FILE_EXTENSIONS;
use crate::workspace_trust::WorkspaceTrustStore;

const GOOSE_RECIPE_PATH_ENV_VAR: &str = "GOOSE_RECIPE_PATH";

pub fn get_recipe_library_dir(is_global: bool) -> PathBuf {
    if is_global {
        Paths::config_dir().join("recipes")
    } else {
        env::current_dir().unwrap().join(".goose/recipes")
    }
}

/// What a directory contributed to recipe discovery.
///
/// Empty and untrusted-with-recipes are different states: an untrusted clone
/// that ships recipes must be reported so the user can trust it, not swallowed
/// as "nothing here".
#[derive(Debug, Clone)]
pub enum RecipeDirInspection {
    Loaded(Vec<(PathBuf, Recipe)>),
    Untrusted { dir: PathBuf, recipe_count: usize },
    Empty,
}

impl RecipeDirInspection {
    pub fn loaded_recipes(&self) -> &[(PathBuf, Recipe)] {
        match self {
            Self::Loaded(recipes) => recipes,
            Self::Untrusted { .. } | Self::Empty => &[],
        }
    }
}

#[derive(Debug, Clone)]
pub struct UntrustedRecipeLocation {
    pub dir: PathBuf,
    pub recipe_count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct LocalRecipeDiscovery {
    pub recipes: Vec<(PathBuf, Recipe)>,
    pub untrusted: Vec<UntrustedRecipeLocation>,
}

fn local_recipe_dirs() -> Vec<PathBuf> {
    let mut local_dirs = vec![PathBuf::from(".")];

    if let Ok(recipe_path_env) = env::var(GOOSE_RECIPE_PATH_ENV_VAR) {
        let path_separator = if cfg!(windows) { ';' } else { ':' };
        local_dirs.extend(recipe_path_env.split(path_separator).map(PathBuf::from));
    }
    local_dirs.push(get_recipe_library_dir(true));
    local_dirs.push(get_recipe_library_dir(false));

    // Also scan .agents/recipes/ for consistency with the .agents/ convention
    if let Ok(cwd) = env::current_dir() {
        local_dirs.push(cwd.join(".agents/recipes"));
    }
    if let Some(home) = dirs::home_dir() {
        local_dirs.push(home.join(".agents/recipes"));
    }

    let mut dirs: Vec<PathBuf> = local_dirs
        .into_iter()
        .map(|dir| dir.canonicalize().unwrap_or(dir))
        .collect();
    dirs.sort();
    dirs.dedup();
    dirs
}

fn recipe_file_names_in(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if !dir.exists() || !dir.is_dir() {
        return files;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if let Some(extension) = path.extension() {
            if RECIPE_FILE_EXTENSIONS.contains(&extension.to_string_lossy().as_ref()) {
                files.push(path);
            }
        }
    }
    files
}

/// Inspect a single directory under the workspace-trust gate.
///
/// Untrusted directories with recipe files are reported, not loaded and not
/// treated as empty.
pub fn inspect_recipe_dir(dir: &Path, store: &WorkspaceTrustStore) -> RecipeDirInspection {
    let files = recipe_file_names_in(dir);
    if files.is_empty() {
        return RecipeDirInspection::Empty;
    }
    if !store.allows_active_config(dir) {
        return RecipeDirInspection::Untrusted {
            dir: dir.to_path_buf(),
            recipe_count: files.len(),
        };
    }
    RecipeDirInspection::Loaded(load_recipe_files(&files))
}

fn load_recipe_files(files: &[PathBuf]) -> Vec<(PathBuf, Recipe)> {
    let mut recipes = Vec::new();
    for path in files {
        match Recipe::from_file_path(path) {
            Ok(recipe) => recipes.push((path.clone(), recipe)),
            Err(e) => {
                tracing::error!("Failed to load recipe from file {}: {}", path.display(), e);
            }
        }
    }
    recipes
}

pub fn discover_local_recipes_with(store: &WorkspaceTrustStore) -> Result<LocalRecipeDiscovery> {
    discover_from_dirs(local_recipe_dirs(), store)
}

pub fn discover_from_dirs(
    dirs: impl IntoIterator<Item = PathBuf>,
    store: &WorkspaceTrustStore,
) -> Result<LocalRecipeDiscovery> {
    let mut discovery = LocalRecipeDiscovery::default();
    for dir in dirs {
        match inspect_recipe_dir(&dir, store) {
            RecipeDirInspection::Loaded(recipes) => discovery.recipes.extend(recipes),
            RecipeDirInspection::Untrusted { dir, recipe_count } => {
                tracing::warn!(
                    dir = %dir.display(),
                    recipe_count,
                    "Recipes found in an untrusted directory; cloning a repository is not consent to run them. Trust this workspace to load them."
                );
                discovery
                    .untrusted
                    .push(UntrustedRecipeLocation { dir, recipe_count });
            }
            RecipeDirInspection::Empty => {}
        }
    }
    Ok(discovery)
}

pub fn discover_local_recipes() -> Result<LocalRecipeDiscovery> {
    discover_local_recipes_with(&WorkspaceTrustStore::default_store())
}

pub fn load_local_recipe_file(recipe_name: &str) -> Result<RecipeFile> {
    load_local_recipe_file_with(recipe_name, &WorkspaceTrustStore::default_store())
}

fn untrusted_recipe_message(recipe_name: &str, dir: &Path) -> String {
    format!(
        "Recipe '{recipe_name}' was found in {} but that directory is not trusted. \
         Cloning a repository is not consent to run its recipes. Trust this workspace first.",
        dir.display()
    )
}

pub fn load_local_recipe_file_with(
    recipe_name: &str,
    store: &WorkspaceTrustStore,
) -> Result<RecipeFile> {
    if RECIPE_FILE_EXTENSIONS
        .iter()
        .any(|ext| recipe_name.ends_with(&format!(".{}", ext)))
    {
        let path = PathBuf::from(recipe_name);
        let file = read_recipe_file(path)?;
        if !store.allows_active_config(&file.parent_dir) {
            return Err(anyhow!(untrusted_recipe_message(
                recipe_name,
                &file.parent_dir
            )));
        }
        return Ok(file);
    }

    if is_file_path(recipe_name) || is_file_name(recipe_name) {
        return Err(anyhow!(
            "Recipe file {} is not a json or yaml file",
            recipe_name
        ));
    }

    let search_dirs = local_recipe_dirs();
    let mut found_untrusted: Option<PathBuf> = None;
    for dir in &search_dirs {
        if !recipe_exists_in_dir(dir, recipe_name) {
            continue;
        }
        if !store.allows_active_config(dir) {
            found_untrusted = Some(dir.clone());
            continue;
        }
        if let Ok(result) = load_recipe_file_from_dir(dir, recipe_name) {
            return Ok(result);
        }
    }

    if let Some(dir) = found_untrusted {
        return Err(anyhow!(untrusted_recipe_message(recipe_name, &dir)));
    }

    let search_dirs_str = search_dirs
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(":");
    Err(anyhow!(
        "ℹ️  Failed to retrieve {}.yaml or {}.json in {}",
        recipe_name,
        recipe_name,
        search_dirs_str
    ))
}

pub fn list_local_recipes() -> Result<Vec<(PathBuf, Recipe)>> {
    Ok(discover_local_recipes()?.recipes)
}

fn is_file_path(recipe_name: &str) -> bool {
    recipe_name.contains('/')
        || recipe_name.contains('\\')
        || recipe_name.starts_with('~')
        || recipe_name.starts_with('.')
}

fn is_file_name(recipe_name: &str) -> bool {
    Path::new(recipe_name).extension().is_some()
}

fn recipe_exists_in_dir(dir: &Path, recipe_name: &str) -> bool {
    RECIPE_FILE_EXTENSIONS
        .iter()
        .any(|ext| dir.join(format!("{}.{}", recipe_name, ext)).is_file())
}

fn load_recipe_file_from_dir(dir: &Path, recipe_name: &str) -> Result<RecipeFile> {
    for ext in RECIPE_FILE_EXTENSIONS {
        let recipe_path = dir.join(format!("{}.{}", recipe_name, ext));
        if let Ok(result) = read_recipe_file(recipe_path) {
            return Ok(result);
        }
    }
    Err(anyhow!(format!(
        "No {}.yaml or {}.json recipe file found in directory: {}",
        recipe_name,
        recipe_name,
        dir.display()
    )))
}

fn generate_recipe_filename(title: &str, recipe_library_dir: &Path) -> PathBuf {
    let base_name = title
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '-')
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join("-");

    let filename = if base_name.is_empty() {
        "untitled-recipe".to_string()
    } else {
        base_name
    };

    let mut candidate = recipe_library_dir.join(format!("{}.yaml", filename));
    if !candidate.exists() {
        return candidate;
    }

    let mut counter = 1;
    loop {
        candidate = recipe_library_dir.join(format!("{}-{}.yaml", filename, counter));
        if !candidate.exists() {
            return candidate;
        }
        counter += 1;
    }
}

pub fn save_recipe_to_file(recipe: Recipe, file_path: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    let recipe_library_dir = get_recipe_library_dir(true);

    let file_path_value = match file_path {
        Some(path) => path,
        None => generate_recipe_filename(&recipe.title, &recipe_library_dir),
    };

    if let Some(parent) = file_path_value.parent() {
        fs::create_dir_all(parent)?;
    }

    let yaml_content = recipe.to_yaml()?;
    fs::write(&file_path_value, yaml_content)?;
    Ok(file_path_value)
}
