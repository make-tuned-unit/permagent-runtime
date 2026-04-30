use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::config::paths::Paths;

/// Primary agent persona configuration.
/// Stored at ~/.permagent/agent.yaml under the `primary` key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrimaryPersona {
    pub first_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(default)]
    pub traits: Vec<String>,
    #[serde(default)]
    pub tone: String,
    #[serde(default)]
    pub opening_greeting: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_id: Option<String>,
}

impl Default for PrimaryPersona {
    fn default() -> Self {
        Self {
            first_name: "Aria".into(),
            last_name: None,
            nickname: None,
            traits: vec!["calm".into(), "precise".into(), "helpful".into()],
            tone: "Speaks clearly and concisely. Warm but professional.".into(),
            opening_greeting: "Hello! I'm ready to help.".into(),
            voice_id: None,
        }
    }
}

impl PrimaryPersona {
    /// Compute display name: nickname > first+last > first alone.
    pub fn display_name(&self) -> String {
        if let Some(ref nick) = self.nickname {
            if !nick.is_empty() {
                return nick.clone();
            }
        }
        match &self.last_name {
            Some(last) if !last.is_empty() => format!("{} {}", self.first_name, last),
            _ => self.first_name.clone(),
        }
    }

    /// Build the persona block for the system prompt.
    pub fn system_prompt_block(&self) -> String {
        let mut block = format!(
            "You are {}. You are a Permagent — a persistent agent with continuity across sessions through Spectral memory.",
            self.display_name()
        );
        if !self.tone.is_empty() {
            block.push_str(&format!("\nTone: {}", self.tone));
        }
        if !self.traits.is_empty() {
            block.push_str(&format!("\nYour nature: {}.", self.traits.join(", ")));
        }
        block
    }
}

/// Top-level agent.yaml schema.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentConfig {
    #[serde(default)]
    pub primary: PrimaryPersona,
}

/// Load agent config from ~/.permagent/agent.yaml.
/// Returns default if file doesn't exist.
pub fn load_agent_config() -> AgentConfig {
    let path = agent_yaml_path();
    if !path.exists() {
        return AgentConfig::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_yaml::from_str(&content).unwrap_or_default(),
        Err(_) => AgentConfig::default(),
    }
}

/// Save agent config to ~/.permagent/agent.yaml.
pub fn save_agent_config(config: &AgentConfig) -> Result<()> {
    let path = agent_yaml_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let yaml = serde_yaml::to_string(config)?;
    std::fs::write(&path, yaml)?;
    Ok(())
}

pub fn agent_yaml_path() -> std::path::PathBuf {
    Paths::config_dir().join("agent.yaml")
}

/// Shared persona state for hot-reload across the daemon.
pub type SharedPersona = Arc<RwLock<PrimaryPersona>>;

/// Create the shared persona from disk.
pub fn load_shared_persona() -> SharedPersona {
    let config = load_agent_config();
    Arc::new(RwLock::new(config.primary))
}
