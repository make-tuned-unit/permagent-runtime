# Unified Agent-Managed Workspace — Architecture Design

**Status:** DESIGN — awaiting Jesse's rulings on the decision points (§7).
**Scope:** #255 (unifying epic) · #256 (CRM as Brain-backed People view) · #257 (cross-surface context layer).
**Constraint:** This document designs *forward* from slice 2a's read-only reality. It rules on
authority, the ID bridge, and the phased path; it does **not** rebuild #530 / #499 / slice 2a.

---

## 1. Established reality (audited, do not redesign)

These are facts confirmed against `origin/main` (cd4f63e6f), not assumptions.

### 1.1 Two stores, two ID schemes

| | **Operational store** | **Semantic store** |
|---|---|---|
| Backing | `permagent.db` (SQLite, `spectral/permagent.db`) | Spectral Brain (Kuzu graph + `memory.db`) |
| Person key | `entity_uuid` (opaque UUIDv7, **immutable**) | `EntityId` (opaque newtype, **stable graph key**) |
| Person handle | `canonical_id` (`"person:jesse-sharratt"`, **mutable**, `UNIQUE`) | `prefix:slug` string (same shape) |
| Person attributes | `display_name, role, company, email, phone, notes, last_contact_at` columns | `EntityField{ field_name, value, source, source_url, updated_at }` rows |
| Provenance | none (plain columns) | `FieldSource::Manual \| Enriched`, **manual-takes-precedence already enforced** |
| Write path | `people.rs::upsert_person / rename_canonical_id` (live) | `SafeBrain::set_entity_field` **exists but UNWIRED — no route calls it** |
| Read path | `GET /api/people`, `GET /api/projects/{id}/people` | `GET /api/brain/graph` → `SafeBrain::entity_fields_for` (live) |

- people table: `spectral_schema.rs:761`. PK `entity_uuid`, mutable `canonical_id UNIQUE`. No FK.
- entity_fields: `brain_handle.rs:237–296`. `EntityField{field_name, value, source: FieldSource, source_url, updated_at}`.
- `set_entity_field()` is implemented and tested (`spectral_smoke.rs`) but **never called in production**. The write
  path is one route away — the blocker is not code volume, it is *deciding who owns the field*.

### 1.2 #530 association layer (schema v20, LIVE)

```sql
project_people  (project_id FK→projects, entity_uuid FK→people, role, added_at)   -- real FKs, ON DELETE CASCADE
project_memories(project_id FK→projects, memory_id TEXT /* no FK */, added_at)    -- memory_id resolves via read_only_brain_conn
```

People and memories associate to projects. `project_people.role` is **project-scoped** and distinct from the
person's global `people.role`. `project_memories.memory_id` is a Spectral memory id with no FK — reads INNER-JOIN
against `memory.db` and silently drop orphans.

### 1.3 The bridge — ABSENT but structurally pre-aligned

No code maps `people.entity_uuid` or `people.canonical_id` → graph `EntityId`. **However:**

- `canonicalize_entity_id(Person, "Jesse Sharratt")` → `"person:jesse-sharratt"` (`canonical.rs:80`)
- Spectral `entity_id("person", "jesse-sharratt")` → `EntityId` with the **identical** normalization and `prefix:slug` form.

So the two ID strings already coincide *today*. The trap: `canonical_id` is **mutable** (rename), `EntityId` is the
graph's **stable** key. Deriving the bridge on-read from `canonical_id` works until the first rename, then silently
points at the wrong (or no) graph node. This is the linchpin decision (§7-B).

### 1.4 Surfaces

- **Projects:** `ProjectWorkspace.tsx` chrome (back / switcher / `ViewToggle`) over two lenses: `'overview' | 'kanban'`.
  `ProjectOverview.tsx` renders Summary / Key Facts / Links / Tasks with **reserved comment slots** for People,
  Memories, Documents. Slice 2a fills the People slot (read-only) — in-flight on `feat/crm-people-slice2`.
- **CRM:** `GET /api/people` (filter `company`/`role`/`q`) returns the full `Person` row. The People view (#256) reads it.
- **Marketing:** **does not exist** — zero references in `ui/command-center/src`. Purely aspirational.
- **Brain:** one `SafeBrain` per daemon (`AppState.brain: Option<SafeBrain>`). Global recall, no project scoping.

---

## 2. The core problem (#256): who owns a person?

Authority is split, and the split is *load-bearing* — it gates editing (slice 2b) and the Enricher (slice 4):

- **people table** is editable-in-principle, fast to list/filter, but **provenance-blind**. If the Enricher writes here,
  it overwrites a user's manual edit with no record of which won or why.
- **entity_fields** is provenance-native (`FieldSource`, `source_url`, manual-wins), the natural Enricher target, but
  is read-only end-to-end today and has no fast list/filter surface (every read is a graph traversal).

Three candidate rulings:

| | A — people table is truth | B — graph is truth | **C — split authority by provenance (RECOMMENDED)** |
|---|---|---|---|
| Source of truth | SQLite columns | entity_fields | **Identity in SQLite, attributes in graph** |
| Enricher writes to | — (can't, no provenance) | entity_fields | entity_fields (`Enriched`) |
| User edits write to | columns | entity_fields (`Manual`) | entity_fields (`Manual`) **and** project the canonical few back to columns |
| Provenance | lost | preserved | preserved |
| Fast list/filter | native | needs a projection/cache | **native** (columns are the projection) |
| Conflict rule | none | `FieldSource` manual-wins (built) | `FieldSource` manual-wins (built) |
| Bridge required | no | yes | yes |

### Recommended ruling — Option C: *the graph is authoritative for typed attributes; the people table is the identity anchor + denormalized read projection.*

Rationale:

1. **The conflict-resolution machinery already exists** in the right place. `FieldSource::Manual | Enriched` with
   manual-takes-precedence is exactly the user-vs-Enricher arbitration slice 4 needs. Option A would force us to rebuild
   that on plain columns. Option C *uses what #499 already shipped.*
2. **Identity ≠ attributes.** `entity_uuid` (stable handle), `canonical_id` (display slug), and `display_name` are
   *identity* — they must live in SQLite because they are the join key for #530's FKs (`project_people.entity_uuid`
   REFERENCES `people`). You cannot move the join key into the graph without breaking the association layer. So SQLite
   *keeps* identity by necessity.
3. **`role / company / email / phone` become a projection.** They are typed attributes → they belong in entity_fields as
   the authoritative copy. The people-table columns become a **materialized read projection** of the graph's *manual*
   fields, refreshed on write, so `GET /api/people` stays a single fast indexed SQLite query (no graph traversal per
   row in a list of hundreds). The list reads the projection; the modal reads the full provenance-rich graph record.

This makes the boundary crisp: **SQLite owns *who* (identity + a fast denormalized card); the graph owns *what we know*
(typed, sourced, arbitrated attributes).** Writes flow into the graph; the projection follows.

### 2.1 Why Option C is one source of truth, not two stores that drift

The objection to any two-table design is the bug class behind half this codebase's incidents: *two stores that can
disagree, and no rule for which is right.* Option C does not have two sources of truth. It has **one source (the graph)
and one cache (the projection columns).** The distinction is structural, not nominal, and it answers the three questions
that gate this ruling:

**(1) Where does manual-vs-Enricher arbitration live? — The graph only. The projection never arbitrates.**

There is exactly one arbiter: `FieldSource` manual-takes-precedence, inside the graph (already built, #499). The write
path for *every* writer — human edit (2b) and Enricher (4) — is a single choke point:

```
write(entity_uuid, field, value, source)            // source = Manual (2b) | Enriched (4)
  ├─ graph_id = bridge(entity_uuid)                  // §3 persisted key
  ├─ set_entity_field(graph_id, field, value, source)  // graph applies manual-wins HERE, the only place
  ├─ authoritative = entity_fields_for(graph_id)[field] // read back the POST-arbitration value
  └─ if field ∈ {role, company, email, ...}: project authoritative → people column
```

The projection write takes the value the graph *already decided*. It never sees a conflict, because conflict resolution
happened upstream and it only ever copies the winner. A human edit and an Enricher write that target the same field both
go through `set_entity_field`; the graph picks the winner by `FieldSource`; the projection mirrors whatever won. **The
SQLite column has no write-time conflict logic because it is never a writer's target — only the graph is.**

**(2) The staleness window — closed by a single write choke point.**

Refresh is *synchronous and in-band*: the projection is written in the same handler, immediately after the graph write,
as step 4 above. There is no async lag for any write that goes through the choke point. The window is non-zero only for a
writer that bypasses it — which is why the design rule is **all entity_fields writes go through the one
`write()` seam, including the Enricher.** The Enricher (slice 4) calls the same function as the 2b edit handler; it does
not write the graph raw. Given that discipline, a list read (projection) and a modal read-through (E, live graph) cannot
disagree on a *committed* write, because the projection commits with it. The only values that differ between projection
and graph are ones mid-flight in a single request — not a persisted state either surface can observe.

**(3) Drift failure mode — self-healing by overwrite, never reconciliation.**

Because the projection holds **zero authoritative state** — every column value is a pure function of the graph's current
post-arbitration fields — drift can only ever be a *stale cache*, never a *competing truth*. Healing is therefore an
unconditional re-derivation, not a merge:

```
reproject(entity_uuid):                  // idempotent, pure overwrite-from-truth
  graph_id   = bridge(entity_uuid)
  fields     = entity_fields_for(graph_id)
  people.columns ← project(fields)       // overwrite; the graph is right by definition
```

`reproject` runs (a) on-demand, (b) as a boot reconciliation sweep over all people, (c) on any detected mismatch. It can
never make a wrong decision because there is no decision — it copies truth downward. This is the same discipline as B:
one immutable/authoritative key, derived views below it. The "two stores that drift" class requires two *sources of
truth*; Option C structurally has one, so the class cannot arise. If the projection is ever wrong, the answer is always
"re-project," never "figure out which store to believe."

> **DECISION POINT A** (§7) — confirm Option C, or pick A/B. Everything downstream (write path, Enricher, the modal's
> data source) keys off this. §2.1 answers the arbitration / staleness / drift questions this ruling depends on.

---

## 3. The bridge (canonical_id ↔ EntityId)

Option C requires a durable `entity_uuid → EntityId` mapping. The graph EntityId must survive a `canonical_id` rename,
so it **cannot** be re-derived from `canonical_id` at read time.

### Recommended: persist the graph key on the people row, set once at creation.

Add a `graph_entity_id TEXT` column to the people table (a future migration, **not built here**), populated at person
creation from `canonicalize_entity_id(...)` *at that moment*, and **never rewritten on rename**. Reads/writes against the
graph use `graph_entity_id`; the user-facing slug (`canonical_id`) is free to change without losing the graph anchor.

- **Bootstrap:** because the formats coincide today (§1.3), existing rows backfill by deriving once from their current
  `canonical_id`. Zero risk *for rows that have never been renamed* — and rename history doesn't exist pre-bridge, so
  the backfill is exact.
- **Why not derive-on-read (the zero-migration option):** it is correct only until the first rename and silently wrong
  after. It also couples two codebases' slug-normalization implementations forever — any divergence is a silent
  mis-join. Persisting decouples them.
- **Rename semantics become explicit:** `rename_canonical_id` changes the display slug *only*; `graph_entity_id` is
  immutable. If we ever need to re-key the graph node itself, that is a deliberate graph operation, not a side effect of
  a display rename.

> **DECISION POINT B** (§7) — persist `graph_entity_id` (recommended) vs derive-on-read. Gates the whole write path.

---

## 4. Cross-surface entity model (#257)

### 4.1 The shared read model: `UnifiedEntity`

Today each surface invents its own shape (`Person`, `ProjectPerson`, `GraphEntityField`). #257's job is **one read
model, sliced** — surfaces differ in *which slice they request*, not in the model.

```
UnifiedEntity {
  kind:        Person | Company | Project          // EntityPrefix already has Person/Org/Project
  id:          entity_uuid (or project id)         // stable handle
  graph_id:    EntityId                            // bridge key, present once §3 lands
  display_name
  identity:    { canonical_id, ... }               // from people/projects table
  attributes:  [ { field_name, value, source, source_url, updated_at } ]  // entity_fields, provenance intact
  relationships: [ { kind, target: UnifiedEntityRef, source } ]           // graph edges + association tables
  memories:    [ MemoryRef ]                        // brain recall + project_memories
}
```

A single **resolver** assembles it from the four backing sources (identity table, entity_fields, graph edges +
`project_people`/`project_memories`, brain recall). Surfaces request projections:

- **CRM People view** → `kind=Person`, identity + attributes (no project scoping).
- **Project Overview People panel** → `kind=Person` filtered by `project_people`, + the project-scoped `role`.
- **Marketing** (future) → same entities, a campaign-oriented slice (attributes + relationships).

The agent (#255) reads/writes through the *same* resolver — no surface-specific agent plumbing.

### 4.2 Company becomes a first-class entity (decision)

Today `company` is a free-text column on `people`. #257's "entity surfaces consistently across all three" implies
**Company is an entity**, not a string — `EntityPrefix::Org` already exists. Promoting Company unlocks
company↔people↔project relationships (the spine of CRM + Marketing) but is a real slice (an org identity row + a
`person.company` → Org reference). It is **not** required for slice 2b/4; it is the §6 slice-5 prerequisite for
Marketing.

> **DECISION POINT C** (§7) — is Company a first-class entity in v1 of the shared model, or does it stay a string until
> Marketing needs it? Recommendation: **defer to slice 5** — keep `company` a projected string attribute until a surface
> needs company-level relationships, to avoid a migration with no consumer.

### 4.3 Where the resolver lives

A new read-only module (`crate::unified` or extend `project_association.rs`) composing `people.rs` +
`SafeBrain::entity_fields_for` + `read_only_brain_conn`. **No new store.** It is a *view*, mirroring the
`read_only_brain_conn` pattern: cross-store reads compose at the resolver, never via a new persisted denormalization
(beyond the §2 projection, which is a write-time cache, not a third source of truth).

---

## 5. Project scoping (the cross-cutting gap)

Brain recall is **global** — there is no `project_id` on memories or entity_fields. #530 bolts project scope on *beside*
the graph via SQLite join tables (`project_people`, `project_memories`), not *inside* it. This is the right seam and the
design preserves it: **project scope is an association-table concern, not a graph-schema concern.** The resolver scopes a
surface by filtering through the join tables; the graph stays global and unaware of projects. This avoids re-keying the
entire Brain by project (a closed-#70-style trap noted across prior work) and keeps a person/company shareable across
projects by construction.

---

## 6. Phased slice sequence (#255)

From slice 2a forward. Each slice is independently shippable and gated only by the ruling it depends on.

| Slice | Deliverable | Depends on | Store touch |
|---|---|---|---|
| **2a** *(in-flight)* | Read-only People panel + person modal + associate/disassociate in Project Overview | — | reads only |
| **2b** | **Edit** person fields (the deferred editing slice) | **A + B** | write path → graph; projection refresh |
| **3** | `UnifiedEntity` resolver + single read endpoint; CRM People view + Project panel both consume it | C (Company ruling) | new read module, no store |
| **4** | **Enricher** — background worker writes `FieldSource::Enriched` into entity_fields | A + B (bridge live) | graph writes via wired `set_entity_field` |
| **5** | **Company as entity** + Marketing surface as a third consumer of the shared model | C = yes | Org identity rows |
| **6** | Full agent-managed workspace: agent reads/writes `UnifiedEntity` across all three surfaces; cross-surface goal origination | 2b–5 | — |

**Critical path:** A → B → 2b → *(manually verify the write path)* → 4. The bridge (B) is the single linchpin: it
unblocks both human editing (2b) and the Enricher (4). **2b precedes 4 deliberately** (D) — manual editing is
lower-risk and proves the write path + B's bridge + the `FieldSource` arbitration *with a human in the loop* before the
Enricher writes autonomously. Get the write path proven by hand, then let the Enricher use the same `write()` seam (§2.1).
Slice 3 (the resolver) is **parallelizable** — it is read-only and depends only on the Company ruling (C), not on the
write path. Marketing (5) and the unified agent (6) are downstream of everything.

### Self-knowledge note

Slices 2b/3/4/5 each add an *agent-visible capability* (edit a person, query the unified model, the Enricher worker, the
Marketing surface). Per the standing rule, **each of those build slices must ship its `<permagent_self>` descriptor in
the same change** — a SURFACE descriptor for the new lens, a WORKER descriptor for the Enricher, gated on the same flag
as the feature. This doc flags the obligation; the slices own the implementation.

---

## 7. Decision points (need Jesse's ruling)

| # | Decision | Ruling | Gates |
|---|---|---|---|
| **A** | Authoritative store for a person's typed attributes | **PENDING Jesse's read** — recommended **Option C** (graph authoritative; people table = identity anchor + projection). §2.1 answers the arbitration / staleness / drift questions this ruling depends on. | slice 2b, slice 4 (everything) |
| **B** | The canonical_id ↔ EntityId bridge | ✅ **RULED 2026-06-29** — persist `graph_entity_id` on the people row, set at creation, **immutable across rename** (not derive-on-read). Derive-on-read is correct today but severs silently on the first rename — the "correct-by-current-conditions, breaks on state change" class. Build B as the foundation. | the entire write path |
| **C** | Is Company a first-class entity in the shared read model? | ✅ **RULED 2026-06-29** — **defer to slice 5.** Keep `company` a projected string until Marketing needs company-level relationships; no entity machinery before a relationship to model. | slice 3 shape, slice 5 |
| **D** | Slice ordering | **PENDING A** — A→B→**2b→4** confirmed direction: human editing (2b) proves the write path + bridge + arbitration **with a human in the loop** *before* the Enricher writes autonomously (4). Gate inserted: `A→B→2b→(manually verify the write path)→4`. Confirm once A is ruled. | the whole roadmap |
| **E** | Projection direction in Option C | ✅ **RULED 2026-06-29** — **projection for list reads, read-through for the modal.** Lists hit the fast SQLite projection; the modal reads through to the live graph for fresh/rich provenance. | slice 2b/3 perf |

**Build status:** B/C/E ruled, but **nothing builds from this doc yet.** B is the buildable foundation and is held for a
future session — it is a Rust change (`graph_entity_id` column + persist-on-create path + migration), it should not start
while slice 2a (#537) is unmerged/undogfooded (2a is the read layer B's write path extends), and **A must be ruled first**
so B is built knowing the authoritative-store model. Scoped, not started.

---

## 8. What this design explicitly does NOT do

- Does **not** touch `ProjectOverview.tsx` / people components (slice 2a owns them).
- Does **not** add the `graph_entity_id` column, wire `set_entity_field`, or build the resolver — those are slices,
  authored after the rulings.
- Does **not** re-key the Brain by project (§5) or move identity into the graph (§2.2) — both would break #530's FKs.
- Does **not** introduce a third store. The only new persisted artifact proposed is one *column* (the bridge) and a
  *write-time projection* of already-authoritative graph data.
