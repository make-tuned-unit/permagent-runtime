# Bonsai 2 27B as a local model for Permagent — review

**Date:** 2026-09-20 · **Status:** research + read-only inspection of the M4; nothing downloaded, built or changed.
**Asked by:** Jesse — "review Bonsai 2 27B to see if it would be a good local model for us."
**Raw notes, every URL, and the raw command output:** `docs/research/agent-market-evidence/bonsai2-27b-review.md`.
**Labels:** `[VENDOR]` PrismML's claim · `[INDEPENDENT]` measured or reported by someone else · `[LOCAL]` observed on
this machine today · `[UNVERIFIED]` single-sourced or not confirmable. The research lane ran out of search budget
before reaching r/LocalLLaMA; Hacker News is the only independent community source.

---

## 0. Verdict

| Use | Verdict | Why |
|---|---|---|
| **Librarian (nightly describe / adjudicate / consolidate)** | **Live candidate — bench it on the M1** | Short-prompt, batch, non-agentic, vision-free: exactly where extreme quantisation hurts least. Would retire the two-mini split. Blocked only by needing a vendor fork of llama.cpp as an unattended service. |
| **Daytime harness worker** | **No** | Tool calling is the gate and BFCL v3 is 74.92 — no better than the ~72 % 9B-class figure we already measured; agentic coding retains only ~75 % of full precision; independent reports of looping. And the M4 has no memory to give it. `gemma-4-26B-A4B` is already in `FEATURED_MODELS` with native tool calling on *stock* llama.cpp. |
| **Typed-decision backend (logprobs)** | **No** | A yes/no is a sub-1B job. Fork logprob support is undocumented, calibration at 1.72 bits is unknown, and we have no logprob plumbing at all. |

**Re-evaluate immediately if PrismML upstreams `PTQ1_0` / `PQ2_0` into mainline llama.cpp.** That erases the largest
objection, and they did upstream Bonsai 1's types.

**The more important finding is not about Bonsai** — see §3: the nightly 27B split has not run in weeks.

---

## 1. What it is

PrismML, released 2026-09-17. **Post-training quantisation** of Alibaba's Qwen3.8-27B — not QAT, not distillation
`[VENDOR]`. Blockwise Hadamard rotation (block 1024) before ternary assignment; groups of 128 weights share one FP16
scale; 1.71–1.72 bits/weight. Only 26.2M parameters (0.098 %) stay high-precision. The 0.47B vision tower is a separate
0.63 GB Q8_0 `mmproj`, not loaded for text-only. Architecture unchanged: hybrid attention (~75 % linear / ~25 % full),
262K context, reasoning on by default, Qwen chat template, OpenAI-shaped native tool calling. **Apache-2.0**, no extra
restrictions. 5.95 GB (`PTQ1_0`) or 7.21 GB (`PQ2_0`).

### Runtime support — the crux

- Model card: ***"Stock llama.cpp will not run these files."*** Mainline rejects the quant types as unknown.
- **A silent-failure trap:** a `Q2_0`-named Bonsai file *loads* on stock builds and **emits garbage with no warning**
  (no Hadamard activation support). Third-party re-uploads — including an `ollama.com` entry that is **not PrismML** —
  are exactly this. Download only from `prism-ml/` on Hugging Face.
- **Required:** PrismML's fork, `github.com/PrismML-Eng/llama.cpp`, tag `prism-b10658`+. Prebuilt macOS arm64, Metal
  supported, `llama-server` works and is OpenAI-compatible. **Not upstream; no timeline.**
- **Ollama: no. LM Studio: no.**
- **MLX:** stock `mlx-lm`, but only through a bundled Python helper with a custom `model_type`; generic loaders emit
  wrong output. No OpenAI-compatible MLX server documented → a Python sidecar, which we have ruled out.
- Logprobs (`n_probs`) in the fork: undocumented `[UNVERIFIED]`. iOS/iPad claims: unsubstantiated for Bonsai 2 (the
  11 tok/s iPhone figure is Bonsai **1**).

### Performance on Apple Silicon

**No measurement exists for any base M1/M2/M3/M4 with 16 GB.** Published: M5 Max 46.8 tok/s, M5 Pro 28.1 gen / 387
prefill, M4 Pro 18 (`[VENDOR]`, flagged "pre-rotation, pending re-measurement"); an HN user on M4 Pro ~15 gen / ~100
prefill at 64k `[INDEPENDENT]`. TTFT: never published. PrismML publishes **no memory tables**. One blog claims ~64
KiB/token KV and a `BONSAI_KV4=1` switch (~3.5× cache cut) `[UNVERIFIED]`. Our extrapolation for a base M4 is
**8–14 tok/s** — a guess, not a number.

### Quality

`[VENDOR]` aggregate 83.9 vs 85.4 = 98.2 %. What that hides: it is measured at **maximum reasoning effort**; at medium
effort it is **96.0 %**; and on **agentic coding (Terminal-Bench 2.1, SWE-bench Verified) it retains only ~75 %**
`[INDEPENDENT]` analysis of vendor tables. Per category `[VENDOR]`: **BFCL v3 74.92 vs 76.74**, τ²+BFCL agentic 77.57
vs 79.74, MMLU-Redux 89.09 vs 91.46, MMMU-Pro 75.49 vs 81.73, OCRBench 56.88 vs 60.99; AIME / MATH / HumanEval+ /
IFEval at or above parity. **Every number is vendor-reported** — at launch nobody outside PrismML had published one.
HN `[INDEPENDENT]`: rare-token damage (a recited poem degrades after one stanza), a WebGPU demo that loops
immediately, several reports of long-context drift and looping in extended reasoning, accusations of cherry-picking,
and several users finding **Qwen3.8-27B Q4 comparable-or-better and more reliable**.

**Vendor credibility.** Weights Apache-2.0, kernels genuinely open in a public fork — better than most. Bonsai 1's
ternary type did land upstream. Red flags: two conflicting aggregate figures (83.9 on 20 benchmarks vs 84.78 on 14),
two conflicting sampling recommendations, no memory tables, and the worst-case numbers disclosed by a third party.

---

## 2. Our side `[LOCAL]`

- **M4, 16 GiB, macOS 26.2.** `iogpu.wired_limit_mb: 0` (default).
- **No headroom:** the split script's own preflight reports *"only 0MB genuinely available (need 9500MB)… swap
  13245/14336MB used, pressure level 2."* **Disk 23 GiB free, 95 % full**; `~/.local/models` already holds 18 GB.
- **Installed llama.cpp is Homebrew build 10470, mainline** — ~190 builds behind the fork's base *and* the wrong
  lineage. Ollama 0.32.9, resident with a 4.35 GB llama-server child. `mlx_lm.server` not installed.
- **Rust in-process provider:** `llama-cpp-2` / `-sys-2` 0.1.154 from crates.io, and the sys crate **vendors its own
  llama.cpp tree** → a custom quant type means **forking the sys crate**, not bumping a version. Separately,
  `local_inference.rs:549` forces every tool call through the text emulator (llama-cpp-rs#1037), discarding native
  tool calling.
- **No `n_probs` / `top_logprobs` request plumbing anywhere in `crates/goose/src/`** — a logprob decision backend is
  unbuilt on our side regardless of model (consistent with the Jev analysis).
- `FEATURED_MODELS`: Llama-3.2-1B/3B, Hermes-2-Pro-7B, Mistral-Small-24B (`native_tool_calling: false`), and
  **gemma-4-E4B** / **gemma-4-26B-A4B** (`native_tool_calling: true`, with mmproj).

### Integration paths, ranked

1. **Forked `llama-server` over HTTP — the only viable path, and it needs zero Rust changes.** `librarian.rs` already
   speaks OpenAI SSE to a llama-server with a three-stage readiness probe; repoint `PERMAGENT_LIBRARIAN_ENDPOINT`.
   Needs `--jinja`, an explicit `-c` (the docs warn `-c 0` exhausts memory), `-ngl 99 -fa on`.
2. **MLX sidecar** — blocked by the no-Python-sidecar rule; nothing installed.
3. **In-process GGUF** — blocked, and the worst option even if unblocked.

---

## 3. Operational finding: the nightly split is not running

Verified directly in `/Users/j/.local/llama/logs/qwen38-split.log`:

- **All six nightly start slots refused on 27 separate nights**, including **every night 2026-09-09 → 09-20**, each
  with *"refusing — only 0MB genuinely available (need 9500MB)."*
- **`llama-server.log` was last written 2026-08-25.** The 27B has not served since. The log line itself says
  *"Librarian will use its Ollama fallback"* — so for weeks the Librarian has been running on **`qwen2.5:7b`**.
- Named memory holders at 01:50 include **three `redaction_real.py` processes from the Sidebar project, running for
  6+ days**. *Not touched — they belong to another project; Jesse's call.*
- The split script's `LLAMA_BIN` defaults to `/Users/j/.local/src/llama.cpp/build/bin`, which **does not exist** (the
  checkout at `04b56914` was never built). Even with memory free, the server would fail to start.

Consequences: (a) the real incumbent any replacement must beat is **`qwen2.5:7b`, not Qwen3.8-27B-Q4** — a much lower
bar; (b) memories described since late August were described by a 7B; (c) this failure was silent for weeks — the
job-health digest enumerates *expected* jobs, and it is worth checking why "split refused six times" did not surface;
(d) the case for a Librarian model that fits on **one** machine is stronger than when this review was commissioned,
whichever model it turns out to be.

---

## 4. Bench plan — for approval, nothing run yet

**Prerequisites.** Run on the **M1, not the M4** (the M4 has no headroom and is the dev machine; the M1 is what a
single-box Librarian would free). *The M1's own free memory and disk were not inspected — check first.* Free ≥ 25 GiB
wherever the model lands. Download **5.95 GB** text-only (6.6 GB with mmproj); ~15 GiB including the fork.

| Step | What | Go / no-go |
|---|---|---|
| 0 | Prebuilt `prism-b10658`+ macOS-arm64 into `~/.local/bonsai2`, **checksum-verified, not on `PATH`**. `huggingface-cli download prism-ml/Ternary-Bonsai-2-27B-gguf Ternary-Bonsai-2-27B-PTQ1_0.gguf` | — |
| 1 | `llama-server -m …PTQ1_0.gguf --port 8082 --host 127.0.0.1 -ngl 99 -fa on -c 8192 --jinja --temp 0.7 --top-p 0.80 --top-k 20 --metrics`. **Port 8082** — 8080 is the Picker, 8081 the split. Repeat at `-c 32768`, and once with `BONSAI_KV4=1`. | starts |
| 2 | **Not-broken gate, before any timing**: recite a known passage, one factual question, strict JSON ×10. Exists because of the silent-garbage mode and the reported looping. | coherent, non-looping, 10/10 valid JSON |
| 3 | 20 runs at 700- and 8,000-token prompts: prefill + generation tok/s, TTFT from first SSE token, peak RSS at 1 Hz. | **≥ 10 tok/s generation AND peak RSS ≤ 11 GB at `-c 32768`** |
| 4 | **Decisive.** 50 real memories, all three Librarian prompts, **blind A/B against the incumbent that is actually running — `qwen2.5:7b`** — and, if pre-08-25 split outputs are on record, against those too. | wins-or-ties **≥ 85 %** on describe/categorise vs the 7B; **≥ 90 %** supersession agreement (stricter: supersession is destructive) |
| 5 | Only if 4 passes: 30 single-turn + 10 multi-turn tool tasks from the harness set. Expect failure; run it for our own number. | ≥ 90 % single-turn for any worker-tier use |
| 6 | Optional: `n_probs: 5` on ten forced binary choices. | returned at all; correct option higher on ≥ 9/10 |

**Add one arm:** the same steps 3–4 for a **stock-llama.cpp model that fits one mini** — `gemma-4-26B-A4B:Q4_K_M` is
already featured — so the decision is "best single-box Librarian model", not "Bonsai or nothing". A model that needs no
fork wins a tie.

Cost: ~6 GB, ~15 GiB, one evening on the M1. No change to the M4, the daemon, or the (non-running) split.

---

## 5. Risks and unknowns

Risks: an unattended nightly service on a fork with no upstreaming timeline; **silent wrong output** from any
mislabelled re-upload; all quality numbers vendor-supplied; long-context drift and looping reported independently;
disk at 95 %. Unknowns: no base-M-series 16 GB measurement exists anywhere; TTFT and peak memory unpublished; KV-cache
and `BONSAI_KV4` figures rest on one blog; fork logprob support undocumented; the M1's current state.
