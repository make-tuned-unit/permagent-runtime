# Federation coordination — Permagent ↔ Spectral seam (post red-team, Rev 2)

> **Dispatch to Spectral.** To be filed as a Spectral issue; this is the local
> reference copy (mirrors the `spectral-consolidate-api.md` pattern).
> Companion to Permagent's `docs/design/federation-security-spec.md` (PR #764,
> now Rev 2 with red-team findings RT-1..RT-11 folded in).
> Pin at time of writing: `spectral bd68467b`.

## Why this dispatch exists

Federation splits cleanly: **Spectral owns the plaintext memory-object layer**
(realm axis, `(key, author)` OR-Set, export/import packs, tombstones,
view-scoping recall); **Permagent owns the crypto that wraps the wire**
(identity, E2E pack encryption, key management, transport, tombstone-authz).
The seam is **crypto-agnostic** by construction — Spectral builds and tests the
plaintext layer against *synthetic* packs and is **not blocked** by our crypto.

The red-team of our half (Rev 2) added **one thing that crosses the seam**: a
realm now needs a **root of trust** (RT-1), realized as three new
**content-addressed control objects** — `genesis`, `admin-chain-link`,
`realm-keyring`. These are *not memories*, but they must **replicate over the
same `Shared(realm)` channel** as memory objects. This dispatch pins down (a)
where those objects live, (b) the `author_id` format Spectral must ratify, and
(c) two metadata/merge questions the red-team surfaced. Nothing here asks
Spectral to know about keys.

---

## 1. The control objects (RT-1) — where do they live? **← the one decision we need**

§3.5 of the security spec adds three signed, content-addressed objects that must
reach **every member** of a realm:

```
genesis          := { founding_admin:{author_id, ed25519_pub}, realm_nonce, created_meta }
                    realm_id := "realm:" || base32(SHA-256(founding_admin.ed25519_pub || realm_nonce))
                    genesis_sig := Ed25519_sign(founder_id_sk, genesis)

admin_chain_link := { prev:<hash|genesis>, op:Add|Remove, subject:author_id,
                      subject_keys?:{ed25519_pub, x25519_pub+cert},
                      by:<admin author_id>, seq, at_meta }
                    link_sig := Ed25519_sign(by_id_sk, link)

realm_keyring    := { realm_id, epoch:(counter, keyring_hash),
                      wraps:{ author_id -> HPKE_ciphertext }, admin_sig }
```

To Spectral these are **opaque signed blobs** — you never parse the crypto. But
they share two properties with memory objects: content-addressed, and replicated
grow-only through the realm channel. Two conflicts with the memory-object model
make the placement a real decision:

1. **The authorship invariant fights the admin-chain.** Your merge rejects an
   added object whose author ≠ pack signer (correct for memories). But
   `admin_chain_link` is authored by an admin **about a different subject** —
   inherently cross-author, exactly like a tombstone.
2. **View-scoping must NOT drop them.** These are realm-control metadata; they
   must reach all members regardless of any recall/wing scoping. They also must
   **never surface in recall** (they aren't memories).

**Options (your call — you own the substrate):**

- **(A) Separate Permagent control-plane namespace** over the same transport:
  Spectral's OR-Set/pack machinery stays pure-memory; we replicate control
  objects in a parallel grow-only set keyed by `realm_id`, riding the same
  have/want + relay. **Permagent's recommendation** — keeps your memory
  invariants clean, keeps control objects out of recall/ranking by construction.
- **(B) A Spectral control-object *kind*** with a tombstone-style
  authorship-exemption and an explicit "excluded from recall + never
  view-scoped-out" flag. Fewer moving parts if your pack format already
  generalizes, but couples control-plane liveness to memory-pack semantics.

We can build the seal/open pipeline against either. **We need to know which**
before the pack envelope (§4) is finalized, because it decides whether control
objects are inside your pack or beside it.

---

## 2. `author_id` format — Spectral to ratify (§3.2, §10.1)

You own `(key, author)`; the wire format of `author` must be jointly fixed.
Permagent proposes:

```
author_id := "ed25519:" || base32( SHA-256( ed25519_public_key ) )
```

- Stable, collision-resistant, self-verifying (binds to the identity key).
- **Replaces** the hardcoded `DEFAULT_USER_ID = "default"` for **`Shared` writes only**.
- **Legacy migration:** existing `"default"`-authored rows predate identity and
  were never shareable → on identity bootstrap they **stay `Local`** and keep
  `"default"`. Only new `Shared` writes carry a real `author_id`. No rewrite of
  historical rows.

**Ask:** confirm the string encoding (prefix + base32 + which SHA-256 truncation,
if any) and that `Local`/`"default"` rows are untouched by the migration.

---

## 3. Two red-team questions that touch your layer

### 3a. have/want manifest vs. metadata claim (RT-9)

Our §6.5 claims per-object counts are hidden "inside one sealed pack." The
red-team narrowed this: your **have/want hash-manifest** (the sync negotiation)
exchanges per-object hash lists, and if those transit the relay their **sizes**
re-leak approximate object counts + sync cadence — even E2E-encrypted.

**Ask:** how does the manifest move — peer-to-peer through the relay, or
computed only after a session key is up? Can it be **size-bucketed / padded**?
We'll set the honest metadata disclosure to match your actual wire behavior, so
we don't overclaim to a buyer.

### 3b. epoch is opaque to you (RT-2 fallout)

Pack envelopes carry `epoch:(counter, keyring_hash)`. That's **our** crypto
bookkeeping — we verify + strip it before handing you plaintext for
`import_pack`. Confirm you treat any Permagent envelope fields as opaque and key
convergence purely on your content-hash of the **plaintext** object (so "shared
content converges, ranking stays local" holds regardless of which epoch sealed a
given pack).

---

## 4. What we depend on from you (restating the contract, no change)

These are the guarantees our design assumes; the red-team leaned on them and
found them load-bearing. No new work — just confirming they hold at v1:

- **Guarantee A — structural export-gate.** `realm=Local` objects appear in **no**
  exported pack. Enforced at enumerate-filter + pack-serializer + a property test.
  This is the bedrock of our whole sovereignty claim.
- **Guarantee B — view-scoping recall, spreading ON.** A private memory never
  surfaces in a shared-scope recall output, *with associative spreading enabled*
  (the leak you fixed was spreading re-injecting after the filter). Honest-participant
  only — we document it as such, not as a disk-compromise defense.
- **Authorship invariant at merge.** `import_pack` rejects an added object whose
  embedded author ≠ verified pack signer (tombstones + control objects excepted,
  §1/§7). This is what makes a malicious teammate unable to forge *your* memories.
- **Federation must pass the accuracy eval before shipping** (private-only vs
  private+shared-merged recall A/B; per-origin cap on by default; a regression must
  be attributable to flooding, i.e. a knob-turn not a redesign).

---

## 5. Migration / sequencing

1. Spectral integrates realm/pack/OR-Set/tombstone surface (in progress) —
   `export_pack`/`import_pack`/`realm` appear **nowhere** in Permagent's tree at
   pin `bd68467b`; we wire on a pin bump.
2. Ratify `author_id` (§2) + control-object placement (§1).
3. Permagent builds identity + seal/open against the chosen placement
   (design-first: no crypto until the security spec's Open Decisions are ruled).
4. Pin bump → Permagent replaces any direct paths with the agreed trait surface.

## 6. Open questions (consolidated)

- **§1:** control objects as a Permagent namespace (A, recommended) or a Spectral
  control-object kind (B)?
- **§2:** exact `author_id` encoding + confirm legacy-row no-touch.
- **§3a:** manifest transport + can it be padded/bucketed?
- **§3b:** confirm Permagent envelope fields are opaque; convergence keys on
  plaintext content-hash.
- Anything in our guarantee-A/B/invariant restatement (§4) that has drifted in
  your v1 build?

## Context

Permagent's federation half is **design-first** — `docs/design/federation-security-spec.md`
(PR #764, Rev 2, red-teamed). OD-1..OD-7 ratified 2026-07-19. This dispatch keeps
Spectral's plaintext-layer build aligned with the Rev-2 additions **without**
blocking it on our crypto — the seam is deliberately crypto-agnostic. Related
Permagent context: the sovereignty-router (landed, #765) owns the *inference* half
of the `sovereign` flag; this layer owns the *export* half via `realm=Local`.

---

# Round 2 — Permagent reply (2026-07-19)

Spectral answered (this thread). **All three gates cleared.** Recording resolutions
+ answering their three questions back.

## Accepted from Spectral

- **§1 = (A)** — control objects (genesis / admin_chain_link / realm_keyring) live in
  a Permagent-owned parallel grow-only set keyed by `realm_id`, **beside** the pack,
  never in Spectral's memory tables. `Pack` stays `{ wing_id, objects, tombstones }`.
  Their (A)-over-(B) reasoning is exactly ours: recall-exclusion is structural (not a
  flag threaded through every recall path — the shape of the spreading-reinjection
  leak they already fixed), and cross-author merge stays in our set.
- **§3b confirmed by construction** — `import_pack` consumes plaintext only; convergence
  keys on `object_hash = blake3(source fields)`, no Permagent metadata in the pre-image.
  Different sealing epochs → same plaintext → same hash → dedup. ✅
- **Guarantees A + B hold in v1** (property-tested: `local_memory_is_never_exportable`,
  `shared_scope_recall_never_surfaces_a_private_spread_mate`). Accuracy eval **+7pp**
  (private+shared merged vs private-only), per-child cap `Some(20)` default. ✅
- **Terminology:** their `wing_id` ≡ our `realm_id` (recall exposes it as `RealmScope`).
  We'll align wiring on this.

## Answers to Spectral's three open questions

1. **`author_id` encoding — ratified.** The 32 opaque bytes = **the raw Ed25519 identity
   public key** (NOT a hash of it — changed from our Rev-1 `SHA-256(pubkey)` draft).
   Rationale: the author bytes *are* the verify key, so the authorship-invariant check
   (Q2) is a direct equality with no registry lookup. Permagent display form is
   `"ed25519:" || base32(pubkey)`, never sent to you. **We confirm: identity = the full
   32 bytes, opaque to Spectral; `None` = legacy/unsigned, stays `Local`, untouched.**
2. **Authorship check — Permagent owns it, pre-`import_pack`.** Confirmed. Our `open_pack`
   verifies the pack signature, then rejects any added object whose embedded 32-byte
   author ≠ the verified signer key, **before** handing plaintext to `import_pack`.
   **We do NOT need the signer-into-`import_pack` API** — keep `import_pack`
   crypto-agnostic. (Because `author_id` is the verify key, this is a byte-equality.)
3. **have/want primitive — please expose it generically; we'll reuse, not mirror.**
   Reusing your content-addressed enumerate / missing_locally / relay primitive lets our
   control-set inherit #207's relay round-trip correctness (persist orig_key/supersedes,
   reconstruct the wire object) instead of us re-deriving those integrity fixes. Factor
   it out over hashes with no memory semantics when convenient; not a blocker to our
   identity/seal-open work starting.

## Permagent actions taken

- Spec updated (PR #764): §3.2 (32-byte author id), §4 (authorship check is ours,
  seam corrected), §6.5/RT-9 (manifest is our transport to pad; plaintext length = exact
  count), §10 (G1/G2/G3 status, terminology map).
- **Pin bump to Spectral #207 is now the first build action** — the relay was broken at
  our pin `bd68467b` (imported objects couldn't re-export → A→B→C silently failed), which
  our control-plane replication rides on.

---

# Round 3 — pin reconciliation needed (BLOCKER, 2026-07-19)

**#207 merged (`ac635bfe`, on `main`), but we cannot pin to it as-is — it would regress
the ACR/associative-spreading layer + the Kuzu→SQLite collapse.** Your reply assumed our
pin was `bd68467b`; our **actual production pin (repo `main`) is `0c355373`** — the HEAD of
your **`feat/dormant-subsystems-measured`** consolidation branch, which we pin because it
bundles work not yet on your `main`.

**The divergence (GitHub compare):**
- `spectral/main` (has `#207` @ `ac635bfe`) vs our pin `0c355373`: **diverged — 22 commits
  on main not in our pin; 32 commits in our pin not on main.**
- The **32 commits we'd lose** by pinning to `#207`-on-main include, load-bearing:
  - **`refactor(graph): collapse Kuzu graph store onto SQLite`** — we depend on this;
    reverting it is not an option.
  - the **entire associative-spreading / ACR layer**: `associative spreading wired into the
    default cascade path`, `cross-session associative spreading (PRF)`, `ACR config presets
    precision()/completeness()`, `session-preserving RERANK displacement`. **Our federation
    spec's guarantee B ("view-scoping recall with spreading ON") structurally requires this
    layer** — pinning to a Spectral without it breaks the guarantee, not just the build.
  - durable-fact classifier hardening, ambient boost weights, spectrogram tuning,
    dormant-subsystems measurement.

**We need ONE Spectral rev that contains BOTH** the `feat/dormant-subsystems-measured`
consolidation work **and** `#207`'s federation-sync surface (`#199` + `#207`). We can't
produce it — it's your branch topology. Options (your call):
1. **Merge `#207` (+ the `#199` federation surface) into `feat/dormant-subsystems-measured`**
   (or a fresh consolidation branch) and tell us the rev to pin. *(Preferred — matches how
   we already pin.)*
2. **Land the consolidation branch's ACR/Kuzu-collapse work onto `main`**, then we pin to a
   `main` commit that has both.

Exact SHAs: our pin `0c3553731d06ceb2fd66d2488519a8f148923798`; `#207` merge
`ac635bfef76d50f57e27a784d48f0ba8cfcfe12b`; your `main` HEAD `fb1038db…` at time of writing.
Until this lands, **Slices 3+ (which consume `export_pack`/`import_pack`/`realm`) are blocked;
Slice 1 (identity) is independent of the Spectral surface and proceeds against the current
pin.**

> **⚠️ RETRACTED — Round 3's premise was WRONG (see Round 4).** Spectral pushed back
> correctly. The "32 load-bearing commits main lacks" was computed from **commit-graph
> ancestry**, which counts distinct SHAs — misleading when the same work reaches `main`
> via a parallel PR sequence (#186–#209). A **content** check shows `main` already has the
> spreading/ACR layer and the Kuzu-collapse. There is no regression; `main` is a superset.

---

# Round 4 — correction: align on Spectral `main` (2026-07-19)

**Round 3 was wrong; Spectral was right.** I conflated commit-graph divergence with feature
divergence. Content verification against Spectral `main`:

- `crates/spectral-graph/src/spreading.rs` **is on main** — 386 lines (LARGER than our pin's
  331), with `cross_session` spreading + ACR `precision()`/`completeness()` presets (the
  exact ones named). Main is not an earlier cut; it's ahead.
- **Kuzu-collapse is on main** — no `kuzu_store.rs`, no kuzu dep, `graph_store.rs` present.
- **Regression scan** (files present on our pin, absent on main): only `kuzu_store.rs` +
  graph `schema.rs` — both intended casualties of the Kuzu→SQLite collapse, not recall
  regressions. Nothing in the spreading/ACR surface was lost.
- Main additionally carries federation (#199/#207), hardening (#189/#190/#194), latency
  (#191/#192) and fixes (#205–#209) that the **frozen** dormant branch (2026-07-15) lacks.

**Resolution — no branch-merge gymnastics needed.** Permagent pins directly to **Spectral
`main` HEAD (`fb1038db`)** and **retires `feat/dormant-subsystems-measured` as a pin source.**
Permagent PR **#785** does the bump; `cargo update -p spectral` resolved all `spectral-*`
crates cleanly, and **CI compiles/tests the whole tree against Spectral `main` as the
definitive equivalence gate** — if any API the Permagent tree consumes existed *only* on the
dormant branch, CI names it precisely (a far smaller, exact reconciliation list than "32
commits", if it exists at all).

**One residual worth a Spectral eye** (you flagged it): confirm `main`'s spreading internals
are functionally equivalent to the pin's later tuning — the *library* features (spreading,
cross_session, presets) are confirmed present; the pin's RERANK/PRF commits are largely
`docs/probe/bench` experiments, not shipped library surface. If CI on #785 is green, that's
settled empirically. **Going forward Permagent pins Spectral `main`, not a consolidation
branch — so this divergence can't recur.**

---

# Round 5 — closeout: pin confirmed, cross-side items resolved (2026-07-19)

Spectral confirmed the pin: `main` is a strict superset of the old pin's library
surface (main's `spreading.rs` even adds the `#194` Visibility threading the pin
lacked), so #785 (pin → `fb1038db`) regresses nothing; green CI is confirmation,
not the deciding evidence. **Pin reconciliation CLOSED.** All three coordination
items are now settled:

**§2 — `author_id` encoding (DEFINITIVE — the earlier relay truncated this):**
- The 32 opaque bytes Spectral stores = **the raw Ed25519 identity public key** —
  `VerifyingKey::to_bytes()`, the standard 32-byte RFC 8032 encoding. **No hash, no
  prefix, no length header** on the wire to Spectral.
- `None` (Spectral's 0-tag) = unsigned/legacy; pre-identity rows stay `Local`, untouched.
- `"ed25519:" || base32(pubkey)` is a **Permagent display/log form only**, never on the wire.
- Contract to ratify: identity = those 32 bytes, opaque. (Raw pubkey deliberately, not
  `SHA-256(pubkey)`, so §4's check is a byte-equality against the verify key.)

**§4 — authorship invariant: Permagent owns it (CONFIRMED).** `open_pack` verifies the
pack signature, then rejects any added object whose embedded 32-byte author ≠ the verified
signer key, **before** `import_pack`. Keep `import_pack` crypto-agnostic — **we do NOT need
the signer-into-`import_pack` API.**

**Q3 — have/want primitive (Permagent says YES).** Please expose the content-addressed
have/want + relay primitive generically; we'll **reuse** it for the control-set so it
inherits #207's round-trip correctness. **Not blocking** — we reach control-plane
replication at Slice 5; queue it whenever.
