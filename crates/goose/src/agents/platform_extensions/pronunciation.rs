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
        let resp = req.send().await.map_err(|e| {
            format!(
                "NOT SAVED — could not reach the voice service: {e}. Tell the user the word \
                 was not learned; do not claim it was."
            )
        })?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(rejected_save_message(status.as_u16(), &body));
        }
        // Read the confirmation back off the store's own response rather than
        // echoing the request. The point of the read-back is that the model can
        // only report what actually persisted, so a save that silently did not
        // happen cannot be narrated as if it had.
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        if !body
            .get("saved")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            return Err(
                "NOT SAVED — the voice service did not confirm the write. Tell the user the \
                 word was not learned; do not claim it was."
                    .to_string(),
            );
        }
        let total = body
            .get("total")
            .and_then(serde_json::Value::as_u64)
            .map(|n| format!(" ({n} pronunciations now stored)"))
            .unwrap_or_default();
        Ok(vec![Content::text(format!(
            "STORED and confirmed by the lexicon{total}: '{word}' is now spoken as \
             \"{sounds_like}\", effective from your very next spoken sentence. Now read that \
             respelling back to the user and say '{word}' aloud, so they hear what was saved \
             instead of being promised it."
        ))])
    }
}

/// The 400 body is the only guidance the user (and the model) get. Swallowing
/// it — we used to return just `400 Bad Request` — is why last night's eight
/// refused saves (`speth`, `ayn`) produced retries with no hint of what would
/// work.
fn rejected_save_message(status: u16, body: &str) -> String {
    let detail = {
        let trimmed = body.trim();
        if trimmed.is_empty() {
            format!("HTTP {status}")
        } else {
            // Axum ErrorResponse is often `{"error":"..."}` — unwrap so the
            // model reads the phoneme sentence, not a JSON blob.
            serde_json::from_str::<serde_json::Value>(trimmed)
                .ok()
                .and_then(|v| {
                    v.get("error")
                        .or_else(|| v.get("message"))
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| trimmed.to_string())
        }
    };
    format!(
        "NOT SAVED — pronunciation save rejected ({status}): {detail}. \
         Tell the user the exact part that failed and a respelling made of \
         ordinary English words that sound the same (e.g. 'else peth' not \
         'el speth', 'pride in' not 'pride ayn'). Do not claim it was learned."
    )
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
                    "description": "The word or name respelled so speech can say it. Prefer REAL English words separated by spaces or hyphens: 'prop tech', 'else peth' (Elspeth), 'tear un' (Taran), 'bar tea' (Barty). Hyphens are fine. Invented junk ('jent') is still refused. Never IPA. If a save is refused, swap the offending part for a real word or a name syllable that sounds the same."
                }
            },
            "required": ["word", "sounds_like"]
        }))
        .expect("static schema");

        vec![Tool::new(
            "save_pronunciation".to_string(),
            "Save how a word is pronounced so speech says it correctly forever. CALLING THIS IS \
             REQUIRED, NOT OPTIONAL: the moment the user says the name, tells you how a word \
             is said, corrects your pronunciation, or winces at it, you MUST call this tool in \
             that same turn, before you write your reply. If you do not know a name, STOP and \
             ask them to say it, then listen — their next words are sounds_like. Replying \
             \"I'll remember that\" without calling it saves NOTHING — the correction dies \
             with the turn and the user has to teach you the same word again, which is exactly \
             what a promise-only answer feels like to them. A pronunciation is only learned \
             once this tool returns. THE RULE: never spell a word out letter by letter. Give \
             `sounds_like` only — never IPA; the speech engine derives the phonemes itself. \
             Then READ BACK the respelling this tool confirms and say the word aloud so they \
             hear that it stuck. A refused save means one of your parts is not a real word: \
             swap it for one that sounds the same and retry."
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

#[cfg(test)]
mod tests {
    use super::*;

    /// One place a model reads pronunciation guidance from. `refers_by_name` is
    /// false for the tool's own description, which is attached to the tool and
    /// says "this tool" — only prose written elsewhere has to name it.
    struct Surface {
        what: &'static str,
        text: String,
        refers_by_name: bool,
    }

    /// Every surface a model reads pronunciation guidance from, named so a
    /// failure says which one regressed.
    fn guidance_surfaces() -> Vec<Surface> {
        let tool = PronunciationClient::get_tools();
        let tool = tool
            .iter()
            .find(|t| t.name == "save_pronunciation")
            .expect("save_pronunciation tool is declared");
        vec![
            Surface {
                what: "save_pronunciation tool description",
                text: tool.description.clone().unwrap_or_default().to_string(),
                refers_by_name: false,
            },
            Surface {
                what: "pronunciation extension why_it_matters",
                text: crate::agents::platform_extensions::PLATFORM_EXTENSIONS
                    .get(EXTENSION_NAME)
                    .expect("pronunciation extension is registered")
                    .why_it_matters
                    .to_string(),
                refers_by_name: true,
            },
            Surface {
                what: "VOICE_FEATURE why_it_matters",
                text: crate::config::agent_identity::VOICE_FEATURE
                    .why_it_matters
                    .to_string(),
                refers_by_name: true,
            },
        ]
    }

    /// THE REGRESSION GUARD (2026-08-19).
    ///
    /// The tool is registered, enabled, reaches a live session's toolset, and
    /// its loopback write works — verified end to end. What failed was the
    /// *instruction*: it described calling `save_pronunciation` as the thing to
    /// do when corrected, which a model satisfies conversationally ("I'll
    /// remember that") without ever emitting a call. The user taught the same
    /// word repeatedly and nothing was ever stored.
    ///
    /// So the guidance must REQUIRE the call, not suggest it. This pins that
    /// property on every surface a model reads it from — the tool description
    /// (in every request's tool list), the extension registry, and the Voice
    /// feature brief — so weakening any one of them back to advice fails here.
    #[test]
    fn pronunciation_guidance_requires_the_call_not_just_suggests_it() {
        for Surface {
            what: surface,
            text,
            refers_by_name,
        } in guidance_surfaces()
        {
            let lower = text.to_lowercase();

            assert!(
                !refers_by_name || lower.contains("save_pronunciation"),
                "{surface} must name the tool the model has to call"
            );

            // Non-optional phrasing. Advice ("call it when…") reads as satisfied
            // by an agreeable sentence; an obligation does not.
            assert!(
                lower.contains("must call")
                    || lower.contains("requires a save_pronunciation")
                    || lower.contains("required, not optional"),
                "{surface} states the save as advice, not an obligation — a model \
                 will satisfy it by agreeing in prose and never emit a tool call. \
                 Say the call is required (\"you MUST call\" / \"REQUIRED, NOT \
                 OPTIONAL\"), in the same turn as the correction."
            );

            // The specific failure mode has to be named, or "must" is just a
            // stronger adjective on the same sentence the model already
            // rationalised its way past.
            assert!(
                lower.contains("stores nothing")
                    || lower.contains("saves nothing")
                    || lower.contains("nothing is stored"),
                "{surface} must rule out the promise-only reply explicitly — say \
                 that answering \"I'll remember that\" without the call stores \
                 nothing. This is the exact behaviour that was reported."
            );

            // Confirmation must come from the store, not from the model's word.
            assert!(
                lower.contains("read back") || lower.contains("read that respelling back"),
                "{surface} must tell the model to read back what the tool \
                 confirmed, so the user gets confirmation instead of a promise"
            );
        }
    }

    /// The registry text drifted out of sync with the schema: it told the model
    /// to save "word + sounds-like + IPA" long after the `ipa` parameter was
    /// deliberately removed, because authoring IPA you cannot hear is not
    /// something a model can be right about. No surface may ask for it again.
    #[test]
    fn a_refused_save_forwards_the_phoneme_reason_not_just_the_status() {
        let msg = rejected_save_message(
            400,
            r#"{"error":"the respelling 'el speth' contains a part that speech cannot pronounce and would spell out letter by letter: speth"}"#,
        );
        assert!(
            msg.contains("speth"),
            "the offending syllable must reach the model: {msg}"
        );
        assert!(
            msg.contains("else peth"),
            "a worked example of what would save must be in the error: {msg}"
        );
        assert!(
            msg.contains("NOT SAVED"),
            "must still forbid claiming the word was learned: {msg}"
        );
    }

    #[test]
    fn no_guidance_surface_asks_for_ipa() {
        let schema_has_ipa = PronunciationClient::get_tools()[0]
            .input_schema
            .get("properties")
            .and_then(|p| p.as_object())
            .is_some_and(|p| p.contains_key("ipa"));
        assert!(
            !schema_has_ipa,
            "the `ipa` parameter is deliberately absent from the schema"
        );

        for Surface {
            what: surface,
            text,
            ..
        } in guidance_surfaces()
        {
            let lower = text.to_lowercase();
            if !lower.contains("ipa") {
                continue;
            }
            // Mentioning IPA is fine only to forbid it.
            assert!(
                lower.contains("never ipa") || lower.contains("(never ipa"),
                "{surface} mentions IPA without forbidding it, contradicting the \
                 tool schema, which has no `ipa` parameter"
            );
        }
    }
}
