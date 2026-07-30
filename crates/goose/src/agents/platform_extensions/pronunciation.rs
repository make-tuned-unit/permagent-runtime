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

    async fn handle_save(&self, word: &str, sounds_like: &str) -> Result<Vec<Content>, String> {
        if word.trim().is_empty() || sounds_like.trim().is_empty() {
            return Err("word and sounds_like are both required".to_string());
        }
        let client = reqwest::Client::new();
        // No `ipa` field: the daemon derives phonemes from the respelling using
        // the same G2P that speaks. Authoring IPA here was the old contract and
        // it could not work — see provider::phonemize_text.
        let mut req = client
            .put("http://127.0.0.1:3001/voice/pronunciations")
            .timeout(std::time::Duration::from_secs(10))
            .json(&serde_json::json!({
                "word": word,
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
        // No `ipa` parameter, deliberately. It used to be required, and it could
        // not work: producing Kokoro-flavoured IPA means authoring an encoding
        // you cannot hear, so there is no way to notice being wrong. The only
        // entry ever saved that way stored "permagent" as "pʌmˈeɪdʒənt" /
        // "PUM-ay-jent" — self-consistent and confidently wrong, since the
        // product is "PER-ma-jent". A respelling is something a model IS
        // reliable at, and the daemon converts it with the same G2P that speaks,
        // so what gets stored is exactly what will be said.
        let schema: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "word": {
                    "type": "string",
                    "description": "The word or name exactly as it is written, e.g. 'proptech'"
                },
                "sounds_like": {
                    "type": "string",
                    "description": "The word respelled as ORDINARY ENGLISH WORDS OR SYLLABLES separated by spaces — this is converted to phonemes by the speech engine itself, so it must be pronounceable English. Good: 'prop tech', 'co working', 'per ma jent', 'ess cue lite'. Bad: IPA symbols, hyphens-as-syllables, or capitals for stress ('PER-ma-jent')."
                }
            },
            "required": ["word", "sounds_like"]
        }))
        .expect("static schema");

        vec![Tool::new(
            "save_pronunciation".to_string(),
            "Save how a word is pronounced so speech says it correctly forever. THE RULE: never \
             spell a word out letter by letter. If you are unsure how a name will sound, or the \
             user corrects you, ask how it is said, respell it as ordinary English words or \
             syllables, save it here, then say the word back so they can confirm. Give \
             `sounds_like` only — never IPA; the speech engine derives the phonemes itself."
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
                match self.handle_save(&arg("word"), &arg("sounds_like")).await {
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
