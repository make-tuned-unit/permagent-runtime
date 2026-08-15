use crate::config::paths::Paths;
use crate::config::{get_enabled_extensions, Config};
use crate::session::spectral_schema::SPECTRAL_SCHEMA_VERSION;
use crate::session::SessionManager;
#[cfg(target_os = "windows")]
use crate::subprocess::SubprocessExt;
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use uuid::Uuid;

const POSTHOG_API_KEY: &str = "phc_RyX5CaY01VtZJCQyhSR5KFh6qimUy81YwxsEpotAftT";
const POSTHOG_CAPTURE_URL: &str = "https://us.i.posthog.com/capture/";

/// Config key for telemetry opt-out preference
pub const TELEMETRY_ENABLED_KEY: &str = "GOOSE_TELEMETRY_ENABLED";

static TELEMETRY_DISABLED_BY_ENV: Lazy<AtomicBool> = Lazy::new(|| {
    std::env::var("GOOSE_TELEMETRY_OFF")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
        .into()
});

/// Check if the user has made a telemetry choice.
///
/// Returns Some(true) if telemetry is enabled, Some(false) if disabled,
/// or None if the user hasn't made a choice yet.
pub fn get_telemetry_choice() -> Option<bool> {
    if TELEMETRY_DISABLED_BY_ENV.load(Ordering::Relaxed) {
        return Some(false);
    }

    let config = Config::global();
    config.get_param::<bool>(TELEMETRY_ENABLED_KEY).ok()
}

/// Check if telemetry is enabled.
///
/// Returns false if:
/// - GOOSE_TELEMETRY_OFF environment variable is set to "1" or "true"
/// - GOOSE_TELEMETRY_ENABLED config value is set to false
/// - User has not made a telemetry choice yet (opt-in required)
///
/// Returns true only if the user has explicitly opted in.
pub fn is_telemetry_enabled() -> bool {
    get_telemetry_choice().unwrap_or(false)
}

// ============================================================================
// PostHog HTTP API
// ============================================================================

#[derive(Serialize)]
struct CaptureEvent {
    api_key: &'static str,
    event: String,
    distinct_id: String,
    properties: HashMap<String, serde_json::Value>,
    timestamp: Option<String>,
}

async fn posthog_capture(
    event_name: &str,
    distinct_id: &str,
    properties: HashMap<String, serde_json::Value>,
) -> Result<(), String> {
    // Consent is enforced again at the network choke point so a future caller
    // cannot bypass opt-in by calling this helper directly.
    if !is_telemetry_enabled() {
        return Ok(());
    }

    // Bring telemetry egress under the sovereignty boundary (#327). This is the
    // single outbound choke point for all PostHog POSTs; under sovereign mode the
    // POST is HARD-SUPPRESSED (fail-closed) and, either way, the attempt is
    // recorded in the append-only egress audit log — so telemetry is no longer an
    // egress the sovereignty story can't see. An audit-write failure also
    // suppresses the POST, so no unaudited telemetry leaves the machine.
    let allowed = crate::sovereignty::guard_outbound_egress(
        crate::sovereignty::EgressKind::Telemetry,
        POSTHOG_CAPTURE_URL,
        event_name,
    )
    .await;
    if !allowed {
        return Ok(());
    }

    let payload = CaptureEvent {
        api_key: POSTHOG_API_KEY,
        event: event_name.to_string(),
        distinct_id: distinct_id.to_string(),
        properties,
        timestamp: Some(Utc::now().to_rfc3339()),
    };

    let client = reqwest::Client::new();
    client
        .post(POSTHOG_CAPTURE_URL)
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("{e}"))?;

    Ok(())
}

// ============================================================================
// Installation Tracking
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InstallationData {
    installation_id: String,
    first_seen: DateTime<Utc>,
    session_count: u32,
}

impl Default for InstallationData {
    fn default() -> Self {
        Self {
            installation_id: Uuid::new_v4().to_string(),
            first_seen: Utc::now(),
            session_count: 0,
        }
    }
}

fn installation_file_path() -> std::path::PathBuf {
    Paths::state_dir().join("telemetry_installation.json")
}

fn load_or_create_installation() -> InstallationData {
    let path = installation_file_path();

    if let Ok(contents) = fs::read_to_string(&path) {
        if let Ok(data) = serde_json::from_str::<InstallationData>(&contents) {
            return data;
        }
    }

    let data = InstallationData::default();
    save_installation(&data);
    data
}

fn save_installation(data: &InstallationData) {
    let path = installation_file_path();

    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    if let Ok(json) = serde_json::to_string_pretty(data) {
        let _ = fs::write(path, json);
    }
}

fn increment_session_count() -> InstallationData {
    let mut data = load_or_create_installation();
    data.session_count += 1;
    save_installation(&data);
    data
}

// ============================================================================
// Platform Info
// ============================================================================

fn get_platform_version() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
    }
    #[cfg(target_os = "linux")]
    {
        fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|content| {
                content
                    .lines()
                    .find(|line| line.starts_with("VERSION_ID="))
                    .map(|line| {
                        line.trim_start_matches("VERSION_ID=")
                            .trim_matches('"')
                            .to_string()
                    })
            })
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "ver"])
            .set_no_window()
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        None
    }
}

fn detect_install_method() -> String {
    let exe_path = std::env::current_exe().ok();

    if let Some(path) = exe_path {
        let path_str = path.to_string_lossy().to_lowercase();

        if path_str.contains("homebrew") || path_str.contains("/opt/homebrew") {
            return "homebrew".to_string();
        }
        if path_str.contains(".cargo") {
            return "cargo".to_string();
        }
        if path_str.contains("applications") || path_str.contains(".app") {
            return "desktop".to_string();
        }
    }

    if std::env::var("GOOSE_DESKTOP").is_ok() {
        return "desktop".to_string();
    }

    "binary".to_string()
}

fn is_dev_mode() -> bool {
    cfg!(debug_assertions)
}

// ============================================================================
// Session Context (set by CLI/Desktop at startup)
// ============================================================================

static SESSION_INTERFACE: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));
static SESSION_IS_RESUMED: AtomicBool = AtomicBool::new(false);

pub fn set_session_context(interface: &str, is_resumed: bool) {
    if let Ok(mut iface) = SESSION_INTERFACE.lock() {
        *iface = Some(interface.to_string());
    }
    SESSION_IS_RESUMED.store(is_resumed, Ordering::Relaxed);
}

fn get_session_interface() -> String {
    SESSION_INTERFACE
        .lock()
        .ok()
        .and_then(|i| i.clone())
        .unwrap_or_else(|| "unknown".to_string())
}

fn get_session_is_resumed() -> bool {
    SESSION_IS_RESUMED.load(Ordering::Relaxed)
}

// ============================================================================
// Property Helpers
// ============================================================================

fn insert(
    props: &mut HashMap<String, serde_json::Value>,
    key: &str,
    val: impl Into<serde_json::Value>,
) {
    props.insert(key.to_string(), val.into());
}

// ============================================================================
// Telemetry Events
// ============================================================================

pub fn emit_session_started() {
    if !is_telemetry_enabled() {
        return;
    }

    let installation = increment_session_count();

    tokio::spawn(async move {
        let _ = send_session_event(&installation).await;
    });
}

#[derive(Default, Clone)]
pub struct ErrorContext {
    pub component: Option<String>,
    pub action: Option<String>,
    pub error_message: Option<String>,
}

pub fn emit_error(error_type: &str, error_message: &str) {
    emit_error_with_context(
        error_type,
        ErrorContext {
            error_message: Some(error_message.to_string()),
            ..Default::default()
        },
    );
}

pub fn emit_error_with_context(error_type: &str, context: ErrorContext) {
    if !is_telemetry_enabled() {
        return;
    }

    // Temporarily disabled - only session_started events are sent
    let _ = (&error_type, &context);
    return;

    #[allow(unreachable_code)]
    let installation = load_or_create_installation();
    let error_type = error_type.to_string();

    tokio::spawn(async move {
        let _ = send_error_event(&installation, &error_type, context).await;
    });
}

pub fn emit_custom_slash_command_used() {
    if !is_telemetry_enabled() {
        return;
    }

    // Temporarily disabled - only session_started events are sent
    return;

    #[allow(unreachable_code)]
    let installation = load_or_create_installation();

    tokio::spawn(async move {
        let _ = send_custom_slash_command_event(&installation).await;
    });
}

async fn send_error_event(
    installation: &InstallationData,
    error_type: &str,
    context: ErrorContext,
) -> Result<(), String> {
    let mut props = HashMap::new();

    insert(&mut props, "error_type", error_type);
    insert(&mut props, "error_category", classify_error(error_type));
    insert(&mut props, "source", "backend");
    insert(&mut props, "version", env!("CARGO_PKG_VERSION"));
    insert(&mut props, "interface", get_session_interface());
    insert(&mut props, "os", std::env::consts::OS);
    insert(&mut props, "arch", std::env::consts::ARCH);

    if let Some(component) = &context.component {
        insert(&mut props, "component", component.as_str());
    }
    if let Some(action) = &context.action {
        insert(&mut props, "action", action.as_str());
    }
    if let Some(error_message) = &context.error_message {
        insert(&mut props, "error_message", sanitize_string(error_message));
    }

    if let Some(platform_version) = get_platform_version() {
        insert(&mut props, "platform_version", platform_version);
    }

    let config = Config::global();
    if let Ok(provider) = config.get_param::<String>("GOOSE_PROVIDER") {
        insert(&mut props, "provider", provider);
    }
    if let Ok(model) = config.get_param::<String>("GOOSE_MODEL") {
        insert(&mut props, "model", model);
    }

    posthog_capture("error", &installation.installation_id, props).await
}

async fn send_custom_slash_command_event(installation: &InstallationData) -> Result<(), String> {
    let mut props = HashMap::new();

    insert(&mut props, "source", "backend");
    insert(&mut props, "version", env!("CARGO_PKG_VERSION"));
    insert(&mut props, "interface", get_session_interface());
    insert(&mut props, "os", std::env::consts::OS);
    insert(&mut props, "arch", std::env::consts::ARCH);

    if let Some(platform_version) = get_platform_version() {
        insert(&mut props, "platform_version", platform_version);
    }

    posthog_capture(
        "custom_slash_command_used",
        &installation.installation_id,
        props,
    )
    .await
}

async fn send_session_event(installation: &InstallationData) -> Result<(), String> {
    let mut props = HashMap::new();

    insert(&mut props, "os", std::env::consts::OS);
    insert(&mut props, "arch", std::env::consts::ARCH);
    insert(&mut props, "version", env!("CARGO_PKG_VERSION"));
    insert(&mut props, "is_dev", is_dev_mode());

    if let Some(platform_version) = get_platform_version() {
        insert(&mut props, "platform_version", platform_version);
    }

    insert(&mut props, "install_method", detect_install_method());
    insert(&mut props, "interface", get_session_interface());
    insert(&mut props, "is_resumed", get_session_is_resumed());
    insert(&mut props, "session_number", installation.session_count);

    let days_since_install = (Utc::now() - installation.first_seen).num_days();
    insert(&mut props, "days_since_install", days_since_install);

    let config = Config::global();
    if let Ok(provider) = config.get_param::<String>("GOOSE_PROVIDER") {
        insert(&mut props, "provider", provider);
    }
    if let Ok(model) = config.get_param::<String>("GOOSE_MODEL") {
        insert(&mut props, "model", model);
    }

    if let Ok(mode) = config.get_param::<String>("GOOSE_MODE") {
        insert(&mut props, "setting_mode", mode);
    }
    if let Ok(max_turns) = config.get_param::<i64>("GOOSE_MAX_TURNS") {
        insert(&mut props, "setting_max_turns", max_turns);
    }

    let extensions = get_enabled_extensions();
    insert(&mut props, "extensions_count", extensions.len() as u64);
    let extension_names: Vec<String> = extensions.iter().map(|e| e.name()).collect();
    insert(
        &mut props,
        "extensions",
        serde_json::Value::Array(
            extension_names
                .into_iter()
                .map(serde_json::Value::String)
                .collect(),
        ),
    );

    insert(
        &mut props,
        "db_schema_version",
        SPECTRAL_SCHEMA_VERSION as u64,
    );

    let session_manager = SessionManager::instance();
    if let Ok(insights) = session_manager.get_insights().await {
        insert(&mut props, "total_sessions", insights.total_sessions as u64);
        insert(&mut props, "total_tokens", insights.total_tokens as u64);
    }

    posthog_capture("session_started", &installation.installation_id, props).await
}

// ============================================================================
// Error Classification
// ============================================================================
pub fn classify_error(error: &str) -> &'static str {
    let error_lower = error.to_lowercase();

    if error_lower.contains("network") || error_lower.contains("fetch") {
        return "network_error";
    }
    if error_lower.contains("timeout") {
        return "timeout";
    }
    if error_lower.contains("rate") && error_lower.contains("limit") {
        return "rate_limit";
    }
    if error_lower.contains("auth")
        || error_lower.contains("unauthorized")
        || error_lower.contains("401")
    {
        return "auth_error";
    }
    if error_lower.contains("permission") || error_lower.contains("403") {
        return "permission_error";
    }
    if error_lower.contains("not found") || error_lower.contains("404") {
        return "not_found";
    }
    if error_lower.contains("provider") {
        return "provider_error";
    }
    if error_lower.contains("config") {
        return "config_error";
    }
    if error_lower.contains("extension") {
        return "extension_error";
    }
    if error_lower.contains("database") || error_lower.contains("db") || error_lower.contains("sql")
    {
        return "database_error";
    }
    if error_lower.contains("migration") {
        return "migration_error";
    }
    if error_lower.contains("render") || error_lower.contains("react") {
        return "render_error";
    }
    if error_lower.contains("chunk") || error_lower.contains("module") {
        return "module_error";
    }

    "unknown_error"
}

// ============================================================================
// Privacy Sanitization
// ============================================================================

/// Redact known-sensitive substrings before any telemetry leaves the machine.
/// Delegates to the shared, non-feature-gated redactor (`crate::privacy::redact`)
/// so telemetry and the crash-report export scrub through exactly one pattern
/// set — a single source of truth (#327).
fn sanitize_string(s: &str) -> String {
    crate::privacy::redact(s)
}

fn sanitize_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => serde_json::Value::String(sanitize_string(&s)),
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(sanitize_value).collect())
        }
        serde_json::Value::Object(obj) => serde_json::Value::Object(
            obj.into_iter()
                .map(|(k, v)| (k, sanitize_value(v)))
                .collect(),
        ),
        other => other,
    }
}

// ============================================================================
// Generic Event API (for frontend)
// ============================================================================

/// The SINGLE consent gate for the outbound analytics beacon (#852).
///
/// Returns `true` only when the user has EXPLICITLY opted in
/// ([`is_telemetry_enabled`], default OFF). Out of the box — no opt-in — it is
/// `false` for EVERY event, including `onboarding_*` and
/// `telemetry_preference_set`. Those formerly BYPASSED the gate to track the
/// onboarding funnel before consent; that bypass is removed so a fresh install
/// makes ZERO analytics network calls.
///
/// Telemetry canon: this PostHog beacon is a stopgap slated for
/// removal/replacement (never PostHog; self-hostable / privacy-first; opt-in
/// only). Until then every send funnels through this gate AND the sovereignty
/// egress guard (see [`posthog_capture`]). Local error logging (tracing/logs)
/// is untouched — only the outbound analytics beacon is gated.
fn analytics_beacon_allowed() -> bool {
    is_telemetry_enabled()
}

pub async fn emit_event(
    event_name: &str,
    mut properties: HashMap<String, serde_json::Value>,
) -> Result<(), String> {
    // #852 — NO analytics beacon without an explicit opt-in. Every send path,
    // including the formerly-bypassing onboarding_* / telemetry_preference_set
    // events, now respects the consent gate. A fresh, un-opted-in install sends
    // NOTHING to PostHog / any analytics endpoint from here.
    if !analytics_beacon_allowed() {
        return Ok(());
    }

    let installation = load_or_create_installation();

    insert(&mut properties, "os", std::env::consts::OS);
    insert(&mut properties, "arch", std::env::consts::ARCH);
    insert(&mut properties, "version", env!("CARGO_PKG_VERSION"));
    insert(&mut properties, "interface", "desktop");
    insert(&mut properties, "source", "ui");

    if let Some(platform_version) = get_platform_version() {
        insert(&mut properties, "platform_version", platform_version);
    }

    // NOTE (#327): crash/error events (`app_crashed`/`error_occurred`) are NOT
    // sent through this analytics beacon. Telemetry canon is self-hostable /
    // privacy-first only (never PostHog for crashes); crash reporting is handled
    // by the local redacted export, not this path.

    let sanitized: HashMap<String, serde_json::Value> = properties
        .into_iter()
        .filter(|(key, _)| {
            let key_lower = key.to_lowercase();
            !key_lower.contains("key")
                && !key_lower.contains("token")
                && !key_lower.contains("secret")
                && !key_lower.contains("password")
                && !key_lower.contains("credential")
        })
        .map(|(k, v)| (k, sanitize_value(v)))
        .collect();

    posthog_capture(event_name, &installation.installation_id, sanitized).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// Out of the box (no opt-in) the consent gate is CLOSED for every event —
    /// including the `onboarding_*` and `telemetry_preference_set` events that
    /// used to bypass it (#852). The gate is consent, not event class.
    #[test]
    #[serial]
    fn no_beacon_allowed_for_any_event_without_opt_in() {
        // Fresh, isolated config with NO telemetry choice recorded (default OFF)
        // and no env override forcing the value either way.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().display().to_string();
        let _env = env_lock::lock_env([
            ("HOME", Some(root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(root.as_str())),
            ("GOOSE_TELEMETRY_OFF", None),
        ]);

        assert!(
            !is_telemetry_enabled(),
            "a fresh install has telemetry OFF by default (explicit opt-in required)"
        );
        // The former name-based bypass is gone: consent gates ALL of these.
        assert!(!analytics_beacon_allowed());
    }

    /// End-to-end proof that `emit_event` sends NOTHING without an opt-in — even
    /// for the events that formerly bypassed the gate. The sovereignty egress
    /// guard is the single choke point every PostHog POST must pass, and it
    /// writes a `telemetry` audit row BEFORE the network send (whether the send
    /// is allowed or suppressed). So zero `telemetry` egress rows after these
    /// calls proves `posthog_capture` was never reached for any event.
    #[tokio::test]
    #[serial]
    async fn emit_event_sends_nothing_without_opt_in_including_onboarding() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().display().to_string();
        let _env = env_lock::lock_env([
            ("HOME", Some(root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(root.as_str())),
            ("GOOSE_TELEMETRY_OFF", None),
        ]);

        assert!(!is_telemetry_enabled(), "telemetry OFF by default");

        // Formerly-bypassing events + a normal one: all must be no-ops.
        emit_event("onboarding_started", HashMap::new())
            .await
            .unwrap();
        emit_event("onboarding_completed", HashMap::new())
            .await
            .unwrap();
        emit_event("telemetry_preference_set", HashMap::new())
            .await
            .unwrap();
        emit_event("session_started", HashMap::new()).await.unwrap();

        // No PostHog POST was attempted for ANY of them: the egress guard (the
        // single choke point) recorded zero telemetry attempts.
        let rows = crate::sovereignty::recent_egress(1000)
            .await
            .unwrap_or_default();
        let telemetry_attempts = rows.iter().filter(|r| r.kind == "telemetry").count();
        assert_eq!(
            telemetry_attempts, 0,
            "no analytics beacon may be attempted without an explicit opt-in"
        );
    }

    /// The final network choke point must independently enforce consent, even
    /// when a caller bypasses the public event helpers' checks.
    #[tokio::test]
    #[serial]
    async fn posthog_capture_is_a_no_op_without_opt_in() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().display().to_string();
        let _env = env_lock::lock_env([
            ("HOME", Some(root.as_str())),
            ("PERMAGENT_PATH_ROOT", Some(root.as_str())),
            ("GOOSE_TELEMETRY_OFF", None),
        ]);

        assert!(!is_telemetry_enabled(), "telemetry OFF by default");
        posthog_capture("direct_internal_call", "test-installation", HashMap::new())
            .await
            .unwrap();

        let rows = crate::sovereignty::recent_egress(1000)
            .await
            .unwrap_or_default();
        assert_eq!(
            rows.iter().filter(|r| r.kind == "telemetry").count(),
            0,
            "the consent guard must return before any egress attempt"
        );
    }
}
