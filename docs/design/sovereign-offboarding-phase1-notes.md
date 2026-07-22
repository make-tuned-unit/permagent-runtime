# Sovereign Offboarding — Phase 1 implementation notes (shipped vs gated)

> Companion to `docs/design/sovereign-offboarding.md` (design, #850). This file
> records what **Phase 1** actually shipped in Permagent, and — precisely — what
> is **Spectral-gated** or **decision-gated** and therefore *not* shipped.
>
> Audited against Spectral pin **`fb1038db`** (`Cargo.toml:88`). Branch:
> `feat/scope-forget-and-ladder`. Part of #850; closes the memory half of the
> #339 re-ingest hard-delete and documents the graph half it cannot.

---

## What shipped (real, tested primitives)

### Claim 1 — "hard-delete by scope" — SHIPPED (memories)

- `SafeBrain::forget_scope(wing)` — an **enumerate → forget sweep**: reads the
  local brain `memory.db`, selects every `key` where the `wing` column equals
  the requested wing, hard-deletes each via the verified per-key
  `Brain::forget`, and returns an aggregate `ScopeForgetReport`
  (`crates/goose/src/brain_handle.rs`).
- `SafeBrain::forget_keys(keys)` — the reusable core (forget an explicit set,
  aggregate the reports). Also the substrate a future federation-wing sweep
  would call once enumeration is reachable.
- `ScopeForgetReport` — the audit roll-up (keys swept, existed, verified
  `fully_forgotten`, `graph_triples_deleted`, forgotten keys).
- Memories in **other** wings and **wingless** memories (chat turns write
  `wing = NULL`) are provably untouched — asserted in tests.

### Claim 5 — "the scope ladder is real and settable" — SHIPPED

- `MemoryScope` — Permagent's 1:1 typed name for Spectral's shipped
  `Visibility` ladder (`Private < Team < Org < Public`), with lossless
  `to_visibility` / `from_visibility`, `as_str`, serde, and `From` conversions
  (`crates/goose/src/brain_handle.rs`).
- `SafeBrain::remember_scoped(key, content, scope, opts)` — the settable write
  path; stamps `opts.visibility` from the chosen level.
- **Default is unchanged**: every existing write path still writes
  `Visibility::Private`. Non-`Private` levels are now *expressible*, never
  imposed. Tests assert the persisted `visibility` string per level **and** the
  real recall filter (a `Private` memory is hidden from a `Team`-clearance
  recall floor; a `Public` memory surfaces at every clearance).

---

## STOP-and-flag — Spectral-gated (cannot ship Permagent-side at `fb1038db`)

### G1 — Graph triples survive `forget` (design-doc **Q2**; the #339 graph half)

`graph_triples_deleted` is **always 0**. Deleting company-derived graph facts
is blocked on **two** missing Spectral surfaces:

1. **No triple/entity delete API.** `spectral_graph::graph_store::GraphStore`
   exposes only `upsert_entity`, `insert_triple`, `get_entity`, `find_triples`,
   `neighborhood`, `upsert_document`, `insert_mention`, `count_mentions` — **no
   `delete`/`remove` of any kind** (audited `graph_store.rs`). The outer
   `spectral::Brain` re-exposes only `store() -> &GraphStore` (read/query). The
   `conn: Mutex<Connection>` is private; there is no raw-SQL escape hatch.
2. **Triples carry no scope key.** The `triple` table is
   `(from_id, to_id, predicate, confidence, source_doc_id, source_brain_id,
   asserted_at, visibility, weight)`. There is **no `wing`/scope column**, so
   even with a delete API there is no way to select "the triples belonging to
   wing X". (`source_doc_id` *does* exist, so a **document-scoped** triple purge
   — the #339 case — would be expressible *if* a delete API existed; the
   offboarding **wing-scoped** case needs both a delete API and a scope tag.)

**Asked of Spectral (minimum viable):** a scoped triple delete on `Brain`, e.g.
`Brain::forget_triples_by_doc(source_doc_id)` (unblocks #339's graph half) and/or
`Brain::forget_triples_in_wing(wing)` backed by a `wing`/scope column on the
`triple` row (unblocks offboarding). Recommendation matches design-doc Q2 (b)/(a).

**Permagent-side stance:** we do **not** reach around the library with raw
DELETEs into `graph.sqlite` — there is no scope key to filter on for the wing
case, and it would violate Spectral's ownership of that schema. We delete
everything deletable (all memory substrates via `forget`) and surface the
residual honestly in `ScopeForgetReport` and the product copy.

### G2 — Federation-wing enumeration is not reachable from Permagent

The design doc's authoritative offboarding boundary is **Axis A** — membership
in a federation *shared wing* (`federation_sync.rs`: `enumerate`, `share`,
`export_pack`, `tombstone`, the `shared_wing_members` table). None of it is
reachable from Permagent at this pin: the outer `spectral::Brain` wrapper
re-exports **none** of those functions, and they require a `&SqliteStore` whose
accessor (`Brain::sqlite_store`) is `pub(crate)`. So `forget_scope` sweeps the
**cognitive wing** (the `wing` column, `RememberOpts.wing`) — the scope
Permagent *can* enumerate — not the federation wing.

**Asked of Spectral:** expose the federation-sync surface on the `Brain`
wrapper (`Brain::enumerate_wing(wing_id)`, `Brain::share(key, wing_id)`), or a
`Brain::local_key_for(object_hash)` resolver, so the offboarding sweep can key
off shared-wing membership.

---

## Decision-gated (Jesse's call — gates any change to defaults)

- **Q1 — the offboarding boundary.** Is "company-scoped" defined by federation
  shared-wing membership (Axis A) *only*, or does `Visibility::Team`/`Org`
  (Axis B) also imply company ownership? This PR keeps them **orthogonal** and
  does **not** treat visibility as an export/ownership boundary. Binding
  `forget_scope` to the federation boundary waits on this ruling (and G2).
- **Q3 — the "wing" naming collision.** Federation `wing_id` (a shared realm)
  and cognitive `RememberOpts.wing` (a topical route) are different axes sharing
  one word. `MemoryScope` and `forget_scope` are documented to keep them
  distinct, but the rename (`realm_id` for the federation id) is Jesse's call
  and gates the offboarding-flow UI.
- **Default visibility** stays `Private` everywhere. Flipping any default is
  explicitly out of scope for this PR and gated on Q1/Q3.
