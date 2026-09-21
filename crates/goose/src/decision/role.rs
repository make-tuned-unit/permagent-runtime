use super::{endpoint_locality, DecisionError, HttpTimeouts, SystemOneHttpBackend};
use crate::config::Config;
use crate::sovereignty::DataLocality;
use std::fmt;
use url::Url;

const DEFAULT_JEV_ENDPOINT: &str = "https://api.typesafe.ai";
const TYPESAFE_API_KEY_NAME: &str = "TYPESAFE_API_KEY";
const TYPESAFE_API_HOST: &str = "api.typesafe.ai";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JudgeProvider {
    Jev,
    SystemOne,
}

impl JudgeProvider {
    fn parse(raw: &str) -> Result<Self, JudgeConfigError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "jev" => Ok(Self::Jev),
            "systemone" => Ok(Self::SystemOne),
            _ => Err(JudgeConfigError::UnknownProvider(raw.trim().to_string())),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct JudgeSettings {
    pub provider: JudgeProvider,
    pub model: String,
    pub endpoint: String,
    pub api_key: String,
    pub locality: DataLocality,
}

impl fmt::Debug for JudgeSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JudgeSettings")
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("endpoint", &self.endpoint)
            .field("api_key", &"[REDACTED]")
            .field("locality", &self.locality)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JudgeResolution {
    Unconfigured,
    DisabledBySovereignMode,
    Configured(JudgeSettings),
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum JudgeConfigError {
    #[error("unknown decision judge provider id '{0}' (expected 'jev' or 'systemone')")]
    UnknownProvider(String),
    #[error("decision judge provider 'jev' requires secret {TYPESAFE_API_KEY_NAME}")]
    MissingJevApiKey,
    #[error("decision judge provider 'systemone' requires PERMAGENT_DECISION_JUDGE_ENDPOINT")]
    MissingSystemOneEndpoint,
    #[error("decision judge provider 'jev' endpoint override must use host '{TYPESAFE_API_HOST}' over https; got host '{0}'")]
    InvalidJevEndpointHost(String),
    #[error("decision judge backend config invalid: {0}")]
    Backend(DecisionError),
}

/// Pure core: a judge is configured only when BOTH provider and model are
/// present and non-empty; any half-configured input is treated as unset.
pub fn resolve_judge(
    provider: Option<&str>,
    model: Option<&str>,
    endpoint: Option<&str>,
    api_key: Option<&str>,
    sovereign: bool,
) -> Result<JudgeResolution, JudgeConfigError> {
    let provider = normalize(provider);
    let model = normalize(model);
    let (provider, model) = match (provider, model) {
        (Some(provider), Some(model)) => (provider, model),
        _ => return Ok(JudgeResolution::Unconfigured),
    };

    let provider = JudgeProvider::parse(&provider)?;
    let endpoint = resolve_endpoint(provider, endpoint)?;
    let locality = endpoint_locality(&endpoint);
    if sovereign && locality != DataLocality::Local {
        return Ok(JudgeResolution::DisabledBySovereignMode);
    }
    let api_key = resolve_api_key(provider, api_key)?;
    Ok(JudgeResolution::Configured(JudgeSettings {
        provider,
        model,
        endpoint,
        api_key,
        locality,
    }))
}

pub fn configured_judge() -> Result<Option<JudgeSettings>, JudgeConfigError> {
    let provider = crate::config::decision_judge_provider();
    let model = crate::config::decision_judge_model();
    let endpoint = crate::config::decision_judge_endpoint();
    let api_key = Config::global()
        .get_secret::<String>(TYPESAFE_API_KEY_NAME)
        .ok();
    let sovereign = crate::sovereignty::global_sovereign_mode();

    let resolution = resolve_judge(
        provider.as_deref(),
        model.as_deref(),
        endpoint.as_deref(),
        api_key.as_deref(),
        sovereign,
    )?;
    match resolution {
        JudgeResolution::Unconfigured => Ok(None),
        JudgeResolution::DisabledBySovereignMode => {
            tracing::warn!(
                "Sovereign mode is on; decision judge config resolved to a cloud endpoint and was disabled."
            );
            Ok(None)
        }
        JudgeResolution::Configured(settings) => Ok(Some(settings)),
    }
}

pub fn build_judge_backend(
    settings: &JudgeSettings,
    timeouts: HttpTimeouts,
) -> Result<SystemOneHttpBackend, JudgeConfigError> {
    SystemOneHttpBackend::new(
        settings.endpoint.clone(),
        settings.model.clone(),
        settings.api_key.clone(),
        timeouts,
    )
    .map_err(JudgeConfigError::Backend)
}

fn normalize(raw: Option<&str>) -> Option<String> {
    raw.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn resolve_endpoint(
    provider: JudgeProvider,
    endpoint: Option<&str>,
) -> Result<String, JudgeConfigError> {
    let endpoint = endpoint
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty());
    match provider {
        JudgeProvider::Jev => {
            let endpoint = endpoint.unwrap_or_else(|| DEFAULT_JEV_ENDPOINT.to_string());
            if endpoint != DEFAULT_JEV_ENDPOINT {
                validate_jev_endpoint_override(&endpoint)?;
            }
            Ok(endpoint)
        }
        JudgeProvider::SystemOne => endpoint.ok_or(JudgeConfigError::MissingSystemOneEndpoint),
    }
}

/// B2a intentionally never forwards TypeSafe credentials to non-TypeSafe
/// providers. A dedicated System-One credential is out of scope here.
fn resolve_api_key(
    provider: JudgeProvider,
    api_key: Option<&str>,
) -> Result<String, JudgeConfigError> {
    let key = api_key.map(|s| s.trim().to_string()).unwrap_or_default();
    match provider {
        JudgeProvider::Jev if key.is_empty() => Err(JudgeConfigError::MissingJevApiKey),
        JudgeProvider::Jev => Ok(key),
        JudgeProvider::SystemOne => Ok(String::new()),
    }
}

fn validate_jev_endpoint_override(endpoint: &str) -> Result<(), JudgeConfigError> {
    let url = Url::parse(endpoint)
        .map_err(|_| JudgeConfigError::InvalidJevEndpointHost("<missing>".to_string()))?;
    let host = url.host_str().unwrap_or("<missing>");
    if !url.username().is_empty() || url.password().is_some() {
        return Err(JudgeConfigError::InvalidJevEndpointHost(host.to_string()));
    }
    if !url.scheme().eq_ignore_ascii_case("https") || !host.eq_ignore_ascii_case(TYPESAFE_API_HOST)
    {
        return Err(JudgeConfigError::InvalidJevEndpointHost(host.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_judge_table() {
        struct Case<'a> {
            provider: Option<&'a str>,
            model: Option<&'a str>,
            endpoint: Option<&'a str>,
            api_key: Option<&'a str>,
            sovereign: bool,
            expected: Result<JudgeResolution, JudgeConfigError>,
        }

        let cases = [
            Case {
                provider: None,
                model: None,
                endpoint: None,
                api_key: None,
                sovereign: false,
                expected: Ok(JudgeResolution::Unconfigured),
            },
            Case {
                provider: Some("jev"),
                model: None,
                endpoint: None,
                api_key: Some("test-secret"),
                sovereign: false,
                expected: Ok(JudgeResolution::Unconfigured),
            },
            Case {
                provider: None,
                model: Some("jev-latest"),
                endpoint: None,
                api_key: Some("test-secret"),
                sovereign: false,
                expected: Ok(JudgeResolution::Unconfigured),
            },
            Case {
                provider: Some("jev"),
                model: Some("jev-latest"),
                endpoint: None,
                api_key: Some("test-secret"),
                sovereign: false,
                expected: Ok(JudgeResolution::Configured(JudgeSettings {
                    provider: JudgeProvider::Jev,
                    model: "jev-latest".to_string(),
                    endpoint: DEFAULT_JEV_ENDPOINT.to_string(),
                    api_key: "test-secret".to_string(),
                    locality: DataLocality::Cloud,
                })),
            },
            Case {
                provider: Some("jev"),
                model: Some("jev-latest"),
                endpoint: None,
                api_key: None,
                sovereign: false,
                expected: Err(JudgeConfigError::MissingJevApiKey),
            },
            Case {
                provider: Some("systemone"),
                model: Some("local-judge"),
                endpoint: None,
                api_key: None,
                sovereign: false,
                expected: Err(JudgeConfigError::MissingSystemOneEndpoint),
            },
            Case {
                provider: Some("systemone"),
                model: Some("local-judge"),
                endpoint: Some("http://127.0.0.1:9000/"),
                api_key: Some("test-secret"),
                sovereign: false,
                expected: Ok(JudgeResolution::Configured(JudgeSettings {
                    provider: JudgeProvider::SystemOne,
                    model: "local-judge".to_string(),
                    endpoint: "http://127.0.0.1:9000".to_string(),
                    api_key: "".to_string(),
                    locality: DataLocality::Local,
                })),
            },
            Case {
                provider: Some("openai"),
                model: Some("gpt-5"),
                endpoint: None,
                api_key: Some("test-secret"),
                sovereign: false,
                expected: Err(JudgeConfigError::UnknownProvider("openai".to_string())),
            },
            Case {
                provider: Some("jev"),
                model: Some("jev-latest"),
                endpoint: None,
                api_key: Some("test-secret"),
                sovereign: true,
                expected: Ok(JudgeResolution::DisabledBySovereignMode),
            },
            Case {
                provider: Some("systemone"),
                model: Some("local-judge"),
                endpoint: Some("http://localhost:7777"),
                api_key: None,
                sovereign: true,
                expected: Ok(JudgeResolution::Configured(JudgeSettings {
                    provider: JudgeProvider::SystemOne,
                    model: "local-judge".to_string(),
                    endpoint: "http://localhost:7777".to_string(),
                    api_key: "".to_string(),
                    locality: DataLocality::Local,
                })),
            },
        ];

        for case in cases {
            let resolved = resolve_judge(
                case.provider,
                case.model,
                case.endpoint,
                case.api_key,
                case.sovereign,
            );
            assert_eq!(resolved, case.expected);
        }
    }

    #[test]
    fn jev_missing_key_names_only_typesafe_key() {
        let err = resolve_judge(Some("jev"), Some("jev-latest"), None, None, false).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("TYPESAFE_API_KEY"));
        assert!(!text.contains("test-secret"));
    }

    #[test]
    fn judge_settings_debug_redacts_api_key() {
        let settings = JudgeSettings {
            provider: JudgeProvider::Jev,
            model: "jev-latest".to_string(),
            endpoint: DEFAULT_JEV_ENDPOINT.to_string(),
            api_key: "test-secret".to_string(),
            locality: DataLocality::Cloud,
        };
        let debug = format!("{settings:?}");
        assert!(!debug.contains("test-secret"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn systemone_never_receives_typesafe_key() {
        let resolved = resolve_judge(
            Some("systemone"),
            Some("local-judge"),
            Some("http://127.0.0.1:9000"),
            Some("test-secret"),
            false,
        )
        .unwrap();
        match resolved {
            JudgeResolution::Configured(settings) => assert_eq!(settings.api_key, ""),
            other => panic!("expected configured systemone settings, got {other:?}"),
        }
    }

    #[test]
    fn jev_override_rejects_other_host_without_leaking_key() {
        let err = resolve_judge(
            Some("jev"),
            Some("jev-latest"),
            Some("https://evil.example"),
            Some("test-secret"),
            false,
        )
        .unwrap_err();
        let text = err.to_string();
        assert!(text.contains("evil.example"));
        assert!(!text.contains("test-secret"));
    }

    #[test]
    fn jev_override_rejects_http_even_on_typesafe_host() {
        let err = resolve_judge(
            Some("jev"),
            Some("jev-latest"),
            Some("http://api.typesafe.ai"),
            Some("test-secret"),
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("api.typesafe.ai"));
    }

    #[test]
    fn jev_override_accepts_case_insensitive_typesafe_host() {
        let resolved = resolve_judge(
            Some("jev"),
            Some("jev-latest"),
            Some("https://API.TYPESAFE.AI/"),
            Some("test-secret"),
            false,
        )
        .unwrap();
        match resolved {
            JudgeResolution::Configured(settings) => {
                assert_eq!(settings.endpoint, "https://API.TYPESAFE.AI");
            }
            other => panic!("expected configured jev settings, got {other:?}"),
        }
    }

    #[test]
    fn jev_override_rejects_userinfo_host_confusion() {
        let err = resolve_judge(
            Some("jev"),
            Some("jev-latest"),
            Some("https://api.typesafe.ai@evil.example"),
            Some("test-secret"),
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("evil.example"));
    }

    #[test]
    fn decision_judge_providers_are_not_registered_in_provider_factory() {
        let create_source = include_str!("../providers/init.rs");
        assert!(
            !create_source.contains("\"jev\""),
            "decision-judge id 'jev' must stay out of providers::create registry"
        );
        assert!(
            !create_source.contains("\"systemone\""),
            "decision-judge id 'systemone' must stay out of providers::create registry"
        );
    }
}
