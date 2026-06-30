# Unified Agent-Managed Workspace — Architecture Design

**Status:** DECIDED — all five decision points (A–E) ruled 2026-06-29 (§7). Sequenced backlog, not in-flight;
the first build (B) is gated behind slice 2a (#537) being merged + dogfooded.
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

## 2. The core problem (#256): who owns a person? — RULED

> **DECISION A — RULED 2026-06-29: graph-authoritative, SQLite people table = PURE IDENTITY + LINKS only.**
> Attributes (`role, company, email, phone, notes, last_contact_at`) live **only** in the graph as `entity_fields`,
> read from there. **No denormalized attribute columns in SQLite — no projection, no cache by default.**

This is a step *beyond* a denormalized-projection design, and deliberately so. The split-authority option (graph owns
attributes, SQLite holds a fast projection of them) was on the table and rejected for one reason: **a projection is two
stores that can drift** — the exact bug class behind the dead-memories-table, #515, and half this session's incidents.
The fix is not *sync-two-stores-carefully*; it is *don't-have-two-stores*. One copy of each attribute, in the graph,
arbitrated by `FieldSource`.

### 2.1 The ruled model

**The people table holds identity and links only:**

```
people {
  entity_uuid       -- opaque, immutable PK (the stable handle; #530 FKs key on this)
  canonical_id      -- mutable display slug, UNIQUE
  display_name      -- identity, not an enrichable attribute
  graph_entity_id   -- NEW (Decision B): immutable bridge key to the graph node
  -- NO role / company / email / phone / notes / last_contact_at columns
}
```

Everything #530 needs (the `project_people.entity_uuid` FK) stays in SQLite because the *join key* is identity, not an
attribute. Everything enrichable moves to the graph. The boundary is now absolute: **SQLite owns *who* (identity + the
project/association links); the graph owns *everything we know about them*.**

### 2.2 Why this is structurally one source of truth

The three questions that gate any attribute-authority design — arbitration, staleness, drift — collapse to trivial under
this ruling, because there is **no second copy of an attribute anywhere:**

**(1) Manual-vs-Enricher arbitration — trivially correct, one field, in the graph.**
There is one `role` field, in `entity_fields`, with one `FieldSource`. A human edit (2b) writes `FieldSource::Manual`;
the Enricher (4) writes `FieldSource::Enriched`; manual beats enriched (#499, already built). There is no second `role`
column to reconcile against — arbitration is the whole mechanism, not a step before a copy.

```
write(entity_uuid, field, value, source)              // source = Manual (2b) | Enriched (4)
  ├─ graph_id = people.graph_entity_id                 // Decision B, persisted (not derived)
  └─ set_entity_field(graph_id, field, value, source)  // graph applies manual-wins; DONE. no projection step.
```

**(2) Staleness window — none, because there is nothing to refresh.**
Every read of an attribute reads the graph. There is no SQLite copy that could lag a graph write. "No stale reads ever"
is satisfied by construction, not by careful sync.

**(3) Drift — cannot occur, no reconciliation primitive needed.**
Drift requires two copies of a value. There is one. There is no `reproject`, no boot reconciliation sweep, no "which
store do we believe" — those mechanisms exist only to manage a projection, and there is no projection. The failure class
is designed out, not mitigated.

### 2.3 Migration implication for slice 2a (note, do not act)

Slice 2a's person modal and People panel **currently read attributes from the people-table list response** (`GET
/api/people` returns the `role/company/email/...` columns). Under Decision A those columns go away. Sequencing
consequence (see §6):

1. **Slice 2a lands first, as-is** — it ships against today's people-table columns. Decision A does not block it.
2. **B/2b then move the attribute source to the graph** — the read path for attributes switches from people-table
   columns to `entity_fields`, and the columns are dropped in the same migration that adds `graph_entity_id`.

So 2a is *not* rebuilt; its attribute-source is *relocated* in the B/2b work, after 2a is merged and dogfooded.

---

## 3. The bridge (canonical_id ↔ EntityId) — RULED

> **DECISION B — RULED 2026-06-29: persist `graph_entity_id` immutably on the people row, set once at creation.**
> Not derive-on-read. This is the **first build** (the column + persist-on-create + migration) and it is now
> **load-bearing for Decision A** — the identity-only people row keys to the graph (where all attributes live) *through
> this column*. Without it, an identity-only row has no durable way to reach its attributes.

The graph EntityId must survive a `canonical_id` rename, so it **cannot** be re-derived from `canonical_id` at read time.

Add a `graph_entity_id TEXT` column to the people table, populated at person creation from `canonicalize_entity_id(...)`
*at that moment*, and **never rewritten on rename**. Reads/writes against the graph use `graph_entity_id`; the user-facing
slug (`canonical_id`) is free to change without losing the graph anchor.

- **Bootstrap:** because the formats coincide today (§1.3), existing rows backfill by deriving once from their current
  `canonical_id`. Zero risk *for rows that have never been renamed* — and rename history doesn't exist pre-bridge, so
  the backfill is exact.
- **Why not derive-on-read (the zero-migration option):** it is correct only until the first rename and silently wrong
  after. It also couples two codebases' slug-normalization implementations forever — any divergence is a silent
  mis-join. Persisting decouples them.
- **Rename semantics become explicit:** `rename_canonical_id` changes the display slug *only*; `graph_entity_id` is
  immutable. If we ever need to re-key the graph node itself, that is a deliberate graph operation, not a side effect of
  a display rename.

- **Bootstrap:** because the formats coincide today (§1.3), existing rows backfill by deriving once from their current
  `canonical_id`. Rename history doesn't exist pre-bridge, so the backfill is exact.
- **Rename semantics become explicit:** `rename_canonical_id` changes the display slug *only*; `graph_entity_id` is
  immutable. Re-keying the graph node itself becomes a deliberate graph operation, never a side effect of a display rename.

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
  identity:    { canonical_id, ... }               // from people/projects table (identity only — Decision A)
  attributes:  [ { field_name, value, source, source_url, updated_at } ]  // entity_fields ONLY (Decision A), provenance intact
  relationships: [ { kind, target: UnifiedEntityRef, source } ]           // graph edges + association tables
  memories:    [ MemoryRef ]                        // brain recall + project_memories
}
```

A single **resolver** assembles it from the four backing sources (identity table, entity_fields, graph edges +
`project_people`/`project_memories`, brain recall). Surfaces request slices of it:

- **CRM People view** → `kind=Person`, identity + attributes (no project scoping).
- **Project Overview People panel** → `kind=Person` filtered by `project_people`, + the project-scoped `role`.
- **Marketing** (future) → same entities, a campaign-oriented slice (attributes + relationships).

The agent (#255) reads/writes through the *same* resolver — no surface-specific agent plumbing.

### 4.2 Company stays a string until slice 5 — RULED

> **DECISION C — RULED 2026-06-29: `company` stays a string attribute (an `entity_field`) until slice 5.**
> No entity machinery before there is a relationship to model.

Under Decision A, `company` is now a graph `entity_field` (a typed string), not a SQLite column. Promoting it to a
first-class `EntityPrefix::Org` entity unlocks company↔people↔project relationships (the spine of CRM + Marketing) but is
a real slice (an Org identity row + a `person.company` → Org reference). It is **not** required for 2b/4 and is the §6
slice-5 prerequisite for Marketing — deferred until then.

### 4.3 Where the resolver lives

A new read-only module (`crate::unified` or extend `project_association.rs`) composing `people.rs` (identity) +
`SafeBrain::entity_fields_for` (attributes) + `read_only_brain_conn` (memories). **No new store, no denormalization.** It
is a pure *view*, mirroring the `read_only_brain_conn` pattern: cross-store reads compose at the resolver at read time.
Under Decision A there is no write-time cache anywhere in the model — the resolver reads each fact from its single home.

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
| **2a** *(in-flight, #537)* | Read-only People panel + person modal + associate/disassociate in Project Overview | — | reads only (today's people-table columns) |
| **B** | `graph_entity_id` column + persist-on-create + migration **+ drop the people-table attribute columns**, relocating attribute reads to `entity_fields` | A + B ruled · **2a merged + dogfooded** | schema migration |
| **2b** | **Edit** person fields → `set_entity_field(..., Manual)` (the deferred editing slice) | B | graph write only (no projection) |
| **3** | `UnifiedEntity` resolver + single read endpoint; CRM People view + Project panel both consume it | C | new read module, no store |
| **4** | **Enricher** — background worker writes `FieldSource::Enriched` through the same `write()` seam | B + 2b verified | graph writes via wired `set_entity_field` |
| **5** | **Company as entity** (`EntityPrefix::Org`) + Marketing surface as a third consumer of the shared model | C → promote | Org identity rows |
| **6** | Full agent-managed workspace: agent reads/writes `UnifiedEntity` across all three surfaces; cross-surface goal origination | 2b–5 | — |

**Critical path:** 2a (merged+dogfooded) → B → 2b → *(manually verify the write path by hand)* → 4. The bridge (B) is the
single linchpin: it carries the identity-only people row to its graph attributes (Decision A) **and** unblocks both human
editing (2b) and the Enricher (4). **2b precedes 4 deliberately** (D) — manual editing is lower-risk and proves the graph
write path + the `FieldSource` arbitration *with a human in the loop* before the Enricher writes autonomously through the
same seam. Prove the mechanism by hand, then automate. Slice 3 (the resolver) is **parallelizable** — read-only, depends
only on C, not on the write path. Marketing (5) and the unified agent (6) are downstream of everything.

**The 2a → B attribute-source migration (Decision A consequence):** 2a ships reading attributes from the people-table
list response *as it exists today*. B's migration drops those columns and relocates the attribute read to `entity_fields`.
2a is therefore **not rebuilt** — its attribute-source is *relocated* in B/2b, which is why B is gated on 2a being merged
and dogfooded first (touching 2a's read path before it has landed would collide).

### Self-knowledge note

Slices 2b/3/4/5 each add an *agent-visible capability* (edit a person, query the unified model, the Enricher worker, the
Marketing surface). Per the standing rule, **each of those build slices must ship its `<permagent_self>` descriptor in
the same change** — a SURFACE descriptor for the new lens, a WORKER descriptor for the Enricher, gated on the same flag
as the feature. This doc flags the obligation; the slices own the implementation.

---

## 7. Decision points — ALL RULED 2026-06-29

| # | Decision | Ruling | Gates |
|---|---|---|---|
| **A** | Authoritative store for a person's attributes | ✅ **graph-authoritative; SQLite people table = PURE IDENTITY + LINKS only** (`entity_uuid, canonical_id, display_name, graph_entity_id`, + #530 project FK links). **No attribute columns in SQLite, no projection, no cache.** Attributes (`role/company/email/phone/notes`) live only in `entity_fields`, read from the graph. A projection is "two stores that can drift" (dead-memories-table, #515); the fix is don't-have-two-stores. Manual-wins is then trivially correct — one field, `FieldSource::Manual` beats `Enriched`, nothing to reconcile. (§2) | slice 2b, slice 4 (everything) |
| **B** | The canonical_id ↔ EntityId bridge | ✅ persist `graph_entity_id` on the people row, at creation, **immutable across rename** (not derive-on-read — correct today, severs silently on first rename). **First build.** Now load-bearing for A: the identity-only row reaches its graph attributes through this column. (§3) | the entire write path |
| **C** | Company a first-class entity now? | ✅ **defer to slice 5.** `company` stays a string `entity_field` until Marketing needs company-level relationships; no entity machinery before a relationship to model. (§4.2) | slice 5 |
| **D** | Slice ordering | ✅ **2a → B → 2b → *(verify by hand)* → 4**, slice 3 (resolver) parallel. 2b (manual editing) before 4 (Enricher): prove the graph write path + arbitration with a human in the loop before the Enricher writes autonomously through it. (§6) | the whole roadmap |
| **E** | Read path for attributes | ✅ **resolved by A — no SQLite projection exists, so both the panel list AND the modal read attributes from the graph** (satisfies "no stale reads ever"). **Mechanism is empirical: MEASURE graph attribute-read latency for a realistic N-person panel first.** If per-render read-through is panel-acceptable, read-through is the default — simplest, drift-proof, no cache. **Only if measured-slow**, add a read cache as an explicit optimization layer (never the authoritative store). Measure before building any cache. | slice 2b/3 perf |

**Build status:** all five ruled — this is now decided, buildable architecture — but **nothing builds from this doc yet:**
1. Gated behind **slice 2a (#537) being dogfooded + merged.** Decision A relocates where 2a's modal sources attributes
   (people-table columns → graph), so 2a lands first as-is, then B/2b move the source.
2. **B is the first build** (the `graph_entity_id` column + persist-on-create + the migration that drops the attribute
   columns) — a Rust/schema change that waits for a clean rebuild slot.

Sequenced backlog (`B → 2b → 4`, slice 3 parallel), **not in-flight.**

---

## 8. What this design explicitly does NOT do

- Does **not** touch `ProjectOverview.tsx` / people components (slice 2a owns them).
- Does **not** add the `graph_entity_id` column, wire `set_entity_field`, or build the resolver — those are slices,
  authored after the rulings.
- Does **not** re-key the Brain by project (§5) or move identity into the graph (§2.1) — both would break #530's FKs.
- Does **not** introduce a third store, a projection, or a cache. The only new persisted artifact proposed is one
  *column* (`graph_entity_id`, the bridge). Decision A removes the attribute columns rather than adding any.
