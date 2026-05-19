# Mesh Distributed Inference — Vision Document

**Author:** Jesse Sharratt / Atlas Atlantic
**Date:** May 15, 2026
**Status:** Vision — not scheduled for build
**Depends on:** Mesh Forum (Phase 2), Chitin ID verification (in progress)

---

## The Vision

Permagent Mesh members pool hardware resources so every participant gets access to models their individual machines can't run. A user with 16GB of RAM can't run a 70B model alone — but four Mesh members pooling their Apple Silicon machines can run it collaboratively, with each member's data staying encrypted and private throughout the inference process.

This turns the Mesh from a knowledge-sharing network into a **compute-sharing network** — and makes Permagent's local-first architecture a collective advantage rather than an individual constraint.

---

## Why This Matters

The local-first promise has a ceiling: your agent is only as capable as the model your hardware can run. An M1 with 16GB runs a 7-8B model. An M4 Max with 128GB runs a 70B model. The quality gap between those tiers is significant — larger models produce better descriptions, better reasoning, better recall.

Mesh distributed inference removes the ceiling. Your hardware determines what you *contribute*, not what you *access*. A 16GB machine contributes its share of compute to the pool and gets access to a model that needs 64GB+ to run. The Mesh makes everyone's agent smarter.

This also creates a natural incentive loop for Mesh participation: join the Mesh → your agent gets better models → your agent produces better work → you contribute more compute → the Mesh gets stronger. The incentive is real capability, not tokens or points.

---

## Architecture — Three Layers

### Layer 1: Identity + Trust (Chitin ID)

**Exists today.** Chitin ID on Base L2, Soul-Bound Tokens, ERC-8004 Passport, reputation scores. This layer determines:

- Who can join the compute pool (verified Chitin members only)
- Trust boundaries (you choose which peers to share compute with)
- Reputation tracking (reliable compute contributors earn reputation; unreliable ones lose it)
- Accountability (on-chain record of who processed what, verifiable)

The trust model: you don't share compute with anonymous strangers. You share it with verified Mesh members whose Chitin reputation you can inspect. This is the same trust boundary that knowledge sharing uses — compute sharing is an extension, not a separate system.

### Layer 2: Coordination (Blockchain)

The blockchain handles:

- **Shard assignment:** which Mesh member holds which model layers, optimized for their hardware capacity
- **Contribution tracking:** on-chain record of compute contributed, verified by result hashes
- **Reward distribution:** reputation and/or token rewards proportional to compute served
- **Result verification:** proof that distributed inference produced a valid output (not garbage from a lazy or malicious node)
- **Model weight distribution:** encrypted model shards stored on Arweave (already in Permagent's Chitin architecture), with on-chain access control determining which verified Mesh members can download which shards

### Layer 3: Private Compute (The Privacy Guarantee)

This is the technically hardest layer and the one that evolves over time. Three approaches, from most practical today to most ambitious:

#### Approach A: Hybrid Split Inference (practical today)

Your machine runs the first few and last few transformer layers locally. Only the middle layers — which operate on abstract internal representations, not raw text — run on Mesh peers' machines.

**How it preserves privacy:** Middle-layer activations are high-dimensional vectors encoding meaning in an abstract feature space. Inverting them back to the original text is theoretically possible but practically very difficult — requires knowing the exact model weights and even then the reconstruction is lossy and ambiguous.

**Trust model:** Practical privacy within a trusted Mesh circle. Not cryptographically proven, but sufficient for Chitin-verified peers. A Mesh member seeing activation vectors from layer 15 of a 32-layer model cannot straightforwardly reconstruct your prompt.

**What makes this work for Permagent:** The split is asymmetric based on hardware. Your 16GB machine runs layers 1-4 and 29-32 locally (small memory footprint). A peer's 64GB machine runs layers 5-28 (the bulk). Your raw text never leaves your machine. The peer does heavy compute on abstract representations.

**Limitation:** Not provably private. A sufficiently motivated attacker with model weights could attempt activation inversion.

#### Approach B: TEE-Attested Inference (near-term, 1-2 years)

Every Apple Silicon Mac has a Secure Enclave. The inference runs in a hardware-isolated environment that the host machine's owner cannot inspect.

**How it preserves privacy:** Your prompt is encrypted on your machine, sent to a Mesh peer, decrypted only inside their Secure Enclave / attested compute environment, inference runs inside the protected region, the result is encrypted, sent back, decrypted on your machine. The peer's OS, applications, even a compromised kernel cannot see your plaintext data.

**Trust model:** You trust Apple's hardware isolation, not the peer. The same trust model that secures Apple Pay, Face ID, and device encryption.

**Current limitation:** Apple's Secure Enclave is designed for small security-critical operations, not transformer-scale matrix multiplications. The path forward is Apple exposing Neural Engine attestation APIs — running inference on the Neural Engine with encrypted memory pages that only the attested process can decrypt. Apple's Private Cloud Compute (announced for Apple Intelligence) is moving in exactly this direction.

**Timeline:** Depends on Apple exposing the right APIs. Private Cloud Compute exists for Apple's own infrastructure; exposing similar attestation for third-party distributed inference is a natural extension but hasn't been announced.

#### Approach C: Fully Encrypted Inference (long-term, 3-5+ years)

Fully Homomorphic Encryption (FHE) applied to neural network inference. Computation happens on encrypted data — no party ever sees plaintext.

**How it preserves privacy:** Mathematically proven. The encryption scheme allows addition and multiplication on ciphertexts. The Mesh peer computes matrix multiplications on your encrypted activations and produces an encrypted result. Nobody sees plaintext at any point.

**Current state:** FHE libraries exist (Zama TFHE-rs, Microsoft SEAL). Encrypted inference has been demonstrated on small networks (MNIST classifiers, small MLPs). Not practical at transformer scale — FHE adds 10,000-1,000,000x overhead per operation. A 7B model inference that takes 2 seconds in plaintext would take weeks under FHE.

**What would change this:** Hardware FHE accelerators (being researched), algorithmic breakthroughs in FHE-friendly network architectures, or hybrid FHE/plaintext schemes that encrypt only the sensitive parts of the computation.

**Timeline:** 3-5 years for small model inference (sub-7B). 5+ years for transformer-scale. Speculative but being actively researched by well-funded teams.

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
└── Raw prompt text — encrypted before leaving, or never leaves
    (depending on approach)

MESH PEER'S MACHINE:
├── Model layers 5-28 (middle layers) — shared, not sensitive
├── Encrypted activations (from your prompt) — cannot be read
│   in plaintext (TEE) or are abstract representations (hybrid)
└── Encrypted result — sent back to you

BLOCKCHAIN:
├── Shard assignments — which peer holds which layers
├── Contribution records — compute served, verified
├── Reputation scores — updated from Chitin ID
├── ZK proofs of correct inference — verifiable
└── Model weight pointers — Arweave CIDs, access-controlled
```

**The guarantee:** Your documents, your memories, your Brain — these never leave your machine under any approach. The only thing that travels is the inference computation, and the privacy of that computation is protected by whichever Layer 3 approach is active.

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

### Phase 4: Hybrid Split Inference (first distributed approach)

Implement Approach A. Your machine runs the first/last layers locally; Mesh peers run the middle layers on abstract activations. No special hardware required. Good-enough privacy for trusted Mesh circles.

This is the proof-of-concept that validates: the routing interface works, the Mesh coordination layer handles shard assignment, peers can contribute compute reliably, and the quality improvement from larger distributed models is worth the latency.

### Phase 5: TEE-Attested Inference (when Apple opens the door)

Upgrade to Approach B when Apple exposes Neural Engine attestation APIs (or equivalent). The trust model shifts from "practical privacy within a trusted circle" to "hardware-guaranteed privacy regardless of peer trust." This is the inflection point where Mesh compute sharing becomes safe enough for sensitive data (email content, financial documents, medical notes).

### Phase 6: Encrypted Inference (when FHE matures)

The endgame. Fully encrypted, fully trustless, blockchain-verified distributed inference. Every Mesh member contributes RAM and compute; no member sees any other member's data; ZK proofs verify correctness on-chain. The Mesh becomes a collective intelligence infrastructure where privacy is mathematical, not social.

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

4. **Apple's roadmap:** Everything in Phase 5 depends on Apple exposing the right APIs. If they don't, TEE-attested inference on Apple Silicon stays theoretical. Contingency: Android/Linux TEE alternatives (AMD SEV, Intel TDX) for non-Apple Mesh members.

5. **Regulatory:** Encrypted distributed inference across international borders raises jurisdictional questions. If a prompt is encrypted in Canada, processed in Germany, and the result decrypted in Japan — whose data protection law applies? Permagent's local-first stance (your data stays on your machine, only encrypted computation travels) is a strong position but may need legal validation.

---

## References

- [Hyperspace AGI](https://github.com/hyperspaceai/agi) — distributed inference framework, potential infrastructure layer
- [Petals](https://github.com/bigscience-workshop/petals) — collaborative inference for large models
- [Exo](https://github.com/exo-explore/exo) — peer-to-peer inference clustering
- [Zama TFHE-rs](https://github.com/zama-ai/tfhe-rs) — Fully Homomorphic Encryption library
- [EZKL](https://github.com/zkonduit/ezkl) — ZK proofs for neural network inference
- [Apple Private Cloud Compute](https://security.apple.com/blog/private-cloud-compute/) — Apple's approach to private AI inference
- [Modulus Labs](https://www.moduluslabs.xyz/) — ZK proofs for AI

---

*This document captures the long-term vision for Mesh distributed inference. It is not a build plan or a commitment to any specific timeline. The technology landscape — especially FHE, TEEs, and Apple's API roadmap — will determine when each phase becomes practical. The key architectural decision (the inference routing interface in Phase 3) should be designed during Phase 2 so that Permagent is ready to plug in distributed inference when the privacy primitives mature.*
