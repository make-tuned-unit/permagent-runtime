// Extension data management for sessions
// Provides a simple way to store extension-specific data with versioned keys

use crate::config::base::Config;
use crate::config::extensions::is_extension_available;
use crate::config::ExtensionConfig;
use crate::session::SessionManager;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use utoipa::ToSchema;

/// Extension data containing all extension states
/// Keys are in format "extension_name.version" (e.g., "todo.v0")
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema)]
pub struct ExtensionData {
    #[serde(flatten)]
    pub extension_states: HashMap<String, Value>,
}

impl ExtensionData {
    /// Create a new empty ExtensionData
    pub fn new() -> Self {
        Self {
            extension_states: HashMap::new(),
        }
    }

    /// Get extension state for a specific extension and version
    pub fn get_extension_state(&self, extension_name: &str, version: &str) -> Option<&Value> {
        let key = format!("{}.{}", extension_name, version);
        self.extension_states.get(&key)
    }

    /// Set extension state for a specific extension and version
    pub fn set_extension_state(&mut self, extension_name: &str, version: &str, state: Value) {
        let key = format!("{}.{}", extension_name, version);
        self.extension_states.insert(key, state);
    }
}

/// Helper trait for extension-specific state management
pub trait ExtensionState: Sized + Serialize + for<'de> Deserialize<'de> {
    /// The name of the extension
    const EXTENSION_NAME: &'static str;

    /// The version of the extension state format
    const VERSION: &'static str;

    /// Convert from JSON value
    fn from_value(value: &Value) -> Result<Self> {
        serde_json::from_value(value.clone()).map_err(|e| {
            anyhow::anyhow!(
                "Failed to deserialize {} state: {}",
                Self::EXTENSION_NAME,
                e
            )
        })
    }

    /// Convert to JSON value
    fn to_value(&self) -> Result<Value> {
        serde_json::to_value(self).map_err(|e| {
            anyhow::anyhow!("Failed to serialize {} state: {}", Self::EXTENSION_NAME, e)
        })
    }

    /// Get state from extension data
    fn from_extension_data(extension_data: &ExtensionData) -> Option<Self> {
        extension_data
            .get_extension_state(Self::EXTENSION_NAME, Self::VERSION)
            .and_then(|v| Self::from_value(v).ok())
    }

    /// Save state to extension data
    fn to_extension_data(&self, extension_data: &mut ExtensionData) -> Result<()> {
        let value = self.to_value()?;
        extension_data.set_extension_state(Self::EXTENSION_NAME, Self::VERSION, value);
        Ok(())
    }
}

/// TODO extension state implementation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoState {
    pub content: String,
}

impl ExtensionState for TodoState {
    const EXTENSION_NAME: &'static str = "todo";
    const VERSION: &'static str = "v0";
}

impl TodoState {
    /// Create a new TODO state
    pub fn new(content: String) -> Self {
        Self { content }
    }
}

/// Enabled extensions state implementation for storing which extensions are active
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnabledExtensionsState {
    pub extensions: Vec<ExtensionConfig>,
}

impl ExtensionState for EnabledExtensionsState {
    const EXTENSION_NAME: &'static str = "enabled_extensions";
    const VERSION: &'static str = "v0";
}

impl EnabledExtensionsState {
    pub fn new(extensions: Vec<ExtensionConfig>) -> Self {
        Self { extensions }
    }

    pub fn from_extension_data(extension_data: &ExtensionData) -> Option<Self> {
        let mut state = <Self as ExtensionState>::from_extension_data(extension_data)?;
        state.extensions.retain(is_extension_available);
        Some(state)
    }

    pub fn extensions_or_default(
        extension_data: Option<&ExtensionData>,
        config: &Config,
    ) -> Vec<ExtensionConfig> {
        extension_data
            .and_then(Self::from_extension_data)
            .map(|state| {
                let mut extensions = state.extensions;
                // Additive refresh (2026-07-28, widened 2026-07-31): the stored
                // set is a snapshot from session creation, so a session that
                // predates an extension never gained its tools — the agent then
                // truthfully told the user "I have no create_person tool" in a
                // months-old chat.
                //
                // Originally this merged PLATFORM extensions only, holding MCP
                // servers back for their auth/process cost. That cost turned out
                // to be bounded — extensions load for the session being USED, not
                // for every historical session — while the gap was not: enabling
                // Brave/Tavily in Settings never reached the running chat, so the
                // agent searched the open web by hand and then insisted the
                // extensions were disabled while they were enabled with valid
                // keys. Config is the user's live intent; honor it for every
                // enabled extension, not just the platform ones.
                let known: std::collections::HashSet<String> =
                    extensions.iter().map(|e| e.name().to_string()).collect();
                for cfg in crate::config::extensions::get_enabled_extensions_with_config(config) {
                    if !known.contains(&cfg.name().to_string()) {
                        extensions.push(cfg);
                    }
                }
                extensions
            })
            .unwrap_or_else(|| {
                crate::config::extensions::get_enabled_extensions_with_config(config)
            })
    }

    pub async fn for_session(
        session_manager: &SessionManager,
        session_id: &str,
        config: &Config,
    ) -> Vec<ExtensionConfig> {
        let session = session_manager.get_session(session_id, false).await.ok();
        Self::extensions_or_default(session.as_ref().map(|s| &s.extension_data), config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::NamedTempFile;
    use test_case::test_case;

    fn test_config() -> Config {
        let config_file = NamedTempFile::new().unwrap();
        let secrets_file = NamedTempFile::new().unwrap();
        Config::new_with_file_secrets(config_file.path(), secrets_file.path()).unwrap()
    }

    fn test_extension() -> ExtensionConfig {
        ExtensionConfig::Builtin {
            name: "developer".into(),
            description: "dev".into(),
            display_name: None,
            timeout: None,
            bundled: None,
            available_tools: vec![],
        }
    }

    fn extension_data_with(extensions: Vec<ExtensionConfig>) -> ExtensionData {
        let mut data = ExtensionData::new();
        EnabledExtensionsState::new(extensions)
            .to_extension_data(&mut data)
            .unwrap();
        data
    }

    #[test_case(None, None ; "no_session_falls_back_to_config")]
    #[test_case(Some(ExtensionData::default()), None ; "empty_session_data_falls_back_to_config")]
    fn test_extensions_or_default(
        extension_data: Option<ExtensionData>,
        expected: Option<Vec<ExtensionConfig>>,
    ) {
        let config = test_config();
        let expected = expected.unwrap_or_else(|| {
            crate::config::extensions::get_enabled_extensions_with_config(&config)
        });
        assert_eq!(
            EnabledExtensionsState::extensions_or_default(extension_data.as_ref(), &config),
            expected,
        );
    }

    /// The stored session set is preserved (and stays first), and every
    /// config-enabled extension missing from the snapshot is merged in
    /// additively — the fix for old sessions never gaining newly enabled tools.
    #[test]
    fn session_data_keeps_stored_set_and_merges_missing_enabled_extensions() {
        let config = test_config();
        let stored = vec![test_extension()];
        let data = extension_data_with(stored.clone());
        let result = EnabledExtensionsState::extensions_or_default(Some(&data), &config);

        // Stored entries survive, in order, at the front.
        assert_eq!(result[0], stored[0]);
        // Every added entry is an extension enabled in config…
        let config_enabled: std::collections::HashSet<String> =
            crate::config::extensions::get_enabled_extensions_with_config(&config)
                .into_iter()
                .map(|e| e.name().to_string())
                .collect();
        for added in &result[1..] {
            assert!(config_enabled.contains(&added.name().to_string()));
        }
        // …and no name appears twice (idempotent merge).
        let mut seen = std::collections::HashSet::new();
        for e in &result {
            assert!(seen.insert(e.name().to_string()), "duplicate {}", e.name());
        }
    }

    /// Regression: an MCP server the user enables in Settings AFTER a session
    /// started must reach that session. Holding stdio servers back meant saving
    /// a Brave/Tavily key enabled search everywhere except the chat the user was
    /// sitting in, and the agent reported search as unavailable.
    #[test]
    fn a_config_enabled_mcp_server_reaches_a_session_that_predates_it() {
        let config = test_config();
        let stored = vec![test_extension()];
        let data = extension_data_with(stored.clone());

        // The user enables a stdio search server after this session was created.
        // Written straight into the test config (the setter helpers target the
        // process-global config, which a unit test must not touch).
        let search = ExtensionConfig::Stdio {
            name: "Brave Search".into(),
            description: "Web search".into(),
            cmd: "/usr/bin/true".into(),
            args: vec![],
            envs: Default::default(),
            env_keys: vec!["BRAVE_API_KEY".into()],
            timeout: None,
            bundled: None,
            available_tools: vec![],
        };
        let mut map = serde_json::Map::new();
        map.insert(
            "bravesearch".to_string(),
            serde_json::to_value(crate::config::extensions::ExtensionEntry {
                enabled: true,
                config: search.clone(),
            })
            .unwrap(),
        );
        config
            .set_param("extensions", serde_json::Value::Object(map))
            .unwrap();

        let result = EnabledExtensionsState::extensions_or_default(Some(&data), &config);
        assert_eq!(result[0], stored[0], "the stored set still comes first");
        assert!(
            result.iter().any(|e| e.name() == search.name()),
            "a newly enabled MCP server must join a session that predates it"
        );
    }

    #[test]
    fn test_extension_data_basic_operations() {
        let mut extension_data = ExtensionData::new();

        // Test setting and getting extension state
        let todo_state = json!({"content": "- Task 1\n- Task 2"});
        extension_data.set_extension_state("todo", "v0", todo_state.clone());

        assert_eq!(
            extension_data.get_extension_state("todo", "v0"),
            Some(&todo_state)
        );
        assert_eq!(extension_data.get_extension_state("todo", "v1"), None);
    }

    #[test]
    fn test_multiple_extension_states() {
        let mut extension_data = ExtensionData::new();

        // Add multiple extension states
        extension_data.set_extension_state("todo", "v0", json!("TODO content"));
        extension_data.set_extension_state("memory", "v1", json!({"items": ["item1", "item2"]}));
        extension_data.set_extension_state("config", "v2", json!({"setting": true}));

        // Check all states exist
        assert_eq!(extension_data.extension_states.len(), 3);
        assert!(extension_data.get_extension_state("todo", "v0").is_some());
        assert!(extension_data.get_extension_state("memory", "v1").is_some());
        assert!(extension_data.get_extension_state("config", "v2").is_some());
    }

    #[test]
    fn test_todo_state_trait() {
        let mut extension_data = ExtensionData::new();

        // Create and save TODO state
        let todo = TodoState::new("- Task 1\n- Task 2".to_string());
        todo.to_extension_data(&mut extension_data).unwrap();

        // Retrieve TODO state
        let retrieved = TodoState::from_extension_data(&extension_data);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().content, "- Task 1\n- Task 2");
    }

    #[test]
    fn test_extension_data_serialization() {
        let mut extension_data = ExtensionData::new();
        extension_data.set_extension_state("todo", "v0", json!("TODO content"));
        extension_data.set_extension_state("memory", "v1", json!({"key": "value"}));

        // Serialize to JSON
        let json = serde_json::to_value(&extension_data).unwrap();

        // Check the structure
        assert!(json.is_object());
        assert_eq!(json.get("todo.v0"), Some(&json!("TODO content")));
        assert_eq!(json.get("memory.v1"), Some(&json!({"key": "value"})));

        // Deserialize back
        let deserialized: ExtensionData = serde_json::from_value(json).unwrap();
        assert_eq!(
            deserialized.get_extension_state("todo", "v0"),
            Some(&json!("TODO content"))
        );
        assert_eq!(
            deserialized.get_extension_state("memory", "v1"),
            Some(&json!({"key": "value"}))
        );
    }

    #[test]
    fn test_enabled_extensions_state_filters_unavailable_platform() {
        let mut extension_data = ExtensionData::new();
        let state = EnabledExtensionsState::new(vec![
            ExtensionConfig::Platform {
                name: "definitely_not_real_platform_extension".to_string(),
                description: "unknown".to_string(),
                display_name: None,
                bundled: None,
                available_tools: Vec::new(),
            },
            ExtensionConfig::Builtin {
                name: "developer".to_string(),
                description: "".to_string(),
                display_name: Some("Developer".to_string()),
                timeout: None,
                bundled: None,
                available_tools: Vec::new(),
            },
        ]);

        state.to_extension_data(&mut extension_data).unwrap();

        let loaded =
            EnabledExtensionsState::from_extension_data(&extension_data).expect("state present");
        let names: Vec<String> = loaded.extensions.iter().map(|ext| ext.name()).collect();

        assert!(names.iter().any(|name| name == "developer"));
        assert!(!names
            .iter()
            .any(|name| name == "definitely_not_real_platform_extension"));
    }
}
