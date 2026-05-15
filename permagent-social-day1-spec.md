# permagent-social — Day 1 Spec

## Goal

Scaffold a new Rust crate `permagent-social` inside the Permagent workspace. Day 1 scope is **foundation only**: crate structure, database schema, seed data, and passing integration tests. No OAuth, no HTTP, no posting logic — those land in Days 2–5.

## Success criteria

1. `cargo build -p permagent-social` succeeds from the workspace root.
2. `cargo test -p permagent-social` passes both integration tests.
3. Manual inspection of the test SQLite database shows three projects with their Kanban columns correctly seeded.
4. No other crate in the workspace is modified except the workspace-level `Cargo.toml` (members list only).

## Architectural decisions (locked)

- **Crate location:** `permagent-social/` inside the Permagent workspace
- **License:** Apache-2.0 (matches Permagent / Goose fork)
- **Transport (later days):** HTTP via existing Permagent daemon
- **Database:** SQLite via `sqlx` with runtime queries (not `query!` macros — avoids `DATABASE_URL` compile-time requirement)
- **Bluesky auth (later days):** Full OAuth 2.0 with DPoP
- **State machine:** Draft → Scheduled → Posting → Posted, with Failed as a sibling
- **Workflow philosophy:** Agent proposes (creates Drafts), user disposes (approves to Scheduled)
- **Default seeded projects:** Atlas Atlantic, evntally, World Litter Run

## File structure

```
permagent-social/
├── Cargo.toml
├── migrations/
│   └── 001_initial.sql
├── src/
│   ├── lib.rs
│   ├── error.rs
│   ├── db/
│   │   └── mod.rs
│   └── models/
│       ├── mod.rs
│       ├── project.rs
│       ├── card.rs
│       ├── social_post.rs
│       └── social_account.rs
└── tests/
    └── integration_db.rs
```

Plus one edit to the workspace `Cargo.toml` to add `permagent-social` to the members list.

## File contents

### `permagent-social/Cargo.toml`

```toml
[package]
name = "permagent-social"
version = "0.1.0"
edition = "2021"
license = "Apache-2.0"

[dependencies]
# Async runtime + HTTP (HTTP unused day 1, prep for day 5)
tokio = { version = "1", features = ["full"] }
axum = "0.7"

# Database
sqlx = { version = "0.8", features = ["runtime-tokio-rustls", "sqlite", "chrono", "macros", "migrate"] }

# Bluesky + OAuth (added day 2-3, prep deps now)
atrium-api = "0.24"
atrium-oauth-client = "0.5"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Error handling
thiserror = "1"
anyhow = "1"

# Crypto / keychain
keyring = "3"
aes-gcm = "0.10"
rand = "0.8"

# Time
chrono = { version = "0.4", features = ["serde"] }

# Logging
tracing = "0.1"

# Async trait
async-trait = "0.1"

[dev-dependencies]
tokio = { version = "1", features = ["test-util", "macros", "rt"] }
tempfile = "3"
```

> **Note on dependency versions:** If `atrium-api` or `atrium-oauth-client` versions specified above are yanked or unresolvable, use the latest compatible version published on crates.io and note the change in your final report. These are not exercised in Day 1 code, only declared.

### Workspace `Cargo.toml` edit

Add `"permagent-social"` to the workspace `members` array. Do not modify any other workspace-level setting.

### `permagent-social/migrations/001_initial.sql`

```sql
-- Projects: the top-level grouping
CREATE TABLE projects (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    slug TEXT UNIQUE NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    archived_at TEXT
);

CREATE INDEX idx_projects_archived ON projects(archived_at);

-- Kanban columns: per-project per-card-type
CREATE TABLE board_columns (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    card_type TEXT NOT NULL,
    name TEXT NOT NULL,
    position INTEGER NOT NULL,
    UNIQUE(project_id, card_type, name)
);

CREATE INDEX idx_columns_project_type ON board_columns(project_id, card_type);

-- Cards: the unified Kanban primitive
CREATE TABLE cards (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    card_type TEXT NOT NULL,
    column_id INTEGER NOT NULL REFERENCES board_columns(id),
    title TEXT NOT NULL,
    body TEXT,
    position INTEGER NOT NULL,
    created_by TEXT NOT NULL CHECK (created_by IN ('agent', 'user')),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_cards_project_type ON cards(project_id, card_type);
CREATE INDEX idx_cards_column ON cards(column_id, position);

-- Connected social accounts
CREATE TABLE social_accounts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    platform TEXT NOT NULL,
    handle TEXT NOT NULL,
    did TEXT,
    access_token_encrypted BLOB NOT NULL,
    refresh_token_encrypted BLOB NOT NULL,
    dpop_jwk_encrypted BLOB,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(platform, handle)
);

-- Social-post-specific payload (1:1 with cards where card_type='social_post')
CREATE TABLE social_posts (
    card_id INTEGER PRIMARY KEY REFERENCES cards(id) ON DELETE CASCADE,
    caption TEXT NOT NULL,
    media_paths TEXT NOT NULL DEFAULT '[]',
    scheduled_for TEXT,
    posted_at TEXT,
    state TEXT NOT NULL DEFAULT 'draft' CHECK (state IN ('draft', 'scheduled', 'posting', 'posted', 'failed')),
    last_error TEXT
);

CREATE INDEX idx_social_posts_state_schedule ON social_posts(state, scheduled_for);

-- One post can fan out to many platform targets
CREATE TABLE social_post_targets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    social_post_card_id INTEGER NOT NULL REFERENCES social_posts(card_id) ON DELETE CASCADE,
    account_id INTEGER NOT NULL REFERENCES social_accounts(id),
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'posting', 'posted', 'failed')),
    remote_post_id TEXT,
    remote_url TEXT,
    posted_at TEXT,
    last_error TEXT
);

CREATE INDEX idx_targets_state ON social_post_targets(state);
```

### `permagent-social/src/lib.rs`

```rust
//! Permagent Social: social media scheduling for Permagent.
//!
//! Architecture:
//! - `models/` defines domain types
//! - `db/` handles SQLite + migrations
//! - `adapters/` provides per-platform implementations (Bluesky, day 2-3)
//! - `scheduler/` runs the posting worker (day 4)
//! - `http/` exposes the HTTP API (day 5)

pub mod db;
pub mod error;
pub mod models;

pub use error::{Error, Result};
```

### `permagent-social/src/error.rs`

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid state transition: {0}")]
    InvalidTransition(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
```

### `permagent-social/src/models/mod.rs`

```rust
mod card;
mod project;
mod social_account;
mod social_post;

pub use card::{Card, CardType, CreatedBy};
pub use project::Project;
pub use social_account::SocialAccount;
pub use social_post::{SocialPost, SocialPostState};
```

### `permagent-social/src/models/project.rs`

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}
```

### `permagent-social/src/models/card.rs`

```rust
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
```

### `permagent-social/src/models/social_post.rs`

```rust
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
```

### `permagent-social/src/models/social_account.rs`

```rust
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
```

### `permagent-social/src/db/mod.rs`

```rust
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::path::Path;

use crate::Result;

pub async fn connect(db_path: &Path) -> Result<SqlitePool> {
    let url = format!("sqlite://{}?mode=rwc", db_path.display());
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await?;
    Ok(pool)
}

pub async fn migrate(pool: &SqlitePool) -> Result<()> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}

pub async fn seed_default_projects(pool: &SqlitePool) -> Result<()> {
    let projects = [
        ("atlas-atlantic", "Atlas Atlantic", "Venture studio main account"),
        ("evntally", "evntally", "Event discovery app"),
        ("world-litter-run", "World Litter Run", "Plogging community and events"),
    ];

    for (slug, name, description) in projects {
        // Skip if already exists (idempotent seeding)
        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM projects WHERE slug = ?"
        )
        .bind(slug)
        .fetch_optional(pool)
        .await?;

        if existing.is_some() {
            continue;
        }

        let project_id: i64 = sqlx::query_scalar(
            "INSERT INTO projects (slug, name, description) VALUES (?, ?, ?) RETURNING id"
        )
        .bind(slug)
        .bind(name)
        .bind(description)
        .fetch_one(pool)
        .await?;

        // Default columns for social_post type
        let social_cols = ["Draft", "Scheduled", "Posted", "Failed"];
        for (pos, col_name) in social_cols.iter().enumerate() {
            sqlx::query(
                "INSERT INTO board_columns (project_id, card_type, name, position) VALUES (?, ?, ?, ?)"
            )
            .bind(project_id)
            .bind("social_post")
            .bind(col_name)
            .bind(pos as i64)
            .execute(pool)
            .await?;
        }

        // Default columns for coding_task type (stubbed for later days)
        let code_cols = ["Backlog", "In Progress", "Review", "Done"];
        for (pos, col_name) in code_cols.iter().enumerate() {
            sqlx::query(
                "INSERT INTO board_columns (project_id, card_type, name, position) VALUES (?, ?, ?, ?)"
            )
            .bind(project_id)
            .bind("coding_task")
            .bind(col_name)
            .bind(pos as i64)
            .execute(pool)
            .await?;
        }
    }

    Ok(())
}
```

### `permagent-social/tests/integration_db.rs`

```rust
use permagent_social::db;
use tempfile::TempDir;

#[tokio::test]
async fn test_migrate_and_seed() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("test.db");

    let pool = db::connect(&db_path).await.expect("connect");
    db::migrate(&pool).await.expect("migrate");
    db::seed_default_projects(&pool).await.expect("seed");

    // Verify three projects exist
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM projects")
        .fetch_one(&pool)
        .await
        .expect("count projects");
    assert_eq!(count, 3);

    // Verify each has 4 social columns + 4 coding columns = 8
    let col_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM board_columns")
        .fetch_one(&pool)
        .await
        .expect("count columns");
    assert_eq!(col_count, 24); // 3 projects × 8 columns

    // Verify Atlas Atlantic has correct social columns in correct order
    let cols: Vec<(String, i64)> = sqlx::query_as(
        "SELECT name, position FROM board_columns 
         WHERE project_id = (SELECT id FROM projects WHERE slug = 'atlas-atlantic')
         AND card_type = 'social_post'
         ORDER BY position"
    )
    .fetch_all(&pool)
    .await
    .expect("fetch columns");

    assert_eq!(cols.len(), 4);
    assert_eq!(cols[0], ("Draft".to_string(), 0));
    assert_eq!(cols[1], ("Scheduled".to_string(), 1));
    assert_eq!(cols[2], ("Posted".to_string(), 2));
    assert_eq!(cols[3], ("Failed".to_string(), 3));
}

#[tokio::test]
async fn test_seed_is_idempotent() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("test.db");

    let pool = db::connect(&db_path).await.unwrap();
    db::migrate(&pool).await.unwrap();
    db::seed_default_projects(&pool).await.unwrap();
    db::seed_default_projects(&pool).await.unwrap(); // run twice

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM projects")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 3, "seeding should be idempotent");
}
```

## Constraints for Claude Code

- Do **not** modify any other crate in the workspace.
- Do **not** add a `main.rs`, binary target, or CLI. This is a library crate only.
- Do **not** switch from runtime `sqlx::query` to compile-time `sqlx::query!` macros — that would require setting `DATABASE_URL`.
- Do **not** add new files beyond those listed above.
- Do **not** run cargo commands beyond `cargo build -p permagent-social` and `cargo test -p permagent-social`.
- If a specified dependency version is yanked or causes resolution failure, use the latest compatible version and note the change in the final report.

## Final report Claude Code should produce

After running build and tests, report back with:

1. Output of `cargo build -p permagent-social`
2. Output of `cargo test -p permagent-social`
3. Any deviations from this spec, with reasons
4. Resolved versions of `atrium-api` and `atrium-oauth-client` selected by Cargo
5. Confirmation that the workspace `Cargo.toml` was edited only to add the new crate to the members list

## What's next (do not start)

Days 2–5 are scoped separately:

- **Day 2:** Bluesky OAuth flow with localhost callback + `client-metadata.json` deployed to `permagent.ai/.well-known/`
- **Day 3:** Bluesky posting adapter (text + media blob upload)
- **Day 4:** Scheduler worker with the Draft → Scheduled → Posting → Posted state machine
- **Day 5:** HTTP API + Command Center UI integration

Do not begin any Day 2+ work. Wait for explicit instruction after Day 1 is green.
