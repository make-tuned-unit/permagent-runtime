# CRM as a Brain-backed People view — architecture decision + phased path

**Issue:** #256 (architecture-decision slice) · **Epic:** #495 (People-graph / CRM as a Brain lens)
**Status:** DESIGN — doc-only, no code in this PR. The architecture was **ruled Option A on
2026-06-24** (epic #495) and is **largely realized on `main`**; this doc consolidates that
ruling against the *actual code* (file:line), audits realized-vs-remaining honestly, sequences
the work that is left, and surfaces the decisions still open for Jesse.
**Author:** design agent (audit-first). Every claim below is tagged **[verified]** (read in the
code at the cited line) or **[assumed]** (inferred / external, not confirmable in-repo).

---

## 0. TL;DR

- **The architecture decision is not open — it is made and shipped.** The CRM People view is a
  **Brain LENS** over the graph entity, not a separate CRM store. The graph entity
  (Spectral/Kuzu) is authoritative for attributes; the `people` table is an identity-only
  directory bridged to it. This is the correct call and this doc **re-affirms it**. **[verified]**
- **What's realized:** single person-create path, graph-authoritative attribute overlay, Henry
  People tools, person→project `works_on` edges surfaced in the Brain graph, the Enricher
  (research briefing → review-gated Decision-Inbox proposal → Enriched-provenance write), and
  three-layer manual-not-clobbered protection. **[verified]**
- **The one load-bearing gap:** there is **no manual field-edit write path**. `set_entity_field`
  with `FieldSource::Manual` has **zero callers** and `/api/people` is GET-only — so a user cannot
  actually type an email/phone/birthday into a profile today. The "enrichment never clobbers
  manual" guarantee is real in Spectral's store but currently **vacuous**, because nothing writes
  Manual. This is the top remaining foundational slice. **[verified]**
- **Secondary gaps:** the Brain-*graph* lens still mixes all entity types (no server-side
  `type=person` filter on `/api/brain/graph`); no per-person activity timeline; auto-generated
  person write-ups are bounded to the persona's 2-hop neighborhood. **[verified]**

---

## 1. Current state — how people/entities/fields/edges are modeled today

### 1.1 Two stores, one authoritative — the resolved split

There are two physical stores, deliberately, with a ruled authority boundary:

- **`people` table** (`permagent.db`, SQLite) — the **identity directory**. It is *identity-only*:
  an opaque immutable `entity_uuid` (v7 UUID, minted once), a mutable name-slug `canonical_id`
  (`person:jane-doe`, UNIQUE, rewritten on rename), `display_name`, and the bridge key
  `graph_entity_id` (bare 64-hex blake3 `EntityId`, set once at creation, never rewritten).
  **[verified]** `crates/goose/src/people.rs:32-54`.
  - The opaque-uuid choice is the `persona_id` pattern: keying durable rows on the driftable slug
    would orphan a person's history on every rename. `people.rs:1-16`, test
    `entity_uuid_stable_across_rename` `people.rs:488-525`. **[verified]**
- **Graph entity** (Spectral / Kuzu `graph.kz`) — the **authoritative** node. Person *attributes*
  live here as typed `entity_fields` with provenance. **[verified]** ruling recorded in epic #495
  body and `people.rs:78-104`.

The `people` table still *has* attribute columns (`role/company/email/phone/notes/
last_contact_at`, `people.rs:82-89`), **but they are no longer the response source.** The read
path clears them and overlays the graph (§1.3). The module doc-comment calls them "a safety net
until the Step-3 drop." **[verified]** `people.rs:107-118`, `routes/people.rs:34-40`.

### 1.2 The one person-create path

Every runtime caller (Henry's `create_person` tool, the UI add route, the v1.5 Librarian
extraction) funnels through **one** function, differing only in `Provenance`:

`crate::people_create::create_person` — ordered **provenance → graph node → directory row**:
1. record provenance in `permagent.db` first (so the reconciler can never prune a node that
   exists without protection), 2. materialize the Kuzu node (`create_person_entity`), 3. mint the
   identity-only directory row via `people_bridge::upsert_identity_from_graph`.
**[verified]** `crates/goose/src/people_create.rs:28-67`. Idempotent by canonical name (both the
content-addressed `EntityId` and the `canonical_id` UNIQUE key dedupe).

### 1.3 Attributes: typed fields with provenance, read graph-authoritative

- **Field model.** Spectral's `entity_fields` store carries typed fields as
  `{field_name, value, source, source_url?, updated_at}` where `source ∈ {Manual, Enriched, …}`
  (`spectral::ingest::FieldSource` / `EntityField`). The Brain handle exposes
  `set_entity_field(id, name, value, source, source_url)`, `get_entity_fields(id)`, and a batched
  `entity_fields_for(ids) -> map<hex, Vec<EntityField>>`. **[verified]**
  `crates/goose/src/brain_handle.rs:590-649`.
- **Manual-not-clobbered rule** is enforced *inside Spectral's store*: an `Enriched` write returns
  `false` (suppressed) against a field whose stored source is `Manual`. **[verified]**
  `brain_handle.rs:445-449`.
- **Read overlay (Decision A).** `GET /api/people` lists identity rows, then
  `overlay_graph_attributes` **clears** every column value and **replaces** it from the graph in
  one batched `entity_fields_for` hop keyed by `graph_entity_id`. If the Brain is absent or the
  read fails, attributes come back blank (never a stale column). Read-through latency is logged for
  the "measure before cache" ruling (Decision E). **[verified]**
  `crates/goose-server/src/routes/people.rs:34-131`.
- **Two field vocabularies, kept as plain strings (no Spectral dep) so both sides share one list:**
  - `PERSON_FIELD_NAMES` (6): `role, company, email, phone, notes, last_contact_at` — the overlay
    set. **[verified]** `people.rs:82-89`.
  - `ENRICHABLE_FIELD_NAMES` (5): `linkedin, job_title, company, x_handle, personal_site` — the
    only fields the Enricher may propose. Structured/verifiable only; manual-only fields
    (email/phone/birthday/notes) are deliberately absent. **[verified]** `people.rs:91-104`.

### 1.4 Edges: person→project `works_on`, surfaced in the Brain graph

- **Assertion.** Associating a person with a project mirrors the link as a `works_on` graph edge,
  best-effort, via `SafeBrain::assert_person_project_edge`. **[verified]**
  `crates/goose-server/src/routes/projects.rs:398-420`.
- **Surfacing.** `/api/brain/graph` picks up to 80 neighborhood entities, then
  `collect_entity_edges` keeps a triple only when **both** endpoints were picked for display and
  dedups on `(from, to, predicate)` (re-asserts append duplicate triples; display needs each
  connection once). The `GraphResponse` carries `entities` + `edges` + `memories`; each
  `GraphEntity` emits `note` (freeform description) **and** its typed `fields` with full provenance
  (`source`, `source_url`, `updated_at`) — so the person modal card can render provenance badges.
  **[verified]** `crates/goose-server/src/routes/brain.rs:270-432`.

### 1.5 Henry's People tools (the read/write surface for the agent)

The `people` platform extension exposes four tools: `create_person`,
`associate_person_with_project`, `enrich_person`, `propose_enrichment`. Name resolution is
explicit — an ambiguous person/project name returns the candidate list rather than guessing.
**[verified]** `crates/goose/src/agents/platform_extensions/people.rs:88-611`.

### 1.6 What the Librarian / Enricher already do

- **Librarian memory-describe pass** reads a memory's own content → FACTS/TERMS/CATEGORIES via
  local Ollama → `set_description` + entity annotations. **[verified]** (per #799 audit;
  `librarian.rs describe_one`).
- **Cross-source enrichment (#626 / #799, MERGED, flag-gated OFF).** `librarian_context.rs`
  assembles a budgeted (~1k-token) cross-source bundle — chats (FTS), projects/goals (card links),
  Jesse-answered decisions, and the ±12h activity-journal window — into the describe prompt, with a
  `SOURCES:` provenance line. Gated behind `LIBRARIAN_CROSS_SOURCE_ENABLED` (default OFF;
  eval-gated on the mac mini). With the flag off, the describe pass is byte-for-byte identical.
  **[verified]** issue #799 body + `crates/goose/src/agents/platform_extensions/librarian_context.rs`
  (module exists). **Note:** this enriches *memory descriptions*, not person `entity_fields`.
- **Entity-description pass (#387-v2)** generates the freeform write-up ("who is this person")
  and writes it via `set_entity_description`, but **only for entities inside the persona's 2-hop
  recall neighborhood** — Spectral on the pinned rev exposes no all-entities enumeration.
  **[verified]** `librarian_entities.rs:925`, bound explained at `brain_handle.rs:466-472`.
- **The Enricher (#495 slice 4, structured fields).** `enrich_person` returns a research *briefing*
  (current graph fields with provenance + the enrichable allowlist + "never guess an identity") and
  **does not browse**; the agent researches with its own web tools, then files findings via
  `propose_enrichment`, which creates a `kind='enrichment_proposal'` Decision-Inbox item. On
  **approve**, the decisions route writes each field with `FieldSource::Enriched` (+ `source_url`),
  reporting applied/protected/skipped counts; **nothing** touches a profile until the user
  approves. **[verified]** `platform_extensions/people.rs:338-516`,
  `crates/goose-server/src/routes/decisions.rs:480-528`.

---

## 2. The architecture decision — Brain LENS, not a separate CRM store

### 2.1 The decision (RE-AFFIRMED — already ruled Option A)

> **The CRM People view is a projection (a lens) over the authoritative Brain graph entity.
> Attributes are typed `entity_fields` with provenance on the graph node. The `people` table is an
> identity-only directory that bridges to the node by an immutable key. There is no second,
> parallel CRM record of truth.**

This was ruled in epic #495 (2026-06-24, "Source of truth: Graph entity … is AUTHORITATIVE") and
is realized in the overlay read path (`routes/people.rs:34-93`) and the single create path
(`people_create.rs`). This doc's role is to certify that ruling against the code and to sequence
the remainder — **not** to reopen it.

### 2.2 The boundary (who owns what)

| Concern | Owner | Evidence |
|---|---|---|
| Stable identity (uuid, rename safety) | `people` table (directory) | `people.rs:32-54,257-282` |
| Person *attributes* (role/company/email/…) | **Graph entity** `entity_fields` | `routes/people.rs:34-93` |
| Freeform "who is this person" write-up | **Graph entity** description (`note`) | `brain_handle.rs:453-464`, `librarian_entities.rs:925` |
| Relationships (person→project/person) | **Graph** triples (`works_on`, …) | `routes/projects.rs:398-420`, `brain.rs:290-315` |
| Provenance (manual vs enriched, source_url) | **Graph** `entity_fields.source` | `brain_handle.rs:445-449`, `people.rs:91-104` |
| Directory lookup / disambiguation | `people` table (`list_people`, resolvers) | `people.rs:315-356`, `platform_extensions/people.rs:139-201` |

The `people` table's attribute columns are a **soft-deprecated safety net**, not a second source
of truth — cleared on every read before the graph overlay. **[verified]** `people.rs:107-118`.

### 2.3 Why Brain-lens is right (and ties to the sovereignty / local-first thesis)

1. **Single source of truth → no duplication-class bugs.** The recurring "empty People/Topics
   modal" bug was rooted in a cross-store split (graph node vs a parallel CRM row). Collapsing to
   one authoritative store (graph) with one overlay read removes the whole class. **[verified]**
   epic #495 audit findings; `routes/people.rs:36-40` clears columns precisely so a stale value
   "can never leak through."
2. **Enrichment composes for free.** Because a person *is* a graph entity, everything the graph
   already knows about them — edges to projects, memories annotated to them, the cross-source
   describe bundle — is reachable without a join across a foreign CRM schema. The CRM is a *view*
   of accumulated knowledge, not a data-entry silo.
3. **Local-first / sovereignty.** The whole model lives in the user's local `permagent.db` +
   Spectral store — no external CRM SaaS, no contact data leaving the device. The Enricher is the
   *only* egress, and it is **review-gated**: web findings sit in the Decision Inbox and reach the
   authoritative store only on explicit approval (`routes/decisions.rs:480-528`). This is the
   sovereignty-router posture — data-in stays local; data-out (a web lookup) is bounded, cited, and
   consented. **[verified].** It also lines up with the MemoryScope ladder (§5, #857): a person node
   is `Private` by default and the visibility axis is now expressible.

### 2.4 Why NOT a separate CRM store (the rejected option)

A dedicated `crm_people` store with its own attribute rows would (a) reintroduce the cross-store
identity split the ruling closed, (b) force a sync path (and its drift/conflict bugs) between CRM
rows and graph nodes, (c) duplicate enrichment plumbing, and (d) fragment provenance. Rejected.
**[verified]** the split it would recreate is exactly what epic #495's ruling resolved.

---

## 3. The enrichment layer

Two complementary enrichment paths already exist; the design keeps them **separate by concern**:

### 3.1 Structured-field enrichment — the Enricher (shipped, review-gated)

- **Bounded to `ENRICHABLE_FIELD_NAMES`** (5 structured, verifiable fields). Manual-only fields are
  off-limits at three independent layers: rejected in `propose_enrichment`
  (`platform_extensions/people.rs:434-453`), rejected/skipped at decision creation and apply
  (`routes/decisions.rs:497-500`), and — even if both were bypassed — suppressed by Spectral's
  Manual-not-clobbered store rule (`brain_handle.rs:445-449`). **[verified]**
- **Flow:** `enrich_person` (briefing, no browse) → agent web-research → `propose_enrichment`
  (Decision-Inbox proposal, each field with a mandatory `source_url`) → user approve → write with
  `FieldSource::Enriched`. **[verified]** §1.6.
- **Identity safety:** if the agent cannot confidently tell *which* person a common name is, it is
  instructed to STOP and report ambiguity rather than propose. **[verified]**
  `platform_extensions/people.rs:390-393`.

### 3.2 Freeform description enrichment — the Librarian (cross-source, flag-gated)

- **Cross-source describe (#626/#799)** already assembles chats + projects/goals + decisions +
  activity-journal into the describe prompt, with cited `SOURCES:`. **[verified]** §1.6. This is the
  substrate for "populate a person's write-up from everything the system knows about them" — it just
  needs to be pointed at **person entities** and turned on after the mini eval clears it.
- **Person write-ups (#387-v2)** already run for neighborhood people. **[verified]** §1.6.

### 3.3 The enrichment gap to close

The cross-source bundle enriches **memory descriptions**, and the #387-v2 pass writes **entity
descriptions** for neighborhood entities — but there is **no pass that assembles the cross-source
bundle specifically to synthesize a person's profile** (structured facts *and* the "how the user
knows them / what they relate to" write-up) from the memories, edges, emails, and activity that
reference that person. The plumbing exists; the person-scoped orchestration does not. This is the
enrichment slice that remains (§4, Slice E). **[verified]** by absence — no person-scoped describe
entry point in `librarian_context.rs` / `librarian_entities.rs`.

> **Divergence to note (decision D3, §5).** Issue #256's later comment envisioned a **Librarian
> task-queue primitive** (Henry enqueues "update her profile", Librarian drains it nightly). The
> shipped design instead chose the **review-gated synchronous Enricher** (agent researches now,
> proposal waits in the Decision Inbox). Both keep a single Brain writer and off-load heavy work;
> they are not the same primitive. The task-queue is *not built* and is a live scope decision, not a
> latent bug.

---

## 4. Phased path

Legend: **[DONE]** shipped on `main` · **[NOW]** buildable immediately, no external gate ·
**[GATE]** blocked on something external (Spectral / mini eval).

| Slice | What | Status | Evidence / gate |
|---|---|---|---|
| **1. Field schema + write path** | Typed `entity_fields` w/ provenance; batched read overlay | **[DONE]** | `brain_handle.rs:590-649`, `routes/people.rs:34-93` |
| **1b. Person create (one path)** | provenance → node → directory row | **[DONE]** | `people_create.rs:28-67` |
| **2a. People tab + modal (read)** | list + graph-fed modal card w/ fields + provenance | **[DONE]** | `routes/people.rs`, `brain.rs:404-430` |
| **2b. Manual field EDIT (write)** | user types email/phone/birthday/notes → `set_entity_field(..Manual)` | **[NOW]** — **top gap** | `FieldSource::Manual` has **0 callers**; `/api/people` GET-only |
| **3. Edges: assert + surface** | person→project `works_on` asserted + rendered | **[DONE]** | `routes/projects.rs:398-420`, `brain.rs:290-315` |
| **3b. person→person edges** | assert + render relationship lines beyond `works_on` | **[NOW]** | only `works_on` asserted today |
| **3c. People-only graph lens filter** | server-side `type=person` on `/api/brain/graph` | **[NOW]** | graph picks **all** entity types (`brain.rs:374-388`) — #256 ask (1) |
| **4. The Enricher (structured)** | briefing → proposal → approve → Enriched write | **[DONE]** | `platform_extensions/people.rs`, `routes/decisions.rs:480-528` |
| **E. Person-profile cross-source enrichment** | point the #626 bundle at a person → synthesize structured facts + write-up | **[GATE]** (mini eval) + build | `librarian_context.rs` exists; no person-scoped entry; `LIBRARIAN_CROSS_SOURCE_ENABLED` OFF |
| **F. Person activity timeline** | per-person view of memories/emails/activity referencing them | **[NOW]** (read model) | `activity` route exists (#619) but not person-filtered |
| **G. Full write-up for all people** | describe people **outside** the 2-hop neighborhood | **[GATE]** (Spectral) | no all-entities enumeration on pinned rev (`brain_handle.rs:466-472`) |
| **H. Librarian task-queue primitive** | Henry-enqueued deferred profile jobs (the #256-comment vision) | **[DECISION]** | not built; competes with the shipped review-gated Enricher (§3.3) |

### Recommended near-term order (buildable now, no external gate)

1. **Slice 2b — manual field-edit write path.** *Highest value, smallest surface.* Add a guarded
   write endpoint (`PATCH /api/people/:entity_uuid` or a Henry tool) that resolves
   `graph_entity_id` and calls `set_entity_field(.., FieldSource::Manual, ..)`. This makes the
   manual-not-clobbered guarantee **non-vacuous** and lets users actually own email/phone/birthday.
   Ships its Henry descriptor in the same PR if a tool is added (self-knowledge discipline).
2. **Slice 3c — `type=person` filter on the graph lens** (the last un-done piece of #256's ask 1),
   and **Slice F — per-person activity timeline** (read model over the existing journal). Both are
   read-side, no external gate.
3. **Slice 3b — person→person edges** (extend the proven `assert_*`/`collect_entity_edges` path).
4. **Slice E — person-profile cross-source enrichment** — build the person-scoped assembly on the
   existing `librarian_context.rs` substrate; keep it behind `LIBRARIAN_CROSS_SOURCE_ENABLED` and
   **eval it on the mini** before it writes production profiles.

### External gates (do not start without)

- **Slice G** (write-ups for people outside the neighborhood) needs a Spectral all-entities
  enumeration; blocked at the pinned rev. **[verified]** `brain_handle.rs:466-472`.
- The task brief references **Spectral entity-registration (spectral#215)** as a potential gate.
  **[assumed]** — no reference to it exists in this repo; treat any dependency on it as unverified
  until confirmed against the Spectral tracker.

---

## 5. Open decisions for Jesse

1. **D1 — Certify Option A and close #256.** The architecture is ruled and realized. Recommendation
   (matching the tracker-drift audit already on #256): **close #256 as superseded by #495**, which
   carries the realized design and the remaining slices. Decision: close, or keep open under the
   `design-needed` label? **[verified]** the audit comment already recommends closing.

2. **D2 — Manual-edit surface (Slice 2b): route, tool, or both?** The gap is real (no Manual
   writer exists). Options: (a) a UI `PATCH` endpoint only; (b) a Henry `set_person_field` tool
   only; (c) both. Recommendation: **(c)** — the UI is the primary path for authoritative user
   data; the tool lets Henry capture "her birthday is the 3rd" conversationally. Scope: which fields
   are manual-editable (the manual-only set: `email, phone, birthday, notes, relationship_strength,
   how_met` — note `birthday/relationship_strength/how_met` are named in #495 but **not** in the
   shipped `PERSON_FIELD_NAMES`, so the field vocabulary needs widening as part of 2b). **[verified]**
   `people.rs:82-104`.

3. **D3 — Enricher pattern: keep review-gated-only, or also build the Librarian task-queue (#256
   comment)?** The shipped Enricher is synchronous-research + Decision-Inbox approval; the #256
   comment envisioned a deferred nightly Librarian queue. They solve overlapping problems. Building
   both adds a second write-initiation path. Recommendation: **keep the review-gated Enricher as the
   one path; defer the task-queue** unless a concrete "Henry noticed a fact mid-chat, log it later"
   flow demands it. Decision: build H, or drop it? **[verified]** divergence, §3.3.

4. **D4 — Privacy / scope for people data (ties to MemoryScope #857).** People nodes are `Private`
   by default and the `Private < Team < Org < Public` visibility ladder is now *expressible* (not
   flipped). Decisions: (a) should a person/profile ever be settable above `Private` (e.g. a
   shared-team contact), and (b) does scope-based **forget** (`forget_scope`) need to cascade to a
   person's `entity_fields` and edges on offboarding? Note the known Spectral residual: graph
   triples currently **survive** a scope forget (no scoped triple-delete API), so a person's *edges*
   would outlive a wing sweep. **[verified]** #857 body (Q2 residual, `graph_triples_deleted`
   always 0).

5. **D5 — When is production person-profile enrichment (Slice E) allowed to write?** It must clear
   the mini recall-quality eval first (the `LIBRARIAN_CROSS_SOURCE_ENABLED` discipline). Decision:
   confirm the bench/threshold before Slice E writes anything to authoritative profiles.
   **[verified]** #799 gating rationale.

---

## Appendix — key files (all paths absolute-in-repo)

- `crates/goose/src/people.rs` — Person model, field vocabularies, directory read/write primitives.
- `crates/goose/src/people_create.rs` — the one runtime create path.
- `crates/goose/src/brain_handle.rs` — `set_entity_field` / `get_entity_fields` / `entity_fields_for`,
  `set_entity_description`, `assert_person_project_edge`, neighborhood enumeration bound.
- `crates/goose/src/agents/platform_extensions/people.rs` — Henry People tools + the Enricher.
- `crates/goose/src/agents/platform_extensions/librarian_context.rs` — cross-source enrichment (#626/#799).
- `crates/goose/src/agents/platform_extensions/librarian_entities.rs` — #387-v2 entity-description pass.
- `crates/goose-server/src/routes/people.rs` — `GET /api/people` + graph-authoritative overlay.
- `crates/goose-server/src/routes/brain.rs` — `/api/brain/graph`: entity + edge emission.
- `crates/goose-server/src/routes/projects.rs` — `works_on` edge assertion on association.
- `crates/goose-server/src/routes/decisions.rs` — `enrichment_proposal` approve/reject apply.
</content>
