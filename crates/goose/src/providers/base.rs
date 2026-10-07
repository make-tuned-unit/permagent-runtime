use anyhow::Result;
use async_trait::async_trait;
use futures::future::BoxFuture;
use futures::Stream;
use serde::{Deserialize, Serialize};

use super::canonical::{map_to_canonical_model, CanonicalModelRegistry};
use super::errors::ProviderError;
use super::inventory::{default_inventory_identity, InventoryIdentityInput};
use super::retry::RetryConfig;
use crate::config::base::ConfigValue;
use crate::config::{Config, ExtensionConfig, GooseMode};
use crate::conversation::message::{Message, MessageContent};
use crate::conversation::Conversation;
use crate::cost_router::cache::SystemPromptParts;
use crate::model::ModelConfig;
use crate::permission::PermissionConfirmation;
use crate::utils::safe_truncate;
use rmcp::model::Tool;
use utoipa::ToSchema;

use once_cell::sync::Lazy;
use regex::Regex;
use std::ops::{Add, AddAssign};
use std::pin::Pin;
use std::sync::LazyLock;
use std::sync::Mutex;

fn strip_xml_tags(text: &str) -> String {
    static BLOCK_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?s)<([a-zA-Z][a-zA-Z0-9_]*)[^>]*>.*?</[a-zA-Z][a-zA-Z0-9_]*>").unwrap()
    });
    static TAG_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"</?[a-zA-Z][a-zA-Z0-9_]*[^>]*>").unwrap());
    let pass1 = BLOCK_RE.replace_all(text, "");
    TAG_RE.replace_all(&pass1, "").into_owned()
}

/// Derive a session title from the first user lines without calling a model.
/// Housekeeping must not pay: titles are local unless there is no usable text.
pub fn local_session_title(messages: &Conversation) -> Option<String> {
    // Join with newlines, not spaces: the title picker reads line structure
    // (headings, a `## Task` section, boilerplate preambles).
    let joined = messages
        .iter()
        .filter(|m| m.role == rmcp::model::Role::User)
        .take(MSG_COUNT_FOR_SESSION_NAME_GENERATION)
        .map(|m| m.as_concat_text())
        .collect::<Vec<_>>()
        .join("\n");
    let stripped = strip_xml_tags(&joined);
    stripped.split_whitespace().next()?;
    Some(safe_truncate(&extract_short_title(&stripped), 100))
}

/// Deterministic title used by the session manager. Empty or attachment-only
/// conversations remain local as well; they do not justify a provider call.
pub fn local_session_title_or_default(messages: &Conversation) -> String {
    local_session_title(messages).unwrap_or_else(|| "New Session".to_string())
}

/// The title of the generic coding-harness recipe. It names the harness, not
/// the task, so sessions running it are titled from their prompt instead.
/// Mirrors `PERMAGENT_CODING_RECIPE_TITLE` in goose-cli's builtin_recipes.rs.
const GENERIC_RECIPE_TITLES: &[&str] = &[crate::tool_inspection::CODING_HARNESS_RECIPE_TITLE];

/// A session's name taken from its recipe, when the recipe title names the
/// job (e.g. "Nightly Health Review"). `None` for a missing, blank, templated
/// or generic (coding-harness) title, so the caller falls back to the prompt.
pub fn recipe_session_title(recipe: Option<&crate::recipe::Recipe>) -> Option<String> {
    let title = recipe?
        .title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty()
        || title.contains("{{")
        || GENERIC_RECIPE_TITLES
            .iter()
            .any(|generic| title.eq_ignore_ascii_case(generic))
    {
        return None;
    }
    Some(safe_truncate(&title, 100))
}

const TITLE_MAX_CHARS: usize = 80;

/// Pick a short title from (possibly long, multi-line) user text.
///
/// Order of preference: the first line of a `## Task` section or a `Goal:`
/// line; otherwise the first line that is not boilerplate (a "You are …"
/// preamble, "Working directory:", a bare heading). Each candidate is cut at
/// its first sentence end or ~80 chars, and one that is mostly punctuation or
/// braces (a code literal) is skipped for the next candidate.
fn extract_short_title(text: &str) -> String {
    let lines: Vec<String> = text
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();

    let mut candidates: Vec<String> = Vec::new();

    // 1. A `## Task` section's lines, up to the next heading.
    if let Some(start) = lines.iter().position(|l| is_task_heading(l)) {
        candidates.extend(
            lines[start + 1..]
                .iter()
                .take_while(|l| !is_heading(l))
                .filter(|l| !l.is_empty())
                .cloned(),
        );
    }
    // 1b. `Goal:` / `Task:` lines (text after the label, else the next line).
    for (i, line) in lines.iter().enumerate() {
        if let Some(rest) = strip_label(line, &["goal:", "task:"]) {
            if !rest.is_empty() {
                candidates.push(rest.to_string());
            } else if let Some(next) = lines[i + 1..].iter().find(|l| !l.is_empty()) {
                if !is_heading(next) {
                    candidates.push(next.clone());
                }
            }
        }
    }
    // 2. Every other non-boilerplate line, in order.
    candidates.extend(
        lines
            .iter()
            .filter(|l| !l.is_empty() && !is_boilerplate_line(l))
            .cloned(),
    );

    for candidate in &candidates {
        let title = shorten_title_line(candidate);
        if is_meaningful_title(&title) {
            return title;
        }
    }

    // Nothing reads like a title: first non-empty line, shortened, so the
    // result is at least stable and bounded.
    lines
        .iter()
        .find(|l| !l.is_empty())
        .map(|l| shorten_title_line(l.trim_start_matches('#').trim()))
        .unwrap_or_default()
}

/// A markdown ATX heading: one or more `#` then a space (or nothing).
fn is_heading(line: &str) -> bool {
    line.starts_with('#') && {
        let rest = line.trim_start_matches('#');
        rest.is_empty() || rest.starts_with(' ')
    }
}

fn is_task_heading(line: &str) -> bool {
    if !is_heading(line) {
        return false;
    }
    let name = line
        .trim_start_matches('#')
        .trim()
        .trim_end_matches(':')
        .trim();
    name.eq_ignore_ascii_case("task")
}

/// `Some(rest)` when `line` starts with one of `labels` (case-insensitive).
fn strip_label<'a>(line: &'a str, labels: &[&str]) -> Option<&'a str> {
    labels.iter().find_map(|label| {
        let head = line.get(..label.len())?;
        let rest = line.get(label.len()..)?;
        head.eq_ignore_ascii_case(label).then(|| rest.trim())
    })
}

fn is_boilerplate_line(line: &str) -> bool {
    let lower = line.to_lowercase();
    is_heading(line)
        || lower.starts_with("you are ")
        || lower.starts_with("working directory:")
        || lower.starts_with("```")
        || line.chars().all(|c| matches!(c, '-' | '=' | '*' | '_'))
}

/// Strip list markers and wrapping quotes, cut at the first sentence end, then
/// cap at ~80 chars on a word boundary with "…".
fn shorten_title_line(line: &str) -> String {
    let mut s = line.trim();
    // List markers: "- ", "* ", "+ ", "1. ", "2) ".
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = s.strip_prefix(marker) {
            s = rest.trim_start();
        }
    }
    if let Some(idx) = s.find(|c: char| !c.is_ascii_digit()) {
        if let Some(tail) = s.get(idx..) {
            if idx > 0 && (tail.starts_with(". ") || tail.starts_with(") ")) {
                s = tail.get(2..).unwrap_or("").trim_start();
            }
        }
    }
    let s = strip_wrapping_quotes(s);
    let s = cut_at_first_sentence(s);
    cap_title_chars(s, TITLE_MAX_CHARS)
}

fn strip_wrapping_quotes(mut s: &str) -> &str {
    loop {
        let trimmed = s.trim();
        let mut chars = trimmed.chars();
        match (chars.next(), chars.next_back()) {
            (Some(a), Some(b))
                if trimmed.chars().count() >= 2 && a == b && matches!(a, '"' | '\'' | '`') =>
            {
                s = trimmed
                    .get(a.len_utf8()..trimmed.len() - b.len_utf8())
                    .unwrap_or("");
            }
            _ => return trimmed,
        }
    }
}

/// Cut at the first ". ", "! " or "? " that ends a sentence of 2+ words.
/// Skips list numbers ("1."), single-letter initials and common abbreviations.
fn cut_at_first_sentence(s: &str) -> &str {
    const ABBREVIATIONS: &[&str] = &["e.g.", "i.e.", "etc.", "vs.", "mr.", "mrs.", "dr."];
    let bytes = s.as_bytes();
    for (i, c) in s.char_indices() {
        if !matches!(c, '.' | '!' | '?') {
            continue;
        }
        let end = i + 1;
        if end >= bytes.len() || !bytes[end].is_ascii_whitespace() {
            continue;
        }
        let Some(head) = s.get(..end) else {
            continue;
        };
        let last_word = head.split_whitespace().last().unwrap_or("");
        let stem = last_word.trim_end_matches(['.', '!', '?']);
        if c == '.'
            && (ABBREVIATIONS.contains(&last_word.to_lowercase().as_str())
                || stem.chars().count() <= 1
                || stem.chars().all(|ch| ch.is_ascii_digit()))
        {
            continue;
        }
        if head.split_whitespace().count() < 2 {
            continue;
        }
        return if c == '.' {
            s.get(..i).unwrap_or(head)
        } else {
            head
        };
    }
    s
}

fn cap_title_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let prefix: String = s.chars().take(max_chars).collect();
    let cut = match prefix.rfind(char::is_whitespace) {
        Some(idx) if idx > 0 => prefix.get(..idx).unwrap_or(prefix.as_str()),
        _ => prefix.as_str(),
    };
    let cut = cut.trim_end_matches(|c: char| {
        c.is_whitespace() || matches!(c, ',' | ':' | ';' | '-' | '–' | '—' | '→' | '(' | '[')
    });
    format!("{cut}…")
}

/// A title must start with a letter or digit, carry some letters, and be
/// mostly alphanumeric — never a brace/punctuation-heavy code literal.
fn is_meaningful_title(title: &str) -> bool {
    let Some(first) = title.chars().next() else {
        return false;
    };
    if !first.is_alphanumeric() {
        return false;
    }
    let letters = title.chars().filter(|c| c.is_alphabetic()).count();
    let visible = title.chars().filter(|c| !c.is_whitespace()).count();
    let alnum = title.chars().filter(|c| c.is_alphanumeric()).count();
    letters >= 2 && alnum * 10 >= visible * 6
}

/// A global store for the current model being used, we use this as when a provider returns, it tells us the real model, not an alias
pub static CURRENT_MODEL: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

/// Set the current model in the global store
pub fn set_current_model(model: &str) {
    if let Ok(mut current_model) = CURRENT_MODEL.lock() {
        *current_model = Some(model.to_string());
    }
}

/// Get the current model from the global store, the real model, not an alias
pub fn get_current_model() -> Option<String> {
    CURRENT_MODEL.lock().ok().and_then(|model| model.clone())
}

pub static MSG_COUNT_FOR_SESSION_NAME_GENERATION: usize = 3;

/// Information about a model's capabilities
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct ModelInfo {
    /// The name of the model
    pub name: String,
    /// The maximum context length this model supports
    pub context_limit: usize,
    /// Cost per token for input in USD (optional)
    pub input_token_cost: Option<f64>,
    /// Cost per token for output in USD (optional)
    pub output_token_cost: Option<f64>,
    /// Currency for the costs (default: "$")
    pub currency: Option<String>,
    /// Whether this model supports cache control
    pub supports_cache_control: Option<bool>,
}

impl ModelInfo {
    /// Create a new ModelInfo with just name and context limit
    pub fn new(name: impl Into<String>, context_limit: usize) -> Self {
        Self {
            name: name.into(),
            context_limit,
            input_token_cost: None,
            output_token_cost: None,
            currency: None,
            supports_cache_control: None,
        }
    }

    /// Create a new ModelInfo with cost information (per token)
    pub fn with_cost(
        name: impl Into<String>,
        context_limit: usize,
        input_cost: f64,
        output_cost: f64,
    ) -> Self {
        Self {
            name: name.into(),
            context_limit,
            input_token_cost: Some(input_cost),
            output_token_cost: Some(output_cost),
            currency: Some("$".to_string()),
            supports_cache_control: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum ProviderType {
    Preferred,
    Builtin,
    Declarative,
    Custom,
}

/// Metadata about a provider's configuration requirements and capabilities
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProviderMetadata {
    /// The unique identifier for this provider
    pub name: String,
    /// Display name for the provider in UIs
    pub display_name: String,
    /// Description of the provider's capabilities
    pub description: String,
    /// The default/recommended model for this provider
    pub default_model: String,
    /// A list of currently known models with their capabilities
    pub known_models: Vec<ModelInfo>,
    /// Link to the docs where models can be found
    pub model_doc_link: String,
    /// Required configuration keys
    pub config_keys: Vec<ConfigKey>,
    /// step-by-step instructions for set up providers eg: api key
    #[serde(default)]
    pub setup_steps: Vec<String>,
    /// Hint shown in the model picker when this provider manages its own model selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_selection_hint: Option<String>,
}

impl ProviderMetadata {
    pub fn new(
        name: &str,
        display_name: &str,
        description: &str,
        default_model: &str,
        model_names: Vec<&str>,
        model_doc_link: &str,
        config_keys: Vec<ConfigKey>,
    ) -> Self {
        Self {
            name: name.to_string(),
            display_name: display_name.to_string(),
            description: description.to_string(),
            default_model: default_model.to_string(),
            known_models: model_names
                .iter()
                .map(|&model_name| ModelInfo {
                    name: model_name.to_string(),
                    context_limit: ModelConfig::new_or_fail(model_name)
                        .with_canonical_limits(name)
                        .context_limit(),
                    input_token_cost: None,
                    output_token_cost: None,
                    currency: None,
                    supports_cache_control: None,
                })
                .collect(),
            model_doc_link: model_doc_link.to_string(),
            config_keys,
            setup_steps: vec![],
            model_selection_hint: None,
        }
    }

    pub fn with_models(
        name: &str,
        display_name: &str,
        description: &str,
        default_model: &str,
        models: Vec<ModelInfo>,
        model_doc_link: &str,
        config_keys: Vec<ConfigKey>,
    ) -> Self {
        Self {
            name: name.to_string(),
            display_name: display_name.to_string(),
            description: description.to_string(),
            default_model: default_model.to_string(),
            known_models: models,
            model_doc_link: model_doc_link.to_string(),
            config_keys,
            setup_steps: vec![],
            model_selection_hint: None,
        }
    }

    pub fn empty() -> Self {
        Self {
            name: "".to_string(),
            display_name: "".to_string(),
            description: "".to_string(),
            default_model: "".to_string(),
            known_models: vec![],
            model_doc_link: "".to_string(),
            config_keys: vec![],
            setup_steps: vec![],
            model_selection_hint: None,
        }
    }

    pub fn with_setup_steps(mut self, steps: Vec<&str>) -> Self {
        self.setup_steps = steps.into_iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn with_model_selection_hint(mut self, hint: &str) -> Self {
        self.model_selection_hint = Some(hint.to_string());
        self
    }
}

/// Configuration key metadata for provider setup
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConfigKey {
    /// The name of the configuration key (e.g., "API_KEY")
    pub name: String,
    /// Whether this key is required for the provider to function
    pub required: bool,
    /// Whether this key should be stored securely (e.g., in keychain)
    pub secret: bool,
    /// Optional default value for the key
    pub default: Option<String>,
    /// Whether this key should be configured using an OAuth flow
    /// When true, the provider's configure_oauth() method will be called instead of prompting for manual input
    pub oauth_flow: bool,
    /// Whether this OAuth flow uses the device code grant (RFC 8628)
    /// When true, the user must enter a verification code in the browser
    #[serde(default)]
    pub device_code_flow: bool,
    /// Whether this key should be shown prominently during provider setup
    /// (onboarding, settings modal, CLI configure)
    #[serde(default)]
    pub primary: bool,
}

impl ConfigKey {
    /// Create a new ConfigKey
    pub fn new(
        name: &str,
        required: bool,
        secret: bool,
        default: Option<&str>,
        primary: bool,
    ) -> Self {
        Self {
            name: name.to_string(),
            required,
            secret,
            default: default.map(|s| s.to_string()),
            oauth_flow: false,
            device_code_flow: false,
            primary,
        }
    }

    pub fn from_value_type<T: ConfigValue>(required: bool, secret: bool, primary: bool) -> Self {
        Self {
            name: T::KEY.to_string(),
            required,
            secret,
            default: Some(T::DEFAULT.to_string()),
            oauth_flow: false,
            device_code_flow: false,
            primary,
        }
    }

    /// Create a new ConfigKey that uses an OAuth flow for configuration
    ///
    /// This is used for providers that support OAuth authentication instead of manual API key entry.
    /// When oauth_flow is true, the configuration system will call the provider's configure_oauth() method.
    pub fn new_oauth(
        name: &str,
        required: bool,
        secret: bool,
        default: Option<&str>,
        primary: bool,
    ) -> Self {
        Self {
            name: name.to_string(),
            required,
            secret,
            default: default.map(|s| s.to_string()),
            oauth_flow: true,
            device_code_flow: false,
            primary,
        }
    }

    /// Create a new ConfigKey that uses OAuth device code flow (RFC 8628) for configuration
    ///
    /// Similar to new_oauth, but indicates the provider uses the device code grant where the user
    /// must enter a verification code in the browser.
    pub fn new_oauth_device_code(
        name: &str,
        required: bool,
        secret: bool,
        default: Option<&str>,
        primary: bool,
    ) -> Self {
        Self {
            name: name.to_string(),
            required,
            secret,
            default: default.map(|s| s.to_string()),
            oauth_flow: true,
            device_code_flow: true,
            primary,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub model: String,
    pub usage: Usage,
    /// Stable identity for the logical provider invocation that produced this
    /// usage. Provider adapters do not have to know about accounting; the
    /// Agent assigns this once per stream and carries it through retries.
    #[serde(default)]
    pub invocation_id: Option<String>,
    /// Set when the provider stopped this response early and said why (the
    /// Responses API's `incomplete_details.reason`, e.g. `max_output_tokens`).
    /// `None` is an ordinary completion. The agent reads it to tell a turn cut
    /// off by the output cap from one the model chose to end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incomplete_reason: Option<String>,
}

impl ProviderUsage {
    pub fn new(model: String, usage: Usage) -> Self {
        Self {
            model,
            usage,
            invocation_id: None,
            incomplete_reason: None,
        }
    }

    pub fn with_incomplete_reason(mut self, reason: Option<String>) -> Self {
        self.incomplete_reason = reason;
        self
    }

    pub fn with_invocation_id(mut self, invocation_id: impl Into<String>) -> Self {
        self.invocation_id = Some(invocation_id.into());
        self
    }

    /// Ensures this ProviderUsage has token counts, estimating them if necessary
    pub async fn ensure_tokens(
        &mut self,
        system_prompt: &str,
        request_messages: &[Message],
        response: &Message,
        tools: &[Tool],
    ) -> Result<(), ProviderError> {
        crate::providers::usage_estimator::ensure_usage_tokens(
            self,
            system_prompt,
            request_messages,
            response,
            tools,
        )
        .await
        .map_err(|e| ProviderError::ExecutionError(format!("Failed to ensure usage tokens: {}", e)))
    }

    /// Combine this ProviderUsage with another, adding their token counts
    /// Uses the model from this ProviderUsage
    pub fn combine_with(&self, other: &ProviderUsage) -> ProviderUsage {
        ProviderUsage {
            model: self.model.clone(),
            usage: self.usage + other.usage,
            invocation_id: self
                .invocation_id
                .clone()
                .or_else(|| other.invocation_id.clone()),
            incomplete_reason: self
                .incomplete_reason
                .clone()
                .or_else(|| other.incomplete_reason.clone()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, Copy)]
pub struct Usage {
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
    pub total_tokens: Option<i32>,
    pub cache_read_input_tokens: Option<i32>,
    pub cache_write_input_tokens: Option<i32>,
}

fn sum_optionals<T>(a: Option<T>, b: Option<T>) -> Option<T>
where
    T: Add<Output = T> + Default,
{
    match (a, b) {
        (Some(x), Some(y)) => Some(x + y),
        (Some(x), None) => Some(x + T::default()),
        (None, Some(y)) => Some(T::default() + y),
        (None, None) => None,
    }
}

impl Add for Usage {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self::new(
            sum_optionals(self.input_tokens, other.input_tokens),
            sum_optionals(self.output_tokens, other.output_tokens),
            sum_optionals(self.total_tokens, other.total_tokens),
        )
        .with_cache_tokens(
            sum_optionals(self.cache_read_input_tokens, other.cache_read_input_tokens),
            sum_optionals(
                self.cache_write_input_tokens,
                other.cache_write_input_tokens,
            ),
        )
    }
}

impl AddAssign for Usage {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl Usage {
    pub fn new(
        input_tokens: Option<i32>,
        output_tokens: Option<i32>,
        total_tokens: Option<i32>,
    ) -> Self {
        let calculated_total = if total_tokens.is_none() {
            match (input_tokens, output_tokens) {
                (Some(input), Some(output)) => Some(input + output),
                (Some(input), None) => Some(input),
                (None, Some(output)) => Some(output),
                (None, None) => None,
            }
        } else {
            total_tokens
        };

        Self {
            input_tokens,
            output_tokens,
            total_tokens: calculated_total,
            cache_read_input_tokens: None,
            cache_write_input_tokens: None,
        }
    }

    pub fn with_cache_tokens(
        mut self,
        cache_read_input_tokens: Option<i32>,
        cache_write_input_tokens: Option<i32>,
    ) -> Self {
        self.cache_read_input_tokens = cache_read_input_tokens;
        self.cache_write_input_tokens = cache_write_input_tokens;
        self
    }
}

pub trait ProviderDef: Send + Sync {
    type Provider: Provider + 'static;

    fn metadata() -> ProviderMetadata
    where
        Self: Sized;

    fn from_env(
        model: ModelConfig,
        extensions: Vec<ExtensionConfig>,
    ) -> BoxFuture<'static, Result<Self::Provider>>
    where
        Self: Sized;

    fn supports_inventory_refresh() -> bool
    where
        Self: Sized,
    {
        false
    }

    fn inventory_identity() -> Result<InventoryIdentityInput>
    where
        Self: Sized,
    {
        let metadata = Self::metadata();
        Ok(default_inventory_identity(
            &metadata.name,
            &metadata.name,
            &metadata.config_keys,
            Config::global(),
        ))
    }

    fn inventory_configured() -> bool
    where
        Self: Sized,
    {
        let metadata = Self::metadata();
        super::inventory::default_inventory_configured(&metadata.config_keys, Config::global())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionRouting {
    ActionRequired,
    Noop,
}

/// Base trait for AI providers (OpenAI, Anthropic, etc)
#[async_trait]
pub trait Provider: Send + Sync {
    /// Opt in only for source-audited request adapters whose physical operations
    /// await ProviderRetry inside the native gate scope. Custom/spawned adapters
    /// are ineligible until they carry and re-enter that required scope.
    fn supports_native_pretransport_gate(&self) -> bool {
        false
    }

    /// Get the name of this provider instance
    fn get_name(&self) -> &str;

    /// Billing classification for calls through this provider.
    ///
    /// This is independent of [`data_locality`](Provider::data_locality):
    /// locality does not prove that an endpoint is free, and a subscription
    /// service is not local. Unknown providers fail closed as chargeable.
    fn cost_tier(&self) -> crate::session::CostTier {
        crate::session::CostTier::PaidApi
    }

    /// Primary streaming method that all providers must implement.
    ///
    /// Note: Do not add `#[instrument]` here — the call sites (`complete` and
    /// `stream_response_from_provider`) create the telemetry span so that
    /// `session.id` is set once rather than in every provider.
    async fn stream(
        &self,
        model_config: &ModelConfig,
        session_id: &str,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError>;

    /// Stream with the system prompt split into its byte-stable prefix and its
    /// turn-volatile suffix, so a provider that supports prompt caching can place
    /// its `cache_control` breakpoint on the prefix alone.
    ///
    /// Additive on purpose: the default flattens the split and calls [`stream`],
    /// so a provider that ignores it is byte-for-byte unchanged. Overriding it
    /// without prompt-cache support would be pure ceremony, and changing
    /// [`stream`]'s signature would touch ~40 provider impls for a property only
    /// a handful of them can act on.
    ///
    /// [`stream`]: Provider::stream
    async fn stream_split(
        &self,
        model_config: &ModelConfig,
        session_id: &str,
        system: &SystemPromptParts,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        self.stream(model_config, session_id, &system.render(), messages, tools)
            .await
    }

    /// Complete with a specific model config.
    #[tracing::instrument(
        skip(self, model_config, session_id, system, messages, tools),
        fields(session.id = %session_id, gen_ai.request.model = %model_config.model_name)
    )]
    async fn complete(
        &self,
        model_config: &ModelConfig,
        session_id: &str,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let stream = self
            .stream(model_config, session_id, system, messages, tools)
            .await?;
        collect_stream(stream).await
    }

    /// [`Self::complete`] with the system prompt split into its byte-stable
    /// prefix and its turn-volatile suffix — the non-streaming counterpart of
    /// [`Self::stream_split`], for callers that go through
    /// `AccountedFastCompletion` (`agents::reply_parts`) rather than the
    /// interactive turn loop.
    ///
    /// 2026-09-25 DAG K cost fix (HARNESS_COST_TRACE.md fix 8): this method
    /// exists so a caller that has separated live/volatile content out of
    /// its system prompt CAN get a real prompt-cache breakpoint on the
    /// stable prefix instead of re-merging everything into one blob (a
    /// single byte of volatile content anywhere in that blob invalidates the
    /// cached read for everything after it — the pattern that would show up
    /// as `cost_ledger.cache_read_tokens` staying flat while `input_tokens`
    /// keeps growing turn over turn).
    ///
    /// NOT YET WIRED: no caller invokes `complete_split` (or the provider
    /// override, e.g. `AnthropicProvider::complete_split`) today. Every
    /// `AccountedFastCompletion` call (the planner, the independent
    /// reviewer, the conversation summarizer) still goes through
    /// `agents::reply_parts::AccountedFastCompletion::complete_one` /
    /// `complete_fast_inner`, which call [`Self::complete`] directly with a
    /// flat `system: &str`. So the cache-invalidation regression described
    /// above is NOT fixed by this method's existence alone — it is
    /// currently dead code from those callers' point of view, exercised
    /// only by its own unit tests and by any provider override.
    ///
    /// TODO(DAG K): wire `AccountedFastCompletion::complete_one` /
    /// `complete_fast_inner` (`crates/goose/src/agents/reply_parts.rs`) to
    /// build a `SystemPromptParts` and call `complete_split` instead of
    /// `complete`, so the planner/reviewer/summarizer callers above actually
    /// benefit. Measure the effect on `cost_ledger.cache_read_tokens` before
    /// and after wiring it — this is a behavior change to cost accounting,
    /// not just plumbing, and should not land right before a release without
    /// that measurement.
    ///
    /// Additive on purpose, exactly like [`Self::stream_split`]: the default
    /// flattens the split via [`SystemPromptParts::render`] and calls
    /// [`Self::complete`], so a provider that does not override it is
    /// byte-for-byte unchanged.
    async fn complete_split(
        &self,
        model_config: &ModelConfig,
        session_id: &str,
        system: &SystemPromptParts,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.complete(model_config, session_id, &system.render(), messages, tools)
            .await
    }

    /// Try fast model first, fall back to regular model on failure.
    async fn complete_fast(
        &self,
        session_id: &str,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let model_config = self.get_model_config();
        let fast_config = model_config.use_fast_model();

        let result = self
            // permagent-dispatch: seam=provider_base_fast_transport_v1 class=excluded reason=caller_owned_fast_accounting authority=accounted_fast_completion
            .complete(&fast_config, session_id, system, messages, tools)
            .await;

        match result {
            Ok(response) => Ok(response),
            Err(e) => {
                if fast_config.model_name != model_config.model_name {
                    tracing::warn!(
                        "Fast model {} failed with error: {}. Falling back to regular model {}",
                        fast_config.model_name,
                        e,
                        model_config.model_name
                    );
                    // permagent-dispatch: seam=provider_base_fast_fallback_transport_v1 class=excluded reason=caller_owned_fast_accounting authority=accounted_fast_completion
                    self.complete(&model_config, session_id, system, messages, tools)
                        .await
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Get the model config from the provider
    fn get_model_config(&self) -> ModelConfig;

    /// Where this provider's inference physically runs — the load-bearing
    /// local-vs-cloud distinction used by the sovereignty guard.
    ///
    /// **Fail-closed default: [`DataLocality::Cloud`].** Any provider that does
    /// not *prove* it runs on this machine is treated as cloud egress, so a new
    /// or unknown provider can never silently be treated as local. Local
    /// providers (in-process inference; a loopback Ollama) override this to
    /// [`DataLocality::Local`].
    fn data_locality(&self) -> crate::sovereignty::DataLocality {
        crate::sovereignty::DataLocality::Cloud
    }

    fn retry_config(&self) -> RetryConfig {
        RetryConfig::default()
    }

    async fn fetch_supported_models(&self) -> Result<Vec<String>, ProviderError> {
        Ok(vec![])
    }

    fn skip_canonical_filtering(&self) -> bool {
        false
    }

    /// Fetch inventory models filtered by canonical registry and usability.
    async fn fetch_recommended_models(&self) -> Result<Vec<String>, ProviderError> {
        let all_models = self.fetch_supported_models().await?;

        if self.skip_canonical_filtering() {
            return Ok(all_models);
        }

        let registry = CanonicalModelRegistry::bundled().map_err(|e| {
            ProviderError::ExecutionError(format!("Failed to load canonical registry: {}", e))
        })?;

        let provider_name = self.get_name();

        // Get all text-capable models with their release dates
        let mut models_with_dates: Vec<(String, Option<String>)> = all_models
            .iter()
            .filter_map(|model| {
                let canonical_id = map_to_canonical_model(provider_name, model, registry)?;

                let (provider, model_name) = canonical_id.split_once('/')?;
                let canonical_model = registry.get(provider, model_name)?;

                if !canonical_model
                    .modalities
                    .input
                    .contains(&crate::providers::canonical::Modality::Text)
                {
                    return None;
                }

                if !canonical_model.tool_call && !self.get_model_config().toolshim {
                    return None;
                }

                let release_date = canonical_model.release_date.clone();

                Some((model.clone(), release_date))
            })
            .collect();

        // Sort by release date (most recent first), then alphabetically for models without dates
        models_with_dates.sort_by(|a, b| match (&a.1, &b.1) {
            (Some(date_a), Some(date_b)) => date_b.cmp(date_a),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.0.cmp(&b.0),
        });

        let inventory_models: Vec<String> = models_with_dates
            .into_iter()
            .map(|(name, _)| name)
            .collect();

        if inventory_models.is_empty() {
            Ok(all_models)
        } else {
            Ok(inventory_models)
        }
    }

    async fn map_to_canonical_model(
        &self,
        provider_model: &str,
    ) -> Result<Option<String>, ProviderError> {
        let registry = CanonicalModelRegistry::bundled().map_err(|e| {
            ProviderError::ExecutionError(format!("Failed to load canonical registry: {}", e))
        })?;

        Ok(map_to_canonical_model(
            self.get_name(),
            provider_model,
            registry,
        ))
    }

    fn supports_embeddings(&self) -> bool {
        false
    }

    /// Whether the provider manages its own conversation context (e.g. CLI
    /// wrappers like Claude Code or Gemini CLI). When true, goose-side
    /// context management such as tool-pair summarization is skipped because
    /// the provider's internal state is the source of truth.
    fn manages_own_context(&self) -> bool {
        false
    }

    async fn supports_cache_control(&self) -> bool {
        false
    }

    /// The context window, in tokens, this provider will really run
    /// `model_config`'s requests in, when it can know that for certain before
    /// sending — a local server whose loaded window is fixed (Ollama reads it
    /// from `/api/show`). `None` when it cannot tell, which is every cloud
    /// provider: their overflow is a normal 400 that compaction answers.
    ///
    /// Routing reads this to keep a goal off a local model whose window
    /// cannot hold the worker's first request. Decorators must forward it.
    async fn request_context_window(&self, _model_config: &ModelConfig) -> Option<usize> {
        None
    }

    /// Refuse, before anything is sent, a request this provider already
    /// knows it cannot serve whole — the same `ContextLengthExceeded` its
    /// `stream` would return for the same request. `Ok(())` when it fits or
    /// the provider cannot tell (the default).
    ///
    /// A dispatched worker runs this on its first request so a refusal ends
    /// the attempt at once with a classified `context_exceeded`, which the
    /// orchestrator's local→cloud fallback acts on, instead of the agent loop
    /// compacting a conversation that has nothing in it yet. Decorators must
    /// forward it.
    async fn preflight_request(
        &self,
        _model_config: &ModelConfig,
        _system: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(), ProviderError> {
        Ok(())
    }

    /// Create embeddings if supported. Default implementation returns an error.
    async fn create_embeddings(
        &self,
        _session_id: &str,
        _texts: Vec<String>,
    ) -> Result<Vec<Vec<f32>>, ProviderError> {
        Err(ProviderError::ExecutionError(
            "This provider does not support embeddings".to_string(),
        ))
    }

    /// Returns the first 3 user messages as strings for session naming
    fn get_initial_user_messages(&self, messages: &Conversation) -> Vec<String> {
        messages
            .iter()
            .filter(|m| m.role == rmcp::model::Role::User)
            .take(MSG_COUNT_FOR_SESSION_NAME_GENERATION)
            .map(|m| m.as_concat_text())
            .collect()
    }

    /// Generate a session name from conversation history without model spend.
    ///
    /// Session naming is housekeeping, not user-requested inference. Keeping it
    /// local prevents an empty/attachment-only first turn from bypassing the
    /// task budget through a background `complete_fast` call.
    async fn generate_session_name(
        &self,
        _session_id: &str,
        messages: &Conversation,
    ) -> Result<String, ProviderError> {
        Ok(local_session_title_or_default(messages))
    }

    /// Configure OAuth authentication for this provider
    ///
    /// This method is called when a provider has configuration keys marked with oauth_flow = true.
    /// Providers that support OAuth should override this method to implement their specific OAuth flow.
    ///
    /// # Returns
    /// * `Ok(())` if OAuth configuration succeeds and credentials are saved
    /// * `Err(ProviderError)` if OAuth fails or is not supported by this provider
    ///
    /// # Default Implementation
    /// The default implementation returns an error indicating OAuth is not supported.
    async fn configure_oauth(&self) -> Result<(), ProviderError> {
        Err(ProviderError::ExecutionError(
            "OAuth configuration not supported by this provider".to_string(),
        ))
    }

    async fn refresh_credentials(&self) -> Result<(), ProviderError> {
        Err(ProviderError::NotImplemented(
            "credential refresh not supported by this provider".to_string(),
        ))
    }

    async fn update_mode(&self, _session_id: &str, _mode: GooseMode) -> Result<(), ProviderError> {
        Ok(())
    }

    fn permission_routing(&self) -> PermissionRouting {
        PermissionRouting::Noop
    }

    async fn handle_permission_confirmation(
        &self,
        _request_id: &str,
        _confirmation: &PermissionConfirmation,
    ) -> bool {
        false
    }
}

/// A message stream yields partial text content but complete tool calls, all within the Message object
/// So a message with text will contain potentially just a word of a longer response, but tool calls
/// messages will only be yielded once concatenated.
pub type MessageStream = Pin<
    Box<dyn Stream<Item = Result<(Option<Message>, Option<ProviderUsage>), ProviderError>> + Send>,
>;

pub fn stream_from_single_message(message: Message, usage: ProviderUsage) -> MessageStream {
    let stream = futures::stream::once(async move { Ok((Some(message), Some(usage))) });
    Box::pin(stream)
}

/// Collect all chunks from a MessageStream into a single Message and ProviderUsage
pub async fn collect_stream(
    mut stream: MessageStream,
) -> Result<(Message, ProviderUsage), ProviderError> {
    use futures::StreamExt;

    let mut final_message: Option<Message> = None;
    let mut final_usage: Option<ProviderUsage> = None;

    while let Some(result) = stream.next().await {
        let (msg_opt, usage_opt) = result?;

        if let Some(msg) = msg_opt {
            final_message = Some(match final_message {
                Some(mut prev) => {
                    for new_content in msg.content {
                        match (&mut prev.content.last_mut(), &new_content) {
                            // Coalesce consecutive text blocks
                            (
                                Some(MessageContent::Text(last_text)),
                                MessageContent::Text(new_text),
                            ) => {
                                last_text.text.push_str(&new_text.text);
                            }
                            _ => {
                                prev.content.push(new_content);
                            }
                        }
                    }
                    prev
                }
                None => msg,
            });
        }

        if let Some(usage) = usage_opt {
            final_usage = Some(usage);
        }
    }

    match final_message {
        Some(msg) => {
            let usage = final_usage
                .unwrap_or_else(|| ProviderUsage::new("unknown".to_string(), Usage::default()));
            Ok((msg, usage))
        }
        None => Err(ProviderError::ExecutionError(
            "Stream yielded no message".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use test_case::test_case;

    use serde_json::json;

    #[test]
    fn test_strip_xml_tags() {
        assert_eq!(strip_xml_tags("<think>reasoning</think>answer"), "answer");
        assert_eq!(strip_xml_tags("before<t>mid</t>after"), "beforeafter");
        assert_eq!(strip_xml_tags("<a>x</a><b>y</b>z"), "z");
        assert_eq!(strip_xml_tags("no tags here"), "no tags here");
        assert_eq!(strip_xml_tags("a < b > c"), "a < b > c");
        assert_eq!(strip_xml_tags("<think>über</think>ok"), "ok");
        assert_eq!(strip_xml_tags("<think>日本語</think>hello"), "hello");
        assert_eq!(strip_xml_tags(""), "");
        assert_eq!(strip_xml_tags("<>stuff</>"), "<>stuff</>");
        // attributes
        assert_eq!(
            strip_xml_tags(r#"<think class="deep">reasoning</think>answer"#),
            "answer"
        );
        // self-closing tags
        assert_eq!(strip_xml_tags("<br/>self closing"), "self closing");
        // orphan closing tags
        assert_eq!(strip_xml_tags("orphan </think> tag"), "orphan  tag");
        // multiline content
        assert_eq!(
            strip_xml_tags("<think>\nline1\nline2\n</think>result"),
            "result"
        );
    }

    #[test]
    fn local_session_title_from_first_user_lines() {
        let conv = Conversation::new_unvalidated(vec![
            Message::user().with_text("fix the login bug please and also")
        ]);
        let title = local_session_title(&conv).expect("user text must yield a local title");
        assert!(title.to_lowercase().contains("login"));
        assert!(title.len() <= 100);
        assert!(!title.contains('<'), "xml must be stripped: {title}");
    }

    #[test]
    fn local_session_title_none_when_user_text_empty() {
        let conv = Conversation::new_unvalidated(vec![Message::user().with_text("   ")]);
        assert_eq!(local_session_title(&conv), None);
        assert_eq!(local_session_title_or_default(&conv), "New Session");
    }

    #[test]
    fn session_name_generation_has_no_provider_dispatch_fallback() {
        let source = include_str!("base.rs");
        let method = source
            .split("async fn generate_session_name")
            .nth(1)
            .and_then(|tail| tail.split("/// Configure OAuth authentication").next())
            .expect("generate_session_name method body");
        assert!(
            !method.contains(".complete("),
            "session naming dispatched a full model"
        );
        assert!(
            !method.contains(".complete_fast("),
            "session naming dispatched a fast model"
        );
        assert!(method.contains("local_session_title_or_default"));
    }

    #[test]
    fn local_session_title_strips_xml_and_truncates() {
        let conv = Conversation::new_unvalidated(vec![Message::user().with_text(
            "please <think>ignore this</think> rewrite the authentication middleware for the public api",
        )]);
        let title = local_session_title(&conv).expect("stripped xml still has words");
        assert!(!title.contains('<'), "{title}");
        assert!(!title.contains("think"), "{title}");
        assert!(title.len() <= 100);
    }

    #[test]
    fn test_extract_short_title() {
        // Short plain text stays as-is.
        assert_eq!(extract_short_title("List files"), "List files");
        assert_eq!(extract_short_title("Hello world"), "Hello world");
        // Line structure is kept: the first real line wins, not the last.
        assert_eq!(
            extract_short_title("List files in folder\nand then some more words here"),
            "List files in folder"
        );
        // Wrapping quotes/backticks are stripped.
        assert_eq!(
            extract_short_title("`View current files`"),
            "View current files"
        );
        assert_eq!(
            extract_short_title("\"List folder files\""),
            "List folder files"
        );
        // Quoted spans inside long text are NOT picked as the title.
        assert_eq!(
            extract_short_title(
                r#"Summarise the release notes and mention "Abc title" and "Zzz title" at the end"#
            ),
            "Summarise the release notes and mention \"Abc title\" and \"Zzz title\" at the end"
        );
        // Cut at the first sentence end.
        assert_eq!(
            extract_short_title("Rename the build step. Then rerun the tests and report back."),
            "Rename the build step"
        );
        // List numbers and abbreviations are not sentence ends.
        assert_eq!(
            extract_short_title("1. Compare e.g. two routers"),
            "Compare e.g. two routers"
        );
        // Over ~80 chars: cut on a word boundary with an ellipsis.
        let long = "word ".repeat(40);
        let title = extract_short_title(&long);
        assert!(title.ends_with('…'), "{title}");
        assert!(title.chars().count() <= TITLE_MAX_CHARS + 1, "{title}");
        // Boilerplate preambles and bare headings are skipped.
        assert_eq!(
            extract_short_title(
                "You are a helpful agent.\n# Heading\nWorking directory: /tmp\nDo the thing"
            ),
            "Do the thing"
        );
        // A `Goal:` line wins over earlier prose.
        assert_eq!(
            extract_short_title("Some context first\nGoal: ship the watch app"),
            "ship the watch app"
        );
        // A punctuation/brace-heavy line is never the title.
        assert_eq!(
            extract_short_title("{name: 'x', depth: 25|50}\nAdd scroll depth tracking"),
            "Add scroll depth tracking"
        );
        // A markdown heading as the only line still yields readable text.
        assert_eq!(extract_short_title("# Fix login"), "Fix login");
    }

    /// (a) The Grow coding-agent prompt shape (codingAgentDirective.ts): a
    /// "You are …" preamble, a working directory, then a `## Task` section, with
    /// a code literal in a later bullet that the old heuristic picked.
    #[test]
    fn local_session_title_uses_task_section_of_grow_prompt() {
        let prompt = "You are a coding agent working in the repository for \"example.com\".\n\
Working directory: /Users/me/dev/example\n\
Implement the growth action below in this repo. Do not stop at drafting copy in chat — write the change into the right files.\n\
\n\
## Task\n\
Fix event_cta_click funnel leak: 3,836 impressions → 940 clicks (24.5% CTR) suggests either CTA placement or copy issues\n\
Category: conversion\n\
\n\
## What to do\n\
Move the CTA above the fold and test new copy.\n\
\n\
## Steps\n\
1. Add a scroll-depth event.\n\
- Track `{name: 'scroll_depth', depth: 25|50|75|100, event_id: '[event-id]'}` on the event page\n\
\n\
## Deliverable\n\
Commit the change.";
        let conv = Conversation::new_unvalidated(vec![Message::user().with_text(prompt)]);
        let title = local_session_title(&conv).expect("grow prompt yields a title");
        assert!(
            title.starts_with("Fix event_cta_click funnel leak: 3,836 impressions → 940 clicks"),
            "{title}"
        );
        assert!(title.ends_with('…'), "{title}");
        assert!(!title.contains('{'), "{title}");
        assert!(!title.contains("scroll_depth"), "{title}");
    }

    /// (b) A health-review-like prompt with a quoted phrase deep inside: the
    /// title comes from the opening sentence, not the quoted phrase.
    #[test]
    fn local_session_title_ignores_quoted_phrase_in_health_review_prompt() {
        let prompt = "Review Permagent's own logs for the exact last 24 hours. This is a\n\
diagnostic job, not a free-form log hunt: the bundled evidence collector owns\n\
file enumeration, timestamps, mirror de-duplication, aggregation, and latency\n\
pairing.\n\
\n\
## Diagnostic rules\n\
\n\
- Session 20260827_1's 129 warnings were the sanitizer dropping Top-of-Mind\n\
  because `\"Merged text content\"` was treated as unexpected.\n";
        let conv = Conversation::new_unvalidated(vec![Message::user().with_text(prompt)]);
        let title = local_session_title(&conv).expect("health prompt yields a title");
        assert_eq!(
            title,
            "Review Permagent's own logs for the exact last 24 hours"
        );
        assert!(!title.contains("Merged text content"), "{title}");
    }

    /// (c) A short plain chat message stays as typed.
    #[test]
    fn local_session_title_keeps_short_chat_message() {
        let conv = Conversation::new_unvalidated(vec![
            Message::user().with_text("what's the weather in Lisbon today")
        ]);
        assert_eq!(
            local_session_title(&conv).as_deref(),
            Some("what's the weather in Lisbon today")
        );
    }

    /// (d) A specific recipe title names the session; the generic coding
    /// harness title does not, so those sessions fall back to the prompt.
    #[test]
    fn recipe_session_title_uses_specific_titles_only() {
        let recipe = |title: &str| {
            crate::recipe::Recipe::builder()
                .version("1.0.0")
                .title(title)
                .description("d")
                .prompt("p")
                .build()
                .unwrap()
        };
        assert_eq!(
            recipe_session_title(Some(&recipe("Nightly Health Review"))).as_deref(),
            Some("Nightly Health Review")
        );
        assert_eq!(
            recipe_session_title(Some(&recipe(
                crate::tool_inspection::CODING_HARNESS_RECIPE_TITLE
            ))),
            None
        );
        assert_eq!(recipe_session_title(Some(&recipe("   "))), None);
        assert_eq!(
            recipe_session_title(Some(&recipe("Review {{ name }}"))),
            None
        );
        assert_eq!(recipe_session_title(None), None);
    }

    #[test]
    fn test_usage_creation() {
        let usage = Usage::new(Some(10), Some(20), Some(30));
        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.output_tokens, Some(20));
        assert_eq!(usage.total_tokens, Some(30));
    }

    fn content_from_str(s: String) -> MessageContent {
        if let Some(img_data) = s.strip_prefix("*img:") {
            MessageContent::image(format!("http://example.com/{}", img_data), "image/png")
        } else if let Some(tool_name) = s.strip_prefix("*tool:") {
            let tool_call = Ok(
                rmcp::model::CallToolRequestParams::new(tool_name.to_string())
                    .with_arguments(serde_json::Map::new()),
            );
            MessageContent::tool_request(format!("tool_{}", tool_name), tool_call)
        } else {
            MessageContent::text(s)
        }
    }

    fn create_test_stream(
        items: Vec<String>,
    ) -> impl Stream<Item = Result<(Option<Message>, Option<ProviderUsage>), ProviderError>> {
        use futures::stream;
        stream::iter(items.into_iter().map(|item| {
            let content = content_from_str(item);
            let message = Message::new(
                rmcp::model::Role::Assistant,
                chrono::Utc::now().timestamp(),
                vec![content],
            );
            Ok((Some(message), None))
        }))
    }

    fn content_to_strings(msg: &Message) -> Vec<String> {
        msg.content
            .iter()
            .map(|c| match c {
                MessageContent::Text(t) => t.text.clone(),
                MessageContent::Image(_) => "*img".to_string(),
                MessageContent::ToolRequest(tr) => {
                    if let Ok(call) = &tr.tool_call {
                        format!("*tool:{}", call.name)
                    } else {
                        "*tool:error".to_string()
                    }
                }
                _ => "*other".to_string(),
            })
            .collect()
    }

    #[test_case(
        vec!["Hello", " ", "world"],
        vec!["Hello world"]
        ; "consecutive text coalesces"
    )]
    #[test_case(
        vec!["Hello", "*img:pic1", "world"],
        vec!["Hello", "*img", "world"]
        ; "non-text breaks coalescing"
    )]
    #[test_case(
        vec!["A", "B", "*img:pic1", "C", "D", "*tool:read", "E", "F"],
        vec!["AB", "*img", "CD", "*tool:read", "EF"]
        ; "multiple text groups"
    )]
    #[test_case(
        vec!["Text1", "*img:pic", "Text2"],
        vec!["Text1", "*img", "Text2"]
        ; "mixed content in chunk"
    )]
    #[tokio::test]
    async fn test_collect_stream_coalescing(input_items: Vec<&str>, expected: Vec<&str>) {
        let items: Vec<String> = input_items.into_iter().map(|s| s.to_string()).collect();
        let stream = create_test_stream(items);
        let (msg, _) = collect_stream(Box::pin(stream)).await.unwrap();
        assert_eq!(content_to_strings(&msg), expected);
    }

    #[tokio::test]
    async fn test_collect_stream_defaults_usage() {
        let stream = create_test_stream(vec!["Hello".to_string()]);
        let (msg, usage) = collect_stream(Box::pin(stream)).await.unwrap();
        assert_eq!(content_to_strings(&msg), vec!["Hello"]);
        assert_eq!(usage.model, "unknown");
    }

    #[test]
    fn test_usage_serialization() -> Result<()> {
        let usage = Usage::new(Some(10), Some(20), Some(30));
        let serialized = serde_json::to_string(&usage)?;
        let deserialized: Usage = serde_json::from_str(&serialized)?;

        assert_eq!(usage.input_tokens, deserialized.input_tokens);
        assert_eq!(usage.output_tokens, deserialized.output_tokens);
        assert_eq!(usage.total_tokens, deserialized.total_tokens);

        // Test JSON structure
        let json_value: serde_json::Value = serde_json::from_str(&serialized)?;
        assert_eq!(json_value["input_tokens"], json!(10));
        assert_eq!(json_value["output_tokens"], json!(20));
        assert_eq!(json_value["total_tokens"], json!(30));

        Ok(())
    }

    #[test]
    fn test_set_and_get_current_model() {
        // Set the model
        set_current_model("gpt-4o");

        // Get the model and verify
        let model = get_current_model();
        assert_eq!(model, Some("gpt-4o".to_string()));

        // Change the model
        set_current_model("claude-sonnet-4-20250514");

        // Get the updated model and verify
        let model = get_current_model();
        assert_eq!(model, Some("claude-sonnet-4-20250514".to_string()));
    }

    #[test]
    fn test_provider_metadata_context_limits() {
        // Test that ProviderMetadata::new correctly sets context limits
        let test_models = vec!["gpt-4o", "claude-sonnet-4-20250514", "unknown-model"];
        let metadata = ProviderMetadata::new(
            "test",
            "Test Provider",
            "Test Description",
            "gpt-4o",
            test_models,
            "https://example.com",
            vec![],
        );

        let model_info: HashMap<String, usize> = metadata
            .known_models
            .into_iter()
            .map(|m| (m.name, m.context_limit))
            .collect();

        // gpt-4o should have 128k limit
        assert_eq!(*model_info.get("gpt-4o").unwrap(), 128_000);

        // claude-sonnet-4-20250514 should have 200k limit
        assert_eq!(
            *model_info.get("claude-sonnet-4-20250514").unwrap(),
            200_000
        );

        // unknown model should have default limit (128k)
        assert_eq!(*model_info.get("unknown-model").unwrap(), 128_000);
    }

    #[test]
    fn test_model_info_creation() {
        // Test direct ModelInfo creation
        let info = ModelInfo {
            name: "test-model".to_string(),
            context_limit: 1000,
            input_token_cost: None,
            output_token_cost: None,
            currency: None,
            supports_cache_control: None,
        };
        assert_eq!(info.context_limit, 1000);

        // Test equality
        let info2 = ModelInfo {
            name: "test-model".to_string(),
            context_limit: 1000,
            input_token_cost: None,
            output_token_cost: None,
            currency: None,
            supports_cache_control: None,
        };
        assert_eq!(info, info2);

        // Test inequality
        let info3 = ModelInfo {
            name: "test-model".to_string(),
            context_limit: 2000,
            input_token_cost: None,
            output_token_cost: None,
            currency: None,
            supports_cache_control: None,
        };
        assert_ne!(info, info3);
    }

    #[test]
    fn test_model_info_with_cost() {
        let info = ModelInfo::with_cost("gpt-4o", 128000, 0.0000025, 0.00001);
        assert_eq!(info.name, "gpt-4o");
        assert_eq!(info.context_limit, 128000);
        assert_eq!(info.input_token_cost, Some(0.0000025));
        assert_eq!(info.output_token_cost, Some(0.00001));
        assert_eq!(info.currency, Some("$".to_string()));
    }

    #[test]
    fn test_usage_addition_includes_cached_tokens() {
        let usage_a =
            Usage::new(Some(100), Some(20), Some(120)).with_cache_tokens(Some(10), Some(5));
        let usage_b = Usage::new(Some(50), Some(8), Some(58)).with_cache_tokens(Some(4), Some(1));

        let combined = usage_a + usage_b;

        assert_eq!(combined.input_tokens, Some(150));
        assert_eq!(combined.output_tokens, Some(28));
        assert_eq!(combined.total_tokens, Some(178));
        assert_eq!(combined.cache_read_input_tokens, Some(14));
        assert_eq!(combined.cache_write_input_tokens, Some(6));
    }

    /// A minimal provider that only implements `stream` (every other method
    /// on the trait is the default) and records the exact `system` string it
    /// was handed, so the default `complete_split` -> `complete` -> `stream`
    /// chain can be verified without a real network call.
    struct RecordingProvider {
        seen_system: std::sync::Mutex<Option<String>>,
    }

    #[async_trait]
    impl Provider for RecordingProvider {
        fn get_name(&self) -> &str {
            "recording-fixture"
        }

        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new("test-model").unwrap()
        }

        async fn stream(
            &self,
            _model_config: &ModelConfig,
            _session_id: &str,
            system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<MessageStream, ProviderError> {
            *self.seen_system.lock().unwrap() = Some(system.to_string());
            Ok(stream_from_single_message(
                Message::assistant().with_text("ok"),
                ProviderUsage::new("test-model".to_string(), Usage::default()),
            ))
        }
    }

    /// 2026-09-25 DAG K cost fix (HARNESS_COST_TRACE.md fix 8): a provider
    /// that does not override `complete_split` gets the trait default's
    /// `system.render()` — the stable prefix and volatile suffix
    /// concatenated back into one flat string — proving the additive
    /// contract documented on `Provider::complete_split` (byte-for-byte the
    /// same as calling `complete` with that flat string directly).
    #[tokio::test]
    async fn complete_split_default_flattens_stable_and_volatile_via_render() {
        let provider = RecordingProvider {
            seen_system: std::sync::Mutex::new(None),
        };
        let parts = SystemPromptParts::new(
            "STABLE: persona and tools".to_string(),
            "VOLATILE: 3 jobs scheduled right now".to_string(),
        );
        let expected = parts.render();

        provider
            .complete_split(
                &ModelConfig::new("test-model").unwrap(),
                "session-1",
                &parts,
                &[Message::user().with_text("hi")],
                &[],
            )
            .await
            .unwrap();

        assert_eq!(
            provider.seen_system.lock().unwrap().as_deref(),
            Some(expected.as_str()),
            "complete_split's default must hand the provider system.render(), \
             not the stable prefix alone or the two halves out of order"
        );
    }

    /// A provider IS free to override `complete_split` to keep the split
    /// intact for its own request builder (exactly what `AnthropicProvider`
    /// now does, routing it through `stream_split` instead of flattening) —
    /// this fixture proves the trait allows that override to run instead of
    /// the default, which is the actual fix: a provider's non-streaming path
    /// can place its cache breakpoint on the true stable prefix alone.
    struct SplitAwareProvider {
        seen_stable: std::sync::Mutex<Option<String>>,
        seen_volatile: std::sync::Mutex<Option<String>>,
    }

    #[async_trait]
    impl Provider for SplitAwareProvider {
        fn get_name(&self) -> &str {
            "split-aware-fixture"
        }

        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new("test-model").unwrap()
        }

        async fn stream(
            &self,
            model_config: &ModelConfig,
            session_id: &str,
            system: &str,
            messages: &[Message],
            tools: &[Tool],
        ) -> Result<MessageStream, ProviderError> {
            // A caller with no split to offer: same fallback every provider's
            // `stream` gets (mirrors `AnthropicProvider::stream`).
            self.stream_split(
                model_config,
                session_id,
                &SystemPromptParts::all_stable(system.to_string()),
                messages,
                tools,
            )
            .await
        }

        async fn stream_split(
            &self,
            _model_config: &ModelConfig,
            _session_id: &str,
            system: &SystemPromptParts,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<MessageStream, ProviderError> {
            *self.seen_stable.lock().unwrap() = Some(system.stable_prefix().to_string());
            *self.seen_volatile.lock().unwrap() = Some(system.volatile_suffix().to_string());
            Ok(stream_from_single_message(
                Message::assistant().with_text("ok"),
                ProviderUsage::new("test-model".to_string(), Usage::default()),
            ))
        }

        async fn complete_split(
            &self,
            model_config: &ModelConfig,
            session_id: &str,
            system: &SystemPromptParts,
            messages: &[Message],
            tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            let stream = self
                .stream_split(model_config, session_id, system, messages, tools)
                .await?;
            collect_stream(stream).await
        }
    }

    #[tokio::test]
    async fn complete_split_override_keeps_the_stable_and_volatile_halves_separate() {
        let provider = SplitAwareProvider {
            seen_stable: std::sync::Mutex::new(None),
            seen_volatile: std::sync::Mutex::new(None),
        };
        let parts = SystemPromptParts::new(
            "STABLE: persona and tools".to_string(),
            "VOLATILE: 3 jobs scheduled right now".to_string(),
        );

        provider
            .complete_split(
                &ModelConfig::new("test-model").unwrap(),
                "session-1",
                &parts,
                &[Message::user().with_text("hi")],
                &[],
            )
            .await
            .unwrap();

        assert_eq!(
            provider.seen_stable.lock().unwrap().as_deref(),
            Some("STABLE: persona and tools")
        );
        assert_eq!(
            provider.seen_volatile.lock().unwrap().as_deref(),
            Some("VOLATILE: 3 jobs scheduled right now")
        );
    }
}
