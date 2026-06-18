use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::config::paths::Paths;

/// Placeholder persona key carried into
/// [`spectral::graph::RecognitionContext::persona`] at every recall site.
///
/// **Spectral does NOT currently key recall on this value.** In the pinned
/// Spectral rev, `RecognitionContext.persona` is reserved-for-future: its only
/// read is `is_empty()`, and retrieval, ranking, and the write path never
/// consult it. Changing this string therefore has **no effect on recall today**.
/// It exists so the recall sites share one intentional, centrally-changeable
/// value instead of scattering a bare `"henry"` literal that reads as
/// load-bearing recall-correctness when it is not.
///
/// This is NOT the authorship-origin `"henry"` token (e.g. `decisions::ACTOR_HENRY`,
/// card `created_by`) — those mean "the system authored this" and are deliberately
/// left untouched.
///
/// Forward path: when Spectral eventually honors persona, this placeholder should
/// be replaced by a real per-install opaque `persona_id` (minted once at first
/// identity-save), and the keying semantics designed together with that Spectral
/// change — not before. Until then this constant is hygiene only. See the deferred
/// "persona_id real-keying" note.
pub const DEFAULT_PERSONA_KEY: &str = "henry";

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

/// Compute display name: nickname > first+last > first alone.
fn compute_display_name(
    first_name: &str,
    last_name: Option<&str>,
    nickname: Option<&str>,
) -> String {
    if let Some(nick) = nickname {
        if !nick.is_empty() {
            return nick.to_string();
        }
    }
    match last_name {
        Some(last) if !last.is_empty() => format!("{} {}", first_name, last),
        _ => first_name.to_string(),
    }
}

impl PrimaryPersona {
    pub fn display_name(&self) -> String {
        compute_display_name(
            &self.first_name,
            self.last_name.as_deref(),
            self.nickname.as_deref(),
        )
    }

    /// Build the persona block for the system prompt.
    pub fn system_prompt_block(&self) -> String {
        let name = self.display_name();
        let mut block = format!(
            "Your name is {name}. When users address you as {name}, respond as {name} — \
             never correct them or claim a different name. \
             You run on the Permagent platform (a persistent AI agent system with continuity \
             across sessions through Spectral memory). \"Permagent\" is the product name, \
             not your name.",
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

fn default_availability() -> String {
    "always".to_string()
}

fn default_cost_tier() -> String {
    "local_free".to_string()
}

/// Worker persona configuration.
/// Workers are specialized agents with a role instead of a greeting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerPersona {
    pub first_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub traits: Vec<String>,
    #[serde(default)]
    pub tone: String,
    /// What this worker can do: code_edit, shell, web_search, memory_ops, etc.
    #[serde(default)]
    pub tool_kinds: Vec<String>,
    /// How to check if this worker is available on this machine.
    /// "bin_exists:<name>" | "api_credential:<env_var>" | "model_loaded:<model>" | "always"
    #[serde(default = "default_availability")]
    pub availability_check: String,
    /// Cost classification: "local_free", "subscription", or "paid_api"
    #[serde(default = "default_cost_tier")]
    pub cost_tier: String,
}

impl Default for WorkerPersona {
    fn default() -> Self {
        Self {
            first_name: String::new(),
            last_name: None,
            nickname: None,
            role: String::new(),
            traits: Vec::new(),
            tone: String::new(),
            tool_kinds: Vec::new(),
            availability_check: default_availability(),
            cost_tier: default_cost_tier(),
        }
    }
}

impl WorkerPersona {
    pub fn display_name(&self) -> String {
        compute_display_name(
            &self.first_name,
            self.last_name.as_deref(),
            self.nickname.as_deref(),
        )
    }

    /// Build the worker persona block for the system prompt.
    pub fn system_prompt_block(&self) -> String {
        let name = self.display_name();
        let mut block = format!(
            "Your name is {name}. You are a worker agent on the Permagent platform — \
             a specialized agent with continuity across sessions through Spectral memory.",
        );
        if !self.role.is_empty() {
            block.push_str(&format!("\nYour role: {}", self.role));
        }
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(default)]
    pub primary: PrimaryPersona,
    #[serde(default)]
    pub workers: HashMap<String, WorkerPersona>,
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

/// Shared full agent config (primary + workers) for hot-reload.
pub type SharedAgentConfig = Arc<RwLock<AgentConfig>>;

/// Create the shared agent config from disk.
pub fn load_shared_agent_config() -> SharedAgentConfig {
    let config = load_agent_config();
    Arc::new(RwLock::new(config))
}

/// Create the shared persona from disk (for backward compat).
pub fn load_shared_persona() -> SharedPersona {
    let config = load_agent_config();
    Arc::new(RwLock::new(config.primary))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_persona_backward_compat_without_new_fields() {
        let yaml = r#"
first_name: Codex
role: "Fast parallel coding agent"
traits: [fast, precise]
tone: concise
"#;
        let persona: WorkerPersona = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(persona.first_name, "Codex");
        assert_eq!(persona.role, "Fast parallel coding agent");
        assert!(persona.tool_kinds.is_empty());
        assert_eq!(persona.availability_check, "always");
        assert_eq!(persona.cost_tier, "local_free");
    }

    #[test]
    fn worker_persona_with_all_new_fields() {
        let yaml = r#"
first_name: Codex
role: "Fast parallel coding agent"
tool_kinds: [code_edit, shell, git]
availability_check: "bin_exists:codex"
cost_tier: subscription
"#;
        let persona: WorkerPersona = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(persona.tool_kinds, vec!["code_edit", "shell", "git"]);
        assert_eq!(persona.availability_check, "bin_exists:codex");
        assert_eq!(persona.cost_tier, "subscription");
    }

    #[test]
    fn worker_persona_defaults_applied() {
        let yaml = r#"
first_name: Lib
"#;
        let persona: WorkerPersona = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(persona.availability_check, "always");
        assert_eq!(persona.cost_tier, "local_free");
        assert!(persona.tool_kinds.is_empty());
        assert!(persona.role.is_empty());
        assert!(persona.traits.is_empty());
    }

    #[test]
    fn agent_config_backward_compat_no_workers() {
        let yaml = r#"
primary:
  first_name: Henry
  tone: friendly
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.primary.first_name, "Henry");
        assert!(config.workers.is_empty());
    }

    #[test]
    fn agent_config_with_mixed_workers() {
        let yaml = r#"
primary:
  first_name: Henry
workers:
  codex:
    first_name: Codex
    role: coding
    tool_kinds: [code_edit]
    availability_check: "bin_exists:codex"
    cost_tier: subscription
  librarian:
    first_name: Librarian
    role: memory
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.workers.len(), 2);

        let codex = &config.workers["codex"];
        assert_eq!(codex.tool_kinds, vec!["code_edit"]);
        assert_eq!(codex.cost_tier, "subscription");

        let lib = &config.workers["librarian"];
        assert!(lib.tool_kinds.is_empty());
        assert_eq!(lib.availability_check, "always");
        assert_eq!(lib.cost_tier, "local_free");
    }

    #[test]
    fn primary_persona_block_interpolates_name() {
        let persona = PrimaryPersona {
            first_name: "Ada".into(),
            ..Default::default()
        };
        let block = persona.system_prompt_block();
        assert!(
            block.contains("Your name is Ada."),
            "Should interpolate first_name: {}",
            block
        );
        assert!(
            block.contains("address you as Ada"),
            "Should use name in identity reinforcement: {}",
            block
        );
        assert!(
            !block.contains("Henry"),
            "Must not contain hardcoded 'Henry': {}",
            block
        );
    }

    #[test]
    fn worker_persona_block_interpolates_name() {
        let persona = WorkerPersona {
            first_name: "Bolt".into(),
            role: "testing".into(),
            ..Default::default()
        };
        let block = persona.system_prompt_block();
        assert!(
            block.contains("Your name is Bolt."),
            "Should interpolate first_name: {}",
            block
        );
        assert!(
            !block.contains("Henry"),
            "Must not contain hardcoded 'Henry': {}",
            block
        );
    }
}
