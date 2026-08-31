#[cfg(test)]
use chrono::DateTime;
use chrono::Utc;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

use crate::agents::extension::ExtensionInfo;
use crate::cost_router::cache::SystemPromptParts;
use crate::hints::load_hints::build_gitignore;
use crate::hints::{get_context_filenames, load_hint_files, SubdirectoryHintTracker};
use crate::providers::model_family::ModelFamily;
use crate::{
    config::{Config, GooseMode},
    prompt_template,
    utils::sanitize_unicode_tags,
};
use std::path::Path;

const MAX_EXTENSIONS: usize = 5;
const MAX_TOOLS: usize = 50;

/// One resolution of "who is this agent", shared by the system prompt and the
/// public accessors so they can never disagree.
struct ResolvedPersona {
    block: String,
    display_name: String,
    opening_greeting: String,
}

pub struct PromptManager {
    system_prompt_override: Option<String>,
    system_prompt_extras: IndexMap<String, String>,
    current_date_timestamp: String,
    subdirectory_hint_tracker: SubdirectoryHintTracker,
    persona: Option<crate::config::agent_identity::SharedPersona>,
    /// Last-known-good persona value, refreshed on every successful read in
    /// `build()`. Used as the fallback when `persona.try_read()` fails under
    /// lock contention (e.g. a concurrent identity hot-reload) so a turn never
    /// silently reverts to the default ("Aria") persona. std RwLock (not tokio)
    /// because `build()` is synchronous and only ever clones a small value.
    last_good_persona: std::sync::RwLock<Option<crate::config::agent_identity::PrimaryPersona>>,
    /// Override persona block and display name (used for worker personas).
    persona_block_override: Option<(String, String)>,
    /// Worker key from agent.yaml (`financier`, `reviewer`, …) when this
    /// session is a specialist. `None` is the primary / Orchestrator session.
    worker_key: Option<String>,
}

impl Default for PromptManager {
    fn default() -> Self {
        PromptManager::new()
    }
}

#[derive(Serialize)]
struct SystemPromptContext {
    agent_persona_block: String,
    agent_display_name: String,
    /// The `permagent_self` brief — an authoritative, live inventory of the
    /// agent's own capabilities. Assembled each turn; empty string when not set.
    permagent_self_block: String,
    /// The `repo_map` system-prompt extra (`goose-cli`'s coding-harness
    /// orientation block), templated in its own slot right after the opening
    /// and BEFORE `permagent_self_block` rather than folded into the general
    /// `# Additional Instructions` join. Before this it landed after the
    /// capability inventory — 91% into a 90KB coding-harness prompt in a
    /// captured session (#1090) — so its offset grew with the inventory
    /// instead of staying fixed near the top. Empty string when no session
    /// registered a `repo_map` extra.
    repo_map_block: String,
    extensions: Vec<ExtensionInfo>,
    current_date_time: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    extension_tool_limits: Option<(usize, usize)>,
    goose_mode: GooseMode,
    is_autonomous: bool,
    enable_subagents: bool,
    max_extensions: usize,
    max_tools: usize,
    code_execution_mode: bool,
}

pub struct SystemPromptBuilder<'a, M> {
    manager: &'a M,

    extensions_info: Vec<ExtensionInfo>,
    frontend_instructions: Option<String>,
    extension_tool_count: Option<(usize, usize)>,
    subagents_enabled: bool,
    hints: Option<String>,
    code_execution_mode: bool,
    goose_mode: Option<GooseMode>,
    /// Live scheduled-job count for the self-knowledge brief (Queryable). Fetched
    /// async at the call site (the scheduler is not reachable in `build()`).
    /// `None` → the Scheduler worker renders without a live status.
    scheduled_job_count: Option<usize>,
    /// Workers the orchestrator can dispatch goals to, with live status. Fetched
    /// async at the call site (the probe may block). Empty → section omitted.
    dispatchable_workers: Vec<crate::agents::self_knowledge::DispatchableWorker>,
    agent_briefings: Option<Vec<crate::agents::self_knowledge::BriefingLine>>,
    /// The extensions this session explicitly declared (recipe/CLI runs),
    /// passed straight through to [`self_knowledge::SelfKnowledgeBuilder`] to
    /// scope the `## Tools you can call` section. `None` — the default, and
    /// what the daemon's resident sessions always pass — renders the full,
    /// unfiltered inventory. See `with_declared_extensions`.
    declared_extensions: Option<Vec<String>>,
    /// The model family answering this turn, if the caller knows it. `None` →
    /// no overlay at all, which is what every non-agent build site (recipes,
    /// tests) wants: they have no provider in hand, and inventing a family for
    /// them would put words in front of a model we did not identify.
    model_family: Option<ModelFamily>,
}

impl<'a> SystemPromptBuilder<'a, PromptManager> {
    pub fn with_extension(mut self, extension: ExtensionInfo) -> Self {
        self.extensions_info.push(extension);
        self
    }

    pub fn with_extensions(mut self, extensions: impl Iterator<Item = ExtensionInfo>) -> Self {
        for extension in extensions {
            self.extensions_info.push(extension);
        }
        self
    }

    pub fn with_frontend_instructions(mut self, frontend_instructions: Option<String>) -> Self {
        self.frontend_instructions = frontend_instructions;
        self
    }

    pub fn with_extension_and_tool_counts(
        mut self,
        extension_count: usize,
        tool_count: usize,
    ) -> Self {
        self.extension_tool_count = Some((extension_count, tool_count));
        self
    }

    pub fn with_code_execution_mode(mut self, enabled: bool) -> Self {
        self.code_execution_mode = enabled;
        self
    }

    pub fn with_hints(mut self, working_dir: &Path) -> Self {
        let hints_filenames = get_context_filenames();
        let ignore_patterns = build_gitignore(working_dir);

        let hints = load_hint_files(working_dir, &hints_filenames, &ignore_patterns);

        if !hints.is_empty() {
            self.hints = Some(hints);
        }
        self
    }

    pub fn with_enable_subagents(mut self, subagents_enabled: bool) -> Self {
        self.subagents_enabled = subagents_enabled;
        self
    }

    pub fn with_goose_mode(mut self, mode: GooseMode) -> Self {
        self.goose_mode = Some(mode);
        self
    }

    /// Provide the live scheduled-job count for the self-knowledge brief.
    pub fn with_scheduled_job_count(mut self, count: Option<usize>) -> Self {
        self.scheduled_job_count = count;
        self
    }

    /// Provide the dispatchable-worker list for the self-knowledge brief.
    pub fn with_dispatchable_workers(
        mut self,
        workers: Vec<crate::agents::self_knowledge::DispatchableWorker>,
    ) -> Self {
        self.dispatchable_workers = workers;
        self
    }

    /// Provide the unread agent briefings for the self-knowledge brief.
    pub fn with_agent_briefings(
        mut self,
        briefings: Option<Vec<crate::agents::self_knowledge::BriefingLine>>,
    ) -> Self {
        self.agent_briefings = briefings;
        self
    }

    /// Scope the self-knowledge capability inventory to this session's
    /// explicitly declared extensions (recipe/CLI runs — see the call site in
    /// `Agent::prepare_tools_and_prompt`, which passes `Some` only for
    /// `GoosePlatform::GooseCli` sessions). `None` (the default if this is
    /// never called) renders the full inventory — the product contract for
    /// the daemon's resident chat sessions, which must always be able to
    /// describe everything Permagent can do regardless of which extensions
    /// happen to be active this turn.
    pub fn with_declared_extensions(mut self, names: Option<Vec<String>>) -> Self {
        self.declared_extensions = names;
        self
    }

    /// Select the per-family prompt overlay from the provider and model that
    /// will answer this turn.
    ///
    /// The overlay is appended to the shared prompt body — one shared body plus
    /// a short family-specific block, never a separate prompt per family. It
    /// lands in the CACHED prefix on purpose: the family is fixed for as long as
    /// the session's provider is, so it is exactly as stable as the persona and
    /// the extension list it sits beside.
    pub fn with_model_family_from(mut self, provider: &str, model: &str) -> Self {
        self.model_family = Some(ModelFamily::resolve(provider, model));
        self
    }

    /// Select the overlay from an already-resolved family (tests, and callers
    /// that resolved it once for logging).
    pub fn with_model_family(mut self, family: ModelFamily) -> Self {
        self.model_family = Some(family);
        self
    }

    /// The rendered system prompt as one string — the stable prefix followed by
    /// the volatile suffix, with nothing inserted between them.
    pub fn build(self) -> String {
        self.build_parts().render()
    }

    /// Build the system prompt split into the byte-stable prefix that carries
    /// the provider `cache_control` breakpoint and the turn-volatile suffix that
    /// rides after it.
    ///
    /// **What is in the prefix:** persona/identity, the capability inventory,
    /// the extension instructions, the tool-count suggestion, the standing
    /// policy sections, and every system-prompt extra that is not turn-specific
    /// (saved skills, project hints, chat-mode, recipe instructions). These
    /// change only when the *user* changes something, and a bust is then
    /// correct.
    ///
    /// **What is in the suffix:** the live-status block (unread briefings, live
    /// worker counters, worker availability probes) and the extras listed in
    /// [`extra_is_volatile`]. Nothing here is dropped — it is context the agent
    /// needs — it is only moved behind the breakpoint.
    pub fn build_parts(self) -> SystemPromptParts {
        let mut extensions_info = self.extensions_info;

        // Add frontend instructions to extensions_info to simplify json rendering
        if let Some(frontend_instructions) = self.frontend_instructions {
            extensions_info.push(ExtensionInfo::new(
                "frontend",
                &frontend_instructions,
                false,
            ));
        }
        // Stable tool ordering is important for multi session prompt caching.
        extensions_info.sort_by(|a, b| a.name.cmp(&b.name));

        let sanitized_extensions_info: Vec<ExtensionInfo> = extensions_info
            .into_iter()
            .map(|mut ext_info| {
                ext_info.instructions = sanitize_unicode_tags(&ext_info.instructions);
                ext_info
            })
            .collect();

        let goose_mode = self
            .goose_mode
            .unwrap_or_else(|| Config::global().get_goose_mode().unwrap_or_default());

        let extension_tool_limits = self
            .extension_tool_count
            .filter(|(extensions, tools)| *extensions > MAX_EXTENSIONS || *tools > MAX_TOOLS);

        // Read persona for system prompt. Worker override takes precedence.
        let ResolvedPersona {
            block: persona_block,
            display_name,
            ..
        } = self.manager.resolve_persona();

        // Pulled out of `system_prompt_extras` into its own template slot
        // (see `SystemPromptContext::repo_map_block`) instead of riding in
        // the general `# Additional Instructions` join below — that join
        // lands after the capability inventory, and `#1090` found a captured
        // coding-harness prompt with its repo map at 91% depth as a result.
        // Extracted here, before that join runs, so it renders once, in its
        // own slot, and is removed from `system_prompt_extras` below so it
        // does not ALSO appear in the join.
        let repo_map_block = self
            .manager
            .system_prompt_extras
            .get("repo_map")
            .map(|s| sanitize_unicode_tags(s))
            .unwrap_or_default();

        // Assemble the permagent_self brief from the resolved display name (so
        // the persona name is interpolated, never hardcoded) plus any live
        // scheduled-job count fetched at the call site, and the feature flags
        // read here rather than inside the renderer (#995 — so a test can render
        // either state without this machine's config deciding for it). Split:
        // the capability inventory is templated into the prompt body; the
        // live-status half is held back for the volatile suffix.
        let (permagent_self_block, live_status_block) =
            crate::agents::self_knowledge::SelfKnowledgeBuilder {
                agent_display_name: display_name.clone(),
                scheduled_job_count: self.scheduled_job_count,
                flags: crate::agents::self_knowledge::FeatureFlags::from_live_config(),
                dispatchable_workers: self.dispatchable_workers.clone(),
                agent_briefings: self.agent_briefings.clone(),
                declared_extensions: self.declared_extensions.clone(),
            }
            .build_parts();

        let context = SystemPromptContext {
            agent_persona_block: persona_block.clone(),
            agent_display_name: display_name,
            permagent_self_block,
            repo_map_block,
            extensions: sanitized_extensions_info,
            current_date_time: self.manager.current_date_timestamp.clone(),
            extension_tool_limits,
            goose_mode,
            is_autonomous: goose_mode == GooseMode::Auto,
            enable_subagents: self.subagents_enabled,
            max_extensions: MAX_EXTENSIONS,
            max_tools: MAX_TOOLS,
            code_execution_mode: self.code_execution_mode,
        };

        let base_prompt = if let Some(override_prompt) = &self.manager.system_prompt_override {
            let sanitized_override_prompt = sanitize_unicode_tags(override_prompt);
            prompt_template::render_string(&sanitized_override_prompt, &context)
        } else {
            prompt_template::render_template("system.md", &context)
        }
        .unwrap_or(persona_block);

        // Append the per-family overlay exactly once, to the END of the shared
        // body and BEFORE `# Additional Instructions`, so the shared body is
        // byte-identical across families and the user's own extras still have
        // the last word. `Other` and `None` both contribute nothing — no
        // separator, no heading, no tokens.
        let base_prompt = match self.model_family.map(|f| f.overlay()).unwrap_or("") {
            "" => base_prompt,
            overlay => format!("{}\n\n{}", base_prompt.trim_end(), overlay.trim_end()),
        };

        let mut system_prompt_extras = self.manager.system_prompt_extras.clone();
        // Already rendered above into its own `repo_map_block` slot — drop it
        // here so it does not also land in the `# Additional Instructions`
        // join a second time.
        system_prompt_extras.shift_remove("repo_map");

        // Add hints if provided
        if let Some(hints) = self.hints {
            system_prompt_extras.insert("hints".to_string(), hints);
        }

        if goose_mode == GooseMode::Chat {
            system_prompt_extras.insert(
                "chat_mode".to_string(),
                "Right now you are in the chat only mode, no access to any tool use and system."
                    .to_string(),
            );
        }

        // Partition the extras by key BEFORE rendering. `system_prompt_extras`
        // is an IndexMap, so its order is insertion order — which for a
        // turn-specific extra means the position it lands in depends on which
        // turn it first appeared. Splitting on the key (not the position) is
        // what keeps the stable half's ordering independent of turn number.
        let mut stable_extras: Vec<String> = Vec::new();
        let mut volatile_extras: Vec<String> = Vec::new();
        for (key, extra) in system_prompt_extras {
            let sanitized = sanitize_unicode_tags(&extra);
            if extra_is_volatile(&key) {
                volatile_extras.push(sanitized);
            } else {
                stable_extras.push(sanitized);
            }
        }

        let stable_prefix = if stable_extras.is_empty() {
            base_prompt
        } else {
            format!(
                "{}\n\n# Additional Instructions:\n\n{}",
                base_prompt,
                stable_extras.join("\n\n")
            )
        };

        let mut volatile_suffix = live_status_block;
        if !volatile_extras.is_empty() {
            volatile_suffix.push_str("\n# Turn-specific Instructions\n\n");
            volatile_suffix.push_str(&volatile_extras.join("\n\n"));
            volatile_suffix.push('\n');
        }
        // The suffix concatenates directly onto the prefix (no separator), so it
        // must open its own gap; the prefix never ends in a blank line.
        if !volatile_suffix.is_empty() {
            volatile_suffix.insert(0, '\n');
        }

        SystemPromptParts::new(stable_prefix, volatile_suffix)
    }
}

/// System-prompt extras whose CONTENT is rebuilt from the current turn, keyed by
/// the key their producer registers them under. These are the reason the prompt
/// cache could not hit before this split: they sit inside `# Additional
/// Instructions`, in the middle of an otherwise fixed prompt, and they change on
/// literally every turn.
///
/// Each entry names its producer so a rename shows up in review:
const VOLATILE_EXTRA_KEYS: &[&str] = &[
    // Brain recall for THIS turn's user query — different memories, different
    // count, every turn (`goose-server::brain_ops`).
    "memory_recall",
    // The ambient-activity digest: `EVENTS_LAST_5MIN`, the last terminal
    // command, the current browser URL, and a `HH:MM`-stamped list of the last
    // 20 events (`activity::context_builder::render_ambient_context`). This is
    // the single most volatile block in the prompt — a wall-clock-windowed
    // counter and per-event timestamps, rebuilt on every turn.
    "ambient_context",
    // Which tab/panel the user is looking at and what they have selected — it
    // changes as they click around (`goose-server::routes::session_events`).
    "app_context",
    // The onboarding coach's once-a-day nudge: present on the turn after its
    // cooldown elapses, absent the next (`teachable::proactive_learn_next_hint`).
    "learn_next_offer",
    // Auto-skill proposals, injected MID-TURN at tool dispatch when repetition
    // is detected (`Agent::dispatch_tool_call`) — an insertion partway through
    // a session by construction.
    "skill_proposals",
];

/// Prefix for the per-subdirectory hint extras the agent accumulates as it works
/// (`SubdirectoryHintTracker::load_new_hints` keys them by directory path), so
/// each new directory touched appends another one mid-session.
const SUBDIR_HINTS_KEY_PREFIX: &str = "subdir_hints:";

/// Whether a system-prompt extra is turn-specific and therefore belongs behind
/// the cache breakpoint rather than inside the cached prefix.
///
/// Keyed on the extra's key rather than sniffed from its content: content
/// heuristics ("does it contain a timestamp?") fail open — a new volatile
/// producer that happens not to match the heuristic silently rejoins the cached
/// prefix, and the cost is invisible. A key list fails loudly instead: a new
/// producer is either listed here or it is not, and the reviewer sees which.
///
/// Everything else defaults to STABLE, deliberately. The rest of the extras —
/// recipe instructions, the app catalog, the final-output contract, the
/// voice-reply style, saved skills, project hints — are set once for the
/// session. Defaulting to volatile would push the whole `# Additional
/// Instructions` block out of the cache to guard against a hypothetical.
fn extra_is_volatile(key: &str) -> bool {
    VOLATILE_EXTRA_KEYS.contains(&key) || key.starts_with(SUBDIR_HINTS_KEY_PREFIX)
}

impl PromptManager {
    pub fn new() -> Self {
        PromptManager {
            system_prompt_override: None,
            system_prompt_extras: IndexMap::new(),
            current_date_timestamp: Utc::now().format("%Y-%m-%d %H:00").to_string(),
            subdirectory_hint_tracker: SubdirectoryHintTracker::new(),
            persona: None,
            last_good_persona: std::sync::RwLock::new(None),
            persona_block_override: None,
            worker_key: None,
        }
    }

    #[cfg(test)]
    pub fn with_timestamp(dt: DateTime<Utc>) -> Self {
        PromptManager {
            system_prompt_override: None,
            system_prompt_extras: IndexMap::new(),
            current_date_timestamp: dt.format("%Y-%m-%d %H:%M:%S").to_string(),
            subdirectory_hint_tracker: SubdirectoryHintTracker::new(),
            persona: None,
            last_good_persona: std::sync::RwLock::new(None),
            persona_block_override: None,
            worker_key: None,
        }
    }

    pub fn set_persona(&mut self, persona: crate::config::agent_identity::SharedPersona) {
        self.persona = Some(persona);
    }

    /// The persona the system prompt would be built from right now.
    ///
    /// Factored out of `SystemPromptBuilder::build_parts`, where it was
    /// inlined, so the accessors below and the prompt itself resolve identity
    /// through exactly ONE path — two copies is how a banner drifts from what
    /// the model was actually told.
    fn resolve_persona(&self) -> ResolvedPersona {
        // Worker override takes precedence.
        if let Some((ref block, ref name)) = self.persona_block_override {
            return ResolvedPersona {
                block: block.clone(),
                display_name: name.clone(),
                // A worker persona has no greeting of its own, and inheriting
                // the primary's would put the wrong voice in the wrong mouth.
                opening_greeting: String::new(),
            };
        }
        let persona = match self.persona.as_ref().and_then(|p| p.try_read().ok()) {
            // Live read succeeded: this is the source of truth. Refresh
            // the last-known-good cache so a later contended turn has a
            // current value to fall back to (renames included).
            Some(guard) => {
                let persona = guard.clone();
                if let Ok(mut cache) = self.last_good_persona.write() {
                    *cache = Some(persona.clone());
                }
                persona
            }
            // Live read failed (lock contention, e.g. a concurrent
            // identity hot-reload). Fall back to the last-known-good
            // value rather than silently reverting to the default
            // ("Aria") persona. Only truly defaults if no successful
            // read has happened yet (pre-startup-load).
            None => self
                .last_good_persona
                .read()
                .ok()
                .and_then(|cache| cache.clone())
                .unwrap_or_default(),
        };
        ResolvedPersona {
            block: persona.system_prompt_block(),
            display_name: persona.display_name(),
            opening_greeting: persona.opening_greeting.clone(),
        }
    }

    /// The display name that would be interpolated into the system prompt
    /// right now. Exists so callers outside `permagent::agents` can assert
    /// persona identity behaviourally — and so the CLI banner can name whoever
    /// the model is actually being told it is.
    pub fn display_name(&self) -> String {
        self.resolve_persona().display_name
    }

    /// The persona's own opening line, for surfaces that greet the user
    /// client-side (Chat already does; the CLI now does too). Empty when the
    /// persona sets none, or when a worker override is installed — a worker
    /// does not greet anyone.
    pub fn opening_greeting(&self) -> String {
        self.resolve_persona().opening_greeting
    }

    pub fn set_persona_block_override(&mut self, block: String, display_name: String) {
        self.persona_block_override = Some((block, display_name));
    }

    pub fn set_worker_key(&mut self, key: Option<String>) {
        self.worker_key = key;
    }

    pub fn worker_key(&self) -> Option<&str> {
        self.worker_key.as_deref()
    }

    /// Add an additional instruction to the system prompt with a key
    /// Using the same key will replace the previous instruction
    pub fn add_system_prompt_extra(&mut self, key: String, instruction: String) {
        self.system_prompt_extras.insert(key, instruction);
    }

    pub fn record_tool_arguments(
        &mut self,
        arguments: &Option<serde_json::Map<String, serde_json::Value>>,
        working_dir: &Path,
    ) {
        self.subdirectory_hint_tracker
            .record_tool_arguments(arguments, working_dir);
    }

    pub fn load_subdirectory_hints(&mut self, working_dir: &Path) -> bool {
        let new_hints = self.subdirectory_hint_tracker.load_new_hints(working_dir);
        let has_new = !new_hints.is_empty();
        for (key, content) in new_hints {
            self.system_prompt_extras.insert(key, content);
        }
        has_new
    }

    /// Override the system prompt with custom text
    pub fn set_system_prompt_override(&mut self, template: String) {
        self.system_prompt_override = Some(template);
    }

    pub fn builder<'a>(&'a self) -> SystemPromptBuilder<'a, Self> {
        SystemPromptBuilder {
            manager: self,

            extensions_info: vec![],
            frontend_instructions: None,
            extension_tool_count: None,
            subagents_enabled: false,
            hints: None,
            code_execution_mode: false,
            goose_mode: None,
            scheduled_job_count: None,
            dispatchable_workers: Vec::new(),
            agent_briefings: None,
            declared_extensions: None,
            model_family: None,
        }
    }

    pub async fn get_recipe_prompt(&self) -> String {
        let context: HashMap<&str, Value> = HashMap::new();
        prompt_template::render_template("recipe.md", &context)
            .unwrap_or_else(|_| "The recipe prompt is busted. Tell the user.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;

    use super::*;
    use crate::config::agent_identity::PrimaryPersona;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn persona_named(name: &str, greeting: &str) -> crate::config::agent_identity::SharedPersona {
        Arc::new(RwLock::new(PrimaryPersona {
            first_name: name.to_string(),
            opening_greeting: greeting.to_string(),
            ..Default::default()
        }))
    }

    /// `Agent::prompt_manager` is private to this module, so the CLI's only
    /// coverage for "the persona actually reached the prompt" was a grep of
    /// its own source for the constructor call. This accessor is what makes
    /// that assertable for real, and it must report the installed persona
    /// rather than the first-run fallback.
    #[test]
    fn display_name_reports_the_installed_persona_not_the_fallback() {
        let mut manager = PromptManager::new();
        let fallback = manager.display_name();
        manager.set_persona(persona_named("Henry", "What are we building?"));
        assert_eq!(manager.display_name(), "Henry");
        assert_eq!(manager.opening_greeting(), "What are we building?");
        assert_ne!(fallback, "Henry", "the fallback must be a different name");
    }

    /// A worker override is what the prompt really carries, so it has to be
    /// what the accessor reports — a banner naming the primary persona while
    /// the model was told it is someone else is worse than no banner.
    #[test]
    fn a_worker_override_outranks_the_shared_persona() {
        let mut manager = PromptManager::new();
        manager.set_persona(persona_named("Henry", "What are we building?"));
        manager.set_persona_block_override("Your name is Steward.".into(), "Steward".into());
        assert_eq!(manager.display_name(), "Steward");
        assert_eq!(manager.opening_greeting(), "", "a worker greets no one");
    }

    /// The accessor and the prompt must read the SAME resolution — two copies
    /// of this logic is how the banner would drift from the system prompt.
    #[test]
    fn the_accessor_and_the_built_prompt_agree_on_the_name() {
        let mut manager = PromptManager::new();
        manager.set_persona(persona_named("Henry", "hi"));
        let name = manager.display_name();
        assert_eq!(name, "Henry");
        let prompt = manager.builder().build();
        assert!(prompt.contains(&name), "{name} missing from the prompt");
    }

    fn briefing(summary: &str) -> crate::agents::self_knowledge::BriefingLine {
        crate::agents::self_knowledge::BriefingLine {
            from: "Steward".to_string(),
            severity: "info".to_string(),
            summary: summary.to_string(),
        }
    }

    /// The property this whole phase exists for: within one session, two turns
    /// whose only differences are turn-specific must produce a BYTE-IDENTICAL
    /// stable prefix. Provider prompt caches are prefix-exact, so one drifting
    /// byte here costs the entire cached prompt.
    #[test]
    fn stable_prefix_is_byte_identical_across_turns_in_one_session() {
        // Pin config resolution to an empty temp root so the build is a
        // function of its inputs, not of this machine's real config.
        let tmp_dir = tempfile::tempdir().unwrap();
        let temp_root = tmp_dir.path().display().to_string();
        let _guard = env_lock::lock_env([
            ("HOME", Some(temp_root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(temp_root.as_str())),
            ("INITIATIVE_ENABLED", Some("false")),
        ]);

        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());

        // Turn 1: two jobs scheduled, one unread briefing, a worker available.
        let turn1 = manager
            .builder()
            .with_extension(ExtensionInfo::new("test", "how to use it", true))
            .with_scheduled_job_count(Some(2))
            .with_agent_briefings(Some(vec![briefing("branch delete proposed")]))
            .with_dispatchable_workers(vec![crate::agents::self_knowledge::DispatchableWorker {
                display_name: "Claude Code".to_string(),
                status: "available".to_string(),
            }])
            .build_parts();

        // Turn 2: the scheduler fired, the briefing was acknowledged on read
        // (Info-severity ones are — see `reply_parts`), the availability probe
        // flipped. Every one of these moves on its own, with no user action.
        let turn2 = manager
            .builder()
            .with_extension(ExtensionInfo::new("test", "how to use it", true))
            .with_scheduled_job_count(Some(9))
            .with_agent_briefings(Some(vec![]))
            .with_dispatchable_workers(vec![crate::agents::self_knowledge::DispatchableWorker {
                display_name: "Claude Code".to_string(),
                status: "unavailable: not signed in".to_string(),
            }])
            .build_parts();

        assert_eq!(
            turn1.stable_prefix(),
            turn2.stable_prefix(),
            "turn-specific state leaked into the cached prefix"
        );
        assert_eq!(turn1.prefix_hash(), turn2.prefix_hash());

        // Non-vacuity in the other direction: the turn-specific state was not
        // silently dropped to buy the byte equality above — it moved.
        assert_ne!(turn1.volatile_suffix(), turn2.volatile_suffix());
        assert!(turn1.volatile_suffix().contains("branch delete proposed"));
        assert!(turn1.volatile_suffix().contains("2 job(s) scheduled"));
        assert!(turn2.volatile_suffix().contains("9 job(s) scheduled"));
        assert!(turn2
            .volatile_suffix()
            .contains("unavailable: not signed in"));

        // …and none of it is in the prefix.
        for needle in [
            "branch delete proposed",
            "job(s) scheduled",
            "unavailable: not signed in",
        ] {
            assert!(
                !turn1.stable_prefix().contains(needle),
                "{needle:?} belongs behind the cache breakpoint"
            );
        }
    }

    /// The guard against a vacuous pass. A test that asserts byte equality would
    /// also pass if `stable_prefix()` returned a constant — so a deliberate
    /// persona change MUST move the prefix.
    #[tokio::test]
    async fn a_persona_change_produces_a_different_stable_prefix() {
        use crate::config::agent_identity::{PrimaryPersona, SharedPersona};
        use std::sync::Arc;
        use tokio::sync::RwLock;

        // Pin config resolution to an empty temp root so the build is a
        // function of its inputs, not of this machine's real config.
        let tmp_dir = tempfile::tempdir().unwrap();
        let temp_root = tmp_dir.path().display().to_string();
        let _guard = env_lock::lock_env([
            ("HOME", Some(temp_root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(temp_root.as_str())),
            ("INITIATIVE_ENABLED", Some("false")),
        ]);

        let build_with = |first_name: &str| {
            let shared: SharedPersona = Arc::new(RwLock::new(PrimaryPersona {
                first_name: first_name.into(),
                ..PrimaryPersona::default()
            }));
            let mut manager =
                PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());
            manager.set_persona(shared);
            manager.builder().build_parts()
        };

        let zephyr = build_with("Zephyr");
        let nova = build_with("Nova");

        assert_ne!(
            zephyr.stable_prefix(),
            nova.stable_prefix(),
            "a persona rename must bust the cached prefix — it changes who the agent is"
        );
        assert_ne!(zephyr.prefix_hash(), nova.prefix_hash());
        assert!(zephyr.stable_prefix().contains("Zephyr"));
        assert!(nova.stable_prefix().contains("Nova"));

        // Rebuilding the same persona is stable — the difference above is the
        // persona, not build-to-build noise.
        assert_eq!(build_with("Zephyr").stable_prefix(), zephyr.stable_prefix());
    }

    /// `build()` must remain exactly prefix + suffix. A separator inserted here
    /// would mean flattening providers see a different prompt than split ones.
    #[test]
    fn build_is_the_concatenation_of_the_two_parts() {
        // Pin config resolution to an empty temp root so the build is a
        // function of its inputs, not of this machine's real config.
        let tmp_dir = tempfile::tempdir().unwrap();
        let temp_root = tmp_dir.path().display().to_string();
        let _guard = env_lock::lock_env([
            ("HOME", Some(temp_root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(temp_root.as_str())),
            ("INITIATIVE_ENABLED", Some("false")),
        ]);

        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());
        let parts = manager
            .builder()
            .with_scheduled_job_count(Some(4))
            .build_parts();
        let flat = manager.builder().with_scheduled_job_count(Some(4)).build();

        assert_eq!(
            format!("{}{}", parts.stable_prefix(), parts.volatile_suffix()),
            flat
        );
    }

    /// Extras keyed as turn-specific ride behind the breakpoint; everything else
    /// stays in the cached prefix. The onboarding nudge appears on one turn and
    /// is gone the next — inside the prefix it would cost the whole prompt.
    #[test]
    fn volatile_extras_ride_behind_the_breakpoint() {
        // Pin config resolution to an empty temp root so the build is a
        // function of its inputs, not of this machine's real config.
        let tmp_dir = tempfile::tempdir().unwrap();
        let temp_root = tmp_dir.path().display().to_string();
        let _guard = env_lock::lock_env([
            ("HOME", Some(temp_root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(temp_root.as_str())),
            ("INITIATIVE_ENABLED", Some("false")),
        ]);

        let mut manager =
            PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());
        // Every real producer of a per-turn extra, keyed exactly as it registers
        // it. If one of these keys is renamed at its producer without being
        // renamed here, this test still passes — which is why each entry in
        // VOLATILE_EXTRA_KEYS names its producer in a comment.
        for key in VOLATILE_EXTRA_KEYS {
            manager.add_system_prompt_extra((*key).to_string(), format!("VOLATILE::{key}"));
        }
        manager.add_system_prompt_extra("subdir_hints:/x/y".into(), "VOLATILE::subdir".into());
        // …and a representative stable one.
        manager.add_system_prompt_extra("saved_skills".into(), "STABLE::skills".into());

        let parts = manager.builder().build_parts();

        assert!(parts.stable_prefix().contains("STABLE::skills"));
        for key in VOLATILE_EXTRA_KEYS {
            let marker = format!("VOLATILE::{key}");
            assert!(
                !parts.stable_prefix().contains(&marker),
                "{key:?} is rebuilt every turn and must not sit in the cached prefix"
            );
            assert!(parts.volatile_suffix().contains(&marker));
        }
        assert!(!parts.stable_prefix().contains("VOLATILE::subdir"));
        assert!(parts.volatile_suffix().contains("VOLATILE::subdir"));

        // Nothing was dropped to buy the separation — determinism bought by
        // deleting context is the failure mode this guards against.
        for key in VOLATILE_EXTRA_KEYS {
            assert!(parts.render().contains(&format!("VOLATILE::{key}")));
        }
    }

    /// The load-bearing property: ONE shared body, not one prompt per family.
    /// Strip each family's overlay off the end of its prefix and what is left
    /// must be byte-identical everywhere — including against a build that
    /// selected no family at all.
    #[test]
    fn the_shared_body_is_identical_across_families() {
        // Pin config resolution to an empty temp root so the build is a
        // function of its inputs, not of this machine's real config.
        let tmp_dir = tempfile::tempdir().unwrap();
        let temp_root = tmp_dir.path().display().to_string();
        let _guard = env_lock::lock_env([
            ("HOME", Some(temp_root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(temp_root.as_str())),
            ("INITIATIVE_ENABLED", Some("false")),
        ]);
        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());

        let baseline = manager
            .builder()
            .with_extension(ExtensionInfo::new("test", "how to use it", true))
            .build_parts();
        let baseline_body = baseline.stable_prefix().trim_end().to_string();

        for family in ModelFamily::ALL {
            let parts = manager
                .builder()
                .with_extension(ExtensionInfo::new("test", "how to use it", true))
                .with_model_family(*family)
                .build_parts();
            let prefix = parts.stable_prefix();
            let overlay = family.overlay().trim_end();
            // `strip_suffix` rather than a byte slice: it is the same check and
            // the same cut in one step, and it cannot land mid-codepoint.
            let body = if overlay.is_empty() {
                prefix.trim_end().to_string()
            } else {
                prefix
                    .strip_suffix(overlay)
                    .unwrap_or_else(|| {
                        panic!(
                            "{} overlay must sit at the end of the shared body",
                            family.as_str()
                        )
                    })
                    .trim_end()
                    .to_string()
            };
            assert_eq!(
                body,
                baseline_body,
                "{} changed the shared body — overlays are additive only",
                family.as_str()
            );
        }
    }

    /// A family's overlay must appear exactly once, and a rebuild must not
    /// accumulate copies of it.
    #[test]
    fn the_overlay_is_applied_exactly_once() {
        // Pin config resolution to an empty temp root so the build is a
        // function of its inputs, not of this machine's real config.
        let tmp_dir = tempfile::tempdir().unwrap();
        let temp_root = tmp_dir.path().display().to_string();
        let _guard = env_lock::lock_env([
            ("HOME", Some(temp_root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(temp_root.as_str())),
            ("INITIATIVE_ENABLED", Some("false")),
        ]);
        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());

        for family in ModelFamily::ALL {
            let overlay = family.overlay().trim_end();
            if overlay.is_empty() {
                continue;
            }
            let rendered = manager
                .builder()
                .with_extension(ExtensionInfo::new("test", "how to use it", true))
                .with_model_family(*family)
                .build();
            assert_eq!(
                rendered.matches(overlay).count(),
                1,
                "{} overlay appeared {} times",
                family.as_str(),
                rendered.matches(overlay).count()
            );
            // Rebuilding from the same manager must not append a second copy.
            let again = manager
                .builder()
                .with_extension(ExtensionInfo::new("test", "how to use it", true))
                .with_model_family(*family)
                .build();
            assert_eq!(again, rendered);
        }
    }

    /// The overlay rides in the CACHED prefix, not the volatile suffix — the
    /// family is fixed for the session, so paying for it every turn would be a
    /// straight cache regression.
    #[test]
    fn the_overlay_rides_in_the_cached_prefix() {
        // Pin config resolution to an empty temp root so the build is a
        // function of its inputs, not of this machine's real config.
        let tmp_dir = tempfile::tempdir().unwrap();
        let temp_root = tmp_dir.path().display().to_string();
        let _guard = env_lock::lock_env([
            ("HOME", Some(temp_root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(temp_root.as_str())),
            ("INITIATIVE_ENABLED", Some("false")),
        ]);
        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());
        let parts = manager
            .builder()
            .with_model_family(ModelFamily::QwenLocal)
            .build_parts();
        assert!(parts
            .stable_prefix()
            .contains("Tool calls must be exact JSON"));
        assert!(!parts
            .volatile_suffix()
            .contains("Tool calls must be exact JSON"));
    }

    /// The user's own extras keep the last word: the overlay is a default the
    /// operator can override, so it must come BEFORE `# Additional
    /// Instructions`, not after.
    #[test]
    fn the_overlay_precedes_the_users_own_extras() {
        // Pin config resolution to an empty temp root so the build is a
        // function of its inputs, not of this machine's real config.
        let tmp_dir = tempfile::tempdir().unwrap();
        let temp_root = tmp_dir.path().display().to_string();
        let _guard = env_lock::lock_env([
            ("HOME", Some(temp_root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(temp_root.as_str())),
            ("INITIATIVE_ENABLED", Some("false")),
        ]);
        let mut manager =
            PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());
        manager.add_system_prompt_extra("recipe".into(), "OPERATOR_SAYS_THIS".into());
        let rendered = manager
            .builder()
            .with_model_family(ModelFamily::QwenLocal)
            .build();
        let overlay_at = rendered.find("Tool calls must be exact JSON").unwrap();
        let extra_at = rendered.find("OPERATOR_SAYS_THIS").unwrap();
        assert!(overlay_at < extra_at);
    }

    /// Hard ceiling on the UNSCOPED prefix — `GoosePlatform::GooseDesktop`,
    /// Aria's resident chat, which by product contract describes everything
    /// Permagent can do. Measured at 19,257 tokens (`qwen-local`, the largest
    /// overlay) when this gate was added; the headroom is ~4%, enough for a
    /// descriptor edit and nowhere near enough to hide a new always-on section.
    const UNSCOPED_PREFIX_TOKEN_CEILING: usize = 20_000;

    /// Hard ceiling on the SCOPED prefix — the `GooseCli` family: the
    /// interactive CLI, the coding harness, in-process goal workers and Summon
    /// subagents, all of which declare an explicit extension set. Measured at
    /// 3,533 tokens for the three extensions `permagent-coding` declares
    /// (developer, analyze, summon), down from 14,289 before the Workers and
    /// Surfaces inventories were scoped out with the tool list — 10,756 tokens
    /// off every turn of every such session. ~5% headroom.
    ///
    /// These two constants are the point of the test. The snapshot below shows
    /// a reviewer WHAT grew; the ceilings stop it growing back unnoticed —
    /// #1090 was a prefix that quietly reached 69KB with nobody watching. The
    /// gap between them is also the guard on the scoping itself: put a section
    /// back on the unscoped path and only the snapshot moves; put one back on
    /// BOTH paths and this ceiling fails.
    const SCOPED_PREFIX_TOKEN_CEILING: usize = 3_700;

    /// The extension set `crates/goose-cli/src/recipes/builtin/permagent-coding.yaml`
    /// declares. Representative of every scoped session shape.
    const CODING_HARNESS_EXTENSIONS: &[&str] = &["developer", "analyze", "summon"];

    /// The whole point of the change, measured: the per-family prompt cost sits
    /// in one snapshot, so a family growing an expensive habit is visible in a
    /// review diff rather than only on a bill — and past a stated ceiling it
    /// fails the build instead of merely showing up in a diff.
    #[test]
    fn family_prompt_size_table() {
        // Pin config resolution to an empty temp root so the build is a
        // function of its inputs, not of this machine's real config.
        let tmp_dir = tempfile::tempdir().unwrap();
        let temp_root = tmp_dir.path().display().to_string();
        let _guard = env_lock::lock_env([
            ("HOME", Some(temp_root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(temp_root.as_str())),
            ("INITIATIVE_ENABLED", Some("false")),
        ]);
        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());

        let declared: Option<Vec<String>> = Some(
            CODING_HARNESS_EXTENSIONS
                .iter()
                .map(|s| s.to_string())
                .collect(),
        );
        let shapes: [(&str, Option<Vec<String>>, usize); 2] = [
            (
                "unscoped (GooseDesktop — full product-contract inventory)",
                None,
                UNSCOPED_PREFIX_TOKEN_CEILING,
            ),
            (
                "scoped (GooseCli — declares developer, analyze, summon)",
                declared,
                SCOPED_PREFIX_TOKEN_CEILING,
            ),
        ];

        let mut table = String::new();
        for (label, declared_extensions, ceiling) in shapes {
            let build = |family: Option<ModelFamily>| {
                let mut b = manager
                    .builder()
                    .with_declared_extensions(declared_extensions.clone());
                if let Some(f) = family {
                    b = b.with_model_family(f);
                }
                b.build().len()
            };

            let baseline = build(None);
            table.push_str(&format!(
                "## {label}\nshared body (no family selected): {} bytes / ~{} tokens\n\n\
                 family        overlay_bytes  total_bytes  approx_total_tokens\n",
                baseline,
                baseline.div_ceil(4)
            ));
            let mut worst = baseline;
            for family in ModelFamily::ALL {
                let total = build(Some(*family));
                worst = worst.max(total);
                table.push_str(&format!(
                    "{:<13} {:>13}  {:>11}  {:>19}\n",
                    family.as_str(),
                    family.overlay().len(),
                    total,
                    total.div_ceil(4)
                ));
            }
            table.push('\n');

            let worst_tokens = worst.div_ceil(4);
            assert!(
                worst_tokens <= ceiling,
                "{label}: the system prefix reached ~{worst_tokens} tokens, over its \
                 stated ceiling of {ceiling}. Every turn of every such session pays this. \
                 Cut something, or raise the ceiling DELIBERATELY and say why here."
            );
        }
        assert_snapshot!(table);
    }

    #[test]
    fn test_build_system_prompt_sanitizes_override() {
        let mut manager = PromptManager::new();
        let malicious_override = "System prompt\u{E0041}\u{E0042}\u{E0043}with hidden text";
        manager.set_system_prompt_override(malicious_override.to_string());

        let result = manager.builder().build();

        assert!(!result.contains('\u{E0041}'));
        assert!(!result.contains('\u{E0042}'));
        assert!(!result.contains('\u{E0043}'));
        assert!(result.contains("System prompt"));
        assert!(result.contains("with hidden text"));
    }

    #[tokio::test]
    async fn test_persona_fallback_uses_last_good_not_default_under_contention() {
        use crate::config::agent_identity::{PrimaryPersona, SharedPersona};
        use std::sync::Arc;
        use tokio::sync::RwLock;

        let custom = PrimaryPersona {
            first_name: "Zephyr".into(),
            ..PrimaryPersona::default()
        };
        let shared: SharedPersona = Arc::new(RwLock::new(custom));

        let mut manager = PromptManager::new();
        manager.set_persona(shared.clone());

        // First build: live read succeeds and primes the last-known-good cache.
        let primed = manager.builder().build();
        assert!(
            primed.contains("Zephyr"),
            "primed build should reflect the live persona"
        );
        assert!(!primed.contains("Aria"));

        // Simulate a concurrent identity hot-reload holding the write lock, so
        // the next `try_read()` in build() fails.
        let write_guard = shared.write().await;

        // Contended build must fall back to the cached "Zephyr", NOT silently
        // revert to the default "Aria" persona.
        let contended = manager.builder().build();
        assert!(
            contended.contains("Zephyr"),
            "contended build must use last-known-good persona, got: {contended}"
        );
        assert!(
            !contended.contains("Aria"),
            "contended build must NOT revert to default Aria"
        );

        drop(write_guard);
    }

    #[test]
    fn test_build_system_prompt_sanitizes_extras() {
        let mut manager = PromptManager::new();
        let malicious_extra = "Extra instruction\u{E0041}\u{E0042}\u{E0043}hidden";
        manager.add_system_prompt_extra("test".to_string(), malicious_extra.to_string());

        let result = manager.builder().build();

        assert!(!result.contains('\u{E0041}'));
        assert!(!result.contains('\u{E0042}'));
        assert!(!result.contains('\u{E0043}'));
        assert!(result.contains("Extra instruction"));
        assert!(result.contains("hidden"));
    }

    #[test]
    fn test_build_system_prompt_sanitizes_multiple_extras() {
        let mut manager = PromptManager::new();
        manager
            .add_system_prompt_extra("test1".to_string(), "First\u{E0041}instruction".to_string());
        manager.add_system_prompt_extra(
            "test2".to_string(),
            "Second\u{E0042}instruction".to_string(),
        );
        manager
            .add_system_prompt_extra("test3".to_string(), "Third\u{E0043}instruction".to_string());

        let result = manager.builder().build();

        assert!(!result.contains('\u{E0041}'));
        assert!(!result.contains('\u{E0042}'));
        assert!(!result.contains('\u{E0043}'));
        assert!(result.contains("Firstinstruction"));
        assert!(result.contains("Secondinstruction"));
        assert!(result.contains("Thirdinstruction"));
    }

    #[test]
    fn test_build_system_prompt_preserves_legitimate_unicode_in_extras() {
        let mut manager = PromptManager::new();
        let legitimate_unicode = "Instruction with 世界 and 🌍 emojis";
        manager.add_system_prompt_extra("test".to_string(), legitimate_unicode.to_string());

        let result = manager.builder().build();

        assert!(result.contains("世界"));
        assert!(result.contains("🌍"));
        assert!(result.contains("Instruction with"));
        assert!(result.contains("emojis"));
    }

    #[test]
    fn test_build_system_prompt_sanitizes_extension_instructions() {
        let manager = PromptManager::new();
        let malicious_extension_info = ExtensionInfo::new(
            "test_extension",
            "Extension help\u{E0041}\u{E0042}\u{E0043}hidden instructions",
            false,
        );

        let result = manager
            .builder()
            .with_extension(malicious_extension_info)
            .build();

        assert!(!result.contains('\u{E0041}'));
        assert!(!result.contains('\u{E0042}'));
        assert!(!result.contains('\u{E0043}'));
        assert!(result.contains("Extension help"));
        assert!(result.contains("hidden instructions"));
    }

    #[test]
    fn test_tool_only_extension_section_suppressed() {
        let manager = PromptManager::new();

        let result = manager
            .builder()
            .with_extension(ExtensionInfo::new("toolonly", "", false))
            .with_extension(ExtensionInfo::new("documented", "use it like this", false))
            .build();

        // A tool-only extension (no instructions, no resources) must not render
        // an empty '## name' section into the brief (#637).
        assert!(!result.contains("## toolonly"));
        // Non-empty sections are unaffected.
        assert!(result.contains("## documented\n\n### Instructions\nuse it like this"));
        // The extensions block still renders (the extension list is non-empty),
        // so the no-extensions fallback must not appear.
        assert!(!result.contains("No extensions are defined"));
    }

    #[test]
    fn test_resources_only_extension_section_kept() {
        let manager = PromptManager::new();

        let result = manager
            .builder()
            .with_extension(ExtensionInfo::new("resourceful", "", true))
            .build();

        // Empty instructions but has_resources: the section still carries
        // content, so it must be kept (with no empty '### Instructions').
        assert!(result.contains("## resourceful\n\nresourceful supports resources.\n"));
        assert!(!result.contains("### Instructions"));
    }

    #[test]
    fn test_basic() {
        // Pin config resolution to an empty temp root so the rendered brief is
        // deterministic regardless of the machine's real config (e.g. local
        // initiative_enabled). Structural — not an operator-remembered env var.
        let tmp_dir = tempfile::tempdir().unwrap();
        let temp_root = tmp_dir.path().display().to_string();
        let _guard = env_lock::lock_env([
            ("HOME", Some(temp_root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(temp_root.as_str())),
            ("INITIATIVE_ENABLED", Some("false")),
        ]);

        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());

        let system_prompt = manager.builder().build();

        assert_snapshot!(system_prompt)
    }

    #[test]
    fn test_one_extension() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let temp_root = tmp_dir.path().display().to_string();
        let _guard = env_lock::lock_env([
            ("HOME", Some(temp_root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(temp_root.as_str())),
            ("INITIATIVE_ENABLED", Some("false")),
        ]);

        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());

        let system_prompt = manager
            .builder()
            .with_extension(ExtensionInfo::new(
                "test",
                "how to use this extension",
                true,
            ))
            .build();

        assert_snapshot!(system_prompt)
    }

    #[test]
    fn test_typical_setup() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let temp_root = tmp_dir.path().display().to_string();
        let _guard = env_lock::lock_env([
            ("HOME", Some(temp_root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(temp_root.as_str())),
            ("INITIATIVE_ENABLED", Some("false")),
        ]);

        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());

        let system_prompt = manager
            .builder()
            .with_extension(ExtensionInfo::new(
                "extension_A",
                "<instructions on how to use extension A>",
                true,
            ))
            .with_extension(ExtensionInfo::new(
                "extension_B",
                "<instructions on how to use extension B (no resources)>",
                false,
            ))
            .with_extension_and_tool_counts(MAX_EXTENSIONS + 1, MAX_TOOLS + 1)
            .build();

        assert_snapshot!(system_prompt)
    }

    #[tokio::test]
    async fn test_all_platform_extensions() {
        use crate::agents::platform_extensions::{PlatformExtensionContext, PLATFORM_EXTENSIONS};
        use crate::config::GooseMode;
        use crate::session::SessionManager;
        use std::sync::Arc;

        let tmp_dir = tempfile::tempdir().unwrap();
        let temp_root = tmp_dir.path().display().to_string();
        let _guard = env_lock::lock_env([
            ("HOME", Some(temp_root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(temp_root.as_str())),
            ("INITIATIVE_ENABLED", Some("false")),
        ]);
        let session_manager = Arc::new(SessionManager::new(tmp_dir.path().to_path_buf()));
        let session = session_manager
            .create_session(
                tmp_dir.path().to_path_buf(),
                "test session".to_owned(),
                crate::session::SessionType::Hidden,
                GooseMode::default(),
            )
            .await
            .unwrap();
        let context = PlatformExtensionContext {
            extension_manager: None,
            session_manager,
            session: Some(Arc::new(session)),
        };

        let mut extensions: Vec<ExtensionInfo> = PLATFORM_EXTENSIONS
            .values()
            .map(|def| {
                let client = (def.client_factory)(context.clone());
                let info = client.get_info();
                let instructions = info
                    .and_then(|i| i.instructions.clone())
                    .unwrap_or_default();
                let has_resources = info
                    .and_then(|i| i.capabilities.resources.as_ref())
                    .is_some();
                ExtensionInfo::new(def.name, &instructions, has_resources)
            })
            .collect();

        extensions.sort_by(|a, b| a.name.cmp(&b.name));

        let manager = PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());
        let system_prompt = manager
            .builder()
            .with_extensions(extensions.into_iter())
            .build();

        assert_snapshot!(system_prompt);
    }

    /// #1090: a captured coding-harness prompt put its `repo_map` extra at
    /// 91% depth, behind the full capability inventory, because it rode in
    /// the general `# Additional Instructions` join at the end of the prompt.
    /// It now renders in its own slot right after the opening and before the
    /// inventory (see `SystemPromptContext::repo_map_block`), so its offset
    /// stays near the top regardless of how large the inventory grows. Built
    /// through the real `PromptManager` builder — the thing that actually
    /// assembles a session's prompt — not by string concatenation, so this
    /// would catch a regression in the template or the extraction, not just
    /// in a hand-rolled string.
    #[test]
    fn repo_map_offset_stays_near_the_top_regardless_of_inventory_size() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let temp_root = tmp_dir.path().display().to_string();
        let _guard = env_lock::lock_env([
            ("HOME", Some(temp_root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(temp_root.as_str())),
            ("INITIATIVE_ENABLED", Some("false")),
        ]);

        let mut manager =
            PromptManager::with_timestamp(DateTime::<Utc>::from_timestamp(0, 0).unwrap());
        manager.add_system_prompt_extra(
            "repo_map".to_string(),
            "<repo_map>\nsrc/\n  main.rs\n</repo_map>".to_string(),
        );

        let parts = manager.builder().build_parts();
        let rendered = parts.render();

        // Sanity: this test is only meaningful with an inventory present —
        // otherwise a small prompt would trivially satisfy the 20% bound.
        assert!(
            rendered.contains("# Who You Are — Your Capabilities"),
            "capability inventory must be present for this test to be meaningful"
        );

        let offset = rendered
            .find("<repo_map>")
            .expect("repo_map block must be present in the rendered prompt");
        let total_len = rendered.len();
        let pct = offset as f64 / total_len as f64;

        assert!(
            pct < 0.20,
            "repo_map at byte {offset} of {total_len} ({:.1}%) is not near the top",
            pct * 100.0
        );

        // And it must be in the CACHE-STABLE half, not the volatile suffix —
        // the whole point of the stable/volatile split (see the comments
        // around `VOLATILE_EXTRA_KEYS`) is that the provider's cache
        // breakpoint sits after this content, not before it.
        assert!(parts.stable_prefix().contains("<repo_map>"));
        assert!(!parts.volatile_suffix().contains("<repo_map>"));
    }
}
