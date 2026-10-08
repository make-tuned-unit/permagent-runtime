# Meeting triage backend — design & API contract (2026-10-07)

Cursor backend lane for durable Orchestrator meeting triage. Frontend coordination: Claude lane (see `RECORD_TEST_PLAN_2026-10-07.md`). Parent owns native staging; no cargo until slot granted.

## Problem

`POST /api/projects/{id}/notes` with `kind: "meeting"` spawns a detached `extract_meeting_todos` task. It has no `recording_id`, no durable status, no idempotent retry, title-only card dedup, and reuses the shared hidden session name `meeting-todo-extraction` — not an inspectable per-meeting Orchestrator triage workflow.

## Goals (bounded)

1. **Idempotent note save** keyed by `(project_id, recording_id)` with body-hash collision detection.
2. **Durable triage row** with `pending | running | completed | failed | skipped`, attempt count, last error, privacy route, dedicated session id, canonical result JSON, card id list.
3. **Inspectable Orchestrator context** — one hidden session per triage: `orchestrator-meeting-triage:{project_id}:{recording_id}`; tool-free completion via existing `meeting_writeup` routing (`AccountedFastCompletion`).
4. **Stable card materialization** via `(triage_id, item_key)` — not global title dedup.
5. **Preserve** raw transcript/user notes, cap 20 todos, no task execution/tools, never inject into active user chat.

Non-goals: generic job queue rewrite, frontend hook changes (Claude lane).

## Data model

### `meeting_triage`

| Column | Purpose |
|---|---|
| `id` | Triage row id (UUIDv7) |
| `project_id` | FK → projects |
| `recording_id` | Client-stable id from Record Start |
| `note_id` | FK → project_notes |
| `body_hash` | SHA-256 of trimmed body; mismatch → 409 |
| `status` | pending / running / completed / failed / skipped |
| `attempt_count` | Retry counter |
| `last_error` | Last failure message |
| `privacy_route` | local_only / trusted_pool / cloud_session / skipped |
| `session_id` | Dedicated hidden session for this triage |
| `result_json` | Canonical parsed extraction (written once) |
| `writeup_provider`, `writeup_model`, `writeup_privacy` | Original write-up provenance (restored on cached retry; legacy → `unknown`) |
| `card_ids_json` | JSON array of materialized card ids |
| `truncated` | 0/1 — transcript truncated for model window |
| `idempotent_recording`, `orchestrator_name` | Client idempotency flag; PrimaryPersona display name |
| timestamps | created / updated / started / completed |

**Unique:** `(project_id, recording_id)`

### `meeting_triage_items`

| Column | Purpose |
|---|---|
| `triage_id` | FK → meeting_triage |
| `item_key` | Stable SHA-256 of `{index}:{normalized_title}` |
| `title`, `context` | Extracted fields |
| `card_id` | Set once materialized |

**Primary key:** `(triage_id, item_key)`

Schema applied version-independently via `meeting_triage::apply_schema` from `init_spectral_db` (same pattern as `brain_feed`).

## HTTP API

### `POST /api/projects/{id}/notes` (extended)

Request (meeting):

```json
{
  "title": "optional",
  "body": "## Your notes\n\n…\n\n## Transcript\n\n…",
  "kind": "meeting",
  "recordingId": "client-uuid-from-start"
}
```

| Case | Response |
|---|---|
| Non-meeting / no kind | Unchanged — existing note path |
| Meeting without `recordingId` | **200** — server generates UUID per save (**non-idempotent** legacy path; receipt has `idempotentRecording: false`) |
| First save | **200** `{ …note fields, triage: { id, status: "pending", recordingId, … } }` |
| Retry same `(project, recordingId, body)` | **200** same note + triage (re-enqueues drain if `failed`) |
| Same recordingId, different body hash | **409** `recording body conflict` |

Non-meeting responses omit `triage` (backward compatible flatten).

### `GET /api/projects/{id}/recordings/{recordingId}/triage`

Returns triage receipt or **404**.

### `POST /api/projects/{id}/recordings/{recordingId}/triage/retry`

Only when status is `failed` or `skipped` (privacy). Resets to `pending` and `attempt_count = 0` (explicit retry only — drain never auto-claims `failed`). **404** if no row.

### `GET /api/projects/{id}/recordings/{recordingId}/triage/session`

Returns Orchestrator session inspectability: `sessionId`, `orchestratorName`, project/recording/note ids, `messageCount`, extension `context`. **404** if triage row or session not yet created.

## Processing flow

```
save_meeting_note (BEGIN IMMEDIATE txn)
  → deterministic note_id + triage_id when client recordingId present
  → insert note + triage (UNIQUE project_id, recording_id)
  → commit (note + triage catalogue outbox rows durable in txn)
  → post-commit: raw note export (first save only) then spawn_drain

daemon startup (state.rs, once — outside Brain mount gate)
  → recover_abandoned_at_startup: `running` + attempt_count < 5 → `pending`;
    `running` + attempt_count ≥ 5 → `failed` (manual retry only)
  → spawn_periodic_drain(pool, Option<Brain>)

drain (background, coalesced mutex; periodic in state.rs)
  → never reset `running` (POST spawn_drain included)
  → claim `pending` only (attempt_count < 5; never auto-claim `failed`)
  → resolve_live_writeup_plan (privacy-aware)
  → skip → status skipped, stop (no cloud)
  → ensure triage session with real PrimaryPersona + extension metadata
  → run_writeup_plan (tool-free, AccountedFastCompletion)
  → persist user+assistant turns in session
  → persist result_json + writeup provenance BEFORE note/card writes
  → apply txn: note body + cards + completed (item_key + stable card ids)
  → post-commit brain_feed for new cards
  → completed | failed (explicit POST …/triage/retry to re-run)
```

**Retry idempotency:** If `result_json` already present, skip model call; apply note+cards from cached JSON using stored writeup provenance (legacy rows without provenance columns → `unknown`, not `local_only`).

**Empty todos:** When extraction yields no commitments, triage completes without board column lookup — personal/legacy projects without columns are supported.

**Card metadata:**

```json
{
  "created_by_agent": "henry",
  "source": "meeting_triage",
  "source_note_id": "...",
  "source_triage_id": "...",
  "source_item_key": "...",
  "source_recording_id": "..."
}
```

## Cross-language contracts

- `split_meeting_body` heading literals remain pinned with `useMeetingDictation.ts` (`## Your notes`, `## Transcript`).
- Frontend must send `recordingId` on meeting save (Claude lane).

## Parent follow-ups

- Native/visual acceptance per `RECORD_TEST_PLAN_2026-10-07.md`.
- `cargo test` when build slot granted (rustfmt done; compile not run here).

## Tests (focused, no cargo until slot)

| Test | Location | Asserts |
|---|---|---|
| `save_meeting_note` idempotent | `meeting_triage.rs` | Same note_id on retry |
| Body hash conflict | `meeting_triage.rs` | Error, no second note |
| `item_key` stability | `meeting_triage.rs` | Same title/index → same key |
| `split_meeting_body` | `meeting_triage.rs` + `projects.rs` | UI contract preserved |
| `retry_triage` | `meeting_triage.rs` | failed → pending, attempt_count reset |
| Concurrent save | `meeting_triage.rs` | One note under parallel save |
| Transaction apply | `meeting_triage.rs` | Cards + completed in one txn |
| Empty todos, no columns | `meeting_triage.rs` | Completes with `card_ids: []` on column-less project |
| Persisted result reuse | `meeting_triage.rs` | result_json + provenance before apply; skip model |
| Provenance on cached retry | `meeting_triage.rs` | Stored provider/model/privacy; legacy → unknown |
| Exhausted startup recovery | `meeting_triage.rs` | attempt_count ≥ 5 running → failed, not stuck pending |
| Export/drain ordering | `projects.rs` | Raw export before drain; skip export on idempotent retry |
| Privacy skip | `meeting_triage.rs` | local_only → skipped, no session cloud |
| `item_key` (HTTP module) | `projects.rs` | Normalization contract |

**Implementation status (2026-10-07):** Source freeze for parent compile; `cargo test` deferred until build slot granted.

## Files changed

| File | Role |
|---|---|
| `crates/goose/src/meeting_triage.rs` | **New** — schema, save, drain, process, card materialization |
| `crates/goose/src/meeting_writeup.rs` | Per-triage session name; `run_writeup_plan_with_session` |
| `crates/goose/src/lib.rs` | Export module |
| `crates/goose/src/session/spectral_schema.rs` | `apply_schema` on fresh init |
| `crates/goose-server/src/routes/projects.rs` | HTTP handlers, legacy recording fallback, `/triage/retry`, `/triage/session` |
| `crates/goose-server/src/state.rs` | Startup recovery + `spawn_periodic_drain` (Brain-optional) |
| `crates/goose/src/project_notes.rs` | `insert_note_tx`, `index_note_in_brain`, `update_note_body_tx` |
