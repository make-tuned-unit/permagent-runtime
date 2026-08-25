//! Applying the CHAT model to a chat turn.
//!
//! The counterpart of the voice path's `apply_voice_model`: read the chat role's
//! configured route (`chat_provider` / `chat_model`, resolved by
//! [`permagent::config::model_roles`]) and point this session's agent at it
//! before the turn runs. Chosen by measurement in
//! `docs/research/MODEL_DEFAULTS_BENCH_2026-08-25.md`.
//!
//! Three rules, and they are the same three the voice path learned:
//!
//! - **A route that cannot be reached is never a failed turn.** A bad model id,
//!   a missing key, no network — every one of them logs a warning and leaves the
//!   session on the model it already had. Someone typing into a chat box should
//!   not see an error because a *default* was wrong.
//! - **The session model wins unless the operator said otherwise.** Unlike voice,
//!   the measured chat default does NOT outrank `GOOSE_MODEL`; see the
//!   precedence discussion in [`permagent::config::model_roles`]. In practice
//!   this function is a no-op for anyone who has configured a session model and
//!   never set the chat keys.
//! - **Chat only.** Proactive turns, scheduled recipes and the CLI do not go
//!   through here.
//!
//! The created provider is cached by route so a chat turn does not rebuild an
//! HTTP client on every keystroke-completed message.

use std::sync::Arc;
use tokio::sync::{Mutex, OnceCell};

use permagent::config::{ModelRole, RoleModel, RoleModelSource};

type CachedProvider = Option<(RoleModel, Arc<dyn permagent::providers::base::Provider>)>;

fn chat_provider_cache() -> &'static OnceCell<Mutex<CachedProvider>> {
    static CACHE: OnceCell<Mutex<CachedProvider>> = OnceCell::const_new();
    &CACHE
}

/// Build the model config for a route, or `None` when the route is unusable.
///
/// Split out from [`apply_chat_model`] so the fallback path — a bad id must
/// never take a chat turn down — is testable without a live agent.
fn chat_model_config(route: &RoleModel) -> Option<permagent::model::ModelConfig> {
    if route.model.trim().is_empty() || route.provider.trim().is_empty() {
        tracing::warn!(
            target: "permagentd::chat",
            "chat route is missing a provider or a model; this turn runs on the session model"
        );
        return None;
    }
    match permagent::model::ModelConfig::new(&route.model) {
        Ok(config) => Some(config.with_canonical_limits(&route.provider)),
        Err(e) => {
            tracing::warn!(
                target: "permagentd::chat",
                chat_provider = %route.provider,
                chat_model = %route.model,
                error = %e,
                "configured chat model is invalid; this turn runs on the session model"
            );
            None
        }
    }
}

/// Point `agent` at the configured chat model for this turn, if one applies.
///
/// Returns the route that was applied, or `None` when the session's existing
/// model stands (nothing configured, explicitly disabled, or the route could not
/// be reached).
pub async fn apply_chat_model(
    agent: &Arc<permagent::agents::Agent>,
    session_id: &str,
) -> Option<RoleModel> {
    let resolved = permagent::config::role_model_from_config(ModelRole::Chat);
    if resolved.source == RoleModelSource::HalfConfigured {
        tracing::warn!(
            target: "permagentd::chat",
            "only one of `{}`/`{}` is set — a half-configured pair cannot route, so it is \
             ignored; set both, or set one to `session` to run chat on the session model",
            ModelRole::Chat.provider_key(),
            ModelRole::Chat.model_key(),
        );
    }
    let route = resolved.route?;

    let cache = chat_provider_cache()
        .get_or_init(|| async { Mutex::new(None) })
        .await;
    let mut cached = cache.lock().await;
    let provider = match cached.as_ref() {
        Some((cached_route, provider)) if *cached_route == route => Arc::clone(provider),
        _ => {
            let model_config = chat_model_config(&route)?;
            let extensions = agent.get_extension_configs().await;
            match permagent::providers::create(&route.provider, model_config, extensions).await {
                Ok(provider) => {
                    *cached = Some((route.clone(), Arc::clone(&provider)));
                    provider
                }
                Err(e) => {
                    tracing::warn!(
                        target: "permagentd::chat",
                        chat_provider = %route.provider,
                        chat_model = %route.model,
                        error = %e,
                        "configured chat model is unreachable; this turn runs on the session model"
                    );
                    return None;
                }
            }
        }
    };
    drop(cached);

    if let Err(e) = agent.update_provider(provider, session_id).await {
        tracing::warn!(
            target: "permagentd::chat",
            chat_provider = %route.provider,
            chat_model = %route.model,
            error = %e,
            "could not switch this session to the chat model; running on the session model"
        );
        return None;
    }
    Some(route)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_half_of_a_route_is_refused_rather_than_routed() {
        for route in [
            RoleModel::new("", "deepseek-chat"),
            RoleModel::new("custom_deepseek", "  "),
        ] {
            assert!(
                chat_model_config(&route).is_none(),
                "{route:?} must fall back to the session model"
            );
        }
    }

    #[test]
    fn a_usable_route_builds_a_model_config_with_that_models_limits() {
        let route = RoleModel::new("custom_deepseek", "deepseek-chat");
        let config = chat_model_config(&route).expect("a valid route builds a config");
        assert_eq!(config.model_name, "deepseek-chat");
    }

    #[test]
    fn an_unknown_model_id_still_builds_a_config_rather_than_panicking() {
        // ModelConfig::new tolerates unknown ids (limits come from the canonical
        // table, which simply has no row); the unreachable case is caught later
        // by providers::create, which is the path that must not take a turn down.
        let route = RoleModel::new("custom_deepseek", "not-a-real-model-id");
        assert!(chat_model_config(&route).is_some());
    }
}
