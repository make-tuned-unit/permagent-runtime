use anyhow::{Context, Result};
use serde::Deserialize;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub database: Database,
    pub daemon: Daemon,
    pub sources: Vec<Source>,
    pub models: Models,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Database {
    pub path: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Daemon {
    pub bind: String,
    pub bearer_token_env: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Source {
    pub kind: String,
    pub path: String,
    pub label: String,
    pub enabled: bool,
    pub cloud_ok: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Models {
    pub base_url: String,
    pub generation: String,
    pub embedding: String,
}

impl Default for Database {
    fn default() -> Self {
        Self {
            path: "~/.permagent/harvest.db".into(),
        }
    }
}

impl Default for Daemon {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:4007".into(),
            bearer_token_env: "PERMAGENT_HARVESTER_TOKEN".into(),
        }
    }
}

impl Default for Source {
    fn default() -> Self {
        Self {
            kind: String::new(),
            path: String::new(),
            label: String::new(),
            enabled: true,
            cloud_ok: false,
        }
    }
}

impl Default for Models {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:11434".into(),
            generation: "qwen2.5:7b".into(),
            embedding: "qwen3-embedding:0.6b".into(),
        }
    }
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let path = match path {
            Some(p) => expand(p)?,
            None => expand(Path::new("~/.permagent/harvester.toml"))?,
        };
        match fs::read_to_string(&path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!(path = %path.display(), "config file missing; using defaults");
                Ok(Self::default())
            }
            Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
        }
    }

    pub fn db_path(&self) -> Result<PathBuf> {
        expand(Path::new(&self.database.path))
    }

    pub fn bearer_token(&self) -> Result<String> {
        let token = env::var(&self.daemon.bearer_token_env).with_context(|| {
            format!(
                "bearer token environment variable {} is not set",
                self.daemon.bearer_token_env
            )
        })?;
        if token.trim().is_empty() {
            anyhow::bail!(
                "bearer token environment variable {} must not be empty or whitespace",
                self.daemon.bearer_token_env
            );
        }
        Ok(token)
    }
}

fn expand(path: &Path) -> Result<PathBuf> {
    let text = path.to_str().context("path is not valid UTF-8")?;
    Ok(PathBuf::from(shellexpand::tilde(text).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_is_enabled_when_field_is_omitted() {
        let source: Source = toml::from_str(
            r#"
kind = "notes"
path = "/tmp/notes"
label = "Notes"
cloud_ok = false
"#,
        )
        .unwrap();

        assert!(source.enabled);
    }

    #[test]
    fn example_config_stays_valid() {
        let config: Config = toml::from_str(include_str!("../harvester.example.toml")).unwrap();

        assert_eq!(config.sources.len(), 3);
    }

    #[test]
    fn unknown_config_keys_are_rejected() {
        let error = toml::from_str::<Config>("[daemon]\nbind_addr = '127.0.0.1:4007'")
            .unwrap_err()
            .to_string();

        assert!(error.contains("unknown field `bind_addr`"));
    }

    #[test]
    fn empty_bearer_token_is_rejected() {
        const VARIABLE: &str = "PERMAGENT_HARVESTER_EMPTY_TOKEN_TEST";
        env::set_var(VARIABLE, "   ");
        let mut config = Config::default();
        config.daemon.bearer_token_env = VARIABLE.into();

        let error = config.bearer_token().unwrap_err().to_string();
        env::remove_var(VARIABLE);

        assert!(error.contains("must not be empty or whitespace"));
    }
}
