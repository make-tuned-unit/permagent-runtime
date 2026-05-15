use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardType {
    SocialPost,
    CodingTask,
    Outreach,
    Lead,
    Sales,
    Note,
}

impl CardType {
    pub fn as_str(&self) -> &'static str {
        match self {
            CardType::SocialPost => "social_post",
            CardType::CodingTask => "coding_task",
            CardType::Outreach => "outreach",
            CardType::Lead => "lead",
            CardType::Sales => "sales",
            CardType::Note => "note",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreatedBy {
    Agent,
    User,
}

impl CreatedBy {
    pub fn as_str(&self) -> &'static str {
        match self {
            CreatedBy::Agent => "agent",
            CreatedBy::User => "user",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    pub id: i64,
    pub project_id: i64,
    pub card_type: String,
    pub column_id: i64,
    pub title: String,
    pub body: Option<String>,
    pub position: i64,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
