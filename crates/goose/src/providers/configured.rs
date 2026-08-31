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
            if provider_type == ProviderType::Custom {
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
