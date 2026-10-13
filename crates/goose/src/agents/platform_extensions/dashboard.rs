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

fn daemon_base() -> String {
    super::browser::control_plane_url("")
}

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

/// First-party (non-manifest) card types with a real component today.
/// Mirrors `ui/command-center/src/components/dashboard/cards/registry.ts`'s
/// `CARD_REGISTRY` keys plus the dagK home-templates lane's new synthesis
/// cards — hand-synced the same way `DEFAULT_LAYOUT`/`default_layout` already
/// are between the frontend and daemon. `preview_dashboard_layout` and
/// `apply_dashboard_layout` validate every card `type` against this list
/// UNIONED with the live manifest list (`list_cards`, below) before packing —
/// the same "reject any type the registry doesn't have" rule
/// `mergeRegistry`/`useLayout.ts` enforce client-side, now enforced before
/// anything reaches the client (LANE brief §3).
const FIRST_PARTY_CARD_TYPES: &[&str] = &[
    "decisions",
    "stats",
    "in_flight",
    "recent",
    "timeline",
    "todos",
    "growth_results",
    "council",
    "today_focus",
    "overnight_digest",
    "fleet_kpis",
    "stalled_pattern",
    "project_health_grid",
];

async fn get_json(path: &str) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    let mut req = client
        .get(format!("{}{path}", daemon_base()))
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

/// PUT/POST with a JSON body, same auth/timeout handling as [`get_json`].
async fn send_json(
    method: reqwest::Method,
    path: &str,
    body: Option<&serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    let mut req = client
        .request(method, format!("{}{path}", daemon_base()))
        .timeout(std::time::Duration::from_secs(20));
    if let Some(token) = daemon_token().await {
        req = req.bearer_auth(token);
    }
    if let Some(b) = body {
        req = req.json(b);
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

    // ── Layout templates (dagK home-templates lane) ────────────────────────

    async fn handle_list_templates(&self) -> Result<Vec<Content>, String> {
        let templates = get_json("/api/dashboard/templates").await?;
        let arr = templates
            .as_array()
            .ok_or_else(|| "The templates response was not a list".to_string())?;
        let mut lines = vec!["The five Home layouts, plus the user's current one:".to_string()];
        for t in arr {
            let id = t.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let question = t.get("question").and_then(|v| v.as_str()).unwrap_or("");
            lines.push(format!("- {name} (`{id}`): {question}"));
        }
        lines.push("- Custom (theirs): whatever they've hand-arranged in Customize".to_string());
        Ok(vec![Content::text(lines.join("\n"))])
    }

    /// Resolve a `template_id` OR an ad-hoc `cards[]` request into a packed
    /// `DashboardLayoutData`-shaped layout, validating every card type
    /// against the merged registry first (LANE brief §3). Returns the layout
    /// alongside the names of any template card that isn't built yet, so the
    /// caller can say so instead of silently dropping it.
    async fn resolve_layout(
        template_id: Option<&str>,
        cards: Option<&[serde_json::Value]>,
    ) -> Result<(serde_json::Value, Vec<String>), String> {
        if let Some(tid) = template_id {
            let templates = get_json("/api/dashboard/templates").await?;
            let arr = templates
                .as_array()
                .ok_or_else(|| "The templates response was not a list".to_string())?;
            let tmpl = arr
                .iter()
                .find(|t| t.get("id").and_then(|v| v.as_str()) == Some(tid))
                .ok_or_else(|| {
                    format!(
                        "No template called '{tid}'. Call list_dashboard_templates for the options."
                    )
                })?;
            let tcards = tmpl
                .get("cards")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();
            let mut coming_soon = Vec::new();
            let mut real: Vec<(String, i32, i32)> = Vec::new();
            for c in &tcards {
                let ctype = c
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if c.get("comingSoon")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    coming_soon.push(ctype);
                    continue;
                }
                let w = c
                    .get("size")
                    .and_then(|s| s.get("w"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(4) as i32;
                let h = c
                    .get("size")
                    .and_then(|s| s.get("h"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(3) as i32;
                real.push((ctype, w, h));
            }
            let packed = crate::dashboard_insights::pack_dense(&real, 12);
            Ok((packed_to_layout(&packed), coming_soon))
        } else if let Some(cs) = cards {
            if cs.is_empty() {
                return Err("Provide at least one card in `cards`".to_string());
            }
            let known_manifest = list_cards().await.unwrap_or_default();
            let known: std::collections::HashSet<String> = FIRST_PARTY_CARD_TYPES
                .iter()
                .map(|s| s.to_string())
                .chain(known_manifest.iter().map(|c| c.card_type.clone()))
                .collect();
            let mut real: Vec<(String, i32, i32)> = Vec::new();
            for c in cs {
                let ctype = c
                    .get("type")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Each card needs a 'type'".to_string())?
                    .to_string();
                if !known.contains(&ctype) {
                    let mut names: Vec<&str> = known.iter().map(|s| s.as_str()).collect();
                    names.sort();
                    return Err(format!(
                        "Unknown card type '{ctype}'. Known types: {}.",
                        names.join(", ")
                    ));
                }
                let w = c.get("w").and_then(|v| v.as_i64()).unwrap_or(4);
                let h = c.get("h").and_then(|v| v.as_i64()).unwrap_or(3);
                if !(1..=12).contains(&w) {
                    return Err(format!("Card '{ctype}' width must be 1-12, got {w}"));
                }
                if h < 1 {
                    return Err(format!("Card '{ctype}' height must be at least 1, got {h}"));
                }
                real.push((ctype, w as i32, h as i32));
            }
            let packed = crate::dashboard_insights::pack_dense(&real, 12);
            Ok((packed_to_layout(&packed), Vec::new()))
        } else {
            Err("Provide either `template_id` or `cards`".to_string())
        }
    }

    /// Deterministic one-sentence justification (§4: time of day / decisions /
    /// stalled counts, no LLM). Falls back to a plain description of what the
    /// template shows when the live signal isn't available — never a
    /// fabricated number (P12).
    fn build_reason(template_id: &str, insights: Option<&serde_json::Value>) -> String {
        let pending = insights
            .and_then(|v| v.get("fleetKpis"))
            .and_then(|f| f.get("pendingDecisionCount"))
            .and_then(|n| n.as_u64());
        let stalled = insights
            .and_then(|v| v.get("fleetKpis"))
            .and_then(|f| f.get("stalledGoalCount"))
            .and_then(|n| n.as_u64());
        match template_id {
            "morning_briefing" => match pending {
                Some(n) if n > 0 => format!(
                    "{} waiting — Morning Briefing puts {} first.",
                    plural_count(n, "decision", "decisions"),
                    if n == 1 { "it" } else { "them" },
                ),
                _ => "Morning Briefing puts today's decisions and what's in flight first."
                    .to_string(),
            },
            "operator_command_center" => match stalled {
                Some(n) if n > 0 => format!(
                    "{} stalled — Operator Command Center surfaces the pattern across projects.",
                    plural_count(n, "goal is", "goals are"),
                ),
                _ => "Operator Command Center shows the state of every project at once."
                    .to_string(),
            },
            "weekly_story" => {
                "Weekly Story summarizes the week across chat, projects and memory.".to_string()
            }
            "relationships_commitments" => {
                "Relationships & Commitments surfaces who's overdue for contact and what's still owed."
                    .to_string()
            }
            "growth_and_money" => {
                "Growth & Money compares agent spend against verified outcomes.".to_string()
            }
            _ => "A layout built for what was asked.".to_string(),
        }
    }

    async fn handle_preview(
        &self,
        template_id: Option<&str>,
        cards: Option<Vec<serde_json::Value>>,
    ) -> Result<Vec<Content>, String> {
        let (layout, coming_soon) = Self::resolve_layout(template_id, cards.as_deref()).await?;
        let insights = get_json("/api/dashboard/insights").await.ok();
        let reason = match template_id {
            Some(tid) => Self::build_reason(tid, insights.as_ref()),
            None => "A layout built from the cards asked for.".to_string(),
        };

        // NEVER persists — only emits a preview event for the frontend to
        // render as a current-vs-proposed overlay. P9 (predictable adaptive
        // UIs): the user inspects and accepts before anything changes.
        crate::events::emit(crate::events::dashboard_layout_preview(
            template_id,
            &layout,
            &reason,
        ));

        let card_count = layout
            .get("cards")
            .and_then(|c| c.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let mut msg = format!(
            "Previewing {} — {reason} ({card_count} card{}). Shown as an overlay to compare \
             against the current Home layout; nothing is saved. Ask to apply it to keep it, or \
             say \"keep what I have\" / \"show me another\" to dismiss it.",
            template_id.unwrap_or("a custom layout"),
            if card_count == 1 { "" } else { "s" },
        );
        if !coming_soon.is_empty() {
            msg.push_str(&format!(
                "\n\nNot built yet, so left out of this preview: {}.",
                coming_soon.join(", ")
            ));
        }
        Ok(vec![Content::text(msg)])
    }

    async fn handle_apply(
        &self,
        template_id: Option<&str>,
        cards: Option<Vec<serde_json::Value>>,
    ) -> Result<Vec<Content>, String> {
        let (layout, coming_soon) = Self::resolve_layout(template_id, cards.as_deref()).await?;
        send_json(reqwest::Method::PUT, "/api/dashboard/layout", Some(&layout)).await?;
        let mut msg = format!(
            "Applied {}. The previous layout was saved — say \"revert my dashboard\" to go back.",
            template_id.unwrap_or("the custom layout"),
        );
        if !coming_soon.is_empty() {
            msg.push_str(&format!(
                " {} from this template aren't built yet and were left out: {}.",
                plural_count(coming_soon.len() as u64, "card", "cards"),
                coming_soon.join(", ")
            ));
        }
        Ok(vec![Content::text(msg)])
    }

    async fn handle_revert(&self) -> Result<Vec<Content>, String> {
        send_json(reqwest::Method::POST, "/api/dashboard/layout/revert", None).await?;
        Ok(vec![Content::text(
            "Reverted to the previous dashboard layout. Doing it again flips back.".to_string(),
        )])
    }
}

/// `DashboardLayoutData`-shaped JSON (`{ cards: [...] }`) from packed cells —
/// the same shape `GET/PUT /api/dashboard/layout` already speaks.
fn packed_to_layout(packed: &[crate::dashboard_insights::PackedCard]) -> serde_json::Value {
    serde_json::json!({
        "cards": packed.iter().map(|p| serde_json::json!({
            "id": p.id,
            "type": p.id,
            "position": { "x": p.x, "y": p.y },
            "size": { "w": p.w, "h": p.h },
            "visible": true,
        })).collect::<Vec<_>>(),
    })
}

/// "N word" with the given noun pluralized properly — never "1 goal(s)".
fn plural_count(n: u64, singular: &str, plural: &str) -> String {
    format!("{n} {}", if n == 1 { singular } else { plural })
}

/// Pull `template_id`/`cards` out of a tool call's raw JSON arguments.
fn layout_args_from(
    arguments: Option<&JsonObject>,
) -> (Option<String>, Option<Vec<serde_json::Value>>) {
    let template_id = arguments
        .and_then(|a| a.get("template_id"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let cards = arguments
        .and_then(|a| a.get("cards"))
        .and_then(|v| v.as_array())
        .cloned();
    (template_id, cards)
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

        let no_args: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        }))
        .expect("static schema");

        let layout_args: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "template_id": {
                    "type": "string",
                    "description": "One of the ids from list_dashboard_templates (e.g. 'morning_briefing'). Provide this OR `cards`, not both."
                },
                "cards": {
                    "type": "array",
                    "description": "An ad-hoc layout instead of a named template: a list of cards to compose and pack. Provide this OR `template_id`, not both.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "type": { "type": "string", "description": "A card type from read_dashboard's card list or the Home registry, e.g. 'decisions', 'in_flight', 'weather'." },
                            "w": { "type": "integer", "description": "Width in grid columns, 1-12. Defaults to 4." },
                            "h": { "type": "integer", "description": "Height in grid rows, >= 1. Defaults to 3." }
                        },
                        "required": ["type"]
                    }
                }
            },
            "required": []
        }))
        .expect("static schema");

        vec![
            Tool::new(
                "read_dashboard".to_string(),
                "Read the live cards on the user's Home dashboard — the same numbers they are looking \
                 at. Covers local weather (already set to their location), system stats, and today's \
                 calendar. USE THIS FIRST for anything a card already answers: asked about the \
                 weather, read the weather card rather than searching the web, which is slower, needs \
                 a key, and will not know where they are. Omit `card_type` to see everything on the \
                 dashboard."
                    .to_string(),
                schema,
            ),
            Tool::new(
                "list_dashboard_templates".to_string(),
                "List the 5 Home dashboard layout templates (Morning Briefing, Operator Command \
                 Center, Weekly Story, Relationships & Commitments, Growth & Money), each with the \
                 question it answers, plus the user's current custom layout. Call this before \
                 preview_dashboard_layout so you can name the right one."
                    .to_string(),
                no_args.clone(),
            ),
            Tool::new(
                "preview_dashboard_layout".to_string(),
                "Propose a Home dashboard layout — a named template OR an ad-hoc set of cards — \
                 WITHOUT changing anything yet. Shows the user a current-vs-proposed overlay so \
                 they can compare before committing. ALWAYS preview before apply_dashboard_layout; \
                 never call apply directly unless the user has already seen and approved a \
                 layout (e.g. they are re-applying one from earlier in the conversation). \
                 Provide exactly one of `template_id` or `cards`."
                    .to_string(),
                layout_args.clone(),
            ),
            Tool::new(
                "apply_dashboard_layout".to_string(),
                "Persist a Home dashboard layout — a named template OR an ad-hoc set of cards — \
                 replacing the user's current one. The previous layout is saved automatically so \
                 revert_dashboard_layout can undo this. ONLY call this after the user has \
                 explicitly said to apply/keep/use the layout you (or they) previewed — never as \
                 the first step, and never silently re-apply a layout later without being asked \
                 again, even if it is the same one as before."
                    .to_string(),
                layout_args,
            ),
            Tool::new(
                "revert_dashboard_layout".to_string(),
                "Undo the last apply_dashboard_layout call, restoring the Home dashboard layout \
                 that was live before it. Calling this twice in a row flips back to the layout you \
                 just reverted away from (one level of undo in either direction)."
                    .to_string(),
                no_args,
            ),
        ]
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
            "list_dashboard_templates" => match self.handle_list_templates().await {
                Ok(content) => Ok(CallToolResult::success(content)),
                Err(e) => Ok(CallToolResult::error(vec![Content::text(e)])),
            },
            "preview_dashboard_layout" => {
                let (template_id, cards) = layout_args_from(arguments.as_ref());
                match self.handle_preview(template_id.as_deref(), cards).await {
                    Ok(content) => Ok(CallToolResult::success(content)),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e)])),
                }
            }
            "apply_dashboard_layout" => {
                let (template_id, cards) = layout_args_from(arguments.as_ref());
                match self.handle_apply(template_id.as_deref(), cards).await {
                    Ok(content) => Ok(CallToolResult::success(content)),
                    Err(e) => Ok(CallToolResult::error(vec![Content::text(e)])),
                }
            }
            "revert_dashboard_layout" => match self.handle_revert().await {
                Ok(content) => Ok(CallToolResult::success(content)),
                Err(e) => Ok(CallToolResult::error(vec![Content::text(e)])),
            },
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

    // ── Layout tool helpers (dagK home-templates lane) ──────────────────────

    #[test]
    fn layout_args_from_reads_template_id_and_cards() {
        let args: JsonObject = serde_json::from_value(json!({
            "template_id": "morning_briefing",
            "cards": [{ "type": "decisions", "w": 5, "h": 4 }]
        }))
        .unwrap();
        let (template_id, cards) = layout_args_from(Some(&args));
        assert_eq!(template_id.as_deref(), Some("morning_briefing"));
        assert_eq!(cards.unwrap().len(), 1);
    }

    #[test]
    fn layout_args_from_handles_neither_being_present() {
        let (template_id, cards) = layout_args_from(None);
        assert!(template_id.is_none());
        assert!(cards.is_none());
    }

    #[test]
    fn packed_to_layout_uses_the_card_id_as_both_id_and_type() {
        let packed = vec![crate::dashboard_insights::PackedCard {
            id: "decisions".to_string(),
            x: 0,
            y: 0,
            w: 5,
            h: 4,
        }];
        let layout = packed_to_layout(&packed);
        let card = &layout["cards"][0];
        assert_eq!(card["id"], "decisions");
        assert_eq!(card["type"], "decisions");
        assert_eq!(card["size"]["w"], 5);
        assert_eq!(card["visible"], true);
    }

    #[test]
    fn build_reason_cites_a_real_pending_count_for_morning_briefing() {
        let insights = json!({ "fleetKpis": { "pendingDecisionCount": 3 } });
        let reason = DashboardClient::build_reason("morning_briefing", Some(&insights));
        assert!(reason.contains('3'));
    }

    #[test]
    fn build_reason_never_fabricates_a_number_with_no_insights() {
        let reason = DashboardClient::build_reason("morning_briefing", None);
        assert!(
            !reason.chars().any(|c| c.is_ascii_digit()),
            "no insights available — reason must not invent a count: {reason}"
        );
    }

    #[test]
    fn build_reason_cites_a_real_stalled_count_for_operator_command_center() {
        let insights = json!({ "fleetKpis": { "stalledGoalCount": 2 } });
        let reason = DashboardClient::build_reason("operator_command_center", Some(&insights));
        assert!(reason.contains('2'));
    }

    #[test]
    fn build_reason_falls_back_for_an_unknown_template_id() {
        let reason = DashboardClient::build_reason("not_a_real_template", None);
        assert!(!reason.is_empty());
    }

    #[test]
    fn build_reason_pluralizes_correctly_for_exactly_one() {
        let one = json!({ "fleetKpis": { "pendingDecisionCount": 1 } });
        let reason = DashboardClient::build_reason("morning_briefing", Some(&one));
        assert!(reason.contains("1 decision "), "{reason}");
        assert!(!reason.contains("decisions"), "{reason}");
        assert!(reason.contains("puts it first"), "{reason}");

        let goal_one = json!({ "fleetKpis": { "stalledGoalCount": 1 } });
        let goal_reason = DashboardClient::build_reason("operator_command_center", Some(&goal_one));
        assert!(goal_reason.contains("1 goal is stalled"), "{goal_reason}");
    }

    #[test]
    fn plural_count_never_writes_the_lazy_s_parenthesis() {
        assert_eq!(plural_count(1, "goal", "goals"), "1 goal");
        assert_eq!(plural_count(3, "goal", "goals"), "3 goals");
        assert_eq!(plural_count(0, "goal", "goals"), "0 goals");
    }
}
