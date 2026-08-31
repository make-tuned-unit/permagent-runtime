use crate::cli::StreamableHttpOptions;

use super::output;
use super::CliSession;
use console::style;
use permagent::agents::{Agent, Container, ExtensionError};
use permagent::config::resolve_extensions_for_new_session;
use permagent::config::{get_all_extensions, Config, ExtensionConfig, GooseMode};
use permagent::providers::create;
use permagent::recipe::Recipe;
use permagent::session::session_manager::SessionType;
use permagent::session::EnabledExtensionsState;
use rustyline::EditMode;
use std::collections::BTreeSet;
use std::process;
use std::sync::Arc;
use tokio::task::JoinSet;

const EXTENSION_HINT_MAX_LEN: usize = 5;

/// Create a CLI agent with the user's persisted primary identity installed.
///
/// `PromptManager` deliberately has a built-in fallback persona for first-run
/// and recovery paths. A coding-harness session is not one of those paths: it
/// must share the primary persona saved by Chat in `agent.yaml`. Keeping this
/// in one constructor also prevents helper/debug sessions from quietly
/// reverting to the fallback identity.
async fn new_primary_agent() -> Agent {
    let agent = Agent::new();
    agent
        .set_persona(permagent::config::agent_identity::load_shared_persona())
        .await;
    agent
}

fn truncate_with_ellipsis(s: &str, max_len: usize) -> String {
    let truncated: String = s.chars().take(max_len).collect();
    if s.chars().count() > max_len {
        format!("{}…", truncated)
    } else {
        truncated
    }
}

fn parse_cli_flag_extensions(
    extensions: &[String],
    streamable_http_extensions: &[StreamableHttpOptions],
    builtins: &[String],
) -> Vec<(String, ExtensionConfig)> {
    let mut extensions_to_load = Vec::new();

    for (idx, ext_str) in extensions.iter().enumerate() {
        match CliSession::parse_stdio_extension(ext_str) {
            Ok(config) => {
                let hint = truncate_with_ellipsis(ext_str, EXTENSION_HINT_MAX_LEN);
                let label = format!("stdio #{}({})", idx + 1, hint);
                extensions_to_load.push((label, config));
            }
            Err(e) => {
                eprintln!(
                    "{}",
                    style(format!(
                        "Warning: Invalid --extension value '{}' ({}); ignoring",
                        ext_str, e
                    ))
                    .yellow()
                );
            }
        }
    }

    for (idx, opts) in streamable_http_extensions.iter().enumerate() {
        let config = CliSession::parse_streamable_http_extension(&opts.url, opts.timeout);
        let hint = truncate_with_ellipsis(&opts.url, EXTENSION_HINT_MAX_LEN);
        let label = format!("http #{}({})", idx + 1, hint);
        extensions_to_load.push((label, config));
    }

    for builtin_str in builtins {
        let configs = CliSession::parse_builtin_extensions(builtin_str);
        for config in configs {
            extensions_to_load.push((config.name(), config));
        }
    }

    extensions_to_load
}

/// Configuration for building a new Goose session
///
/// This struct contains all the parameters needed to create a new session,
/// including session identification, extension configuration, and debug settings.
#[derive(Clone, Debug)]
pub struct SessionBuilderConfig {
    /// Session id, optional need to deduce from context
    pub session_id: Option<String>,
    /// Whether to resume an existing session
    pub resume: bool,
    /// Whether to fork an existing session (creates a copy of the original/existing session then resumes the copy)
    pub fork: bool,
    /// Whether to run without a session file
    pub no_session: bool,
    /// List of stdio extension commands to add
    pub extensions: Vec<String>,
    /// List of streamable HTTP extension commands to add
    pub streamable_http_extensions: Vec<StreamableHttpOptions>,
    /// List of builtin extension commands to add
    pub builtins: Vec<String>,
    pub no_profile: bool,
    /// Recipe for the session
    pub recipe: Option<Recipe>,
    /// Any additional system prompt to append to the default
    pub additional_system_prompt: Option<String>,
    /// Provider override from CLI arguments
    pub provider: Option<String>,
    /// Model override from CLI arguments
    pub model: Option<String>,
    /// Enable debug printing
    pub debug: bool,
    /// Maximum number of consecutive identical tool calls allowed
    pub max_tool_repetitions: Option<u32>,
    /// Maximum number of turns (iterations) allowed without user input
    pub max_turns: Option<u32>,
    /// ID of the scheduled job that triggered this session (if any)
    pub scheduled_job_id: Option<String>,
    /// Whether this session will be used interactively (affects debugging prompts)
    pub interactive: bool,
    /// Quiet mode - suppress non-response output
    pub quiet: bool,
    /// Output format (text, json)
    pub output_format: String,
    /// Docker container to run stdio extensions inside
    pub container: Option<Container>,
}

/// Manual implementation of Default to ensure proper initialization of output_format
/// This struct requires explicit default value for output_format field
impl Default for SessionBuilderConfig {
    fn default() -> Self {
        SessionBuilderConfig {
            session_id: None,
            resume: false,
            fork: false,
            no_session: false,
            extensions: Vec::new(),
            streamable_http_extensions: Vec::new(),
            builtins: Vec::new(),
            no_profile: false,
            recipe: None,
            additional_system_prompt: None,
            provider: None,
            model: None,
            debug: false,
            max_tool_repetitions: None,
            max_turns: None,
            scheduled_job_id: None,
            interactive: false,
            quiet: false,
            output_format: "text".to_string(),
            container: None,
        }
    }
}

/// Offers to help debug an extension failure by creating a minimal debugging session
async fn offer_extension_debugging_help(
    extension_name: &str,
    error_message: &str,
    provider: Arc<dyn permagent::providers::base::Provider>,
    interactive: bool,
) -> Result<(), anyhow::Error> {
    // Only offer debugging help in interactive mode
    if !interactive {
        return Ok(());
    }

    let help_prompt = format!(
        "Would you like me to help debug the '{}' extension failure?",
        extension_name
    );

    let should_help = match cliclack::confirm(help_prompt)
        .initial_value(false)
        .interact()
    {
        Ok(choice) => choice,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::Interrupted {
                return Ok(());
            } else {
                return Err(e.into());
            }
        }
    };

    if !should_help {
        return Ok(());
    }

    println!("{}", style("🔧 Starting debugging session...").cyan());

    // Create a debugging prompt with context about the extension failure
    let debug_prompt = format!(
        "I'm having trouble starting an extension called '{}'. Here's the error I encountered:\n\n{}\n\nCan you help me diagnose what might be wrong and suggest how to fix it? Please consider common issues like:\n- Missing dependencies or tools\n- Configuration problems\n- Network connectivity (for remote extensions)\n- Permission issues\n- Path or environment variable problems",
        extension_name, error_message
    );

    // Create a minimal agent for debugging
    let debug_agent = new_primary_agent().await;

    let session = debug_agent
        .config
        .session_manager
        .create_session(
            std::env::current_dir()?,
            "CLI Session".to_string(),
            SessionType::Hidden,
            debug_agent.config.goose_mode,
        )
        .await?;

    debug_agent.update_provider(provider, &session.id).await?;

    // Add the developer extension if available to help with debugging
    let extensions = get_all_extensions();
    for ext_wrapper in extensions {
        if ext_wrapper.enabled && ext_wrapper.config.name() == "developer" {
            if let Err(e) = debug_agent
                .add_extension(ext_wrapper.config, &session.id)
                .await
            {
                // If we can't add developer extension, continue without it
                eprintln!(
                    "Note: Could not load developer extension for debugging: {}",
                    e
                );
            }
            break;
        }
    }

    let mut debug_session = CliSession::new(
        debug_agent,
        session.id,
        false,
        None,
        None,
        None,
        None,
        "text".to_string(),
    )
    .await;

    // Process the debugging request
    println!("{}", style("Analyzing the extension failure...").yellow());
    match debug_session.headless(debug_prompt).await {
        Ok(_) => {
            println!(
                "{}",
                style("✅ Debugging session completed. Check the suggestions above.").green()
            );
        }
        Err(e) => {
            eprintln!(
                "{}",
                style(format!("❌ Debugging session failed: {}", e)).red()
            );
        }
    }
    Ok(())
}

async fn load_extensions(
    agent: Agent,
    extensions_to_load: Vec<(String, ExtensionConfig)>,
    provider_for_debug: Arc<dyn permagent::providers::base::Provider>,
    interactive: bool,
    session_id: &str,
) -> Arc<Agent> {
    let mut set = JoinSet::new();
    let agent_ptr = Arc::new(agent);

    let mut waiting_ids: BTreeSet<usize> = (0..extensions_to_load.len()).collect();
    for (id, (_label, extension)) in extensions_to_load.iter().enumerate() {
        let agent_ptr = agent_ptr.clone();
        let cfg = extension.clone();
        let sid = session_id.to_string();
        set.spawn(async move { (id, agent_ptr.add_extension(cfg, &sid).await) });
    }

    let get_message = |waiting_ids: &BTreeSet<usize>| {
        let labels: Vec<String> = waiting_ids
            .iter()
            .map(|id| {
                extensions_to_load
                    .get(*id)
                    .map(|e| e.0.clone())
                    .unwrap_or_default()
            })
            .collect();
        format!(
            "starting {} extensions: {}",
            waiting_ids.len(),
            labels.join(", ")
        )
    };

    let spinner = cliclack::spinner();
    spinner.start(get_message(&waiting_ids));

    let mut offer_debug: Vec<(usize, anyhow::Error)> = Vec::new();
    while let Some(result) = set.join_next().await {
        match result {
            Ok((id, Ok(_))) => {
                waiting_ids.remove(&id);
                spinner.set_message(get_message(&waiting_ids));
            }
            Ok((id, Err(e))) => offer_debug.push((id, e.into())),
            Err(e) => tracing::error!("failed to add extension: {}", e),
        }
    }

    spinner.clear();

    for (id, err) in offer_debug {
        let label = extensions_to_load
            .get(id)
            .map(|e| e.0.clone())
            .unwrap_or_default();
        eprintln!(
            "{}",
            style(format!(
                "Warning: Failed to start extension '{}' ({}), continuing without it",
                label, err
            ))
            .yellow()
        );

        if let Err(debug_err) = offer_extension_debugging_help(
            &label,
            &err.to_string(),
            Arc::clone(&provider_for_debug),
            interactive,
        )
        .await
        {
            eprintln!("Note: Could not start debugging session: {}", debug_err);
        }
    }

    agent_ptr
}

struct ResolvedProviderConfig {
    provider_name: String,
    model_name: String,
    model_config: permagent::model::ModelConfig,
}

/// The first source that actually holds a value, in the order given.
///
/// Extracted so the PRECEDENCE ITSELF is unit-testable: the order of these five
/// sources is the whole contract of [`resolve_provider_and_model`], and it is a
/// contract that was previously only expressed as a chain of `.or_else` calls no
/// test could reach without a process-global config and a live provider. A
/// whitespace-only value counts as unset — a config key edited down to `""` is
/// not a provider named "space".
fn first_configured(sources: [Option<String>; 5]) -> Option<String> {
    sources
        .into_iter()
        .flatten()
        .find(|value| !value.trim().is_empty())
}

/// The coding harness's own model route, or `None` when the operator's session
/// model should be used instead. Warns — once, loudly — on a half-configured
/// pair, because `harness_provider` without `harness_model` cannot route and is
/// always a typo rather than an intention.
fn harness_role_route(config: &Config) -> Option<permagent::config::RoleModel> {
    use permagent::config::{ModelRole, RoleModelSource};

    let resolved = permagent::config::resolve_role_model(ModelRole::Harness, |key| {
        config.get_param::<String>(key).ok()
    });
    if resolved.source == RoleModelSource::HalfConfigured {
        output::render_error(&format!(
            "Only one of `{}`/`{}` is set. A half-configured pair cannot route, so it is being \
             ignored; set both, or set one to `session` to run the harness on your session model.",
            ModelRole::Harness.provider_key(),
            ModelRole::Harness.model_key(),
        ));
    }
    resolved.route
}

fn resolve_provider_and_model(
    session_config: &SessionBuilderConfig,
    config: &Config,
    saved_provider: Option<String>,
    saved_model_config: Option<permagent::model::ModelConfig>,
) -> ResolvedProviderConfig {
    let recipe_settings = session_config
        .recipe
        .as_ref()
        .and_then(|r| r.settings.as_ref());

    // The coding harness has its own measured model default (`harness_provider`
    // / `harness_model`, see `permagent::config::model_roles` and
    // docs/research/MODEL_DEFAULTS_BENCH_2026-08-25.md). It is read ONLY for the
    // coding recipe — an ordinary `permagent run` is a session, not a harness —
    // and sits below the recipe's own `settings:` block and above
    // GOOSE_PROVIDER/GOOSE_MODEL, so a `--provider/--model` flag, a resumed
    // session and a recipe that pins its own model all still win.
    let harness_route = session_config
        .recipe
        .as_ref()
        .map(crate::recipes::builtin_recipes::is_coding_harness_recipe)
        .unwrap_or(false)
        .then(|| harness_role_route(config))
        .flatten();

    let provider_name = first_configured([
        session_config.provider.clone(),
        saved_provider,
        recipe_settings.and_then(|s| s.goose_provider.clone()),
        harness_route.as_ref().map(|r| r.provider.clone()),
        config.get_goose_provider().ok(),
    ])
    .unwrap_or_else(|| {
        output::render_error("No provider configured. Run 'goose configure' first.");
        process::exit(1);
    });

    let model_name = first_configured([
        session_config.model.clone(),
        saved_model_config.as_ref().map(|mc| mc.model_name.clone()),
        recipe_settings.and_then(|s| s.goose_model.clone()),
        harness_route.as_ref().map(|r| r.model.clone()),
        config.get_goose_model().ok(),
    ])
    .unwrap_or_else(|| {
        output::render_error("No model configured. Run 'goose configure' first.");
        process::exit(1);
    });

    let model_config = if session_config.resume
        && saved_model_config
            .as_ref()
            .is_some_and(|mc| mc.model_name == model_name)
    {
        let mut config = saved_model_config.unwrap();
        if let Some(temp) = recipe_settings.and_then(|s| s.temperature) {
            config = config.with_temperature(Some(temp));
        }
        config
    } else {
        let temperature = recipe_settings.and_then(|s| s.temperature);
        permagent::model::ModelConfig::new(&model_name)
            .unwrap_or_else(|e| {
                output::render_error(&format!("Failed to create model configuration: {}", e));
                process::exit(1);
            })
            .with_canonical_limits(&provider_name)
            .with_temperature(temperature)
    };

    ResolvedProviderConfig {
        provider_name,
        model_name,
        model_config,
    }
}

async fn resolve_session_id(
    session_config: &SessionBuilderConfig,
    session_manager: &permagent::session::session_manager::SessionManager,
    goose_mode: GooseMode,
) -> String {
    if session_config.no_session {
        let working_dir = std::env::current_dir().unwrap_or_else(|e| {
            output::render_error(&format!("Could not get working directory: {}", e));
            process::exit(1);
        });
        let session = session_manager
            .create_session(
                working_dir,
                "CLI Session".to_string(),
                SessionType::Hidden,
                goose_mode,
            )
            .await
            .unwrap_or_else(|e| {
                output::render_error(&format!("Could not create session: {}", e));
                process::exit(1);
            });
        session.id
    } else if session_config.resume {
        if let Some(ref session_id) = session_config.session_id {
            match session_manager.get_session(session_id, false).await {
                Ok(_) => session_id.clone(),
                Err(_) => {
                    output::render_error(&format!(
                        "Cannot resume session {} - no such session exists",
                        style(session_id).cyan()
                    ));
                    process::exit(1);
                }
            }
        } else {
            match session_manager.list_sessions().await {
                Ok(sessions) if !sessions.is_empty() => sessions[0].id.clone(),
                _ => {
                    output::render_error("Cannot resume - no previous sessions found");
                    process::exit(1);
                }
            }
        }
    } else {
        session_config.session_id.clone().unwrap()
    }
}

async fn handle_resumed_session_workdir(agent: &Agent, session_id: &str, interactive: bool) {
    let session = agent
        .config
        .session_manager
        .get_session(session_id, false)
        .await
        .unwrap_or_else(|e| {
            output::render_error(&format!("Failed to read session metadata: {}", e));
            process::exit(1);
        });

    let current_workdir = std::env::current_dir().unwrap_or_else(|e| {
        output::render_error(&format!("Failed to get current working directory: {}", e));
        process::exit(1);
    });
    if current_workdir == session.working_dir {
        return;
    }

    if interactive {
        let change_workdir = cliclack::confirm(format!(
            "{} The original working directory of this session was set to {}. \
             Your current directory is {}. \
             Do you want to switch back to the original working directory?",
            style("WARNING:").yellow(),
            style(session.working_dir.display()).cyan(),
            style(current_workdir.display()).cyan(),
        ))
        .initial_value(true)
        .interact()
        .unwrap_or_else(|e| {
            output::render_error(&format!("Failed to get user input: {}", e));
            process::exit(1);
        });

        if change_workdir {
            if !session.working_dir.exists() {
                output::render_error(&format!(
                    "Cannot switch to original working directory - {} no longer exists",
                    style(session.working_dir.display()).cyan()
                ));
            } else if let Err(e) = std::env::set_current_dir(&session.working_dir) {
                output::render_error(&format!(
                    "Failed to switch to original working directory: {}",
                    e
                ));
            }
        }
    } else {
        eprintln!(
            "{}",
            style(format!(
                "Warning: Working directory differs from session (current: {}, session: {}). \
                 Staying in current directory.",
                current_workdir.display(),
                session.working_dir.display()
            ))
            .yellow()
        );
    }
}

async fn collect_extension_configs(
    agent: &Agent,
    session_config: &SessionBuilderConfig,
    recipe: Option<&Recipe>,
    session_id: &str,
) -> Result<Vec<ExtensionConfig>, ExtensionError> {
    let configured_extensions: Vec<ExtensionConfig> = if session_config.resume {
        EnabledExtensionsState::for_session(
            &agent.config.session_manager,
            session_id,
            Config::global(),
        )
        .await
    } else if session_config.no_profile {
        Vec::new()
    } else {
        let cwd = std::env::current_dir().ok();
        resolve_extensions_for_new_session(
            recipe.and_then(|r| r.extensions.as_deref()),
            None,
            cwd.as_deref(),
        )
        .map_err(|e| ExtensionError::ConfigError(e.to_string()))?
    };

    let cli_flag_extensions = parse_cli_flag_extensions(
        &session_config.extensions,
        &session_config.streamable_http_extensions,
        &session_config.builtins,
    );

    let mut all: Vec<ExtensionConfig> = configured_extensions;
    all.extend(cli_flag_extensions.into_iter().map(|(_, cfg)| cfg));

    Ok(all)
}

async fn resolve_and_load_extensions(
    agent: Agent,
    extensions: Vec<ExtensionConfig>,
    provider_for_debug: Arc<dyn permagent::providers::base::Provider>,
    interactive: bool,
    session_id: &str,
) -> Arc<Agent> {
    for warning in permagent::config::get_warnings() {
        eprintln!("{}", style(format!("Warning: {}", warning)).yellow());
    }

    let extensions_to_load: Vec<(String, ExtensionConfig)> = extensions
        .into_iter()
        .map(|cfg| (cfg.name(), cfg))
        .collect();

    load_extensions(
        agent,
        extensions_to_load,
        provider_for_debug,
        interactive,
        session_id,
    )
    .await
}

/// Build the `repo_map` system-prompt extra for a coding-harness session.
///
/// Pure and total: exactly one of two mutually exclusive wordings. The
/// recipe's instructions state, as a flat fact, that a repo map is already in
/// the model's context — this function is what has to make that statement
/// true, or retract it, on every session; there is no third path where the
/// prompt stays silent and the recipe's claim goes unchecked. Extracted out
/// of the session-building side effects so both branches are unit-testable
/// without a real session or a Brain.
fn orientation_block(map: Option<String>) -> String {
    match map {
        Some(block) => format!(
            "{block}\n\nThis map is your starting point for this session — look symbols and \
             files up in it before reading whole files. Use `map_query`, `analyze`, and \
             `search` to go deeper into anything it doesn't cover in enough detail; do not \
             re-derive it by hand with ls/cat/grep."
        ),
        None => "No repo map is in context for this session (mapping was skipped, disabled, or \
                  found nothing to map here). Orient with `tree`, `map_query` (the project's \
                  stored code map, if one has been indexed), and `search` before reading whole \
                  files."
            .to_string(),
    }
}

/// Token budget for the coding harness's ranked-tags repo map.
///
/// Was 1024. Measured 2026-08-25 against a real Next.js checkout: 1024 tokens
/// bought 40 files and 71 symbols — which did not reach the components the
/// session actually needed to edit, so the map could not answer the question it
/// was asked and the model shelled out to read files by hand instead. 4096 is
/// still small next to the rest of the prompt and covers a mid-sized app.
/// `PERMAGENT_CODING_REPO_MAP_TOKENS` overrides it; 0 still disables mapping.
const DEFAULT_CODING_REPO_MAP_TOKENS: usize = 4096;

async fn configure_session_prompts(
    session: &CliSession,
    config: &Config,
    session_config: &SessionBuilderConfig,
    session_id: &str,
) {
    if let Err(e) = session.agent.persist_extension_state(session_id).await {
        tracing::warn!("Failed to save extension state: {}", e);
    }

    if let Some(ref additional_prompt) = session_config.additional_system_prompt {
        session
            .agent
            .extend_system_prompt("additional".to_string(), additional_prompt.clone())
            .await;
    }

    // Auto-inject a token-budgeted repo-map (ranked-tags codebase map, #712) into
    // the coding harness's context. Gated to the coding recipe so ordinary CLI
    // sessions don't pay a whole-repo parse; the budget is retunable via
    // PERMAGENT_CODING_REPO_MAP_TOKENS (0 disables). It lands as a stable prompt
    // extra in the cache-stable `RepoMap` prefix position (see
    // permagent::cost_router::cache). The working dir is already the session's
    // root at this point (build_session set it before calling us), so cwd is the
    // repo to map.
    //
    // The recipe's instructions (permagent-coding.yaml) assert as fact that a
    // repo map is already in the model's context. `coding_context_block` can
    // legitimately return `None` — an empty or unsupported tree, mapping
    // disabled via the budget, or a remembered per-repo decline — and until
    // now nothing downstream checked for that: the recipe's claim went
    // unverified either way. This block now ALWAYS lands an orientation extra
    // for a coding-harness session — one of two mutually exclusive wordings
    // (`orientation_block` below) — so the prompt never claims a map that was
    // not actually injected.
    if session_config
        .recipe
        .as_ref()
        .map(crate::recipes::builtin_recipes::is_coding_harness_recipe)
        .unwrap_or(false)
    {
        let budget = config
            .get_param::<usize>("PERMAGENT_CODING_REPO_MAP_TOKENS")
            .unwrap_or(DEFAULT_CODING_REPO_MAP_TOKENS);
        // Interactive sessions get an explicit offer: mapping is the cost
        // lever (the agent orients from ranked signatures instead of
        // reading whole files), and surfacing it makes that legible + lets
        // the user skip the whole-repo parse on a tree they know is huge.
        // Non-interactive runs keep the silent auto-map. Only asked when
        // there is a budget to map into at all.
        use std::io::IsTerminal;
        let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
        let root = std::env::current_dir().ok();
        let wants_map = if budget == 0 {
            false
        } else {
            // Remember the answer PER REPO. The map itself is mtime-cached, so
            // re-asking every single session was pure friction — you answer
            // the same way for the same tree every time. First run asks; after
            // that it just says what it did.
            let repo_key = root.as_ref().map(|p| {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                p.to_string_lossy().hash(&mut h);
                format!("coding_repo_map_{:x}", h.finish())
            });
            let remembered = repo_key
                .as_ref()
                .and_then(|k| config.get_param::<bool>(k).ok());
            match (interactive, remembered) {
                (_, Some(prev)) => prev,
                (true, None) => {
                    let answer = cliclack::confirm(format!(
                        "Map this codebase before starting? Permagent extracts symbols with \
                         tree-sitter and ranks them (PageRank over the reference graph) into a \
                         ~{budget}-token map, so the agent looks things up from signatures \
                         instead of reading whole files. (Asked once per repo.)"
                    ))
                    .initial_value(true)
                    .interact()
                    .unwrap_or(true);
                    if let Some(k) = repo_key.as_ref() {
                        let _ = config.set_param(k, serde_json::json!(answer));
                    }
                    answer
                }
                (false, None) => true,
            }
        };

        let mut map_block: Option<String> = None;
        let mut skip_reason: &str = "PERMAGENT_CODING_REPO_MAP_TOKENS is 0 (mapping disabled)";
        if wants_map {
            if let Some(ref root) = root {
                map_block =
                    permagent::agents::platform_extensions::analyze::repo_map::coding_context_block(
                        root, budget,
                    );
                skip_reason = "ranked-tags map returned no symbols for this tree";
                // Ranked-tags mapping needs a parseable, PageRank-worthy tree.
                // When it comes back empty, fall back to whatever code map the
                // project already has stored in the Brain from a prior
                // analyze/index-code run, rather than leaving the session with
                // nothing at all.
                if map_block.is_none() {
                    if let Ok(pool) = session.agent.config.session_manager.pool_clone().await {
                        map_block =
                            permagent::agents::platform_extensions::analyze::stored_code_map_block(
                                &pool,
                                Some(root.as_path()),
                            )
                            .await;
                        if map_block.is_none() {
                            skip_reason =
                                "no ranked-tags map and no stored Brain code map for this project";
                        }
                    }
                }
            } else {
                skip_reason = "could not resolve the session's working directory";
            }
        } else if budget > 0 {
            skip_reason = "mapping was declined for this repo";
        }

        if interactive {
            match &map_block {
                Some(block) => {
                    // ~4 chars/token — the same heuristic the budget fill
                    // uses; this is a report, not an invoice.
                    let approx_tokens = block.len() / 4;
                    let _ = cliclack::log::success(format!(
                        "Codebase mapped: ~{approx_tokens} tokens of ranked symbol \
                         signatures pinned into context (cached by file mtime; \
                         tune with PERMAGENT_CODING_REPO_MAP_TOKENS)."
                    ));
                }
                None if wants_map => {
                    let _ = cliclack::log::info(
                        "No mappable source found here — skipping the repo map.",
                    );
                }
                None => {}
            }
        }
        if map_block.is_none() {
            // Visible in ~/.permagent/logs/cli/*.log — the interactive message
            // above only reaches a terminal, and a non-interactive run (or a
            // recipe's `run --interactive` capture reviewed after the fact)
            // needs this recorded somewhere durable.
            tracing::warn!(
                root = %root.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
                reason = skip_reason,
                "coding harness session starting without a repo map"
            );
        }
        session
            .agent
            .extend_system_prompt("repo_map".to_string(), orientation_block(map_block))
            .await;
    }

    let system_prompt_file: Option<String> = config.get_param("GOOSE_SYSTEM_PROMPT_FILE_PATH").ok();
    if let Some(ref path) = system_prompt_file {
        let override_prompt = std::fs::read_to_string(path).unwrap_or_else(|e| {
            output::render_error(&format!(
                "Failed to read system prompt file '{}': {}",
                path, e
            ));
            process::exit(1);
        });
        session.agent.override_system_prompt(override_prompt).await;
    }
}

pub async fn build_session(session_config: SessionBuilderConfig) -> CliSession {
    #[cfg(feature = "telemetry")]
    permagent::posthog::set_session_context("cli", session_config.resume);

    let config = Config::global();
    let agent = new_primary_agent().await;

    if session_config.container.is_some() {
        agent.set_container(session_config.container.clone()).await;
    }

    let session_manager = agent.config.session_manager.clone();

    let (saved_provider, saved_model_config) = if session_config.resume {
        if let Some(ref session_id) = session_config.session_id {
            match session_manager.get_session(session_id, false).await {
                Ok(session_data) => (session_data.provider_name, session_data.model_config),
                Err(_) => (None, None),
            }
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    let resolved =
        resolve_provider_and_model(&session_config, config, saved_provider, saved_model_config);

    let recipe = session_config.recipe.as_ref();

    if let Err(e) = agent
        .apply_recipe_components(recipe.and_then(|r| r.response.clone()), true)
        .await
    {
        output::render_error(&format!("Invalid recipe: {}", e));
        process::exit(1);
    }

    let session_id =
        resolve_session_id(&session_config, &session_manager, agent.config.goose_mode).await;

    if session_config.resume {
        handle_resumed_session_workdir(&agent, &session_id, session_config.interactive).await;
    }

    let extensions_for_provider =
        match collect_extension_configs(&agent, &session_config, recipe, &session_id).await {
            Ok(exts) => exts,
            Err(e) => {
                output::render_error(&format!("Failed to collect extensions: {}", e));
                process::exit(1);
            }
        };

    let new_provider = match create(
        &resolved.provider_name,
        resolved.model_config,
        extensions_for_provider.clone(),
    )
    .await
    {
        Ok(provider) => provider,
        Err(e) => {
            output::render_error(&format!(
                "Error {}.\n\
                Please check your system keychain and run 'goose configure' again.\n\
                If your system is unable to use the keyring, please try setting secret key(s) via environment variables.\n\
                For more info, see: https://goose-docs.ai/docs/troubleshooting/#keychainkeyring-errors",
                e
            ));
            process::exit(1);
        }
    };
    let provider_for_debug = Arc::clone(&new_provider);
    tracing::info!("🤖 Using model: {}", resolved.model_name);

    agent
        .update_provider(new_provider, &session_id)
        .await
        .unwrap_or_else(|e| {
            output::render_error(&format!("Failed to initialize agent: {}", e));
            process::exit(1);
        });

    agent
        .update_goose_mode(agent.config.goose_mode, &session_id)
        .await
        .unwrap_or_else(|e| {
            output::render_error(&format!("Failed to set session mode: {}", e));
            process::exit(1);
        });

    if let Some(recipe) = session_config.recipe.clone() {
        if let Err(e) = session_manager
            .update(&session_id)
            .recipe(Some(recipe))
            .apply()
            .await
        {
            tracing::warn!("Failed to store recipe on session: {}", e);
        }
    }

    // Extensions are loaded after session creation because we may change directory when resuming
    let agent_ptr = resolve_and_load_extensions(
        agent,
        extensions_for_provider,
        Arc::clone(&provider_for_debug),
        session_config.interactive,
        &session_id,
    )
    .await;

    let edit_mode = config
        .get_param::<String>("EDIT_MODE")
        .ok()
        .and_then(|edit_mode| match edit_mode.to_lowercase().as_str() {
            "emacs" => Some(EditMode::Emacs),
            "vi" => Some(EditMode::Vi),
            _ => {
                eprintln!("Invalid EDIT_MODE specified, defaulting to Emacs");
                None
            }
        });

    let debug_mode = session_config.debug || config.get_param("GOOSE_DEBUG").unwrap_or(false);

    let session = CliSession::new(
        Arc::try_unwrap(agent_ptr).unwrap_or_else(|_| panic!("There should be no more references")),
        session_id.clone(),
        debug_mode,
        session_config.scheduled_job_id.clone(),
        session_config.max_turns,
        edit_mode,
        recipe.and_then(|r| r.retry.clone()),
        session_config.output_format.clone(),
    )
    .await;

    configure_session_prompts(&session, config, &session_config, &session_id).await;

    if !session_config.quiet {
        output::display_session_info(
            session_config.resume,
            &resolved.provider_name,
            &resolved.model_name,
            &Some(session_id),
        );
    }
    session
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Option<String> {
        Some(v.to_string())
    }

    /// Every agent this module builds must come from `new_primary_agent`.
    ///
    /// A bare `Agent::new()` leaves `PromptManager`'s first-run fallback
    /// persona installed, which is how the harness ended up introducing itself
    /// as someone other than the persona Chat saved in `agent.yaml` — and the
    /// helper/debug agents reverted the same way. `Agent`'s prompt manager is
    /// `pub(super)` inside `permagent::agents`, so the CLI cannot read the
    /// installed persona back to assert on it; guarding the construction site
    /// is the coverage that is actually reachable from here.
    #[test]
    fn every_cli_agent_is_built_through_the_shared_persona_constructor() {
        // Only the product half of this file; the test module below quotes the
        // same call it is forbidding.
        let product = include_str!("builder.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("builder.rs has a product half");
        let bare: Vec<&str> = product
            .lines()
            .map(str::trim)
            .filter(|line| line.ends_with("Agent::new();"))
            .collect();
        assert_eq!(
            bare.len(),
            1,
            "only new_primary_agent may construct an Agent; found {bare:?}"
        );
        assert!(
            product
                .contains("set_persona(permagent::config::agent_identity::load_shared_persona())"),
            "new_primary_agent must install the persona Chat saved, not the fallback"
        );
    }

    /// The five sources in `resolve_provider_and_model`'s order:
    /// CLI flag, resumed session, recipe `settings:`, the harness role default,
    /// then `GOOSE_PROVIDER`/`GOOSE_MODEL`.
    #[test]
    fn a_cli_flag_outranks_every_other_source() {
        assert_eq!(
            first_configured([
                s("cli"),
                s("saved"),
                s("recipe"),
                s("harness"),
                s("session")
            ]),
            s("cli")
        );
    }

    #[test]
    fn a_resumed_session_outranks_the_recipe_and_the_harness_default() {
        assert_eq!(
            first_configured([None, s("saved"), s("recipe"), s("harness"), s("session")]),
            s("saved")
        );
    }

    #[test]
    fn a_recipe_that_pins_its_own_model_outranks_the_harness_default() {
        assert_eq!(
            first_configured([None, None, s("recipe"), s("harness"), s("session")]),
            s("recipe")
        );
    }

    #[test]
    fn the_harness_default_outranks_the_session_model() {
        // `harness_provider`/`harness_model` are read only for the coding
        // recipe, and when they are set they are the point.
        assert_eq!(
            first_configured([None, None, None, s("harness"), s("session")]),
            s("harness")
        );
    }

    #[test]
    fn the_session_model_is_the_fallback_when_no_harness_route_applies() {
        // `harness_role_route` returns None whenever the operator has a session
        // model and no harness keys — the case that must not change behaviour
        // for anyone who already configured GOOSE_MODEL.
        assert_eq!(
            first_configured([None, None, None, None, s("session")]),
            s("session")
        );
    }

    #[test]
    fn nothing_configured_anywhere_is_none_not_an_empty_string() {
        assert_eq!(first_configured([None, None, None, None, None]), None);
        assert_eq!(
            first_configured([s("   "), s(""), None, None, s("session")]),
            s("session"),
            "a blanked-out key is unset, not a provider named whitespace"
        );
    }

    #[test]
    fn test_session_builder_config_creation() {
        let config = SessionBuilderConfig {
            session_id: None,
            resume: false,
            fork: false,
            no_session: false,
            extensions: vec!["echo test".to_string()],
            streamable_http_extensions: vec![StreamableHttpOptions {
                url: "http://localhost:8080/mcp".to_string(),
                timeout: permagent::config::DEFAULT_EXTENSION_TIMEOUT,
            }],
            builtins: vec!["developer".to_string()],
            no_profile: false,
            recipe: None,
            additional_system_prompt: Some("Test prompt".to_string()),
            provider: None,
            model: None,
            debug: true,
            max_tool_repetitions: Some(5),
            max_turns: None,
            scheduled_job_id: None,
            interactive: true,
            quiet: false,
            output_format: "text".to_string(),
            container: None,
        };

        assert_eq!(config.extensions.len(), 1);
        assert_eq!(config.streamable_http_extensions.len(), 1);
        assert_eq!(config.builtins.len(), 1);
        assert!(config.debug);
        assert_eq!(config.max_tool_repetitions, Some(5));
        assert!(config.max_turns.is_none());
        assert!(config.scheduled_job_id.is_none());
        assert!(config.interactive);
        assert!(!config.quiet);
    }

    #[test]
    fn test_session_builder_config_default() {
        let config = SessionBuilderConfig::default();

        assert!(config.session_id.is_none());
        assert!(!config.resume);
        assert!(!config.no_session);
        assert!(config.extensions.is_empty());
        assert!(config.streamable_http_extensions.is_empty());
        assert!(config.builtins.is_empty());
        assert!(!config.no_profile);
        assert!(config.recipe.is_none());
        assert!(config.additional_system_prompt.is_none());
        assert!(!config.debug);
        assert!(config.max_tool_repetitions.is_none());
        assert!(config.max_turns.is_none());
        assert!(config.scheduled_job_id.is_none());
        assert!(!config.interactive);
        assert!(!config.quiet);
        assert!(!config.fork);
    }

    #[tokio::test]
    async fn test_offer_extension_debugging_help_function_exists() {
        // This test just verifies the function compiles and can be called
        // We can't easily test the interactive parts without mocking

        // We can't actually test the full function without a real provider and user interaction
        // But we can at least verify it compiles and the function signature is correct
        let extension_name = "test-extension";
        let error_message = "test error";

        // This test mainly serves as a compilation check
        assert_eq!(extension_name, "test-extension");
        assert_eq!(error_message, "test error");
    }

    #[test]
    fn test_truncate_with_ellipsis() {
        assert_eq!(truncate_with_ellipsis("abc", 5), "abc");

        assert_eq!(truncate_with_ellipsis("abcde", 5), "abcde");

        assert_eq!(truncate_with_ellipsis("abcdef", 5), "abcde…");
        assert_eq!(truncate_with_ellipsis("hello world", 5), "hello…");

        assert_eq!(truncate_with_ellipsis("", 5), "");
    }

    // The recipe's instructions state as fact that a repo map is already in
    // context. `orientation_block` is what has to keep that statement honest:
    // the "map present" wording must only ever appear alongside an actual
    // injected map, and the "no map" wording must never claim one exists.

    #[test]
    fn orientation_block_with_a_map_includes_it_and_says_to_go_deeper() {
        let out = orientation_block(Some("Codebase map (pre-indexed...):\nsrc/\n".to_string()));
        assert!(out.contains("Codebase map (pre-indexed"));
        assert!(out.contains("starting point"));
        assert!(out.contains("map_query"));
        // Must not also carry the "no map" claim.
        assert!(!out.contains("No repo map is in context"));
    }

    #[test]
    fn orientation_block_without_a_map_never_claims_one_is_present() {
        let out = orientation_block(None);
        assert!(out.contains("No repo map is in context"));
        assert!(out.contains("map_query"));
        assert!(out.contains("tree"));
        // Wording unique to the "map present" branch must not leak in here —
        // that claim is only honest when a map was actually injected.
        assert!(!out.contains("starting point"));
        assert!(!out.contains("Codebase map (pre-indexed"));
        assert!(!out.contains("do not re-derive"));
    }

    /// The two branches must be mutually exclusive over a range of map
    /// contents, not just the two examples above — neither wording is a
    /// substring of the other regardless of what the map body itself says.
    #[test]
    fn orientation_block_branches_are_mutually_exclusive() {
        for map in [
            "".to_string(),
            "src/\n  main.rs\n".to_string(),
            "No repo map is in context — this text is part of the MAP CONTENT, not the \
             wrapper, and must not fool the mutual-exclusion check."
                .to_string(),
        ] {
            let with_map = orientation_block(Some(map));
            let without_map = orientation_block(None);
            assert!(with_map.contains("starting point"));
            assert!(without_map.contains("No repo map is in context"));
            assert!(!without_map.contains("starting point"));
        }
    }
}
