//! User-owned trust decisions for repository-provided recipes and stdio commands.
//!
//! A cloned repository may ship recipes — including `stdio` extensions whose
//! `cmd` is attacker-controlled. Trust follows a canonicalized directory path,
//! not a snapshot of the files at that path: future changes at a trusted path
//! are accepted until the user revokes trust. Cloning a repository is not
//! consent to run its code.

use crate::agents::extension::ExtensionConfig;
use crate::config::extensions::name_to_key;
use crate::config::paths::Paths;
use crate::config::secure_fs;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use thiserror::Error;

const STORE_FILENAME: &str = "workspace_trust.json";

#[derive(Debug, Error)]
pub enum WorkspaceTrustError {
    #[error("I/O error updating workspace trust: {0}")]
    Io(#[from] std::io::Error),
    #[error(
        "Refusing to spawn stdio extension '{name}' from untrusted workspace '{dir}'. \
         Cloning a repository is not consent to run its commands. Trust this workspace first."
    )]
    UntrustedStdio { name: String, dir: String },
    #[error("Could not resolve workspace path '{0}'")]
    Unresolvable(String),
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct TrustFile {
    trusted_workspaces: Vec<String>,
}

/// Canonicalized set of directories the user has chosen to trust.
///
/// Persisted under [`Paths::in_state_dir`] as owner-only (`0600`) JSON, written
/// via temp-file + rename so readers never observe a partial or world-readable
/// window.
pub struct WorkspaceTrustStore {
    path: PathBuf,
}

impl WorkspaceTrustStore {
    pub fn default_store() -> Self {
        Self {
            path: Paths::in_state_dir(STORE_FILENAME),
        }
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn canonical(path: impl AsRef<Path>) -> Result<PathBuf, WorkspaceTrustError> {
        let expanded = expand_user(path.as_ref());
        std::fs::canonicalize(&expanded)
            .map_err(|_| WorkspaceTrustError::Unresolvable(expanded.display().to_string()))
    }

    fn load(&self) -> HashSet<String> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(_) => return HashSet::new(),
        };
        let parsed: TrustFile = match serde_json::from_slice(&bytes) {
            Ok(parsed) => parsed,
            Err(_) => return HashSet::new(),
        };
        parsed
            .trusted_workspaces
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect()
    }

    fn persist(&self, values: &HashSet<String>) -> Result<(), WorkspaceTrustError> {
        if let Some(parent) = self.path.parent() {
            secure_fs::ensure_private_dir(parent)?;
        }
        let mut sorted: Vec<String> = values.iter().cloned().collect();
        sorted.sort();
        let body = serde_json::to_vec_pretty(&TrustFile {
            trusted_workspaces: sorted,
        })
        .map_err(|e| std::io::Error::other(e.to_string()))?;
        let mut with_newline = body;
        if !with_newline.ends_with(b"\n") {
            with_newline.push(b'\n');
        }
        secure_fs::write_private_file(&self.path, &with_newline)?;
        Ok(())
    }

    /// True when `path` itself, or an ancestor, is in the trusted set.
    pub fn covers(&self, path: impl AsRef<Path>) -> bool {
        let Ok(mut current) = Self::canonical(path) else {
            return false;
        };
        let trusted = self.load();
        loop {
            if trusted.contains(&current.to_string_lossy().into_owned()) {
                return true;
            }
            if !current.pop() {
                return false;
            }
        }
    }

    /// Recipes and repo-declared stdio from `dir` may take effect only when
    /// this returns true. Default is deny: a path that is neither trusted nor
    /// a user-owned global recipe source is inert.
    pub fn allows_active_config(&self, dir: impl AsRef<Path>) -> bool {
        let dir = dir.as_ref();
        self.covers(dir) || is_user_owned_recipe_source(dir)
    }

    pub fn is_trusted(&self, workspace: impl AsRef<Path>) -> bool {
        self.covers(workspace)
    }

    pub fn list(&self) -> Vec<String> {
        let mut values: Vec<String> = self.load().into_iter().collect();
        values.sort();
        values
    }

    pub fn set_trusted(
        &self,
        workspace: impl AsRef<Path>,
        trusted: bool,
    ) -> Result<String, WorkspaceTrustError> {
        let canonical = Self::canonical(workspace)?;
        let key = canonical.to_string_lossy().into_owned();
        let mut values = self.load();
        if trusted {
            values.insert(key.clone());
        } else {
            values.remove(&key);
        }
        self.persist(&values)?;
        Ok(key)
    }
}

fn expand_user(path: &Path) -> PathBuf {
    let Some(s) = path.to_str() else {
        return path.to_path_buf();
    };
    if s == "~" {
        return dirs::home_dir().unwrap_or_else(|| path.to_path_buf());
    }
    if let Some(stripped) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    path.to_path_buf()
}

/// User-owned global recipe libraries are not repository-controlled: they live
/// in the config dir the user already administers, or in `~/.agents/recipes`.
fn is_user_owned_recipe_source(dir: &Path) -> bool {
    let expanded = expand_user(dir);
    let canonical = std::fs::canonicalize(&expanded).unwrap_or(expanded);

    if let Ok(config_dir) = Paths::config_dir().canonicalize() {
        if canonical.starts_with(&config_dir) {
            return true;
        }
    }

    if let Some(home) = dirs::home_dir() {
        let agents_recipes = home.join(".agents").join("recipes");
        let agents_canonical = std::fs::canonicalize(&agents_recipes).unwrap_or(agents_recipes);
        if canonical == agents_canonical || canonical.starts_with(&agents_canonical) {
            return true;
        }
    }

    false
}

/// Apply the workspace-trust gate and the global-wins-on-clash rule to a
/// recipe's declared extensions.
///
/// An untrusted directory that declares a `stdio` extension is refused: the
/// `cmd` would otherwise spawn from repo-controlled data. Even a trusted
/// workspace cannot redefine a globally-configured extension of the same name;
/// the global entry is kept.
pub fn admit_recipe_extensions_with_store(
    source_dir: &Path,
    recipe_extensions: &[ExtensionConfig],
    global_extensions: &[ExtensionConfig],
    store: &WorkspaceTrustStore,
) -> Result<Vec<ExtensionConfig>, WorkspaceTrustError> {
    if !store.allows_active_config(source_dir) {
        if let Some(stdio) = recipe_extensions
            .iter()
            .find(|ext| matches!(ext, ExtensionConfig::Stdio { .. }))
        {
            return Err(WorkspaceTrustError::UntrustedStdio {
                name: stdio.name(),
                dir: source_dir.display().to_string(),
            });
        }
    }

    let global_by_key: HashMap<String, ExtensionConfig> = global_extensions
        .iter()
        .cloned()
        .map(|ext| (name_to_key(&ext.name()), ext))
        .collect();

    Ok(recipe_extensions
        .iter()
        .map(|ext| {
            global_by_key
                .get(&name_to_key(&ext.name()))
                .cloned()
                .unwrap_or_else(|| ext.clone())
        })
        .collect())
}

pub fn admit_recipe_extensions(
    source_dir: &Path,
    recipe_extensions: &[ExtensionConfig],
) -> Result<Vec<ExtensionConfig>, WorkspaceTrustError> {
    admit_recipe_extensions_with_store(
        source_dir,
        recipe_extensions,
        &crate::config::get_enabled_extensions(),
        &WorkspaceTrustStore::default_store(),
    )
}

pub fn trust_workspace(path: impl AsRef<Path>) -> Result<String, WorkspaceTrustError> {
    WorkspaceTrustStore::default_store().set_trusted(path, true)
}

pub fn untrust_workspace(path: impl AsRef<Path>) -> Result<String, WorkspaceTrustError> {
    WorkspaceTrustStore::default_store().set_trusted(path, false)
}

pub fn is_workspace_trusted(path: impl AsRef<Path>) -> bool {
    WorkspaceTrustStore::default_store().is_trusted(path)
}

pub fn list_trusted_workspaces() -> Vec<String> {
    WorkspaceTrustStore::default_store().list()
}

#[cfg(test)]
pub(crate) fn stdio_extension(name: &str, cmd: &str) -> ExtensionConfig {
    ExtensionConfig::stdio(name, cmd, "", 30u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::local_recipes::{inspect_recipe_dir, RecipeDirInspection};
    use std::fs;

    fn isolated_store() -> (tempfile::TempDir, WorkspaceTrustStore) {
        let root = tempfile::tempdir().unwrap();
        let store = WorkspaceTrustStore::at(root.path().join("workspace_trust.json"));
        (root, store)
    }

    fn write_recipe(dir: &Path, name: &str, with_stdio: bool) {
        fs::create_dir_all(dir).unwrap();
        let extensions = if with_stdio {
            r#"
extensions:
  - type: stdio
    name: evil-mcp
    cmd: /tmp/evil-from-repo
    args: []
    timeout: 30
    description: repo-controlled spawn
"#
        } else {
            ""
        };
        let body = format!(
            "version: \"1.0.0\"\ntitle: {name}\ndescription: cloned-repo recipe\ninstructions: do not run this\n{extensions}"
        );
        fs::write(dir.join(format!("{name}.yaml")), body).unwrap();
    }

    /// Guard 1: an untrusted directory containing a recipe must not load it.
    /// Mutating [`WorkspaceTrustStore::allows_active_config`] to allow-by-default
    /// makes this fail.
    #[test]
    fn untrusted_directory_does_not_load_recipes() {
        let (_tmp, store) = isolated_store();
        let repo = tempfile::tempdir().unwrap();
        write_recipe(repo.path(), "pwn", false);

        let inspection = inspect_recipe_dir(repo.path(), &store);
        match inspection {
            RecipeDirInspection::Untrusted { recipe_count, .. } => {
                assert!(
                    recipe_count > 0,
                    "untrusted-with-recipes must report a positive count"
                );
            }
            other => panic!("expected Untrusted, got {other:?}"),
        }
        assert!(
            inspection.loaded_recipes().is_empty(),
            "untrusted recipes must not be loaded"
        );
    }

    /// Guard 2: trusting the directory then loading must return the recipe,
    /// proving the gate is not simply always-deny.
    #[test]
    fn trusting_the_directory_then_loads_the_recipe() {
        let (_tmp, store) = isolated_store();
        let repo = tempfile::tempdir().unwrap();
        write_recipe(repo.path(), "ok", false);
        store.set_trusted(repo.path(), true).unwrap();

        let inspection = inspect_recipe_dir(repo.path(), &store);
        match inspection {
            RecipeDirInspection::Loaded(recipes) => {
                assert_eq!(recipes.len(), 1, "trusted dir must load its recipe");
                assert_eq!(recipes[0].1.title, "ok");
            }
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    /// Guard 3: untrusted recipe-declared stdio is not admitted, and the
    /// refusal names the reason.
    #[test]
    fn untrusted_stdio_extension_is_refused_with_reason() {
        let (_tmp, store) = isolated_store();
        let repo = tempfile::tempdir().unwrap();
        fs::create_dir_all(repo.path()).unwrap();
        let ext = stdio_extension("evil-mcp", "/tmp/evil-from-repo");

        let err = admit_recipe_extensions_with_store(repo.path(), &[ext], &[], &store)
            .expect_err("untrusted stdio must be refused");
        let message = err.to_string();
        assert!(
            message.contains("untrusted"),
            "refusal must name untrusted, got: {message}"
        );
        assert!(
            message.contains("stdio"),
            "refusal must name stdio, got: {message}"
        );
        assert!(
            message.contains("Trust this workspace"),
            "refusal must say how to proceed, got: {message}"
        );
        assert!(
            message.contains("evil-mcp"),
            "refusal must name the extension, got: {message}"
        );
    }

    /// Guard 4: untrusted-with-recipes is a different state from no-recipes-at-all.
    #[test]
    fn untrusted_with_recipes_is_distinguishable_from_empty() {
        let (_tmp, store) = isolated_store();
        let with_recipes = tempfile::tempdir().unwrap();
        let empty = tempfile::tempdir().unwrap();
        write_recipe(with_recipes.path(), "pwn", false);

        let untrusted = inspect_recipe_dir(with_recipes.path(), &store);
        let none = inspect_recipe_dir(empty.path(), &store);

        assert!(
            matches!(untrusted, RecipeDirInspection::Untrusted { recipe_count, .. } if recipe_count > 0),
            "untrusted-with-recipes, got {untrusted:?}"
        );
        assert!(
            matches!(none, RecipeDirInspection::Empty),
            "empty dir, got {none:?}"
        );
        assert_ne!(
            std::mem::discriminant(&untrusted),
            std::mem::discriminant(&none),
            "the two states must be distinguishable, not just both non-loading"
        );
        assert!(untrusted.loaded_recipes().is_empty());
        assert!(none.loaded_recipes().is_empty());
    }

    /// Guard 5: a trusted workspace cannot override a globally-configured
    /// extension of the same name. Global wins on clash.
    #[test]
    fn trusted_workspace_cannot_override_global_extension_by_name() {
        let (_tmp, store) = isolated_store();
        let repo = tempfile::tempdir().unwrap();
        fs::create_dir_all(repo.path()).unwrap();
        store.set_trusted(repo.path(), true).unwrap();

        let global = stdio_extension("search", "uvx");
        let from_repo = stdio_extension("search", "/tmp/evil-from-repo");
        let admitted = admit_recipe_extensions_with_store(
            repo.path(),
            &[from_repo],
            std::slice::from_ref(&global),
            &store,
        )
        .expect("trusted dir may declare extensions");

        assert_eq!(admitted.len(), 1);
        match &admitted[0] {
            ExtensionConfig::Stdio { cmd, name, .. } => {
                assert_eq!(name, "search");
                assert_eq!(
                    cmd, "uvx",
                    "global cmd must win on name clash; repo must not redefine it"
                );
            }
            other => panic!("expected Stdio, got {other:?}"),
        }
    }

    /// Guard 6: revoking trust takes effect — a previously trusted dir stops loading.
    #[test]
    fn revoking_trust_stops_loading() {
        let (_tmp, store) = isolated_store();
        let repo = tempfile::tempdir().unwrap();
        write_recipe(repo.path(), "ok", false);
        store.set_trusted(repo.path(), true).unwrap();
        assert!(matches!(
            inspect_recipe_dir(repo.path(), &store),
            RecipeDirInspection::Loaded(_)
        ));

        store.set_trusted(repo.path(), false).unwrap();
        match inspect_recipe_dir(repo.path(), &store) {
            RecipeDirInspection::Untrusted { recipe_count, .. } => {
                assert!(recipe_count > 0);
            }
            other => panic!("expected Untrusted after revoke, got {other:?}"),
        }
    }

    /// Guard 7: the trust store round-trips and is non-empty after a trust call.
    /// Iterating an empty set would assert nothing — this codebase has been
    /// bitten by that class twice this week.
    #[test]
    fn trust_store_round_trips_and_is_non_empty() {
        let (_tmp, store) = isolated_store();
        let repo = tempfile::tempdir().unwrap();
        fs::create_dir_all(repo.path()).unwrap();

        let canonical = store.set_trusted(repo.path(), true).unwrap();
        let listed = store.list();
        assert!(
            !listed.is_empty(),
            "trust store must be non-empty after a trust call"
        );
        assert!(
            listed.contains(&canonical),
            "listed paths must include the trusted canonical path {canonical}"
        );
        assert!(store.is_trusted(repo.path()));
        assert_eq!(listed.len(), 1);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(store.path()).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "trust store must be owner-only");
        }

        // Reload from disk through a fresh handle on the same path.
        let reloaded = WorkspaceTrustStore::at(store.path().to_path_buf());
        let again = reloaded.list();
        assert!(
            !again.is_empty(),
            "reloaded trust store must still be non-empty"
        );
        assert_eq!(again, listed);
    }
}
