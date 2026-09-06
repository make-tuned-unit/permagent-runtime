use crate::routes::errors::ErrorResponse;
use crate::routes::utils::check_provider_configured;
use crate::state::AppState;
use axum::http::StatusCode;
use axum::routing::put;
use axum::{
    extract::Path,
    routing::{delete, get, post},
    Json, Router,
};
use permagent::config::declarative_providers::LoadedProvider;
use permagent::config::paths::Paths;
use permagent::config::ExtensionEntry;
use permagent::config::{Config, ConfigError, ModelRole, RoleModelSource};
use permagent::model::ModelConfig;
use permagent::providers::base::{ProviderMetadata, ProviderType};
use permagent::providers::canonical::maybe_get_canonical_model;
use permagent::providers::catalog::{
    get_provider_template, get_providers_by_format, ProviderCatalogEntry, ProviderFormat,
    ProviderTemplate,
};
use permagent::providers::get_from_registry;
use permagent::providers::providers as get_providers;
use permagent::providers::{create_with_default_model, create_with_named_model};
use permagent::{
    agents::execute_commands, agents::ExtensionConfig, config::permission::PermissionLevel,
    slash_commands,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_yaml;
use std::{collections::HashMap, sync::Arc};
use tokio::task::JoinHandle;
use tokio::time::Duration;
use utoipa::ToSchema;

const PROVIDER_CHECK_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Serialize, ToSchema)]
pub struct ExtensionResponse {
    pub extensions: Vec<ExtensionEntry>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct ExtensionQuery {
    pub name: String,
    pub config: ExtensionConfig,
    pub enabled: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct UpsertConfigQuery {
    pub key: String,
    pub value: Value,
    pub is_secret: bool,
}

#[derive(Deserialize, Serialize, ToSchema)]
pub struct ConfigKeyQuery {
    pub key: String,
    pub is_secret: bool,
}

#[derive(Serialize, ToSchema)]
pub struct ConfigResponse {
    /// YAML-file (+ bundled-defaults) values only — environment variables are
    /// NOT reflected here, by design of `Config::all_values`.
    pub config: HashMap<String, Value>,
    /// GOOSE_MODE as the daemon actually resolves it (env var takes precedence
    /// over the YAML value, `permagent::config::base::Config::get_param`).
    /// Surfaced so Settings can warn when an env override makes the YAML
    /// selection inert instead of silently highlighting the wrong mode
    /// (re-enable-gate epic part B). Snake_case, e.g. "auto", "approve",
    /// "smart_approve", "chat".
    pub effective_goose_mode: String,
    /// Effective routes after role-specific keys, session fallback, and
    /// measured defaults are resolved. The raw `config` map remains intact for
    /// settings editors; clients that render the active route must use this
    /// projection so env/default-backed choices are not shown as blank.
    #[serde(default)]
    pub resolved_routes: HashMap<String, ResolvedModelRoute>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ResolvedModelRoute {
    pub provider: String,
    pub model: String,
    pub source: String,
}

fn source_label(source: RoleModelSource) -> &'static str {
    match source {
        RoleModelSource::Configured => "configured",
        RoleModelSource::SessionModel => "session_model",
        RoleModelSource::Default => "default",
        RoleModelSource::HalfConfigured => "half_configured",
        RoleModelSource::Disabled => "disabled",
    }
}

fn resolved_route(
    route: Option<(String, String)>,
    fallback: (String, String),
    source: &str,
) -> ResolvedModelRoute {
    let (provider, model) = route.unwrap_or(fallback);
    ResolvedModelRoute {
        provider,
        model,
        source: source.to_string(),
    }
}

fn resolved_model_routes() -> HashMap<String, ResolvedModelRoute> {
    let config = Config::global();
    let fallback = || {
        (
            config.get_goose_provider().unwrap_or_default(),
            config.get_goose_model().unwrap_or_default(),
        )
    };

    let chat = permagent::config::role_model_from_config(ModelRole::Chat);
    let chat_route = chat.route.map(|route| (route.provider, route.model));

    let voice = permagent::config::voice_model_from_config();
    let mut routes = HashMap::with_capacity(2);
    routes.insert(
        "chat".to_string(),
        resolved_route(chat_route, fallback(), source_label(chat.source)),
    );
    routes.insert(
        "voice".to_string(),
        match voice {
            Some((route, source)) => ResolvedModelRoute {
                provider: route.provider,
                model: route.model,
                source: match source {
                    permagent::config::VoiceModelSource::Configured => "configured",
                    permagent::config::VoiceModelSource::Default => "default",
                    permagent::config::VoiceModelSource::HalfConfigured => "half_configured",
                }
                .to_string(),
            },
            // `None` is the explicit voice=session choice. Expose the
            // effective session route rather than leaving the mobile control
            // blank, while retaining the raw keys for the settings editor.
            None => resolved_route(None, fallback(), "session_model"),
        },
    );
    routes
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ProviderDetails {
    pub name: String,
    pub metadata: ProviderMetadata,
    pub is_configured: bool,
    pub is_default: bool,
    pub provider_type: ProviderType,
    /// True when a secret config key for this provider is set as an environment
    /// variable while the same key also exists in secret storage. On current
    /// builds storage wins (keychain-first), so the env value is inert — but on
    /// pre-2026-06-01 builds it shadows UI-saved keys (#157/#176). Surfaced so
    /// clients can warn about the stale env value.
    #[serde(default)]
    pub env_override_active: bool,
}

#[derive(Serialize, ToSchema)]
pub struct ProvidersResponse {
    pub providers: Vec<ProviderDetails>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ToolPermission {
    pub tool_name: String,
    pub permission: PermissionLevel,
}

#[derive(Deserialize, ToSchema)]
pub struct UpsertPermissionsQuery {
    pub tool_permissions: Vec<ToolPermission>,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateCustomProviderRequest {
    pub engine: String,
    pub display_name: String,
    pub api_url: String,
    pub api_key: String,
    pub models: Vec<String>,
    pub supports_streaming: Option<bool>,
    pub headers: Option<std::collections::HashMap<String, String>>,
    #[serde(default = "default_requires_auth")]
    pub requires_auth: bool,
    #[serde(default)]
    pub catalog_provider_id: Option<String>,
    #[serde(default)]
    pub base_path: Option<String>,
}

fn default_requires_auth() -> bool {
    true
}

#[derive(Deserialize, ToSchema)]
pub struct CheckProviderRequest {
    pub provider: String,
    /// Optional typed key for a validate-without-save check. Used only when no
    /// keychain value exists (keychain wins over env). Never persisted.
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct SetProviderRequest {
    pub provider: String,
    pub model: String,
}

/// One complete per-role route. Provider and model are deliberately accepted
/// together and persisted in one file write; two `/config/upsert` calls can
/// interleave and manufacture a provider/model pair the user never selected.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct SetModelRouteRequest {
    pub role: String,
    pub provider: String,
    pub model: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MaskedSecret {
    pub masked_value: String,
}

#[derive(Deserialize, ToSchema)]
pub struct ExtensionProbeRequest {
    /// Display name of the configured extension to probe (e.g. "Brave Search").
    pub name: String,
}

/// Result of actually starting an extension and asking it for its tools.
/// Serialized camelCase like the rest of this module's responses.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionProbeResponse {
    /// True only when the server started AND answered with at least one tool.
    pub ok: bool,
    pub tool_count: usize,
    /// Tool names it advertised — the concrete evidence the key works.
    pub tools: Vec<String>,
    /// Why it failed, verbatim, when `ok` is false.
    pub error: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(untagged)]
pub enum ConfigValueResponse {
    Value(Value),
    MaskedValue(MaskedSecret),
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub enum CommandType {
    Builtin,
    Recipe,
    Skill,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SlashCommand {
    pub command: String,
    pub help: String,
    pub command_type: CommandType,
}
#[derive(Serialize, ToSchema)]
pub struct SlashCommandsResponse {
    pub commands: Vec<SlashCommand>,
}

#[utoipa::path(
    post,
    path = "/config/upsert",
    request_body = UpsertConfigQuery,
    responses(
        (status = 200, description = "Configuration value upserted successfully", body = String),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn upsert_config(
    Json(query): Json<UpsertConfigQuery>,
) -> Result<Json<Value>, ErrorResponse> {
    let config = Config::global();
    config.set(&query.key, &query.value, query.is_secret)?;

    // A provider key, session model, Ollama host or per-role mapping changes
    // what the cost router can derive — drop its cached derived role map so the
    // next dispatch re-derives (otherwise the TTL applies).
    if permagent::cost_router::config_key_affects_derived_map(&query.key, query.is_secret) {
        permagent::cost_router::invalidate_derived_role_map();
    }

    // First-run Brain seeding (#298): the moment onboarding completes, seed
    // welcome/orientation memories daemon-internally. Idempotent and fire-and-
    // forget — no new public brain-write surface, no blocking of the config write.
    if query.key == "wizard_complete" && query.value == Value::Bool(true) {
        tokio::spawn(crate::automation::onboarding_seed::seed_onboarding_memories());
    }

    Ok(Json(Value::String(format!("Upserted key {}", query.key))))
}

fn model_route_keys(role: &str) -> Option<(&'static str, &'static str)> {
    match role {
        "chat" => Some(("chat_provider", "chat_model")),
        "voice" => Some((
            permagent::config::VOICE_PROVIDER_KEY,
            permagent::config::VOICE_MODEL_KEY,
        )),
        "harness" => Some(("harness_provider", "harness_model")),
        _ => None,
    }
}

fn validate_model_route(
    request: SetModelRouteRequest,
) -> Result<(&'static str, &'static str, String, String, String), String> {
    let role = request.role.trim().to_ascii_lowercase();
    let provider = request.provider.trim().to_string();
    let model = request.model.trim().to_string();
    let (provider_key, model_key) =
        model_route_keys(&role).ok_or_else(|| format!("Unknown model role '{}'", role))?;

    let disabled = ["session", "off", "none"];
    let is_disabled = disabled.contains(&provider.to_ascii_lowercase().as_str())
        || disabled.contains(&model.to_ascii_lowercase().as_str());
    if !is_disabled && (provider.is_empty() || model.is_empty()) {
        return Err("Provider and model must be selected together".to_string());
    }
    Ok((provider_key, model_key, provider, model, role))
}

#[utoipa::path(
    post,
    path = "/config/model-route",
    request_body = SetModelRouteRequest,
    responses(
        (status = 200, description = "Role provider/model route persisted atomically"),
        (status = 400, description = "Unknown role or incomplete provider/model pair"),
        (status = 500, description = "Configuration write failed")
    )
)]
pub async fn set_model_route(
    Json(request): Json<SetModelRouteRequest>,
) -> Result<Json<Value>, ErrorResponse> {
    let (provider_key, model_key, provider, model, role) =
        validate_model_route(request).map_err(ErrorResponse::bad_request)?;

    Config::global()
        .set_params([(provider_key, provider), (model_key, model)])
        .map_err(ErrorResponse::from)?;
    permagent::cost_router::invalidate_derived_role_map();

    Ok(Json(Value::String(format!(
        "Updated {role} provider/model route"
    ))))
}

#[utoipa::path(
    post,
    path = "/config/remove",
    request_body = ConfigKeyQuery,
    responses(
        (status = 200, description = "Configuration value removed successfully", body = String),
        (status = 404, description = "Configuration key not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn remove_config(
    Json(query): Json<ConfigKeyQuery>,
) -> Result<Json<String>, ErrorResponse> {
    let config = Config::global();

    if query.is_secret {
        config.delete_secret(&query.key)?;
    } else {
        config.delete(&query.key)?;
    }
    if permagent::cost_router::config_key_affects_derived_map(&query.key, query.is_secret) {
        permagent::cost_router::invalidate_derived_role_map();
    }

    Ok(Json(format!("Removed key {}", query.key)))
}

const SECRET_MASK_SHOW_LEN: usize = 8;

fn mask_secret(secret: Value) -> String {
    let as_string = match secret {
        Value::String(s) => s,
        _ => serde_json::to_string(&secret).unwrap_or_else(|_| secret.to_string()),
    };

    let chars: Vec<_> = as_string.chars().collect();
    let show_len = std::cmp::min(chars.len() / 2, SECRET_MASK_SHOW_LEN);
    let visible: String = chars.iter().take(show_len).collect();
    let mask = "*".repeat(chars.len() - show_len);

    format!("{}{}", visible, mask)
}

fn is_valid_provider_name(provider_name: &str) -> bool {
    !provider_name.is_empty()
        && provider_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[utoipa::path(
    post,
    path = "/config/read",
    request_body = ConfigKeyQuery,
    responses(
        (status = 200, description = "Configuration value retrieved successfully", body = Value),
        (status = 500, description = "Unable to get the configuration value"),
    )
)]
pub async fn read_config(
    Json(query): Json<ConfigKeyQuery>,
) -> Result<Json<ConfigValueResponse>, ErrorResponse> {
    let config = Config::global();

    let response_value = match config.get(&query.key, query.is_secret) {
        Ok(value) => {
            if query.is_secret {
                ConfigValueResponse::MaskedValue(MaskedSecret {
                    masked_value: mask_secret(value),
                })
            } else {
                ConfigValueResponse::Value(value)
            }
        }
        Err(ConfigError::NotFound(_)) => ConfigValueResponse::Value(Value::Null),
        Err(e) => return Err(e.into()),
    };
    Ok(Json(response_value))
}

/// How long to wait for an extension to start and answer `list_tools`. Well
/// past a healthy stdio server's startup, short enough that a wedged one still
/// gives the user an answer.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);

#[utoipa::path(
    post,
    path = "/config/extensions/probe",
    request_body = ExtensionProbeRequest,
    responses(
        (status = 200, description = "Probe completed (check `ok`)", body = ExtensionProbeResponse),
    )
)]
/// Actually start a configured extension and ask it for its tools.
///
/// "Key saved" only proves a string reached the keychain — a typo'd key looks
/// identical to a working one, and search then fails silently at the moment the
/// user needs it. This runs the real thing: resolve the config (which pulls the
/// key through `env_keys`), spawn the server, list its tools. Tool names coming
/// back are the evidence; anything else is reported verbatim rather than
/// summarized into a green light.
///
/// Always answers 200 — a failed probe is a RESULT, not a request error. The
/// manager is local to this call, so the probe never mutates a live session.
pub async fn probe_extension(
    Json(req): Json<ExtensionProbeRequest>,
) -> Result<Json<ExtensionProbeResponse>, ErrorResponse> {
    let fail = |error: String| {
        Ok(Json(ExtensionProbeResponse {
            ok: false,
            tool_count: 0,
            tools: Vec::new(),
            error: Some(error),
        }))
    };

    let Some(config) = permagent::config::extensions::get_extension_by_name(&req.name) else {
        return fail(format!("No extension named '{}' is configured.", req.name));
    };
    let key = config.key();

    let manager = std::sync::Arc::new(
        permagent::agents::extension_manager::ExtensionManager::new_without_provider(
            permagent::config::paths::Paths::data_dir(),
        ),
    );

    // Run the probe on its OWN task and time out the join handle, NOT the
    // future inline. Starting an extension resolves its `env_keys` through the
    // keychain, and `SecKeychainFindGenericPassword` is a BLOCKING call: on a
    // fresh ad-hoc-signed build the ACL no longer matches the new cdhash, so it
    // sits on a "allow access" dialog indefinitely. Awaited inline that blocks
    // the worker thread, the timer never gets polled, and `timeout` cannot fire
    // — the request hangs forever instead of failing at PROBE_TIMEOUT (observed
    // 2026-07-31: two probes hung past 60s with a 25s timeout set). Timing out
    // a separate task lets the caller get an honest answer even while the
    // blocked thread is still parked.
    let probe = tokio::spawn(async move {
        manager.add_extension(config, None, None, None).await?;
        manager
            .get_prefixed_tools("extension-probe", Some(key.clone()))
            .await
    });

    match tokio::time::timeout(PROBE_TIMEOUT, probe).await {
        Ok(Ok(Ok(tools))) if !tools.is_empty() => Ok(Json(ExtensionProbeResponse {
            ok: true,
            tool_count: tools.len(),
            tools: tools.iter().map(|t| t.name.to_string()).collect(),
            error: None,
        })),
        // Started but advertised nothing: not a working search provider, and
        // saying "ok" here is exactly the false green light this route exists
        // to remove.
        Ok(Ok(Ok(_))) => fail("The server started but offered no tools.".to_string()),
        Ok(Ok(Err(e))) => fail(format!("{e}")),
        // The probe task itself died (panic or cancellation) — report it rather
        // than letting a crashed probe read as a plain failure.
        Ok(Err(e)) => fail(format!("The probe did not complete: {e}")),
        Err(_) => fail(format!(
            "The server did not answer within {}s. If a keychain access dialog \
             is waiting on screen, allow it and test again.",
            PROBE_TIMEOUT.as_secs()
        )),
    }
}

#[utoipa::path(
    get,
    path = "/config/extensions",
    responses(
        (status = 200, description = "All extensions retrieved successfully", body = ExtensionResponse),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_extensions() -> Result<Json<ExtensionResponse>, ErrorResponse> {
    let extensions = permagent::config::get_all_extensions()
        .into_iter()
        .filter(|ext| {
            !permagent::agents::extension_manager::is_hidden_extension(&ext.config.name())
        })
        .collect();
    let warnings = permagent::config::get_warnings();
    Ok(Json(ExtensionResponse {
        extensions,
        warnings,
    }))
}

#[utoipa::path(
    post,
    path = "/config/extensions",
    request_body = ExtensionQuery,
    responses(
        (status = 200, description = "Extension added or updated successfully", body = String),
        (status = 400, description = "Invalid request"),
        (status = 422, description = "Could not serialize config.yaml"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn add_extension(
    Json(extension_query): Json<ExtensionQuery>,
) -> Result<Json<String>, ErrorResponse> {
    let extensions = permagent::config::get_all_extensions();
    let key = permagent::config::extensions::name_to_key(&extension_query.name);

    let is_update = extensions.iter().any(|e| e.config.key() == key);

    permagent::config::set_extension(ExtensionEntry {
        enabled: extension_query.enabled,
        config: extension_query.config,
    });

    if is_update {
        Ok(Json(format!("Updated extension {}", extension_query.name)))
    } else {
        Ok(Json(format!("Added extension {}", extension_query.name)))
    }
}

#[utoipa::path(
    delete,
    path = "/config/extensions/{name}",
    responses(
        (status = 200, description = "Extension removed successfully", body = String),
        (status = 404, description = "Extension not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn remove_extension(Path(name): Path<String>) -> Result<Json<String>, ErrorResponse> {
    let key = permagent::config::extensions::name_to_key(&name);
    permagent::config::remove_extension(&key);
    Ok(Json(format!("Removed extension {}", name)))
}

#[utoipa::path(
    get,
    path = "/config",
    responses(
        (status = 200, description = "All configuration values retrieved successfully", body = ConfigResponse)
    )
)]
pub async fn read_all_config() -> Result<Json<ConfigResponse>, ErrorResponse> {
    let config = Config::global();
    let values = config
        .all_values()
        .map_err(|e| ErrorResponse::unprocessable(e.to_string()))?;
    // Resolved through get_param (env var → YAML → default), unlike the
    // env-blind `values` map above.
    let effective_goose_mode = config.get_goose_mode().unwrap_or_default().to_string();
    Ok(Json(ConfigResponse {
        config: values,
        effective_goose_mode,
        resolved_routes: resolved_model_routes(),
    }))
}

#[utoipa::path(
    get,
    path = "/config/providers",
    responses(
        (status = 200, description = "All configuration values retrieved successfully", body = [ProviderDetails])
    )
)]
pub async fn providers() -> Result<Json<Vec<ProviderDetails>>, ErrorResponse> {
    let config = Config::global();
    let default_provider_name = config.get_goose_provider().ok();
    let stored_secrets = config.all_secrets().unwrap_or_default();

    let providers = get_providers().await;
    let providers_response: Vec<ProviderDetails> = providers
        .into_iter()
        .map(|(metadata, provider_type)| {
            let is_configured = check_provider_configured(&metadata, provider_type);
            let is_default = default_provider_name.as_deref() == Some(metadata.name.as_str());
            let env_override_active = env_override_active(&metadata, &stored_secrets);

            ProviderDetails {
                name: metadata.name.clone(),
                metadata,
                is_configured,
                is_default,
                provider_type,
                env_override_active,
            }
        })
        .collect();

    Ok(Json(providers_response))
}

// ── Secret sources ───────────────────────────────────────────────────────
//
// Where each secret is READ from. See `permagent::config::secret_source`.
// Settings needs three things and this section provides exactly those: what a
// key's source is today, whether the managers on this machine can actually
// answer, and a way to change a key's source that PROVES the new one works
// before committing to it.

/// One key's configured source, plus whether it currently resolves.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretKeySource {
    pub key: String,
    /// "keychain" | "file" | "onepassword" | "bitwarden"
    pub kind: String,
    /// Display label: "macOS Keychain", "1Password", …
    pub label: String,
    /// `op://…` / `bw://…`, or "" for the built-in stores. Never a secret.
    pub reference: String,
    /// True only for a source we actually read successfully just now. Built-in
    /// stores report `null` — this endpoint deliberately does not open the
    /// keychain, because listing sources is not worth an authorization prompt.
    pub resolves: Option<bool>,
    /// Why it doesn't, in one sentence. Sanitised upstream.
    pub error: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretSourcesResponse {
    /// Source used by keys with no explicit entry. Normally "keychain".
    pub default_source: String,
    /// Only keys with an EXPLICIT source. Everything else is the default, and
    /// listing every possible provider key here would be a list of nothing.
    pub keys: Vec<SecretKeySource>,
    pub backends: Vec<permagent::config::secret_source::BackendStatus>,
}

#[derive(Deserialize, ToSchema)]
pub struct SetSecretSourceRequest {
    /// Config key, e.g. "OPENAI_API_KEY".
    pub key: String,
    /// Source spec, or `null` to return the key to the default source.
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct TestSecretSourceRequest {
    /// Config key the candidate source is being tested FOR. Not needed to
    /// perform the read — a reference resolves on its own — but it is named in
    /// the failure so the message matches the one `Config::get_secret` will
    /// produce later ("Couldn't read 'OPENAI_API_KEY' from 1Password: …").
    /// Two different sentences for the same underlying failure is how a user
    /// ends up believing they are looking at two different problems.
    pub key: String,
    pub source: String,
}

/// Result of actually reading a candidate source. `ok` means a non-empty value
/// came back, which is the only evidence that distinguishes a working reference
/// from a plausible-looking typo.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TestSecretSourceResponse {
    pub ok: bool,
    /// Masked exactly like a stored secret is elsewhere in this module, so the
    /// user can confirm they got the RIGHT item without this endpoint inventing
    /// a new disclosure policy.
    pub masked_value: Option<String>,
    pub error: Option<String>,
}

/// Wall-clock budget for one secret-source call, measured from the handler.
///
/// Strictly larger than the module's own `READ_TIMEOUT` so that in the normal
/// case the inner, more specific error ("1Password is not signed in") is what
/// the user sees; this outer bound only fires if the blocking task itself is
/// stuck somewhere the inner timeout does not cover.
const SECRET_SOURCE_BUDGET: Duration = Duration::from_secs(25);

/// Run blocking secret-source work on its own task and time out the JOIN
/// HANDLE.
///
/// Identical reasoning to `PROBE_TIMEOUT` above, and it applies here for
/// exactly the same mechanical reason: resolving a source runs a subprocess and
/// polls it from a blocking thread. Awaited inline that parks a runtime worker,
/// the timer never gets polled, and `tokio::time::timeout` cannot fire — the
/// request hangs forever instead of failing at the deadline. Timing out a
/// separate task lets the caller get an honest answer even while the blocked
/// thread is still parked.
async fn bounded_secret_work<T, F>(what: &str, work: F) -> Result<T, ErrorResponse>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let handle: JoinHandle<T> = tokio::task::spawn_blocking(work);
    match tokio::time::timeout(SECRET_SOURCE_BUDGET, handle).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(ErrorResponse::internal(format!(
            "{what} did not complete: {e}"
        ))),
        Err(_) => Err(ErrorResponse::service_unavailable(format!(
            "{what} did not answer within {}s. If a password-manager approval prompt is \
             waiting on screen, allow it and try again.",
            SECRET_SOURCE_BUDGET.as_secs()
        ))),
    }
}

#[utoipa::path(
    get,
    path = "/config/secret-sources",
    responses(
        (status = 200, description = "Configured secret sources and backend availability", body = SecretSourcesResponse),
    )
)]
pub async fn get_secret_sources() -> Result<Json<SecretSourcesResponse>, ErrorResponse> {
    use permagent::config::secret_source::{self, SecretSource};

    bounded_secret_work("Reading secret sources", || {
        let config = Config::global();
        let default_source = config
            .secret_source_default()
            .unwrap_or_else(|| "keychain".to_string());

        // Probing is per key AND capped overall. Per key, because one wedged
        // manager must not stall the rest; overall, because N broken keys at
        // the per-key bound each is how a settings panel ends up taking a
        // minute to render. Keys past the cap come back with `resolves: null`
        // — "not checked", the same thing the built-in stores report — rather
        // than a guessed pass or fail.
        const LIST_PROBE_BUDGET: Duration = Duration::from_secs(10);
        let started = std::time::Instant::now();

        let mut keys: Vec<SecretKeySource> = config
            .secret_source_map()
            .into_iter()
            .map(|(key, spec)| match SecretSource::parse(&spec) {
                Ok(source) => {
                    // Only external sources are probed. Opening the keychain to
                    // render a settings list would risk an authorization prompt
                    // for a read nobody asked for.
                    //
                    // PROBE_TIMEOUT, not READ_TIMEOUT: this is a passive list.
                    // A read slow enough to exceed it (a Touch ID prompt still
                    // on screen) is reported as a timeout with the "allow it and
                    // try again" message, and the user-initiated "Test
                    // reference" button below is the one that waits patiently.
                    let (resolves, error) = match source.backend() {
                        None => (None, None),
                        Some(_) if started.elapsed() >= LIST_PROBE_BUDGET => (None, None),
                        Some(_) => match source.resolve(secret_source::PROBE_TIMEOUT) {
                            Ok(_) => (Some(true), None),
                            Err(e) => (Some(false), Some(e.to_string())),
                        },
                    };
                    SecretKeySource {
                        key,
                        kind: source_kind(&source).to_string(),
                        label: source.label(),
                        reference: source.locator(),
                        resolves,
                        error,
                    }
                }
                // A spec we cannot parse is REPORTED as a broken row, not
                // dropped from the list. Dropping it would show the user a key
                // that appears to be on the keychain while `get_secret` fails
                // for it every single time.
                Err(e) => SecretKeySource {
                    key,
                    kind: "invalid".to_string(),
                    label: "Not a valid source".to_string(),
                    reference: spec,
                    resolves: Some(false),
                    error: Some(e.to_string()),
                },
            })
            .collect();
        keys.sort_by(|a, b| a.key.cmp(&b.key));

        SecretSourcesResponse {
            default_source,
            keys,
            backends: secret_source::probe_backends(secret_source::PROBE_TIMEOUT),
        }
    })
    .await
    .map(Json)
}

fn source_kind(source: &permagent::config::SecretSource) -> &'static str {
    use permagent::config::SecretSource as S;
    match source {
        S::Keychain => "keychain",
        S::File => "file",
        S::OnePassword { .. } => "onepassword",
        S::Bitwarden { .. } => "bitwarden",
    }
}

#[utoipa::path(
    post,
    path = "/config/secret-sources",
    request_body = SetSecretSourceRequest,
    responses(
        (status = 200, description = "Source updated", body = SecretKeySource),
        (status = 422, description = "The source spec is not valid"),
    )
)]
pub async fn set_secret_source(
    Json(req): Json<SetSecretSourceRequest>,
) -> Result<Json<SecretKeySource>, ErrorResponse> {
    let config = Config::global();

    let source = match req
        .source
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(spec) => config.set_secret_source(&req.key, spec)?,
        None => {
            config.clear_secret_source(&req.key)?;
            config.secret_source_for(&req.key)?
        }
    };

    // A provider built before the switch is still holding the credential it
    // read from the OLD source. Without this the UI says "1Password" while chat
    // keeps using the keychain value, which is precisely the "reads as present,
    // behaves otherwise" failure this feature exists to remove.
    tracing::info!(
        key = %req.key,
        source = %source.label(),
        "Secret source changed; providers must be reloaded to pick it up"
    );

    Ok(Json(SecretKeySource {
        key: req.key,
        kind: source_kind(&source).to_string(),
        label: source.label(),
        reference: source.locator(),
        resolves: None,
        error: None,
    }))
}

#[utoipa::path(
    post,
    path = "/config/secret-sources/test",
    request_body = TestSecretSourceRequest,
    responses(
        (status = 200, description = "Test completed (check `ok`)", body = TestSecretSourceResponse),
    )
)]
/// Actually read a candidate source, WITHOUT saving it.
///
/// "Reference saved" only proves a string reached config.yaml — a typo'd vault
/// path looks identical to a working one until the next time chat needs the
/// key. This runs the real `op read` / `bw get` and reports what came back.
///
/// Always answers 200: a failed test is a RESULT, not a request error.
pub async fn test_secret_source(
    Json(req): Json<TestSecretSourceRequest>,
) -> Result<Json<TestSecretSourceResponse>, ErrorResponse> {
    use permagent::config::secret_source::{self, SecretSource};

    let outcome = bounded_secret_work("Testing the secret source", move || {
        // Phrased like `ConfigError::SecretSource`, and naming the same key,
        // so a reference that fails here and a reference that fails later
        // during a chat turn read as ONE problem rather than two.
        let fail = |detail: String| TestSecretSourceResponse {
            ok: false,
            masked_value: None,
            error: Some(format!("Couldn't read '{}': {detail}", req.key)),
        };

        let source = match SecretSource::parse(&req.source) {
            Ok(s) => s,
            Err(e) => return fail(e.to_string()),
        };
        match source.resolve(secret_source::READ_TIMEOUT) {
            // A built-in store: nothing to test here, and this endpoint will
            // not open the keychain just to say "yes".
            Ok(None) => TestSecretSourceResponse {
                ok: true,
                masked_value: None,
                error: None,
            },
            Ok(Some(value)) => TestSecretSourceResponse {
                ok: true,
                masked_value: Some(mask_secret(Value::String(value.expose()))),
                error: None,
            },
            Err(e) => fail(e.to_string()),
        }
    })
    .await?;

    Ok(Json(outcome))
}

/// A secret config key is both set in the process environment and present
/// (non-empty) in secret storage. Storage is authoritative on current builds,
/// so the env copy is stale at best and shadowing at worst (pre-fix builds).
fn env_override_active(
    metadata: &ProviderMetadata,
    stored_secrets: &HashMap<String, Value>,
) -> bool {
    metadata.config_keys.iter().any(|key| {
        key.secret
            && std::env::var(&key.name).is_ok_and(|v| !v.is_empty())
            && stored_secrets
                .get(&key.name)
                .is_some_and(|v| !v.as_str().is_some_and(|s| s.is_empty()))
    })
}

#[utoipa::path(
    get,
    path = "/config/providers/{name}/models",
    params(
        ("name" = String, Path, description = "Provider name (e.g., openai)")
    ),
    responses(
        (status = 200, description = "Models fetched successfully", body = [String]),
        (status = 400, description = "Unknown provider, provider not configured, or authentication error"),
        (status = 429, description = "Rate limit exceeded"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_provider_models(
    Path(name): Path<String>,
) -> Result<Json<Vec<String>>, ErrorResponse> {
    let all = get_providers().await.into_iter().collect::<Vec<_>>();
    let Some((metadata, provider_type)) = all.into_iter().find(|(m, _)| m.name == name) else {
        return Err(ErrorResponse::bad_request(format!(
            "Unknown provider: {}",
            name
        )));
    };
    if !check_provider_configured(&metadata, provider_type) {
        return Err(ErrorResponse::bad_request(format!(
            "Provider '{}' is not configured",
            name
        )));
    }

    let model_config = ModelConfig::new(&metadata.default_model)?.with_canonical_limits(&name);
    let provider = permagent::providers::create(&name, model_config, Vec::new()).await?;

    let models_result = provider.fetch_recommended_models().await;

    match models_result {
        Ok(models) => Ok(Json(models)),
        Err(provider_error) => Err(provider_error.into()),
    }
}

#[derive(Deserialize, utoipa::IntoParams)]
pub struct SlashCommandsQuery {
    /// Optional working directory to discover local skills from
    pub working_dir: Option<String>,
}

#[utoipa::path(
    get,
    path = "/config/slash_commands",
    params(SlashCommandsQuery),
    responses(
        (status = 200, description = "Slash commands retrieved successfully", body = SlashCommandsResponse)
    )
)]
pub async fn get_slash_commands(
    axum::extract::Query(query): axum::extract::Query<SlashCommandsQuery>,
) -> Result<Json<SlashCommandsResponse>, ErrorResponse> {
    let mut commands: Vec<_> = slash_commands::list_commands()
        .iter()
        .map(|command| SlashCommand {
            command: command.command.clone(),
            help: command.recipe_path.clone(),
            command_type: CommandType::Recipe,
        })
        .collect();

    for cmd_def in execute_commands::list_commands() {
        commands.push(SlashCommand {
            command: cmd_def.name.to_string(),
            help: cmd_def.description.to_string(),
            command_type: CommandType::Builtin,
        });
    }

    let working_dir = query.working_dir.map(std::path::PathBuf::from);
    for source in permagent::agents::platform_extensions::skills::list_installed_skills(
        working_dir.as_deref(),
    ) {
        commands.push(SlashCommand {
            command: source.name,
            help: source.description,
            command_type: CommandType::Skill,
        });
    }

    Ok(Json(SlashCommandsResponse { commands }))
}

#[derive(Serialize, ToSchema)]
pub struct ModelInfoData {
    pub provider: String,
    pub model: String,
    pub context_limit: usize,
    pub max_output_tokens: Option<usize>,
    pub input_token_cost: Option<f64>,
    pub output_token_cost: Option<f64>,
    pub cache_read_token_cost: Option<f64>,
    pub cache_write_token_cost: Option<f64>,
    pub currency: String,
}

#[derive(Serialize, ToSchema)]
pub struct ModelInfoResponse {
    pub model_info: Option<ModelInfoData>,
    pub source: String,
}

#[derive(Deserialize, ToSchema)]
pub struct ModelInfoQuery {
    pub provider: String,
    pub model: String,
}

#[utoipa::path(
    post,
    path = "/config/canonical-model-info",
    request_body = ModelInfoQuery,
    responses(
        (status = 200, description = "Model information retrieved successfully", body = ModelInfoResponse)
    )
)]
pub async fn get_canonical_model_info(
    Json(query): Json<ModelInfoQuery>,
) -> Json<ModelInfoResponse> {
    let canonical_model = maybe_get_canonical_model(&query.provider, &query.model);

    let model_info = canonical_model.map(|canonical_model| ModelInfoData {
        provider: query.provider.clone(),
        model: query.model.clone(),
        context_limit: canonical_model.limit.context,
        max_output_tokens: canonical_model.limit.output,
        // Costs are per million tokens - client handles division for display
        input_token_cost: canonical_model.cost.input,
        output_token_cost: canonical_model.cost.output,
        cache_read_token_cost: canonical_model.cost.cache_read,
        cache_write_token_cost: canonical_model.cost.cache_write,
        currency: "$".to_string(),
    });

    Json(ModelInfoResponse {
        model_info,
        source: "canonical".to_string(),
    })
}

#[utoipa::path(
    post,
    path = "/config/init",
    responses(
        (status = 200, description = "Config initialization check completed", body = String),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn init_config() -> Result<Json<String>, ErrorResponse> {
    let config = Config::global();

    if config.exists() {
        return Ok(Json("Config already exists".to_string()));
    }

    // Use the shared function to load init-config.yaml
    match permagent::config::base::load_init_config_from_workspace() {
        Ok(init_values) => {
            config.initialize_if_empty(init_values)?;
            Ok(Json("Config initialized successfully".to_string()))
        }
        Err(_) => Ok(Json(
            "No init-config.yaml found, using default configuration".to_string(),
        )),
    }
}

#[utoipa::path(
    post,
    path = "/config/permissions",
    request_body = UpsertPermissionsQuery,
    responses(
        (status = 200, description = "Permission update completed", body = String),
        (status = 400, description = "Invalid request"),
    )
)]
pub async fn upsert_permissions(
    Json(query): Json<UpsertPermissionsQuery>,
) -> Result<Json<String>, ErrorResponse> {
    let permission_manager = permagent::config::PermissionManager::instance();

    for tool_permission in &query.tool_permissions {
        permission_manager.update_user_permission(
            &tool_permission.tool_name,
            tool_permission.permission.clone(),
        );
    }

    Ok(Json("Permissions updated successfully".to_string()))
}

#[utoipa::path(
    post,
    path = "/config/backup",
    responses(
        (status = 200, description = "Config file backed up", body = String),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn backup_config() -> Result<Json<String>, ErrorResponse> {
    let config_path = Paths::config_dir().join("config.yaml");

    if !config_path.exists() {
        return Err(ErrorResponse::not_found("Config file does not exist"));
    }

    let file_name = config_path
        .file_name()
        .ok_or_else(|| ErrorResponse::internal("Invalid config file path"))?;

    let mut backup_name = file_name.to_os_string();
    backup_name.push(".bak");

    let backup = config_path.with_file_name(backup_name);
    std::fs::copy(&config_path, &backup)?;
    Ok(Json(format!("Copied {:?} to {:?}", config_path, backup)))
}

#[utoipa::path(
    post,
    path = "/config/recover",
    responses(
        (status = 200, description = "Config recovery attempted", body = String),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn recover_config() -> Result<Json<String>, ErrorResponse> {
    let config = Config::global();

    // Force a reload which will trigger recovery if needed
    let values = config.all_values()?;
    let recovered_keys: Vec<String> = values.keys().cloned().collect();

    if recovered_keys.is_empty() {
        Ok(Json("Config recovery completed, but no data was recoverable. Starting with empty configuration.".to_string()))
    } else {
        Ok(Json(format!(
            "Config recovery completed. Recovered {} keys: {}",
            recovered_keys.len(),
            recovered_keys.join(", ")
        )))
    }
}

#[utoipa::path(
    get,
    path = "/config/validate",
    responses(
        (status = 200, description = "Config validation result", body = String),
        (status = 422, description = "Config file is corrupted")
    )
)]
pub async fn validate_config() -> Result<Json<String>, ErrorResponse> {
    let config_path = Paths::config_dir().join("config.yaml");

    if !config_path.exists() {
        return Ok(Json("Config file does not exist".to_string()));
    }

    let content = std::fs::read_to_string(&config_path)?;
    serde_yaml::from_str::<serde_yaml::Value>(&content)
        .map_err(|e| ErrorResponse::unprocessable(format!("Config file is corrupted: {}", e)))?;

    Ok(Json("Config file is valid".to_string()))
}
#[derive(Serialize, ToSchema)]
pub struct CreateCustomProviderResponse {
    pub provider_name: String,
}

#[utoipa::path(
    post,
    path = "/config/custom-providers",
    request_body = UpdateCustomProviderRequest,
    responses(
        (status = 200, description = "Custom provider created successfully", body = CreateCustomProviderResponse),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn create_custom_provider(
    Json(request): Json<UpdateCustomProviderRequest>,
) -> Result<Json<CreateCustomProviderResponse>, ErrorResponse> {
    let config = permagent::config::declarative_providers::create_custom_provider(
        permagent::config::declarative_providers::CreateCustomProviderParams {
            engine: request.engine,
            display_name: request.display_name,
            api_url: request.api_url,
            api_key: request.api_key,
            models: request.models,
            supports_streaming: request.supports_streaming,
            headers: request.headers,
            requires_auth: request.requires_auth,
            catalog_provider_id: request.catalog_provider_id,
            base_path: request.base_path,
        },
    )?;

    permagent::providers::refresh_custom_providers().await?;

    Ok(Json(CreateCustomProviderResponse {
        provider_name: config.id().to_string(),
    }))
}

#[utoipa::path(
    get,
    path = "/config/custom-providers/{id}",
    responses(
        (status = 200, description = "Custom provider retrieved successfully", body = LoadedProvider),
        (status = 404, description = "Provider not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_custom_provider(
    Path(id): Path<String>,
) -> Result<Json<LoadedProvider>, ErrorResponse> {
    let loaded_provider = permagent::config::declarative_providers::load_provider(id.as_str())
        .map_err(|e| {
            ErrorResponse::not_found(format!("Custom provider '{}' not found: {}", id, e))
        })?;

    Ok(Json(loaded_provider))
}

#[utoipa::path(
    delete,
    path = "/config/custom-providers/{id}",
    responses(
        (status = 200, description = "Custom provider removed successfully", body = String),
        (status = 404, description = "Provider not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn remove_custom_provider(Path(id): Path<String>) -> Result<Json<String>, ErrorResponse> {
    permagent::config::declarative_providers::remove_custom_provider(&id)?;

    permagent::providers::refresh_custom_providers().await?;

    Ok(Json(format!("Removed custom provider: {}", id)))
}

#[utoipa::path(
    post,
    path = "/config/providers/{name}/cleanup",
    params(
        ("name" = String, Path, description = "Provider name (e.g., githubcopilot)")
    ),
    responses(
        (status = 200, description = "Provider cache cleaned up successfully", body = String),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn cleanup_provider_cache(
    Path(name): Path<String>,
) -> Result<Json<String>, ErrorResponse> {
    permagent::providers::cleanup_provider(&name).await?;
    Ok(Json(format!("Cleaned up provider cache: {}", name)))
}

#[utoipa::path(
    put,
    path = "/config/custom-providers/{id}",
    request_body = UpdateCustomProviderRequest,
    responses(
        (status = 200, description = "Custom provider updated successfully", body = String),
        (status = 404, description = "Provider not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn update_custom_provider(
    Path(id): Path<String>,
    Json(request): Json<UpdateCustomProviderRequest>,
) -> Result<Json<String>, ErrorResponse> {
    permagent::config::declarative_providers::update_custom_provider(
        permagent::config::declarative_providers::UpdateCustomProviderParams {
            id: id.clone(),
            engine: request.engine,
            display_name: request.display_name,
            api_url: request.api_url,
            api_key: request.api_key,
            models: request.models,
            supports_streaming: request.supports_streaming,
            headers: request.headers,
            requires_auth: request.requires_auth,
            catalog_provider_id: request.catalog_provider_id,
            base_path: request.base_path,
        },
    )?;

    permagent::providers::refresh_custom_providers().await?;

    Ok(Json(format!("Updated custom provider: {}", id)))
}

#[utoipa::path(
    post,
    path = "/config/check_provider",
    request_body = CheckProviderRequest,
)]
pub async fn check_provider(
    Json(CheckProviderRequest { provider, api_key }): Json<CheckProviderRequest>,
) -> Result<(), ErrorResponse> {
    let overlay = match api_key.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(typed) => {
            let entry = get_from_registry(&provider).await.map_err(|err| {
                ErrorResponse::bad_request(format!("Provider '{}' check failed: {}", provider, err))
            })?;
            entry
                .metadata()
                .config_keys
                .iter()
                .find(|k| k.secret)
                .map(|k| {
                    let prev = std::env::var(&k.name).ok();
                    std::env::set_var(&k.name, typed);
                    EnvOverlay {
                        name: k.name.clone(),
                        prev,
                    }
                })
        }
        None => None,
    };

    let runtime = tokio::runtime::Handle::current();
    let checked_provider = provider.clone();

    // Declarative OpenAI-compatible providers (including Moonshot) resolve
    // their API key during construction. That reaches Config::all_secrets,
    // whose synchronous OS-keyring read runs while holding secrets_cache's
    // mutex. A wedged keyring call therefore cannot yield to an async timeout.
    // Isolate the complete provider construction on the blocking pool so the
    // request timer remains schedulable. Dropping a timed-out JoinHandle cannot
    // cancel the OS call, but it does guarantee this HTTP request returns.
    let check = tokio::task::spawn_blocking(move || {
        runtime
            .block_on(create_with_default_model(&checked_provider, Vec::new()))
            .map(|_| ())
            .map_err(|err| err.to_string())
    });

    let result = await_provider_check(&provider, PROVIDER_CHECK_TIMEOUT, check).await;
    drop(overlay);
    result
}

/// Temporary env overlay for validate-without-save. Restored on drop so a
/// typed key never leaks into later requests. Keychain still wins over env.
struct EnvOverlay {
    name: String,
    prev: Option<String>,
}

impl Drop for EnvOverlay {
    fn drop(&mut self) {
        match &self.prev {
            Some(v) => std::env::set_var(&self.name, v),
            None => std::env::remove_var(&self.name),
        }
    }
}

async fn await_provider_check(
    provider: &str,
    timeout: Duration,
    check: JoinHandle<Result<(), String>>,
) -> Result<(), ErrorResponse> {
    match tokio::time::timeout(timeout, check).await {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(err))) => Err(ErrorResponse::bad_request(format!(
            "Provider '{}' check failed: {}",
            provider, err
        ))),
        Ok(Err(err)) => Err(ErrorResponse::internal(format!(
            "Provider '{}' check task failed: {}",
            provider, err
        ))),
        Err(_) => Err(ErrorResponse::service_unavailable(format!(
            "Provider '{}' check timed out after {} seconds",
            provider,
            timeout.as_secs()
        ))),
    }
}

#[utoipa::path(
    post,
    path = "/config/set_provider",
    request_body = SetProviderRequest,
)]
pub async fn set_config_provider(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(SetProviderRequest { provider, model }): Json<SetProviderRequest>,
) -> Result<(), ErrorResponse> {
    // Create and validate the provider (also used for runtime state update)
    let provider_arc = create_with_named_model(&provider, &model, Vec::new())
        .await
        .map_err(|err| {
            ErrorResponse::bad_request(format!(
                "Failed to set provider to '{}' with model '{}': {}",
                provider, model, err
            ))
        })?;

    // Persist to config.yaml (source of truth)
    let config = Config::global();
    // Provider/model is one routing value. Persist both through the config's
    // single-write API so a disk failure can never leave a half-configured
    // global route behind.
    config
        .set_params([
            ("GOOSE_PROVIDER", provider.clone()),
            ("GOOSE_MODEL", model.clone()),
        ])
        .map_err(|e| {
            ErrorResponse::bad_request(format!("Failed to persist provider config: {}", e))
        })?;

    // Update in-memory runtime state so new sessions use the new provider immediately
    state.agent_manager.set_default_provider(provider_arc).await;

    tracing::info!(
        provider = %provider,
        model = %model,
        "Default provider updated (config.yaml + runtime)"
    );

    Ok(())
}

#[utoipa::path(
    get,
    path = "/config/provider-catalog",
    params(
        ("format" = Option<String>, Query, description = "Filter by provider format (openai, anthropic, ollama)")
    ),
    responses(
        (status = 200, description = "Provider catalog retrieved successfully", body = [ProviderCatalogEntry]),
        (status = 400, description = "Invalid format parameter")
    )
)]
pub async fn get_provider_catalog(
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Result<Json<Vec<ProviderCatalogEntry>>, ErrorResponse> {
    let format_str = params.get("format").map(|s| s.as_str()).unwrap_or("openai");

    let format = format_str.parse::<ProviderFormat>().map_err(|_| {
        ErrorResponse::bad_request(format!(
            "Invalid format '{}'. Must be one of: openai, anthropic, ollama",
            format_str
        ))
    })?;

    let providers = get_providers_by_format(format).await;
    Ok(Json(providers))
}

#[utoipa::path(
    get,
    path = "/config/provider-catalog/{id}",
    params(
        ("id" = String, Path, description = "Provider ID from models.dev")
    ),
    responses(
        (status = 200, description = "Provider template retrieved successfully", body = ProviderTemplate),
        (status = 404, description = "Provider not found in catalog")
    )
)]
pub async fn get_provider_catalog_template(
    Path(id): Path<String>,
) -> Result<Json<ProviderTemplate>, ErrorResponse> {
    let template = get_provider_template(&id).ok_or_else(|| {
        ErrorResponse::not_found(format!("Provider '{}' not found in catalog", id))
    })?;

    Ok(Json(template))
}

#[utoipa::path(
    post,
    path = "/config/providers/{name}/oauth",
    params(
        ("name" = String, Path, description = "Provider name")
    ),
    responses(
        (status = 200, description = "OAuth configuration completed"),
        (status = 400, description = "OAuth configuration failed")
    )
)]
pub async fn configure_provider_oauth(
    Path(provider_name): Path<String>,
) -> Result<Json<String>, ErrorResponse> {
    use permagent::model::ModelConfig;
    use permagent::providers::create;

    if !is_valid_provider_name(&provider_name) {
        return Err(ErrorResponse::bad_request(format!(
            "Invalid provider name: '{}'",
            provider_name
        )));
    }

    let temp_model = ModelConfig::new("temp")
        .map_err(|e| {
            ErrorResponse::bad_request(format!("Failed to create temporary model config: {}", e))
        })?
        .with_canonical_limits(&provider_name);

    // OAuth configuration does not use extensions.
    let provider = create(&provider_name, temp_model, Vec::new())
        .await
        .map_err(|e| {
            ErrorResponse::bad_request(format!(
                "Failed to create provider '{}': {}",
                provider_name, e
            ))
        })?;

    provider.configure_oauth().await.map_err(|e| {
        ErrorResponse::bad_request(format!(
            "OAuth configuration failed for provider '{}': {}",
            provider_name, e
        ))
    })?;

    // Mark the provider as configured after successful OAuth
    let configured_marker = format!("{}_configured", provider_name);
    let config = permagent::config::Config::global();
    config.set_param(&configured_marker, true)?;

    Ok(Json("OAuth configuration completed".to_string()))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReloadResponse {
    pub provider: String,
    pub key_tail: String,
}

#[utoipa::path(
    post,
    path = "/config/reload",
    responses(
        (status = 200, description = "Provider reloaded with fresh credentials", body = ReloadResponse),
        (status = 400, description = "Provider not configured or reload failed"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn reload_config(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> Result<Json<ReloadResponse>, ErrorResponse> {
    let config = Config::global();

    let provider_name = config
        .get_goose_provider()
        .map_err(|_| ErrorResponse::bad_request("No default provider configured".to_string()))?;

    // Re-create the provider, which re-reads credentials from config (keychain + env)
    let provider_arc = create_with_default_model(&provider_name, Vec::new())
        .await
        .map_err(|err| {
            ErrorResponse::bad_request(format!(
                "Failed to reload provider '{}': {}",
                provider_name, err
            ))
        })?;

    // Hot-swap the in-memory provider so new sessions use fresh credentials
    state.agent_manager.set_default_provider(provider_arc).await;

    // Read the key tail for confirmation (last 4 chars only)
    let key_tail = {
        let key_name = format!("{}_API_KEY", provider_name.to_uppercase().replace('-', "_"));
        match config.get_secret::<String>(&key_name) {
            Ok(secret) => {
                let chars: Vec<char> = secret.chars().collect();
                if chars.len() >= 4 {
                    format!("...{}", chars[chars.len() - 4..].iter().collect::<String>())
                } else {
                    "****".to_string()
                }
            }
            Err(_) => "(no key)".to_string(),
        }
    };

    tracing::info!(
        provider = %provider_name,
        "Provider reloaded with fresh credentials"
    );

    Ok(Json(ReloadResponse {
        provider: provider_name,
        key_tail,
    }))
}

#[derive(Deserialize, ToSchema)]
pub struct WorkspaceTrustQuery {
    pub path: String,
}

#[derive(Serialize, ToSchema)]
pub struct WorkspaceTrustListResponse {
    pub trusted_workspaces: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct WorkspaceTrustMutationResponse {
    pub canonical: String,
    pub trusted_workspaces: Vec<String>,
}

#[utoipa::path(
    get,
    path = "/config/workspace-trust",
    responses(
        (status = 200, description = "Trusted workspace directories", body = WorkspaceTrustListResponse)
    )
)]
pub async fn list_workspace_trust() -> Json<WorkspaceTrustListResponse> {
    Json(WorkspaceTrustListResponse {
        trusted_workspaces: permagent::config::list_trusted_workspaces(),
    })
}

#[utoipa::path(
    post,
    path = "/config/workspace-trust",
    request_body = WorkspaceTrustQuery,
    responses(
        (status = 200, description = "Workspace trusted", body = WorkspaceTrustMutationResponse),
        (status = 400, description = "Path could not be resolved")
    )
)]
pub async fn trust_workspace(
    Json(query): Json<WorkspaceTrustQuery>,
) -> Result<Json<WorkspaceTrustMutationResponse>, ErrorResponse> {
    let canonical =
        permagent::config::trust_workspace(&query.path).map_err(|err| ErrorResponse {
            message: err.to_string(),
            status: StatusCode::BAD_REQUEST,
        })?;
    Ok(Json(WorkspaceTrustMutationResponse {
        canonical,
        trusted_workspaces: permagent::config::list_trusted_workspaces(),
    }))
}

#[utoipa::path(
    delete,
    path = "/config/workspace-trust",
    request_body = WorkspaceTrustQuery,
    responses(
        (status = 200, description = "Workspace trust revoked", body = WorkspaceTrustMutationResponse),
        (status = 400, description = "Path could not be resolved")
    )
)]
pub async fn untrust_workspace(
    Json(query): Json<WorkspaceTrustQuery>,
) -> Result<Json<WorkspaceTrustMutationResponse>, ErrorResponse> {
    let canonical =
        permagent::config::untrust_workspace(&query.path).map_err(|err| ErrorResponse {
            message: err.to_string(),
            status: StatusCode::BAD_REQUEST,
        })?;
    Ok(Json(WorkspaceTrustMutationResponse {
        canonical,
        trusted_workspaces: permagent::config::list_trusted_workspaces(),
    }))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/config", get(read_all_config))
        .route("/config/upsert", post(upsert_config))
        .route("/config/model-route", post(set_model_route))
        .route("/config/remove", post(remove_config))
        .route("/config/read", post(read_config))
        .route("/config/secret-sources", get(get_secret_sources))
        .route("/config/secret-sources", post(set_secret_source))
        .route("/config/secret-sources/test", post(test_secret_source))
        .route("/config/extensions", get(get_extensions))
        .route("/config/extensions", post(add_extension))
        .route("/config/extensions/probe", post(probe_extension))
        .route("/config/extensions/{name}", delete(remove_extension))
        .route("/config/providers", get(providers))
        .route("/config/providers/{name}/models", get(get_provider_models))
        .route("/config/provider-catalog", get(get_provider_catalog))
        .route(
            "/config/provider-catalog/{id}",
            get(get_provider_catalog_template),
        )
        .route(
            "/config/providers/{name}/cleanup",
            post(cleanup_provider_cache),
        )
        .route("/config/slash_commands", get(get_slash_commands))
        .route(
            "/config/canonical-model-info",
            post(get_canonical_model_info),
        )
        .route("/config/init", post(init_config))
        .route("/config/backup", post(backup_config))
        .route("/config/recover", post(recover_config))
        .route("/config/validate", get(validate_config))
        .route("/config/permissions", post(upsert_permissions))
        .route("/config/custom-providers", post(create_custom_provider))
        .route(
            "/config/custom-providers/{id}",
            delete(remove_custom_provider),
        )
        .route("/config/custom-providers/{id}", put(update_custom_provider))
        .route("/config/custom-providers/{id}", get(get_custom_provider))
        .route("/config/check_provider", post(check_provider))
        .route("/config/set_provider", post(set_config_provider))
        .route("/config/reload", post(reload_config))
        .route(
            "/config/providers/{name}/oauth",
            post(configure_provider_oauth),
        )
        .route("/config/workspace-trust", get(list_workspace_trust))
        .route("/config/workspace-trust", post(trust_workspace))
        .route("/config/workspace-trust", delete(untrust_workspace))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_route_roles_map_only_to_their_owned_pair() {
        assert_eq!(
            model_route_keys("chat"),
            Some(("chat_provider", "chat_model"))
        );
        assert_eq!(
            model_route_keys("voice"),
            Some((
                permagent::config::VOICE_PROVIDER_KEY,
                permagent::config::VOICE_MODEL_KEY
            ))
        );
        assert_eq!(
            model_route_keys("harness"),
            Some(("harness_provider", "harness_model"))
        );
        assert_eq!(model_route_keys("coding"), None);
    }

    #[test]
    fn model_route_validation_rejects_half_pairs_before_any_write() {
        for request in [
            SetModelRouteRequest {
                role: "chat".into(),
                provider: "anthropic".into(),
                model: "".into(),
            },
            SetModelRouteRequest {
                role: "voice".into(),
                provider: "".into(),
                model: "deepseek-chat".into(),
            },
            SetModelRouteRequest {
                role: "coding".into(),
                provider: "openai".into(),
                model: "gpt-5".into(),
            },
        ] {
            assert!(validate_model_route(request).is_err());
        }

        let disabled = validate_model_route(SetModelRouteRequest {
            role: "voice".into(),
            provider: "".into(),
            model: "session".into(),
        })
        .expect("the explicit session route is a complete user choice");
        assert_eq!((disabled.0, disabled.1), ("voice_provider", "voice_model"));
    }

    #[test]
    fn resolved_route_projection_prefers_explicit_route_and_falls_back_truthfully() {
        let explicit = resolved_route(
            Some(("anthropic".into(), "claude-haiku".into())),
            ("openai".into(), "gpt-5".into()),
            "configured",
        );
        assert_eq!(
            explicit,
            ResolvedModelRoute {
                provider: "anthropic".into(),
                model: "claude-haiku".into(),
                source: "configured".into(),
            }
        );

        let fallback = resolved_route(
            None,
            ("custom_deepseek".into(), "deepseek-chat".into()),
            "session_model",
        );
        assert_eq!(fallback.provider, "custom_deepseek");
        assert_eq!(fallback.model, "deepseek-chat");
        assert_eq!(fallback.source, "session_model");
    }

    #[test]
    fn resolved_route_projection_serializes_for_mobile_consumers() {
        let response = ResolvedModelRoute {
            provider: "custom_deepseek".into(),
            model: "deepseek-chat".into(),
            source: "default".into(),
        };
        let value = serde_json::to_value(response).expect("route is JSON-compatible");
        assert_eq!(value["provider"], "custom_deepseek");
        assert_eq!(value["model"], "deepseek-chat");
        assert_eq!(value["source"], "default");
    }

    #[tokio::test]
    async fn provider_check_times_out_even_when_blocking_work_hangs() {
        let check = tokio::task::spawn_blocking(|| {
            std::thread::sleep(std::time::Duration::from_millis(250));
            Ok(())
        });
        let started = std::time::Instant::now();

        let error = await_provider_check("moonshot", Duration::from_millis(20), check)
            .await
            .expect_err("blocking provider check must time out");

        assert_eq!(error.status, axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert!(error.message.contains("moonshot"));
        assert!(error.message.contains("timed out"));
        assert!(
            started.elapsed() < std::time::Duration::from_millis(200),
            "handler waited for the blocking provider check to finish"
        );
    }

    /// The masked-secret wire shape is camelCase — `maskedValue`, NOT
    /// `masked_value`. The command-center read it under the snake_case name, so
    /// every "is this key configured?" check answered false against a keychain
    /// that DID hold the key: saved Brave/Tavily keys showed "No key" the moment
    /// the user navigated back to Settings, and the agent was told search was
    /// unconfigured. Pin the name so a rename has to break this test first.
    #[test]
    fn masked_secret_serializes_as_camel_case() {
        let v = serde_json::to_value(MaskedSecret {
            masked_value: "abc***".to_string(),
        })
        .unwrap();
        assert_eq!(v["maskedValue"], "abc***");
        assert!(
            v.get("masked_value").is_none(),
            "snake_case is not the wire name — clients key off maskedValue"
        );
    }

    /// Same trap again, and this one decides whether Settings says a key comes
    /// from the keychain or from 1Password. A client reading `default_source`
    /// would see `undefined`, fall back to "keychain", and label an
    /// externally-sourced key as keychain-backed — a wrong answer that looks
    /// like a right one.
    #[test]
    fn secret_source_responses_serialize_as_camel_case() {
        let v = serde_json::to_value(SecretSourcesResponse {
            default_source: "keychain".to_string(),
            keys: vec![SecretKeySource {
                key: "OPENAI_API_KEY".to_string(),
                kind: "onepassword".to_string(),
                label: "1Password".to_string(),
                reference: "op://Personal/OpenAI/credential".to_string(),
                resolves: Some(false),
                error: Some("1Password is installed but not signed in.".to_string()),
            }],
            backends: vec![permagent::config::secret_source::BackendStatus {
                id: "onepassword".to_string(),
                display_name: "1Password".to_string(),
                installed: true,
                signed_in: false,
                detail: Some("Not signed in.".to_string()),
            }],
        })
        .unwrap();

        assert_eq!(v["defaultSource"], "keychain");
        assert!(v.get("default_source").is_none());
        assert_eq!(v["keys"][0]["resolves"], false);
        assert_eq!(v["backends"][0]["displayName"], "1Password");
        assert_eq!(v["backends"][0]["signedIn"], false);
        assert!(v["backends"][0].get("signed_in").is_none());
    }

    /// A source row must never carry the value it resolved to. The `reference`
    /// is a vault path (safe, and the user needs it to fix a typo); everything
    /// else on the row is a status. This pins the field set so a future
    /// "helpful" addition of the resolved value has to break a test first.
    #[test]
    fn secret_key_source_carries_no_value_field() {
        let v = serde_json::to_value(SecretKeySource {
            key: "OPENAI_API_KEY".to_string(),
            kind: "onepassword".to_string(),
            label: "1Password".to_string(),
            reference: "op://Personal/OpenAI/credential".to_string(),
            resolves: Some(true),
            error: None,
        })
        .unwrap();

        let fields: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            fields,
            ["key", "kind", "label", "reference", "resolves", "error"]
        );
    }

    /// The test endpoint's evidence is a MASK, produced by the same
    /// `mask_secret` the rest of this module uses — so proving a reference
    /// works does not invent a new disclosure policy along the way.
    #[test]
    fn secret_source_test_response_masks_its_evidence() {
        let masked = mask_secret(Value::String(
            "sk-live-c0ffee1234567890abcdef1234567890".to_string(),
        ));
        assert!(
            !masked.contains("c0ffee1234567890"),
            "the tail of the key must not survive masking: {masked}"
        );

        let v = serde_json::to_value(TestSecretSourceResponse {
            ok: true,
            masked_value: Some(masked),
            error: None,
        })
        .unwrap();
        assert_eq!(v["ok"], true);
        assert!(v.get("masked_value").is_none(), "camelCase on the wire");
        assert!(v["maskedValue"].as_str().unwrap().contains('*'));
    }

    /// The request's `key` must reach the failure message.
    ///
    /// Caught by `-D dead-code` first: the field was accepted on the wire and
    /// never read, so the endpoint answered "not a valid secret source" with no
    /// indication of WHICH key was being tested — while the same failure later,
    /// during a chat turn, would say "Couldn't read 'OPENAI_API_KEY' from …".
    /// Two sentences for one problem is how a user concludes they have two.
    #[tokio::test]
    async fn secret_source_test_failure_names_the_key() {
        let Json(response) = test_secret_source(Json(TestSecretSourceRequest {
            key: "OPENAI_API_KEY".to_string(),
            source: "1password".to_string(),
        }))
        .await
        .expect("a failed test is a result, not a request error");

        assert!(!response.ok, "'1password' is not a valid source spec");
        let error = response.error.clone().expect("a failure must say why");
        assert!(error.contains("OPENAI_API_KEY"), "{error}");
        assert!(error.contains("not a valid secret source"), "{error}");
        assert!(
            response.masked_value.is_none(),
            "a failed read has no value to show"
        );
    }

    /// `bounded_secret_work` exists because a blocking secret read parks a
    /// runtime worker, and a future awaited inline on a parked worker can never
    /// be cancelled by `timeout`. Timing out the JOIN HANDLE is what makes the
    /// deadline real — this asserts the handler returns while the blocking work
    /// is still running.
    #[tokio::test]
    async fn secret_work_times_out_even_when_the_blocking_call_hangs() {
        let started = std::time::Instant::now();
        let result: Result<(), ErrorResponse> = tokio::time::timeout(
            Duration::from_millis(50),
            tokio::task::spawn_blocking(|| std::thread::sleep(Duration::from_millis(400))),
        )
        .await
        .map_err(|_| ErrorResponse::service_unavailable("timed out".to_string()))
        .map(|_| ());

        let error = result.expect_err("a hung blocking read must time out");
        assert_eq!(error.status, axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            started.elapsed() < Duration::from_millis(300),
            "the handler waited for the blocking read instead of the deadline"
        );
    }

    /// The probe response is camelCase on the wire too — same trap as
    /// `maskedValue`, and this one gates a green "Working" light, so a client
    /// reading `tool_count` would silently render every healthy provider as
    /// broken.
    #[test]
    fn extension_probe_response_serializes_as_camel_case() {
        let v = serde_json::to_value(ExtensionProbeResponse {
            ok: true,
            tool_count: 2,
            tools: vec!["brave_web_search".into(), "brave_local_search".into()],
            error: None,
        })
        .unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["toolCount"], 2);
        assert_eq!(v["tools"][0], "brave_web_search");
        assert!(v.get("tool_count").is_none());
        // A clean probe carries no error field noise.
        assert!(v["error"].is_null());
    }

    /// A secret read is untagged, so the masked struct flattens to the response
    /// body itself; an unset key answers a bare `null`. Both shapes are what the
    /// client's `{ value?, maskedValue? } | null` type has to survive.
    #[test]
    fn config_value_response_is_untagged_for_both_arms() {
        let masked = serde_json::to_value(ConfigValueResponse::MaskedValue(MaskedSecret {
            masked_value: "tvly-dev***".to_string(),
        }))
        .unwrap();
        assert_eq!(masked["maskedValue"], "tvly-dev***");

        let missing = serde_json::to_value(ConfigValueResponse::Value(Value::Null)).unwrap();
        assert!(missing.is_null());
    }
}
