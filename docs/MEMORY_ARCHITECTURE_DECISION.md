# Memory Architecture Decision Record

*Date: 2026-05-03. Baseline commit: ec0ca72787.*

## Problem Statement

Permagent has two distinct memory systems that both use the word "memory" but serve different purposes, live in different databases, and are wired to different code paths. Before any feature work touches memory, we need a clear architectural decision on which system is canonical, what each is for, and whether both are needed.

---

## System A: The Spectral Brain

**Location:** `~/.permagent/brain/`

**Files:**
| File | Size | Purpose |
|------|------|---------|
| `memory.db` | 49 MB | SQLite — memories, FTS, constellation fingerprints |
| `graph.kz` + `.wal` | 7.7 MB | Compressed knowledge graph (binary format, managed by spectral crate) |
| `ontology.toml` | 49 KB | Entity taxonomy (persons, projects, tools with aliases) |
| `brain.id` | 64 B | SHA-256 identity hash |
| `brain.key` / `brain.pub` | 32 B each | Cryptographic keypair (likely Ed25519 for device identity) |

**Schema (`memory.db`):**
```sql
CREATE TABLE memories (
    id TEXT PRIMARY KEY, key TEXT NOT NULL UNIQUE,
    content TEXT NOT NULL, category TEXT DEFAULT 'core',
    wing TEXT, hall TEXT,
    signal_score REAL DEFAULT 0.5, visibility TEXT DEFAULT 'private',
    created_at TEXT, updated_at TEXT,
    source TEXT, device_id BLOB, confidence REAL DEFAULT 1.0,
    last_reinforced_at TEXT
);

CREATE TABLE constellation_fingerprints (
    id TEXT PRIMARY KEY, fingerprint_hash TEXT NOT NULL,
    anchor_memory_id TEXT, target_memory_id TEXT,
    wing TEXT, anchor_hall TEXT, target_hall TEXT,
    time_delta_bucket TEXT, created_at TEXT
);

CREATE TABLE memory_spectrogram (
    memory_id TEXT PRIMARY KEY,
    entity_density REAL, action_type TEXT,
    decision_polarity REAL, causal_depth REAL,
    emotional_valence REAL, temporal_specificity REAL,
    novelty REAL, peak_dimensions TEXT, created_at TEXT
);
```

**Scale (as of audit):**
- 1,002 memories (date range: 2026-03-23 to 2026-05-03)
- 140,245 constellation fingerprints (2,136 created today alone from test session)
- 0 memory_spectrogram rows (designed but unpopulated)

**Memory sources:**
| Source | Count | Description |
|--------|-------|-------------|
| `import-core` | 337 | Bulk-imported foundational knowledge |
| `import-task` | 263 | Task records from prior system |
| `import-conversation` | 222 | Historical conversation summaries |
| `chat` | 67 | Live chat turns from running daemon |
| `import-openbird` | 65 | External project data |
| `import-daily` | 36 | Daily summaries |
| `scheduled` | 4 | Memories from scheduled recipe sessions |
| Other imports | 8 | Various smaller imports |

**Taxonomy:** Wing-based spatial organization. 28 wings observed, including `jesse` (287), `general` (239), `henry-infra` (152), `permagent` (97), `getladle` (61). No `hall` or `room` used in Brain memories. The ontology.toml defines entity types (person, project, tool) with canonical names and aliases.

### How it's populated

**Live write path** — Two identical code blocks in the daemon:

1. **`crates/goose-server/src/routes/reply.rs:477-528`** (legacy `/reply` endpoint)
2. **`crates/goose-server/src/routes/session_events.rs:640-685`** (new `/sessions/{id}/reply` endpoint)

Both do the same thing: after each chat turn completes, if both user and assistant text are non-empty, fire a `tokio::spawn` that calls:
```rust
brain.remember_with(&key, &content, spectral::RememberOpts {
    source: Some("chat".into()),
    device_id: Some(device_id),
    confidence: Some(1.0),
    visibility: spectral::Visibility::Private,
})
```
The key is `chat-{session_id}-{turn_idx}`. Content is `"User: {text}\nAssistant: {text}"`.

**Bulk import path** — The 935 `import-*` memories were loaded by an external process (likely a migration script or CLI tool not in the current codebase). They pre-date the daemon's write path.

**Constellation fingerprints** — Generated automatically by the `spectral` crate's `remember_with()` method. Every new memory triggers fingerprint computation against related memories in the same wing. This is an internal operation of the spectral library — no application code manages fingerprints.

### How it's consumed

1. **Brain recall on every chat turn** (`reply.rs:312-349`, `session_events.rs:466-520`): Before the model is invoked, the user's message is used to query `brain.recall()`. Top-3 hits with `signal_score >= 0.7` are injected into the system prompt as "Relevant memories from past context."

2. **Brain search API** (`routes/brain.rs:91-137`): `GET /api/brain/search?q=...` calls `brain.recall()` and merges results (source: `"memory"`) with chat FTS results (source: `"chat"`). Returns unified ranked results.

3. **Brain graph API** (`routes/brain.rs:256-336`): `GET /api/brain/graph` calls `brain.recall()` using the persona name as seed, extracts `graph.neighborhood.entities` and `memory_hits` for a force-directed visualization in the Brain UI panel.

4. **Dashboard** (`routes/dashboard.rs:88-100`): Calls `brain.recall()` to count memories for the dashboard stats display.

### External dependency

The Brain is powered by the `spectral` crate, an external Rust library at `github.com/make-tuned-unit/spectral` (pinned to rev `66cb19a`). It provides:
- `Brain::builder().data_dir().ontology_path().device_id().build()` — initialization
- `brain.remember_with(key, content, opts)` — write (auto-generates fingerprints)
- `brain.recall(query, visibility)` — read (returns `memory_hits` + `graph.neighborhood`)
- `brain.device_id()` — device identity
- The `graph.kz` binary format (knowledge graph managed internally by spectral)

The `spectral` crate is a **black box** from Permagent's perspective. The application calls `remember` and `recall`; fingerprinting, graph construction, and semantic retrieval are handled internally.

---

## System B: The Spectral DB Memory Tables

**Location:** `~/.permagent/spectral/permagent.db` (same database as sessions, messages, tasks, skills)

**Schema:**
```sql
CREATE TABLE memories (
    id TEXT PRIMARY KEY, user_id TEXT REFERENCES users(id),
    key TEXT NOT NULL, content TEXT NOT NULL,
    category TEXT DEFAULT 'core',
    wing TEXT, hall TEXT, room TEXT,
    embedding BLOB,
    valid_from TEXT, valid_until TEXT, superseded_by TEXT,
    confidence REAL DEFAULT 1.0, signal_score REAL DEFAULT 0.5,
    source_session TEXT REFERENCES sessions(id),
    created_at TEXT, updated_at TEXT
);

CREATE TABLE knowledge_graph (
    id TEXT PRIMARY KEY,
    subject TEXT NOT NULL, predicate TEXT NOT NULL, object TEXT NOT NULL,
    valid_from TEXT NOT NULL, valid_until TEXT,
    source_memory_id TEXT REFERENCES memories(id),
    confidence REAL DEFAULT 1.0, created_at TEXT
);

-- Views
CREATE VIEW current_memories AS SELECT * FROM memories WHERE valid_until IS NULL;
CREATE VIEW current_knowledge AS SELECT * FROM knowledge_graph WHERE valid_until IS NULL;
```

**Scale:** 0 rows in both tables. Never populated by the running daemon.

**Key design differences from the Brain:**
| Feature | Brain (`memory.db`) | Spectral DB (`permagent.db`) |
|---------|--------------------|-----------------------------|
| `user_id` | None (single-user) | Yes (FK to users table) |
| `room` | None | Present (3rd hierarchy level) |
| `embedding` | None (spectral handles internally) | BLOB column (explicit vector storage) |
| `valid_from` / `valid_until` | None | Present (temporal versioning) |
| `superseded_by` | None (has `last_reinforced_at`) | Present (chain of supersession) |
| `source_session` | None (has `source` string + `device_id`) | FK to sessions table |
| Knowledge graph | Internal `graph.kz` binary | Explicit RDF-like triples table |
| Multi-user | No | Yes (by schema design) |

### How it's populated

**CLI only.** `crates/goose-cli/src/commands/memory.rs` provides:
- `handle_memory_add(key, content, category, wing, hall)` — inserts into the Spectral DB `memories` table
- `handle_memory_search(query, limit)` — searches via `memories_fts`
- `handle_memory_list(wing, hall, category, limit, offset)` — lists memories

These are CLI commands (`permagent memory add`, `permagent memory search`, `permagent memory list`). They have never been called (0 rows). The daemon never writes to these tables.

The `knowledge_graph` table has **no write path anywhere in the codebase**. No INSERT statement exists for it in any Rust file. It is schema-only.

### How it's consumed

**Only the CLI reads from it** (search, list). The daemon does not query these tables. The brain search API queries the Brain's `memory.db`, not the Spectral DB's `memories` table.

---

## Where Each Is Referenced In Code

### Brain (`spectral::Brain`)
| Location | Operation | Purpose |
|----------|-----------|---------|
| `goose-server/src/state.rs:60-117` | `Brain::builder().build()` | Startup initialization |
| `goose-server/src/routes/reply.rs:316-349` | `brain.recall()` | Pre-turn memory injection (legacy endpoint) |
| `goose-server/src/routes/reply.rs:477-528` | `brain.remember_with()` | Post-turn memory write (legacy endpoint) |
| `goose-server/src/routes/session_events.rs:466-520` | `brain.recall()` | Pre-turn memory injection (SSE endpoint) |
| `goose-server/src/routes/session_events.rs:640-685` | `brain.remember_with()` | Post-turn memory write (SSE endpoint) |
| `goose-server/src/routes/brain.rs:91-137` | `brain.recall()` | `/api/brain/search` API |
| `goose-server/src/routes/brain.rs:256-336` | `brain.recall()` | `/api/brain/graph` API |
| `goose-server/src/routes/dashboard.rs:88-100` | `brain.recall()` | Dashboard memory count |
| `Cargo.toml:87` | dependency | `spectral` crate pinned to rev `66cb19a` |

### Spectral DB (`permagent.db` memories/knowledge_graph tables)
| Location | Operation | Purpose |
|----------|-----------|---------|
| `goose/src/session/spectral_schema.rs:173-292` | CREATE TABLE | Schema definition (migration v5) |
| `goose/src/session/spectral_schema.rs:553-563` | CREATE VIEW | `current_memories`, `current_knowledge` views |
| `goose-cli/src/commands/memory.rs:38-97` | SELECT | CLI search/list commands |
| `goose-cli/src/commands/memory.rs:184-221` | INSERT | CLI memory add command |

---

## Three Possible Relationships

### Option 1: Brain is canonical — deprecate Spectral DB tables

**Argument:** The Brain is the only system that is actually running. It has 1,002 memories, it's queried on every chat turn, it writes on every chat turn, it serves the search API and the graph API. The Spectral DB tables have 0 rows and no daemon write path. The Brain's `spectral` crate handles fingerprinting, graph construction, and semantic recall internally — reimplementing this in raw SQL would be massive effort with no clear benefit.

**What this means:**
- Remove `memories` and `knowledge_graph` tables from `spectral_schema.rs` (or mark deprecated)
- Remove `memories_fts` triggers and views
- CLI `permagent memory add/search/list` would either be redirected to `spectral::Brain` or removed
- The `user_id` multi-user capability of the Spectral DB schema is lost (but isn't used today anyway)
- The `embedding` BLOB column approach is abandoned in favor of the Brain's internal fingerprinting
- The `valid_from/valid_until/superseded_by` temporal model is abandoned in favor of `last_reinforced_at`

**Risk:** The Brain is an external dependency (`spectral` crate). If that library evolves incompatibly, there's no fallback. The Spectral DB tables could serve as a simpler, self-owned alternative.

### Option 2: Both needed, serving different roles

**Argument:** The two systems were designed for different purposes:
- **Brain** = operational recall. Fast, semantic, fingerprint-based. Optimized for "what's relevant right now?" during a conversation turn. Treats memories as opaque content with signal scores. Managed by the `spectral` crate.
- **Spectral DB** = structured knowledge management. Multi-user aware, session-linked, temporally versioned, with explicit knowledge graph triples. Designed for "what does the agent know about X as a fact?" and "how has this knowledge changed over time?"

Under this model:
- Brain handles real-time recall (as it does today)
- Spectral DB tables become the home for agent-curated structured knowledge — facts extracted from conversations, entity relationships, preferences with explicit validity windows
- A consolidation process (the "librarian") would periodically review Brain memories and promote important ones to structured Spectral DB entries with proper temporal semantics
- The `knowledge_graph` triples table would hold extracted relationships (Jesse → works_on → Permagent)
- The `embedding` column would hold vector embeddings for semantic search beyond FTS

**What this means:**
- Keep both schemas
- Build the Spectral DB write path (a librarian/consolidation agent)
- Bridge them: librarian reads from Brain, writes structured facts to Spectral DB
- The CLI commands become the management interface for curated knowledge
- Long-term: Brain is the "fast cache," Spectral DB is the "permanent record"

**Risk:** Maintaining two overlapping systems adds complexity. The "librarian" is a significant piece of new infrastructure. The Spectral DB tables need a daemon write path and query integration that doesn't exist today.

### Option 3: Layered architecture — Spectral DB is incomplete layer above Brain

**Argument:** The Spectral DB tables were designed as a higher-level abstraction layer that was meant to sit on top of the Brain. The schema has features the Brain lacks (multi-user, temporal versioning, session linkage, explicit embeddings), suggesting it was designed to be the application-level memory API while the Brain provides the retrieval engine underneath. The architecture was started (schema created) but never completed (no write path, no integration).

Under this model:
- The `spectral::Brain` is the storage + retrieval engine (keep as-is)
- The Spectral DB `memories` table becomes the application-layer view: every Brain memory gets a corresponding row with user_id, source_session, temporal metadata
- The `knowledge_graph` table provides structured query capability that the Brain's opaque `recall()` cannot offer
- Writes go through the Spectral DB (which validates, adds metadata) then propagate to the Brain
- Reads can go through either path depending on query type: semantic recall via Brain, structured queries via Spectral DB

**What this means:**
- Spectral DB becomes the write-through API layer
- Brain becomes the retrieval backend
- Each memory exists in both (Spectral DB row + Brain memory)
- The `source_session` FK enables "show me what the agent learned in session X"
- The `valid_until/superseded_by` columns enable temporal queries the Brain can't support
- The `embedding` BLOB enables alternative vector search backends

**Risk:** Dual-write consistency. If one system writes and the other fails, they drift. Adds latency to the write path. Requires a synchronization mechanism.

---

## Recommendation

**Option 1: Brain is canonical. Deprecate the Spectral DB memory tables.**

### Reasoning

1. **The Brain works today.** It has 1,002 memories, it's integrated into every chat turn (both recall and write), it serves the search and graph APIs, and the UI displays its data. Any decision that breaks this pipeline needs extraordinary justification.

2. **The Spectral DB tables were aspirational.** They have 0 rows. No daemon write path. No daemon read path. The only consumer is a CLI module that has never been exercised. The schema has features (multi-user, embeddings, temporal chains) that belong to a future architecture, not the current one.

3. **The `spectral` crate is a strategic choice, not a liability.** It's from `make-tuned-unit` (Jesse's organization). It implements TACT + Constellation, the memory architecture that Permagent is designed around. Replacing it with raw SQL would lose the fingerprint-based semantic recall that makes memory useful.

4. **Multi-user is not a near-term need.** The system is hardcoded to `user_id = 'default'` throughout. Adding `user_id` to the Brain can happen if/when multi-user is actually built, rather than maintaining an unused schema for it now.

5. **Temporal versioning can be added to the Brain.** The `last_reinforced_at` column already exists. Adding `valid_until` and `superseded_by` to the Brain schema is a migration, not an architecture change.

6. **Avoiding dual-write complexity.** Options 2 and 3 both require keeping two stores in sync. This is the #1 source of data integrity bugs in memory systems. A single canonical store eliminates an entire class of problems.

### What to preserve from the Spectral DB design

Even though the tables are deprecated, their design captured valuable requirements:

- **`source_session` linkage** — Add to Brain schema as a new column. "What was learned in session X?" is a legitimate query.
- **`knowledge_graph` triples** — The Brain already has `graph.kz` internally. If structured triple queries are needed, expose them through the `spectral` crate's API rather than duplicating in SQL.
- **CLI memory management** — Redirect `permagent memory add/search/list` to use `spectral::Brain` instead of raw SQL against the Spectral DB.

---

## Migration Plan

### Phase 1: Redirect CLI (no data movement)
- Modify `crates/goose-cli/src/commands/memory.rs` to use `spectral::Brain` API instead of direct SQL against `permagent.db`
- `memory add` → `brain.remember_with()`
- `memory search` → `brain.recall()` + format results
- `memory list` → new method needed on Brain (or list from `memory.db` directly)

### Phase 2: Deprecate schema (no data movement)
- Add a comment to `spectral_schema.rs` marking `memories`, `knowledge_graph`, and related tables/views/FTS as deprecated
- Do NOT drop the tables — they may have been created in existing installations
- Stop creating them in new installations (gate behind a version check)

### Phase 3: Feature additions to Brain (if needed)
- If `source_session` tracking is needed: add column to `memory.db`'s `memories` table via `spectral` crate migration
- If structured knowledge queries are needed: expose `graph.kz` query API through the `spectral` crate
- If multi-user is needed: add `device_id` or `user_id` scoping to the `spectral` crate

### Phase 4: Cleanup (months later)
- If no code references remain, drop the deprecated tables in a major version migration
- Remove FTS triggers, views, and indexes for the deprecated tables

### What NOT to do
- Do NOT migrate Brain memories into the Spectral DB tables. They serve different schemas, and the Brain is the system that works.
- Do NOT build a dual-write layer. The maintenance cost exceeds the benefit.
- Do NOT remove the Brain in favor of the Spectral DB. That would require reimplementing fingerprinting, semantic recall, and graph construction from scratch.

---

## Appendix: Data Snapshot

### Brain memories by source
```
import-core:         337   (bulk-loaded foundational knowledge)
import-task:         263   (historical task records)
import-conversation: 222   (historical conversation summaries)
chat:                 67   (live daemon — growing)
import-openbird:      65
import-daily:         36
scheduled:             4   (from cron-triggered sessions)
Other:                 8
Total:             1,002
```

### Brain wings by memory count
```
jesse:           287    general:        239
henry-infra:     152    permagent:       97
getladle:         61    atlasatlantic:   49
polybot:          31    (17 others):    86
```

### Constellation fingerprints by wing
```
jesse:        75,134    general:      57,482
permagent:     2,908    henry-infra:   2,852
getladle:      1,031    polybot:         717
(others):        121
Total:       140,245
```

### Spectral DB tables (all empty)
```
memories:          0 rows
knowledge_graph:   0 rows
memories_fts:      0 rows
```
