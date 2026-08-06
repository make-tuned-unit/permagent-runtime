//! Import configuration from Claude Code and Codex without copying credentials.

use anyhow::{Context, Result};
use clap::{Args, ValueEnum};
use serde::Serialize;
use serde_json::{Map as JsonMap, Value as JsonValue};
use serde_yaml::{Mapping, Value as YamlValue};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const CONFIG_FILE: &str = "config.yaml";

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ImportSource {
    Auto,
    Claude,
    Codex,
}

#[derive(Args, Debug)]
pub struct ImportAgentArgs {
    /// Setup to import. Auto imports every setup it finds.
    #[arg(value_enum, default_value = "auto")]
    pub source: ImportSource,

    /// Override the source directory (normally ~/.claude or ~/.codex)
    #[arg(long, value_name = "DIR")]
    pub source_dir: Option<PathBuf>,

    /// Project whose CLAUDE.md/AGENTS.md and project MCP config should be imported
    #[arg(long, value_name = "DIR")]
    pub project: Option<PathBuf>,

    /// Show the complete import report without changing files
    #[arg(long)]
    pub dry_run: bool,

    /// Emit the report as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Outcome {
    Imported,
    Skipped,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReportItem {
    outcome: Outcome,
    category: String,
    item: String,
    reason: String,
}

#[derive(Default, Debug, Serialize)]
pub struct ImportReport {
    dry_run: bool,
    imported: usize,
    skipped: usize,
    items: Vec<ReportItem>,
}

impl ImportReport {
    fn add(
        &mut self,
        outcome: Outcome,
        category: &str,
        item: impl Into<String>,
        reason: impl Into<String>,
    ) {
        match outcome {
            Outcome::Imported => self.imported += 1,
            Outcome::Skipped => self.skipped += 1,
        }
        self.items.push(ReportItem {
            outcome,
            category: category.to_string(),
            item: item.into(),
            reason: reason.into(),
        });
    }

    fn imported(&mut self, category: &str, item: impl Into<String>, reason: impl Into<String>) {
        self.add(Outcome::Imported, category, item, reason);
    }

    fn skipped(&mut self, category: &str, item: impl Into<String>, reason: impl Into<String>) {
        self.add(Outcome::Skipped, category, item, reason);
    }
}

#[derive(Clone)]
struct ImportPaths {
    home: PathBuf,
    project: PathBuf,
    target: PathBuf,
}

pub fn handle_import_agent(args: ImportAgentArgs) -> Result<()> {
    let home = dirs::home_dir().context("could not determine the home directory")?;
    let paths = ImportPaths {
        project: args.project.unwrap_or(std::env::current_dir()?),
        target: permagent::config::paths::Paths::config_dir(),
        home,
    };
    let report = run_import(
        args.source,
        args.source_dir.as_deref(),
        &paths,
        args.dry_run,
    )?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        for item in &report.items {
            let mark = if item.outcome == Outcome::Imported {
                "IMPORTED"
            } else {
                "SKIPPED"
            };
            println!(
                "{mark} {:<12} {} — {}",
                item.category, item.item, item.reason
            );
        }
        println!(
            "\nImported: {}  Skipped: {}",
            report.imported, report.skipped
        );
        if report.dry_run {
            println!("Dry run: no files were changed.");
        }
    }
    if report.items.is_empty() {
        anyhow::bail!("no Claude Code or Codex setup was found");
    }
    Ok(())
}

fn run_import(
    requested: ImportSource,
    source_override: Option<&Path>,
    paths: &ImportPaths,
    dry_run: bool,
) -> Result<ImportReport> {
    let mut report = ImportReport {
        dry_run,
        ..Default::default()
    };
    let mut config = read_target_config(&paths.target.join(CONFIG_FILE))?;
    let mut changed_config = false;
    let sources: &[ImportSource] = match requested {
        ImportSource::Auto => &[ImportSource::Claude, ImportSource::Codex],
        ImportSource::Claude => &[ImportSource::Claude],
        ImportSource::Codex => &[ImportSource::Codex],
    };

    for source in sources {
        let root = source_override
            .map(Path::to_path_buf)
            .unwrap_or_else(|| match source {
                ImportSource::Claude => paths.home.join(".claude"),
                ImportSource::Codex => paths.home.join(".codex"),
                ImportSource::Auto => unreachable!(),
            });
        let found = match source {
            ImportSource::Claude => import_claude(
                &root,
                paths,
                &mut config,
                &mut changed_config,
                &mut report,
                dry_run,
            )?,
            ImportSource::Codex => import_codex(
                &root,
                paths,
                &mut config,
                &mut changed_config,
                &mut report,
                dry_run,
            )?,
            ImportSource::Auto => false,
        };
        if !found && requested != ImportSource::Auto {
            report.skipped(
                "setup",
                format!("{:?}", source).to_lowercase(),
                format!("no supported setup files found under {}", root.display()),
            );
        }
    }

    if changed_config && !dry_run {
        fs::create_dir_all(&paths.target)?;
        let serialized = serde_yaml::to_string(&config)?;
        fs::write(paths.target.join(CONFIG_FILE), serialized)
            .context("failed to write Permagent config.yaml")?;
    }
    Ok(report)
}

fn import_claude(
    root: &Path,
    paths: &ImportPaths,
    config: &mut Mapping,
    changed: &mut bool,
    report: &mut ImportReport,
    dry_run: bool,
) -> Result<bool> {
    let mut found = false;
    let mut candidates = vec![root.join("settings.json"), paths.project.join(".mcp.json")];
    if let Some(parent) = root.parent() {
        candidates.insert(0, parent.join(".claude.json"));
    }
    for path in candidates {
        if !path.is_file() {
            continue;
        }
        found = true;
        let value: JsonValue = serde_json::from_str(&fs::read_to_string(&path)?)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        if let Some(servers) = value.get("mcpServers").and_then(JsonValue::as_object) {
            import_mcp_servers("claude", servers, config, changed, report);
        }
        if let Some(projects) = value.get("projects").and_then(JsonValue::as_object) {
            let project_key = paths.project.to_string_lossy().into_owned();
            if let Some(servers) = projects
                .get(&project_key)
                .and_then(|project| project.get("mcpServers"))
                .and_then(JsonValue::as_object)
            {
                import_mcp_servers("claude project", servers, config, changed, report);
            }
        }
        if let Some(model) = value.get("model").and_then(JsonValue::as_str) {
            import_model("anthropic", model, config, changed, report, "Claude Code");
        }
    }
    found |= import_instructions(
        "claude",
        &[root.join("CLAUDE.md"), paths.project.join("CLAUDE.md")],
        paths,
        config,
        changed,
        report,
        dry_run,
    )?;
    found |= import_skills(
        "claude",
        &root.join("skills"),
        &paths.target.join("skills"),
        report,
        dry_run,
    )?;
    found |= import_commands(
        "claude",
        &root.join("commands"),
        &paths.target.join("skills"),
        report,
        dry_run,
    )?;
    Ok(found)
}

fn import_codex(
    root: &Path,
    paths: &ImportPaths,
    config: &mut Mapping,
    changed: &mut bool,
    report: &mut ImportReport,
    dry_run: bool,
) -> Result<bool> {
    let config_path = root.join("config.toml");
    let mut found = false;
    if config_path.is_file() {
        found = true;
        let value: toml::Value = toml::from_str(&fs::read_to_string(&config_path)?)
            .with_context(|| format!("failed to parse {}", config_path.display()))?;
        if let Some(servers) = value.get("mcp_servers").and_then(toml::Value::as_table) {
            let converted: JsonMap<String, JsonValue> = servers
                .iter()
                .filter_map(|(name, v)| serde_json::to_value(v).ok().map(|v| (name.clone(), v)))
                .collect();
            import_mcp_servers("codex", &converted, config, changed, report);
        }
        if let Some(model) = value.get("model").and_then(toml::Value::as_str) {
            let provider = value
                .get("model_provider")
                .and_then(toml::Value::as_str)
                .unwrap_or("openai");
            import_model(provider, model, config, changed, report, "Codex");
        } else if value.get("model_provider").is_some() {
            report.skipped(
                "model",
                "Codex provider",
                "provider has no accompanying model choice",
            );
        }
    }
    found |= import_instructions(
        "codex",
        &[root.join("AGENTS.md"), paths.project.join("AGENTS.md")],
        paths,
        config,
        changed,
        report,
        dry_run,
    )?;
    found |= import_skills(
        "codex",
        &root.join("skills"),
        &paths.target.join("skills"),
        report,
        dry_run,
    )?;
    found |= import_commands(
        "codex",
        &root.join("prompts"),
        &paths.target.join("skills"),
        report,
        dry_run,
    )?;
    Ok(found)
}

fn import_mcp_servers(
    origin: &str,
    servers: &JsonMap<String, JsonValue>,
    config: &mut Mapping,
    changed: &mut bool,
    report: &mut ImportReport,
) {
    let extensions_key = YamlValue::String("extensions".into());
    if !config.contains_key(&extensions_key) {
        config.insert(extensions_key.clone(), YamlValue::Mapping(Mapping::new()));
    }
    let Some(extensions) = config
        .get_mut(&extensions_key)
        .and_then(YamlValue::as_mapping_mut)
    else {
        for name in servers.keys() {
            report.skipped("mcp", name, "target extensions setting is not a mapping");
        }
        return;
    };
    for (name, server) in servers {
        let key = sanitize_name(name);
        if key.is_empty() {
            report.skipped("mcp", name, "name has no usable letters or digits");
            continue;
        }
        let yaml_key = YamlValue::String(key.clone());
        if extensions.contains_key(&yaml_key) {
            report.skipped(
                "mcp",
                name,
                "an extension with this normalized name already exists",
            );
            continue;
        }
        let Some(obj) = server.as_object() else {
            report.skipped("mcp", name, "server definition is not an object/table");
            continue;
        };
        for field in obj.keys().filter(|field| {
            ![
                "command", "args", "env", "envs", "url", "headers", "type", "enabled", "disabled",
                "timeout",
            ]
            .contains(&field.as_str())
        }) {
            report.skipped(
                "mcp setting",
                format!("{name}.{field}"),
                "setting has no clean Permagent mapping",
            );
        }
        if obj.get("command").is_some() && obj.get("url").is_some() {
            report.skipped(
                "mcp",
                name,
                "server defines both command and url transports",
            );
            continue;
        }
        if obj
            .get("command")
            .and_then(JsonValue::as_str)
            .is_some_and(content_looks_secret)
        {
            report.skipped(
                "mcp",
                name,
                "command appears to contain a credential; configure it manually",
            );
            continue;
        }
        if obj
            .get("type")
            .and_then(JsonValue::as_str)
            .is_some_and(|kind| kind.eq_ignore_ascii_case("sse"))
        {
            report.skipped(
                "mcp",
                name,
                "SSE transport is unsupported; migrate the server to streamable HTTP",
            );
            continue;
        }
        let args = match obj.get("args") {
            None => Vec::new(),
            Some(JsonValue::Array(values)) if values.iter().all(JsonValue::is_string) => values
                .iter()
                .filter_map(JsonValue::as_str)
                .map(str::to_string)
                .collect(),
            Some(_) => {
                report.skipped("mcp", name, "args must be an array of strings");
                continue;
            }
        };
        if arguments_contain_credential(&args) {
            report.skipped("mcp", name, "command arguments appear to contain a credential; configure it through `permagent configure`");
            continue;
        }
        let mut entry = Mapping::new();
        let enabled = obj
            .get("enabled")
            .and_then(JsonValue::as_bool)
            .unwrap_or_else(|| {
                !obj.get("disabled")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false)
            });
        entry.insert("enabled".into(), enabled.into());
        entry.insert("name".into(), name.clone().into());
        entry.insert(
            "description".into(),
            format!("Imported from {origin}").into(),
        );
        entry.insert(
            "timeout".into(),
            obj.get("timeout")
                .and_then(JsonValue::as_u64)
                .unwrap_or(300)
                .into(),
        );
        entry.insert("bundled".into(), false.into());
        entry.insert("available_tools".into(), YamlValue::Sequence(vec![]));
        let mut envs = Mapping::new();
        let mut env_keys = Vec::new();
        if let Some(env) = obj
            .get("env")
            .or_else(|| obj.get("envs"))
            .and_then(JsonValue::as_object)
        {
            for (env_name, value) in env {
                if is_credential_name(env_name) || value.as_str().is_some_and(value_looks_secret) {
                    env_keys.push(YamlValue::String(env_name.clone()));
                    report.skipped(
                        "credential",
                        format!("{name}.{env_name}"),
                        "credential value was not copied; store it through `permagent configure`",
                    );
                } else if let Some(value) = value.as_str() {
                    envs.insert(env_name.clone().into(), value.to_string().into());
                } else {
                    report.skipped(
                        "mcp env",
                        format!("{name}.{env_name}"),
                        "environment value is not a string",
                    );
                }
            }
        } else if obj.get("env").is_some() || obj.get("envs").is_some() {
            report.skipped(
                "mcp env",
                name,
                "environment configuration is not a string map",
            );
        }
        entry.insert("envs".into(), YamlValue::Mapping(envs));
        entry.insert("env_keys".into(), YamlValue::Sequence(env_keys));

        if let Some(command) = obj.get("command").and_then(JsonValue::as_str) {
            entry.insert("type".into(), "stdio".into());
            entry.insert("cmd".into(), command.into());
            entry.insert(
                "args".into(),
                serde_yaml::to_value(args).unwrap_or_default(),
            );
            if let Some(headers) = obj.get("headers").and_then(JsonValue::as_object) {
                for header in headers.keys() {
                    report.skipped(
                        "mcp setting",
                        format!("{name}.headers.{header}"),
                        "headers do not apply to a stdio server",
                    );
                }
            }
        } else if let Some(url) = obj.get("url").and_then(JsonValue::as_str) {
            if url_looks_secret(url) {
                report.skipped("mcp", name, "server URL contains embedded credentials or a secret query; configure it manually");
                continue;
            }
            entry.insert("type".into(), "streamable_http".into());
            entry.insert("uri".into(), url.into());
            entry.insert("socket".into(), YamlValue::Null);
            if !args.is_empty() {
                report.skipped(
                    "mcp setting",
                    format!("{name}.args"),
                    "arguments do not apply to an HTTP server",
                );
            }
            let mut imported_headers = Mapping::new();
            if let Some(headers) = obj.get("headers").and_then(JsonValue::as_object) {
                for (header, value) in headers {
                    if is_credential_name(header) || value.as_str().is_some_and(value_looks_secret)
                    {
                        report.skipped(
                            "credential",
                            format!("{name} header {header}"),
                            "credential header was not copied; configure authentication manually",
                        );
                    } else if let Some(value) = value.as_str() {
                        imported_headers.insert(header.clone().into(), value.to_string().into());
                    } else {
                        report.skipped(
                            "mcp header",
                            format!("{name}.{header}"),
                            "header value is not a string",
                        );
                    }
                }
            } else if obj.get("headers").is_some() {
                report.skipped("mcp header", name, "headers setting is not a map");
            }
            entry.insert("headers".into(), YamlValue::Mapping(imported_headers));
        } else {
            report.skipped(
                "mcp",
                name,
                "only stdio servers with command or HTTP servers with url map cleanly",
            );
            continue;
        }
        extensions.insert(yaml_key, YamlValue::Mapping(entry));
        *changed = true;
        report.imported(
            "mcp",
            name,
            format!(
                "{} {origin} server imported as `{key}`",
                if enabled { "enabled" } else { "disabled" }
            ),
        );
    }
}

fn import_model(
    provider: &str,
    model: &str,
    config: &mut Mapping,
    changed: &mut bool,
    report: &mut ImportReport,
    origin: &str,
) {
    if value_looks_secret(provider) || value_looks_secret(model) {
        report.skipped(
            "model",
            origin,
            "provider/model setting appears to contain credential material",
        );
        return;
    }
    let normalized = match provider.to_ascii_lowercase().as_str() {
        "openai" | "anthropic" | "ollama" | "openrouter" | "databricks" | "azure" => {
            provider.to_ascii_lowercase()
        }
        other => {
            report.skipped(
                "model",
                format!("{provider}/{model}"),
                format!("provider `{other}` has no clean Permagent mapping"),
            );
            return;
        }
    };
    for (key, value) in [
        ("GOOSE_PROVIDER", normalized),
        ("GOOSE_MODEL", model.to_string()),
    ] {
        let yaml_key = YamlValue::String(key.into());
        if let Some(existing) = config.get(&yaml_key) {
            report.skipped(
                "model",
                key,
                format!(
                    "target already has `{}`",
                    existing.as_str().unwrap_or("a configured value")
                ),
            );
        } else {
            config.insert(yaml_key, value.clone().into());
            *changed = true;
            report.imported("model", key, format!("set to `{value}` from {origin}"));
        }
    }
}

fn import_instructions(
    origin: &str,
    candidates: &[PathBuf],
    paths: &ImportPaths,
    config: &mut Mapping,
    changed: &mut bool,
    report: &mut ImportReport,
    dry_run: bool,
) -> Result<bool> {
    let mut seen = HashSet::new();
    let existing: Vec<&PathBuf> = candidates
        .iter()
        .filter(|path| path.is_file() && seen.insert((*path).clone()))
        .collect();
    if existing.is_empty() {
        return Ok(false);
    }
    let config_key = YamlValue::String("GOOSE_SYSTEM_PROMPT_FILE_PATH".into());
    if config.contains_key(&config_key) {
        for path in existing {
            report.skipped(
                "instructions",
                path.display().to_string(),
                "target already configures a custom system prompt file",
            );
        }
        return Ok(true);
    }
    let destination = paths.target.join("imported-instructions.md");
    if destination.exists() {
        for path in existing {
            report.skipped(
                "instructions",
                path.display().to_string(),
                format!("{} already exists", destination.display()),
            );
        }
        return Ok(true);
    }
    let mut body = String::new();
    let mut imported_any = false;
    for path in &existing {
        let content = fs::read_to_string(path)?;
        if content_looks_secret(&content) {
            report.skipped(
                "instructions",
                path.display().to_string(),
                "file appears to contain credential material and was not copied",
            );
            continue;
        }
        body.push_str(&format!("\n<!-- imported from {} -->\n\n", path.display()));
        body.push_str(&content);
        body.push('\n');
        imported_any = true;
        report.imported(
            "instructions",
            path.display().to_string(),
            format!("combined into {}", destination.display()),
        );
    }
    if !imported_any {
        return Ok(true);
    }
    if !dry_run {
        fs::create_dir_all(&paths.target)?;
        fs::write(&destination, body.trim_start())?;
    }
    config.insert(config_key, destination.to_string_lossy().to_string().into());
    *changed = true;
    let _ = origin;
    Ok(true)
}

fn import_skills(
    origin: &str,
    source: &Path,
    target: &Path,
    report: &mut ImportReport,
    dry_run: bool,
) -> Result<bool> {
    if !source.is_dir() {
        return Ok(false);
    }
    let mut found = false;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            report.skipped(
                "skill",
                path.display().to_string(),
                "skills must be directories containing SKILL.md",
            );
            continue;
        }
        found = true;
        if !path.join("SKILL.md").is_file() {
            report.skipped("skill", path.display().to_string(), "missing SKILL.md");
            continue;
        }
        if contains_symlink(&path)? {
            report.skipped(
                "skill",
                path.display().to_string(),
                "skill contains a symbolic link; links are not followed during import",
            );
            continue;
        }
        if directory_contains_credentials(&path)? {
            report.skipped(
                "skill",
                path.display().to_string(),
                "skill appears to contain credential material and was not copied",
            );
            continue;
        }
        let name = sanitize_name(&entry.file_name().to_string_lossy());
        if name.is_empty() {
            report.skipped(
                "skill",
                path.display().to_string(),
                "folder name has no usable letters or digits",
            );
            continue;
        }
        let destination = target.join(&name);
        if destination.exists() {
            report.skipped(
                "skill",
                name,
                "a target skill with this name already exists",
            );
            continue;
        }
        if !dry_run {
            copy_dir_safe(&path, &destination)?;
        }
        report.imported("skill", name, format!("copied from {origin}"));
    }
    Ok(found)
}

fn import_commands(
    origin: &str,
    source: &Path,
    target: &Path,
    report: &mut ImportReport,
    dry_run: bool,
) -> Result<bool> {
    if !source.is_dir() {
        return Ok(false);
    }
    let mut found = false;
    for entry in fs::read_dir(source)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            report.skipped(
                "command",
                path.display().to_string(),
                "only Markdown commands map cleanly to skills",
            );
            continue;
        }
        found = true;
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("command");
        let name = sanitize_name(&format!("{origin}-{stem}"));
        let destination = target.join(&name);
        if destination.exists() {
            report.skipped(
                "command",
                stem,
                "a converted skill with this name already exists",
            );
            continue;
        }
        let content = fs::read_to_string(&path)?;
        if content_looks_secret(&content) {
            report.skipped(
                "command",
                stem,
                "command appears to contain credential material and was not copied",
            );
            continue;
        }
        let skill = format!("---\nname: {name}\ndescription: 'Imported {origin} custom command {stem}'\n---\n\n{content}\n");
        if !dry_run {
            fs::create_dir_all(&destination)?;
            fs::write(destination.join("SKILL.md"), skill)?;
        }
        report.imported("command", stem, format!("converted to skill `{name}`"));
    }
    Ok(found)
}

fn copy_dir_safe(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let destination = target.join(entry.file_name());
        debug_assert!(!ty.is_symlink());
        if ty.is_dir() {
            copy_dir_safe(&entry.path(), &destination)?;
        } else if ty.is_file() {
            fs::copy(entry.path(), destination)?;
        }
    }
    Ok(())
}

fn contains_symlink(directory: &Path) -> Result<bool> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_symlink() || (ty.is_dir() && contains_symlink(&entry.path())?) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn directory_contains_credentials(directory: &Path) -> Result<bool> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() && directory_contains_credentials(&entry.path())? {
            return Ok(true);
        }
        if ty.is_file() {
            let bytes = fs::read(entry.path())?;
            if let Ok(content) = std::str::from_utf8(&bytes) {
                if content_looks_secret(content) {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

fn read_target_config(path: &Path) -> Result<Mapping> {
    if !path.exists() {
        return Ok(Mapping::new());
    }
    let value: YamlValue = serde_yaml::from_str(&fs::read_to_string(path)?)?;
    value
        .as_mapping()
        .cloned()
        .context("Permagent config.yaml root is not a mapping")
}

fn sanitize_name(name: &str) -> String {
    let mut result = String::new();
    let mut dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            result.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !result.is_empty() {
            result.push('-');
            dash = true;
        }
    }
    result.trim_matches('-').to_string()
}

fn is_credential_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    [
        "KEY",
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "CREDENTIAL",
        "AUTH",
        "COOKIE",
    ]
    .iter()
    .any(|word| upper.contains(word))
}

fn value_looks_secret(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with("sk-")
        || lower.starts_with("ghp_")
        || lower.starts_with("github_pat_")
        || lower.starts_with("xox")
        || lower.starts_with("bearer ")
        || lower.contains("-----begin private key-----")
        || lower
            .split_once("://")
            .and_then(|(_, rest)| rest.split('/').next())
            .is_some_and(|authority| authority.contains('@'))
}

fn argument_looks_secret(arg: &str) -> bool {
    value_looks_secret(arg)
        || arg
            .split_once('=')
            .is_some_and(|(key, value)| is_credential_name(key) && !value.starts_with('$'))
}

fn arguments_contain_credential(args: &[String]) -> bool {
    args.iter().enumerate().any(|(index, arg)| {
        argument_looks_secret(arg)
            || (arg.starts_with('-')
                && is_credential_name(arg.trim_start_matches('-'))
                && args
                    .get(index + 1)
                    .is_some_and(|value| !value.starts_with('$')))
    })
}

fn content_looks_secret(content: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim();
        value_looks_secret(trimmed)
            || trimmed
                .split_once('=')
                .is_some_and(|(key, value)| credential_assignment(key, value))
            || trimmed
                .split_once(':')
                .is_some_and(|(key, value)| credential_assignment(key, value))
    })
}

fn credential_assignment(key: &str, value: &str) -> bool {
    let value = value.trim().trim_matches(['\'', '"']);
    is_credential_name(key.trim().trim_matches(['\'', '"']))
        && !value.is_empty()
        && !value.starts_with('$')
        && !value.starts_with('<')
        && !value.contains("YOUR_")
        && !value.eq_ignore_ascii_case("redacted")
}

fn url_looks_secret(url: &str) -> bool {
    url.split_once("://")
        .and_then(|(_, rest)| rest.split('/').next())
        .is_some_and(|authority| authority.contains('@'))
        || url.split_once('?').is_some_and(|(_, query)| {
            query.split('&').any(|pair| {
                pair.split_once('=')
                    .is_some_and(|(key, value)| is_credential_name(key) && !value.starts_with('$'))
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn paths(temp: &TempDir) -> ImportPaths {
        ImportPaths {
            home: temp.path().join("home"),
            project: temp.path().join("project"),
            target: temp.path().join("target"),
        }
    }

    #[test]
    fn imports_claude_setup_and_reports_credential_skip() {
        let temp = TempDir::new().unwrap();
        let p = paths(&temp);
        let source = temp.path().join("claude");
        fs::create_dir_all(source.join("skills/review")).unwrap();
        fs::create_dir_all(source.join("commands")).unwrap();
        fs::write(source.join("CLAUDE.md"), "Always test.").unwrap();
        fs::write(
            source.join("skills/review/SKILL.md"),
            "---\nname: review\n---",
        )
        .unwrap();
        fs::write(source.join("commands/fix.md"), "Fix the issue.").unwrap();
        fs::write(source.join("settings.json"), r#"{
          "model":"claude-sonnet-4-5",
          "mcpServers":{"db":{"command":"npx","args":["db-server"],"env":{"DB_HOST":"localhost","API_TOKEN":"fixture-value"}}}
        }"#).unwrap();

        let report = run_import(ImportSource::Claude, Some(&source), &p, false).unwrap();
        assert!(report
            .items
            .iter()
            .any(|i| i.outcome == Outcome::Skipped && i.category == "credential"));
        assert!(p.target.join("skills/review/SKILL.md").is_file());
        assert!(p.target.join("skills/claude-fix/SKILL.md").is_file());
        let config = fs::read_to_string(p.target.join(CONFIG_FILE)).unwrap();
        assert!(config.contains("DB_HOST"));
        assert!(!config.contains("fixture-value"));
        assert!(config.contains("API_TOKEN"));
        let parsed: YamlValue = serde_yaml::from_str(&config).unwrap();
        let db = parsed
            .get("extensions")
            .and_then(|value| value.get("db"))
            .cloned()
            .unwrap();
        serde_yaml::from_value::<permagent::config::ExtensionEntry>(db).unwrap();
    }

    #[test]
    fn imports_codex_toml_and_does_not_overwrite_target_choices() {
        let temp = TempDir::new().unwrap();
        let p = paths(&temp);
        let source = temp.path().join("codex");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&p.target).unwrap();
        fs::write(p.target.join(CONFIG_FILE), "GOOSE_PROVIDER: anthropic\n").unwrap();
        fs::write(
            source.join("config.toml"),
            r#"
model = "gpt-5.4"
model_provider = "openai"
[mcp_servers.docs]
url = "https://example.test/mcp"
"#,
        )
        .unwrap();

        let report = run_import(ImportSource::Codex, Some(&source), &p, false).unwrap();
        let config = fs::read_to_string(p.target.join(CONFIG_FILE)).unwrap();
        assert!(config.contains("GOOSE_PROVIDER: anthropic"));
        assert!(config.contains("GOOSE_MODEL: gpt-5.4"));
        assert!(config.contains("streamable_http"));
        assert!(report
            .items
            .iter()
            .any(|i| i.outcome == Outcome::Skipped && i.item == "GOOSE_PROVIDER"));
    }

    #[test]
    fn dry_run_reports_but_writes_nothing() {
        let temp = TempDir::new().unwrap();
        let p = paths(&temp);
        let source = temp.path().join("codex");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("config.toml"), "model = \"gpt-5\"").unwrap();
        let report = run_import(ImportSource::Codex, Some(&source), &p, true).unwrap();
        assert!(report.imported > 0);
        assert!(!p.target.exists());
    }

    #[test]
    fn secret_detection_catches_urls_and_arguments() {
        assert!(url_looks_secret("https://u:p@example.test/mcp"));
        assert!(url_looks_secret("https://example.test/mcp?api_key=abc"));
        assert!(argument_looks_secret("--token=abc"));
        assert!(!argument_looks_secret("--token=$MCP_TOKEN"));
        assert!(arguments_contain_credential(&[
            "--api-key".into(),
            "literal".into()
        ]));
    }
}
