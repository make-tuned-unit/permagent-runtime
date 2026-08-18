pub mod handler;
pub mod manager;
pub mod pairing;
pub mod telegram;
pub mod telegram_format;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio_util::sync::CancellationToken;
use utoipa::ToSchema;

use handler::GatewayHandler;

/// Self-knowledge descriptor for the Telegram gateway — the one registered
/// [`Gateway`] today (`telegram.rs`, run by [`manager::GatewayManager`]).
/// A Surface in the self-knowledge sense: it is a way to reach the agent, like
/// voice or a paired device, not a background loop with a live status to
/// merge. Static, and honest that it is set up from the CLI — there is no
/// in-app switch for it. Registered in
/// [`crate::agents::self_knowledge::SURFACE_DESCRIPTORS`].
pub const TELEGRAM_GATEWAY_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "telegram_gateway",
        display_name: "Telegram gateway",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "A Telegram bot the user can message to reach you from any phone or desktop that has \
             Telegram — plain text and voice notes (a voice note arrives as an audio file you \
             are asked to transcribe with local tools). Each Telegram user pairs once by typing a \
             short-lived pairing code into the chat and then talks to their own session on this \
             daemon. It is set up from the terminal today, not from the app: `permagent gateway \
             start telegram --bot-token …` saves the bot token in the secret store, `permagent \
             gateway pair telegram` mints a code, and `permagent gateway status` / `permagent \
             gateway stop telegram` round it out; a saved gateway starts again with the daemon",
        why_it_matters:
            "It is the lowest-friction remote channel: no pairing URL, no tailnet, just a chat \
             the user already has open. When they want to reach you from anywhere and the iOS \
             companion is not an option, this is the answer — but be honest that enabling it is \
             a terminal step, and never invent an in-app switch for it",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[],
    };

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformUser {
    pub platform: String,
    pub user_id: String,
    pub display_name: Option<String>,
}

impl PartialEq for PlatformUser {
    fn eq(&self, other: &Self) -> bool {
        self.platform == other.platform && self.user_id == other.user_id
    }
}

impl Eq for PlatformUser {}

impl std::hash::Hash for PlatformUser {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.platform.hash(state);
        self.user_id.hash(state);
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct IncomingMessage {
    pub user: PlatformUser,
    pub text: String,
    pub platform_message_id: Option<String>,
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Attachment {
    pub filename: String,
    pub mime_type: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutgoingMessage {
    Text { body: String },
    Typing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PairingState {
    Unpaired,
    PendingCode { code: String, expires_at: i64 },
    Paired { session_id: String, paired_at: i64 },
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GatewayConfig {
    pub gateway_type: String,
    pub platform_config: serde_json::Value,
    pub max_sessions: usize,
}

#[async_trait]
#[allow(dead_code)]
pub trait Gateway: Send + Sync + 'static {
    fn gateway_type(&self) -> &str;

    async fn start(&self, handler: GatewayHandler, cancel: CancellationToken)
        -> anyhow::Result<()>;

    async fn send_message(
        &self,
        user: &PlatformUser,
        message: OutgoingMessage,
    ) -> anyhow::Result<()>;

    async fn validate_config(&self) -> anyhow::Result<()>;

    fn info(&self) -> HashMap<String, String> {
        HashMap::new()
    }
}

pub fn create_gateway(config: &mut GatewayConfig) -> anyhow::Result<std::sync::Arc<dyn Gateway>> {
    match config.gateway_type.as_str() {
        "telegram" => Ok(std::sync::Arc::new(telegram::TelegramGateway::new(config)?)),
        other => anyhow::bail!("Unknown gateway type: {}", other),
    }
}
