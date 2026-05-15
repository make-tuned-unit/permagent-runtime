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
