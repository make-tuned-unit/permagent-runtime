//! Who sits on the Council: configured chat-completion providers.

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::providers::base::{ConfigKey, ProviderMetadata, ProviderType};

pub const EXCLUDE_KEY: &str = "council_exclude";

/// Coding-CLI / ACP engines are workers, not debate seats.
const CLI_ACP_NAMES: &[&str] = &[
    "amp",
    "chatgpt_codex",
    "claude_acp",
    "claude_code",
    "codex",
    "codex_acp",
    "copilot_acp",
    "cursor_agent",
    "gemini_cli",
    "kimi_code",
    "pi_acp",
    "testprovider",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub provider: String,
    pub display_name: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seat {
    pub provider: String,
    pub display_name: String,
    pub model: String,
    pub configured: bool,
    pub excluded: bool,
    pub cli_or_acp: bool,
}

impl Seat {
    pub fn eligible(&self) -> bool {
        self.configured && !self.excluded && !self.cli_or_acp
    }
}

pub fn is_cli_or_acp(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    CLI_ACP_NAMES.contains(&n.as_str()) || n.ends_with("_acp") || n.contains("acp")
}

pub fn excluded_providers() -> Vec<String> {
    let cfg = Config::global();
    if let Ok(list) = cfg.get_param::<Vec<String>>(EXCLUDE_KEY) {
        return list
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    if let Ok(s) = cfg.get_param::<String>(EXCLUDE_KEY) {
        return s
            .split(',')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
    }
    Vec::new()
}

pub fn set_excluded(names: &[String]) -> Result<(), String> {
    Config::global()
        .set_param(EXCLUDE_KEY, &names.to_vec())
        .map_err(|e| e.to_string())
}

pub fn provider_is_configured(meta: &ProviderMetadata, provider_type: ProviderType) -> bool {
    let cfg = Config::global();
    if meta.name == "local" {
        return true;
    }
    let has_oauth = meta.config_keys.iter().any(|k| k.oauth_flow);
    if has_oauth {
        let marker = format!("{}_configured", meta.name);
        if matches!(cfg.get_param::<bool>(&marker), Ok(true)) {
            return true;
        }
    }
    let required: Vec<&ConfigKey> = meta.config_keys.iter().filter(|k| k.required).collect();
    if required.is_empty() {
        // Host-only providers (Ollama, Apple FM) count as connected when the
        // operator has not excluded them; they need no key.
        return true;
    }
    required.iter().all(|key| {
        if key.secret {
            cfg.get_secret::<String>(&key.name).is_ok()
        } else {
            std::env::var(&key.name).is_ok() || cfg.get_param::<String>(&key.name).is_ok()
        }
    }) || (provider_type == ProviderType::Custom || provider_type == ProviderType::Declarative)
        && meta.config_keys.is_empty()
}

pub fn model_for_provider(meta: &ProviderMetadata) -> String {
    let cfg = Config::global();
    let chat_provider = cfg
        .get_param::<String>("chat_provider")
        .ok()
        .or_else(|| cfg.get_param::<String>("GOOSE_PROVIDER").ok());
    let chat_model = cfg
        .get_param::<String>("chat_model")
        .ok()
        .or_else(|| cfg.get_param::<String>("GOOSE_MODEL").ok());
    if chat_provider.as_deref() == Some(meta.name.as_str()) {
        if let Some(model) = chat_model.filter(|m| !m.trim().is_empty()) {
            return model;
        }
    }
    meta.default_model.clone()
}

pub fn seats_from(listed: &[(ProviderMetadata, ProviderType)], excluded: &[String]) -> Vec<Seat> {
    let excluded_l: Vec<String> = excluded.iter().map(|s| s.to_ascii_lowercase()).collect();
    listed
        .iter()
        .map(|(meta, ty)| {
            let cli = is_cli_or_acp(&meta.name);
            Seat {
                provider: meta.name.clone(),
                display_name: meta.display_name.clone(),
                model: model_for_provider(meta),
                configured: provider_is_configured(meta, *ty),
                excluded: excluded_l.contains(&meta.name.to_ascii_lowercase()),
                cli_or_acp: cli,
            }
        })
        .collect()
}

pub fn members_from_seats(seats: &[Seat]) -> Vec<Member> {
    seats
        .iter()
        .filter(|s| s.eligible())
        .map(|s| Member {
            provider: s.provider.clone(),
            display_name: s.display_name.clone(),
            model: s.model.clone(),
        })
        .collect()
}

pub async fn resolve_members() -> Vec<Member> {
    let listed = crate::providers::providers().await;
    let seats = seats_from(&listed, &excluded_providers());
    members_from_seats(&seats)
}

pub async fn resolve_seats() -> Vec<Seat> {
    let listed = crate::providers::providers().await;
    seats_from(&listed, &excluded_providers())
}

/// Henry's chat model chairs the synthesis.
pub fn chair_route() -> (String, String) {
    use crate::config::model_roles::{
        role_model_from_config, ModelRole, RoleModelSource, SESSION_MODEL_KEY, SESSION_PROVIDER_KEY,
    };
    let cfg = Config::global();
    let res = role_model_from_config(ModelRole::Chat);
    if let Some(route) = res.route {
        return (route.provider, route.model);
    }
    if matches!(
        res.source,
        RoleModelSource::SessionModel | RoleModelSource::Disabled | RoleModelSource::HalfConfigured
    ) {
        let p = cfg
            .get_param::<String>(SESSION_PROVIDER_KEY)
            .ok()
            .filter(|s| !s.trim().is_empty());
        let m = cfg
            .get_param::<String>(SESSION_MODEL_KEY)
            .ok()
            .filter(|s| !s.trim().is_empty());
        if let (Some(p), Some(m)) = (p, m) {
            return (p, m);
        }
    }
    let def = ModelRole::Chat.measured_default();
    (def.provider, def.model)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::base::{ConfigKey, ProviderMetadata, ProviderType};

    fn meta(name: &str, display: &str, default_model: &str) -> ProviderMetadata {
        ProviderMetadata::new(
            name,
            display,
            "test",
            default_model,
            vec![default_model],
            "https://example.com",
            vec![ConfigKey::new("X_API_KEY", true, true, None, true)],
        )
    }

    fn ollama() -> ProviderMetadata {
        ProviderMetadata::new(
            "ollama",
            "Ollama",
            "local",
            "qwen2.5:7b",
            vec!["qwen2.5:7b"],
            "https://example.com",
            vec![],
        )
    }

    #[test]
    fn skips_cli_and_acp_and_honours_exclude() {
        let listed = vec![
            (
                meta("anthropic", "Anthropic", "claude-haiku"),
                ProviderType::Preferred,
            ),
            (meta("openai", "OpenAI", "gpt-4.1"), ProviderType::Preferred),
            (
                meta("claude_code", "Claude Code", "opus"),
                ProviderType::Builtin,
            ),
            (
                meta("cursor_agent", "Cursor", "auto"),
                ProviderType::Builtin,
            ),
            (ollama(), ProviderType::Builtin),
        ];
        let seats = seats_from(&listed, &["ollama".into()]);
        let members = members_from_seats(&seats);
        let names: Vec<&str> = members.iter().map(|m| m.provider.as_str()).collect();
        // anthropic/openai need secrets → not configured in this test.
        // ollama is configured (no keys) but excluded.
        // CLI skipped.
        assert!(names.is_empty(), "{names:?}");
        let ollama_seat = seats.iter().find(|s| s.provider == "ollama").unwrap();
        assert!(ollama_seat.configured);
        assert!(ollama_seat.excluded);
        assert!(!ollama_seat.eligible());
        let cc = seats.iter().find(|s| s.provider == "claude_code").unwrap();
        assert!(cc.cli_or_acp);
        assert!(!cc.eligible());
    }

    fn host_only(name: &str) -> ProviderMetadata {
        ProviderMetadata::new(
            name,
            name,
            "local",
            "m",
            vec!["m"],
            "https://example.com",
            vec![],
        )
    }

    #[test]
    fn configured_chat_provider_gets_a_seat() {
        let listed = vec![
            (host_only("openai"), ProviderType::Builtin),
            (
                meta("claude_code", "Claude Code", "opus"),
                ProviderType::Builtin,
            ),
            (ollama(), ProviderType::Builtin),
        ];
        let seats = seats_from(&listed, &["ollama".into()]);
        let names: Vec<&str> = members_from_seats(&seats)
            .iter()
            .map(|m| m.provider.as_str())
            .collect();
        assert_eq!(names, vec!["openai"]);
    }

    #[test]
    fn is_cli_detects_acp_suffix() {
        assert!(is_cli_or_acp("copilot_acp"));
        assert!(is_cli_or_acp("claude_code"));
        assert!(!is_cli_or_acp("anthropic"));
        assert!(!is_cli_or_acp("openai"));
    }
}
