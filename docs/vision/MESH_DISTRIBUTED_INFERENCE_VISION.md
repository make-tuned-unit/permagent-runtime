# Mesh Distributed Inference — Vision Document

**Author:** Jesse Sharratt / Atlas Atlantic
**Date:** May 15, 2026
**Status:** Future TODO — validated research retained; implementation deferred while coding-harness instrumentation is active
**Depends on:** Mesh Forum (Phase 2), Chitin ID verification (in progress)

**Validated 2026-09-04:** The collective-compute vision remains sound, but its
privacy guarantee is conditional on the execution mode. Chitin and the
blockchain can establish identity, policy, commitments, reputation, and
settlement; they cannot make a peer's runtime confidential. FileVault or an
encrypted APFS volume protects model and request data at rest, not while a
normal macOS process is using plaintext during inference. NVIDIA PAIR can
federate independent requests across a user's trusted LAN, but it does not
split one model across machines or provide confidential execution. The phased
architecture below makes those boundaries explicit.

---

## The Vision

Permagent Mesh members pool hardware resources so every participant can access
models or aggregate inference capacity beyond one machine. A user with 16GB of
RAM may contribute to a trusted distributed pod that runs a larger model, or
may send an independent job to a verified whole-model host. Prompts remain
host-confidential only when the selected execution tier has a verified
confidential-compute guarantee; encrypted transport and encrypted storage alone
do not provide that guarantee.

This turns the Mesh from a knowledge-sharing network into a **compute-sharing network** — and makes Permagent's local-first architecture a collective advantage rather than an individual constraint.

---

## Why This Matters

The local-first promise has a ceiling: your agent is only as capable as the model your hardware can run. An M1 with 16GB runs a 7-8B model. An M4 Max with 128GB runs a 70B model. The quality gap between those tiers is significant — larger models produce better descriptions, better reasoning, better recall.

Mesh distributed inference removes the ceiling. Your hardware determines what you *contribute*, not what you *access*. A 16GB machine contributes its share of compute to the pool and gets access to a model that needs 64GB+ to run. The Mesh makes everyone's agent smarter.

This also creates a natural incentive loop for Mesh participation: join the Mesh → your agent gets better models or more throughput → your agent produces better work → you contribute more compute → the Mesh gets stronger. The incentive is real capability, with on-chain reputation or settlement accounting for contributions without placing prompts or inference state on-chain.

---

## Architecture — Three Layers

### Layer 1: Identity + Trust (Chitin ID)

Chitin's identity direction provides the eligibility and accountability seam.
The present repository contains integration seams, but the distributed-compute
trust path is not yet a production guarantee. This layer determines:

- Who can join the compute pool (verified Chitin members only)
- Trust boundaries (you choose which peers to share compute with)
- Reputation tracking (reliable compute contributors earn reputation; unreliable ones lose it)
- Accountability (privacy-preserving receipts and disputes tied to a stable identity)

The trust model begins with verified Mesh members whose Chitin reputation the
requester can inspect and whose device keys can be revoked. Verification is an
admission and accountability control; it is not proof that a device is
uncompromised, that computation is correct, or that its owner cannot inspect a
plaintext workload. Execution policy must still distinguish `local`,
`trusted-circle`, `attested-confidential`, and `fully-encrypted` workloads.

### Layer 2: Coordination, commitments, and settlement

Coordination has a hot path and a durable path. Keeping them separate is
essential for token latency and for keeping private request metadata off a
public ledger.

**Permagent's off-chain hot path handles:**

- live capacity, model inventory, queue depth, bandwidth, and health;
- topology-aware request or shard assignment, leases, cancellation, retry, and backpressure;
- mutually authenticated encrypted transport and ephemeral request keys;
- execution receipts, canary challenges, redundant verification when justified, and failover.

**Chitin/Base L2 handles:**

- membership, device-key authorization/revocation commitments, and reputation;
- content-addressed model manifests, permitted model/version hashes, and policy commitments;
- batched contribution receipts, dispute outcomes, and reward settlement;
- commitments to verification evidence or future proof systems, never raw prompts, activations, keys, or per-token routing.

Large model weights are not hosted on the blockchain. Encrypted, content-addressed
model blobs may live on Arweave or another storage fabric, while the chain holds
their hashes, policy, and settlement references. Access control means releasing
a decryption key to an eligible device; it cannot revoke a plaintext model that
a device has already legitimately decrypted, so model licensing and key
rotation remain separate controls.

### Layer 3: Private Compute (The Privacy Guarantee)

This is the technically hardest layer and the one that evolves over time. Three approaches, from most practical today to most ambitious:

#### Approach A: Hybrid Split Inference (practical today)

Your machine runs the first few and last few transformer layers locally. Only
the middle layers run on Mesh peers' machines.

**Privacy boundary:** Middle-layer activations are not plaintext tokens, but
they encode the prompt and are not ciphertext. Activation inversion and
attribute-inference attacks are an active research area, and every model-serving
peer normally has the model weights. Treat this mode as confidential only
inside an explicitly trusted circle and only for workloads whose policy permits
that exposure.

**Trust model:** Practical risk reduction within a trusted Mesh circle, not a
cryptographic privacy guarantee. Chitin verification improves accountability
but does not prevent a peer from recording or attacking activations.

**What makes this work for Permagent:** The split is asymmetric based on hardware. Your 16GB machine runs layers 1-4 and 29-32 locally (small memory footprint). A peer's 64GB machine runs layers 5-28 (the bulk). Your raw text never leaves your machine. The peer does heavy compute on abstract representations.

**Limitation:** Not provably private. A sufficiently motivated attacker with model weights could attempt activation inversion.

#### Approach B: Attested confidential inference (blocked on a real execution primitive)

Every Apple Silicon Mac has a Secure Enclave, but third-party applications do
not run arbitrary transformer code inside it. Public CryptoKit APIs expose
hardware-backed key agreement and signing; FileVault/Data Protection use
hardware keys and inline storage encryption. A model running in a normal
macOS, Metal, or Neural Engine process still consumes plaintext activations in
the host's ordinary runtime boundary. An encrypted partition therefore protects
the model and cached requests when powered off or locked, not from the machine
owner or a compromised privileged runtime during inference.

Apple Private Cloud Compute demonstrates the right *system* pattern: custom
Apple-silicon server hardware, a hardened and inspectable OS image, no privileged
runtime access, stateless request handling, hardware-rooted attestation, public
software measurements, and clients that encrypt only to approved measurements.
That is not a general API that turns an ordinary participant's Mac into a PCC
node.

**Required gate:** MESH may label a route `attested_confidential` only when a
supported runtime proves the exact executable/model measurement, binds an
ephemeral request key to that measurement, prevents host privileged access,
provides rollback/revocation and public verification material, and demonstrates
prompt non-retention. Until such a primitive exists on participant hardware,
sensitive prompts stay on the user's own devices or an independently qualified
confidential-compute service.

#### Approach C: Fully Encrypted Inference (research horizon; no promised date)

Fully Homomorphic Encryption (FHE) applied to neural network inference. Computation happens on encrypted data — no party ever sees plaintext.

**How it preserves privacy:** Mathematically proven. The encryption scheme allows addition and multiplication on ciphertexts. The Mesh peer computes matrix multiplications on your encrypted activations and produces an encrypted result. Nobody sees plaintext at any point.

**Current state:** FHE libraries and encrypted-inference research exist, but
interactive transformer-scale inference is not a qualified MESH deployment
path. Performance, supported operators, quantization effects, model quality,
communication volume, and proof costs must be measured against a frozen model
and workload instead of projected from a headline multiplier.

**What would change this:** Hardware FHE accelerators (being researched), algorithmic breakthroughs in FHE-friendly network architectures, or hybrid FHE/plaintext schemes that encrypt only the sensitive parts of the computation.

**Timeline:** Unknown. This phase is gated by measured feasibility and a
security review, not a calendar estimate.

#### Supporting Technology: Zero-Knowledge Proofs of Inference

ZK proofs verify that "this result was produced by running this specific model on some input" without revealing the input. The Mesh peer proves correctness; you verify the proof on-chain.

**Current state:** EZKL, Modulus Labs have demonstrated ZK proofs for small neural networks. Proofs are verifiable on-chain. Mathematically sound.

**Limitation:** Proof generation overhead for 7B+ models is currently impractical. Hardware acceleration of ZK proof generation (Ingonyama, Cysic) is the key enabler.

**Role in the architecture:** ZK proofs are the *verification* layer, not the *privacy* layer. They complement TEE or FHE by letting you verify a peer computed correctly without re-running the computation yourself.

---

## Data Flow — What Stays Local, What Travels

```
YOUR MACHINE (always local):
├── Brain (Spectral DB) — never leaves
├── Documents — never leave
├── Memories — never leave
├── Model layers 1-4, 29-32 (local split) — never leave
└── Raw prompt text — stays local in split mode, or is encrypted in
    transit to an executor that can read it unless attested/FHE guarantees apply

MESH PEER'S MACHINE:
├── Model layers 5-28 (middle layers) — potentially licensed/access-controlled
├── Activations — plaintext representations in trusted split mode,
│   or encrypted to a qualified confidential runtime in an attested mode
└── Encrypted result — sent back to you

BLOCKCHAIN:
├── Membership/model/policy commitments — not live shard routing
├── Batched contribution receipts — no prompt or activation content
├── Reputation scores — updated from Chitin ID
├── ZK proofs of correct inference — verifiable
└── Model weight pointers — Arweave CIDs, access-controlled
```

**The guarantee:** The Brain database and document/memory stores are never
federated as stores. However, when their content is included in an inference
prompt, that information necessarily enters the inference payload or its
derived activations. The route's privacy label must therefore describe what the
remote executor can actually observe. `Encrypted in transit`, `trusted peer`,
and `host-confidential` are separate claims and must never be collapsed into
one badge.

---

## Implementation Sequence

### Phase 2 (current trajectory): Knowledge Mesh

The Mesh launches as a knowledge-sharing network. Agents share memories, skills, descriptions — text, not compute. The trust boundary is Chitin ID. No distributed inference yet. This is valuable on its own and establishes the Mesh infrastructure that compute sharing builds on.

### Phase 3: Inference Routing Interface

Design the abstraction layer. The agent's inference path becomes a trait/interface:

```rust
trait InferenceProvider {
    async fn complete(&self, prompt: &str, context: &RecognitionContext)
        -> Result<CompletionResult>;
}

// Implementations:
struct LocalOllama;          // Today's default
struct MeshPeer;             // Hybrid split or TEE-attested
struct CloudAPI;             // Anthropic, OpenAI fallback
struct DistributedCluster;   // Full Mesh pool
```

The routing decision considers: model size needed, local hardware capacity, Mesh peer availability, user's privacy preference, cost. The user controls the routing policy — "always local," "Mesh peers OK," "cloud fallback allowed."

**This is the most important thing to build early.** The interface is useful regardless of which privacy approach matures. It separates "where does inference run" from "how is it protected" — and lets Permagent plug in new approaches without refactoring.

The route result must carry both placement and an enforceable privacy class:
`LocalOnly`, `TrustedPlaintext`, `ActivationExposed`,
`AttestedConfidential`, or `FullyEncrypted`. UI copy and audit receipts derive
from this value; trust reputation must never silently upgrade it.

### Phase 3.5: PAIR local-cluster adapter

Integrate NVIDIA PAIR beneath `InferenceProvider` as an optional loopback
provider on a participant's desktop. PAIR discovers Ollama/LM Studio engines
and routes each independent request to one eligible node in that participant's
trusted LAN. One MESH member may therefore advertise a signed capability for
an entire local PAIR cluster without exposing the cluster's internal nodes to
the Internet.

PAIR does **not** become Chitin identity, the cross-user transport, the MESH
scheduler, or the confidential/sharded inference runtime. It does not pool GPU
memory or split an in-flight request. Its first value is fan-out: separate
Council workers, batch memories, or coding DAG nodes can occupy separate local
machines. Its endpoint remains loopback behind the participant's Permagent
daemon; the daemon enforces workload policy, authenticates MESH peers, records
receipts, and chooses whether any request may leave the user's trust boundary.

### Phase 4A: Verified whole-model federation (first cross-user approach)

Verified users opt in to serve named, hashed model versions for explicitly
non-sensitive or trusted-circle jobs. Permagent distributes independent jobs,
not layers, and measures correctness, queue time, first-token latency, energy,
and failure recovery. This validates Chitin admission, secure transport,
receipts, settlement, and abuse controls before the system takes on sharding.

### Phase 4B: Hybrid Split Inference (trusted-pod research)

Implement Approach A only inside a small, mutually trusted, high-bandwidth pod.
Your machine may run the first/last layers while peers run middle layers, but
activation exposure is explicit and this mode is not offered for sensitive
workloads.

This is the proof-of-concept that validates: the routing interface works, the Mesh coordination layer handles shard assignment, peers can contribute compute reliably, and the quality improvement from larger distributed models is worth the latency.

### Phase 5: Attested confidential inference (when a qualified runtime exists)

Upgrade to Approach B only after the full attestation gate above passes on the
actual participant hardware/runtime. Apple exposing a key or Neural Engine API
alone is insufficient; the verifier must cover every component that can observe
plaintext and prevent privileged runtime access. This is the inflection point
where MESH can evaluate sensitive-data workloads, subject to independent
security and legal review.

### Phase 6: Encrypted Inference (when FHE matures)

The endgame. Fully encrypted, fully trustless, blockchain-verified distributed inference. Every Mesh member contributes RAM and compute; no member sees any other member's data; ZK proofs verify correctness on-chain. The Mesh becomes a collective intelligence infrastructure where privacy is mathematical, not social.

### Execution DAG and gates

```text
V0 threat model + privacy taxonomy
 └─→ V1 local PAIR compatibility spike
      └─→ V2 Chitin/off-chain contract split
           └─→ V3 verified whole-model federation
                └─→ V4 trusted-pod sharding research
                     └─→ V5 attested confidential runtime
                          └─→ V6 adversarial/FHE/ZK research
```

- **V0 gate:** every route has a machine-enforced privacy class; threat model
  distinguishes identity, transport, at-rest protection, host visibility, and
  correctness; UI copy cannot claim more than the selected class proves.
- **V1 gate:** PAIR remains optional and loopback-only behind the daemon;
  OpenAI/Ollama streaming, cancellation, model identity, node loss, and rollback
  pass; no paid fallback or remote plaintext listener appears.
- **V2 gate:** prompts, tokens, activations, ephemeral keys, IP addresses, and
  per-token scheduling never enter chain transactions; contract tests cover
  membership, device-key revocation commitments, model hashes, batched receipts,
  disputes, and settlement.
- **V3 gate:** verified-user request federation passes mutual authentication,
  revocation, NAT/relay, policy, canary/redundant verification, accounting, abuse,
  model-license, and non-sensitive-workload tests. A PAIR cluster advertises as
  one capability endpoint; its internal nodes stay private.
- **V4 gate:** a frozen large model that cannot fit on one node runs across a
  trusted wired pod with measured quality, throughput, first-token latency,
  activation traffic, shard loss/recovery, and deterministic model hashes.
  `ActivationExposed` remains visible throughout.
- **V5 gate:** an independent security review verifies measured code/model,
  request-key binding, protected runtime memory including accelerator paths, no
  privileged inspection, stateless deletion, revocation/rollback, public
  verification material, and resistance to replay/downgrade. Only then may
  sensitive workloads opt in.
- **V6 gate:** cryptographic privacy/correctness claims are demonstrated on the
  actual target model and latency budget; proofs are bound to model and runtime
  hashes; verification and settlement are cheaper than redundant trusted
  execution; failures cannot leak prompts or strand a request.

Each phase retains raw evidence and a rollback route. A later phase cannot
retroactively upgrade the privacy label of an earlier execution receipt.

---

## The Incentive Loop

```
Join the Mesh
  → Your agent gets access to larger models (capability upgrade)
  → Your agent produces better descriptions, better recall
  → You contribute compute to the pool (hardware contribution)
  → Your Chitin reputation grows (on-chain, permanent)
  → Higher reputation → priority access to the best Mesh models
  → The Mesh gets stronger as more members join
```

This is a genuine flywheel, not a manufactured one. The capability upgrade from larger models is real and measurable (the Librarian's description quality scales with model size — we proved this with the qwen2.5:3b → 7b upgrade). The compute contribution is real (your machine actually processes inference for peers). The reputation is on-chain and permanent via Chitin ID.

---

## Open Questions (for future design sessions)

1. **Latency:** Distributed inference across a network adds round-trip latency between each layer-group. For the Librarian's batch processing (where latency doesn't matter), this is fine. For interactive chat (where the user is waiting), the latency budget is ~2-5 seconds. Can distributed inference meet that?

2. **Model selection:** Who decides which model the Mesh runs? Is it a Mesh-wide vote, a market (highest-reputation members choose), or per-request (each member picks the model for their query)?

3. **Shard economics:** If a 70B model needs 4 machines and one goes offline mid-inference, the computation fails. How does the Mesh handle reliability — redundant shard copies, inference checkpointing, failover to a different peer?

4. **Confidential-compute availability:** Standard Apple Silicon does not expose
   PCC-style third-party inference isolation today. If Apple does not provide a
   complete attested runtime, evaluate independently attestable server TEEs
   such as AMD SEV-SNP or Intel TDX, while treating their GPU/accelerator path
   and side-channel properties as separate qualification work.

5. **Regulatory:** Distributed inference across international borders raises
   jurisdictional questions even when transport is encrypted. If a prompt is
   sent from Canada, processed in Germany, and returned to Japan, which data
   protection and export rules apply? A host-confidential mode improves the
   technical posture but does not eliminate the need for legal validation.

---

## References

- [Hyperspace AGI](https://github.com/hyperspaceai/agi) — distributed inference framework, potential infrastructure layer
- [Petals](https://github.com/bigscience-workshop/petals) — collaborative inference for large models
- [Exo](https://github.com/exo-explore/exo) — peer-to-peer inference clustering
- [Zama TFHE-rs](https://github.com/zama-ai/tfhe-rs) — Fully Homomorphic Encryption library
- [EZKL](https://github.com/zkonduit/ezkl) — ZK proofs for neural network inference
- [Apple Private Cloud Compute](https://security.apple.com/blog/private-cloud-compute/) — Apple's approach to private AI inference
- [Apple Secure Enclave CryptoKit APIs](https://developer.apple.com/documentation/cryptokit/secureenclave) — public key-agreement/signing boundary
- [Apple hardware security overview](https://support.apple.com/guide/security/hardware-security-overview-secf020d1074/web) — Secure Enclave and at-rest encryption roles
- [NVIDIA PAIR](https://github.com/NVIDIA/Personal-AI-Router) — LAN request federation; explicitly not model sharding
- [NVIDIA Dynamo](https://docs.nvidia.com/dynamo/) — disaggregated serving and KV-aware distributed inference
- [Modulus Labs](https://www.moduluslabs.xyz/) — ZK proofs for AI

---

*This document captures the long-term vision for Mesh distributed inference. It is not a build plan or a commitment to any specific timeline. The technology landscape — especially FHE, TEEs, and Apple's API roadmap — will determine when each phase becomes practical. The key architectural decision (the inference routing interface in Phase 3) should be designed during Phase 2 so that Permagent is ready to plug in distributed inference when the privacy primitives mature.*
