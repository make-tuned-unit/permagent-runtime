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
    dirs::home_dir()
        .expect("home dir required")
        .join(".permagent")
        .join("secrets")
}

fn config_path() -> std::path::PathBuf {
    dirs::home_dir()
        .expect("home dir required")
        .join(".permagent")
        .join("config.yaml")
}

fn ensure_secrets_dir() -> Result<(), String> {
    let dir = secrets_dir();
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create secrets dir: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| format!("Failed to set secrets dir permissions: {e}"))?;
        }
    }
    Ok(())
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

    let redirect_uri = format!("http://localhost:{}/integrations/gmail/callback", CALLBACK_PORT);

    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?\
         client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent",
        urlencoding::encode(&req.client_id),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(GMAIL_SCOPES),
    );

    // Store credentials temporarily for the callback to use
    let creds_path = secrets_dir().join("gmail_pending_oauth.json");
    let creds_json = serde_json::json!({
        "client_id": req.client_id,
        "client_secret": req.client_secret,
        "redirect_uri": redirect_uri,
    });
    std::fs::write(&creds_path, serde_json::to_string(&creds_json).unwrap())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to store pending creds: {e}")))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&creds_path, std::fs::Permissions::from_mode(0o600));
    }

    Ok(Json(GmailConnectResponse {
        auth_url,
        callback_port: CALLBACK_PORT,
    }))
}

/// GET /integrations/gmail/callback — OAuth callback handler
async fn gmail_callback(
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let code = match params.get("code") {
        Some(c) => c.clone(),
        None => {
            let error = params.get("error").cloned().unwrap_or_else(|| "unknown".into());
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
                "<html><body><h2>Error</h2><p>Corrupted pending OAuth data.</p></body></html>".into()
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

    // Save token
    let token_path = secrets_dir().join("gmail_token.json");
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

    if let Err(e) = std::fs::write(&token_path, serde_json::to_string_pretty(&token_json).unwrap()) {
        return Html(format!(
            "<html><body><h2>Failed to save token</h2><p>{e}</p></body></html>"
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600));
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
    let token_path = secrets_dir().join("gmail_token.json");
    let existed = token_path.exists();

    if existed {
        // Attempt to revoke the token with Google
        if let Ok(data) = std::fs::read_to_string(&token_path) {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&data) {
                if let Some(token) = parsed["token"].as_str() {
                    let client = reqwest::Client::new();
                    let _ = client
                        .post("https://oauth2.googleapis.com/revoke")
                        .form(&[("token", token)])
                        .send()
                        .await;
                }
            }
        }
        std::fs::remove_file(&token_path)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to remove token: {e}")))?;
    }

    let _ = upsert_gmail_config(false);

    Ok(Json(serde_json::json!({
        "provider": "gmail",
        "disconnected": true,
        "token_removed": existed,
    })))
}

/// GET /integrations — list all integration statuses
async fn list_integrations() -> Json<Vec<IntegrationStatus>> {
    let gmail_token = secrets_dir().join("gmail_token.json").exists();
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
    let path = config_path();
    let mut doc: serde_yaml::Value = if path.exists() {
        let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        serde_yaml::from_str(&content).unwrap_or(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()))
    } else {
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    };

    let root = doc.as_mapping_mut().ok_or("config root is not a mapping")?;

    // Ensure extensions section exists
    let ext_key = serde_yaml::Value::String("extensions".into());
    if !root.contains_key(&ext_key) {
        root.insert(ext_key.clone(), serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
    }

    let extensions = root.get_mut(&ext_key).unwrap().as_mapping_mut()
        .ok_or("extensions is not a mapping")?;

    let gmail_key = serde_yaml::Value::String("gmail".into());
    let token_path = secrets_dir().join("gmail_token.json").display().to_string();

    let mut entry = serde_yaml::Mapping::new();
    entry.insert("type".into(), "stdio".into());
    entry.insert("cmd".into(), "permagent-gmail-mcp".into());
    entry.insert("enabled".into(), serde_yaml::Value::Bool(enabled));

    let mut envs = serde_yaml::Mapping::new();
    envs.insert("GMAIL_TOKEN_PATH".into(), token_path.into());
    entry.insert("envs".into(), serde_yaml::Value::Mapping(envs));

    extensions.insert(gmail_key, serde_yaml::Value::Mapping(entry));

    let yaml = serde_yaml::to_string(&doc).map_err(|e| e.to_string())?;
    std::fs::write(&path, yaml).map_err(|e| e.to_string())?;
    Ok(())
}
