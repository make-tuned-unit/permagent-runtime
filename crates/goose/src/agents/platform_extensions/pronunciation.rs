//! Pronunciation extension — the never-spell-it-out rule's tool.
//!
//! When the agent is unsure how a word will be SPOKEN by TTS (coined names,
//! brands, people), the rule is: never spell it aloud — ask the user how it's
//! pronounced, save it here once, and it is spoken correctly forever. The
//! daemon applies saved entries on every synthesis call (voice/user_lexicon).

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use async_trait::async_trait;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool,
};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "pronunciation";

/// The daemon's own bearer token, read from the same file the server loads it
/// from at startup (secrets/daemon_token.json). This tool runs in-process in
/// the daemon, so reading it is same-trust — it just lets the loopback HTTP
/// call pass the auth middleware.
async fn daemon_token() -> Option<String> {
    let path = crate::config::paths::Paths::data_dir()
        .join("secrets")
        .join("daemon_token.json");
    let content = tokio::fs::read_to_string(path).await.ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    Some(parsed.get("token")?.as_str()?.to_string())
}

pub struct PronunciationClient {
    info: InitializeResult,
}

impl PronunciationClient {
    pub fn new(_context: PlatformExtensionContext) -> Result<Self, anyhow::Error> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME.to_string(), "1.0.0".to_string())
                    .with_title("Pronunciation"),
            );
        Ok(Self { info })
    }

    async fn handle_save(
        &self,
        word: &str,
        sounds_like: &str,
        ipa: &str,
    ) -> Result<Vec<Content>, String> {
        if word.trim().is_empty() || ipa.trim().is_empty() {
            return Err("word and ipa are both required".to_string());
        }
        let client = reqwest::Client::new();
        let mut req = client
            .put("http://127.0.0.1:3001/voice/pronunciations")
            .timeout(std::time::Duration::from_secs(5))
            .json(&serde_json::json!({
                "word": word,
                "ipa": ipa,
                "sounds_like": sounds_like,
            }));
        // /voice/pronunciations sits behind the daemon's bearer choke point
        // (#309); an unauthenticated loopback call is a guaranteed 401.
        if let Some(token) = daemon_token().await {
            req = req.bearer_auth(token);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("Failed to reach the voice service: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("Pronunciation save rejected: {}", resp.status()));
        }
        Ok(vec![Content::text(format!(
            "Saved: '{word}' will be pronounced \"{sounds_like}\" from the very next sentence. \
             Say it aloud now so the user can confirm it sounds right."
        ))])
    }
}

impl PronunciationClient {
    pub(crate) fn get_tools() -> Vec<Tool> {
        let schema: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "word": {
                    "type": "string",
                    "description": "The word or name exactly as it is written, e.g. 'Permagent'"
                },
                "sounds_like": {
                    "type": "string",
                    "description": "Human respelling for display, e.g. 'PER-ma-jent'"
                },
                "ipa": {
                    "type": "string",
                    "description": "IPA phonemes in the Kokoro/misaki style the TTS engine speaks, derived from how the user said it, e.g. 'pˈɜːməʤɛnt'"
                }
            },
            "required": ["word", "sounds_like", "ipa"]
        }))
        .expect("static schema");

        vec![Tool::new(
            "save_pronunciation".to_string(),
            "Save how a word is pronounced so speech says it correctly forever. THE RULE: never \
             spell a word out loud — if you are unsure how a name will sound (or the user \
             corrects your pronunciation), ask them to say it, derive the IPA from their \
             pronunciation, save it here, then say the word back so they can confirm."
                .to_string(),
            schema,
        )]
    }
}

#[async_trait]
impl McpClientTrait for PronunciationClient {
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
        let arg = |key: &str| -> String {
            arguments
                .as_ref()
                .and_then(|a| a.get(key))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        match name {
            "save_pronunciation" => {
                match self
                    .handle_save(&arg("word"), &arg("sounds_like"), &arg("ipa"))
                    .await
                {
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
