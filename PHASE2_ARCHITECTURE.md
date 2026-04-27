# Permagent Phase 2 — Architecture Specification

**Date:** 2026-04-27
**Status:** Scope defined, ready for implementation
**Prerequisite:** Phase 1 complete (see PHASE1_ARCHITECTURE.md)
**Reference:** CAPABILITY_INVENTORY.md for gap analysis and Goose UI source mapping

---

## Theme

Surface the runtime's full capability through a polymorphic chat panel and four-workspace shell. Add the Brain view as the memory inspector, port four pieces from upstream Goose, and remove the gap between what the runtime can do and what users can see.

---

## Seven tracks (in dependency order)

### Track 1: POLYMORPHIC CHAT RENDERER (foundation)

The chat panel currently renders all tool results as raw JSON cards. This track replaces them with typed renderers that route on content type.

**Renderer types:**
- `text/markdown` — Markdown with GFM, syntax-highlighted fenced code blocks
- `image/*` — Inline image with lightbox on click
- `audio/*` — Audio player with waveform
- `application/vnd.vega.v5+json` — Vega-Lite chart via `@mcp-ui/client` sandboxed iframe
- `table` — Sortable data table from JSON arrays
- `code` — Syntax-highlighted code block with copy button
- `error` — Red-bordered error card with stack trace toggle
- `json-fallback` — Pretty-printed JSON for unrecognized types

**File upload:**
- Drag/drop and paste onto chat input
- `POST /api/sessions/:id/upload` → stores in `attachments` table
- `GET /api/sessions/:id/attachments/:id` → retrieves
- Schema v6: `CREATE TABLE attachments (id, session_id, filename, mime_type, size_bytes, storage_path, created_at)`

**This is the foundation for all other tracks** — chart rendering for Track 5, file upload for Track 7, formatted results everywhere.

**Effort:** M (3-5 days)

### Track 2: SCHEMA v5 MIGRATION — Brain as fourth workspace

Adds the Brain workspace preset. Existing v4 databases get Brain appended without modifying Work/World/Build. Fresh installs seed all four.

**Implemented in this commit** (see below).

**Effort:** S (done)

### Track 3: PORT FROM GOOSE — Sessions UI

**Source:** `upstream/main:ui/desktop/src/components/sessions/`

**Components to port:**
- `SessionListView.tsx` — Session list with search, date grouping, actions (rename, delete, export, fork, import)
- `SessionHistoryView.tsx` — Conversation replay with resume button
- `SessionItem.tsx` — Card component with metadata display

**Porting work:**
- Strip `react-intl` `defineMessages`/`useIntl` → plain strings
- Replace `react-router-dom` navigation → Zustand store `setActivePanel`
- Replace shadcn `Card`/`Button`/`ScrollArea` → Tailwind utility classes
- Add `SearchView` component for FTS (calls `GET /api/sessions/search`)

**API endpoints (all exist):** GET `/api/sessions`, GET `/api/sessions/search`, GET `/api/sessions/:id/export`, POST `/api/sessions/import`, POST `/api/sessions/:id/fork`

**Effort:** S (1 day)

### Track 4: PORT FROM GOOSE — Settings (model switching)

**Source:** `upstream/main:ui/desktop/src/components/settings/models/`

**Components to port:**
- `SwitchModelModal.tsx` — Provider dropdown, model dropdown, thinking-level selector, API key entry
- `ModelsSection.tsx` — Current model display card

**Porting work:**
- Replace `useConfig()` / `ModelAndProviderContext` → direct `api.getConfig()` / `api.upsertConfig()` calls
- Replace `useNavigation` → Zustand store

**API endpoints (all exist):** GET `/config/providers`, GET `/config/providers/:name/models`, POST `/config/set_provider`, POST `/config/upsert`

**Effort:** S (half day)

### Track 5: PORT FROM GOOSE — Visualization rendering

**Source:** `upstream/main:ui/desktop/src/components/McpApps/`

**Components to port:**
- `MCPUIResourceRenderer.tsx` — Renders `EmbeddedResource` content from tool results in sandboxed iframes via `@mcp-ui/client` SDK

**Integration point:** Modify `ToolCallCard.tsx` to detect `resource` content type in tool results → delegate to `ResourceRenderer` instead of raw JSON dump.

**Dependencies to add:** `@mcp-ui/client` (v6.1.0)

**Couples with Track 1** — the polymorphic chat renderer decides which renderer to use.

**Effort:** S-M (1 day)

### Track 6: PORT FROM GOOSE — Schedules

**Source:** `upstream/main:ui/desktop/src/components/schedule/`

**Components to port:**
- `SchedulesView.tsx` — Schedule list with CRUD, status badges, action buttons
- `CronPicker.tsx` — Visual cron expression builder (period/time selectors)
- `ScheduleModal.tsx` — Create/edit dialog with recipe selector
- `ScheduleDetailView.tsx` — Past sessions list for a schedule

**Dependencies to add:** `cronstrue` (human-readable cron descriptions)

**API endpoints (all exist):** POST `/schedule/create`, GET `/schedule/list`, DELETE `/schedule/:id`, POST `/schedule/:id/pause`, POST `/schedule/:id/unpause`, POST `/schedule/:id/run_now`, POST `/schedule/:id/kill`, GET `/schedule/:id/inspect`

**Note:** ScheduleModal depends on the recipe selector. Minimal first version: text input for recipe ID, upgrade to full recipe picker later.

**Effort:** M (2-3 days)

### Track 7: BRAIN VIEW (greenfield, fourth workspace)

The differentiating feature for v1 launch. Three layers, shipped incrementally:

**Layer 1 — Memory list (ship first):**
- Searchable, paginated list of memories from Spectral `memories` table
- Filter by category, wing/hall/room, signal score
- Each card: key, content preview, category badge, confidence indicator, timestamps
- REST: `GET /api/memories?q=&category=&wing=&limit=&offset=`
- Requires new server route module `routes/memories.rs`

**Layer 2 — Knowledge graph (ship second):**
- Force-directed graph visualization of `knowledge_graph` triples
- Nodes = subjects/objects, edges = predicates
- Click node → filter memories related to that entity
- Library: `@xyflow/react` (React Flow) or `d3-force` + SVG
- REST: `GET /api/knowledge-graph?subject=&limit=`

**Layer 3 — Curation (ship third, may slip to Phase 3):**
- Add/edit/delete memories from the UI
- "Promote to memory" button in chat (saves assistant response as memory)
- Bulk operations: merge, supersede, archive
- REST: POST/PUT/DELETE on `/api/memories/:id`

**Effort:** L (1-2 weeks for Layers 1-2; Layer 3 is a separate increment)

---

## Out of scope for Phase 2 (deferred to Phase 3+)

- **Hub System** — Multi-agent ownership domains. Design doc v0.2 exists.
- **Mesh** — Paid agent-to-agent network. Blocked on Chitin API availability.
- **Mobile** — Capacitor + Tailscale remote access. Requires tunnel hardening.
- **Permagent Brain** — Managed LLM subscription tier. Requires Stripe + Supabase.
- **Founding Senator email reservation** — Blocked on Stripe + Supabase + Chitin wiring.
- **Custom user-defined workspaces** — Four presets only in Phase 2. Custom workspaces in Phase 3.

---

## Open questions for Jesse

1. ~~Does Brain replace the placeholder World View, or do they coexist?~~
   **ANSWERED:** Brain is the fourth workspace. World remains for future mesh visualization.

2. Should custom user-defined workspaces be supported in Phase 2, or four presets only?
   **Recommendation:** Four presets only. Custom workspaces add workspace CRUD UI complexity without clear user signal. Defer.

3. When the chat renderer ships (Track 1), do existing sessions' raw-JSON tool results need retroactive migration to typed renderers?
   **Recommendation:** No migration. The renderer should handle both old (raw JSON) and new (typed resource) formats gracefully. Old messages render as `json-fallback`; new messages get typed rendering.
