//! Who sits on the Council: configured chat-completion providers.

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::providers::base::{ProviderMetadata, ProviderType};

pub const EXCLUDE_KEY: &str = "council_exclude";

/// Coding-CLI / ACP engines are workers, not debate seats.
///
/// Spelled snake_case; `is_cli_or_acp` normalizes the registry's kebab-case ids
/// before comparing. Entries must mirror a real `PROVIDER_NAME` — `"amp"` and
/// `"testprovider"` used to sit here and matched nothing (the real ids are
/// `amp-acp` and `test`), which is how the list drifted from the registry
/// unnoticed. `real_registry_cli_acp_ids_are_all_recognized` now pins it.
const CLI_ACP_NAMES: &[&str] = &[
    "amp_acp",
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
    // Registry ids use both kebab-case (`claude-code`, `codex-acp`) and
    // snake_case (`claude_code`, `codex_acp`). Normalize before checking or a
    // coding harness can leak into the debate-seat list under one spelling.
    let n = name.trim().to_ascii_lowercase().replace('-', "_");
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
        .set_param(EXCLUDE_KEY, names.to_vec())
        .map_err(|e| e.to_string())
}

pub fn provider_is_configured(meta: &ProviderMetadata, provider_type: ProviderType) -> bool {
    // Keep Council, `/model`, and every other model picker on one definition
    // of "connected". The old copy missed custom provider credentials and
    // several defaulted/OAuth shapes, so working models disappeared here even
    // while Chat or the harness was actively using them.
    crate::providers::configured::is_provider_configured(meta, provider_type)
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

    fn local() -> ProviderMetadata {
        ProviderMetadata::new(
            "local",
            "Local Inference",
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
            (local(), ProviderType::Builtin),
        ];
        let seats = seats_from(&listed, &["local".into()]);
        let members = members_from_seats(&seats);
        let names: Vec<&str> = members.iter().map(|m| m.provider.as_str()).collect();
        // anthropic/openai need secrets → not configured in this test.
        // local inference is always configured but explicitly excluded.
        // CLI skipped.
        assert!(names.is_empty(), "{names:?}");
        let local_seat = seats.iter().find(|s| s.provider == "local").unwrap();
        assert!(local_seat.configured);
        assert!(local_seat.excluded);
        assert!(!local_seat.eligible());
        let cc = seats.iter().find(|s| s.provider == "claude_code").unwrap();
        assert!(cc.cli_or_acp);
        assert!(!cc.eligible());
    }

    #[test]
    fn cli_worker_ids_are_recognized_in_both_registry_spellings() {
        for name in [
            "claude_code",
            "claude-code",
            "codex_acp",
            "codex-acp",
            "cursor_agent",
            "cursor-agent",
            "gemini_cli",
            "gemini-cli",
        ] {
            assert!(is_cli_or_acp(name), "{name} must be a worker, not a seat");
        }
    }

    /// The registry spells coding-harness ids kebab-case (`claude-code`,
    /// `cursor-agent`) while `CLI_ACP_NAMES` is snake_case, so `is_cli_or_acp`
    /// matched none of them and every one leaked into the debate-seat list.
    ///
    /// Names come from each provider's own `metadata()` — i.e. the real
    /// `PROVIDER_NAME` constants — and are cross-checked against the live async
    /// registry, so a rename, a new provider, or a third separator convention
    /// fails here rather than silently at runtime. The old test hand-typed its
    /// strings, which is precisely why it stayed green while the live path was
    /// wrong.
    #[tokio::test]
    async fn real_registry_cli_acp_ids_are_all_recognized() {
        use crate::providers::amp_acp::AmpAcpProvider;
        use crate::providers::base::ProviderDef;
        use crate::providers::chatgpt_codex::ChatGptCodexProvider;
        use crate::providers::claude_acp::ClaudeAcpProvider;
        use crate::providers::claude_code::ClaudeCodeProvider;
        use crate::providers::codex::CodexProvider;
        use crate::providers::codex_acp::CodexAcpProvider;
        use crate::providers::copilot_acp::CopilotAcpProvider;
        use crate::providers::cursor_agent::CursorAgentProvider;
        use crate::providers::gemini_cli::GeminiCliProvider;
        use crate::providers::kimicode::KimiCodeProvider;
        use crate::providers::pi_acp::PiAcpProvider;

        let workers = [
            AmpAcpProvider::metadata().name,
            ChatGptCodexProvider::metadata().name,
            ClaudeAcpProvider::metadata().name,
            ClaudeCodeProvider::metadata().name,
            CodexAcpProvider::metadata().name,
            CodexProvider::metadata().name,
            CopilotAcpProvider::metadata().name,
            CursorAgentProvider::metadata().name,
            GeminiCliProvider::metadata().name,
            KimiCodeProvider::metadata().name,
            PiAcpProvider::metadata().name,
        ];
        assert!(
            workers.iter().any(|n| n.contains('-')),
            "fixture drift: no kebab-case worker id left, this test no longer covers the bug"
        );

        let listed = crate::providers::providers().await;
        for name in &workers {
            assert!(
                listed.iter().any(|(m, _)| &m.name == name),
                "{name} is missing from the live registry — update this test's expectations"
            );
            assert!(is_cli_or_acp(name), "{name} must be a worker, not a seat");
        }

        for name in [
            crate::providers::anthropic::AnthropicProvider::metadata().name,
            crate::providers::openai::OpenAiProvider::metadata().name,
            crate::providers::google::GoogleProvider::metadata().name,
        ] {
            assert!(
                !is_cli_or_acp(&name),
                "{name} is a debate seat, not a coding worker"
            );
        }
    }

    /// Anthropic marks BOTH `ANTHROPIC_API_KEY` and `ANTHROPIC_HOST` required,
    /// but the host carries a compiled default and is never written to config.
    /// The old check demanded every required key be independently present, so
    /// the provider this machine actively runs on reported "not configured".
    ///
    /// Written against the real `metadata()`: the module's synthetic `meta()`
    /// helper only ever built one required key with no default, a shape that
    /// cannot tell the old logic from the new one.
    #[test]
    fn real_anthropic_metadata_is_configured_with_only_the_api_key_set() {
        use crate::providers::anthropic::AnthropicProvider;
        use crate::providers::base::ProviderDef;

        let meta = AnthropicProvider::metadata();
        assert!(
            meta.config_keys
                .iter()
                .any(|k| k.name == "ANTHROPIC_HOST" && k.required && k.default.is_some()),
            "fixture drift: ANTHROPIC_HOST is no longer required-with-default, \
             so this test no longer reproduces the bug"
        );
        assert!(
            std::env::var("ANTHROPIC_HOST").is_err(),
            "ANTHROPIC_HOST is set in this environment; the test cannot prove the fix"
        );

        let cfg = Config::global();
        cfg.set_secret("ANTHROPIC_API_KEY", &"sk-test-not-a-real-key".to_string())
            .expect("store test key");
        let configured = provider_is_configured(&meta, ProviderType::Preferred);
        cfg.delete_secret("ANTHROPIC_API_KEY").ok();

        assert!(
            configured,
            "anthropic must count as connected with only its API key stored"
        );
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
            (local(), ProviderType::Builtin),
        ];
        let seats = seats_from(&listed, &[]);
        let members = members_from_seats(&seats);
        let names: Vec<&str> = members.iter().map(|m| m.provider.as_str()).collect();
        assert_eq!(names, vec!["local"]);
    }

    #[test]
    fn is_cli_detects_acp_suffix() {
        assert!(is_cli_or_acp("copilot_acp"));
        assert!(is_cli_or_acp("claude_code"));
        assert!(!is_cli_or_acp("anthropic"));
        assert!(!is_cli_or_acp("openai"));
    }
}
