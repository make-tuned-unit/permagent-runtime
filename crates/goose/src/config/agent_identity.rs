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

/// Self-knowledge descriptor for the Persona picker surface (name, voice,
/// audition). Added in Phase 2. Static for brief rendering, but its lesson
/// confirms via `PersonaConfigured` — a real queryable read-back, since a
/// personalized name surfaces in the brief's "You are <name>" line next turn.
/// Co-located here; aggregated by `crate::agents::self_knowledge`.
pub const PERSONA_PICKER_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "persona",
        display_name: "Persona picker",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "Where the user names you, picks your voice, and hears an audition before settling on one",
        why_it_matters:
            "It is the one thing that makes you feel like theirs rather than a generic assistant — lead with it warmly",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[
            crate::agents::self_knowledge::TeachingStep {
                title: "Make me yours",
                body: "Open the identity settings so they can give you a name, choose a voice, and hear it audition out loud. Invite them to make it personal.",
                open_surface: Some(crate::agents::self_knowledge::SurfaceRef {
                    tab: "Settings",
                    section: Some("identity"),
                }),
                confirm: None,
            },
            crate::agents::self_knowledge::TeachingStep {
                title: "Confirm the new you",
                body: "Once they have chosen a name (and maybe a voice), acknowledge it warmly in your own words — they have just made you theirs.",
                open_surface: None,
                confirm: Some(crate::agents::self_knowledge::ConfirmCheck::PersonaConfigured),
            },
        ],
    };

/// Self-knowledge descriptor for the voice modality (#353). Co-located with
/// the persona config because the voice identity is chosen here; the audio
/// pipeline itself lives in the daemon (sherpa STT + Kokoro TTS).
pub const VOICE_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "voice",
        display_name: "Voice",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "Push-to-talk speech input and spoken replies: the user holds the mic, speaks, and \
             you answer out loud in the persona's chosen voice. Works everywhere chat works, \
             including hands-free instructions like asking you to open a site and read it aloud",
        why_it_matters:
            "Many users drive you primarily by voice — treat spoken turns exactly like typed \
             ones. When asked to 'read' something aloud, answer in flowing sentences suited to \
             listening, not bullet fragments",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[crate::agents::self_knowledge::TeachingStep {
            title: "Talk instead of type",
            body: "Show the user the mic control: hold to talk, release to send. Your reply \
                   comes back both as text and spoken in the voice they chose for you.",
            open_surface: None,
            confirm: None,
        }],
    };

/// Self-knowledge descriptor for web search (#353). The search tools arrive
/// via a bundled MCP server (Brave or Tavily) the user connects in the wizard
/// or Settings — the descriptor keeps the brief honest either way.
pub const WEB_SEARCH_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "web_search",
        display_name: "Web search",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "Live web lookup through a connected search provider (Brave or Tavily). When one is \
             connected its search tools appear in your tool list; without one, offer to set it \
             up — a one-key step in Settings or the setup wizard",
        why_it_matters:
            "Fresh information beyond your training data. Check your live tool list before \
             claiming you can or cannot search — the truth is whatever tools are present",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[],
    };

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

/// How a worker is actually run when the orchestrator dispatches a goal to it.
///
/// Internally tagged so `agent.yaml` reads naturally, e.g.
/// `engine: { type: external_cli, bin: claude, args: [...] }`. Absent → the
/// default in-process engine (`#[serde(default)]` on the field).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerEngineKind {
    /// In-process subagent on the parent session's provider (default, always
    /// available — no external binary required).
    #[default]
    InternalSubagent,
    /// External agentic CLI (Claude Code, Codex, …) spawned in an isolated git
    /// worktree. `args` may contain the literal token `{prompt}`, replaced with
    /// the goal prompt at dispatch time.
    ExternalCli { bin: String, args: Vec<String> },
    /// Registered and probed, but no runnable engine wired yet. Such a worker is
    /// visible in the roster but never selected for a real goal — it must not be
    /// dispatched and silently fail.
    Pending,
}

impl WorkerEngineKind {
    /// Short, stable label for surfacing in the API / self-knowledge.
    pub fn label(&self) -> &'static str {
        match self {
            Self::InternalSubagent => "internal_subagent",
            Self::ExternalCli { .. } => "external_cli",
            Self::Pending => "pending",
        }
    }
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
    /// How this worker is run when dispatched. Absent → in-process subagent.
    #[serde(default)]
    pub engine: WorkerEngineKind,
    /// Per-dispatch wall-clock bound override, in seconds (#467). Absent →
    /// `goal_engine::DEFAULT_EXTERNAL_CLI_TIMEOUT_SECS` (2 h). On expiry the
    /// goal parks with an unblock decision — this bounds a hung worker, so
    /// keep it finite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
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
            engine: WorkerEngineKind::default(),
            timeout_secs: None,
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

/// The embedded default worker roster, seeded at first run (when no `agent.yaml`
/// exists, or one exists with no workers). Explicit config over hidden magic:
/// each worker carries its engine kind + availability probe so what the
/// orchestrator can dispatch to is visible and overridable.
///
/// - **Claude Code** — external-CLI reference worker (gated on the `claude`
///   binary). Runs autonomously in an isolated worktree.
/// - **Codex** — external-CLI fast-follow (gated on the `codex` binary). Its
///   invocation args are best-effort and want a behavioral dogfood.
/// - **Librarian** — registered + probed (Ollama `qwen2.5`) but `Pending`: its
///   in-process engine is not wired here, so it is never selected for a goal.
pub fn default_roster() -> HashMap<String, WorkerPersona> {
    let mut roster = HashMap::new();

    roster.insert(
        "claude_code".to_string(),
        WorkerPersona {
            first_name: "Claude Code".to_string(),
            role: "Autonomous coding agent — implements a goal end to end in an isolated git \
                   worktree, leaving commits for review"
                .to_string(),
            tool_kinds: vec![
                "code_edit".to_string(),
                "shell".to_string(),
                "git".to_string(),
            ],
            availability_check: "bin_exists:claude".to_string(),
            cost_tier: "subscription".to_string(),
            engine: WorkerEngineKind::ExternalCli {
                bin: "claude".to_string(),
                args: vec![
                    "-p".to_string(),
                    "{prompt}".to_string(),
                    "--dangerously-skip-permissions".to_string(),
                    "--output-format".to_string(),
                    "stream-json".to_string(),
                    "--verbose".to_string(),
                ],
            },
            ..Default::default()
        },
    );

    roster.insert(
        "codex".to_string(),
        WorkerPersona {
            first_name: "Codex".to_string(),
            role: "Fast coding agent — implements a goal in an isolated git worktree".to_string(),
            tool_kinds: vec!["code_edit".to_string(), "shell".to_string()],
            availability_check: "bin_exists:codex".to_string(),
            cost_tier: "subscription".to_string(),
            engine: WorkerEngineKind::ExternalCli {
                // Best-effort invocation — confirm against the installed codex
                // CLI during the behavioral dogfood before relying on it.
                bin: "codex".to_string(),
                args: vec!["exec".to_string(), "{prompt}".to_string()],
            },
            ..Default::default()
        },
    );

    roster.insert(
        "librarian".to_string(),
        WorkerPersona {
            first_name: "Librarian".to_string(),
            role: "Brain-curation worker (Ollama) — engine not yet wired".to_string(),
            tool_kinds: vec!["memory_ops".to_string()],
            availability_check: "model_loaded:qwen2.5".to_string(),
            cost_tier: "local_free".to_string(),
            engine: WorkerEngineKind::Pending,
            ..Default::default()
        },
    );

    roster
}

/// Load agent config from ~/.permagent/agent.yaml.
///
/// Seeds the embedded [`default_roster`] in-memory when the file is absent or
/// carries no workers (no disk write — the existing PUT/save path persists any
/// user edits). The `primary` persona still defaults independently.
pub fn load_agent_config() -> AgentConfig {
    let path = agent_yaml_path();
    let mut config = if !path.exists() {
        AgentConfig::default()
    } else {
        match std::fs::read_to_string(&path) {
            Ok(content) => serde_yaml::from_str(&content).unwrap_or_default(),
            Err(_) => AgentConfig::default(),
        }
    };
    if config.workers.is_empty() {
        config.workers = default_roster();
    }
    config
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
    fn primary_persona_block_injects_user_authored_traits() {
        // W1: traits the user authors past the wizard presets (incl. custom
        // free-text) must reach the system prompt verbatim via "Your nature: …".
        let persona = PrimaryPersona {
            first_name: "Ada".into(),
            traits: vec![
                "precise".into(),
                "ruthlessly skeptical".into(), // custom free-text trait
            ],
            ..Default::default()
        };
        let block = persona.system_prompt_block();
        assert!(
            block.contains("Your nature: precise, ruthlessly skeptical."),
            "user-authored traits (preset + custom) must be injected: {}",
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

    #[test]
    fn default_roster_seeds_three_workers_with_engines() {
        let roster = default_roster();
        assert_eq!(roster.len(), 3, "expected claude_code + codex + librarian");

        // Claude Code: external CLI, prompt token present, gated on the binary.
        match &roster["claude_code"].engine {
            WorkerEngineKind::ExternalCli { bin, args } => {
                assert_eq!(bin, "claude");
                assert!(
                    args.iter().any(|a| a == "{prompt}"),
                    "claude args must carry the prompt token: {:?}",
                    args
                );
            }
            other => panic!("claude_code must be ExternalCli, got {:?}", other),
        }
        assert_eq!(
            roster["claude_code"].availability_check,
            "bin_exists:claude"
        );

        // Codex: external CLI fast-follow.
        assert!(matches!(
            roster["codex"].engine,
            WorkerEngineKind::ExternalCli { .. }
        ));

        // Librarian: registered + probed, but engine pending (never dispatched).
        assert_eq!(roster["librarian"].engine, WorkerEngineKind::Pending);
        assert_eq!(
            roster["librarian"].availability_check,
            "model_loaded:qwen2.5"
        );
    }

    #[test]
    fn load_seeds_roster_when_workers_empty() {
        // The seeding branch load_agent_config runs when workers is empty.
        let mut config = AgentConfig::default();
        assert!(config.workers.is_empty());
        if config.workers.is_empty() {
            config.workers = default_roster();
        }
        assert_eq!(config.workers.len(), 3);
    }

    #[test]
    fn select_excludes_pending_and_picks_claude_code_for_a_code_goal() {
        use crate::goal_state::{select_best_worker, WorkerCandidate};

        // Mirror select_worker's candidate build: Pending excluded, all probed
        // available, no active sessions.
        let candidates: Vec<WorkerCandidate> = default_roster()
            .iter()
            .filter(|(_, p)| !matches!(p.engine, WorkerEngineKind::Pending))
            .map(|(k, p)| WorkerCandidate {
                key: k.clone(),
                available: true,
                tool_kinds: p.tool_kinds.clone(),
                cost_tier: p.cost_tier.clone(),
                active_sessions: 0,
            })
            .collect();

        assert!(
            !candidates.iter().any(|c| c.key == "librarian"),
            "the engine-pending librarian must be excluded from selection"
        );

        // claude_code and codex are both subscription/code-capable; the
        // deterministic alphabetical tie-break picks claude_code.
        let chosen =
            select_best_worker(&candidates, &["code_edit".to_string(), "shell".to_string()])
                .expect("a code worker should be selected");
        assert_eq!(
            chosen, "claude_code",
            "expected the Claude Code reference worker"
        );
    }

    /// #467: `timeout_secs` is an optional per-worker override; absent means
    /// the goal-engine default (2 h) applies at the dispatch site.
    #[test]
    fn worker_timeout_secs_parses_and_defaults_to_none() {
        let with: WorkerPersona =
            serde_yaml::from_str("first_name: W\ntimeout_secs: 7200\n").unwrap();
        assert_eq!(with.timeout_secs, Some(7200));

        let without: WorkerPersona = serde_yaml::from_str("first_name: W\n").unwrap();
        assert_eq!(without.timeout_secs, None);
        assert_eq!(
            crate::agents::platform_extensions::goal_engine::DEFAULT_EXTERNAL_CLI_TIMEOUT_SECS,
            2 * 60 * 60,
            "the #467 default is 2 h"
        );
    }
}
