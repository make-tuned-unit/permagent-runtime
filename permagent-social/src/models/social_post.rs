use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SocialPostState {
    Draft,
    Scheduled,
    Posting,
    Posted,
    Failed,
}

impl SocialPostState {
    pub fn as_str(&self) -> &'static str {
        match self {
            SocialPostState::Draft => "draft",
            SocialPostState::Scheduled => "scheduled",
            SocialPostState::Posting => "posting",
            SocialPostState::Posted => "posted",
            SocialPostState::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialPost {
    pub card_id: i64,
    pub caption: String,
    pub media_paths: Vec<String>,
    pub scheduled_for: Option<DateTime<Utc>>,
    pub posted_at: Option<DateTime<Utc>>,
    pub state: String,
    pub last_error: Option<String>,
}
