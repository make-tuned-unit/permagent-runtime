use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, WebviewUrl, WebviewWindowBuilder};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const CALLBACK_ADDR: &str = "127.0.0.1:9844";

#[derive(Serialize, Deserialize, Clone)]
pub struct IntegrationStatus {
    pub provider: String,
    pub connected: bool,
    pub scopes: Option<String>,
    pub error: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    scope: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct StoredToken {
    access_token: String,
    refresh_token: Option<String>,
    expiry: Option<String>,
    scopes: String,
}

struct OAuthEndpoints {
    auth_url: &'static str,
    token_url: &'static str,
}

fn endpoints_for(provider: &str) -> Result<OAuthEndpoints, String> {
    match provider {
        "gmail" => Ok(OAuthEndpoints {
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth",
            token_url: "https://oauth2.googleapis.com/token",
        }),
        _ => Err(format!("Unknown provider: {}", provider)),
    }
}

fn secrets_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("Cannot determine home directory")?;
    Ok(home.join(".permagent").join("secrets"))
}

fn token_path(provider: &str) -> Result<PathBuf, String> {
    Ok(secrets_dir()?.join(format!("{}_token.json", provider)))
}

pub fn get_status(provider: &str) -> Result<IntegrationStatus, String> {
    let path = token_path(provider)?;
    if path.exists() {
        let data = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let token: StoredToken = serde_json::from_str(&data).map_err(|e| e.to_string())?;
        Ok(IntegrationStatus {
            provider: provider.to_string(),
            connected: true,
            scopes: Some(token.scopes),
            error: None,
        })
    } else {
        Ok(IntegrationStatus {
            provider: provider.to_string(),
            connected: false,
            scopes: None,
            error: None,
        })
    }
}

pub fn disconnect(app: &AppHandle, provider: &str) -> Result<(), String> {
    let path = token_path(provider)?;
    if path.exists() {
        fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    let _ = app.emit("integration_disconnected", provider);
    Ok(())
}

pub async fn start_oauth_flow(
    app: AppHandle,
    provider: String,
    client_id: String,
    client_secret: String,
    scopes: String,
) -> Result<String, String> {
    let endpoints = endpoints_for(&provider)?;
    let redirect_uri = format!("http://{}/callback", CALLBACK_ADDR);

    let auth_url = format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent",
        endpoints.auth_url,
        urlencoding::encode(&client_id),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(&scopes),
    );

    // Bind callback server before opening the window
    let listener = TcpListener::bind(CALLBACK_ADDR)
        .await
        .map_err(|e| format!("Failed to bind callback server on {}: {}", CALLBACK_ADDR, e))?;

    // Open 500x700 OAuth webview window
    let parsed_url: url::Url = auth_url.parse().map_err(|e: url::ParseError| e.to_string())?;
    let oauth_window = WebviewWindowBuilder::new(
        &app,
        "oauth",
        WebviewUrl::External(parsed_url),
    )
    .title(format!("Connect {}", provider))
    .inner_size(500.0, 700.0)
    .center()
    .build()
    .map_err(|e| format!("Failed to open OAuth window: {}", e))?;

    // Wait for callback (5 minute timeout)
    let code = match tokio::time::timeout(
        std::time::Duration::from_secs(300),
        wait_for_callback(&listener),
    )
    .await
    {
        Ok(Ok(code)) => code,
        Ok(Err(e)) => {
            let _ = oauth_window.close();
            return Err(format!("Callback error: {}", e));
        }
        Err(_) => {
            let _ = oauth_window.close();
            return Err("OAuth timed out after 5 minutes".to_string());
        }
    };

    let _ = oauth_window.close();

    // Exchange authorization code for tokens
    let client = reqwest::Client::new();
    let resp = client
        .post(endpoints.token_url)
        .form(&[
            ("code", code.as_str()),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
        .map_err(|e| format!("Token exchange request failed: {}", e))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Token exchange failed: {}", body));
    }

    let tokens: TokenResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response: {}", e))?;

    // Compute expiry timestamp
    let expiry = tokens.expires_in.map(|secs| {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + secs;
        ts.to_string()
    });

    let stored = StoredToken {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        expiry,
        scopes: tokens.scope.unwrap_or_else(|| scopes.clone()),
    };

    // Write token file with chmod 600
    let dir = secrets_dir()?;
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create secrets dir: {}", e))?;

    let path = token_path(&provider)?;
    let json = serde_json::to_string_pretty(&stored).map_err(|e| e.to_string())?;
    fs::write(&path, &json).map_err(|e| format!("Failed to write token file: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("Failed to set permissions: {}", e))?;
    }

    // Emit integration_connected event to frontend
    let _ = app.emit("integration_connected", &provider);

    Ok(format!("{} connected successfully", provider))
}

async fn wait_for_callback(listener: &TcpListener) -> Result<String, String> {
    let (mut stream, _) = listener
        .accept()
        .await
        .map_err(|e| format!("Accept failed: {}", e))?;

    let mut buf = vec![0u8; 4096];
    let n = stream
        .read(&mut buf)
        .await
        .map_err(|e| format!("Read failed: {}", e))?;

    let request = String::from_utf8_lossy(&buf[..n]);

    // Extract code from: GET /callback?code=XXXX&scope=... HTTP/1.1
    let code = request
        .lines()
        .next()
        .and_then(|line| {
            let path = line.split_whitespace().nth(1)?;
            let full = format!("http://localhost{}", path);
            let parsed = url::Url::parse(&full).ok()?;
            parsed
                .query_pairs()
                .find(|(k, _)| k == "code")
                .map(|(_, v)| v.to_string())
        })
        .ok_or_else(|| {
            // Check for error parameter
            let err = request
                .lines()
                .next()
                .and_then(|line| {
                    let path = line.split_whitespace().nth(1)?;
                    let full = format!("http://localhost{}", path);
                    let parsed = url::Url::parse(&full).ok()?;
                    parsed
                        .query_pairs()
                        .find(|(k, _)| k == "error")
                        .map(|(_, v)| v.to_string())
                })
                .unwrap_or_else(|| "unknown".to_string());
            format!("OAuth denied or failed: {}", err)
        })?;

    // Respond with success page
    let html = concat!(
        "<!DOCTYPE html><html><body style=\"font-family:system-ui;text-align:center;padding:60px;",
        "background:#0B1120;color:#e2e8f0\">",
        "<h2>Connected!</h2><p>You can close this window.</p>",
        "<script>setTimeout(()=>window.close(),2000)</script>",
        "</body></html>"
    );

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    let _ = stream.write_all(response.as_bytes()).await;

    Ok(code)
}
