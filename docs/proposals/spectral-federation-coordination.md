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
  our control-plane replication rides on. **Please confirm the exact rev to pin once #207
  merges** (it read OPEN on 2026-07-19; main was `fc310a83`).
