# NVIDIA PAIR evaluation for Permagent

Date: 2026-09-04  
Status: future TODO; research retained, with no dependency, network, or runtime integration approved

## Verdict

NVIDIA Personal AI Router (PAIR) is a strong candidate for an **optional desktop-side local inference router**. It is not a replacement for Tailscale and it is not split inference in the model-parallel sense.

- **Use PAIR:** route separate Ollama/LM Studio/OpenAI-compatible requests across trusted computers on the same LAN. This can help Permagent Council and worker fan-out use idle local machines.
- **Do not describe it as model splitting:** one request and one model run on one selected node. PAIR does not shard layers, pool GPU memory, or split an in-flight request.
- **Do not use it as the phone transport:** PAIR supports Windows, Linux, and macOS nodes, not iOS; local clients use loopback-only HTTP; a machine that is not a paired PAIR node has no supported ingress.
- **Do not remove Tailscale because PAIR is installed:** PAIR supplies LAN discovery, cluster membership, and mTLS for its own peer services. It does not supply a general private network, WAN NAT traversal, relay fallback, mobile background networking, or access to the Permagent daemon.

## The correct boundary

```text
iPhone Permagent
    │
    │ Permagent secure device link
    │ (Tailscale today; a separate first-party transport if built later)
    ▼
Desktop Permagent daemon
    │
    │ loopback OpenAI/Ollama-compatible request
    ▼
PAIR proxy on 127.0.0.1
    │
    │ PAIR-selected mTLS LAN route
    ▼
One eligible Ollama or LM Studio node
```

PAIR belongs below the daemon as an inference provider. The phone remains a Permagent client, not a PAIR inference node.

## Why PAIR cannot replace Tailscale

PAIR's security and transport are intentionally narrower:

1. It is LAN-first. Discovery uses mDNS, with manual IP entry as a LAN fallback.
2. The plaintext OpenAI/Ollama compatibility endpoint accepts loopback clients only. Remote plaintext requests receive `403`.
3. Cluster ingress uses certificates pinned during a six-digit-PIN pairing flow, but only for PAIR peer services.
4. NVIDIA explicitly says a machine that is not a node has no way into the proxy, and a network-reachable inference endpoint is outside PAIR's scope.
5. PAIR exposes some discovery/node telemetry over unauthenticated plaintext HTTP and treats the LAN as a trust-relevant boundary. NVIDIA warns that the PIN is low entropy and pairing should occur only on a trusted network.

Tailscale solves a different layer: device addressing, encrypted general IP connectivity, NAT traversal across different networks, and encrypted relay fallback when a direct connection is impossible. Replacing it requires solving those capabilities independently.

Porting only PAIR's certificate protocol into the iOS app would be a poor shortcut. It would make the phone a new, unsupported PAIR node; require us to maintain compatibility with PAIR's internal cluster services; expose more cluster control than the phone needs; and still would not provide NAT traversal or relays away from home.

## Recommended PAIR integration DAG

### P0 — Compatibility spike

- Run the signed PAIR release on a supported desktop without changing Permagent defaults.
- Verify `/v1/models`, streaming `/v1/chat/completions`, Ollama chat, embeddings, cancellation, error mapping, and exact model-name behavior through loopback.
- Confirm that PAIR and the existing Ollama provider do not fight over port ownership.

Gate: all traffic remains loopback from Permagent to PAIR, remote plaintext requests are refused, and a feature flag restores the existing provider instantly.

### P1 — Optional provider adapter

- Represent PAIR as a local routing endpoint, not a new model vendor.
- Discover health and supported engines without copying PAIR's control plane.
- Preserve the user-selected model and expose the serving node as a routing receipt when PAIR makes it available.
- Classify inference as local/no API charge while still measuring energy, latency, tokens, and success.

Gate: existing Ollama/LM Studio behavior is unchanged when PAIR is off; no silent fallback to a paid provider; provider conformance tests pass.

### P2 — Multi-worker evaluation

- Hold models/tasks constant and compare one local engine against the PAIR cluster.
- Measure pass rate, time to first token/tool, wall time, queue time, tokens, node assignment, cancellation, and failure recovery.
- Test concurrent Council workers because PAIR's benefit is request-level parallelism.

Gate: non-inferior correctness and a material throughput/latency improvement under concurrency. A single sequential prompt is not sufficient evidence.

### P3 — Production hardening

- Pin a PAIR release and review its Apache-2.0 license and third-party notices.
- Threat-model LAN discovery, low-entropy bootstrap, certificate storage, unauthenticated telemetry, CORS, logs, model downloads, and updates.
- Add health degradation and clean rollback when PAIR or one node disappears.

Gate: security review passes and removing PAIR leaves the original local provider operational.

## Separate Tailscale-removal DAG

Treat this as a distinct product/security project, not part of PAIR integration.

### N0 — Requirements

- Same-LAN and remote cellular/Wi-Fi use.
- iOS background/reconnect behavior.
- No inbound router port forwarding.
- End-to-end authenticated encryption, per-device revocation, key rotation, replay protection, least-privilege service access, and recovery.
- Direct path when possible plus an encrypted relay fallback.

### N1 — Architecture decision

Evaluate three honest options:

1. **Keep Tailscale** for the transport layer and add PAIR only for inference. Lowest risk and fastest.
2. **Self-host a WireGuard-compatible coordination layer** such as Headscale. This reduces hosted control-plane dependency but still relies on the Tailscale protocol/client ecosystem and must be checked carefully on iOS.
3. **Build Permagent Secure Link:** QR/passkey enrollment; device-bound keys in Keychain/Secure Enclave; mutually authenticated QUIC or WireGuard; STUN/ICE-style NAT traversal; an end-to-end encrypted relay fallback; capability-scoped daemon tokens; revocation and rotation; APNs-aware reconnect; security audits and fuzzing. This removes the dependency but is substantial networking and security engineering.

Gate: select by threat model and lifetime ownership cost. “Works on home Wi-Fi” is not sufficient for mobile connectivity.

### N2 — Parallel migration

Run the new link beside Tailscale, compare route success and reconnect latency across home Wi-Fi, guest Wi-Fi, cellular, CGNAT, double NAT, sleep/wake, IP changes, and relay-only networks. Keep Tailscale as rollback until the new path graduates on repeated runs.

Gate: no downgrade in authenticated connectivity or recovery; independent security review; staged opt-in before default change.

## Could PAIR become the Permagent MESH foundation?

This section is subordinate to the existing
`docs/vision/MESH_DISTRIBUTED_INFERENCE_VISION.md`. PAIR does not replace its
Chitin identity, contribution, or confidential-compute direction; it fills one
specific southbound placement role.

Only for one of two meanings of “pool compute”:

| MESH mode | What happens | PAIR fit |
|---|---|---|
| Request federation | Every eligible node holds the whole requested model; independent agent/model calls are sent to different idle nodes | **Useful foundation inside one trusted LAN cluster** |
| Cooperative large-model inference | No node can hold the model; layers, experts, prefill/decode, tensors, or KV cache move between machines during one request | **Not a foundation; PAIR explicitly does not implement this data plane** |

The cleanest long-term composition is a **federation of local PAIR clusters**, not one Internet-sized PAIR cluster. A verified Permagent user could offer a signed capability endpoint representing their local machine or PAIR cluster. The Permagent MESH would own cross-user identity, connectivity, policy, accounting, privacy, and scheduling. PAIR would continue to own local model inventory and request placement within that user's trusted LAN.

```text
Chitin / Base L2 (durable control and settlement)
  identity · device-key commitments · model hashes · reputation · receipts
                              │
Permagent MESH control plane (off-chain hot path)
  privacy policy · capability discovery · leases · scheduling · verification
                              │
Permagent secure transport (cross-user encrypted link)
                              │
Participant Permagent daemon
                 ┌────────────┴────────────┐
      PAIR loopback adapter        distributed-runtime adapter
  independent local requests       layers / prefill-decode / KV
                 │                              │
        trusted LAN nodes          trusted or attested compute pod
```

The blockchain should not host multi-gigabyte model weights or take part in
per-token scheduling. Encrypted, content-addressed model blobs can live on
Arweave or another storage layer; the chain anchors model/version hashes,
eligibility policy, batched contribution receipts, reputation, disputes, and
settlement. Live placement, retries, queue state, ephemeral keys, prompts,
activations, and tokens remain off-chain.

Likewise, Apple Silicon storage encryption is not the MESH confidential-runtime
primitive. FileVault/APFS protects model shards and cached request material at
rest. Public Secure Enclave APIs protect key agreement/signing keys, but do not
run arbitrary transformer inference inside an enclave. A normal Metal or
Neural Engine process exposes plaintext runtime state to its host trust
boundary. Apple's Private Cloud Compute is the relevant architecture pattern—a
hardened system, no privileged runtime access, stateless handling, measured
software, and hardware-rooted attestation—but ordinary third-party Macs are not
PCC nodes. Until an equivalent third-party runtime exists, `encrypted_at_rest`,
`trusted_peer`, `activation_exposed`, and `attested_confidential` must be
separate route labels.

For a model larger than any contributing machine, MESH needs a separate distributed runtime. Relevant research inputs include:

- **NVIDIA Dynamo** for production-style disaggregated prefill/decode, KV-aware routing, cache transfer, and autoscaling. It targets managed clusters and fast interconnects rather than untrusted volunteer devices.
- **exo** for heterogeneous peer-to-peer model partitioning on everyday devices. Its partitioning/data-plane ideas are closer to the desired cooperative mode than PAIR's proxy routing.
- **Petals** as proof that Internet-distributed layer serving can run very large models, and as a warning: its published work identifies data privacy as a limitation because intermediate activations travel to peers without privacy protection.

Do not use llama.cpp's raw RPC backend as an Internet-facing shortcut. Its own security record includes critical unauthenticated remote-code-execution issues, and it is not an untrusted-mesh security boundary.

### Permagent MESH planes

1. **Identity:** verified user plus device-bound keys, revocation, rotation, and optional hardware attestation. User verification alone does not prove that a contributed process or GPU is trustworthy.
2. **Connectivity:** NAT traversal, encrypted direct paths, relay fallback, mobile reconnect, and service-scoped authorization. PAIR does not provide this across the Internet.
3. **Resource:** signed model/artifact hashes, engine/runtime versions, memory, bandwidth, latency, queue depth, thermal/power limits, current ownership, and an explicit execution-confidentiality class.
4. **Scheduling:** request-level federation first; topology-aware sharded inference only among nodes whose bandwidth, latency, model/runtime, and trust policy satisfy the job.
5. **Execution:** content-addressed model shards, deterministic protocol versions, backpressure, checkpoint/retry boundaries, and recovery when a peer disappears mid-token.
6. **Verification:** challenge/canary work, redundant execution where justified, result/evidence receipts, reputation, dispute handling, and quarantine. Correct TLS identity is not proof of correct computation.
7. **Privacy:** explicit workload classes. Sensitive personal prompts should remain on the user's devices or a trusted circle unless a proven confidential-compute design protects them. Ordinary TLS does not hide plaintext/activations from the node doing inference.
8. **Economics:** opt-in resource limits, energy/thermal controls, credits/payments, metering, abuse limits, and model-license enforcement.

### Safe sequence

1. **M0: trusted-home federation** — Permagent routes independent calls across one user's PAIR cluster.
2. **M1: trusted-circle federation** — verified users explicitly share whole-model capacity for non-sensitive jobs through the Permagent secure link. No model sharding.
3. **M2: high-bandwidth cooperative pods** — experiment with exo/Dynamo-style partitioning among a small, mutually trusted, wired cluster. Intermediate activations are classified as exposed unless a separately qualified runtime proves otherwise. Measure tensor/KV traffic and fault recovery.
4. **M3: attested confidential execution** — require measured runtime and model hashes, per-request keys bound to an attestation, protected runtime memory, no privileged inspection, non-retention, revocation, and independent verification. Do not infer this property from FileVault, Secure Enclave key storage, or TLS.
5. **M4: adversarial mesh research** — only after identity, computation verification, privacy classification, NAT/relay, and abuse economics pass dedicated threat-model gates; evaluate FHE/ZK only against measured transformer-scale feasibility.

PAIR therefore gives MESH a useful **southbound local-cluster adapter** and some discovery/scheduling lessons. It should not become MESH's cross-user identity system, secure overlay, or cooperative inference protocol.

## Primary sources

- NVIDIA PAIR repository and supported platforms: <https://github.com/NVIDIA/Personal-AI-Router>
- NVIDIA PAIR overview: <https://docs.nvidia.com/local-ai/nvpair/>
- NVIDIA PAIR architecture: <https://docs.nvidia.com/local-ai/nvpair/architecture/>
- NVIDIA PAIR getting started and loopback-only client rule: <https://docs.nvidia.com/local-ai/nvpair/getting-started/>
- NVIDIA PAIR security model and cautions: <https://github.com/NVIDIA/Personal-AI-Router/blob/main/SECURITY.md>
- Tailscale encryption and relays: <https://tailscale.com/docs/concepts/tailscale-encryption>
- Tailscale NAT traversal: <https://tailscale.com/blog/how-nat-traversal-works>
- NVIDIA Dynamo: <https://docs.nvidia.com/dynamo/>
- exo distributed inference: <https://github.com/exo-explore/exo>
- Petals paper: <https://arxiv.org/abs/2209.01188>
- llama.cpp RPC security advisories: <https://github.com/ggml-org/llama.cpp/security/advisories>
- Apple hardware security overview: <https://support.apple.com/guide/security/hardware-security-overview-secf020d1074/web>
- Apple Secure Enclave CryptoKit APIs: <https://developer.apple.com/documentation/cryptokit/secureenclave>
- Apple Private Cloud Compute architecture: <https://security.apple.com/blog/private-cloud-compute/>
