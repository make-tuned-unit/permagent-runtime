//! Shared provider-connection detection for model pickers.
//!
//! One definition of "connected", used by Council seats, `/model`, and the
//! Settings picker. Three near-copies of this had drifted, so a provider that
//! was actively serving traffic could still read "not configured" in one place
//! and "connected" in another.
//!
//! The subtle rule is the `default` split: a `required` key that carries a
//! compiled-in default (`ANTHROPIC_HOST`, `OPENAI_BASE_PATH`, …) is satisfied
//! by that default and must NOT gate "configured" on being written to config.
//! Demanding every `required` key be independently present is what made
//! Anthropic — two required keys, one of them the defaulted host — report as
//! unconfigured whenever only `ANTHROPIC_API_KEY` was stored.

use super::base::{ConfigKey, ProviderMetadata, ProviderType};
use crate::config::declarative_providers::load_provider;
use crate::config::Config;
use std::env;

pub fn is_provider_configured(metadata: &ProviderMetadata, provider_type: ProviderType) -> bool {
    let config = Config::global();
    if metadata.name == "local" {
        return true;
    }
    if matches!(
        provider_type,
        ProviderType::Custom | ProviderType::Declarative
    ) {
        if let Ok(loaded) = load_provider(&metadata.name) {
            if !loaded.config.requires_auth {
                return true;
            }
            if !loaded.config.api_key_env.is_empty()
                && config
                    .get_secret::<String>(&loaded.config.api_key_env)
                    .is_ok()
            {
                return true;
            }
            // A hand-made custom provider is connected by existence ONLY when it
            // names no api_key_env — header/env_var auth leaves nothing for the
            // check above to find, so the file itself is the only signal. Once a
            // provider declares an api_key_env AND requires_auth, the check above
            // is meaningful and an unconditional `true` here would override it,
            // reporting a provider with no stored key as connected: a Council
            // seat and a model-picker row that fail on first use.
            //
            // Mirrored EXACTLY in
            // `crates/goose-server/src/routes/utils.rs::check_provider_configured`.
            // The two must not diverge — a split here is what made a provider
            // read "connected" in one picker and "not configured" in another.
            if provider_type == ProviderType::Custom && loaded.config.api_key_env.is_empty() {
                return true;
            }
        }
    }
    let marker = format!("{}_configured", metadata.name);
    if metadata.config_keys.iter().any(|key| key.oauth_flow)
        && matches!(config.get_param::<bool>(&marker), Ok(true))
    {
        return true;
    }
    if metadata.config_keys.is_empty() {
        return config.get_param::<bool>(&marker).is_ok();
    }
    let required: Vec<&ConfigKey> = metadata
        .config_keys
        .iter()
        .filter(|key| key.required)
        .collect();
    let key_is_set =
        |key: &ConfigKey| env::var(&key.name).is_ok() || config.get(&key.name, key.secret).is_ok();
    if required.len() == 1 && required[0].default.is_some() {
        return key_is_set(required[0]);
    }
    if required.is_empty()
        && metadata
            .config_keys
            .iter()
            .all(|key| !key.required && key.default.is_some())
    {
        return config.get_param::<bool>(&marker).is_ok();
    }
    let no_default: Vec<&ConfigKey> = required
        .iter()
        .filter(|key| key.default.is_none())
        .copied()
        .collect();
    if no_default.is_empty() {
        // Every required key has a compiled fallback, so nothing is strictly
        // mandatory. OpenAI-shaped metadata marks its actual credential
        // `required: false` (local OpenAI-compatible hosts need no auth), so a
        // stored key still has to count as "connected". Key off `primary` —
        // the structural "this is the field we ask the user for" flag already
        // on `ConfigKey` — rather than a `_API_KEY` name suffix, which silently
        // missed any provider that names its secret something else.
        return required.iter().any(|key| key_is_set(key))
            || metadata
                .config_keys
                .iter()
                .any(|key| key.secret && !key.required && key.primary && key_is_set(key));
    }
    no_default.iter().all(|key| key_is_set(key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::declarative_providers::custom_providers_dir;
    use crate::providers::base::ProviderMetadata;

    /// Write a custom-provider definition to the temp `custom_providers` dir the
    /// test ctor pins `PERMAGENT_PATH_ROOT` at, and return matching metadata.
    ///
    /// Writing the file is the whole point: with nothing on disk `load_provider`
    /// returns `Err` and the custom/declarative branch never runs, so a test that
    /// skips this step passes no matter what the branch says.
    fn custom_provider_on_disk(name: &str, api_key_env: &str) -> ProviderMetadata {
        let dir = custom_providers_dir();
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

        // No config keys, and no `<name>_configured` marker is ever written, so
        // every path after the custom/declarative branch returns false. Anything
        // true here came from the branch under test.
        ProviderMetadata::new(name, name, "", "", vec![], "", vec![])
    }

    #[test]
    fn a_custom_provider_that_declares_an_api_key_needs_the_key_stored() {
        let key = "PERMAGENT_TEST_CUSTOM_DECLARED_KEY";
        let meta = custom_provider_on_disk("permagent-test-custom-declared", key);

        assert!(
            !is_provider_configured(&meta, ProviderType::Custom),
            "a custom provider that requires auth and names an api_key_env is \
             NOT connected until that key exists — reporting it as connected \
             puts a seat in Council and a row in the model picker that fails on \
             first use"
        );

        Config::global()
            .set_secret(key, &"sk-test".to_string())
            .expect("store secret");
        assert!(
            is_provider_configured(&meta, ProviderType::Custom),
            "with the declared key stored, the same provider is connected"
        );
        Config::global().delete_secret(key).ok();
    }

    #[test]
    fn a_custom_provider_with_no_api_key_env_stays_connected_by_existence() {
        // Header/env-var auth: the credential lives in `headers`/`env_vars`, not
        // in a named api_key_env, so its mere existence on disk is the only
        // signal available and must keep counting as connected.
        let meta = custom_provider_on_disk("permagent-test-custom-headers", "");
        assert!(is_provider_configured(&meta, ProviderType::Custom));
    }
}
