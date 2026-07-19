use crate::state::AppState;
use axum::{
    extract::Query,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use utoipa::ToSchema;

const GMAIL_SCOPES: &str = "https://www.googleapis.com/auth/gmail.readonly";
const CALLBACK_PORT: u16 = 8095;

fn secrets_dir() -> std::path::PathBuf {
    permagent::config::paths::Paths::data_dir().join("secrets")
}

fn config_path() -> std::path::PathBuf {
    dirs::home_dir()
        .expect("home dir required")
        .join(".permagent")
        .join("config.yaml")
}

fn ensure_secrets_dir() -> Result<(), String> {
    // Created 0700 from the start and re-enforced on every call, so a
    // pre-existing loose directory gets tightened rather than trusted.
    permagent::config::secure_fs::ensure_private_dir(&secrets_dir())
        .map_err(|e| format!("Failed to create secrets dir: {e}"))
}

// ---- Request / Response types ----

#[derive(Deserialize, ToSchema)]
pub struct GmailConnectRequest {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Serialize, ToSchema)]
pub struct GmailConnectResponse {
    pub auth_url: String,
    pub callback_port: u16,
}

#[derive(Serialize, ToSchema)]
pub struct IntegrationStatus {
    pub provider: String,
    pub connected: bool,
    pub token_present: bool,
}

#[derive(Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    token_type: String,
    expires_in: u64,
}

// ---- Routes ----

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/integrations/gmail/connect", post(gmail_connect))
        .route("/integrations/gmail/callback", get(gmail_callback))
        .route("/integrations/gmail", delete(gmail_disconnect))
        .route("/integrations", get(list_integrations))
        .with_state(state)
}

/// POST /integrations/gmail/connect — initiate OAuth flow, return auth URL
async fn gmail_connect(
    Json(req): Json<GmailConnectRequest>,
) -> Result<Json<GmailConnectResponse>, (StatusCode, String)> {
    ensure_secrets_dir().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let redirect_uri = format!(
        "http://localhost:{}/integrations/gmail/callback",
        CALLBACK_PORT
    );

    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?\
         client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent",
        urlencoding::encode(&req.client_id),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(GMAIL_SCOPES),
    );

    // Store credentials temporarily for the callback to use. Written
    // atomically with 0600 permissions from the first byte (contains the
    // OAuth client secret); removed again once the callback completes.
    let creds_path = secrets_dir().join("gmail_pending_oauth.json");
    let creds_json = serde_json::json!({
        "client_id": req.client_id,
        "client_secret": req.client_secret,
        "redirect_uri": redirect_uri,
    });
    permagent::config::secure_fs::write_private_file(
        &creds_path,
        serde_json::to_string(&creds_json).unwrap().as_bytes(),
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to store pending creds: {e}"),
        )
    })?;

    Ok(Json(GmailConnectResponse {
        auth_url,
        callback_port: CALLBACK_PORT,
    }))
}

/// GET /integrations/gmail/callback — OAuth callback handler
async fn gmail_callback(Query(params): Query<HashMap<String, String>>) -> impl IntoResponse {
    let code = match params.get("code") {
        Some(c) => c.clone(),
        None => {
            let error = params
                .get("error")
                .cloned()
                .unwrap_or_else(|| "unknown".into());
            return Html(format!(
                "<html><body><h2>Authorization failed</h2><p>Error: {error}</p></body></html>"
            ));
        }
    };

    // Read pending OAuth credentials
    let creds_path = secrets_dir().join("gmail_pending_oauth.json");
    let pending = match std::fs::read_to_string(&creds_path) {
        Ok(s) => s,
        Err(_) => {
            return Html(
                "<html><body><h2>Error</h2><p>No pending OAuth flow found. Please initiate connect again.</p></body></html>".into()
            );
        }
    };
    let pending: serde_json::Value = match serde_json::from_str(&pending) {
        Ok(v) => v,
        Err(_) => {
            return Html(
                "<html><body><h2>Error</h2><p>Corrupted pending OAuth data.</p></body></html>"
                    .into(),
            );
        }
    };

    let client_id = pending["client_id"].as_str().unwrap_or_default();
    let client_secret = pending["client_secret"].as_str().unwrap_or_default();
    let redirect_uri = pending["redirect_uri"].as_str().unwrap_or_default();

    // Exchange code for tokens
    let client = reqwest::Client::new();
    let resp = match client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("code", code.as_str()),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return Html(format!(
                "<html><body><h2>Token exchange failed</h2><p>{e}</p></body></html>"
            ));
        }
    };

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Html(format!(
            "<html><body><h2>Token exchange failed</h2><p>{body}</p></body></html>"
        ));
    }

    let token: GoogleTokenResponse = match resp.json().await {
        Ok(t) => t,
        Err(e) => {
            return Html(format!(
                "<html><body><h2>Failed to parse token</h2><p>{e}</p></body></html>"
            ));
        }
    };

    // Save token keyring-first (file fallback is atomic + 0600 and only used
    // when the secret store rejects the write).
    let token_json = serde_json::json!({
        "token": token.access_token,
        "refresh_token": token.refresh_token,
        "token_type": token.token_type,
        "expires_in": token.expires_in,
        "client_id": client_id,
        "client_secret": client_secret,
        "token_uri": "https://oauth2.googleapis.com/token",
        "scopes": [GMAIL_SCOPES],
    });

    if let Err(e) = permagent::config::gmail_oauth::store_token(
        &serde_json::to_string_pretty(&token_json).unwrap(),
    ) {
        return Html(format!(
            "<html><body><h2>Failed to save token</h2><p>{e}</p></body></html>"
        ));
    }

    // Update config.yaml
    let _ = upsert_gmail_config(true);

    // Clean up pending file
    let _ = std::fs::remove_file(&creds_path);

    Html(
        "<html><body><h2>Gmail connected!</h2><p>You can close this tab and return to Permagent.</p></body></html>".into()
    )
}

/// DELETE /integrations/gmail — revoke and remove tokens
async fn gmail_disconnect() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Keyring-first (migrates any legacy plaintext file on read).
    let stored = permagent::config::gmail_oauth::load_token();
    let existed = stored.is_some();

    if let Some(data) = stored {
        // Attempt to revoke the token with Google. Server-issued tokens use
        // the "token" field; CLI-issued ones historically used "access_token".
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&data) {
            let token = parsed["token"]
                .as_str()
                .or_else(|| parsed["access_token"].as_str());
            if let Some(token) = token {
                let client = reqwest::Client::new();
                let _ = client
                    .post("https://oauth2.googleapis.com/revoke")
                    .form(&[("token", token)])
                    .send()
                    .await;
            }
        }
    }

    permagent::config::gmail_oauth::delete_token().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to remove token: {e}"),
        )
    })?;

    let _ = upsert_gmail_config(false);

    Ok(Json(serde_json::json!({
        "provider": "gmail",
        "disconnected": true,
        "token_removed": existed,
    })))
}

/// GET /integrations — list all integration statuses
async fn list_integrations() -> Json<Vec<IntegrationStatus>> {
    let gmail_token = permagent::config::gmail_oauth::token_present();
    let slack_token = secrets_dir().join("slack_token.json").exists();

    Json(vec![
        IntegrationStatus {
            provider: "gmail".into(),
            connected: gmail_token,
            token_present: gmail_token,
        },
        IntegrationStatus {
            provider: "slack".into(),
            connected: slack_token,
            token_present: slack_token,
        },
    ])
}

// ---- Config helpers ----

fn upsert_gmail_config(enabled: bool) -> Result<(), String> {
    use permagent::agents::extension::Envs;
    use permagent::config::extensions::ExtensionEntry;
    use permagent::config::gmail_oauth;
    use permagent::config::{ExtensionConfig, DEFAULT_EXTENSION_TIMEOUT};

    let path = config_path();
    let mut doc: serde_yaml::Value = if path.exists() {
        let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        serde_yaml::from_str(&content)
            .unwrap_or(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()))
    } else {
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    };

    let root = doc.as_mapping_mut().ok_or("config root is not a mapping")?;

    // Ensure extensions section exists
    let ext_key = serde_yaml::Value::String("extensions".into());
    if !root.contains_key(&ext_key) {
        root.insert(
            ext_key.clone(),
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
        );
    }

    let extensions = root
        .get_mut(&ext_key)
        .unwrap()
        .as_mapping_mut()
        .ok_or("extensions is not a mapping")?;

    // Build a schema-valid extension entry (the previous hand-rolled mapping
    // was missing `name`/`args` and was skipped as malformed at load time).
    // The OAuth token is injected at spawn time from the keyring via
    // `env_keys`; `GMAIL_TOKEN_PATH` remains only as a transitional fallback
    // for tokens that have not been migrated yet.
    let legacy_token_path = gmail_oauth::legacy_token_path().display().to_string();
    let entry = ExtensionEntry {
        enabled,
        config: ExtensionConfig::Stdio {
            name: "gmail".to_string(),
            description: "Read-only Gmail access (OAuth)".to_string(),
            cmd: "permagent-gmail-mcp".to_string(),
            args: vec![],
            envs: Envs::new(HashMap::from([(
                "GMAIL_TOKEN_PATH".to_string(),
                legacy_token_path,
            )])),
            env_keys: vec![gmail_oauth::GMAIL_OAUTH_TOKEN_KEY.to_string()],
            timeout: Some(DEFAULT_EXTENSION_TIMEOUT),
            bundled: None,
            available_tools: vec![],
        },
    };

    let gmail_key = serde_yaml::Value::String("gmail".into());
    let entry_value = serde_yaml::to_value(&entry).map_err(|e| e.to_string())?;
    extensions.insert(gmail_key, entry_value);

    let yaml = serde_yaml::to_string(&doc).map_err(|e| e.to_string())?;
    std::fs::write(&path, yaml).map_err(|e| e.to_string())?;
    Ok(())
}
