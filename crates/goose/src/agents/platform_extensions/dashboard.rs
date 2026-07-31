//! Dashboard extension — let the agent read the cards the user is looking at.
//!
//! The Home dashboard renders "manifest cards": declarative definitions the
//! daemon serves from `/api/dashboard/card-types`, each naming a data endpoint
//! that returns a normalized `{ cells, note, configured }` payload (weather via
//! Open-Meteo, system stats via sysinfo, calendar via the macOS bridge).
//!
//! Until now that data was frontend-only. Asked for the weather, the agent went
//! hunting the open web — Brave, weather.gc.ca, weather.com, DuckDuckGo, `curl`
//! — while the answer sat on the user's screen, one loopback call away, already
//! localized to the location they configured. This is the read seam for it: the
//! agent sees what the user sees.
//!
//! Generic by construction. It walks the manifest list rather than hardcoding
//! card types, so a card added to `builtin_card_manifests` (or contributed by a
//! future skill pack) is readable the day it ships, with no change here.

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use async_trait::async_trait;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool,
};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "dashboard";

const DAEMON: &str = "http://127.0.0.1:3001";

/// The daemon's own bearer token, read from the file the server loads at
/// startup. This tool runs in-process in the daemon, so reading it is
/// same-trust — it just lets the loopback call pass the auth middleware (#309).
async fn daemon_token() -> Option<String> {
    let path = crate::config::paths::Paths::data_dir()
        .join("secrets")
        .join("daemon_token.json");
    let content = tokio::fs::read_to_string(path).await.ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    Some(parsed.get("token")?.as_str()?.to_string())
}

async fn get_json(path: &str) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    let mut req = client
        .get(format!("{DAEMON}{path}"))
        .timeout(std::time::Duration::from_secs(20));
    if let Some(token) = daemon_token().await {
        req = req.bearer_auth(token);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("Failed to reach the dashboard service: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Dashboard request failed: {}", resp.status()));
    }
    resp.json()
        .await
        .map_err(|e| format!("Failed to parse the dashboard response: {e}"))
}

/// One card's manifest, reduced to what the reader needs.
struct CardRef {
    card_type: String,
    name: String,
    endpoint: String,
}

/// The manifests are serialized camelCase with `card_type` rendered as `type`.
async fn list_cards() -> Result<Vec<CardRef>, String> {
    let v = get_json("/api/dashboard/card-types").await?;
    let arr = v
        .as_array()
        .ok_or_else(|| "The card-types response was not a list".to_string())?;
    Ok(arr
        .iter()
        .filter_map(|m| {
            Some(CardRef {
                card_type: m.get("type")?.as_str()?.to_string(),
                name: m.get("name")?.as_str()?.to_string(),
                endpoint: m.get("dataEndpoint")?.as_str()?.to_string(),
            })
        })
        .collect())
}

/// Render a card's `CardData` payload as compact lines. Reports the card's own
/// `note` verbatim (that is where "needs setup" and "unavailable" live) rather
/// than inventing a reason, and never fabricates a value for an empty card.
fn format_card(card: &CardRef, data: &serde_json::Value) -> String {
    let mut out = format!("{} ({})", card.name, card.card_type);

    let cells = data.get("cells").and_then(|c| c.as_array());
    let rows: Vec<String> = cells
        .map(|cs| {
            cs.iter()
                .filter_map(|c| {
                    let label = c.get("label")?.as_str()?;
                    let value = c.get("value")?.as_str()?;
                    let sub = c.get("sub").and_then(|s| s.as_str());
                    Some(match sub {
                        Some(s) if !s.is_empty() => format!("  {label}: {value} ({s})"),
                        _ => format!("  {label}: {value}"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    if rows.is_empty() {
        let note = data
            .get("note")
            .and_then(|n| n.as_str())
            .unwrap_or("no data");
        out.push_str(&format!("\n  — {note}"));
    } else {
        out.push('\n');
        out.push_str(&rows.join("\n"));
        // A note can accompany real cells (e.g. a staleness caveat) — keep it.
        if let Some(note) = data.get("note").and_then(|n| n.as_str()) {
            if !note.is_empty() {
                out.push_str(&format!("\n  — {note}"));
            }
        }
    }
    out
}

pub struct DashboardClient {
    info: InitializeResult,
}

impl DashboardClient {
    pub fn new(_context: PlatformExtensionContext) -> Result<Self, anyhow::Error> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME.to_string(), "1.0.0".to_string())
                    .with_title("Dashboard"),
            );
        Ok(Self { info })
    }

    async fn handle_read(&self, card_type: &str) -> Result<Vec<Content>, String> {
        let cards = list_cards().await?;
        if cards.is_empty() {
            return Ok(vec![Content::text(
                "The dashboard has no cards registered.",
            )]);
        }

        let wanted = card_type.trim();
        let selected: Vec<&CardRef> = if wanted.is_empty() {
            cards.iter().collect()
        } else {
            let matched: Vec<&CardRef> = cards
                .iter()
                .filter(|c| c.card_type.eq_ignore_ascii_case(wanted))
                .collect();
            if matched.is_empty() {
                let names: Vec<&str> = cards.iter().map(|c| c.card_type.as_str()).collect();
                return Err(format!(
                    "No dashboard card called '{wanted}'. Available: {}.",
                    names.join(", ")
                ));
            }
            matched
        };

        let mut sections = Vec::with_capacity(selected.len());
        for card in selected {
            // One unreachable card must not sink the whole read — report it in
            // place and keep going, so "what's on my dashboard" still answers.
            match get_json(&card.endpoint).await {
                Ok(data) => sections.push(format_card(card, &data)),
                Err(e) => sections.push(format!(
                    "{} ({})\n  — unavailable: {e}",
                    card.name, card.card_type
                )),
            }
        }

        Ok(vec![Content::text(format!(
            "The user's dashboard, as it reads right now:\n\n{}",
            sections.join("\n\n")
        ))])
    }
}

impl DashboardClient {
    pub(crate) fn get_tools() -> Vec<Tool> {
        let schema: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "card_type": {
                    "type": "string",
                    "description": "Which card to read, e.g. 'weather', 'system_stats', 'calendar'. Omit to read every card on the dashboard."
                }
            },
            "required": []
        }))
        .expect("static schema");

        vec![Tool::new(
            "read_dashboard".to_string(),
            "Read the live cards on the user's Home dashboard — the same numbers they are looking \
             at. Covers local weather (already set to their location), system stats, and today's \
             calendar. USE THIS FIRST for anything a card already answers: asked about the \
             weather, read the weather card rather than searching the web, which is slower, needs \
             a key, and will not know where they are. Omit `card_type` to see everything on the \
             dashboard."
                .to_string(),
            schema,
        )]
    }
}

#[async_trait]
impl McpClientTrait for DashboardClient {
    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        Ok(ListToolsResult {
            tools: Self::get_tools(),
            next_cursor: None,
            meta: None,
        })
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }

    async fn call_tool(
        &self,
        _ctx: &ToolCallContext,
        name: &str,
        arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        match name {
            "read_dashboard" => {
                let card_type = arguments
                    .as_ref()
                    .and_then(|a| a.get("card_type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                match self.handle_read(card_type).await {
                    Ok(content) => Ok(CallToolResult::success(content)),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e)])),
                }
            }
            _ => Ok(CallToolResult::error(vec![Content::text(format!(
                "Unknown tool: {name}"
            ))])),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn card() -> CardRef {
        CardRef {
            card_type: "weather".to_string(),
            name: "Weather".to_string(),
            endpoint: "/api/dashboard/weather".to_string(),
        }
    }

    #[test]
    fn formats_cells_as_labelled_lines() {
        let data = json!({
            "cells": [
                { "label": "HALIFAX, NOVA SCOTIA", "value": "21° Overcast" },
                { "label": "HIGH / LOW", "value": "21° / 17°" },
                { "label": "HUMIDITY", "value": "92%" }
            ]
        });
        let out = format_card(&card(), &data);
        assert!(out.starts_with("Weather (weather)"));
        assert!(out.contains("HALIFAX, NOVA SCOTIA: 21° Overcast"));
        assert!(out.contains("HUMIDITY: 92%"));
    }

    #[test]
    fn an_empty_card_reports_its_own_note_instead_of_inventing_data() {
        let data = json!({ "cells": [], "note": "Set your location to see local weather" });
        let out = format_card(&card(), &data);
        assert!(out.contains("Set your location to see local weather"));
        // Nothing that looks like a reading is manufactured.
        assert!(!out.contains('°'));
    }

    #[test]
    fn a_note_alongside_real_cells_is_kept() {
        let data = json!({
            "cells": [{ "label": "CPU", "value": "10%" }],
            "note": "Last updated 20 minutes ago"
        });
        let out = format_card(&card(), &data);
        assert!(out.contains("CPU: 10%"));
        assert!(out.contains("Last updated 20 minutes ago"));
    }

    #[test]
    fn a_cell_subtitle_rides_along() {
        let data = json!({
            "cells": [{ "label": "Standup", "value": "09:00", "sub": "Zoom" }]
        });
        assert!(format_card(&card(), &data).contains("Standup: 09:00 (Zoom)"));
    }
}
