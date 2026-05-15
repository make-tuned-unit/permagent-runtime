use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialAccount {
    pub id: i64,
    pub platform: String,
    pub handle: String,
    pub did: Option<String>,
    // Token bytes intentionally not exposed via Serialize — handled separately by crypto layer.
    #[serde(skip)]
    pub access_token_encrypted: Vec<u8>,
    #[serde(skip)]
    pub refresh_token_encrypted: Vec<u8>,
    #[serde(skip)]
    pub dpop_jwk_encrypted: Option<Vec<u8>>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}
