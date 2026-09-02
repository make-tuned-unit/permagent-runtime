use permagent::config::declarative_providers::load_provider;
use permagent::config::Config;
use permagent::providers::base::{ConfigKey, ProviderMetadata, ProviderType};
use serde::{Deserialize, Serialize};
use std::env;
use std::error::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum KeyLocation {
    Environment,
    ConfigFile,
    Keychain,
    NotFound,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyInfo {
    pub name: String,
    pub is_set: bool,
    pub location: KeyLocation,
    pub is_secret: bool,
    pub value: Option<String>, // Only populated for non-secret keys that are set
}

/// Inspects a configuration key to determine if it's set, its location, and value (for non-secret keys)
#[allow(dead_code)]
pub fn inspect_key(key_name: &str, is_secret: bool) -> Result<KeyInfo, Box<dyn Error>> {
    let config = Config::global();

    // Check environment variable first
    let env_value = env::var(key_name).ok();

    if let Some(value) = env_value {
        return Ok(KeyInfo {
            name: key_name.to_string(),
            is_set: true,
            location: KeyLocation::Environment,
            is_secret,
            // Only include value for non-secret keys
            value: if !is_secret { Some(value) } else { None },
        });
    }

    // Check config store
    let config_result = if is_secret {
        config.get_secret(key_name).map(|v| (v, true))
    } else {
        config.get_param(key_name).map(|v| (v, false))
    };

    match config_result {
        Ok((value, is_secret_actual)) => {
            // Determine location based on whether it's a secret value
            let location = if is_secret_actual {
                KeyLocation::Keychain
            } else {
                KeyLocation::ConfigFile
            };

            Ok(KeyInfo {
                name: key_name.to_string(),
                is_set: true,
                location,
                is_secret: is_secret_actual,
                // Only include value for non-secret keys
                value: if !is_secret_actual { Some(value) } else { None },
            })
        }
        Err(_) => Ok(KeyInfo {
            name: key_name.to_string(),
            is_set: false,
            location: KeyLocation::NotFound,
            is_secret,
            value: None,
        }),
    }
}

/// Inspects multiple keys at once
#[allow(dead_code)]
pub fn inspect_keys(
    keys: &[(String, bool)], // (name, is_secret) pairs
) -> Result<Vec<KeyInfo>, Box<dyn Error>> {
    let mut results = Vec::new();

    for (key_name, is_secret) in keys {
        let info = inspect_key(key_name, *is_secret)?;
        results.push(info);
    }

    Ok(results)
}

pub fn check_provider_configured(metadata: &ProviderMetadata, provider_type: ProviderType) -> bool {
    let config = Config::global();

    // Special override
    if metadata.name == "local" {
        return true;
    }

    if provider_type == ProviderType::Custom || provider_type == ProviderType::Declarative {
        if let Ok(loaded_provider) = load_provider(metadata.name.as_str()) {
            if !loaded_provider.config.requires_auth {
                return true;
            }

            if !loaded_provider.config.api_key_env.is_empty() {
                let api_key_result =
                    config.get_secret::<String>(&loaded_provider.config.api_key_env);
                if api_key_result.is_ok() {
                    return true;
                }
            }

            // A hand-made custom provider is connected by existence ONLY when it
            // names no api_key_env — header/env_var auth leaves nothing for the
            // check above to find, so the file itself is the only signal. Once a
            // provider declares an api_key_env AND requires_auth, the check above
            // is meaningful and an unconditional `true` here would override it,
            // reporting a provider with no stored key as connected.
            //
            // Mirrored EXACTLY from
            // `permagent::providers::configured::is_provider_configured`. The two
            // must not diverge — a split here is what made a provider read
            // "connected" in one picker and "not configured" in another.
            return provider_type == ProviderType::Custom
                && loaded_provider.config.api_key_env.is_empty();
        }
    }

    // Special case: OAuth providers - check for configured marker
    let has_oauth_key = metadata.config_keys.iter().any(|key| key.oauth_flow);
    if has_oauth_key {
        let configured_marker = format!("{}_configured", metadata.name);
        if matches!(config.get_param::<bool>(&configured_marker), Ok(true)) {
            return true;
        }
    }

    // Special case: Zero-config providers (no config keys)
    if metadata.config_keys.is_empty() {
        // Check if the provider has been explicitly configured via the UI
        let configured_marker = format!("{}_configured", metadata.name);
        return config.get_param::<bool>(&configured_marker).is_ok();
    }

    // Get all required keys
    let required_keys: Vec<&ConfigKey> = metadata
        .config_keys
        .iter()
        .filter(|key| key.required)
        .collect();

    // Special case: If a provider has exactly one required key and that key
    // has a default value, check if it's explicitly set
    if required_keys.len() == 1 && required_keys[0].default.is_some() {
        let key = &required_keys[0];

        // Check if the key is explicitly set (either in env or config)
        let is_set_in_env = env::var(&key.name).is_ok();
        let is_set_in_config = config.get(&key.name, key.secret).is_ok();

        return is_set_in_env || is_set_in_config;
    }

    // Special case: If a provider has only optional keys with defaults,
    // check if a configuration marker exists
    if required_keys.is_empty() && !metadata.config_keys.is_empty() {
        let all_optional_with_defaults = metadata
            .config_keys
            .iter()
            .all(|key| !key.required && key.default.is_some());

        if all_optional_with_defaults {
            // Check if the provider has been explicitly configured via the UI
            let configured_marker = format!("{}_configured", metadata.name);
            return config.get_param::<bool>(&configured_marker).is_ok();
        }
    }

    // For providers with multiple keys or keys without defaults:
    // Find required keys that don't have default values
    let required_non_default_keys: Vec<&ConfigKey> = required_keys
        .iter()
        .filter(|key| key.default.is_none())
        .cloned()
        .collect();

    // If there are no non-default keys, this provider needs at least one key explicitly set
    if required_non_default_keys.is_empty() {
        let any_required_explicit = required_keys.iter().any(|key| {
            let is_set_in_env = env::var(&key.name).is_ok();
            let is_set_in_config = config.get(&key.name, key.secret).is_ok();

            is_set_in_env || is_set_in_config
        });
        if any_required_explicit {
            return true;
        }
        // OpenAI (and similar) mark the API key optional so local compatible
        // hosts work without auth — but Settings still needs "saved key ⇒
        // configured". Count optional *_API_KEY secrets here.
        return metadata.config_keys.iter().any(|key| {
            key.secret
                && !key.required
                && key.name.ends_with("_API_KEY")
                && (env::var(&key.name).is_ok() || config.get(&key.name, true).is_ok())
        });
    }

    // Otherwise, all non-default keys must be set
    required_non_default_keys.iter().all(|key| {
        let is_set_in_env = env::var(&key.name).is_ok();
        let is_set_in_config = config.get(&key.name, key.secret).is_ok();

        is_set_in_env || is_set_in_config
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use permagent::providers::configured::is_provider_configured;

    /// Write a custom-provider definition to disk and return matching metadata.
    ///
    /// The file has to actually exist: with nothing there `load_provider` returns
    /// `Err`, the custom/declarative branch never runs, and the assertions below
    /// would hold for reasons unrelated to the branch they are guarding.
    fn custom_provider_on_disk(name: &str, api_key_env: &str) -> ProviderMetadata {
        crate::test_support::test_root();
        let dir = permagent::config::declarative_providers::custom_providers_dir();
        std::fs::create_dir_all(&dir).expect("custom_providers dir");
        let json = serde_json::json!({
            "name": name,
            "engine": "openai",
            "display_name": name,
            "api_key_env": api_key_env,
            "base_url": "https://example.invalid",
            "models": [],
            "requires_auth": true,
        });
        std::fs::write(
            dir.join(format!("{name}.json")),
            serde_json::to_string(&json).expect("serialize provider"),
        )
        .expect("write provider json");

        // No config keys and no `<name>_configured` marker, so every path after
        // the custom/declarative branch is false. A `true` came from that branch.
        ProviderMetadata::new(name, name, "", "", vec![], "", vec![])
    }

    /// This route's check and `providers::configured::is_provider_configured` are
    /// deliberate near-copies. They have drifted before — a provider read
    /// "connected" in one picker and "not configured" in another — so assert the
    /// two agree on the custom-provider hatch rather than trusting the comments.
    #[test]
    fn the_custom_provider_hatch_matches_the_shared_configured_check() {
        let key = "PERMAGENT_TEST_DAEMON_CUSTOM_DECLARED_KEY";
        let declared = custom_provider_on_disk("permagent-test-daemon-declared", key);
        let headers = custom_provider_on_disk("permagent-test-daemon-headers", "");

        for meta in [&declared, &headers] {
            assert_eq!(
                check_provider_configured(meta, ProviderType::Custom),
                is_provider_configured(meta, ProviderType::Custom),
                "{} disagrees with the shared configured check",
                meta.name
            );
        }

        assert!(
            !check_provider_configured(&declared, ProviderType::Custom),
            "a custom provider that requires auth and names an api_key_env is not \
             connected until that key is stored"
        );
        assert!(
            check_provider_configured(&headers, ProviderType::Custom),
            "a custom provider with no api_key_env authenticates via headers/env \
             vars and stays connected by existence"
        );

        Config::global()
            .set_secret(key, &"sk-test".to_string())
            .expect("store secret");
        assert!(check_provider_configured(&declared, ProviderType::Custom));
        assert_eq!(
            check_provider_configured(&declared, ProviderType::Custom),
            is_provider_configured(&declared, ProviderType::Custom),
        );
        Config::global().delete_secret(key).ok();
    }
}
