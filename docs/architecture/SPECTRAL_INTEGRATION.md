# Spectral Integration Plan

**Status:** Phase 1 complete (foundation). Phases 2-5 pending.
**Goal:** Replace Permagent's custom 18-table memory schema with Spectral's Brain API.
**Scope:** v1.0 ships with chat memory through Spectral. v1.1 adds activity capture.

## What Spectral provides

Spectral is a hybrid memory substrate combining a Kuzu knowledge graph,
a SQLite + FTS5 fingerprint store, ontology validation, and federation
primitives (DeviceId, BrainId, Visibility, source attribution). It
replaces what Permagent currently does with 18 custom tables, plus
provides project detection (wing_rules), graph relationships, and
content-addressed identity for future federation.

Repo: https://github.com/make-tuned-unit/spectral
License: Apache 2.0
Performance: 393us single ingest, 564us warm recall on 1k memories.

## Cargo dependency

Spectral is consumed as a git dependency, branch = "main", until v0.1.0 is
published to crates.io. The local patch override in Cargo.toml is
commented out by default — uncomment it only when iterating on Spectral's
API locally.

**cxx-build version pinning:** `cxx-build` must match `cxx` (both 1.0.138)
in `Cargo.lock` to avoid kuzu FFI symbol mismatches. If the lockfile drifts
after a `cargo update`, run `cargo update -p cxx-build --precise 1.0.138`.

## Ontology

Permagent ships its own ontology at `crates/goose/assets/ontology.toml`,
embedded into the binary and written to `~/.permagent/ontology.toml` on
first run.

### v1.0 ontology (chat memory only)

- Entities: jesse-sharratt (person), permagent (project), spectral (project)
- Predicates: worked_on (person -> project)
- New entities are added at runtime as the user interacts

### v1.1 ontology additions (activity capture, planned)

- Entity types: chat_session, skill, activity
- Predicates: discussed_in, uses_skill, mentions_person, mentions_project,
  occurred_in_project, started_at, involves_person

Note: Spectral validates that all predicate domain/range types have at least
one entity instance. Predicates referencing future entity types (chat_session,
skill) must wait until those types are populated at runtime.

## Single-writer constraint

Spectral uses Kuzu, which is undefined behavior under concurrent process
access to the same data_dir. Permagent satisfies this by routing all
brain writes through `permagentd` (single process, multi-threaded).
Activity capture in v1.1 will run as a tokio task inside `permagentd`
rather than as a separate daemon -- keeps the single-writer guarantee
without RPC overhead.

## Migration phases

### Phase 1: Foundation (this commit)

- Add Spectral as workspace dependency with branch = "main"
- Patch override committed but commented out, with documentation
- Ontology file at `crates/goose/assets/ontology.toml`
- Smoke test at `crates/goose/tests/spectral_smoke.rs` proving:
  - Brain compiles inside Permagent's tree
  - Memory round-trips with full provenance fields
  - Graph triples write and read back
  - Schema persists across brain reopen

### Phase 2: Audit existing memory schema

Map Permagent's 18 current tables to Spectral primitives. Output is
`docs/architecture/SCHEMA_AUDIT.md` with one row per table:

| Table | Disposition | Notes |
| ----- | ----------- | ----- |
| chat_messages | Migrate to brain.remember_with() | source = "chat" |
| chat_sessions | Keep in Permagent SQLite | Session metadata, not memory |
| memory_records | Retire | Covered by Spectral's fingerprint store |
| ... | ... | ... |

Identifies which tables retire entirely, which migrate to Spectral, and
which stay in Permagent's own SQLite (session metadata, agent state,
settings -- not memory).

### Phase 3: Migration script (if needed)

Pre-release: probably no migration needed. Permagent's existing brain.db
isn't structured the way Spectral wants, and pre-release users can start
fresh. If post-release migration becomes necessary, write a one-time
script that reads from `~/.permagent/brain.db` (custom schema) and
writes to `~/.permagent/brain/` (Spectral data dir).

### Phase 4: Production cutover

- Replace memory write paths with `brain.remember_with()`
- Replace memory read paths with `brain.recall()`
- Remove the 18 custom tables from Permagent's SQLite migrations
- Verify chat works end-to-end through Spectral
- Run `./scripts/verify-provider-alignment.sh` to confirm no regression
- Add a similar Spectral-specific verification script

### Phase 5: Ship v1.0

Permagent v1.0 with Spectral as memory substrate. Activity capture
deferred to v1.1.

## v1.1 forward references

Activity capture against Spectral primitives is documented separately
at `docs/architecture/ACTIVITY_CAPTURE_v1.1.md`. Wing rules, project
detection, and the Henry-OpenBird-derived design lessons live there.

## What's NOT in Spectral yet

These don't block Permagent integration but are worth knowing:

- **Federation protocol.** Primitives exist; sync mechanism doesn't.
  v1.0 ships single-device. Cross-machine via rsync of `~/.permagent/brain/`
  works as interim (Spectral's idempotent writes resolve conflicts).
- **Cognitive Spectrogram.** Phase 2 architecture, reserved as
  spectral-spectrogram crate but unimplemented. Recall works at full
  quality without it.
- **Memify feedback loop.** Self-reinforcing recall. Not built yet.

## Verification

Phase 1 success criteria:

- `cargo build --release -p permagent-cli -p permagent-daemon` clean
- `cargo test -p permagent --test spectral_smoke` passes
- `./scripts/verify-provider-alignment.sh` still passes (no regression)
