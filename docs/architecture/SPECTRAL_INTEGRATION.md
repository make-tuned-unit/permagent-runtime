# Spectral Integration

> Pinned to `rev = "e9a80d8"`. Spectral's integration audit at
> `docs/internal/permagent-integration-audit-2026-05-11.md` in the
> [Spectral repo](https://github.com/make-tuned-unit/spectral) reflects
> rev `5b9c457f` — verify struct fields if pin moves past `e9a80d8`.

## Current pin

```toml
# Cargo.toml (workspace root)
spectral = { git = "https://github.com/make-tuned-unit/spectral", rev = "e9a80d8" }
```

## Authoritative reference

Spectral owns its own API surface documentation. Do not duplicate it here.

- **Spectral API audit:** `spectral/docs/internal/permagent-integration-audit-2026-05-11.md`
- **Brain wrapper source:** `spectral/crates/spectral/src/lib.rs` (public API)
- **Memory/MemoryHit structs:** `spectral/crates/spectral-ingest/src/lib.rs`
- **Brain graph methods:** `spectral/crates/spectral-graph/src/brain.rs`

## Permagent-side gotchas

### spawn_blocking requirement

All Brain methods use `block_on()` internally. Calling them from an async
context without `spawn_blocking` deadlocks the tokio runtime.

```rust
let brain = brain.clone();
let result = tokio::task::spawn_blocking(move || {
    brain.recall(&query, spectral::Visibility::Private)
}).await??;
```

Every call site in Permagent follows this pattern. Do not add new Brain
calls without wrapping.

### Brain access

Brain is available to platform extensions via a global `OnceLock`:

```rust
use crate::agents::platform_extensions::get_global_brain;
let brain = get_global_brain().ok_or("Brain not available")?;
```

Set once at daemon startup in `state.rs`. Not threaded through
`PlatformExtensionContext` (would require plumbing through
ExtensionManager -> Agent -> per-session context).

### Brain import path

Use `spectral::Brain` (the wrapper type). Methods `get_memory`,
`set_description`, `list_undescribed` are on the wrapper as of rev
`5b9c457f` (PR #78).

### session_id convention

Permagent's `sessions.id` (UUID in `permagent.db`) maps 1:1 to what
the UI calls a "conversation." Stable across all turns, never reused.
Wire directly to Spectral's `RecognitionContext.session_id` when that
plumbing lands (Step 5.5, pending).

### Librarian write path

The Librarian calls `set_description(id, text)` only. It never calls
`remember_with`. Idempotency is at the `describe_one` layer: if
`force=false` and `memory.description.is_some()`, returns cached
without calling Ollama.

## Recent decisions

| Decision | Status | Reference |
|---|---|---|
| Spectral pin `e9a80d8` (PR #85 non-destructive remember_with) | Merged | PR #38 |
| Librarian idempotency (force param, BATCH_MUTEX, persisted warm date) | Merged | PR #35 |
| Non-destructive `remember_with` (Spectral PR #85) | **Shipped** — same-key writes now append-only via content_hash dedup | PR #38 |
| `RecognitionContext` wiring (`session_id` from sessions table) | Pending | Step 5.5 |
| BATCH_MUTEX concurrency test | Filed | Issue #36 |

### Re-write vector (closed by PR #38, Spectral PR #85)

`session_events.rs:740` and `reply.rs:596` both construct the key
`chat-{session_id}-{turn_idx}` where `turn_idx = all_messages.len()`.
Two reply endpoints (`POST /reply` and `POST /sessions/{id}/reply`)
produce identical keys for the same logical turn. Under the current
pin (5b9c457f), `remember_with` uses destructive upsert on same-key
writes. Content is typically identical (same turn text), so this is a
data integrity concern, not data loss. PR #85 (shipped via pin bump PR #38) makes `remember_with`
non-destructive (append-only via content_hash dedup), eliminating
the overwrite.

Other `remember_with` call sites use fresh keys (UUID-derived or
monotonically increasing) and are not affected.

## Write paths into Spectral

| Caller | Method | Purpose | Key pattern |
|---|---|---|---|
| `session_events.rs` | `remember_with` | Chat turn memory | `chat-{session_id}-{turn_idx}` |
| `reply.rs` | `remember_with` | Chat turn memory | `chat-{session_id}-{turn_idx}` |
| `ingestion.rs` | `remember_with` | Activity events | `activity:{ts}:{type}:{event_id[..8]}` |
| `context_builder.rs` | `remember_with` | Test data only | hardcoded test keys |
| `scheduler.rs` | `remember_with` | Scheduled job output |
| `librarian.rs` | `set_description` | Memory descriptions |

## Read paths from Spectral

| Caller | Method | Purpose |
|---|---|---|
| `session_events.rs` | `recall` | Pre-reply context |
| `reply.rs` | `recall` | Pre-reply context |
| `context_builder.rs` | `probe_recent` | Activity awareness |
| `brain.rs` (routes) | `recall` | Brain search API |
| `librarian.rs` | `get_memory`, `list_undescribed`, `recall` | Description generation |
