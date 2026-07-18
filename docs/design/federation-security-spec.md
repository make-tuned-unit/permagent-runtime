# Federation Security Spec — Permagent's half of "git for federated brains"

> **Status: DESIGN — for review. No crypto is implemented by this document.**
> This is the security architecture for the layer Permagent owns. It exists to be
> torn apart by the most security-sophisticated person in the room before a line of
> crypto is written.
>
> **Scope split (the boundary is crypto-agnostic):**
> - **Spectral owns** the *plaintext* memory-object layer: the `realm` axis
>   (`Local` never exports vs `Shared(id)` replicates), content-addressed
>   `(key, author)` OR-Set objects, `export_pack` / `import_pack`, tombstones, and
>   scope-spanning recall. Spectral emits **plaintext** packs and consumes
>   **plaintext** packs.
> - **Permagent owns** (this doc): identity/auth, E2E **encryption of the exported
>   packs on the wire**, transport/sync, **key management** (distribution, rotation,
>   revocation), and the **who-may-tombstone-whom** authorization.
>
> The seam: Spectral hands us a plaintext pack → we encrypt + sign + move it → the
> peer's Permagent verifies + decrypts → hands Spectral plaintext to re-index. We
> never ask Spectral to know about keys; Spectral never asks us to know about memory
> semantics.

---

## 0. Ground truth — what exists today (Phase 0)

Audited at Spectral pin `bd68467b`, repo `main` @ `fa7e49e`. What we build **on** vs. what is **greenfield**:

### Identity / auth — mostly greenfield
- **`crates/goose-server/src/auth.rs` is a 3-line stub** ("intentionally empty… Auth will be re-added in Phase 2"). This is the *multi-user identity* gap. This spec fills it.
- **The real transport auth is `crates/goose-server/src/middleware/auth.rs`** — `require_bearer_token` compares `Authorization: Bearer <token>` against a single `AppState.daemon_token`. That is **device-pairing** auth (one shared secret gates the daemon), **not** per-person identity. Do not conflate the two: the bearer token says "this client may drive this hub"; it says nothing about *who* authored a memory.
- **One hardcoded user.** `session_manager.rs`: `pub const DEFAULT_USER_ID: &str = "default"`. There is a `users(id, display_name)` table, but it holds exactly one row. Spectral's OR-Set `author` has, until now, had nothing stable and verifiable to bind to.
- **AppState** carries only `daemon_token: Option<String>` — no user, device, or key fields.

### Crypto primitives — partial, usable
- **`keyring` v3.6.2** with platform-native backends (`apple-native` / `windows-native` / `sync-secret-service`) is already a dependency and already stores provider API keys ("encrypted in your system keychain — they never leave your device"). **This is the correct home for long-lived private keys.**
- **`rand` 0.10** at the workspace root is the CSPRNG source (`load_or_create_daemon_token` already does `rand::random::<[u8;32]>()`).
- **`aws-lc-rs`** is present transitively via `rustls` (goose-server `rustls-tls` feature). It can do X25519 / Ed25519 / AES-GCM / ChaCha20-Poly1305 / HKDF, but it is low-level and only wired for TLS today.
- **`rcgen` self-signed certs + SHA-256 fingerprint pinning** (`tls.rs`): the daemon prints `GOOSED_CERT_FINGERPRINT=…` and the parent process pins it. **This is a real, shipping TOFU-pinning pattern we reuse conceptually for peer-identity verification.**
- **No E2E crypto exists.** Every `ed25519` hit in the tree is SSH-key *detection* in the secret-scanner (`steward/secret_scan.rs`, `security/patterns.rs`), not crypto we perform. No nacl / libsodium / age / group-messaging code anywhere.

### Transport / fabric — a strong *intra-user* substrate, not a *cross-person* one
- **`docs/architecture/MULTI_DEVICE.md`** is the ruling: hub-and-spoke, **"don't sync — connect."** The strongest machine is the hub; every other device of *the same person* is a thin client over that person's **Tailscale** tailnet, holding nothing but a pairing token. WireGuard encrypts transport; the bearer token gates the app.
- **Multi-user is an explicit non-goal there:** *"the daemon is single-user `default` throughout — a second person is a second hub."* **This spec deliberately crosses that line** — see §1.
- The **pairing-URL** flow (`Settings → Devices`, token rides the `#fragment`, scrubbed by `api.ts browserToken()`) is real and wired. The **gateway `generate_pairing_code(expiry)`** flow (`gateway/manager.rs`) is a real time-boxed-code pattern (built for Slack/Discord inbound users). Both are **pattern precedents** for member invites.
- **Mesh / Forum is vision-only.** The "sovereign chiti-identified agents in the Forum" language is all Three.js world-rendering (`WorldView`, `Agora`, `GlyphField`); there is **no `/mesh` route, no `MeshStatus`, no peer transport backend.** Federation transport is greenfield.

### The on-chain agent identity — a **red herring for team identity**
- `ui/command-center/src/services/identity.ts` reads a single **agent** persona (Henry/"Hank", `did:chitin:henry-malcolm`, SBT #54 + passport #38105 on Base) **read-only** from chain / chitin.id, owned by an external wallet `0x95Ab…`.
- **Verdict: not team-member identity, and unusable as E2E key material** — we do not hold its private key; it is a *public, read-only provenance/vanity badge* for one agent. It cannot sign packs, cannot be wrapped to, cannot be revoked by us.
- Two *legitimate, optional* roles only: (i) a **display badge** — show a peer's verified agent-SBT next to their real author id as human-friendly provenance; (ii) it belongs to the **public** Forum/mesh presence layer, which is a *different axis* from private realm membership. **Do not conflate the public sovereign-agent-in-the-Forum identity with the private realm-member identity.** (§3 keeps them separate.)

### The existing privacy UI — honestly-labeled preview, not wired
- `SettingsView.tsx DataPanel`: a **"End-to-end encryption — Required when you have external collaborators"** toggle and a **"Keep everything on this device"** toggle. Both are `useState` only — **not persisted, not wired** — under an explicit `<PreviewNotice/>` ("these controls activate as remote features land"). This team **removes** unwired UI (the Export/Delete buttons were cut in the 2026-07-10 audit with a comment). **§7 replaces these placeholders with real wiring** — this spec is the feature that back-fills them.

**Bottom line:** we build on `keyring`, `rand`, the secrets-dir + fingerprint-pinning patterns, the invite/pairing-code patterns, and Tailscale for the *intra-user* leg. Everything else — per-person identity, pack E2E, key management, cross-person transport, tombstone authz — is greenfield.

---

## 1. Topology — federation is **hub ↔ hub (person ↔ person)**

This reframes everything, so it comes first.

- **Multi-device** (solved) is `device → hub`: many thin clients of **one** person's single Brain, over **one** person's tailnet. "Don't sync — connect."
- **Federation** (this spec) is `hub ↔ hub`: **two sovereign people, each with their own single-user hub**, who choose to share a *bounded wing* of memory. They are **not** on the same tailnet by default, and neither is a thin client of the other — each owns their own Brain.

Consequences that a reviewer must hold onto:
1. **Identity is per-person (= per-hub), not per-device.** Within a person's fleet, only the hub writes to the Brain; satellites are thin clients. So the hub holds the person's **one** federation identity keypair, and every shared object that person authors is signed by that one key. Devices never hold federation keys — they inherit federation by being clients of the hub, exactly as MULTI_DEVICE.md already has them inherit everything else.
2. **Federation is a *deliberate, scoped exception* to "don't sync."** We sync **only** the `Shared(realm)` wing, as a CRDT (Spectral's OR-Set), **never** the `Local` wing and **never** the whole Brain. "Don't sync — connect" remains correct *intra-user*; *cross-person* it cannot apply (two people can't both be thin clients of one Brain — nobody owns the other's). This is not a contradiction of the ruling; it is the ruling's boundary. State it as such to Jesse.
3. **A "team" is a set of hubs that replicate one or more shared realms.** Small N (2–10 people is the Permagent reality). This bounds the whole key-management design — we do **not** need large-group cryptographic machinery for v1 (§5).

---

## 2. Threat model

### 2.1 The two sovereignty guarantees, at two layers, defending different adversaries

This is the crux a buyer will probe. **Do not conflate these.**

| # | Guarantee | Layer / owner | Enforcement | Defends against | Strength |
|---|---|---|---|---|---|
| **A** | **Structural export-gate.** A `realm=Local` object is **never** present in *any* exported pack. Nothing about a Local memory ever leaves the machine. | Memory-object layer (**Spectral**) | Enumerate-filter **+** pack-serializer **+** a property test asserting no `Local` object can appear in a pack | The relay, the network, **and** teammates | **HARD.** Cryptographic-grade sovereignty — the confidentiality of Local memory reduces to "it is never serialized outbound." |
| **B** | **View-scoping recall filter.** A private memory never *surfaces in a **shared-scope recall output***. | Memory-object layer (**Spectral**), read path | Scope filter on recall results, **which must run with associative spreading ON** — the leak Spectral fixed was spreading re-injecting filtered memories *after* the scope filter | A **shared view leaking your own private memories** into what a teammate-facing recall returns | **Weaker — honest-participant only.** It shapes what a *cooperating* recall returns; it is **not** a barrier against a hostile local process. |
| **C** | **Local-device confidentiality.** Your on-disk plaintext is unreadable to someone who steals the machine/file. | **OS / Permagent** | Full-disk encryption (FileVault / LUKS / BitLocker), `keyring`-held private keys, `0600` secrets | A **compromised or stolen device**, a malicious local process reading its own SQLite | Depends entirely on disk/OS encryption. |

**The overclaim to avoid (a buyer will catch it):** guarantee **B does not defend against local-device compromise.** A malicious process on the machine can read the SQLite file directly and never calls recall. Reading your own private memories off your own disk is **guarantee C's** job (disk/OS encryption), not the recall filter's. The recall filter defends *shared views from leaking private mates to a cooperating reader*; it is not a confidentiality boundary against a hostile endpoint. The spec must never imply otherwise.

**Where this spec (Permagent) sits:** guarantee **A** is Spectral's and is the bedrock this whole design assumes. Permagent adds a **fourth** guarantee on top of A:

> **D — Wire confidentiality + authenticity of the *shared* wing.** Everything that *is* exported (i.e. `Shared(realm)` objects, which by A excludes all Local ones) is E2E-encrypted so the relay/network see only ciphertext, and signed so its authorship can't be forged. §3–§5.

### 2.2 Adversaries × assets

**Assets:** ① plaintext shared memories; ② *metadata* — which/how-many memories exist, and activity timing; ③ key material (private identity keys, realm keys); ④ team membership (who is in a realm).

| Adversary | ① Shared plaintext | ② Metadata (count / timing) | ③ Keys | ④ Membership | Notes |
|---|---|---|---|---|---|
| **Relay — honest-but-curious** | ✗ ciphertext only (A + D) | **Partial** — sees ciphertext sizes + timing + a per-realm routing tag → can infer *approximate volume/activity*. Per-object counts are hidden inside one encrypted pack. | ✗ never (wraps are ciphertext) | **Partial** — sees a set of pseudonymous recipient tags per realm, not names | Metadata leakage is the honest residual; §6.4 + padding (v2). |
| **Relay — malicious** | ✗ (same) | same + can correlate | ✗ | same | Additionally can **drop / delay / reorder / replay / withhold** and attempt **MITM on key exchange**. Cannot forge authored objects (Ed25519) or decrypt (no keys). Freshness/availability are the residual — §6.5. |
| **Malicious *current* teammate** | ✓ **for realms they're in — by design (non-goal to prevent)** | ✓ within their realms | Their own only | ✓ for their realms | Cannot read *other* realms (separate keys). Cannot **forge** another member's authorship (pack-signer-authorship invariant, §4). Can exfiltrate what they can already see — unstoppable (a human can screenshot). |
| **Removed teammate** | ✓ **only what they already synced+decrypted** (non-goal to claw back) | historic only | stale epoch keys only | historic | **Cannot decrypt epoch ≥ N+1** (realm-key rotation on removal, §5). Can still reach the relay but receives only undecryptable ciphertext. |
| **Network attacker (passive + active MITM)** | ✗ | what the relay sees, at most | ✗ | ✗ | Transport is TLS/WireGuard; even a broken transport yields only the same ciphertext the relay holds (E2E). Active MITM on first key exchange defended by **OOB fingerprint verification** (§3.3). |
| **Compromised / stolen device** | ✓ **for that user** (game over locally) | ✓ | ✓ **that device's keys** | ✓ | Mitigations: `keyring` + disk encryption (guarantee C); **rotation cuts the blast radius** — rekeying realms after a known compromise cuts the attacker off *going forward* (same mechanism as member removal). |
| **Curious platform operator (us / relay host)** | ✗ | same as HBC relay | ✗ **we never hold realm or private keys — they are client-side only** | partial routing tags | The "we can't read your shared memories even under subpoena" property. Client trust anchor = open source + reproducible builds. |

### 2.3 Explicit non-goals (state these plainly — they are correctness, not gaps)

1. We do **not** stop an authorized current teammate from reading what they're authorized to read.
2. **Revocation is forward-looking.** A teammate who already synced+decrypted a memory keeps that copy forever. We cut future access; we do not claw back plaintext.
3. We do **not** guarantee **availability** against a malicious relay — it can deny service. Mitigation is P2P fallback + multiple relays, not a cryptographic guarantee.
4. We do **not** (v1) hide **volume/timing metadata** from the relay. Padding / batching / cover traffic is a v2 hardening.
5. We do **not** defend a **fully compromised endpoint** from reading its own user's data — that is guarantee C (disk/OS encryption), not this layer.
6. The **recall view-scoping filter (B) is honest-participant only** — it is *not* a defense against a local/disk compromise.

---

## 3. Identity model

### 3.1 Per-person keypairs (two keys, one identity)

Each person (hub) holds, generated once at first run and stored in the OS `keyring`:

- **Ed25519 signing keypair** — the *stable identity*. Signs pack manifests and per-object provenance. Rotating this = becoming a new identity, so it is long-lived.
- **X25519 encryption keypair** — the *recipient key* realm keys are wrapped to (ECDH / HPKE). Medium-lived — rotatable without changing identity.
- **Binding:** the X25519 public key is **certified by the Ed25519 identity key** (a self-signed `enc-key-cert = Ed25519_sign(id_sk, x25519_pub || not_before || not_after)`). This lets a member rotate their encryption key without re-establishing identity, exactly as modern messengers separate identity from medium-term keys.

### 3.2 The `author` id — what binds to Spectral's OR-Set

Spectral's OR-Set is keyed `(key, author)` and needs `author` to be **stable and verifiable**. We define:

```
author_id := "ed25519:" || base32( SHA-256( ed25519_public_key ) )     # stable, collision-resistant
```

- This replaces the hardcoded `"default"` for **shared** writes.
- **Migration (coordination point with Spectral, §10):** existing rows are authored `"default"`; they predate identity and were never shareable. On identity bootstrap they **stay `Local`** and keep `"default"`; only new `Shared` writes carry the real `author_id`. The exact on-wire format of `author_id` **must be agreed with Spectral** since Spectral owns `(key, author)`.

### 3.3 Establishing + verifying identities (invite + key exchange)

The hard problem is **key authenticity** — Alice must know Bob's pubkey is *Bob's*, not the relay's (MITM). We reuse two patterns already in the tree (fingerprint pinning from `tls.rs`; time-boxed pairing codes from `gateway`):

1. **Invite** (mirrors `generate_pairing_code(expiry)` + the pairing-URL flow). The inviter (a realm admin) emits a signed invite:
   ```
   invite = { realm_id, inviter: {author_id, ed25519_pub, x25519_pub+cert},
              capability: <admin-signed grant>, one_time_nonce, expiry }
   sig    = Ed25519_sign(inviter_id_sk, invite)
   ```
   Delivered out-of-band (the same way a pairing URL is shared today — "treat it like a password").
2. **Response.** The invitee returns their signed key bundle `{author_id, ed25519_pub, x25519_pub+cert}`.
3. **TOFU + optional OOB verification.** Both sides **pin** each other's Ed25519 key on first contact (like `known_hosts` / the cert fingerprint). The UI surfaces a short **safety number** (a hash of the two identity keys, Signal-style) that the two humans can compare over an existing trusted channel (a phone call) to defeat an active MITM on the invite. Pinning is mandatory; OOB comparison is offered and encouraged.

### 3.4 How this extends `auth.rs` and the multi-device model

- **Fills `crates/goose-server/src/auth.rs`** (today's stub) with: the identity keypair lifecycle (generate/load from `keyring`), the **peer registry** (pinned `author_id → {ed25519_pub, x25519_pub+cert, verified?}`), and realm-membership state.
- **Layers cleanly under `middleware/auth.rs`:** the bearer `daemon_token` still gates "may this client drive this hub" (unchanged, device-pairing). The new identity layer answers the orthogonal "*who* authored this shared object / *who* may I wrap a key to." Two independent questions, two mechanisms.
- **Devices inherit identity from their hub** — no per-device federation keys, per §1.
- **Keep the public Forum/chiti-ID identity separate** (§0): chiti-ID = *public presence* in the Forum; the Ed25519 `author_id` = *private realm membership + authorship*. A peer may *display* their agent-SBT as a badge, but membership and signatures never depend on it.

---

## 4. E2E pack encryption

Spectral hands us a **plaintext pack** for a realm at a given point in the have/want negotiation. We produce a **sealed pack**:

```
epoch          := current key-epoch of realm_id
ct             := AEAD_encrypt(realm_key[epoch], nonce, plaintext_pack,
                               aad = { realm_id, epoch, sender = author_id, pack_version })
sealed_pack    := { realm_id, epoch, sender: author_id, nonce, ct,
                    sig = Ed25519_sign(sender_id_sk, SHA-256(realm_id || epoch || nonce || ct)) }
```

Design points, each defending a specific attack:

- **AEAD cipher:** **XChaCha20-Poly1305** (192-bit nonce → random nonces are safe with no counter bookkeeping; misuse-resistant; widely audited). AES-256-GCM (available in `aws-lc-rs`, hardware-accelerated) is an acceptable alternative *only* with strict nonce discipline. **Recommend XChaCha20-Poly1305.**
- **Confidentiality + integrity of content:** the AEAD. The relay/network see only `ct`.
- **Sender authentication — the property the AEAD alone does NOT give you.** Every realm member holds `realm_key`, so AEAD integrity proves "*some* member sealed this," not *which*. The **Ed25519 signature** over `(realm_id, epoch, nonce, ct)` binds the pack to a specific `author_id`, verified against the pinned key **before** decryption.
- **Anti-splicing:** the signature and the AEAD `aad` both bind **`realm_id` + `epoch`**, so a malicious relay cannot replay realm R1's pack into realm R2's channel, nor a stale epoch's pack as current.
- **The authorship-forgery invariant (state this explicitly — it is what makes `(key, author)` trustworthy):**

  > **A pack signed by member X may only *add* objects whose `author == X`.** On `import_pack`, the recipient rejects any added object whose embedded author ≠ the verified pack signer. (Tombstones are the sole cross-author exception — governed by §7.)

  This is why a malicious teammate cannot forge *your* memories: they can seal packs, but any object they add is constrained to *their own* authorship, and cross-author adds are dropped at merge-validation.
- **Granularity:** sign at **pack** level for v1 (simpler; the invariant above already prevents cross-author injection). Per-object signatures are a hardening option for non-repudiation and are noted, not required.
- **Recipient trust flow:** verify `sig` against pinned `ed25519_pub` for `sender` → check `epoch` is ≥ the member's current epoch and `realm_id` matches the channel → AEAD-decrypt → hand plaintext to Spectral `import_pack` → merge-validation enforces the authorship invariant. Any failure → reject, do not re-index.

---

## 5. Key management (the part that decides whether this is real)

### 5.1 Realm key + epochs

- A **realm key** is a random 256-bit symmetric key. It has an **epoch** (monotonic counter). All pack AEAD uses `realm_key[current_epoch]`.
- On realm creation the admin generates `realm_key[0]`.

### 5.2 Distribution — flat per-member wrapping (a "realm keyring" object)

For each member M with certified X25519 pubkey `x_M`:
```
wrap_M[epoch] := HPKE_seal(x_M, realm_key[epoch], info = { realm_id, epoch })
```
The set `{ wrap_M[epoch] }` for all members is a **realm-keyring object**, itself replicated through the *same untrusted relay* — it is ciphertext; only the holder of `x_M`'s secret can unwrap. The keyring object is **admin-signed** and carries the monotonic `epoch` so members can't be silently downgraded.

- **Why flat wrapping, not a tree:** N is 2–10. Flat wrapping is **O(N)** per rotation and trivially auditable. **We do not need MLS/TreeKEM's `log N` machinery for v1** — it is named as the scale-out path (§Open Decisions) if teams ever get large.
- **Wrapping primitive:** **HPKE (RFC 9180: X25519-HKDF-SHA256 + ChaCha20-Poly1305)** — the modern, standardized sealed-box. (libsodium-style `crypto_box_seal` is an equivalent fallback.)

### 5.3 Member ADD

1. Admin wraps the current epoch to the new member: `wrap_new[current] = HPKE_seal(x_new, realm_key[current], …)`.
2. **History access is a policy knob:** to give the new member the retained shared history (usually *desired* for a team knowledge base), also wrap **all currently-live epochs** to them. To withhold history, wrap only `current`.
3. Publish the updated admin-signed keyring. **No key rotation is required on add** (adding a reader doesn't compromise past keys for existing members).
   - Recommend: **wrap all live epochs on add** (read-the-history default), configurable per realm.

### 5.4 Member REMOVE / REVOCATION — **rotate the realm key** (forward-looking)

The crux. To ensure a removed member cannot read **future** shared memories:

1. Admin generates **`realm_key[N+1]`** (new epoch).
2. Re-wraps `realm_key[N+1]` to **every remaining member** (NOT the removed one) → new admin-signed keyring at epoch `N+1`.
3. All subsequent packs are sealed under epoch `N+1`.
4. The removed member still holds `realm_key[≤N]` **and any plaintext they already synced** — **this is not clawed back** (non-goal #2, stated honestly). They keep reaching the relay but get only epoch-`N+1` ciphertext they cannot unwrap.

**The trust boundary a buyer will push on:** revocation confidentiality rests **entirely on not wrapping `realm_key[N+1]` to the removed member's pubkey** — *not* on the relay refusing them service. We assume the relay is hostile and may hand the removed member every byte; it does them no good without the wrap. That is the correct E2E posture: confidentiality is in the cryptography, not in an access-control list on an untrusted server.

- **Same mechanism = device-compromise response.** Suspected key compromise → rotate the affected realms; the attacker's stolen `realm_key[≤N]` is cut off from epoch `N+1`.
- **Forward secrecy of past traffic:** because each epoch has an independent random key, a compromised *current* key exposes only the *current* epoch's packs, not all history — *provided we actually rotate* on a cadence.

### 5.5 Rotation cadence

- **Mandatory** on every member **removal** (correctness — without it, revocation is meaningless).
- **Recommended** periodic epoch bump (hygiene — bounds the blast radius of an undetected key leak).
- **Not required** on member add.

### 5.6 The distributed wrinkle to flag — offline admin

Rotation requires *someone with the rotate capability* to be online to generate + re-wrap. If the sole admin is offline when you must remove a rogue member, you cannot rotate. **Mitigation: ≥2 admins per realm** (any admin can rotate), or a member-initiated rekey protocol that converges. Flagged as an operational decision, not silently assumed away.

### 5.7 Key material at rest

- **Long-lived private keys** (Ed25519 identity, X25519 recipient) → **OS `keyring`** (the existing pattern), never a flat file.
- **Realm keys** (symmetric, per-epoch) → the encrypted secrets store / `~/.permagent/secrets/` at `0600` (the `daemon_token` precedent), or keyring if per-realm entries are acceptable. Their confidentiality also leans on guarantee **C** (disk encryption).

---

## 6. Transport

### 6.1 The two questions

(a) the **fabric** — reuse Tailscale, or app-level over any network; (b) the **topology** — direct P2P daemon↔daemon, or a dumb relay.

### 6.2 The Tailscale reality check

Tailscale is excellent for the **intra-user** leg (multi-device) — one person, one tailnet, WireGuard, ACLs. But **two different people are on two different tailnets.** Cross-person federation over Tailscale needs **node-sharing / tailnet-lock**, which works but (i) is operationally heavy, (ii) couples both parties to Tailscale accounts, and (iii) does not serve **offline/async** members at all (P2P needs both ends up). So Tailscale is a *fast-path when it applies*, **not a foundation cross-person federation can require.**

### 6.3 The topologies

- **P2P direct (daemon↔daemon):** lowest latency; no third party sees even ciphertext; but needs both peers online + NAT traversal, and fails offline/async teams.
- **Dumb encrypted relay (blind store-and-forward of ciphertext):** holds sealed packs + keyring blobs, serves on demand. **Honest-but-curious-safe by construction** — it only ever sees ciphertext + minimal routing metadata. Enables **offline/async** members (the real team case). This is how shipping E2E systems (Signal, Matrix+E2EE, Keybase) achieve availability without trusting the server.

### 6.4 Recommendation for v1 — relay floor + opportunistic P2P

**Mandatory floor: a dumb encrypted relay.** Transport-agnostic, works across arbitrary networks, serves offline members, sees only ciphertext. **Opportunistic fast-path: P2P over the existing Tailscale fabric** when both peers share a tailnet (reuse the multi-device substrate for the LAN/shared-tailnet case). Spectral's **have/want hash-manifest** negotiation rides on top of either; **we move the bytes, we don't compute the manifest.**

### 6.5 What the relay sees, precisely (the honest metadata disclosure)

- **Sees:** an opaque per-realm **routing tag** (recommend a *rotating/blinded* tag, not a stable realm-id, to limit long-term linkability), a per-realm pseudonymous **sender/recipient tag**, **ciphertext size**, **timing**, and **IP**.
- **Does NOT see:** any plaintext; per-object counts (hidden inside one sealed pack); memory content; key material.
- **Residual leak:** approximate **volume/activity** from sizes + timing. **v2 hardening:** pad packs to size buckets / batch on a schedule / cover traffic.
- **Freshness vs. availability against a *malicious* relay:** the OR-Set is a CRDT, so **reorder/delay/replay are safe for convergence** (adds/tombstones commute; replay of a known element is a no-op). The residual attacks are **withholding** (member misses updates) and **downgrade** (serve a stale keyring). Downgrade is defended by **monotonic admin-signed epochs** (members never accept an epoch < their current). Withholding is an **availability** attack — non-goal to prevent cryptographically; mitigated by P2P cross-check + multiple relays.

---

## 7. Who-may-tombstone-whom (authorization)

A tombstone is just another OR-Set object; "unauthorized" tombstones are stopped at the recipient's **merge-validation**, consistent with §4's authorship invariant. The policy is *which signer a recipient will accept a tombstone from*:

- **Author retracts own memory — always allowed.** A pack signed by X may tombstone X's own objects. Uncontroversial and always on.
- **Admin/scope-owner retracts *others'* memory — a policy choice.** Options:
  - **(a) Author-only (strictest):** only the author can tombstone their object. Clean, censorship-proof — but a realm can't remove a departed/rogue member's content.
  - **(b) Admin-override:** realm admin(s) may tombstone anyone's object (moderation / cleaning up a departed member). The tombstone must be **admin-signed**, and all members must recognize the admin set (which then needs its own management — who anoints admins?).
  - **(c) Hybrid (recommended):** author-only for normal retraction **+** an admin **"quarantine"** that is itself a *visible, disputable* OR-Set object (transparent moderation, not silent deletion). Preserves the E2E principle that no one silently rewrites your authored history, while giving realms a real remove-a-departed-member story.

**Recommendation: v1 = hybrid (author-only retraction + transparent admin-quarantine).** Enforced by recipients accepting a tombstone only if its signer ∈ `{ object's author } ∪ { realm admins }`, with quarantine surfaced in the UI rather than applied silently.

**Flag as a product decision:** admin **hard-delete** (silent, non-disputable removal of another member's memory) has real censorship/trust implications in an E2E system. Do not enable it without an explicit product ruling. (Even then, note: in a CRDT you cannot force *other members* to honor a tombstone they don't accept — "delete" is always "delete for cooperating readers," never physical erasure from a member who forks.)

---

## 8. Realm UI + the sovereign-flag unification

### 8.1 Realm assignment UI

- The user marks a **memory** or (more usefully) a **project/scope** as `Shared(team)` vs `Local`.
- **Home:** back-fill the honestly-labeled-preview controls in `SettingsView.tsx DataPanel` (the dead `e2e` / "Keep everything on this device" toggles) with **real** wiring, plus a **per-project** realm selector (a project is the natural sharing unit).
- **Default = `Local`.** Sharing is explicit opt-in — matches "your data is yours." Every control must hit a real mounted endpoint (no dead UI — the team's hard bar).

### 8.2 `sovereign` = one flag, **two enforcement points at two layers**

`sovereign` is a **single user-facing flag** with two *independent* enforcement points at two *different* layers, defending different things:

| Half | Layer / owner | What it enforces | Can the memory layer guarantee it? |
|---|---|---|---|
| **(a) EXPORT half** | Memory-object layer (**Spectral**) — `realm=Local` | The object is structurally excluded from every pack (guarantee **A**). Free, structural, cryptographic-grade. | **Yes** — it's the export-gate itself. |
| **(b) INFERENCE half** | Actor/inference boundary (**our sovereignty-router**) — `local-only-inference` | Context for this project is only ever sent to a **local** model, never a cloud provider. | **No** — the memory layer hands back context *regardless of where inference runs*; only the router can pin inference local. **This half is ours.** |

**The relationship (state it exactly):**
```
sovereign(project)  ⇒  realm=Local  AND  local-only-inference
realm=Local  ⊇  sovereign          # realm=Local is the SUPERSET; sovereign is the STRICT specialization
```
i.e. an object can be **`realm=Local`** (never exported) yet still have its context sent to a **cloud** model at inference time. Marking the project **`sovereign`** *additionally* pins inference local. **Sovereign = Local realm + local-only inference.** One concept, two enforcement points, two layers — and the memory layer alone can only ever guarantee the export half.

### 8.3 Dependency to reconcile (do NOT hard-spec here)

The **sovereignty-router's Phase-0 is landing separately** and will decide **where the `sovereign` flag physically lives** (project row? config? Brain?). This spec **does not** hard-spec that storage. The reconciliation requirement is only this:

> **Whatever stores `sovereign` must drive *both* enforcement points** — set `realm=Local` at the memory layer **and** pin `local-only-inference` at the router. A single source of truth, two consumers.

---

## 9. Recommended cryptographic primitives (so "the crypto is right")

| Role | Recommendation | Why / alternative |
|---|---|---|
| Identity signing | **Ed25519** | Fast, small, ubiquitous; `author_id` + pack sigs. |
| Key agreement / wrapping | **X25519** via **HPKE (RFC 9180)** | Standardized sealed-box; `crypto_box_seal` is an equivalent fallback. |
| Pack AEAD | **XChaCha20-Poly1305** | 192-bit nonce → safe random nonces, no counter bookkeeping. AES-256-GCM (in `aws-lc-rs`) acceptable *with* strict nonce discipline. |
| KDF | **HKDF-SHA-256** | Standard; already inside HPKE. |
| Safety number / fingerprint | **SHA-256 truncation, Signal-style** | Reuses the `tls.rs` fingerprint-pinning mental model. |
| CSPRNG | **`rand` 0.10** (already in-tree) | Same source as `daemon_token`. |
| Private-key storage | **`keyring` v3.6.2** (already in-tree) | Platform-native keychain; the existing API-key pattern. |

**Library options for the E2E stack** (pick one, in Open Decisions territory): the **RustCrypto suite** (`ed25519-dalek`, `x25519-dalek`, `chacha20poly1305`, `hkdf`, `hpke`) — composable, audited, maintained (recommended); or **`age`** (X25519 + ChaCha, audited, recipient-oriented — a clean fit for encrypting *packs* as files, less so for rotation bookkeeping); or **`dryoc`/`orion`** (pure-Rust libsodium-ish). **`aws-lc-rs`** is already present (FIPS-friendly) but low-level. **`OpenMLS`** is the named scale-out option **only if** we ever outgrow flat wrapping.

---

## 10. Dependencies & coordination points

1. **Spectral — `author_id` format.** Spectral owns `(key, author)`; the exact on-wire `author_id` encoding (§3.2) must be agreed jointly, plus the migration of legacy `"default"` rows (they stay `Local`).
2. **Spectral — realm/pack surface not yet integrated.** `export_pack` / `import_pack` / `realm` / OR-Set / tombstone appear **nowhere** in the Permagent tree at pin `bd68467b`. This spec assumes the *contract* (plaintext packs in/out, structural export-gate, view-scoping recall with spreading-on); wiring is pending a pin bump. Track it.
3. **Sovereignty-router — `sovereign` flag storage (§8.3).** Landing separately; reconcile so one source of truth drives both enforcement points. Do not duplicate the flag.
4. **`auth.rs` fill (§3.4).** The identity keypair + peer registry + membership live here; the bearer `daemon_token` in `middleware/auth.rs` stays as-is (orthogonal, device-pairing).

---

## Open Decisions — hard tradeoffs for Jesse + coordinator (recommendation each)

> These are deliberately **not** decided unilaterally. Each is a real fork with security/product weight.

### OD-1. P2P vs. relay for v1
- **Tradeoff:** P2P = no third party sees even ciphertext, lowest latency, but both peers must be online + NAT traversal, and offline/async teams are unserved. Relay = serves offline/async (the real team case) and is honest-but-curious-safe by construction, but a third party sees ciphertext + timing/size metadata and can withhold (availability).
- **Recommendation:** **Dumb encrypted relay as the mandatory floor + opportunistic P2P-over-Tailscale fast-path.** Real teams are async; the relay only ever sees ciphertext; P2P is a latency optimization where a shared tailnet exists, never a requirement. (§6.4)

### OD-2. Per-user keypair identity vs. simpler team-shared-secret
- **Tradeoff:** A team-shared-secret is *simpler* (one key, wrap nothing) but **fatally can't attribute authorship** (any member can forge any `(key, author)`), can't do per-member revocation without rotating for everyone with no cryptographic notion of *who* left, and can't support OOB identity verification. Per-user keypairs cost an identity/registry layer.
- **Recommendation:** **Per-user keypairs (Ed25519 + X25519), no contest.** The verifiable `author` that Spectral's OR-Set requires is *impossible* with a shared secret. The simplicity of the shared secret buys a design that fails the core requirement. (§3, §4)

### OD-3. Key-rotation / revocation mechanism
- **Tradeoff:** Flat per-member wrapping = O(N) rotations, dead-simple, auditable — but O(N) work per membership change. MLS/TreeKEM = O(log N) + post-compromise security, but heavy machinery for tiny teams.
- **Recommendation:** **Epoch'd realm key + flat HPKE per-member wrapping; rotate on every removal, wrap-all-live-epochs on add, optional periodic bump.** Flag the **offline-admin** wrinkle → **≥2 admins per realm.** Defer **MLS/OpenMLS** to a large-team future. (§5)

### OD-4. Who-may-tombstone-whom
- **Tradeoff:** Author-only = censorship-proof but can't remove a departed member's content. Admin-override = moderation power but needs a managed admin set and risks silent history rewrites in an E2E system.
- **Recommendation:** **Hybrid — author-only retraction + *transparent, disputable* admin-quarantine.** **Flag admin hard-delete as a product decision** with explicit censorship implications. **Product decision, surfaced not decided.** (§7)

### OD-5. Reuse Tailscale fabric vs. purpose-built transport
- **Tradeoff:** Tailscale = free WireGuard identity/encryption/ACLs, already shipping for multi-device — but two people are on two tailnets (needs node-sharing), couples both to Tailscale accounts, and can't serve offline members. Purpose-built app-level E2E over a dumb relay = network-agnostic, offline-capable, no third-party account dependency — but we build/run relay infrastructure.
- **Recommendation:** **Purpose-built app-level E2E over a dumb relay for the general cross-person case; Tailscale as an opportunistic fast-path only** (intra-user multi-device stays Tailscale as today). Cross-person federation must **not** *require* both parties to share a tailnet. (§6.2–6.4)

---

## Appendix — what filling `auth.rs` looks like (design sketch, not code)

- **Identity module:** generate-or-load `{ed25519, x25519}` from `keyring`; expose `author_id`; sign/verify helpers.
- **Peer registry:** persisted `author_id → {ed25519_pub, x25519_pub+cert, verified: bool, pinned_at}` — TOFU pin on first contact, `verified` set when OOB safety number is confirmed.
- **Realm state:** `realm_id → {members: Set<author_id>, admins: Set<author_id>, current_epoch, keyring, routing_tag}`.
- **Seal/open pipeline (§4):** `seal_pack(realm_id, plaintext) -> sealed_pack`; `open_pack(sealed_pack) -> plaintext` (verify sig → check epoch/realm → AEAD-open → hand to Spectral `import_pack`, which enforces the authorship invariant at merge).
- **Membership ops (§5):** `add_member`, `remove_member` (→ rotate), `rotate` (→ new epoch, re-wrap remaining, publish admin-signed keyring).
- Untouched: `middleware/auth.rs` bearer `daemon_token` (device-pairing) stays exactly as-is.
```
