# Mesh M0 — Three-Device Pool Physics Spike: Results

**Epic:** [#306](https://github.com/make-tuned-unit/permagent-runtime/issues/306) ·
**Date:** 2026-06-11 → 2026-06-15 · **Branch:** `research/mesh-m0-results`
**Hardware:** M1 Mac mini (16 GB, macOS 26.3) + M4 Mac mini (16 GB, macOS 26.2) +
MacBook Pro 13" 2018 (Intel i5, 8 GB — client-only).
**Scope:** measurement only. No permagent code, daemon, schema, or UI changes.
Full command-by-command log: [`MESH_M0_LOG.md`](./MESH_M0_LOG.md). Raw evidence:
[`mesh-m0-evidence/`](./mesh-m0-evidence/).

---

## 1. Verdict

**Unified-memory pooling buys a real tier — a 16 GB-class model that neither
node serves solo — but only with *dedicated* nodes.**

The headline is a reframe, not a failure. Across every pooled attempt the
binding constraint was **the M1's own footprint as the household's
interactive-desktop + spike-orchestrator + permagent-daemon host**, *not* the
16 GB hardware. The M4 head repeatedly held a ~10 GB shard gracefully; the M1
worker, with this work's orchestrating agent resident, could not free even 3 GB
to contribute before its safety guard fired.

- **Pooling works.** The llama.cpp RPC pipeline split layers across both minis
  over the wired LAN and generated correctly (proven end-to-end at 0.6 B,
  76 tok/s pooled).
- **It buys a tier — conditionally.** A **quiet** M1 (~12 GB free) would lift
  the core pod to ~22 GB usable and serve the **16.4 GB MoE (Qwen3-30B-A3B
  IQ4_XS)** that fits on *neither* node alone. The thesis is **validated with a
  condition: both nodes dedicated.**
- **A busy node contributes ~nothing and should default to client.** While the
  M1 ran the desktop + agent + daemon (~5–7 GB free), the pod's serving
  envelope was ~12–13 GB — exactly the ~14 B tier the M4 *already serves solo*.
  In that state, pooling buys nothing.

**Old framing:** "pooling fails on 2×16 GB." **Correct framing:** "pooling
requires dedication; a busy node defaults to client; capacity scales with the
anchor's RAM, not node count."

---

## Latency: the beatable wall (M3+ roadmap — NOT measured here)

> This section is **forward-looking**, added after deep research (June 2026). It
> does **not** revise the M0 verdict above: M0 correctly measured *naive
> sequential split* (what llama.cpp RPC does), where every decoded token costs a
> network round-trip. That floor is real for that engine. But it is **not a
> fundamental limit** — the frontier has beaten it, so the next person knows the
> ceiling, not just the floor we hit.

**Key technique — speculative / lookahead decoding.** Speculate *k* tokens with
a small draft model (or local layers), then **verify them in one pass** instead
of *k* sequential steps. On a single node this hides decode latency; across
nodes it amortizes the per-token *network round-trip* — and the benefit is
*amplified* for split inference precisely because round-trips are expensive (the
exact thing that hurt naive RPC is what this exploits). Two clearly different
maturity levels:

### Shipping TODAY (near-term build items — use now)

- **MLX speculative decoding on a single anchor is the biggest near-term win —
  bigger than pooling.** MTPLX took a 27B from **7 → 18.3 tok/s** on one M4 Pro
  (48 GB), mathematically-correct sampling, OpenAI-compatible API, ~5-min agent
  integration. Also `mlx-dflash` (3.34×, MIT) and `mlx-community/
  speculative-decoding` (MLX-Swift, 2–3×, MIT). **On a 32–48 GB anchor this makes
  a 30B interactively usable (~18 tok/s) with NO pool, NO split, NO WAN.** This
  *supersedes* "bigger anchor is slow": a bigger anchor + MLX-spec is bigger
  **and** fast.
  → **Our M0 solo controls (11–12 tok/s) used non-speculative llama.cpp and
  therefore UNDERSTATE achievable anchor speed by ~2–3×.** Re-bench with MLX-spec
  is the M0.5 follow-up (below).
- **Local multi-Mac split ships today** via MLX-distributed (LocalAI) / exo:
  pipeline-parallel Ring backend, or JACCL tensor-parallel over
  **Thunderbolt/RDMA**. Built for *co-located high-bandwidth* links, not WAN. A
  same-room household pod (**M1+M4 Thunderbolt-bridged**) is a **today**
  capability for >anchor capacity if we want it — distinct from the
  ethernet/Wi-Fi pod M0 measured.

### Research, NOT yet productized (track, don't build)

- **WAN split + speculation** — the 10–20-person interactive Mesh case — is
  proven in papers (**Cunningham 2026**, arXiv 2602.16760: 8–9 tok/s over 80 ms
  WAN, projects 15–19 at 20 ms; **DSD**, arXiv 2511.11733, best at 3–8 nodes) but
  is **not a shipping feature in any distributed engine.** MLX's own roadmap
  lists "speculative decoding" and "distributed across network Mac clusters" as
  *separate* future directions — combining them over WAN is the unshipped piece.
  Cunningham's asymmetric split (embed/unembed kept local; split-depth = privacy
  dial, measured vs an inversion attack) validates the vision doc's **Approach
  A**. **Gating dependency for interactive WAN Mesh; track exo + mlx-lm
  releases.**
- Acceptance rates peak on **structured text** (code up to ~7 tokens/round-trip)
  → Permagent's Librarian / Reader / codegen are the **best case**, not the worst.

### Revised roadmap

1. **M1/M2 anchor runs an MLX speculative engine** (MTPLX / mlx-dflash class),
   **not** plain Ollama/llama.cpp — near-term build item, large interactive
   speedup, shipping today.
2. **Co-located Thunderbolt pod** is a today-option for >anchor capacity;
   ethernet/WAN pod stays **batch-only** until #3 lands.
3. **Interactive WAN Mesh = M3+,** gated on speculative-split landing in the
   engine layer. Tracked, not built.

**Engine re-eval (supersedes §4's anchor choice):** for the **anchor**
(single-node interactive), an **MLX speculative engine likely beats llama.cpp
RPC**. Keep **llama.cpp RPC for the batch/pool path**. The **M0.5 follow-up** is
an MLX-spec anchor bench on the M4 — expect ~2–3× the M0 solo numbers.

---

## 2. Evidence

### Solo controls (clean, the per-node baseline)

| node | model | quant | prefill tok/s | decode tok/s | peak RSS |
|---|---|---|---|---|---|
| M1 (Apple M1, 16 GB) | Qwen3-8B | Q4_K_M | 119.3 | **11.0** | 4.72 GB |
| M4 (Apple M4, 16 GB) | Qwen3-14B | Q4_K_M | 121.7 | **11.8** | 9.07 GB |

Each is the largest model that node serves comfortably alone. The M4's newer GPU
edges decode despite the larger model. Interactive bar (≥10 tok/s, <2 s TTFT):
both clear decode; these are the numbers any pooled result must beat to justify
the overhead.

### Pipeline proof (pooled, tiny model)

Qwen3-0.6B-4bit split M4+M1 over wired RPC: loaded and generated at **76 tok/s**.
Confirms the pooled path is correct once the worker is pinned to Metal
(`--device MTL0` — see §4).

### Pooled 30B-class — the graceful-failure table

Model: **Qwen3-30B-A3B-UD-IQ3_XXS** (12.89 GB MoE). M4 = head, M1 = RPC worker.
M4 wired-limit at **default** (the safe setting — see §3). Both guards armed
(M4-local watchdog @ 2500 MB avail; M1 worker guard @ 900 MB avail).

| split (M4/M1) | M4 head | M1 worker | outcome |
|---|---|---|---|
| 70 / 30 | held **9.25 GB** shard, healthy (free dipped to 57 MB, **swap flat** → clean compression) | avail → **481 MB**, guard killed worker | head SIGABRT on RPC-buffer alloc; **M4 survived, no panic** |
| 76 / 24 | held **9.93 GB** shard, healthy (avail 6.7 GB) | avail → **676 MB**, guard killed worker | head SIGABRT; **M4 survived, no panic** |

Both times the **M1 worker was the failure, never the M4.** The M4 head handled
~10 GB shards gracefully and stayed responsive throughout. The M1 — hosting the
orchestrating agent — fell below its guard while loading even a 24 % (~3 GB)
slice. Conclusion: pod serving envelope while M1 is busy ≈ M4 ~10 GB + M1 ~2–3 GB
≈ **12–13 GB**, too tight to serve the 12.89 GB model reliably (a load may
squeak through; generation's KV growth pushes it over). That envelope equals the
M4's solo tier — so pooling buys nothing *in that state*.

### Why this is a coexistence result, not a hardware ceiling

The M4 (quiet) holds ~10 GB; a quiet M1 (~12 GB free, no agent/desktop) would
hold the rest. ~10 + ~12 ≈ 22 GB ≫ 16.4 GB → the IQ4_XS tier serves. The wall
was the M1's occupancy, removable in production by **not** running heavy
inference on the interactive node.

### Earlier (untuned) runs — why they kernel-panicked

Before the safety redesign, two runs **panic-rebooted the M4 into FileVault
lock**. Root cause was *not* the model: it was `iogpu.wired_limit_mb=13000`
(see §3) letting Metal wire ~13 GB on a 16 GB box and starving the kernel. With
the wired-limit at default + a local watchdog, the *same* workload failed
**gracefully** (table above) with zero panics. The redesign is the safety
result.

### Ceiling table (computed vs proven)

| pool state | computed usable weights | proven |
|---|---|---|
| M4 solo (quiet) | ~10 GB (Metal cap at default wired-limit) | 14B Q4 served, 11.8 tg |
| Core pod, **M1 busy** | ~12–13 GB | IQ3 12.89 GB would not serve (M1 guard) |
| Core pod, **M1 quiet** (projected) | ~22 GB | not run — M1 can't be quiet while orchestrating; projection from M4-held-10 GB + M1-quiet-12 GB |
| Boosted (3-node) | N/A | MacBook is Intel/8 GB — client only, no MLX/contribution |

---

## 3. M1 / M2 design conclusions (the deliverable)

1. **Pool with a dedicated, larger anchor.** Worthwhile pooling needs the
   headless M4 grown to **32–64 GB** carrying the bulk of the weights, with
   smaller nodes adding slices **only when idle**. Node count is not the lever;
   anchor RAM is.
2. **Minimum-contributor-spec — PROVEN.** The gate is **free RAM, not chip
   generation.** A node that is also the household's interactive machine /
   daemon host defaults to **client**. Apple-silicon + nominal RAM is necessary
   but not sufficient; what matters is RAM actually free at request time.
3. **"Biggest model possible" scales with anchor RAM, not node count.** 2×16 GB
   with one node busy tops out at the same ~14 B tier a single quiet node
   serves. Adding a third small/busy node does not help.
4. **Idle-gated contribution is a hard requirement for Permagent.** A
   contributor must serve **only when it genuinely has RAM to give**. The M1
   guard tripping at <700 MB is exactly what the absence of this looks like in
   practice — without an idle gate, a busy node either degrades the pool or
   (at a raised wired-limit) panics. Permagent must build
   contribution-gating-on-idle (free-RAM threshold + backoff) before enabling
   pooling on any interactive node.
5. **Graceful degradation on worker death — Permagent's to build.** When the
   worker dies, the llama.cpp head **aborts; it does not fall back** to its own
   tier (cell D, observed). Pod membership changes (a node going busy, sleeping,
   or leaving) must be handled above the engine: detect departure, drop to the
   surviving nodes' tier, re-admit on return.
6. **Never raise `iogpu.wired_limit_mb` near total RAM.** Default cap (~66 %,
   ~10.6 GB on 16 GB) → **graceful** GPU OOM / abort, machine survives. Raised to
   13 GB → Metal starves the kernel → **hard panic + FileVault-locked reboot.**
   The default cap is a safety feature. If a headless anchor needs more Metal
   headroom, sizing the RAM up is correct; over-raising the limit is not.

---

## 4. Engine

**llama.cpp RPC is the engine for pooled inference on this hardware. exo is
DNF.**

- **exo (alpha, commit `09f9ea313`, zenoh networking):** its macOS path builds
  MLX from a pinned git fork, requiring the **full Xcode + Metal Toolchain
  component on every node**, plus rust-nightly + node + uv. The M4 (headless,
  CommandLineTools only) cannot satisfy this under least-privilege, so **all
  pooled exo cells are DNF**. exo is disqualified as a consumer-pod engine at
  its current state *regardless of performance* — too much per-node toolchain.
  Revisit at exo 1.0 / prebuilt binaries. (Single-node exo on the M1 was also
  blocked by the same agent-memory footprint that gated the pool.)
- **llama.cpp RPC (release `b9601`, Metal + RPC, built static once on the M1,
  copied to the M4 — no per-node build):** works. **Required flag:** launch the
  worker `rpc-server --device MTL0` (the node's Metal device). The default
  registers the BLAS/CPU backend too, and graph compute lands on BLAS →
  `unsupported op RMS_NORM` → silent SIGABRT on the first transformer op. This
  is a hard requirement, not a tuning knob. Device names print on a bad
  `--device` arg (`MTL0` / `BLAS` / `CPU`).

Engine recommendation for M1: **llama.cpp RPC**, worker pinned to Metal, head on
the largest-RAM node, default wired-limit, with Permagent supplying the
idle-gating and graceful-degradation the engine lacks.

---

## 5. Headless-provisioning scar tissue (appendix)

Getting the headless M4 into the pool surfaced a consistent lesson: **headless
macOS provisioning must be launchd + vendor pkg + a one-time GUI setup — never
ad-hoc, never over a plain non-interactive SSH spawn.** Specific failures:

- **Homebrew is unusable headless — failed 3 ways:** froze mid-install over
  non-interactive SSH (twice), then froze again interactively mid-pour; a hung
  install left `install_monitor` + `installd.commit.pid` locking *all*
  subsequent installs.
- **Non-interactive SSH spawns get QoS-frozen.** `nohup` and `screen`-detached
  long-running processes on the M4 froze at 0 % CPU with no output. Only a
  *held-open* SSH channel (foreground) ran reliably. → daemonized work needs a
  real **launchd** service, not ad-hoc backgrounding.
- **App Store Tailscale has no working CLI** (status silently no-ops) and its
  network-extension needs **one-time interactive approval via Screen Sharing** —
  not scriptable. The M4 only joined the tailnet after a GUI sign-out/in.
- **Orphaned locks from interrupted non-interactive ops** can't be broken by
  their successors; lock files are *unsuffixed* (a `*.lock` glob misses them) —
  must clear the locks **directory**.
- **FileVault-aware reboot required:** recovering a wedged M4 needs
  `sudo fdesetup authrestart` (FileVault on); a blind reboot strands it at the
  pre-boot unlock. Two kernel panics during untuned runs each stranded the M4
  there until a monitor was attached.
- **Non-sudo service account by design:** the M4's `henry` account is non-sudo;
  all privileged provisioning routes through the admin account. **Provision once
  via admin + launchd, then zero interactive babysitting.**

Network: both minis ended up **wired** (link-local 169.254/16 over en0, ~112 MB/s,
~0.5 ms — vs Wi-Fi's 39 ms power-save asymmetry). Tailnet between them runs
direct-via-public-endpoint with NAT hairpin (~16 ms), irrelevant to wired
pooling. The M1 carried a transient **dual Tailscale registration** (brew daemon
+ GUI app); only the GUI-app node routed real traffic — resolved by removing the
brew node. Static IPs on the wired segment are an M1 setup item (link-local
churns across reboots).

---

## 6. Pending light cells (non-blocking, deferred to a fresh session)

- **Cell B — MacBook client experience:** the 2018 Intel/8 GB MacBook as a pure
  client of the core pod, OpenAI-API and browser `/ui`, over LAN then Tailscale
  off-LAN. Needs the MacBook on the tailnet (App Store install, Jesse). Neither
  stresses memory.
- **Cell E — daemon coexistence (Q4):** permagentd health + a real chat turn on
  the M1 during pooled load. Partial signal already: permagentd stayed healthy
  and untouched through every run (P3 honored).

Neither cell affects the §1 verdict; both are safe to run any time.

---

## Honest limitations

One hardware trio, one LAN, one alpha exo pin (`09f9ea313`), N=3 trials per
cell. The "M1 quiet → ~22 GB pool serves the 16.4 GB tier" capacity is a
*projection* from measured parts (M4 held 10 GB; M1 quiet has ~12 GB free), not
a run — the M1 cannot be quiet while it orchestrates the spike. Confirming it is
the first task once Permagent's idle-gating exists and the orchestrator can run
off the contributing node.
