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
                    section: Some("agent"),
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
             including hands-free instructions like asking you to open a site and read it aloud. \
             The same on-device speech recognition also powers dictation — the user can speak a \
             note and have it transcribed to text locally, such as dictating a note onto a project",
        why_it_matters:
            "Many users drive you primarily by voice — treat spoken turns exactly like typed \
             ones. When asked to 'read' something aloud, answer in flowing sentences suited to \
             listening, not bullet fragments. NEVER spell a word out letter by letter. The \
             speech engine spells out any word it does not know, so coined names and product \
             words are the risk: when the user corrects your pronunciation, or you are about to \
             say a name you have not said before, call save_pronunciation with the word respelled \
             using REAL English words ('prop tech', 'co working', 'per ma gent') — never IPA and \
             never invented syllables, since the engine looks each part up and refuses a save it \
             would have to spell out — then say it back so they can confirm. Saved \
             once, it is correct forever, so teach a word the first time rather than working \
             around it",
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
    /// SUPERVISED external CLI (S1, #427): a stream-json Claude Code session
    /// launched into a VISIBLE Build-tab terminal with permission gates
    /// ENABLED, in an isolated git worktree. Unlike `ExternalCli` there is no
    /// `args` field — the flag roster is composed by the supervised launcher
    /// (`platform_extensions::supervised_cli`) because every flag is
    /// load-bearing for gate detection; only the binary is configurable.
    /// Opt-in via `agent.yaml` (`engine: { type: supervised_cli, bin: claude }`)
    /// until the S2 parser + S3 inbox bridge make supervision end-to-end.
    SupervisedCli { bin: String },
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
            Self::SupervisedCli { .. } => "supervised_cli",
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
    /// Optional explicit workflow-role tag for cost routing — one of
    /// `orchestrate`/`hard` (frontier reasoning), `edit`, `mechanical`, `review`,
    /// `local`. When set, it overrides the role derived from [`Self::tool_kinds`]
    /// (see `cost_router::role_map::derive_role`), so the operator can pin which
    /// configured per-role model a worker's goals route to. Absent → role is
    /// derived from the tool kinds; if neither yields a role, dispatch stays on
    /// the single session model (no baked default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_role: Option<String>,
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
            workflow_role: None,
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

    /// The cost-routing workflow role this worker's work plays — from its explicit
    /// [`Self::workflow_role`] tag if set, else derived from [`Self::tool_kinds`].
    /// `None` when neither yields a role, so dispatch stays on the single session
    /// model. Both dispatch paths (summon's `resolve_provider` and the goal engine)
    /// route by this. See `cost_router::role_map::derive_role`.
    pub fn routing_role(&self) -> Option<crate::cost_router::WorkflowRole> {
        crate::cost_router::derive_role(&self.tool_kinds, self.workflow_role.as_deref())
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

    // The Steward owns repo hygiene and CI. It already exists as a scheduled
    // recipe (the `git-steward` starter) and a safety core in crate::steward;
    // this roster entry is what lets that scheduled run carry the Steward's own
    // identity instead of borrowing Henry's. Without it the job resolves to the
    // primary persona, which is why Henry's HUD used to advertise the Steward's
    // cron as his own.
    //
    // `engine: Pending` deliberately — the Steward is not a dispatchable worker
    // (the roster's engine-pending entries are excluded from selection). It
    // proposes and detects; it does not take handed-off goals.
    roster.insert(
        "steward".to_string(),
        WorkerPersona {
            first_name: "Steward".to_string(),
            role: "Repo hygiene and CI — keeps git repos clean, investigates \
                   CI failures, proposes (never performs) destructive git ops"
                .to_string(),
            tool_kinds: vec!["shell".to_string()],
            availability_check: "bin_exists:git".to_string(),
            cost_tier: "local_free".to_string(),
            engine: WorkerEngineKind::Pending,
            ..Default::default()
        },
    );

    roster.insert(
        "strix".to_string(),
        WorkerPersona {
            first_name: "Strix".to_string(),
            role: "Security review — continuously probes the user's own projects for \
                   exposed secrets, vulnerable dependencies, injection and access-control \
                   weaknesses, and risky configuration; REPORTS findings and PROPOSES \
                   (never performs) anything intrusive"
                .to_string(),
            tool_kinds: vec!["shell".to_string(), "review".to_string()],
            availability_check: "bin_exists:docker".to_string(),
            cost_tier: "local_free".to_string(),
            engine: WorkerEngineKind::Pending,
            ..Default::default()
        },
    );

    roster.insert(
        "reviewer".to_string(),
        WorkerPersona {
            first_name: "Reviewer".to_string(),
            // The adversarial framing is injected into the subagent's system
            // prompt via `system_prompt_block()` — this IS the reviewer's mandate.
            role: "Independent adversarial code reviewer — a DIFFERENT engineer than the author. \
                   After the coding harness's own tests pass, you check the diff to REFUTE it: \
                   assume a bug until you have checked, and treat the author's reasoning as a \
                   claim to test, not evidence. Review through five lenses — correctness, \
                   security, performance, spec-fit, and test-integrity (were tests weakened or \
                   deleted to pass?). You are READ-ONLY: read and analyze, never edit. Default \
                   to reject — if you cannot confidently sign off, the verdict is UNCERTAIN, \
                   not APPROVE"
                .to_string(),
            // `review` derives WorkflowRole::Review (cost_router::role_map::derive_role), so a
            // configured REVIEW role→model routes this delegate to a DIFFERENT-family model;
            // unset ⇒ it falls back to the main session model (no baked-in vendor default).
            tool_kinds: vec!["review".to_string()],
            workflow_role: Some("review".to_string()),
            // Always runnable: it is an in-process subagent whose model is chosen by the
            // Review role→model map, not gated on any local binary.
            availability_check: "always".to_string(),
            cost_tier: "paid_api".to_string(),
            engine: WorkerEngineKind::InternalSubagent,
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
            Ok(content) => parse_agent_config(&content),
            Err(e) => {
                tracing::warn!(
                    "Could not read {} ({}); falling back to the default persona",
                    path.display(),
                    e
                );
                AgentConfig::default()
            }
        }
    };
    if config.workers.is_empty() {
        config.workers = default_roster();
    }
    config
}

/// Parse agent.yaml content. A parse failure falls back to defaults but is
/// LOUD about it — silently reverting to the default persona is exactly what
/// "my saved persona didn't persist" looks like to the user (#167).
fn parse_agent_config(content: &str) -> AgentConfig {
    match serde_yaml::from_str(content) {
        Ok(config) => config,
        Err(e) => {
            tracing::warn!(
                "agent.yaml did not parse ({}); the saved persona is being \
                 IGNORED and the default persona used instead. Fix or delete \
                 the file to clear this.",
                e
            );
            AgentConfig::default()
        }
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
    fn supervised_cli_engine_kind_roundtrips_and_is_selectable() {
        // S1 (#427): the agent.yaml opt-in shape for a supervised worker.
        let yaml = r#"
first_name: Claude Code (supervised)
tool_kinds: [code_edit, shell, git]
availability_check: "bin_exists:claude"
engine:
  type: supervised_cli
  bin: claude
"#;
        let persona: WorkerPersona = serde_yaml::from_str(yaml).unwrap();
        match &persona.engine {
            WorkerEngineKind::SupervisedCli { bin } => assert_eq!(bin, "claude"),
            other => panic!("expected SupervisedCli, got {:?}", other),
        }
        assert_eq!(persona.engine.label(), "supervised_cli");
        // Unlike Pending, a supervised worker IS dispatchable (select_worker
        // filters only Pending engines).
        assert!(!matches!(persona.engine, WorkerEngineKind::Pending));
        // And it round-trips through serialization unchanged.
        let out = serde_yaml::to_string(&persona).unwrap();
        let back: WorkerPersona = serde_yaml::from_str(&out).unwrap();
        assert_eq!(back.engine, persona.engine);
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

    // ── Persona save/load round-trip (#167: "save not persisting") ─────

    /// The exact bytes save_agent_config writes must load back to the same
    /// persona — a serialize/deserialize mismatch here IS the saved-persona-
    /// silently-reverts-on-restart bug.
    #[test]
    fn saved_persona_yaml_round_trips() {
        let config = AgentConfig {
            primary: PrimaryPersona {
                first_name: "Henry".into(),
                last_name: Some("Permagent".into()),
                nickname: Some("H".into()),
                traits: vec!["curious".into(), "direct".into()],
                tone: "Warm, direct".into(),
                opening_greeting: "Hey boss!".into(),
                voice_id: Some("af_heart".into()),
            },
            workers: default_roster(),
        };
        let yaml = serde_yaml::to_string(&config).unwrap();
        let loaded = parse_agent_config(&yaml);
        assert_eq!(loaded.primary.first_name, "Henry");
        assert_eq!(loaded.primary.nickname.as_deref(), Some("H"));
        assert_eq!(loaded.primary.opening_greeting, "Hey boss!");
        assert_eq!(loaded.primary.voice_id.as_deref(), Some("af_heart"));
        assert_eq!(loaded.workers.len(), config.workers.len());
    }

    /// Corrupt yaml falls back to defaults (loudly, via tracing) instead of
    /// crashing — pins the documented degradation path.
    #[test]
    fn corrupt_agent_yaml_falls_back_to_default() {
        let loaded = parse_agent_config("primary: [this is: not, valid");
        assert_eq!(
            loaded.primary.first_name,
            PrimaryPersona::default().first_name
        );
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
    fn routing_role_derives_from_tool_kinds_and_tag() {
        use crate::cost_router::WorkflowRole;
        let roster = default_roster();
        // The coding workers (code_edit) route to the EDIT role's configured model.
        assert_eq!(
            roster["claude_code"].routing_role(),
            Some(WorkflowRole::Edit)
        );
        assert_eq!(roster["codex"].routing_role(), Some(WorkflowRole::Edit));
        // The Librarian (memory_ops, read-only) routes to MECHANICAL.
        assert_eq!(
            roster["librarian"].routing_role(),
            Some(WorkflowRole::Mechanical)
        );
        // The Reviewer routes to REVIEW — the cross-vendor critic role.
        assert_eq!(
            roster["reviewer"].routing_role(),
            Some(WorkflowRole::Review)
        );

        // An explicit workflow_role tag overrides the tool-kind derivation.
        let tagged = WorkerPersona {
            tool_kinds: vec!["code_edit".to_string()],
            workflow_role: Some("review".to_string()),
            ..Default::default()
        };
        assert_eq!(tagged.routing_role(), Some(WorkflowRole::Review));

        // A worker with no role signal → None (dispatch stays single-model).
        let bare = WorkerPersona {
            first_name: "Nobody".to_string(),
            ..Default::default()
        };
        assert_eq!(bare.routing_role(), None);
    }

    #[test]
    fn default_roster_seeds_expected_workers_with_engines() {
        let roster = default_roster();
        // Assert the actual keys, not a bare count — a count tells you the
        // roster changed but not how, and this assertion has already drifted
        // from its own name once ("three" while asserting four).
        let mut keys: Vec<&str> = roster.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "claude_code",
                "codex",
                "librarian",
                "reviewer",
                "steward",
                "strix",
            ],
        );

        // The reviewer: in-process subagent, review-tagged (routes to the Review
        // role's cross-vendor model), always runnable — no external binary.
        let reviewer = &roster["reviewer"];
        assert_eq!(reviewer.engine, WorkerEngineKind::InternalSubagent);
        assert_eq!(reviewer.availability_check, "always");
        assert_eq!(
            reviewer.routing_role(),
            Some(crate::cost_router::WorkflowRole::Review),
            "the reviewer must route to the Review role for cross-vendor dispatch"
        );

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

        // Steward: owns repo hygiene / CI. Registered so its scheduled recipe
        // carries the Steward's identity rather than borrowing the primary's,
        // but engine-pending so it is never dispatched a goal — it detects and
        // proposes, it does not take handed-off work.
        assert_eq!(roster["steward"].engine, WorkerEngineKind::Pending);
        assert_eq!(roster["steward"].availability_check, "bin_exists:git");
    }

    #[test]
    fn load_seeds_roster_when_workers_empty() {
        // The seeding branch load_agent_config runs when workers is empty.
        let mut config = AgentConfig::default();
        assert!(config.workers.is_empty());
        if config.workers.is_empty() {
            config.workers = default_roster();
        }
        assert_eq!(config.workers.len(), default_roster().len());
        assert!(config.workers.contains_key("steward"));
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
