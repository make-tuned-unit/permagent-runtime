//! CLI handler for `permagent activity tail`.
//!
//! Connects to the daemon's WebSocket /events endpoint and streams
//! activity-channel events to stdout in human-readable or JSON format.

use anyhow::Result;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use tokio_tungstenite::tungstenite::http::Request;
use tokio_tungstenite::tungstenite::Message;

/// Run the activity tail subcommand.
pub async fn handle_tail(json: bool, filter: Option<String>, since: Option<String>) -> Result<()> {
    let daemon_url = crate::commands::daemon::daemon_ws_url()?;
    let token = crate::commands::daemon::load_daemon_token()?;

    let url = format!("{}/events", daemon_url);

    let request = Request::builder()
        .uri(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .header("Host", "127.0.0.1")
        .body(())?;

    let (ws_stream, _) = tokio_tungstenite::connect_async(request).await?;
    let (_, mut read) = ws_stream.split();

    let since_dt: Option<DateTime<Utc>> = since.and_then(|s| s.parse().ok());

    eprintln!("Connected to daemon activity stream. Press Ctrl-C to exit.");

    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                let parsed: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                // Only show activity-channel events
                let channel = parsed.get("channel").and_then(|v| v.as_str());
                if channel != Some("activity") {
                    continue;
                }

                let event = match parsed.get("event") {
                    Some(e) => e,
                    None => continue,
                };

                // Apply source filter
                if let Some(ref f) = filter {
                    let source = event
                        .get("source_surface")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !source.eq_ignore_ascii_case(f) {
                        continue;
                    }
                }

                // Apply since filter
                if let Some(cutoff) = since_dt {
                    if let Some(ts_str) = event.get("timestamp").and_then(|v| v.as_str()) {
                        if let Ok(ts) = ts_str.parse::<DateTime<Utc>>() {
                            if ts < cutoff {
                                continue;
                            }
                        }
                    }
                }

                if json {
                    println!("{}", event);
                } else {
                    print_event_human(event);
                }
            }
            Ok(Message::Close(_)) => {
                eprintln!("Daemon closed connection.");
                break;
            }
            Err(e) => {
                eprintln!("WebSocket error: {}", e);
                break;
            }
            _ => {}
        }
    }

    Ok(())
}

fn print_event_human(event: &serde_json::Value) {
    let ts = event
        .get("timestamp")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<DateTime<Utc>>().ok())
        .map(|dt| dt.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "??:??:??".into());

    let source = event
        .get("source_surface")
        .and_then(|v| v.as_str())
        .unwrap_or("?");

    let event_type = event
        .get("event_type")
        .and_then(|v| v.as_str())
        .unwrap_or("?");

    let payload = event
        .get("payload")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let summary = summarize_payload(event_type, &payload);

    println!("{} [{}] {} {}", ts, source, event_type, summary);
}

fn summarize_payload(event_type: &str, payload: &serde_json::Value) -> String {
    match event_type {
        "ChatTurnStarted" => {
            let sid = payload
                .get("session_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("session={}", sid.get(..sid.len().min(8)).unwrap_or(sid))
        }
        "ChatTurnCompleted" => {
            let dur = payload
                .get("duration_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            format!("({}ms)", dur)
        }
        "TerminalCommandStarted" | "TerminalCommandCompleted" => {
            let cmd = payload
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            format!("\"{}\"", cmd.chars().take(60).collect::<String>())
        }
        "BrowserNavigated" => {
            let url = payload.get("url").and_then(|v| v.as_str()).unwrap_or("?");
            url.chars().take(80).collect()
        }
        "ProjectSelected" => {
            let name = payload
                .get("project_name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            name.to_string()
        }
        "FileEdited" => {
            let path = payload
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            path.chars().take(60).collect()
        }
        _ => String::new(),
    }
}
