use anyhow::{Context, Result};
use comfy_table::{presets, ContentArrangement, Table};
use std::io::Write;
use std::path::PathBuf;

fn secrets_dir() -> PathBuf {
    dirs::home_dir()
        .expect("home dir required")
        .join(".permagent")
        .join("secrets")
}

fn config_path() -> PathBuf {
    dirs::home_dir()
        .expect("home dir required")
        .join(".permagent")
        .join("config.yaml")
}

fn ensure_secrets_dir() -> Result<()> {
    let dir = secrets_dir();
    if !dir.exists() {
        std::fs::create_dir_all(&dir).context("Failed to create ~/.permagent/secrets/")?;
        // Set permissions to 700 on the secrets directory
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
        // Write .gitignore
        let gitignore = dir.join(".gitignore");
        std::fs::write(&gitignore, "*.json\n*.yaml\n")
            .context("Failed to write secrets .gitignore")?;
    }
    Ok(())
}

/// Read the config.yaml integrations section
fn read_config_integrations() -> Result<serde_yaml::Value> {
    let path = config_path();
    if !path.exists() {
        return Ok(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
    }
    let content = std::fs::read_to_string(&path).context("Failed to read config.yaml")?;
    let doc: serde_yaml::Value =
        serde_yaml::from_str(&content).context("Failed to parse config.yaml")?;
    Ok(doc
        .get("integrations")
        .cloned()
        .unwrap_or(serde_yaml::Value::Mapping(serde_yaml::Mapping::new())))
}

/// Update or insert an integration entry in config.yaml
fn upsert_config_integration(provider: &str, enabled: bool) -> Result<()> {
    let path = config_path();
    let mut doc: serde_yaml::Value = if path.exists() {
        let content = std::fs::read_to_string(&path).context("Failed to read config.yaml")?;
        serde_yaml::from_str(&content).unwrap_or(serde_yaml::Value::Mapping(
            serde_yaml::Mapping::new(),
        ))
    } else {
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    };

    let root = doc
        .as_mapping_mut()
        .ok_or_else(|| anyhow::anyhow!("config.yaml root is not a mapping"))?;

    let integrations_key = serde_yaml::Value::String("integrations".into());
    if !root.contains_key(&integrations_key) {
        root.insert(
            integrations_key.clone(),
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
        );
    }

    let integrations = root
        .get_mut(&integrations_key)
        .unwrap()
        .as_mapping_mut()
        .ok_or_else(|| anyhow::anyhow!("integrations section is not a mapping"))?;

    let provider_key = serde_yaml::Value::String(provider.into());

    let token_path = secrets_dir()
        .join(format!("{}_token.json", provider))
        .display()
        .to_string();

    let mut entry = serde_yaml::Mapping::new();
    entry.insert(
        serde_yaml::Value::String("type".into()),
        serde_yaml::Value::String("stdio".into()),
    );
    entry.insert(
        serde_yaml::Value::String("cmd".into()),
        serde_yaml::Value::String(format!("permagent-{}-mcp", provider)),
    );
    entry.insert(
        serde_yaml::Value::String("enabled".into()),
        serde_yaml::Value::Bool(enabled),
    );
    entry.insert(
        serde_yaml::Value::String("token_path".into()),
        serde_yaml::Value::String(token_path),
    );

    integrations.insert(provider_key, serde_yaml::Value::Mapping(entry));

    let yaml = serde_yaml::to_string(&doc).context("Failed to serialize config.yaml")?;
    std::fs::write(&path, yaml).context("Failed to write config.yaml")?;
    Ok(())
}

/// Remove an integration's tokens and mark it disabled in config.yaml
fn remove_integration_tokens(provider: &str) -> Result<bool> {
    let token_file = secrets_dir().join(format!("{}_token.json", provider));
    let existed = token_file.exists();
    if existed {
        std::fs::remove_file(&token_file)
            .with_context(|| format!("Failed to remove {}", token_file.display()))?;
    }
    Ok(existed)
}

// ---- Handlers ----

pub async fn handle_integrations_list() -> Result<()> {
    let integrations = read_config_integrations()?;

    let mapping = match integrations.as_mapping() {
        Some(m) if !m.is_empty() => m,
        _ => {
            println!("No integrations configured.");
            println!();
            println!("Connect one with:");
            println!("  permagent integrations connect gmail");
            println!("  permagent integrations connect slack");
            return Ok(());
        }
    };

    let mut table = Table::new();
    table
        .load_preset(presets::UTF8_FULL_CONDENSED)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec!["Provider", "Status", "Type", "Token"]);

    for (key, value) in mapping.iter() {
        let provider = key.as_str().unwrap_or("?");
        let entry = value.as_mapping();

        let enabled = entry
            .and_then(|e| e.get("enabled"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let integration_type = entry
            .and_then(|e| e.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("stdio");

        let token_path = entry
            .and_then(|e| e.get("token_path"))
            .and_then(|v| v.as_str())
            .map(PathBuf::from);

        let token_exists = token_path
            .as_ref()
            .map(|p| p.exists())
            .unwrap_or(false);

        let status = if enabled && token_exists {
            "connected"
        } else if enabled && !token_exists {
            "no token"
        } else {
            "disconnected"
        };

        table.add_row(vec![
            provider.to_string(),
            status.to_string(),
            integration_type.to_string(),
            if token_exists { "present" } else { "missing" }.to_string(),
        ]);
    }

    println!("{}", table);
    Ok(())
}

pub async fn handle_integrations_connect(provider: &str) -> Result<()> {
    match provider {
        "gmail" => connect_gmail().await,
        "slack" => connect_slack().await,
        _ => {
            anyhow::bail!(
                "Unknown provider: '{}'. Supported providers: gmail, slack",
                provider
            );
        }
    }
}

pub async fn handle_integrations_disconnect(provider: &str) -> Result<()> {
    match provider {
        "gmail" | "slack" => {}
        _ => {
            anyhow::bail!(
                "Unknown provider: '{}'. Supported providers: gmail, slack",
                provider
            );
        }
    }

    let removed = remove_integration_tokens(provider)?;
    upsert_config_integration(provider, false)?;

    if removed {
        println!("Disconnected {} — tokens removed and integration disabled.", provider);
    } else {
        println!("Disconnected {} — integration disabled (no tokens were stored).", provider);
    }
    Ok(())
}

// ---- Gmail OAuth Flow ----

async fn connect_gmail() -> Result<()> {
    ensure_secrets_dir()?;

    println!("Connecting Gmail...");
    println!();
    println!("You need Google Cloud OAuth credentials (client_id and client_secret).");
    println!("Create them at: https://console.cloud.google.com/apis/credentials");
    println!("Required scope: https://www.googleapis.com/auth/gmail.readonly");
    println!();

    // Prompt for credentials
    print!("Client ID: ");
    std::io::stdout().flush()?;
    let mut client_id = String::new();
    std::io::stdin().read_line(&mut client_id)?;
    let client_id = client_id.trim().to_string();
    if client_id.is_empty() {
        anyhow::bail!("Client ID is required");
    }

    print!("Client Secret: ");
    std::io::stdout().flush()?;
    let mut client_secret = String::new();
    std::io::stdin().read_line(&mut client_secret)?;
    let client_secret = client_secret.trim().to_string();
    if client_secret.is_empty() {
        anyhow::bail!("Client Secret is required");
    }

    // Start local callback server
    let callback_port = 8095u16;
    let redirect_uri = format!("http://localhost:{}/callback", callback_port);
    let scopes = "https://www.googleapis.com/auth/gmail.readonly";

    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?\
         client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent",
        urlencoding::encode(&client_id),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(scopes),
    );

    println!();
    println!("Opening browser for Google authorization...");
    println!("If it doesn't open, visit this URL:");
    println!("{}", auth_url);
    println!();

    let _ = webbrowser::open(&auth_url);

    // Listen for OAuth callback using a one-shot axum server
    let auth_code = listen_for_oauth_callback(callback_port).await?;

    println!("Received authorization code, exchanging for tokens...");

    // Exchange code for tokens
    let token_response = exchange_google_code(
        &auth_code,
        &client_id,
        &client_secret,
        &redirect_uri,
    )
    .await?;

    // Save tokens
    let token_path = secrets_dir().join("gmail_token.json");
    let token_json = serde_json::json!({
        "access_token": token_response.access_token,
        "refresh_token": token_response.refresh_token,
        "token_type": token_response.token_type,
        "expires_in": token_response.expires_in,
        "scope": scopes,
    });
    std::fs::write(&token_path, serde_json::to_string_pretty(&token_json)?)
        .context("Failed to write gmail_token.json")?;

    // Set file permissions to 600
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600))?;
    }

    // Update config.yaml
    upsert_config_integration("gmail", true)?;

    println!("Gmail connected successfully!");
    println!("Token stored at: {}", token_path.display());
    Ok(())
}

// ---- Slack Connect ----

async fn connect_slack() -> Result<()> {
    ensure_secrets_dir()?;

    println!("Connecting Slack...");
    println!();
    println!("You need a Slack Bot User OAuth Token (xoxb-...).");
    println!("Create a Slack app at: https://api.slack.com/apps");
    println!("Required scopes: chat:write, channels:read, search:read, reminders:write");
    println!();

    print!("Slack Bot Token: ");
    std::io::stdout().flush()?;
    let mut token = String::new();
    std::io::stdin().read_line(&mut token)?;
    let token = token.trim().to_string();
    if token.is_empty() {
        anyhow::bail!("Slack token is required");
    }

    // Verify token with auth.test
    println!("Verifying token...");
    let client = reqwest::Client::new();
    let resp = client
        .post("https://slack.com/api/auth.test")
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("Failed to call Slack auth.test")?;

    let body: serde_json::Value = resp
        .json()
        .await
        .context("Failed to parse Slack auth.test response")?;

    if body.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let error = body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        anyhow::bail!("Slack token verification failed: {}", error);
    }

    let team = body
        .get("team")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let user = body
        .get("user")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    // Save token
    let token_path = secrets_dir().join("slack_token.json");
    let token_json = serde_json::json!({
        "access_token": token,
        "team": team,
        "user": user,
        "scopes": "chat:write,channels:read,search:read,reminders:write",
    });
    std::fs::write(&token_path, serde_json::to_string_pretty(&token_json)?)
        .context("Failed to write slack_token.json")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600))?;
    }

    // Update config.yaml
    upsert_config_integration("slack", true)?;

    println!("Slack connected successfully!");
    println!("  Team: {}", team);
    println!("  User: {}", user);
    println!("Token stored at: {}", token_path.display());
    Ok(())
}

// ---- OAuth Callback Server ----

async fn listen_for_oauth_callback(port: u16) -> Result<String> {
    use axum::{extract::Query, response::Html, routing::get, Router};
    use std::collections::HashMap;
    use tokio::sync::oneshot;

    let (tx, rx) = oneshot::channel::<String>();
    let tx = std::sync::Arc::new(tokio::sync::Mutex::new(Some(tx)));

    let app = Router::new().route(
        "/callback",
        get({
            let tx = tx.clone();
            move |Query(params): Query<HashMap<String, String>>| {
                let tx = tx.clone();
                async move {
                    if let Some(code) = params.get("code") {
                        if let Some(sender) = tx.lock().await.take() {
                            let _ = sender.send(code.clone());
                        }
                        Html("<html><body><h2>Authorization received!</h2><p>You can close this tab and return to the terminal.</p></body></html>".to_string())
                    } else {
                        let error = params.get("error").cloned().unwrap_or_else(|| "unknown".into());
                        Html(format!("<html><body><h2>Authorization failed</h2><p>Error: {}</p></body></html>", error))
                    }
                }
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port))
        .await
        .with_context(|| format!("Failed to bind OAuth callback server on port {}", port))?;

    println!("Listening for OAuth callback on http://127.0.0.1:{}...", port);

    // Spawn the server
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    // Wait for the auth code with a timeout
    let code = tokio::time::timeout(std::time::Duration::from_secs(120), rx)
        .await
        .context("OAuth callback timed out after 120 seconds")?
        .context("OAuth callback channel closed unexpectedly")?;

    server_handle.abort();
    Ok(code)
}

// ---- Google Token Exchange ----

#[derive(serde::Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    token_type: String,
    expires_in: u64,
}

async fn exchange_google_code(
    code: &str,
    client_id: &str,
    client_secret: &str,
    redirect_uri: &str,
) -> Result<GoogleTokenResponse> {
    let client = reqwest::Client::new();
    let resp = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("code", code),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
        .context("Failed to exchange authorization code")?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Token exchange failed: {}", body);
    }

    let token: GoogleTokenResponse = resp
        .json()
        .await
        .context("Failed to parse token response")?;

    Ok(token)
}
