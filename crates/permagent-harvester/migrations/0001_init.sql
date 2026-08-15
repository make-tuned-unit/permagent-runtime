-- All timestamps are RFC3339 TEXT values in UTC.
CREATE TABLE sources (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('git','agent_session','notes')),
    path TEXT NOT NULL,
    label TEXT NOT NULL,
    enabled INTEGER NOT NULL,
    cloud_ok INTEGER NOT NULL,
    last_scanned_at TEXT
);

CREATE TABLE signals (
    id INTEGER PRIMARY KEY,
    source_id INTEGER NOT NULL REFERENCES sources(id),
    kind TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    entities_json TEXT NOT NULL,
    project TEXT,
    salience REAL NOT NULL,
    dedupe_hash TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

CREATE TABLE trends (
    id INTEGER PRIMARY KEY,
    provider TEXT NOT NULL,
    external_id TEXT NOT NULL,
    url TEXT NOT NULL,
    title TEXT NOT NULL,
    summary TEXT NOT NULL,
    topics_json TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    heat REAL NOT NULL,
    expires_at TEXT,
    UNIQUE(provider, external_id)
);

CREATE TABLE links (
    id INTEGER PRIMARY KEY,
    signal_id INTEGER NOT NULL REFERENCES signals(id),
    trend_id INTEGER NOT NULL REFERENCES trends(id),
    score REAL NOT NULL,
    rationale TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE formats (
    id INTEGER PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    platform TEXT NOT NULL,
    weight REAL NOT NULL,
    path TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE drafts (
    id INTEGER PRIMARY KEY,
    signal_id INTEGER REFERENCES signals(id),
    trend_id INTEGER REFERENCES trends(id),
    format_id INTEGER REFERENCES formats(id),
    platform TEXT NOT NULL,
    body TEXT NOT NULL,
    critic_score REAL,
    critic_notes_json TEXT,
    status TEXT NOT NULL
        CHECK (status IN ('suggested','approved','scheduled','posted','killed')),
    scheduled_for TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE draft_revisions (
    id INTEGER PRIMARY KEY,
    draft_id INTEGER NOT NULL REFERENCES drafts(id),
    body TEXT NOT NULL,
    author TEXT NOT NULL CHECK (author IN ('agent','human')),
    created_at TEXT NOT NULL
);

CREATE TABLE runs (
    id INTEGER PRIMARY KEY,
    stage TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT NOT NULL,
    ok INTEGER NOT NULL,
    items_in INTEGER NOT NULL,
    items_out INTEGER NOT NULL,
    error TEXT,
    tokens_in INTEGER NOT NULL,
    tokens_out INTEGER NOT NULL
);

CREATE INDEX runs_stage_id_idx ON runs (stage, id DESC);
