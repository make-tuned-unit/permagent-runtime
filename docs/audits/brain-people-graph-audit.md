# Audit — Brain People-Graph + Description Pipeline (Phase 0, no code)

Scope: (1) does the schema support a relationship graph, and (2) why do
People / Projects / Topics lack descriptions when memories have them.
Read-only. File:line evidence below. Spectral source read from the pinned
checkout `~/.cargo/git/checkouts/spectral-121c60948af2c3d3/2c1f6bf`.

---

## TL;DR

- **The relationship graph already exists** — in Spectral, fully built (the
  `Triple` REL table + traversal API). Permagent **uses none of it**: zero calls
  to `assert`, `insert_triple`, `find_triples`, `neighborhood`-for-edges, or
  `set_entity_description` anywhere in `crates/`.
- **The description gap is not one bug, it's three different stores** that were
  never wired together:
  - **Topics** (Brain graph entities): the column EXISTS (`Entity.description`,
    "set by Librarian") and the read path EXISTS — but **nothing writes it** and
    the route **hardcodes it to empty** anyway (`note: String::new()`).
  - **People** (permagent.db): **no description column at all** (only `notes`),
    and **no frontend** consumes the endpoint.
  - **Projects** (permagent.db): fully working — column written and read. The
    odd-one-out that *works*.
- The "set_description writes a bio" mental model is the trap: `set_description`
  only ever touches the **memories** SQLite table. People/Projects/Topics are in
  two *other* stores it never reaches.

---

## PART 1 — Relationship-graph schema

Spectral keeps **two stores**: a Kuzu property-graph (`graph.kz`) for
entities + edges, and SQLite (`memory.db`) for memories + FTS. Permagent's
own `permagent.db` is a **third** store (projects, people, cards, recognition).

| Graph need | Status | Evidence |
|---|---|---|
| People as nodes (entities) | **EXISTS** (Spectral) | `Entity` node table — `spectral-graph/src/schema.rs:78-88`; in-mem struct `kuzu_store.rs:46-63`. `entity_type` is a free string; `person`/`project` used at `kuzu_store.rs:162-163`. |
| Person→Project edges | **EXISTS** (Spectral), **UNUSED** (Permagent) | `Triple` REL table `schema.rs:104-114` (FROM Entity TO Entity, `predicate`, `confidence`, `weight`, `visibility`, provenance). Write via `Brain::assert/assert_typed` `brain.rs:530-626`. **No permagent caller** (grep `assert(`/`insert_triple`/`assert_typed` in `crates/` → 0 hits). |
| Person→Person edges | **EXISTS** (Spectral), **UNUSED** | Same `Triple` table — type-agnostic; any entity_type → any entity_type. |
| Read/traverse the graph | **EXISTS** (Spectral), **PARTIAL** (Permagent) | `KuzuStore::find_triples` `kuzu_store.rs:331-376`, `neighborhood()` BFS `kuzu_store.rs:380-453`. Permagent calls `recall()` (which internally does a neighborhood) at `routes/brain.rs:290-292`, but **discards the edges** — only entity nodes are emitted, no `Triple`/edge list in the response. |
| Edges surfaced to UI | **MISSING** | `routes/brain.rs` builds `GraphEntity` nodes only (`brain.rs:235-241,300-309`); no edge array is serialized. The World/Brain view has nodes, not connection lines. |

**Cross-store note (the architecturally-significant part).** A graph "person"
is a Kuzu `Entity` in `graph.kz`. A CRM "person" is a row in the `people`
table in `permagent.db` (`spectral_schema.rs:879-893`). **Nothing joins them**
— no FK, no shared key. The only soft bridge is a naming *convention*
(`canonical_id` slugs like `person:jane-doe`, `identity/canonical.rs:80-125`),
and even that is not used as a lookup across the two stores (grep for
people↔Brain references → 0). `recognition.rs:19` explicitly documents the
permagent.db tables as "distinct from Spectral's own" store. So a "Projects
view showing who's associated" must decide *which* person identity is
authoritative: the Kuzu graph entity (which has edges) or the CRM row (which
has structured fields). They are currently two unlinked populations.

---

## PART 2 — Description pipeline gap (root cause, pinned)

### How memory descriptions get written (the path that works)
Librarian batch loop → `list_undescribed` → `describe_one` → `set_description`:
- `librarian.rs:566-608` `run_batch` iterates `Vec<spectral::ingest::Memory>`.
- `brain_handle.rs:180-189` `list_undescribed` returns **memories only**.
- `librarian.rs:457-460` → `brain_handle.rs:222-230` `set_description(id, desc)`.
- Spectral target: `memories.description` (TEXT) +
  `memories.description_generated_at` — `spectral-ingest/src/sqlite_store.rs:384-386`;
  API `brain.rs:1400-1404`.
- Scheduled: `routes/librarian/scheduling.rs:336-445` (daily window, batch 20,
  line 325). **Every pass — describe, consolidate, co-retrieval — is memories-only.**

So `set_description` writes the **memories** table in **memory.db**. It has no
entity_type parameter and structurally cannot target anything else.

### Do People / Projects / Topics have a description field?
| Entity | Store | Description column? | Written by? | Read by modal? |
|---|---|---|---|---|
| **Topic** (Brain entity) | Kuzu `graph.kz` | **YES** — `Entity.description` `schema.rs:88`; struct field doc-commented *"set by Librarian"* `kuzu_store.rs:62-63` | **NOTHING** — `set_entity_description` (`brain.rs:1407-1413`) has **0 callers** in `crates/` | reads `selected.note` `BrainView.tsx:339` |
| **People** | permagent.db | **NO** — only `notes` `spectral_schema.rs:879-893` / `people.rs:33-48` | n/a | **no frontend** consumes `/api/people` at all |
| **Project** | permagent.db | **YES** — `description NOT NULL DEFAULT ''` `spectral_schema.rs:585` | project create/edit flow | `project.description` `ProjectsView.tsx:302` — **works** |

### Is the Librarian loop scoped to memories only?
Yes, structurally. The loop's element type is `spectral::ingest::Memory`
(`librarian.rs:566`); there is no entity_type branch, no `list_undescribed_people`,
no `list_undescribed_entities`. The describe loop **was never invoked for
non-memory types** — there is nothing to invoke. This is the STOP-and-flag
finding the dispatch predicted: **the pipeline was genuinely never wired for
non-memory types.**

### The modal read path — why each is empty
- **Topic:** double gap. (a) Nothing ever writes `Entity.description`
  (`set_entity_description` uncalled). (b) Even if it were written, the route
  **throws it away**: `routes/brain.rs:304` hardcodes `note: String::new()`
  while iterating real graph entities (`brain.rs:297-307`) — so the modal's
  `selected.note` is *always* empty by construction. This is a write-gap **and**
  a read-gap stacked.
- **People:** the column doesn't exist (no `description`), and no UI reads it.
  Two missing pieces, not a wiring bug.
- **Project:** works end-to-end. Confirms the mechanism is sound when all three
  pieces (column + writer + reader) are present.

### PINNED root cause
There is no single bug; there are **three distinct causes**, one per entity:

1. **Topics — write-gap + hardcoded-empty read.** Column exists and Spectral
   even labels it "set by Librarian", but Permagent's Librarian only writes
   *memory* descriptions, never `set_entity_description`; and the graph route
   hardcodes `note` to `""` (`routes/brain.rs:304`). **Fix direction:** (a) add
   an entity-describe pass that calls `Brain::set_entity_description`, and
   (b) change `routes/brain.rs:304` to read `ent.description` (the Spectral
   `Entity` struct already carries `description: Option<String>`,
   `kuzu_store.rs:62`).
2. **People — missing column + missing UI.** **Fix direction:** add a
   `description`/`bio` column to the `people` table (new migration), a writer,
   and the absent People modal/card. (Largest of the three.)
3. **Projects — none.** Already correct; use as the reference pattern.

The recurring failure is explained: every prior attempt assumed
`set_description` was a general "describe any entity" primitive. It is not — it
is hardwired to the memories SQLite table, a different store from where People
and Projects live, and a different *column* from where Topics are read.

---

## PART 3 — Enrichment sub-agent feasibility

Henry dispatching a web-search worker to enrich a person profile is feasible and
matches the proven orchestrator pattern, **but the write target depends on which
"person" we mean** (the Part-1 cross-store split). Two clean options:

- **Target the Brain graph entity** → `Brain::set_entity_description(entity_id,
  bio)` (`brain.rs:1407`). This is the *intended* sink ("set by Librarian"), and
  the future per-entity describe pass would write the same column — so an
  enrichment worker is just a richer describer for one entity. Blocker: the read
  route must stop hardcoding `note` (Part 2, fix 1b) or the bio won't show.
- **Target the CRM row** → needs the new `people.description` column (Part 2,
  fix 2) and a write endpoint (`/api/people` is read-only today, `people.rs:8`).

Recommendation lands on the Brain-entity sink: the write API already exists, it
unifies with the Librarian describe pass, and it's where graph edges already
live (so the "click person → connected projects" payoff comes from the same
node). No new storage primitive needed — only the read-route fix. The CRM-row
path needs both a column and a write endpoint first.

---

## Overall sizing — is this "wire existing pieces"?

**Mixed, and honest about it:**

- **Relationship graph: wire existing pieces.** Spectral's edge store is
  complete and battle-tested. The work is Permagent-side: (a) *write* edges
  (call `assert`/`assert_typed` somewhere — e.g. Librarian extraction or an
  enrichment worker), and (b) *surface* edges in `routes/brain.rs` (serialize the
  `Triple`s already returned by `recall().graph.neighborhood`, which are
  currently dropped). No new schema. Medium.
- **Topic descriptions: small.** One read-route fix (`brain.rs:304`) + one
  entity-describe pass (`set_entity_description`). Both APIs exist.
- **People bio: build.** New column + migration + writer + the entirely-absent
  People frontend. This is the heaviest single piece.
- **Projects: done.**

So the click-person→Brain-modal-with-bio flow is **partly wire-existing-pieces
(graph edges, topic bios) and partly genuine build (people column + people UI,
and an edge-writer since nothing currently asserts triples)**. The graph
*capability* is not the blocker — the *population* of it (no code writes edges or
entity descriptions) and the *cross-store identity* (graph-person vs CRM-person
are unlinked) are.

---

## STOP-and-flag (all three predicted conditions confirmed)

1. **Description pipeline was never wired for non-memory types — CONFIRMED.**
   `set_description`/`list_undescribed` are memory-only by construction; no
   entity-type dispatch exists. This is the root cause of the repeated failures.
2. **Graph needs a write layer, not a schema — CONFIRMED.** Edges/traversal
   exist in Spectral but **no Permagent code asserts a triple or sets an entity
   description**. The relationship *layer* (storage) is there; the *population*
   path is not built.
3. **Cross-store split is architecturally significant — CONFIRMED.** Graph
   entities (`graph.kz`) and CRM people (`permagent.db`) are unlinked; a
   product decision is required on which is the authoritative "person" before
   person-project linkage can be drawn. **This is a Jesse ruling, not an
   auto-build.**
