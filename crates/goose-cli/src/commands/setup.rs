use anyhow::{Context, Result};
use dialoguer::{Confirm, Input, Password, Select};
use permagent::config::paths::Paths;
use permagent::config::Config;
use permagent::session::spectral_schema::{init_spectral_db, is_schema_initialized};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Provider choices for the setup wizard
const PROVIDERS: &[(&str, &str)] = &[
    ("anthropic", "Anthropic (Claude)"),
    ("openai", "OpenAI (GPT-4)"),
    ("ollama", "Ollama (local)"),
    ("google", "Google (Gemini)"),
    ("openrouter", "Other (OpenRouter, Azure, etc.)"),
];

/// Known models per provider for interactive selection
fn models_for_provider(provider: &str) -> Vec<&'static str> {
    match provider {
        "anthropic" => vec!["claude-sonnet-4-5", "claude-opus-4-6", "claude-haiku-4-5"],
        "openai" => vec!["gpt-4o", "gpt-4o-mini", "o3"],
        "ollama" => vec!["qwen2.5:14b", "llama3.1", "mistral", "codestral"],
        "google" => vec!["gemini-2.0-flash", "gemini-2.5-pro"],
        _ => vec!["default"],
    }
}

/// API key env var name for each provider
fn api_key_name(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "ANTHROPIC_API_KEY",
        "openai" => "OPENAI_API_KEY",
        "google" => "GOOGLE_API_KEY",
        "openrouter" => "OPENROUTER_API_KEY",
        _ => "LLM_API_KEY",
    }
}

/// API key help URL for each provider
fn api_key_url(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "https://console.anthropic.com/settings/keys",
        "openai" => "https://platform.openai.com/api-keys",
        "google" => "https://aistudio.google.com/apikey",
        "openrouter" => "https://openrouter.ai/keys",
        _ => "",
    }
}

/// Non-interactive setup options passed via CLI flags
pub struct NonInteractiveOpts {
    pub provider: String,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub agent_name: Option<String>,
}

/// Test that Ollama is reachable at localhost:11434.
async fn test_ollama_connection() -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;
    let resp = client.get("http://localhost:11434/api/tags").send().await?;
    if resp.status().is_success() {
        Ok(())
    } else {
        anyhow::bail!("HTTP {}", resp.status())
    }
}

/// Run the setup wizard interactively.
pub async fn handle_setup_interactive() -> Result<()> {
    let permagent_dir = Paths::config_dir();

    // Welcome screen
    println!();
    println!("Welcome to Permagent");
    println!("{}", "=".repeat(20));
    println!();
    println!("Permagent is your personal AI agent that runs locally on your Mac.");
    println!("It learns your workflows, remembers context across sessions, and");
    println!("connects to your tools (Gmail, Slack, and more) to act on your behalf.");
    println!();
    println!("This wizard will configure your LLM provider, initialize the memory");
    println!("database, and start the background daemon.");
    println!();

    // Check for existing installation
    if permagent_dir.exists() {
        let config_path = permagent_dir.join("config.yaml");
        if config_path.exists() {
            let overwrite = Confirm::new()
                .with_prompt("~/.permagent/config.yaml already exists. Overwrite config?")
                .default(false)
                .interact()?;
            if !overwrite {
                println!("Setup cancelled. Existing config preserved.");
                return Ok(());
            }
        }
    }

    // Step 1/6: LLM Provider
    println!("\nStep 1/6: LLM Provider");
    println!("{}", "-".repeat(21));

    let provider_labels: Vec<&str> = PROVIDERS.iter().map(|(_, label)| *label).collect();
    let provider_idx = Select::new()
        .with_prompt("Which LLM provider will you use?")
        .items(&provider_labels)
        .default(0)
        .interact()?;
    let provider_name = PROVIDERS[provider_idx].0;

    // Step 2/6: API Key
    println!("\nStep 2/6: API Key");
    println!("{}", "-".repeat(17));

    let api_key = if provider_name == "ollama" {
        println!("Ollama runs locally - no API key needed.");
        // Test Ollama connection
        print!("Testing connection to Ollama at localhost:11434... ");
        match test_ollama_connection().await {
            Ok(()) => println!("connected."),
            Err(e) => {
                println!("failed.");
                eprintln!(
                    "Warning: Could not reach Ollama ({}). Make sure it's running.",
                    e
                );
                eprintln!("Install Ollama from https://ollama.ai and run `ollama serve`.");
            }
        }
        None
    } else {
        let url = api_key_url(provider_name);
        if !url.is_empty() {
            println!("(Get one at {})", url);
        }
        let key = Password::new()
            .with_prompt(format!("Enter your {} API key", PROVIDERS[provider_idx].1))
            .interact()?;
        if key.is_empty() {
            println!("Warning: No API key provided. You can set it later with `permagent provider key <key>`.");
            None
        } else {
            Some(key)
        }
    };

    // Store API key
    if let Some(ref key) = api_key {
        let key_name = api_key_name(provider_name);
        // Ensure config dir exists before writing secrets
        fs::create_dir_all(&permagent_dir)?;
        let config = Config::global();
        match config.set_secret(key_name, key) {
            Ok(()) => println!("Key stored in system keyring (service: \"permagent\")"),
            Err(e) => {
                eprintln!(
                    "Warning: Keyring unavailable ({}), falling back to secrets.yaml",
                    e
                );
                write_secrets_fallback(&permagent_dir, key_name, key)?;
            }
        }
    }

    // Step 3/6: Model Selection
    println!("\nStep 3/6: Model Selection");
    println!("{}", "-".repeat(25));

    let models = models_for_provider(provider_name);
    let model_idx = Select::new()
        .with_prompt("Which model? (arrow keys to select)")
        .items(&models)
        .default(0)
        .interact()?;
    let default_model = models[model_idx];

    // Step 4/6: Agent Name
    println!("\nStep 4/6: Agent Name");
    println!("{}", "-".repeat(20));

    let agent_name: String = Input::new()
        .with_prompt("What should this agent be called?")
        .default("permagent".to_string())
        .interact_text()?;

    // Step 5/6: Initialize Spectral Memory
    println!("\nStep 5/6: Initialize Spectral Memory");
    println!("{}", "-".repeat(37));

    let spectral_result = init_spectral(&permagent_dir).await?;
    println!("{}", spectral_result);

    // Step 6/6: Start Daemon
    println!("\nStep 6/6: Start Daemon");
    println!("{}", "-".repeat(22));

    // Write config before daemon step
    write_config(&permagent_dir, provider_name, default_model, &agent_name)?;

    // Belt-and-suspenders: also write through Config API
    {
        let config = Config::global();
        let _ = config.set_goose_provider(provider_name);
        let _ = config.set_goose_model(default_model);
    }
    println!("Config written to ~/.permagent/config.yaml");

    let register_daemon = Confirm::new()
        .with_prompt("Register Permagent daemon with launchd? (keeps agent running in background)")
        .default(true)
        .interact()?;

    if register_daemon {
        match register_launchd_daemon() {
            Ok(()) => {
                println!("Daemon registered: ai.permagent.daemon");
                println!("Daemon started on localhost:3001");
            }
            Err(e) => {
                eprintln!("Warning: Could not register daemon: {}", e);
                eprintln!("You can start it manually with `permagent start`.");
            }
        }

        let open_browser = Confirm::new()
            .with_prompt("Open Command Center now?")
            .default(true)
            .interact()?;

        if open_browser {
            if let Err(e) = webbrowser::open("http://localhost:3001/ui/") {
                eprintln!(
                    "Could not open browser: {}. Visit http://localhost:3001/ui/",
                    e
                );
            }
        }
    } else {
        println!("Skipped daemon registration. Start manually with `permagent start`.");
    }

    println!("\nSetup complete!");
    Ok(())
}

/// Run setup in non-interactive mode (for CI/automation).
pub async fn handle_setup_non_interactive(opts: NonInteractiveOpts) -> Result<()> {
    let permagent_dir = Paths::config_dir();
    fs::create_dir_all(&permagent_dir)?;

    let provider_name = &opts.provider;

    // Store API key if provided and non-empty
    if let Some(ref key) = opts.api_key {
        if key.trim().is_empty() {
            eprintln!("Warning: Empty API key provided, skipping key storage.");
            eprintln!("Set the key later with: permagent provider key <key>");
        } else {
            let key_name = api_key_name(provider_name);
            let config = Config::global();
            match config.set_secret(key_name, key) {
                Ok(()) => println!("API key stored in system keyring"),
                Err(_) => {
                    write_secrets_fallback(&permagent_dir, key_name, key)?;
                    println!("API key stored in secrets.yaml (keyring unavailable)");
                }
            }
        }
    }

    // Pick default model
    let models = models_for_provider(provider_name);
    let fallback_model = models.first().copied().unwrap_or("default");
    let default_model = opts.model.as_deref().unwrap_or(fallback_model);

    // Write config template (creates full YAML structure)
    let agent_name = opts.agent_name.as_deref().unwrap_or("permagent");
    write_config(&permagent_dir, provider_name, default_model, agent_name)?;

    // Belt-and-suspenders: also write through Config API to ensure the values
    // survive any YAML round-trip (migration, set_param on other keys, etc.)
    {
        let config = Config::global();
        if let Err(e) = config.set_goose_provider(provider_name) {
            eprintln!(
                "Warning: Failed to persist GOOSE_PROVIDER via Config API: {}",
                e
            );
        }
        if let Err(e) = config.set_goose_model(default_model) {
            eprintln!(
                "Warning: Failed to persist GOOSE_MODEL via Config API: {}",
                e
            );
        }
    }
    println!("Config written to ~/.permagent/config.yaml");

    // Initialize Spectral
    let spectral_result = init_spectral(&permagent_dir).await?;
    println!("{}", spectral_result);

    // Create logs dir
    fs::create_dir_all(Paths::logs_dir())?;

    // Register and start the daemon (same as interactive path)
    match register_launchd_daemon() {
        Ok(()) => {
            println!("Daemon registered: ai.permagent.daemon");
            println!("Daemon started on localhost:3001");
        }
        Err(e) => {
            eprintln!("Warning: Could not register daemon: {}", e);
            eprintln!("Start manually: permagent start");
        }
    }

    println!("Setup complete (non-interactive).");
    Ok(())
}

/// Initialize the Spectral database, respecting existing data.
async fn init_spectral(permagent_dir: &Path) -> Result<String> {
    let spectral_dir = permagent_dir.join("spectral");
    fs::create_dir_all(&spectral_dir)?;

    let db_path = spectral_dir.join("permagent.db");

    // If DB already exists, check if schema is initialized
    if db_path.exists() {
        let connect_opts =
            SqliteConnectOptions::from_str(&format!("sqlite:{}", db_path.display()))?
                .create_if_missing(false);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(connect_opts)
            .await?;

        if is_schema_initialized(&pool).await? {
            pool.close().await;
            return Ok(
                "Spectral database already initialized - skipping (use `permagent setup` interactively to overwrite)"
                    .to_string(),
            );
        }
        pool.close().await;
    }

    // Create and initialize
    let connect_opts = SqliteConnectOptions::from_str(&format!("sqlite:{}", db_path.display()))?
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(connect_opts)
        .await
        .context("Failed to create Spectral database")?;

    init_spectral_db(&pool)
        .await
        .context("Failed to initialize Spectral schema")?;

    // Count tables and indexes for confirmation
    let table_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '%_fts%'")
            .fetch_one(&pool)
            .await?;
    let index_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name LIKE '%_fts%'",
    )
    .fetch_one(&pool)
    .await?;

    pool.close().await;

    Ok(format!(
        "Spectral initialized ({} tables, {} FTS indexes)",
        table_count.0, index_count.0
    ))
}

/// Write ~/.permagent/config.yaml per Section E.3 format.
fn write_config(
    permagent_dir: &PathBuf,
    provider_name: &str,
    default_model: &str,
    agent_name: &str,
) -> Result<()> {
    fs::create_dir_all(permagent_dir)?;
    fs::create_dir_all(Paths::logs_dir())?;

    let config_content = format!(
        r#"# ~/.permagent/config.yaml
version: 1

GOOSE_PROVIDER: {provider}
GOOSE_MODEL: {model}

agent:
  name: {agent_name}

daemon:
  host: 127.0.0.1
  port: 3001
  auto_start: true

spectral:
  db_path: ~/.permagent/spectral/permagent.db

integrations: {{}}
  # gmail:
  #   enabled: true
  #   scopes: ["readonly"]
  # slack:
  #   enabled: true
  #   scopes: ["chat:write", "channels:read"]

skills:
  auto_detect: true
  repetition_threshold: 2
  repetition_window_days: 7
"#,
        agent_name = agent_name,
        provider = provider_name,
        model = default_model,
    );

    let config_path = permagent_dir.join("config.yaml");
    fs::write(&config_path, config_content)?;
    Ok(())
}

/// Fallback: write API key to secrets.yaml when keyring is unavailable.
fn write_secrets_fallback(permagent_dir: &Path, key_name: &str, key: &str) -> Result<()> {
    let secrets_path = permagent_dir.join("secrets.yaml");

    // Load existing secrets if any
    let mut secrets: serde_yaml::Value = if secrets_path.exists() {
        let content = fs::read_to_string(&secrets_path)?;
        serde_yaml::from_str(&content).unwrap_or(serde_yaml::Value::Mapping(Default::default()))
    } else {
        serde_yaml::Value::Mapping(Default::default())
    };

    if let serde_yaml::Value::Mapping(ref mut map) = secrets {
        map.insert(
            serde_yaml::Value::String(key_name.to_string()),
            serde_yaml::Value::String(key.to_string()),
        );
    }

    let yaml_str = serde_yaml::to_string(&secrets)?;
    // Atomic write, 0600 from the first byte — never observable
    // world-readable, even transiently.
    permagent::config::secure_fs::write_private_file(&secrets_path, yaml_str.as_bytes())?;

    Ok(())
}

/// Generate and register the launchd plist for the Permagent daemon.
fn register_launchd_daemon() -> Result<()> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    let launch_agents_dir = home.join("Library/LaunchAgents");
    fs::create_dir_all(&launch_agents_dir)?;

    let plist_path = launch_agents_dir.join("ai.permagent.daemon.plist");
    let home_str = home.display();

    // Find the permagentd binary - check common locations
    let daemon_binary = which_daemon_binary()?;

    let plist_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>ai.permagent.daemon</string>

    <key>ProgramArguments</key>
    <array>
        <string>{binary}</string>
        <string>agent</string>
    </array>

    <key>RunAtLoad</key>
    <true/>

    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>

    <!-- Across-restart backoff (durability F1/F6): if the daemon exits (a real
         crash, or the panic circuit-breaker forcing a clean exit(1)), launchd
         waits at least this many seconds before relaunching, so a crash-loop
         cannot tight-loop and hammer the CPU/disk. 30s mirrors the UI watchdog. -->
    <key>ThrottleInterval</key>
    <integer>30</integer>

    <key>StandardOutPath</key>
    <string>{home}/.permagent/logs/daemon.log</string>

    <key>StandardErrorPath</key>
    <string>{home}/.permagent/logs/daemon.err</string>

    <key>EnvironmentVariables</key>
    <dict>
        <key>PERMAGENT_CONFIG</key>
        <string>{home}/.permagent/config.yaml</string>
        <key>PERMAGENT_SPECTRAL_DB</key>
        <string>{home}/.permagent/spectral/permagent.db</string>
    </dict>

    <key>ProcessType</key>
    <string>Standard</string>
</dict>
</plist>
"#,
        binary = daemon_binary,
        home = home_str,
    );

    fs::write(&plist_path, &plist_content)?;

    // Unload any stale plist first (ignore errors — may not be loaded)
    let _ = std::process::Command::new("launchctl")
        .args(["unload"])
        .arg(&plist_path)
        .status();

    // Load the freshly-written plist
    let status = std::process::Command::new("launchctl")
        .args(["load", "-w"])
        .arg(&plist_path)
        .status()
        .context("Failed to run launchctl")?;

    if !status.success() {
        anyhow::bail!("launchctl load failed with exit code: {:?}", status.code());
    }

    Ok(())
}

/// Find the permagentd binary, checking cargo target dir and PATH.
fn which_daemon_binary() -> Result<String> {
    // Check if permagentd is next to the current binary
    if let Ok(current_exe) = std::env::current_exe() {
        let sibling = current_exe.parent().unwrap().join("permagentd");
        if sibling.exists() {
            return Ok(sibling.display().to_string());
        }
    }

    // Check common install paths
    for path in &["/usr/local/bin/permagentd", "/opt/homebrew/bin/permagentd"] {
        if std::path::Path::new(path).exists() {
            return Ok(path.to_string());
        }
    }

    // Try which
    if let Ok(output) = std::process::Command::new("which")
        .arg("permagentd")
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(path);
            }
        }
    }

    // Fall back to the standard install location
    Ok("/usr/local/bin/permagentd".to_string())
}
