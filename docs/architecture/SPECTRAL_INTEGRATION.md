# Spectral Integration

> Pinned to `rev = "5b9c457f"`. Spectral's integration audit at
> `docs/internal/permagent-integration-audit-2026-05-11.md` in the
> [Spectral repo](https://github.com/make-tuned-unit/spectral) reflects
> that rev. **If the pin moves, re-verify the audit.**

## Current pin

```toml
# Cargo.toml (workspace root)
spectral = { git = "https://github.com/make-tuned-unit/spectral", rev = "5b9c457f" }
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
| Spectral pin `5b9c457f` (PR #78 wrapper methods) | Merged | PR #34 |
| Librarian idempotency (force param, BATCH_MUTEX, persisted warm date) | Merged | PR #35 |
| Native non-destructive `remember_with` (Spectral PR #85) | Pending next pin bump | Spectral PR #85 |
| `RecognitionContext` wiring (`session_id` from sessions table) | Pending | Step 5.5 |
| BATCH_MUTEX concurrency test | Filed | Issue #36 |

## Write paths into Spectral

| Caller | Method | Purpose |
|---|---|---|
| `session_events.rs` | `remember_with` | Chat turn memory |
| `reply.rs` | `remember_with` | Chat turn memory |
| `ingestion.rs` | `remember_with` | Activity events |
| `context_builder.rs` | `remember_with` | Context snapshots |
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
