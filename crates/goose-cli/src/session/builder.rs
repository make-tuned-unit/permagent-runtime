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
        false,
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

/// Lane R18f: extensions that failed to register, by label (`cfg.name()`).
/// Previously `load_extensions` only ever printed a warning and pressed on —
/// fine for an interactive session where the operator can see the banner and
/// decide, fatal-by-omission for a headless coding-harness run whose
/// `orchestrator` extension is the ONLY route to `decompose_roadmap` (see
/// `build_session`'s `headless_coding_dispatch` block).
async fn load_extensions(
    agent: Agent,
    extensions_to_load: Vec<(String, ExtensionConfig)>,
    provider_for_debug: Arc<dyn permagent::providers::base::Provider>,
    interactive: bool,
    session_id: &str,
) -> (Arc<Agent>, Vec<String>) {
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

    let failed_labels: Vec<String> = offer_debug
        .iter()
        .map(|(id, _)| {
            extensions_to_load
                .get(*id)
                .map(|e| e.0.clone())
                .unwrap_or_default()
        })
        .collect();

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

    (agent_ptr, failed_labels)
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

/// Whether `build_session` should install the headless goal-dispatch hook
/// (harness loop, Lane S / M3-1).
///
/// `nudge_dispatch_ready_goals` (`orchestrator.rs`) only ever starts a root
/// goal's worker through the process-global `GOAL_DISPATCH_HOOK` — the daemon
/// installs it at startup (`goose-server/src/commands/agent.rs`), but headless
/// `permagent run` never did, so a coding-harness session's `create_roadmap`
/// always reported "0 root goal(s) dispatched to workers" and the recipe's
/// NON-NEGOTIABLE protocol (coordinator presents/creates the roadmap; a
/// dispatched DAG worker does the actual work) could never complete —
/// confirmed live in 8/10 of Lane M3's measured fixtures.
///
/// Scoped to exactly the coding harness recipe running under `GooseMode::Auto`
/// — the one launch shape whose NON-NEGOTIABLE protocol requires dispatch to
/// happen with no human in the loop. An ordinary chat/run session (even one
/// that happens to load the `orchestrator` extension) must not silently start
/// spawning real worker subagents the operator never asked to auto-dispatch.
fn wants_headless_goal_dispatch(coding_harness: bool, goose_mode: GooseMode) -> bool {
    coding_harness && goose_mode == GooseMode::Auto
}

/// Lane R18f: does `tools` expose a tool named `base_name`, prefixed or not?
/// Same base-name match `permagent::after_turn`'s guards use for tool-request
/// matching (strip everything up to and including the last `__`), so "is
/// `decompose_roadmap` resolvable" reads identically here and in the guard
/// that watches for its absence downstream.
fn tool_list_exposes(tools: &[rmcp::model::Tool], base_name: &str) -> bool {
    tools
        .iter()
        .any(|tool| tool.name.rsplit("__").next().unwrap_or(tool.name.as_ref()) == base_name)
}

/// Lane R18f: pure retry core of `build_session`'s orchestrator-tool-registration
/// wait. `probe` is called up to `max_attempts` times, sleeping `retry_delay`
/// between misses, and returns as soon as one call answers `true`. Extracted
/// so a test can simulate "found on the Nth attempt" or "never found" with a
/// synthetic counter instead of a live `Agent`/session — see
/// `wait_for_tool_*` in this module's test suite.
async fn wait_for_tool<F, Fut>(
    max_attempts: u32,
    retry_delay: std::time::Duration,
    mut probe: F,
) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for attempt in 0..max_attempts {
        if probe().await {
            return true;
        }
        if attempt + 1 < max_attempts {
            tokio::time::sleep(retry_delay).await;
        }
    }
    false
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

/// A complete `harness_provider`/`harness_model` pair is an operator choice,
/// just like a CLI `--provider/--model` pair.  If that choice is not yet
/// graduated, preserve it and emit the spend warning instead of silently
/// replacing it with a paid fallback.
fn harness_role_selection_is_explicit(config: &Config) -> bool {
    use permagent::config::{ModelRole, RoleModelSource};

    let source = permagent::config::resolve_role_model(ModelRole::Harness, |key| {
        config.get_param::<String>(key).ok()
    })
    .source;
    source == RoleModelSource::Configured
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

/// Keep an ungraduated model out of the coding harness's top-level
/// coordinator seat.  The operator may still select the same local model for
/// bounded mechanical workers; this gate only protects the judgment-dense
/// session that must create and advance the coding DAG.
///
/// The default route is trusted explicitly; the knowledge table only supplies
/// a heuristic status for other models until a coding-suite graduation record
/// exists. Unknown models therefore fail closed instead of being treated as
/// capable because they happen to answer a provider request.
fn enforce_harness_coordinator_route(
    resolved: ResolvedProviderConfig,
    coding_harness: bool,
    configured_fallback: Option<permagent::config::RoleModel>,
    explicit_override: bool,
    interactive: bool,
) -> (ResolvedProviderConfig, Option<String>) {
    if !coding_harness {
        return (resolved, None);
    }

    // The decision itself lives in `permagent::config::gate_harness_coordinator`
    // so the daemon's Settings projection (`GET /config` resolved_routes.harness)
    // shows exactly the coordinator this launch will run. Only the wording and
    // the model-config plumbing live here.
    use permagent::config::HarnessGateOutcome;
    let decision = permagent::config::gate_harness_coordinator(
        permagent::config::RoleModel::new(&resolved.provider_name, &resolved.model_name),
        configured_fallback,
        explicit_override,
    );
    let eligibility = decision.eligibility;
    let target = match decision.outcome {
        HarnessGateOutcome::Kept => return (resolved, None),
        HarnessGateOutcome::KeptExplicit => {
            let notice = format!(
                "Coding harness coordinator {}/{} is not trusted ({}), but the model was explicitly selected; automatic fallback is refused to avoid unexpected provider spend. The selected model remains in control, and the cost meter records any work it performs.",
                resolved.provider_name,
                resolved.model_name,
                eligibility.reason(),
            );
            return (resolved, deliver_failover_notice(notice, interactive));
        }
        // This should be unreachable while the measured default remains in the
        // knowledge table. Keep the user's route if the table is deliberately
        // edited into an inconsistent state, but make the missing evidence
        // visible rather than claiming the coordinator is safe.
        HarnessGateOutcome::KeptNoFallback => {
            let notice = format!(
                "Coding harness coordinator {}/{} is not graduated ({}), and no measured fallback is available; keeping the configured route.",
                resolved.provider_name,
                resolved.model_name,
                eligibility.reason(),
            );
            return (resolved, deliver_failover_notice(notice, interactive));
        }
        HarnessGateOutcome::Rerouted => decision.route,
    };

    let model_config = permagent::model::ModelConfig::new(&target.model)
        .expect("a graduated harness fallback must have a valid model id")
        .with_canonical_limits(&target.provider)
        .with_temperature(resolved.model_config.temperature);
    let notice = format!(
        "Coding harness coordinator {}/{} is not graduated ({}); routed the top-level session to {}/{}. This fallback may incur provider spend and is recorded by the cost meter; local models remain eligible for bounded mechanical worker nodes.",
        resolved.provider_name,
        resolved.model_name,
        eligibility.reason(),
        target.provider,
        target.model,
    );
    tracing::warn!(
        target: "permagent::cost_router",
        from_provider = %resolved.provider_name,
        from_model = %resolved.model_name,
        to_provider = %target.provider,
        to_model = %target.model,
        reason = eligibility.reason(),
        "ungraduated model blocked from coding coordinator role"
    );
    (
        ResolvedProviderConfig {
            provider_name: target.provider,
            model_name: target.model,
            model_config,
        },
        deliver_failover_notice(notice, interactive),
    )
}

/// Pure: what a resolved-but-dead local endpoint becomes, and the sentence that
/// says so. `None` when nothing is configured to fall back to — the caller must
/// then stay put and report the dead endpoint rather than substitute a model
/// nobody chose.
///
/// The announcement is the SAME builder the mid-turn failover uses
/// (`cost_router::fallback::precommit_failover_reply`), so a switch reads
/// identically whether it happened at startup or three turns in.
fn startup_failover(
    resolved: &ResolvedProviderConfig,
    fallback: Option<&permagent::cost_router::role_map::RoleModel>,
) -> Option<(ResolvedProviderConfig, String)> {
    let target = fallback?;
    let mc = permagent::model::ModelConfig::new(&target.model)
        .ok()?
        .with_canonical_limits(&target.provider)
        .with_temperature(resolved.model_config.temperature);
    // An Ollama stand-in has no canonical row, so backfill the knowledge
    // row's window. Same rule as the mid-turn path
    // (`Agent::switch_to_permanent_failure_fallback`). It becomes the
    // request's `num_ctx`, which Ollama's OpenAI-compatible endpoint drops
    // (the 2026-09-19 run sent 32768 and Ollama served 4096); the provider
    // now sends such a request to `/api/chat`, where it is honoured, and
    // `fit_extensions_to_local_window` fits the session to it.
    let model_config = if mc.context_limit.is_none() {
        mc.with_context_limit(permagent::cost_router::fallback::local_context_limit(
            &target.provider,
            &target.model,
        ))
    } else {
        mc
    };
    let notice = permagent::cost_router::fallback::precommit_failover_reply(
        &resolved.provider_name,
        &permagent::providers::errors::ProviderError::NetworkError(String::new()),
        target,
    );
    Some((
        ResolvedProviderConfig {
            provider_name: target.provider.clone(),
            model_name: target.model.clone(),
            model_config,
        },
        notice,
    ))
}

/// Put a startup-failover notice where this session's reader will actually
/// find it, and return it when the caller still has to file it.
///
/// Interactive gets the terminal print and nothing else — the user is looking
/// at the terminal, and the footer will agree with it from the first turn.
/// Headless has neither: `render_notice` writes to a tty nobody reads, and a
/// scheduled recipe or Build-tab launch has no footer at all. So the sentence
/// goes to stderr (the harness run's captured output — never stdout, which
/// `--output-format json` owns) AND comes back for the session journal, which
/// is the only place a reader can find it afterwards.
fn deliver_failover_notice(notice: String, interactive: bool) -> Option<String> {
    if interactive {
        output::render_notice(&notice);
        None
    } else {
        eprintln!("note: {notice}");
        Some(notice)
    }
}

/// Probe a self-hosted endpoint BEFORE the first turn, and fail over now rather
/// than burning a turn discovering it.
///
/// Session 20260831_10 (2026-08-31): the coding harness resolved
/// `qwen38_split`/`qwen3.8-27b` from the harness role default — a source that
/// answers "is it configured", never "is it up" — printed that as the banner,
/// then spent a whole turn's latency finding the split's port closed before the
/// mid-turn failover moved it to `anthropic/claude-haiku-4-5`.
///
/// Only endpoints the user hosts themselves are probed, on exactly the gate
/// `/model` already uses (`super::provider_probe_url`, loopback or private LAN);
/// a cloud provider pays no round trip here.
///
/// Returns the announcement when the caller still owes it a home — see
/// [`deliver_failover_notice`].
async fn failover_if_endpoint_is_dead(
    resolved: ResolvedProviderConfig,
    interactive: bool,
) -> (ResolvedProviderConfig, Option<String>) {
    let Some(url) = super::provider_probe_url(&resolved.provider_name) else {
        return (resolved, None);
    };
    if super::endpoint_is_reachable(&url).await {
        return (resolved, None);
    }

    // Probe-aware selection, not the configured order alone. The split's port
    // being closed is exactly why we are here; the sync, unprobed selector
    // would skip the split as the failed provider, see Ollama as "unknown",
    // and land the session on the metered REVIEW model (zai/glm-5.2) while
    // Ollama sat up on :11434 — the user's "assign basic tasks to qwen or
    // MiniCPM" never reached a local model.
    let fallback = permagent::cost_router::fallback::permanent_failure_fallback_available(
        &resolved.provider_name,
    )
    .await
    .map(|choice| choice.target);
    match startup_failover(&resolved, fallback.as_ref()) {
        Some((switched, notice)) => {
            tracing::warn!(
                target: "permagent::cost_router",
                from_provider = %resolved.provider_name,
                to_provider = %switched.provider_name,
                to_model = %switched.model_name,
                // The window an Ollama stand-in's `num_ctx` is sized from. The
                // 2026-09-19 QA run could not confirm it: no line carried it.
                context_limit = ?switched.model_config.context_limit,
                probe_url = %url,
                "startup probe found the endpoint dead; switched before the first turn"
            );
            (switched, deliver_failover_notice(notice, interactive))
        }
        // Nothing to fall back to. Say what is wrong now, in a sentence that
        // names the port, instead of letting turn one die on a raw socket error.
        None => {
            output::render_error(&format!(
                "`{}` is {} No fallback model is configured, so this session will \
                 start on it anyway and the first turn will fail.",
                resolved.provider_name,
                super::unreachable_notice(&url)
            ));
            (resolved, None)
        }
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
        let inherited_parent = std::env::var(permagent::session::PARENT_SESSION_ID_ENV)
            .ok()
            .filter(|id| !id.trim().is_empty());
        let session = session_manager
            .create_session_with_parent(
                inherited_parent.as_deref(),
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
        // `SessionType::Hidden` is deliberately non-interactive
        // (`is_interactive()` in session_manager.rs gates approval/UX, not
        // budget accounting), so `Agent::reply` never mints a budget task for
        // it the way it does for User/Terminal sessions. A child with an
        // inherited parent already copied the parent's durable task id at
        // creation (`create_session` in session_manager.rs); a top-level
        // `--no-session` run has no parent, so without this it has no budget
        // task identity at all. ProgressMonitor then reads `spend_snapshot`,
        // gets `MissingTaskIdentity`, and refuses every tool call — this is
        // the same gate the scenario runner hit and fixed the same way
        // (scenario_runner.rs, e03e6d4).
        if inherited_parent.is_none() {
            session_manager
                .begin_budget_task(&session.id)
                .await
                .unwrap_or_else(|e| {
                    output::render_error(&format!(
                        "Could not start a budget task for this session: {}",
                        e
                    ));
                    process::exit(1);
                });
        }
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

/// The extensions this session will load, plus the keys of the ones someone
/// asked for by name — the recipe's own list and the CLI flags — which a
/// local-window trim (`fit_extensions_to_local_window`) must never drop.
async fn collect_extension_configs(
    agent: &Agent,
    session_config: &SessionBuilderConfig,
    recipe: Option<&Recipe>,
    session_id: &str,
) -> Result<(Vec<ExtensionConfig>, BTreeSet<String>), ExtensionError> {
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

    let mut explicit: BTreeSet<String> = recipe
        .and_then(|r| r.extensions.as_deref())
        .unwrap_or_default()
        .iter()
        .map(|cfg| permagent::config::name_to_key(&cfg.name()))
        .collect();
    explicit.extend(
        cli_flag_extensions
            .iter()
            .map(|(_, cfg)| permagent::config::name_to_key(&cfg.name())),
    );

    let mut all: Vec<ExtensionConfig> = configured_extensions;
    all.extend(cli_flag_extensions.into_iter().map(|(_, cfg)| cfg));

    Ok((all, explicit))
}

/// What a local-window trim keeps from the profile's default set: the file
/// and shell tools, about 1k tokens of schema. Everything a recipe or a CLI
/// flag named is kept as well.
const LOCAL_WINDOW_CORE_EXTENSIONS: &[&str] = &["developer"];

/// How much of a local window the system prompt and tool schemas may take:
/// three quarters. The last quarter has to carry the conversation and the
/// reply.
fn local_request_budget(window: usize) -> usize {
    window - window / 4
}

/// The system prompt and tool schemas a session sends on every turn, in o200k
/// tokens (the provider's own preflight counts the same way).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LocalRequestSize {
    system: usize,
    tools: usize,
}

impl LocalRequestSize {
    fn total(&self) -> usize {
        self.system.saturating_add(self.tools)
    }
}

/// The loaded extensions a trim removes: everything not in `keep` (keys as
/// `name_to_key` spells them).
fn extensions_to_shed(loaded: &[String], keep: &BTreeSet<String>) -> Vec<String> {
    loaded
        .iter()
        .filter(|name| !keep.contains(&permagent::config::name_to_key(name)))
        .cloned()
        .collect()
}

/// Measure what this session will send on its first turn: the tool schemas
/// as loaded, and the system prompt rendered the way the agent renders it for
/// a CLI session (these extensions' instructions, the inventory scoped to
/// them, project hints, saved skills, the model family's overlay).
///
/// A lower bound, not the exact prompt: the agent adds the live-status block
/// (briefings, worker probes) and a one-line model identity at turn time.
/// That part cannot be rendered here without its side effects (reading the
/// briefings acknowledges them), so it is left out; the provider's preflight
/// measures the real request and refuses what does not fit.
async fn measure_local_request(
    agent: &Agent,
    session_id: &str,
    provider_name: &str,
    model_name: &str,
) -> Option<LocalRequestSize> {
    let tools = agent.list_tools(session_id, None).await;
    let tools_json = serde_json::to_string(&tools).ok()?;
    let working_dir = std::env::current_dir().ok()?;
    let extensions_info = agent
        .extension_manager
        .get_extensions_info(&working_dir)
        .await;
    let saved_skills = match agent.config.session_manager.pool_clone().await {
        Ok(pool) => permagent::skills::build_skills_prompt(&pool)
            .await
            .ok()
            .flatten(),
        Err(_) => None,
    };
    let counter = permagent::token_counter::create_token_counter()
        .await
        .ok()?;
    let provider_name = provider_name.to_string();
    let model_name = model_name.to_string();
    // Rendering and counting a 100KB prompt is CPU work; keep it off the
    // runtime.
    tokio::task::spawn_blocking(move || {
        let declared: Vec<String> = extensions_info.iter().map(|e| e.name.clone()).collect();
        let mut manager = permagent::agents::prompt_manager::PromptManager::new();
        if let Some(skills) = saved_skills {
            manager.add_system_prompt_extra("saved_skills".to_string(), skills);
        }
        let system = manager
            .builder()
            .with_extensions(extensions_info.into_iter())
            .with_declared_extensions(Some(declared))
            .with_hints(&working_dir)
            .with_model_family_from(&provider_name, &model_name)
            .build();
        LocalRequestSize {
            system: counter.count_tokens(&system),
            tools: counter.count_tokens(&tools_json),
        }
    })
    .await
    .ok()
}

/// The sentence the user reads when a session does not comfortably fit its
/// local window, worded by what the measurement after any trim found.
fn local_window_trim_notice(
    model: &str,
    window: usize,
    before: LocalRequestSize,
    after: LocalRequestSize,
    kept: &[String],
    shed: &[String],
) -> String {
    let mut notice = format!(
        "ollama/{model} runs in a {window}-token window, and this session's system prompt \
         and tools came to about {} tokens",
        before.total()
    );
    if shed.is_empty() {
        notice.push_str(". Every loaded extension was asked for by name, so none was dropped");
    } else {
        let loads = if kept.is_empty() {
            "no extensions".to_string()
        } else {
            format!("only {}", kept.join(", "))
        };
        notice.push_str(&format!(
            ", so it now loads {loads} (dropped: {})",
            shed.join(", ")
        ));
    }
    let breakdown = format!(
        "about {} tokens (system prompt {}, tools {})",
        after.total(),
        after.system,
        after.tools
    );
    if after.total() > window {
        notice.push_str(&format!(
            ". They are still at least {breakdown}, more than the whole window, so every \
             turn will be refused rather than silently cut. Use a model with a larger window."
        ));
    } else if after.total() > local_request_budget(window) {
        notice.push_str(&format!(
            ". They are still at least {breakdown}, which leaves little of the window for \
             the conversation and the reply; once it fills, turns will be refused rather \
             than silently cut. A model with a larger window would do better."
        ));
    } else {
        notice.push_str(&format!(
            ". That brings them to {breakdown}, which leaves room for the conversation."
        ));
    }
    notice
}

/// Fit a new session's extensions to the window a local Ollama model will
/// really run it in.
///
/// Why: `permagent run` fell back to `ollama/qwen2.5:7b` and sent it every
/// profile extension — 156 tool schemas and a 101KB system prompt, 62,364
/// tokens by the reservation's count (cli log 20260919_030958) — and Ollama
/// kept the last 2,050 of them. The provider now honours the fallback's
/// 32768 window (see `permagent::providers::ollama::request_window`), but
/// that request does not fit 32768 either. Most of it is extensions the turn
/// never needed, so shed them here, before the first turn, measuring the
/// system prompt and the tools together (the system prompt is the larger
/// part), and say what the result is. The provider's preflight refuses
/// whatever still does not fit.
///
/// Returns the notice when the session did not fit the budget.
async fn fit_extensions_to_local_window(
    agent: &Agent,
    session_id: &str,
    provider_name: &str,
    model_config: &permagent::model::ModelConfig,
    explicit: &BTreeSet<String>,
) -> Option<String> {
    if provider_name != "ollama" {
        return None;
    }
    let window = permagent::providers::ollama::request_window_for_model_config(model_config)
        .await?
        .tokens;
    let model_name = model_config.model_name.as_str();
    let before = measure_local_request(agent, session_id, provider_name, model_name).await?;
    if before.total() <= local_request_budget(window) {
        return None;
    }

    let mut keep = explicit.clone();
    keep.extend(
        LOCAL_WINDOW_CORE_EXTENSIONS
            .iter()
            .map(|name| permagent::config::name_to_key(name)),
    );
    let loaded = agent.list_extensions().await;
    let shed = extensions_to_shed(&loaded, &keep);
    for name in &shed {
        if let Err(error) = agent.remove_extension(name, session_id).await {
            tracing::warn!(
                extension = %name,
                %error,
                "could not drop extension to fit the local window"
            );
        }
    }
    let after = if shed.is_empty() {
        before
    } else {
        measure_local_request(agent, session_id, provider_name, model_name)
            .await
            .unwrap_or(before)
    };
    let mut kept: Vec<String> = agent.list_extensions().await;
    kept.sort();
    tracing::warn!(
        target: "permagent::cost_router",
        provider = provider_name,
        model = model_name,
        window,
        system_tokens_before = before.system,
        tool_tokens_before = before.tools,
        system_tokens_after = after.system,
        tool_tokens_after = after.tools,
        fits_window = after.total() <= window,
        shed = %shed.join(","),
        "fitted extensions to the local model's window"
    );
    Some(local_window_trim_notice(
        model_name, window, before, after, &kept, &shed,
    ))
}

async fn resolve_and_load_extensions(
    agent: Agent,
    extensions: Vec<ExtensionConfig>,
    provider_for_debug: Arc<dyn permagent::providers::base::Provider>,
    interactive: bool,
    session_id: &str,
) -> (Arc<Agent>, Vec<String>) {
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
        let orientation = orientation_block(map_block.clone());
        if let Some(map) = map_block {
            session
                .agent
                .extend_system_prompt_with_context(
                    "repo_map".to_string(),
                    orientation,
                    permagent::context_packet::ContextContribution::project_memory(
                        // The attribution is the exact text installed in the
                        // extra, including its orientation guardrails; do
                        // not reconstruct it from prompt prose later.
                        orientation_block(Some(map)),
                        ["project:repo_map".to_string()],
                    ),
                )
                .await;
        } else {
            session
                .agent
                .extend_system_prompt("repo_map".to_string(), orientation)
                .await;
        }
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
    let mut agent = new_primary_agent().await;

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

    let recipe = session_config.recipe.as_ref();
    let coding_harness = recipe
        .map(crate::recipes::builtin_recipes::is_coding_harness_recipe)
        .unwrap_or(false);
    agent.config.headless_coding_coordinator =
        wants_headless_goal_dispatch(coding_harness, agent.config.goose_mode)
            && !session_config.interactive;

    let resolved =
        resolve_provider_and_model(&session_config, config, saved_provider, saved_model_config);
    let (resolved, coordinator_notice) = enforce_harness_coordinator_route(
        resolved,
        coding_harness,
        harness_role_route(config),
        session_config.provider.is_some()
            || session_config.model.is_some()
            || harness_role_selection_is_explicit(config),
        session_config.interactive,
    );
    // Resolution answers "what is configured", not "what is up". Settle a dead
    // self-hosted endpoint HERE, so the banner below prints what will actually
    // serve and turn one is not spent finding a closed port.
    let (resolved, headless_failover_notice) =
        failover_if_endpoint_is_dead(resolved, session_config.interactive).await;

    // Preserve both startup routing receipts when the coordinator fallback is
    // followed by an endpoint failover. Headless runs journal this combined
    // evidence below; interactive runs already saw each notice at the point
    // it was emitted.
    let startup_notice = match (coordinator_notice, headless_failover_notice) {
        (Some(coordinator), Some(failover)) => Some(format!("{coordinator}\n{failover}")),
        (Some(notice), None) | (None, Some(notice)) => Some(notice),
        (None, None) => None,
    };

    if let Err(e) = agent
        .apply_recipe_components(recipe.and_then(|r| r.response.clone()), true)
        .await
    {
        output::render_error(&format!("Invalid recipe: {}", e));
        process::exit(1);
    }

    let session_id =
        resolve_session_id(&session_config, &session_manager, agent.config.goose_mode).await;

    // M2b: file the startup failover in the session's own journal, now that the
    // session exists. A headless run has no banner and no footer, so without
    // this the transcript claims nothing about which model actually served it —
    // and whoever reads the run afterwards has no way to find out.
    if let Some(notice) = startup_notice {
        let message = permagent::conversation::message::Message::assistant()
            .with_system_notification(
                permagent::conversation::message::SystemNotificationType::InlineMessage,
                notice,
            );
        if let Err(e) = session_manager.add_message(&session_id, &message).await {
            tracing::warn!("Failed to journal the startup failover notice: {e}");
        }
    }

    if session_config.resume {
        handle_resumed_session_workdir(&agent, &session_id, session_config.interactive).await;
    }

    let (extensions_for_provider, explicit_extensions) =
        match collect_extension_configs(&agent, &session_config, recipe, &session_id).await {
            Ok(collected) => collected,
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

    // Harness loop, Lane S / M3-1: a headless coding-harness Auto-mode launch
    // has no daemon to install the real goal dispatcher, so root goals a
    // `create_roadmap` call approves would otherwise sit in Ready forever
    // (`nudge_dispatch_ready_goals` returns 0 with no hook installed). Install
    // the SAME production hook the daemon installs at startup
    // (`install_dispatch_hook`, idempotent — an OnceLock, so a second install
    // anywhere else in this process is a harmless no-op) so the coordinator's
    // own `create_roadmap` call can actually dispatch. See
    // `wants_headless_goal_dispatch`'s doc comment for the scoping rationale.
    //
    // Lane R18f: captured once here (before `agent` moves into
    // `resolve_and_load_extensions` below) and reused after extension
    // loading to gate the orchestrator-tool-registration check.
    let headless_coding_dispatch =
        wants_headless_goal_dispatch(coding_harness, agent.config.goose_mode);
    if headless_coding_dispatch {
        // r64 s5: the `cost_routing planner=` receipt names the planner that
        // physically ANSWERED a decompose call this run, or `none` when no
        // decompose call happened (a resume-only run). It is set only by
        // `decompose_roadmap` (orchestrator.rs); seeding it here with the
        // coordinator claimed a planner nobody asked, and let a decompose
        // routed elsewhere hide behind it. `permagent-eval` reads `none` as
        // an explicit value, not an unresolved planner.
        // R1(b) (harness loop, Lane D1, from Lane F8 forensics): seed the
        // project this coordinator's own `create_roadmap` will resolve onto
        // with a real `root_path` before any goal can dispatch, so
        // `decisions_effects::land_approved_goal`'s ordinary path
        // (`project.root_path`) works without ever needing R1(a)'s
        // worktree-git fallback. The eval harness's default `personal`
        // project (and any project created with no local folder) is seeded
        // `root_path` NULL (`session/spectral_schema.rs:610`) and nothing
        // upstream of this ever set it — confirmed identically NULL across
        // all 5 of Lane F8's measured fixtures. Best-effort: a pool/session
        // lookup failure here must never block a headless launch.
        if let Ok(pool) = session_manager.pool_clone().await {
            if let Ok(session) = session_manager.get_session(&session_id, false).await {
                permagent::agents::platform_extensions::orchestrator::ensure_headless_project_root_path(
                    &pool,
                    &session.working_dir,
                )
                .await;
            }
        }
        // Harness loop, Lane T / T1+T2: NOT the daemon's `install_dispatch_hook`
        // (that closure's context always carries `session: None`, so its
        // InternalSubagentEngine worker-selection arm hard-fails with
        // "Extension manager not available" — S1 in HARNESS_LOOP_LOG.md — and
        // its ranked selection can pick an ExternalCli worker, spawning the
        // operator's own Claude Code/Codex/Cursor CLI unattended). The CLI's
        // own installer carries this coordinator session so the hook can
        // resolve a provider from it and restrict ranked selection to
        // in-process workers by default.
        permagent::agents::platform_extensions::orchestrator::install_dispatch_hook_for_headless_session(
            session_manager.clone(),
            session_id.clone(),
        );
        // Z2-2 (harness loop, coding-harness excellence iteration, Round C,
        // M6 #3): same idea as the dispatch hook above, for the `escalate`
        // tool's decision sink. The daemon installs the SQL-backed sink at
        // `AppState::new`; a headless CLI session never runs that path, so
        // without this an `escalate` call here recorded into the ephemeral
        // in-memory default sink — success reported, no `decisions` row ever
        // existed, no error, the task stalled until the outer cap. Idempotent
        // (`OnceLock`) — harmless if a daemon in the same process already
        // installed it.
        permagent::agents::platform_extensions::orchestrator::install_decision_sink_for_headless_session(
            &session_manager,
        )
        .await;
    }

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
    let (agent_ptr, failed_extensions) = resolve_and_load_extensions(
        agent,
        extensions_for_provider,
        Arc::clone(&provider_for_debug),
        session_config.interactive,
        &session_id,
    )
    .await;

    // A new session only: trimming persists the smaller set, and a resumed
    // session's extensions are the user's saved choice. There the provider's
    // preflight refusal is the guard.
    if !session_config.resume {
        if let Some(notice) = fit_extensions_to_local_window(
            &agent_ptr,
            &session_id,
            &resolved.provider_name,
            &provider_for_debug.get_model_config(),
            &explicit_extensions,
        )
        .await
        {
            if let Some(notice) = deliver_failover_notice(notice, session_config.interactive) {
                let message = permagent::conversation::message::Message::assistant()
                    .with_system_notification(
                        permagent::conversation::message::SystemNotificationType::InlineMessage,
                        notice,
                    );
                if let Err(e) = session_manager.add_message(&session_id, &message).await {
                    tracing::warn!("Failed to journal the local-window trim notice: {e}");
                }
            }
        }
    }

    // Harness loop, Lane R18f (A17 analysis Class 4, HARNESS_LOOP_LOG.md): a
    // DeepSeek coding-harness round's first `decompose_roadmap` call came
    // back `-32002: Tool 'decompose_roadmap' not found` even though the
    // orchestrator extension itself had already finished registering
    // (`search_available_extensions` reported it loaded) — the tool list a
    // turn's first dispatch resolves against was not verified before the
    // turn started. `resolve_and_load_extensions` above already AWAITS every
    // extension's `add_extension` call, so this is not a missing `.await`;
    // it is a missing VERIFICATION that the awaited registration actually
    // produced a resolvable tool set. Scoped to exactly the launch shape
    // that cannot recover from this the way an interactive session can (ask
    // the human, retry next message): the headless coding-harness coordinator,
    // whose NON-NEGOTIABLE first tool call is `decompose_roadmap`.
    if headless_coding_dispatch {
        let orchestrator_name =
            permagent::agents::platform_extensions::orchestrator::EXTENSION_NAME;
        if failed_extensions
            .iter()
            .any(|name| name == orchestrator_name)
        {
            // The load genuinely FAILED (not raced) — `load_extensions`
            // already printed the underlying error to stderr; this is fatal
            // for a coding-harness run specifically, because there is no
            // honest way to run the recipe's NON-NEGOTIABLE protocol without
            // decompose_roadmap/create_roadmap/escalate.
            tracing::error!(
                extension = orchestrator_name,
                "Lane R18f: orchestrator extension failed to load for a headless coding-harness \
                 session; aborting rather than running a turn that cannot dispatch or escalate"
            );
            output::render_error(&format!(
                "The '{orchestrator_name}' extension failed to load for this headless \
                 coding-harness session (see the warning above for why). decompose_roadmap, \
                 create_roadmap and escalate are unavailable, so this run cannot honestly \
                 follow the coding recipe's protocol. Aborting instead of starting a turn that \
                 can only end in a -32002 (Lane R18f)."
            ));
            process::exit(1);
        }

        // Bounded wait: `add_extension` completing does not, by itself, prove
        // the tool this session will dispatch is resolvable — verify it
        // directly against the same `list_tools` path a turn's first
        // dispatch uses, retrying briefly in case the registration that
        // *did* succeed above needs one more tick to be visible.
        const MAX_ATTEMPTS: u32 = 10;
        const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(150);
        let decompose_roadmap_ready = wait_for_tool(MAX_ATTEMPTS, RETRY_DELAY, || {
            let agent_ptr = &agent_ptr;
            let session_id = &session_id;
            async move {
                let tools = agent_ptr.list_tools(session_id, None).await;
                tool_list_exposes(&tools, "decompose_roadmap")
            }
        })
        .await;
        if !decompose_roadmap_ready {
            tracing::error!(
                tool = "decompose_roadmap",
                extension = orchestrator_name,
                attempts = MAX_ATTEMPTS,
                "Lane R18f: decompose_roadmap never became resolvable for a headless \
                 coding-harness session; aborting rather than starting a turn that would hit \
                 -32002 on its NON-NEGOTIABLE first tool call"
            );
            output::render_error(
                "decompose_roadmap did not become available for this headless coding-harness \
                 session after waiting for extension registration to settle. The coordinator's \
                 first tool call is required to be decompose_roadmap; starting the turn anyway \
                 would only reproduce the -32002 'tool not found' failure (Lane R18f, A17 \
                 analysis Class 4).",
            );
            process::exit(1);
        }
    }

    // Lane R61a: the coding harness's recall is scoped to the repo it works in
    // (`brain_sync::MemoryScope`). The one way to widen it is to ASK: a
    // frontend tool, so the call parks in the agent until this CLI's session
    // loop answers it with the user's terminal decision (or a refusal when
    // there is no terminal). A failed registration leaves the harness
    // repo-scoped with no way to widen — the safe direction.
    if coding_harness {
        if let Err(error) = agent_ptr
            .add_extension(
                ExtensionConfig::Frontend {
                    name: super::brain_sync::MEMORY_ACCESS_EXTENSION_NAME.to_string(),
                    description: "Ask the user for memory access beyond this repo".to_string(),
                    tools: vec![super::brain_sync::memory_access_tool()],
                    instructions: Some(super::brain_sync::MEMORY_ACCESS_INSTRUCTIONS.to_string()),
                    bundled: None,
                    available_tools: Vec::new(),
                },
                &session_id,
            )
            .await
        {
            tracing::warn!(
                %error,
                "could not register request_memory_access; memory recall stays repo-scoped"
            );
        }
    }

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

    // Read the identity out before the agent is moved into the session: the
    // banner has to name whoever the model is being told it is, and
    // `new_primary_agent` installed that several hundred lines ago.
    let persona_name = agent_ptr.persona_display_name().await;

    let mut session = CliSession::new(
        Arc::try_unwrap(agent_ptr).unwrap_or_else(|_| panic!("There should be no more references")),
        session_id.clone(),
        debug_mode,
        session_config.scheduled_job_id.clone(),
        session_config.max_turns,
        edit_mode,
        recipe.and_then(|r| r.retry.clone()),
        session_config.output_format.clone(),
        coding_harness,
    )
    .await;

    // A coding-harness process announces itself in `harness_process_leases`
    // for its whole lifetime, so the daemon and other terminals can see it is
    // live (and notice when it is not). No-op for any other session.
    session
        .start_harness_lease(session_config.interactive, recipe.map(|r| r.title.as_str()))
        .await;

    configure_session_prompts(&session, config, &session_config, &session_id).await;

    if !session_config.quiet {
        output::display_session_info(
            session_config.resume,
            &resolved.provider_name,
            &resolved.model_name,
            &Some(session_id),
            &persona_name,
        );
        // No opening greeting here. The banner already ends on "<name> is
        // ready", and a second line saying hello is one more thing to read
        // before you can start working (ruling 2026-08-31, the user: "I don't
        // need it to say Hello sir."). The persona's `opening_greeting` is
        // unchanged and still Chat's — a graphical client opening on an empty
        // thread has a blank space to fill; a terminal prompt does not.
    }
    session
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Option<String> {
        Some(v.to_string())
    }

    // ── headless goal dispatch (harness loop, Lane S / M3-1) ───────────────

    #[test]
    fn headless_goal_dispatch_wants_coding_harness_and_auto_mode_together() {
        assert!(wants_headless_goal_dispatch(true, GooseMode::Auto));
    }

    #[test]
    fn headless_goal_dispatch_skips_a_non_coding_harness_session_even_in_auto_mode() {
        assert!(!wants_headless_goal_dispatch(false, GooseMode::Auto));
    }

    #[test]
    fn headless_goal_dispatch_skips_the_coding_harness_outside_auto_mode() {
        assert!(!wants_headless_goal_dispatch(true, GooseMode::Approve));
        assert!(!wants_headless_goal_dispatch(true, GooseMode::Chat));
    }

    /// r64 s5: the session builder never seeds the `cost_routing planner=`
    /// receipt with the coordinator. Only `decompose_roadmap` sets it, from the
    /// planner that answered; a run with no decompose call prints
    /// `planner=none`. A structural, source-text check (like
    /// `cost_routing_receipt_is_printed_only_after_the_goal_drain` in
    /// `session/mod.rs`): `build_session`'s error paths call
    /// `process::exit(1)`, which a unit test must not invoke.
    #[test]
    fn cost_routing_planner_is_not_seeded_with_the_coordinator_at_session_start() {
        let src = include_str!("builder.rs");
        let body = src.split("\nmod tests {").next().unwrap();
        assert!(
            body.contains("if headless_coding_dispatch {"),
            "the headless coding-harness dispatch gate must still exist"
        );
        assert_eq!(
            body.matches("routing_receipt::set_planner(").count(),
            0,
            "the planner receipt is the decompose call's answering planner, never the \
             coordinator seeded at session start"
        );
    }

    // ── orchestrator tool-registration wait (Lane R18f, A17 analysis Class 4) ──

    fn tool_named(name: &str) -> rmcp::model::Tool {
        rmcp::model::Tool::new(
            name.to_string(),
            "test tool".to_string(),
            rmcp::object!({"type": "object"}),
        )
    }

    #[test]
    fn tool_list_exposes_matches_the_unprefixed_name() {
        let tools = vec![tool_named("decompose_roadmap")];
        assert!(tool_list_exposes(&tools, "decompose_roadmap"));
    }

    #[test]
    fn tool_list_exposes_matches_the_prefixed_name() {
        let tools = vec![tool_named("orchestrator__decompose_roadmap")];
        assert!(tool_list_exposes(&tools, "decompose_roadmap"));
    }

    #[test]
    fn tool_list_exposes_is_false_when_the_tool_is_absent() {
        let tools = vec![
            tool_named("developer__shell"),
            tool_named("analyze__search"),
        ];
        assert!(!tool_list_exposes(&tools, "decompose_roadmap"));
    }

    /// The exact Class-4 repair: an extension whose registration is still in
    /// flight when the first probe runs (as `search_available_extensions`
    /// showed "loaded" while the tool list was still stale) must not fail the
    /// session outright — a later probe within the bounded window finds it.
    #[tokio::test]
    async fn wait_for_tool_returns_true_once_a_late_probe_finds_it() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let calls_probe = calls.clone();
        let found = wait_for_tool(5, std::time::Duration::from_millis(0), move || {
            let calls_probe = calls_probe.clone();
            async move {
                let n = calls_probe.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                n >= 3
            }
        })
        .await;
        assert!(found, "a probe that eventually succeeds must be found");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            3,
            "must stop retrying as soon as the tool is found, not run all attempts"
        );
    }

    /// A permanently missing tool (the extension genuinely failed, or the
    /// tool was never going to register) must give up after `max_attempts`
    /// rather than retry forever — this is the branch `build_session` turns
    /// into the named hard error and `process::exit(1)`.
    #[tokio::test]
    async fn wait_for_tool_gives_up_after_max_attempts() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let calls_probe = calls.clone();
        let found = wait_for_tool(4, std::time::Duration::from_millis(0), move || {
            let calls_probe = calls_probe.clone();
            async move {
                calls_probe.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                false
            }
        })
        .await;
        assert!(
            !found,
            "a permanently missing tool must never be reported found"
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 4);
    }

    /// REPLACES a source-text grep.
    ///
    /// The old guard read this file's own product half looking for one
    /// `Agent::new();` and a literal `set_persona(...)` call. That could tell
    /// you the constructor was written, never that the persona it installs is
    /// the one the model is actually told about — the thing that was broken.
    /// `Agent::persona_display_name` now reads the identity back out of the
    /// prompt manager, so this asserts the contract instead of its spelling.
    #[tokio::test]
    async fn the_cli_agent_carries_the_persona_that_reaches_the_prompt() {
        let saved = permagent::config::agent_identity::load_shared_persona();
        let expected = saved.read().await.display_name();

        let agent = new_primary_agent().await;
        assert_eq!(
            agent.persona_display_name().await,
            expected,
            "the harness must run as the identity saved in agent.yaml, not \
             PromptManager's first-run fallback"
        );
    }

    /// …and the accessor is reading real state, not returning a constant: an
    /// agent with nothing installed reports the fallback, and installing a
    /// persona changes what the prompt would say.
    #[tokio::test]
    async fn the_installed_persona_is_what_the_accessor_reports() {
        let agent = Agent::new();
        let fallback = agent.persona_display_name().await;
        agent
            .set_persona(std::sync::Arc::new(tokio::sync::RwLock::new(
                permagent::config::agent_identity::PrimaryPersona {
                    first_name: "Henry".into(),
                    opening_greeting: "What are we building?".into(),
                    ..Default::default()
                },
            )))
            .await;
        assert_eq!(agent.persona_display_name().await, "Henry");
        assert_eq!(
            agent.persona_opening_greeting().await,
            "What are we building?"
        );
        assert_ne!(fallback, "Henry");
    }

    // ── startup reachability (M-startup) ──────────────────────────────────

    fn dead_split() -> ResolvedProviderConfig {
        ResolvedProviderConfig {
            provider_name: "qwen38_split".to_string(),
            model_name: "qwen3.8-27b".to_string(),
            model_config: permagent::model::ModelConfig::new("qwen3.8-27b").unwrap(),
        }
    }

    /// Source #4 (the harness role default) is an UNVERIFIED claim:
    /// `qwen38_split` declares `requires_auth: false`, so it reads as
    /// configured for the ~18 hours a day the split is not running. Waiting
    /// for the first turn to discover that costs a full turn of latency and,
    /// before M2, said nothing. Resolve the switch at startup instead.
    #[test]
    fn a_dead_local_endpoint_resolves_to_the_fallback_before_the_first_turn() {
        let target = permagent::cost_router::role_map::RoleModel {
            provider: "anthropic".to_string(),
            model: "claude-haiku-4-5".to_string(),
        };
        let (switched, notice) =
            startup_failover(&dead_split(), Some(&target)).expect("a configured fallback applies");
        assert_eq!(switched.provider_name, "anthropic");
        assert_eq!(switched.model_name, "claude-haiku-4-5");
        assert_eq!(
            switched.model_config.model_name, "claude-haiku-4-5",
            "the ModelConfig must be rebuilt for the new model, not carried over"
        );
        assert!(notice.contains("qwen38_split"), "{notice}");
        assert!(notice.contains("anthropic/claude-haiku-4-5"), "{notice}");
    }

    /// The startup path can now land on an Ollama stand-in. That model has no
    /// canonical limits, so the ModelConfig carries the knowledge row's window.
    /// The Ollama provider turns it into a native `/api/chat` request's
    /// `num_ctx` (`/v1` would drop it), and `fit_extensions_to_local_window`
    /// sizes the session to it.
    #[test]
    fn an_ollama_startup_fallback_carries_its_local_context_window() {
        let target = permagent::cost_router::role_map::RoleModel {
            provider: "ollama".to_string(),
            model: "qwen2.5:7b".to_string(),
        };
        let expected =
            permagent::cost_router::fallback::local_context_limit("ollama", "qwen2.5:7b");
        assert!(
            expected.is_some(),
            "qwen2.5:7b must have a local knowledge row"
        );
        let (switched, _notice) =
            startup_failover(&dead_split(), Some(&target)).expect("a configured fallback applies");
        assert_eq!(switched.provider_name, "ollama");
        // The invariant is "the fallback carries a window", not a specific
        // source for it: a canonical or config-supplied limit would also do.
        assert!(
            switched.model_config.context_limit.is_some(),
            "an Ollama fallback reached at startup must carry a context window"
        );
    }

    // ── fitting a local Ollama window (r3-finding-1) ─────────────────────
    // `permagent run` fell back to ollama/qwen2.5:7b with every profile
    // extension: 156 tools and a 101KB system prompt, 62,364 counted prompt
    // tokens, cut by Ollama to its last 2,050.

    fn keys(names: &[&str]) -> BTreeSet<String> {
        names
            .iter()
            .map(|n| permagent::config::name_to_key(n))
            .collect()
    }

    fn keys_in_order(names: &[&str]) -> Vec<String> {
        names
            .iter()
            .map(|n| permagent::config::name_to_key(n))
            .collect()
    }

    fn size(system: usize, tools: usize) -> LocalRequestSize {
        LocalRequestSize { system, tools }
    }

    /// The budget is the system prompt AND the tools against the window,
    /// with a quarter left for the conversation and the reply.
    #[test]
    fn the_budget_counts_the_system_prompt_with_the_tools() {
        assert_eq!(local_request_budget(4096), 3072);
        assert_eq!(local_request_budget(32_768), 24_576);
        // The reported request: over any window qwen2.5:7b has.
        assert!(size(22_000, 40_364).total() > local_request_budget(32_768));
        // Tools alone small, but the static system prompt (~20k tokens for
        // the `basic` snapshot) is what breaks a 4096 window.
        assert!(size(20_000, 1_000).total() > local_request_budget(4096));
        assert!(size(15_000, 1_000).total() <= local_request_budget(32_768));
    }

    #[test]
    fn a_trim_keeps_the_core_and_everything_named_explicitly() {
        // Loaded names are keyed exactly as ExtensionManager keys them, with
        // name_to_key(config.name()): "My Recipe Tool" loads as
        // "myrecipetool" (whitespace is dropped, not turned into '_').
        let loaded: Vec<String> = keys_in_order(&[
            "developer",
            "orchestrator",
            "librarian",
            "skills",
            "finance",
            "My Recipe Tool",
        ]);
        assert_eq!(loaded.last().map(String::as_str), Some("myrecipetool"));
        let mut keep = keys(LOCAL_WINDOW_CORE_EXTENSIONS);
        // A recipe or CLI flag named this one, spelled as its config name.
        keep.extend(keys(&["My Recipe Tool"]));
        let shed = extensions_to_shed(&loaded, &keep);
        assert_eq!(shed, vec!["orchestrator", "librarian", "skills", "finance"]);
    }

    #[test]
    fn nothing_is_shed_when_every_loaded_extension_was_asked_for() {
        let loaded = vec!["developer".to_string(), "orchestrator".to_string()];
        let keep = keys(&["developer", "orchestrator"]);
        assert!(extensions_to_shed(&loaded, &keep).is_empty());
    }

    /// The trimmed session fits: say what was dropped and what it now takes,
    /// and do not threaten a refusal.
    #[test]
    fn a_trim_that_fits_says_so() {
        let notice = local_window_trim_notice(
            "qwen2.5:7b",
            32_768,
            size(22_000, 40_000),
            size(15_000, 1_000),
            &["developer".to_string()],
            &["orchestrator".to_string(), "skills".to_string()],
        );
        assert!(notice.contains("ollama/qwen2.5:7b"), "{notice}");
        assert!(notice.contains("32768-token window"), "{notice}");
        assert!(notice.contains("about 62000 tokens"), "{notice}");
        assert!(notice.contains("only developer"), "{notice}");
        assert!(notice.contains("orchestrator, skills"), "{notice}");
        assert!(notice.contains("about 16000 tokens"), "{notice}");
        assert!(!notice.contains("refused"), "{notice}");
    }

    /// The case the old notice got wrong: the TOOLS fit after the trim, but
    /// the system prompt alone is bigger than the window. The notice must
    /// say turns will be refused, not just "loads only developer".
    #[test]
    fn a_trim_that_cannot_fit_the_system_prompt_says_turns_will_be_refused() {
        let notice = local_window_trim_notice(
            "qwen2.5:7b",
            4096,
            size(22_000, 40_000),
            size(15_000, 1_000),
            &["developer".to_string()],
            &["orchestrator".to_string()],
        );
        assert!(notice.contains("only developer"), "{notice}");
        assert!(
            notice.contains("system prompt 15000, tools 1000"),
            "{notice}"
        );
        assert!(notice.contains("every turn will be refused"), "{notice}");
        assert!(notice.contains("larger window"), "{notice}");

        // Inside the window but past the budget: warn, do not promise a refusal
        // on the first turn.
        let notice = local_window_trim_notice(
            "qwen25-16k:latest",
            16_384,
            size(20_000, 30_000),
            size(13_000, 1_000),
            &["developer".to_string()],
            &["orchestrator".to_string()],
        );
        assert!(notice.contains("leaves little of the window"), "{notice}");
        assert!(!notice.contains("every turn will be refused"), "{notice}");

        // Nothing could be dropped at all: still say what will happen.
        let notice = local_window_trim_notice(
            "qwen2.5:7b",
            4096,
            size(15_000, 1_000),
            size(15_000, 1_000),
            &["developer".to_string()],
            &[],
        );
        assert!(notice.contains("none was dropped"), "{notice}");
        assert!(notice.contains("every turn will be refused"), "{notice}");
    }

    /// No fallback configured is not a licence to pretend: stay put, and let
    /// the caller say the endpoint is dead.
    #[test]
    fn no_configured_fallback_means_no_silent_substitution() {
        assert!(startup_failover(&dead_split(), None).is_none());
    }

    #[test]
    fn unrated_local_model_is_rerouted_before_the_coding_turn() {
        let resolved = ResolvedProviderConfig {
            provider_name: "ollama".into(),
            model_name: "qwen25-16k:latest".into(),
            model_config: permagent::model::ModelConfig::new("qwen25-16k:latest").unwrap(),
        };
        let (routed, notice) =
            enforce_harness_coordinator_route(resolved, true, None, false, false);
        assert_eq!(routed.provider_name, "openai");
        assert_eq!(routed.model_name, "gpt-5.4-mini");
        assert_eq!(routed.model_config.model_name, "gpt-5.4-mini");
        let notice = notice.expect("headless routing must retain a durable receipt");
        assert!(notice.contains("not graduated"), "{notice}");
        assert!(notice.contains("ollama/qwen25-16k:latest"), "{notice}");
        assert!(notice.contains("openai/gpt-5.4-mini"), "{notice}");
        assert!(notice.contains("cost meter"), "{notice}");
    }

    #[test]
    fn trusted_default_model_is_not_overridden() {
        let resolved = ResolvedProviderConfig {
            provider_name: "openai".into(),
            model_name: "gpt-5.4-mini".into(),
            model_config: permagent::model::ModelConfig::new("gpt-5.4-mini").unwrap(),
        };
        let (routed, notice) =
            enforce_harness_coordinator_route(resolved, true, None, false, false);
        assert_eq!(routed.provider_name, "openai");
        assert_eq!(routed.model_name, "gpt-5.4-mini");
        assert!(notice.is_none());
    }

    #[test]
    fn heuristic_non_local_model_still_needs_harness_evidence() {
        let resolved = ResolvedProviderConfig {
            provider_name: "minimax".into(),
            model_name: "MiniMax-M2.5".into(),
            model_config: permagent::model::ModelConfig::new("MiniMax-M2.5").unwrap(),
        };
        let (routed, notice) =
            enforce_harness_coordinator_route(resolved, true, None, false, false);
        assert_eq!(routed.provider_name, "openai");
        assert_eq!(routed.model_name, "gpt-5.4-mini");
        assert!(notice
            .expect("headless routing must retain a receipt")
            .contains("graduation evidence is missing"));
    }

    #[test]
    fn explicit_unrated_model_is_preserved_without_paid_fallback() {
        let resolved = ResolvedProviderConfig {
            provider_name: "ollama".into(),
            model_name: "qwen25-16k:latest".into(),
            model_config: permagent::model::ModelConfig::new("qwen25-16k:latest").unwrap(),
        };
        let (routed, notice) = enforce_harness_coordinator_route(resolved, true, None, true, false);
        assert_eq!(routed.provider_name, "ollama");
        assert_eq!(routed.model_name, "qwen25-16k:latest");
        let notice = notice.expect("headless explicit override must retain a receipt");
        assert!(notice.contains("explicitly selected"), "{notice}");
        assert!(notice.contains("unexpected provider spend"), "{notice}");
    }

    #[test]
    fn configured_harness_role_is_preserved_without_paid_fallback() {
        let root = std::env::temp_dir().join(format!(
            "permagent-harness-role-explicit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("test clock is after the Unix epoch")
                .as_nanos()
        ));
        let config =
            Config::new_with_file_secrets(root.join("config.yaml"), root.join("secrets.yaml"))
                .expect("temporary config should be constructible");
        config
            .set_params([
                ("harness_provider", "ollama"),
                ("harness_model", "qwen25-16k:latest"),
            ])
            .expect("temporary role config should be writable");
        assert!(harness_role_selection_is_explicit(&config));

        let resolved = ResolvedProviderConfig {
            provider_name: "ollama".into(),
            model_name: "qwen25-16k:latest".into(),
            model_config: permagent::model::ModelConfig::new("qwen25-16k:latest").unwrap(),
        };
        let (routed, notice) = enforce_harness_coordinator_route(
            resolved,
            true,
            harness_role_route(&config),
            harness_role_selection_is_explicit(&config),
            false,
        );
        assert_eq!(routed.provider_name, "ollama");
        assert_eq!(routed.model_name, "qwen25-16k:latest");
        assert!(notice
            .expect("headless configured override must retain a receipt")
            .contains("unexpected provider spend"));
    }

    #[test]
    fn local_model_is_untouched_outside_the_coding_harness() {
        let resolved = ResolvedProviderConfig {
            provider_name: "ollama".into(),
            model_name: "qwen25-16k:latest".into(),
            model_config: permagent::model::ModelConfig::new("qwen25-16k:latest").unwrap(),
        };
        let (routed, notice) =
            enforce_harness_coordinator_route(resolved, false, None, false, false);
        assert_eq!(routed.provider_name, "ollama");
        assert_eq!(routed.model_name, "qwen25-16k:latest");
        assert!(notice.is_none());
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

    /// M2b: a headless run has no terminal and no footer, so the notice the
    /// interactive path prints must survive as something the session journal
    /// can hold. Returning `None` here is what made a scheduled recipe run on
    /// a model its own transcript never named.
    #[test]
    fn a_headless_failover_notice_survives_for_the_journal() {
        let notice = "Switched to anthropic/claude-haiku-4-5.".to_string();
        assert_eq!(
            deliver_failover_notice(notice.clone(), false),
            Some(notice.clone()),
            "headless must keep the sentence — stderr alone is not a journal"
        );
        assert_eq!(
            deliver_failover_notice(notice, true),
            None,
            "interactive already printed it; journalling it twice would double up"
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

    // ── `--no-session` budget task identity (lane R63a) ────────────────────
    //
    // A top-level `permagent run --no-session` creates a `SessionType::Hidden`
    // session with no parent. `Agent::reply` only mints a budget task for
    // interactive session types, so before this fix that Hidden session had no
    // durable task identity at all, and `spend_snapshot` refused with
    // `MissingTaskIdentity` — which the tool gate (56cee825) reads as "deny
    // every tool call". These tests exercise `resolve_session_id` directly
    // against a temp-dir `SessionManager`, without touching the real Agent or
    // any process-global state.

    // Also serialized on `no_session_parent_env`: `PARENT_SESSION_ID_ENV` is a
    // process-global var, and without this a concurrently-running inherited-
    // parent test could set it mid-test, sending this test's session into
    // `create_session_with_parent` with a parent id from a *different* temp
    // dir's database — `process::exit(1)`-ing the whole test binary.
    #[tokio::test]
    #[serial_test::serial(no_session_parent_env)]
    async fn top_level_no_session_run_gets_a_durable_budget_task() {
        std::env::remove_var(permagent::session::PARENT_SESSION_ID_ENV);
        let temp_dir = tempfile::tempdir().unwrap();
        let session_manager =
            permagent::session::session_manager::SessionManager::new(temp_dir.path().to_path_buf());
        let config = SessionBuilderConfig {
            no_session: true,
            ..Default::default()
        };

        let session_id = resolve_session_id(&config, &session_manager, GooseMode::default()).await;

        let pool = session_manager.pool_clone().await.unwrap();
        let snapshot = permagent::agents::platform_extensions::orchestrator::spend_snapshot(
            &pool,
            &session_id,
        )
        .await;
        assert!(
            !matches!(
                snapshot,
                Err(
                    permagent::agents::platform_extensions::orchestrator::SpendSnapshotError::MissingTaskIdentity(_)
                )
            ),
            "a top-level `--no-session` run must have a durable budget task \
             identity so tool calls are not refused; spend_snapshot returned \
             {snapshot:?}"
        );

        let session = session_manager
            .get_session(&session_id, false)
            .await
            .unwrap();
        assert!(
            permagent::session::budget_task_id(&session.extension_data).is_some(),
            "the Hidden session's extension data must carry a budget_task id"
        );
    }

    /// The env var this reads (`PARENT_SESSION_ID_ENV`) is process-global, so
    /// this test and the one below serialize on the same key to avoid racing
    /// each other's mutations.
    #[tokio::test]
    #[serial_test::serial(no_session_parent_env)]
    async fn inherited_parent_no_session_run_reuses_the_parents_task() {
        std::env::remove_var(permagent::session::PARENT_SESSION_ID_ENV);
        let temp_dir = tempfile::tempdir().unwrap();
        let session_manager =
            permagent::session::session_manager::SessionManager::new(temp_dir.path().to_path_buf());

        let parent = session_manager
            .create_session(
                temp_dir.path().to_path_buf(),
                "parent".to_string(),
                SessionType::User,
                GooseMode::default(),
            )
            .await
            .unwrap();
        let parent_task = session_manager.begin_budget_task(&parent.id).await.unwrap();

        std::env::set_var(permagent::session::PARENT_SESSION_ID_ENV, &parent.id);
        let config = SessionBuilderConfig {
            no_session: true,
            ..Default::default()
        };
        let session_id = resolve_session_id(&config, &session_manager, GooseMode::default()).await;
        std::env::remove_var(permagent::session::PARENT_SESSION_ID_ENV);

        let child = session_manager
            .get_session(&session_id, false)
            .await
            .unwrap();
        let child_task = permagent::session::budget_task_id(&child.extension_data);
        assert_eq!(
            child_task,
            Some(parent_task),
            "a no_session run with an inherited parent must reuse the \
             parent's budget task, not mint a fresh one"
        );
    }
}
