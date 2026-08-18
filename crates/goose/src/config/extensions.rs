use super::base::Config;
use crate::agents::extension::PLATFORM_EXTENSIONS;
use crate::agents::ExtensionConfig;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_yaml::Mapping;
use std::collections::HashSet;
use tracing::warn;
use utoipa::ToSchema;

pub const DEFAULT_EXTENSION: &str = "developer";
pub const DEFAULT_EXTENSION_TIMEOUT: u64 = 300;
pub const DEFAULT_EXTENSION_DESCRIPTION: &str = "";
pub const DEFAULT_DISPLAY_NAME: &str = "Developer";
const EXTENSIONS_CONFIG_KEY: &str = "extensions";

#[derive(Debug, Deserialize, Serialize, Clone, ToSchema)]
pub struct ExtensionEntry {
    pub enabled: bool,
    #[serde(flatten)]
    pub config: ExtensionConfig,
}

pub fn name_to_key(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    for c in name.chars() {
        result.push(match c {
            c if c.is_ascii_alphanumeric() || c == '_' || c == '-' => c,
            c if c.is_whitespace() => continue,
            _ => '_',
        });
    }
    result.to_lowercase()
}

pub(crate) fn is_extension_available(config: &ExtensionConfig) -> bool {
    match config {
        ExtensionConfig::Platform { name, .. } => {
            PLATFORM_EXTENSIONS.contains_key(name_to_key(name).as_str())
        }
        _ => true,
    }
}

fn get_extensions_map_with_config(config: &Config) -> IndexMap<String, ExtensionEntry> {
    let raw: Mapping = config
        .get_param(EXTENSIONS_CONFIG_KEY)
        .unwrap_or_else(|err| {
            warn!(
                "Failed to load {}: {err}. Falling back to empty object.",
                EXTENSIONS_CONFIG_KEY
            );
            Default::default()
        });

    let mut extensions_map = IndexMap::with_capacity(raw.len());
    for (k, v) in raw {
        match (k, serde_yaml::from_value::<ExtensionEntry>(v)) {
            (serde_yaml::Value::String(key), Ok(entry)) => {
                if !is_extension_available(&entry.config) {
                    continue;
                }
                extensions_map.insert(key, entry);
            }
            (k, v) => {
                warn!(
                    key = ?k,
                    value = ?v,
                    "Skipping malformed extension config entry"
                );
            }
        }
    }

    extensions_map
}

fn get_extensions_map() -> IndexMap<String, ExtensionEntry> {
    get_extensions_map_with_config(Config::global())
}

fn save_extensions_map(extensions: IndexMap<String, ExtensionEntry>) {
    let config = Config::global();
    if let Err(e) = config.set_param(EXTENSIONS_CONFIG_KEY, &extensions) {
        tracing::warn!("Failed to save extensions config: {}", e);
    }
}

pub fn get_extension_by_name(name: &str) -> Option<ExtensionConfig> {
    let extensions = get_extensions_map();
    extensions
        .values()
        .find(|entry| entry.config.name() == name)
        .map(|entry| entry.config.clone())
}

pub fn set_extension(entry: ExtensionEntry) {
    let mut extensions = get_extensions_map();
    let key = entry.config.key();
    extensions.insert(key, entry);
    save_extensions_map(extensions);
}

pub fn remove_extension(key: &str) {
    let mut extensions = get_extensions_map();
    extensions.shift_remove(key);
    save_extensions_map(extensions);
}

pub fn set_extension_enabled(key: &str, enabled: bool) {
    let mut extensions = get_extensions_map();
    if let Some(entry) = extensions.get_mut(key) {
        entry.enabled = enabled;
        save_extensions_map(extensions);
    }
}

pub fn get_all_extensions() -> Vec<ExtensionEntry> {
    let extensions = get_extensions_map();
    extensions.into_values().collect()
}

pub fn get_all_extension_names() -> Vec<String> {
    let extensions = get_extensions_map();
    extensions.keys().cloned().collect()
}

pub fn is_extension_enabled(key: &str) -> bool {
    let extensions = get_extensions_map();
    extensions.get(key).map(|e| e.enabled).unwrap_or(false)
}

/// True when `key` may be granted to an agent at all. Refusing unknown or
/// globally-disabled keys at the write boundary prevents the roster from
/// claiming a permission that runtime resolution would silently discard.
pub fn extension_is_grantable(key: &str) -> bool {
    is_extension_enabled(key)
}

pub fn get_enabled_extensions() -> Vec<ExtensionConfig> {
    get_all_extensions()
        .into_iter()
        .filter(|ext| ext.enabled)
        .map(|ext| ext.config)
        .collect()
}

pub fn get_enabled_extensions_with_config(config: &Config) -> Vec<ExtensionConfig> {
    get_extensions_map_with_config(config)
        .into_values()
        .filter(|ext| ext.enabled)
        .map(|ext| ext.config)
        .collect()
}

pub fn get_warnings() -> Vec<String> {
    let raw: Mapping = Config::global()
        .get_param(EXTENSIONS_CONFIG_KEY)
        .unwrap_or_default();

    let mut warnings = Vec::new();
    for (k, v) in raw {
        if let (serde_yaml::Value::String(key), Ok(entry)) =
            (k, serde_yaml::from_value::<ExtensionEntry>(v))
        {
            if matches!(entry.config, ExtensionConfig::Sse { .. }) {
                warnings.push(format!(
                    "'{}': SSE is unsupported, migrate to streamable_http",
                    key
                ));
            }
        }
    }
    warnings
}

pub fn resolve_extensions_for_new_session(
    recipe_extensions: Option<&[ExtensionConfig]>,
    override_extensions: Option<Vec<ExtensionConfig>>,
) -> Vec<ExtensionConfig> {
    let extensions = if let Some(exts) = recipe_extensions {
        exts.to_vec()
    } else if let Some(exts) = override_extensions {
        exts
    } else {
        get_enabled_extensions()
    };

    extensions
        .into_iter()
        .filter(is_extension_available)
        .collect()
}

/// Narrow an already-resolved extension set to what an agent is granted.
/// `None` returns `base` verbatim — the pre-grant behaviour, unchanged, so
/// an agent.yaml with no grants dispatches exactly as it did before. An
/// explicit empty list grants nothing.
///
/// Narrowing is a `retain` over the caller's own set, so the result is a
/// subset by construction: a grant naming an extension the run never had
/// cannot manufacture it, and a globally-disabled extension cannot be
/// revived for one agent. That is what stops a grant being a
/// privilege-escalation seam.
pub fn narrow_extensions_for_agent(
    mut base: Vec<ExtensionConfig>,
    grants: Option<&[String]>,
) -> Vec<ExtensionConfig> {
    let Some(grants) = grants else {
        return base;
    };

    let granted: HashSet<&str> = grants.iter().map(String::as_str).collect();
    base.retain(|config| granted.contains(config.key().as_str()));
    base
}

/// Resolves the ordinary per-run extension set, then narrows it to the agent's
/// grants.
pub fn resolve_extensions_for_agent(
    grants: Option<&[String]>,
    recipe_extensions: Option<&[ExtensionConfig]>,
    override_extensions: Option<Vec<ExtensionConfig>>,
) -> Vec<ExtensionConfig> {
    let base = resolve_extensions_for_new_session(recipe_extensions, override_extensions);
    narrow_extensions_for_agent(base, grants)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The config-sync path asks the extension manager "is `Brave Search`
    /// loaded?" while the manager keys everything by `name_to_key`. If those two
    /// ever disagree, a newly-enabled extension looks missing forever and gets
    /// re-added on every reply — or looks present and never gets added at all.
    ///
    /// These are the real display names and the real keys from a config where a
    /// resident session could not reach either search provider (2026-08-13).
    #[test]
    fn display_names_normalise_to_the_keys_config_stores_them_under() {
        for (display, key) in [
            ("Brave Search", "bravesearch"),
            ("Tavily Web Search", "tavilywebsearch"),
            ("Extension Manager", "extensionmanager"),
            ("Computer Controller", "computercontroller"),
        ] {
            assert_eq!(name_to_key(display), key, "display name {display:?}");
        }
    }

    /// Keying is idempotent: a key fed back through normalisation is unchanged.
    /// Without this, a second sync pass could fail to recognise what the first
    /// one registered.
    #[test]
    fn normalising_a_key_again_is_a_no_op() {
        for key in [
            "bravesearch",
            "tavilywebsearch",
            "developer",
            "file_to_project",
        ] {
            assert_eq!(name_to_key(key), key);
        }
    }

    #[test]
    fn test_is_extension_available_filters_unknown_platform() {
        let unknown_platform = ExtensionConfig::Platform {
            name: "definitely_not_real_platform_extension".to_string(),
            description: "unknown".to_string(),
            display_name: None,
            bundled: None,
            available_tools: Vec::new(),
        };

        let builtin = ExtensionConfig::Builtin {
            name: "developer".to_string(),
            description: "".to_string(),
            display_name: Some("Developer".to_string()),
            timeout: None,
            bundled: None,
            available_tools: Vec::new(),
        };

        assert!(!is_extension_available(&unknown_platform));
        assert!(is_extension_available(&builtin));
    }

    fn builtin(name: &str) -> ExtensionConfig {
        ExtensionConfig::Builtin {
            name: name.to_string(),
            description: String::new(),
            display_name: None,
            timeout: None,
            bundled: None,
            available_tools: Vec::new(),
        }
    }

    /// Filtering the already-resolved base is the security boundary: naming an
    /// extension that the run did not receive must never manufacture it.
    #[test]
    fn agent_grant_never_widens_the_base() {
        let resolved = resolve_extensions_for_agent(
            Some(&["bravesearch".to_string()]),
            None,
            Some(vec![builtin("developer")]),
        );
        assert!(resolved.is_empty());
    }

    /// Existing agent files have no grant field, so absence must remain byte-for-
    /// byte equivalent at the resolved configuration boundary.
    #[test]
    fn absent_agent_grants_preserve_session_resolution() {
        let overrides = vec![builtin("developer"), builtin("bravesearch")];
        let expected = resolve_extensions_for_new_session(None, Some(overrides.clone()));
        let actual = resolve_extensions_for_agent(None, None, Some(overrides));
        assert_eq!(actual, expected);
    }

    /// An explicit empty list is an intentional denial and must not be confused
    /// with the absent field's backwards-compatible inheritance semantics.
    #[test]
    fn empty_agent_grants_deny_every_extension() {
        let resolved =
            resolve_extensions_for_agent(Some(&[]), None, Some(vec![builtin("developer")]));
        assert!(resolved.is_empty());
    }

    /// A partial grant must preserve only matching members of the run's base,
    /// keeping both the selection and its order predictable for dispatch.
    #[test]
    fn agent_grants_keep_only_named_base_extensions() {
        let resolved = resolve_extensions_for_agent(
            Some(&["bravesearch".to_string()]),
            None,
            Some(vec![builtin("developer"), builtin("bravesearch")]),
        );
        assert_eq!(resolved, vec![builtin("bravesearch")]);
    }

    /// A dispatch scope composes with worker grants by retaining only their
    /// intersection from the caller's already-resolved extension set.
    #[test]
    fn dispatch_scope_yields_only_granted_extensions() {
        let base = vec![
            builtin("developer"),
            builtin("bravesearch"),
            builtin("browser"),
        ];
        let worker_grants = ["developer".to_string(), "bravesearch".to_string()];
        let dispatch_scope = ["bravesearch".to_string()];
        let resolved = narrow_extensions_for_agent(
            narrow_extensions_for_agent(base, Some(&worker_grants)),
            Some(&dispatch_scope),
        );
        assert_eq!(resolved, vec![builtin("bravesearch")]);
    }

    /// Composed narrowing cannot add a scope member missing from either the
    /// parent set or the worker's grants.
    #[test]
    fn composed_dispatch_scope_never_widens() {
        let base = vec![builtin("developer"), builtin("bravesearch")];
        let worker_grants = ["developer".to_string()];
        let dispatch_scope = ["bravesearch".to_string(), "browser".to_string()];
        let resolved = narrow_extensions_for_agent(
            narrow_extensions_for_agent(base, Some(&worker_grants)),
            Some(&dispatch_scope),
        );
        assert!(resolved.is_empty());
    }

    /// An absent dispatch scope preserves today's worker-grant resolution
    /// byte-for-byte.
    #[test]
    fn absent_dispatch_scope_preserves_agent_resolution() {
        let base = vec![builtin("developer"), builtin("bravesearch")];
        let grants = ["developer".to_string()];
        let expected = narrow_extensions_for_agent(base.clone(), Some(&grants));
        let actual =
            narrow_extensions_for_agent(narrow_extensions_for_agent(base, Some(&grants)), None);
        assert_eq!(actual, expected);
    }

    /// An explicit empty dispatch scope denies the worker every extension.
    #[test]
    fn empty_dispatch_scope_denies_every_extension() {
        let base = vec![builtin("developer"), builtin("bravesearch")];
        let resolved = narrow_extensions_for_agent(base, Some(&[]));
        assert!(resolved.is_empty());
    }
}
