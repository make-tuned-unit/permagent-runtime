# People-in-Graph v1 — Durable Runtime Person Creation

**Status:** DESIGN — awaiting ruling. Zero code this round. Six decision points in §8.
**Scope:** Designs the *one* durable path by which a person becomes a graph entity at runtime,
survives the reconciler, and mints into the CRM directory. Foundation for #578 (Henry create/associate
tool), the UI "add person" action, and the v1.5 conversation-extraction epic (#256) — all three are
callers of the single path this document specs.
**Tracks:** #256 (CRM as Brain-backed People view + enrichment) · unblocks #578 · part of epic #255.
**Constraint:** Designs *forward* from the audited reality below. It does **not** rebuild #530 / #554 /
the ontology / the pinned `spectral` dep. The pinned rev (`7300ad8`) stays frozen unless Decision D
rules otherwise.

---

## 1. Established reality (audited against `origin/main` e78b8747e + pinned `spectral` 7300ad8)

These are facts read from code, not assumptions. File:line cited so the ruling can be checked.

### 1.1 A person exists in three places, keyed by one content-addressed id

| Layer | Backing | How a person gets here |
|---|---|---|
| **Ontology** | `~/.permagent/brain/ontology.toml` (14 `type="person"` entries today) | Hand-curated TOML. `Ontology` is **`Deserialize`-only** — no serializer, no `add`/`save`, and **zero** ontology-write code exists in permagent. |
| **Graph** | Kuzu `graph.kz`, node table `Entity` | A node is written **only when a triple is asserted about it** (`KuzuStore::upsert_entity`, `kuzu_store.rs:223`). `Brain::open` (`brain.rs:420`) does **not** eagerly seed the ontology into Kuzu. |
| **Directory** | `permagent.db` `people` table | Minted by the bridge at startup (`people_bridge::sync_people_from_ontology`). |

The three share **one key**: `EntityId = blake3(SALT + "person:" + canonical)`
(`spectral-core/entity_id.rs:64`). permagent's `graph_entity_id_hex("person", name)`
(`identity/canonical.rs:150`) computes the identical hash. **Content-addressed ⇒ the same name
produces the same id in all three layers ⇒ dedupe is automatic and free.** This is the single most
important fact in the design: an ontology "sabaa quao" and a runtime "sabaa quao" are the *same node*,
not two.

### 1.2 The startup pipeline (all person machinery runs here, once)

`goose-server/src/state.rs:151–191, 269–270`, in order:

1. `brain_sync::sync_graph_with_ontology(brain_dir, ontology_path)` — the **reconciler**.
2. `Brain::open(...)` with the ontology path.
3. `people_bridge::sync_people_from_ontology(pool, ontology_path)` — the **bridge**.

### 1.3 The reconciler is prune-only and prunes everything not in the ontology

`brain_sync.rs:sync_graph_with_ontology` (permagent-side, `goose-server`):
- Gated on an md5 of `ontology.toml` — runs only when the file changed.
- Backs up `graph.kz`, then `MATCH (e:Entity) RETURN e.id, e.canonical, e.entity_type`.
- For every graph entity **not** in the ontology's id set: `DETACH DELETE`. It **never adds**;
  it only deletes.

**This is the B failure mode.** A person written to the graph at runtime is, by construction, not in
the curated ontology → the next ontology edit **silently deletes it**. Any runtime-create built on top
of today's reconciler is a data-loss bug waiting for the curator's next keystroke.

### 1.4 The bridge reads the ontology, NOT the graph

`people_bridge::sync_people_from_ontology` loads `Ontology::load(...)` and iterates its `person`
entities, calling `upsert_identity_from_graph(pool, canonical, graph_entity_id)` per person — which
mints an identity-only `people` row (opaque `entity_uuid` + the content-addressed `graph_entity_id`).

> **Dispatch-premise correction:** the brief states "the bridge mints from the graph regardless of
> source (it already enumerates graph persons)." It does **not** — it enumerates the *ontology*. And
> switching it to enumerate the graph would today mint *fewer* than 14, because the graph only holds
> persons that have an asserted triple (§1.1). The design therefore keeps the ontology bridge **and
> adds** a graph bridge; it does not replace one with the other (§5).

### 1.5 The runtime graph-write primitive exists but is unsurfaced

- `KuzuStore::upsert_entity(&Entity)` (`kuzu_store.rs:223`) `MERGE`s a node by id — idempotent, the
  right primitive.
- `SafeBrain` (`brain_handle.rs`) wraps Brain reads/writes in `spawn_blocking`, but exposes **no
  entity-create** method. `set_entity_field`, `remember_with`, `entity_fields_for` exist; there is no
  `create_entity`.
- spectral's `Brain` has `runtime_entities: Mutex<Vec<OntologyEntity>>` (`brain.rs:391`) for
  `EntityPolicy::AutoCreate`, but it is **in-memory and re-initialized empty on every open**
  (`brain.rs:494`) — **not** a durable provenance store. We do not build on it.
- `EntityPolicy` default is `Strict` (`brain.rs:31`) — `assert()` on an unknown entity *fails*. So
  extraction does not silently create graph persons today.

### 1.6 Association only needs the directory row

`project_association::associate_person(pool, project_id, entity_uuid, role)` — FK is on
`people.entity_uuid`. It needs the `people` row, nothing from the graph. **So the instant the bridge
mints the row, associate works** — graph triples/attributes can lag.

---

## 2. The problem, precisely

A runtime `create_person("Sabaa Quao")` must produce a person that:

1. Is a real graph node (graph-authoritative identity, not a `people`-table row inventing a dangling
   `graph_entity_id` — the rejected `upsert_person` compromise).
2. **Survives the reconciler across restarts** — the load-bearing requirement. Everything else is
   plumbing; this is the design.
3. Mints into the `people` directory so it is visible and associable.

The obstacle is §1.3: the reconciler cannot today tell "curated person the curator deleted from the
ontology" (should prune) from "person created at runtime" (must never prune). It has no notion of
**where an entity came from**. That notion — **provenance** — is the whole design.

---

## 3. The durable design in one picture

```
create_person(name, source)                     [ONE path — §5]
   │
   ▼
SafeBrain::create_person_entity(name, visibility, source)   [§4.2 — new, validated, narrow]
   │   1. canonicalize + validate (reject empty-after-normalization)
   │   2. id = entity_id("person", canonical)          (content-addressed, §1.1)
   │   3. WRITE PROVENANCE ROW  (permagent.db, source)  ← durable FIRST  [§4.1]
   │   4. KuzuStore::upsert_entity(person node)         ← MERGE by id
   ▼
upsert_identity_from_graph(pool, canonical, id_hex)  [§4.4 — existing bridge fn, reused]
   │   → mints people row, returns entity_uuid
   ▼
associate_person(pool, project_id, entity_uuid, role)  [existing, unchanged]

── on restart ─────────────────────────────────────────────────────────────
sync_graph_with_ontology  [§4.3 — CHANGED: prune-by-provenance]
   for each graph entity NOT in ontology:
      provenance(id) ∈ {runtime, extracted}  → KEEP  (log "protected")
      else                                    → PRUNE (log "stale ontology entity")
sync_people_from_ontology   [unchanged — mints the curated 14]
sync_people_from_graph      [§4.4 — NEW: mints runtime/extracted graph persons, idempotent]
```

---

## 4. The components

### 4.1 Provenance model — *where the `source` field lives* (Decision A)

`source ∈ { ontology, runtime, extracted }`.

- **`ontology`** — from the curated `ontology.toml`. Owned by the curator; the reconciler may prune it.
- **`runtime`** — created by an explicit human/agent action (Henry tool, UI add). Never pruned.
- **`extracted`** — created by automated extraction (v1.5 Librarian). Never pruned.

**Recommended: store provenance in `permagent.db`, not on the Kuzu node.**

```sql
CREATE TABLE entity_provenance (
    entity_id_hex TEXT PRIMARY KEY,   -- bare 64-hex EntityId (same key as people.graph_entity_id)
    source        TEXT NOT NULL CHECK (source IN ('ontology','runtime','extracted')),
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
```

Rationale:
- The pinned `spectral` dep stays frozen (`7300ad8`). The Kuzu `Entity` node (`schema.rs:78`) has no
  provenance column; adding one is a spectral schema migration + `upsert_entity` param + rev bump +
  cross-repo coordination (that is Decision D, the alternative).
- The reconciler and both bridges are **already permagent-side**. Provenance is a permagent concern
  (which of *its* writes to protect); it belongs in permagent's store, next to `people`,
  `project_people`, and the `entity_fields` overlay it already keeps.
- Keyed on the same `entity_id_hex` the bridge and `entity_fields_for` already use — one join key
  everywhere.

**Migration for the existing 14: none required for correctness.** The reconciler rule (§4.3) treats
*absence of a provenance row* as `ontology` (prunable). The 14 are in the ontology, so they are never
pruned regardless of whether they have a row. An **optional backfill** — enumerate ontology persons
present in the graph, insert `source='ontology'` — buys auditability ("query any entity's origin")
but changes no behavior. Recommend running it anyway for observability (Decision B).

**Alternative (Decision D):** put `source STRING DEFAULT 'ontology'` on the Kuzu `Entity` node in
spectral (mirrors the existing `ALTER TABLE Entity ADD description` idempotent-migration at
`schema.rs:92`). Single source of truth on the node; but couples the design to a spectral rev bump and
a Kuzu column migration on every existing graph. Deferred unless the ruling wants provenance to be a
first-class graph property (e.g. for cross-device graph sync where permagent.db is not shipped).

### 4.2 The safe runtime graph-write API (Decision C — validation surface)

New on `SafeBrain` (`brain_handle.rs`), the *only* way to write a person node — narrow and validated,
never a raw `upsert_entity` passthrough:

```rust
/// Create (or no-op MERGE) a person node in the graph with durable provenance.
/// Returns the content-addressed EntityId hex — the people-bridge key.
pub async fn create_person_entity(
    &self,
    display_name: &str,
    visibility: Visibility,     // default Private (matches ontology persons)
    source: Provenance,         // runtime | extracted  (never ontology via this path)
) -> anyhow::Result<String>;
```

Validation (this is "no malformed person nodes"):
1. `canonical = graph_canonical(display_name)` (`canonical.rs`). **Reject** empty-after-normalization
   (punctuation-only) — the same guard `upsert_identity_from_graph` already applies.
2. `entity_type` is fixed `"person"`. No caller-supplied entity type ⇒ no arbitrary node injection.
3. `id = entity_id("person", canonical)` — content-addressed, so a repeat call is a MERGE no-op.
4. **Write order (the safety spec):** provenance row **first** (durable in `permagent.db`), *then*
   `upsert_entity`. If the provenance write fails, abort — never write the node. If the node write
   fails after the provenance row lands, the orphan row is harmless (points at a nonexistent id,
   ignored by every reader). **A node can never exist without protecting provenance.** This ordering
   is what makes the "runtime person is never pruned" invariant hold even under partial failure.

`source` is `runtime` for explicit create, `extracted` for automated. It is *never* `ontology` on this
path — only the curator's file produces `ontology`.

### 4.3 The reconciler change — prune-by-provenance (Decision E — THE critical decision)

`brain_sync.rs:sync_graph_with_ontology`, the prune loop only. Today:

```
prune(e)  iff  e.id ∉ ontology_ids
```

Changed:

```
prune(e)  iff  e.id ∉ ontology_ids  AND  provenance(e.id) ∉ { runtime, extracted }
```

- `provenance(e.id)` is one indexed lookup in `entity_provenance` (batch-load all rows once before the
  loop — the graph is small).
- **Deterministic, no heuristics:** the decision is a set-membership test on an explicit column. There
  is no name-matching, no similarity, no timestamp guessing.
- **Curated pruning still works exactly as before:** an ontology person the curator deletes from
  `ontology.toml` has provenance `ontology` (or none) ⇒ falls to the `else` ⇒ pruned. The whole point
  of the reconciler — "pruning `ontology.toml` takes effect" — is preserved.
- **Observable — every branch logs (constraint):**
  - prune → `target: "permagentd::brain_sync"`, `info!`: `"pruned stale ontology entity '{canonical}' ({type})"` (as today).
  - protect → `info!`: `"kept runtime/extracted entity '{canonical}' ({type}) — provenance={source}, absent from ontology"`.

  So a reader of the logs sees not just *what* was kept but *why*. A protected count sits alongside the
  existing `kept`/`removed` tally.
- **No silent data loss:** a `runtime`/`extracted` entity is *structurally excluded* from the prune
  predicate. The only way it is deleted is an explicit future "delete person" action (out of scope,
  §7).

### 4.4 The mint — a second, graph-enumerating bridge (Decision F — mint topology)

Two mint entry points, **both reusing the existing `upsert_identity_from_graph`** (identity-only row,
idempotent by `canonical_id`):

1. **Eager (create path, immediate visibility):** `create_person` calls
   `upsert_identity_from_graph(pool, canonical, id_hex)` right after `create_person_entity` returns.
   The `people` row exists before the call returns ⇒ associate works in the same turn.
2. **Sweep (startup, durability backstop + the v1.5 path):** a **new**
   `sync_people_from_graph(pool, store)` that enumerates Kuzu person entities whose provenance is
   `runtime`/`extracted` (`MATCH (e:Entity {entity_type:'person'})` filtered by the provenance set)
   and mints any missing `people` row. Idempotent; runs in `state.rs` right after the existing
   `sync_people_from_ontology`.

Why both, not a replacement (see §1.4): `sync_people_from_ontology` remains the mint for the curated
14 (they may have no graph node). `sync_people_from_graph` mints the persons that live *only* in the
graph (runtime + future extracted). Their union = the full directory. They cannot double-mint: same
`canonical_id` ⇒ `upsert_identity_from_graph` is a no-op on the second.

### 4.5 create → mint → associate, end to end

`create_person("Sabaa Quao"[, associate_with="Wealthie"])`:

1. `SafeBrain::create_person_entity("Sabaa Quao", Private, Provenance::Runtime)` → provenance row
   (`runtime`) + Kuzu person node. Returns `id_hex`. **Durable.**
2. `upsert_identity_from_graph(pool, "sabaa quao", id_hex)` → `people` row, returns `entity_uuid`.
3. (optional) resolve project "Wealthie" → id; `associate_person(pool, project_id, entity_uuid, None)`.
4. **Restart:** reconciler sees the runtime node, provenance=`runtime` → **kept** (logged as
   protected). `sync_people_from_graph` re-mints the row if the DB was reset. Curated 14 unaffected.

At no point does a `people` row carry a `graph_entity_id` that points at a nonexistent node (the
rejected `upsert_person` failure), and at no point can the runtime person be pruned.

---

## 5. The ONE path — Henry tools, UI, and v1.5 extraction all consume it

`SafeBrain::create_person_entity(name, visibility, source)` + eager `upsert_identity_from_graph` is the
single choke point. The three callers differ **only** in the `source` argument and who invokes them:

| Caller | Invocation | `source` | Notes |
|---|---|---|---|
| **Henry `create_person` tool** (#578) | agent tool → in-process call | `runtime` | ships in the follow-up wiring dispatch, *after* this design is ruled |
| **UI "Add person"** | `POST /api/people` (today 405s) → same call | `runtime` | the route that does not exist yet becomes a thin wrapper over the choke point |
| **v1.5 extraction** (#256) | Librarian "person mentioned" → same call | `extracted` | "people emerge from being mentioned" is *automated calling of this path*, nothing new |

Building this path now **is** building v1.5's foundation — the extraction epic becomes "call
`create_person_entity(..., Extracted)` from the Librarian," not a second creation mechanism. This is the
design intent: person-creation architecture is specified **once, here**, and never re-litigated.

---

## 6. Safety invariant & failure-mode ledger

**Invariant:** *A `runtime` or `extracted` graph entity is never deleted by the reconciler.*

| Failure | Outcome under this design |
|---|---|
| Provenance write fails | Node is never written (write-order §4.2). No unprotected node exists. |
| Node write fails after provenance lands | Orphan provenance row, ignored by all readers. No corruption. |
| Curator deletes a curated person from `ontology.toml` | Pruned as today (provenance `ontology`/absent). Intended. |
| Curator later *adds* a runtime person to `ontology.toml` | Same `entity_id` (content-addressed) ⇒ now in `ontology_ids` ⇒ never pruned regardless of provenance. No duplicate node. Provenance row may stay `runtime` (harmless) or be promoted (Decision G). |
| `permagent.db` reset (provenance lost), graph intact | Runtime persons fall to `else` on next ontology change → **would be pruned.** Mitigation: the reconciler is gated on ontology-hash change, and `sync_people_from_graph` re-mints rows; but provenance loss is the one place the invariant is at risk. See Decision D (in-graph provenance removes this risk) and §7. |

The last row is the honest edge: permagent-side provenance ties durability to `permagent.db`. That is
acceptable if `permagent.db` is treated as durable co-equal with the graph (it already holds the whole
`people`/association layer). If the ruling wants provenance to survive independently of `permagent.db`,
that is the argument for Decision D (provenance on the Kuzu node).

---

## 7. What this design explicitly does NOT do

- **Delete-person.** No runtime deletion path. Removing a runtime person is a separate, deliberate
  action (it must also clear provenance + the `people` row + associations). Out of scope.
- **Edit curated (ontology) persons at runtime.** The ontology stays hand-curated and read-only at
  runtime. Renaming/re-aliasing a curated person is a curator edit to `ontology.toml`. (Decision G
  covers the narrow "promotion" question.)
- **Write `ontology.toml`.** No ontology serializer is introduced. The curated file remains the
  curator's; runtime persons live in the graph + provenance table, not the file.
- **Attributes/enrichment.** Role/company/email remain graph-authoritative via `entity_fields`
  (Decision A, #255) and land through the slice-2b write path — unchanged and orthogonal. A
  runtime-created person is identity-only until enriched, exactly like a bridged ontology person.
- **Change the pinned `spectral` rev** (unless Decision D rules for in-graph provenance).

---

## 8. Decision points for ruling

| # | Decision | Options | Recommendation |
|---|---|---|---|
| **A** | Where does provenance live? | (1) `permagent.db` `entity_provenance` side table · (2) `source` column on the Kuzu `Entity` node (spectral change) | **(1)** — freezes the pinned dep, keeps provenance beside the reconciler/bridge that already own it. (2) is Decision D. |
| **B** | Provenance model shape | (1) explicit 3-valued `source` on all known entities, backfill the 14 as `ontology` · (2) protect-list only (record `runtime`/`extracted`; absence = `ontology`) | **(1)** for auditability (matches your "explicit provenance over implicit rules"); the backfill is a no-op behaviorally but makes origin queryable. Absence still defaults to `ontology` as a safety net. |
| **C** | Runtime write-API surface | narrow `create_person_entity` (person-only, validated) vs a general `create_entity(type, …)` | **Narrow person-only** for v1 — no arbitrary node injection. Generalize when a second entity type needs runtime creation. |
| **D** | In-graph provenance now, or defer? | defer (permagent.db) vs bump spectral to add `Entity.source` | **Defer.** Revisit only if provenance must survive independently of `permagent.db` (e.g. cross-device graph-only sync). Note the §6 last-row risk this would remove. |
| **E** | Reconciler prune predicate | `prune iff ∉ontology AND provenance∉{runtime,extracted}` | **Adopt as written** — deterministic, logged both branches, curated pruning preserved. This is the core change. |
| **F** | Mint topology | keep ontology bridge + add graph bridge (union) vs replace with graph-only | **Union.** Graph-only would drop curated persons with no asserted triple (§1.4). |
| **G** | Ontology↔runtime "promotion" | when a runtime person is later added to `ontology.toml`: (1) no-op (in-ontology never prunes anyway) · (2) rewrite provenance to `ontology` | **(1)** for v1 — behavior is already correct; (2) only if you want the provenance table to reflect current authority for audits. |

---

## 9. Build sequence once ruled (for the follow-up dispatch — not this round)

1. `entity_provenance` table + schema-version bump (permagent.db) + optional 14-person backfill.
2. `SafeBrain::create_person_entity` (validated write, provenance-first ordering) + `Provenance` enum.
3. Reconciler: batch-load provenance, change the prune predicate, add the protect-branch log.
4. `sync_people_from_graph` + wire into `state.rs` after `sync_people_from_ontology`.
5. `create_person` orchestration (create_person_entity → eager mint) as a reusable in-process fn.
6. Consumers (separate dispatches): Henry `create_person`/`associate_person_with_project` tools +
   self-knowledge descriptor (#578); `POST /api/people` route + UI; v1.5 Librarian extraction (#256).

Each of 1–5 is independently gated (`cargo check/clippy -p permagent` for the daemon crate — verified
package name is **`permagent`**, dir `crates/goose/`). No frontend in 1–5.
