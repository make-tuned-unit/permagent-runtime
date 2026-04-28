# Permagent Schema Audit for Spectral Integration

**Status:** Phase 2 Track B complete — schema mapped to Spectral primitives.
**Purpose:** Define the surface area Phase 4 cutover touches.
**Companion:** docs/architecture/SPECTRAL_INTEGRATION.md (full plan).

## Inventory summary

Total storage surfaces audited: 32

- A. Migrate to Spectral memory: 2
- B. Migrate to Spectral graph: 1
- C. Keep in Permagent SQLite: 16
- D. Retire entirely: 7
- ? Ambiguous, needs discussion: 1

Non-SQL persistent surfaces (all Keep): 5

## Disposition table — SQL tables

| Surface | Purpose | Disposition | Rationale | Phase 4 files |
|---------|---------|-------------|-----------|---------------|
| schema_version | Schema migration tracking | C | Operational metadata, not memory | n/a |
| users | User profiles (display_name, email, active_workspace_id) | C | Operational identity state, not memory | n/a |
| sessions | Session metadata (id, name, working_dir, tokens, recipe) | C | Operational session state; Spectral doesn't model sessions | n/a |
| messages | Chat turns (role, content_json, metadata, tokens) | A | Free-text conversation content; maps to brain.remember_with() with source="chat" | session_manager.rs:304, session_manager.rs:868 |
| threads | Legacy thread containers (name, working_dir) | D | Goose backward-compat; not used in Permagent UI | spectral_schema.rs (remove DDL) |
| thread_messages | Legacy thread-message junction | D | Goose backward-compat; not used in Permagent UI | spectral_schema.rs (remove DDL) |
| memories | Long-term memory (key, content, wing/hall/room, embedding, confidence) | A | Core memory store; directly maps to brain.remember_with() with full provenance | spectral_schema.rs, session_manager.rs, chatrecall.rs |
| memories_fts | FTS5 virtual table on memories | D | Spectral's TACT retrieval replaces this entirely | spectral_schema.rs (remove DDL + triggers) |
| knowledge_graph | Triple store (subject, predicate, object, confidence) | B | Entity relationships; maps to brain.assert() | spectral_schema.rs, session_manager.rs |
| knowledge_graph_fts | FTS5 virtual table on knowledge_graph | D | Spectral's graph recall replaces this | spectral_schema.rs (remove DDL) |
| tasks | Tool execution history (tool_used, steps, status, input/output) | C | Skill learning operational state; not memory content | n/a |
| skills | Learned automation patterns (name, definition, trigger) | C | Operational automation state | n/a |
| skill_executions | Skill execution logs (status, input, output, timing) | C | Operational audit trail | n/a |
| skill_triggers | Trigger configurations per skill (type, config) | C | Operational automation config | n/a |
| skill_dismissals | Dismissed skill suggestions per user | C | Operational preference state | n/a |
| integrations | Third-party service connections (provider, scopes, tokens) | C | OAuth/auth operational state | n/a |
| provider_inventory_entries | Cached LLM provider metadata | C | Provider cache, operational | n/a |
| provider_inventory_models | Model lists per provider | C | Provider cache, operational | n/a |
| workspaces | UI workspace layouts (name, icon, layout_json) | C | UI state, not memory | n/a |
| attachments | File upload references (filename, mime, path) | C | Session operational data; files stay on disk | n/a |

## Disposition table — SQL views

| Surface | Purpose | Disposition | Rationale |
|---------|---------|-------------|-----------|
| current_memories | Active memories (valid_until IS NULL) | D | Retire with memories table; Spectral manages validity |
| current_knowledge | Active triples (valid_until IS NULL) | D | Retire with knowledge_graph table |
| recent_tasks | Latest 100 completed tasks | C | Keep; operational view for skill learning |
| repetition_candidates | Tool patterns repeated 2+ times in 7 days | C | Keep; operational view for skill detection |

## Disposition table — file-based storage

| Surface | Purpose | Disposition | Rationale |
|---------|---------|-------------|-----------|
| MCP memory (.txt files in ~/.config/goose/memory/ and .goose/memory/) | ? | See Ambiguous surfaces below |
| config.yaml | Application configuration | C | Operational config, not memory |
| secrets.yaml / keyring | API keys and credentials | C | Auth state, not memory |
| permission.yaml + tool_permissions.json | Tool permission rules | C | Operational preferences |
| instance_id | Installation UUID | C | Telemetry identifier |
| telemetry_installation.json | PostHog installation record | C | Telemetry state |

## Ambiguous surfaces

### MCP memory files (disposition ?)

**What it is:** The `goose-mcp` memory extension stores user-explicit notes as
plain .txt files organized by category (e.g., `preferences.txt`,
`project_notes.txt`). Located at `~/.config/goose/memory/` (global) and
`.goose/memory/` (project-scoped). Implementation at
`crates/goose-mcp/src/memory/mod.rs` (668 lines).

**Why ambiguous:** This is user-facing note-taking ("remember that I prefer
dark mode"), not agent-inferred memory. It overlaps with Spectral's
`brain.remember()` but serves a different UX contract:

- Users explicitly invoke `remember_memory` and `retrieve_memories` MCP tools
- Content is plain text with optional tags, not structured provenance
- Two scopes (global vs. project-local) that don't map to Spectral's Visibility model
- Currently working and actively used

**Decision needed before Phase 4:**

Option 1: Migrate to Spectral. MCP tools call `brain.remember_with(source="user_explicit")`.
Global scope maps to `Visibility::Private`, project scope needs a project-scoped brain or
a convention like `wing = "project:{name}"`. Advantage: unified memory. Risk: scope mismatch.

Option 2: Keep as-is. MCP memory stays file-based; Spectral handles agent-inferred memory only.
Advantage: zero migration risk, clear separation. Risk: two memory systems to reason about.

**Recommendation:** Option 2 for v1.0 (keep file-based MCP memory). Revisit for v1.1 when
activity capture introduces project-scoped Spectral brains.

## Predicate audit

### v1.0 ontology (current state)

The corrected ontology at `crates/goose/assets/ontology.toml` declares:

**5 entity types:** person, project, chat_session, skill, topic

**7 predicates:**

| Predicate | Domain | Range |
|-----------|--------|-------|
| worked_on | person | project |
| discussed_in | topic | chat_session |
| uses_skill | chat_session | skill |
| mentions_person | chat_session | person |
| mentions_project | chat_session | project |
| mentions_topic | chat_session | topic |
| related_to | topic | topic |

All predicates needed for Phase 4 cutover exist in the ontology. The remaining
blocker is **runtime entity creation**: `brain.assert()` requires entity instances
to exist for the canonicalizer to resolve mentions. Spectral's
AutoCreateWithCanonicalizer (Phase 2 Track A, ETA ~24h) will create entities
on-the-fly when first referenced. Until then, graph assertions fail with
`UnresolvedMention`.

### Existing relationships mapped to v1.0 predicates

| Existing relationship | v1.0 predicate match | Ambiguity? | Action needed |
|-----------------------|----------------------|------------|---------------|
| memories.source_session (memory originated in session) | No match | None | Covered by remember_with(source="chat:{session_id}") provenance field, not a graph edge |
| knowledge_graph (subject, predicate, object triples) | All 7 predicates available | None | Each existing triple maps to brain.assert(); predicate names must match ontology. Gated on Track A for entity creation. |
| sessions.user_id (user owns session) | No match needed | None | Operational FK, stays in Permagent SQLite |
| messages.session_id (message belongs to session) | No match needed | None | Operational FK; message content migrates to memory, session link is provenance |
| memories.superseded_by (memory versioning) | No match | None | Spectral handles via content-addressed IDs; duplicate writes are idempotent |
| tasks.session_id (task triggered in session) | No match needed | None | Operational FK, stays in Permagent SQLite |
| skills.source_task_id (skill derived from task) | No match needed | None | Operational FK, stays in Permagent SQLite |

### Predicates with multiple valid domains

No ambiguity found. Each relationship in the current schema has a clear single
domain and range type. The `mentions_*` predicates are type-specific (person vs.
project vs. topic) rather than generic, so `brain.assert()` won't encounter
AmbiguousEntityType errors once entity instances exist.

## Phase 4 hot paths

Files that will change during production cutover (sorted by complexity):

### Trivial (drop-in DDL removal)

| File | Lines | Change |
|------|-------|--------|
| crates/goose/src/session/spectral_schema.rs | 809 | Remove DDL for: threads, thread_messages, memories, memories_fts (+ triggers), knowledge_graph, knowledge_graph_fts, current_memories view, current_knowledge view. ~250 lines removed. |

### Moderate (API substitution)

| File | Lines | Current | Spectral equivalent | Scope |
|------|-------|---------|---------------------|-------|
| crates/goose/src/session/session_manager.rs | 1704 | add_message() inserts into messages table | brain.remember_with(key, content, opts) for memory-worthy turns | ~50 lines changed in message write path |
| crates/goose/src/agents/platform_extensions/chatrecall.rs | 309 | SQL queries against sessions + messages for recall | brain.recall() for memory search, keep session metadata queries | ~80 lines; core recall logic replaced |
| crates/goose-cli/src/commands/memory.rs | 221 | CLI commands for memory search/list/add against SQL | brain.remember_with() and brain.recall() | ~100 lines; straightforward API swap |

### Complex (new design needed)

| File | Lines | Current | Why complex | Design questions |
|------|-------|---------|-------------|-----------------|
| crates/goose/src/session/session_manager.rs | 1704 | Messages stored as structured JSON with role/content/metadata/tokens | Not all messages are memories — need a filter deciding which turns get brain.remember_with(). System messages, tool results, and streaming deltas should NOT become memories. Need a classification heuristic. | What constitutes a "memory-worthy" message? User turns + final assistant responses? Or everything? |
| crates/goose-mcp/src/memory/mod.rs | 668 | File-based MCP memory with categories and tags | If migrating (Option 1 from ambiguous surfaces), needs project-scoped brain concept. If keeping (Option 2), no changes. | Decision depends on ambiguous surface resolution above. |

## Total cutover scope

- Files to modify: 4 (spectral_schema.rs, session_manager.rs, chatrecall.rs, memory.rs CLI)
- Files to retire: 0 (DDL removal is modification, not file deletion)
- New files needed: 1 (brain initialization module — opens Brain, manages lifecycle in permagentd)
- Estimated lines of change: ~480 (250 DDL removal + 130 API swap + 100 new brain init)

## Open questions

1. **Memory-worthiness filter:** Which message types become Spectral memories?
   User turns and final assistant responses are clear candidates. Tool call
   results, system messages, and streaming deltas are not. Need a simple
   heuristic before Phase 4 cutover.

2. **MCP memory disposition:** Keep file-based (Option 2, recommended for v1.0)
   or migrate to Spectral (Option 1, deferred to v1.1)?

3. **Existing knowledge_graph data:** Pre-release users may have triples with
   predicates not in the v1.0 ontology. Per Phase 3 of the integration plan,
   pre-release users can start fresh. Document this in release notes.

4. **Runtime entity creation:** The `chat_session` entity type exists in
   the v1.0 ontology, but specific chat_session instances (and person,
   project, topic instances) must be created at runtime when first
   referenced. This is gated on Spectral's AutoCreateWithCanonicalizer
   (Phase 2 Track A). Without it, `brain.assert()` fails with
   `UnresolvedMention` because no entity instances exist to resolve against.

5. **Token accounting:** The `messages` table tracks per-message token counts
   and the `sessions` table accumulates totals. If messages migrate to Spectral
   memory, token accounting must stay in Permagent's SQLite (it's operational
   billing data, not memory content). Phase 4 must preserve this path.

## Next steps

- Phase 2 Track A: lock in AutoCreateWithCanonicalizer integration
  (waiting on Spectral PR, ~24h)
- Phase 3: migration script — likely skip per integration plan (pre-release
  users start fresh)
- Phase 4: production cutover (this audit defines the surface)
