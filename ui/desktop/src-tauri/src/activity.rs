//! Tauri command for emitting activity events to the daemon bus.
//!
//! Frontend surfaces (browser, terminal, file viewer, project picker) call
//! `emit_activity` instead of POSTing to the daemon directly. This centralizes
//! auth token management, event construction, and session context in one place.

use serde::{Deserialize, Serialize};

fn read_daemon_token() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let path = std::path::PathBuf::from(home)
        .join(".permagent/secrets/daemon_token.json");
    let content = std::fs::read_to_string(path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    parsed.get("token")?.as_str().map(|s| s.to_string())
}

#[derive(Serialize)]
struct EmitPayload {
    event_id: String,
    event_type: String,
    source_surface: String,
    timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_id: Option<String>,
    payload: serde_json::Value,
    tier: String,
}

#[derive(Deserialize, Serialize)]
pub struct EmitResult {
    pub accepted: bool,
    #[serde(default)]
    pub event_id: String,
    #[serde(default)]
    pub error: Option<String>,
}

#[tauri::command(rename_all = "snake_case")]
pub async fn emit_activity(
    event_type: String,
    source_surface: String,
    payload: serde_json::Value,
    session_id: Option<String>,
    project_id: Option<String>,
) -> Result<EmitResult, String> {
    let token = read_daemon_token().ok_or("failed to read daemon token")?;

    let event = EmitPayload {
        event_id: uuid::Uuid::new_v4().to_string(),
        event_type,
        source_surface,
        timestamp: chrono::Utc::now().to_rfc3339(),
        session_id,
        project_id,
        payload,
        tier: "ephemeral".to_string(),
    };

    let client = reqwest::Client::new();
    let resp = client
        .post("http://127.0.0.1:3001/activity/emit")
        .bearer_auth(&token)
        .json(&event)
        .send()
        .await
        .map_err(|e| format!("daemon request failed: {e}"))?;

    if resp.status().is_success() {
        let result: EmitResult = resp
            .json()
            .await
            .map_err(|e| format!("invalid response: {e}"))?;
        Ok(result)
    } else {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        Err(format!("daemon returned {status}: {body}"))
    }
}
